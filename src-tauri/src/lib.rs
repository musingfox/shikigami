mod sumvox;

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};

fn toggle_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
        } else {
            let _ = w.show();
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let toggle = MenuItem::with_id(app, "toggle", "Show / Hide", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => toggle_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            sumvox::spawn_watcher(app.handle().clone());

            // ponytail: macOS only — Linux hotkey = Hyprland bind (M4), Wayland can't self-register
            #[cfg(target_os = "macos")]
            {
                use tauri_plugin_global_shortcut::{
                    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
                };

                let toggle_shortcut =
                    Shortcut::new(Some(Modifiers::SUPER | Modifiers::CONTROL), Code::KeyS);
                app.handle().plugin(
                    tauri_plugin_global_shortcut::Builder::new()
                        .with_handler(move |app, shortcut, event| {
                            if shortcut == &toggle_shortcut
                                && event.state() == ShortcutState::Pressed
                            {
                                toggle_main_window(app);
                            }
                        })
                        .build(),
                )?;
                app.global_shortcut().register(toggle_shortcut)?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
