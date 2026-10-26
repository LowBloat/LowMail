//! LowMail webmail server as a library, so tests can run it in-process.

pub mod app;
pub mod assets;
pub mod check;
pub mod config;
pub mod sessions;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use lowmail_core::store::Store;

use crate::app::AppState;
use crate::config::Config;
use crate::sessions::{LoginLimiter, Sessions};

/// The router and its state for `config`, without listening.
pub fn build(config: Config) -> anyhow::Result<(axum::Router, Arc<AppState>)> {
    let store = Arc::new(Store::open(&config.data_dir.join("lowmail.db")).context("opening the database")?);
    let state = Arc::new(AppState { config, store, sessions: Sessions::default(), limiter: LoginLimiter::default() });
    Ok((app::router(state.clone()), state))
}

fn state_config_hosts(state: &AppState) -> Vec<String> {
    let c = &state.config;
    let mut hosts = vec![c.imap.host.clone(), c.smtp.host.clone()];
    if let Some(s) = &c.sieve {
        hosts.push(s.host.clone());
    }
    hosts
}

fn config_uses_domain_template(hosts: &[String]) -> bool {
    hosts.iter().any(|h| h.contains("{domain}"))
}

pub async fn serve(config: Config) -> anyhow::Result<()> {
    let listen = config.listen;
    let idle = config.session_idle;
    tracing::info!(
        imap = %format!("{}:{}", config.imap.host, config.imap.port),
        smtp = %format!("{}:{}", config.smtp.host, config.smtp.port),
        "LowMail {} listening on http://{listen}",
        env!("CARGO_PKG_VERSION")
    );
    if !assets::available() {
        tracing::warn!("the web interface is missing: run `bun run build` before building the server");
    }
    let (router, state) = build(config)?;
    if config_uses_domain_template(&state_config_hosts(&state)) && state.config.allowed_domains.is_empty() {
        tracing::warn!(
            "server host names use {{domain}} but LOWMAIL_ALLOWED_DOMAINS is empty: anyone can make this server connect to imap.<their domain>. Set LOWMAIL_ALLOWED_DOMAINS."
        );
    }
    let reaper = state.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            reaper.sessions.reap(idle).await;
        }
    });
    let listener = tokio::net::TcpListener::bind(listen).await.with_context(|| format!("listening on {listen}"))?;
    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutting down");
        })
        .await?;
    Ok(())
}
