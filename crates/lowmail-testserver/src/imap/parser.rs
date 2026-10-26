//! Cursor-based tokenizer for IMAP command arguments (RFC 3501 §9).
//!
//! The input is a complete command with literals inlined (`{n}\r\n` + data),
//! without the final CRLF.

use std::fmt;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};

#[derive(Debug, Clone)]
pub(crate) struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

pub(crate) type PResult<T> = Result<T, ParseError>;

fn err<T>(msg: impl Into<String>) -> PResult<T> {
    Err(ParseError(msg.into()))
}

/// ATOM-CHAR: any CHAR except atom-specials (`]` excluded too).
pub(crate) fn is_atom_char(c: u8) -> bool {
    c > 0x20 && c < 0x7f && !b"(){%*\"\\]".contains(&c)
}

/// ASTRING-CHAR: ATOM-CHAR or `]`.
pub(crate) fn is_astring_char(c: u8) -> bool {
    is_atom_char(c) || c == b']'
}

pub(crate) const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// One endpoint of a sequence range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SeqNum {
    Num(u32),
    Star,
}

/// A parsed sequence set (`1:4,7,9:*`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SeqSet(pub Vec<(SeqNum, SeqNum)>);

impl SeqSet {
    /// Whether `n` is in the set; `*` resolves to `max`. Ranges are
    /// unordered, so `100:*` matches `max` even if `max < 100`.
    pub(crate) fn contains(&self, n: u32, max: u32) -> bool {
        let resolve = |s: SeqNum| match s {
            SeqNum::Num(v) => v,
            SeqNum::Star => max,
        };
        self.0.iter().any(|&(a, b)| {
            let (a, b) = (resolve(a), resolve(b));
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            (lo..=hi).contains(&n)
        })
    }
}

