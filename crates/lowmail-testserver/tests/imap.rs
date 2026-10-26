//! IMAP end-to-end tests using async-imap (and therefore imap-proto's parser).

mod common;

use std::time::Duration;

use async_imap::extensions::idle::IdleResponse;
use async_imap::imap_proto::{
    self, BodyStructure, MailboxDatum, Response, ResponseCode, Status, rfc4315::UidSetMember,
};
use async_imap::types::{Fetch, Flag, NameAttribute};
use common::*;
use futures::TryStreamExt;

/// Runs a raw command and collects every response up to the tagged one,
/// as `(untagged, tagged)`. A macro because async-imap's `ResponseData`
/// type cannot be named outside the crate.
macro_rules! raw_command {
    ($session:expr, $command:expr $(,)?) => {{
        let id = $session.run_command($command).await.unwrap();
        let mut untagged = Vec::new();
        loop {
            let resp = tokio::time::timeout(TIMEOUT, $session.read_response())
                .await
                .expect("timely response")
                .unwrap()
                .expect("response");
            if let Response::Done { tag, .. } = resp.parsed()
                && *tag == id
            {
                break (untagged, resp);
            }
            untagged.push(resp);
        }
    }};
}

fn done_ok(resp: &Response<'_>) -> bool {
    matches!(
        resp,
        Response::Done {
            status: Status::Ok,
            ..
        }
    )
}

/// Status and response code of a tagged response.
fn done_parts<'a>(resp: &'a Response<'a>) -> (&'a Status, Option<&'a ResponseCode<'a>>) {
    match resp {
        Response::Done {
            status, outcome, ..
        } => (status, outcome.code.as_ref()),
        other => panic!("tagged response expected, got {other:?}"),
    }
}

