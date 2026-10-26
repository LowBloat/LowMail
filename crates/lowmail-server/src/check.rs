//! `lowmail check`: runs the main operations of the webmail against the
//! configured mail server with one account and prints a report. It changes
//! nothing that it does not undo: a temporary folder and a draft are created
//! and removed, filters are only read, and mail is sent only with `--send-to`.

use std::sync::Arc;
use std::time::Instant;

use lowmail_core::compose::QuoteStyle;
use lowmail_core::model::*;
use lowmail_core::service::MailClient;
use lowmail_core::store::Store;

use crate::config::Config;

struct Report {
    failures: usize,
    warnings: usize,
}

impl Report {
    fn ok(&self, step: &str, detail: impl AsRef<str>, started: Instant) {
        println!("  [ ok ] {step}: {} ({} ms)", detail.as_ref(), started.elapsed().as_millis());
    }

    fn warn(&mut self, step: &str, detail: impl AsRef<str>) {
        self.warnings += 1;
        println!("  [warn] {step}: {}", detail.as_ref());
    }

    fn fail(&mut self, step: &str, e: &lowmail_core::Error) {
        self.failures += 1;
        println!("  [FAIL] {step}: {} ({:?}){}", e.message, e.kind, e.detail.as_deref().map(|d| format!(" — {d}")).unwrap_or_default());
    }
}

fn endpoint(e: &Endpoint) -> String {
    format!("{}:{} {:?}{}", e.host, e.port, e.security, if e.accept_invalid_certs { " (any certificate)" } else { "" })
}

