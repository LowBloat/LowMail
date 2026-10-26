//! HTTP API and pages.

use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Multipart, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use futures::Stream;
use lowmail_core::service::MailClient;
use lowmail_core::store::Store;
use lowmail_core::{Error, ErrorKind};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::{BroadcastStream, IntervalStream};
use tower_http::compression::CompressionLayer;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::assets;
use crate::config::Config;
use crate::sessions::{LoginLimiter, Session, Sessions};

pub const COOKIE: &str = "lowmail_session";
/// Header every state-changing API request must carry. Browsers do not let
/// other sites set it without a CORS preflight, which this server never
/// answers, so it blocks cross-site request forgery.
pub const CSRF_HEADER: &str = "x-lowmail";

pub struct AppState {
    pub config: Config,
    pub store: Arc<Store>,
    pub sessions: Sessions,
    pub limiter: LoginLimiter,
}

pub type Shared = Arc<AppState>;

pub struct ApiError {
    status: Option<StatusCode>,
    error: Error,
}

impl From<Error> for ApiError {
    fn from(error: Error) -> Self {
        ApiError { status: None, error }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status.unwrap_or(match self.error.kind {
            ErrorKind::Auth | ErrorKind::SessionExpired => StatusCode::UNAUTHORIZED,
            ErrorKind::Invalid => StatusCode::BAD_REQUEST,
            ErrorKind::NotFound => StatusCode::NOT_FOUND,
            ErrorKind::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ErrorKind::Unsupported => StatusCode::NOT_IMPLEMENTED,
            ErrorKind::Server => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorKind::Network | ErrorKind::Tls => StatusCode::BAD_GATEWAY,
            ErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        });
        if status.is_server_error() {
            tracing::warn!(kind = ?self.error.kind, detail = ?self.error.detail, "{}", self.error.message);
        }
        (status, Json(&self.error)).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

pub fn router(state: Shared) -> Router {
    let api = Router::new()
        .route("/config", get(config))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/session", get(session_info))
        // Drafts carry pasted images inline, so messages can be large.
        .route("/rpc/{method}", post(rpc).layer(DefaultBodyLimit::max(48 * 1024 * 1024)))
        .route("/events", get(events))
        .route("/upload", post(upload).layer(DefaultBodyLimit::max(lowmail_core::service::MAX_UPLOAD + 1024 * 1024)))
        .route("/download/attachment", get(download_attachment))
        .route("/download/message", get(download_message))
        .route("/download/contacts", get(download_contacts))
        .layer(middleware::from_fn(csrf))
        .layer(SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-store")));

    Router::new()
        .nest("/api", api)
        .route("/healthz", get(|| async { "ok" }))
        .fallback(get(static_file))
        .layer(DefaultBodyLimit::max(8 * 1024 * 1024))
        .layer(CompressionLayer::new())
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(SetResponseHeaderLayer::overriding(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer")))
        .layer(SetResponseHeaderLayer::overriding(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("camera=(), microphone=(), geolocation=(), interest-cohort=()"),
        ))
        .with_state(state)
}

async fn static_file(uri: axum::http::Uri) -> Response {
    assets::serve(uri.path())
}

async fn csrf(request: Request, next: Next) -> Response {
    let safe = matches!(*request.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe && request.headers().get(CSRF_HEADER).is_none() {
        return (StatusCode::FORBIDDEN, Json(Error::invalid("Missing request header."))).into_response();
    }
    next.run(request).await
}

fn client_ip(state: &AppState, addr: SocketAddr, headers: &HeaderMap) -> IpAddr {
    if state.config.trust_proxy
        && let Some(forwarded) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        && let Some(first) = forwarded.split(',').next()
        && let Ok(ip) = first.trim().parse()
    {
        return ip;
    }
    addr.ip()
}

fn secure_request(state: &AppState, headers: &HeaderMap) -> bool {
    state
        .config
        .secure_cookies
        .unwrap_or_else(|| headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok()).is_some_and(|v| v.eq_ignore_ascii_case("https")))
}

fn session(state: &AppState, jar: &CookieJar) -> ApiResult<(String, Arc<Session>)> {
    let id = jar.get(COOKIE).map(|c| c.value().to_string()).unwrap_or_default();
    match state.sessions.get(&id, state.config.session_idle) {
        Some(s) => Ok((id, s)),
        None => Err(Error::new(ErrorKind::SessionExpired, "Your session ended. Sign in again.").into()),
    }
}

async fn config(State(state): State<Shared>) -> Json<Value> {
    Json(json!({
        "productName": state.config.product_name,
        "domain": state.config.default_domain,
        "version": env!("CARGO_PKG_VERSION"),
        "filters": state.config.sieve.is_some(),
    }))
}

#[derive(Deserialize)]
struct LoginBody {
    email: String,
    password: String,
}

async fn login(
    State(state): State<Shared>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(body): Json<LoginBody>,
) -> ApiResult<(CookieJar, Json<Value>)> {
    let email = state.config.normalize_login(&body.email).map_err(Error::invalid)?;
    let ip = client_ip(&state, addr, &headers);
    if state.limiter.blocked(ip, &email) {
        return Err(ApiError {
            status: Some(StatusCode::TOO_MANY_REQUESTS),
            error: Error::new(ErrorKind::Auth, "Too many failed attempts. Wait a few minutes and try again."),
        });
    }
    if body.password.is_empty() {
        return Err(Error::invalid("Enter your password.").into());
    }
    let account = state.config.account_for(&email);
    let client = match MailClient::connect(account, body.password, state.store.clone(), email.clone(), state.config.pool_size).await {
        Ok(c) => c,
        Err(e) => {
            if e.kind == ErrorKind::Auth {
                state.limiter.failed(ip, &email);
                tracing::info!(user = %email, %ip, "sign-in failed");
            } else {
                tracing::warn!(user = %email, detail = ?e.detail, "could not reach the mail server: {}", e.message);
            }
            return Err(e.into());
        }
    };
    state.limiter.succeeded(&email);
    tracing::info!(user = %email, %ip, "signed in");
    let info = client.info();
    let id = state.sessions.create(client, email);
    let cookie =
        Cookie::build((COOKIE, id)).http_only(true).same_site(SameSite::Strict).secure(secure_request(&state, &headers)).path("/").build();
    Ok((jar.add(cookie), Json(json!({ "account": info }))))
}

async fn logout(State(state): State<Shared>, jar: CookieJar) -> (CookieJar, Json<Value>) {
    if let Some(c) = jar.get(COOKIE) {
        state.sessions.remove(c.value()).await;
    }
    (jar.remove(Cookie::build(COOKIE).path("/")), Json(json!({})))
}

async fn session_info(State(state): State<Shared>, jar: CookieJar) -> ApiResult<Json<Value>> {
    let (_, s) = session(&state, &jar)?;
    Ok(Json(json!({ "account": s.client.info() })))
}

async fn rpc(State(state): State<Shared>, jar: CookieJar, Path(method): Path<String>, body: Option<Json<Value>>) -> ApiResult<Json<Value>> {
    let (_, s) = session(&state, &jar)?;
    let params = body.map(|Json(v)| v).unwrap_or(Value::Null);
    let value = lowmail_core::rpc::dispatch(&s.client, &method, params).await?;
    Ok(Json(value))
}

async fn events(State(state): State<Shared>, jar: CookieJar) -> ApiResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let (id, s) = session(&state, &jar)?;
    s.client.start_watching();
    let mail = BroadcastStream::new(s.client.subscribe())
        .filter_map(|e| e.ok())
        .map(|event| Event::default().json_data(event).unwrap_or_else(|_| Event::default().comment("bad event")));
    let shared = state.clone();
    let alive = IntervalStream::new(tokio::time::interval(Duration::from_secs(25))).map(move |_| {
        shared.sessions.touch(&id);
        Event::default().comment("ping")
    });
    let stream = mail.merge(alive).map(Ok::<_, Infallible>);
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn upload(State(state): State<Shared>, jar: CookieJar, mut multipart: Multipart) -> ApiResult<Json<Value>> {
    let (_, s) = session(&state, &jar)?;
    let mut uploads = Vec::new();
    while let Some(field) =
        multipart.next_field().await.map_err(|e| Error::invalid("The upload was interrupted.").with_detail(e.to_string()))?
    {
        let filename = field.file_name().unwrap_or("file").to_string();
        let content_type = field.content_type().unwrap_or("").to_string();
        let bytes = field.bytes().await.map_err(|e| {
            Error::new(ErrorKind::TooLarge, "The file is too large or the upload was interrupted.").with_detail(e.to_string())
        })?;
        uploads.push(s.client.add_upload(&filename, &content_type, bytes.to_vec())?);
    }
    Ok(Json(json!({ "uploads": uploads })))
}

/// `attachment; filename="..."; filename*=UTF-8''...` for any file name.
fn disposition(kind: &str, filename: &str) -> HeaderValue {
    let ascii: String = filename.chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' }).collect();
    let encoded = percent_encoding::utf8_percent_encode(filename, percent_encoding::NON_ALPHANUMERIC).to_string();
    HeaderValue::from_str(&format!("{kind}; filename=\"{ascii}\"; filename*=UTF-8''{encoded}"))
        .unwrap_or(HeaderValue::from_static("attachment"))
}

fn file_response(filename: &str, content_type: &str, bytes: Vec<u8>, inline: bool) -> Response {
    const PREVIEW: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp", "image/bmp", "application/pdf", "text/plain"];
    let ct = content_type.to_ascii_lowercase();
    let previewable = PREVIEW.contains(&ct.as_str()) || ct.starts_with("audio/") || ct.starts_with("video/");
    let inline = inline && previewable;
    let served_type = if !inline {
        "application/octet-stream".to_string()
    } else if ct == "text/plain" {
        "text/plain; charset=utf-8".to_string()
    } else {
        ct.clone()
    };
    let mut resp = Response::new(Body::from(bytes));
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_str(&served_type).unwrap_or(HeaderValue::from_static("application/octet-stream")));
    h.insert(header::CONTENT_DISPOSITION, disposition(if inline { "inline" } else { "attachment" }, filename));
    if ct != "application/pdf" {
        // Never let a downloaded file run as a page of this origin.
        h.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("sandbox; default-src 'none'; img-src data:; media-src 'self'; style-src 'unsafe-inline'"),
        );
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=300"));
    resp
}

#[derive(Deserialize)]
struct PartQuery {
    folder: String,
    uid: u32,
    part: String,
    #[serde(default)]
    inline: Option<u8>,
}

async fn download_attachment(State(state): State<Shared>, jar: CookieJar, Query(q): Query<PartQuery>) -> ApiResult<Response> {
    let (_, s) = session(&state, &jar)?;
    let (name, ct, bytes) = s.client.attachment(&q.folder, q.uid, &q.part).await?;
    Ok(file_response(&name, &ct, bytes, q.inline == Some(1)))
}

#[derive(Deserialize)]
struct MessageQuery {
    folder: String,
    uid: u32,
}

async fn download_message(State(state): State<Shared>, jar: CookieJar, Query(q): Query<MessageQuery>) -> ApiResult<Response> {
    let (_, s) = session(&state, &jar)?;
    let raw = s.client.raw(&q.folder, q.uid).await?;
    let name = MailClient::eml_name(&raw);
    Ok(file_response(&name, "message/rfc822", raw.as_ref().clone(), false))
}

async fn download_contacts(State(state): State<Shared>, jar: CookieJar) -> ApiResult<Response> {
    let (_, s) = session(&state, &jar)?;
    let contacts = s.client.store().contacts(s.client.owner(), "", false, 100_000)?;
    let vcf = lowmail_core::vcard::export(&contacts);
    let mut resp = file_response("contacts.vcf", "text/vcard", vcf.into_bytes(), false);
    resp.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/vcard; charset=utf-8"));
    Ok(resp)
}
