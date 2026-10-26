//! Server-side filters and vacation replies over ManageSieve (RFC 5804).
//! LowMail keeps its rules as JSON inside the script it writes, so the rules
//! can be edited again; scripts written by other tools are shown as text.

use base64::Engine;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::error::{Error, ErrorKind, Result};
use crate::model::{AccountConfig, Endpoint, Security};
use crate::net::{self, MailStream};
use crate::utf7;

pub const SCRIPT_NAME: &str = "lowmail";
const META_PREFIX: &str = "# lowmail:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Field {
    #[default]
    From,
    To,
    Cc,
    /// To or Cc.
    Recipient,
    Subject,
    Body,
    /// Any header, named in `header`.
    Header,
    Size,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Op {
    #[default]
    Contains,
    NotContains,
    Is,
    IsNot,
    StartsWith,
    EndsWith,
    /// Size over, in KB.
    Over,
    Under,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    pub field: Field,
    #[serde(default)]
    pub header: String,
    pub op: Op,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    /// Folder path as listed by IMAP (modified UTF-7).
    Move { folder: String },
    Copy { folder: String },
    MarkRead,
    Flag,
    Forward { to: String, #[serde(default)] keep: bool },
    Discard,
    Reject { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// All conditions must match (true) or any of them.
    #[serde(default = "yes")]
    pub match_all: bool,
    #[serde(default)]
    pub conditions: Vec<Condition>,
    #[serde(default)]
    pub actions: Vec<Action>,
    /// Do not evaluate the following rules.
    #[serde(default = "yes")]
    pub stop: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vacation {
    pub enabled: bool,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub body: String,
    #[serde(default = "one")]
    pub days: u32,
    /// `YYYY-MM-DD`, inclusive.
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub end: Option<String>,
    /// Addresses of the account that receive replies (besides the login).
    #[serde(default)]
    pub addresses: Vec<String>,
}

fn one() -> u32 {
    1
}

impl Default for Vacation {
    fn default() -> Self {
        Vacation { enabled: false, subject: String::new(), body: String::new(), days: 1, start: None, end: None, addresses: vec![] }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Filters {
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub vacation: Vacation,
}

/// What the settings screen shows.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterState {
    pub filters: Filters,
    /// Name of the active script when it was not written by LowMail.
    pub foreign_script: Option<String>,
    pub foreign_content: Option<String>,
    /// Sieve extensions the server supports.
    pub extensions: Vec<String>,
}

fn sieve_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn sieve_list(items: &[String]) -> String {
    format!("[{}]", items.iter().map(|s| sieve_string(s)).collect::<Vec<_>>().join(", "))
}

fn test_for(c: &Condition, needs: &mut Vec<&'static str>) -> Option<String> {
    let (negate, matcher, value) = match c.op {
        Op::Contains => (false, ":contains", c.value.clone()),
        Op::NotContains => (true, ":contains", c.value.clone()),
        Op::Is => (false, ":is", c.value.clone()),
        Op::IsNot => (true, ":is", c.value.clone()),
        Op::StartsWith => (false, ":matches", format!("{}*", c.value.replace('*', "\\\\*").replace('?', "\\\\?"))),
        Op::EndsWith => (false, ":matches", format!("*{}", c.value.replace('*', "\\\\*").replace('?', "\\\\?"))),
        Op::Over | Op::Under => {
            let kb: u64 = c.value.trim().parse().ok()?;
            let dir = if c.op == Op::Over { ":over" } else { ":under" };
            return Some(format!("size {dir} {kb}K"));
        }
    };
    let v = sieve_string(&value);
    let test = match c.field {
        Field::From => format!("address {matcher} \"from\" {v}"),
        Field::To => format!("address {matcher} \"to\" {v}"),
        Field::Cc => format!("address {matcher} \"cc\" {v}"),
        Field::Recipient => format!("address {matcher} [\"to\", \"cc\"] {v}"),
        Field::Subject => format!("header {matcher} \"subject\" {v}"),
        Field::Body => {
            needs.push("body");
            format!("body :text {matcher} {v}")
        }
        Field::Header => {
            if c.header.trim().is_empty() {
                return None;
            }
            format!("header {matcher} {} {v}", sieve_string(c.header.trim()))
        }
        Field::Size => return None,
    };
    // Plain "contains" on addresses should match display names too.
    let test = if matches!(c.field, Field::From | Field::To | Field::Cc | Field::Recipient) && matcher == ":contains" {
        let header = match c.field {
            Field::From => "\"from\"".to_string(),
            Field::To => "\"to\"".to_string(),
            Field::Cc => "\"cc\"".to_string(),
            _ => "[\"to\", \"cc\"]".to_string(),
        };
        format!("header :contains {header} {v}")
    } else {
        test
    };
    Some(if negate { format!("not {test}") } else { test })
}

/// The Sieve script for `filters`, with the rules embedded for editing.
pub fn generate(filters: &Filters) -> Result<String> {
    let mut needs: Vec<&'static str> = Vec::new();
    let mut body = String::new();

    let v = &filters.vacation;
    if v.enabled {
        needs.push("vacation");
        let mut dates = Vec::new();
        if let Some(start) = v.start.as_deref().filter(|s| !s.is_empty()) {
            needs.extend(["date", "relational"]);
            dates.push(format!("currentdate :value \"ge\" \"date\" {}", sieve_string(start)));
        }
        if let Some(end) = v.end.as_deref().filter(|s| !s.is_empty()) {
            needs.extend(["date", "relational"]);
            dates.push(format!("currentdate :value \"le\" \"date\" {}", sieve_string(end)));
        }
        let mut cmd = format!("vacation :days {}", v.days.max(1));
        if !v.subject.trim().is_empty() {
            cmd.push_str(&format!(" :subject {}", sieve_string(v.subject.trim())));
        }
        let addresses: Vec<String> = v.addresses.iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
        if !addresses.is_empty() {
            cmd.push_str(&format!(" :addresses {}", sieve_list(&addresses)));
        }
        cmd.push_str(&format!(" {};", sieve_string(&v.body.replace("\r\n", "\n"))));
        body.push_str("# Vacation reply\n");
        if dates.is_empty() {
            body.push_str(&cmd);
            body.push('\n');
        } else {
            body.push_str(&format!("if allof ({}) {{\n    {cmd}\n}}\n", dates.join(", ")));
        }
    }

    for rule in filters.rules.iter().filter(|r| r.enabled) {
        let tests: Vec<String> = rule.conditions.iter().filter_map(|c| test_for(c, &mut needs)).collect();
        let mut actions: Vec<String> = Vec::new();
        for a in &rule.actions {
            match a {
                Action::Move { folder } => {
                    needs.push("fileinto");
                    actions.push(format!("fileinto {};", sieve_string(&utf7::decode(folder))));
                }
                Action::Copy { folder } => {
                    needs.extend(["fileinto", "copy"]);
                    actions.push(format!("fileinto :copy {};", sieve_string(&utf7::decode(folder))));
                }
                Action::MarkRead => {
                    needs.push("imap4flags");
                    actions.insert(0, "addflag \"\\\\Seen\";".into());
                }
                Action::Flag => {
                    needs.push("imap4flags");
                    actions.insert(0, "addflag \"\\\\Flagged\";".into());
                }
                Action::Forward { to, keep } => {
                    if *keep {
                        needs.push("copy");
                        actions.push(format!("redirect :copy {};", sieve_string(to.trim())));
                    } else {
                        actions.push(format!("redirect {};", sieve_string(to.trim())));
                    }
                }
                Action::Discard => actions.push("discard;".into()),
                Action::Reject { message } => {
                    needs.push("reject");
                    actions.push(format!("reject {};", sieve_string(message)));
                }
            }
        }
        if actions.is_empty() {
            continue;
        }
        if rule.stop {
            actions.push("stop;".into());
        }
        body.push_str(&format!("# rule: {}\n", rule.name.replace('\n', " ")));
        let actions = actions.iter().map(|a| format!("    {a}")).collect::<Vec<_>>().join("\n");
        if tests.is_empty() {
            body.push_str(&format!("if true {{\n{actions}\n}}\n"));
        } else if tests.len() == 1 {
            body.push_str(&format!("if {} {{\n{actions}\n}}\n", tests[0]));
        } else {
            let join = if rule.match_all { "allof" } else { "anyof" };
            body.push_str(&format!("if {join} ({}) {{\n{actions}\n}}\n", tests.join(", ")));
        }
    }

    needs.sort_unstable();
    needs.dedup();
    let meta = base64::engine::general_purpose::STANDARD.encode(serde_json::to_vec(filters)?);
    let mut script = String::from("# Filters and vacation reply managed by LowMail.\n# Edits made here by hand are replaced the next time LowMail saves.\n");
    script.push_str(&format!("{META_PREFIX}{meta}\n"));
    if !needs.is_empty() {
        script.push_str(&format!("require {};\n", sieve_list(&needs.iter().map(|s| s.to_string()).collect::<Vec<_>>())));
    }
    script.push('\n');
    script.push_str(&body);
    Ok(script)
}

/// The rules embedded in a script written by LowMail.
pub fn parse_meta(script: &str) -> Option<Filters> {
    let line = script.lines().find(|l| l.starts_with(META_PREFIX))?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(line[META_PREFIX.len()..].trim()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// A signed-in ManageSieve connection.
pub struct Client {
    stream: BufReader<MailStream>,
    pub extensions: Vec<String>,
}

#[derive(Debug)]
enum Line {
    Ok,
    No(String),
    Bye(String),
    Data(String),
}

impl Client {
    pub async fn connect(endpoint: &Endpoint, username: &str, password: &str) -> Result<Client> {
        let tcp = net::connect_tcp(&endpoint.host, endpoint.port).await?;
        let stream = match endpoint.security {
            Security::Tls => net::upgrade(tcp, &endpoint.host, endpoint.accept_invalid_certs).await?,
            _ => MailStream::Plain(tcp),
        };
        let mut client = Client { stream: BufReader::new(stream), extensions: Vec::new() };
        let mut caps = client.read_response().await?;
        if endpoint.security == Security::StartTls {
            client.send("STARTTLS").await?;
            client.read_response().await?;
            let MailStream::Plain(tcp) = client.stream.into_inner() else {
                return Err(Error::internal("Unexpected stream state."));
            };
            let tls = net::upgrade(tcp, &endpoint.host, endpoint.accept_invalid_certs).await?;
            client = Client { stream: BufReader::new(tls), extensions: Vec::new() };
            caps = client.read_response().await?;
        }
        for line in &caps {
            let parts = parse_strings(line);
            if parts.first().is_some_and(|k| k.eq_ignore_ascii_case("SIEVE")) {
                client.extensions = parts.get(1).map(|v| v.split_whitespace().map(str::to_string).collect()).unwrap_or_default();
            }
        }
        let auth = base64::engine::general_purpose::STANDARD.encode(format!("\0{username}\0{password}"));
        client.send(&format!("AUTHENTICATE \"PLAIN\" \"{auth}\"")).await?;
        client.read_response().await.map_err(|e| {
            if e.kind == ErrorKind::Server {
                Error::new(ErrorKind::Auth, "The filter server rejected the sign-in.").with_detail(e.detail.unwrap_or_default())
            } else {
                e
            }
        })?;
        Ok(client)
    }

    async fn send(&mut self, line: &str) -> Result<()> {
        let stream = self.stream.get_mut();
        stream.write_all(line.as_bytes()).await?;
        stream.write_all(b"\r\n").await?;
        stream.flush().await?;
        Ok(())
    }

    async fn read_line(&mut self) -> Result<Line> {
        let mut buf = Vec::new();
        let n = self.stream.read_until(b'\n', &mut buf).await?;
        if n == 0 {
            return Err(Error::new(ErrorKind::Network, "The filter server closed the connection."));
        }
        let mut line = String::from_utf8_lossy(&buf).trim_end_matches(['\r', '\n']).to_string();
        // A literal at the end of the line: read it and append its content.
        while let Some(size) = literal_size(&line) {
            let mut data = vec![0u8; size];
            self.stream.read_exact(&mut data).await?;
            line.truncate(line.rfind('{').unwrap_or(line.len()));
            line.push_str(&format!("\"{}\"", String::from_utf8_lossy(&data).replace('\\', "\\\\").replace('"', "\\\"")));
            let mut rest = Vec::new();
            self.stream.read_until(b'\n', &mut rest).await?;
            line.push_str(String::from_utf8_lossy(&rest).trim_end_matches(['\r', '\n']));
        }
        let upper = line.to_ascii_uppercase();
        Ok(if upper.starts_with("OK") {
            Line::Ok
        } else if upper.starts_with("NO") {
            Line::No(line[2..].trim().to_string())
        } else if upper.starts_with("BYE") {
            Line::Bye(line[3..].trim().to_string())
        } else {
            Line::Data(line)
        })
    }

    async fn read_response(&mut self) -> Result<Vec<String>> {
        let mut data = Vec::new();
        loop {
            match self.read_line().await? {
                Line::Data(d) => data.push(d),
                Line::Ok => return Ok(data),
                Line::No(reason) => {
                    let reason = parse_strings(&reason).into_iter().last().unwrap_or(reason);
                    return Err(Error::server("The filter server refused the operation.").with_detail(reason));
                }
                Line::Bye(reason) => {
                    return Err(Error::new(ErrorKind::Network, "The filter server closed the connection.").with_detail(reason));
                }
            }
        }
    }

    /// Script names and which one is active.
    pub async fn list(&mut self) -> Result<Vec<(String, bool)>> {
        self.send("LISTSCRIPTS").await?;
        let lines = self.read_response().await?;
        Ok(lines
            .iter()
            .filter_map(|l| {
                let name = parse_strings(l).into_iter().next()?;
                Some((name, l.to_ascii_uppercase().trim_end().ends_with("ACTIVE")))
            })
            .collect())
    }

    pub async fn get(&mut self, name: &str) -> Result<String> {
        self.send(&format!("GETSCRIPT {}", sieve_string(name))).await?;
        let lines = self.read_response().await?;
        Ok(lines.iter().flat_map(|l| parse_strings(l)).collect::<Vec<_>>().join("\n"))
    }

    pub async fn put(&mut self, name: &str, script: &str) -> Result<()> {
        let script = script.replace("\r\n", "\n").replace('\n', "\r\n");
        self.send(&format!("PUTSCRIPT {} {{{}+}}\r\n{}", sieve_string(name), script.len(), script)).await?;
        self.read_response().await.map(|_| ())
    }

    pub async fn activate(&mut self, name: &str) -> Result<()> {
        self.send(&format!("SETACTIVE {}", sieve_string(name))).await?;
        self.read_response().await.map(|_| ())
    }

    pub async fn logout(mut self) {
        let _ = self.send("LOGOUT").await;
    }
}

fn literal_size(line: &str) -> Option<usize> {
    let open = line.rfind('{')?;
    let inner = line[open + 1..].strip_suffix('}')?;
    inner.trim_end_matches('+').parse().ok()
}

/// The quoted strings of a response line, unescaped; bare atoms are kept too.
fn parse_strings(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(n) = chars.next() {
                                s.push(n);
                            }
                        }
                        '"' => break,
                        _ => s.push(c),
                    }
                }
                out.push(s);
            }
            c if c.is_whitespace() => {}
            c => {
                let mut s = String::from(c);
                while let Some(&n) = chars.peek() {
                    if n.is_whitespace() || n == '"' {
                        break;
                    }
                    s.push(n);
                    chars.next();
                }
                out.push(s);
            }
        }
    }
    out
}

