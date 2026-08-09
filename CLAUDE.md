# Shikigami — Project Context

Desktop-floating voice companion. Positioning: **1:1:N** — one human, one
shikigami, N coding agents. One floating avatar that **observes** your agents
(reports progress by voice + reaction), answers questions about them, and
relays your **voice commands** to them.

Shikigami **is** an agent — it is the 1, and the target is JARVIS: enough
intelligence and memory to consolidate what every agent (local or online) is
doing and to dispatch them onward. It is **not** an agent engine — it never
runs the N. Those are two different claims and an earlier wording ran them
together.

**The rule that survives: shikigami never becomes one of the N.** It never
owns work without a pane you can take over. R2d's summon boundary and the
`claude -p` / Agent SDK / ACP-spawn ban both hang off this rule — "shikigami
is an agent" is not a licence to spawn.

Which engine the brain runs on is **open**, see
`pm/shikigami/docs/core-brain-open-question.md` (2026-08-10).

Metaphor: you = onmyōji; each backend agent = a *shikigami* (式神) you already
summoned; this app is how you hear from and speak to them.

## Two channels

```
① OBSERVE (inbound)         agent event → avatar reaction + spoken report
                            DONE for coarse status (idle/working/blocked);
                            "what is it stuck ON" needs R-observe
② COMMAND (outbound)        you speak → STT → brain → act (inject) → speak back
                            STT/brain/speak DONE 2026-07-15 (answer-only); act lands in R2
```

## Current state (2026-07-23)

M0 skeleton + M1 channel ① + M2 voice Q&A are **built and verified**:

- Tauri 2, vanilla TS + Vite (bun), deps = tauri + global-shortcut plugin only
- Transparent undecorated always-on-top window, whole-window drag,
  all-workspaces; tray (Mute / Recent / Open Config / Quit); hotkey
  Cmd+Ctrl+S toggles visibility (macOS)
- Canvas blob avatar (48-pt Catmull-Rom, ported from SumVox
  `prototype/orb-canvas.html`), idle 20fps dim / speaking 60fps bright
