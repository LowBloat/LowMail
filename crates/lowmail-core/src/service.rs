//! One signed-in mail account: everything the UI asks for, on top of the
//! IMAP pool, SMTP, ManageSieve and the local store.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, watch};

use crate::compose::{self, DraftMeta, QuoteStyle, ResolvedAttachment};
use crate::error::{Error, ErrorKind, Result};
use crate::imap::{FolderList, Pool};
use crate::mime::{self, ReadOptions};
use crate::model::*;
use crate::sanitize;
use crate::sieve;
use crate::smtp;
use crate::store::Store;

/// Runs `$body` (an async block body using `$conn: &mut Conn`) on a pooled
/// connection with a time limit. A connection that fails at the network
/// level is dropped, and the body runs once more on a fresh connection when
/// `$retry` allows it. A macro rather than a closure so the futures stay `Send`.
macro_rules! with_conn {
    ($pool:expr, $retry:expr, |$conn:ident| $body:expr) => {{
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut pooled = $pool.get_checked(!$retry).await?;
            let result = {
                let $conn = pooled.conn();
                match tokio::time::timeout(crate::imap::COMMAND_TIMEOUT, async { $body }).await {
                    Ok(r) => r,
                    Err(_) => Err(Error::new(ErrorKind::Network, "The server did not answer in time.")),
                }
            };
            match result {
                Err(e) if e.is_connection_error() => {
                    pooled.broken = true;
                    drop(pooled);
                    if $retry && attempt < 2 {
                        continue;
                    }
                    break Err(e);
                }
                other => break other,
            }
        }
    }};
}

/// Raw messages kept in memory so switching between them is instant.
const CACHE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_UPLOAD: usize = 50 * 1024 * 1024;
const MAX_UPLOADS_TOTAL: usize = 200 * 1024 * 1024;
const UPLOAD_TTL: Duration = Duration::from_secs(24 * 3600);
/// Results of a search in every folder.
const SEARCH_ALL_LIMIT: usize = 300;

#[derive(Default)]
struct RawCache {
    map: HashMap<(String, u32, u32), Arc<Vec<u8>>>,
    order: VecDeque<(String, u32, u32)>,
    bytes: usize,
}

impl RawCache {
    fn get(&self, key: &(String, u32, u32)) -> Option<Arc<Vec<u8>>> {
        self.map.get(key).cloned()
    }

    fn put(&mut self, key: (String, u32, u32), raw: Arc<Vec<u8>>) {
        if raw.len() > CACHE_BYTES / 4 || self.map.contains_key(&key) {
            return;
        }
        self.bytes += raw.len();
        self.order.push_back(key.clone());
        self.map.insert(key, raw);
        while self.bytes > CACHE_BYTES {
            let Some(old) = self.order.pop_front() else { break };
            if let Some(v) = self.map.remove(&old) {
                self.bytes -= v.len();
            }
        }
    }

    fn forget_folder(&mut self, folder: &str) {
        self.map.retain(|k, _| k.0 != folder);
        self.order.retain(|k| k.0 != folder);
        self.bytes = self.map.values().map(|v| v.len()).sum();
    }
}

struct StoredUpload {
    info: Upload,
    bytes: Arc<Vec<u8>>,
    created: Instant,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlagChange {
    pub seen: Option<bool>,
    pub flagged: Option<bool>,
    pub answered: Option<bool>,
    pub forwarded: Option<bool>,
}

/// Where moved messages went and their UIDs there (empty when the server
/// does not tell), so the UI can undo the move.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Moved {
    pub folder: String,
    pub uids: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    pub message_id: String,
    /// A copy was saved in Sent.
    pub saved: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub id: String,
    pub email: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub filters: bool,
}

pub struct MailClient {
    owner: String,
    pool: Arc<Pool>,
    store: Arc<Store>,
    cache: Mutex<RawCache>,
    uploads: Mutex<HashMap<String, StoredUpload>>,
    folders: Mutex<Option<FolderList>>,
    events: broadcast::Sender<MailEvent>,
    stop_watch: Mutex<Option<watch::Sender<bool>>>,
}

impl MailClient {
    /// Signs in (the first connection checks the password) and returns the client.
    pub async fn connect(
        account: AccountConfig,
        password: String,
        store: Arc<Store>,
        owner: String,
        pool_size: usize,
    ) -> Result<Arc<MailClient>> {
        let pool = Pool::new(account, password, pool_size);
        drop(pool.get().await?);
        let (events, _) = broadcast::channel(64);
        Ok(Arc::new(MailClient {
            owner,
            pool,
            store,
            cache: Mutex::new(RawCache::default()),
            uploads: Mutex::new(HashMap::new()),
            folders: Mutex::new(None),
            events,
            stop_watch: Mutex::new(None),
        }))
    }

