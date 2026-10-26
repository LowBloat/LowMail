//! One IMAP connection: command loop, state machine and mailbox sync.

use std::collections::HashSet;
use std::io;
use std::sync::Arc;

use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::watch;

use super::CAPABILITIES;
use super::fetch::{self, FetchAtt, ItemCtx};
use super::parser::{ParseError, Parser, SeqSet};
use super::search::{self, Bounds, Candidate, SearchError};
use super::sort;
use super::wire;
use crate::io::{LineReader, literal_marker, trim_eol};
use crate::message::MessageData;
use crate::sasl;
use crate::store::{
    self, DELIMITER, MailboxError, SYSTEM_FLAGS, Store, add_flag, canonical_flag, has_flag,
    normalize_mailbox, remove_flag,
};

/// Longest command line accepted (literals excluded).
const MAX_LINE: usize = 8 << 20;
/// Largest literal / total command size accepted.
const MAX_LITERAL: usize = 64 << 20;
/// Quota limit reported for STORAGE, in KiB.
const QUOTA_LIMIT_KB: usize = 102_400;
/// Keywords always offered in FLAGS.
const DEFAULT_KEYWORDS: [&str; 4] = ["$Forwarded", "$Junk", "$NotJunk", "$MDNSent"];

enum Reply {
    Ok(String),
    Logout,
}

enum Failure {
    No(String),
    Bad(String),
}

impl From<ParseError> for Failure {
    fn from(e: ParseError) -> Self {
        Failure::Bad(e.0)
    }
}

type CmdResult = Result<Reply, Failure>;

fn ok(text: impl Into<String>) -> CmdResult {
    Ok(Reply::Ok(text.into()))
}

fn no<T>(text: impl Into<String>) -> Result<T, Failure> {
    Err(Failure::No(text.into()))
}

fn bad<T>(text: impl Into<String>) -> Result<T, Failure> {
    Err(Failure::Bad(text.into()))
}

fn search_failure(e: SearchError) -> Failure {
    match e {
        SearchError::BadCharset => {
            Failure::No("[BADCHARSET (UTF-8 US-ASCII)] Unsupported charset".into())
        }
        SearchError::Parse(e) => Failure::Bad(e.0),
    }
}

/// What the client has been told about one message.
struct Known {
    uid: u32,
    flags: Vec<String>,
}

/// Per-connection view of the selected mailbox. Sequence numbers are
/// indexes into `known`, which only changes when the client is told.
struct Selected {
    name: String,
    uid_validity: u32,
    read_only: bool,
    known: Vec<Known>,
    max_uid: u32,
    recent: HashSet<u32>,
}

impl Selected {
    /// `(seq, uid)` pairs addressed by a sequence or UID set.
    fn targets(&self, set: &SeqSet, uid: bool) -> Vec<(u32, u32)> {
        let max = if uid {
            self.known.last().map_or(0, |k| k.uid)
        } else {
            self.known.len() as u32
        };
        self.known
            .iter()
            .enumerate()
            .map(|(i, k)| (i as u32 + 1, k.uid))
            .filter(|&(seq, u)| set.contains(if uid { u } else { seq }, max))
            .collect()
    }
}

fn same_flags(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().all(|f| has_flag(b, f))
}

/// `*`/`%` mailbox pattern matching.
///
/// Dynamic programming over name prefixes, so hostile patterns such as
/// `*%*%*%...` stay linear in the pattern length.
fn wildcard_match(pattern: &[u8], name: &[u8]) -> bool {
    let n = name.len();
    // reach[j]: the pattern consumed so far can match `name[..j]`.
    let mut reach = vec![false; n + 1];
    reach[0] = true;
    for &pc in pattern {
        let mut next = vec![false; n + 1];
        match pc {
            b'*' | b'%' => {
                let mut open = false;
                for j in 0..=n {
                    open |= reach[j];
                    next[j] = open;
                    if pc == b'%' && j < n && name[j] == DELIMITER as u8 {
                        open = false;
                    }
                }
            }
            c => {
                for j in 1..=n {
                    next[j] = reach[j - 1] && name[j - 1] == c;
                }
            }
        }
        reach = next;
    }
    reach[n]
}

/// Makes a leading `inbox` in a LIST pattern match the canonical `INBOX`.
fn normalize_pattern(pattern: &str) -> String {
    let b = pattern.as_bytes();
    if b.len() >= 5
        && b[..5].eq_ignore_ascii_case(b"INBOX")
        && (b.len() == 5 || b[5] == DELIMITER as u8)
    {
        format!("INBOX{}", &pattern[5..])
    } else {
        pattern.to_owned()
    }
}

/// Validates and canonicalizes client-supplied flags (`\Recent` dropped).
fn client_flags(flags: Vec<String>) -> Result<Vec<String>, Failure> {
    let mut out = Vec::new();
    for flag in flags {
        let flag = canonical_flag(&flag);
        if flag == "\\Recent" {
            continue;
        }
        if flag.starts_with('\\') && !SYSTEM_FLAGS.contains(&flag.as_str()) {
            return bad(format!("Invalid flag {flag}"));
        }
        add_flag(&mut out, &flag);
    }
    Ok(out)
}

enum Incoming {
    Command(Vec<u8>),
    TooBig(Vec<u8>),
    Eof,
}

enum IdleEvent {
    Line(io::Result<Option<Vec<u8>>>),
    Changed(bool),
}

