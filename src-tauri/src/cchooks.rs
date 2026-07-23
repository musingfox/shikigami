// Claude Code hooks source (channel ①): precise identity signals.
// scripts/cc-hook.sh appends one NDJSON line per hook firing to
// ~/.config/shikigami/hooks.ndjson; we tail the spool and emit agent:activity.
// Decision (R1b): activity never toasts — SumVox keeps owning agent:report
// (the spoken summary); these events exist for attribution only.
// ponytail: 500ms file poll, same pattern as the sumvox watcher; the spool
// grows unbounded at ~bytes per report — rotate if it ever matters.

use crate::events::{Activity, AGENT_ACTIVITY};
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Duration;
use tauri::Emitter;

pub fn spool_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::Path::new(&home).join(".config/shikigami/hooks.ndjson")
}

/// One spool line → Activity. Malformed lines yield None and are skipped —
/// the spool is written by a shell one-liner and must never wedge the watcher.
pub fn parse_line(line: &str) -> Option<Activity> {
    let v: Value = serde_json::from_str(line).ok()?;
    let kind = v.get("kind")?.as_str()?.to_string();
    let s = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let session = v
        .get("payload")
        .and_then(|p| p.get("session_id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Some(Activity {
        source: "cchooks",
        session,
        pane: s("pane"),
        kind,
        ts: s("ts"),
    })
}

pub fn spawn_watcher(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let path = spool_path();
        // start at EOF: entries predating this run are stale signals
        let mut offset = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let mut pending = String::new(); // holds an incomplete trailing line
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let len = match std::fs::metadata(&path) {
                Ok(m) => m.len(),
                Err(_) => continue, // spool not created yet
            };
            if len < offset {
                // truncated/rotated underneath us
                offset = 0;
                pending.clear();
            }
            if len == offset {
                continue;
            }
            let Ok(mut f) = std::fs::File::open(&path) else { continue };
            if f.seek(SeekFrom::Start(offset)).is_err() {
                continue;
            }
            let mut buf = String::new();
            // invalid UTF-8 (a write caught mid-multibyte-char) → retry next
            // poll without advancing the offset
            if f.read_to_string(&mut buf).is_err() {
                continue;
            }
            offset = len;
            pending.push_str(&buf);
            while let Some(nl) = pending.find('\n') {
                let line: String = pending.drain(..=nl).collect();
                if let Some(act) = parse_line(line.trim()) {
                    let _ = app.emit(AGENT_ACTIVITY, act);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // exact shape scripts/cc-hook.sh produces
    const LIVE_LIKE: &str = r#"{"kind":"stop","pane":"wT:p1","ts":"2026-07-23T10:00:00Z","payload":{"session_id":"abc-123","transcript_path":"/x/y.jsonl","cwd":"/Users/x/proj"}}"#;

    #[test]
    fn t1_parses_full_line() {
        assert_eq!(
            parse_line(LIVE_LIKE),
            Some(Activity {
                source: "cchooks",
                session: "abc-123".into(),
                pane: "wT:p1".into(),
                kind: "stop".into(),
                ts: "2026-07-23T10:00:00Z".into(),
            })
        );
    }

    #[test]
    fn t2_tolerates_missing_optionals() {
        let line = r#"{"kind":"notification","payload":{}}"#;
        let a = parse_line(line).unwrap();
        assert_eq!(a.kind, "notification");
        assert_eq!(a.session, "");
        assert_eq!(a.pane, "");
        assert_eq!(a.ts, "");
    }

    #[test]
    fn t3_rejects_garbage_and_missing_kind() {
        assert_eq!(parse_line("not json"), None);
        assert_eq!(parse_line(""), None);
        assert_eq!(parse_line(r#"{"pane":"x"}"#), None); // no kind
        assert_eq!(parse_line(r#"{"kind":42}"#), None); // kind not a string
    }
}
