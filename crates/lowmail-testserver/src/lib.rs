//! In-memory IMAP4rev1 / SMTP / ManageSieve server for LowMail development and
//! end-to-end tests.
//!
//! The server speaks the real protocols over plain TCP (no TLS), keeps all
//! state in memory and can seed a realistic demo mailbox. It is meant for
//! tests and UI demos only: there is no persistence and no security.
//!
//! ```no_run
//! # async fn demo() -> anyhow::Result<()> {
//! let server = lowmail_testserver::start(lowmail_testserver::Config {
//!     imap_port: 0,
//!     smtp_port: 0,
//!     sieve_port: 0,
//!     ..Default::default()
//! })
//! .await?;
//! println!("IMAP on {}", server.imap_addr());
//! server.shutdown().await;
//! # Ok(())
//! # }
//! ```

mod imap;
mod io;
mod message;
mod sasl;
mod seed;
mod sieve;
mod smtp;
mod store;

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use anyhow::Context as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

use crate::message::MessageData;
use crate::store::Store;

/// Server configuration. Ports set to `0` select an ephemeral port.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address all listeners bind to.
    pub bind: IpAddr,
    /// IMAP4rev1 port.
    pub imap_port: u16,
    /// SMTP submission port.
    pub smtp_port: u16,
    /// ManageSieve port.
    pub sieve_port: u16,
    /// Populate the first user's mailbox with demo data.
    pub seed_demo_data: bool,
    /// Accounts as `(email, password)`; the email is also the login name.
    pub users: Vec<(String, String)>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            imap_port: 1143,
            smtp_port: 1025,
            sieve_port: 4190,
            seed_demo_data: true,
            users: vec![
                ("alice@lowmail.test".to_owned(), "alice".to_owned()),
                ("bob@lowmail.test".to_owned(), "bob".to_owned()),
            ],
        }
    }
}

/// A message accepted over SMTP for a recipient that is not a local user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundMail {
    /// Envelope sender (`MAIL FROM`), possibly empty.
    pub from: String,
    /// Non-local envelope recipients (`RCPT TO`).
    pub rcpt: Vec<String>,
    /// The message as received in `DATA`, dot-unstuffed.
    pub raw: Vec<u8>,
}

/// A running server; dropping it (or calling [`Server::shutdown`]) stops all
/// listeners and connections.
pub struct Server {
    store: Arc<Store>,
    imap_addr: SocketAddr,
    smtp_addr: SocketAddr,
    sieve_addr: SocketAddr,
    tasks: Vec<JoinHandle<()>>,
}

/// Starts the IMAP, SMTP and ManageSieve listeners.
pub async fn start(config: Config) -> anyhow::Result<Server> {
    let store = Arc::new(Store::new(&config.users));
    if config.seed_demo_data {
        seed::seed(&store, &config.users, chrono::Utc::now());
    }

    let imap = bind(config.bind, config.imap_port, "IMAP").await?;
    let smtp = bind(config.bind, config.smtp_port, "SMTP").await?;
    let sieve = bind(config.bind, config.sieve_port, "ManageSieve").await?;
    let imap_addr = imap.local_addr()?;
    let smtp_addr = smtp.local_addr()?;
    let sieve_addr = sieve.local_addr()?;

    let tasks = vec![
        tokio::spawn(accept_loop(imap, store.clone(), imap::handle)),
        tokio::spawn(accept_loop(smtp, store.clone(), smtp::handle)),
        tokio::spawn(accept_loop(sieve, store.clone(), sieve::handle)),
    ];
    tracing::info!(%imap_addr, %smtp_addr, %sieve_addr, "lowmail test server listening");

    Ok(Server {
        store,
        imap_addr,
        smtp_addr,
        sieve_addr,
        tasks,
    })
}

async fn bind(ip: IpAddr, port: u16, what: &str) -> anyhow::Result<TcpListener> {
    TcpListener::bind((ip, port))
        .await
        .with_context(|| format!("cannot bind {what} listener on {ip}:{port}"))
}

/// Accepts connections until the task is aborted; aborting drops the
/// [`JoinSet`], which aborts every connection task as well.
async fn accept_loop<F, Fut>(listener: TcpListener, store: Arc<Store>, handler: F)
where
    F: Fn(TcpStream, Arc<Store>) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    tracing::debug!(%peer, "connection accepted");
                    let _ = stream.set_nodelay(true);
                    connections.spawn(handler(stream, store.clone()));
                }
                Err(err) => {
                    tracing::warn!(%err, "accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
}

impl Server {
    /// Address of the IMAP listener.
    pub fn imap_addr(&self) -> SocketAddr {
        self.imap_addr
    }

    /// Address of the SMTP listener.
    pub fn smtp_addr(&self) -> SocketAddr {
        self.smtp_addr
    }

    /// Address of the ManageSieve listener.
    pub fn sieve_addr(&self) -> SocketAddr {
        self.sieve_addr
    }

    /// Appends `raw` to `mailbox` of `user` (creating the mailbox if needed)
    /// and notifies connected IMAP sessions. Returns the new UID.
    ///
    /// # Panics
    ///
    /// Panics if `user` is not a configured account.
    pub fn deliver(&self, user: &str, mailbox: &str, raw: &[u8], flags: &[&str]) -> u32 {
        let flags: Vec<String> = flags.iter().map(|f| store::canonical_flag(f)).collect();
        let uid = {
            let mut inner = self.store.lock();
            assert!(inner.account(user).is_some(), "unknown user {user}");
            let name = store::normalize_mailbox(mailbox);
            if inner.mailbox(user, &name).is_none() {
                inner
                    .create_mailbox(user, &name, None)
                    .expect("mailbox can be created");
            }
            let mbox = inner.mailbox_mut(user, &name).expect("mailbox exists");
            mbox.append(
                Arc::new(MessageData::new(raw.to_vec())),
                flags,
                chrono::Local::now().fixed_offset(),
            )
        };
        self.store.notify();
        uid
    }

    /// Messages accepted over SMTP for non-local recipients, oldest first.
    pub fn outbound(&self) -> Vec<OutboundMail> {
        self.store.lock().outbound.clone()
    }

    /// Sieve scripts of `user` as `(name, content, active)`.
    pub fn sieve_scripts(&self, user: &str) -> Vec<(String, String, bool)> {
        let inner = self.store.lock();
        inner
            .account(user)
            .map(|account| {
                account
                    .scripts
                    .iter()
                    .map(|s| (s.name.clone(), s.content.clone(), s.active))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Stops all listeners and open connections.
    pub async fn shutdown(mut self) {
        for task in std::mem::take(&mut self.tasks) {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
