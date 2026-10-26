//! The web server over HTTP against the in-memory mail server.

use std::net::SocketAddr;
use std::time::Duration;

use lowmail_core::model::Security;
use lowmail_server::config::{Config, ServerTemplate};
use reqwest::StatusCode;
use serde_json::{Value, json};

struct Env {
    base: String,
    _mail: lowmail_testserver::Server,
}

async fn start() -> Env {
    let mail = lowmail_testserver::start(lowmail_testserver::Config { imap_port: 0, smtp_port: 0, sieve_port: 0, ..Default::default() })
        .await
        .unwrap();
    let tpl = |addr: SocketAddr| ServerTemplate { host: addr.ip().to_string(), port: addr.port(), security: Security::None };
    let dir = std::env::temp_dir().join(format!("lowmail-http-{}", rand::random::<u64>()));
    let config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        data_dir: dir,
        imap: tpl(mail.imap_addr()),
        smtp: tpl(mail.smtp_addr()),
        sieve: Some(tpl(mail.sieve_addr())),
        accept_invalid_certs: false,
        default_domain: Some("lowmail.test".into()),
        allowed_domains: vec!["lowmail.test".into()],
        login_local_part: false,
        session_idle: Duration::from_secs(600),
        secure_cookies: Some(false),
        product_name: "LowMail".into(),
        trust_proxy: false,
        pool_size: 2,
        smtp_auth: true,
    };
    let (router, _) = lowmail_server::build(config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();
    });
    Env { base: format!("http://{addr}"), _mail: mail }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().cookie_store(true).build().unwrap()
}

async fn post(c: &reqwest::Client, url: &str, body: Value) -> (StatusCode, Value) {
    let res = c.post(url).header("x-lowmail", "1").json(&body).send().await.unwrap();
    let status = res.status();
    (status, res.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn sign_in_and_csrf() {
    let env = start().await;
    let c = client();
    let cfg: Value = c.get(format!("{}/api/config", env.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(cfg["domain"], "lowmail.test");

    // No CSRF header: refused before anything else.
    let res = c.post(format!("{}/api/login", env.base)).json(&json!({"email": "alice", "password": "alice"})).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // Other domains are refused.
    let (status, body) = post(&c, &format!("{}/api/login", env.base), json!({"email": "x@evil.example", "password": "x"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, body) = post(&c, &format!("{}/api/login", env.base), json!({"email": "alice", "password": "wrong"})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["kind"], "auth");

    // Not signed in yet.
    let (status, body) = post(&c, &format!("{}/api/rpc/folders.list", env.base), json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["kind"], "sessionExpired");

    let res = c
        .post(format!("{}/api/login", env.base))
        .header("x-lowmail", "1")
        .json(&json!({"email": "alice", "password": "alice"}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cookie = res.headers().get("set-cookie").unwrap().to_str().unwrap().to_string();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"), "{cookie}");
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["account"]["email"], "alice@lowmail.test");

    let (status, folders) = post(&c, &format!("{}/api/rpc/folders.list", env.base), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(folders.as_array().unwrap().iter().any(|f| f["role"] == "sent"));
    let (status, _) = post(&c, &format!("{}/api/rpc/no.such", env.base), json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = post(&c, &format!("{}/api/logout", env.base), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post(&c, &format!("{}/api/rpc/folders.list", env.base), json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn password_guessing_is_slowed_down() {
    let env = start().await;
    let c = client();
    let mut last = StatusCode::OK;
    for _ in 0..10 {
        let (status, _) = post(&c, &format!("{}/api/login", env.base), json!({"email": "bob", "password": "nope"})).await;
        last = status;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
    // Even the right password waits.
    let (status, _) = post(&c, &format!("{}/api/login", env.base), json!({"email": "bob", "password": "bob"})).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn upload_send_and_download() {
    let env = start().await;
    let c = client();
    let (status, _) = post(&c, &format!("{}/api/login", env.base), json!({"email": "alice@lowmail.test", "password": "alice"})).await;
    assert_eq!(status, StatusCode::OK);

    let form = reqwest::multipart::Form::new()
        .part("file", reqwest::multipart::Part::bytes(b"conteudo".to_vec()).file_name("nota fiscal.txt").mime_str("text/plain").unwrap());
    let res = c.post(format!("{}/api/upload", env.base)).header("x-lowmail", "1").multipart(form).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let up: Value = res.json().await.unwrap();
    let up = &up["uploads"][0];
    assert_eq!(up["filename"], "nota fiscal.txt");

    let (_, draft) = post(&c, &format!("{}/api/rpc/compose.prepare", env.base), json!({"mode": "new"})).await;
    let mut draft = draft;
    draft["to"] = json!([{"email": "bob@lowmail.test"}]);
    draft["subject"] = json!("Pela web");
    draft["html"] = json!("<p>Oi</p>");
    draft["attachments"] =
        json!([{"source": "upload", "id": up["id"], "filename": up["filename"], "contentType": up["contentType"], "size": up["size"]}]);
    let (status, sent) = post(&c, &format!("{}/api/rpc/compose.send", env.base), json!({"draft": draft})).await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    assert_eq!(sent["saved"], true);

    // Attachment of the sent copy, as a download and as a preview.
    let (_, page) = post(&c, &format!("{}/api/rpc/messages.list", env.base), json!({"folder": "Sent", "limit": 1})).await;
    let uid = page["items"][0]["uid"].as_u64().unwrap();
    let (_, msg) = post(&c, &format!("{}/api/rpc/messages.get", env.base), json!({"folder": "Sent", "uid": uid})).await;
    let part = msg["attachments"][0]["part"].as_str().unwrap().to_string();
    let res = c.get(format!("{}/api/download/attachment?folder=Sent&uid={uid}&part={part}", env.base)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let h = res.headers().clone();
    assert!(h["content-disposition"].to_str().unwrap().starts_with("attachment;"));
    assert!(h["content-disposition"].to_str().unwrap().contains("filename*=UTF-8''nota%20fiscal%2Etxt"));
    assert_eq!(h["content-type"], "application/octet-stream");
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert!(h["content-security-policy"].to_str().unwrap().starts_with("sandbox"));
    assert_eq!(res.bytes().await.unwrap().as_ref(), b"conteudo");
    let res = c.get(format!("{}/api/download/attachment?folder=Sent&uid={uid}&part={part}&inline=1", env.base)).send().await.unwrap();
    assert_eq!(res.headers()["content-type"], "text/plain; charset=utf-8");

    let res = c.get(format!("{}/api/download/message?folder=Sent&uid={uid}", env.base)).send().await.unwrap();
    assert!(res.headers()["content-disposition"].to_str().unwrap().contains("Pela web.eml"));

    // Another browser without the cookie cannot download.
    let res = client().get(format!("{}/api/download/attachment?folder=Sent&uid={uid}&part={part}", env.base)).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn security_headers() {
    let env = start().await;
    let res = client().get(format!("{}/healthz", env.base)).send().await.unwrap();
    let h = res.headers();
    assert_eq!(h["x-frame-options"], "DENY");
    assert_eq!(h["referrer-policy"], "no-referrer");
    assert_eq!(h["x-content-type-options"], "nosniff");
    if lowmail_server::assets::available() {
        let res = client().get(format!("{}/", env.base)).send().await.unwrap();
        let csp = res.headers()["content-security-policy"].to_str().unwrap().to_string();
        assert!(csp.contains("frame-ancestors 'none'") && csp.contains("object-src 'none'"), "{csp}");
        assert!(!csp.contains("'unsafe-inline'; img") || csp.contains("script-src 'self' 'sha256-"), "{csp}");
    }
}
