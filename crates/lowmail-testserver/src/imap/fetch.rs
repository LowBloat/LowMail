//! FETCH attributes: parsing, ENVELOPE / BODYSTRUCTURE generation and body
//! section extraction.

use chrono::{DateTime, FixedOffset};
use mail_parser::{MessageParser, MessagePart, MessageRef};

use super::parser::{PResult, ParseError, Parser, is_atom_char};
use super::wire::{self, push_literal, push_nstring, push_string};
use crate::message::{MessageData, header_fields, unfold};

/// What a body section refers to inside the addressed part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SectionText {
    /// `[]` or `[1.2]`.
    Full,
    Header,
    HeaderFields {
        not: bool,
        fields: Vec<String>,
    },
    Text,
    Mime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Section {
    pub path: Vec<u32>,
    pub text: SectionText,
}

impl Section {
    /// The section spec as echoed in the response, without brackets.
    fn echo(&self) -> String {
        let mut out = self.path.iter().map(u32::to_string).collect::<Vec<_>>().join(".");
        let name = match &self.text {
            SectionText::Full => return out,
            SectionText::Header => "HEADER".to_owned(),
            SectionText::Text => "TEXT".to_owned(),
            SectionText::Mime => "MIME".to_owned(),
            SectionText::HeaderFields { not, fields } => {
                let list: Vec<String> =
                    fields.iter().map(|f| if !f.is_empty() && f.bytes().all(is_atom_char) { f.clone() } else { wire::string(f) }).collect();
                format!("HEADER.FIELDS{} ({})", if *not { ".NOT" } else { "" }, list.join(" "))
            }
        };
        if !out.is_empty() {
            out.push('.');
        }
        out.push_str(&name);
        out
    }
}

/// One requested FETCH data item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FetchAtt {
    Uid,
    Flags,
    InternalDate,
    Size,
    Envelope,
    BodyStructure,
    Body,
    Rfc822,
    Rfc822Header,
    Rfc822Text,
    Section { peek: bool, section: Section, partial: Option<(u32, u32)> },
}

impl FetchAtt {
    /// Whether fetching this item sets `\Seen`.
    pub(crate) fn sets_seen(&self) -> bool {
        matches!(self, FetchAtt::Rfc822 | FetchAtt::Rfc822Text | FetchAtt::Section { peek: false, .. })
    }
}

/// Parses a macro, a single item or a parenthesized item list.
pub(crate) fn parse_atts(p: &mut Parser<'_>) -> PResult<Vec<FetchAtt>> {
    use FetchAtt::*;
    if p.eat(b'(') {
        let mut atts = Vec::new();
        p.opt_sp();
        while !p.eat(b')') {
            atts.push(parse_att(p)?);
            p.opt_sp();
            if p.is_end() {
                return Err(ParseError("unterminated fetch list".into()));
            }
        }
        if atts.is_empty() {
            return Err(ParseError("empty fetch list".into()));
        }
        return Ok(atts);
    }
    if p.keyword("ALL") {
        return Ok(vec![Flags, InternalDate, Size, Envelope]);
    }
    if p.keyword("FAST") {
        return Ok(vec![Flags, InternalDate, Size]);
    }
    if p.keyword("FULL") {
        return Ok(vec![Flags, InternalDate, Size, Envelope, Body]);
    }
    Ok(vec![parse_att(p)?])
}

fn parse_att(p: &mut Parser<'_>) -> PResult<FetchAtt> {
    let name = p.take_while(|c| is_atom_char(c) && c != b'[' && c != b'<');
    let name = String::from_utf8_lossy(name).to_ascii_uppercase();
    Ok(match name.as_str() {
        "UID" => FetchAtt::Uid,
        "FLAGS" => FetchAtt::Flags,
        "INTERNALDATE" => FetchAtt::InternalDate,
        "RFC822.SIZE" => FetchAtt::Size,
        "ENVELOPE" => FetchAtt::Envelope,
        "BODYSTRUCTURE" => FetchAtt::BodyStructure,
        "RFC822" => FetchAtt::Rfc822,
        "RFC822.HEADER" => FetchAtt::Rfc822Header,
        "RFC822.TEXT" => FetchAtt::Rfc822Text,
        "BODY" | "BODY.PEEK" if p.peek() == Some(b'[') => {
            let section = parse_section(p)?;
            let partial = if p.eat(b'<') {
                let start = p.number()?;
                p.expect(b'.')?;
                let len = p.number()?;
                p.expect(b'>')?;
                Some((start, len))
            } else {
                None
            };
            FetchAtt::Section { peek: name == "BODY.PEEK", section, partial }
        }
        "BODY" => FetchAtt::Body,
        "" => return Err(ParseError("expected fetch attribute".into())),
        other => return Err(ParseError(format!("unsupported fetch attribute {other}"))),
    })
}

