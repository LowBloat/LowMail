//! LowMail engine: everything that talks to mail servers or keeps local
//! data, shared by the desktop app and the web server.

pub mod compose;
pub mod discover;
pub mod error;
pub mod imap;
pub mod mime;
pub mod model;
pub mod net;
pub mod rpc;
pub mod sanitize;
pub mod service;
pub mod sieve;
pub mod smtp;
pub mod store;
pub mod utf7;
pub mod vcard;

pub use error::{Error, ErrorKind, Result};
