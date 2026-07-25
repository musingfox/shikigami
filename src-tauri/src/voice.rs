// Voice loop coordinator (PTT -> mic capture -> stt -> brain -> sumvox say/report).
// For BrainReply contract: exposes reply step.
// ponytail: macOS cfg only for hotkeys; core logic cross.

use crate::brain::{self, SummonAction};
use crate::events::AgentEntry;

/// How one utterance ends — the two outcomes are mutually exclusive. A summon
/// proposal is data for the confirm bar, never something to read out, so the
/// brain's JSON can't reach the speaker.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Utterance {
    Spoken { text: String },
    Summon { project: String, task: String, cwd: String },
}

/// Decide an utterance's ending: speech stays speech, and a summon becomes a
/// proposal only once its project resolved to a real directory. An unresolved
/// project is asked about out loud rather than guessed at.
pub fn route(action: SummonAction, cwd: Option<String>) -> Utterance {
    match action {
        SummonAction::Speak(text) => Utterance::Spoken { text },
        SummonAction::Summon { project, task } => match cwd {
            Some(cwd) => Utterance::Summon { project, task, cwd },
            None => Utterance::Spoken {
                text: format!("找不到專案 {project}，要開哪個專案？"),
            },
        },
    }
}

/// Each voice reply is grounded in the roster snapshot passed by the caller at
/// call time, so "誰在工作" names the agents actually working right now.
pub async fn reply_to_transcript(
    transcript: &str,
    roster: &[AgentEntry],
) -> Result<String, String> {
    if transcript.trim().is_empty() {
        return Err("empty transcript".to_string());
    }
    brain::ask(transcript, roster).await
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

    // UtteranceOutcomeRouting
    fn summon_action() -> SummonAction {
        SummonAction::Summon { project: "cyris".into(), task: "跑測試".into() }
    }

    #[test]
    fn uo1_speech_is_spoken_as_is() {
        assert_eq!(
            route(SummonAction::Speak("你好".into()), None),
            Utterance::Spoken { text: "你好".into() }
        );
    }

    #[test]
    fn uo2_resolved_summon_becomes_a_proposal_for_the_frontend() {
        let out = route(summon_action(), Some("/Users/x/workspace/cyris".into()));
        assert_eq!(
            serde_json::to_value(&out).unwrap(),
            serde_json::json!({
                "kind": "summon",
                "project": "cyris",
                "task": "跑測試",
                "cwd": "/Users/x/workspace/cyris"
            })
        );
    }

    #[test]
    fn uo3_unresolved_project_is_asked_about_never_summoned() {
        assert_eq!(
            route(summon_action(), None),
            Utterance::Spoken { text: "找不到專案 cyris，要開哪個專案？".into() }
        );
    }

    #[test]
    fn uo4_spoken_serializes_with_its_kind() {
        let out = Utterance::Spoken { text: "hi".into() };
        assert_eq!(
            serde_json::to_value(&out).unwrap(),
            serde_json::json!({ "kind": "spoken", "text": "hi" })
        );
    }

    // VoiceReplyUsesLiveRoster contract
    #[test]
    fn vr1_empty_transcript_rejected_before_brain() {
        let err = tauri::async_runtime::block_on(reply_to_transcript("", &[])).unwrap_err();
        assert_eq!(err, "empty transcript");
    }

    // T2: live path (needs herdr + API key) — run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn vr2_live_roster_answer_nonempty() {
        let roster = crate::herdr::get_roster();
        let reply =
            tauri::async_runtime::block_on(reply_to_transcript("現在誰在工作", &roster)).unwrap();
        assert!(!reply.trim().is_empty());
    }
}

pub fn transcribe_bytes(pcm: Vec<u8>, vocab: &[String]) -> Result<String, String> {
    let f = crate::stt::pcm_bytes_to_f32(&pcm)?;
    crate::stt::transcribe(&f, vocab)
}