pub(crate) struct Connection {
    reader: LineReader<OwnedReadHalf>,
    writer: BufWriter<OwnedWriteHalf>,
    store: Arc<Store>,
    changes: watch::Receiver<u64>,
    user: Option<String>,
    selected: Option<Selected>,
    out: Vec<u8>,
}

impl Connection {
    pub(crate) fn new(stream: TcpStream, store: Arc<Store>) -> Self {
        let (read, write) = stream.into_split();
        let changes = store.subscribe();
        Self {
            reader: LineReader::new(read),
            writer: BufWriter::with_capacity(64 * 1024, write),
            store,
            changes,
            user: None,
            selected: None,
            out: Vec::new(),
        }
    }

    pub(crate) async fn run(mut self) {
        self.out.extend_from_slice(
            format!("* OK [CAPABILITY {CAPABILITIES}] LowMail test IMAP server ready\r\n")
                .as_bytes(),
        );
        if self.flush().await.is_err() {
            return;
        }
        loop {
            let incoming = match self.read_command().await {
                Ok(incoming) => incoming,
                Err(err) => {
                    if err.kind() == io::ErrorKind::InvalidData {
                        self.out.extend_from_slice(b"* BYE Command too long\r\n");
                        let _ = self.flush().await;
                    }
                    break;
                }
            };
            let close = match incoming {
                Incoming::Eof => break,
                Incoming::TooBig(line) => {
                    let tag = Parser::new(&line).tag().unwrap_or_else(|_| "*".into());
                    self.respond(&tag, "BAD", "Literal too large");
                    false
                }
                Incoming::Command(cmd) => self.handle(&cmd).await,
            };
            if self.flush().await.is_err() || close {
                break;
            }
        }
        tracing::debug!(user = ?self.user, "imap connection closed");
    }

    async fn flush(&mut self) -> io::Result<()> {
        if !self.out.is_empty() {
            self.writer.write_all(&self.out).await?;
            self.out.clear();
        }
        self.writer.flush().await
    }

