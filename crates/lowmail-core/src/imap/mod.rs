//! IMAP connections: sign-in, capabilities, the selected folder and a small
//! pool so the list, the reading pane and background work do not wait on
//! each other.

mod folders;
mod messages;
mod search;
pub mod watch;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_imap::imap_proto::{Response, Status};
use parking_lot::Mutex;
use tokio::io::AsyncWriteExt;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::error::{Error, ErrorKind, Result};
use crate::model::{AccountConfig, Security};
use crate::net::{self, MailStream};

pub use folders::{FolderList, child_path};
pub use search::build_criteria;

pub type Session = async_imap::Session<MailStream>;

/// How long an operation may take before the connection is considered dead.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
/// Idle pooled connections older than this are checked with NOOP before use.
const STALE_AFTER: Duration = Duration::from_secs(60);
/// The same for operations that must not run twice, so a connection the
/// server dropped is found before the command instead of in the middle.
const STALE_AFTER_CAREFUL: Duration = Duration::from_secs(5);
/// Pooled connections idle for longer than this are closed.
const MAX_IDLE: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, Default)]
pub struct Caps(HashSet<String>);

impl Caps {
    pub fn has(&self, name: &str) -> bool {
        self.0.contains(&name.to_ascii_uppercase())
    }

    pub fn list(&self) -> Vec<String> {
        let mut v: Vec<String> = self.0.iter().cloned().collect();
        v.sort();
        v
    }
}

#[derive(Debug, Clone)]
pub struct Selected {
    pub path: String,
    pub uid_validity: u32,
    pub exists: u32,
    pub uid_next: u32,
    pub read_only: bool,
}

/// One signed-in IMAP connection.
pub struct Conn {
    pub session: Session,
    pub caps: Caps,
    pub selected: Option<Selected>,
    last_used: Instant,
}

impl Conn {
    pub async fn open(account: &AccountConfig, password: &str) -> Result<Conn> {
        let ep = &account.imap;
        let tcp = net::connect_tcp(&ep.host, ep.port).await?;
        let stream = match ep.security {
            Security::Tls => net::upgrade(tcp, &ep.host, ep.accept_invalid_certs).await?,
            Security::None | Security::StartTls => MailStream::Plain(tcp),
        };
        let mut client = async_imap::Client::new(stream);
        let greeting = tokio::time::timeout(net::CONNECT_TIMEOUT, client.read_response())
            .await
            .map_err(|_| Error::new(ErrorKind::Network, "The IMAP server did not greet in time."))??;
        if greeting.is_none() {
            return Err(Error::new(ErrorKind::Network, "The IMAP server closed the connection."));
        }

        if ep.security == Security::StartTls {
            client
                .run_command_and_check_ok("STARTTLS", None)
                .await
                .map_err(|e| Error::new(ErrorKind::Tls, "The IMAP server does not support STARTTLS.").with_detail(e.to_string()))?;
            let MailStream::Plain(tcp) = client.into_inner() else {
                return Err(Error::internal("Unexpected stream state."));
            };
            let stream = net::upgrade(tcp, &ep.host, ep.accept_invalid_certs).await?;
            client = async_imap::Client::new(stream);
        }

        let session = match client.login(&account.username, password).await {
            Ok(session) => session,
            Err((e, _)) => {
                let detail = Error::from(e).detail.unwrap_or_default();
                return Err(Error::new(ErrorKind::Auth, "Wrong user name or password.").with_detail(detail));
            }
        };
        let mut conn = Conn { session, caps: Caps::default(), selected: None, last_used: Instant::now() };
        conn.refresh_caps().await?;
        if conn.caps.has("ID") {
            // Some servers (and their logs) like to know the client.
            let _ = conn.raw(&format!("ID (\"name\" \"LowMail\" \"version\" \"{}\")", env!("CARGO_PKG_VERSION"))).await;
        }
        Ok(conn)
    }

    async fn refresh_caps(&mut self) -> Result<()> {
        let caps = self.session.capabilities().await?;
        let mut set = HashSet::new();
        for cap in caps.iter() {
            use async_imap::types::Capability;
            match cap {
                Capability::Imap4rev1 => {
                    set.insert("IMAP4REV1".to_string());
                }
                Capability::Auth(a) => {
                    set.insert(format!("AUTH={}", a.to_ascii_uppercase()));
                }
                Capability::Atom(a) => {
                    set.insert(a.to_ascii_uppercase());
                }
            }
        }
        self.caps = Caps(set);
        Ok(())
    }