fn sort_result<'a>(mut responses: impl Iterator<Item = &'a Response<'a>>) -> Vec<u32> {
    responses
        .find_map(|r| match r {
            Response::MailboxData(MailboxDatum::Sort(ids)) => Some(ids.clone()),
            _ => None,
        })
        .expect("SORT response")
}
#[tokio::test]
async fn login_list_status_select() {
    let server = server().await;
    let mut s = imap_login(&server, ALICE, "alice").await;

    let caps = s.capabilities().await.unwrap();
    for cap in [
        "IDLE",
        "MOVE",
        "UIDPLUS",
        "SPECIAL-USE",
        "SORT",
        "LITERAL+",
        "QUOTA",
    ] {
        assert!(caps.has_str(cap), "missing capability {cap}");
    }
    assert!(caps.has_str("AUTH=PLAIN"));

    let names: Vec<_> = s
        .list(Some(""), Some("*"))
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let find = |name: &str| {
        names
            .iter()
            .find(|n| n.name() == name)
            .unwrap_or_else(|| panic!("mailbox {name} listed"))
    };
    assert_eq!(names[0].name(), "INBOX");
    assert_eq!(find("INBOX").delimiter(), Some("/"));
    assert!(find("Sent").attributes().contains(&NameAttribute::Sent));
    assert!(find("Drafts").attributes().contains(&NameAttribute::Drafts));
    assert!(find("Trash").attributes().contains(&NameAttribute::Trash));
    assert!(find("Junk").attributes().contains(&NameAttribute::Junk));
    assert!(
        find("Archive")
            .attributes()
            .contains(&NameAttribute::Archive)
    );
    assert!(
        find("Projetos")
            .attributes()
            .iter()
            .any(|a| matches!(a, NameAttribute::Extension(e) if e == "\\HasChildren"))
    );
    find("Projetos/Relat&APM-rios");
    find("Projetos/Clientes");

    // `%` does not descend into the hierarchy.
    let top: Vec<_> = s
        .list(Some(""), Some("%"))
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(top.iter().all(|n| !n.name().contains('/')));
    let sub: Vec<_> = s
        .list(Some("Projetos/"), Some("%"))
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(sub.len(), 2);

    let lsub: Vec<_> = s
        .lsub(Some(""), Some("*"))
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(lsub.len(), names.len());

    // LIST (SPECIAL-USE) only returns the special folders.
    let (untagged, done) = raw_command!(s, "LIST (SPECIAL-USE) \"\" \"*\"");
    assert!(done_ok(done.parsed()));
    assert_eq!(untagged.len(), 5);

    let status = s
        .status("INBOX", "(MESSAGES RECENT UIDNEXT UIDVALIDITY UNSEEN)")
        .await
        .unwrap();
    assert!(status.exists >= 55, "INBOX has {} messages", status.exists);
    assert!(status.unseen.unwrap() > 0);

    let mbox = s.select("inbox").await.unwrap();
    assert_eq!(mbox.exists, status.exists);
    assert_eq!(mbox.uid_validity, status.uid_validity);
    assert_eq!(mbox.uid_next, status.uid_next);
    assert!(mbox.unseen.is_some());
    assert!(
        mbox.permanent_flags
            .iter()
            .any(|f| matches!(f, Flag::MayCreate))
    );

    let mbox = s.examine("Projetos/Relat&APM-rios").await.unwrap();
    assert_eq!(mbox.exists, 2);

    let (_, root) = s.get_quota_root("INBOX").await.unwrap();
    let storage = &root[0].resources[0];
    assert!(storage.usage > 0);
    assert_eq!(storage.limit, 102_400);

    s.logout().await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn fetch_parses_every_seeded_message() {
    let server = server().await;
    let mut s = imap_login(&server, ALICE, "alice").await;
    let names: Vec<String> = s
        .list(Some(""), Some("*"))
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
        .iter()
        .map(|n| n.name().to_owned())
        .collect();

    let mut total = 0;
    for name in &names {
        let mbox = s.examine(name).await.unwrap();
        if mbox.exists == 0 {
            continue;
        }
        let fetches: Vec<Fetch> = s
            .uid_fetch(
                "1:*",
                "(FLAGS ENVELOPE BODYSTRUCTURE RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER.FIELDS (SUBJECT FROM)])",
            )
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(fetches.len() as u32, mbox.exists, "mailbox {name}");
        for f in &fetches {
            assert!(f.uid.is_some(), "UID FETCH must include UID");
            assert!(f.envelope().is_some());
            assert!(f.bodystructure().is_some());
            assert!(f.internal_date().is_some());
            assert!(f.size.unwrap() > 0);
        }
        total += fetches.len();
    }
    assert!(total >= 75, "fetched {total} messages");

    s.examine("INBOX").await.unwrap();
    let fetches: Vec<Fetch> = s
        .fetch("1:*", "(UID ENVELOPE BODYSTRUCTURE)")
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();

    // Envelope values stay RFC 2047 encoded, like real servers.
    let encoded = fetches
        .iter()
        .find(|f| {
            f.envelope()
                .and_then(|e| e.subject.as_ref())
                .is_some_and(|s| s.starts_with(b"=?UTF-8?B?"))
        })
        .expect("B-encoded subject");
    let from = &encoded.envelope().unwrap().from.as_ref().unwrap()[0];
    assert_eq!(from.mailbox.as_deref(), Some(&b"patricia.lopes"[..]));
    assert_eq!(from.host.as_deref(), Some(&b"acme.com.br"[..]));

    // The contract message: multipart/mixed with a PDF attachment.
    let contract = fetches
        .iter()
        .find(|f| {
            f.envelope()
                .and_then(|e| e.subject.as_ref())
                .is_some_and(|s| s.as_ref() == b"Contrato revisado em anexo")
        })
        .expect("contract message");
    let Some(BodyStructure::Multipart { common, bodies, .. }) = contract.bodystructure() else {
        panic!("multipart expected");
    };
    assert!(common.ty.subtype.eq_ignore_ascii_case("mixed"));
    assert_eq!(bodies.len(), 2);
    let BodyStructure::Basic { common, other, .. } = &bodies[1] else {
        panic!("basic part expected");
    };
    assert!(common.ty.subtype.eq_ignore_ascii_case("pdf"));
    let disposition = common.disposition.as_ref().unwrap();
    assert_eq!(disposition.ty, "attachment");
    assert_eq!(
        disposition.params.as_ref().unwrap()[0].1,
        "contrato-revisado.pdf"
    );
    assert_eq!(other.transfer_encoding, imap_proto::ContentEncoding::Base64);

    // Body sections: the decoded PDF starts with the PDF magic.
    let uid = contract.uid.unwrap();
    let parts: Vec<Fetch> = s
        .uid_fetch(
            uid.to_string(),
            "(BODY.PEEK[2] BODY.PEEK[1.MIME] BODY.PEEK[]<0.20>)",
        )
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let pdf_b64 = parts[0]
        .section(&imap_proto::SectionPath::Part(vec![2], None))
        .unwrap();
    use base64::Engine as _;
    let cleaned: Vec<u8> = pdf_b64
        .iter()
        .copied()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    let pdf = base64::engine::general_purpose::STANDARD
        .decode(cleaned)
        .unwrap();
    assert!(pdf.starts_with(b"%PDF-1.4"));
    let mime = parts[0]
        .section(&imap_proto::SectionPath::Part(
            vec![1],
            Some(imap_proto::MessageSection::Mime),
        ))
        .unwrap();
    assert!(mime.starts_with(b"Content-Type: text/plain"));

    s.logout().await.unwrap();
}

#[tokio::test]
async fn search_and_sort() {
    let server = server().await;
    let mut s = imap_login(&server, ALICE, "alice").await;
    let mbox = s.select("INBOX").await.unwrap();

    let unseen = s.uid_search("UNSEEN").await.unwrap();
    assert!(!unseen.is_empty());
    let all = s.uid_search("ALL").await.unwrap();
    assert_eq!(all.len() as u32, mbox.exists);
    let seen = s.uid_search("NOT UNSEEN").await.unwrap();
    assert_eq!(seen.len() + unseen.len(), all.len());

    // Decoded RFC 2047 subjects and names, case-insensitive.
    assert_eq!(
        s.uid_search("CHARSET UTF-8 SUBJECT \"relatório mensal\"")
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        s.uid_search("CHARSET UTF-8 FROM \"JOÃO GONÇALVES\"")
            .await
            .unwrap()
            .len(),
        1
    );
    // ISO-8859-1 encoded subject and body.
    assert_eq!(
        s.uid_search("CHARSET UTF-8 SUBJECT \"promoção de verão\"")
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        s.uid_search("CHARSET UTF-8 BODY \"manutenção do elevador\"")
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        !s.uid_search("OR FLAGGED ANSWERED")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        s.uid_search("HEADER List-Id rust-br").await.unwrap().len(),
        1
    );
    assert!(
        !s.uid_search("SINCE 1-Jan-2000 LARGER 1000")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(s.uid_search("BEFORE 1-Jan-2000").await.unwrap().is_empty());

    // Literal search argument with UTF-8 bytes.
    let needle = "migração";
    let (untagged, done) = raw_command!(
        s,
        &format!(
            "UID SEARCH CHARSET UTF-8 SUBJECT {{{}+}}\r\n{needle}",
            needle.len()
        ),
    );
    assert!(done_ok(done.parsed()));
    let Response::MailboxData(MailboxDatum::Search(hits)) = untagged[0].parsed() else {
        panic!("SEARCH response expected");
    };
    assert_eq!(hits.len(), 4);

    let (untagged, done) = raw_command!(s, "UID SORT (REVERSE DATE) UTF-8 ALL");
    assert!(done_ok(done.parsed()));
    let by_date = sort_result(untagged.iter().map(|r| r.parsed()));
    assert_eq!(by_date.len() as u32, mbox.exists);
    assert_eq!(by_date[0], *all.iter().max().unwrap());

    let (untagged, _) = raw_command!(s, "UID SORT (ARRIVAL) UTF-8 ALL");
    let arrival = sort_result(untagged.iter().map(|r| r.parsed()));
    let mut sorted = arrival.clone();
    sorted.sort_unstable();
    assert_eq!(arrival, sorted);

    let (untagged, _) = raw_command!(s, "SORT (SUBJECT REVERSE SIZE) UTF-8 SUBJECT servidor");
    assert!(sort_result(untagged.iter().map(|r| r.parsed())).len() >= 5);

    let (_, done) = raw_command!(s, "SEARCH CHARSET KOI8-R ALL");
    let (status, code) = done_parts(done.parsed());
    assert_eq!(*status, Status::No);
    assert!(matches!(code, Some(ResponseCode::BadCharset(_))));
    s.logout().await.unwrap();
}

#[tokio::test]
async fn store_append_copy_move_expunge() {
    let server = server().await;
    let mut s = imap_login(&server, ALICE, "alice").await;
    let inbox = s.select("INBOX").await.unwrap();
    let first_uid = *s.uid_search("ALL").await.unwrap().iter().min().unwrap();

    let updates: Vec<Fetch> = s
        .uid_store(first_uid.to_string(), "+FLAGS (\\Flagged $Forwarded)")
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].uid, Some(first_uid));
    let flags: Vec<Flag<'_>> = updates[0].flags().collect();
    assert!(flags.contains(&Flag::Flagged));
    assert!(flags.contains(&Flag::Custom("$Forwarded".into())));
    let silent: Vec<Fetch> = s
        .uid_store(first_uid.to_string(), "-FLAGS.SILENT ($Forwarded)")
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(silent.is_empty());

    // Non-PEEK body fetch sets \Seen.
    let unseen = *s.uid_search("UNSEEN").await.unwrap().iter().min().unwrap();
    let fetched: Vec<Fetch> = s
        .uid_fetch(unseen.to_string(), "BODY[TEXT]")
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(fetched[0].text().is_some());
    assert!(fetched[0].flags().any(|f| f == Flag::Seen));

    // APPEND with a synchronizing literal (async-imap waits for "+").
    s.append(
        "INBOX",
        Some("(\\Seen)"),
        Some("\"07-Oct-2025 10:20:30 -0300\""),
        sample_message("appended sync"),
    )
    .await
    .unwrap();

    // APPEND with LITERAL+ and APPENDUID.
    let msg = sample_message("appended nonsync");
    let (_, done) = raw_command!(
        s,
        &format!("APPEND INBOX (\\Seen $Label1) {{{}+}}\r\n{msg}", msg.len()),
    );
    let (status, code) = done_parts(done.parsed());
    assert_eq!(*status, Status::Ok);
    let Some(ResponseCode::AppendUid(validity, uids)) = code else {
        panic!("APPENDUID expected: {:?}", done.parsed());
    };
    assert_eq!(Some(*validity), inbox.uid_validity);
    let UidSetMember::Uid(appended) = uids[0] else {
        panic!("single uid expected");
    };
    s.noop().await.unwrap();
    let found = s.uid_search("SUBJECT \"appended nonsync\"").await.unwrap();
    assert_eq!(found.into_iter().collect::<Vec<_>>(), vec![appended]);

    // APPEND to a missing mailbox suggests TRYCREATE.
    let (_, done) = raw_command!(s, "APPEND Missing {3+}\r\nabc");
    let (status, code) = done_parts(done.parsed());
    assert_eq!(*status, Status::No);
    assert_eq!(code, Some(&ResponseCode::TryCreate));

    // COPY with COPYUID.
    let trash_before = s.status("Trash", "(MESSAGES)").await.unwrap().exists;
    let (_, done) = raw_command!(s, &format!("UID COPY {appended} Trash"));
    let (status, code) = done_parts(done.parsed());
    assert_eq!(*status, Status::Ok);
    assert!(matches!(code, Some(ResponseCode::CopyUid(..))));
    assert_eq!(
        s.status("Trash", "(MESSAGES)").await.unwrap().exists,
        trash_before + 1
    );

    // MOVE removes the message from the source.
    let archive_before = s.status("Archive", "(MESSAGES)").await.unwrap().exists;
    s.uid_mv(appended.to_string(), "Archive").await.unwrap();
    assert_eq!(
        s.status("Archive", "(MESSAGES)").await.unwrap().exists,
        archive_before + 1
    );
    assert!(
        s.uid_search(&format!("UID {appended}"))
            .await
            .unwrap()
            .is_empty()
    );

    // \Deleted + EXPUNGE.
    let count = s.uid_search("ALL").await.unwrap().len();
    let _: Vec<Fetch> = s
        .uid_store(first_uid.to_string(), "+FLAGS.SILENT (\\Deleted)")
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let expunged: Vec<u32> = s.expunge().await.unwrap().try_collect().await.unwrap();
    assert_eq!(expunged, vec![1]);
    assert_eq!(s.uid_search("ALL").await.unwrap().len(), count - 1);

    // UID EXPUNGE only touches the given UIDs.
    let uids: Vec<u32> = {
        let mut v: Vec<u32> = s.uid_search("ALL").await.unwrap().into_iter().collect();
        v.sort_unstable();
        v
    };
    let set = format!("{},{}", uids[0], uids[1]);
    let _: Vec<Fetch> = s
        .uid_store(&set, "+FLAGS.SILENT (\\Deleted)")
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let removed: Vec<u32> = s
        .uid_expunge(uids[0].to_string())
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(removed.len(), 1);

    // Mailbox management: create (with parents), rename (with children), delete.
    s.create("Teste/Sub/Folha").await.unwrap();
    s.rename("Teste", "Renomeado").await.unwrap();
    let names: Vec<String> = s
        .list(Some(""), Some("Renomeado*"))
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
        .iter()
        .map(|n| n.name().to_owned())
        .collect();
    assert_eq!(
        names,
        vec!["Renomeado", "Renomeado/Sub", "Renomeado/Sub/Folha"]
    );
    assert!(s.delete("Renomeado").await.is_err(), "has children");
    s.delete("Renomeado/Sub/Folha").await.unwrap();
    s.delete("Renomeado/Sub").await.unwrap();
    s.delete("Renomeado").await.unwrap();
    assert!(s.create("INBOX").await.is_err());
    s.subscribe("Archive").await.unwrap();
    s.unsubscribe("Archive").await.unwrap();

    s.logout().await.unwrap();
}

