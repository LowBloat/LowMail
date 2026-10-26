//! Shared helpers for the integration tests.
#![allow(dead_code)]

use std::time::Duration;

use lowmail_testserver::{Config, Server};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

pub const ALICE: &str = "alice@lowmail.test";
pub const BOB: &str = "bob@lowmail.test";
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// Starts a seeded server on ephemeral ports.
pub async fn server() -> Server {
    lowmail_testserver::start(Config {
        imap_port: 0,
        smtp_port: 0,
        sieve_port: 0,
        ..Config::default()
    })
    .await
    .expect("server starts")
}

pub type ImapSession = async_imap::Session<TcpStream>;

/// Connects with async-imap and logs in.
pub async fn imap_login(server: &Server, user: &str, password: &str) -> ImapSession {
    let stream = TcpStream::connect(server.imap_addr()).await.unwrap();
    let mut client = async_imap::Client::new(stream);
    let greeting = client.read_response().await.unwrap().expect("greeting");
    assert!(matches!(
        greeting.parsed(),
        async_imap::imap_proto::Response::Data { .. }
    ));
    client
        .login(user, password)
        .await
        .map_err(|(e, _)| e)
        .unwrap()
}

/// Line-oriented raw protocol client.
pub struct Raw {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl Raw {
    pub async fn connect(addr: std::net::SocketAddr) -> Raw {
        let stream = TcpStream::connect(addr).await.unwrap();
        let (r, w) = stream.into_split();
        Raw {
            reader: BufReader::new(r),
            writer: w,
        }
    }

    pub async fn send(&mut self, data: &str) {
        self.send_bytes(data.as_bytes()).await;
    }

    pub async fn send_bytes(&mut self, data: &[u8]) {
        self.writer.write_all(data).await.unwrap();
        self.writer.flush().await.unwrap();
    }

    /// Next line without CRLF.
    pub async fn line(&mut self) -> String {
        let mut buf = Vec::new();
        let n = tokio::time::timeout(TIMEOUT, self.reader.read_until(b'\n', &mut buf))
            .await
            .expect("timely response")
            .unwrap();
        assert!(n > 0, "connection closed");
        String::from_utf8_lossy(&buf)
            .trim_end_matches(['\r', '\n'])
            .to_owned()
    }

    /// Reads exactly `n` bytes.
    pub async fn bytes(&mut self, n: usize) -> Vec<u8> {
        let mut buf = vec![0; n];
        tokio::time::timeout(TIMEOUT, self.reader.read_exact(&mut buf))
            .await
            .expect("timely response")
            .unwrap();
        buf
    }

    /// Reads lines until one starts with `prefix`; returns all of them.
    pub async fn until(&mut self, prefix: &str) -> Vec<String> {
        let mut lines = Vec::new();
        loop {
            let line = self.line().await;
            let done = line.starts_with(prefix);
            lines.push(line);
            if done {
                return lines;
            }
        }
    }
}

pub fn plain_auth(user: &str, password: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(format!("\0{user}\0{password}"))
}

pub fn sample_message(subject: &str) -> String {
    format!(
        "From: Test <test@example.com>\r\nTo: Alice <alice@lowmail.test>\r\nSubject: {subject}\r\nDate: Tue, 7 Oct 2025 10:20:30 -0300\r\nMessage-ID: <{}@example.com>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello from the integration tests.\r\n",
        subject.replace(' ', "-")
    )
}
