//! SMTP submission server (RFC 5321) with AUTH PLAIN/LOGIN and local delivery.

use std::io;
use std::sync::Arc;

use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

use crate::OutboundMail;
use crate::io::{LineReader, trim_eol};
use crate::message::MessageData;
use crate::sasl;
use crate::store::Store;

const HOSTNAME: &str = "lowmail.test";
const MAX_SIZE: usize = 52_428_800;
const MAX_LINE: usize = 64 * 1024;
const MAX_RECIPIENTS: usize = 100;

struct Session {
    reader: LineReader<OwnedReadHalf>,
    writer: BufWriter<OwnedWriteHalf>,
    store: Arc<Store>,
    helo: Option<String>,
    user: Option<String>,
    from: Option<String>,
    rcpts: Vec<String>,
    queued: u64,
}

/// Serves one SMTP connection until QUIT or disconnect.
pub(crate) async fn handle(stream: TcpStream, store: Arc<Store>) {
    let (read, write) = stream.into_split();
    let mut session = Session {
        reader: LineReader::new(read),
        writer: BufWriter::new(write),
        store,
        helo: None,
        user: None,
        from: None,
        rcpts: Vec::new(),
        queued: 0,
    };
    if let Err(err) = session.run().await {
        tracing::debug!(%err, "smtp connection ended");
    }
}

/// Extracts the address from `FROM:<addr> PARAMS` / `TO:<addr>`.
fn parse_path(arg: &str, prefix: &str) -> Option<String> {
    let rest = arg.get(..prefix.len())?;
    if !rest.eq_ignore_ascii_case(prefix) {
        return None;
    }
    let rest = arg[prefix.len()..].trim_start();
    if let Some(inner) = rest.strip_prefix('<') {
        let end = inner.find('>')?;
        Some(inner[..end].trim().to_owned())
    } else {
        let addr = rest.split_whitespace().next()?;
        Some(addr.to_owned())
    }
}

impl Session {
    async fn reply(&mut self, text: &str) -> io::Result<()> {
        self.writer.write_all(text.as_bytes()).await?;
        self.writer.write_all(b"\r\n").await?;
        self.writer.flush().await
    }

    async fn read_line(&mut self) -> io::Result<String> {
        match self.reader.read_line(MAX_LINE).await? {
            Some(line) => Ok(String::from_utf8_lossy(trim_eol(&line)).into_owned()),
            None => Err(io::ErrorKind::UnexpectedEof.into()),
        }
    }

    fn reset(&mut self) {
        self.from = None;
        self.rcpts.clear();
    }

    async fn run(&mut self) -> io::Result<()> {
        self.reply(&format!("220 {HOSTNAME} ESMTP LowMail Test")).await?;
        loop {
            let line = match self.reader.read_line(MAX_LINE).await {
                Ok(Some(line)) => String::from_utf8_lossy(trim_eol(&line)).into_owned(),
                Ok(None) => return Ok(()),
                Err(err) if err.kind() == io::ErrorKind::InvalidData => {
                    self.reply("500 5.5.2 Line too long").await?;
                    return Ok(());
                }
                Err(err) => return Err(err),
            };
            let (verb, arg) = match line.split_once(' ') {
                Some((v, a)) => (v.to_ascii_uppercase(), a.trim().to_owned()),
                None => (line.trim().to_ascii_uppercase(), String::new()),
            };
            tracing::debug!(%verb, "smtp command");
            match verb.as_str() {
                "EHLO" => {
                    self.helo = Some(arg.clone());
                    self.reset();
                    let reply = format!(
                        "250-{HOSTNAME} Hello {arg}\r\n250-AUTH PLAIN LOGIN\r\n250-SIZE {MAX_SIZE}\r\n250-8BITMIME\r\n250-SMTPUTF8\r\n250 ENHANCEDSTATUSCODES"
                    );
                    self.reply(&reply).await?;
                }
                "HELO" => {
                    self.helo = Some(arg.clone());
                    self.reset();
                    self.reply(&format!("250 {HOSTNAME}")).await?;
                }
                "AUTH" => self.auth(&arg).await?,
                "MAIL" => self.mail(&arg).await?,
                "RCPT" => self.rcpt(&arg).await?,
                "DATA" => self.data().await?,
                "RSET" => {
                    self.reset();
                    self.reply("250 2.0.0 OK").await?;
                }
                "NOOP" => self.reply("250 2.0.0 OK").await?,
                "VRFY" => self.reply("252 2.5.0 Cannot VRFY user").await?,
                "HELP" => self.reply("214 2.0.0 See RFC 5321").await?,
                "QUIT" => {
                    self.reply(&format!("221 2.0.0 {HOSTNAME} closing connection")).await?;
                    return Ok(());
                }
                "STARTTLS" | "BDAT" | "EXPN" | "ETRN" => self.reply("502 5.5.1 Command not implemented").await?,
                _ => self.reply("500 5.5.2 Command not recognized").await?,
            }
        }
    }

