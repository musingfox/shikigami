mod events;
mod sumvox;
mod tray;
mod brain;
mod voice;
use tauri::Emitter;
use crate::events::VOICE_LISTENING;

// raw bytes for the frontend's WebAudio decode (lip-sync envelope)
#[tauri::command]
fn read_file(path: String) -> Result<tauri::ipc::Response, String> {
    std::fs::read(&path)
        .map(tauri::ipc::Response::new)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn brain_reply(transcript: String) -> Result<String, String> {
    voice::reply_to_transcript(&transcript).await
}

#[tauri::command]
fn process_utterance(pcm: Vec<u8>) -> Result<(), String> {
    // bytes are f32le mono 16k from frontend; consumed by stt later
    if pcm.is_empty() {
        return Err("empty pcm".into());
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![read_file, brain_reply, process_utterance])
        .setup(|app| {
            tray::init(app.handle())?;
            sumvox::spawn_watcher(app.handle().clone());

            // ponytail: macOS only — Linux hotkey = Hyprland bind (M4), Wayland can't self-register
            #[cfg(target_os = "macos")]
            {
                use tauri::Manager;
                use tauri_plugin_global_shortcut::{
                    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
                };

                let toggle_shortcut =
                    Shortcut::new(Some(Modifiers::SUPER | Modifiers::CONTROL), Code::KeyS);
                let ptt_shortcut =
                    Shortcut::new(Some(Modifiers::SUPER | Modifiers::CONTROL), Code::KeyM);
                app.handle().plugin(
                    tauri_plugin_global_shortcut::Builder::new()
                        .with_handler(move |app, shortcut, event| {
                            let st = event.state();
                            if shortcut == &toggle_shortcut {
                                if st == ShortcutState::Pressed {
                                    if let Some(w) = app.get_webview_window("main") {
                                        if w.is_visible().unwrap_or(false) {
                                            let _ = w.hide();
                                        } else {
                                            let _ = w.show();
                                        }
                                    }
                                }
                            } else if shortcut == &ptt_shortcut {
                                let is_pressed = st == ShortcutState::Pressed;
                                let _ = app.emit(VOICE_LISTENING, is_pressed);
                            }
                        })
                        .build(),
                )?;
                app.global_shortcut().register(toggle_shortcut)?;
                app.global_shortcut().register(ptt_shortcut)?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
