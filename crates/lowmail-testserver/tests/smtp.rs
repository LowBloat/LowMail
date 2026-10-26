//! Raw-socket SMTP tests.

mod common;

use base64::Engine as _;
use common::*;

/// Sends a command and returns the final reply line (multi-line aware).
async fn cmd(c: &mut Raw, line: &str) -> Vec<String> {
    c.send(&format!("{line}\r\n")).await;
    reply(c).await
}

async fn reply(c: &mut Raw) -> Vec<String> {
    let mut lines = Vec::new();
    loop {
        let line = c.line().await;
        let last = line.as_bytes().get(3) != Some(&b'-');
        lines.push(line);
        if last {
            return lines;
        }
    }
}

#[tokio::test]
async fn submission_and_local_delivery() {
    let server = server().await;
    let mut c = Raw::connect(server.smtp_addr()).await;
    assert!(c.line().await.starts_with("220 "));

    let ehlo = cmd(&mut c, "EHLO client.example").await;
    assert!(ehlo.iter().all(|l| l.starts_with("250")));
    let caps = ehlo.join("\n");
    for cap in [
        "AUTH PLAIN LOGIN",
        "SIZE 52428800",
        "8BITMIME",
        "SMTPUTF8",
        "ENHANCEDSTATUSCODES",
    ] {
        assert!(caps.contains(cap), "missing {cap}");
    }
    for cap in ["PIPELINING", "STARTTLS", "CHUNKING"] {
        assert!(!caps.contains(cap), "must not advertise {cap}");
    }

    assert!(cmd(&mut c, "MAIL FROM:<alice@lowmail.test>").await[0].starts_with("530"));
    assert!(cmd(&mut c, "VRFY bob").await[0].starts_with("252"));
    assert!(
        cmd(&mut c, &format!("AUTH PLAIN {}", plain_auth(ALICE, "nope"))).await[0]
            .starts_with("535")
    );
    assert!(
        cmd(
            &mut c,
            &format!("AUTH PLAIN {}", plain_auth(ALICE, "alice"))
        )
        .await[0]
            .starts_with("235")
    );
    assert!(cmd(&mut c, "RCPT TO:<bob@lowmail.test>").await[0].starts_with("503"));
    assert!(
        cmd(
            &mut c,
            "MAIL FROM:<alice@lowmail.test> SIZE=1000 BODY=8BITMIME"
        )
        .await[0]
            .starts_with("250")
    );
    assert!(cmd(&mut c, "RCPT TO:<bob@lowmail.test>").await[0].starts_with("250"));
    assert!(cmd(&mut c, "RCPT TO:<Alice@LowMail.test>").await[0].starts_with("250"));
    assert!(cmd(&mut c, "RCPT TO:<someone@example.com>").await[0].starts_with("250"));
    assert!(cmd(&mut c, "DATA").await[0].starts_with("354"));
    c.send(
        "From: Alice <alice@lowmail.test>\r\nTo: Bob <bob@lowmail.test>, someone@example.com\r\nSubject: SMTP test\r\n\r\nFirst line\r\n..hidden dot line\r\n.\r\n",
    )
    .await;
    assert!(reply(&mut c).await[0].starts_with("250 2.0.0"));
    assert!(cmd(&mut c, "RSET").await[0].starts_with("250"));
    assert!(cmd(&mut c, "NOOP").await[0].starts_with("250"));
    assert!(cmd(&mut c, "DATA").await[0].starts_with("503"));
    assert!(cmd(&mut c, "BOGUS").await[0].starts_with("500"));
    assert!(cmd(&mut c, "QUIT").await[0].starts_with("221"));

    // Remote recipients are recorded, dot-unstuffed.
    let outbound = server.outbound();
    assert_eq!(outbound.len(), 1);
    assert_eq!(outbound[0].from, ALICE);
    assert_eq!(outbound[0].rcpt, vec!["someone@example.com"]);
    let raw = String::from_utf8(outbound[0].raw.clone()).unwrap();
    assert!(raw.contains("\r\n.hidden dot line\r\n"), "{raw}");
    assert!(raw.ends_with("\r\n"));

    // Local recipients (including the Bcc-style one) got it in INBOX.
    let mut bob = imap_login(&server, BOB, "bob").await;
    assert_eq!(bob.select("INBOX").await.unwrap().exists, 4);
    let hits = bob.search("SUBJECT \"SMTP test\"").await.unwrap();
    assert_eq!(hits.len(), 1);
    bob.logout().await.unwrap();
    let mut alice = imap_login(&server, ALICE, "alice").await;
    alice.select("INBOX").await.unwrap();
    assert_eq!(
        alice
            .search("SUBJECT \"SMTP test\" HEADER Return-Path alice")
            .await
            .unwrap()
            .len(),
        1
    );
    alice.logout().await.unwrap();
}

#[tokio::test]
async fn auth_login_and_continuations() {
    let server = server().await;
    let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);

    let mut c = Raw::connect(server.smtp_addr()).await;
    c.line().await;
    cmd(&mut c, "HELO client").await;
    assert_eq!(cmd(&mut c, "AUTH LOGIN").await[0], "334 VXNlcm5hbWU6");
    assert_eq!(cmd(&mut c, &b64(BOB)).await[0], "334 UGFzc3dvcmQ6");
    assert!(cmd(&mut c, &b64("bob")).await[0].starts_with("235"));
    assert!(cmd(&mut c, "AUTH PLAIN x").await[0].starts_with("503"));

    let mut c = Raw::connect(server.smtp_addr()).await;
    c.line().await;
    cmd(&mut c, "EHLO client").await;
    assert_eq!(cmd(&mut c, "AUTH PLAIN").await[0], "334 ");
    assert!(cmd(&mut c, &plain_auth(ALICE, "alice")).await[0].starts_with("235"));
    assert!(cmd(&mut c, "MAIL FROM:<>").await[0].starts_with("250"));
    assert!(cmd(&mut c, "MAIL FROM:<x@y>").await[0].starts_with("503"));
}
