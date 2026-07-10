// Tray menu (presentation): mute toggle, recent reports (click re-toasts),
// open config dir. Talks to the notifier only through the sumvox adapter's
// public fns and emits only core events. Rebuilt by the watcher whenever
// muted/history change.
//
// ponytail: mute/recent/config-dir go straight to the sumvox adapter; put a
// port trait in front when a second notifier actually lands.

use tauri::{
    menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager,
};

use crate::events::AGENT_REPORT;
use crate::sumvox;

const TRAY_ID: &str = "main";
const RECENT_N: usize = 5;

fn label(text: &str) -> String {
    let mut s: String = text.chars().take(40).collect();
    if s.len() < text.len() {
        s.push('…');
    }
    s
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let toggle = MenuItem::with_id(app, "toggle", "Show / Hide", true, None::<&str>)?;
    let mute = CheckMenuItem::with_id(app, "mute", "Mute", true, sumvox::is_muted(), None::<&str>)?;

    let reports = sumvox::recent(RECENT_N);
    let mut hist_items: Vec<MenuItem<tauri::Wry>> = Vec::new();
    for (i, r) in reports.iter().enumerate() {
        hist_items.push(MenuItem::with_id(
            app,
            format!("hist-{i}"),
            label(&r.text),
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
            sumvox::set_muted(!sumvox::is_muted());
            refresh(app); // watcher also refreshes, this just avoids the poll lag
        }
        "open-config" => {
            // ponytail: macOS `open`; xdg-open when Linux lands (M4)
            let _ = std::process::Command::new("open")
                .arg(sumvox::config_dir())
                .spawn();
        }
        "quit" => app.exit(0),
        _ => {
            if let Some(i) = id.strip_prefix("hist-").and_then(|n| n.parse::<usize>().ok()) {
                if let Some(r) = sumvox::recent(RECENT_N).into_iter().nth(i) {
                    let _ = app.emit(AGENT_REPORT, r);
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
