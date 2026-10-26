//! SASL PLAIN helpers shared by IMAP, SMTP and ManageSieve.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// Decodes base64 text, ignoring surrounding whitespace.
pub(crate) fn decode_base64(text: &[u8]) -> Option<Vec<u8>> {
    let text: Vec<u8> = text.iter().copied().filter(|b| !b.is_ascii_whitespace()).collect();
    STANDARD.decode(text).ok()
}

pub(crate) fn encode_base64(data: &[u8]) -> String {
    STANDARD.encode(data)
}

/// Decodes a base64 SASL PLAIN response into `(authcid, password)`.
pub(crate) fn decode_plain(text: &[u8]) -> Option<(String, String)> {
    let raw = decode_base64(text)?;
    let mut parts = raw.split(|&b| b == 0);
    let _authzid = parts.next()?;
    let user = String::from_utf8(parts.next()?.to_vec()).ok()?;
    let pass = String::from_utf8(parts.next()?.to_vec()).ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((user, pass))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain() {
        let b64 = encode_base64(b"\0alice@lowmail.test\0alice");
        assert_eq!(decode_plain(b64.as_bytes()), Some(("alice@lowmail.test".into(), "alice".into())));
        assert_eq!(decode_plain(b"!!"), None);
    }
}
