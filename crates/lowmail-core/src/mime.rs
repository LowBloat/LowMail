//! Reading messages: headers for the list, the full message for the reading
//! pane, and single parts for downloads.

use std::collections::HashMap;

use base64::Engine;
use mail_parser::{AddressList, HeaderValue, MessageParser, MessagePart, PartKind};

use crate::error::{Error, Result};
use crate::model::{Address, AttachmentInfo, Flags, Message, MessageSummary, Priority};
use crate::sanitize;

/// Inline images larger than this are not embedded into the body.
const MAX_INLINE_IMAGE: usize = 8 * 1024 * 1024;

pub fn parser() -> MessageParser {
    MessageParser::new()
}

pub fn addresses(list: Option<AddressList<'_>>) -> Vec<Address> {
    let Some(list) = list else { return Vec::new() };
    list.mailboxes()
        .filter_map(|m| {
            let email = m.address()?.trim().to_string();
            if email.is_empty() {
                return None;
            }
            Some(Address::new(m.name().map(|n| n.trim().trim_matches('"').to_string()), email))
        })
        .collect()
}

fn ids(list: Option<mail_parser::TextList<'_>>) -> Vec<String> {
    list.map(|l| l.iter().map(|s| s.trim_matches(|c| c == '<' || c == '>').to_string()).collect()).unwrap_or_default()
}

fn raw_header(headers: &mail_parser::Headers<'_>, name: &str) -> Option<String> {
    headers.get(name).map(|h| unfold(&String::from_utf8_lossy(h.raw_value())))
}

fn unfold(s: &str) -> String {
    s.split(['\r', '\n']).map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
}

fn priority(headers: &mail_parser::Headers<'_>) -> Priority {
    if let Some(p) = raw_header(headers, "X-Priority") {
        match p.trim().chars().next() {
            Some('1' | '2') => return Priority::High,
            Some('4' | '5') => return Priority::Low,
            _ => {}
        }
    }
    for name in ["Importance", "Priority"] {
        if let Some(v) = raw_header(headers, name) {
            let v = v.to_ascii_lowercase();
            if v.contains("high") || v.contains("urgent") {
                return Priority::High;
            }
            if v.contains("low") || v.contains("non-urgent") {
                return Priority::Low;
            }
        }
    }
    Priority::Normal
}

/// A list row from the header fields fetched with the list.
pub fn summary_from_headers(
    header: &[u8],
    folder: &str,
    uid: u32,
    size: u32,
    internal_date: Option<i64>,
    flags: Flags,
    has_attachments: bool,
) -> MessageSummary {
    let parsed = parser().parse_headers(header);
    match parsed {
        Some(msg) => {
            let headers = msg.headers();
            MessageSummary {
                uid,
                folder: folder.to_string(),
                message_id: msg.message_id().map(str::to_string),
                subject: msg.subject().map(|s| unfold(s)).unwrap_or_default(),
                from: addresses(msg.from()),
                to: addresses(msg.to()),
                cc: addresses(msg.cc()),
                date: msg
                    .date()
                    .map(|d| d.to_timestamp() * 1000)
                    .or(internal_date)
                    .unwrap_or(0),
                size,
                flags,
                has_attachments,
                priority: priority(&headers),
                in_reply_to: ids(msg.in_reply_to()).into_iter().next(),
                references: ids(msg.references()),
                list_unsubscribe: headers.contains("List-Unsubscribe"),
            }
        }
        None => MessageSummary {
            uid,
            folder: folder.to_string(),
            message_id: None,
            subject: String::new(),
            from: vec![],
            to: vec![],
            cc: vec![],
            date: internal_date.unwrap_or(0),
            size,
            flags,
            has_attachments,
            priority: Priority::Normal,
            in_reply_to: None,
            references: vec![],
            list_unsubscribe: false,
        },
    }
}

fn content_type_of(part: &MessagePart<'_>) -> String {
    part.content_type()
        .map(|ct| format!("{}/{}", ct.ctype(), ct.subtype().unwrap_or("octet-stream")).to_ascii_lowercase())
        .unwrap_or_else(|| match part.kind() {
            PartKind::Html => "text/html".into(),
            PartKind::Text => "text/plain".into(),
            PartKind::Message(_) => "message/rfc822".into(),
            _ => "application/octet-stream".into(),
        })
}

fn extension_for(content_type: &str) -> &'static str {
    match content_type {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "text/html" => "html",
        "text/calendar" => "ics",
        "text/vcard" | "text/x-vcard" => "vcf",
        "message/rfc822" => "eml",
        "application/zip" => "zip",
        _ => "bin",
    }
}

fn filename_of(part: &MessagePart<'_>, index: usize) -> String {
    if let Some(name) = part.attachment_name() {
        let clean = name.replace(['/', '\\', '\0'], "_");
        if !clean.trim().is_empty() {
            return clean.trim().to_string();
        }
    }
    let ct = content_type_of(part);
    if let PartKind::Message(nested) = part.kind() {
        let subject = nested.headers().subject().unwrap_or("message").replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        return format!("{}.eml", subject.trim());
    }
    format!("attachment-{}.{}", index + 1, extension_for(&ct))
}

