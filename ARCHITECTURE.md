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
   `config_dir`, `spawn_watcher`). The same rule holds for the model providers:
   `backend.rs` is the **port** — the `Backend` "advance the turn one step" trait,
   the provider-neutral `StepAction` / `ToolResult` values, provider selection and
   credentials — and every request/response key belongs to one of its two
   adapters, `backend_gemini.rs` (native function calling) and `backend_text.rs`
   (openai + anthropic through the prose JSON-action path). `brain.rs` keeps the
   prompt, the decision loop and the memory commit, and names no provider;
   `tools.rs` holds what the model may reach for during a turn.
3. **Presentation modules have one-verb APIs**: `initAvatar`, `setLevel`,
   `setSpeaking`, `lipsync(path)`, `toast(text)`. The composition root
   (`src/main.ts`) is the only place events meet presenters.

## Persistent state (memory)

`memory.rs` is **not an adapter** — it faces no outside system; it is the core's
own state, on disk so it survives a restart. Three layers, two files under
`config::config_dir()` (`~/.config/shikigami/`):

- `memory.jsonl` — append-only, one JSON object per line, `verb` distinguishes
  `turn` (conversation) from `inject` / `summon` (the fact layer). Rotates at
  1 MiB keeping one generation. **The fact layer is machine-written**: every row
  comes from data already present after the user's confirm — no model call, no
  extraction, and a field that isn't known is omitted rather than guessed.
- `MEMORY.md` — hand-written by the user, injected into every system prompt as
  trusted text. Absent / blank / unreadable leaves the prompt byte-identical.

Only `turn` rows reach the model on their own, as the rolling layer. Every row —
`turn`, `inject` and `summon` alike, across both generations — is searchable by
the `recall` tool: it costs nothing until the model asks for it, and what it
fetches lives for that turn only.

`SHIKIGAMI_CONFIG_DIR` relocates that whole root (tests, and advanced use). It
moves `models/` and `hooks.ndjson` too, while `scripts/cc-hook.sh` keeps writing
the real path — so it is not a profile mechanism.

**One deliberate exception to rule 2**: `sumvox::rfc3339_utc` is `pub(crate)` and
shared with `memory.rs`. It is a pure calendar function carrying none of SumVox's
format; a second copy would be worse than the borrow. Move it to a neutral module
if this ever stops being the only exception.

## Adding a new agent source

Write one Rust module with a `spawn_watcher(app)` (or socket handler) that
normalizes the source's activity into `agent:speech` / `agent:report` emits,
register it in `lib.rs::run`. Nothing else changes.

Depth normalization stays in the Rust brain only; the source wire shape remains in adapters.

## Known simplifications (ponytail ledger)

- Tray's mute/recent go straight to the sumvox adapter; a port trait in front
  is deferred until a second notifier actually lands.
- `read_file` command is a generic byte-reader serving lip-sync decode; scope
  it down if the capability surface ever matters.
- Watcher is a 500ms stat poll, not vnode/notify.
