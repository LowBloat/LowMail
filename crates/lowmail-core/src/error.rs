//! One error type for the whole engine. It crosses the RPC boundary as
//! `{ kind, message, detail? }`; the UI translates `kind` and shows `detail`
//! to people who want to know more.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    /// Wrong user name or password.
    Auth,
    /// The server cannot be reached or dropped the connection.
    Network,
    /// TLS handshake or certificate problem.
    Tls,
    NotFound,
    /// Invalid input from the caller.
    Invalid,
    /// The server answered NO or BAD.
    Server,
    /// The server lacks a capability the operation needs.
    Unsupported,
    TooLarge,
    /// The web session expired; the user must sign in again.
    SessionExpired,
    Internal,
}

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), detail: None }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Invalid, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, message)
    }

    pub fn server(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Server, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unsupported, message)
    }

    /// Errors after which the connection that produced them must be dropped.
    pub fn is_connection_error(&self) -> bool {
        matches!(self.kind, ErrorKind::Network | ErrorKind::Tls)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        use std::io::ErrorKind as Io;
        let message = match e.kind() {
            Io::ConnectionRefused => "The server refused the connection.",
            Io::TimedOut => "The server did not answer in time.",
            Io::NotFound => "Not found.",
            _ => "Connection to the server failed.",
        };
        let kind = if e.kind() == Io::NotFound { ErrorKind::NotFound } else { ErrorKind::Network };
        Error::new(kind, message).with_detail(e.to_string())
    }
}

impl From<async_imap::error::Error> for Error {
    fn from(e: async_imap::error::Error) -> Self {
        use async_imap::error::Error as E;
        match e {
            E::Io(io) => io.into(),
            E::ConnectionLost => Error::new(ErrorKind::Network, "The server closed the connection."),
            E::No(info) => Error::server("The server refused the operation.").with_detail(clean_imap_info(&info)),
            E::Bad(info) => Error::server("The server rejected the command.").with_detail(clean_imap_info(&info)),
            E::Parse(p) => Error::new(ErrorKind::Network, "Unexpected answer from the server.").with_detail(p.to_string()),
            E::Validate(v) => Error::invalid("Invalid characters in the request.").with_detail(v.to_string()),
            E::Append => Error::server("The server did not accept the message."),
            other => Error::server("IMAP error.").with_detail(other.to_string()),
        }
    }
}

/// async-imap formats NO/BAD as `code: .., info: Some("text")`; keep the text.
fn clean_imap_info(info: &str) -> String {
    if let Some(start) = info.find("info: Some(\"") {
        let rest = &info[start + 12..];
        if let Some(end) = rest.rfind("\")") {
            return rest[..end].replace("\\\"", "\"");
        }
    }
    info.to_string()
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        match e {
            rusqlite::Error::QueryReturnedNoRows => Error::not_found("Not found."),
            other => Error::internal("Local database error.").with_detail(other.to_string()),
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::invalid("Invalid data.").with_detail(e.to_string())
    }
}

impl From<lettre::transport::smtp::Error> for Error {
    fn from(e: lettre::transport::smtp::Error) -> Self {
        let detail = e.to_string();
        if e.is_permanent() || e.is_transient() {
            let lower = detail.to_ascii_lowercase();
            if lower.contains("auth") || lower.contains("535") {
                return Error::new(ErrorKind::Auth, "The outgoing server rejected the sign-in.").with_detail(detail);
            }
            return Error::server("The outgoing server refused the message.").with_detail(detail);
        }
        if detail.to_ascii_lowercase().contains("tls") || detail.contains("certificate") {
            return Error::new(ErrorKind::Tls, "Secure connection to the outgoing server failed.").with_detail(detail);
        }
        Error::new(ErrorKind::Network, "Could not reach the outgoing server.").with_detail(detail)
    }
}

impl From<lettre::address::AddressError> for Error {
    fn from(e: lettre::address::AddressError) -> Self {
        Error::invalid("Invalid e-mail address.").with_detail(e.to_string())
    }
}
