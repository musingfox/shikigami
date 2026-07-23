// herdr adapter (channel ①): agent roster + coarse status → core events.
// Everything herdr-specific stays in this module (iron rule 3).
// Wire (verified live against herdr 0.7.5, protocol 17): NDJSON over
// ~/.config/herdr/herdr.sock — request {id,method,params} (params required,
// {} for no-arg), response {id,result|error}, one JSON object per line, and
// the server CLOSES the connection after one request/response cycle → we
// connect per request (local unix socket, negligible).
// ponytail: 2s agent.list poll + diff, not events.subscribe — herdr's own
// screen-based status detection has seconds of latency anyway, and per-pane
// subscriptions would need pane lifecycle tracking; subscribe if latency matters.

use crate::events::{AgentEntry, AgentStatusChange, AGENT_ROSTER, AGENT_STATUS};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::Mutex;
use std::time::Duration;
use tauri::Emitter;

const POLL: Duration = Duration::from_secs(2);
const RETRY: Duration = Duration::from_secs(3);

// Last seen roster, for the get_roster command — the frontend may start
// listening after the last agent:roster event already fired.
static ROSTER: Mutex<Vec<AgentEntry>> = Mutex::new(Vec::new());

#[tauri::command]
pub fn get_roster() -> Vec<AgentEntry> {
    ROSTER.lock().map(|r| r.clone()).unwrap_or_default()
}

fn socket_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::Path::new(&home).join(".config/herdr/herdr.sock")
}

/// One request/response round-trip on a fresh connection (the server closes
/// after each response, so there is nothing to keep alive).
fn call(id: u64, method: &str, params: Value) -> Result<Value, String> {
    let stream =
        UnixStream::connect(socket_path()).map_err(|e| format!("herdr connect: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| format!("herdr timeout cfg: {e}"))?;
    let mut conn = BufReader::new(stream);
    let req = serde_json::json!({
        "id": format!("shikigami-{id}"),
        "method": method,
        "params": params,
    });
    let mut line = req.to_string();
    line.push('\n');
    conn.get_mut()
        .write_all(line.as_bytes())
        .map_err(|e| format!("herdr write: {e}"))?;
    let mut buf = String::new();
    conn.read_line(&mut buf).map_err(|e| format!("herdr read: {e}"))?;
    if buf.is_empty() {
        return Err("herdr: connection closed".to_string());
    }
    let v: Value = serde_json::from_str(&buf).map_err(|e| format!("herdr parse: {e}"))?;
    if let Some(err) = v.get("error") {
        return Err(format!("herdr {method}: {err}"));
    }
    v.get("result")
        .cloned()
        .ok_or_else(|| format!("herdr {method}: no result"))
}

/// Normalize an `agent.list` result into core AgentEntry rows.
pub fn parse_agent_list(result: &Value) -> Vec<AgentEntry> {
    result
        .get("agents")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(agent_entry).collect())
        .unwrap_or_default()
}

fn agent_entry(v: &Value) -> Option<AgentEntry> {
    let id = v.get("terminal_id")?.as_str()?.to_string();
    let pane = v.get("pane_id")?.as_str()?.to_string();
    let status = v
        .get("agent_status")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let str_field = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let cwd = str_field("cwd").unwrap_or_default();
    let title = str_field("terminal_title_stripped").unwrap_or_default();
    let name = str_field("name")
        .or_else(|| {
            std::path::Path::new(&cwd)
                .file_name()
                .map(|b| b.to_string_lossy().into_owned())
        })
        .or_else(|| str_field("agent"))
        .unwrap_or_else(|| id.clone());
    Some(AgentEntry { id, name, pane, status, title, cwd })
}

/// Channel ② inject (R2a): submit a user-confirmed prompt to an agent.
/// herdr's agent.prompt composes AND submits (documented semantics) — no
/// key plumbing needed. Only ever called after the frontend confirm step.
#[tauri::command]
pub fn prompt_agent(pane: String, text: String) -> Result<(), String> {
    call(0, "agent.prompt", serde_json::json!({ "target": pane, "text": text }))?;
    Ok(())
}

/// Jump to an agent's pane: herdr switches workspace/pane focus, then we
/// bring the terminal app forward.
/// ponytail: terminal app hardcoded to Ghostty; configurable when needed.
#[tauri::command]
pub fn focus_agent(pane: String) -> Result<(), String> {
    call(0, "agent.focus", serde_json::json!({ "target": pane }))?;
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").args(["-a", "Ghostty"]).spawn();
    }
    Ok(())
}

/// Diff old→new: whether anything changed (membership, name, pane, status)
/// and the per-agent status transitions.
pub fn diff_roster(old: &[AgentEntry], new: &[AgentEntry]) -> (bool, Vec<AgentStatusChange>) {
    let changes = new
        .iter()
        .filter_map(|n| {
            old.iter()
                .find(|o| o.id == n.id)
                .filter(|o| o.status != n.status)
                .map(|_| AgentStatusChange { id: n.id.clone(), status: n.status.clone() })
        })
        .collect();
    (old != new, changes)
}

