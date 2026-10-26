//! The engine against a real (in-memory) IMAP/SMTP/ManageSieve server.

use std::sync::Arc;
use std::time::Duration;

use lowmail_core::compose::QuoteStyle;
use lowmail_core::model::*;
use lowmail_core::rpc::dispatch;
use lowmail_core::service::{FlagChange, MailClient};
use lowmail_core::sieve;
use lowmail_core::store::Store;
use serde_json::json;

fn account(server: &lowmail_testserver::Server, email: &str) -> AccountConfig {
    let ep = |addr: std::net::SocketAddr| Endpoint { host: addr.ip().to_string(), port: addr.port(), security: Security::None, accept_invalid_certs: false };
    AccountConfig {
        id: email.into(),
        label: String::new(),
        name: "Alice".into(),
        email: email.into(),
        username: email.into(),
        imap: ep(server.imap_addr()),
        smtp: ep(server.smtp_addr()),
        sieve: Some(ep(server.sieve_addr())),
        smtp_auth: true,
    }
}

async fn setup() -> (lowmail_testserver::Server, Arc<MailClient>) {
    let server = lowmail_testserver::start(lowmail_testserver::Config { imap_port: 0, smtp_port: 0, sieve_port: 0, ..Default::default() })
        .await
        .unwrap();
    let store = Arc::new(Store::memory().unwrap());
    let client = MailClient::connect(account(&server, "alice@lowmail.test"), "alice".into(), store, "alice@lowmail.test".into(), 3)
        .await
        .unwrap();
    (server, client)
}

#[tokio::test]
async fn wrong_password_is_auth_error() {
    let server = lowmail_testserver::start(lowmail_testserver::Config { imap_port: 0, smtp_port: 0, sieve_port: 0, ..Default::default() })
        .await
        .unwrap();
    let store = Arc::new(Store::memory().unwrap());
    let err = MailClient::connect(account(&server, "alice@lowmail.test"), "nope".into(), store, "x".into(), 1).await.err().unwrap();
    assert_eq!(err.kind, lowmail_core::ErrorKind::Auth);
}

#[tokio::test]
async fn folders_list_read_and_flags() {
    let (_server, c) = setup().await;
    let folders = c.folders(true).await.unwrap();
    let role = |r| folders.iter().find(|f| f.role == Some(r)).map(|f| f.path.clone());
    assert_eq!(role(FolderRole::Inbox).as_deref(), Some("INBOX"));
    assert_eq!(role(FolderRole::Sent).as_deref(), Some("Sent"));
    assert_eq!(role(FolderRole::Trash).as_deref(), Some("Trash"));
    let rel = folders.iter().find(|f| f.display_path == "Projetos/Relatórios").expect("decoded UTF-7 folder");
    assert_eq!(rel.depth, 1);
    let inbox = folders.iter().find(|f| f.path == "INBOX").unwrap();
    assert!(inbox.total >= 50 && inbox.unread > 0);

    let page = c.list("INBOX", 0, 20, &Sort::default(), &Filter::default()).await.unwrap();
    assert_eq!(page.total, inbox.total);
    assert_eq!(page.items.len(), 20);
    for w in page.items.windows(2) { assert!(w[0].date >= w[1].date, "newest first: {} {} / {} {}", w[0].subject, w[0].date, w[1].subject, w[1].date); }
    assert!(page.items.iter().all(|m| !m.from.is_empty()));

    // Every message parses.
    for uid in page.uids.iter().take(60) {
        let m = c.message("INBOX", *uid, false, false).await.unwrap();
        assert!(m.html.is_some() || m.text.is_some(), "uid {uid} has a body");
    }

    let first = page.items.iter().find(|m| !m.flags.seen).expect("an unread message");
    let m = c.message("INBOX", first.uid, false, true).await.unwrap();
    assert!(m.summary.flags.seen);
    c.set_flags("INBOX", &[first.uid], &FlagChange { seen: Some(false), flagged: Some(true), ..Default::default() }).await.unwrap();
    let again = c.summaries("INBOX", &[first.uid]).await.unwrap();
    assert!(!again[0].flags.seen && again[0].flags.flagged);

    // Search.
    let found = c.list("INBOX", 0, 50, &Sort::default(), &Filter { text: "relatório".into(), scope: SearchScope::Everything, ..Default::default() }).await.unwrap();
    assert!(found.total >= 1, "UTF-8 search finds messages");
    let unread = c.list("INBOX", 0, 500, &Sort::default(), &Filter { unread: true, ..Default::default() }).await.unwrap();
    assert!(unread.items.iter().all(|m| !m.flags.seen));
    let everywhere = c.search_all(&Filter { text: "a".into(), ..Default::default() }).await.unwrap();
    assert!(everywhere.iter().any(|m| m.folder != "INBOX"));

    // Sorting by subject.
    let by_subject = c.list("INBOX", 0, 30, &Sort { field: SortField::Subject, descending: false }, &Filter::default()).await.unwrap();
    assert_eq!(by_subject.items.len(), 30);
}