pub(crate) struct Parser<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub(crate) fn peek(&self) -> Option<u8> {
        self.buf.get(self.pos).copied()
    }

    /// Byte `n` positions ahead of the cursor.
    pub(crate) fn peek_n(&self, n: usize) -> Option<u8> {
        self.buf.get(self.pos + n).copied()
    }

    pub(crate) fn is_end(&self) -> bool {
        self.pos >= self.buf.len()
    }

    /// Consumes `b` if it is next.
    pub(crate) fn eat(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    pub(crate) fn expect(&mut self, b: u8) -> PResult<()> {
        if self.eat(b) {
            Ok(())
        } else {
            err(format!("expected '{}'", b as char))
        }
    }

    /// One or more spaces.
    pub(crate) fn sp(&mut self) -> PResult<()> {
        if !self.eat(b' ') {
            return err("expected space");
        }
        while self.eat(b' ') {}
        Ok(())
    }

    /// Consumes spaces if present; returns whether any were found.
    pub(crate) fn opt_sp(&mut self) -> bool {
        let mut any = false;
        while self.eat(b' ') {
            any = true;
        }
        any
    }

    /// Requires the end of input (trailing spaces tolerated).
    pub(crate) fn end(&mut self) -> PResult<()> {
        self.opt_sp();
        if self.is_end() {
            Ok(())
        } else {
            err("unexpected extra arguments")
        }
    }

    pub(crate) fn take_while(&mut self, f: impl Fn(u8) -> bool) -> &'a [u8] {
        let start = self.pos;
        while self.peek().is_some_and(&f) {
            self.pos += 1;
        }
        &self.buf[start..self.pos]
    }

    pub(crate) fn atom(&mut self) -> PResult<String> {
        let a = self.take_while(is_atom_char);
        if a.is_empty() {
            return err("expected atom");
        }
        Ok(String::from_utf8_lossy(a).into_owned())
    }

    /// Command tag: ASTRING-CHAR except `+`.
    pub(crate) fn tag(&mut self) -> PResult<String> {
        let t = self.take_while(|c| is_astring_char(c) && c != b'+');
        if t.is_empty() {
            return err("missing tag");
        }
        Ok(String::from_utf8_lossy(t).into_owned())
    }

    /// Consumes the keyword `kw` (case-insensitive) if it is the next atom.
    pub(crate) fn keyword(&mut self, kw: &str) -> bool {
        let save = self.pos;
        let word = self.take_while(is_atom_char);
        if word.eq_ignore_ascii_case(kw.as_bytes()) {
            true
        } else {
            self.pos = save;
            false
        }
    }

    pub(crate) fn quoted(&mut self) -> PResult<Vec<u8>> {
        self.expect(b'"')?;
        let mut out = Vec::new();
        loop {
            match self.peek() {
                None | Some(b'\r') | Some(b'\n') => return err("unterminated quoted string"),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.peek() {
                        Some(c) => {
                            out.push(c);
                            self.pos += 1;
                        }
                        None => return err("unterminated quoted string"),
                    }
                }
                Some(c) => {
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
    }

    /// `{n}` / `{n+}` followed by CRLF and `n` bytes (already inlined).
    pub(crate) fn literal(&mut self) -> PResult<Vec<u8>> {
        self.expect(b'{')?;
        let digits = self.take_while(|c| c.is_ascii_digit());
        let n: usize = std::str::from_utf8(digits)
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| ParseError("invalid literal length".into()))?;
        self.eat(b'+');
        self.expect(b'}')?;
        self.eat(b'\r');
        self.expect(b'\n')?;
        let end = self.pos.saturating_add(n);
        if end > self.buf.len() {
            return err("truncated literal");
        }
        let data = self.buf[self.pos..end].to_vec();
        self.pos = end;
        Ok(data)
    }

    pub(crate) fn string(&mut self) -> PResult<Vec<u8>> {
        match self.peek() {
            Some(b'"') => self.quoted(),
            Some(b'{') => self.literal(),
            _ => err("expected string"),
        }
    }

    pub(crate) fn astring(&mut self) -> PResult<Vec<u8>> {
        match self.peek() {
            Some(b'"') | Some(b'{') => self.string(),
            _ => {
                let a = self.take_while(|c| is_astring_char(c) || c >= 0x80);
                if a.is_empty() {
                    err("expected astring")
                } else {
                    Ok(a.to_vec())
                }
            }
        }
    }

    pub(crate) fn astring_utf8(&mut self) -> PResult<String> {
        Ok(String::from_utf8_lossy(&self.astring()?).into_owned())
    }

    /// Mailbox pattern: like astring but `%` and `*` are allowed unquoted.
    pub(crate) fn list_mailbox(&mut self) -> PResult<String> {
        let bytes = match self.peek() {
            Some(b'"') | Some(b'{') => self.string()?,
            _ => {
                let a =
                    self.take_while(|c| is_astring_char(c) || c >= 0x80 || c == b'%' || c == b'*');
                if a.is_empty() {
                    return err("expected mailbox pattern");
                }
                a.to_vec()
            }
        };
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    pub(crate) fn number(&mut self) -> PResult<u32> {
        let d = self.take_while(|c| c.is_ascii_digit());
        std::str::from_utf8(d)
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| ParseError("expected number".into()))
    }

    pub(crate) fn number64(&mut self) -> PResult<u64> {
        let d = self.take_while(|c| c.is_ascii_digit());
        std::str::from_utf8(d)
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| ParseError("expected number".into()))
    }

    fn seq_num(&mut self) -> PResult<SeqNum> {
        if self.eat(b'*') {
            return Ok(SeqNum::Star);
        }
        match self.number()? {
            0 => err("sequence numbers start at 1"),
            n => Ok(SeqNum::Num(n)),
        }
    }

    pub(crate) fn sequence_set(&mut self) -> PResult<SeqSet> {
        let mut ranges = Vec::new();
        loop {
            let a = self.seq_num()?;
            let b = if self.eat(b':') { self.seq_num()? } else { a };
            ranges.push((a, b));
            if !self.eat(b',') {
                break;
            }
        }
        Ok(SeqSet(ranges))
    }

    /// `\System` flag, `\*` or keyword atom.
    pub(crate) fn flag(&mut self) -> PResult<String> {
        let backslash = self.eat(b'\\');
        if backslash && self.eat(b'*') {
            return Ok("\\*".into());
        }
        let a = self.atom()?;
        Ok(if backslash { format!("\\{a}") } else { a })
    }

    /// `(flag flag ...)`.
    pub(crate) fn flag_list(&mut self) -> PResult<Vec<String>> {
        self.expect(b'(')?;
        let mut flags = Vec::new();
        self.opt_sp();
        while !self.eat(b')') {
            flags.push(self.flag()?);
            self.opt_sp();
            if self.is_end() {
                return err("unterminated flag list");
            }
        }
        Ok(flags)
    }

    /// `(item item ...)` of atoms, uppercased.
    pub(crate) fn atom_list(&mut self) -> PResult<Vec<String>> {
        self.expect(b'(')?;
        let mut items = Vec::new();
        self.opt_sp();
        while !self.eat(b')') {
            items.push(self.atom()?.to_ascii_uppercase());
            self.opt_sp();
            if self.is_end() {
                return err("unterminated list");
            }
        }
        Ok(items)
    }

    /// Search date `d-Mon-yyyy`, optionally quoted.
    pub(crate) fn date(&mut self) -> PResult<NaiveDate> {
        let text = if self.peek() == Some(b'"') {
            self.quoted()?
        } else {
            self.take_while(|c| c.is_ascii_alphanumeric() || c == b'-')
                .to_vec()
        };
        parse_date(std::str::from_utf8(&text).unwrap_or_default())
            .ok_or_else(|| ParseError("invalid date".into()))
    }

    /// Quoted `dd-Mon-yyyy hh:mm:ss +zzzz`.
    pub(crate) fn date_time(&mut self) -> PResult<DateTime<FixedOffset>> {
        let text = self.quoted()?;
        parse_date_time(std::str::from_utf8(&text).unwrap_or_default())
            .ok_or_else(|| ParseError("invalid date-time".into()))
    }
}

