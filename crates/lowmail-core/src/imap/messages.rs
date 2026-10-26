//! Listing, reading, flagging and moving messages.

use std::collections::HashMap;

use async_imap::imap_proto::{BodyStructure, MailboxDatum, Response};
use async_imap::types::Flag;
use futures::TryStreamExt;

use super::{Conn, build_criteria, quote, uid_set};
use crate::error::{Error, Result};
use crate::mime;
use crate::model::{Filter, Flags, MessageSummary, Sort, SortField};

/// Header fields the list needs; the rest of the message stays on the server.
const LIST_HEADERS: &str = "DATE FROM TO CC SUBJECT MESSAGE-ID IN-REPLY-TO REFERENCES X-PRIORITY IMPORTANCE PRIORITY LIST-UNSUBSCRIBE";
/// Most UIDs per command, to keep command lines reasonable.
const CHUNK: usize = 500;

impl Conn {
    /// UIDs of the selected folder matching `filter`, in `sort` order.
    pub async fn sorted_uids(&mut self, sort: &Sort, filter: &Filter) -> Result<Vec<u32>> {
        let literal_plus = self.caps.has("LITERAL+");
        let (criteria, utf8) = build_criteria(filter, literal_plus);
        if self.caps.has("SORT") {
            let key = match sort.field {
                SortField::Date => "DATE",
                SortField::Arrival => "ARRIVAL",
                SortField::From => "FROM",
                SortField::To => "TO",
                SortField::Subject => "SUBJECT",
                SortField::Size => "SIZE",
            };
            let program = if sort.descending { format!("REVERSE {key}") } else { key.to_string() };
            // Some servers only sort in US-ASCII; plain criteria are fine with it.
            let charsets: &[&str] = if utf8 { &["UTF-8"] } else { &["UTF-8", "US-ASCII"] };
            for charset in charsets {
                let mut uids: Vec<u32> = Vec::new();
                let result = self
                    .raw_each(&format!("UID SORT ({program}) {charset} {criteria}"), |resp| {
                        if let Response::MailboxData(MailboxDatum::Sort(list)) = resp {
                            uids.extend_from_slice(list);
                        }
                    })
                    .await;
                match result {
                    Ok(()) => return Ok(uids),
                    Err(e) if e.kind == crate::error::ErrorKind::Server => continue,
                    Err(e) => return Err(e),
                }
            }
            // SORT refused: search and order by arrival below.
        }
        let mut uids = self.search_uids(filter).await?;
        // Without SORT, UIDs follow arrival order, which is close to date order.
        uids.sort_unstable();
        if sort.descending {
            uids.reverse();
        }
        Ok(uids)
    }

    /// UIDs matching `filter`, unordered. Retries without `CHARSET UTF-8`
    /// for servers that refuse it (UTF-8 inside quotes still works on most).
    pub async fn search_uids(&mut self, filter: &Filter) -> Result<Vec<u32>> {
        let (criteria, utf8) = build_criteria(filter, self.caps.has("LITERAL+"));
        let attempts: &[&str] = if utf8 { &["CHARSET UTF-8 ", ""] } else { &[""] };
        let mut last = None;
        for charset in attempts {
            let mut uids = Vec::new();
            let result = self
                .raw_each(&format!("UID SEARCH {charset}{criteria}"), |resp| {
                    if let Response::MailboxData(MailboxDatum::Search(list)) = resp {
                        uids.extend_from_slice(list);
                    }
                })
                .await;
            match result {
                Ok(()) => return Ok(uids),
                Err(e) if e.kind == crate::error::ErrorKind::Server => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| crate::error::Error::server("The server refused the search.")))
    }

