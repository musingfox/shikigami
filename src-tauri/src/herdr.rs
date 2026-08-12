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
use std::time::{Duration, Instant};
use tauri::Emitter;

const POLL: Duration = Duration::from_secs(2);
const RETRY: Duration = Duration::from_secs(3);
/// Everyday read timeout: every non-blocking herdr method answers instantly.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);

// Last seen roster, for the get_roster command — the frontend may start
// listening after the last agent:roster event already fired.
static ROSTER: Mutex<Vec<AgentEntry>> = Mutex::new(Vec::new());

#[tauri::command]
pub fn get_roster() -> Vec<AgentEntry> {
    ROSTER.lock().map(|r| r.clone()).unwrap_or_default()
}

/// Drain the cached roster on a herdr disconnect and return its previous
/// value. A dead socket means the last snapshot is stale, so we clear it and
/// hand the caller the old contents — a non-empty return tells the caller an
/// empty agent:roster must be emitted; an empty return means nothing changed.
fn take_roster_on_disconnect() -> Vec<AgentEntry> {
    ROSTER.lock().map(|mut r| std::mem::take(&mut *r)).unwrap_or_default()
}

fn socket_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::Path::new(&home).join(".config/herdr/herdr.sock")
}

/// One request/response round-trip on a fresh connection (the server closes
/// after each response, so there is nothing to keep alive).
fn call(id: u64, method: &str, params: Value) -> Result<Value, String> {
    call_at(&socket_path(), CALL_TIMEOUT, id, method, params)
}