pub fn spawn_watcher(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let mut seq: u64 = 0;
        let mut force = true; // first success and every post-error success re-emit
        let mut was_ok = true; // log error transitions once, not every retry
        loop {
            seq += 1;
            match poll_once(&app, seq, force) {
                Ok(()) => {
                    if !was_ok || force {
                        eprintln!("[herdr] connected");
                    }
                    force = false;
                    was_ok = true;
                    std::thread::sleep(POLL);
                }
                Err(e) => {
                    if was_ok {
                        eprintln!("[herdr] {e}; retrying every {}s", RETRY.as_secs());
                    }
                    force = true;
                    was_ok = false;
                    std::thread::sleep(RETRY);
                }
            }
        }
    });
}

fn poll_once(app: &tauri::AppHandle, seq: u64, force: bool) -> Result<(), String> {
    let result = call(seq, "agent.list", serde_json::json!({}))?;
    let roster = parse_agent_list(&result);
    let (changed, status_changes) = {
        let last = ROSTER.lock().map_err(|e| e.to_string())?;
        diff_roster(&last, &roster)
    };
    if force || changed {
        let _ = app.emit(AGENT_ROSTER, roster.clone());
    }
    for c in status_changes {
        let _ = app.emit(AGENT_STATUS, c);
    }
    if let Ok(mut last) = ROSTER.lock() {
        *last = roster;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::AgentEntry;

    fn entry(id: &str, name: &str, pane: &str, status: &str) -> AgentEntry {
        entry_full(id, name, pane, status, "", "")
    }

    fn entry_full(id: &str, name: &str, pane: &str, status: &str, title: &str, cwd: &str) -> AgentEntry {
        AgentEntry {
            id: id.into(),
            name: name.into(),
            pane: pane.into(),
            status: status.into(),
            title: title.into(),
            cwd: cwd.into(),
        }
    }

    // field shapes taken from a live `herdr api snapshot` (v0.7.5, protocol 17)
    fn live_like_result() -> Value {
        serde_json::json!({
            "type": "agent_list",
            "agents": [
                {
                    "terminal_id": "term_a", "pane_id": "wD:p1",
                    "workspace_id": "wD", "tab_id": "wD:t1", "focused": true,
                    "revision": 3, "agent_status": "blocked", "agent": "claude",
                    "name": null, "cwd": "/Users/x/workspace/cyris"
                },
                {
                    "terminal_id": "term_b", "pane_id": "wT:p1",
                    "workspace_id": "wT", "tab_id": "wT:t1", "focused": false,
                    "revision": 12, "agent_status": "working", "agent": "claude",
                    "name": "builder", "cwd": "/Users/x/workspace/shikigami",
                    "terminal_title_stripped": "設計 1:1:N 架構"
                }
            ]
        })
    }

    #[test]
    fn t1_parse_maps_fields_and_name_fallbacks() {
        let rows = parse_agent_list(&live_like_result());
        assert_eq!(
            rows,
            vec![
                // name null → cwd basename; no title in source → ""
                entry_full("term_a", "cyris", "wD:p1", "blocked", "", "/Users/x/workspace/cyris"),
                // declared name wins; title from terminal_title_stripped
                entry_full("term_b", "builder", "wT:p1", "working", "設計 1:1:N 架構", "/Users/x/workspace/shikigami"),
            ]
        );
    }

    #[test]
    fn t2_parse_tolerates_missing_optionals() {
        let v = serde_json::json!({ "agents": [
            { "terminal_id": "t", "pane_id": "p" }
        ]});
        assert_eq!(parse_agent_list(&v), vec![entry("t", "t", "p", "unknown")]);
    }

    #[test]
    fn t3_parse_skips_rows_without_identity() {
        let v = serde_json::json!({ "agents": [ { "pane_id": "p" }, null ] });
        assert_eq!(parse_agent_list(&v), vec![]);
        assert_eq!(parse_agent_list(&serde_json::json!({})), vec![]);
    }

    #[test]
    fn t4_diff_no_change() {
        let a = vec![entry("t", "n", "p", "idle")];
        let (changed, transitions) = diff_roster(&a, &a.clone());
        assert!(!changed);
        assert!(transitions.is_empty());
    }

    #[test]
    fn t5_diff_status_transition() {
        let old = vec![entry("t", "n", "p", "working")];
        let new = vec![entry("t", "n", "p", "idle")];
        let (changed, transitions) = diff_roster(&old, &new);
        assert!(changed);
        assert_eq!(
            transitions,
            vec![AgentStatusChange { id: "t".into(), status: "idle".into() }]
        );
    }

    #[test]
    fn t6_diff_membership_change_is_roster_only() {
        let old = vec![entry("t", "n", "p", "idle")];
        let new = vec![
            entry("t", "n", "p", "idle"),
            entry("u", "m", "q", "working"),
        ];
        let (changed, transitions) = diff_roster(&old, &new);
        assert!(changed);
        assert!(transitions.is_empty()); // new arrivals surface via roster, not status
    }

    /// Integration receipt against a live herdr — run manually:
    /// `cargo test -- --ignored live_herdr`
    #[test]
    #[ignore]
    fn live_herdr_agent_list_round_trip() {
        call(0, "ping", serde_json::json!({})).expect("ping");
        let result = call(1, "agent.list", serde_json::json!({})).expect("agent.list");
        let roster = parse_agent_list(&result);
        eprintln!("[live] roster = {roster:?}");
        for a in &roster {
            assert!(!a.id.is_empty());
            assert!(!a.pane.is_empty());
            assert!(["idle", "working", "blocked", "done", "unknown"]
                .contains(&a.status.as_str()));
        }
    }
}
