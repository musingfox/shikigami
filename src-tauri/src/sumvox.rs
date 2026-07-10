// Watches SumVox's file IPC dir (~/.config/sumvox/) and re-emits changes as
// Tauri events. shikigami is a second consumer of the same files; SumVox
// itself is untouched.
//
//   now_playing  → "sumvox:now-playing"  payload = audio file path
//   history.log  → "sumvox:history"      payload = last non-empty line
//   muted        → "sumvox:muted"        payload = bool (file exists)

use std::{fs, path::PathBuf, thread, time::Duration};
use tauri::Emitter;

// ponytail: 500ms stat poll over 3 files; switch to vnode/notify if it ever matters
const POLL: Duration = Duration::from_millis(500);

fn last_line(path: &PathBuf) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    s.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(str::to_string)
}

pub fn spawn_watcher(app: tauri::AppHandle) {
    thread::spawn(move || {
        let dir = PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".config")
            .join("sumvox");
        let np = dir.join("now_playing");
        let hist = dir.join("history.log");
        let muted = dir.join("muted");

        // baseline: don't replay pre-existing state on startup, except muted
        let mut np_mtime = fs::metadata(&np).and_then(|m| m.modified()).ok();
        let mut hist_len = fs::metadata(&hist).map(|m| m.len()).unwrap_or(0);
        let mut is_muted = muted.exists();
        let _ = app.emit("sumvox:muted", is_muted);

        loop {
            thread::sleep(POLL);

            let m = fs::metadata(&np).and_then(|m| m.modified()).ok();
            if m != np_mtime {
                np_mtime = m;
                if let Ok(path) = fs::read_to_string(&np) {
                    let path = path.trim().to_string();
                    if !path.is_empty() {
                        eprintln!("[sumvox] now-playing: {path}");
                        let _ = app.emit("sumvox:now-playing", path);
                    }
                }
            }

            let len = fs::metadata(&hist).map(|m| m.len()).unwrap_or(0);
            if len != hist_len {
                hist_len = len;
                if let Some(line) = last_line(&hist) {
                    eprintln!("[sumvox] history: {line}");
                    let _ = app.emit("sumvox:history", line);
                }
            }

            let mu = muted.exists();
            if mu != is_muted {
                is_muted = mu;
                eprintln!("[sumvox] muted: {mu}");
                let _ = app.emit("sumvox:muted", mu);
            }
        }
    });
}
