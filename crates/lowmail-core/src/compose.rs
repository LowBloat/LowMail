//! Writing messages: replies and forwards with quotes, drafts, and the MIME
//! message that is sent and saved.

use base64::Engine;
use chrono::{FixedOffset, TimeZone};
use mail_builder::MessageBuilder;
use mail_builder::headers::address::Address as MbAddress;
use mail_builder::headers::content_type::ContentType;
use mail_builder::headers::message_id::MessageId;
use mail_builder::headers::raw::Raw;
use mail_builder::headers::text::Text;
use mail_builder::mime::MimePart;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::mime::format_address;
use crate::model::{Address, ComposeMode, Draft, Identity, Message, MessageRef, Priority};
use crate::sanitize::{escape, html_to_text};

/// Header holding what a reopened draft needs (identity, original message).
pub const DRAFT_HEADER: &str = "X-LowMail-Draft";

/// A file ready to be put into a message.
#[derive(Debug, Clone)]
pub struct ResolvedAttachment {
    pub filename: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
    pub inline: bool,
    pub content_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DraftMeta {
    #[serde(default)]
    pub identity_id: Option<String>,
    #[serde(default)]
    pub source: Option<MessageRef>,
    #[serde(default)]
    pub mode: ComposeMode,
    #[serde(default)]
    pub html: bool,
    #[serde(default)]
    pub request_receipt: bool,
}

/// How quotes are introduced, in the reader's language and time zone.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteStyle {
    #[serde(default = "default_locale")]
    pub locale: String,
    /// Minutes east of UTC.
    #[serde(default)]
    pub tz_offset_minutes: i32,
    /// Reply above the quote (true) or below it.
    #[serde(default = "yes")]
    pub reply_on_top: bool,
}

fn default_locale() -> String {
    "pt-BR".into()
}

fn yes() -> bool {
    true
}

impl Default for QuoteStyle {
    fn default() -> Self {
        QuoteStyle { locale: default_locale(), tz_offset_minutes: 0, reply_on_top: true }
    }
}

struct Words {
    wrote: &'static str,
    forwarded: &'static str,
    from: &'static str,
    date: &'static str,
    subject: &'static str,
    to: &'static str,
    cc: &'static str,
}

fn words(locale: &str) -> Words {
    if locale.to_ascii_lowercase().starts_with("pt") {
        Words {
            wrote: "escreveu",
            forwarded: "---------- Mensagem encaminhada ----------",
            from: "De",
            date: "Data",
            subject: "Assunto",
            to: "Para",
            cc: "Cc",
        }
    } else {
        Words {
            wrote: "wrote",
            forwarded: "---------- Forwarded message ----------",
            from: "From",
            date: "Date",
            subject: "Subject",
            to: "To",
            cc: "Cc",
        }
    }
}

fn format_date(ms: i64, style: &QuoteStyle) -> String {
    let offset = FixedOffset::east_opt(style.tz_offset_minutes * 60).unwrap_or(FixedOffset::east_opt(0).unwrap());
    let Some(dt) = offset.timestamp_millis_opt(ms).single() else { return String::new() };
    if style.locale.to_ascii_lowercase().starts_with("pt") {
        dt.format("%d/%m/%Y %H:%M").to_string()
    } else {
        dt.format("%b %-d, %Y, %H:%M").to_string()
    }
}

/// Subject with one `Re:`/`Fwd:` prefix, whatever the original carried
/// (including Outlook's Portuguese `RES:` and `ENC:`).
pub fn prefixed_subject(subject: &str, prefix: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lower = s.to_ascii_lowercase();
        let stripped = ["re:", "res:", "fwd:", "fw:", "enc:", "tr:", "aw:", "wg:", "rif:"]
            .iter()
            .find(|p| lower.starts_with(*p))
            .map(|p| s[p.len()..].trim_start());
        match stripped {
            Some(rest) => s = rest,
            None => break,
        }
    }
    format!("{prefix} {s}")
}

fn is_mine(address: &str, identities: &[Identity]) -> bool {
    identities.iter().any(|i| i.email.eq_ignore_ascii_case(address))
}

fn dedupe(list: Vec<Address>, exclude: &[Address], identities: &[Identity]) -> Vec<Address> {
    let mut out: Vec<Address> = Vec::new();
    for a in list {
        let taken = out.iter().chain(exclude.iter()).any(|o| o.email.eq_ignore_ascii_case(&a.email));
        if !taken && !is_mine(&a.email, identities) {
            out.push(a);
        }
    }
    out
}

