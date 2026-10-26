//! The web interface (Vite build in `dist/`), embedded into the binary in
//! release builds and read from disk in debug builds.

use std::sync::OnceLock;

use axum::body::Body;
use axum::http::{HeaderValue, Response, StatusCode, header};
use base64::Engine;
use sha2::{Digest, Sha256};

#[derive(rust_embed::Embed)]
#[folder = "../../dist"]
#[allow_missing = true]
#[exclude = "*.map"]
struct Ui;

/// Content-Security-Policy for the app pages. Inline scripts of index.html
/// (the theme bootstrap) are allowed by hash; message bodies render in
/// sandboxed frames with their own, stricter policy.
pub fn csp() -> &'static str {
    static CSP: OnceLock<String> = OnceLock::new();
    CSP.get_or_init(|| {
        let mut hashes = String::new();
        if let Some(index) = Ui::get("index.html") {
            let html = String::from_utf8_lossy(&index.data);
            for script in inline_scripts(&html) {
                let digest = Sha256::digest(script.as_bytes());
                hashes.push_str(&format!(" 'sha256-{}'", base64::engine::general_purpose::STANDARD.encode(digest)));
            }
        }
        format!(
            "default-src 'self'; script-src 'self'{hashes}; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https: http:; \
             font-src 'self' data:; connect-src 'self'; frame-src 'self' blob: data:; media-src 'self' blob: data:; object-src 'none'; \
             base-uri 'self'; form-action 'self'; frame-ancestors 'none'"
        )
    })
}

fn inline_scripts(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("<script") {
        let Some(open_end) = rest[start..].find('>').map(|e| start + e) else { break };
        let tag = &rest[start..open_end];
        let Some(close) = rest[open_end..].find("</script>").map(|e| open_end + e) else { break };
        if !tag.contains("src=") {
            out.push(rest[open_end + 1..close].to_string());
        }
        rest = &rest[close + 9..];
    }
    out
}

pub fn available() -> bool {
    Ui::get("index.html").is_some()
}

pub fn serve(path: &str) -> Response<Body> {
    let path = path.trim_start_matches('/');
    let (file, name) = match Ui::get(path).filter(|_| !path.is_empty()) {
        Some(f) => (Some(f), path),
        // Unknown paths get the app, which routes on the client.
        None if !path.starts_with("assets/") => (Ui::get("index.html"), "index.html"),
        None => (None, path),
    };
    let Some(file) = file else {
        let mut resp = Response::new(Body::from(if available() {
            "Not found"
        } else {
            "The web interface was not built. Run `bun run build` in the project folder and restart."
        }));
        *resp.status_mut() = StatusCode::NOT_FOUND;
        return resp;
    };
    let mime = mime_guess::from_path(name).first_or_octet_stream();
    let mut resp = Response::new(Body::from(file.data.into_owned()));
    let headers = resp.headers_mut();
    headers
        .insert(header::CONTENT_TYPE, HeaderValue::from_str(mime.as_ref()).unwrap_or(HeaderValue::from_static("application/octet-stream")));
    let cache = if name.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    if name.ends_with(".html") {
        headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_str(csp()).expect("valid CSP"));
    }
    resp
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_inline_scripts() {
        let html = r#"<head><script>a()</script><script type="module" src="/x.js"></script><script>b</script></head>"#;
        assert_eq!(super::inline_scripts(html), vec!["a()", "b"]);
    }
}
