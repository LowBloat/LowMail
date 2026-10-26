//! `lowmail`: the LowMail webmail server.
//!
//!   lowmail serve        run the server (default)
//!   lowmail healthcheck  exit 0 when the local server answers
//!   lowmail demo         run with a built-in test mail server (needs the `demo` feature)

use std::time::Duration;

use anyhow::Context;
use lowmail_server::config::Config;
use lowmail_server::serve;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(EnvFilter::try_from_env("LOWMAIL_LOG").unwrap_or_else(|_| EnvFilter::new("info"))).init();
    let command = std::env::args().nth(1).unwrap_or_else(|| "serve".into());
    match command.as_str() {
        "serve" => serve(Config::from_env()?).await,
        "healthcheck" => healthcheck().await,
        "demo" => demo().await,
        "--version" | "-V" | "version" => {
            println!("lowmail {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => {
            eprintln!("usage: lowmail [serve|healthcheck|demo|version]\nSettings come from LOWMAIL_* environment variables (see README).");
            std::process::exit(2);
        }
    }
}

async fn healthcheck() -> anyhow::Result<()> {
    let listen = std::env::var("LOWMAIL_LISTEN").unwrap_or_else(|_| "127.0.0.1:8480".into());
    let port = listen.rsplit(':').next().unwrap_or("8480");
    let mut stream = tokio::time::timeout(Duration::from_secs(3), tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")))
        .await
        .context("timeout")??;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream.write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n").await?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    if String::from_utf8_lossy(&buf).starts_with("HTTP/1.0 200") || String::from_utf8_lossy(&buf).starts_with("HTTP/1.1 200") {
        Ok(())
    } else {
        anyhow::bail!("unhealthy")
    }
}

#[cfg(feature = "demo")]
async fn demo() -> anyhow::Result<()> {
    let mail = lowmail_testserver::start(lowmail_testserver::Config::default()).await?;
    let set = |k: &str, v: String| {
        if std::env::var(format!("LOWMAIL_{k}")).is_err() {
            // SAFETY: single-threaded at this point of start-up for these variables.
            unsafe { std::env::set_var(format!("LOWMAIL_{k}"), v) };
        }
    };
    set("IMAP_HOST", mail.imap_addr().ip().to_string());
    set("IMAP_PORT", mail.imap_addr().port().to_string());
    set("IMAP_SECURITY", "none".into());
    set("SMTP_HOST", mail.smtp_addr().ip().to_string());
    set("SMTP_PORT", mail.smtp_addr().port().to_string());
    set("SMTP_SECURITY", "none".into());
    set("SIEVE_HOST", mail.sieve_addr().ip().to_string());
    set("SIEVE_PORT", mail.sieve_addr().port().to_string());
    set("SIEVE_SECURITY", "none".into());
    set("DOMAIN", "lowmail.test".into());
    set("LISTEN", "127.0.0.1:8480".into());
    tracing::info!("demo mail server running; sign in as alice@lowmail.test / alice or bob@lowmail.test / bob");
    let result = serve(Config::from_env()?).await;
    mail.shutdown().await;
    result
}

#[cfg(not(feature = "demo"))]
async fn demo() -> anyhow::Result<()> {
    anyhow::bail!("this build has no demo mail server; run `cargo run -p lowmail-server --features demo -- demo`")
}
