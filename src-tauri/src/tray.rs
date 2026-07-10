// Tray menu with SumVox parity: mute toggle (muted flag file), recent
// notifications (re-toast on click), open config dir. Rebuilt by the
// watcher whenever muted/history change.

use std::fs;
use tauri::{
    menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager,
};

use crate::sumvox;

const TRAY_ID: &str = "main";
const RECENT_N: usize = 5;

fn recent_lines() -> Vec<String> {
    fs::read_to_string(sumvox::dir().join("history.log"))
        .map(|s| {
            s.lines()
                .rev()
                .filter(|l| !l.trim().is_empty())
                .take(RECENT_N)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

// "RFC3339\ttext" → truncated text for a menu label
fn label(line: &str) -> String {
    let text = line.split_once('\t').map(|(_, t)| t).unwrap_or(line);
    let mut s: String = text.chars().take(40).collect();
    if s.len() < text.len() {
        s.push('…');
    }
    s
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let toggle = MenuItem::with_id(app, "toggle", "Show / Hide", true, None::<&str>)?;
    let muted = sumvox::dir().join("muted").exists();
    let mute = CheckMenuItem::with_id(app, "mute", "Mute", true, muted, None::<&str>)?;

    let lines = recent_lines();
    let mut hist_items: Vec<MenuItem<tauri::Wry>> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        hist_items.push(MenuItem::with_id(
            app,
            format!("hist-{i}"),
            label(line),
            true,
            None::<&str>,
        )?);
    }
    let hist_refs: Vec<&dyn IsMenuItem<tauri::Wry>> =
        hist_items.iter().map(|i| i as _).collect();
    let recent = Submenu::with_items(app, "Recent", !hist_refs.is_empty(), &hist_refs)?;

    let open_config = MenuItem::with_id(app, "open-config", "Open Config Folder", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    Menu::with_items(app, &[&toggle, &mute, &recent, &open_config, &sep, &quit])
}

pub fn refresh(app: &AppHandle) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY_ID), build_menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn toggle_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
        } else {
            let _ = w.show();
        }
    }
}

fn on_menu_event(app: &AppHandle, id: &str) {
    match id {
        "toggle" => toggle_main_window(app),
        "mute" => {
            let flag = sumvox::dir().join("muted");
            let _ = if flag.exists() {
                fs::remove_file(&flag)
            } else {
                fs::write(&flag, "")
            };
            refresh(app); // watcher also refreshes, this just avoids the poll lag
        }
        "open-config" => {
            // ponytail: macOS `open`; xdg-open when Linux lands (M4)
            let _ = std::process::Command::new("open")
                .arg(sumvox::dir())
                .spawn();
        }
        "quit" => app.exit(0),
        _ => {
            if let Some(i) = id.strip_prefix("hist-").and_then(|n| n.parse::<usize>().ok()) {
                if let Some(line) = recent_lines().get(i) {
                    let _ = app.emit("sumvox:history", line.clone());
                }
            }
        }
    }
}

pub fn init(app: &AppHandle) -> tauri::Result<()> {
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&build_menu(app)?)
        .on_menu_event(|app, event| on_menu_event(app, event.id.as_ref()))
        .build(app)?;
    Ok(())
}