pub(crate) fn parse_date(text: &str) -> Option<NaiveDate> {
    let mut it = text.trim().split('-');
    let day: u32 = it.next()?.trim().parse().ok()?;
    let month_name = it.next()?;
    let month = MONTHS
        .iter()
        .position(|m| m.eq_ignore_ascii_case(month_name))? as u32
        + 1;
    let year: i32 = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    NaiveDate::from_ymd_opt(year, month, day)
}

pub(crate) fn parse_date_time(text: &str) -> Option<DateTime<FixedOffset>> {
    let mut parts = text.split_whitespace();
    let date = parse_date(parts.next()?)?;
    let time = NaiveTime::parse_from_str(parts.next()?, "%H:%M:%S").ok()?;
    let zone = parts.next()?;
    if parts.next().is_some() || zone.len() != 5 || !zone.is_ascii() {
        return None;
    }
    let sign = match &zone[..1] {
        "+" => 1,
        "-" => -1,
        _ => return None,
    };
    let hours: i32 = zone[1..3].parse().ok()?;
    let minutes: i32 = zone[3..5].parse().ok()?;
    let offset = FixedOffset::east_opt(sign * (hours * 3600 + minutes * 60))?;
    NaiveDateTime::new(date, time)
        .and_local_timezone(offset)
        .single()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings() {
        let mut p = Parser::new(b"atom \"quo\\\"ted\" {5}\r\nlit\r\n {3+}\r\nabc");
        assert_eq!(p.astring().unwrap(), b"atom");
        p.sp().unwrap();
        assert_eq!(p.astring().unwrap(), b"quo\"ted");
        p.sp().unwrap();
        assert_eq!(p.astring().unwrap(), b"lit\r\n");
        p.sp().unwrap();
        assert_eq!(p.astring().unwrap(), b"abc");
        assert!(p.is_end());
    }

    #[test]
    fn sequence_sets() {
        let s = Parser::new(b"1:3,7,10:*").sequence_set().unwrap();
        assert!(s.contains(2, 20));
        assert!(!s.contains(5, 20));
        assert!(s.contains(15, 20));
        let s = Parser::new(b"100:*").sequence_set().unwrap();
        assert!(s.contains(50, 50));
        assert!(!s.contains(49, 50));
        assert!(Parser::new(b"0").sequence_set().is_err());
    }

    #[test]
    fn dates() {
        assert_eq!(
            parse_date("1-Feb-1994"),
            NaiveDate::from_ymd_opt(1994, 2, 1)
        );
        let dt = parse_date_time(" 7-Oct-2025 10:20:30 -0300").unwrap();
        assert_eq!(dt.to_rfc3339(), "2025-10-07T10:20:30-03:00");
        let mut p = Parser::new(b"(\\Seen $Junk \\*)");
        assert_eq!(p.flag_list().unwrap(), vec!["\\Seen", "$Junk", "\\*"]);
    }
}
