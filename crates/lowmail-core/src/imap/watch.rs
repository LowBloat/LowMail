//! Watches the inbox with IDLE (or polling when the server has no IDLE) on a
//! connection of its own, and refreshes the counters of the other folders
//! every few minutes.

use std::collections::HashMap;
use std::sync::Weak;
use std::time::{Duration, Instant};

use async_imap::imap_proto::{MailboxDatum, Response};
use tokio::sync::watch;

use super::Conn;
use crate::error::Result;
use crate::model::{FolderCount, MailEvent};
use crate::service::MailClient;

const IDLE_FOR: Duration = Duration::from_secs(4 * 60);
const POLL_EVERY: Duration = Duration::from_secs(60);
const COUNTS_EVERY: Duration = Duration::from_secs(4 * 60);
const MAX_BACKOFF: Duration = Duration::from_secs(120);

pub async fn run(client: Weak<MailClient>, mut stop: watch::Receiver<bool>) {
    let mut backoff = Duration::from_secs(5);
    let mut online = true;
    let mut counts: HashMap<String, (u32, u32)> = HashMap::new();
    loop {
        if *stop.borrow() {
            return;
        }
        let Some(c) = client.upgrade() else { return };
        let (account, password) = (c.account().clone(), c.password().to_string());
        drop(c);
        match Conn::open(&account, &password).await {
            Ok(conn) => {
                if !online {
                    emit(&client, MailEvent::Connection { online: true });
                    online = true;
                }
                backoff = Duration::from_secs(5);
                match watch_inbox(conn, &client, &mut stop, &mut counts).await {
                    Ok(()) => return,
                    Err(e) => tracing::debug!("inbox watch interrupted: {e}"),
                }
            }
            Err(e) => {
                tracing::debug!("inbox watch could not connect: {e}");
                if online {
                    emit(&client, MailEvent::Connection { online: false });
                    online = false;
                }
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = stop.changed() => return,
                }
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }
}

fn emit(client: &Weak<MailClient>, event: MailEvent) {
    if let Some(c) = client.upgrade() {
        c.emit(event);
    }
}

/// Ok when asked to stop; Err when the connection must be reopened.
async fn watch_inbox(
    mut conn: Conn,
    client: &Weak<MailClient>,
    stop: &mut watch::Receiver<bool>,
    counts: &mut HashMap<String, (u32, u32)>,
) -> Result<()> {
    let sel = conn.select("INBOX").await?;
    let mut uid_next = sel.uid_next;
    let mut last_counts = Instant::now() - COUNTS_EVERY;
    loop {
        if last_counts.elapsed() >= COUNTS_EVERY {
            refresh_counts(&mut conn, client, counts).await;
            last_counts = Instant::now();
        }
        let changed = if conn.caps.has("IDLE") {
            let Conn { session, caps, selected, last_used } = conn;
            let mut handle = session.idle();
            handle.init().await?;
            let outcome = {
                let (wait, _interrupt) = handle.wait_with_timeout(IDLE_FOR);
                tokio::select! {
                    r = wait => Some(r),
                    _ = stop.changed() => None,
                }
            };
            let session = handle.done().await?;
            conn = Conn { session, caps, selected, last_used };
            match outcome {
                None => {
                    conn.logout().await;
                    return Ok(());
                }
                Some(Err(e)) => return Err(e.into()),
                Some(Ok(async_imap::extensions::idle::IdleResponse::NewData(_))) => true,
                Some(Ok(_)) => false,
            }
        } else {
            tokio::select! {
                _ = tokio::time::sleep(POLL_EVERY) => {}
                _ = stop.changed() => {
                    conn.logout().await;
                    return Ok(());
                }
            }
            conn.session.noop().await?;
            true
        };
        conn.drain_unsolicited();
        if *stop.borrow() {
            conn.logout().await;
            return Ok(());
        }
        if client.upgrade().is_none() {
            return Ok(());
        }
        // New messages: UIDs at or above the last known UIDNEXT.
        let mut new_uids: Vec<u32> = Vec::new();
        conn.raw_each(&format!("UID SEARCH UID {}:*", uid_next.max(1)), |resp| {
            if let Response::MailboxData(MailboxDatum::Search(list)) = resp {
                new_uids.extend(list.iter().copied());
            }
        })
        .await?;
        new_uids.retain(|&u| u >= uid_next);
        if !new_uids.is_empty() {
            uid_next = new_uids.iter().max().copied().unwrap_or(uid_next) + 1;
            let mut unseen: Vec<u32> = Vec::new();
            conn.raw_each("UID SEARCH UNSEEN UNDELETED", |resp| {
                if let Response::MailboxData(MailboxDatum::Search(list)) = resp {
                    unseen.extend(list.iter().copied());
                }
            })
            .await?;
            let mut fresh: Vec<u32> = new_uids.iter().copied().filter(|u| unseen.contains(u)).collect();
            fresh.sort_unstable_by(|a, b| b.cmp(a));
            fresh.truncate(5);
            let latest = conn.fetch_summaries("INBOX", &fresh).await?;
            emit(client, MailEvent::NewMail { folder: "INBOX".into(), unread: unseen.len() as u32, latest });
        } else if changed {
            emit(client, MailEvent::FolderChanged { folder: "INBOX".into() });
        }
    }
}

async fn refresh_counts(conn: &mut Conn, client: &Weak<MailClient>, known: &mut HashMap<String, (u32, u32)>) {
    let Some(c) = client.upgrade() else { return };
    let paths: Vec<String> = c
        .cached_folders()
        .map(|list| list.folders.iter().filter(|f| f.selectable && f.subscribed && f.path != "INBOX").map(|f| f.path.clone()).collect())
        .unwrap_or_default();
    drop(c);
    let mut changed: Vec<FolderCount> = Vec::new();
    for path in paths {
        if let Ok(count) = conn.status_counts(&path).await
            && known.insert(path.clone(), count) != Some(count)
        {
            changed.push(FolderCount { path, total: count.0, unread: count.1 });
        }
    }
    if !changed.is_empty() {
        emit(client, MailEvent::Counts { folders: changed });
    }
}
