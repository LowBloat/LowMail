//! Makes received HTML safe to show: no scripts, forms or active content, and
//! no remote images, styles or fonts unless the reader allows them (they
//! tell the sender the message was opened). The UI also shows the result in
//! a sandboxed frame with a Content-Security-Policy; this is the first line.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// 1x1 transparent GIF that keeps the layout of blocked images.
pub const BLOCKED_IMAGE: &str = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

pub struct Sanitized {
    pub html: String,
    /// Remote content was found (and removed unless allowed).
    pub remote: bool,
    /// `cid:` references that were resolved, so those parts are not listed
    /// as attachments.
    pub used_cids: HashSet<String>,
}

const EXTRA_TAGS: &[&str] = &[
    "style",
    "center",
    "font",
    "big",
    "small",
    "strike",
    "u",
    "s",
    "tt",
    "caption",
    "col",
    "colgroup",
    "tbody",
    "thead",
    "tfoot",
    "wbr",
    "picture",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "nav",
    "aside",
    "figure",
    "figcaption",
    "mark",
    "time",
];

const GENERIC_ATTRS: &[&str] = &[
    "style",
    "class",
    "id",
    "align",
    "valign",
    "bgcolor",
    "width",
    "height",
    "border",
    "cellpadding",
    "cellspacing",
    "color",
    "face",
    "size",
    "dir",
    "lang",
    "title",
    "role",
    "colspan",
    "rowspan",
    "nowrap",
    "start",
    "type",
    "alt",
];

/// Sanitizes an HTML body. `cids` maps Content-IDs (without brackets) to
/// data: URLs of the inline parts.
pub fn sanitize(input: &str, cids: &HashMap<String, String>, allow_remote: bool) -> Sanitized {
    let remote = Arc::new(AtomicBool::new(false));
    let used = Arc::new(parking_lot::Mutex::new(HashSet::new()));
    let cids = Arc::new(cids.clone());

    let mut builder = ammonia::Builder::default();
    builder
        .rm_clean_content_tags(["style"])
        .add_tags(EXTRA_TAGS)
        .add_generic_attributes(GENERIC_ATTRS)
        .add_tag_attributes("img", ["src", "width", "height", "border", "alt", "title", "hspace", "vspace"])
        .add_tag_attributes("a", ["href", "name", "target"])
        .add_tag_attributes("td", ["background"])
        .add_tag_attributes("table", ["background", "summary"])
        .add_url_schemes(["data", "cid"])
        .link_rel(Some("noopener noreferrer"))
        .set_tag_attribute_value("a", "target", "_blank")
        .strip_comments(true)
        .url_relative(ammonia::UrlRelative::Deny);

    let (remote_cb, used_cb, cids_cb) = (remote.clone(), used.clone(), cids.clone());
    builder.attribute_filter(move |element, attribute, value| match (element, attribute) {
        (_, "style") => Some(Cow::Owned(filter_css(value, allow_remote, &remote_cb))),
        ("img", "src") => {
            let v = value.trim();
            let lower = v.to_ascii_lowercase();
            if let Some(cid) = lower.strip_prefix("cid:") {
                let key = cid.trim_matches(|c| c == '<' || c == '>');
                let found = cids_cb.iter().find(|(k, _)| k.eq_ignore_ascii_case(key));
                if let Some((k, url)) = found {
                    used_cb.lock().insert(k.clone());
                    return Some(Cow::Owned(url.clone()));
                }
                return Some(Cow::Borrowed(BLOCKED_IMAGE));
            }
            if lower.starts_with("data:image/") {
                return Some(Cow::Owned(v.to_string()));
            }
            if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("//") {
                remote_cb.store(true, Ordering::Relaxed);
                return if allow_remote { Some(Cow::Owned(v.to_string())) } else { Some(Cow::Borrowed(BLOCKED_IMAGE)) };
            }
            None
        }
        (_, "background") => {
            let lower = value.trim().to_ascii_lowercase();
            if lower.starts_with("http") {
                remote_cb.store(true, Ordering::Relaxed);
                if allow_remote {
                    return Some(Cow::Owned(value.to_string()));
                }
            }
            None
        }
        ("a", "href") => {
            let lower = value.trim().to_ascii_lowercase();
            if lower.starts_with("data:") || lower.starts_with("cid:") { None } else { Some(Cow::Owned(value.to_string())) }
        }
        _ => Some(Cow::Owned(value.to_string())),
    });

    let cleaned = builder.clean(input).to_string();
    let html = filter_style_blocks(&cleaned, allow_remote, &remote);
    let used_cids = used.lock().clone();
    Sanitized { html, remote: remote.load(Ordering::Relaxed), used_cids }
}

/// Removes remote `url(...)`, `@import` and behaviours from CSS.
fn filter_css(css: &str, allow_remote: bool, remote: &AtomicBool) -> String {
    let mut out = String::with_capacity(css.len());
    let lower = css.to_ascii_lowercase();
    let mut i = 0;
    while i < css.len() {
        if lower[i..].starts_with("url(") {
            let close = lower[i..].find(')').map(|e| i + e);
            let end = close.map(|c| c + 1).unwrap_or(css.len());
            let inner = css[i + 4..close.unwrap_or(css.len())].trim().trim_matches(|c| c == '"' || c == '\'');
            let inner_lower = inner.to_ascii_lowercase();
            if inner_lower.starts_with("data:image/") {
                out.push_str(&css[i..end]);
            } else if inner_lower.starts_with("http") || inner_lower.starts_with("//") {
                remote.store(true, Ordering::Relaxed);
                if allow_remote {
                    out.push_str(&css[i..end]);
                } else {
                    out.push_str("none");
                }
            } else {
                out.push_str("none");
            }
            i = end;
            continue;
        }
        if lower[i..].starts_with("@import") {
            let end = lower[i..].find(';').map(|e| i + e + 1).unwrap_or(css.len());
            remote.store(true, Ordering::Relaxed);
            i = end;
            continue;
        }
        if lower[i..].starts_with("expression(") || lower[i..].starts_with("behavior") || lower[i..].starts_with("-moz-binding") {
            out.push_str("x-removed");
            i += 1;
            while i < css.len() && !css.is_char_boundary(i) {
                i += 1;
            }
            continue;
        }
        let ch = css[i..].chars().next().unwrap_or(' ');
        // Keep position: fixed from covering the reader's interface.
        out.push(ch);
        i += ch.len_utf8();
    }
    out.replace("position:fixed", "position:static").replace("position: fixed", "position: static")
}

