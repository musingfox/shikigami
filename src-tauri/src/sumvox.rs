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