    /// Reads a full command, inlining literals (sending `+` for
    /// synchronizing ones).
    async fn read_command(&mut self) -> io::Result<Incoming> {
        let mut cmd = Vec::new();
        loop {
            let Some(line) = self.reader.read_line(MAX_LINE).await? else {
                return if cmd.is_empty() {
                    Ok(Incoming::Eof)
                } else {
                    Err(io::ErrorKind::UnexpectedEof.into())
                };
            };
            let content = trim_eol(&line);
            if cmd.is_empty() && content.is_empty() {
                continue;
            }
            let marker = literal_marker(content);
            cmd.extend_from_slice(&line);
            let Some((n, sync)) = marker else {
                return Ok(Incoming::Command(cmd));
            };
            if n.saturating_add(cmd.len()) > MAX_LITERAL {
                return if sync {
                    Ok(Incoming::TooBig(cmd))
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "literal too large",
                    ))
                };
            }
            if sync {
                self.out.extend_from_slice(b"+ Ready for literal data\r\n");
                self.flush().await?;
            }
            let data = self.reader.read_exact(n).await?;
            cmd.extend_from_slice(&data);
        }
    }

    fn untagged(&mut self, text: &str) {
        self.out.extend_from_slice(b"* ");
        self.out.extend_from_slice(text.as_bytes());
        self.out.extend_from_slice(b"\r\n");
    }

    fn respond(&mut self, tag: &str, status: &str, text: &str) {
        self.out
            .extend_from_slice(format!("{tag} {status} {text}\r\n").as_bytes());
    }

    /// Runs one command; returns `true` when the connection must close.
    async fn handle(&mut self, cmd: &[u8]) -> bool {
        let mut p = Parser::new(trim_eol(cmd));
        let Ok(tag) = p.tag() else {
            self.untagged("BAD Missing command tag");
            return false;
        };
        let Ok(mut command) = p.sp().and_then(|_| p.atom()) else {
            self.respond(&tag, "BAD", "Missing command");
            return false;
        };
        command.make_ascii_uppercase();
        let mut uid = false;
        if command == "UID" {
            uid = true;
            match p.sp().and_then(|_| p.atom()) {
                Ok(c) => command = c.to_ascii_uppercase(),
                Err(_) => {
                    self.respond(&tag, "BAD", "Missing UID command");
                    return false;
                }
            }
        }
        tracing::debug!(user = ?self.user, %tag, %command, uid, "imap command");

        let result = self.dispatch(&command, uid, &mut p).await;
        // Expunges must not be announced during non-UID FETCH/STORE/SEARCH
        // (RFC 3501 §7.4.1), since that would shift sequence numbers.
        let allow_expunge = uid
            || !matches!(
                command.as_str(),
                "FETCH" | "STORE" | "SEARCH" | "SORT" | "COPY"
            );
        match result {
            Ok(Reply::Ok(text)) => {
                self.sync(allow_expunge);
                self.respond(&tag, "OK", &text);
                false
            }
            Ok(Reply::Logout) => {
                self.untagged("BYE LowMail test server logging out");
                self.respond(&tag, "OK", "LOGOUT completed");
                true
            }
            Err(Failure::No(text)) => {
                self.sync(allow_expunge);
                self.respond(&tag, "NO", &text);
                false
            }
            Err(Failure::Bad(text)) => {
                self.respond(&tag, "BAD", &text);
                false
            }
        }
    }

    async fn dispatch(&mut self, command: &str, uid: bool, p: &mut Parser<'_>) -> CmdResult {
        if uid
            && !matches!(
                command,
                "FETCH" | "STORE" | "COPY" | "MOVE" | "SEARCH" | "SORT" | "EXPUNGE"
            )
        {
            return bad("Unknown UID command");
        }
        match command {
            "CAPABILITY" => {
                p.end()?;
                self.untagged(&format!("CAPABILITY {CAPABILITIES}"));
                ok("CAPABILITY completed")
            }
            "NOOP" => ok("NOOP completed"),
            "LOGOUT" => Ok(Reply::Logout),
            "ID" => self.id(p),
            "LOGIN" => self.login(p),
            "AUTHENTICATE" => self.authenticate(p).await,
            "STARTTLS" => bad("STARTTLS is not supported by this test server"),
            "ENABLE" => {
                self.untagged("ENABLED");
                ok("ENABLE completed")
            }
            _ => {
                let Some(user) = self.user.clone() else {
                    return bad("Not authenticated or unknown command");
                };
                self.authenticated(command, uid, p, &user).await
            }
        }
    }

    async fn authenticated(
        &mut self,
        command: &str,
        uid: bool,
        p: &mut Parser<'_>,
        user: &str,
    ) -> CmdResult {
        match command {
            "SELECT" => self.select(p, user, false),
            "EXAMINE" => self.select(p, user, true),
            "CREATE" => self.create(p, user),
            "DELETE" => self.delete(p, user),
            "RENAME" => self.rename(p, user),
            "SUBSCRIBE" => self.subscribe(p, user, true),
            "UNSUBSCRIBE" => self.subscribe(p, user, false),
            "LIST" => self.list(p, user, false),
            "LSUB" => self.list(p, user, true),
            "STATUS" => self.status(p, user),
            "APPEND" => self.append(p, user),
            "NAMESPACE" => {
                p.end()?;
                self.untagged("NAMESPACE ((\"\" \"/\")) NIL NIL");
                ok("NAMESPACE completed")
            }
            "GETQUOTAROOT" => self.quota_root(p, user),
            "GETQUOTA" => self.quota(p, user),
            "SETQUOTA" => no("Quota cannot be changed"),
            "IDLE" => self.idle(p).await,
            "CHECK" | "CLOSE" | "UNSELECT" | "EXPUNGE" | "SEARCH" | "SORT" | "FETCH" | "STORE"
            | "COPY" | "MOVE" => {
                if self.selected.is_none() {
                    return bad("No mailbox selected");
                }
                match command {
                    "CHECK" => ok("CHECK completed"),
                    "CLOSE" => self.close(user, true),
                    "UNSELECT" => self.close(user, false),
                    "EXPUNGE" => self.expunge(p, user, uid),
                    "SEARCH" => self.search(p, user, uid),
                    "SORT" => self.sort(p, user, uid),
                    "FETCH" => self.fetch(p, user, uid),
                    "STORE" => self.store_flags(p, user, uid),
                    "COPY" => self.copy(p, user, uid, false),
                    _ => self.copy(p, user, uid, true),
                }
            }
            _ => bad("Unknown command"),
        }
    }

    // -- unsolicited updates ------------------------------------------------

    /// Tells the client about changes made by other connections.
    fn sync(&mut self, allow_expunge: bool) {
        let Some(user) = self.user.clone() else {
            return;
        };
        let Some(sel) = self.selected.as_mut() else {
            return;
        };
        let store = self.store.clone();
        let mut inner = store.lock();
        let mbox = inner
            .mailbox_mut(&user, &sel.name)
            .filter(|m| m.uid_validity == sel.uid_validity);
        let Some(mbox) = mbox else {
            // Deleted or replaced: everything is gone for this client.
            if allow_expunge && !sel.known.is_empty() {
                for i in (1..=sel.known.len()).rev() {
                    self.out
                        .extend_from_slice(format!("* {i} EXPUNGE\r\n").as_bytes());
                }
                sel.known.clear();
                sel.recent.clear();
            }
            return;
        };
        if allow_expunge {
            let mut i = 0;
            while i < sel.known.len() {
                if mbox.find(sel.known[i].uid).is_none() {
                    self.out
                        .extend_from_slice(format!("* {} EXPUNGE\r\n", i + 1).as_bytes());
                    let gone = sel.known.remove(i);
                    sel.recent.remove(&gone.uid);
                } else {
                    i += 1;
                }
            }
        }
        for (i, known) in sel.known.iter_mut().enumerate() {
            if let Some(m) = mbox.find(known.uid)
                && !same_flags(&m.flags, &known.flags)
            {
                known.flags = m.flags.clone();
                let flags = wire::flag_list(&known.flags, sel.recent.contains(&known.uid));
                self.out.extend_from_slice(
                    format!("* {} FETCH (UID {} FLAGS {flags})\r\n", i + 1, known.uid).as_bytes(),
                );
            }
        }
        let mut added = false;
        let known_max = sel.max_uid;
        for m in mbox.messages.iter_mut().filter(|m| m.uid > known_max) {
            if m.recent && !sel.read_only {
                m.recent = false;
                sel.recent.insert(m.uid);
            }
            sel.known.push(Known {
                uid: m.uid,
                flags: m.flags.clone(),
            });
            sel.max_uid = m.uid;
            added = true;
        }
        if added {
            self.out.extend_from_slice(
                format!(
                    "* {} EXISTS\r\n* {} RECENT\r\n",
                    sel.known.len(),
                    sel.recent.len()
                )
                .as_bytes(),
            );
        }
    }

    // -- not authenticated ----------------------------------------------------

    fn id(&mut self, p: &mut Parser<'_>) -> CmdResult {
        p.sp()?;
        if !p.keyword("NIL") {
            p.expect(b'(')?;
            loop {
                p.opt_sp();
                if p.eat(b')') {
                    break;
                }
                if !p.keyword("NIL") {
                    p.string()?;
                }
                if p.is_end() {
                    return bad("Unterminated ID list");
                }
            }
        }
        p.end()?;
        self.untagged(&format!(
            "ID (\"name\" \"LowMail Test\" \"version\" \"{}\" \"vendor\" \"LowBloat\")",
            env!("CARGO_PKG_VERSION")
        ));
        ok("ID completed")
    }

    fn login(&mut self, p: &mut Parser<'_>) -> CmdResult {
        if self.user.is_some() {
            return bad("Already authenticated");
        }
        p.sp()?;
        let user = p.astring_utf8()?;
        p.sp()?;
        let pass = p.astring_utf8()?;
        p.end()?;
        match self.store.authenticate(&user, &pass) {
            Some(login) => {
                tracing::debug!(%login, "imap login");
                self.user = Some(login);
                ok("LOGIN completed")
            }
            None => no("[AUTHENTICATIONFAILED] Invalid credentials"),
        }
    }

    async fn authenticate(&mut self, p: &mut Parser<'_>) -> CmdResult {
        if self.user.is_some() {
            return bad("Already authenticated");
        }
        p.sp()?;
        let mechanism = p.atom()?;
        if !mechanism.eq_ignore_ascii_case("PLAIN") {
            return no("Unsupported authentication mechanism");
        }
        let response = if p.opt_sp() && !p.is_end() {
            let initial = p.take_while(|c| c != b' ').to_vec();
            p.end()?;
            if initial == b"=" { Vec::new() } else { initial }
        } else {
            self.out.extend_from_slice(b"+ \r\n");
            if self.flush().await.is_err() {
                return no("Connection lost");
            }
            match self.reader.read_line(MAX_LINE).await {
                Ok(Some(line)) => trim_eol(&line).to_vec(),
                _ => return no("Connection lost"),
            }
        };
        if response == b"*" {
            return bad("AUTHENTICATE cancelled");
        }
        let login = sasl::decode_plain(&response)
            .and_then(|(user, pass)| self.store.authenticate(&user, &pass));
        match login {
            Some(login) => {
                self.user = Some(login);
                ok("AUTHENTICATE completed")
            }
            None => no("[AUTHENTICATIONFAILED] Invalid credentials"),
        }
    }

    // -- authenticated ------------------------------------------------------

    fn mailbox_arg(p: &mut Parser<'_>) -> Result<String, Failure> {
        p.sp()?;
        Ok(normalize_mailbox(&p.astring_utf8()?))
    }

    fn select(&mut self, p: &mut Parser<'_>, user: &str, read_only: bool) -> CmdResult {
        let name = Self::mailbox_arg(p)?;
        // Optional select parameters (e.g. CONDSTORE) are ignored.
        self.selected = None;
        let store = self.store.clone();
        let mut inner = store.lock();
        let Some(mbox) = inner.mailbox_mut(user, &name) else {
            return no("[NONEXISTENT] Mailbox does not exist");
        };
        let mut recent = HashSet::new();
        for m in &mut mbox.messages {
            if m.recent {
                recent.insert(m.uid);
                if !read_only {
                    m.recent = false;
                }
            }
        }
        let mut keywords: Vec<String> = DEFAULT_KEYWORDS.iter().map(|k| k.to_string()).collect();
        for m in &mbox.messages {
            for f in m.flags.iter().filter(|f| !f.starts_with('\\')) {
                add_flag(&mut keywords, f);
            }
        }
        let system = SYSTEM_FLAGS.join(" ");
        let keywords = keywords.join(" ");
        self.untagged(&format!("FLAGS ({system} {keywords})"));
        if read_only {
            self.untagged("OK [PERMANENTFLAGS ()] Read-only mailbox");
        } else {
            self.untagged(&format!(
                "OK [PERMANENTFLAGS ({system} {keywords} \\*)] Flags permitted"
            ));
        }
        self.untagged(&format!("{} EXISTS", mbox.messages.len()));
        self.untagged(&format!("{} RECENT", recent.len()));
        if let Some(i) = mbox.messages.iter().position(|m| !m.has_flag("\\Seen")) {
            self.untagged(&format!("OK [UNSEEN {}] First unseen", i + 1));
        }
        self.untagged(&format!(
            "OK [UIDVALIDITY {}] UIDs valid",
            mbox.uid_validity
        ));
        self.untagged(&format!(
            "OK [UIDNEXT {}] Predicted next UID",
            mbox.uid_next
        ));
        self.selected = Some(Selected {
            name,
            uid_validity: mbox.uid_validity,
            read_only,
            known: mbox
                .messages
                .iter()
                .map(|m| Known {
                    uid: m.uid,
                    flags: m.flags.clone(),
                })
                .collect(),
            max_uid: mbox.messages.last().map_or(0, |m| m.uid),
            recent,
        });
        if read_only {
            ok("[READ-ONLY] EXAMINE completed")
        } else {
            ok("[READ-WRITE] SELECT completed")
        }
    }

    fn create(&mut self, p: &mut Parser<'_>, user: &str) -> CmdResult {
        let name = Self::mailbox_arg(p)?;
        p.end()?;
        if name == "INBOX" {
            return no("[ALREADYEXISTS] INBOX already exists");
        }
        let result = self.store.lock().create_mailbox(user, &name, None);
        match result {
            Ok(()) => {
                self.store.notify();
                ok("CREATE completed")
            }
            Err(MailboxError::Exists) => no("[ALREADYEXISTS] Mailbox already exists"),
            Err(_) => no("Invalid mailbox name"),
        }
    }

    fn delete(&mut self, p: &mut Parser<'_>, user: &str) -> CmdResult {
        let name = Self::mailbox_arg(p)?;
        p.end()?;
        if name == "INBOX" {
            return no("INBOX cannot be deleted");
        }
        {
            let mut inner = self.store.lock();
            let Some(account) = inner.account_mut(user) else {
                return no("Unknown user");
            };
            if !account.mailboxes.contains_key(&name) {
                return no("[NONEXISTENT] Mailbox does not exist");
            }
            if account.has_children(&name) {
                return no("[HASCHILDREN] Mailbox has children");
            }
            account.mailboxes.remove(&name);
        }
        self.store.notify();
        ok("DELETE completed")
    }

    fn rename(&mut self, p: &mut Parser<'_>, user: &str) -> CmdResult {
        let from = Self::mailbox_arg(p)?;
        let to = Self::mailbox_arg(p)?;
        p.end()?;
        if to == "INBOX" {
            return no("[ALREADYEXISTS] INBOX already exists");
        }
        if to.starts_with(&format!("{from}{DELIMITER}")) {
            return no("Cannot rename a mailbox into itself");
        }
        {
            let mut inner = self.store.lock();
            if inner.mailbox(user, &from).is_none() {
                return no("[NONEXISTENT] Mailbox does not exist");
            }
            if inner.mailbox(user, &to).is_some() {
                return no("[ALREADYEXISTS] Target mailbox already exists");
            }
            if from == "INBOX" {
                // RFC 3501: renaming INBOX moves its messages and leaves it empty.
                if inner.create_mailbox(user, &to, None).is_err() {
                    return no("Invalid mailbox name");
                }
                let messages = std::mem::take(
                    &mut inner
                        .mailbox_mut(user, "INBOX")
                        .expect("INBOX exists")
                        .messages,
                );
                let target = inner.mailbox_mut(user, &to).expect("just created");
                for m in messages {
                    target.append(m.data, m.flags, m.internal_date);
                }
            } else {
                if let Some((parent, _)) = to.rsplit_once(DELIMITER)
                    && inner.mailbox(user, parent).is_none()
                    && inner.create_mailbox(user, parent, None).is_err()
                {
                    return no("Invalid mailbox name");
                }
                let account = inner.account_mut(user).expect("authenticated user");
                let prefix = format!("{from}{DELIMITER}");
                let names: Vec<String> = account
                    .mailboxes
                    .keys()
                    .filter(|k| **k == from || k.starts_with(&prefix))
                    .cloned()
                    .collect();
                for old in names {
                    let new = format!("{to}{}", &old[from.len()..]);
                    if let Some(mut mbox) = account.mailboxes.remove(&old) {
                        mbox.name = new.clone();
                        account.mailboxes.insert(new, mbox);
                    }
                }
            }
        }
        self.store.notify();
        ok("RENAME completed")
    }

    fn subscribe(&mut self, p: &mut Parser<'_>, user: &str, subscribe: bool) -> CmdResult {
        let name = Self::mailbox_arg(p)?;
        p.end()?;
        let mut inner = self.store.lock();
        match inner.mailbox_mut(user, &name) {
            Some(mbox) => mbox.subscribed = subscribe,
            None if subscribe => return no("[NONEXISTENT] Mailbox does not exist"),
            None => {}
        }
        ok(if subscribe {
            "SUBSCRIBE completed"
        } else {
            "UNSUBSCRIBE completed"
        })
    }

    fn list(&mut self, p: &mut Parser<'_>, user: &str, lsub: bool) -> CmdResult {
        let word = if lsub { "LSUB" } else { "LIST" };
        p.sp()?;
        let mut special_only = false;
        let mut subscribed_only = lsub;
        let mut return_subscribed = false;
        if !lsub && p.peek() == Some(b'(') {
            let options = p.atom_list()?;
            special_only = options.iter().any(|o| o == "SPECIAL-USE");
            if options.iter().any(|o| o == "SUBSCRIBED") {
                subscribed_only = true;
                return_subscribed = true;
            }
            p.sp()?;
        }
        let reference = p.list_mailbox()?;
        p.sp()?;
        let patterns = if p.peek() == Some(b'(') {
            p.expect(b'(')?;
            let mut list = Vec::new();
            loop {
                p.opt_sp();
                if p.eat(b')') {
                    break;
                }
                list.push(p.list_mailbox()?);
                if p.is_end() {
                    return bad("Unterminated pattern list");
                }
            }
            list
        } else {
            vec![p.list_mailbox()?]
        };
        if p.opt_sp() && p.keyword("RETURN") {
            p.sp()?;
            let options = p.atom_list()?;
            return_subscribed |= options.iter().any(|o| o == "SUBSCRIBED");
        }
        p.end()?;

        if patterns.len() == 1 && patterns[0].is_empty() {
            self.untagged(&format!("{word} (\\Noselect) \"/\" \"\""));
            return ok(format!("{word} completed"));
        }
        let patterns: Vec<String> = patterns
            .iter()
            .map(|pat| normalize_pattern(&format!("{reference}{pat}")))
            .collect();

        let mut lines = Vec::new();
        {
            let inner = self.store.lock();
            let Some(account) = inner.account(user) else {
                return no("Unknown user");
            };
            let mut boxes: Vec<_> = account.mailboxes.values().collect();
            boxes.sort_by_key(|m| (m.name != "INBOX", m.name.clone()));
            for mbox in boxes {
                if !patterns
                    .iter()
                    .any(|pat| wildcard_match(pat.as_bytes(), mbox.name.as_bytes()))
                {
                    continue;
                }
                if (subscribed_only && !mbox.subscribed)
                    || (special_only && mbox.special_use.is_none())
                {
                    continue;
                }
                let mut attrs = Vec::new();
                if let Some(special) = mbox.special_use {
                    attrs.push(special.attribute());
                }
                attrs.push(if account.has_children(&mbox.name) {
                    "\\HasChildren"
                } else {
                    "\\HasNoChildren"
                });
                if return_subscribed && mbox.subscribed {
                    attrs.push("\\Subscribed");
                }
                lines.push(format!(
                    "{word} ({}) \"/\" {}",
                    attrs.join(" "),
                    wire::string(&mbox.name)
                ));
            }
        }
        for line in lines {
            self.untagged(&line);
        }
        ok(format!("{word} completed"))
    }

    fn status(&mut self, p: &mut Parser<'_>, user: &str) -> CmdResult {
        let name = Self::mailbox_arg(p)?;
        p.sp()?;
        let items = p.atom_list()?;
        p.end()?;
        let line = {
            let inner = self.store.lock();
            let Some(mbox) = inner.mailbox(user, &name) else {
                return no("[NONEXISTENT] Mailbox does not exist");
            };
            let mut parts = Vec::new();
            for item in &items {
                let value = match item.as_str() {
                    "MESSAGES" => mbox.messages.len(),
                    "RECENT" => mbox.recent(),
                    "UIDNEXT" => mbox.uid_next as usize,
                    "UIDVALIDITY" => mbox.uid_validity as usize,
                    "UNSEEN" => mbox.unseen(),
                    other => return bad(format!("Unknown status item {other}")),
                };
                parts.push(format!("{item} {value}"));
            }
            format!("STATUS {} ({})", wire::string(&name), parts.join(" "))
        };
        self.untagged(&line);
        ok("STATUS completed")
    }

    fn append(&mut self, p: &mut Parser<'_>, user: &str) -> CmdResult {
        let name = Self::mailbox_arg(p)?;
        p.sp()?;
        let mut flags = Vec::new();
        if p.peek() == Some(b'(') {
            flags = client_flags(p.flag_list()?)?;
            p.sp()?;
        }
        let mut date = chrono::Local::now().fixed_offset();
        if p.peek() == Some(b'"') {
            date = p.date_time()?;
            p.sp()?;
        }
        if p.peek() != Some(b'{') {
            return bad("Expected message literal");
        }
        let raw = p.literal()?;
        p.end()?;
        let (validity, uid) = {
            let mut inner = self.store.lock();
            let Some(mbox) = inner.mailbox_mut(user, &name) else {
                return no("[TRYCREATE] Mailbox does not exist");
            };
            let uid = mbox.append(Arc::new(MessageData::new(raw)), flags, date);
            (mbox.uid_validity, uid)
        };
        self.store.notify();
        ok(format!("[APPENDUID {validity} {uid}] APPEND completed"))
    }

    fn quota_root(&mut self, p: &mut Parser<'_>, user: &str) -> CmdResult {
        let name = Self::mailbox_arg(p)?;
        p.end()?;
        if self.store.lock().mailbox(user, &name).is_none() {
            return no("[NONEXISTENT] Mailbox does not exist");
        }
        self.untagged(&format!("QUOTAROOT {} \"\"", wire::string(&name)));
        let line = self.quota_line(user);
        self.untagged(&line);
        ok("GETQUOTAROOT completed")
    }

    fn quota(&mut self, p: &mut Parser<'_>, user: &str) -> CmdResult {
        p.sp()?;
        let root = p.astring()?;
        p.end()?;
        if !root.is_empty() {
            return no("No such quota root");
        }
        let line = self.quota_line(user);
        self.untagged(&line);
        ok("GETQUOTA completed")
    }

    fn quota_line(&self, user: &str) -> String {
        let used = self.store.lock().usage(user).div_ceil(1024);
        format!("QUOTA \"\" (STORAGE {used} {QUOTA_LIMIT_KB})")
    }

    async fn idle(&mut self, p: &mut Parser<'_>) -> CmdResult {
        p.end()?;
        self.changes.borrow_and_update();
        self.sync(true);
        self.out.extend_from_slice(b"+ idling\r\n");
        if self.flush().await.is_err() {
            return no("Connection lost");
        }
        loop {
            let event = tokio::select! {
                line = self.reader.read_line(MAX_LINE) => IdleEvent::Line(line),
                changed = self.changes.changed() => IdleEvent::Changed(changed.is_ok()),
            };
            match event {
                IdleEvent::Line(Ok(Some(line))) => {
                    return if trim_eol(&line).eq_ignore_ascii_case(b"DONE") {
                        ok("IDLE terminated")
                    } else {
                        bad("Expected DONE")
                    };
                }
                IdleEvent::Line(_) | IdleEvent::Changed(false) => return no("Connection closed"),
                IdleEvent::Changed(true) => {
                    self.changes.borrow_and_update();
                    self.sync(true);
                    if self.flush().await.is_err() {
                        return no("Connection lost");
                    }
                }
            }
        }
    }

    // -- selected -------------------------------------------------------------

    fn close(&mut self, user: &str, expunge: bool) -> CmdResult {
        let sel = self.selected.take().expect("selected");
        if expunge && !sel.read_only {
            let removed = {
                let mut inner = self.store.lock();
                inner.mailbox_mut(user, &sel.name).is_some_and(|mbox| {
                    let before = mbox.messages.len();
                    mbox.messages.retain(|m| !m.has_flag("\\Deleted"));
                    before != mbox.messages.len()
                })
            };
            if removed {
                self.store.notify();
            }
        }
        ok(if expunge {
            "CLOSE completed"
        } else {
            "UNSELECT completed"
        })
    }

    fn expunge(&mut self, p: &mut Parser<'_>, user: &str, uid: bool) -> CmdResult {
        let set = if uid {
            p.sp()?;
            Some(p.sequence_set()?)
        } else {
            None
        };
        p.end()?;
        let sel = self.selected.as_ref().expect("selected");
        if sel.read_only {
            return no("Mailbox is read-only");
        }
        let max_uid = sel.known.last().map_or(0, |k| k.uid);
        let removed = {
            let mut inner = self.store.lock();
            inner.mailbox_mut(user, &sel.name).is_some_and(|mbox| {
                let before = mbox.messages.len();
                mbox.messages.retain(|m| {
                    !(m.has_flag("\\Deleted")
                        && set.as_ref().is_none_or(|s| s.contains(m.uid, max_uid)))
                });
                before != mbox.messages.len()
            })
        };
        if removed {
            self.store.notify();
        }
        // The EXPUNGE responses are produced by `sync` before the tagged OK.
        ok("EXPUNGE completed")
    }

    /// Snapshots the selected messages for SEARCH/SORT.
    fn candidates(&self, user: &str) -> (Vec<Candidate>, Bounds) {
        let sel = self.selected.as_ref().expect("selected");
        let inner = self.store.lock();
        let mut list = Vec::new();
        if let Some(mbox) = inner.mailbox(user, &sel.name) {
            for (i, known) in sel.known.iter().enumerate() {
                if let Some(m) = mbox.find(known.uid) {
                    list.push(Candidate {
                        seq: i as u32 + 1,
                        uid: m.uid,
                        flags: m.flags.clone(),
                        recent: sel.recent.contains(&m.uid),
                        internal_date: m.internal_date,
                        data: m.data.clone(),
                    });
                }
            }
        }
        let bounds = Bounds {
            max_seq: sel.known.len() as u32,
            max_uid: sel.known.last().map_or(0, |k| k.uid),
        };
        (list, bounds)
    }

    fn search(&mut self, p: &mut Parser<'_>, user: &str, uid: bool) -> CmdResult {
        p.sp()?;
        let key = search::parse_program(p, true).map_err(search_failure)?;
        let (candidates, bounds) = self.candidates(user);
        let mut line = String::from("SEARCH");
        for c in candidates
            .iter()
            .filter(|c| search::matches(&key, c, &bounds))
        {
            line.push_str(&format!(" {}", if uid { c.uid } else { c.seq }));
        }
        self.untagged(&line);
        ok("SEARCH completed")
    }

    fn sort(&mut self, p: &mut Parser<'_>, user: &str, uid: bool) -> CmdResult {
        p.sp()?;
        let criteria = sort::parse_criteria(p)?;
        p.sp()?;
        search::check_charset(&p.astring_utf8()?).map_err(search_failure)?;
        p.sp()?;
        let key = search::parse_keys(p).map_err(search_failure)?;
        let (candidates, bounds) = self.candidates(user);
        let mut hits: Vec<Candidate> = candidates
            .into_iter()
            .filter(|c| search::matches(&key, c, &bounds))
            .collect();
        sort::sort(&mut hits, &criteria);
        let mut line = String::from("SORT");
        for c in &hits {
            line.push_str(&format!(" {}", if uid { c.uid } else { c.seq }));
        }
        self.untagged(&line);
        ok("SORT completed")
    }

    fn fetch(&mut self, p: &mut Parser<'_>, user: &str, uid: bool) -> CmdResult {
        p.sp()?;
        let set = p.sequence_set()?;
        p.sp()?;
        let mut atts = fetch::parse_atts(p)?;
        p.end()?;
        if uid && !atts.contains(&FetchAtt::Uid) {
            atts.insert(0, FetchAtt::Uid);
        }
        let store = self.store.clone();
        let sel = self.selected.as_mut().expect("selected");
        let set_seen = !sel.read_only && atts.iter().any(FetchAtt::sets_seen);
        let mut changed_any = false;
        for (seq, msg_uid) in sel.targets(&set, uid) {
            let snapshot = {
                let mut inner = store.lock();
                inner
                    .mailbox_mut(user, &sel.name)
                    .and_then(|mbox| mbox.find_mut(msg_uid))
                    .map(|m| {
                        let changed = set_seen && add_flag(&mut m.flags, "\\Seen");
                        (m.data.clone(), m.flags.clone(), m.internal_date, changed)
                    })
            };
            let Some((data, flags, internal_date, changed)) = snapshot else {
                continue;
            };
            if changed {
                changed_any = true;
                sel.known[seq as usize - 1].flags = flags.clone();
            }
            let ctx = ItemCtx {
                uid: msg_uid,
                flags: &flags,
                recent: sel.recent.contains(&msg_uid),
                internal_date: &internal_date,
                data: &data,
            };
            self.out
                .extend_from_slice(format!("* {seq} FETCH (").as_bytes());
            for (i, att) in atts.iter().enumerate() {
                if i > 0 {
                    self.out.push(b' ');
                }
                fetch::render(att, &ctx, &mut self.out);
            }
            if changed && !atts.contains(&FetchAtt::Flags) {
                self.out.push(b' ');
                fetch::render(&FetchAtt::Flags, &ctx, &mut self.out);
            }
            self.out.extend_from_slice(b")\r\n");
        }
        if changed_any {
            store.notify();
        }
        ok("FETCH completed")
    }

    fn store_flags(&mut self, p: &mut Parser<'_>, user: &str, uid: bool) -> CmdResult {
        enum Mode {
            Replace,
            Add,
            Remove,
        }
        p.sp()?;
        let set = p.sequence_set()?;
        p.sp()?;
        let item = p.atom()?.to_ascii_uppercase();
        let (mode, silent) = match item.as_str() {
            "FLAGS" => (Mode::Replace, false),
            "FLAGS.SILENT" => (Mode::Replace, true),
            "+FLAGS" => (Mode::Add, false),
            "+FLAGS.SILENT" => (Mode::Add, true),
            "-FLAGS" => (Mode::Remove, false),
            "-FLAGS.SILENT" => (Mode::Remove, true),
            other => return bad(format!("Unknown STORE item {other}")),
        };
        p.sp()?;
        let flags = if p.peek() == Some(b'(') {
            p.flag_list()?
        } else {
            let mut list = vec![p.flag()?];
            while p.opt_sp() && !p.is_end() {
                list.push(p.flag()?);
            }
            list
        };
        p.end()?;
        let flags = client_flags(flags)?;
        let store = self.store.clone();
        let sel = self.selected.as_mut().expect("selected");
        if sel.read_only {
            return no("Mailbox is read-only");
        }
        let mut changed_any = false;
        for (seq, msg_uid) in sel.targets(&set, uid) {
            let updated = {
                let mut inner = store.lock();
                inner
                    .mailbox_mut(user, &sel.name)
                    .and_then(|mbox| mbox.find_mut(msg_uid))
                    .map(|m| {
                        let before = m.flags.clone();
                        match mode {
                            Mode::Replace => m.flags = flags.clone(),
                            Mode::Add => {
                                for f in &flags {
                                    add_flag(&mut m.flags, f);
                                }
                            }
                            Mode::Remove => {
                                for f in &flags {
                                    remove_flag(&mut m.flags, f);
                                }
                            }
                        }
                        (m.flags.clone(), !same_flags(&before, &m.flags))
                    })
            };
            let Some((new_flags, changed)) = updated else {
                continue;
            };
            if changed {
                changed_any = true;
            }
            sel.known[seq as usize - 1].flags = new_flags.clone();
            if !silent {
                let list = wire::flag_list(&new_flags, sel.recent.contains(&msg_uid));
                let line = if uid {
                    format!("* {seq} FETCH (UID {msg_uid} FLAGS {list})\r\n")
                } else {
                    format!("* {seq} FETCH (FLAGS {list})\r\n")
                };
                self.out.extend_from_slice(line.as_bytes());
            }
        }
        if changed_any {
            store.notify();
        }
        ok("STORE completed")
    }

    fn copy(&mut self, p: &mut Parser<'_>, user: &str, uid: bool, is_move: bool) -> CmdResult {
        p.sp()?;
        let set = p.sequence_set()?;
        let dest = Self::mailbox_arg(p)?;
        p.end()?;
        let sel = self.selected.as_ref().expect("selected");
        if is_move && sel.read_only {
            return no("Mailbox is read-only");
        }
        let targets = sel.targets(&set, uid);
        let source = sel.name.clone();
        let (validity, src_uids, dst_uids) = {
            let mut inner = self.store.lock();
            let messages: Vec<store::Message> = match inner.mailbox(user, &source) {
                Some(mbox) => targets
                    .iter()
                    .filter_map(|&(_, u)| mbox.find(u).cloned())
                    .collect(),
                None => Vec::new(),
            };
            let Some(target) = inner.mailbox_mut(user, &dest) else {
                return no("[TRYCREATE] Mailbox does not exist");
            };
            let mut src_uids = Vec::new();
            let mut dst_uids = Vec::new();
            for m in messages {
                dst_uids.push(target.append(m.data, m.flags, m.internal_date));
                src_uids.push(m.uid);
            }
            let validity = target.uid_validity;
            if is_move && let Some(mbox) = inner.mailbox_mut(user, &source) {
                let moved: HashSet<u32> = src_uids.iter().copied().collect();
                mbox.messages.retain(|m| !moved.contains(&m.uid));
            }
            (validity, src_uids, dst_uids)
        };
        if src_uids.is_empty() {
            return ok(if is_move {
                "No messages moved"
            } else {
                "No messages copied"
            });
        }
        self.store.notify();
        let code = format!(
            "[COPYUID {validity} {} {}]",
            wire::uid_set(&src_uids),
            wire::uid_set(&dst_uids)
        );
        if is_move {
            self.untagged(&format!("OK {code} Moved"));
            ok("MOVE completed")
        } else {
            ok(format!("{code} COPY completed"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        assert!(wildcard_match(b"*", b"Projetos/Clientes"));
        assert!(wildcard_match(b"%", b"Projetos"));
        assert!(!wildcard_match(b"%", b"Projetos/Clientes"));
        assert!(wildcard_match(b"Projetos/%", b"Projetos/Clientes"));
        assert!(wildcard_match(b"Proj*", b"Projetos/Clientes"));
        assert!(!wildcard_match(b"Proj%", b"Projetos/Clientes"));
        assert_eq!(normalize_pattern("inbox/*"), "INBOX/*");
        assert_eq!(normalize_pattern("inboxes"), "inboxes");
    }
}
