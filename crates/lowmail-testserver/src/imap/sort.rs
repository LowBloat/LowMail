//! SORT (RFC 5256) criteria parsing and ordering.

use std::cmp::Ordering;

use super::parser::{PResult, ParseError, Parser};
use super::search::Candidate;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SortKey {
    Arrival,
    Cc,
    Date,
    From,
    Size,
    Subject,
    To,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Criterion {
    pub key: SortKey,
    pub reverse: bool,
}

/// Parses `(REVERSE DATE SUBJECT ...)`.
pub(crate) fn parse_criteria(p: &mut Parser<'_>) -> PResult<Vec<Criterion>> {
    let words = p.atom_list()?;
    let mut criteria = Vec::new();
    let mut reverse = false;
    for word in words {
        let key = match word.as_str() {
            "REVERSE" => {
                reverse = true;
                continue;
            }
            "ARRIVAL" => SortKey::Arrival,
            "CC" => SortKey::Cc,
            "DATE" => SortKey::Date,
            "FROM" => SortKey::From,
            "SIZE" => SortKey::Size,
            "SUBJECT" => SortKey::Subject,
            "TO" => SortKey::To,
            other => return Err(ParseError(format!("unknown sort key {other}"))),
        };
        criteria.push(Criterion { key, reverse });
        reverse = false;
    }
    if reverse || criteria.is_empty() {
        return Err(ParseError("invalid sort criteria".into()));
    }
    Ok(criteria)
}

fn compare(key: SortKey, a: &Candidate, b: &Candidate) -> Ordering {
    let (ma, mb) = (a.data.meta(), b.data.meta());
    match key {
        SortKey::Arrival => a.internal_date.cmp(&b.internal_date),
        SortKey::Date => {
            let date = |c: &Candidate, m: &crate::message::Meta| {
                m.sent.map_or(c.internal_date.timestamp(), |s| s.timestamp)
            };
            date(a, ma).cmp(&date(b, mb))
        }
        SortKey::Size => a.data.raw.len().cmp(&b.data.raw.len()),
        SortKey::From => ma.sort_from.cmp(&mb.sort_from),
        SortKey::To => ma.sort_to.cmp(&mb.sort_to),
        SortKey::Cc => ma.sort_cc.cmp(&mb.sort_cc),
        SortKey::Subject => ma.base_subject.cmp(&mb.base_subject),
    }
}

/// Sorts candidates by the criteria, ties broken by sequence number.
pub(crate) fn sort(candidates: &mut [Candidate], criteria: &[Criterion]) {
    candidates.sort_by(|a, b| {
        for c in criteria {
            let ord = compare(c.key, a, b);
            let ord = if c.reverse { ord.reverse() } else { ord };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        a.seq.cmp(&b.seq)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn criteria() {
        let c = parse_criteria(&mut Parser::new(b"(REVERSE DATE subject)")).unwrap();
        assert_eq!(
            c,
            vec![
                Criterion {
                    key: SortKey::Date,
                    reverse: true
                },
                Criterion {
                    key: SortKey::Subject,
                    reverse: false
                }
            ]
        );
        assert!(parse_criteria(&mut Parser::new(b"(REVERSE)")).is_err());
    }
}
