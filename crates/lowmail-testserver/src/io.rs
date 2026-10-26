//! Buffered, cancellation-safe line/literal reader shared by all protocols.

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt};

/// Reads CRLF (or LF) terminated lines and fixed-size blocks.
///
/// All state lives in the internal buffer, so the read futures can be
/// dropped (e.g. inside `tokio::select!`) without losing data.
pub(crate) struct LineReader<R> {
    inner: R,
    buf: Vec<u8>,
    scanned: usize,
}

impl<R: AsyncRead + Unpin> LineReader<R> {
    pub(crate) fn new(inner: R) -> Self {
        Self {
            inner,
            buf: Vec::with_capacity(8192),
            scanned: 0,
        }
    }

    async fn fill(&mut self, hint: usize) -> io::Result<bool> {
        self.buf.reserve(hint.clamp(4096, 1 << 20));
        let n = self.inner.read_buf(&mut self.buf).await?;
        Ok(n > 0)
    }

    /// Reads one line including its terminator; `Ok(None)` on a clean EOF.
    pub(crate) async fn read_line(&mut self, max: usize) -> io::Result<Option<Vec<u8>>> {
        loop {
            if let Some(i) = self.buf[self.scanned..].iter().position(|&b| b == b'\n') {
                let end = self.scanned + i + 1;
                let rest = self.buf.split_off(end);
                self.scanned = 0;
                return Ok(Some(std::mem::replace(&mut self.buf, rest)));
            }
            self.scanned = self.buf.len();
            if self.buf.len() > max {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
            }
            if !self.fill(0).await? {
                return if self.buf.is_empty() {
                    Ok(None)
                } else {
                    Err(io::ErrorKind::UnexpectedEof.into())
                };
            }
        }
    }

    /// Reads exactly `n` bytes.
    pub(crate) async fn read_exact(&mut self, n: usize) -> io::Result<Vec<u8>> {
        while self.buf.len() < n {
            let missing = n - self.buf.len();
            if !self.fill(missing).await? {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
        }
        let rest = self.buf.split_off(n);
        self.scanned = 0;
        Ok(std::mem::replace(&mut self.buf, rest))
    }
}

/// Strips a trailing CRLF or LF.
pub(crate) fn trim_eol(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// If `line` (without EOL) ends with an IMAP-style literal marker `{n}` or
/// `{n+}`, returns `(n, synchronizing)`.
pub(crate) fn literal_marker(line: &[u8]) -> Option<(usize, bool)> {
    let inner = line.strip_suffix(b"}")?;
    let open = inner.iter().rposition(|&b| b == b'{')?;
    let mut digits = &inner[open + 1..];
    let sync = match digits.strip_suffix(b"+") {
        Some(d) => {
            digits = d;
            false
        }
        None => true,
    };
    if digits.is_empty() || digits.len() > 12 || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let n = std::str::from_utf8(digits).ok()?.parse().ok()?;
    Some((n, sync))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_markers() {
        assert_eq!(literal_marker(b"a APPEND INBOX {12}"), Some((12, true)));
        assert_eq!(literal_marker(b"a APPEND INBOX {12+}"), Some((12, false)));
        assert_eq!(literal_marker(b"a LOGIN x y"), None);
        assert_eq!(literal_marker(b"a {}"), None);
    }

    #[tokio::test]
    async fn reads_lines_and_blocks() {
        let data: &[u8] = b"one\r\ntwo\nabcdethree\r\n";
        let mut r = LineReader::new(data);
        assert_eq!(r.read_line(100).await.unwrap().unwrap(), b"one\r\n");
        assert_eq!(r.read_line(100).await.unwrap().unwrap(), b"two\n");
        assert_eq!(r.read_exact(5).await.unwrap(), b"abcde");
        assert_eq!(r.read_line(100).await.unwrap().unwrap(), b"three\r\n");
        assert!(r.read_line(100).await.unwrap().is_none());
    }
}