    pub fn account(&self) -> &AccountConfig {
        self.pool.account()
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    pub fn password(&self) -> &str {
        self.pool.password()
    }

    pub fn info(&self) -> AccountInfo {
        let a = self.account();
        AccountInfo {
            id: a.id.clone(),
            email: a.email.clone(),
            name: a.name.clone(),
            capabilities: self.pool.caps().map(|c| c.list()).unwrap_or_default(),
            filters: a.sieve.is_some(),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<MailEvent> {
        self.events.subscribe()
    }

    pub(crate) fn emit(&self, event: MailEvent) {
        let _ = self.events.send(event);
    }

    /// Starts watching the inbox for new mail (IDLE, or polling).
    pub fn start_watching(self: &Arc<Self>) {
        let mut guard = self.stop_watch.lock();
        if guard.is_some() {
            return;
        }
        let (tx, rx) = watch::channel(false);
        *guard = Some(tx);
        tokio::spawn(crate::imap::watch::run(Arc::downgrade(self), rx));
    }

    pub async fn close(&self) {
        if let Some(tx) = self.stop_watch.lock().take() {
            let _ = tx.send(true);
        }
        self.pool.close().await;
    }

    // ---- folders -------------------------------------------------------

    pub async fn folders(&self, counts: bool) -> Result<Vec<Folder>> {
        let list = with_conn!(self.pool, true, |conn| conn.list_folders(counts).await)?;
        let folders = list.folders.clone();
        *self.folders.lock() = Some(list);
        Ok(folders)
    }

    /// The last folder list, without asking the server.
    pub fn cached_folders(&self) -> Option<FolderList> {
        self.folders.lock().clone()
    }

    async fn folder_list(&self) -> Result<FolderList> {
        if let Some(list) = self.folders.lock().clone() {
            return Ok(list);
        }
        let list = with_conn!(self.pool, true, |conn| conn.list_folders(false).await)?;
        *self.folders.lock() = Some(list.clone());
        Ok(list)
    }

    pub async fn role_path(&self, role: FolderRole) -> Result<Option<String>> {
        Ok(self.folder_list().await?.by_role(role).map(|f| f.path.clone()))
    }

    /// The folder with `role`, created with a default name when missing.
    async fn ensure_role(&self, role: FolderRole) -> Result<String> {
        if let Some(path) = self.role_path(role).await? {
            return Ok(path);
        }
        let name = match role {
            FolderRole::Inbox => return Ok("INBOX".into()),
            FolderRole::Sent => "Sent",
            FolderRole::Drafts => "Drafts",
            FolderRole::Trash => "Trash",
            FolderRole::Junk => "Junk",
            FolderRole::Archive => "Archive",
        };
        let list = self.folder_list().await?;
        let path = crate::imap::child_path(&list, None, name)?;
        if list.get(&path).is_none() {
            let p = path.clone();
            with_conn!(self.pool, false, |conn| conn.create_folder(&p).await)?;
        }
        *self.folders.lock() = None;
        Ok(path)
    }

    pub async fn create_folder(&self, parent: Option<String>, name: &str) -> Result<String> {
        let list = self.folder_list().await?;
        let path = crate::imap::child_path(&list, parent.as_deref(), name)?;
        if list.get(&path).is_some() {
            return Err(Error::invalid("A folder with this name already exists."));
        }
        let p = path.clone();
        with_conn!(self.pool, false, |conn| conn.create_folder(&p).await)?;
        *self.folders.lock() = None;
        Ok(path)
    }

    pub async fn rename_folder(&self, path: &str, new_name: &str, new_parent: Option<Option<String>>) -> Result<String> {
        let list = self.folder_list().await?;
        let folder = list.get(path).ok_or_else(|| Error::not_found("The folder does not exist."))?;
        if folder.role == Some(FolderRole::Inbox) {
            return Err(Error::invalid("The inbox cannot be renamed."));
        }
        let parent = match new_parent {
            Some(p) => p,
            None => folder.parent.clone(),
        };
        if parent.as_deref().is_some_and(|p| p == path || p.starts_with(&format!("{path}{}", list.delimiter.as_deref().unwrap_or("/")))) {
            return Err(Error::invalid("A folder cannot be moved into itself."));
        }
        let target = crate::imap::child_path(&list, parent.as_deref(), new_name)?;
        if target == path {
            return Ok(target);
        }
        if list.get(&target).is_some() {
            return Err(Error::invalid("A folder with this name already exists."));
        }
        let (from, to) = (path.to_string(), target.clone());
        with_conn!(self.pool, false, |conn| conn.rename_folder(&from, &to).await)?;
        *self.folders.lock() = None;
        self.cache.lock().forget_folder(path);
        Ok(target)
    }

    pub async fn delete_folder(&self, path: &str) -> Result<()> {
        let list = self.folder_list().await?;
        let folder = list.get(path).ok_or_else(|| Error::not_found("The folder does not exist."))?;
        if folder.role == Some(FolderRole::Inbox) {
            return Err(Error::invalid("The inbox cannot be deleted."));
        }
        // Children first, deepest first.
        let delim = list.delimiter.clone().unwrap_or_else(|| "/".into());
        let mut doomed: Vec<String> =
            list.folders.iter().filter(|f| f.path.starts_with(&format!("{path}{delim}"))).map(|f| f.path.clone()).collect();
        doomed.sort_by_key(|p| std::cmp::Reverse(p.matches(delim.as_str()).count()));
        doomed.push(path.to_string());
        with_conn!(self.pool, false, |conn| {
            for p in &doomed {
                conn.delete_folder(p).await?;
            }
            Ok(())
        })?;
        *self.folders.lock() = None;
        self.cache.lock().forget_folder(path);
        Ok(())
    }

    pub async fn set_subscribed(&self, path: &str, subscribed: bool) -> Result<()> {
        with_conn!(self.pool, true, |conn| conn.set_subscribed(path, subscribed).await)?;
        *self.folders.lock() = None;
        Ok(())
    }

    pub async fn empty_folder(&self, path: &str) -> Result<()> {
        with_conn!(self.pool, false, |conn| {
            conn.select(path).await?;
            conn.delete_all().await
        })?;
        self.cache.lock().forget_folder(path);
        self.emit(MailEvent::FolderChanged { folder: path.to_string() });
        Ok(())
    }

    pub async fn mark_folder_read(&self, path: &str) -> Result<()> {
        with_conn!(self.pool, true, |conn| {
            conn.select(path).await?;
            conn.store_flags_all(&["\\Seen"]).await
        })
    }

    pub async fn counts(&self, paths: &[String]) -> Result<Vec<FolderCount>> {
        let paths = paths.to_vec();
        with_conn!(self.pool, true, |conn| Ok(conn.counts(&paths).await))
    }

    // ---- messages ------------------------------------------------------

    pub async fn list(&self, folder: &str, offset: u32, limit: u32, sort: &Sort, filter: &Filter) -> Result<MessagePage> {
        let limit = limit.clamp(1, 500);
        with_conn!(self.pool, true, |conn| {
            let sel = conn.select(folder).await?;
            let uids = conn.sorted_uids(sort, filter).await?;
            let page: Vec<u32> = uids.iter().skip(offset as usize).take(limit as usize).copied().collect();
            let items = conn.fetch_summaries(folder, &page).await?;
            Ok(MessagePage { folder: folder.to_string(), uid_validity: sel.uid_validity, total: uids.len() as u32, offset, items, uids })
        })
    }

    /// Summaries for UIDs the UI already knows (rows scrolled into view).
    pub async fn summaries(&self, folder: &str, uids: &[u32]) -> Result<Vec<MessageSummary>> {
        with_conn!(self.pool, true, |conn| {
            conn.select(folder).await?;
            conn.fetch_summaries(folder, uids).await
        })
    }

    /// Searches every folder; newest first.
    pub async fn search_all(&self, filter: &Filter) -> Result<Vec<MessageSummary>> {
        let folders: Vec<Folder> = self.folders(false).await?.into_iter().filter(|f| f.selectable).collect();
        let mut all: Vec<MessageSummary> = Vec::new();
        for f in folders {
            let path = f.path.clone();
            let found = with_conn!(self.pool, true, |conn| {
                let sel = conn.select(&path).await?;
                if sel.exists == 0 {
                    return Ok(Vec::new());
                }
                let mut uids = conn.search_uids(filter).await?;
                uids.sort_unstable_by(|a, b| b.cmp(a));
                uids.truncate(SEARCH_ALL_LIMIT);
                conn.fetch_summaries(&path, &uids).await
            });
            match found {
                Ok(items) => all.extend(items),
                Err(e) if e.is_connection_error() => return Err(e),
                Err(_) => {}
            }
        }
        all.sort_by_key(|m| std::cmp::Reverse(m.date));
        all.truncate(SEARCH_ALL_LIMIT);
        Ok(all)
    }

    async fn raw_and_flags(&self, folder: &str, uid: u32) -> Result<(Arc<Vec<u8>>, Flags)> {
        with_conn!(self.pool, true, |conn| {
            let sel = conn.select(folder).await?;
            let key = (folder.to_string(), sel.uid_validity, uid);
            let cached = self.cache.lock().get(&key);
            if let Some(raw) = cached {
                let flags = conn.fetch_summaries(folder, &[uid]).await?.into_iter().next().map(|s| s.flags);
                return match flags {
                    Some(flags) => Ok((raw, flags)),
                    None => Err(Error::not_found("The message no longer exists. It may have been moved or deleted.")),
                };
            }
            let (raw, flags) = conn.fetch_raw(uid).await?;
            let raw = Arc::new(raw);
            self.cache.lock().put(key, raw.clone());
            Ok((raw, flags))
        })
    }

    pub async fn raw(&self, folder: &str, uid: u32) -> Result<Arc<Vec<u8>>> {
        Ok(self.raw_and_flags(folder, uid).await?.0)
    }

    pub async fn message(&self, folder: &str, uid: u32, allow_remote: bool, mark_seen: bool) -> Result<Message> {
        let (raw, flags) = self.raw_and_flags(folder, uid).await?;
        let mut message = mime::parse_message(&raw, folder, uid, flags, &ReadOptions { allow_remote })?;
        if mark_seen && !message.summary.flags.seen {
            self.set_flags(folder, &[uid], &FlagChange { seen: Some(true), ..Default::default() }).await?;
            message.summary.flags.seen = true;
        }
        Ok(message)
    }

    /// The conversation of a message: messages of the same folder and of Sent
    /// linked by Message-ID and References, oldest first. Empty when the
    /// message stands alone.
    pub async fn thread(&self, folder: &str, uid: u32) -> Result<Vec<MessageSummary>> {
        let raw = self.raw(folder, uid).await?;
        let head = mime::summary_from_headers(&raw, folder, uid, 0, None, Flags::default(), false);
        let mut ids: Vec<String> = head.references.clone();
        ids.extend(head.in_reply_to.clone());
        ids.extend(head.message_id.clone());
        ids.dedup();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut folders = vec![folder.to_string()];
        if let Some(sent) = self.role_path(FolderRole::Sent).await?
            && sent != folder
        {
            folders.push(sent);
        }
        if let Some(inbox) = Some("INBOX".to_string()).filter(|i| !folders.contains(i)) {
            folders.push(inbox);
        }
        let mut found: Vec<MessageSummary> = Vec::new();
        for f in folders {
            let ids = ids.clone();
            let items = with_conn!(self.pool, true, |conn| {
                let sel = conn.select(&f).await?;
                if sel.exists == 0 {
                    return Ok(Vec::new());
                }
                let uids = conn.related_uids(&ids).await?;
                conn.fetch_summaries(&f, &uids).await
            });
            if let Ok(items) = items {
                found.extend(items);
            }
        }
        // The same message can be in two folders (a copy in Sent and INBOX).
        let mut seen = std::collections::HashSet::new();
        found.retain(|m| m.message_id.as_ref().map(|id| seen.insert(id.clone())).unwrap_or(true));
        found.sort_by_key(|m| m.date);
        Ok(if found.len() > 1 { found } else { Vec::new() })
    }

    /// An image attachment as a `data:` URL for previews (at most 8 MB).
    pub async fn attachment_data(&self, folder: &str, uid: u32, part: &str) -> Result<String> {
        let (_, ct, bytes) = self.attachment(folder, uid, part).await?;
        if !ct.starts_with("image/") || ct == "image/svg+xml" {
            return Err(Error::invalid("Only images have a preview."));
        }
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(Error::new(ErrorKind::TooLarge, "The image is too large for a preview."));
        }
        Ok(format!("data:{ct};base64,{}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)))
    }

    pub async fn attachment(&self, folder: &str, uid: u32, part: &str) -> Result<(String, String, Vec<u8>)> {
        let raw = self.raw(folder, uid).await?;
        mime::extract_part(&raw, part)
    }

    /// File name for downloading a whole message.
    pub fn eml_name(raw: &[u8]) -> String {
        let subject = mime::subject_of(raw);
        let clean: String = subject.chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
        let clean = clean.trim();
        format!("{}.eml", if clean.is_empty() { "message" } else { clean })
    }

    pub async fn set_flags(&self, folder: &str, uids: &[u32], change: &FlagChange) -> Result<()> {
        let mut add: Vec<&str> = Vec::new();
        let mut remove: Vec<&str> = Vec::new();
        for (value, flag) in
            [(change.seen, "\\Seen"), (change.flagged, "\\Flagged"), (change.answered, "\\Answered"), (change.forwarded, "$Forwarded")]
        {
            match value {
                Some(true) => add.push(flag),
                Some(false) => remove.push(flag),
                None => {}
            }
        }
        if uids.is_empty() || (add.is_empty() && remove.is_empty()) {
            return Ok(());
        }
        with_conn!(self.pool, true, |conn| {
            conn.select(folder).await?;
            conn.store_flags(uids, &add, &remove).await
        })
    }

    pub async fn move_messages(&self, folder: &str, uids: &[u32], dest: &str) -> Result<Moved> {
        if folder == dest || uids.is_empty() {
            return Ok(Moved { folder: dest.to_string(), uids: Vec::new() });
        }
        let created = with_conn!(self.pool, false, |conn| {
            conn.select(folder).await?;
            conn.move_uids(uids, dest).await
        })?;
        self.emit(MailEvent::FolderChanged { folder: dest.to_string() });
        Ok(Moved { folder: dest.to_string(), uids: created })
    }

    pub async fn copy_messages(&self, folder: &str, uids: &[u32], dest: &str) -> Result<Moved> {
        let created = with_conn!(self.pool, false, |conn| {
            conn.select(folder).await?;
            conn.copy_uids(uids, dest).await
        })?;
        self.emit(MailEvent::FolderChanged { folder: dest.to_string() });
        Ok(Moved { folder: dest.to_string(), uids: created })
    }

    /// Moves to Trash, or deletes for good inside Trash, when `permanent`, or
    /// when there is no Trash. Returns where the messages went, if anywhere.
    pub async fn delete_messages(&self, folder: &str, uids: &[u32], permanent: bool) -> Result<Option<Moved>> {
        let trash = if permanent { None } else { self.role_path(FolderRole::Trash).await? };
        match trash {
            Some(trash) if trash != folder => self.move_messages(folder, uids, &trash).await.map(Some),
            _ => {
                with_conn!(self.pool, false, |conn| {
                    conn.select(folder).await?;
                    conn.delete_uids(uids).await
                })?;
                Ok(None)
            }
        }
    }

    pub async fn archive(&self, folder: &str, uids: &[u32]) -> Result<Moved> {
        let archive = self.ensure_role(FolderRole::Archive).await?;
        self.move_messages(folder, uids, &archive).await
    }

    /// Marks as spam (moves to Junk) or as not spam (back to the inbox).
    pub async fn junk(&self, folder: &str, uids: &[u32], junk: bool) -> Result<Moved> {
        let dest = if junk { self.ensure_role(FolderRole::Junk).await? } else { "INBOX".to_string() };
        let (add, remove) = if junk { ("$Junk", "$NotJunk") } else { ("$NotJunk", "$Junk") };
        with_conn!(self.pool, true, |conn| {
            conn.select(folder).await?;
            // Servers without keywords refuse; the move still matters.
            let _ = conn.store_flags(uids, &[add], &[remove]).await;
            Ok(())
        })?;
        self.move_messages(folder, uids, &dest).await
    }

    pub async fn quota(&self) -> Result<Option<Quota>> {
        let q = with_conn!(self.pool, true, |conn| conn.quota().await)?;
        Ok(q.map(|(used, limit)| Quota { used, limit }))
    }

    // ---- uploads -------------------------------------------------------

    pub fn add_upload(&self, filename: &str, content_type: &str, bytes: Vec<u8>) -> Result<Upload> {
        if bytes.len() > MAX_UPLOAD {
            return Err(Error::new(ErrorKind::TooLarge, format!("Files can have at most {} MB.", MAX_UPLOAD / 1024 / 1024)));
        }
        let mut uploads = self.uploads.lock();
        uploads.retain(|_, u| u.created.elapsed() < UPLOAD_TTL);
        let total: usize = uploads.values().map(|u| u.bytes.len()).sum();
        if total + bytes.len() > MAX_UPLOADS_TOTAL {
            return Err(Error::new(ErrorKind::TooLarge, "Too many attached files. Send or discard some messages first."));
        }
        let filename = filename.replace(['/', '\\', '\0'], "_");
        let content_type = if content_type.trim().is_empty() || !content_type.contains('/') {
            guess_type(&filename).to_string()
        } else {
            content_type.to_ascii_lowercase()
        };
        let info = Upload { id: uuid::Uuid::new_v4().to_string(), filename, content_type, size: bytes.len() as u64 };
        uploads.insert(info.id.clone(), StoredUpload { info: info.clone(), bytes: Arc::new(bytes), created: Instant::now() });
        Ok(info)
    }

    pub fn upload_bytes(&self, id: &str) -> Option<(Upload, Arc<Vec<u8>>)> {
        self.uploads.lock().get(id).map(|u| (u.info.clone(), u.bytes.clone()))
    }

    pub fn delete_upload(&self, id: &str) {
        self.uploads.lock().remove(id);
    }

    async fn resolve_attachments(&self, list: &[DraftAttachment]) -> Result<Vec<ResolvedAttachment>> {
        let mut out = Vec::new();
        for a in list {
            let bytes = match &a.source {
                AttachmentSource::Upload { id } => {
                    let (_, bytes) = self
                        .upload_bytes(id)
                        .ok_or_else(|| Error::not_found(format!("The attached file \"{}\" expired. Attach it again.", a.filename)))?;
                    bytes.as_ref().clone()
                }
                AttachmentSource::Part { folder, uid, part } => self.attachment(folder, *uid, part).await?.2,
                AttachmentSource::Message { folder, uid } => self.raw(folder, *uid).await?.as_ref().clone(),
            };
            out.push(ResolvedAttachment {
                filename: a.filename.clone(),
                content_type: a.content_type.clone(),
                bytes,
                inline: a.inline,
                content_id: a.content_id.clone(),
            });
        }
        Ok(out)
    }

    // ---- composing -----------------------------------------------------

    pub fn identities(&self) -> Result<Vec<Identity>> {
        let a = self.account();
        self.store.identities_or_default(&self.owner, &a.email, &a.name)
    }

    pub async fn prepare(&self, mode: ComposeMode, source: Option<MessageRef>, html: bool, style: &QuoteStyle) -> Result<Draft> {
        let identities = self.identities()?;
        let Some(source) = source.filter(|_| mode != ComposeMode::New) else {
            let identity = identities.iter().find(|i| i.is_default).or(identities.first());
            return Ok(Draft {
                mode: ComposeMode::New,
                identity_id: identity.map(|i| i.id.clone()),
                from: identity.map(|i| Address::new(Some(i.name.clone()), i.email.clone())),
                html: html.then(String::new),
                ..Default::default()
            });
        };
        let raw = self.raw(&source.folder, source.uid).await?;
        let mut original =
            mime::parse_message(&raw, &source.folder, source.uid, Flags::default(), &ReadOptions { allow_remote: style.remote_images })?;
        original.html = original.html.map(|h| sanitize::strip_style_blocks(&h));
        let mut draft = compose::prepare(mode, &original, source.clone(), &identities, style, html);

        let keep_files = matches!(mode, ComposeMode::Forward | ComposeMode::EditAsNew | ComposeMode::Draft);
        if keep_files {
            for a in original.attachments.iter().filter(|a| !a.inline) {
                let (filename, content_type, bytes) = mime::extract_part(&raw, &a.part)?;
                let up = self.add_upload(&filename, &content_type, bytes)?;
                draft.attachments.push(DraftAttachment {
                    source: AttachmentSource::Upload { id: up.id },
                    filename: up.filename,
                    content_type: up.content_type,
                    size: up.size,
                    inline: false,
                    content_id: None,
                });
            }
        }
        if mode == ComposeMode::ForwardAttachment {
            let name = Self::eml_name(&raw);
            let up = self.add_upload(&name, "message/rfc822", raw.as_ref().clone())?;
            draft.attachments.push(DraftAttachment {
                source: AttachmentSource::Upload { id: up.id },
                filename: up.filename,
                content_type: up.content_type,
                size: up.size,
                inline: false,
                content_id: None,
            });
        }
        if mode == ComposeMode::Draft {
            draft.draft_ref = Some(source);
        }
        Ok(draft)
    }

    fn sender(&self, draft: &Draft) -> Result<(Address, Option<Identity>)> {
        let identities = self.identities()?;
        let identity = draft
            .identity_id
            .as_ref()
            .and_then(|id| identities.iter().find(|i| &i.id == id))
            .or_else(|| draft.from.as_ref().and_then(|f| identities.iter().find(|i| i.email.eq_ignore_ascii_case(&f.email))))
            .or_else(|| identities.iter().find(|i| i.is_default))
            .cloned();
        let from = match (&identity, &draft.from) {
            (Some(i), _) => Address::new(Some(i.name.clone()), i.email.clone()),
            (None, Some(f)) => f.clone(),
            (None, None) => Address::new(Some(self.account().name.clone()), self.account().email.clone()),
        };
        Ok((from, identity))
    }

    fn meta(draft: &Draft, identity: &Option<Identity>) -> DraftMeta {
        DraftMeta {
            identity_id: identity.as_ref().map(|i| i.id.clone()),
            source: draft.source.clone(),
            mode: draft.mode,
            html: draft.html.is_some(),
            request_receipt: draft.request_receipt,
        }
    }

    /// Appends a message to `folder` and returns its UID when known. A
    /// connection the server dropped is retried once on a new one, after
    /// checking by Message-ID that the first try did not land, so the
    /// message is never stored twice.
    async fn append_unique(&self, folder: &str, flags: &[&str], raw: &[u8], message_id: &str) -> Result<Option<u32>> {
        let mut retry = false;
        loop {
            let result = with_conn!(self.pool, false, |conn| {
                if retry {
                    conn.select(folder).await?;
                    if let Some(uid) = conn.find_by_message_id(message_id).await? {
                        return Ok(Some(uid));
                    }
                }
                match conn.append(folder, flags, raw).await? {
                    Some((_, uid)) => Ok(Some(uid)),
                    None => {
                        conn.select(folder).await?;
                        conn.find_by_message_id(message_id).await
                    }
                }
            });
            match result {
                Err(e) if e.is_connection_error() && !retry => retry = true,
                other => return other,
            }
        }
    }

    /// Saves the draft in Drafts (replacing the previous save).
    pub async fn save_draft(&self, mut draft: Draft) -> Result<Draft> {
        let (from, identity) = self.sender(&draft)?;
        let files = self.resolve_attachments(&draft.attachments).await?;
        let meta = Self::meta(&draft, &identity);
        let built = compose::build(&draft, &from, &files, true, Some(&meta))?;
        let drafts = self.ensure_role(FolderRole::Drafts).await?;
        let (raw, message_id) = (built.raw, built.message_id.clone());
        let uid = self.append_unique(&drafts, &["\\Draft", "\\Seen"], &raw, &message_id).await?;
        if let Some(old) = draft.draft_ref.take()
            && !(old.folder == drafts && Some(old.uid) == uid)
        {
            let _ = self.delete_messages(&old.folder, &[old.uid], true).await;
        }
        draft.message_id = Some(built.message_id);
        draft.draft_ref = uid.map(|uid| MessageRef { folder: drafts.clone(), uid });
        draft.from = Some(from);
        self.emit(MailEvent::FolderChanged { folder: drafts });
        Ok(draft)
    }

    pub async fn discard_draft(&self, draft_ref: &MessageRef) -> Result<()> {
        self.delete_messages(&draft_ref.folder, &[draft_ref.uid], true).await.map(|_| ())
    }

    pub async fn send(&self, mut draft: Draft, save_copy: bool) -> Result<SendResult> {
        let (from, identity) = self.sender(&draft)?;
        if let Some(i) = &identity {
            if let Some(bcc) = i.bcc.as_ref().map(|b| b.trim()).filter(|b| !b.is_empty())
                && !draft.bcc.iter().any(|a| a.email.eq_ignore_ascii_case(bcc))
            {
                draft.bcc.push(Address::new(None, bcc));
            }
            if draft.reply_to.is_empty()
                && let Some(rt) = i.reply_to.as_ref().map(|r| r.trim()).filter(|r| !r.is_empty())
            {
                draft.reply_to.push(Address::new(None, rt));
            }
        }
        let recipients = compose::envelope_recipients(&draft);
        if recipients.is_empty() {
            return Err(Error::invalid("Add at least one recipient."));
        }
        for r in &recipients {
            compose::validate_address(r)?;
        }
        compose::validate_address(&from.email)?;
        let files = self.resolve_attachments(&draft.attachments).await?;
        let outgoing = compose::build(&draft, &from, &files, false, None)?;
        draft.message_id = Some(outgoing.message_id.clone());
        smtp::send(self.account(), self.password(), &from.email, &recipients, &outgoing.raw).await?;

        let mut warnings = Vec::new();
        let mut saved = false;
        if save_copy {
            let copy = compose::build(&draft, &from, &files, true, None)?;
            match self.ensure_role(FolderRole::Sent).await {
                Ok(sent) => match self.append_unique(&sent, &["\\Seen"], &copy.raw, &copy.message_id).await {
                    Ok(_) => {
                        saved = true;
                        self.emit(MailEvent::FolderChanged { folder: sent });
                    }
                    Err(e) => {
                        tracing::warn!("copy in Sent not saved: {}", e.detail.unwrap_or(e.message));
                        warnings.push("sentCopyFailed".into());
                    }
                },
                Err(e) => {
                    tracing::warn!("no Sent folder: {}", e.detail.unwrap_or(e.message));
                    warnings.push("noSentFolder".into());
                }
            }
        }
        if let Some(source) = &draft.source {
            let change = match draft.mode {
                ComposeMode::Reply | ComposeMode::ReplyAll => Some(FlagChange { answered: Some(true), ..Default::default() }),
                ComposeMode::Forward | ComposeMode::ForwardAttachment => Some(FlagChange { forwarded: Some(true), ..Default::default() }),
                _ => None,
            };
            if let Some(change) = change {
                let _ = self.set_flags(&source.folder, &[source.uid], &change).await;
            }
        }
        if let Some(old) = &draft.draft_ref {
            let _ = self.delete_messages(&old.folder, &[old.uid], true).await;
        }
        let mut people = draft.to.clone();
        people.extend(draft.cc.iter().cloned());
        people.extend(draft.bcc.iter().cloned());
        let _ = self.store.collect(&self.owner, &people);
        for a in &draft.attachments {
            if let AttachmentSource::Upload { id } = &a.source {
                self.delete_upload(id);
            }
        }
        Ok(SendResult { message_id: outgoing.message_id, saved, warnings })
    }

    /// Sends the read receipt a message asked for (RFC 8098) and flags it.
    pub async fn send_receipt(&self, folder: &str, uid: u32) -> Result<()> {
        let message = self.message(folder, uid, false, false).await?;
        let to = message.receipt_to.clone().ok_or_else(|| Error::invalid("The message did not ask for a read receipt."))?;
        let identities = self.identities()?;
        let identity = compose::pick_identity(&message, &identities).cloned();
        let from = identity
            .map(|i| Address::new(Some(i.name), i.email))
            .unwrap_or_else(|| Address::new(Some(self.account().name.clone()), self.account().email.clone()));
        let raw = build_receipt(&message, &from, &to)?;
        smtp::send(self.account(), self.password(), &from.email, std::slice::from_ref(&to.email), &raw).await?;
        with_conn!(self.pool, true, |conn| {
            conn.select(folder).await?;
            conn.store_flags(&[uid], &["$MDNSent"], &[]).await
        })
    }

    // ---- filters -------------------------------------------------------

    // Each call opens its own ManageSieve connection, and reading or writing
    // the same script twice is harmless, so a dropped connection is retried.

    pub async fn filters(&self) -> Result<sieve::FilterState> {
        match sieve::load(self.account(), self.password()).await {
            Err(e) if e.is_connection_error() => sieve::load(self.account(), self.password()).await,
            other => other,
        }
    }

    pub async fn save_filters(&self, filters: &sieve::Filters) -> Result<()> {
        match sieve::save(self.account(), self.password(), filters).await {
            Err(e) if e.is_connection_error() => sieve::save(self.account(), self.password(), filters).await,
            other => other,
        }
    }
}

fn build_receipt(message: &Message, from: &Address, to: &Address) -> Result<Vec<u8>> {
    let subject = format!("Read: {}", message.summary.subject);
    let original_id = message.summary.message_id.clone().unwrap_or_default();
    let domain = from.email.rsplit('@').next().unwrap_or("lowmail.local");
    let boundary = format!("lm-{}", uuid::Uuid::new_v4().simple());
    let date = chrono::Utc::now().to_rfc2822();
    let text = format!("This is a receipt for the message you sent to {}.\r\nIt was displayed on {}.\r\n", from.email, date);
    let report = format!(
        "Reporting-UA: {domain}; LowMail\r\nFinal-Recipient: rfc822;{}\r\nOriginal-Message-ID: <{original_id}>\r\nDisposition: manual-action/MDN-sent-manually; displayed\r\n",
        from.email
    );
    let encode = |s: &str| {
        if s.is_ascii() {
            s.to_string()
        } else {
            format!("=?UTF-8?B?{}?=", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, s))
        }
    };
    let raw = format!(
        "From: {}\r\nTo: {}\r\nSubject: {}\r\nDate: {date}\r\nMessage-ID: <{}@{domain}>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/report; report-type=disposition-notification; boundary=\"{boundary}\"\r\n\r\n--{boundary}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n--{boundary}\r\nContent-Type: message/disposition-notification\r\n\r\n{report}\r\n--{boundary}--\r\n",
        mime::format_address(&Address::new(from.name.as_ref().map(|n| encode(n)), from.email.clone())),
        to.email,
        encode(&subject),
        uuid::Uuid::new_v4(),
    );
    Ok(raw.into_bytes())
}

/// Content type from a file name, for uploads that arrive without one.
pub fn guess_type(filename: &str) -> &'static str {
    let ext = filename.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "ics" => "text/calendar",
        "vcf" => "text/vcard",
        "eml" => "message/rfc822",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "json" => "application/json",
        "xml" => "application/xml",
        _ => "application/octet-stream",
    }
}

/// Checks that the IMAP and SMTP servers accept the account's sign-in.
pub async fn test_account(account: &AccountConfig, password: &str) -> Result<()> {
    let conn = crate::imap::Conn::open(account, password).await?;
    conn.logout().await;
    smtp::test(account, password).await.map_err(|e| {
        let mut e = e;
        e.message = format!("Incoming mail works, but the outgoing server failed: {}", e.message);
        e
    })
}

#[cfg(test)]
mod send_check {
    use super::*;
    fn assert_send<T: Send>(_: T) {}
    #[allow(dead_code)]
    fn futures_are_send(c: Arc<MailClient>) {
        let f = Filter::default();
        let s = Sort::default();
        assert_send(async move {
            let _ = c.message("INBOX", 1, false, false).await;
            let _ = c.list("INBOX", 0, 10, &s, &f).await;
            let _ = c.folders(true).await;
            let _ = c.search_all(&f).await;
            let _ = c.prepare(ComposeMode::Reply, None, true, &QuoteStyle::default()).await;
            let _ = c.save_draft(Draft::default()).await;
            let _ = c.send(Draft::default(), true).await;
            let _ = c.send_receipt("INBOX", 1).await;
            let _ = c.delete_folder("x").await;
            let _ = c.rename_folder("x", "y", None).await;
            let _ = c.create_folder(None, "x").await;
            let _ = c.junk("INBOX", &[1], true).await;
            let _ = c.archive("INBOX", &[1]).await;
            let _ = c.filters().await;
            let _ = c.quota().await;
            let _ = c.counts(&[]).await;
            let _ = crate::rpc::dispatch(&c, "x", serde_json::Value::Null).await;
        });
    }
}