#[tokio::test]
async fn attachments_inline_images_and_remote_content() {
    let (_server, c) = setup().await;
    let page = c.list("INBOX", 0, 500, &Sort::default(), &Filter::default()).await.unwrap();
    let with_files = page.items.iter().filter(|m| m.has_attachments).collect::<Vec<_>>();
    assert!(!with_files.is_empty());
    let mut saw_pdf = false;
    for s in with_files {
        let m = c.message("INBOX", s.uid, false, false).await.unwrap();
        for a in m.attachments.iter().filter(|a| !a.inline) {
            let (name, ct, bytes) = c.attachment("INBOX", s.uid, &a.part).await.unwrap();
            assert_eq!(name, a.filename);
            assert_eq!(bytes.len() as u64, a.size);
            if ct == "application/pdf" {
                assert!(bytes.starts_with(b"%PDF"));
                saw_pdf = true;
            }
        }
    }
    assert!(saw_pdf);
    let mut saw_remote = false;
    let mut saw_cid = false;
    for uid in &page.uids {
        let m = c.message("INBOX", *uid, false, false).await.unwrap();
        let html = m.html.unwrap_or_default();
        assert!(!html.to_lowercase().contains("<script"));
        if m.remote_content {
            saw_remote = true;
            assert!(!html.contains("src=\"https://"), "remote images blocked");
            let allowed = c.message("INBOX", *uid, true, false).await.unwrap();
            assert!(allowed.html.unwrap().contains("https://"));
        }
        if html.contains("data:image/") && m.attachments.iter().any(|a| a.inline) {
            saw_cid = true;
        }
    }
    assert!(saw_remote && saw_cid);
}

#[tokio::test]
async fn move_delete_archive_junk() {
    let (_server, c) = setup().await;
    let page = c.list("INBOX", 0, 10, &Sort::default(), &Filter::default()).await.unwrap();
    let uids: Vec<u32> = page.uids[..4].to_vec();
    let total = page.total;
    c.move_messages("INBOX", &uids[..1], "Projetos/Clientes").await.unwrap();
    c.delete_messages("INBOX", &uids[1..2], false).await.unwrap();
    c.archive("INBOX", &uids[2..3]).await.unwrap();
    c.junk("INBOX", &uids[3..4], true).await.unwrap();
    let after = c.list("INBOX", 0, 10, &Sort::default(), &Filter::default()).await.unwrap();
    assert_eq!(after.total, total - 4);
    let trash = c.list("Trash", 0, 100, &Sort::default(), &Filter::default()).await.unwrap();
    assert!(trash.total >= 1);
    // Deleting inside Trash removes for good.
    c.delete_messages("Trash", &trash.uids, false).await.unwrap();
    assert_eq!(c.list("Trash", 0, 10, &Sort::default(), &Filter::default()).await.unwrap().total, 0);
    let junk = c.list("Junk", 0, 100, &Sort::default(), &Filter::default()).await.unwrap();
    let back = junk.uids[0];
    c.junk("Junk", &[back], false).await.unwrap();
    assert_eq!(c.list("INBOX", 0, 10, &Sort::default(), &Filter::default()).await.unwrap().total, total - 3);
}

#[tokio::test]
async fn folder_management() {
    let (_server, c) = setup().await;
    let path = c.create_folder(None, "Notas fiscais").await.unwrap();
    assert_eq!(path, "Notas fiscais");
    let child = c.create_folder(Some(path.clone()), "2026").await.unwrap();
    assert_eq!(child, "Notas fiscais/2026");
    let renamed = c.rename_folder(&path, "Faturas", None).await.unwrap();
    let folders = c.folders(false).await.unwrap();
    assert!(folders.iter().any(|f| f.path == "Faturas/2026"), "children follow the rename");
    c.delete_folder(&renamed).await.unwrap();
    let folders = c.folders(false).await.unwrap();
    assert!(!folders.iter().any(|f| f.path.starts_with("Faturas")));
    assert!(c.delete_folder("INBOX").await.is_err());
}