fn parse_section(p: &mut Parser<'_>) -> PResult<Section> {
    p.expect(b'[')?;
    let mut path = Vec::new();
    if p.peek().is_some_and(|c| c.is_ascii_digit()) {
        loop {
            let n = p.number()?;
            if n == 0 {
                return Err(ParseError("section parts start at 1".into()));
            }
            path.push(n);
            if p.peek() == Some(b'.') && p.peek_n(1).is_some_and(|c| c.is_ascii_digit()) {
                p.eat(b'.');
                continue;
            }
            break;
        }
        if p.eat(b']') {
            return Ok(Section { path, text: SectionText::Full });
        }
        p.expect(b'.')?;
    } else if p.eat(b']') {
        return Ok(Section { path, text: SectionText::Full });
    }
    let kw = p.take_while(|c| c.is_ascii_alphanumeric() || c == b'.');
    let text = match String::from_utf8_lossy(kw).to_ascii_uppercase().as_str() {
        "HEADER" => SectionText::Header,
        "TEXT" => SectionText::Text,
        "MIME" if !path.is_empty() => SectionText::Mime,
        kw @ ("HEADER.FIELDS" | "HEADER.FIELDS.NOT") => {
            let not = kw.ends_with(".NOT");
            p.sp()?;
            p.expect(b'(')?;
            let mut fields = Vec::new();
            p.opt_sp();
            while !p.eat(b')') {
                fields.push(p.astring_utf8()?);
                p.opt_sp();
                if p.is_end() {
                    return Err(ParseError("unterminated header list".into()));
                }
            }
            SectionText::HeaderFields { not, fields }
        }
        other => return Err(ParseError(format!("invalid section {other}"))),
    };
    p.expect(b']')?;
    Ok(Section { path, text })
}

/// Per-message data needed to render FETCH items.
pub(crate) struct ItemCtx<'a> {
    pub uid: u32,
    pub flags: &'a [String],
    pub recent: bool,
    pub internal_date: &'a DateTime<FixedOffset>,
    pub data: &'a MessageData,
}

/// Appends one `NAME value` FETCH item.
pub(crate) fn render(att: &FetchAtt, ctx: &ItemCtx<'_>, out: &mut Vec<u8>) {
    match att {
        FetchAtt::Uid => out.extend_from_slice(format!("UID {}", ctx.uid).as_bytes()),
        FetchAtt::Flags => out.extend_from_slice(format!("FLAGS {}", wire::flag_list(ctx.flags, ctx.recent)).as_bytes()),
        FetchAtt::InternalDate => out.extend_from_slice(format!("INTERNALDATE {}", wire::date_time(ctx.internal_date)).as_bytes()),
        FetchAtt::Size => out.extend_from_slice(format!("RFC822.SIZE {}", ctx.data.raw.len()).as_bytes()),
        FetchAtt::Envelope => {
            out.extend_from_slice(b"ENVELOPE ");
            out.extend_from_slice(&ctx.data.meta().envelope);
        }
        FetchAtt::BodyStructure => {
            out.extend_from_slice(b"BODYSTRUCTURE ");
            out.extend_from_slice(&ctx.data.meta().body_structure);
        }
        FetchAtt::Body => {
            out.extend_from_slice(b"BODY ");
            out.extend_from_slice(&ctx.data.meta().body);
        }
        FetchAtt::Rfc822 => {
            out.extend_from_slice(b"RFC822 ");
            push_literal(out, &ctx.data.raw);
        }
        FetchAtt::Rfc822Header => {
            out.extend_from_slice(b"RFC822.HEADER ");
            push_literal(out, ctx.data.header());
        }
        FetchAtt::Rfc822Text => {
            out.extend_from_slice(b"RFC822.TEXT ");
            push_literal(out, ctx.data.text());
        }
        FetchAtt::Section { section, partial, .. } => {
            let data = extract(ctx.data, section);
            out.extend_from_slice(format!("BODY[{}]", section.echo()).as_bytes());
            match partial {
                Some((start, len)) => {
                    out.extend_from_slice(format!("<{start}> ").as_bytes());
                    let start = (*start as usize).min(data.len());
                    let end = start.saturating_add(*len as usize).min(data.len());
                    push_literal(out, &data[start..end]);
                }
                None => {
                    out.push(b' ');
                    push_literal(out, &data);
                }
            }
        }
    }
}

