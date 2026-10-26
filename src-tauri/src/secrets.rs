//! Account passwords.
//!
//! On the desktop they live in the OS credential store (Windows Credential
//! Manager, macOS Keychain, Secret Service). Android has no such store
//! reachable from Rust, so there each password is a file in the app's private
//! data folder: other apps cannot read it, and the system encrypts that folder
//! at rest. Android mail apps keep account passwords the same way.

use lowmail_core::Result;

#[cfg(desktop)]
mod imp {
    use lowmail_core::{Error, Result};

    const SERVICE: &str = "LowMail";

    fn entry(id: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, id)
            .map_err(|e| Error::internal("The system password store is not available.").with_detail(e.to_string()))
    }

    pub fn get(id: &str) -> Option<String> {
        entry(id).ok()?.get_password().ok()
    }

    pub fn set(id: &str, password: &str) -> Result<()> {
        entry(id)?
            .set_password(password)
            .map_err(|e| Error::internal("The password could not be saved in the system password store.").with_detail(e.to_string()))
    }

    pub fn forget(id: &str) {
        if let Ok(e) = entry(id) {
            let _ = e.delete_credential();
        }
    }
}

#[cfg(mobile)]
mod imp {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    use lowmail_core::{Error, Result};

    static DIR: OnceLock<PathBuf> = OnceLock::new();

    /// Set once at start-up to a folder inside the app's private data.
    pub fn init(dir: PathBuf) {
        let _ = DIR.set(dir);
    }

    fn file(id: &str) -> Option<PathBuf> {
        // Account ids are UUIDs; anything else never reaches the disk.
        let safe = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        safe.then(|| DIR.get().map(|d| d.join(id))).flatten()
    }

    pub fn get(id: &str) -> Option<String> {
        std::fs::read_to_string(file(id)?).ok()
    }

    pub fn set(id: &str, password: &str) -> Result<()> {
        let path = file(id).ok_or_else(|| Error::internal("The password store is not ready."))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::internal("The password could not be saved.").with_detail(e.to_string()))?;
        }
        std::fs::write(&path, password).map_err(|e| Error::internal("The password could not be saved.").with_detail(e.to_string()))
    }

    pub fn forget(id: &str) {
        if let Some(path) = file(id) {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(mobile)]
pub use imp::init;

pub fn stored(id: &str) -> Option<String> {
    imp::get(id)
}

pub fn remember(id: &str, password: &str) -> Result<()> {
    imp::set(id, password)
}

pub fn forget(id: &str) {
    imp::forget(id)
}