    async fn auth(&mut self, arg: &str) -> io::Result<()> {
        if self.user.is_some() {
            return self.reply("503 5.5.1 Already authenticated").await;
        }
        let mut parts = arg.split_whitespace();
        let mechanism = parts.next().unwrap_or_default().to_ascii_uppercase();
        let initial = parts.next().map(str::to_owned);
        let credentials = match mechanism.as_str() {
            "PLAIN" => {
                let response = match initial {
                    Some(ir) => ir,
                    None => {
                        self.reply("334 ").await?;
                        self.read_line().await?
                    }
                };
                if response == "*" {
                    return self.reply("501 5.7.0 Authentication cancelled").await;
                }
                let response = if response == "=" { String::new() } else { response };
                sasl::decode_plain(response.as_bytes())
            }
            "LOGIN" => {
                let user = match initial {
                    Some(ir) => ir,
                    None => {
                        self.reply("334 VXNlcm5hbWU6").await?;
                        self.read_line().await?
                    }
                };
                if user == "*" {
                    return self.reply("501 5.7.0 Authentication cancelled").await;
                }
                self.reply("334 UGFzc3dvcmQ6").await?;
                let pass = self.read_line().await?;
                if pass == "*" {
                    return self.reply("501 5.7.0 Authentication cancelled").await;
                }
                let decode = |s: &str| sasl::decode_base64(s.as_bytes()).and_then(|b| String::from_utf8(b).ok());
                decode(&user).zip(decode(&pass))
            }
            _ => {
                return self.reply("504 5.5.4 Unrecognized authentication type").await;
            }
        };
        match credentials.and_then(|(u, p)| self.store.authenticate(&u, &p)) {
            Some(login) => {
                tracing::debug!(%login, "smtp auth");
                self.user = Some(login);
                self.reply("235 2.7.0 Authentication successful").await
            }
            None => self.reply("535 5.7.8 Authentication credentials invalid").await,
        }
    }

    async fn mail(&mut self, arg: &str) -> io::Result<()> {
        if self.user.is_none() {
            return self.reply("530 5.7.0 Authentication required").await;
        }
        if self.from.is_some() {
            return self.reply("503 5.5.1 Nested MAIL command").await;
        }
        let Some(from) = parse_path(arg, "FROM:") else {
            return self.reply("501 5.5.4 Syntax: MAIL FROM:<address>").await;
        };
        let size = arg
            .split_whitespace()
            .filter_map(|p| p.split_once('='))
            .find(|(k, _)| k.eq_ignore_ascii_case("SIZE"))
            .and_then(|(_, v)| v.parse::<usize>().ok());
        if size.is_some_and(|s| s > MAX_SIZE) {
            return self.reply("552 5.3.4 Message size exceeds fixed maximum message size").await;
        }
        self.from = Some(from);
        self.rcpts.clear();
        self.reply("250 2.1.0 OK").await
    }

