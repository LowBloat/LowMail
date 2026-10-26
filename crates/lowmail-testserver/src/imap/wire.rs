//! Serialization helpers for IMAP response data.

use chrono::{DateTime, FixedOffset};

/// Whether `s` can be sent as a quoted string.
fn quotable(s: &[u8]) -> bool {
    s.len() < 1024
        && s.iter()
            .all(|&c| (0x20..0x7f).contains(&c) && c != b'"' && c != b'\\')
}

/// Appends `{n}\r\n` followed by the data.
pub(crate) fn push_literal(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(format!("{{{}}}\r\n", data.len()).as_bytes());
    out.extend_from_slice(data);
}

/// Appends a quoted string when safe, a literal otherwise.
pub(crate) fn push_string(out: &mut Vec<u8>, s: &[u8]) {
    if quotable(s) {
        out.push(b'"');
        out.extend_from_slice(s);
        out.push(b'"');
    } else {
        push_literal(out, s);
    }
}

pub(crate) fn push_nstring(out: &mut Vec<u8>, s: Option<&[u8]>) {
    match s {
        Some(s) => push_string(out, s),
        None => out.extend_from_slice(b"NIL"),
    }
}

/// `"..."`/literal for a string, as a `String` (only safe for ASCII output).
pub(crate) fn string(s: &str) -> String {
    let mut out = Vec::new();
    push_string(&mut out, s.as_bytes());
    String::from_utf8_lossy(&out).into_owned()
}

/// `(\Seen \Flagged)`, appending `\Recent` when requested.
pub(crate) fn flag_list(flags: &[String], recent: bool) -> String {
    let mut out = String::from("(");
    for (i, f) in flags.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(f);
    }
    if recent {
        if !flags.is_empty() {
            out.push(' ');
        }
        out.push_str("\\Recent");
    }
    out.push(')');
    out
}

/// IMAP date-time: `"07-Oct-2025 10:20:30 -0300"`.
pub(crate) fn date_time(dt: &DateTime<FixedOffset>) -> String {
    format!("\"{}\"", dt.format("%d-%b-%Y %H:%M:%S %z"))
}

/// Compact UID set: `1:3,5,9:10`, preserving the given order.
pub(crate) fn uid_set(uids: &[u32]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < uids.len() {
        let start = uids[i];
        let mut end = start;
        while i + 1 < uids.len() && uids[i + 1] == end + 1 {
            end += 1;
            i += 1;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings() {
        let mut out = Vec::new();
        push_string(&mut out, b"plain");
        push_string(&mut out, "ção".as_bytes());
        push_nstring(&mut out, None);
        assert_eq!(out, b"\"plain\"{5}\r\n\xc3\xa7\xc3\xa3oNIL".to_vec());
        assert_eq!(uid_set(&[1, 2, 3, 5, 9, 10]), "1:3,5,9:10");
        assert_eq!(flag_list(&["\\Seen".into()], true), "(\\Seen \\Recent)");
        assert_eq!(flag_list(&[], false), "()");
    }
}
