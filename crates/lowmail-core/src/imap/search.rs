//! IMAP SEARCH criteria from the list filter.

use chrono::{Days, NaiveDate};

use crate::model::{Filter, SearchScope};

/// A string argument: quoted when 7-bit, otherwise a non-synchronizing
/// literal (LITERAL+), or UTF-8 inside quotes, which most servers accept.
pub fn astring(s: &str, literal_plus: bool) -> String {
    let ascii = s.bytes().all(|b| (0x20..0x7f).contains(&b));
    if !ascii && literal_plus {
        return format!("{{{}+}}\r\n{}", s.len(), s);
    }
    let clean: String = s.chars().filter(|c| *c != '\r' && *c != '\n').collect();
    format!("\"{}\"", clean.replace('\\', "\\\\").replace('"', "\\\""))
}

fn imap_date(date: NaiveDate) -> String {
    date.format("%-d-%b-%Y").to_string()
}

/// Search keys for `filter` and whether they need `CHARSET UTF-8`.
pub fn build_criteria(filter: &Filter, literal_plus: bool) -> (String, bool) {
    let mut keys: Vec<String> = vec!["UNDELETED".into()];
    let text = filter.text.trim();
    let utf8 = !text.is_ascii();
    if !text.is_empty() {
        let s = astring(text, literal_plus);
        keys.push(match filter.scope {
            SearchScope::Headers => format!("OR OR OR FROM {s} TO {s} CC {s} SUBJECT {s}"),
            SearchScope::From => format!("FROM {s}"),
            SearchScope::To => format!("OR TO {s} CC {s}"),
            SearchScope::Subject => format!("SUBJECT {s}"),
            SearchScope::Body => format!("BODY {s}"),
            SearchScope::Everything => format!("TEXT {s}"),
        });
    }
    if filter.unread {
        keys.push("UNSEEN".into());
    }
    if filter.flagged {
        keys.push("FLAGGED".into());
    }
    if filter.answered {
        keys.push("ANSWERED".into());
    }
    if filter.unanswered {
        keys.push("UNANSWERED".into());
    }
    if let Some(d) = filter.since.as_deref().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) {
        keys.push(format!("SINCE {}", imap_date(d)));
    }
    if let Some(d) = filter.before.as_deref().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) {
        // BEFORE is exclusive; the filter's end date is inclusive.
        let next = d.checked_add_days(Days::new(1)).unwrap_or(d);
        keys.push(format!("BEFORE {}", imap_date(next)));
    }
    if let Some(n) = filter.larger {
        keys.push(format!("LARGER {n}"));
    }
    (keys.join(" "), utf8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn criteria() {
        let f = Filter { text: "relatório".into(), unread: true, since: Some("2026-03-01".into()), ..Default::default() };
        let (c, utf8) = build_criteria(&f, true);
        assert!(utf8);
        assert!(c.starts_with("UNDELETED OR OR OR FROM {10+}\r\nrelatório TO"));
        assert!(c.ends_with("UNSEEN SINCE 1-Mar-2026"));
        let (c, _) = build_criteria(&Filter { text: "say \"hi\"".into(), scope: SearchScope::Subject, ..Default::default() }, false);
        assert_eq!(c, "UNDELETED SUBJECT \"say \\\"hi\\\"\"");
    }
}
