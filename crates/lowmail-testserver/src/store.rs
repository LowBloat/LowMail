//! Shared in-memory state: accounts, mailboxes, messages and sieve scripts.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, FixedOffset};
use parking_lot::{Mutex, MutexGuard};
use tokio::sync::watch;

use crate::OutboundMail;
use crate::message::MessageData;

/// Hierarchy delimiter for mailbox names.
pub(crate) const DELIMITER: char = '/';

/// The five system flags that can be stored (`\Recent` is session state).
pub(crate) const SYSTEM_FLAGS: [&str; 5] = ["\\Answered", "\\Flagged", "\\Deleted", "\\Seen", "\\Draft"];

/// RFC 6154 special-use attributes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpecialUse {
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
}

impl SpecialUse {
    pub(crate) fn attribute(self) -> &'static str {
        match self {
            SpecialUse::Sent => "\\Sent",
            SpecialUse::Drafts => "\\Drafts",
            SpecialUse::Trash => "\\Trash",
            SpecialUse::Junk => "\\Junk",
            SpecialUse::Archive => "\\Archive",
        }
    }
}

/// A stored message.
#[derive(Clone)]
pub(crate) struct Message {
    pub uid: u32,
    pub flags: Vec<String>,
    pub internal_date: DateTime<FixedOffset>,
    pub data: Arc<MessageData>,
    /// Not yet claimed by a session that selected the mailbox.
    pub recent: bool,
}

impl Message {
    pub(crate) fn has_flag(&self, flag: &str) -> bool {
        has_flag(&self.flags, flag)
    }

    pub(crate) fn size(&self) -> usize {
        self.data.raw.len()
    }
}

pub(crate) fn has_flag(flags: &[String], flag: &str) -> bool {
    flags.iter().any(|f| f.eq_ignore_ascii_case(flag))
}

/// Adds `flag` unless present; returns whether the set changed.
pub(crate) fn add_flag(flags: &mut Vec<String>, flag: &str) -> bool {
    if has_flag(flags, flag) {
        false
    } else {
        flags.push(flag.to_owned());
        true
    }
}

/// Removes `flag`; returns whether the set changed.
pub(crate) fn remove_flag(flags: &mut Vec<String>, flag: &str) -> bool {
    let before = flags.len();
    flags.retain(|f| !f.eq_ignore_ascii_case(flag));
    flags.len() != before
}

/// Canonical spelling of system flags (`\seen` -> `\Seen`); keywords are kept.
pub(crate) fn canonical_flag(flag: &str) -> String {
    SYSTEM_FLAGS
        .iter()
        .chain(std::iter::once(&"\\Recent"))
        .find(|f| f.eq_ignore_ascii_case(flag))
        .map(|f| (*f).to_owned())
        .unwrap_or_else(|| flag.to_owned())
}

/// Normalizes a mailbox name: `INBOX` is case-insensitive, trailing
/// delimiters are dropped.
pub(crate) fn normalize_mailbox(name: &str) -> String {
    let name = name.trim_end_matches(DELIMITER);
    let (head, tail) = match name.find(DELIMITER) {
        Some(i) => name.split_at(i),
        None => (name, ""),
    };
    if head.eq_ignore_ascii_case("INBOX") { format!("INBOX{tail}") } else { name.to_owned() }
}

/// A mailbox and its messages, ordered by UID.
pub(crate) struct Mailbox {
    pub name: String,
    pub uid_validity: u32,
    pub uid_next: u32,
    pub subscribed: bool,
    pub special_use: Option<SpecialUse>,
    pub messages: Vec<Message>,
}

impl Mailbox {
    fn new(name: String, uid_validity: u32, special_use: Option<SpecialUse>) -> Self {
        Self { name, uid_validity, uid_next: 1, subscribed: true, special_use, messages: Vec::new() }
    }

    pub(crate) fn position(&self, uid: u32) -> Option<usize> {
        self.messages.binary_search_by_key(&uid, |m| m.uid).ok()
    }

    pub(crate) fn find(&self, uid: u32) -> Option<&Message> {
        self.position(uid).map(|i| &self.messages[i])
    }

    pub(crate) fn find_mut(&mut self, uid: u32) -> Option<&mut Message> {
        self.position(uid).map(|i| &mut self.messages[i])
    }

    /// Appends a message (marked recent) and returns its UID.
    pub(crate) fn append(&mut self, data: Arc<MessageData>, flags: Vec<String>, internal_date: DateTime<FixedOffset>) -> u32 {
        let uid = self.uid_next;
        self.uid_next += 1;
        let mut clean = Vec::new();
        for flag in flags {
            if !flag.eq_ignore_ascii_case("\\Recent") {
                add_flag(&mut clean, &canonical_flag(&flag));
            }
        }
        self.messages.push(Message { uid, flags: clean, internal_date, data, recent: true });
        uid
    }

    pub(crate) fn unseen(&self) -> usize {
        self.messages.iter().filter(|m| !m.has_flag("\\Seen")).count()
    }

    pub(crate) fn recent(&self) -> usize {
        self.messages.iter().filter(|m| m.recent).count()
    }

    pub(crate) fn size(&self) -> usize {
        self.messages.iter().map(Message::size).sum()
    }
}

/// A ManageSieve script.
pub(crate) struct SieveScript {
    pub name: String,
    pub content: String,
    pub active: bool,
}

/// A user account.
pub(crate) struct Account {
    pub password: String,
    pub mailboxes: BTreeMap<String, Mailbox>,
    pub scripts: Vec<SieveScript>,
}