- RMS lip-sync: `agent:speech` audio → WebAudio → 40ms envelope → mouth
- Typewriter toast placed diagonally between avatar and screen center
- Channel ① source: **SumVox file IPC** (`~/.config/sumvox/`), 500ms poll
- Voice Q&A (merged 2026-07-15, PR #1): Cmd+Ctrl+M PTT or double-click orb →
  getUserMedia 16kHz → whisper-rs (Breeze-ASR-25 q5_k, zh-pinned) → brain
  (Anthropic/Gemini/OpenAI, cheapest-first) → reply via history.log +
  `sumvox say`. Answer-only — no injection yet
- Radial settings menu (right-click orb), orb gestures: double-click = talk,
  press+hold = drag

## Architecture

See `ARCHITECTURE.md` — hexagonal, one core event contract
(`agent:speech` / `agent:report` / `voice:muted`, defined in
`src-tauri/src/events.rs` + mirrored in `src/events.ts`). Iron rules:

1. Frontend subscribes to core events only; adapter formats never cross into TS.
2. Everything SumVox-specific lives inside `src-tauri/src/sumvox.rs`.
3. New agent source = one Rust module normalizing into core events + one
   registration line in `lib.rs`. Nothing else changes.

## Load-bearing constraints (do not relearn these)

- **Never spawn Claude Code programmatically.** (2026-07-23 re-verified: the
  2026-06-15 billing change — `claude -p` / Agent SDK / ACP to a separate
  Agent SDK credit — is **paused**, everything currently bills to the
  subscription; Anthropic says it is re-planning.) Decision unchanged, for
  two reasons: positioning — ACP-spawned agents have no TUI, which conflicts
  with "agents run in *your* terminals"; and the billing risk if the pause
  lifts. Channel ② injects into the user's live pane, never spawns.
- **Summon boundary (R2d).** Voice summon opens a herdr tab in the project
  directory and starts claude in that pane (`tab.create` → `agent.start` →
  `agent.wait` → `agent.prompt`). This does not weaken the rule above: the new
  agent is an ordinary interactive TUI the user can take over, attach to, or
  Ctrl-C — shikigami opened a terminal on their behalf, exactly as they would
  have. What stays banned is the headless path: `claude -p`, the Agent SDK, and
  ACP spawns, none of which leave a pane to take over. A summon is always
  preceded by an explicit confirm.
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

Transport = one port (roster + send), multiple adapters (2026-07-23 research):

- **herdr** (`herdr.dev`) — standalone multiplexer (vendored portable-pty,
  NOT built on tmux → `tmux send-keys` cannot reach herdr panes). Socket API
  (NDJSON over Unix socket): `agent.list`, `pane.send_text`,
  `events.subscribe` (coarse idle/working/blocked, screen-snapshot based —
  precise events come from hooks, not herdr). v0.x: isolate behind the port.
- **tmux** — `send-keys` / `list-panes`; biggest install base, R3.
- **zellij** — usable since **v0.44.0** (2026-03): `action write-chars
  --pane-id`, `list-panes --json`, `subscribe`; CLI subprocess only, no
  socket. (Old "unsuitable" verdict is obsolete.) R3.
- **ACP** — spawn model (stdio JSON-RPC), for *background* non-CC agents
  (25+ agents ecosystem), R4. **A2A** — watch list only: mature spec (v1.0,
  Linux Foundation) but no coding-agent adoption (Gemini CLI experimental
  only, 2026-07).

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

## Roadmap (R-series, 2026-07-23 — supersedes old M2–M4 numbering)

Ordering logic: identity is the dependency of everything → see N first, then
speak to N, then breadth. Avatar/Linux are off the critical path.

- **R0 — debt**: CLAUDE.md refresh (this), fix `VOICE_MUTED`
  double-subscribe, single source of truth for listening state.
- **R1 — see N**: herdr adapter (roster + coarse status → new core events
  `agent:roster` / `agent:status`) ∥ hooks source module (CC
  Stop/Notification direct, with session/pane id) → roster UI + attributed
  toasts. Accept: two CCs in herdr, shikigami tells who's working / who just
  reported. Atomize into task cards first (obw:pm, vault "obsidian",
  `pm/shikigami/`).
- **R2 — speak to N**: brain gets roster context → voice target resolution
  (ask when ambiguous, never guess) → inject via `pane.send_text` with
  **confirm-before-inject** → multi-turn brain memory. Accept: "叫X跑測試" →
  confirm → text lands in X's pane. Product thesis proven here.
- **R-observe — depth of what it sees** (2026-08-10, promoted out of R1):
  `AgentEntry` carries one 5-way status plus a terminal title, so the brain can
  say "A is stuck" but never "stuck on what". A stronger brain over that input
  still only produces a status board — depth is upstream of intelligence.
  Ticket `r-observe-depth`.
- **R-memory — memory that survives a restart** (2026-08-10, promoted out of
  R2's tail): brain history is 6 turns in a process `Mutex`, gone on restart.
  JARVIS needs yesterday's assignments, not the last six lines. Independent of
  model tier — a better model does not produce memory. Ticket
  `r-brain-durable-memory`.
- **R3 — transport breadth**: tmux + zellij (≥0.44 version check) adapters;
  port surface stays roster+send.
- **R4 — ecosystem/platform**: ACP adapter (background agents) — this is also
  where **online** agents land (OpenAB is a candidate adapter here, *not* a
  candidate brain); Linux/Hyprland (window rules, Hyprland hotkey bind,
  xdg-open in tray, Wayland caveats above).
- **R5 — polish**: pre-baked AI video clips replacing the canvas blob (grok
  image→video, details in SumVox project memory).
- **Watch (unscheduled)**: A2A, TanStack AI realtime voice.

## Related repos / naming

- `../SumVox` — Rust CLI, summarize + TTS, owns the CC hook. The "mouth".
- `../shikigami-bridge` — browser/macOS control, speaks MCP. The "hands".
  (GitHub repo renamed `shikigami` → `shikigami-bridge` on 2026-07-14;
  `musingfox/shikigami` now hosts this aggregator repo.)
- PM: Obsidian vault "obsidian", `pm/shikigami/` (tasks/, archive/, docs/).
