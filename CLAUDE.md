# Shikigami — Project Context

Handoff from the SumVox session where this was scoped (2026-07-09). Read this
first when opening a fresh session here.

## What it is

A **desktop-floating voice companion**. One floating avatar + voice, that:
1. **Observes** your running agents and reports progress ("CC finished the task")
   — voice + an avatar reaction. Like SumVox's Stop hook, embodied & persistent.
2. **Takes voice commands** (Siri-like): you speak → it does something → speaks back.

It is **NOT an agent engine**. It does not run/host coding agents. Agents run
where they always run (your terminals); shikigami is the face that watches them
and, when asked, pokes them.

Metaphor: you = onmyōji; each backend agent = a *shikigami* (式神) you already
summoned; this app is how you hear from and speak to them.

## Two channels (the whole architecture)

```
① OBSERVE (inbound)
   agent event ─▶ shikigami ─▶ avatar reaction + spoken report
   ("有訊息才有反應" lofi model; the "message" = an agent turn finishing)

② COMMAND (outbound · the Siri part)
   you speak ─▶ STT ─▶ brain (LLM + tools) ─▶ act ─▶ speak back
   acting = send a prompt into an agent, or drive the Mac via shikigami-bridge
```

## Agent I/O — how we watch & poke agents

Two directions, per agent. **herdr covers BOTH for pane-based agents** and is
the recommended integration; native hooks / ACP are precise per-agent alternates.

| Agent | OBSERVE | COMMAND |
|-------|---------|---------|
| Claude Code | `Stop`/`Notification`/`TaskCompleted` hook (SumVox already does this) — cmd hook, JSON on stdin | inject into live TUI pane: **tmux `send-keys`** or **herdr `agent send`/`pane run`** |
| Codex CLI | `notify` (`agent-turn-complete`) in `~/.codex/config.toml` | herdr / send-keys (or ACP if it speaks it) |
| Hermes (NousResearch) | `on_session_end` hook in `cli-config.yaml` | herdr / send-keys |
| ACP agents (gemini --acp, …) | their events / herdr | **`session/prompt`** JSON-RPC direct |
| OpenClaw | ⚠️ only cron-webhook, no lifecycle stream | herdr / send-keys |

- **herdr** (`herdr.dev`, github.com/ogulcancelik/herdr): a Rust "tmux for AI
  agents". Auto-detects CC/Codex per pane, tracks idle/working/blocked/done,
  exposes a **local Unix-socket JSON API + CLI, bidirectional** (subscribe to
  state events; `agent.send`/`pane.send_text`/`pane.send_keys` to inject). It
  already IS the "monitor + poke many agents" layer — **consume it, don't
  rebuild**. Cost: user must run agents inside herdr; young project, verify
  socket-API stability. `agent send` auto-submit (trailing newline) unverified —
  check `herdr agent send --help`.
- **tmux `send-keys`**: rock-solid universal fallback — writes to the pane's
  live pty *as if typed*, does NOT spawn a new process. This is why it's the
  clean way to drive CC (see billing below).
- zellij is weak for us: plugin events are in-process wasm only, no external
  event subscription, no silence detection.

## Claude Code billing — do NOT drive it programmatically

Confirmed & sourced (memory [[claude-code-agent-sdk-billing]]): since 2026-06-15,
`claude -p` / Agent SDK / ACP (`claude-agent-acp`) all draw from a **separate
Agent SDK credit**, NOT the subscription pool. Only the **interactive TUI**
(no `-p`) bills to subscription.

→ Therefore shikigami **never spawns CC programmatically** (that includes
TanStack AI's `claudeCodeText` harness — it uses the Agent SDK, so it would burn
the separate credit). To send CC a command, we **inject into the user's live
interactive pane** (send-keys / herdr) — that stays interactive = subscription.

## SumVox's role

SumVox (`../SumVox`) = Rust CLI, summarize + TTS **only** (no STT). It stays the
standalone CC-hook notifier. shikigami reuses its hook/transcript model; whether
the new app calls SumVox as a tool or does summarize+TTS itself is open (TanStack
AI or SumVox both cover summarize+TTS).

## TanStack AI — voice-brain candidate ONLY

Its coding-agent sandbox/harness layer (`claudeCodeText`, `acpCompatible`, etc.)
is **out of scope** — we don't run agents, and its CC harness breaks subscription
billing. What's still a candidate for channel ②'s brain: `chat()` + tools +
built-in **MCP client** (→ plug shikigami-bridge) + **realtime voice / TTS /
transcription** (OpenAI, ElevenLabs). Caveat: its STT is cloud; user wants
**local (Mac) STT** → likely a custom whisper.cpp adapter. TS/web → pairs with
Tauri, not with Native SDK. Undecided; revisit when scoping channel ②.

## Avatar direction (decided)

- Soul: cozy **lofi companion** — idle "doing its own thing", reacts when an
  agent event arrives. A character, not an orb.
- Tech: **pre-baked AI video** (grok image→video) idle-loop + reaction clips,
  ffmpeg ping-pong for seamless loop. Runtime = zero API, tiny files (~100-300KB).
  Live2D only if we later need lots of live expression.
- Lip-sync: audio-amplitude (RMS) → mouth crossfade from 2 frames.
- API details in SumVox project memory ([[grok-imagine-api]],
  [[avatar-rendering-direction]]).

## Shell / stack

**Tauri** lean (shared Rust core + web UI + per-platform shells; system webview
makes web rendering portable). Confirm before scaffolding.

**Omarchy / Hyprland (Wayland) caveats** — floating companion is well-supported
(`float`+`pin` frees it from tiling; place via `move` rule), but:
- Wayland clients **can't self-position** — `set_position` is a no-op; placement
  = compositor `windowrule` or user drag. Persisting a dragged position is awkward.
- **Global hotkey** must be bound in Hyprland config (`bind =`), not registered
  by the app (Tauri global-shortcut is X11-only). Fits the "hotkey activates it" plan.
- WebKitGTK transparency on Wayland is flaky → `WEBKIT_DISABLE_DMABUF_RENDERER=1`.
- Avatar video codec: WebKitGTK uses system GStreamer → also bake a **VP9/webm**.
- tray via SNI — works (Omarchy runs waybar).

**Native SDK** (native-sdk.dev, Vercel's Zig native-UI toolkit, no WebView):
would erase the WebKitGTK/Wayland pain, but (a) Zig = a 3rd language, (b) brand
new, (c) ⚠️ unverified whether it can render our video/canvas avatar (it's a
widget toolkit). On the radar; spike later, gated on the avatar question.
NB: native-sdk.dev ≠ Vercel AI SDK (different products).

## Naming (settled)

- Product = **shikigami** (umbrella). crates.io `shikigami` is free.
- Former `musingfox/shikigami` (browser/macOS control tool) → renamed
  **shikigami-bridge** (`../shikigami-bridge`), the "hands", already speaks MCP.
  Local dir renamed; **GitHub repo rename + this repo's new remote still PENDING**
  (deferred by user — do the new project first, rename later).
- Rejected: tama/tamago/kodama/tomo (collisions or "pet you raise" ≠ this).

## Next (M0)

M0 skeleton (repo + Tauri + hotkey-via-Hyprland + tray) → M1 channel ① (observe
CC via hook/herdr → avatar reaction + spoken report; reuse SumVox) → M2 channel ②
(voice command: STT → brain → send-keys/bridge) → M3 avatar polish → M4 per-platform.

Open before building: (1) confirm Tauri, (2) herdr-first vs native-hooks-first
for channel ①, (3) scope channel ② (and whether TanStack AI powers its brain),
(4) atomize M0.
