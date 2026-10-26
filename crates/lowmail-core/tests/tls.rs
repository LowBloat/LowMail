//! TLS and STARTTLS against the test mail server, behind small TLS proxies
//! with a self-signed certificate.

use std::net::SocketAddr;
use std::sync::Arc;

use lowmail_core::model::*;
use lowmail_core::service::MailClient;
use lowmail_core::store::Store;
use lowmail_core::{ErrorKind, sieve};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

fn acceptor() -> TlsAcceptor {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let der = CertificateDer::from(cert.cert.der().to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der()));
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![der], key)
        .unwrap();
    TlsAcceptor::from(Arc::new(config))
}

#[derive(Clone, Copy)]
enum Mode {
    /// TLS from the first byte.
    Implicit,
    /// IMAP STARTTLS: greeting in clear, then `tag STARTTLS`.
    ImapStartTls,
    /// ManageSieve STARTTLS: capabilities, `STARTTLS`, capabilities again.
    SieveStartTls,
}

async fn pipe<A, B>(a: A, b: B)
where
    A: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    B: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut ar, mut aw) = tokio::io::split(a);
    let (mut br, mut bw) = tokio::io::split(b);
    let _ = tokio::join!(tokio::io::copy(&mut ar, &mut bw), tokio::io::copy(&mut br, &mut aw));
}

/// Listens on an ephemeral port and forwards to `backend` through TLS.
async fn proxy(backend: SocketAddr, mode: Mode) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let tls = acceptor();
    tokio::spawn(async move {
        loop {
            let Ok((client, _)) = listener.accept().await else { return };
            let tls = tls.clone();
            tokio::spawn(async move {
                let server = TcpStream::connect(backend).await.unwrap();
                match mode {
                    Mode::Implicit => {
                        let Ok(client) = tls.accept(client).await else { return };
                        pipe(client, server).await;
                    }
                    Mode::ImapStartTls => {
                        let mut server = BufReader::new(server);
                        let mut client = BufReader::new(client);
                        let mut greeting = String::new();
                        server.read_line(&mut greeting).await.unwrap();
                        client.get_mut().write_all(greeting.as_bytes()).await.unwrap();
                        let mut line = String::new();
                        loop {
                            line.clear();
                            if client.read_line(&mut line).await.unwrap_or(0) == 0 {
                                return;
                            }
                            let tag = line.split_whitespace().next().unwrap_or("*").to_string();
                            if line.to_ascii_uppercase().contains("STARTTLS") {
                                client.get_mut().write_all(format!("{tag} OK Begin TLS\r\n").as_bytes()).await.unwrap();
                                break;
                            }
                            // Anything before STARTTLS (CAPABILITY) goes to the server.
                            server.get_mut().write_all(line.as_bytes()).await.unwrap();
                            let mut reply = String::new();
                            loop {
                                reply.clear();
                                server.read_line(&mut reply).await.unwrap();
                                client.get_mut().write_all(reply.as_bytes()).await.unwrap();
                                if reply.starts_with(&tag) {
                                    break;
                                }
                            }
                        }
                        let Ok(client) = tls.accept(client.into_inner()).await else { return };
                        pipe(client, server.into_inner()).await;
                    }
                    Mode::SieveStartTls => {
                        let mut server = BufReader::new(server);
                        let mut caps = Vec::new();
                        loop {
                            let mut l = String::new();
                            server.read_line(&mut l).await.unwrap();
                            caps.push(l.clone());
                            if l.starts_with("OK") {
                                break;
                            }
                        }
                        let mut client = BufReader::new(client);
                        let advertised: String =
                            caps.iter().map(|l| if l.starts_with("OK") { "\"STARTTLS\"\r\n".to_string() + l } else { l.clone() }).collect();
                        client.get_mut().write_all(advertised.as_bytes()).await.unwrap();
                        let mut line = String::new();
                        client.read_line(&mut line).await.unwrap();
                        assert!(line.to_ascii_uppercase().starts_with("STARTTLS"), "{line}");
                        client.get_mut().write_all(b"OK \"Begin TLS\"\r\n").await.unwrap();
                        let Ok(mut client) = tls.accept(client.into_inner()).await else { return };
                        // After STARTTLS the server sends its capabilities again.
                        let again: String = caps.concat();
                        client.write_all(again.as_bytes()).await.unwrap();
                        pipe(client, server.into_inner()).await;
                    }
                }
            });
        }
    });
    addr
}

