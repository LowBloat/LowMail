//! Server settings from `LOWMAIL_*` environment variables.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, bail};
use lowmail_core::model::{AccountConfig, Endpoint, Security};

#[derive(Debug, Clone)]
pub struct ServerTemplate {
    /// Host name; `{domain}` is replaced by the domain of the address signing in.
    pub host: String,
    pub port: u16,
    pub security: Security,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    pub data_dir: PathBuf,
    pub imap: ServerTemplate,
    pub smtp: ServerTemplate,
    pub sieve: Option<ServerTemplate>,
    pub accept_invalid_certs: bool,
    /// Appended to user names typed without `@domain`.
    pub default_domain: Option<String>,
    /// Only these domains may sign in (empty: any).
    pub allowed_domains: Vec<String>,
    /// Sign-in name sent to the servers: the full address, or the part before `@`.
    pub login_local_part: bool,
    pub session_idle: Duration,
    /// `Secure` cookies: always, never, or when the request came over HTTPS.
    pub secure_cookies: Option<bool>,
    pub product_name: String,
    pub trust_proxy: bool,
    /// IMAP connections per signed-in user.
    pub pool_size: usize,
    /// Sign in to the SMTP server (off for relays that trust the network).
    pub smtp_auth: bool,
}

fn env(name: &str) -> Option<String> {
    std::env::var(format!("LOWMAIL_{name}")).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn security(name: &str, default: Security) -> anyhow::Result<Security> {
    Ok(match env(name).as_deref().map(str::to_ascii_lowercase).as_deref() {
        None => default,
        Some("tls" | "ssl" | "implicit") => Security::Tls,
        Some("starttls") => Security::StartTls,
        Some("none" | "plain") => Security::None,
        Some(other) => bail!("LOWMAIL_{name} must be tls, starttls or none, not {other}"),
    })
}

fn port(name: &str, default: u16) -> anyhow::Result<u16> {
    match env(name) {
        Some(v) => v.parse().with_context(|| format!("LOWMAIL_{name} is not a port number")),
        None => Ok(default),
    }
}

fn flag(name: &str) -> Option<bool> {
    env(name).map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

impl Config {
    pub fn from_env() -> anyhow::Result<Config> {
        let imap_host = env("IMAP_HOST").unwrap_or_else(|| "localhost".into());
        let imap_security = security("IMAP_SECURITY", Security::Tls)?;
        let imap = ServerTemplate {
            port: port(
                "IMAP_PORT",
                match imap_security {
                    Security::Tls => 993,
                    _ => 143,
                },
            )?,
            host: imap_host.clone(),
            security: imap_security,
        };
        let smtp_security = security("SMTP_SECURITY", Security::StartTls)?;
        let smtp = ServerTemplate {
            host: env("SMTP_HOST").unwrap_or_else(|| imap_host.clone()),
            port: port(
                "SMTP_PORT",
                match smtp_security {
                    Security::Tls => 465,
                    Security::StartTls => 587,
                    Security::None => 25,
                },
            )?,
            security: smtp_security,
        };
        let sieve = if flag("SIEVE").unwrap_or(true) {
            Some(ServerTemplate {
                host: env("SIEVE_HOST").unwrap_or_else(|| imap_host.clone()),
                port: port("SIEVE_PORT", 4190)?,
                security: security("SIEVE_SECURITY", if imap.security == Security::None { Security::None } else { Security::StartTls })?,
            })
        } else {
            None
        };
        let minutes: u64 = env("SESSION_MINUTES").map(|v| v.parse()).transpose().context("LOWMAIL_SESSION_MINUTES")?.unwrap_or(240);
        Ok(Config {
            listen: env("LISTEN").unwrap_or_else(|| "0.0.0.0:8480".into()).parse().context("LOWMAIL_LISTEN")?,
            data_dir: env("DATA_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("data")),
            imap,
            smtp,
            sieve,
            accept_invalid_certs: flag("ACCEPT_INVALID_CERTS").unwrap_or(false),
            default_domain: env("DOMAIN").map(|d| d.trim_start_matches('@').to_ascii_lowercase()),
            allowed_domains: env("ALLOWED_DOMAINS")
                .map(|v| v.split(',').map(|d| d.trim().trim_start_matches('@').to_ascii_lowercase()).filter(|d| !d.is_empty()).collect())
                .unwrap_or_default(),
            login_local_part: flag("LOGIN_LOCAL_PART").unwrap_or(false),
            session_idle: Duration::from_secs(minutes.max(5) * 60),
            secure_cookies: flag("SECURE_COOKIES"),
            product_name: env("PRODUCT_NAME").unwrap_or_else(|| "LowMail".into()),
            trust_proxy: flag("TRUST_PROXY").unwrap_or(false),
            pool_size: env("IMAP_CONNECTIONS").and_then(|v| v.parse().ok()).unwrap_or(3usize).clamp(1, 10),
            smtp_auth: flag("SMTP_AUTH").unwrap_or(true),
        })
    }

    /// The full address for a typed user name, or why it cannot sign in.
    pub fn normalize_login(&self, typed: &str) -> Result<String, &'static str> {
        let typed = typed.trim().to_ascii_lowercase();
        if typed.is_empty() {
            return Err("Enter your e-mail address.");
        }
        let email = if typed.contains('@') {
            typed
        } else {
            match &self.default_domain {
                Some(d) => format!("{typed}@{d}"),
                None => return Err("Enter your full e-mail address."),
            }
        };
        let domain = email.rsplit('@').next().unwrap_or("");
        if domain.is_empty() || email.starts_with('@') {
            return Err("Enter your full e-mail address.");
        }
        if !self.allowed_domains.is_empty() && !self.allowed_domains.iter().any(|d| d == domain) {
            return Err("This server does not handle that domain.");
        }
        Ok(email)
    }

    pub fn account_for(&self, email: &str) -> AccountConfig {
        let domain = email.rsplit('@').next().unwrap_or("");
        let local = email.split('@').next().unwrap_or(email);
        let endpoint = |t: &ServerTemplate| Endpoint {
            host: t.host.replace("{domain}", domain),
            port: t.port,
            security: t.security,
            accept_invalid_certs: self.accept_invalid_certs,
        };
        AccountConfig {
            id: email.to_string(),
            label: String::new(),
            name: String::new(),
            email: email.to_string(),
            username: if self.login_local_part { local.to_string() } else { email.to_string() },
            imap: endpoint(&self.imap),
            smtp: endpoint(&self.smtp),
            sieve: self.sieve.as_ref().map(endpoint),
            smtp_auth: self.smtp_auth,
        }
    }
}
