//! vCard import and export for the address book (versions 2.1, 3.0 and 4.0 in,
//! 3.0 out), plus CSV import from webmail and Outlook exports.

use crate::model::Contact;

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n").replace(',', "\\,").replace(';', "\\;")
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Folds a content line at 75 octets as RFC 6350 asks.
fn fold(line: &str) -> String {
    let mut out = String::new();
    let mut count = 0;
    for c in line.chars() {
        let len = c.len_utf8();
        if count + len > 75 {
            out.push_str("\r\n ");
            count = 1;
        }
        out.push(c);
        count += len;
    }
    out.push_str("\r\n");
    out
}

pub fn export(contacts: &[Contact]) -> String {
    let mut out = String::new();
    for c in contacts {
        out.push_str("BEGIN:VCARD\r\nVERSION:3.0\r\n");
        let name = if c.name.is_empty() { c.emails.first().cloned().unwrap_or_default() } else { c.name.clone() };
        out.push_str(&fold(&format!("FN:{}", escape(&name))));
        let mut parts = name.rsplitn(2, ' ');
        let last = parts.next().unwrap_or("");
        let first = parts.next().unwrap_or("");
        out.push_str(&fold(&format!("N:{};{};;;", escape(last), escape(first))));
        for (i, e) in c.emails.iter().enumerate() {
            let pref = if i == 0 { ";TYPE=INTERNET,PREF" } else { ";TYPE=INTERNET" };
            out.push_str(&fold(&format!("EMAIL{pref}:{}", escape(e))));
        }
        for p in &c.phones {
            out.push_str(&fold(&format!("TEL:{}", escape(p))));
        }
        if !c.organization.is_empty() {
            out.push_str(&fold(&format!("ORG:{}", escape(&c.organization))));
        }
        if !c.job_title.is_empty() {
            out.push_str(&fold(&format!("TITLE:{}", escape(&c.job_title))));
        }
        if let Some(b) = c.birthday.as_ref().filter(|b| !b.is_empty()) {
            out.push_str(&fold(&format!("BDAY:{b}")));
        }
        if !c.notes.is_empty() {
            out.push_str(&fold(&format!("NOTE:{}", escape(&c.notes))));
        }
        if !c.groups.is_empty() {
            out.push_str(&fold(&format!("CATEGORIES:{}", c.groups.iter().map(|g| escape(g)).collect::<Vec<_>>().join(","))));
        }
        out.push_str(&fold(&format!("UID:{}", c.id)));
        out.push_str("END:VCARD\r\n");
    }
    out
}

/// Splits on `sep` unless escaped with a backslash.
fn split_unescaped(s: &str, sep: char) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            out.last_mut().unwrap().push('\\');
            out.last_mut().unwrap().push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == sep {
            out.push(String::new());
        } else {
            out.last_mut().unwrap().push(c);
        }
    }
    out.into_iter().map(|p| unescape(&p)).collect()
}

