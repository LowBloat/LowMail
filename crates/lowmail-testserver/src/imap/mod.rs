//! IMAP4rev1 server (RFC 3501) with the extensions LowMail relies on.

pub(crate) mod fetch;
pub(crate) mod parser;
mod search;
mod session;
mod sort;
mod wire;

use std::sync::Arc;

use tokio::net::TcpStream;

use crate::store::Store;

/// Advertised in the greeting and in CAPABILITY responses.
pub(crate) const CAPABILITIES: &str = "IMAP4rev1 LITERAL+ SASL-IR AUTH=PLAIN IDLE UIDPLUS MOVE \
     SPECIAL-USE SORT QUOTA ID NAMESPACE UNSELECT CHILDREN";

/// Serves one IMAP connection until logout or disconnect.
pub(crate) async fn handle(stream: TcpStream, store: Arc<Store>) {
    session::Connection::new(stream, store).run().await;
}
