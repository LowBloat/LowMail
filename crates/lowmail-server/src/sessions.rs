//! Signed-in browser sessions. The IMAP password lives only in memory, inside
//! the session's mail client; a restart signs everybody out.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use lowmail_core::service::MailClient;
use parking_lot::RwLock;
use rand::RngCore;

pub struct Session {
    pub client: Arc<MailClient>,
    pub email: String,
    last_seen: parking_lot::Mutex<Instant>,
}

#[derive(Default)]
pub struct Sessions {
    map: RwLock<HashMap<String, Arc<Session>>>,
}

impl Sessions {
    pub fn create(&self, client: Arc<MailClient>, email: String) -> String {
        let mut bytes = [0u8; 32];
        rand::rng().fill_bytes(&mut bytes);
        let id = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let session = Arc::new(Session { client, email, last_seen: parking_lot::Mutex::new(Instant::now()) });
        self.map.write().insert(id.clone(), session);
        id
    }

    pub fn get(&self, id: &str, idle: Duration) -> Option<Arc<Session>> {
        let session = self.map.read().get(id).cloned()?;
        let mut last = session.last_seen.lock();
        if last.elapsed() > idle {
            return None;
        }
        *last = Instant::now();
        drop(last);
        Some(session)
    }

    /// Keeps a session alive without a request (an open event stream).
    pub fn touch(&self, id: &str) {
        if let Some(s) = self.map.read().get(id) {
            *s.last_seen.lock() = Instant::now();
        }
    }

    pub async fn remove(&self, id: &str) {
        let removed = self.map.write().remove(id);
        if let Some(s) = removed {
            s.client.close().await;
        }
    }

    /// Ends idle sessions; runs every minute.
    pub async fn reap(&self, idle: Duration) {
        let expired: Vec<Arc<Session>> = {
            let mut map = self.map.write();
            let ids: Vec<String> = map.iter().filter(|(_, s)| s.last_seen.lock().elapsed() > idle).map(|(k, _)| k.clone()).collect();
            ids.iter().filter_map(|id| map.remove(id)).collect()
        };
        for s in expired {
            tracing::info!(user = %s.email, "session expired");
            s.client.close().await;
        }
    }

    pub fn count(&self) -> usize {
        self.map.read().len()
    }
}

/// Failed sign-ins per address and per user, to slow down password guessing.
#[derive(Default)]
pub struct LoginLimiter {
    failures: parking_lot::Mutex<HashMap<String, Vec<Instant>>>,
}

const WINDOW: Duration = Duration::from_secs(15 * 60);
const MAX_PER_IP: usize = 20;
const MAX_PER_USER: usize = 8;

impl LoginLimiter {
    fn recent(list: &mut Vec<Instant>) -> usize {
        list.retain(|t| t.elapsed() < WINDOW);
        list.len()
    }

    pub fn blocked(&self, ip: IpAddr, user: &str) -> bool {
        let mut map = self.failures.lock();
        let ip_count = map.get_mut(&format!("ip:{ip}")).map(Self::recent).unwrap_or(0);
        let user_count = map.get_mut(&format!("user:{user}")).map(Self::recent).unwrap_or(0);
        ip_count >= MAX_PER_IP || user_count >= MAX_PER_USER
    }

    pub fn failed(&self, ip: IpAddr, user: &str) {
        let mut map = self.failures.lock();
        map.entry(format!("ip:{ip}")).or_default().push(Instant::now());
        map.entry(format!("user:{user}")).or_default().push(Instant::now());
        if map.len() > 50_000 {
            map.retain(|_, v| Self::recent(v) > 0);
        }
    }

    pub fn succeeded(&self, user: &str) {
        self.failures.lock().remove(&format!("user:{user}"));
    }
}