#[tokio::test]
async fn idle_and_unsolicited_updates() {
    let server = server().await;
    let mut watcher = imap_login(&server, ALICE, "alice").await;
    let mut other = imap_login(&server, ALICE, "alice").await;
    let exists = watcher.select("INBOX").await.unwrap().exists;
    other.select("INBOX").await.unwrap();

    // IDLE gets EXISTS when another connection appends.
    let mut idle = watcher.idle();
    idle.init().await.unwrap();
    let (wait, _stop) = idle.wait_with_timeout(Duration::from_secs(10));
    let appender = async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        other
            .append("INBOX", None, None, sample_message("idle wakeup"))
            .await
            .unwrap();
    };
    let (response, ()) = tokio::join!(wait, appender);
    let IdleResponse::NewData(data) = response.unwrap() else {
        panic!("expected new data");
    };
    assert!(
        matches!(data.parsed(), Response::MailboxData(MailboxDatum::Exists(n)) if *n == exists + 1)
    );
    let watcher = idle.done().await.unwrap();

    // IDLE also wakes up for library deliveries (as SMTP does).
    let mut idle = watcher.idle();
    idle.init().await.unwrap();
    let (wait, _stop) = idle.wait_with_timeout(Duration::from_secs(10));
    let deliver = async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        server.deliver(ALICE, "INBOX", sample_message("delivered").as_bytes(), &[]);
    };
    let (response, ()) = tokio::join!(wait, deliver);
    assert!(matches!(response.unwrap(), IdleResponse::NewData(_)));
    let mut watcher = idle.done().await.unwrap();
    watcher.noop().await.unwrap();

    // Flag changes and expunges by another connection show up at the next
    // command, with consistent sequence numbers.
    let mut uids: Vec<u32> = other.uid_search("ALL").await.unwrap().into_iter().collect();
    uids.sort_unstable();
    let _: Vec<Fetch> = other
        .uid_store(
            format!("{},{}", uids[2], uids[4]),
            "+FLAGS.SILENT (\\Deleted)",
        )
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let _: Vec<Fetch> = other
        .uid_store(uids[0].to_string(), "+FLAGS.SILENT (\\Flagged)")
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    other
        .expunge()
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();

    let (untagged, done) = raw_command!(watcher, "NOOP");
    assert!(done_ok(done.parsed()));
    let expunges: Vec<u32> = untagged
        .iter()
        .filter_map(|r| match r.parsed() {
            Response::Expunge(n) => Some(*n),
            _ => None,
        })
        .collect();
    assert_eq!(expunges, vec![3, 4], "seq 3 then old seq 5 shifted to 4");
    assert!(
        untagged
            .iter()
            .any(|r| matches!(r.parsed(), Response::Fetch(1, _)))
    );

    watcher.logout().await.unwrap();
    other.logout().await.unwrap();
}

