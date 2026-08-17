use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;
use crate::events::AgentEntry;


// Every number that bounds what reaches the model lives in `budget.rs`, so the
// four context layers can be read off one page instead of three files.
use crate::budget::{
    CURATED_MAX_CHARS, HISTORY_DEPTH, ROTATE_MAX_BYTES, TURN_MAX_CHARS, TURN_MAX_LINES,
};

pub(crate) fn fit_turn(text: &str) -> String {
    crate::depth::normalize(text, TURN_MAX_LINES, TURN_MAX_CHARS)
}

pub(crate) fn history_snapshot_in(dir: &Path) -> Vec<(String, String)> {
    let mut current = turn_rows_in(&dir.join("memory.jsonl"));
    if current.len() >= HISTORY_DEPTH {
        return current.split_off(current.len() - HISTORY_DEPTH);
    }

    let mut history = turn_rows_in(&dir.join("memory.jsonl.1"));
    history.append(&mut current);
    if history.len() > HISTORY_DEPTH {
        history.drain(..history.len() - HISTORY_DEPTH);
    }
    history
}

fn turn_rows_in(path: &Path) -> Vec<(String, String)> {
    fs::read_to_string(path)
        .map(|text| {
            text.lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .filter(|row| row["verb"] == "turn")
                .filter_map(|row| {
                    Some((
                        fit_turn(row["user"].as_str()?),
                        fit_turn(row["assistant"].as_str()?),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One row of the memory file, whichever layer wrote it, with its own timestamp
/// kept. `history_snapshot_in` deliberately drops both the ts and every fact row
/// — it feeds the rolling layer. This reader is the other half: it keeps every
/// verb and every ts, because a ts is the only handle a later turn has on "which
/// record was this". Fields the row did not carry stay `None`; a fact row has no
/// `user`/`assistant`, a turn row has no `pane`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemoryRow {
    pub ts: String,
    pub verb: String,
    pub text: String,
    pub pane: Option<String>,
    pub agent: Option<String>,
    pub project: Option<String>,
    pub user: Option<String>,
    pub assistant: Option<String>,
}

/// Every row still on disk, oldest generation first. Never fails: a missing file,
/// an unreadable one, or a line that is not JSON contributes nothing and the rest
/// still comes back. Reads both generations, because rotation would otherwise
/// make everything written before it silently unrememberable.
pub(crate) fn memory_rows_in(dir: &Path) -> Vec<MemoryRow> {
    let mut rows = memory_rows_of(&dir.join("memory.jsonl.1"));
    rows.append(&mut memory_rows_of(&dir.join("memory.jsonl")));
    rows
}

fn memory_rows_of(path: &Path) -> Vec<MemoryRow> {
    fs::read_to_string(path)
        .map(|text| text.lines().filter_map(parse_row).collect())
        .unwrap_or_default()
}

fn parse_row(line: &str) -> Option<MemoryRow> {
    let value = serde_json::from_str::<serde_json::Value>(line).ok()?;
    let field = |key: &str| {
        value[key]
            .as_str()
            .map(str::to_string)
            .filter(|text| !text.is_empty())
    };
    Some(MemoryRow {
        ts: field("ts")?,
        verb: field("verb")?,
        text: field("text").unwrap_or_default(),
        pane: field("pane"),
        agent: field("agent"),
        project: field("project"),
        user: field("user"),
        assistant: field("assistant"),
    })
}

static APPEND_LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize)]
struct TurnRow<'a> {
    ts: String,
    verb: &'static str,
    user: &'a str,
    assistant: &'a str,
}

pub(crate) fn append_turn_in(
    dir: &Path,
    user: &str,
    assistant: &str,
    unix_secs: i64,
) -> Result<(), String> {
    let row = serde_json::to_string(&TurnRow {
        ts: crate::sumvox::rfc3339_utc(unix_secs),
        verb: "turn",
        user,
        assistant,
    })
    .expect("serializing a memory row cannot fail");
    append_row_in(dir, &row, ROTATE_MAX_BYTES)
}


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

pub(crate) fn ensure_dir(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())
}

pub(crate) fn curated_in(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join("MEMORY.md")).ok()?;
    let text = text.trim().to_string();
    if text.is_empty() {
        return None;
    }
    if let Some(warning) = curated_warning(text.chars().count()) {
        eprintln!("[memory] {warning}");
    }
    Some(text)
}

fn curated_warning(chars: usize) -> Option<String> {
    (chars > CURATED_MAX_CHARS).then(|| {
        format!(
            "MEMORY.md has {chars} characters; recommended maximum is {CURATED_MAX_CHARS}"
        )
    })
}

fn pane_name<'a>(roster: &'a [AgentEntry], pane: &str) -> Option<&'a str> {
    roster.iter().find(|entry| entry.pane == pane).map(|entry| entry.name.as_str())
}

