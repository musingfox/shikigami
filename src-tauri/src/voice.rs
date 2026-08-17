// Voice loop coordinator (PTT -> mic capture -> stt -> brain -> sumvox say/report).
// For BrainReply contract: exposes reply step.
// ponytail: macOS cfg only for hotkeys; core logic cross.

use crate::brain::{self, SummonAction};
use crate::events::AgentEntry;
use crate::depth::AgentDepth;

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
/// the start of the utterance. The brain already decided what the turn amounts
/// to, so the action comes back as an action — nothing re-serialises it into
/// text for the caller to re-parse.
pub async fn reply_to_transcript(
    transcript: &str,
    roster: &[AgentEntry],
    depth: &[AgentDepth],
) -> Result<SummonAction, String> {
    if transcript.trim().is_empty() {
        return Err("empty transcript".to_string());
    }
    brain::ask(transcript, roster, depth).await
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

    // VoiceDepthPlumbing contract
    #[test]
    fn vdp_t1_empty_transcript_rejected_before_brain() {
        let depth = [AgentDepth {
            pane: "%1".into(),
            precise: None,
            screen: Some("must not trigger a brain request".into()),
        }];
        let err =
            tauri::async_runtime::block_on(reply_to_transcript("   ", &[], &depth)).unwrap_err();
        assert_eq!(err, "empty transcript");
    }

    #[test]
    #[ignore]
    fn vdp_t2_live_depth_reaches_single_brain_request() {
        // NOT get_roster(): that reads the cache poll_once fills, and this test
        // binary never runs the polling thread, so it would always be empty and
        // this receipt would never be collectable.
        let roster = crate::herdr::fetch_roster_now().expect("live herdr agent.list");
        // Same reason, for the other half: HOOK_DEPTHS is filled by the spool
        // tailer, which this binary never starts, so read the real spool here.
        // Until the screen prefetch was removed this test could pass on the screen
        // half alone — which is exactly why R-observe's hook half had no receipt.
        // Now the join can only be proven by a hook line, so this is that receipt.
        //
        // HOOK_DEPTHS is process-global and cchooks' own tests assert on it, so
        // take their lock: without it, `cargo test -- --include-ignored` lets this
        // test's 64-odd real spool entries land inside one of theirs. The guard is
        // built before anything is written, so a panic on the spool read cannot
        // leave the global dirty. It **clears** rather than preserving what was
        // there — safe only because the lock serializes access and every other
        // reader clears first too.
        let _lock = crate::cchooks::DEPTH_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        struct Restore;
        impl Drop for Restore {
            fn drop(&mut self) {
                crate::cchooks::clear_depths();
            }
        }
        let _restore = Restore;
        crate::cchooks::clear_depths();
        let spool = std::fs::read_to_string(crate::cchooks::spool_path())
            .expect("live test needs the real hooks.ndjson spool");
        // Keep what the spool file itself said, parsed straight from disk. This is
        // the only independent half of the receipt: AgentDepth.pane is copied from
        // the roster row, so printing it next to the roster's own pane id would be
        // the same value twice under two labels.
        let mut hook_panes: Vec<String> = Vec::new();
        for line in spool.lines() {
            if let Some(depth) = crate::cchooks::parse_depth(line) {
                if !hook_panes.contains(&depth.pane) {
                    hook_panes.push(depth.pane.clone());
                }
            }
            crate::cchooks::record_depth(line);
        }
        let depth = crate::depth::collect(&roster);
        let observed = roster
            .iter()
            .find(|agent| {
                depth.iter().any(|item| item.pane == agent.pane)
                    && matches!(agent.status.as_str(), "blocked" | "working")
            })
            .expect("live test needs one blocked or working agent carrying a hook line");
        let matched = depth
            .iter()
            .find(|item| item.pane == observed.pane)
            .expect("live depth must join by the exact herdr pane_id");
        // The hook half, on its own: herdr's pane_id really is the hook's
        // HERDR_PANE_ID, with no screen excerpt propping the join up.
        assert!(matched.precise.is_some());
        assert!(matched.screen.is_none());

        // The join really is proven by `matched.precise.is_some()` above —
        // `precise_for(&agent.pane)` can only answer for a pane a spool line
        // carried. What is printed has to show that independently, so: the roster's
        // pane id, the pane ids the spool file itself carried, and the hook's own
        // words that ended up attached to that agent.
        assert!(hook_panes.contains(&observed.pane));
        let prompt = crate::brain::system_prompt(&roster, &depth, None, &[]);
        println!(
            "join receipt: herdr said pane_id={}, and that value is among the {} \
             HERDR_PANE_ID values the spool file itself carried\n\
             hook's own words now attached to that agent: {:?}\n\nsystem prompt:\n{}",
            observed.pane,
            hook_panes.len(),
            matched.precise.as_ref().map(|p| &p.detail),
            prompt
        );
        assert!(prompt.contains("精確訊號") || prompt.contains("畫面節錄"));

        let outcome = tauri::async_runtime::block_on(reply_to_transcript(
            "它卡在什麼？",
            &roster,
            &depth,
        ))
        .unwrap();
        let SummonAction::Speak(reply) = outcome else {
            panic!("a 卡在什麼 question must be answered, not summoned: {outcome:?}");
        };
        assert!(!reply.trim().is_empty());
    }
}

pub fn transcribe_bytes(pcm: Vec<u8>, vocab: &[String]) -> Result<String, String> {
    let f = crate::stt::pcm_bytes_to_f32(&pcm)?;
    crate::stt::transcribe(&f, vocab)
}