    /// Summaries of `uids` in the selected folder, in the order given.
    pub async fn fetch_summaries(&mut self, folder: &str, uids: &[u32]) -> Result<Vec<MessageSummary>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let query = format!("(UID FLAGS INTERNALDATE RFC822.SIZE BODYSTRUCTURE BODY.PEEK[HEADER.FIELDS ({LIST_HEADERS})])");
        let mut found: HashMap<u32, MessageSummary> = HashMap::new();
        for chunk in uids.chunks(CHUNK) {
            let fetches: Vec<_> = self.session.uid_fetch(uid_set(chunk), &query).await?.try_collect().await?;
            for fetch in &fetches {
                let Some(uid) = fetch.uid else { continue };
                let flags = flags_of(fetch.flags());
                let internal = fetch.internal_date().map(|d| d.timestamp_millis());
                let has_attachments = fetch.bodystructure().map(has_attachments).unwrap_or(false);
                let header = fetch.header().unwrap_or_default();
                let summary = mime::summary_from_headers(header, folder, uid, fetch.size.unwrap_or(0), internal, flags, has_attachments);
                found.insert(uid, summary);
            }
        }
        self.drain_unsolicited();
        Ok(uids.iter().filter_map(|u| found.remove(u)).collect())
    }

    /// The whole message, without marking it read.
    pub async fn fetch_raw(&mut self, uid: u32) -> Result<(Vec<u8>, Flags)> {
        let fetches: Vec<_> = self.session.uid_fetch(uid.to_string(), "(UID FLAGS BODY.PEEK[])").await?.try_collect().await?;
        self.drain_unsolicited();
        for fetch in &fetches {
            if fetch.uid == Some(uid)
                && let Some(body) = fetch.body()
            {
                return Ok((body.to_vec(), flags_of(fetch.flags())));
            }
        }
        Err(Error::not_found("The message no longer exists. It may have been moved or deleted."))
    }

    /// Adds and removes flags (`\Seen`, `$Forwarded`...) on `uids`.
    pub async fn store_flags(&mut self, uids: &[u32], add: &[&str], remove: &[&str]) -> Result<()> {
        for chunk in uids.chunks(CHUNK) {
            let set = uid_set(chunk);
            if !add.is_empty() {
                self.raw(&format!("UID STORE {set} +FLAGS.SILENT ({})", add.join(" "))).await?;
            }
            if !remove.is_empty() {
                self.raw(&format!("UID STORE {set} -FLAGS.SILENT ({})", remove.join(" "))).await?;
            }
        }
        self.drain_unsolicited();
        Ok(())
    }

    /// Flags every message of the selected folder.
    pub async fn store_flags_all(&mut self, add: &[&str]) -> Result<()> {
        if self.selected.as_ref().is_some_and(|s| s.exists == 0) {
            return Ok(());
        }
        self.raw(&format!("UID STORE 1:* +FLAGS.SILENT ({})", add.join(" "))).await?;
        self.drain_unsolicited();
        Ok(())
    }

    /// Runs a COPY or MOVE and returns the UIDs the messages got in the
    /// destination (`COPYUID`, UIDPLUS), when the server tells them.
    async fn transfer(&mut self, command: &str) -> Result<Vec<u32>> {
        use async_imap::imap_proto::ResponseCode;
        use async_imap::imap_proto::rfc4315::UidSetMember;
        let mut created = Vec::new();
        self.raw_each(command, |resp| {
            let outcome = match resp {
                Response::Data { outcome, .. } | Response::Done { outcome, .. } => outcome,
                _ => return,
            };
            if let Some(ResponseCode::CopyUid(_, _, dest)) = &outcome.code {
                for member in dest {
                    match member {
                        UidSetMember::Uid(u) => created.push(*u),
                        UidSetMember::UidRange(r) => created.extend(r.clone()),
                    }
                }
            }
        })
        .await?;
        Ok(created)
    }

    pub async fn copy_uids(&mut self, uids: &[u32], dest: &str) -> Result<Vec<u32>> {
        let mut created = Vec::new();
        for chunk in uids.chunks(CHUNK) {
            created.extend(self.transfer(&format!("UID COPY {} {}", uid_set(chunk), quote(dest))).await?);
        }
        Ok(created)
    }

    /// Moves `uids` to `dest`; returns their new UIDs there when known.
    pub async fn move_uids(&mut self, uids: &[u32], dest: &str) -> Result<Vec<u32>> {
        let mut created = Vec::new();
        if self.caps.has("MOVE") {
            for chunk in uids.chunks(CHUNK) {
                created.extend(self.transfer(&format!("UID MOVE {} {}", uid_set(chunk), quote(dest))).await?);
            }
        } else {
            created = self.copy_uids(uids, dest).await?;
            self.store_flags(uids, &["\\Deleted"], &[]).await?;
            self.expunge_uids(uids).await?;
        }
        self.drain_unsolicited();
        Ok(created)
    }

    /// Removes `uids` for good (they must already be flagged `\Deleted`).
    pub async fn expunge_uids(&mut self, uids: &[u32]) -> Result<()> {
        if self.caps.has("UIDPLUS") {
            for chunk in uids.chunks(CHUNK) {
                self.raw(&format!("UID EXPUNGE {}", uid_set(chunk))).await?;
            }
        } else {
            self.raw("EXPUNGE").await?;
        }
        self.drain_unsolicited();
        Ok(())
    }

    pub async fn delete_uids(&mut self, uids: &[u32]) -> Result<()> {
        self.store_flags(uids, &["\\Deleted"], &[]).await?;
        self.expunge_uids(uids).await
    }

    /// Empties the selected folder.
    pub async fn delete_all(&mut self) -> Result<()> {
        if self.selected.as_ref().is_some_and(|s| s.exists == 0) {
            return Ok(());
        }
        self.raw("UID STORE 1:* +FLAGS.SILENT (\\Deleted)").await?;
        self.raw("EXPUNGE").await?;
        self.drain_unsolicited();
        Ok(())
    }

    /// UIDs in the selected folder of messages with one of `ids` as their
    /// Message-ID or among their References (one conversation).
    pub async fn related_uids(&mut self, ids: &[String]) -> Result<Vec<u32>> {
        let keys: Vec<String> = ids
            .iter()
            .take(20)
            .map(|id| {
                let id = super::search::astring(&format!("<{}>", id.trim_matches(|c| c == '<' || c == '>')), false);
                format!("OR HEADER Message-ID {id} HEADER References {id}")
            })
            .collect();
        let Some((first, rest)) = keys.split_first() else { return Ok(Vec::new()) };
        // OR is binary: OR (OR a b) c ...
        let criteria = rest.iter().fold(first.clone(), |acc, k| format!("OR ({acc}) ({k})"));
        let mut uids = Vec::new();
        self.raw_each(&format!("UID SEARCH UNDELETED ({criteria})"), |resp| {
            if let Response::MailboxData(MailboxDatum::Search(list)) = resp {
                uids.extend_from_slice(list);
            }
        })
        .await?;
        Ok(uids)
    }

    /// UID of the message with `message_id` in the selected folder.
    pub async fn find_by_message_id(&mut self, message_id: &str) -> Result<Option<u32>> {
        let id = format!("<{}>", message_id.trim_matches(|c| c == '<' || c == '>'));
        let mut uids = Vec::new();
        self.raw_each(&format!("UID SEARCH HEADER Message-ID {}", super::search::astring(&id, false)), |resp| {
            if let Response::MailboxData(MailboxDatum::Search(list)) = resp {
                uids.extend_from_slice(list);
            }
        })
        .await?;
        Ok(uids.into_iter().max())
    }

    /// Usage and limit of the account's storage quota, in bytes.
    pub async fn quota(&mut self) -> Result<Option<(u64, u64)>> {
        if !self.caps.has("QUOTA") {
            return Ok(None);
        }
        let (_, quotas) = self.session.get_quota_root("INBOX").await?;
        for quota in quotas {
            for res in quota.resources {
                if res.name == async_imap::types::QuotaResourceName::Storage {
                    return Ok(Some((res.usage * 1024, res.limit * 1024)));
                }
            }
        }
        Ok(None)
    }
}

