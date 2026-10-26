//! Guesses the servers of an address by trying the usual host names and
//! ports, for the desktop account setup. Everything it picks can be changed.

use std::time::Duration;

use tokio::net::TcpStream;

use crate::error::{Error, Result};
use crate::model::{AccountConfig, Endpoint, Security};

const PROBE_TIMEOUT: Duration = Duration::from_secs(4);

async fn open(host: &str, port: u16) -> bool {
    matches!(tokio::time::timeout(PROBE_TIMEOUT, TcpStream::connect((host, port))).await, Ok(Ok(_)))
}

/// The first candidate that accepts a connection, tried in parallel.
async fn first_open(candidates: Vec<(String, u16, Security)>) -> Option<(String, u16, Security)> {
    let probes = candidates.iter().map(|(h, p, _)| open(h, *p));
    let results = futures::future::join_all(probes).await;
    candidates.into_iter().zip(results).find(|(_, ok)| *ok).map(|(c, _)| c)
}

pub async fn discover(email: &str) -> Result<AccountConfig> {
    let email = email.trim().to_ascii_lowercase();
    let domain = email
        .rsplit_once('@')
        .map(|(_, d)| d.to_string())
        .filter(|d| d.contains('.'))
        .ok_or_else(|| Error::invalid("Enter a valid e-mail address."))?;
    let hosts = [format!("imap.{domain}"), format!("mail.{domain}"), domain.clone(), format!("smtp.{domain}")];
    let imap_candidates: Vec<(String, u16, Security)> =
        hosts[..3].iter().flat_map(|h| [(h.clone(), 993, Security::Tls), (h.clone(), 143, Security::StartTls)]).collect();
    let smtp_hosts = [hosts[3].clone(), hosts[1].clone(), hosts[2].clone()];
    let smtp_candidates: Vec<(String, u16, Security)> =
        smtp_hosts.iter().flat_map(|h| [(h.clone(), 465, Security::Tls), (h.clone(), 587, Security::StartTls)]).collect();
    let (imap, smtp) = tokio::join!(first_open(imap_candidates), first_open(smtp_candidates));
    let (imap_host, imap_port, imap_security) =
        imap.ok_or_else(|| Error::not_found(format!("No mail server found for {domain}. Fill in the server settings.")))?;
    let (smtp_host, smtp_port, smtp_security) = smtp.unwrap_or((imap_host.clone(), 587, Security::StartTls));
    let sieve = open(&imap_host, 4190).await.then(|| Endpoint {
        host: imap_host.clone(),
        port: 4190,
        security: Security::StartTls,
        accept_invalid_certs: false,
    });
    Ok(AccountConfig {
        id: String::new(),
        label: String::new(),
        name: String::new(),
        email: email.clone(),
        username: email,
        imap: Endpoint { host: imap_host, port: imap_port, security: imap_security, accept_invalid_certs: false },
        smtp: Endpoint { host: smtp_host, port: smtp_port, security: smtp_security, accept_invalid_certs: false },
        sieve,
        smtp_auth: true,
    })
}
