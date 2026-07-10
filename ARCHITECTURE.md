# Architecture

Hexagonal, sized for what exists today. The core is an event contract; every
outside system talks to it through an adapter.

```
inbound adapters              core contract               presentation
─────────────────            ──────────────              ─────────────
sumvox.rs (file IPC) ──┐                            ┌── avatar.ts  (blob, setLevel/setSpeaking)
                       ├──▶  agent:speech (path)  ──┼── lipsync.ts (RMS envelope → mouth)
future:                │     agent:report (Report) ─┼── toast.ts   (typewriter bubble)
  codex notify         │     voice:muted  (bool)    └── tray.rs    (menu; also re-emits reports)
  shikigami notify CLI ┘
  herdr / ACP
```

## The contract

Defined once per side, kept in sync by hand:

- `src-tauri/src/events.rs` — event names + `Report { source, ts, text }`
- `src/events.ts` — TS mirror

Rules that keep the core clean:

1. **Frontend subscribes to core events only.** No adapter-specific event
   names or formats (e.g. SumVox's `RFC3339\ttext` lines) ever cross into TS.
2. **Adapters own their own dirt.** Everything SumVox-specific — paths under
   `~/.config/sumvox/`, the history line format, the `muted` flag file — lives
   inside `sumvox.rs` behind its public fns (`recent`, `is_muted`, `set_muted`,
   `config_dir`, `spawn_watcher`).
3. **Presentation modules have one-verb APIs**: `initAvatar`, `setLevel`,
   `setSpeaking`, `lipsync(path)`, `toast(text)`. The composition root
   (`src/main.ts`) is the only place events meet presenters.

## Adding a new agent source

Write one Rust module with a `spawn_watcher(app)` (or socket handler) that
normalizes the source's activity into `agent:speech` / `agent:report` emits,
register it in `lib.rs::run`. Nothing else changes.

## Known simplifications (ponytail ledger)

- Tray's mute/recent go straight to the sumvox adapter; a port trait in front
  is deferred until a second notifier actually lands.
- `read_file` command is a generic byte-reader serving lip-sync decode; scope
  it down if the capability surface ever matters.
- Watcher is a 500ms stat poll, not vnode/notify.