fn endpoint(account: &AccountConfig) -> Result<&Endpoint> {
    account.sieve.as_ref().ok_or_else(|| Error::unsupported("Filters are not available for this account."))
}

pub async fn load(account: &AccountConfig, password: &str) -> Result<FilterState> {
    let mut client = Client::connect(endpoint(account)?, &account.username, password).await?;
    let scripts = client.list().await?;
    let active = scripts.iter().find(|(_, a)| *a).map(|(n, _)| n.clone());
    let mut state = FilterState { filters: Filters::default(), foreign_script: None, foreign_content: None, extensions: client.extensions.clone() };
    if scripts.iter().any(|(n, _)| n == SCRIPT_NAME) {
        let text = client.get(SCRIPT_NAME).await?;
        if let Some(f) = parse_meta(&text) {
            state.filters = f;
        }
    }
    if let Some(name) = active.filter(|n| n != SCRIPT_NAME) {
        state.foreign_content = client.get(&name).await.ok();
        state.foreign_script = Some(name);
    }
    client.logout().await;
    Ok(state)
}

/// Writes and activates the LowMail script.
pub async fn save(account: &AccountConfig, password: &str, filters: &Filters) -> Result<()> {
    let script = generate(filters)?;
    let mut client = Client::connect(endpoint(account)?, &account.username, password).await?;
    client.put(SCRIPT_NAME, &script).await?;
    client.activate(SCRIPT_NAME).await?;
    client.logout().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_and_reads_back() {
        let filters = Filters {
            rules: vec![Rule {
                id: "r1".into(),
                name: "Newsletters".into(),
                enabled: true,
                match_all: false,
                conditions: vec![
                    Condition { field: Field::From, header: String::new(), op: Op::Contains, value: "news@".into() },
                    Condition { field: Field::Subject, header: String::new(), op: Op::StartsWith, value: "[Promo]".into() },
                ],
                actions: vec![Action::Move { folder: "Projetos/Relat&APM-rios".into() }, Action::MarkRead],
                stop: true,
            }],
            vacation: Vacation { enabled: true, subject: "Férias".into(), body: "Volto dia 10.\n\"Até\"".into(), days: 3, start: Some("2026-01-02".into()), end: None, addresses: vec![] },
        };
        let script = generate(&filters).unwrap();
        assert!(script.contains("require [\"date\", \"fileinto\", \"imap4flags\", \"relational\", \"vacation\"];"));
        assert!(script.contains("fileinto \"Projetos/Relatórios\";"));
        assert!(script.contains("addflag \"\\\\Seen\";"));
        assert!(script.contains("if anyof (header :contains \"from\" \"news@\", header :matches \"subject\" \"[Promo]*\")"));
        assert!(script.contains("vacation :days 3 :subject \"Férias\" \"Volto dia 10.\n\\\"Até\\\"\";"));
        let back = parse_meta(&script).unwrap();
        assert_eq!(back.rules[0].name, "Newsletters");
        assert!(back.vacation.enabled);
    }

    #[test]
    fn response_strings() {
        assert_eq!(parse_strings("\"SIEVE\" \"fileinto vacation\""), vec!["SIEVE", "fileinto vacation"]);
        assert_eq!(parse_strings("\"my \\\"script\\\"\" ACTIVE"), vec!["my \"script\"", "ACTIVE"]);
        assert_eq!(literal_size("{12}"), Some(12));
        assert_eq!(literal_size("OK"), None);
    }
}
