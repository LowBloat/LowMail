//! Stored message bytes plus lazily computed, cached metadata.

use std::sync::OnceLock;

use chrono::NaiveDate;
use mail_parser::{AddressList, HeaderForm, MessageParser, PartKind};

use crate::imap::fetch;

/// Raw RFC 5322 message with cached derived data.
pub(crate) struct MessageData {
    pub raw: Vec<u8>,
    meta: OnceLock<Meta>,
    search_text: OnceLock<String>,
}

/// Sent date taken from the `Date:` header.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SentDate {
    pub timestamp: i64,
    /// Calendar date in the header's own timezone.
    pub date: NaiveDate,
}

/// Everything FETCH, SEARCH and SORT need repeatedly.
pub(crate) struct Meta {
    pub header_len: usize,
    pub envelope: Vec<u8>,
    pub body_structure: Vec<u8>,
    pub body: Vec<u8>,
    pub sent: Option<SentDate>,
    /// Decoded values, used for case-insensitive SEARCH.
    pub subject: String,
    pub from: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    /// RFC 5256 sort keys (lowercase).
    pub sort_from: String,
    pub sort_to: String,
    pub sort_cc: String,
    pub base_subject: String,
}

impl MessageData {
    pub(crate) fn new(raw: Vec<u8>) -> Self {
        Self { raw, meta: OnceLock::new(), search_text: OnceLock::new() }
    }

    pub(crate) fn meta(&self) -> &Meta {
        self.meta.get_or_init(|| Meta::compute(&self.raw))
    }

    /// Header block including the separating blank line.
    pub(crate) fn header(&self) -> &[u8] {
        &self.raw[..self.meta().header_len]
    }

    /// Everything after the header block.
    pub(crate) fn text(&self) -> &[u8] {
        &self.raw[self.meta().header_len..]
    }

    /// Lowercase decoded text of all text parts (HTML converted to text).
    pub(crate) fn body_text(&self) -> &str {
        self.search_text.get_or_init(|| {
            let mut out = String::new();
            if let Some(msg) = MessageParser::new().parse(&self.raw) {
                for part in msg.parts() {
                    let Some(text) = part.text() else { continue };
                    if matches!(part.kind(), PartKind::Html) {
                        out.push_str(&mail_parser::html_to_text(&text));
                    } else {
                        out.push_str(&text);
                    }
                    out.push('\n');
                }
            } else {
                out.push_str(&String::from_utf8_lossy(self.text()));
            }
            out.to_lowercase()
        })
    }
}

impl Meta {
    fn compute(raw: &[u8]) -> Meta {
        let header_len = header_end(raw);
        let mut envelope = Vec::new();
        fetch::envelope(&raw[..header_len], &mut envelope);
        let mut body_structure = Vec::new();
        let mut body = Vec::new();
        let mut meta = Meta {
            header_len,
            envelope,
            body_structure: Vec::new(),
            body: Vec::new(),
            sent: None,
            subject: String::new(),
            from: String::new(),
            to: String::new(),
            cc: String::new(),
            bcc: String::new(),
            sort_from: String::new(),
            sort_to: String::new(),
            sort_cc: String::new(),
            base_subject: String::new(),
        };
        let Some(msg) = MessageParser::new().parse(raw) else {
            fetch::fallback_body_structure(raw.len() - header_len, &mut meta.body_structure);
            meta.body = meta.body_structure.clone();
            return meta;
        };
        let root = msg.root();
        fetch::body_structure(root.root_part(), true, &mut body_structure);
        fetch::body_structure(root.root_part(), false, &mut body);
        meta.body_structure = body_structure;
        meta.body = body;

        let headers = root.headers();
        meta.subject = headers.subject().unwrap_or_default().to_owned();
        meta.base_subject = mail_parser::thread_name(&meta.subject).to_lowercase();
        meta.from = format_addresses(headers.from());
        meta.to = format_addresses(headers.to());
        meta.cc = format_addresses(headers.cc());
        meta.bcc = format_addresses(headers.bcc());
        meta.sort_from = sort_address(headers.from());
        meta.sort_to = sort_address(headers.to());
        meta.sort_cc = sort_address(headers.cc());
        meta.sent = headers.date().filter(|d| d.is_valid()).and_then(|d| {
            Some(SentDate { timestamp: d.to_timestamp(), date: NaiveDate::from_ymd_opt(d.year.into(), d.month.into(), d.day.into())? })
        });
        meta
    }
}

