# Shikigami — Project Context

Handoff from the SumVox session where this was scoped (2026-07-09). Read this
first when opening a fresh session here.

## What it is

A **desktop-floating AI assistant** that is a thin **agent aggregator**: one
floating avatar + voice front-end, backed by a thin wrapper that fronts many
backend agents (Claude Code, Codex, OpenClaw, Hermes, …). You talk to one face;
it routes the request to the right backend and speaks the result back.

Metaphor (drives the mental model): you = onmyōji; the app = the interface you
summon from; each backend agent = a *shikigami* (式神, summoned servant-spirit).

## The real substance (not the face)

The moat is **not** the avatar. It's:
- **adapter** — normalize heterogeneous agent interfaces into one `agent` trait.
  Known shapes: Claude Code = **PTY-driven** (keeps subscription billing —
  programmatic/SDK driving bills separately; only interactive TUI counts toward
  the subscription), Codex/Hermes = API, OpenClaw = TBD.
- **router** — decide which backend a given utterance/task goes to.

The avatar + voice is the *skin*. Build the adapter+router as the spine.

## Component map

| Role  | Component        | Where | Notes |
|-------|------------------|-------|-------|
| brain | adapter + router | this repo | the actual product |
| mouth | **SumVox**       | `../SumVox` | Rust CLI, summarize + TTS **only** (no STT). Used as a tool. |
| ears  | local STT        | this repo / TBD | Mac-local |
| hands | **shikigami-bridge** | `../shikigami-bridge` | the former `musingfox/shikigami`, renamed. Bun/TS, browser + macOS control, **already exposes MCP** → consume it as a tool, don't rebuild. |

## Avatar direction (already explored, decisions made)

- Soul: **cozy lofi-companion** — idle "doing its own thing", reacts when a
  message arrives. Not a geometric orb, a character.
- Tech: **pre-baked AI video** (grok image→video) for idle-loop + reaction
  clips, ffmpeg ping-pong for seamless loop. Runtime = zero API, tiny files
  (~100–300KB). Beats Live2D for "idle + a few reactions"; Live2D only if we
  need lots of live expression later.
- Lip-sync: audio-amplitude (RMS) → mouth crossfade from just 2 frames gives
  real lip-sync without a full rig.
- grok Imagine API details + the SumVox-session memory notes live under the
  SumVox project memory (`grok-imagine-api`, `avatar-rendering-direction`).

## Stack lean (not locked)

**Tauri** — shared Rust core + shared web UI + per-platform native shells.
System webview per platform makes the web rendering portable, so avatar tech
becomes a scheduling choice, not a platform blocker. Cross-platform but
performance-conscious: share a core, then optimize per platform.
Confirm before scaffolding.

## Naming decisions (settled)

- Product = **shikigami** (umbrella). crates.io `shikigami` is free.
- The former `musingfox/shikigami` (control tool) → renamed **shikigami-bridge**,
  folded in as the "hands". Local dir already renamed; GitHub repo rename +
  new remote for this repo still **pending** (outward action, needs go-ahead).
- Rejected: tama/tamago/kodama/tomo (collisions or wrong soul — "pet you raise"
  ≠ "aggregator that summons a fleet"). tachi/tachikoma fit the hive-of-agents
  idea but shikigami won.

## Next (M0)

Milestones: **M0** skeleton (repo + Tauri + hotkey + tray) → M1 single-turn
voice (STT → one backend → TTS) → M2 multi-tool brain (adapter+router,
CC via PTY) → M3 avatar → M4 per-platform optimize.

Open before building: (1) confirm Tauri, (2) define the `agent` adapter trait
+ router contract, (3) atomize M0.