#[tokio::test]
async fn compose_reply_send_and_drafts() {
    let (server, c) = setup().await;
    let page = c.list("INBOX", 0, 5, &Sort::default(), &Filter::default()).await.unwrap();
    let original = &page.items[0];
    let source = MessageRef { folder: "INBOX".into(), uid: original.uid };
    let mut draft = c.prepare(ComposeMode::Reply, Some(source.clone()), true, &QuoteStyle::default()).await.unwrap();
    assert!(draft.subject.starts_with("Re: "));
    assert_eq!(draft.to[0].email, original.from[0].email);
    draft.to = vec![Address::new(Some("Bob".into()), "bob@lowmail.test"), Address::new(None, "someone@example.com")];
    draft.bcc = vec![Address::new(None, "hidden@example.com")];
    draft.html = Some(format!("<p>Obrigado!</p>{}", draft.html.unwrap_or_default()));
    let up = c.add_upload("notas.txt", "text/plain", b"linha 1\nlinha 2".to_vec()).unwrap();
    draft.attachments.push(DraftAttachment {
        source: AttachmentSource::Upload { id: up.id.clone() },
        filename: up.filename,
        content_type: up.content_type,
        size: up.size,
        inline: false,
        content_id: None,
    });

    // Save as draft, then save again: one draft remains.
    let saved = c.save_draft(draft.clone()).await.unwrap();
    assert!(saved.draft_ref.is_some());
    let saved2 = c.save_draft(saved.clone()).await.unwrap();
    let drafts = c.list("Drafts", 0, 50, &Sort::default(), &Filter::default()).await.unwrap();
    assert_eq!(drafts.items.iter().filter(|m| m.subject == draft.subject).count(), 1);
    // Reopen the draft.
    let reopened = c.prepare(ComposeMode::Draft, saved2.draft_ref.clone(), true, &QuoteStyle::default()).await.unwrap();
    assert_eq!(reopened.subject, draft.subject);
    assert_eq!(reopened.bcc.len(), 1);
    assert_eq!(reopened.attachments.len(), 1);
    assert_eq!(reopened.source.as_ref().map(|s| s.uid), Some(original.uid));

    let result = c.send(reopened, true).await.unwrap();
    assert!(result.saved, "{:?}", result.warnings);
    // Remote recipients went out, Bcc only in the envelope.
    let out = server.outbound();
    let sent = out.last().unwrap();
    assert!(sent.rcpt.iter().any(|r| r == "someone@example.com"));
    assert!(sent.rcpt.iter().any(|r| r == "hidden@example.com"));
    assert!(!String::from_utf8_lossy(&sent.raw).contains("hidden@example.com"));
    // Bob received it.
    let bob = MailClient::connect(account(&server, "bob@lowmail.test"), "bob".into(), Arc::new(Store::memory().unwrap()), "bob".into(), 1).await.unwrap();
    let bob_inbox = bob.list("INBOX", 0, 10, &Sort::default(), &Filter::default()).await.unwrap();
    let got = bob.message("INBOX", bob_inbox.items[0].uid, false, false).await.unwrap();
    assert_eq!(got.summary.subject, draft.subject);
    assert_eq!(got.attachments.iter().filter(|a| !a.inline).count(), 1);
    assert_eq!(got.summary.in_reply_to, original.message_id);
    // Sent copy keeps Bcc, the draft is gone and the original is answered.
    let sent_folder = c.list("Sent", 0, 5, &Sort::default(), &Filter::default()).await.unwrap();
    let copy = c.message("Sent", sent_folder.items[0].uid, false, false).await.unwrap();
    assert_eq!(copy.bcc[0].email, "hidden@example.com");
    let drafts = c.list("Drafts", 0, 50, &Sort::default(), &Filter::default()).await.unwrap();
    assert!(drafts.items.iter().all(|m| m.subject != draft.subject));
    assert!(c.summaries("INBOX", &[original.uid]).await.unwrap()[0].flags.answered);
    // Recipients were collected for suggestions.
    assert!(!c.store().suggest(c.owner(), "bob", 5).unwrap().is_empty());
}

