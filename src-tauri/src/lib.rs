mod cchooks;
mod config;
mod events;
mod herdr;
mod sumvox;
mod memory;
mod tray;
mod brain;
mod voice;
mod stt;
mod depth;
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

// Full voice loop: pcm f32le@16k → whisper STT → transcript event → brain →
// routed outcome. A spoken reply is said out loud here; a summon proposal is
// returned for the frontend to confirm, and deliberately never spoken.
// Errors carry a stage label ("stt:"/"brain:"/"speak:") for the frontend toast.
// ponytail: pcm crosses IPC as a JSON byte array; switch to InvokeBody::Raw if latency matters
#[tauri::command]
async fn process_utterance(
    app: tauri::AppHandle,
    pcm: Vec<u8>,
) -> Result<voice::Utterance, String> {
    let names = stt_vocab_now();
    let transcript =
        tauri::async_runtime::spawn_blocking(move || voice::transcribe_bytes(pcm, &names))
            .await
        .map_err(|e| format!("stt: {e}"))?
        .map_err(|e| format!("stt: {e}"))?;
    let _ = app.emit(events::VOICE_TRANSCRIPT, transcript.clone());
    // Roster + depth are blocking herdr socket calls (up to 3 pane reads at
    // CALL_TIMEOUT each), so they go off the executor exactly like STT above.
    let asked = transcript.clone();
    let (roster, depth) = tauri::async_runtime::spawn_blocking(move || {
        depth::roster_and_depth_with(&asked, herdr::get_roster, depth::collect)
    })
    .await
    .map_err(|e| format!("brain: {e}"))?;
    let reply = voice::reply_to_transcript(&transcript, &roster, &depth)
        .await
        .map_err(|e| format!("brain: {e}"))?;
    let action = brain::parse_action(&reply);
    let cwd = match &action {
        brain::SummonAction::Summon { project, .. } => resolve_project(&workspace_root(), project)
            .map(|p| p.to_string_lossy().into_owned()),
        brain::SummonAction::Speak(_) => None,
    };
    let outcome = voice::route(action, cwd);
    if let voice::Utterance::Spoken { text } = &outcome {
        sumvox::record_to(&sumvox::config_dir(), text).map_err(|e| format!("speak: {e}"))?;
        sumvox::spawn_say(text).map_err(|e| format!("speak: {e}"))?;
    }
    Ok(outcome)
}

// STT-only step for targeted talk (R2a): no brain, no speak-back — the
// frontend owns the transcript-confirm-inject flow from here.
#[tauri::command]
async fn transcribe_utterance(pcm: Vec<u8>) -> Result<String, String> {
    let names = stt_vocab_now();
    tauri::async_runtime::spawn_blocking(move || voice::transcribe_bytes(pcm, &names))
        .await
        .map_err(|e| format!("stt: {e}"))?
        .map_err(|e| format!("stt: {e}"))
}

// The confirmed half of a summon proposal: the frontend confirm bar hands back
// exactly what process_utterance proposed. Blocking (herdr brings claude up in
// the new pane before returning), so it runs off the IPC thread. Answers with
// the new pane id, which the toast layer uses to point at the summoned agent.
#[tauri::command]
async fn summon_agent(project: String, task: String, cwd: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || herdr::summon(&project, &task, &cwd))
        .await
        .map_err(|e| format!("summon: {e}"))?
}

#[tauri::command]
fn prompt_agent(pane: String, text: String) -> Result<(), String> {
    herdr::prompt_agent(pane.clone(), text.clone())?;
    if let Err(error) = memory::log_inject(&herdr::get_roster(), &pane, &text) {
        eprintln!("[memory] log inject: {error}");
    }
    Ok(())
}

/// Roster agent names for the STT vocab bias — spoken agent names should
/// survive zh-pinned decoding (e.g. "investment-base").
fn roster_names() -> Vec<String> {
    herdr::get_roster().into_iter().map(|a| a.name).collect()
}

/// Project directory names under `root`, sorted. Files and dotted entries are
/// not projects; an unreadable root simply has none.
fn project_names_in(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    names
}

/// What whisper is primed with: agent names plus project names, each once.
/// Project names need the same bias agent names do — "cyris" spoken into a
/// zh-pinned decoder comes back as homophone soup without it.
fn stt_vocab(roster: &[String], projects: &[String]) -> Vec<String> {
    let mut vocab: Vec<String> = Vec::new();
    for name in roster.iter().chain(projects) {
        let name = name.trim();
        if !name.is_empty() && !vocab.iter().any(|seen| seen == name) {
            vocab.push(name.to_string());
        }
    }
    vocab
}

/// The live vocab for one utterance: whoever is on the roster right now, plus
/// every project that could be summoned.
fn stt_vocab_now() -> Vec<String> {
    stt_vocab(&roster_names(), &project_names_in(&workspace_root()))
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![read_file, process_utterance, transcribe_utterance, toggle_mute, get_muted, open_config, quit_app, toggle_listening, set_listening, summon_agent, prompt_agent, herdr::get_roster, herdr::focus_agent])
        .setup(|app| {
            if let Err(error) = memory::ensure_dir(&config::config_dir()) {
                eprintln!("[memory] create config dir: {error}");
            }
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

    // SttProjectVocab
    #[test]
    fn sv1_only_visible_directories_are_projects() {
        let t = Tmp::new("sv1");
        t.dir("cyris").dir("heartwood").dir(".git").file("90day.pptx");
        assert_eq!(project_names_in(&t.0), vec!["cyris", "heartwood"]);
    }

    #[test]
    fn sv2_missing_root_has_no_projects() {
        let missing = std::env::temp_dir().join("shk-ws-does-not-exist");
        let _ = std::fs::remove_dir_all(&missing);
        assert!(project_names_in(&missing).is_empty());
    }

    #[test]
    fn sv3_vocab_merges_roster_and_projects_without_repeats() {
        let roster = vec!["builder".to_string()];
        let projects = vec!["cyris".to_string(), "builder".to_string()];
        let vocab = stt_vocab(&roster, &projects);
        assert_eq!(vocab.iter().filter(|n| *n == "builder").count(), 1);
        assert_eq!(vocab.iter().filter(|n| *n == "cyris").count(), 1);
        assert_eq!(vocab.len(), 2);
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
