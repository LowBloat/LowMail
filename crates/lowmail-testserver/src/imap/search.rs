//! SEARCH program parsing and evaluation (RFC 3501 §6.4.4).

use std::sync::Arc;

use chrono::{DateTime, FixedOffset, NaiveDate};

use super::parser::{PResult, ParseError, Parser, SeqSet};
use crate::message::{MessageData, decode_text, header_fields};
use crate::store::has_flag;

/// Which address header a string key targets.
#[derive(Clone, Copy, Debug)]
pub(crate) enum AddrField {
    From,
    To,
    Cc,
    Bcc,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum DateCmp {
    Before,
    On,
    Since,
}

#[derive(Clone, Debug)]
pub(crate) enum Key {
    All,
    /// Flag present (`true`) or absent (`false`).
    Flag(String, bool),
    Recent,
    New,
    Old,
    Addr(AddrField, String),
    Subject(String),
    Body(String),
    Text(String),
    Header(String, String),
    Internal(DateCmp, NaiveDate),
    Sent(DateCmp, NaiveDate),
    Larger(u64),
    Smaller(u64),
    Uid(SeqSet),
    Seq(SeqSet),
    Not(Box<Key>),
    Or(Box<Key>, Box<Key>),
    And(Vec<Key>),
}

/// Why a search program was rejected.
pub(crate) enum SearchError {
    Parse(ParseError),
    BadCharset,
}

impl From<ParseError> for SearchError {
    fn from(e: ParseError) -> Self {
        SearchError::Parse(e)
    }
}

/// Accepts the charsets we can search with.
pub(crate) fn check_charset(name: &str) -> Result<(), SearchError> {
    if name.eq_ignore_ascii_case("UTF-8") || name.eq_ignore_ascii_case("US-ASCII") { Ok(()) } else { Err(SearchError::BadCharset) }
}

/// Parses `[CHARSET x] key *(SP key)` up to the end of the command.
pub(crate) fn parse_program(p: &mut Parser<'_>, allow_charset: bool) -> Result<Key, SearchError> {
    if allow_charset && p.keyword("CHARSET") {
        p.sp()?;
        check_charset(&p.astring_utf8()?)?;
        p.sp()?;
    }
    parse_keys(p)
}

/// Parses keys until the end of input; they are ANDed together.
pub(crate) fn parse_keys(p: &mut Parser<'_>) -> Result<Key, SearchError> {
    let mut keys = Vec::new();
    loop {
        keys.push(parse_key(p)?);
        p.opt_sp();
        if p.is_end() {
            break;
        }
    }
    Ok(if keys.len() == 1 { keys.remove(0) } else { Key::And(keys) })
}

fn string_arg(p: &mut Parser<'_>) -> PResult<String> {
    p.sp()?;
    Ok(p.astring_utf8()?.to_lowercase())
}

/// Nesting limit for NOT / OR / parenthesized groups.
const MAX_DEPTH: usize = 64;

fn parse_key(p: &mut Parser<'_>) -> PResult<Key> {
    parse_key_at(p, 0)
}

fn parse_key_at(p: &mut Parser<'_>, depth: usize) -> PResult<Key> {
    if depth > MAX_DEPTH {
        return Err(ParseError("search program nested too deeply".into()));
    }
    if p.eat(b'(') {
        let mut keys = Vec::new();
        p.opt_sp();
        while !p.eat(b')') {
            keys.push(parse_key_at(p, depth + 1)?);
            p.opt_sp();
            if p.is_end() {
                return Err(ParseError("unterminated search group".into()));
            }
        }
        return Ok(Key::And(keys));
    }
    if p.peek().is_some_and(|c| c.is_ascii_digit() || c == b'*') {
        return Ok(Key::Seq(p.sequence_set()?));
    }
    let word = p.atom()?.to_ascii_uppercase();
    let flag = |name: &str, set: bool| Ok(Key::Flag(name.to_owned(), set));
    match word.as_str() {
        "ALL" => Ok(Key::All),
        "SEEN" => flag("\\Seen", true),
        "UNSEEN" => flag("\\Seen", false),
        "FLAGGED" => flag("\\Flagged", true),
        "UNFLAGGED" => flag("\\Flagged", false),
        "ANSWERED" => flag("\\Answered", true),
        "UNANSWERED" => flag("\\Answered", false),
        "DELETED" => flag("\\Deleted", true),
        "UNDELETED" => flag("\\Deleted", false),
        "DRAFT" => flag("\\Draft", true),
        "UNDRAFT" => flag("\\Draft", false),
        "RECENT" => Ok(Key::Recent),
        "NEW" => Ok(Key::New),
        "OLD" => Ok(Key::Old),
        "KEYWORD" | "UNKEYWORD" => {
            p.sp()?;
            let kw = p.flag()?;
            Ok(Key::Flag(kw, word == "KEYWORD"))
        }
        "FROM" => Ok(Key::Addr(AddrField::From, string_arg(p)?)),
        "TO" => Ok(Key::Addr(AddrField::To, string_arg(p)?)),
        "CC" => Ok(Key::Addr(AddrField::Cc, string_arg(p)?)),
        "BCC" => Ok(Key::Addr(AddrField::Bcc, string_arg(p)?)),
        "SUBJECT" => Ok(Key::Subject(string_arg(p)?)),
        "BODY" => Ok(Key::Body(string_arg(p)?)),
        "TEXT" => Ok(Key::Text(string_arg(p)?)),
        "HEADER" => {
            p.sp()?;
            let name = p.astring_utf8()?;
            Ok(Key::Header(name, string_arg(p)?))
        }
        "BEFORE" | "ON" | "SINCE" | "SENTBEFORE" | "SENTON" | "SENTSINCE" => {
            p.sp()?;
            let date = p.date()?;
            let cmp = match word.trim_start_matches("SENT") {
                "BEFORE" => DateCmp::Before,
                "ON" => DateCmp::On,
                _ => DateCmp::Since,
            };
            Ok(if word.starts_with("SENT") { Key::Sent(cmp, date) } else { Key::Internal(cmp, date) })
        }
        "LARGER" => {
            p.sp()?;
            Ok(Key::Larger(p.number64()?))
        }
        "SMALLER" => {
            p.sp()?;
            Ok(Key::Smaller(p.number64()?))
        }
        "UID" => {
            p.sp()?;
            Ok(Key::Uid(p.sequence_set()?))
        }
        "NOT" => {
            p.sp()?;
            Ok(Key::Not(Box::new(parse_key_at(p, depth + 1)?)))
        }
        "OR" => {
            p.sp()?;
            let a = parse_key_at(p, depth + 1)?;
            p.sp()?;
            let b = parse_key_at(p, depth + 1)?;
            Ok(Key::Or(Box::new(a), Box::new(b)))
        }
        other => Err(ParseError(format!("unknown search key {other}"))),
    }
}

/// A message snapshot to evaluate keys against.
pub(crate) struct Candidate {
    pub seq: u32,
    pub uid: u32,
    pub flags: Vec<String>,
    pub recent: bool,
    pub internal_date: DateTime<FixedOffset>,
    pub data: Arc<MessageData>,
}

/// Mailbox-wide values needed to resolve `*`.
pub(crate) struct Bounds {
    pub max_seq: u32,
    pub max_uid: u32,
}

fn compare(cmp: DateCmp, date: NaiveDate, key: NaiveDate) -> bool {
    match cmp {
        DateCmp::Before => date < key,
        DateCmp::On => date == key,
        DateCmp::Since => date >= key,
    }
}

pub(crate) fn matches(key: &Key, c: &Candidate, bounds: &Bounds) -> bool {
    match key {
        Key::All => true,
        Key::Flag(flag, set) => has_flag(&c.flags, flag) == *set,
        Key::Recent => c.recent,
        Key::New => c.recent && !has_flag(&c.flags, "\\Seen"),
        Key::Old => !c.recent,
        Key::Addr(field, needle) => {
            let meta = c.data.meta();
            let hay = match field {
                AddrField::From => &meta.from,
                AddrField::To => &meta.to,
                AddrField::Cc => &meta.cc,
                AddrField::Bcc => &meta.bcc,
            };
            hay.to_lowercase().contains(needle.as_str())
        }
        Key::Subject(needle) => c.data.meta().subject.to_lowercase().contains(needle.as_str()),
        Key::Body(needle) => c.data.body_text().contains(needle.as_str()),
        Key::Text(needle) => header_contains(&c.data, None, needle) || c.data.body_text().contains(needle.as_str()),
        Key::Header(name, needle) => header_contains(&c.data, Some(name), needle),
        Key::Internal(cmp, date) => compare(*cmp, c.internal_date.date_naive(), *date),
        Key::Sent(cmp, date) => {
            let sent = c.data.meta().sent.map_or_else(|| c.internal_date.date_naive(), |s| s.date);
            compare(*cmp, sent, *date)
        }
        Key::Larger(n) => c.data.raw.len() as u64 > *n,
        Key::Smaller(n) => (c.data.raw.len() as u64) < *n,
        Key::Uid(set) => set.contains(c.uid, bounds.max_uid),
        Key::Seq(set) => set.contains(c.seq, bounds.max_seq),
        Key::Not(k) => !matches(k, c, bounds),
        Key::Or(a, b) => matches(a, c, bounds) || matches(b, c, bounds),
        Key::And(keys) => keys.iter().all(|k| matches(k, c, bounds)),
    }
}

/// Case-insensitive substring match on decoded header values. With `name`
/// only that field is checked (an empty needle then means "present").
fn header_contains(data: &MessageData, name: Option<&str>, needle: &str) -> bool {
    header_fields(data.header()).iter().any(|field| {
        if let Some(name) = name
            && !field.is(name)
        {
            return false;
        }
        if needle.is_empty() {
            return true;
        }
        if decode_text(field.value).to_lowercase().contains(needle) {
            return true;
        }
        name.is_none() && String::from_utf8_lossy(field.name).to_lowercase().contains(needle)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(raw: &str, flags: &[&str]) -> Candidate {
        Candidate {
            seq: 1,
            uid: 10,
            flags: flags.iter().map(|f| f.to_string()).collect(),
            recent: false,
            internal_date: DateTime::parse_from_rfc3339("2025-10-07T10:00:00-03:00").unwrap(),
            data: Arc::new(MessageData::new(raw.as_bytes().to_vec())),
        }
    }

    fn search(program: &str, c: &Candidate) -> bool {
        let mut p = Parser::new(program.as_bytes());
        let Ok(key) = parse_program(&mut p, true) else { panic!("parse failed: {program}") };
        matches(&key, c, &Bounds { max_seq: 1, max_uid: 10 })
    }

    #[test]
    fn evaluates() {
        let c = cand(
            "From: =?UTF-8?Q?Jo=C3=A3o?= <joao@x.com>\r\nSubject: Relat\u{f3}rio\r\nX-Tag: blue\r\nDate: Mon, 6 Oct 2025 10:00:00 -0300\r\n\r\nCorpo da mensagem\r\n",
            &["\\Seen"],
        );
        assert!(search("CHARSET UTF-8 FROM joão", &c));
        assert!(search("SUBJECT RELATÓRIO", &c));
        assert!(search("BODY \"corpo da\"", &c));
        assert!(search("NOT UNSEEN", &c));
        assert!(search("OR FLAGGED SEEN", &c));
        assert!(search("(SEEN UID 5:*) SENTON 6-Oct-2025 SINCE 7-Oct-2025", &c));
        assert!(search("HEADER X-Tag BLUE", &c));
        assert!(search("TEXT x-tag", &c));
        assert!(!search("HEADER X-Missing \"\"", &c));
        assert!(search("1:* LARGER 10 SMALLER 100000", &c));
        assert!(!search("BEFORE 7-Oct-2025", &c));
        assert!(search("SUBJECT {5}\r\nrelat", &c));
    }
}
