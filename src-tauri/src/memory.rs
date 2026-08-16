use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;

pub const HISTORY_DEPTH: usize = 6;
pub const ROTATE_MAX_BYTES: u64 = 1_048_576;

static APPEND_LOCK: Mutex<()> = Mutex::new(());


#[derive(Serialize)]
struct ActionRow<'a> {
    ts: String,
    verb: &'a str,
    pane: &'a str,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<&'a str>,
}

fn action_row(
    verb: &str,
    pane: &str,
    text: &str,
    project: Option<&str>,
    agent: Option<&str>,
    unix_secs: i64,
) -> String {
    serde_json::to_string(&ActionRow {
        ts: crate::sumvox::rfc3339_utc(unix_secs),
        verb,
        pane,
        text,
        project,
        agent,
    })
    .expect("serializing a memory row cannot fail")
}

fn append_row_in(dir: &Path, row: &str, cap: u64) -> Result<(), String> {
    let _guard = APPEND_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let current = dir.join("memory.jsonl");
    if fs::metadata(&current).map(|m| m.len() >= cap).unwrap_or(false) {
        let previous = dir.join("memory.jsonl.1");
        if previous.exists() {
            fs::remove_file(&previous).map_err(|e| e.to_string())?;
        }
        fs::rename(&current, previous).map_err(|e| e.to_string())?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(current)
        .map_err(|e| e.to_string())?;
    file.write_all(row.as_bytes())
        .and_then(|_| file.write_all(b"\n"))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "shikigami-memory-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }


    #[test]
    fn action_row_encodes_known_fields_and_omits_unknowns() {
        assert_eq!(
            action_row("inject", "%1", "跑測試", None, Some("cyris"), 0),
            r#"{"ts":"1970-01-01T00:00:00Z","verb":"inject","pane":"%1","text":"跑測試","agent":"cyris"}"#
        );
        assert_eq!(
            action_row("summon", "%9", "跑測試", Some("cyris"), None, 0),
            r#"{"ts":"1970-01-01T00:00:00Z","verb":"summon","pane":"%9","text":"跑測試","project":"cyris"}"#
        );
        let row = action_row("inject", "%1", "hi", None, None, 0);
        assert!(!row.contains("agent"));
        assert!(!row.contains("project"));
    }

    #[test]
    fn memory_rows_append_without_rewriting_or_interleaving() {
        let tmp = Tmp::new();
        append_row_in(&tmp.0, r#"{"a":1}"#, ROTATE_MAX_BYTES).unwrap();
        append_row_in(&tmp.0, r#"{"b":2}"#, ROTATE_MAX_BYTES).unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.0.join("memory.jsonl")).unwrap(),
            "{\"a\":1}\n{\"b\":2}\n"
        );
        assert!(!tmp.0.join("memory.jsonl.1").exists());

        let missing = tmp.0.join("nope");
        assert!(append_row_in(&missing, "x", ROTATE_MAX_BYTES).is_err());

        let concurrent = Tmp::new();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| append_row_in(&concurrent.0, "same-width", ROTATE_MAX_BYTES).unwrap());
            }
        });
        let rows = std::fs::read_to_string(concurrent.0.join("memory.jsonl")).unwrap();
        assert_eq!(rows.lines().count(), 8);
        assert!(rows.lines().all(|row| row == "same-width"));
    }

    #[test]
    fn memory_rotation_keeps_one_previous_generation() {
        assert_eq!(ROTATE_MAX_BYTES, 1_048_576);
        let tmp = Tmp::new();
        append_row_in(&tmp.0, "0123456789", 10).unwrap();
        append_row_in(&tmp.0, "x", 10).unwrap();
        assert_eq!(std::fs::read_to_string(tmp.0.join("memory.jsonl")).unwrap(), "x\n");
        assert_eq!(
            std::fs::read_to_string(tmp.0.join("memory.jsonl.1")).unwrap(),
            "0123456789\n"
        );

        append_row_in(&tmp.0, "0123456789", 10).unwrap();
        append_row_in(&tmp.0, "y", 10).unwrap();
        assert_eq!(std::fs::read_to_string(tmp.0.join("memory.jsonl")).unwrap(), "y\n");
        assert_eq!(
            std::fs::read_to_string(tmp.0.join("memory.jsonl.1")).unwrap(),
            "x\n0123456789\n"
        );

        let short = Tmp::new();
        append_row_in(&short.0, r#"{"a":1}"#, ROTATE_MAX_BYTES).unwrap();
        append_row_in(&short.0, r#"{"b":2}"#, ROTATE_MAX_BYTES).unwrap();
        assert!(!short.0.join("memory.jsonl.1").exists());
    }
}