pub struct ReadOptions {
    pub allow_remote: bool,
}

/// The reading-pane view of a message.
pub fn parse_message(raw: &[u8], folder: &str, uid: u32, flags: Flags, opts: &ReadOptions) -> Result<Message> {
    let msg = parser().parse(raw).ok_or_else(|| Error::invalid("The message could not be read."))?;
    let root = msg.root();
    let headers = root.headers();

    // Inline images by Content-ID, as data: URLs.
    let mut cids: HashMap<String, String> = HashMap::new();
    for part in msg.parts() {
        if let Some(cid) = part.content_id() {
            let ct = content_type_of(&part);
            if ct.starts_with("image/") && part.decoded_len() <= MAX_INLINE_IMAGE {
                let data = base64::engine::general_purpose::STANDARD.encode(part.decoded());
                cids.insert(cid.trim_matches(|c| c == '<' || c == '>').to_string(), format!("data:{ct};base64,{data}"));
            }
        }
    }

    // The HTML view: HTML parts as they are, text parts converted, images inline.
    let mut html_body = String::new();
    let mut has_html = false;
    for part in root.html_body() {
        match part.kind() {
            PartKind::Html => {
                has_html = true;
                if let Some(t) = part.text() {
                    html_body.push_str(&t);
                }
            }
            PartKind::Text => {
                if let Some(t) = part.text() {
                    html_body.push_str(&sanitize::text_to_html(&t));
                }
            }
            PartKind::InlineBinary | PartKind::Binary => {
                let ct = content_type_of(&part);
                if ct.starts_with("image/") && part.decoded_len() <= MAX_INLINE_IMAGE {
                    let data = base64::engine::general_purpose::STANDARD.encode(part.decoded());
                    html_body.push_str(&format!("<p><img src=\"data:{ct};base64,{data}\" style=\"max-width:100%\"></p>"));
                }
            }
            _ => {}
        }
    }

    let mut text_body = String::new();
    for part in root.text_body() {
        match part.kind() {
            PartKind::Text => {
                if let Some(t) = part.text() {
                    text_body.push_str(&t);
                }
            }
            PartKind::Html => {
                if let Some(t) = part.text() {
                    text_body.push_str(&sanitize::html_to_text(&t));
                }
            }
            _ => {}
        }
        if !text_body.ends_with('\n') && !text_body.is_empty() {
            text_body.push('\n');
        }
    }

    let sanitized = sanitize::sanitize(&html_body, &cids, opts.allow_remote);

    let mut attachments = Vec::new();
    for (index, part) in root.attachments().enumerate() {
        let ct = content_type_of(&part);
        let cid = part.content_id().map(|c| c.trim_matches(|c| c == '<' || c == '>').to_string());
        let shown_inline = cid.as_ref().is_some_and(|c| sanitized.used_cids.contains(c))
            || (part.in_html_body() && ct.starts_with("image/"));
        attachments.push(AttachmentInfo {
            part: part.id().to_string(),
            filename: filename_of(&part, index),
            content_type: ct,
            size: part.decoded_len() as u64,
            inline: shown_inline,
            content_id: cid,
        });
    }

    let summary = MessageSummary {
        uid,
        folder: folder.to_string(),
        message_id: headers.message_id().map(str::to_string),
        subject: headers.subject().map(unfold).unwrap_or_default(),
        from: addresses(headers.from()),
        to: addresses(headers.to()),
        cc: addresses(headers.cc()),
        date: headers.date().map(|d| d.to_timestamp() * 1000).unwrap_or(0),
        size: raw.len() as u32,
        flags,
        has_attachments: attachments.iter().any(|a| !a.inline),
        priority: priority(&headers),
        in_reply_to: ids(headers.in_reply_to()).into_iter().next(),
        references: ids(headers.references()),
        list_unsubscribe: headers.contains("List-Unsubscribe"),
    };

    let receipt_to = headers
        .value("Disposition-Notification-To")
        .and_then(|v| match v {
            HeaderValue::Address(list) => addresses(Some(list)).into_iter().next(),
            other => other.as_text().and_then(parse_one_address),
        })
        .or_else(|| raw_header(&headers, "Disposition-Notification-To").and_then(|v| parse_one_address(&v)));

    let mut all_headers = Vec::new();
    for h in headers.iter() {
        let value = match h.value() {
            HeaderValue::Text(t) => unfold(t),
            HeaderValue::Address(list) => addresses(Some(list))
                .iter()
                .map(format_address)
                .collect::<Vec<_>>()
                .join(", "),
            _ => unfold(&String::from_utf8_lossy(h.raw_value())),
        };
        all_headers.push((h.raw_name().to_string(), value));
    }

    Ok(Message {
        summary,
        reply_to: addresses(headers.reply_to()),
        bcc: addresses(headers.bcc()),
        html: Some(sanitized.html).filter(|h| !h.trim().is_empty()),
        text: Some(text_body).filter(|t| !t.trim().is_empty()),
        remote_content: sanitized.remote,
        attachments,
        receipt_to,
        unsubscribe: raw_header(&headers, "List-Unsubscribe"),
        unsubscribe_post: headers.contains("List-Unsubscribe-Post"),
        headers: all_headers,
    })
    .map(|mut m| {
        if !has_html && m.html.is_none() && m.text.is_some() {
            m.html = m.text.as_deref().map(sanitize::text_to_html);
        }
        m
    })
}