pub fn flags_of<'a>(flags: impl Iterator<Item = Flag<'a>>) -> Flags {
    let mut out = Flags::default();
    for flag in flags {
        match flag {
            Flag::Seen => out.seen = true,
            Flag::Answered => out.answered = true,
            Flag::Flagged => out.flagged = true,
            Flag::Deleted => out.deleted = true,
            Flag::Draft => out.draft = true,
            Flag::Custom(k) => {
                let lower = k.to_ascii_lowercase();
                match lower.as_str() {
                    "$forwarded" => out.forwarded = true,
                    "$junk" | "junk" => out.junk = true,
                    "$notjunk" | "notjunk" | "nonjunk" | "$mdnsent" | "\\recent" => {}
                    _ if !k.starts_with('\\') => out.keywords.push(k.to_string()),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

/// Whether the structure has parts people would call attachments: files,
/// not the body or images shown inside it.
pub fn has_attachments(structure: &BodyStructure<'_>) -> bool {
    fn walk(part: &BodyStructure<'_>, in_related: bool, top: bool) -> bool {
        match part {
            BodyStructure::Multipart { common, bodies, .. } => {
                let related = common.ty.subtype.eq_ignore_ascii_case("related");
                bodies.iter().enumerate().any(|(i, b)| walk(b, related || (in_related && i > 0), false))
            }
            BodyStructure::Message { .. } => true,
            BodyStructure::Text { common, .. } => {
                let attachment = common.disposition.as_ref().is_some_and(|d| d.ty.eq_ignore_ascii_case("attachment"));
                let named = common
                    .disposition
                    .as_ref()
                    .and_then(|d| d.params.as_ref())
                    .is_some_and(|p| p.iter().any(|(k, _)| k.eq_ignore_ascii_case("filename")));
                !top && (attachment || named)
            }
            BodyStructure::Basic { common, other, .. } => {
                let disposition = common.disposition.as_ref().map(|d| d.ty.to_ascii_lowercase());
                match disposition.as_deref() {
                    Some("attachment") => true,
                    Some("inline") if other.id.is_some() || in_related => false,
                    _ => !(in_related && other.id.is_some()),
                }
            }
        }
    }
    walk(structure, false, true)
}