/// Bytes of a body section; empty when the section does not exist.
fn extract(data: &MessageData, section: &Section) -> Vec<u8> {
    if section.path.is_empty() {
        return match &section.text {
            SectionText::Full => data.raw.clone(),
            SectionText::Header => data.header().to_vec(),
            SectionText::Text => data.text().to_vec(),
            SectionText::HeaderFields { not, fields } => filter_header(data.header(), fields, *not),
            SectionText::Mime => Vec::new(),
        };
    }
    let Some(msg) = MessageParser::new().parse(&data.raw) else {
        return Vec::new();
    };
    let Some(part) = resolve(msg.root(), &section.path) else {
        return Vec::new();
    };
    match &section.text {
        SectionText::Full => part.raw_body().to_vec(),
        SectionText::Mime => part.raw_headers().to_vec(),
        text => {
            let Some(nested) = part.nested() else {
                return Vec::new();
            };
            let root = nested.root_part();
            match text {
                SectionText::Header => root.raw_headers().to_vec(),
                SectionText::Text => root.raw_body().to_vec(),
                SectionText::HeaderFields { not, fields } => filter_header(root.raw_headers(), fields, *not),
                _ => Vec::new(),
            }
        }
    }
}

/// Resolves an IMAP part number (`1.2.3`) to a MIME part.
fn resolve<'m>(msg: MessageRef<'m>, path: &[u32]) -> Option<MessagePart<'m>> {
    let mut part = msg.root_part();
    for (i, &n) in path.iter().enumerate() {
        if i > 0
            && let Some(nested) = part.nested()
        {
            part = nested.root_part();
        }
        if part.is_multipart() {
            part = part.children().nth((n as usize).checked_sub(1)?)?;
        } else if n != 1 {
            return None;
        }
    }
    Some(part)
}

/// `HEADER.FIELDS` / `HEADER.FIELDS.NOT`, terminated by a blank line.
fn filter_header(header: &[u8], fields: &[String], not: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for field in header_fields(header) {
        let listed = fields.iter().any(|name| field.is(name));
        if listed != not {
            out.extend_from_slice(field.raw);
            if !field.raw.ends_with(b"\n") {
                out.extend_from_slice(b"\r\n");
            }
        }
    }
    out.extend_from_slice(b"\r\n");
    out
}

// ---------------------------------------------------------------------------
// ENVELOPE

struct EnvAddress {
    name: Option<Vec<u8>>,
    mailbox: Vec<u8>,
    host: Vec<u8>,
}

/// Writes the ENVELOPE structure for a header block, using raw (still
/// RFC 2047 encoded) header values like real servers do.
pub(crate) fn envelope(header: &[u8], out: &mut Vec<u8>) {
    let fields = header_fields(header);
    let get = |name: &str| fields.iter().find(|f| f.is(name)).map(|f| unfold(f.value));
    let addresses = |name: &str| get(name).map(|v| parse_addresses(&v)).filter(|list| !list.is_empty());
    let from = addresses("From");
    let sender = addresses("Sender").or_else(|| addresses("From"));
    let reply_to = addresses("Reply-To").or_else(|| addresses("From"));

    out.push(b'(');
    push_nstring(out, get("Date").as_deref());
    out.push(b' ');
    push_nstring(out, get("Subject").as_deref());
    for list in [from, sender, reply_to, addresses("To"), addresses("Cc"), addresses("Bcc")] {
        out.push(b' ');
        push_address_list(out, list.as_deref());
    }
    out.push(b' ');
    push_nstring(out, get("In-Reply-To").as_deref());
    out.push(b' ');
    push_nstring(out, get("Message-ID").as_deref());
    out.push(b')');
}