impl Account {
    /// Whether any mailbox lives below `name`.
    pub(crate) fn has_children(&self, name: &str) -> bool {
        let prefix = format!("{name}{DELIMITER}");
        self.mailboxes.keys().any(|k| k.starts_with(&prefix))
    }
}

/// Everything behind the store lock.
pub(crate) struct Inner {
    accounts: BTreeMap<String, Account>,
    pub outbound: Vec<OutboundMail>,
    next_validity: u32,
}

/// Mailbox-level failures surfaced to IMAP clients.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MailboxError {
    Exists,
    Missing,
    Invalid,
}

impl Inner {
    pub(crate) fn account(&self, user: &str) -> Option<&Account> {
        self.accounts.get(&user.to_ascii_lowercase())
    }

    pub(crate) fn account_mut(&mut self, user: &str) -> Option<&mut Account> {
        self.accounts.get_mut(&user.to_ascii_lowercase())
    }

    pub(crate) fn mailbox(&self, user: &str, name: &str) -> Option<&Mailbox> {
        self.account(user)?.mailboxes.get(name)
    }

    pub(crate) fn mailbox_mut(&mut self, user: &str, name: &str) -> Option<&mut Mailbox> {
        self.account_mut(user)?.mailboxes.get_mut(name)
    }

    pub(crate) fn new_uid_validity(&mut self) -> u32 {
        self.next_validity += 1;
        self.next_validity
    }

    /// Creates `name` (already normalized) and any missing parents.
    pub(crate) fn create_mailbox(&mut self, user: &str, name: &str, special_use: Option<SpecialUse>) -> Result<(), MailboxError> {
        if name.is_empty() || name.split(DELIMITER).any(str::is_empty) {
            return Err(MailboxError::Invalid);
        }
        let mut path = String::new();
        let segments: Vec<&str> = name.split(DELIMITER).collect();
        for (i, segment) in segments.iter().enumerate() {
            if i > 0 {
                path.push(DELIMITER);
            }
            path.push_str(segment);
            let last = i + 1 == segments.len();
            let exists = self.mailbox(user, &path).is_some();
            if exists {
                if last {
                    return Err(MailboxError::Exists);
                }
                continue;
            }
            let validity = self.new_uid_validity();
            let account = self.account_mut(user).ok_or(MailboxError::Missing)?;
            let special = if last { special_use } else { None };
            account.mailboxes.insert(path.clone(), Mailbox::new(path.clone(), validity, special));
        }
        Ok(())
    }

    /// Total storage used by `user`, in bytes.
    pub(crate) fn usage(&self, user: &str) -> usize {
        self.account(user).map(|a| a.mailboxes.values().map(Mailbox::size).sum()).unwrap_or(0)
    }
}

/// Thread-safe store plus a change counter that wakes IMAP sessions.
pub(crate) struct Store {
    inner: Mutex<Inner>,
    changes: watch::Sender<u64>,
}

impl Store {
    pub(crate) fn new(users: &[(String, String)]) -> Self {
        let base = (chrono::Utc::now().timestamp() as u32) & 0x3fff_ffff;
        let mut inner = Inner { accounts: BTreeMap::new(), outbound: Vec::new(), next_validity: base };
        for (email, password) in users {
            inner.accounts.insert(
                email.to_ascii_lowercase(),
                Account { password: password.clone(), mailboxes: BTreeMap::new(), scripts: Vec::new() },
            );
            let defaults = [
                ("INBOX", None),
                ("Sent", Some(SpecialUse::Sent)),
                ("Drafts", Some(SpecialUse::Drafts)),
                ("Trash", Some(SpecialUse::Trash)),
                ("Junk", Some(SpecialUse::Junk)),
                ("Archive", Some(SpecialUse::Archive)),
            ];
            for (name, special) in defaults {
                inner.create_mailbox(email, name, special).expect("default mailbox");
            }
        }
        let (changes, _) = watch::channel(0);
        Self { inner: Mutex::new(inner), changes }
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock()
    }

    /// Wakes every session waiting for mailbox changes.
    pub(crate) fn notify(&self) {
        self.changes.send_modify(|v| *v = v.wrapping_add(1));
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Checks credentials and returns the canonical (lowercase) login.
    pub(crate) fn authenticate(&self, user: &str, password: &str) -> Option<String> {
        let inner = self.lock();
        let account = inner.account(user)?;
        (account.password == password).then(|| user.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailbox_names() {
        assert_eq!(normalize_mailbox("inbox"), "INBOX");
        assert_eq!(normalize_mailbox("Inbox/Sub"), "INBOX/Sub");
        assert_eq!(normalize_mailbox("Projetos/"), "Projetos");
        assert_eq!(normalize_mailbox("INBOXES"), "INBOXES");
    }

    #[test]
    fn flags() {
        assert_eq!(canonical_flag("\\seen"), "\\Seen");
        assert_eq!(canonical_flag("$Forwarded"), "$Forwarded");
        let mut f = vec![];
        assert!(add_flag(&mut f, "\\Seen"));
        assert!(!add_flag(&mut f, "\\SEEN"));
        assert!(remove_flag(&mut f, "\\seen"));
    }

    #[test]
    fn create_with_parents() {
        let store = Store::new(&[("a@x".into(), "p".into())]);
        let mut inner = store.lock();
        inner.create_mailbox("a@x", "A/B/C", None).unwrap();
        assert!(inner.mailbox("a@x", "A").is_some());
        assert!(inner.mailbox("a@x", "A/B").is_some());
        assert_eq!(inner.create_mailbox("a@x", "A/B", None), Err(MailboxError::Exists));
        assert_eq!(inner.create_mailbox("a@x", "A//B", None), Err(MailboxError::Invalid));
    }
}
