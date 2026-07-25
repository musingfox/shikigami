mod cchooks;
mod events;
mod herdr;
mod sumvox;
mod tray;
mod brain;
mod voice;
mod stt;
use tauri::Emitter;
use crate::events::VOICE_LISTENING;
use std::path::{Path, PathBuf};

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

// Orb double-click talk: flip the shared listening state; the VOICE_LISTENING
// event then drives capture start/stop — the exact same path as the PTT hotkey.
#[tauri::command]
fn toggle_listening(app: tauri::AppHandle) -> bool {
    use std::sync::atomic::Ordering;
    let new_state = !voice::LISTENING.fetch_xor(true, Ordering::SeqCst);
    let _ = app.emit(VOICE_LISTENING, new_state);
    new_state
}

// Mic-failure rollback: frontend resets listening here too, otherwise a stale
// LISTENING=true makes shortcut_action swallow the next PTT press-release cycle.
#[tauri::command]
fn set_listening(app: tauri::AppHandle, on: bool) {
    use std::sync::atomic::Ordering;
    voice::LISTENING.store(on, Ordering::SeqCst);
    let _ = app.emit(VOICE_LISTENING, on);
}

// Full voice loop: pcm f32le@16k → whisper STT → transcript event → brain → speak-back.
// Errors carry a stage label ("stt:"/"brain:"/"speak:") for the frontend toast.
// ponytail: pcm crosses IPC as a JSON byte array; switch to InvokeBody::Raw if latency matters
#[tauri::command]
async fn process_utterance(app: tauri::AppHandle, pcm: Vec<u8>) -> Result<(), String> {
    let names = roster_names();
    let transcript =
        tauri::async_runtime::spawn_blocking(move || voice::transcribe_bytes(pcm, &names))
            .await
        .map_err(|e| format!("stt: {e}"))?
        .map_err(|e| format!("stt: {e}"))?;
    let _ = app.emit(events::VOICE_TRANSCRIPT, transcript.clone());
    let reply = voice::reply_to_transcript(&transcript, &herdr::get_roster())
        .await
        .map_err(|e| format!("brain: {e}"))?;
    sumvox::record_to(&sumvox::config_dir(), &reply).map_err(|e| format!("speak: {e}"))?;
    sumvox::spawn_say(&reply).map_err(|e| format!("speak: {e}"))?;
    Ok(())
}

// STT-only step for targeted talk (R2a): no brain, no speak-back — the
// frontend owns the transcript-confirm-inject flow from here.
#[tauri::command]
async fn transcribe_utterance(pcm: Vec<u8>) -> Result<String, String> {
    let names = roster_names();
    tauri::async_runtime::spawn_blocking(move || voice::transcribe_bytes(pcm, &names))
        .await
        .map_err(|e| format!("stt: {e}"))?
        .map_err(|e| format!("stt: {e}"))
}

/// Roster agent names for the STT vocab bias — spoken agent names should
/// survive zh-pinned decoding (e.g. "investment-base").
fn roster_names() -> Vec<String> {
    herdr::get_roster().into_iter().map(|a| a.name).collect()
}

/// Where a spoken project name is looked up.
fn workspace_root() -> PathBuf {
    Path::new(&std::env::var("HOME").unwrap_or_default()).join("workspace")
}

/// A spoken project name → the real directory it names under `root`, or None.
/// Only a bare directory name resolves: anything carrying a path separator or
/// starting with a dot is refused before it touches the filesystem, so a
/// misheard phrase can never address a path outside the workspace. A name that
/// matches a file rather than a directory is not a project either.
fn resolve_project(root: &Path, name: &str) -> Option<PathBuf> {
    let name = name.trim();
    if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
        return None;
    }
    let path = root.join(name);
    path.is_dir().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch workspace laid out like ~/workspace, removed on drop.
    struct Tmp(PathBuf);

    impl Tmp {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("shk-ws-{tag}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Tmp(dir)
        }
        fn dir(&self, name: &str) -> &Self {
            std::fs::create_dir_all(self.0.join(name)).unwrap();
            self
        }
        fn file(&self, name: &str) -> &Self {
            std::fs::write(self.0.join(name), b"x").unwrap();
            self
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    // ProjectResolution
    #[test]
    fn pr1_existing_directory_resolves() {
        let t = Tmp::new("pr1");
        t.dir("cyris");
        assert_eq!(resolve_project(&t.0, "cyris"), Some(t.0.join("cyris")));
    }

    #[test]
    fn pr2_unknown_name_resolves_to_nothing() {
        let t = Tmp::new("pr2");
        assert_eq!(resolve_project(&t.0, "nope"), None);
    }

    #[test]
    fn pr3_a_file_is_not_a_project() {
        let t = Tmp::new("pr3");
        t.file("90day.pptx");
        assert_eq!(resolve_project(&t.0, "90day.pptx"), None);
    }

    #[test]
    fn pr4_path_traversal_and_separators_refused() {
        let t = Tmp::new("pr4");
        t.dir("a/b");
        assert_eq!(resolve_project(&t.0, "../etc"), None);
        assert_eq!(resolve_project(&t.0, "a/b"), None);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![read_file, process_utterance, transcribe_utterance, toggle_mute, get_muted, open_config, quit_app, toggle_listening, set_listening, herdr::get_roster, herdr::focus_agent, herdr::prompt_agent])
        .setup(|app| {
            tray::init(app.handle())?;
            sumvox::spawn_watcher(app.handle().clone());
            herdr::spawn_watcher(app.handle().clone());
            cchooks::spawn_watcher(app.handle().clone());
            std::thread::spawn(stt::warmup); // model load off the first utterance

            // ponytail: macOS only — Linux hotkey = Hyprland bind (M4), Wayland can't self-register
            #[cfg(target_os = "macos")]
            {
                use tauri::Manager;
                use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

                let toggle_shortcut =
                    Shortcut::new(Some(Modifiers::SUPER | Modifiers::CONTROL), Code::KeyS);
                let ptt_shortcut =
                    Shortcut::new(Some(Modifiers::SUPER | Modifiers::CONTROL), Code::KeyM);
                app.handle().plugin(
                    tauri_plugin_global_shortcut::Builder::new()
                        .with_handler(move |app, shortcut, event| {
                            use std::sync::atomic::Ordering;
                            use voice::VoiceShortcut;
                            let action = voice::shortcut_action(
                                shortcut == &ptt_shortcut,
                                event.state(),
                                voice::LISTENING.load(Ordering::SeqCst),
                            );
                            match action {
                                VoiceShortcut::StartListening => {
                                    voice::LISTENING.store(true, Ordering::SeqCst);
                                    let _ = app.emit(VOICE_LISTENING, true);
                                }
                                VoiceShortcut::StopListening => {
                                    voice::LISTENING.store(false, Ordering::SeqCst);
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