fn push_address_list(out: &mut Vec<u8>, list: Option<&[EnvAddress]>) {
    let Some(list) = list else {
        out.extend_from_slice(b"NIL");
        return;
    };
    out.push(b'(');
    for addr in list {
        out.push(b'(');
        push_nstring(out, addr.name.as_deref());
        out.extend_from_slice(b" NIL ");
        push_string(out, &addr.mailbox);
        out.push(b' ');
        push_string(out, &addr.host);
        out.push(b')');
    }
    out.push(b')');
}

/// Splits an address list at top-level commas, ignoring group names.
fn split_addresses(value: &[u8]) -> Vec<&[u8]> {
    let mut items = Vec::new();
    let (mut start, mut quote, mut escape, mut paren, mut angle) = (0, false, false, 0u32, false);
    for (i, &c) in value.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        match c {
            b'\\' if quote || paren > 0 => escape = true,
            b'"' if paren == 0 => quote = !quote,
            b'(' if !quote => paren += 1,
            b')' if !quote && paren > 0 => paren -= 1,
            b'<' if !quote && paren == 0 => angle = true,
            b'>' if !quote && paren == 0 => angle = false,
            b':' if !quote && paren == 0 && !angle => start = i + 1,
            b',' | b';' if !quote && paren == 0 && !angle => {
                items.push(&value[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    items.push(&value[start..]);
    items
}

/// Removes quotes, backslash escapes and comments from a display name.
fn clean_phrase(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut quote, mut paren, mut i) = (false, 0u32, 0);
    while i < raw.len() {
        let c = raw[i];
        match c {
            b'\\' if quote && i + 1 < raw.len() => {
                out.push(raw[i + 1]);
                i += 1;
            }
            b'"' if paren == 0 => quote = !quote,
            b'(' if !quote => paren += 1,
            b')' if !quote && paren > 0 => paren -= 1,
            _ if paren > 0 => {}
            _ => out.push(c),
        }
        i += 1;
    }
    out.trim_ascii().to_vec()
}

/// Extracts the text of the first comment, e.g. `user@host (Name)`.
fn first_comment(raw: &[u8]) -> Option<Vec<u8>> {
    let open = raw.iter().position(|&b| b == b'(')?;
    let close = raw[open..].iter().position(|&b| b == b')')? + open;
    let text = raw[open + 1..close].trim_ascii();
    (!text.is_empty()).then(|| text.to_vec())
}

fn parse_addresses(value: &[u8]) -> Vec<EnvAddress> {
    let mut list = Vec::new();
    for item in split_addresses(value) {
        let item = item.trim_ascii();
        if item.is_empty() {
            continue;
        }
        let (name, spec) = match find_angle(item) {
            Some(lt) => {
                let rest = &item[lt + 1..];
                let gt = rest.iter().position(|&b| b == b'>').unwrap_or(rest.len());
                let mut spec = &rest[..gt];
                if spec.starts_with(b"@")
                    && let Some(colon) = spec.iter().position(|&b| b == b':')
                {
                    spec = &spec[colon + 1..];
                }
                (clean_phrase(&item[..lt]), spec.trim_ascii().to_vec())
            }
            None => (
                first_comment(item).unwrap_or_default(),
                clean_phrase(item).split(|b| b.is_ascii_whitespace()).collect::<Vec<_>>().concat(),
            ),
        };
        if spec.is_empty() {
            continue;
        }
        let (mailbox, host) = match spec.iter().rposition(|&b| b == b'@') {
            Some(at) => (spec[..at].to_vec(), spec[at + 1..].to_vec()),
            None => (spec.clone(), b"MISSING_DOMAIN".to_vec()),
        };
        list.push(EnvAddress { name: (!name.is_empty()).then_some(name), mailbox, host });
    }
    list
}

/// Position of the `<` that starts an angle-addr, outside quotes.
fn find_angle(item: &[u8]) -> Option<usize> {
    let mut quote = false;
    let mut escape = false;
    for (i, &c) in item.iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        match c {
            b'\\' if quote => escape = true,
            b'"' => quote = !quote,
            b'<' if !quote => return Some(i),
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// BODYSTRUCTURE

/// Raw (unfolded) value of a MIME header of `part`.
fn raw_header(part: &MessagePart<'_>, name: &str) -> Option<Vec<u8>> {
    part.headers().iter().find(|h| h.raw_name().eq_ignore_ascii_case(name)).map(|h| unfold(h.raw_value()))
}

fn line_count(body: &[u8]) -> usize {
    let lines = body.iter().filter(|&&b| b == b'\n').count();
    if body.is_empty() || body.ends_with(b"\n") { lines } else { lines + 1 }
}

fn push_params<'a>(out: &mut Vec<u8>, params: impl Iterator<Item = (&'a str, &'a str)>, upper_names: bool) {
    let mut any = false;
    for (name, value) in params {
        out.push(if any { b' ' } else { b'(' });
        any = true;
        let name = if upper_names { name.to_ascii_uppercase() } else { name.to_owned() };
        push_string(out, name.as_bytes());
        out.push(b' ');
        push_string(out, value.as_bytes());
    }
    if any {
        out.push(b')');
    } else {
        out.extend_from_slice(b"NIL");
    }
}

fn push_disposition(out: &mut Vec<u8>, part: &MessagePart<'_>) {
    match part.content_disposition() {
        Some(disp) => {
            out.push(b'(');
            push_string(out, disp.ctype().as_bytes());
            out.push(b' ');
            push_params(out, disp.attributes(), false);
            out.push(b')');
        }
        None => out.extend_from_slice(b"NIL"),
    }
}

/// A placeholder `text/plain` structure for unparseable content.
pub(crate) fn fallback_body_structure(size: usize, out: &mut Vec<u8>) {
    out.extend_from_slice(format!("(\"TEXT\" \"PLAIN\" (\"CHARSET\" \"us-ascii\") NIL NIL \"7BIT\" {size} 0)").as_bytes());
}

/// Writes BODYSTRUCTURE (`extended`) or BODY for a MIME part.
pub(crate) fn body_structure(part: MessagePart<'_>, extended: bool, out: &mut Vec<u8>) {
    let content_type = part.content_type();
    if part.is_multipart() {
        out.push(b'(');
        let mut children = 0;
        for child in part.children() {
            body_structure(child, extended, out);
            children += 1;
        }
        if children == 0 {
            fallback_body_structure(0, out);
        }
        let subtype = content_type.and_then(|c| c.subtype()).unwrap_or("mixed");
        out.push(b' ');
        push_string(out, subtype.to_ascii_uppercase().as_bytes());
        if extended {
            out.push(b' ');
            match content_type {
                Some(ct) => push_params(out, ct.attributes(), true),
                None => out.extend_from_slice(b"NIL"),
            }
            out.push(b' ');
            push_disposition(out, &part);
            out.extend_from_slice(b" NIL NIL");
        }
        out.push(b')');
        return;
    }

    let (ty, subtype) = match content_type {
        Some(ct) => {
            let ty = ct.ctype().to_ascii_uppercase();
            let subtype =
                ct.subtype().map(str::to_ascii_uppercase).unwrap_or_else(|| if ty == "TEXT" { "PLAIN" } else { "OCTET-STREAM" }.to_owned());
            (ty, subtype)
        }
        None => ("TEXT".to_owned(), "PLAIN".to_owned()),
    };
    let body = part.raw_body();

    out.push(b'(');
    push_string(out, ty.as_bytes());
    out.push(b' ');
    push_string(out, subtype.as_bytes());
    out.push(b' ');
    let has_charset = content_type.is_some_and(|ct| ct.has_attribute("charset"));
    let default_charset = (ty == "TEXT" && !has_charset).then_some(("charset", "us-ascii"));
    let params = content_type.into_iter().flat_map(|ct| ct.attributes()).chain(default_charset);
    push_params(out, params, true);
    out.push(b' ');
    push_nstring(out, raw_header(&part, "Content-ID").as_deref());
    out.push(b' ');
    push_nstring(out, raw_header(&part, "Content-Description").as_deref());
    out.push(b' ');
    let encoding = raw_header(&part, "Content-Transfer-Encoding")
        .filter(|e| !e.is_empty())
        .map(|e| e.to_ascii_uppercase())
        .unwrap_or_else(|| b"7BIT".to_vec());
    push_string(out, &encoding);
    out.extend_from_slice(format!(" {}", body.len()).as_bytes());

    if ty == "TEXT" {
        out.extend_from_slice(format!(" {}", line_count(body)).as_bytes());
    } else if ty == "MESSAGE"
        && subtype == "RFC822"
        && let Some(nested) = part.nested()
    {
        let root = nested.root_part();
        out.push(b' ');
        envelope(root.raw_headers(), out);
        out.push(b' ');
        body_structure(root, extended, out);
        out.extend_from_slice(format!(" {}", line_count(body)).as_bytes());
    }

    if extended {
        out.extend_from_slice(b" NIL ");
        push_disposition(out, &part);
        out.extend_from_slice(b" NIL NIL");
    }
    out.push(b')');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(header: &str) -> String {
        let mut out = Vec::new();
        envelope(header.as_bytes(), &mut out);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn envelope_addresses() {
        let e = env(
            "From: \"Silva, Maria\" <maria@x.com>\r\nTo: a@b.c, =?UTF-8?Q?Jo=C3=A3o?= <joao@d.e>\r\nCc: undisclosed-recipients:;\r\nSubject: Hi\r\n\r\n",
        );
        assert_eq!(
            e,
            "(NIL \"Hi\" ((\"Silva, Maria\" NIL \"maria\" \"x.com\")) ((\"Silva, Maria\" NIL \"maria\" \"x.com\")) ((\"Silva, Maria\" NIL \"maria\" \"x.com\")) ((NIL NIL \"a\" \"b.c\")(\"=?UTF-8?Q?Jo=C3=A3o?=\" NIL \"joao\" \"d.e\")) NIL NIL NIL NIL)"
        );
    }

    #[test]
    fn sections() {
        let mut p = Parser::new(b"BODY.PEEK[HEADER.FIELDS (From Subject)]<0.100>");
        let atts = parse_atts(&mut p).unwrap();
        let FetchAtt::Section { peek, section, partial } = &atts[0] else { panic!() };
        assert!(*peek);
        assert_eq!(partial, &Some((0, 100)));
        assert_eq!(section.echo(), "HEADER.FIELDS (From Subject)");
        let mut p = Parser::new(b"(UID BODY[1.2.MIME] BODY[2] BODY[])");
        let atts = parse_atts(&mut p).unwrap();
        assert_eq!(atts.len(), 4);
        let FetchAtt::Section { section, .. } = &atts[1] else { panic!() };
        assert_eq!(section.path, vec![1, 2]);
        assert_eq!(section.echo(), "1.2.MIME");
    }

    #[test]
    fn multipart_structure() {
        let raw = b"Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nhello\r\n--b\r\nContent-Type: application/pdf; name=\"x.pdf\"\r\nContent-Disposition: attachment; filename=\"x.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nAAAA\r\n--b--\r\n";
        let data = MessageData::new(raw.to_vec());
        let bs = String::from_utf8(data.meta().body_structure.clone()).unwrap();
        assert_eq!(
            bs,
            "((\"TEXT\" \"PLAIN\" (\"CHARSET\" \"utf-8\") NIL NIL \"7BIT\" 5 1 NIL NIL NIL NIL)(\"APPLICATION\" \"PDF\" (\"NAME\" \"x.pdf\") NIL NIL \"BASE64\" 4 NIL (\"attachment\" (\"filename\" \"x.pdf\")) NIL NIL) \"MIXED\" (\"BOUNDARY\" \"b\") NIL NIL NIL)"
        );
        let section = Section { path: vec![2], text: SectionText::Full };
        assert_eq!(extract(&data, &section), b"AAAA");
        let section = Section { path: vec![1], text: SectionText::Mime };
        assert_eq!(extract(&data, &section), b"Content-Type: text/plain; charset=utf-8\r\n\r\n");
    }
}
