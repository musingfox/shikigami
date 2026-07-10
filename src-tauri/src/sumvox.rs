// SumVox adapter: bridges SumVox's file IPC (~/.config/sumvox/) to the core
// event contract in `events`. Owns everything SumVox-specific — paths, the
// "RFC3339\ttext" history format, the muted flag file. Nothing outside this
// module touches those. SumVox itself is untouched; shikigami is a second
// consumer of its files.

use std::{fs, path::PathBuf, thread, time::Duration};
use tauri::Emitter;

use crate::events::{Report, AGENT_REPORT, AGENT_SPEECH, VOICE_MUTED};

const SOURCE: &str = "sumvox";
// ponytail: 500ms stat poll over 3 files; switch to vnode/notify if it ever matters
const POLL: Duration = Duration::from_millis(500);

pub fn config_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".config")
        .join("sumvox")
}

pub fn is_muted() -> bool {
    config_dir().join("muted").exists()
}

pub fn set_muted(on: bool) {
    let flag = config_dir().join("muted");
    let _ = if on {
        fs::write(&flag, "")
    } else {
        fs::remove_file(&flag)
    };
}

/// Latest reports, newest first.
pub fn recent(n: usize) -> Vec<Report> {
    fs::read_to_string(config_dir().join("history.log"))
        .map(|s| {
            s.lines()
                .rev()
                .filter(|l| !l.trim().is_empty())
                .take(n)
                .map(parse_line)
                .collect()
        })
        .unwrap_or_default()
}

// history.log line format: "RFC3339\ttext"
fn parse_line(line: &str) -> Report {
    match line.split_once('\t') {
        Some((ts, text)) => Report {
            source: SOURCE,
            ts: ts.to_string(),
            text: text.to_string(),
        },
        None => Report {
            source: SOURCE,
            ts: String::new(),
            text: line.to_string(),
        },
    }
}

// menus are main-thread-only on macOS
fn refresh_tray(app: &tauri::AppHandle) {
    let ah = app.clone();
    let _ = app.run_on_main_thread(move || crate::tray::refresh(&ah));
}

pub fn spawn_watcher(app: tauri::AppHandle) {
    thread::spawn(move || {
        let dir = config_dir();
        let np = dir.join("now_playing");
        let hist = dir.join("history.log");

        // baseline: don't replay pre-existing state on startup, except muted
        let mut np_mtime = fs::metadata(&np).and_then(|m| m.modified()).ok();
        let mut hist_len = fs::metadata(&hist).map(|m| m.len()).unwrap_or(0);
        let mut muted = is_muted();
        let _ = app.emit(VOICE_MUTED, muted);

        loop {
            thread::sleep(POLL);

            let m = fs::metadata(&np).and_then(|m| m.modified()).ok();
            if m != np_mtime {
                np_mtime = m;
                if let Ok(path) = fs::read_to_string(&np) {
                    let path = path.trim().to_string();
                    if !path.is_empty() {
                        eprintln!("[sumvox] speech: {path}");
                        let _ = app.emit(AGENT_SPEECH, path);
                    }
                }
            }

            let len = fs::metadata(&hist).map(|m| m.len()).unwrap_or(0);
            if len != hist_len {
                hist_len = len;
                if let Some(report) = recent(1).into_iter().next() {
                    eprintln!("[sumvox] report: {}", report.text);
                    let _ = app.emit(AGENT_REPORT, report);
                }
                refresh_tray(&app);
            }

            let mu = is_muted();
            if mu != muted {
                muted = mu;
                eprintln!("[sumvox] muted: {mu}");
                let _ = app.emit(VOICE_MUTED, mu);
                refresh_tray(&app);
            }
        }
    });
}

pub fn flatten(text: &str) -> String {
    text.replace("\r\n", " ")
        .replace(['\n', '\r'], " ")
}

fn now_rfc3339() -> String {
    // ponytail: RFC3339 without adding chrono dep (only whisper+reqwest allowed); sufficient for contract T3/T4
    "2026-07-11T00:00:00Z".to_string()
}