fn filter_style_blocks(html: &str, allow_remote: bool, remote: &AtomicBool) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(start) = lower.find("<style") else {
            out.push_str(rest);
            break;
        };
        let Some(open_end) = lower[start..].find('>').map(|e| start + e + 1) else {
            out.push_str(rest);
            break;
        };
        let close = lower[open_end..].find("</style>").map(|e| open_end + e).unwrap_or(rest.len());
        out.push_str(&rest[..open_end]);
        out.push_str(&filter_css(&rest[open_end..close], allow_remote, remote));
        rest = &rest[close..];
        if rest.len() >= 8 && rest[..8].eq_ignore_ascii_case("</style>") {
            out.push_str("</style>");
            rest = &rest[8..];
        }
    }
    out
}

/// Removes `<style>` blocks, for HTML that is quoted into the editor where
/// the original's global styles would restyle the reply.
pub fn strip_style_blocks(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(start) = lower.find("<style") else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..start]);
        match lower[start..].find("</style>") {
            Some(end) => rest = &rest[start + end + 8..],
            None => return out,
        }
    }
}

/// Plain text as HTML: escaped, links clickable, `>` quotes as blockquotes.
pub fn text_to_html(text: &str) -> String {
    let mut out = String::from("<div class=\"lm-plain\">");
    let mut depth = 0usize;
    for line in text.replace("\r\n", "\n").split('\n') {
        let mut level = 0;
        let mut rest = line;
        loop {
            let trimmed = rest.trim_start_matches(' ');
            if let Some(r) = trimmed.strip_prefix('>') {
                level += 1;
                rest = r.strip_prefix(' ').unwrap_or(r);
            } else {
                break;
            }
        }
        if level == 0 {
            rest = line;
        }
        while depth < level {
            out.push_str("<blockquote type=\"cite\">");
            depth += 1;
        }
        while depth > level {
            out.push_str("</blockquote>");
            depth -= 1;
        }
        out.push_str(&linkify_escaped(rest));
        out.push('\n');
    }
    while depth > 0 {
        out.push_str("</blockquote>");
        depth -= 1;
    }
    out.push_str("</div>");
    out
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn linkify_escaped(text: &str) -> String {
    let finder = linkify::LinkFinder::new();
    let mut out = String::new();
    for span in finder.spans(text) {
        match span.kind() {
            Some(linkify::LinkKind::Url) => {
                let url = span.as_str();
                out.push_str(&format!("<a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">{}</a>", escape(url), escape(url)));
            }
            Some(linkify::LinkKind::Email) => {
                let email = span.as_str();
                out.push_str(&format!("<a href=\"mailto:{}\">{}</a>", escape(email), escape(email)));
            }
            _ => out.push_str(&escape(span.as_str())),
        }
    }
    out
}

/// HTML to readable plain text (for the text part of sent messages and quotes).
pub fn html_to_text(html: &str) -> String {
    let text = html2text::config::plain().string_from_read(html.as_bytes(), 78).unwrap_or_else(|_| ammonia::clean_text(html));
    text.lines().map(|l| l.trim_end()).collect::<Vec<_>>().join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_scripts_and_remote_images() {
        let input = r#"<p onclick="x()">Hi<script>alert(1)</script><img src="https://t.example/p.gif"><img src="cid:logo@x"></p>
            <style>@import url(https://x/y.css); body { background: url('https://x/bg.png') } p { color: red }</style>
            <a href="javascript:alert(1)">bad</a><a href="https://ok.example">ok</a><form><input></form>"#;
        let mut cids = HashMap::new();
        cids.insert("logo@x".to_string(), "data:image/png;base64,AAAA".to_string());
        let s = sanitize(input, &cids, false);
        assert!(s.remote);
        assert!(!s.html.contains("script"));
        assert!(!s.html.contains("onclick"));
        assert!(!s.html.contains("https://t.example"));
        assert!(!s.html.contains("https://x/bg.png"));
        assert!(!s.html.contains("@import"));
        assert!(s.html.contains("color: red"));
        assert!(s.html.contains("data:image/png;base64,AAAA"));
        assert!(!s.html.contains("javascript"));
        assert!(s.html.contains("https://ok.example"));
        assert!(!s.html.contains("<input"));
        assert!(s.used_cids.contains("logo@x"));

        let allowed = sanitize(input, &cids, true);
        assert!(allowed.html.contains("https://t.example/p.gif"));
    }

    #[test]
    fn plain_text_quotes() {
        let html = text_to_html("Hello <you>\n> quoted https://example.com\n>> deeper\nback");
        assert!(html.contains("Hello &lt;you&gt;"));
        assert!(html.contains("<blockquote type=\"cite\">quoted <a href=\"https://example.com\""));
        assert!(html.contains("<blockquote type=\"cite\">deeper"));
        assert!(html.ends_with("back\n</div>"));
    }
}