    /// Selects `path` unless it is already selected. A selected folder gets a
    /// NOOP instead; the EXISTS and EXPUNGE it brings keep the message count
    /// current.
    pub async fn select(&mut self, path: &str) -> Result<Selected> {
        if self.selected.as_ref().is_some_and(|s| s.path == path) {
            self.session.noop().await?;
            self.apply_unsolicited();
            return Ok(self.selected.clone().expect("selected"));
        }
        self.selected = None;
        let mailbox = self.session.select(path).await.map_err(|e| {
            let err = Error::from(e);
            if err.kind == ErrorKind::Server {
                Error::not_found("The folder does not exist.").with_detail(err.detail.unwrap_or_default())
            } else {
                err
            }
        })?;
        let sel = Selected {
            path: path.to_string(),
            uid_validity: mailbox.uid_validity.unwrap_or(0),
            exists: mailbox.exists,
            uid_next: mailbox.uid_next.unwrap_or(0),
            read_only: false,
        };
        self.drain_unsolicited();
        self.selected = Some(sel.clone());
        Ok(sel)
    }

    /// Applies the EXISTS/EXPUNGE the server sent on its own to the selected
    /// folder's message count.
    fn apply_unsolicited(&mut self) {
        use async_imap::types::UnsolicitedResponse;
        while let Ok(update) = self.session.unsolicited_responses.try_recv() {
            if let Some(sel) = self.selected.as_mut() {
                match update {
                    UnsolicitedResponse::Exists(n) => sel.exists = n,
                    UnsolicitedResponse::Expunge(_) => sel.exists = sel.exists.saturating_sub(1),
                    _ => {}
                }
            }
        }
    }

    /// Forgets changes the server reported on its own; callers refetch what
    /// they show.
    pub fn drain_unsolicited(&mut self) {
        self.apply_unsolicited();
    }

    /// Runs a command and hands each untagged response, then the tagged OK,
    /// to `each`; fails on NO/BAD.
    pub async fn raw_each(&mut self, command: &str, mut each: impl FnMut(&Response<'_>)) -> Result<()> {
        let tag = self.session.run_command(command).await?;
        loop {
            let Some(resp) = self.session.read_response().await? else {
                return Err(Error::new(ErrorKind::Network, "The server closed the connection."));
            };
            if let Response::Done { tag: done, status, outcome } = resp.parsed() {
                if *done == tag {
                    return match status {
                        Status::Ok => {
                            each(resp.parsed());
                            Ok(())
                        }
                        _ => Err(Error::server("The server refused the operation.")
                            .with_detail(outcome.information.as_deref().unwrap_or("").to_string())),
                    };
                }
                continue;
            }
            if let Some(sel) = self.selected.as_mut() {
                match resp.parsed() {
                    Response::MailboxData(async_imap::imap_proto::MailboxDatum::Exists(n)) => sel.exists = *n,
                    Response::Expunge(_) => sel.exists = sel.exists.saturating_sub(1),
                    _ => {}
                }
            }
            each(resp.parsed());
        }
    }

    /// Runs a command, ignoring its untagged responses.
    pub async fn raw(&mut self, command: &str) -> Result<()> {
        self.raw_each(command, |_| {}).await
    }

    /// APPEND with the `APPENDUID` answer (UIDPLUS), which async-imap drops.
    pub async fn append(&mut self, path: &str, flags: &[&str], message: &[u8]) -> Result<Option<(u32, u32)>> {
        let flags = if flags.is_empty() { String::new() } else { format!(" ({})", flags.join(" ")) };
        let literal_plus = self.caps.has("LITERAL+");
        let command = if literal_plus {
            format!("APPEND {}{} {{{}+}}", quote(path), flags, message.len())
        } else {
            format!("APPEND {}{} {{{}}}", quote(path), flags, message.len())
        };
        let tag = self.session.run_command(&command).await?;
        if !literal_plus {
            loop {
                let Some(resp) = self.session.read_response().await? else {
                    return Err(Error::new(ErrorKind::Network, "The server closed the connection."));
                };
                match resp.parsed() {
                    Response::Continue { .. } => break,
                    Response::Done { tag: done, outcome, .. } if *done == tag => {
                        return Err(Error::server("The server did not accept the message.")
                            .with_detail(outcome.information.as_deref().unwrap_or("").to_string()));
                    }
                    _ => {}
                }
            }
        }
        let stream = self.session.get_mut();
        stream.write_all(message).await?;
        stream.write_all(b"\r\n").await?;
        stream.flush().await?;
        loop {
            let Some(resp) = self.session.read_response().await? else {
                return Err(Error::new(ErrorKind::Network, "The server closed the connection."));
            };
            if let Response::Done { tag: done, status, outcome } = resp.parsed() {
                if *done != tag {
                    continue;
                }
                if *status != Status::Ok {
                    let info = outcome.information.as_deref().unwrap_or("").to_string();
                    let lower = info.to_ascii_lowercase();
                    let err = if lower.contains("quota") || lower.contains("overquota") {
                        Error::new(ErrorKind::TooLarge, "The mailbox is full.")
                    } else {
                        Error::server("The server did not accept the message.")
                    };
                    return Err(err.with_detail(info));
                }
                use async_imap::imap_proto::ResponseCode;
                use async_imap::imap_proto::rfc4315::UidSetMember;
                if let Some(ResponseCode::AppendUid(validity, uids)) = &outcome.code
                    && let Some(member) = uids.first()
                {
                    let uid = match member {
                        UidSetMember::Uid(u) => *u,
                        UidSetMember::UidRange(r) => *r.start(),
                    };
                    return Ok(Some((*validity, uid)));
                }
                return Ok(None);
            }
        }
    }

    pub async fn logout(mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(3), self.session.logout()).await;
    }
}