fn decode_qp(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'='
            && i + 2 < bytes.len() + 1
            && let (Some(h), Some(l)) = (bytes.get(i + 1), bytes.get(i + 2))
            && let Ok(v) = u8::from_str_radix(&format!("{}{}", *h as char, *l as char), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn import(text: &str) -> Vec<Contact> {
    // Unfold: a line starting with a space or tab continues the previous one
    // (and vCard 2.1 quoted-printable soft breaks end with '=').
    let mut lines: Vec<String> = Vec::new();
    for raw in text.replace("\r\n", "\n").split('\n') {
        if (raw.starts_with(' ') || raw.starts_with('\t')) && !lines.is_empty() {
            lines.last_mut().unwrap().push_str(&raw[1..]);
        } else if lines.last().is_some_and(|l| l.ends_with('=') && l.to_ascii_uppercase().contains("QUOTED-PRINTABLE")) {
            let last = lines.last_mut().unwrap();
            last.pop();
            last.push_str(raw);
        } else {
            lines.push(raw.to_string());
        }
    }
    let mut out = Vec::new();
    let mut current: Option<Contact> = None;
    for line in lines {
        let Some((head, value)) = line.split_once(':') else { continue };
        let mut params = head.split(';');
        let name = params.next().unwrap_or("").to_ascii_uppercase();
        let name = name.rsplit('.').next().unwrap_or(&name).to_string(); // item1.EMAIL
        let params: Vec<String> = params.map(|p| p.to_ascii_uppercase()).collect();
        let value = if params.iter().any(|p| p.contains("QUOTED-PRINTABLE")) { decode_qp(value) } else { value.to_string() };
        match name.as_str() {
            "BEGIN" if value.eq_ignore_ascii_case("VCARD") => current = Some(Contact::default()),
            "END" if value.eq_ignore_ascii_case("VCARD") => {
                if let Some(mut c) = current.take() {
                    if c.name.is_empty() {
                        c.name = c.emails.first().cloned().unwrap_or_default();
                    }
                    if !c.name.is_empty() || !c.emails.is_empty() {
                        out.push(c);
                    }
                }
            }
            _ => {
                let Some(c) = current.as_mut() else { continue };
                match name.as_str() {
                    "FN" => c.name = unescape(value.trim()),
                    "N" if c.name.is_empty() => {
                        let parts = split_unescaped(&value, ';');
                        let first = parts.get(1).map(String::as_str).unwrap_or("");
                        let last = parts.first().map(String::as_str).unwrap_or("");
                        c.name = format!("{first} {last}").trim().to_string();
                    }
                    "EMAIL" => {
                        let e = unescape(value.trim());
                        if !e.is_empty() {
                            c.emails.push(e);
                        }
                    }
                    "TEL" => c.phones.push(unescape(value.trim().trim_start_matches("tel:"))),
                    "ORG" => {
                        c.organization = split_unescaped(&value, ';').into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ")
                    }
                    "TITLE" => c.job_title = unescape(value.trim()),
                    "NOTE" => c.notes = unescape(&value),
                    "BDAY" => c.birthday = Some(value.trim().to_string()),
                    "CATEGORIES" => {
                        c.groups =
                            split_unescaped(&value, ',').into_iter().map(|g| g.trim().to_string()).filter(|g| !g.is_empty()).collect()
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

fn csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    let sep = if text.lines().next().is_some_and(|l| l.matches(';').count() > l.matches(',').count()) { ';' } else { ',' };
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' {
            quoted = true;
        } else if c == sep {
            row.push(std::mem::take(&mut field));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            if row.iter().any(|f| !f.is_empty()) {
                rows.push(std::mem::take(&mut row));
            } else {
                row.clear();
            }
        } else {
            field.push(c);
        }
    }
    row.push(field);
    if row.iter().any(|f| !f.is_empty()) {
        rows.push(row);
    }
    rows
}

/// Imports a CSV export (Roundcube, Gmail, Outlook) by guessing the columns.
pub fn import_csv(text: &str) -> Vec<Contact> {
    let rows = csv_rows(text);
    let Some(header) = rows.first() else { return Vec::new() };
    let col = |names: &[&str]| -> Vec<usize> {
        header
            .iter()
            .enumerate()
            .filter(|(_, h)| {
                let h = h.trim().to_lowercase();
                names.iter().any(|n| h == *n || (n.ends_with('*') && h.starts_with(n.trim_end_matches('*'))))
            })
            .map(|(i, _)| i)
            .collect()
    };
    let name_cols = col(&["name", "display name", "nome", "nome de exibição", "full name"]);
    let first_cols = col(&["first name", "given name", "nome próprio", "primeiro nome"]);
    let last_cols = col(&["last name", "family name", "sobrenome"]);
    let email_cols = col(&[
        "e-mail",
        "email",
        "e-mail address",
        "email address",
        "e-mail 2 address",
        "e-mail 3 address",
        "email 2",
        "email 3",
        "e-mail 1 - value",
        "e-mail 2 - value",
        "endereço de email",
        "endereço de e-mail",
    ]);
    let phone_cols =
        col(&["phone", "telefone", "mobile phone", "business phone", "home phone", "phone 1 - value", "phone 2 - value", "celular"]);
    let org_cols = col(&["company", "organization", "organization 1 - name", "empresa", "organização"]);
    let notes_cols = col(&["notes", "notas", "observações"]);
    let get = |row: &Vec<String>, cols: &[usize]| -> Vec<String> {
        cols.iter().filter_map(|&i| row.get(i)).map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect()
    };
    rows.iter()
        .skip(1)
        .filter_map(|row| {
            let mut emails: Vec<String> = Vec::new();
            for e in get(row, &email_cols) {
                for part in e.split([':', ';', ',']).map(str::trim) {
                    if part.contains('@') && !emails.contains(&part.to_string()) {
                        emails.push(part.to_string());
                    }
                }
            }
            let mut name = get(row, &name_cols).into_iter().next().unwrap_or_default();
            if name.is_empty() {
                name = format!(
                    "{} {}",
                    get(row, &first_cols).into_iter().next().unwrap_or_default(),
                    get(row, &last_cols).into_iter().next().unwrap_or_default()
                )
                .trim()
                .to_string();
            }
            if name.is_empty() && emails.is_empty() {
                return None;
            }
            Some(Contact {
                name,
                emails,
                phones: get(row, &phone_cols),
                organization: get(row, &org_cols).into_iter().next().unwrap_or_default(),
                notes: get(row, &notes_cols).into_iter().next().unwrap_or_default(),
                ..Default::default()
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let c = Contact {
            id: "1".into(),
            name: "Ana; Souza".into(),
            emails: vec!["ana@example.com".into(), "ana2@example.com".into()],
            phones: vec!["+55 11 99999-0000".into()],
            organization: "LowBloat".into(),
            notes: "Linha 1\nLinha 2".into(),
            groups: vec!["Trabalho".into(), "Amigos".into()],
            ..Default::default()
        };
        let text = export(&[c]);
        let back = import(&text);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].name, "Ana; Souza");
        assert_eq!(back[0].emails.len(), 2);
        assert_eq!(back[0].notes, "Linha 1\nLinha 2");
        assert_eq!(back[0].groups, vec!["Trabalho", "Amigos"]);
    }

    #[test]
    fn v21_quoted_printable() {
        let text = "BEGIN:VCARD\r\nVERSION:2.1\r\nN;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:Concei=C3=A7=C3=A3o;Jo=C3=A3o;;;\r\nEMAIL;INTERNET:joao@example.com\r\nEND:VCARD\r\n";
        let c = import(text);
        assert_eq!(c[0].name, "João Conceição");
    }

    #[test]
    fn csv() {
        let text = "First Name,Last Name,E-mail Address,Company\nAna,Souza,ana@example.com,ACME\n,,bob@example.com,\n";
        let c = import_csv(text);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].name, "Ana Souza");
        assert_eq!(c[1].emails, vec!["bob@example.com"]);
    }
}