fn parse_one_address(text: &str) -> Option<Address> {
    let line = format!(" {text}\r\n");
    let parsed = mail_parser::HeaderForm::Addresses.parse(line.as_bytes());
    let list = parsed.value().as_address()?;
    addresses(Some(list)).into_iter().next()
}

pub fn format_address(a: &Address) -> String {
    match &a.name {
        Some(n) if !n.is_empty() => format!("{n} <{}>", a.email),
        _ => a.email.clone(),
    }
}

/// One part, decoded, for download: (file name, content type, bytes).
pub fn extract_part(raw: &[u8], part_id: &str) -> Result<(String, String, Vec<u8>)> {
    let msg = parser().parse(raw).ok_or_else(|| Error::invalid("The message could not be read."))?;
    let id: u32 = part_id.parse().map_err(|_| Error::invalid("Invalid attachment."))?;
    let part = msg.part(id).ok_or_else(|| Error::not_found("The attachment no longer exists."))?;
    let index = msg.root().attachments().position(|p| p.id() == id).unwrap_or(0);
    let filename = filename_of(&part, index);
    let ct = content_type_of(&part);
    let bytes = match part.kind() {
        // A forwarded message is downloaded as the original .eml.
        PartKind::Message(_) => part.decoded().into_owned(),
        _ => part.decoded().into_owned(),
    };
    Ok((filename, ct, bytes))
}

/// Subject of a raw message, for naming downloads.
pub fn subject_of(raw: &[u8]) -> String {
    parser().parse_headers(raw).and_then(|m| m.subject().map(unfold)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = concat!(
        "From: =?UTF-8?Q?Jo=C3=A3o_Silva?= <joao@example.com>\r\n",
        "To: Ana <ana@example.com>, bob@example.com\r\n",
        "Subject: =?ISO-8859-1?Q?Relat=F3rio?= mensal\r\n",
        "Date: Mon, 2 Mar 2026 10:00:00 -0300\r\n",
        "Message-ID: <abc@example.com>\r\n",
        "X-Priority: 1\r\n",
        "Disposition-Notification-To: joao@example.com\r\n",
        "Content-Type: multipart/mixed; boundary=\"b1\"\r\n",
        "\r\n",
        "--b1\r\n",
        "Content-Type: multipart/related; boundary=\"b2\"\r\n",
        "\r\n",
        "--b2\r\n",
        "Content-Type: text/html; charset=utf-8\r\n",
        "\r\n",
        "<p>Olá <img src=\"cid:logo@x\"><img src=\"https://tracker.example/p.gif\"></p>\r\n",
        "--b2\r\n",
        "Content-Type: image/png\r\n",
        "Content-ID: <logo@x>\r\n",
        "Content-Transfer-Encoding: base64\r\n",
        "\r\n",
        "iVBORw0KGgo=\r\n",
        "--b2--\r\n",
        "--b1\r\n",
        "Content-Type: application/pdf; name=\"relatório.pdf\"\r\n",
        "Content-Disposition: attachment; filename=\"relatório.pdf\"\r\n",
        "Content-Transfer-Encoding: base64\r\n",
        "\r\n",
        "JVBERi0xLjQK\r\n",
        "--b1--\r\n",
    );

    #[test]
    fn parses_message() {
        let m = parse_message(SAMPLE.as_bytes(), "INBOX", 7, Flags::default(), &ReadOptions { allow_remote: false }).unwrap();
        assert_eq!(m.summary.subject, "Relatório mensal");
        assert_eq!(m.summary.from[0].name.as_deref(), Some("João Silva"));
        assert_eq!(m.summary.to.len(), 2);
        assert_eq!(m.summary.priority, Priority::High);
        assert!(m.remote_content);
        let html = m.html.unwrap();
        assert!(html.contains("data:image/png;base64,"));
        assert!(!html.contains("tracker.example"));
        let files: Vec<_> = m.attachments.iter().filter(|a| !a.inline).collect();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "relatório.pdf");
        assert!(m.summary.has_attachments);
        assert_eq!(m.receipt_to.unwrap().email, "joao@example.com");
        let (name, ct, bytes) = extract_part(SAMPLE.as_bytes(), &files[0].part).unwrap();
        assert_eq!(name, "relatório.pdf");
        assert_eq!(ct, "application/pdf");
        assert_eq!(&bytes[..4], b"%PDF");
    }

    #[test]
    fn list_summary() {
        let s = summary_from_headers(SAMPLE.as_bytes(), "INBOX", 1, 100, None, Flags::default(), true);
        assert_eq!(s.subject, "Relatório mensal");
        assert_eq!(s.message_id.as_deref(), Some("abc@example.com"));
        assert_eq!(s.date, 1_772_456_400_000);
    }
}
