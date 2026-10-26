//! Local data in SQLite: preferences, identities and contacts per owner (an
//! e-mail address on the web server, an account id on the desktop), and the
//! desktop's account list. Passwords are never stored here.

use std::path::Path;

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{Error, Result};
use crate::model::{AccountConfig, Address, Contact, Identity};

pub struct Store {
    db: Mutex<Connection>,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS kv (
    owner TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (owner, key)
);
CREATE TABLE IF NOT EXISTS identities (
    owner TEXT NOT NULL,
    id TEXT NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    data TEXT NOT NULL,
    PRIMARY KEY (owner, id)
);
CREATE TABLE IF NOT EXISTS contacts (
    owner TEXT NOT NULL,
    id TEXT NOT NULL,
    name TEXT NOT NULL,
    emails TEXT NOT NULL,
    collected INTEGER NOT NULL DEFAULT 0,
    uses INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL,
    data TEXT NOT NULL,
    PRIMARY KEY (owner, id)
);
CREATE INDEX IF NOT EXISTS contacts_owner_name ON contacts (owner, name);
CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    position INTEGER NOT NULL DEFAULT 0,
    data TEXT NOT NULL
);
"#;

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::internal("Could not create the data folder.").with_detail(e.to_string()))?;
        }
        let db = Connection::open(path)?;
        Self::init(db)
    }

    pub fn memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(db: Connection) -> Result<Store> {
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch(SCHEMA)?;
        Ok(Store { db: Mutex::new(db) })
    }

    // ---- key/value ---------------------------------------------------------

    pub fn get_value(&self, owner: &str, key: &str) -> Result<Option<serde_json::Value>> {
        let db = self.db.lock();
        let text: Option<String> =
            db.query_row("SELECT value FROM kv WHERE owner = ?1 AND key = ?2", params![owner, key], |r| r.get(0)).optional()?;
        Ok(text.and_then(|t| serde_json::from_str(&t).ok()))
    }

    pub fn set_value(&self, owner: &str, key: &str, value: &serde_json::Value) -> Result<()> {
        self.db.lock().execute(
            "INSERT INTO kv (owner, key, value) VALUES (?1, ?2, ?3)
             ON CONFLICT (owner, key) DO UPDATE SET value = excluded.value",
            params![owner, key, value.to_string()],
        )?;
        Ok(())
    }

    // ---- identities --------------------------------------------------------

    pub fn identities(&self, owner: &str) -> Result<Vec<Identity>> {
        let db = self.db.lock();
        let mut stmt = db.prepare("SELECT data FROM identities WHERE owner = ?1 ORDER BY position, rowid")?;
        let rows = stmt.query_map(params![owner], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            if let Ok(identity) = serde_json::from_str::<Identity>(&row?) {
                out.push(identity);
            }
        }
        Ok(out)
    }

    /// Identities of `owner`, creating the default one from the account
    /// address on first use.
    pub fn identities_or_default(&self, owner: &str, email: &str, name: &str) -> Result<Vec<Identity>> {
        let list = self.identities(owner)?;
        if !list.is_empty() {
            return Ok(list);
        }
        let identity = Identity {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            email: email.to_string(),
            reply_to: None,
            bcc: None,
            signature: String::new(),
            is_default: true,
        };
        self.save_identity(owner, identity)?;
        self.identities(owner)
    }

    pub fn save_identity(&self, owner: &str, mut identity: Identity) -> Result<Identity> {
        identity.email = identity.email.trim().to_string();
        crate::compose::validate_address(&identity.email)?;
        if identity.id.is_empty() {
            identity.id = uuid::Uuid::new_v4().to_string();
        }
        let mut db = self.db.lock();
        let tx = db.transaction()?;
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM identities WHERE owner = ?1", params![owner], |r| r.get(0))?;
        if count == 0 {
            identity.is_default = true;
        }
        if identity.is_default {
            // Only one default: clear the flag on the others.
            let mut stmt = tx.prepare("SELECT id, data FROM identities WHERE owner = ?1 AND id != ?2")?;
            let others: Vec<(String, String)> =
                stmt.query_map(params![owner, identity.id], |r| Ok((r.get(0)?, r.get(1)?)))?.filter_map(|r| r.ok()).collect();
            drop(stmt);
            for (id, data) in others {
                if let Ok(mut other) = serde_json::from_str::<Identity>(&data)
                    && other.is_default
                {
                    other.is_default = false;
                    tx.execute(
                        "UPDATE identities SET data = ?3 WHERE owner = ?1 AND id = ?2",
                        params![owner, id, serde_json::to_string(&other)?],
                    )?;
                }
            }
        }
        tx.execute(
            "INSERT INTO identities (owner, id, position, data) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (owner, id) DO UPDATE SET data = excluded.data",
            params![owner, identity.id, count, serde_json::to_string(&identity)?],
        )?;
        tx.commit()?;
        Ok(identity)
    }

    pub fn delete_identity(&self, owner: &str, id: &str) -> Result<()> {
        let list = self.identities(owner)?;
        if list.len() <= 1 {
            return Err(Error::invalid("Keep at least one identity."));
        }
        let was_default = list.iter().any(|i| i.id == id && i.is_default);
        self.db.lock().execute("DELETE FROM identities WHERE owner = ?1 AND id = ?2", params![owner, id])?;
        if was_default && let Some(mut first) = self.identities(owner)?.into_iter().next() {
            first.is_default = true;
            self.save_identity(owner, first)?;
        }
        Ok(())
    }

    // ---- contacts ----------------------------------------------------------

    fn row_to_contact(data: String, collected: bool, updated_at: i64, id: String) -> Option<Contact> {
        let mut c: Contact = serde_json::from_str(&data).ok()?;
        c.id = id;
        c.collected = collected;
        c.updated_at = updated_at;
        Some(c)
    }

    pub fn contacts(&self, owner: &str, query: &str, include_collected: bool, limit: usize) -> Result<Vec<Contact>> {
        let db = self.db.lock();
        let like = format!("%{}%", query.trim().to_lowercase().replace('%', "\\%").replace('_', "\\_"));
        let mut stmt = db.prepare(
            "SELECT id, data, collected, updated_at FROM contacts
             WHERE owner = ?1 AND (?2 = 1 OR collected = 0)
               AND (lower(name) LIKE ?3 ESCAPE '\\' OR lower(emails) LIKE ?3 ESCAPE '\\')
             ORDER BY collected, lower(name), lower(emails) LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![owner, include_collected as i64, like, limit as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? != 0, r.get::<_, i64>(3)?))
        })?;
        Ok(rows.filter_map(|r| r.ok()).filter_map(|(id, data, col, at)| Self::row_to_contact(data, col, at, id)).collect())
    }

    /// Recipients for the address field: saved contacts first, then
    /// addresses collected from sent mail, most used first.
    pub fn suggest(&self, owner: &str, query: &str, limit: usize) -> Result<Vec<Address>> {
        let db = self.db.lock();
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let like = format!("%{}%", q.replace('%', "\\%").replace('_', "\\_"));
        let mut stmt = db.prepare(
            "SELECT data FROM contacts WHERE owner = ?1
               AND (lower(name) LIKE ?2 ESCAPE '\\' OR lower(emails) LIKE ?2 ESCAPE '\\')
             ORDER BY collected, uses DESC, lower(name) LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![owner, like, (limit * 2) as i64], |r| r.get::<_, String>(0))?;
        let mut out: Vec<Address> = Vec::new();
        for data in rows.filter_map(|r| r.ok()) {
            let Ok(c) = serde_json::from_str::<Contact>(&data) else { continue };
            let name_match = c.name.to_lowercase().contains(&q);
            for email in &c.emails {
                if (name_match || email.to_lowercase().contains(&q)) && !out.iter().any(|a| a.email.eq_ignore_ascii_case(email)) {
                    out.push(Address::new(Some(c.name.clone()), email.clone()));
                }
            }
            if out.len() >= limit {
                break;
            }
        }
        out.truncate(limit);
        Ok(out)
    }

    pub fn contact(&self, owner: &str, id: &str) -> Result<Contact> {
        let db = self.db.lock();
        let (data, collected, at): (String, bool, i64) =
            db.query_row("SELECT data, collected, updated_at FROM contacts WHERE owner = ?1 AND id = ?2", params![owner, id], |r| {
                Ok((r.get(0)?, r.get::<_, i64>(1)? != 0, r.get(2)?))
            })?;
        Self::row_to_contact(data, collected, at, id.to_string()).ok_or_else(|| Error::not_found("Contact not found."))
    }

    pub fn save_contact(&self, owner: &str, mut contact: Contact) -> Result<Contact> {
        contact.emails = contact.emails.iter().map(|e| e.trim().to_string()).filter(|e| !e.is_empty()).collect();
        contact.phones = contact.phones.iter().map(|e| e.trim().to_string()).filter(|e| !e.is_empty()).collect();
        contact.name = contact.name.trim().to_string();
        for e in &contact.emails {
            crate::compose::validate_address(e)?;
        }
        if contact.name.is_empty() && contact.emails.is_empty() {
            return Err(Error::invalid("Enter a name or an e-mail address."));
        }
        if contact.id.is_empty() {
            contact.id = uuid::Uuid::new_v4().to_string();
        }
        contact.updated_at = chrono::Utc::now().timestamp_millis();
        let emails = contact.emails.join(" ").to_lowercase();
        let db = self.db.lock();
        // Saving a contact replaces the collected entries for its addresses.
        for e in &contact.emails {
            db.execute(
                "DELETE FROM contacts WHERE owner = ?1 AND collected = 1 AND lower(emails) = ?2 AND id != ?3",
                params![owner, e.to_lowercase(), contact.id],
            )?;
        }
        db.execute(
            "INSERT INTO contacts (owner, id, name, emails, collected, uses, updated_at, data)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7)
             ON CONFLICT (owner, id) DO UPDATE SET name = excluded.name, emails = excluded.emails,
               collected = excluded.collected, updated_at = excluded.updated_at, data = excluded.data",
            params![
                owner,
                contact.id,
                contact.name,
                emails,
                contact.collected as i64,
                contact.updated_at,
                serde_json::to_string(&contact)?
            ],
        )?;
        Ok(contact)
    }

    pub fn delete_contacts(&self, owner: &str, ids: &[String]) -> Result<()> {
        let db = self.db.lock();
        for id in ids {
            db.execute("DELETE FROM contacts WHERE owner = ?1 AND id = ?2", params![owner, id])?;
        }
        Ok(())
    }

    /// Remembers recipients of sent mail for address suggestions.
    pub fn collect(&self, owner: &str, addresses: &[Address]) -> Result<()> {
        let db = self.db.lock();
        for a in addresses {
            let email = a.email.trim().to_lowercase();
            if email.is_empty() {
                continue;
            }
            let existing: Option<(String, i64)> = db
                .query_row(
                    "SELECT id, collected FROM contacts WHERE owner = ?1 AND (' ' || lower(emails) || ' ') LIKE ?2",
                    params![owner, format!("% {email} %")],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            match existing {
                Some((id, _)) => {
                    db.execute("UPDATE contacts SET uses = uses + 1 WHERE owner = ?1 AND id = ?2", params![owner, id])?;
                }
                None => {
                    let contact = Contact {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: a.name.clone().unwrap_or_default(),
                        emails: vec![a.email.trim().to_string()],
                        collected: true,
                        updated_at: chrono::Utc::now().timestamp_millis(),
                        ..Default::default()
                    };
                    db.execute(
                        "INSERT INTO contacts (owner, id, name, emails, collected, uses, updated_at, data)
                         VALUES (?1, ?2, ?3, ?4, 1, 1, ?5, ?6)",
                        params![owner, contact.id, contact.name, email, contact.updated_at, serde_json::to_string(&contact)?],
                    )?;
                }
            }
        }
        Ok(())
    }

    pub fn contact_groups(&self, owner: &str) -> Result<Vec<String>> {
        let mut groups: Vec<String> = self.contacts(owner, "", false, 100_000)?.into_iter().flat_map(|c| c.groups).collect();
        groups.sort_by_key(|g| g.to_lowercase());
        groups.dedup();
        Ok(groups)
    }

    // ---- desktop accounts --------------------------------------------------

    pub fn accounts(&self) -> Result<Vec<AccountConfig>> {
        let db = self.db.lock();
        let mut stmt = db.prepare("SELECT data FROM accounts ORDER BY position, rowid")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).filter_map(|d| serde_json::from_str(&d).ok()).collect())
    }

    pub fn save_account(&self, account: &AccountConfig) -> Result<()> {
        let db = self.db.lock();
        let count: i64 = db.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get(0))?;
        db.execute(
            "INSERT INTO accounts (id, position, data) VALUES (?1, ?2, ?3)
             ON CONFLICT (id) DO UPDATE SET data = excluded.data",
            params![account.id, count, serde_json::to_string(account)?],
        )?;
        Ok(())
    }

    pub fn delete_account(&self, id: &str) -> Result<()> {
        let mut db = self.db.lock();
        let tx = db.transaction()?;
        tx.execute("DELETE FROM accounts WHERE id = ?1", params![id])?;
        for table in ["kv", "identities", "contacts"] {
            tx.execute(&format!("DELETE FROM {table} WHERE owner = ?1"), params![id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn reorder_accounts(&self, ids: &[String]) -> Result<()> {
        let db = self.db.lock();
        for (i, id) in ids.iter().enumerate() {
            db.execute("UPDATE accounts SET position = ?2 WHERE id = ?1", params![id, i as i64])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contacts_and_suggestions() {
        let s = Store::memory().unwrap();
        s.collect("me", &[Address::new(Some("Ana".into()), "ana@example.com"), Address::new(None, "bob@example.com")]).unwrap();
        s.collect("me", &[Address::new(None, "ana@example.com")]).unwrap();
        let saved = s
            .save_contact(
                "me",
                Contact {
                    name: "Ana Souza".into(),
                    emails: vec!["ana@example.com".into()],
                    groups: vec!["Trabalho".into()],
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!saved.id.is_empty());
        let all = s.contacts("me", "", true, 100).unwrap();
        assert_eq!(all.len(), 2, "collected Ana replaced by the saved contact");
        assert_eq!(s.contacts("me", "", false, 100).unwrap().len(), 1);
        let sug = s.suggest("me", "an", 10).unwrap();
        assert_eq!(sug[0].name.as_deref(), Some("Ana Souza"));
        assert!(s.suggest("other", "an", 10).unwrap().is_empty());
        assert_eq!(s.contact_groups("me").unwrap(), vec!["Trabalho"]);
    }

    #[test]
    fn identities_default() {
        let s = Store::memory().unwrap();
        let ids = s.identities_or_default("me", "me@example.com", "Me").unwrap();
        assert_eq!(ids.len(), 1);
        assert!(ids[0].is_default);
        let second = s
            .save_identity(
                "me",
                Identity {
                    id: String::new(),
                    name: "Sales".into(),
                    email: "sales@example.com".into(),
                    reply_to: None,
                    bcc: None,
                    signature: String::new(),
                    is_default: true,
                },
            )
            .unwrap();
        let ids = s.identities("me").unwrap();
        assert_eq!(ids.iter().filter(|i| i.is_default).count(), 1);
        assert!(ids.iter().find(|i| i.id == second.id).unwrap().is_default);
        s.delete_identity("me", &second.id).unwrap();
        assert!(s.identities("me").unwrap()[0].is_default);
    }
}
