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

/// Guardrail 1 of the curated writer: a write may only name a source that is
/// really on disk. The ts set comes from `memory_rows_in`, so "does this record
/// exist" is answered by the file, never by the model's own claim — the same
/// shape as `PaneRead::consulted`, where the tool reports what it actually
/// reached rather than what it says it reached. This brain has twice been caught
/// naming a source it never had; a prompt rule would be a third chance to.
///
/// Refusing is not an error: the reason goes back to the model as that call's
/// result, so it can correct itself inside the same turn.
pub(crate) fn curated_source_ok(known_ts: &[String], text: &str, source: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("沒有內容可以記：text 是空的。".to_string());
    }
    let source = source.trim();
    if source.is_empty() {
        return Err(
            "每一條長期記憶都要有來源：source 是空的。先用 recall 查出那一列開頭的時間戳，再把它逐字填進 source。"
                .to_string(),
        );
    }
    if !known_ts.iter().any(|ts| ts == source) {
        return Err(format!(
            "查不到來源 {source}：長期記憶裡沒有這個時間戳的紀錄，所以這一條沒有寫進去。先用 recall 查出那一列開頭的時間戳，再把它逐字填進 source。"
        ));
    }
    Ok(())
}

/// What every model-written line of `MEMORY.md` starts with. It is both the
/// audit mark a human reads and the only handle the prompt has on authorship, so
/// it is checked per line rather than per block: under append-only the user's
/// later edits land *after* the model's lines, and a block heading would read
/// their words as the model's.
pub(crate) const MODEL_LINE_PREFIX: &str = "- 式神 ";

/// Guardrail 2: one model write, as one auditable line — who wrote it, when, and
/// which record it was drawn from. Newlines are flattened to spaces because a
/// second line would carry no mark and would then be injected as the user's own
/// words.
pub(crate) fn curated_line(text: &str, source: &str, now_ts: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    format!(
        "{MODEL_LINE_PREFIX}{now_ts}（來源 {}）：{flat}",
        source.trim()
    )
}

/// Guardrail 3: the curated layer has a ceiling, and reaching it is loud. A write
/// that would carry the file past `CURATED_MAX_CHARS` is refused whole — never
/// trimmed to fit, because under append-only trimming means rewriting lines that
/// may be the user's own. Counted on the raw file, so an existing file the user
/// pushed over the ceiling themselves simply admits no more model writes.
pub(crate) fn curated_cap_ok(existing: &str, line: &str) -> Result<(), String> {
    let have = existing.chars().count();
    if have + line.chars().count() <= CURATED_MAX_CHARS {
        return Ok(());
    }
    Err(format!(
        "長期記憶已滿，這一條沒有寫進去：MEMORY.md 目前 {have} 字元，加上這一行會超過上限 {CURATED_MAX_CHARS} 字元。請使用者自己整併 MEMORY.md，我不會改動既有內容。"
    ))
}

/// Its own lock: `APPEND_LOCK` bounds `memory.jsonl`, a different file with a
/// different rotation rule.
static CURATED_LOCK: Mutex<()> = Mutex::new(());

/// Guardrail 4: the model may only ever add to the end of `MEMORY.md`. There is
/// no code path in this app that rewrites or truncates what is already in that
/// file, so whatever the user hand-wrote survives every model write by
/// construction rather than by a block parser being correct — the file before a
/// write is always a byte prefix of the file after it.
///
/// The one subtlety is the separator: a hand-edited file often ends without a
/// newline, and appending straight onto it would glue the marker into the middle
/// of the user's last line. That line would then carry no marker of its own, so
/// the model's words would be injected as the user's trusted ones — the exact
/// direction the trust split must never fail in. Writing the missing newline
/// first is still pure append.
pub(crate) fn curated_append_in(dir: &Path, line: &str) -> Result<(), String> {
    let _guard = CURATED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = dir.join("MEMORY.md");
    let joiner = match fs::read_to_string(&path) {
        Ok(text) if !text.is_empty() && !text.ends_with('\n') => "\n",
        _ => "",
    };
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    // Line and terminator in one write, same reason as `append_row_in`.
    file.write_all(format!("{joiner}{line}\n").as_bytes())
        .map_err(|e| e.to_string())
}

