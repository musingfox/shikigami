// Voice loop coordinator (PTT -> mic capture -> stt -> brain -> sumvox say/report).
// For BrainReply contract: exposes reply step.
// ponytail: macOS cfg only for hotkeys; core logic cross.

use crate::brain;

pub async fn reply_to_transcript(transcript: &str) -> Result<String, String> {
    if transcript.trim().is_empty() {
        return Err("empty transcript".to_string());
    }
    brain::ask(transcript).await
}

use tauri_plugin_global_shortcut::ShortcutState;

/// Single source of truth for listening state. Written by the PTT hotkey
/// handler AND the frontend (toggle_listening / set_listening commands) so
/// the two entry points can never desync; every change is broadcast as
/// VOICE_LISTENING and the frontend only mirrors it.
pub static LISTENING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum VoiceShortcut {
    StartListening,
    StopListening,
    Ignore,
    ToggleWindow,
}

/// Pure logic for PttHotkeySignal contract tests + handler.
pub fn shortcut_action(is_ptt: bool, state: ShortcutState, listening: bool) -> VoiceShortcut {
    if is_ptt {
        match state {
            ShortcutState::Pressed => {
                if listening {
                    VoiceShortcut::Ignore
                } else {
                    VoiceShortcut::StartListening
                }
            }
            ShortcutState::Released => {
                if listening {
                    VoiceShortcut::StopListening
                } else {
                    VoiceShortcut::Ignore
                }
            }
        }
    } else {
        if state == ShortcutState::Pressed {
            VoiceShortcut::ToggleWindow
        } else {
            VoiceShortcut::Ignore
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_global_shortcut::ShortcutState;

    #[test]
    fn t1_ptt_pressed_not_listening_starts() {
        assert_eq!(shortcut_action(true, ShortcutState::Pressed, false), VoiceShortcut::StartListening);
    }

    #[test]
    fn t2_ptt_pressed_listening_ignores_repeat() {
        assert_eq!(shortcut_action(true, ShortcutState::Pressed, true), VoiceShortcut::Ignore);
    }

    #[test]
    fn t3_ptt_released_listening_stops() {
        assert_eq!(shortcut_action(true, ShortcutState::Released, true), VoiceShortcut::StopListening);
    }

    #[test]
    fn t4_ptt_released_not_listening_ignores() {
        assert_eq!(shortcut_action(true, ShortcutState::Released, false), VoiceShortcut::Ignore);
    }

    #[test]
    fn t5_toggle_pressed_not_listening_toggles_window() {
        assert_eq!(shortcut_action(false, ShortcutState::Pressed, false), VoiceShortcut::ToggleWindow);
    }
}

pub fn transcribe_bytes(pcm: Vec<u8>) -> Result<String, String> {
    let f = crate::stt::pcm_bytes_to_f32(&pcm)?;
    crate::stt::transcribe(&f)
}