/// `call` with the socket path and read timeout spelled out. Blocking herdr
/// methods (`agent.start`, `agent.wait`) run far longer than the everyday 5s,
/// and the socket read must outlast herdr's own `timeout_ms` — otherwise we
/// time out first and lose the real error it was about to send.
fn call_at(
    sock: &std::path::Path,
    timeout: Duration,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let stream = UnixStream::connect(sock).map_err(|e| format!("herdr connect: {e}"))?;
    stream
        .set_read_timeout(Some(timeout))
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

/// How long herdr may spend bringing a new agent up. Its own budget is
/// SUMMON_TIMEOUT_MS (>3000, <=300000 per protocol 17); our socket read has to
/// outlast that, or we abandon the connection just before herdr explains what
/// went wrong.
const SUMMON_TIMEOUT_MS: u64 = 120_000;
const SUMMON_READ_TIMEOUT: Duration = Duration::from_secs(150);
const PROMPT_RETRY_INTERVAL: Duration = Duration::from_millis(500);
/// agent.prompt is not a blocking herdr method — an accept comes back in
/// milliseconds and a not-ready rejection took ~70ms in the 0.8.0 repro. Giving
/// each retry attempt the 150s summon read timeout would let the last attempt
/// push the give-up time to ~270s, well past the 120s budget.
const PROMPT_READ_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a fresh tab may take to reach an idle shell prompt. Measured at
/// 2–5s with this fish config; the budget also covers agent.start's own
/// `agent_pane_busy` retries.
const SHELL_READY_TIMEOUT: Duration = Duration::from_secs(30);
const SHELL_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// herdr keeps one agent per name for as long as the pane lives, so naming a
/// summoned agent after its project alone meant the second summon of that
/// project — or the first one after a failed summon left its pane open — died
/// with `agent_name_taken`. The suffix is the clock truncated to four digits:
/// short enough to still say out loud (the roster feeds voice targeting), and
/// only repeats for two summons landing exactly 10000s apart.
fn agent_name(project: &str, epoch_secs: u64) -> String {
    format!("{project}-{:04}", epoch_secs % 10_000)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Retry one herdr call for as long as it keeps failing with a single expected
/// error code, then report which step gave up. Matching the quoted `"code":"…"`
/// pair rather than the bare token keeps a message body that merely mentions the
/// code from being mistaken for the code itself.
fn retry_while<F: FnMut() -> Result<Value, String>>(
    step: &str,
    code: &str,
    deadline: Duration,
    interval: Duration,
    mut send: F,
) -> Result<Value, String> {
    let expected = format!("\"code\":\"{code}\"");
    let start = Instant::now();
    let mut attempts = 0;
    // The deadline is checked after the sleep, never after the send: a send is
    // only ever issued from inside the deadline, so giving up overshoots by one
    // attempt's read timeout at most. Checking it on the send's own error would
    // let an attempt started at deadline-ε run its full read timeout on top.
    let last_err = loop {
        attempts += 1;
        match send() {
            Ok(result) => return Ok(result),
            Err(err) if !err.contains(&expected) => return Err(err),
            Err(err) => {
                std::thread::sleep(interval);
                if start.elapsed() >= deadline {
                    break err;
                }
            }
        }
    };
    Err(format!(
        "{step} still rejected as {code} after {}ms ({attempts} attempts); last error: {last_err}",
        start.elapsed().as_millis()
    ))
}

fn prompt_until_accepted<F: FnMut() -> Result<Value, String>>(
    deadline: Duration,
    interval: Duration,
    send: F,
) -> Result<Value, String> {
    retry_while("agent.prompt", "agent_not_ready", deadline, interval, send)
}

/// A pane is only startable once its shell is back at the prompt. A freshly
/// created tab is not: fish runs its config first, so for a second or two the
/// pane's foreground process is a startup child (`cut`, `cat`, …) rather than
/// the shell itself — measured against herdr 0.8.0 on 2026-08-12, the pane
/// flipped busy → idle → busy again before settling.
fn shell_is_idle(info: &Value) -> bool {
    let p = &info["process_info"];
    let Some(shell) = p["shell_pid"].as_u64() else { return false };
    let Some(fg) = p["foreground_processes"].as_array() else { return false };
    fg.len() == 1 && fg[0]["pid"].as_u64() == Some(shell)
}

/// Poll until the pane's shell is idle. Probe errors are retried, not fatal —
/// a pane herdr has not registered yet answers `agent_pane_not_found`.
fn wait_for_shell<F: FnMut() -> Result<Value, String>>(
    deadline: Duration,
    interval: Duration,
    mut probe: F,
) -> Result<(), String> {
    let start = Instant::now();
    let mut last: String;
    loop {
        match probe() {
            Ok(info) if shell_is_idle(&info) => return Ok(()),
            Ok(_) => last = "shell still busy with its startup".to_string(),
            Err(err) => last = err,
        }
        std::thread::sleep(interval);
        if start.elapsed() >= deadline {
            return Err(format!(
                "pane never reached an idle shell prompt within {}ms; last probe: {last}",
                start.elapsed().as_millis()
            ));
        }
    }
}

/// herdr answers `agent.start` on a pane whose shell is not up yet by deferring
/// the launch — `ok` with `launch_pending: true`. It does start the agent, but
/// the launch never completes: the name stays out of the agent registry, so
/// every later `agent.prompt` is refused with `agent_not_ready`, forever
/// (measured 2026-08-12: 238 retries over 120s, then still pending minutes
/// later). There is no way back from that branch, so treat it as the failure it
/// is instead of retrying a prompt that cannot be accepted.
fn launch_completed(result: &Value) -> Result<(), String> {
    let pending = result["launch_pending"].as_bool().unwrap_or(false)
        || result["agent"]["launch_pending"].as_bool().unwrap_or(false);
    if pending {
        return Err(
            "herdr deferred the launch (launch_pending) — the pane's shell was not ready, so the \
             agent starts unnamed and will never accept a prompt"
                .to_string(),
        );
    }
    Ok(())
}

fn prompt_step(sock: &std::path::Path, pane: &str, task: &str) -> Result<Value, String> {
    prompt_until_accepted(
        Duration::from_millis(SUMMON_TIMEOUT_MS),
        PROMPT_RETRY_INTERVAL,
        || {
            call_at(
                sock,
                PROMPT_READ_TIMEOUT,
                0,
                "agent.prompt",
                serde_json::json!({ "target": pane, "text": task }),
            )
        },
    )
}

/// The pane a fresh `tab.create` opened, from its `tab_created` result.
pub fn extract_pane_id(result: &Value) -> Result<String, String> {
    result
        .get("root_pane")
        .and_then(|p| p.get("pane_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "summon tab.create: result has no root_pane.pane_id".to_string())
}

/// Tag a failed summon step with the herdr method that broke, so a half-built
/// chain reports where it stopped.
fn step(stage: &str, result: Result<Value, String>) -> Result<Value, String> {
    result.map_err(|e| format!("summon {stage}: {e}"))
}

/// Summon a new claude agent: open a tab in the project directory, start
/// claude in its pane, wait for it to settle, then hand it the task. One
/// confirmed request, one fully-briefed agent — the pane is an ordinary
/// interactive TUI the user can take over at any time (see CLAUDE.md: no
/// headless spawn). Returns the new pane id, so the caller can name or focus
/// the agent it just summoned.
/// Wire shapes verified against the bundled schema of the installed herdr
/// (`herdr api schema --json`, protocol 19): agent.start takes `pane_id`, and
/// agent.wait takes `target`/`until`/`timeout_ms`.
pub fn summon(project: &str, task: &str, cwd: &str) -> Result<String, String> {
    let sock = socket_path();
    let tab = step(
        "tab.create",
        call_at(
            &sock,
            CALL_TIMEOUT,
            0,
            "tab.create",
            serde_json::json!({ "cwd": cwd, "label": project }),
        ),
    )?;
    let pane = extract_pane_id(&tab)?;
    step(
        "pane.process_info",
        wait_for_shell(SHELL_READY_TIMEOUT, SHELL_POLL_INTERVAL, || {
            call_at(
                &sock,
                CALL_TIMEOUT,
                0,
                "pane.process_info",
                serde_json::json!({ "pane_id": pane }),
            )
        })
        .map(|_| Value::Null),
    )?;
    // No `timeout_ms`: herdr's CLI omits it and the call then completes the
    // launch synchronously — name registered, ready for a prompt. The retry
    // covers the pane flipping back to busy between our probe and this call.
    let started = step(
        "agent.start",
        retry_while(
            "agent.start",
            "agent_pane_busy",
            SHELL_READY_TIMEOUT,
            PROMPT_RETRY_INTERVAL,
            || {
                call_at(
                    &sock,
                    SUMMON_READ_TIMEOUT,
                    0,
                    "agent.start",
                    serde_json::json!({
                        "name": agent_name(project, now_secs()),
                        "kind": "claude",
                        "pane_id": pane,
                    }),
                )
            },
        ),
    )?;
    step("agent.start", launch_completed(&started).map(|_| Value::Null))?;
    step(
        "agent.wait",
        call_at(
            &sock,
            SUMMON_READ_TIMEOUT,
            0,
            "agent.wait",
            // herdr's own `agent wait` default (per its CLI help): settled
            // means idle, done, OR blocked. Waiting only for idle would hang
            // the full timeout whenever a fresh claude stops at a prompt —
            // a first-run trust dialog reports blocked, not idle.
            serde_json::json!({
                "target": pane,
                "until": ["idle", "done", "blocked"],
                "timeout_ms": SUMMON_TIMEOUT_MS,
            }),
        ),
    )?;
    step("agent.prompt", prompt_step(&sock, &pane, task))?;
    Ok(pane)
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
                        // Socket died: the cached roster is now a dead snapshot.
                        // Clear it and tell observers exactly once, on the
                        // ok→error transition — not on every failed retry tick.
                        if !take_roster_on_disconnect().is_empty() {
                            let _ = app.emit(AGENT_ROSTER, Vec::<AgentEntry>::new());
                        }
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

    // Both tests mutate the shared ROSTER static; serialize them so cargo's
    // parallel runner can't interleave their setup.
    static ROSTER_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn t7_take_on_disconnect_returns_prev_and_clears() {
        let _g = ROSTER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *ROSTER.lock().unwrap() = vec![entry("t", "n", "p", "working")];
        let prev = take_roster_on_disconnect();
        assert_eq!(prev, vec![entry("t", "n", "p", "working")]);
        assert!(get_roster().is_empty());
    }

    #[test]
    fn t8_take_on_disconnect_when_empty_returns_empty() {
        let _g = ROSTER_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *ROSTER.lock().unwrap() = Vec::new();
        let prev = take_roster_on_disconnect();
        assert!(prev.is_empty());
    }

    // HerdrCallTimeout — a throwaway listener stands in for herdr. Socket names
    // stay short: macOS caps a unix path at ~104 bytes.
    fn tmp_sock(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("shk-{tag}.sock"));
        let _ = std::fs::remove_file(&p); // a stale file from a past run means AddrInUse
        p
    }

    #[test]
    fn ct1_call_at_returns_result_within_timeout() {
        let path = tmp_sock("ct1");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let h = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut req = String::new();
            BufReader::new(s.try_clone().unwrap()).read_line(&mut req).unwrap();
            s.write_all(b"{\"id\":\"shikigami-1\",\"result\":{\"ok\":true}}\n").unwrap();
        });
        let got = call_at(&path, Duration::from_secs(2), 1, "ping", serde_json::json!({}));
        h.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(got.unwrap(), serde_json::json!({"ok": true}));
    }

    #[test]
    fn ct2_call_at_times_out_when_server_never_answers() {
        let path = tmp_sock("ct2");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        // Hold the accepted stream: dropping it would close the connection and
        // yield "connection closed" instead of the read timeout under test.
        let h = std::thread::spawn(move || {
            let (_held, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_secs(2));
        });
        let err = call_at(&path, Duration::from_millis(200), 1, "ping", serde_json::json!({}))
            .unwrap_err();
        assert!(err.contains("herdr read"), "{err}");
        h.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn ct3_call_at_missing_socket_is_a_connect_error() {
        let path = std::env::temp_dir().join("shk-nonexistent.sock");
        let _ = std::fs::remove_file(&path);
        let err =
            call_at(&path, Duration::from_secs(1), 1, "ping", serde_json::json!({})).unwrap_err();
        assert!(err.contains("herdr connect"), "{err}");
    }

    // SummonChain
    #[test]
    fn sc1_pane_id_read_from_tab_created() {
        let result =
            serde_json::json!({ "root_pane": { "pane_id": "wZ:p1" }, "tab_id": "wZ:t1" });
        assert_eq!(extract_pane_id(&result).unwrap(), "wZ:p1");
    }

    #[test]
    fn sc2_missing_pane_id_names_the_step_that_failed() {
        let err = extract_pane_id(&serde_json::json!({})).unwrap_err();
        assert!(err.contains("tab.create"), "{err}");
    }

    #[test]
    fn sc3_step_failure_carries_its_herdr_method() {
        let err = step("agent.start", Err("connection closed".to_string())).unwrap_err();
        assert_eq!(err, "summon agent.start: connection closed");
    }

    // PromptRetry
    #[test]
    fn pr3a_non_not_ready_error_fails_immediately() {
        let calls = std::cell::Cell::new(0usize);
        let err = "herdr agent.prompt: {\"code\":\"agent_not_found\",\"message\":\"no such agent\"}";
        let start = std::time::Instant::now();
        let got = prompt_until_accepted(Duration::from_secs(1), Duration::from_millis(1), || {
            calls.set(calls.get() + 1);
            Err(err.to_string())
        });
        assert_eq!(got.unwrap_err(), err);
        assert_eq!(calls.get(), 1);
        assert!(start.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn pr3b_message_token_without_code_pair_fails_immediately() {
        let calls = std::cell::Cell::new(0usize);
        let err = "herdr agent.prompt: {\"code\":\"agent_not_found\",\"message\":\"agent_not_ready was not the problem\"}";
        let got = prompt_until_accepted(Duration::from_secs(1), Duration::from_millis(1), || {
            calls.set(calls.get() + 1);
            Err(err.to_string())
        });
        assert_eq!(got.unwrap_err(), err);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn pr1_retries_not_ready_until_accepted() {
        let not_ready = "herdr agent.prompt: {\"code\":\"agent_not_ready\",\"message\":\"agent wD:pB is not an active named agent\"}";
        let script = [
            Err(not_ready.to_string()),
            Err(not_ready.to_string()),
            Ok(serde_json::json!({"ok": true})),
        ];
        let calls = std::cell::Cell::new(0usize);
        let start = std::time::Instant::now();
        let got = prompt_until_accepted(Duration::from_secs(1), Duration::from_millis(1), || {
            let n = calls.get();
            calls.set(n + 1);
            script[n].clone()
        });
        assert_eq!(got.unwrap(), serde_json::json!({"ok": true}));
        assert_eq!(calls.get(), 3);
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn pr4_accepts_first_prompt_without_sleeping() {
        let calls = std::cell::Cell::new(0usize);
        let start = std::time::Instant::now();
        let got = prompt_until_accepted(Duration::from_secs(1), Duration::from_millis(1), || {
            calls.set(calls.get() + 1);
            Ok(serde_json::json!({"ok": true}))
        });
        assert_eq!(got.unwrap(), serde_json::json!({"ok": true}));
        assert_eq!(calls.get(), 1);
        assert!(start.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn pr2_timeout_names_prompt_step_and_preserves_last_error() {
        let calls = std::cell::Cell::new(0usize);
        let err = "herdr agent.prompt: {\"code\":\"agent_not_ready\",\"message\":\"agent wD:pB is not an active named agent\"}";
        let start = std::time::Instant::now();
        let got = prompt_until_accepted(
            Duration::from_millis(30),
            Duration::from_millis(5),
            || {
                calls.set(calls.get() + 1);
                Err(err.to_string())
            },
        );
        let elapsed = start.elapsed();
        let err = got.unwrap_err();
        assert!(err.contains("agent.prompt"), "{err}");
        assert!(err.contains("agent_not_ready"), "{err}");
        assert!(err.contains("agent wD:pB is not an active named agent"), "{err}");
        assert!(elapsed >= Duration::from_millis(30), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
        assert!((2..=100).contains(&calls.get()), "{}", calls.get());
    }

    /// The bound the give-up time relies on: with a coarse interval the old
    /// shape woke up past the deadline and still issued one more send, which
    /// then got a full read timeout to run in.
    fn proc_info(fg: Value, shell: u64) -> Value {
        serde_json::json!({
            "process_info": { "shell_pid": shell, "foreground_processes": fg }
        })
    }

    #[test]
    fn sh1_only_the_shell_alone_in_the_foreground_counts_as_idle() {
        assert!(shell_is_idle(&proc_info(serde_json::json!([{"pid": 89527, "name": "fish"}]), 89527)));
        // fish running its config: the foreground is a startup child
        assert!(!shell_is_idle(&proc_info(serde_json::json!([{"pid": 89600, "name": "cut"}]), 89527)));
        assert!(!shell_is_idle(&proc_info(serde_json::json!([]), 89527)));
        assert!(!shell_is_idle(&serde_json::json!({})));
    }

    #[test]
    fn sh2_waits_through_busy_probes_and_probe_errors() {
        let calls = std::cell::Cell::new(0usize);
        let got = wait_for_shell(Duration::from_secs(1), Duration::from_millis(1), || {
            calls.set(calls.get() + 1);
            match calls.get() {
                1 => Err("herdr pane.process_info: {\"code\":\"agent_pane_not_found\"}".to_string()),
                2 => Ok(proc_info(serde_json::json!([{"pid": 2, "name": "cut"}]), 1)),
                _ => Ok(proc_info(serde_json::json!([{"pid": 1, "name": "fish"}]), 1)),
            }
        });
        assert!(got.is_ok(), "{got:?}");
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn sh3_timeout_says_what_the_pane_was_doing() {
        let err = wait_for_shell(Duration::from_millis(30), Duration::from_millis(5), || {
            Ok(proc_info(serde_json::json!([{"pid": 2, "name": "cat"}]), 1))
        })
        .unwrap_err();
        assert!(err.contains("idle shell prompt"), "{err}");
        assert!(err.contains("shell still busy"), "{err}");
    }

    #[test]
    fn st1_agent_start_retries_while_the_pane_is_busy() {
        let calls = std::cell::Cell::new(0usize);
        let got = retry_while(
            "agent.start",
            "agent_pane_busy",
            Duration::from_secs(1),
            Duration::from_millis(1),
            || {
                calls.set(calls.get() + 1);
                if calls.get() < 3 {
                    Err("herdr agent.start: {\"code\":\"agent_pane_busy\",\"message\":\"not an available shell\"}".to_string())
                } else {
                    Ok(serde_json::json!({"ok": true}))
                }
            },
        );
        assert_eq!(got.unwrap(), serde_json::json!({"ok": true}));
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn st2_a_deferred_launch_is_a_failure_not_something_to_wait_out() {
        let err = launch_completed(&serde_json::json!({"agent": {"launch_pending": true}})).unwrap_err();
        assert!(err.contains("launch_pending"), "{err}");
        assert!(launch_completed(&serde_json::json!({"launch_pending": true})).is_err());
        assert!(launch_completed(&serde_json::json!({"agent": {"name": "shikigami-0816"}})).is_ok());
    }

    #[test]
    fn an1_agent_name_carries_the_project_and_a_four_digit_clock() {
        assert_eq!(agent_name("shikigami", 1_786_000_123), "shikigami-0123");
        assert_eq!(agent_name("shikigami", 7), "shikigami-0007");
    }

    #[test]
    fn an2_two_summons_a_second_apart_get_different_names() {
        assert_ne!(agent_name("shikigami", 1_786_000_123), agent_name("shikigami", 1_786_000_124));
    }

    #[test]
    fn pr7_no_send_is_issued_after_the_deadline() {
        let start = std::time::Instant::now();
        let sends = std::cell::RefCell::new(Vec::new());
        let deadline = Duration::from_millis(30);
        let got = prompt_until_accepted(deadline, Duration::from_millis(20), || {
            sends.borrow_mut().push(start.elapsed());
            Err("herdr agent.prompt: {\"code\":\"agent_not_ready\",\"message\":\"x\"}".to_string())
        });
        assert!(got.is_err());
        let sends = sends.borrow();
        assert!(sends.len() >= 2, "{sends:?}");
        for at in sends.iter() {
            assert!(*at < deadline, "send issued at {at:?}, past the {deadline:?} deadline");
        }
    }

    #[test]
    fn pr5_prompt_step_retries_agent_prompt_request() {
        let path = tmp_sock("pr5");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let h = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for response in [
                b"{\"id\":\"shikigami-0\",\"error\":{\"code\":\"agent_not_ready\",\"message\":\"agent wD:pB is not an active named agent\"}}\n".as_slice(),
                b"{\"id\":\"shikigami-0\",\"result\":{\"ok\":true}}\n".as_slice(),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut req = String::new();
                BufReader::new(stream.try_clone().unwrap()).read_line(&mut req).unwrap();
                requests.push(serde_json::from_str::<Value>(&req).unwrap());
                stream.write_all(response).unwrap();
            }
            requests
        });
        let got = prompt_step(&path, "wD:pB", "跑測試");
        // Assert before joining: a single-shot regression never opens the second
        // connection, so joining first would block the run forever instead of
        // failing it (cargo test has no per-test timeout).
        assert_eq!(got.unwrap(), serde_json::json!({"ok": true}));
        let requests = h.join().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(requests.len(), 2);
        for request in requests {
            assert_eq!(request["method"], "agent.prompt");
            assert_eq!(
                request["params"],
                serde_json::json!({"target": "wD:pB", "text": "跑測試"})
            );
        }
    }

    /// T4 — the whole chain against a live herdr, on a real project directory.
    /// Run manually: `cargo test -- --ignored live_summon`. Leaves a claude
    /// agent running in a new tab.
    #[test]
    #[ignore]
    fn live_summon_opens_a_briefed_claude_agent() {
        let cwd = std::path::Path::new(&std::env::var("HOME").unwrap())
            .join("workspace/shikigami");
        assert!(cwd.is_dir(), "expected a real project at {}", cwd.display());
        let cwd = cwd.to_str().unwrap();
        let pane =
            summon("shikigami", "回報你在哪個目錄，不要改任何檔案", cwd).expect("summon chain");
        let listed = call(0, "agent.list", serde_json::json!({})).expect("agent.list");
        let found = listed["agents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["cwd"] == cwd && a["agent"] == "claude" && a["pane_id"] == pane.as_str());
        assert!(found, "no claude agent at {cwd} in pane {pane}: {listed}");
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