/// One model write of the curated layer, end to end: the four guardrails in
/// order, against the file and the rows actually on disk. `Ok` carries the line
/// that really landed; `Err` carries the reason to hand back to the model, which
/// is a refusal far more often than it is a failure. Either way the caller can
/// tell the two apart — nothing was written unless this returns `Ok`.
pub(crate) fn curated_write_in(
    dir: &Path,
    text: &str,
    source: &str,
    now_ts: &str,
) -> Result<String, String> {
    let existing = fs::read_to_string(dir.join("MEMORY.md")).unwrap_or_default();
    let known: Vec<String> = memory_rows_in(dir).into_iter().map(|row| row.ts).collect();
    curated_source_ok(&known, text, source)?;
    let line = curated_line(text, source, now_ts);
    curated_cap_ok(&existing, &line)?;
    curated_append_in(dir, &line)?;
    Ok(line)
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

    // CuratedWriteSourceCheck — guardrail 1. A source the disk cannot confirm is
    // not a source, however confidently the model names it.
    fn known() -> Vec<String> {
        vec!["2026-08-16T09:12:03Z".to_string()]
    }

    #[test]
    fn a_write_naming_a_row_that_really_exists_is_accepted() {
        assert_eq!(
            curated_source_ok(&known(), "使用者偏好 rebase", "2026-08-16T09:12:03Z"),
            Ok(())
        );
    }

    #[test]
    fn a_source_no_row_carries_is_refused_and_points_at_recall() {
        let reason = curated_source_ok(&known(), "使用者偏好 rebase", "2020-01-01T00:00:00Z")
            .unwrap_err();
        assert!(reason.contains("2020-01-01T00:00:00Z"));
        assert!(reason.contains("查不到"));
        assert!(reason.contains("recall"));
    }

    #[test]
    fn a_blank_source_is_refused_before_anything_is_looked_up() {
        let reason = curated_source_ok(&known(), "使用者偏好 rebase", "   ").unwrap_err();
        assert!(reason.contains("source"));
        assert!(reason.contains("recall"));
    }

    // Nothing on disk means nothing citable, so the guardrail is structural: with
    // an empty memory no write can get through at all.
    #[test]
    fn with_no_rows_at_all_no_write_can_get_through() {
        assert!(curated_source_ok(&[], "使用者偏好 rebase", "2026-08-16T09:12:03Z").is_err());
        assert!(curated_source_ok(&[], "使用者偏好 rebase", "隨便一個字串").is_err());
    }

    #[test]
    fn blank_content_is_not_worth_writing_however_good_the_source_is() {
        assert!(curated_source_ok(&known(), "   ", "2026-08-16T09:12:03Z").is_err());
    }

    // CuratedWriteAuditLine — guardrail 2. Opening MEMORY.md tells you at a
    // glance which lines are not your own.
    #[test]
    fn a_model_written_line_says_who_when_and_from_which_record() {
        assert_eq!(
            curated_line(
                "使用者偏好 rebase",
                "2026-08-16T09:12:03Z",
                "2026-08-17T05:00:00Z"
            ),
            "- 式神 2026-08-17T05:00:00Z（來源 2026-08-16T09:12:03Z）：使用者偏好 rebase"
        );
    }

    #[test]
    fn a_model_written_line_carries_the_marker_the_prompt_splits_on() {
        let line = curated_line(
            "使用者偏好 rebase",
            "2026-08-16T09:12:03Z",
            "2026-08-17T05:00:00Z",
        );
        assert!(line.starts_with(MODEL_LINE_PREFIX));
        assert_eq!(MODEL_LINE_PREFIX, "- 式神 ");
    }

    // A second line would carry no marker, so it would be injected as the user's
    // own trusted words — the one way a model write could escape its mark.
    #[test]
    fn a_multi_line_write_stays_one_marked_line() {
        let line = curated_line("第一行\n第二行", "T1", "T2");
        assert_eq!(line.lines().count(), 1);
        assert_eq!(line, "- 式神 T2（來源 T1）：第一行 第二行");
    }

    // CuratedWriteCap — guardrail 3. The file never grows silently, and nothing
    // already in it is ever cut to make room.
    //
    // The production write path lands with CuratedAppendOnly; this composes the
    // three pure guardrails with a plain test append so the "a refusal leaves the
    // file byte-identical" half has a receipt here too.
    fn plan_and_write(dir: &Path, known: &[String], text: &str, source: &str) -> Result<String, String> {
        let path = dir.join("MEMORY.md");
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        curated_source_ok(known, text, source)?;
        let line = curated_line(text, source, "2026-08-17T05:00:00Z");
        curated_cap_ok(&existing, &line)?;
        std::fs::write(&path, format!("{existing}{line}\n")).unwrap();
        Ok(line)
    }

    #[test]
    fn a_write_that_would_pass_the_ceiling_is_refused_naming_both_numbers() {
        let existing = "記".repeat(3990);
        let line = "字".repeat(50);
        let reason = curated_cap_ok(&existing, &line).unwrap_err();
        assert_eq!(CURATED_MAX_CHARS, 4000);
        assert!(reason.contains("4000"));
        assert!(reason.contains("3990"));
        assert!(reason.contains("整併"));
    }

    #[test]
    fn a_write_with_room_left_is_accepted() {
        assert_eq!(
            curated_cap_ok(&"記".repeat(100), &"字".repeat(50)),
            Ok(())
        );
    }

    // The read path only warns, so the user can put the file over the ceiling
    // themselves. From then on no model write fits — and their words stay put.
    #[test]
    fn an_already_oversized_file_admits_no_write_and_loses_nothing() {
        let tmp = Tmp::new();
        let theirs = "記".repeat(5000);
        std::fs::write(tmp.0.join("MEMORY.md"), &theirs).unwrap();
        let reason = plan_and_write(&tmp.0, &known(), "使用者偏好 rebase", "2026-08-16T09:12:03Z")
            .unwrap_err();
        assert!(reason.contains("5000"));
        assert!(reason.contains("4000"));
        assert_eq!(
            std::fs::read_to_string(tmp.0.join("MEMORY.md")).unwrap(),
            theirs
        );
        assert_eq!(curated_in(&tmp.0).unwrap().chars().count(), 5000);
    }

    #[test]
    fn a_refused_write_leaves_not_half_a_line_behind() {
        let tmp = Tmp::new();
        let before = format!("{}\n", "記".repeat(3990));
        std::fs::write(tmp.0.join("MEMORY.md"), &before).unwrap();
        assert!(plan_and_write(&tmp.0, &known(), &"字".repeat(50), "2026-08-16T09:12:03Z").is_err());
        assert_eq!(
            std::fs::read_to_string(tmp.0.join("MEMORY.md")).unwrap(),
            before
        );

        // and a write that does fit really does land, so the refusal above is the
        // cap talking and not a helper that never writes
        let small = Tmp::new();
        std::fs::write(small.0.join("MEMORY.md"), "使用者的話\n").unwrap();
        let line = plan_and_write(&small.0, &known(), "使用者偏好 rebase", "2026-08-16T09:12:03Z")
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(small.0.join("MEMORY.md")).unwrap(),
            format!("使用者的話\n{line}\n")
        );
    }

    // CuratedAppendOnly — guardrail 4. Whatever the user wrote is still there,
    // byte for byte, after the model has written.
    #[test]
    fn a_model_write_lands_after_the_users_words_never_over_them() {
        let tmp = Tmp::new();
        let theirs = "使用者的話\n";
        std::fs::write(tmp.0.join("MEMORY.md"), theirs).unwrap();
        let line = curated_line("使用者偏好 rebase", "T1", "T2");
        curated_append_in(&tmp.0, &line).unwrap();

        let after = std::fs::read_to_string(tmp.0.join("MEMORY.md")).unwrap();
        assert_eq!(after, format!("{theirs}{line}\n"));
        assert!(after.as_bytes().starts_with(theirs.as_bytes()));
    }

    #[test]
    fn the_first_write_creates_the_file() {
        let tmp = Tmp::new();
        let line = curated_line("使用者偏好 rebase", "T1", "T2");
        curated_append_in(&tmp.0, &line).unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.0.join("MEMORY.md")).unwrap(),
            format!("{line}\n")
        );
    }

    #[test]
    fn each_write_keeps_every_earlier_one_as_a_prefix() {
        let tmp = Tmp::new();
        curated_append_in(&tmp.0, "L1").unwrap();
        let first = std::fs::read_to_string(tmp.0.join("MEMORY.md")).unwrap();
        curated_append_in(&tmp.0, "L2").unwrap();
        let second = std::fs::read_to_string(tmp.0.join("MEMORY.md")).unwrap();
        assert_eq!(second, "L1\nL2\n");
        assert!(second.as_bytes().starts_with(first.as_bytes()));
    }

    // A hand-edited file with no closing newline is ordinary. If the appended
    // marker landed mid-line it would lose its mark and be injected as the user's
    // own words — so the separator goes in first, still as an append.
    #[test]
    fn a_file_that_ends_without_a_newline_does_not_swallow_the_marker() {
        let tmp = Tmp::new();
        let theirs = "使用者的話（沒有換行結尾）";
        std::fs::write(tmp.0.join("MEMORY.md"), theirs).unwrap();
        let line = curated_line("使用者偏好 rebase", "T1", "T2");
        curated_append_in(&tmp.0, &line).unwrap();

        let after = std::fs::read_to_string(tmp.0.join("MEMORY.md")).unwrap();
        assert_eq!(after, format!("{theirs}\n{line}\n"));
        assert!(after.as_bytes().starts_with(theirs.as_bytes()));
        assert!(after
            .lines()
            .any(|row| row.starts_with(MODEL_LINE_PREFIX)));
    }

    #[test]
    fn what_was_written_is_what_the_next_turn_reads() {
        let tmp = Tmp::new();
        std::fs::write(tmp.0.join("MEMORY.md"), "使用者的話\n").unwrap();
        let line = curated_line("使用者偏好 rebase", "T1", "T2");
        curated_append_in(&tmp.0, &line).unwrap();
        let loaded = curated_in(&tmp.0).unwrap();
        assert!(loaded.contains("使用者的話"));
        assert!(loaded.contains(&line));

        // an unwritable directory is a failure message, never a panic
        assert!(curated_append_in(&tmp.0.join("missing"), "L").is_err());
    }

    // The whole chain against real files: the row it names has to be on disk.
    #[test]
    fn the_end_to_end_write_only_lands_when_every_guardrail_agrees() {
        let tmp = Tmp::new();
        std::fs::write(
            tmp.0.join("memory.jsonl"),
            "{\"ts\":\"T1\",\"verb\":\"inject\",\"pane\":\"%1\",\"text\":\"跑測試\"}\n",
        )
        .unwrap();
        let line = curated_write_in(&tmp.0, "使用者偏好 rebase", "T1", "T2").unwrap();
        assert_eq!(
            std::fs::read_to_string(tmp.0.join("MEMORY.md")).unwrap(),
            format!("{line}\n")
        );

        let before = std::fs::read_to_string(tmp.0.join("MEMORY.md")).unwrap();
        assert!(curated_write_in(&tmp.0, "編造的來源", "沒這一列", "T3").is_err());
        assert_eq!(
            std::fs::read_to_string(tmp.0.join("MEMORY.md")).unwrap(),
            before
        );
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
