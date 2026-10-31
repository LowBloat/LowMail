//! The tray icon and "Start with the system" (desktop only). Closing the
//! window hides LowMail in the tray, where it keeps watching for new mail; the
//! tray reopens or quits it. Started by the system at sign-in, it stays there.

use std::sync::atomic::{AtomicBool, Ordering};

use lowmail_core::{Error, Result};
use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime, Wry};
use tauri_plugin_autostart::ManagerExt;

use crate::AppState;

/// The argument the system starts LowMail with at sign-in.
pub const AUTOSTART_ARG: &str = "--autostart";

/// App-wide preferences live in the `kv` table under this owner (account
/// preferences use the account id).
const OWNER: &str = "app";
const KEY: &str = "desktop";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Prefs {
    close_to_tray: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { close_to_tray: true }
    }
}

pub struct Tray {
    close_to_tray: AtomicBool,
    open: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

/// Tray menu labels for the interface language (`pt-BR` or `en`).
fn labels(language: &str) -> (&'static str, &'static str) {
    if language.starts_with("pt") { ("Abrir o LowMail", "Sair do LowMail") } else { ("Open LowMail", "Quit LowMail") }
}

fn load(app: &AppHandle) -> Prefs {
    let stored = app.state::<AppState>().store.get_value(OWNER, KEY).ok().flatten();
    stored.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let (open_label, quit_label) = labels("en");
    let open = MenuItem::with_id(app, "open", open_label, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", quit_label, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("LowMail")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => crate::window::show(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                crate::window::show(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    app.manage(Tray { close_to_tray: AtomicBool::new(load(app).close_to_tray), open, quit });
    Ok(())
}

/// Whether closing the window only hides it.
pub fn closes_to_tray<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.try_state::<Tray>().is_some_and(|t| t.close_to_tray.load(Ordering::SeqCst))
}

/// Started by the system at sign-in with "close to tray" on: the window
/// stays hidden until the tray icon opens it.
pub fn starts_hidden<R: Runtime>(app: &AppHandle<R>) -> bool {
    closes_to_tray(app) && std::env::args().any(|a| a == AUTOSTART_ARG)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopSettings {
    close_to_tray: bool,
    autostart: bool,
}

#[tauri::command]
pub fn desktop_settings(app: AppHandle) -> DesktopSettings {
    DesktopSettings { close_to_tray: closes_to_tray(&app), autostart: app.autolaunch().is_enabled().unwrap_or(false) }
}

#[tauri::command]
pub fn set_close_to_tray(app: AppHandle, on: bool) -> Result<()> {
    let value = serde_json::to_value(Prefs { close_to_tray: on }).unwrap_or_default();
    app.state::<AppState>().store.set_value(OWNER, KEY, &value)?;
    if let Some(t) = app.try_state::<Tray>() {
        t.close_to_tray.store(on, Ordering::SeqCst);
    }
    Ok(())
}

/// Adds or removes LowMail from the programs the system starts at sign-in.
#[tauri::command]
pub fn set_autostart(app: AppHandle, on: bool) -> Result<()> {
    let launcher = app.autolaunch();
    let done = if on { launcher.enable() } else { launcher.disable() };
    done.map_err(|e| Error::internal("Could not change whether LowMail starts with the system.").with_detail(e.to_string()))
}

/// The page reports its language on start and whenever it changes.
#[tauri::command]
pub fn set_tray_language(app: AppHandle, language: String) {
    if let Some(t) = app.try_state::<Tray>() {
        let (open, quit) = labels(&language);
        let _ = t.open.set_text(open);
        let _ = t.quit.set_text(quit);
    }
}