fn format_addresses(list: Option<AddressList<'_>>) -> String {
    let Some(list) = list else {
        return String::new();
    };
    let mut out = String::new();
    for mailbox in list.mailboxes() {
        if !out.is_empty() {
            out.push_str(", ");
        }
        if let Some(name) = mailbox.name() {
            out.push_str(name);
            out.push(' ');
        }
        out.push('<');
        out.push_str(mailbox.address().unwrap_or_default());
        out.push('>');
    }
    out
}

/// RFC 5256: the addr-mailbox (local part) of the first address.
fn sort_address(list: Option<AddressList<'_>>) -> String {
    list.and_then(|l| l.first()).and_then(|m| m.address()).map(|a| a.split('@').next().unwrap_or(a).to_lowercase()).unwrap_or_default()
}

/// Offset just past the blank line that ends the header block (or the
/// whole length when there is no body).
pub(crate) fn header_end(raw: &[u8]) -> usize {
    if raw.starts_with(b"\r\n") {
        return 2;
    }
    if raw.starts_with(b"\n") {
        return 1;
    }
    let mut i = 0;
    while let Some(off) = raw[i..].iter().position(|&b| b == b'\n') {
        let nl = i + off;
        let rest = &raw[nl + 1..];
        if rest.starts_with(b"\r\n") {
            return nl + 3;
        }
        if rest.starts_with(b"\n") {
            return nl + 2;
        }
        i = nl + 1;
    }
    raw.len()
}

/// One header field as written.
#[derive(Clone, Copy)]
pub(crate) struct RawField<'a> {
    /// Field name, trimmed.
    pub name: &'a [u8],
    /// The complete field including name, folding and line break.
    pub raw: &'a [u8],
    /// Everything after the colon, folding included.
    pub value: &'a [u8],
}

impl RawField<'_> {
    pub(crate) fn is(&self, name: &str) -> bool {
        self.name.eq_ignore_ascii_case(name.as_bytes())
    }
}

/// Splits a header block into fields, stopping at the first blank line.
pub(crate) fn header_fields(block: &[u8]) -> Vec<RawField<'_>> {
    // (start, colon, end) indices.
    let mut spans: Vec<(usize, usize, usize)> = Vec::new();
    let mut i = 0;
    while i < block.len() {
        let end = block[i..].iter().position(|&b| b == b'\n').map_or(block.len(), |p| i + p + 1);
        let line = crate::io::trim_eol(&block[i..end]);
        if line.is_empty() {
            break;
        }
        if line[0] == b' ' || line[0] == b'\t' {
            if let Some(last) = spans.last_mut() {
                last.2 = end;
            }
        } else if let Some(colon) = line.iter().position(|&b| b == b':') {
            spans.push((i, i + colon, end));
        }
        i = end;
    }
    spans
        .into_iter()
        .map(|(start, colon, end)| RawField {
            name: block[start..colon].trim_ascii(),
            raw: &block[start..end],
            value: &block[colon + 1..end],
        })
        .collect()
}

/// Removes folding line breaks and surrounding whitespace.
pub(crate) fn unfold(value: &[u8]) -> Vec<u8> {
    let joined: Vec<u8> = value.iter().copied().filter(|&b| b != b'\r' && b != b'\n').collect();
    joined.trim_ascii().to_vec()
}

/// Decodes RFC 2047 encoded words of an unstructured header value.
pub(crate) fn decode_text(value: &[u8]) -> String {
    HeaderForm::Text
        .parse(value)
        .value()
        .as_text()
        .map(str::to_owned)
        .unwrap_or_else(|| String::from_utf8_lossy(&unfold(value)).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_block() {
        let raw = b"Subject: a\r\n b\r\nFrom: x@y\r\n\r\nbody\r\n";
        assert_eq!(header_end(raw), 29);
        let fields = header_fields(raw);
        assert_eq!(fields.len(), 2);
        assert!(fields[0].is("subject"));
        assert_eq!(unfold(fields[0].value), b"a b");
        assert_eq!(fields[0].raw, b"Subject: a\r\n b\r\n");
        assert_eq!(header_end(b"Subject: x\r\n"), 12);
    }

    #[test]
    fn decoded_meta() {
        let raw = b"From: =?UTF-8?Q?Jo=C3=A3o?= <joao@x.com>\r\nSubject: =?ISO-8859-1?Q?Promo=E7=E3o?=\r\nDate: Mon, 6 Oct 2025 10:00:00 -0300\r\n\r\nOl\xe1\r\n";
        let data = MessageData::new(raw.to_vec());
        let meta = data.meta();
        assert_eq!(meta.subject, "Promoção");
        assert!(meta.from.contains("João"));
        assert_eq!(meta.sort_from, "joao");
        assert_eq!(meta.sent.unwrap().date, NaiveDate::from_ymd_opt(2025, 10, 6).unwrap());
    }
}