fn endpoint(addr: SocketAddr, security: Security, lenient: bool) -> Endpoint {
    Endpoint { host: "localhost".into(), port: addr.port(), security, accept_invalid_certs: lenient }
}

#[tokio::test]
async fn implicit_tls_and_starttls() {
    let mail = lowmail_testserver::start(lowmail_testserver::Config { imap_port: 0, smtp_port: 0, sieve_port: 0, ..Default::default() })
        .await
        .unwrap();
    let imaps = proxy(mail.imap_addr(), Mode::Implicit).await;
    let imap_starttls = proxy(mail.imap_addr(), Mode::ImapStartTls).await;
    let smtps = proxy(mail.smtp_addr(), Mode::Implicit).await;
    let sieve_starttls = proxy(mail.sieve_addr(), Mode::SieveStartTls).await;

    let account = |imap: Endpoint, lenient: bool| AccountConfig {
        id: "a".into(),
        label: String::new(),
        name: "Alice".into(),
        email: "alice@lowmail.test".into(),
        username: "alice@lowmail.test".into(),
        imap,
        smtp: endpoint(smtps, Security::Tls, lenient),
        sieve: Some(endpoint(sieve_starttls, Security::StartTls, lenient)),
        smtp_auth: true,
    };

    // A self-signed certificate is refused unless the account allows it.
    let strict = MailClient::connect(
        account(endpoint(imaps, Security::Tls, false), false),
        "alice".into(),
        Arc::new(Store::memory().unwrap()),
        "a".into(),
        1,
    )
    .await;
    assert_eq!(strict.err().map(|e| e.kind), Some(ErrorKind::Tls));

    for imap in [endpoint(imaps, Security::Tls, true), endpoint(imap_starttls, Security::StartTls, true)] {
        let c = MailClient::connect(account(imap, true), "alice".into(), Arc::new(Store::memory().unwrap()), "a".into(), 1).await.unwrap();
        let page = c.list("INBOX", 0, 5, &Sort::default(), &Filter::default()).await.unwrap();
        assert_eq!(page.items.len(), 5);
        c.message("INBOX", page.uids[0], false, false).await.unwrap();
        c.close().await;
    }

    // SMTP over TLS and the full test of an account.
    lowmail_core::service::test_account(&account(endpoint(imaps, Security::Tls, true), true), "alice").await.unwrap();
    let c = MailClient::connect(
        account(endpoint(imaps, Security::Tls, true), true),
        "alice".into(),
        Arc::new(Store::memory().unwrap()),
        "a".into(),
        1,
    )
    .await
    .unwrap();
    let mut draft = c.prepare(ComposeMode::New, None, false, &Default::default()).await.unwrap();
    draft.to = vec![Address::new(None, "someone@example.com")];
    draft.subject = "Over TLS".into();
    draft.text = "ok".into();
    c.send(draft, false).await.unwrap();
    assert!(mail.outbound().iter().any(|m| String::from_utf8_lossy(&m.raw).contains("Over TLS")));

    // ManageSieve with STARTTLS.
    let state = c.filters().await.unwrap();
    c.save_filters(&sieve::Filters {
        rules: vec![],
        vacation: sieve::Vacation { enabled: true, subject: "Fora".into(), body: "Volto logo".into(), ..state.filters.vacation },
    })
    .await
    .unwrap();
    assert!(mail.sieve_scripts("alice@lowmail.test").iter().any(|(n, s, active)| n == "lowmail" && *active && s.contains("vacation")));

    // Wrong certificate on SMTP: a TLS error, not a hang.
    let strict_smtp = AccountConfig { smtp: endpoint(smtps, Security::Tls, false), ..account(endpoint(imaps, Security::Tls, true), true) };
    let err = lowmail_core::smtp::test(&strict_smtp, "alice").await.unwrap_err();
    assert!(matches!(err.kind, ErrorKind::Tls | ErrorKind::Network), "{err:?}");
}
