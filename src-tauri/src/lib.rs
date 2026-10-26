//! LowMail desktop: the shared engine (`lowmail-core`) behind Tauri commands.
//! Accounts live in the local database, passwords in the OS credential store.

mod files;
mod window;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use lowmail_core::model::{AccountConfig, Upload};
use lowmail_core::service::{AccountInfo, MailClient};
use lowmail_core::store::Store;
use lowmail_core::{Error, ErrorKind, Result};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

/// IMAP connections per account on the desktop.
const POOL_SIZE: usize = 3;
const KEYRING_SERVICE: &str = "LowMail";

pub struct AppState {
    store: Arc<Store>,
    clients: tokio::sync::Mutex<HashMap<String, Arc<MailClient>>>,
    watching: Mutex<HashSet<String>>,
}

fn keyring(id: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, id).map_err(|e| Error::internal("The system password store is not available.").with_detail(e.to_string()))
}

fn stored_password(id: &str) -> Option<String> {
    keyring(id).ok()?.get_password().ok()
}

fn remember_password(id: &str, password: &str) -> Result<()> {
    keyring(id)?.set_password(password).map_err(|e| Error::internal("The password could not be saved in the system password store.").with_detail(e.to_string()))
}

fn forget_password(id: &str) {
    if let Ok(entry) = keyring(id) {
        let _ = entry.delete_credential();
    }
}

impl AppState {
    fn account(&self, id: &str) -> Result<AccountConfig> {
        self.store.accounts()?.into_iter().find(|a| a.id == id).ok_or_else(|| Error::not_found("Account not found."))
    }

    /// The signed-in client of an account, signing in when needed.
    async fn client(&self, id: Option<&str>, password: Option<String>) -> Result<Arc<MailClient>> {
        let id = id.ok_or_else(|| Error::invalid("No account selected."))?;
        let mut clients = self.clients.lock().await;
        if password.is_none()
            && let Some(c) = clients.get(id)
        {
            return Ok(c.clone());
        }
        let account = self.account(id)?;
        let typed = password.is_some();
        let password = match password.or_else(|| stored_password(id)) {
            Some(p) => p,
            None => return Err(Error::new(ErrorKind::Auth, "password-required")),
        };
        let client = MailClient::connect(account, password.clone(), self.store.clone(), id.to_string(), POOL_SIZE).await?;
        if typed {
            let _ = remember_password(id, &password);
        }
        if let Some(old) = clients.insert(id.to_string(), client.clone()) {
            tokio::spawn(async move { old.close().await });
        }
        Ok(client)
    }

    async fn drop_client(&self, id: &str) {
        let removed = self.clients.lock().await.remove(id);
        self.watching.lock().remove(id);
        if let Some(c) = removed {
            c.close().await;
        }
    }
}

#[tauri::command]
async fn rpc(state: State<'_, AppState>, account_id: Option<String>, method: String, params: serde_json::Value) -> Result<serde_json::Value> {
    let client = state.client(account_id.as_deref(), None).await?;
    lowmail_core::rpc::dispatch(&client, &method, params).await
}

#[tauri::command]
fn accounts_list(state: State<'_, AppState>) -> Result<Vec<AccountConfig>> {
    state.store.accounts()
}

#[tauri::command]
async fn account_save(state: State<'_, AppState>, mut account: AccountConfig, password: Option<String>) -> Result<AccountConfig> {
    account.email = account.email.trim().to_string();
    lowmail_core::compose::validate_address(&account.email)?;
    if account.username.trim().is_empty() {
        account.username = account.email.clone();
    }
    if account.id.is_empty() {
        account.id = uuid::Uuid::new_v4().to_string();
    }
    if let Some(p) = password.filter(|p| !p.is_empty()) {
        remember_password(&account.id, &p)?;
    }
    state.store.save_account(&account)?;
    // The identity name follows the account name the first time.
    if !account.name.trim().is_empty() {
        let ids = state.store.identities(&account.id)?;
        if ids.is_empty() {
            let _ = state.store.identities_or_default(&account.id, &account.email, account.name.trim());
        }
    }
    // Server settings may have changed: sign in again on next use.
    state.drop_client(&account.id).await;
    Ok(account)
}

#[tauri::command]
async fn account_delete(state: State<'_, AppState>, id: String) -> Result<()> {
    state.drop_client(&id).await;
    forget_password(&id);
    state.store.delete_account(&id)
}

#[tauri::command]
async fn account_test(account: AccountConfig, password: Option<String>) -> Result<()> {
    let password = password.filter(|p| !p.is_empty()).or_else(|| stored_password(&account.id)).ok_or_else(|| Error::invalid("Enter the password."))?;
    lowmail_core::service::test_account(&account, &password).await
}

#[tauri::command]
async fn account_discover(email: String) -> Result<AccountConfig> {
    lowmail_core::discover::discover(&email).await
}

#[tauri::command]
async fn account_connect(state: State<'_, AppState>, id: String, password: Option<String>) -> Result<AccountInfo> {
    let client = state.client(Some(&id), password.filter(|p| !p.is_empty())).await?;
    Ok(client.info())
}

#[tauri::command]
fn account_has_password(id: String) -> bool {
    stored_password(&id).is_some()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventPayload {
    account_id: String,
    event: lowmail_core::model::MailEvent,
}

/// Starts watching an account for new mail; events reach the page as `mail-event`.
#[tauri::command]
async fn watch(app: AppHandle, state: State<'_, AppState>, account_id: Option<String>) -> Result<()> {
    let client = state.client(account_id.as_deref(), None).await?;
    let id = account_id.unwrap_or_default();
    if !state.watching.lock().insert(id.clone()) {
        return Ok(());
    }
    client.start_watching();
    let mut events = client.subscribe();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    let _ = app.emit("mail-event", EventPayload { account_id: id.clone(), event });
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
    Ok(())
}

/// Attachment bytes sent as the raw request body.
#[tauri::command]
async fn upload(state: State<'_, AppState>, request: tauri::ipc::Request<'_>) -> Result<Upload> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(Error::invalid("Invalid upload."));
    };
    let header = |name: &str| {
        request
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|v| percent_encoding::percent_decode_str(v).decode_utf8_lossy().into_owned())
            .unwrap_or_default()
    };
    let account = header("x-account");
    let client = state.client(Some(&account), None).await?;
    client.add_upload(&header("x-filename"), &header("x-content-type"), bytes.clone())
}

pub fn run() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_env("LOWMAIL_LOG").unwrap_or_else(|_| "warn".into())).init();
    let mut builder = tauri::Builder::default();
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }));
    }
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .manage(window::FirstShow::default())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let store = Store::open(&dir.join("lowmail.db")).map_err(|e| anyhow_like(e.to_string()))?;
            app.manage(AppState { store: Arc::new(store), clients: tokio::sync::Mutex::new(HashMap::new()), watching: Mutex::new(HashSet::new()) });
            window::start_fallback(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            rpc,
            accounts_list,
            account_save,
            account_delete,
            account_test,
            account_discover,
            account_connect,
            account_has_password,
            watch,
            upload,
            files::upload_pick,
            files::attachment_save,
            files::attachment_open,
            files::message_save,
            files::text_save,
            window::window_painted,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LowMail");
}

fn anyhow_like(message: String) -> Box<dyn std::error::Error> {
    message.into()
}
