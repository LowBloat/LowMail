//! Files on disk: attaching, saving attachments and messages, opening
//! attachments with the system's apps. Files go through the fs plugin, which
//! also reads and writes the `content://` locations Android's pickers return.

use std::io::Write as _;

use lowmail_core::model::Upload;
use lowmail_core::service::MailClient;
use lowmail_core::{Error, Result};
#[cfg(desktop)]
use tauri::Manager;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tauri_plugin_fs::{FsExt, OpenOptions};
#[cfg(desktop)]
use tauri_plugin_opener::OpenerExt;

use crate::AppState;

async fn ask_save(app: &AppHandle, filename: &str) -> Option<FilePath> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_file_name(filename).save_file(move |path| {
        let _ = tx.send(path);
    });
    rx.await.ok().flatten()
}

/// The name of a picked file: the last part of its path, or of its
/// (percent-encoded) `content://` address.
fn file_name(file: &FilePath) -> String {
    let raw = match file {
        FilePath::Path(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()),
        FilePath::Url(u) => {
            let decoded = percent_encoding::percent_decode_str(u.path()).decode_utf8_lossy().into_owned();
            decoded.rsplit(['/', ':']).next().map(str::to_string)
        }
    };
    raw.filter(|n| !n.is_empty()).unwrap_or_else(|| "file".into())
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
#[cfg_attr(mobile, allow(dead_code))]
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

fn write(app: &AppHandle, target: FilePath, bytes: &[u8]) -> Result<()> {
    let failed = |e: std::io::Error| Error::internal("The file could not be saved.").with_detail(e.to_string());
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    let local = target.clone().into_path().ok();
    let mut file = app.fs().open(target, options).map_err(failed)?;
    file.write_all(bytes).map_err(failed)?;
    if let Some(path) = local {
        mark_from_internet(&path);
    }
    Ok(())
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
        let name = file_name(&f);
        let bytes = app.fs().read(f).map_err(|e| Error::not_found("The file could not be read.").with_detail(e.to_string()))?;
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
        Some(target) => write(&app, target, &bytes).map(|_| true),
        None => Ok(false),
    }
}

/// Opens an attachment with the default app, from a private temporary folder.
/// On Android, where other apps cannot read that folder, it is saved instead.
#[cfg(mobile)]
#[tauri::command]
pub async fn attachment_open(
    app: AppHandle,
    state: State<'_, AppState>,
    account_id: Option<String>,
    folder: String,
    uid: u32,
    part: String,
) -> Result<()> {
    attachment_save(app, state, account_id, folder, uid, part).await.map(|_| ())
}

#[cfg(desktop)]
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
    write(&app, FilePath::Path(path.clone()), &bytes)?;
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
        Some(target) => write(&app, target, &raw).map(|_| true),
        None => Ok(false),
    }
}

#[tauri::command]
pub async fn text_save(app: AppHandle, filename: String, text: String) -> Result<bool> {
    match ask_save(&app, &safe_name(&filename)).await {
        Some(target) => write(&app, target, text.as_bytes()).map(|_| true),
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