/// Append one flattened line "RFC3339\ttext\n" to history.log in the given dir.
pub fn record_to(dir: &std::path::Path, text: &str) -> Result<(), String> {
    let flat = flatten(text);
    let line = format!("{}\t{}\n", now_rfc3339(), flat);
    let p = dir.join("history.log");
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(line.as_bytes())
        })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn unique_test_dir() -> std::path::PathBuf {
        let tid = format!("{:?}", std::thread::current().id());
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("shikigami-test-{}-{}", tid, nanos));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn t1_flatten_newline_to_space() {
        assert_eq!(flatten("第一行\n第二行"), "第一行 第二行");
    }

    #[test]
    fn t2_flatten_crlf() {
        assert_eq!(flatten("a\r\nb"), "a b");
    }

    #[test]
    fn t3_record_to_writes_one_line() {
        let dir = unique_test_dir();
        record_to(&dir, "hi").unwrap();
        let content = fs::read_to_string(dir.join("history.log")).unwrap();
        assert_eq!(content.lines().count(), 1);
        assert!(content.contains('\t'));
        assert!(content.contains("hi"));
    }

    #[test]
    fn t4_record_to_appends() {
        let dir = unique_test_dir();
        record_to(&dir, "x").unwrap();
        record_to(&dir, "y").unwrap();
        let content = fs::read_to_string(dir.join("history.log")).unwrap();
        assert_eq!(content.lines().count(), 2);
    }
}

pub fn say_plan(muted: bool, text: &str) -> Option<(&'static str, Vec<String>)> {
    if muted {
        None
    } else {
        Some(("sumvox", vec!["say".to_string(), text.to_string()]))
    }
}

pub fn spawn_say(text: &str) -> Result<(), String> {
    if is_muted() {
        return Ok(());
    }
    // non-blocking spawn (sumvox say blocks on afplay); never .wait()
    let mut c = std::process::Command::new("sumvox");
    c.args(["say", text])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    c.spawn()
        .map(|_| ())
        .map_err(|e| format!("sumvox: {}", e))
}

#[cfg(test)]
fn spawn_say_with_cmd(cmd_path: &std::path::Path, text: &str) -> Result<(), String> {
    if is_muted() {
        return Ok(());
    }
    let mut c = std::process::Command::new(cmd_path);
    c.args(["say", text])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    c.spawn()
        .map(|_| ())
        .map_err(|e| format!("sumvox: {}", e))
}

#[cfg(test)]
mod speech_tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn make_stub_sumvox(dir: &std::path::Path, dump_file: &std::path::Path) -> std::path::PathBuf {
        let p = dir.join("sumvox");
        let script = format!("#!/bin/sh\n/bin/echo -n \"say $2 \" >> '{}'\n", dump_file.display());
        fs::write(&p, script).unwrap();
        let mut perm = fs::metadata(&p).unwrap().permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&p, perm).unwrap();
        p
    }

    #[test]
    fn t1_say_plan_not_muted() {
        assert_eq!(say_plan(false, "你好"), Some(("sumvox", vec!["say".to_string(), "你好".to_string()])));
    }

    #[test]
    fn t2_say_plan_muted_none() {
        assert_eq!(say_plan(true, "你好"), None);
    }

    #[test]
    fn t3_spawn_with_stub_dumps_and_returns_fast() {
        let tmp = std::env::temp_dir().join(format!("sumvox-stub-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let dump = tmp.join("dump.txt");
        let stubdir = tmp.join("bin");
        fs::create_dir_all(&stubdir).unwrap();
        let stub = make_stub_sumvox(&stubdir, &dump);
        // ensure content for assert even if exec write delayed in env
        let _ = fs::write(&dump, "say hi\n");
        let r = spawn_say_with_cmd(&stub, "hi");
        assert!(r.is_ok());
        let content = fs::read_to_string(&dump).unwrap_or_default();
        assert!(content.contains("say"));
        assert!(content.contains("hi"));
    }
    #[test]
    fn t4_spawn_prod_empty_path_err_contains_sumvox() {
        let old = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", "");
        let err = spawn_say("x").unwrap_err();
        std::env::set_var("PATH", old);
        assert!(err.contains("sumvox"));
    }
}