/// The identity a reply should come from: the one the original was sent to.
pub fn pick_identity<'a>(original: &Message, identities: &'a [Identity]) -> Option<&'a Identity> {
    let recipients = original.summary.to.iter().chain(original.summary.cc.iter()).chain(original.bcc.iter());
    for r in recipients {
        if let Some(i) = identities.iter().find(|i| i.email.eq_ignore_ascii_case(&r.email)) {
            return Some(i);
        }
    }
    // Messages in Sent: the sender is one of us.
    for f in &original.summary.from {
        if let Some(i) = identities.iter().find(|i| i.email.eq_ignore_ascii_case(&f.email)) {
            return Some(i);
        }
    }
    identities.iter().find(|i| i.is_default).or(identities.first())
}

fn quote_text(text: &str) -> String {
    text.lines().map(|l| if l.starts_with('>') { format!(">{l}") } else { format!("> {l}") }).collect::<Vec<_>>().join("\n")
}

fn address_line(list: &[Address]) -> String {
    list.iter().map(format_address).collect::<Vec<_>>().join(", ")
}

/// A reply, reply-all, forward or "edit as new" draft for `original`.
pub fn prepare(
    mode: ComposeMode,
    original: &Message,
    source: MessageRef,
    identities: &[Identity],
    style: &QuoteStyle,
    html: bool,
) -> Draft {
    let identity = pick_identity(original, identities);
    let from = identity.map(|i| Address::new(Some(i.name.clone()), i.email.clone()));
    let w = words(&style.locale);
    let s = &original.summary;
    let mut draft = Draft { mode, identity_id: identity.map(|i| i.id.clone()), from, source: Some(source), ..Default::default() };
    let original_html = original.html.clone().unwrap_or_default();
    let original_text = original.text.clone().unwrap_or_else(|| html_to_text(&original_html));
    let sender = s.from.first().map(format_address).unwrap_or_default();
    let date = format_date(s.date, style);

    match mode {
        ComposeMode::Reply | ComposeMode::ReplyAll => {
            let sent_by_me = s.from.iter().any(|f| is_mine(&f.email, identities));
            let primary: Vec<Address> = if sent_by_me {
                s.to.clone()
            } else if !original.reply_to.is_empty() {
                original.reply_to.clone()
            } else {
                s.from.clone()
            };
            draft.to = primary.clone();
            if mode == ComposeMode::ReplyAll {
                let mut others = s.to.clone();
                others.extend(s.cc.clone());
                if !sent_by_me {
                    others.extend(s.from.clone());
                }
                draft.cc = dedupe(others, &primary, identities);
            }
            draft.subject = prefixed_subject(&s.subject, "Re:");
            draft.in_reply_to = s.message_id.clone();
            let mut refs = s.references.clone();
            if refs.is_empty()
                && let Some(parent) = &s.in_reply_to
            {
                refs.push(parent.clone());
            }
            if let Some(id) = &s.message_id {
                refs.push(id.clone());
            }
            draft.references = refs;
            let header = format!("{} {}, {} {}:", if style.locale.starts_with("pt") { "Em" } else { "On" }, date, sender, w.wrote);
            if html {
                let quote = format!(
                    "<div class=\"lm-quote-header\">{}</div><blockquote type=\"cite\" style=\"margin:0 0 0 .8ex;border-left:2px solid #a9b1a5;padding-left:1ex\">{}</blockquote>",
                    escape(&header),
                    original_html
                );
                draft.html =
                    Some(if style.reply_on_top { format!("<p><br></p><p><br></p>{quote}") } else { format!("{quote}<p><br></p>") });
                draft.text = String::new();
            } else {
                let quote = format!("{header}\n{}", quote_text(&original_text));
                draft.text = if style.reply_on_top { format!("\n\n{quote}") } else { format!("{quote}\n\n") };
            }
        }
        ComposeMode::Forward | ComposeMode::ForwardAttachment => {
            draft.subject = prefixed_subject(&s.subject, "Fwd:");
            draft.references = s.message_id.iter().cloned().collect();
            if mode == ComposeMode::Forward {
                let mut lines = vec![
                    (w.from, address_line(&s.from)),
                    (w.date, date.clone()),
                    (w.subject, s.subject.clone()),
                    (w.to, address_line(&s.to)),
                ];
                if !s.cc.is_empty() {
                    lines.push((w.cc, address_line(&s.cc)));
                }
                if html {
                    let rows: String = lines.iter().map(|(k, v)| format!("<b>{}:</b> {}<br>", escape(k), escape(v))).collect();
                    draft.html = Some(format!(
                        "<p><br></p><p><br></p><div class=\"lm-forward\">{}<br>{rows}<br></div>{original_html}",
                        escape(w.forwarded)
                    ));
                } else {
                    let rows: String = lines.iter().map(|(k, v)| format!("{k}: {v}\n")).collect();
                    draft.text = format!("\n\n{}\n{rows}\n{original_text}", w.forwarded);
                }
            } else if html {
                draft.html = Some("<p><br></p>".into());
            }
        }
        ComposeMode::EditAsNew | ComposeMode::Draft | ComposeMode::New => {
            draft.to = s.to.clone();
            draft.cc = s.cc.clone();
            draft.bcc = original.bcc.clone();
            draft.reply_to = original.reply_to.clone();
            draft.subject = s.subject.clone();
            draft.priority = s.priority;
            draft.source = None;
            if html && original.html.is_some() {
                draft.html = Some(original_html);
            } else {
                draft.html = if html { Some(crate::sanitize::text_to_html(&original_text)) } else { None };
                draft.text = original_text;
            }
            if mode == ComposeMode::Draft {
                draft.in_reply_to = s.in_reply_to.clone();
                draft.references = s.references.clone();
                draft.message_id = s.message_id.clone();
                draft.request_receipt = original.receipt_to.is_some();
                if let Some(meta) = draft_meta(original) {
                    draft.identity_id = meta.identity_id.or(draft.identity_id);
                    draft.source = meta.source;
                    // Sending a reopened reply still marks the original as answered.
                    draft.mode = meta.mode;
                    if !meta.html {
                        draft.html = None;
                    }
                    draft.request_receipt = meta.request_receipt;
                }
                // The sender saved in the draft wins over the guessed identity.
                if let Some(f) = s.from.first() {
                    draft.from = Some(f.clone());
                    if let Some(i) = identities.iter().find(|i| i.email.eq_ignore_ascii_case(&f.email)) {
                        draft.identity_id = Some(i.id.clone());
                    }
                }
            }
        }
    }
    draft
}