/// Quotes a string for IMAP. Folder names are 7-bit (modified UTF-7), so a
/// quoted string always works for them; search terms go through
/// `search::astring`, which can send literals.
pub fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// UIDs as a compact sequence set: `1:4,7,9:12`.
pub fn uid_set(uids: &[u32]) -> String {
    let mut sorted: Vec<u32> = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut out = String::new();
    let mut i = 0;
    while i < sorted.len() {
        let start = sorted[i];
        let mut end = start;
        while i + 1 < sorted.len() && sorted[i + 1] == end + 1 {
            i += 1;
            end = sorted[i];
        }
        if !out.is_empty() {
            out.push(',');
        }
        if start == end {
            out.push_str(&start.to_string());
        } else {
            out.push_str(&format!("{start}:{end}"));
        }
        i += 1;
    }
    out
}

/// A few signed-in connections for one account.
pub struct Pool {
    account: AccountConfig,
    password: String,
    idle: Mutex<Vec<Conn>>,
    permits: Arc<Semaphore>,
    caps: Mutex<Option<Caps>>,
}

impl Pool {
    pub fn new(account: AccountConfig, password: String, size: usize) -> Arc<Pool> {
        Arc::new(Pool {
            account,
            password,
            idle: Mutex::new(Vec::new()),
            permits: Arc::new(Semaphore::new(size.max(1))),
            caps: Mutex::new(None),
        })
    }

    pub fn account(&self) -> &AccountConfig {
        &self.account
    }

    pub fn password(&self) -> &str {
        &self.password
    }

    /// Capabilities of the server, known after the first connection.
    pub fn caps(&self) -> Option<Caps> {
        self.caps.lock().clone()
    }

    pub async fn get(self: &Arc<Self>) -> Result<Pooled> {
        self.get_checked(false).await
    }

    /// A connection; `careful` checks it sooner, for operations that cannot
    /// simply be retried on another connection.
    pub async fn get_checked(self: &Arc<Self>, careful: bool) -> Result<Pooled> {
        let stale_after = if careful { STALE_AFTER_CAREFUL } else { STALE_AFTER };
        let permit = self.permits.clone().acquire_owned().await.map_err(|_| Error::internal("Pool closed."))?;
        loop {
            let candidate = self.idle.lock().pop();
            let Some(mut conn) = candidate else { break };
            let age = conn.last_used.elapsed();
            if age > MAX_IDLE {
                tokio::spawn(conn.logout());
                continue;
            }
            if age > stale_after {
                match tokio::time::timeout(Duration::from_secs(10), conn.session.noop()).await {
                    Ok(Ok(())) => conn.drain_unsolicited(),
                    _ => continue,
                }
            }
            return Ok(Pooled { conn: Some(conn), pool: self.clone(), _permit: permit, broken: false });
        }
        let conn = Conn::open(&self.account, &self.password).await?;
        *self.caps.lock() = Some(conn.caps.clone());
        Ok(Pooled { conn: Some(conn), pool: self.clone(), _permit: permit, broken: false })
    }

    /// Closes every idle connection.
    pub async fn close(&self) {
        let conns: Vec<Conn> = std::mem::take(&mut *self.idle.lock());
        for conn in conns {
            conn.logout().await;
        }
    }
}

pub struct Pooled {
    conn: Option<Conn>,
    pool: Arc<Pool>,
    _permit: OwnedSemaphorePermit,
    pub broken: bool,
}

impl Pooled {
    pub fn conn(&mut self) -> &mut Conn {
        self.conn.as_mut().expect("connection present until drop")
    }
}

impl Drop for Pooled {
    fn drop(&mut self) {
        if let Some(mut conn) = self.conn.take()
            && !self.broken
        {
            conn.last_used = Instant::now();
            self.pool.idle.lock().push(conn);
        }
    }
}
