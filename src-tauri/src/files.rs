//! Files on disk: attaching, saving attachments and messages, opening
//! attachments with the system's apps.

use std::path::PathBuf;

use lowmail_core::model::Upload;
use lowmail_core::service::MailClient;
use lowmail_core::{Error, Result};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tauri_plugin_opener::OpenerExt;

use crate::AppState;

fn path_of(file: FilePath) -> Result<PathBuf> {
    file.into_path().map_err(|_| Error::invalid("Unsupported file location."))
}

async fn ask_save(app: &AppHandle, filename: &str) -> Option<PathBuf> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_file_name(filename).save_file(move |path| {
        let _ = tx.send(path);
    });
    rx.await.ok().flatten().and_then(|p| path_of(p).ok())
}

/// A file name safe on every system: no separators, reserved characters,
/// trailing dots or names like `..` and `CON`.
fn safe_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    let stem = cleaned.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "LPT1", "LPT2", "LPT3"];
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') || reserved.contains(&stem.as_str()) {
        format!("attachment-{cleaned}").trim_end_matches(['.', '-']).to_string()
    } else {
        cleaned
    }
}

/// File types that run code when opened with their default app.
fn is_executable(name: &str) -> bool {
    const RISKY: &[&str] = &[
        "exe",
        "com",
        "scr",
        "bat",
        "cmd",
        "pif",
        "msi",
        "msp",
        "msix",
        "appx",
        "appxbundle",
        "js",
        "jse",
        "vbs",
        "vbe",
        "wsf",
        "wsh",
        "ps1",
        "psm1",
        "hta",
        "lnk",
        "url",
        "reg",
        "cpl",
        "jar",
        "scf",
        "inf",
        "dll",
        "sys",
        "gadget",
        "application",
        "iso",
        "img",
        "vhd",
        "vhdx",
        "chm",
        "xll",
        "sh",
        "command",
        "app",
        "pkg",
        "dmg",
        "desktop",
        "appimage",
        "run",
        "bin",
        "deb",
        "rpm",
    ];
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    name.contains('.') && RISKY.contains(&ext.as_str())
}

/// Marks a file as downloaded from the Internet (Windows Mark of the Web),
/// so Office opens it in Protected View and SmartScreen checks it.
fn mark_from_internet(path: &std::path::Path) {
    #[cfg(windows)]
    {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        let _ = std::fs::write(stream, "[ZoneTransfer]\r\nZoneId=3\r\n");
    }
    #[cfg(not(windows))]
    let _ = path;
}

fn write(path: &PathBuf, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes).map_err(|e| Error::internal("The file could not be saved.").with_detail(e.to_string()))
}

/// Picks files with the native dialog and attaches them.
#[tauri::command]
pub async fn upload_pick(app: AppHandle, state: State<'_, AppState>, account_id: Option<String>) -> Result<Vec<Upload>> {
    let client = state.client(account_id.as_deref(), None).await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_files(move |files| {
        let _ = tx.send(files);
    });
    let Some(files) = rx.await.ok().flatten() else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for f in files {
        let path = path_of(f)?;
        let bytes = std::fs::read(&path).map_err(|e| Error::not_found("The file could not be read.").with_detail(e.to_string()))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
        out.push(client.add_upload(&name, "", bytes)?);
    }
    Ok(out)
}

/// Saves an attachment where the user chooses. False when cancelled.
#[tauri::command]
pub async fn attachment_save(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: Option<String>,
    folder: String,
    uid: u32,
    part: String,
) -> Result<bool> {
    let client = state.client(account_id.as_deref(), None).await?;
    let (name, _, bytes) = client.attachment(&folder, uid, &part).await?;
    match ask_save(&app, &safe_name(&name)).await {
        Some(path) => {
            write(&path, &bytes)?;
            mark_from_internet(&path);
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Opens an attachment with the default app, from a private temporary folder.
#[tauri::command]
pub async fn attachment_open(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: Option<String>,
    folder: String,
    uid: u32,
    part: String,
) -> Result<()> {
    let client = state.client(account_id.as_deref(), None).await?;
    let (name, _, bytes) = client.attachment(&folder, uid, &part).await?;
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| Error::internal("No cache folder.").with_detail(e.to_string()))?
        .join("attachments")
        .join(uuid::Uuid::new_v4().simple().to_string());
    std::fs::create_dir_all(&dir).map_err(|e| Error::internal("The temporary folder could not be created.").with_detail(e.to_string()))?;
    let name = safe_name(&name);
    if is_executable(&name) {
        return Err(Error::invalid(
            "This kind of file can run programs on the computer, so LowMail does not open it. Save it and open it yourself only if you trust the sender.",
        ));
    }
    let path = dir.join(&name);
    write(&path, &bytes)?;
    mark_from_internet(&path);
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| Error::internal("No app opens this kind of file.").with_detail(e.to_string()))
}

#[tauri::command]
pub async fn message_save(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: Option<String>,
    folder: String,
    uid: u32,
) -> Result<bool> {
    let client = state.client(account_id.as_deref(), None).await?;
    let raw = client.raw(&folder, uid).await?;
    match ask_save(&app, &safe_name(&MailClient::eml_name(&raw))).await {
        Some(path) => write(&path, &raw).map(|_| true),
        None => Ok(false),
    }
}

#[tauri::command]
pub async fn text_save(app: AppHandle, filename: String, text: String) -> Result<bool> {
    match ask_save(&app, &safe_name(&filename)).await {
        Some(path) => write(&path, text.as_bytes()).map(|_| true),
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn safe_names() {
        assert_eq!(super::safe_name(".."), "attachment");
        assert_eq!(super::safe_name("a/b\\c.pdf"), "a_b_c.pdf");
        assert_eq!(super::safe_name("con.txt"), "attachment-con.txt");
        assert!(super::is_executable("fatura.pdf.exe"));
        assert!(super::is_executable("x.JS"));
        assert!(!super::is_executable("fatura.pdf"));
        assert_eq!(super::safe_name("relatório: final?.pdf"), "relatório_ final_.pdf");
    }
}
