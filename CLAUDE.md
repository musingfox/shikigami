# Shikigami — Project Context

Desktop-floating voice companion. One floating avatar that **observes** your
coding agents (reports progress by voice + reaction) and — later — **takes
voice commands**. It is NOT an agent engine: agents run in your terminals;
shikigami watches them and, when asked, pokes them.

Metaphor: you = onmyōji; each backend agent = a *shikigami* (式神) you already
summoned; this app is how you hear from and speak to them.

## Two channels

```
① OBSERVE (inbound, DONE)   agent event → avatar reaction + spoken report
② COMMAND (outbound, M2)    you speak → STT → brain → act (send-keys / bridge) → speak back
```

## Current state (2026-07-10)

M0 skeleton + M1 channel ① are **built and verified**:

- Tauri 2, vanilla TS + Vite (bun), deps = tauri + global-shortcut plugin only
- Transparent undecorated always-on-top window, whole-window drag,
  all-workspaces; tray (Mute / Recent / Open Config / Quit); hotkey
  Cmd+Ctrl+S toggles visibility (macOS)
- Canvas blob avatar (48-pt Catmull-Rom, ported from SumVox
  `prototype/orb-canvas.html`), idle 20fps dim / speaking 60fps bright
- RMS lip-sync: `agent:speech` audio → WebAudio → 40ms envelope → mouth
- Typewriter toast placed diagonally between avatar and screen center
- Channel ① source: **SumVox file IPC** (`~/.config/sumvox/`), 500ms poll

## Architecture

See `ARCHITECTURE.md` — hexagonal, one core event contract
(`agent:speech` / `agent:report` / `voice:muted`, defined in
`src-tauri/src/events.rs` + mirrored in `src/events.ts`). Iron rules:

1. Frontend subscribes to core events only; adapter formats never cross into TS.
2. Everything SumVox-specific lives inside `src-tauri/src/sumvox.rs`.
3. New agent source = one Rust module normalizing into core events + one
   registration line in `lib.rs`. Nothing else changes.

## Load-bearing constraints (do not relearn these)

- **Never spawn Claude Code programmatically.** Since 2026-06-15, `claude -p`
  / Agent SDK / ACP bill to a separate Agent SDK credit, not the subscription.
  Only the interactive TUI bills to subscription → command channel ② must
  inject into the user's live pane (tmux `send-keys` / herdr), never spawn.
- **SumVox stays untouched.** shikigami is a second consumer of its files
  (`now_playing` = audio path, `history.log` = "RFC3339\ttext" lines,
  `muted` = flag file). SumVox's own CC Stop hook is what feeds channel ①.
- **macOS transparency** needs `macOSPrivateApi: true` + tauri
  `macos-private-api` feature (already set; blocks App Store, fine).
- **Wayland/Hyprland (M4)**: apps can't self-position or self-register global
  hotkeys (bind in Hyprland config); WebKitGTK transparency needs
  `WEBKIT_DISABLE_DMABUF_RENDERER=1`; also bake a VP9/webm if avatar ever
  becomes video.

## Agent I/O reference (for channel ② and future sources)

| Agent | OBSERVE | COMMAND |
|-------|---------|---------|
| Claude Code | SumVox hook (current) or `Stop`/`Notification` hooks | tmux `send-keys` / herdr `agent send` |
| Codex CLI | `notify` (`agent-turn-complete`) in `~/.codex/config.toml` | herdr / send-keys |
| Hermes | `on_session_end` hook in `cli-config.yaml` | herdr / send-keys |
| ACP agents | their events / herdr | `session/prompt` JSON-RPC |

herdr (`herdr.dev`) = tmux-for-agents with Unix-socket JSON API, covers both
directions for pane-based agents — consume it at the multi-agent phase, don't
rebuild. zellij is unsuitable (in-process wasm plugins only).

## Dev workflow

```sh
bun run tauri dev        # run app (auto-rebuilds Rust + TS)
scripts/demo.sh "text"   # fire a fake notification: toast + lip-sync + audio
bun run build            # tsc + vite build (frontend check)
cargo check / clippy     # in src-tauri/
```

Manual regression: run demo.sh → avatar mouth moves with audio, bubble types
out text toward screen center, tray Recent gains the entry, Mute toggle
creates/removes `~/.config/sumvox/muted`.

## Roadmap

- **M2 — channel ②**: local STT (whisper.cpp / whisper-rs) → brain (lean
  default: Rust direct Anthropic API chat loop; TanStack AI only if its
  realtime voice earns its keep) → act via send-keys / shikigami-bridge (MCP)
  → speak via SumVox. Not yet atomized — build task cards first (obw:pm,
  vault "obsidian", `pm/shikigami/`).
- **M3 — avatar polish**: possibly pre-baked AI video clips (grok
  image→video, details in SumVox project memory) replacing the canvas blob.
- **M4 — Linux/Hyprland**: window rules, Hyprland hotkey bind, xdg-open in
  tray, Wayland caveats above.

## Related repos / naming

- `../SumVox` — Rust CLI, summarize + TTS, owns the CC hook. The "mouth".
- `../shikigami-bridge` — browser/macOS control, speaks MCP. The "hands".
  (GitHub repo rename from `shikigami` still pending, deferred.)
- PM: Obsidian vault "obsidian", `pm/shikigami/` (tasks/, archive/, docs/).