#[tokio::test]
async fn raw_protocol_edge_cases() {
    let server = server().await;
    let mut c = Raw::connect(server.imap_addr()).await;
    assert!(c.line().await.starts_with("* OK [CAPABILITY IMAP4rev1"));

    // Synchronizing literal, then a non-synchronizing one.
    c.send("a1 LOGIN {18}\r\n").await;
    assert!(c.line().await.starts_with("+ "));
    c.send("alice@lowmail.test {5+}\r\nalice\r\n").await;
    assert!(c.line().await.starts_with("a1 OK"));

    // Pipelined commands, an unknown command, and a command split in pieces.
    c.send("a2 NOOP\r\na3 FROBNICATE\r\na4 SEL").await;
    c.send("ECT inbox\r\n").await;
    let lines = c.until("a4 ").await;
    assert!(lines.iter().any(|l| l.starts_with("a2 OK")));
    assert!(lines.iter().any(|l| l.starts_with("a3 BAD")));
    assert!(lines.last().unwrap().starts_with("a4 OK [READ-WRITE]"));
    assert!(lines.iter().any(|l| l.starts_with("* OK [UIDVALIDITY ")));

    // Malformed input never kills the connection.
    for bad in [
        "a5 FETCH\r\n",
        "a5 FETCH x:y FLAGS\r\n",
        "a5 UID FETCH 1 (BODY[9.9.9.HEADERX])\r\n",
        "a5 SEARCH (\r\n",
        "\"\r\n",
    ] {
        c.send(bad).await;
        let line = c.line().await;
        assert!(line.contains("BAD"), "{bad:?} -> {line}");
    }

    // Partial fetch.
    c.send("a6 FETCH 1 (BODY.PEEK[]<0.10>)\r\n").await;
    let line = c.line().await;
    assert!(line.starts_with("* 1 FETCH (BODY[]<0> {10}"), "{line}");
    c.bytes(10).await;
    assert_eq!(c.line().await, ")");
    assert!(c.line().await.starts_with("a6 OK"));

    // Missing parts are empty strings, not errors.
    c.send("a7 FETCH 1 (BODY.PEEK[7.3])\r\n").await;
    assert_eq!(c.line().await, "* 1 FETCH (BODY[7.3] {0}");
    assert_eq!(c.line().await, ")");
    assert!(c.line().await.starts_with("a7 OK"));

    // FETCH BODY (non-extensible structure) and ENVELOPE of a seeded message.
    c.send("a8 FETCH 1 (BODY ENVELOPE)\r\n").await;
    let line = c.line().await;
    assert!(line.starts_with("* 1 FETCH (BODY ("), "{line}");
    assert!(c.until("a8 ").await.last().unwrap().starts_with("a8 OK"));

    c.send("a9 NAMESPACE\r\n").await;
    assert_eq!(c.line().await, "* NAMESPACE ((\"\" \"/\")) NIL NIL");
    assert!(c.line().await.starts_with("a9 OK"));

    // Hostile inputs are rejected or answered quickly.
    c.send(&format!("a11 SEARCH {}ALL\r\n", "NOT ".repeat(10_000)))
        .await;
    assert!(c.line().await.starts_with("a11 BAD"));
    c.send(&format!("a12 LIST \"\" \"{}x\"\r\n", "*%".repeat(200)))
        .await;
    assert!(c.line().await.starts_with("a12 OK"));
    c.send("a13 APPEND INBOX (\\Seen) \"99-Foo-2025 99:99:99 +zz\u{e9}\" {1+}\r\nx\r\n")
        .await;
    assert!(c.line().await.starts_with("a13 BAD"));
    c.send("a14 FETCH 1 BODY[]<0.99999999999>\r\n").await;
    assert!(c.line().await.starts_with("a14 BAD"));
    c.send("a15 SEARCH SUBJECT {99999999999999999999}\r\n")
        .await;
    assert!(c.line().await.starts_with("a15 BAD"));

    c.send("a10 LOGOUT\r\n").await;
    assert!(c.line().await.starts_with("* BYE"));
    assert!(c.line().await.starts_with("a10 OK"));

    // AUTHENTICATE PLAIN with SASL-IR and with a continuation.
    let mut c = Raw::connect(server.imap_addr()).await;
    c.line().await;
    c.send(&format!(
        "b1 AUTHENTICATE PLAIN {}\r\n",
        plain_auth(ALICE, "alice")
    ))
    .await;
    assert!(c.line().await.starts_with("b1 OK"));

    let mut c = Raw::connect(server.imap_addr()).await;
    c.line().await;
    c.send("c1 AUTHENTICATE PLAIN\r\n").await;
    assert_eq!(c.line().await, "+ ");
    c.send(&format!("{}\r\n", plain_auth(BOB, "wrong"))).await;
    assert!(c.line().await.starts_with("c1 NO"));
    c.send("c2 AUTHENTICATE PLAIN\r\n").await;
    assert_eq!(c.line().await, "+ ");
    c.send(&format!("{}\r\n", plain_auth(BOB, "bob"))).await;
    assert!(c.line().await.starts_with("c2 OK"));
    c.send("c3 SELECT INBOX\r\n").await;
    let lines = c.until("c3 ").await;
    assert!(lines.iter().any(|l| l == "* 3 EXISTS"));
}