pub(crate) fn keep_action_result<T>(
    action: Result<T, String>,
    memory: Result<(), String>,
) -> Result<T, String> {
    if let Err(error) = memory {
        eprintln!("[memory] append: {error}");
    }
    action
}

fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub(crate) fn log_inject(roster: &[AgentEntry], pane: &str, text: &str) -> Result<(), String> {
    log_inject_in(&crate::config::config_dir(), roster, pane, text, unix_secs())
}

fn log_inject_in(
    dir: &Path,
    roster: &[AgentEntry],
    pane: &str,
    text: &str,
    unix_secs: i64,
) -> Result<(), String> {
    append_row_in(
        dir,
        &action_row("inject", pane, text, None, pane_name(roster, pane), unix_secs),
        ROTATE_MAX_BYTES,
    )
}

pub(crate) fn log_summon(
    roster: &[AgentEntry],
    pane: &str,
    project: &str,
    text: &str,
) -> Result<(), String> {
    log_summon_in(
        &crate::config::config_dir(),
        roster,
        pane,
        project,
        text,
        unix_secs(),
    )
}

fn log_summon_in(
    dir: &Path,
    roster: &[AgentEntry],
    pane: &str,
    project: &str,
    text: &str,
    unix_secs: i64,
) -> Result<(), String> {
    append_row_in(
        dir,
        &action_row(
            "summon",
            pane,
            text,
            Some(project),
            pane_name(roster, pane),
            unix_secs,
        ),
        ROTATE_MAX_BYTES,
    )
}