#[tokio::test]
async fn forward_keeps_attachments() {
    let (_server, c) = setup().await;
    let page = c.list("INBOX", 0, 500, &Sort::default(), &Filter::default()).await.unwrap();
    let with_files = page.items.iter().find(|m| m.has_attachments).unwrap();
    let source = MessageRef { folder: "INBOX".into(), uid: with_files.uid };
    let fwd = c.prepare(ComposeMode::Forward, Some(source.clone()), true, &QuoteStyle::default()).await.unwrap();
    assert!(fwd.subject.starts_with("Fwd: "));
    assert!(!fwd.attachments.is_empty());
    let as_file = c.prepare(ComposeMode::ForwardAttachment, Some(source), true, &QuoteStyle::default()).await.unwrap();
    assert_eq!(as_file.attachments[0].content_type, "message/rfc822");
}

#[tokio::test]
async fn filters_round_trip() {
    let (server, c) = setup().await;
    let mut state = c.filters().await.unwrap();
    assert!(state.extensions.iter().any(|e| e == "fileinto"));
    state.filters.rules.push(sieve::Rule {
        id: "1".into(),
        name: "Boletos".into(),
        enabled: true,
        match_all: true,
        conditions: vec![sieve::Condition { field: sieve::Field::Subject, header: String::new(), op: sieve::Op::Contains, value: "boleto".into() }],
        actions: vec![sieve::Action::Move { folder: "Projetos/Clientes".into() }],
        stop: true,
    });
    state.filters.vacation = sieve::Vacation { enabled: true, subject: "Ausente".into(), body: "Volto logo".into(), ..Default::default() };
    c.save_filters(&state.filters).await.unwrap();
    let scripts = server.sieve_scripts("alice@lowmail.test");
    assert!(scripts.iter().any(|(name, _, active)| name == "lowmail" && *active));
    let back = c.filters().await.unwrap();
    assert_eq!(back.filters.rules[0].name, "Boletos");
    assert!(back.filters.vacation.enabled);
}

#[tokio::test]
async fn rpc_and_events() {
    let (server, c) = setup().await;
    let folders = dispatch(&c, "folders.list", json!({})).await.unwrap();
    assert!(folders.as_array().unwrap().len() > 5);
    let list = dispatch(&c, "messages.list", json!({ "folder": "INBOX", "limit": 5 })).await.unwrap();
    assert_eq!(list["items"].as_array().unwrap().len(), 5);
    let err = dispatch(&c, "nope", json!({})).await.unwrap_err();
    assert_eq!(err.kind, lowmail_core::ErrorKind::NotFound);
    dispatch(&c, "settings.set", json!({ "settings": { "layout": "rows" } })).await.unwrap();
    assert_eq!(dispatch(&c, "settings.get", json!({})).await.unwrap()["layout"], "rows");
    let quota = dispatch(&c, "quota.get", json!({})).await.unwrap();
    assert!(quota["limit"].as_u64().unwrap() > 0);
    let saved = dispatch(&c, "contacts.save", json!({ "contact": { "name": "Ana", "emails": ["ana@example.com"] } })).await.unwrap();
    assert!(!saved["id"].as_str().unwrap().is_empty());
    let imported = dispatch(&c, "contacts.import", json!({ "text": "BEGIN:VCARD\r\nFN:Zé\r\nEMAIL:ze@example.com\r\nEND:VCARD\r\n" })).await.unwrap();
    assert_eq!(imported["added"], 1);

    // New mail arrives while watching.
    let mut events = c.subscribe();
    c.start_watching();
    tokio::time::sleep(Duration::from_millis(500)).await;
    server.deliver("alice@lowmail.test", "INBOX", b"From: Carol <carol@example.com>\r\nTo: alice@lowmail.test\r\nSubject: Novidade\r\nDate: Mon, 26 Oct 2026 10:00:00 -0300\r\nMessage-ID: <new1@example.com>\r\n\r\nOi!\r\n", &[]);
    let event = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(MailEvent::NewMail { latest, .. }) = events.recv().await {
                return latest;
            }
        }
    })
    .await
    .expect("a new-mail event");
    assert_eq!(event[0].subject, "Novidade");
    c.close().await;
}