#[tokio::test]
async fn large_append_round_trip() {
    let server = server().await;
    let mut s = imap_login(&server, BOB, "bob").await;
    let line = "QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVphYmNkZWZnaGlqa2xtbm9wcXJzdHV2d3h5ejAxMjM0\r\n";
    let mut message = String::from(
        "From: Big <big@example.com>\r\nTo: bob@lowmail.test\r\nSubject: Large attachment\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"big\"\r\n\r\n--big\r\nContent-Type: text/plain\r\n\r\nSee attachment.\r\n--big\r\nContent-Type: application/octet-stream; name=\"big.bin\"\r\nContent-Transfer-Encoding: base64\r\n\r\n",
    );
    while message.len() < 30 * 1024 * 1024 {
        message.push_str(line);
    }
    message.push_str("--big--\r\n");

    s.select("INBOX").await.unwrap();
    s.append("INBOX", None, None, &message).await.unwrap();
    s.noop().await.unwrap();
    let uid = *s
        .uid_search("SUBJECT \"Large attachment\"")
        .await
        .unwrap()
        .iter()
        .next()
        .unwrap();
    let fetched: Vec<Fetch> = s
        .uid_fetch(uid.to_string(), "(RFC822.SIZE BODYSTRUCTURE BODY.PEEK[])")
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(fetched[0].size, Some(message.len() as u32));
    assert_eq!(fetched[0].body().map(<[u8]>::len), Some(message.len()));
    let Some(BodyStructure::Multipart { bodies, .. }) = fetched[0].bodystructure() else {
        panic!("multipart expected");
    };
    assert_eq!(bodies.len(), 2);
    s.logout().await.unwrap();
}