/// Metadata stored by LowMail in a saved draft.
pub fn draft_meta(message: &Message) -> Option<DraftMeta> {
    let value = message.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(DRAFT_HEADER))?.1.clone();
    let bytes = base64::engine::general_purpose::STANDARD.decode(value.trim()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Header text without line breaks, so nothing can add header fields.
fn one_line(s: &str) -> String {
    s.chars().map(|c| if c == '\r' || c == '\n' || c == '\0' { ' ' } else { c }).collect()
}

fn mb_address(a: &Address) -> MbAddress<'static> {
    MbAddress::new_address(a.name.as_deref().map(one_line).filter(|n| !n.trim().is_empty()), one_line(a.email.trim()))
}

fn mb_list(list: &[Address]) -> MbAddress<'static> {
    MbAddress::new_list(list.iter().map(mb_address).collect())
}

pub fn validate_address(email: &str) -> Result<()> {
    email.parse::<lettre::Address>().map(|_| ()).map_err(|_| Error::invalid(format!("Invalid e-mail address: {email}")))
}

/// Pulls `data:` images out of the HTML into inline parts referenced by `cid:`.
fn extract_data_images(html: &str, domain: &str) -> (String, Vec<ResolvedAttachment>) {
    let mut out = String::with_capacity(html.len());
    let mut parts = Vec::new();
    let mut rest = html;
    while let Some(pos) = rest.find("data:image/") {
        // Only inside src="..." or src='...'.
        let before = &rest[..pos];
        let quote = before.chars().last();
        if !matches!(quote, Some('"') | Some('\'')) || !before[..before.len() - 1].trim_end().to_ascii_lowercase().ends_with("src=") {
            out.push_str(&rest[..pos + 11]);
            rest = &rest[pos + 11..];
            continue;
        }
        let q = quote.unwrap();
        let Some(end) = rest[pos..].find(q).map(|e| pos + e) else { break };
        let url = &rest[pos..end];
        let parsed = url.strip_prefix("data:").and_then(|u| u.split_once(";base64,"));
        match parsed.and_then(|(ct, data)| base64::engine::general_purpose::STANDARD.decode(data.trim()).ok().map(|b| (ct.to_string(), b)))
        {
            Some((ct, bytes)) => {
                let cid = format!("{}@{}", uuid::Uuid::new_v4().simple(), domain);
                let ext = ct.split('/').nth(1).unwrap_or("png").split('+').next().unwrap_or("png").to_string();
                out.push_str(&rest[..pos]);
                out.push_str("cid:");
                out.push_str(&cid);
                parts.push(ResolvedAttachment {
                    filename: format!("image-{}.{ext}", parts.len() + 1),
                    content_type: ct,
                    bytes,
                    inline: true,
                    content_id: Some(cid),
                });
            }
            None => out.push_str(&rest[..end]),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    (out, parts)
}

pub struct Built {
    pub message_id: String,
    pub raw: Vec<u8>,
}

/// Builds the MIME message. `include_bcc` is true for the copies kept in
/// Sent and Drafts and false for the one handed to the SMTP server.
pub fn build(
    draft: &Draft,
    from: &Address,
    attachments: &[ResolvedAttachment],
    include_bcc: bool,
    meta: Option<&DraftMeta>,
) -> Result<Built> {
    let domain = from.email.rsplit('@').next().unwrap_or("lowmail.local").to_string();
    let message_id = draft
        .message_id
        .as_deref()
        .map(one_line)
        .filter(|id| !id.trim().is_empty() && !id.contains(' '))
        .unwrap_or_else(|| format!("{}@{}", uuid::Uuid::new_v4(), domain));

    let mut builder = MessageBuilder::new()
        .from(mb_address(from))
        .subject(Text::new(one_line(&draft.subject)))
        .message_id(MessageId::new(message_id.clone()))
        .date(chrono::Utc::now().timestamp())
        .header("User-Agent", Raw::new(format!("LowMail/{}", env!("CARGO_PKG_VERSION"))));
    if !draft.to.is_empty() {
        builder = builder.to(mb_list(&draft.to));
    }
    if !draft.cc.is_empty() {
        builder = builder.cc(mb_list(&draft.cc));
    }
    if include_bcc && !draft.bcc.is_empty() {
        builder = builder.bcc(mb_list(&draft.bcc));
    }
    if !draft.reply_to.is_empty() {
        builder = builder.reply_to(mb_list(&draft.reply_to));
    }
    if let Some(id) = draft.in_reply_to.as_ref().filter(|s| !s.is_empty()) {
        builder = builder.in_reply_to(MessageId::new(one_line(id)));
    }
    if !draft.references.is_empty() {
        builder = builder.references(MessageId::from(draft.references.iter().map(|r| one_line(r)).collect::<Vec<_>>()));
    }
    match draft.priority {
        Priority::High => {
            builder = builder.header("X-Priority", Raw::new("1 (Highest)")).header("Importance", Raw::new("high"));
        }
        Priority::Low => {
            builder = builder.header("X-Priority", Raw::new("5 (Lowest)")).header("Importance", Raw::new("low"));
        }
        Priority::Normal => {}
    }
    if draft.request_receipt {
        builder = builder.header("Disposition-Notification-To", mb_address(from));
    }
    if let Some(meta) = meta {
        let json = serde_json::to_vec(meta)?;
        builder = builder.header(DRAFT_HEADER, Raw::new(base64::engine::general_purpose::STANDARD.encode(json)));
    }

    let (html, mut inline) = match &draft.html {
        Some(h) => {
            let (h, parts) = extract_data_images(h, &domain);
            (Some(h), parts)
        }
        None => (None, Vec::new()),
    };
    for a in attachments.iter().filter(|a| a.inline && a.content_id.is_some()) {
        if html.as_ref().is_some_and(|h| h.contains(a.content_id.as_deref().unwrap_or("-"))) {
            inline.push(a.clone());
        }
    }
    let files: Vec<&ResolvedAttachment> = attachments
        .iter()
        .filter(|a| !(a.inline && html.as_ref().is_some_and(|h| a.content_id.as_ref().is_some_and(|c| h.contains(c.as_str())))))
        .collect();

    let text = match (&html, draft.text.trim().is_empty()) {
        (Some(h), true) => html_to_text(h),
        _ => draft.text.clone(),
    };
    let text_part = MimePart::new("text/plain", normalize_newlines(&text));
    let body_part = match html {
        Some(h) => {
            let doc = format!("<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body>{h}</body></html>");
            let html_part = MimePart::new("text/html", doc);
            let html_part = if inline.is_empty() {
                html_part
            } else {
                let mut related = vec![html_part];
                for img in &inline {
                    related.push(
                        MimePart::new(ContentType::new(img.content_type.clone()), img.bytes.clone())
                            .inline()
                            .cid(img.content_id.clone().unwrap_or_default()),
                    );
                }
                MimePart::new("multipart/related", related)
            };
            MimePart::new("multipart/alternative", vec![text_part, html_part])
        }
        None => text_part,
    };
    let body = if files.is_empty() {
        body_part
    } else {
        let mut mixed = vec![body_part];
        for f in files {
            let ct = ContentType::new(f.content_type.clone()).attribute("name", f.filename.clone());
            let part = MimePart::new(ct, f.bytes.clone()).attachment(f.filename.clone());
            mixed.push(if f.content_type.eq_ignore_ascii_case("message/rfc822") { part.transfer_encoding("8bit") } else { part });
        }
        MimePart::new("multipart/mixed", mixed)
    };
    let raw = builder.body(body).write_to_vec().map_err(|e| Error::internal("Could not build the message.").with_detail(e.to_string()))?;
    Ok(Built { message_id, raw })
}

fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Every recipient of the draft, for the SMTP envelope.
pub fn envelope_recipients(draft: &Draft) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in draft.to.iter().chain(draft.cc.iter()).chain(draft.bcc.iter()) {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&a.email)) {
            out.push(a.email.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mime::{ReadOptions, parse_message};
    use crate::model::Flags;

    fn identity(email: &str) -> Identity {
        Identity {
            id: "i1".into(),
            name: "Me".into(),
            email: email.into(),
            reply_to: None,
            bcc: None,
            signature: String::new(),
            is_default: true,
        }
    }

    #[test]
    fn no_header_injection() {
        let draft = Draft {
            to: vec![Address::new(Some("Eve\r\nBcc: x@evil.example".into()), "a@example.com")],
            subject: "Hi\r\nBcc: victim@example.com".into(),
            text: "body".into(),
            ..Default::default()
        };
        let built = build(&draft, &Address::new(None, "me@example.com"), &[], false, None).unwrap();
        let raw = String::from_utf8_lossy(&built.raw);
        assert!(!raw.contains("\r\nBcc:"), "{raw}");
    }

    #[test]
    fn subjects() {
        assert_eq!(prefixed_subject("RES: ENC: Re: Hello", "Re:"), "Re: Hello");
        assert_eq!(prefixed_subject("Hello", "Fwd:"), "Fwd: Hello");
    }

    #[test]
    fn builds_and_parses_back() {
        let draft = Draft {
            to: vec![Address::new(Some("Ana Souza".into()), "ana@example.com")],
            bcc: vec![Address::new(None, "secret@example.com")],
            subject: "Relatório de março".into(),
            html: Some("<p>Olá <b>Ana</b> <img src=\"data:image/png;base64,iVBORw0KGgo=\"></p>".into()),
            priority: Priority::High,
            ..Default::default()
        };
        let files = vec![ResolvedAttachment {
            filename: "nota.txt".into(),
            content_type: "text/plain".into(),
            bytes: b"oi".to_vec(),
            inline: false,
            content_id: None,
        }];
        let from = Address::new(Some("Me".into()), "me@example.com");
        let built = build(&draft, &from, &files, false, None).unwrap();
        let text = String::from_utf8_lossy(&built.raw);
        assert!(!text.contains("secret@example.com"));
        let msg = parse_message(&built.raw, "Sent", 1, Flags::default(), &ReadOptions { allow_remote: false }).unwrap();
        assert_eq!(msg.summary.subject, "Relatório de março");
        assert_eq!(msg.summary.priority, Priority::High);
        assert!(msg.html.as_deref().unwrap().contains("data:image/png;base64"));
        assert_eq!(msg.attachments.iter().filter(|a| !a.inline).count(), 1);
        assert!(msg.text.as_deref().unwrap().contains("Olá"));

        let with_bcc = build(&draft, &from, &files, true, None).unwrap();
        assert!(String::from_utf8_lossy(&with_bcc.raw).contains("secret@example.com"));
    }

    #[test]
    fn reply_all_recipients() {
        let raw = "From: Bob <bob@example.com>\r\nTo: me@example.com, carol@example.com\r\nCc: dave@example.com, Me <ME@example.com>\r\nSubject: Plan\r\nMessage-ID: <m1@example.com>\r\nDate: Mon, 2 Mar 2026 10:00:00 +0000\r\n\r\nHello\r\n";
        let msg = parse_message(raw.as_bytes(), "INBOX", 1, Flags::default(), &ReadOptions { allow_remote: false }).unwrap();
        let ids = vec![identity("me@example.com")];
        let d = prepare(ComposeMode::ReplyAll, &msg, MessageRef { folder: "INBOX".into(), uid: 1 }, &ids, &QuoteStyle::default(), false);
        assert_eq!(d.to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), vec!["bob@example.com"]);
        assert_eq!(d.cc.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), vec!["carol@example.com", "dave@example.com"]);
        assert_eq!(d.subject, "Re: Plan");
        assert_eq!(d.in_reply_to.as_deref(), Some("m1@example.com"));
        assert!(d.text.contains("> Hello"));
        assert!(d.text.contains("escreveu:"));
    }
}
