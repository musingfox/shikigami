mod events;
mod sumvox;
mod tray;
mod brain;
mod voice;
mod stt;
use tauri::Emitter;
use crate::events::VOICE_LISTENING;

// raw bytes for the frontend's WebAudio decode (lip-sync envelope)
#[tauri::command]
fn read_file(path: String) -> Result<tauri::ipc::Response, String> {
    std::fs::read(&path)
        .map(tauri::ipc::Response::new)
        .map_err(|e| e.to_string())
}

// toggle full mute flag (for radial menu + tray sync); returns the *new* state
#[tauri::command]
fn toggle_mute(app: tauri::AppHandle) -> bool {
    let dir = sumvox::config_dir();
    let new_state = sumvox::toggle_muted_in(&dir);
    // tray check item must update immediately; run_on_main_thread per sumvox.rs:66-69 convention
    let ah = app.clone();
    let _ = app.run_on_main_thread(move || crate::tray::refresh(&ah));
    new_state
}

// query current muted synchronously (no reliance on startup emit timing)
#[tauri::command]
fn get_muted() -> bool {
    sumvox::muted_in(&sumvox::config_dir())
}

// open SumVox config dir via Finder; thin wrapper over shared spawn (same plan as tray's open-config)
#[tauri::command]
fn open_config() -> Result<(), String> {
    sumvox::spawn_open_config(&sumvox::config_dir())
}

// quit from the radial menu (parity with tray Quit)
#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

// Full voice loop: pcm f32le@16k → whisper STT → transcript event → brain → speak-back.
// Errors carry a stage label ("stt:"/"brain:"/"speak:") for the frontend toast.
// ponytail: pcm crosses IPC as a JSON byte array; switch to InvokeBody::Raw if latency matters
#[tauri::command]
async fn process_utterance(app: tauri::AppHandle, pcm: Vec<u8>) -> Result<(), String> {
    let transcript = tauri::async_runtime::spawn_blocking(move || voice::transcribe_bytes(pcm))
        .await
        .map_err(|e| format!("stt: {e}"))?
        .map_err(|e| format!("stt: {e}"))?;
    let _ = app.emit(events::VOICE_TRANSCRIPT, transcript.clone());
    let reply = voice::reply_to_transcript(&transcript)
        .await
        .map_err(|e| format!("brain: {e}"))?;
    sumvox::record_to(&sumvox::config_dir(), &reply).map_err(|e| format!("speak: {e}"))?;
    sumvox::spawn_say(&reply).map_err(|e| format!("speak: {e}"))?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![read_file, process_utterance, toggle_mute, get_muted, open_config, quit_app])
        .setup(|app| {
            tray::init(app.handle())?;
            sumvox::spawn_watcher(app.handle().clone());

            // ponytail: macOS only — Linux hotkey = Hyprland bind (M4), Wayland can't self-register
            #[cfg(target_os = "macos")]
            {
                use tauri::Manager;
                use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

                let toggle_shortcut =
                    Shortcut::new(Some(Modifiers::SUPER | Modifiers::CONTROL), Code::KeyS);
                let ptt_shortcut =
                    Shortcut::new(Some(Modifiers::SUPER | Modifiers::CONTROL), Code::KeyM);
                static LISTENING: std::sync::atomic::AtomicBool =
                    std::sync::atomic::AtomicBool::new(false);
                app.handle().plugin(
                    tauri_plugin_global_shortcut::Builder::new()
                        .with_handler(move |app, shortcut, event| {
                            use std::sync::atomic::Ordering;
                            use voice::VoiceShortcut;
                            let action = voice::shortcut_action(
                                shortcut == &ptt_shortcut,
                                event.state(),
                                LISTENING.load(Ordering::SeqCst),
                            );
                            match action {
                                VoiceShortcut::StartListening => {
                                    LISTENING.store(true, Ordering::SeqCst);
                                    let _ = app.emit(VOICE_LISTENING, true);
                                }
                                VoiceShortcut::StopListening => {
                                    LISTENING.store(false, Ordering::SeqCst);
                                    let _ = app.emit(VOICE_LISTENING, false);
                                }
                                VoiceShortcut::ToggleWindow => {
                                    if let Some(w) = app.get_webview_window("main") {
                                        if w.is_visible().unwrap_or(false) {
                                            let _ = w.hide();
                                        } else {
                                            let _ = w.show();
                                        }
                                    }
                                }
                                VoiceShortcut::Ignore => {}
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