fn append_row_in(dir: &Path, row: &str, cap: u64) -> Result<(), String> {
    let _guard = APPEND_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let current = dir.join("memory.jsonl");
    if fs::metadata(&current).map(|m| m.len() >= cap).unwrap_or(false) {
        let previous = dir.join("memory.jsonl.1");
        if previous.exists() {
            if let Err(e) = fs::remove_file(&previous) {
                eprintln!("[memory] rotate: {e}");
            }
        }
        if let Err(e) = fs::rename(&current, &previous) {
            eprintln!("[memory] rotate: {e}");
        }
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(current)
        .map_err(|e| e.to_string())?;
    // Row and terminator in one write: a process that dies mid-append must not
    // leave a line the next append would concatenate onto (both turns unparsable).
    file.write_all(format!("{row}\n").as_bytes())
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


    // MemoryRowReader — the fact layer finally has a reader, and it reads both
    // generations: a row that survived rotation is still a row.
    #[test]
    fn every_row_of_both_generations_comes_back_oldest_first() {
        let tmp = Tmp::new();
        std::fs::write(
            tmp.0.join("memory.jsonl.1"),
            "{\"ts\":\"A\",\"verb\":\"inject\",\"pane\":\"%1\",\"text\":\"跑測試\",\"agent\":\"cyris\"}\n",
        )
        .unwrap();
        std::fs::write(
            tmp.0.join("memory.jsonl"),
            "{\"ts\":\"B\",\"verb\":\"turn\",\"user\":\"u\",\"assistant\":\"a\"}\n",
        )
        .unwrap();

        let rows = memory_rows_in(&tmp.0);
        assert_eq!(
            rows.iter().map(|row| row.ts.as_str()).collect::<Vec<_>>(),
            ["A", "B"]
        );
        assert_eq!(rows[1].user.as_deref(), Some("u"));
        assert_eq!(rows[1].assistant.as_deref(), Some("a"));
        assert_eq!(rows[1].pane, None);
    }

    #[test]
    fn a_broken_line_is_skipped_and_the_rest_of_the_file_still_reads() {
        let tmp = Tmp::new();
        std::fs::write(
            tmp.0.join("memory.jsonl"),
            "{\"ts\":\"A\",\"verb\":\"turn\",\"user\":\"u\",\"assistant\":\"a\"}\n\
             壞掉的一行\n\
             {\"ts\":\"B\",\"verb\":\"summon\",\"pane\":\"%9\",\"text\":\"跑測試\",\"project\":\"cyris\"}\n",
        )
        .unwrap();
        let rows = memory_rows_in(&tmp.0);
        assert_eq!(
            rows.iter().map(|row| row.ts.as_str()).collect::<Vec<_>>(),
            ["A", "B"]
        );
        assert_eq!(rows[1].project.as_deref(), Some("cyris"));
    }

    #[test]
    fn no_memory_file_at_all_is_no_rows_not_a_panic() {
        let tmp = Tmp::new();
        assert!(memory_rows_in(&tmp.0).is_empty());
        assert!(memory_rows_in(&tmp.0.join("missing")).is_empty());
    }

    #[test]
    fn a_fact_row_keeps_its_timestamp_and_its_absent_fields_stay_absent() {
        let tmp = Tmp::new();
        std::fs::write(
            tmp.0.join("memory.jsonl"),
            "{\"ts\":\"A\",\"verb\":\"inject\",\"pane\":\"%1\",\"text\":\"跑測試\",\"agent\":\"cyris\"}\n",
        )
        .unwrap();
        let rows = memory_rows_in(&tmp.0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].ts, "A");
        assert_eq!(rows[0].verb, "inject");
        assert_eq!(rows[0].text, "跑測試");
        assert_eq!(rows[0].agent.as_deref(), Some("cyris"));
        assert_eq!(rows[0].pane.as_deref(), Some("%1"));
        assert_eq!(rows[0].project, None);
        assert_eq!(rows[0].user, None);
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
    fn memory_append_never_leaves_the_file_mid_line() {
        let tmp = Tmp::new();
        let path = tmp.0.join("memory.jsonl");
        append_row_in(&tmp.0, turn_rows(1..=1).trim_end(), ROTATE_MAX_BYTES).unwrap();
        // the terminator lands with its row, never as a second write
        assert!(std::fs::read_to_string(&path).unwrap().ends_with('\n'));

        append_row_in(&tmp.0, turn_rows(2..=2).trim_end(), ROTATE_MAX_BYTES).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().ends_with('\n'));
        assert_eq!(
            history_snapshot_in(&tmp.0),
            vec![("q1".into(), "a1".into()), ("q2".into(), "a2".into())]
        );
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

    #[test]
    fn memory_append_survives_a_failed_rotation() {
        let tmp = Tmp::new();
        std::fs::write(tmp.0.join("memory.jsonl"), "0123456789\n").unwrap();
        let previous = tmp.0.join("memory.jsonl.1");
        std::fs::create_dir(&previous).unwrap();
        std::fs::write(previous.join("blocker"), "x").unwrap();

        assert!(append_row_in(&tmp.0, "x", 10).is_ok());
        let rows = std::fs::read_to_string(tmp.0.join("memory.jsonl")).unwrap();
        assert_eq!(rows, "0123456789\nx\n");
        assert!(previous.is_dir());
    }

    #[test]
    fn memory_directory_creation_is_recursive_and_idempotent() {
        let tmp = Tmp::new();
        let nested = tmp.0.join("a/b/c");
        ensure_dir(&nested).unwrap();
        assert!(nested.is_dir());
        ensure_dir(&nested).unwrap();

        let file = tmp.0.join("f");
        std::fs::write(&file, "x").unwrap();
        assert!(ensure_dir(&file.join("x")).is_err());
    }

    #[test]
    fn pane_name_uses_only_exact_roster_matches() {
        let roster = [crate::events::AgentEntry {
            id: "1".into(),
            name: "cyris".into(),
            pane: "%1".into(),
            status: "working".into(),
            title: String::new(),
            cwd: String::new(),
        }];
        assert_eq!(pane_name(&roster, "%1"), Some("cyris"));
        assert_eq!(pane_name(&roster, "%9"), None);
        assert_eq!(pane_name(&[], "%1"), None);
    }

    #[test]
    fn successful_injection_writes_exact_confirmed_fact() {
        let tmp = Tmp::new();
        let roster = [crate::events::AgentEntry {
            id: "1".into(),
            name: "cyris".into(),
            pane: "%1".into(),
            status: "working".into(),
            title: String::new(),
            cwd: String::new(),
        }];
        log_inject_in(&tmp.0, &roster, "%1", "跑測試", 0).unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.0.join("memory.jsonl")).unwrap(),
            "{\"ts\":\"1970-01-01T00:00:00Z\",\"verb\":\"inject\",\"pane\":\"%1\",\"text\":\"跑測試\",\"agent\":\"cyris\"}\n"
        );

        // Unknown pane: the name is dropped, every other field stays put.
        let empty = Tmp::new();
        log_inject_in(&empty.0, &[], "%1", "跑測試", 0).unwrap();
        let row = std::fs::read_to_string(empty.0.join("memory.jsonl")).unwrap();
        assert!(!row.contains("agent"));
        assert_eq!(
            row,
            "{\"ts\":\"1970-01-01T00:00:00Z\",\"verb\":\"inject\",\"pane\":\"%1\",\"text\":\"跑測試\"}\n"
        );
    }

    #[test]
    fn memory_failure_never_changes_action_result() {
        assert_eq!(
            keep_action_result(Ok("%9"), Err("boom".to_string())),
            Ok("%9")
        );
        assert_eq!(
            keep_action_result::<&str>(Err("herdr down".to_string()), Ok(())),
            Err("herdr down".to_string())
        );
        let tmp = Tmp::new();
        assert!(log_inject_in(&tmp.0.join("missing"), &[], "%1", "x", 0).is_err());
    }

    #[test]
    fn successful_summon_writes_project_task_pane_and_known_agent() {
        let empty = Tmp::new();
        log_summon_in(&empty.0, &[], "%9", "cyris", "跑測試", 0).unwrap();
        assert_eq!(
            std::fs::read_to_string(empty.0.join("memory.jsonl")).unwrap(),
            "{\"ts\":\"1970-01-01T00:00:00Z\",\"verb\":\"summon\",\"pane\":\"%9\",\"text\":\"跑測試\",\"project\":\"cyris\"}\n"
        );

        let known = Tmp::new();
        let roster = [crate::events::AgentEntry {
            id: "2".into(),
            name: "cyris-2".into(),
            pane: "%9".into(),
            status: "working".into(),
            title: String::new(),
            cwd: String::new(),
        }];
        log_summon_in(&known.0, &roster, "%9", "cyris", "跑測試", 0).unwrap();
        let row = std::fs::read_to_string(known.0.join("memory.jsonl")).unwrap();
        assert!(row.contains(r#""project":"cyris""#));
        assert!(row.contains(r#""agent":"cyris-2""#));
    }

    #[test]
    fn absent_unreadable_or_blank_curated_memory_is_ignored() {
        let tmp = Tmp::new();
        assert_eq!(curated_in(&tmp.0), None);
        std::fs::create_dir(tmp.0.join("MEMORY.md")).unwrap();
        assert_eq!(curated_in(&tmp.0), None);

        let blank = Tmp::new();
        std::fs::write(blank.0.join("MEMORY.md"), "   \n\n").unwrap();
        assert_eq!(curated_in(&blank.0), None);
    }

    #[test]
    fn curated_memory_is_trimmed_but_preserved() {
        let tmp = Tmp::new();
        std::fs::write(tmp.0.join("MEMORY.md"), "記得我用 fish shell\n").unwrap();
        assert_eq!(curated_in(&tmp.0), Some("記得我用 fish shell".to_string()));
    }

    #[test]
    fn oversized_curated_memory_warns_without_truncating() {
        let warning = curated_warning(CURATED_MAX_CHARS + 1).unwrap();
        assert!(warning.contains("MEMORY.md"));
        assert!(warning.contains("4000"));
        assert_eq!(curated_warning(CURATED_MAX_CHARS), None);

        let tmp = Tmp::new();
        let content = "記".repeat(CURATED_MAX_CHARS + 1);
        std::fs::write(tmp.0.join("MEMORY.md"), &content).unwrap();
        let loaded = curated_in(&tmp.0).unwrap();
        assert_eq!(loaded.chars().count(), 4001);
        assert_eq!(loaded, content);
        assert!(!loaded.contains('…'));
    }

    #[test]
    fn history_turns_keep_only_bounded_unicode_tail() {
        let long = "a".repeat(500);
        let fitted = fit_turn(&long);
        assert_eq!(fitted.chars().count(), 401);
        assert!(fitted.starts_with('…'));
        assert_eq!(fitted.chars().skip(1).collect::<String>(), "a".repeat(400));
        assert_eq!(fit_turn("你好"), "你好");
        assert_eq!(fit_turn(&"界".repeat(400)), "界".repeat(400));

        let lines = (1..=12)
            .map(|i| format!("{i:03}"))
            .collect::<Vec<_>>()
            .join("\n");
        let fitted = fit_turn(&lines);
        assert!(fitted.starts_with('…'));
        assert_eq!(fitted.lines().count(), 8);
        assert!(fitted.ends_with("012"));

        assert_eq!(HISTORY_DEPTH, 6);
        assert_eq!(TURN_MAX_LINES, 8);
        assert_eq!(TURN_MAX_CHARS, 400);
        assert_eq!(CURATED_MAX_CHARS, 4000);
        assert_eq!(ROTATE_MAX_BYTES, 1_048_576);
    }

    fn turn_rows(range: std::ops::RangeInclusive<usize>) -> String {
        range
            .map(|i| format!(r#"{{"ts":"..","verb":"turn","user":"q{i}","assistant":"a{i}"}}"#))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }

    #[test]
    fn conversation_restores_six_recent_valid_turns_across_rotation() {
        let fresh = Tmp::new();
        assert!(history_snapshot_in(&fresh.0).is_empty());

        let eight = Tmp::new();
        std::fs::write(eight.0.join("memory.jsonl"), turn_rows(1..=8)).unwrap();
        let history = history_snapshot_in(&eight.0);
        assert_eq!(history.len(), 6);
        assert_eq!(history.first().unwrap(), &("q3".into(), "a3".into()));
        assert_eq!(history.last().unwrap(), &("q8".into(), "a8".into()));

        let mixed = Tmp::new();
        std::fs::write(
            mixed.0.join("memory.jsonl"),
            "garbage\n\n{\"ts\":\"..\",\"verb\":\"turn\",\"user\":\"q\",\"assistant\":\"a\"}\n",
        )
        .unwrap();
        assert_eq!(history_snapshot_in(&mixed.0), vec![("q".into(), "a".into())]);

        let actions = Tmp::new();
        std::fs::write(
            actions.0.join("memory.jsonl"),
            "{\"verb\":\"inject\"}\n{\"verb\":\"summon\"}\n",
        )
        .unwrap();
        assert!(history_snapshot_in(&actions.0).is_empty());

        let rotated = Tmp::new();
        std::fs::write(rotated.0.join("memory.jsonl.1"), turn_rows(1..=10)).unwrap();
        std::fs::write(
            rotated.0.join("memory.jsonl"),
            format!(
                "{}{{\"verb\":\"inject\",\"text\":\"x\"}}\n{}",
                turn_rows(11..=11),
                turn_rows(12..=12)
            ),
        )
        .unwrap();
        assert_eq!(
            history_snapshot_in(&rotated.0),
            (7..=12)
                .map(|i| (format!("q{i}"), format!("a{i}")))
                .collect::<Vec<_>>()
        );

        let enough = Tmp::new();
        std::fs::write(enough.0.join("memory.jsonl"), turn_rows(1..=8)).unwrap();
        std::fs::write(enough.0.join("memory.jsonl.1"), "not json at all").unwrap();
        assert_eq!(history_snapshot_in(&enough.0).first().unwrap().0, "q3");
    }
}
