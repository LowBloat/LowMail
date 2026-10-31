//! LowMail app (desktop and Android): the shared engine (`lowmail-core`)
//! behind Tauri commands. Accounts live in the local database, passwords in
//! the system's credential store (see `secrets`).

mod files;
mod secrets;
#[cfg(desktop)]
mod tray;
mod updates;
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

pub struct AppState {
    store: Arc<Store>,
    clients: tokio::sync::Mutex<HashMap<String, Arc<MailClient>>>,
    watching: Mutex<HashSet<String>>,
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
        let password = match password.or_else(|| secrets::stored(id)) {
            Some(p) => p,
            None => return Err(Error::new(ErrorKind::Auth, "password-required")),
        };
        let client = MailClient::connect(account, password.clone(), self.store.clone(), id.to_string(), POOL_SIZE).await?;
        if typed {
            let _ = secrets::remember(id, &password);
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
async fn rpc(
    state: State<'_, AppState>,
    account_id: Option<String>,
    method: String,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
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
        secrets::remember(&account.id, &p)?;
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
    secrets::forget(&id);
    state.store.delete_account(&id)
}

#[tauri::command]
async fn account_test(account: AccountConfig, password: Option<String>) -> Result<()> {
    let password =
        password.filter(|p| !p.is_empty()).or_else(|| secrets::stored(&account.id)).ok_or_else(|| Error::invalid("Enter the password."))?;
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
    secrets::stored(&id).is_some()
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

/// `mailto:` links the app was started with (the OS passes them on launch).
#[tauri::command]
fn startup_urls(app: AppHandle) -> Vec<String> {
    use tauri_plugin_deep_link::DeepLinkExt;
    app.deep_link().get_current().ok().flatten().map(|urls| urls.into_iter().map(|u| u.to_string()).collect()).unwrap_or_default()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("LOWMAIL_LOG").unwrap_or_else(|_| "warn".into()))
        .init();
    let policy = updates::UpdatePolicy::detect();
    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default();
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| window::show(app)));
        builder = builder
            .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec![tray::AUTOSTART_ARG])));
        builder = builder.on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
                && tray::closes_to_tray(window.app_handle())
            {
                api.prevent_close();
                let _ = window.hide();
            }
        });
        // Signed updates from GitHub Releases (see `updates`).
        let mut updater = tauri_plugin_updater::Builder::new();
        if let Some(target) = &policy.target {
            updater = updater.target(target);
        }
        builder = builder.plugin(updater.build());
    }
    builder
        .manage(policy)
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .manage(window::FirstShow::default())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            #[cfg(mobile)]
            secrets::init(dir.join("secrets"));
            let store = Store::open(&dir.join("lowmail.db")).map_err(|e| anyhow_like(e.to_string()))?;
            app.manage(AppState {
                store: Arc::new(store),
                clients: tokio::sync::Mutex::new(HashMap::new()),
                watching: Mutex::new(HashSet::new()),
            });
            #[cfg(desktop)]
            tray::setup(app.handle())?;
            window::start_fallback(app.handle());
            // mailto: links opened while LowMail runs reach the page as `open-url`.
            use tauri_plugin_deep_link::DeepLinkExt;
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    let _ = handle.emit("open-url", url.to_string());
                }
                window::show(&handle);
            });
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
            startup_urls,
            updates::update_policy,
            #[cfg(desktop)]
            tray::desktop_settings,
            #[cfg(desktop)]
            tray::set_close_to_tray,
            #[cfg(desktop)]
            tray::set_autostart,
            #[cfg(desktop)]
            tray::set_tray_language,
        ])
        .build(tauri::generate_context!())
        .expect("error while running LowMail")
        .run(|_app, _event| {
            // On macOS the Dock icon reopens a window hidden in the tray.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                window::show(_app);
            }
        });
}

fn anyhow_like(message: String) -> Box<dyn std::error::Error> {
    message.into()
}