/// Returns true when every step passed.
pub async fn run(config: Config, email: &str, password: String, send_to: Option<String>) -> bool {
    let mut r = Report { failures: 0, warnings: 0 };
    let email = match config.normalize_login(email) {
        Ok(e) => e,
        Err(m) => {
            println!("  [FAIL] address: {m}");
            return false;
        }
    };
    let account = config.account_for(&email);
    println!("LowMail {} check for {email}", env!("CARGO_PKG_VERSION"));
    println!("  IMAP  {}", endpoint(&account.imap));
    println!("  SMTP  {}", endpoint(&account.smtp));
    println!("  Sieve {}", account.sieve.as_ref().map(endpoint).unwrap_or_else(|| "off".into()));
    println!("  login name: {}", account.username);
    println!();

    let store = Arc::new(Store::memory().expect("memory store"));
    let t = Instant::now();
    let client = match MailClient::connect(account.clone(), password.clone(), store, email.clone(), 2).await {
        Ok(c) => {
            r.ok("IMAP sign-in", "accepted", t);
            c
        }
        Err(e) => {
            r.fail("IMAP sign-in", &e);
            println!("\nStopped: nothing else can be checked without IMAP.");
            return false;
        }
    };
    let caps = client.info().capabilities;
    println!("         capabilities: {}", caps.join(" "));
    for wanted in ["IDLE", "SORT", "UIDPLUS", "MOVE", "SPECIAL-USE"] {
        if !caps.iter().any(|c| c == wanted) {
            r.warn("capabilities", format!("{wanted} missing (LowMail works without it, more slowly or with fallbacks)"));
        }
    }

    // Folders and their roles.
    let t = Instant::now();
    let folders = match client.folders(true).await {
        Ok(f) => {
            let roles: Vec<String> = f.iter().filter_map(|x| x.role.map(|r| format!("{r:?}={}", x.display_path))).collect();
            r.ok("folders", format!("{} folders; {}", f.len(), roles.join(", ")), t);
            for role in [FolderRole::Sent, FolderRole::Drafts, FolderRole::Trash, FolderRole::Junk] {
                if !f.iter().any(|x| x.role == Some(role)) {
                    r.warn("folders", format!("no {role:?} folder found; LowMail creates it when first needed"));
                }
            }
            f
        }
        Err(e) => {
            r.fail("folders", &e);
            Vec::new()
        }
    };
    let _ = folders;

    // The inbox list, the newest message and its conversation.
    let t = Instant::now();
    let mut newest: Option<MessageSummary> = None;
    match client.list("INBOX", 0, 50, &Sort::default(), &Filter::default()).await {
        Ok(page) => {
            r.ok("inbox list", format!("{} messages, first {} loaded", page.total, page.items.len()), t);
            newest = page.items.first().cloned();
        }
        Err(e) => r.fail("inbox list", &e),
    }
    if let Some(m) = &newest {
        let t = Instant::now();
        match client.message("INBOX", m.uid, false, false).await {
            Ok(full) => r.ok(
                "read newest message",
                format!(
                    "\"{}\", {} attachment(s), html: {}, remote content blocked: {}",
                    full.summary.subject.chars().take(60).collect::<String>(),
                    full.attachments.iter().filter(|a| !a.inline).count(),
                    full.html.is_some(),
                    full.remote_content
                ),
                t,
            ),
            Err(e) => r.fail("read newest message", &e),
        }
        let t = Instant::now();
        match client.thread("INBOX", m.uid).await {
            Ok(list) => r.ok("conversation", format!("{} related message(s)", list.len()), t),
            Err(e) => r.fail("conversation", &e),
        }
        // Search with a word of its subject, in the folder and everywhere.
        let word: String = m.subject.split_whitespace().find(|w| w.chars().count() > 3).unwrap_or("e").to_string();
        let filter = Filter { text: word.clone(), scope: SearchScope::Everything, ..Default::default() };
        let t = Instant::now();
        match client.list("INBOX", 0, 10, &Sort::default(), &filter).await {
            Ok(p) => r.ok("search", format!("\"{word}\": {} result(s)", p.total), t),
            Err(e) => r.fail("search", &e),
        }
        let t = Instant::now();
        match client.search_all(&Filter { text: word.clone(), ..Default::default() }).await {
            Ok(list) => r.ok("search in every folder", format!("{} result(s)", list.len()), t),
            Err(e) => r.fail("search in every folder", &e),
        }
    } else {
        r.warn("inbox", "empty: reading and search were not checked");
    }
    let t = Instant::now();
    match client
        .list("INBOX", 0, 10, &Sort { field: SortField::Subject, descending: false }, &Filter { unread: true, ..Default::default() })
        .await
    {
        Ok(p) => r.ok("sort by subject, unread only", format!("{} message(s)", p.total), t),
        Err(e) => r.fail("sort by subject, unread only", &e),
    }
    let t = Instant::now();
    match client.quota().await {
        Ok(Some(q)) => r.ok("quota", format!("{} MB of {} MB", q.used / 1_048_576, q.limit / 1_048_576), t),
        Ok(None) => r.ok("quota", "not reported by the server", t),
        Err(e) => r.fail("quota", &e),
    }

    // A temporary folder: create, rename, delete.
    let name = format!("LowMail check {}", rand::random::<u16>());
    let t = Instant::now();
    match client.create_folder(None, &name).await {
        Ok(path) => match client.rename_folder(&path, &format!("{name} renamed"), None).await {
            Ok(renamed) => match client.delete_folder(&renamed).await {
                Ok(()) => r.ok("folder create/rename/delete", format!("\"{name}\""), t),
                Err(e) => r.fail("folder delete (remove the test folder by hand)", &e),
            },
            Err(e) => {
                r.fail("folder rename", &e);
                let _ = client.delete_folder(&path).await;
            }
        },
        Err(e) => r.fail("folder create", &e),
    }

    // A draft: save, reopen, discard.
    let t = Instant::now();
    match client.prepare(ComposeMode::New, None, true, &QuoteStyle::default()).await {
        Ok(mut d) => {
            d.to = vec![Address::new(None, email.clone())];
            d.subject = "LowMail check draft (removed automatically)".into();
            d.html = Some("<p>Draft written by <b>lowmail check</b>.</p>".into());
            match client.save_draft(d).await {
                Ok(saved) => match saved.draft_ref.clone() {
                    Some(r#ref) => match client.prepare(ComposeMode::Draft, Some(r#ref.clone()), true, &QuoteStyle::default()).await {
                        Ok(reopened) if reopened.subject == saved.subject => match client.discard_draft(&r#ref).await {
                            Ok(()) => r.ok("draft save/reopen/discard", format!("in {}", r#ref.folder), t),
                            Err(e) => r.fail("draft discard (delete the test draft by hand)", &e),
                        },
                        Ok(_) => {
                            r.warn("draft", "reopened draft differs");
                            let _ = client.discard_draft(&r#ref).await;
                        }
                        Err(e) => {
                            r.fail("draft reopen", &e);
                            let _ = client.discard_draft(&r#ref).await;
                        }
                    },
                    None => r.warn("draft", "saved, but the server did not say where (no UIDPLUS and not found by Message-ID)"),
                },
                Err(e) => r.fail("draft save", &e),
            }
        }
        Err(e) => r.fail("draft", &e),
    }

    // Filters are only read.
    if account.sieve.is_some() {
        let t = Instant::now();
        match client.filters().await {
            Ok(state) => r.ok(
                "filters (ManageSieve)",
                format!(
                    "{} LowMail rule(s), vacation {}, other active script: {}",
                    state.filters.rules.len(),
                    if state.filters.vacation.enabled { "on" } else { "off" },
                    state.foreign_script.as_deref().unwrap_or("none")
                ),
                t,
            ),
            Err(e) => r.fail("filters (ManageSieve; set LOWMAIL_SIEVE=false if the server has none)", &e),
        }
    }

    // SMTP: sign-in always, a real message only when asked.
    let t = Instant::now();
    match lowmail_core::smtp::test(&account, &password).await {
        Ok(()) => r.ok("SMTP sign-in", "accepted", t),
        Err(e) => r.fail("SMTP sign-in", &e),
    }
    if let Some(to) = send_to {
        let t = Instant::now();
        match client.prepare(ComposeMode::New, None, true, &QuoteStyle::default()).await {
            Ok(mut d) => {
                d.to = vec![Address::new(None, to.clone())];
                d.subject = format!("LowMail check {}", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"));
                d.html = Some("<p>Test message sent by <b>lowmail check</b>. Acentuação: ação, café, você.</p>".into());
                match client.send(d, true).await {
                    Ok(res) if res.saved => r.ok("send", format!("to {to}, copy saved in Sent"), t),
                    Ok(res) => r.warn("send", format!("sent to {to}, but no copy in Sent: {:?}", res.warnings)),
                    Err(e) => r.fail("send", &e),
                }
            }
            Err(e) => r.fail("send", &e),
        }
    }

    client.close().await;
    println!();
    if r.failures == 0 {
        println!("All checks passed ({} warning(s)).", r.warnings);
    } else {
        println!("{} check(s) failed, {} warning(s). Run with LOWMAIL_LOG=debug for the protocol details.", r.failures, r.warnings);
    }
    r.failures == 0
}
