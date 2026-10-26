//! Types shared by the engine, the web server, the desktop app and the UI
//! (serialized in camelCase; see `src/lib/types.ts`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Security {
    /// TLS from the first byte (IMAPS 993, SMTPS 465).
    #[default]
    Tls,
    /// Plain connection upgraded with STARTTLS (IMAP 143, submission 587, Sieve 4190).
    StartTls,
    /// No encryption. Only for local test servers.
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub security: Security,
    /// Accept self-signed or otherwise invalid certificates.
    #[serde(default)]
    pub accept_invalid_certs: bool,
}

/// How to reach and sign in to one mailbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountConfig {
    pub id: String,
    /// Shown in the account list ("Work", "Personal").
    #[serde(default)]
    pub label: String,
    /// Display name used for the default identity.
    #[serde(default)]
    pub name: String,
    pub email: String,
    /// Sign-in name; usually the e-mail address.
    pub username: String,
    pub imap: Endpoint,
    pub smtp: Endpoint,
    /// ManageSieve (filters and vacation); `None` turns those features off.
    #[serde(default)]
    pub sieve: Option<Endpoint>,
    /// The SMTP server signs in with the same credentials (almost always true).
    #[serde(default = "yes")]
    pub smtp_auth: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FolderRole {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    /// Name on the server (modified UTF-7); the folder's identity.
    pub path: String,
    /// Last segment, decoded.
    pub name: String,
    /// Full path, decoded, segments joined by `delimiter`.
    pub display_path: String,
    /// Path of the parent folder, if any.
    pub parent: Option<String>,
    pub delimiter: Option<String>,
    pub depth: usize,
    pub role: Option<FolderRole>,
    pub subscribed: bool,
    pub selectable: bool,
    pub can_have_children: bool,
    pub total: u32,
    pub unread: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Address {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub email: String,
}

impl Address {
    pub fn new(name: Option<String>, email: impl Into<String>) -> Self {
        Self { name: name.filter(|n| !n.trim().is_empty()), email: email.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Flags {
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub forwarded: bool,
    pub draft: bool,
    pub deleted: bool,
    pub junk: bool,
    /// Other keywords (labels), as stored on the server.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Priority {
    High,
    #[default]
    Normal,
    Low,
}

/// One row of the message list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSummary {
    pub uid: u32,
    pub folder: String,
    pub message_id: Option<String>,
    pub subject: String,
    pub from: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    /// Milliseconds since the epoch (sent date, else arrival).
    pub date: i64,
    pub size: u32,
    pub flags: Flags,
    pub has_attachments: bool,
    pub priority: Priority,
    pub in_reply_to: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
    pub list_unsubscribe: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SortField {
    #[default]
    Date,
    Arrival,
    From,
    To,
    Subject,
    Size,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sort {
    #[serde(default)]
    pub field: SortField,
    #[serde(default = "yes")]
    pub descending: bool,
}

impl Default for Sort {
    /// Newest first.
    fn default() -> Self {
        Sort { field: SortField::Date, descending: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SearchScope {
    /// Sender, recipients and subject.
    #[default]
    Headers,
    From,
    To,
    Subject,
    Body,
    /// Headers and body.
    Everything,
}

/// What the message list shows. Empty means every message.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub scope: SearchScope,
    #[serde(default)]
    pub unread: bool,
    #[serde(default)]
    pub flagged: bool,
    #[serde(default)]
    pub answered: bool,
    /// Dates as `YYYY-MM-DD`, inclusive.
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub before: Option<String>,
    /// Size in bytes.
    #[serde(default)]
    pub larger: Option<u32>,
}

impl Filter {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
            && !self.unread
            && !self.flagged
            && !self.answered
            && self.since.is_none()
            && self.before.is_none()
            && self.larger.is_none()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePage {
    pub folder: String,
    pub uid_validity: u32,
    /// Messages matching the filter.
    pub total: u32,
    pub offset: u32,
    pub items: Vec<MessageSummary>,
    /// Every UID matching the filter, in list order, so the UI can select all
    /// and move through the list without loading every row.
    pub uids: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentInfo {
    /// Index of the part inside the message; stable for the same message.
    pub part: String,
    pub filename: String,
    pub content_type: String,
    pub size: u64,
    /// Shown inside the body (cid: image) rather than as a file.
    pub inline: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    #[serde(flatten)]
    pub summary: MessageSummary,
    pub reply_to: Vec<Address>,
    pub bcc: Vec<Address>,
    /// Sanitized HTML document for a sandboxed frame, when the message has
    /// HTML or the text was converted to it.
    pub html: Option<String>,
    pub text: Option<String>,
    /// The original contains remote images or styles that were blocked.
    pub remote_content: bool,
    pub attachments: Vec<AttachmentInfo>,
    /// `Disposition-Notification-To`: the sender asked for a read receipt.
    pub receipt_to: Option<Address>,
    /// `List-Unsubscribe` header value (`<mailto:...>, <https://...>`).
    pub unsubscribe: Option<String>,
    /// One-click unsubscribe (RFC 8058) is offered.
    pub unsubscribe_post: bool,
    /// Headers in order, decoded, for the "all headers" view.
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MessageRef {
    pub folder: String,
    pub uid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "camelCase")]
pub enum AttachmentSource {
    /// A file uploaded during this session.
    Upload { id: String },
    /// A part of a message on the server (forwarding, reopening a draft).
    Part { folder: String, uid: u32, part: String },
    /// A whole message attached as `message/rfc822`.
    Message { folder: String, uid: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftAttachment {
    #[serde(flatten)]
    pub source: AttachmentSource,
    pub filename: String,
    pub content_type: String,
    #[serde(default)]
    pub size: u64,
    /// Inline image referenced from the HTML as `cid:<content_id>`.
    #[serde(default)]
    pub inline: bool,
    #[serde(default)]
    pub content_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ComposeMode {
    #[default]
    New,
    Reply,
    ReplyAll,
    Forward,
    ForwardAttachment,
    /// Reopen a sent or received message as a new one.
    EditAsNew,
    /// Continue a draft saved on the server.
    Draft,
}

/// A message being written. Saved as a draft or sent.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    #[serde(default)]
    pub mode: ComposeMode,
    #[serde(default)]
    pub identity_id: Option<String>,
    pub from: Option<Address>,
    #[serde(default)]
    pub to: Vec<Address>,
    #[serde(default)]
    pub cc: Vec<Address>,
    #[serde(default)]
    pub bcc: Vec<Address>,
    #[serde(default)]
    pub reply_to: Vec<Address>,
    #[serde(default)]
    pub subject: String,
    /// HTML body; `None` sends plain text only.
    #[serde(default)]
    pub html: Option<String>,
    /// Plain-text body. Derived from `html` when it is empty and HTML is set.
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<DraftAttachment>,
    /// The message this one answers or forwards.
    #[serde(default)]
    pub source: Option<MessageRef>,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default)]
    pub request_receipt: bool,
    /// The saved draft this replaces, removed after saving or sending.
    #[serde(default)]
    pub draft_ref: Option<MessageRef>,
    /// Message-ID, kept across saves of the same draft.
    #[serde(default)]
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub email: String,
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub bcc: Option<String>,
    /// HTML signature.
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(default)]
    pub phones: Vec<String>,
    #[serde(default)]
    pub organization: String,
    #[serde(default)]
    pub job_title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub birthday: Option<String>,
    #[serde(default)]
    pub groups: Vec<String>,
    /// Added automatically from sent messages, not by the user.
    #[serde(default)]
    pub collected: bool,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Quota {
    /// Bytes.
    pub used: u64,
    pub limit: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Upload {
    pub id: String,
    pub filename: String,
    pub content_type: String,
    pub size: u64,
}

/// Something the UI should react to without asking.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MailEvent {
    /// Messages arrived in a folder; `latest` are the newest unread ones.
    NewMail { folder: String, unread: u32, latest: Vec<MessageSummary> },
    /// Something changed in a folder (flags, removals, other clients).
    FolderChanged { folder: String },
    /// Counters of several folders changed.
    Counts { folders: Vec<FolderCount> },
    /// The connection to the server was lost or came back.
    Connection { online: bool },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderCount {
    pub path: String,
    pub total: u32,
    pub unread: u32,
}
