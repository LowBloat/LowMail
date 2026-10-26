//! Guesses the servers of an address for the desktop account setup: the
//! usual host names, then the domain's MX hosts (hosted mail such as MXRoute
//! or cPanel serves IMAP there), on the usual ports. A server wins only when
//! its certificate is valid for its name, so a web host answering on
//! `imap.<domain>` with someone else's certificate is skipped. Everything it
//! picks can be changed.

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::error::{Error, Result};
use crate::model::{AccountConfig, Endpoint, Security};
use crate::net;

const PROBE_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, PartialEq)]
enum Protocol {
    Imap,
    Smtp,
}

#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Probe {
    Closed,
    /// Answers, but the secure connection failed (often a certificate for
    /// another name).
    Open,
    Verified,
}

async fn open(host: &str, port: u16) -> bool {
    matches!(tokio::time::timeout(PROBE_TIMEOUT, TcpStream::connect((host, port))).await, Ok(Ok(_)))
}

/// Connects and completes TLS, directly or after STARTTLS, with the normal
/// certificate checks.
async fn probe(host: &str, port: u16, security: Security, protocol: Protocol) -> Probe {
    let attempt = async {
        let Ok(stream) = net::connect_tcp(host, port).await else { return Probe::Closed };
        let stream = match security {
            Security::Tls => stream,
            Security::StartTls => match starttls(stream, protocol).await {
                Ok(s) => s,
                Err(_) => return Probe::Open,
            },
            Security::None => return Probe::Open,
        };
        match net::upgrade(stream, host, false).await {
            Ok(_) => Probe::Verified,
            Err(_) => Probe::Open,
        }
    };
    tokio::time::timeout(PROBE_TIMEOUT, attempt).await.unwrap_or(Probe::Closed)
}

/// Asks for STARTTLS and returns the socket ready for the TLS handshake.
async fn starttls(stream: TcpStream, protocol: Protocol) -> std::io::Result<TcpStream> {
    let refused = || std::io::Error::other("STARTTLS refused");
    let mut r = BufReader::new(stream);
    match protocol {
        Protocol::Imap => {
            let mut line = String::new();
            r.read_line(&mut line).await?;
            if !line.starts_with("* OK") {
                return Err(refused());
            }
            r.get_mut().write_all(b"a STARTTLS\r\n").await?;
            loop {
                line.clear();
                if r.read_line(&mut line).await? == 0 {
                    return Err(refused());
                }
                if line.starts_with("a ") {
                    break;
                }
            }
            if !line.to_ascii_uppercase().starts_with("A OK") {
                return Err(refused());
            }
        }
        Protocol::Smtp => {
            if smtp_reply(&mut r).await? != 220 {
                return Err(refused());
            }
            r.get_mut().write_all(b"EHLO lowmail\r\n").await?;
            if smtp_reply(&mut r).await? != 250 {
                return Err(refused());
            }
            r.get_mut().write_all(b"STARTTLS\r\n").await?;
            if smtp_reply(&mut r).await? != 220 {
                return Err(refused());
            }
        }
    }
    Ok(r.into_inner())
}

/// The code of a (possibly multi-line) SMTP reply.
async fn smtp_reply(r: &mut BufReader<TcpStream>) -> std::io::Result<u16> {
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line).await? == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        if line.as_bytes().get(3) != Some(&b'-') {
            return Ok(line.get(..3).and_then(|c| c.parse().ok()).unwrap_or(0));
        }
    }
}

/// The first candidate with a valid certificate, else the first that
/// answers (a server of your own with a self-signed certificate). All are
/// tried in parallel; the order of `candidates` is the preference.
async fn best(candidates: Vec<(String, u16, Security)>, protocol: Protocol) -> Option<(String, u16, Security)> {
    let probes = candidates.iter().map(|(h, p, s)| probe(h, *p, *s, protocol));
    let results = futures::future::join_all(probes).await;
    let pick = |level: Probe| candidates.iter().zip(&results).find(|(_, r)| **r == level).map(|(c, _)| c.clone());
    pick(Probe::Verified).or_else(|| pick(Probe::Open))
}

/// Mail exchangers of the domain, most preferred first.
pub async fn mx_hosts(domain: &str) -> Vec<String> {
    let lookup = async {
        let resolver = hickory_resolver::TokioResolver::builder_tokio().ok()?.build().ok()?;
        let answer = resolver.mx_lookup(format!("{domain}.")).await.ok()?;
        let mut mx: Vec<(u16, String)> = answer
            .answers()
            .iter()
            .filter_map(|r| match &r.data {
                hickory_resolver::proto::rr::RData::MX(mx) => {
                    Some((mx.preference, mx.exchange.to_utf8().trim_end_matches('.').to_ascii_lowercase()))
                }
                _ => None,
            })
            .filter(|(_, h)| !h.is_empty())
            .collect();
        mx.sort();
        Some(mx.into_iter().map(|(_, h)| h).collect::<Vec<_>>())
    };
    tokio::time::timeout(PROBE_TIMEOUT, lookup).await.ok().flatten().unwrap_or_default()
}

fn ports(hosts: &[String], first: (u16, Security), second: (u16, Security)) -> Vec<(String, u16, Security)> {
    hosts.iter().flat_map(|h| [(h.clone(), first.0, first.1), (h.clone(), second.0, second.1)]).collect()
}

pub async fn discover(email: &str) -> Result<AccountConfig> {
    let email = email.trim().to_ascii_lowercase();
    let domain = email
        .rsplit_once('@')
        .map(|(_, d)| d.to_string())
        .filter(|d| d.contains('.'))
        .ok_or_else(|| Error::invalid("Enter a valid e-mail address."))?;
    let mx = mx_hosts(&domain).await;
    let with_mx = |mut hosts: Vec<String>| {
        for h in &mx {
            if !hosts.contains(h) {
                hosts.push(h.clone());
            }
        }
        hosts
    };
    let imap_hosts = with_mx(vec![format!("imap.{domain}"), format!("mail.{domain}"), domain.clone()]);
    let smtp_hosts = with_mx(vec![format!("smtp.{domain}"), format!("mail.{domain}"), domain.clone()]);
    let (imap, smtp) = tokio::join!(
        best(ports(&imap_hosts, (993, Security::Tls), (143, Security::StartTls)), Protocol::Imap),
        best(ports(&smtp_hosts, (465, Security::Tls), (587, Security::StartTls)), Protocol::Smtp),
    );
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
