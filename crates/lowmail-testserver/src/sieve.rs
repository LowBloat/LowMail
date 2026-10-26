//! ManageSieve server (RFC 5804) storing per-user scripts in memory.
//! Scripts are stored only; they are never executed.

use std::io;
use std::sync::Arc;

use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

use crate::imap::parser::{PResult, Parser};
use crate::io::{LineReader, literal_marker, trim_eol};
use crate::sasl;
use crate::store::{SieveScript, Store};

const MAX_LINE: usize = 64 * 1024;
const MAX_SCRIPT: usize = 1 << 20;

const CAPABILITIES: &str = "\"IMPLEMENTATION\" \"LowMail Test\"\r\n\
\"SASL\" \"PLAIN\"\r\n\
\"SIEVE\" \"fileinto reject envelope vacation imap4flags copy body variables date relational regex\"\r\n\
\"VERSION\" \"1.0\"\r\n\
\"MAXREDIRECTS\" \"5\"\r\n";

/// Quotes a ManageSieve string.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        if c != '\r' && c != '\n' {
            out.push(c);
        }
    }
    out.push('"');
    out
}

fn ok(text: &str) -> String {
    format!("OK {}\r\n", quote(text))
}

fn no(code: Option<&str>, text: &str) -> String {
    match code {
        Some(code) => format!("NO ({code}) {}\r\n", quote(text)),
        None => format!("NO {}\r\n", quote(text)),
    }
}

struct Session {
    reader: LineReader<OwnedReadHalf>,
    writer: BufWriter<OwnedWriteHalf>,
    store: Arc<Store>,
    user: Option<String>,
}

/// Serves one ManageSieve connection until LOGOUT or disconnect.
pub(crate) async fn handle(stream: TcpStream, store: Arc<Store>) {
    let (read, write) = stream.into_split();
    let mut session = Session { reader: LineReader::new(read), writer: BufWriter::new(write), store, user: None };
    if let Err(err) = session.run().await {
        tracing::debug!(%err, "sieve connection ended");
    }
}

/// Reads a string argument (quoted or literal).
fn string_arg(p: &mut Parser<'_>) -> PResult<String> {
    p.sp()?;
    Ok(String::from_utf8_lossy(&p.string()?).into_owned())
}

impl Session {
    async fn send(&mut self, text: &str) -> io::Result<()> {
        self.writer.write_all(text.as_bytes()).await?;
        self.writer.flush().await
    }