    async fn rcpt(&mut self, arg: &str) -> io::Result<()> {
        if self.from.is_none() {
            return self.reply("503 5.5.1 Need MAIL command").await;
        }
        let Some(rcpt) = parse_path(arg, "TO:").filter(|r| !r.is_empty()) else {
            return self.reply("501 5.5.4 Syntax: RCPT TO:<address>").await;
        };
        if self.rcpts.len() >= MAX_RECIPIENTS {
            return self.reply("452 4.5.3 Too many recipients").await;
        }
        self.rcpts.push(rcpt);
        self.reply("250 2.1.5 OK").await
    }

    async fn data(&mut self) -> io::Result<()> {
        if self.from.is_none() {
            return self.reply("503 5.5.1 Need MAIL command").await;
        }
        if self.rcpts.is_empty() {
            return self.reply("503 5.5.1 Need RCPT command").await;
        }
        self.reply("354 End data with <CR><LF>.<CR><LF>").await?;
        let mut body = Vec::new();
        let mut too_big = false;
        loop {
            let Some(line) = self.reader.read_line(MAX_SIZE).await? else {
                return Err(io::ErrorKind::UnexpectedEof.into());
            };
            let content = trim_eol(&line);
            if content == b"." {
                break;
            }
            if too_big {
                continue;
            }
            // Dot-unstuffing, and normalize line endings to CRLF.
            let content = content.strip_prefix(b".").unwrap_or(content);
            body.extend_from_slice(content);
            if line.ends_with(b"\n") {
                body.extend_from_slice(b"\r\n");
            }
            too_big = body.len() > MAX_SIZE;
        }
        if too_big {
            self.reset();
            return self.reply("552 5.3.4 Message size exceeds fixed maximum message size").await;
        }
        let id = self.deliver(body);
        self.reset();
        self.reply(&format!("250 2.0.0 OK: queued as {id}")).await
    }

    /// Stores the message for local recipients and records the rest.
    fn deliver(&mut self, body: Vec<u8>) -> String {
        self.queued += 1;
        let id = format!("LM{:08X}{}", rand_id(), self.queued);
        let from = self.from.clone().unwrap_or_default();
        let now = chrono::Local::now().fixed_offset();
        let helo = self.helo.clone().unwrap_or_else(|| "unknown".into());
        let mut remote = Vec::new();
        {
            let mut inner = self.store.lock();
            let mut delivered: Vec<String> = Vec::new();
            for rcpt in &self.rcpts {
                let key = rcpt.to_ascii_lowercase();
                if inner.account(&key).is_none() {
                    remote.push(rcpt.clone());
                    continue;
                }
                if delivered.contains(&key) {
                    continue;
                }
                delivered.push(key.clone());
                let mut raw = format!(
                    "Return-Path: <{from}>\r\nReceived: from {helo} by {HOSTNAME} (LowMail Test) with ESMTP id {id}\r\n for <{rcpt}>; {}\r\n",
                    now.to_rfc2822()
                )
                .into_bytes();
                raw.extend_from_slice(&body);
                if let Some(inbox) = inner.mailbox_mut(&key, "INBOX") {
                    inbox.append(Arc::new(MessageData::new(raw)), Vec::new(), now);
                }
            }
            if !remote.is_empty() {
                inner.outbound.push(OutboundMail { from: from.clone(), rcpt: remote, raw: body });
            }
        }
        self.store.notify();
        tracing::info!(%id, %from, rcpts = ?self.rcpts, "smtp message accepted");
        id
    }
}

/// Cheap unique-ish component for queue ids.
fn rand_id() -> u32 {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or_default();
    nanos ^ std::process::id()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(parse_path("FROM:<a@b.c> SIZE=100", "FROM:"), Some("a@b.c".into()));
        assert_eq!(parse_path("from: <>", "FROM:"), Some(String::new()));
        assert_eq!(parse_path("TO:x@y", "TO:"), Some("x@y".into()));
        assert_eq!(parse_path("TO", "TO:"), None);
    }
}