    /// Reads a line plus any literals it announces (`{n+}` or `{n}`).
    async fn read_command(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut cmd = Vec::new();
        loop {
            let Some(line) = self.reader.read_line(MAX_LINE).await? else {
                return if cmd.is_empty() { Ok(None) } else { Err(io::ErrorKind::UnexpectedEof.into()) };
            };
            let marker = literal_marker(trim_eol(&line));
            cmd.extend_from_slice(&line);
            match marker {
                Some((n, _)) if n <= MAX_SCRIPT => {
                    let data = self.reader.read_exact(n).await?;
                    cmd.extend_from_slice(&data);
                }
                Some(_) => {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "literal too large"));
                }
                None => return Ok(Some(cmd)),
            }
        }
    }

    async fn run(&mut self) -> io::Result<()> {
        self.send(&format!("{CAPABILITIES}{}", ok("LowMail ManageSieve ready"))).await?;
        while let Some(cmd) = self.read_command().await? {
            let line = trim_eol(&cmd);
            if line.is_empty() {
                continue;
            }
            let mut p = Parser::new(line);
            let command = p.atom().unwrap_or_default().to_ascii_uppercase();
            tracing::debug!(%command, "sieve command");
            let reply = match command.as_str() {
                "LOGOUT" => {
                    self.send(&ok("Logout completed")).await?;
                    return Ok(());
                }
                "CAPABILITY" => format!("{CAPABILITIES}{}", ok("Capability completed")),
                "NOOP" => match string_arg(&mut p) {
                    Ok(tag) => format!("OK (TAG {}) \"Done\"\r\n", quote(&tag)),
                    Err(_) => ok("Done"),
                },
                "AUTHENTICATE" => self.authenticate(&mut p).await?,
                "UNAUTHENTICATE" => {
                    self.user = None;
                    ok("Unauthenticate completed")
                }
                "STARTTLS" => no(None, "TLS is not supported by this test server"),
                _ => match self.user.clone() {
                    None => no(None, "Authenticate first"),
                    Some(user) => self.authenticated(&command, &mut p, &user).unwrap_or_else(|e| no(None, &format!("Syntax error: {e}"))),
                },
            };
            self.send(&reply).await?;
        }
        Ok(())
    }

    async fn authenticate(&mut self, p: &mut Parser<'_>) -> io::Result<String> {
        if self.user.is_some() {
            return Ok(no(None, "Already authenticated"));
        }
        let Ok(mechanism) = string_arg(p) else {
            return Ok(no(None, "Syntax: AUTHENTICATE \"PLAIN\" [initial-response]"));
        };
        if !mechanism.eq_ignore_ascii_case("PLAIN") {
            return Ok(no(None, "Unsupported SASL mechanism"));
        }
        let response = if p.opt_sp() && !p.is_end() {
            p.string().ok()
        } else {
            self.send("\"\"\r\n").await?;
            let Some(line) = self.read_command().await? else {
                return Err(io::ErrorKind::UnexpectedEof.into());
            };
            Parser::new(trim_eol(&line)).string().ok()
        };
        let Some(response) = response else {
            return Ok(no(None, "Invalid SASL response"));
        };
        if response == b"*" {
            return Ok(no(None, "Authentication aborted"));
        }
        match sasl::decode_plain(&response).and_then(|(u, pw)| self.store.authenticate(&u, &pw)) {
            Some(login) => {
                self.user = Some(login);
                Ok(ok("Authenticated"))
            }
            None => Ok(no(None, "Authentication failed")),
        }
    }

    fn authenticated(&self, command: &str, p: &mut Parser<'_>, user: &str) -> PResult<String> {
        let mut inner = self.store.lock();
        let Some(account) = inner.account_mut(user) else {
            return Ok(no(None, "Unknown user"));
        };
        let scripts = &mut account.scripts;
        let find = |scripts: &[SieveScript], name: &str| scripts.iter().position(|s| s.name == name);
        Ok(match command {
            "LISTSCRIPTS" => {
                let mut out = String::new();
                for s in scripts.iter() {
                    out.push_str(&quote(&s.name));
                    if s.active {
                        out.push_str(" ACTIVE");
                    }
                    out.push_str("\r\n");
                }
                out.push_str(&ok("Listscripts completed"));
                out
            }
            "GETSCRIPT" => {
                let name = string_arg(p)?;
                match find(scripts, &name) {
                    Some(i) => {
                        let content = &scripts[i].content;
                        format!("{{{}}}\r\n{content}\r\n{}", content.len(), ok("Getscript completed"))
                    }
                    None => no(Some("NONEXISTENT"), "There is no script by that name"),
                }
            }
            "PUTSCRIPT" => {
                let name = string_arg(p)?;
                let content = string_arg(p)?;
                p.end()?;
                if name.is_empty() {
                    no(None, "Script name must not be empty")
                } else if content.trim().is_empty() {
                    no(None, "Script is empty")
                } else {
                    match find(scripts, &name) {
                        Some(i) => scripts[i].content = content,
                        None => scripts.push(SieveScript { name, content, active: false }),
                    }
                    ok("Putscript completed")
                }
            }
            "CHECKSCRIPT" => {
                let content = string_arg(p)?;
                if content.trim().is_empty() { no(None, "Script is empty") } else { ok("Script is valid") }
            }
            "SETACTIVE" => {
                let name = string_arg(p)?;
                if name.is_empty() {
                    scripts.iter_mut().for_each(|s| s.active = false);
                    ok("No script is active now")
                } else if let Some(i) = find(scripts, &name) {
                    for (j, s) in scripts.iter_mut().enumerate() {
                        s.active = i == j;
                    }
                    ok("Setactive completed")
                } else {
                    no(Some("NONEXISTENT"), "There is no script by that name")
                }
            }
            "DELETESCRIPT" => {
                let name = string_arg(p)?;
                match find(scripts, &name) {
                    Some(i) if scripts[i].active => no(Some("ACTIVE"), "You may not delete an active script"),
                    Some(i) => {
                        scripts.remove(i);
                        ok("Deletescript completed")
                    }
                    None => no(Some("NONEXISTENT"), "There is no script by that name"),
                }
            }
            "RENAMESCRIPT" => {
                let old = string_arg(p)?;
                let new = string_arg(p)?;
                if find(scripts, &new).is_some() {
                    no(Some("ALREADYEXISTS"), "A script with that name already exists")
                } else if let Some(i) = find(scripts, &old) {
                    scripts[i].name = new;
                    ok("Renamescript completed")
                } else {
                    no(Some("NONEXISTENT"), "There is no script by that name")
                }
            }
            "HAVESPACE" => {
                let _name = string_arg(p)?;
                p.sp()?;
                let size = p.number64()?;
                if size as usize > MAX_SCRIPT { no(Some("QUOTA/MAXSIZE"), "Script too large") } else { ok("Putscript would succeed") }
            }
            _ => no(None, "Unknown command"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(quote("a \"b\" \\"), "\"a \\\"b\\\" \\\\\"");
        assert_eq!(no(Some("ACTIVE"), "x"), "NO (ACTIVE) \"x\"\r\n");
    }
}
