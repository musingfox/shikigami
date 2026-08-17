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

Which engine the brain runs on was settled **2026-08-17: build it here**, not
adopt Hermes / OpenClaw / OpenAB — see `pm/shikigami/docs/core-brain-open-question.md`
(now CLOSED) and the evidence in `pm/shikigami/docs/resident-agent-survey.md`.
Both candidates *can* be driven programmatically (OpenAI-compatible endpoints,
built-in tools disableable), so the decision rested elsewhere: the sub-second
dispatch path has no published latency figures and OpenClaw has no documented
way to skip its per-run prompt assembly; adopting drags in a Python or Node
runtime where today there is none; and confirm-before-inject would move its
enforcement point into a program that doesn't know the rule exists. **The
untested crux, if this is ever reopened**: does a `tools` array POSTed to their
`/v1/chat/completions` come back as `tool_calls`? If not, confirm-gated inject
cannot be expressed through an external brain at all.

### Two axes — 1:1:N only describes the first

| | manage agents | local affairs (computer use) |
|---|---|---|
| subject | the N agents | the machine itself |
| port | roster + send | MCP tools |
| adapter | herdr (today) → tmux / zellij / ACP | shikigami-bridge |

The second axis has no N, so the 1:1:N formula cannot express it. Reading
1:1:N as the whole picture is a mistake — half the point is that shikigami
helps with what you do on this computer, not only with what your agents do.

**Repo boundary (2026-08-10):** this repo owns the **core** and **how it
interacts with the human and with agents**. Every tool — the hands — lives in
`../shikigami-bridge`. So the MCP *client* belongs here (the core has to be
able to reach for a tool); no tool *implementation* does.

A shell-capable CLI is not disqualified from the second axis: `bash` + files
+ MCP is a general computer interface, not a coding-specific one, and
shikigami-bridge's own README puts `Claude / MCP client` at the top of its
architecture — it was built to be driven this way. Tool *shape* therefore does
not discriminate between core candidates; what does is on the two axes above
plus latency, memory, and the pane rule.

Metaphor: you = onmyōji; each backend agent = a *shikigami* (式神) you already
summoned; this app is how you hear from and speak to them.

## Two channels

```
① OBSERVE (inbound)         agent event → avatar reaction + spoken report
                            DONE incl. depth (R-observe, 2026-08-16): the brain
                            gets the hook's own words + a pane excerpt, so it
                            answers "stuck on WHAT", and says 不知道 when it has
                            nothing rather than inventing a source
② COMMAND (outbound)        you speak → STT → brain → act (inject) → speak back
                            DONE: answer 2026-07-15, targeted inject + summon
                            (R2a-d, 2026-07-26/08-12) — confirm before both
```

## Current state (2026-08-17)

**R0–R2, R-observe, R-memory and R-tool-loop increment 1 are built and
verified.** Both channels are closed loops: it tells you what each agent is doing
*and why it is stuck*, and you can talk back to a named agent or summon a new one
— each behind a confirm. It remembers across a restart. And it can now *look
something up and then answer* — a real multi-step tool-use loop, verified live.

What it still is **not**: an agent that acts on its own. **Nothing wakes it but
your voice** (`r-brain-event-driven`), and it still cannot search its own memory
or write to it — `recall`, the curated-layer `memory` writer and `/context` are
increments 3–4 of `r-brain-tool-loop`, still open. (Increment 2 landed the same
day: an utterance no longer costs a pane read.)

Shell and voice Q&A:

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

Seeing and speaking to N (R1/R2, 2026-07-23 → 08-12):

- **herdr adapter** (`herdr.rs`, 2s poll of `agent.list`) → `agent:roster` /
  `agent:status`; **CC hooks source** (`cchooks.rs`, 500ms spool tail) →
  `agent:activity`. Roster bubbles arc around the orb, idle ones collapse
  behind `+N`
- **Targeted talk** (R2a): bubble 🎙 → transcribe → editable confirm →
  `agent.prompt`. **Voice summon** (R2d): confirm → herdr tab → type `claude`
  in the pane → brief it. Never `agent.start` — see the summon note below
- Brain has the live roster in its system prompt + 6 turns of memory
  (durable since R-memory, 2026-08-17 — see below)

Depth of observation (R-observe, 2026-08-16, `main` @ `f352e16`):

- **`depth.rs`** — on-demand only, never on a polling path. Per utterance it
  takes the hook's precise signal (`notification.message` /
  `stop.last_assistant_message`, parsed in `cchooks.rs`, snake_case with
  camelCase fallback) plus, for ≤3 blocked/working agents, a `pane.read`
  excerpt. Bounded: 6 lines/200 chars precise, 12/600 screen, 2000 global,
  truncated by char with the tail kept.
  **Superseded 2026-08-17 (increment 2):** the `pane.read` half is gone from this
  path — it is the `read_pane` tool now, so an utterance costs no pane read at all.
  Only the precise half is still prefetched, and every one of those numbers now
  lives in `budget.rs`
- `render_roster` now marks status as herdr's screen *inference*, and the depth
  block is fenced as observed output, not instruction
- The join key is herdr's `pane_id` == the hook's `HERDR_PANE_ID`. **Both halves
  are now verified live 2026-08-17** — `vdp_t2` used to pass on the screen half
  alone, which is precisely why the hook half had no receipt; removing the screen
  prefetch left it nowhere to hide, and it now asserts `precise.is_some()` **and**
  `screen.is_none()`. Receipt: `HERDR_PANE_ID=w7Q:p1 herdr pane_id=w7Q:p1` with
  `精確訊號：permission_prompt：Claude needs your permission` in the prompt. The
  archived ticket's "未收齊的收據" is closed. Note the mechanism: a test binary runs
  neither the roster poll nor the spool tailer, so a live test must read both
  itself (`herdr::fetch_roster_now`, and `record_depth` over `spool_path`)
- `AgentEntry` and `events.ts` deliberately untouched: depth reaches the brain
  only. Showing it in the UI is a separate, unopened ticket

Memory that survives a restart (R-memory, 2026-08-17):

- **`memory.rs`** — three layers, two files under `config::config_dir()`
  (`~/.config/shikigami/`, created once at startup):
  **conversation** (last 6 turns, replacing the old in-process `Mutex`),
  **fact** (append-only action log), **curated** (`MEMORY.md`, hand-written)
- **The fact layer is machine-written, never model-written.** Each confirmed
  inject / summon writes one row from data already at hand — no model call, no
  extraction from transcripts; an unknown field (e.g. an agent name the live
  roster can't confirm) is **omitted, never guessed**. R-observe proved this
  brain invents sources when it has none; the fact layer is designed so it has
  nothing to invent. Fact rows never enter the prompt — they await R-tool-loop's
  `recall`
- `MEMORY.md` is injected into every prompt as **trusted** text (no observation
  fence — that fence exists so agent output can't read as instruction, and this
  file is the user's own words). Absent / blank / unreadable → prompt byte-identical
- `memory.jsonl` rotates at 1 MiB keeping one generation. A **failed** rotation
  logs and still appends: memory continuity beats a hard bound, so the cap is
  soft in that case, deliberately
- `SHIKIGAMI_CONFIG_DIR` relocates the whole config root (tests + advanced use).
  Not a profile mechanism: it moves `models/` and `hooks.ndjson` while
  `scripts/cc-hook.sh` still writes the real path, silently killing channel ①
- Not done here, by design: `recall` / tool-use loop / model-written curated
  memory (`r-brain-tool-loop`), heartbeat (`r-brain-event-driven`), UI surfacing

The loop that makes it an agent (R-tool-loop increment 1, 2026-08-17):

- **`backend.rs` is the port**: a `Backend` trait whose one verb advances the turn
  by a step and answers in provider-neutral values (`StepAction` / `ToolResult`),
  plus provider selection and credentials. Two adapters own every wire key —
  `backend_gemini.rs` (native `functionDeclarations`) and `backend_text.rs`
  (openai + anthropic through the existing prose JSON-action path, `tools()` empty,
  so their behaviour is unchanged and the loop cannot tell them apart).
  **`brain.rs` names no provider** — the four old leak points (`build_request`,
  `extract_reply`, the URL/header match, `Provider::model`) are gone from it
- Bounds: **5 steps, 45s wall clock**, checked before every model step *and* every
  tool call; the per-request HTTP timeout is `remaining.min(30s)`, so a request can
  never outlive the turn. Below **`MIN_STEP_BUDGET` (5s)** the turn counts as out
  of time — derived, not picked: the same floor guards the tool phase and one herdr
  read blocks up to `CALL_TIMEOUT` (5s). Exhaustion **speaks** what it consulted
  and that it has no conclusion; it must never go silent or invent
- `Speak` is a variant of the step type but a bare text part on the wire, so the
  fast path still answers in one request. **`Summon` is loop-terminal** — the loop
  module cannot reach `herdr::summon`/`prompt_agent` at all, so the confirm is
  non-bypassable *by construction*, not by policy
- `read_pane(agent)` wraps `pane_recent_text` + `normalize(_, SCREEN_MAX_*)` on
  `spawn_blocking`, agent resolved against the turn's roster snapshot (never a
  fresh socket), result fenced as observed output
- `consulted` carries only panes actually reached, under the **roster's** name. A
  failed read or a name the model invented is told to the model but never spoken
  as a source — R-observe's invented-source defect reappeared here in new clothes
  and the `PaneRead { text, consulted }` contract is what closes it
- One `verb:"turn"` row per utterance regardless of step count; tool calls and
  results are the on-demand layer and never persist. Single-attempt/no-failover is
  **kept deliberately** and pinned by a test — not changed inside a refactor

The fast path stops paying (increment 2, 2026-08-17):

- The screen half of depth left the prefetch path, so **an utterance costs zero
  pane reads** — that is what makes the ticket's "the loop moves the cost to the
  turn that needs it" true; until this landed, both paths ran and the fast path
  paid twice. The precise half stays prefetched: it reads the hook spool already in
  memory, costs nothing, and it is what answers "stuck on WHAT"
- `select_panes` and its `take(3)` are gone with it, and `roster_and_depth_with`
  no longer blocks at all (roster is herdr's cache, precise is the spool — two mutex
  reads), so `lib.rs` dropped the `spawn_blocking` around it
- **`budget.rs` is the one page** of context budgets, grouped by the four layers.
  `AgentDepth::screen` is kept though nothing populates it: `render_roster` still
  renders it, and that is the slot an event-driven turn would fill
- Known gap, **not** caused here: `dai_live`'s "no source → must say 不知道" assertion
  is flaky against the live model (~1 in 5). The request on that path is
  byte-equivalent to `404f40c`'s, so R-observe's central invariant is *unlikely*
  rather than *closed* — worth its own ticket

## Architecture

See `ARCHITECTURE.md` — hexagonal, one core event contract: 8 constants in
`src-tauri/src/events.rs` (`agent:speech` / `agent:report` / `agent:roster` /
`agent:status` / `agent:activity` / `voice:muted` / `voice:listening` /
`voice:transcript`), hand-mirrored in `src/events.ts` — **no codegen, no
drift check**, so the two files are kept in sync by discipline alone. Iron
rules:

1. Frontend subscribes to core events only; adapter formats never cross into TS.
2. Everything SumVox-specific lives inside `src-tauri/src/sumvox.rs`; likewise
   herdr's wire shape inside `herdr.rs`. Depth normalization (`depth.rs`) works
   on core types only — that is why the live-roster helper it needed returns
   `Vec<AgentEntry>` rather than exposing herdr's `call`. The same rule holds for
   model providers: `backend.rs` is the port, every request/response key lives in
   `backend_gemini.rs` or `backend_text.rs`, and `brain.rs` names no provider.
3. New agent source = one Rust module normalizing into core events + one
   registration line in `lib.rs`. Nothing else changes.

## Load-bearing constraints (do not relearn these)

- **Never spawn Claude Code programmatically.** (2026-07-23 re-verified: the
  2026-06-15 billing change — `claude -p` / Agent SDK / ACP to a separate
  Agent SDK credit — is **paused**, everything currently bills to the
  subscription; Anthropic says it is re-planning.) Decision unchanged, for
  two reasons: positioning — ACP-spawned agents have no TUI, so shikigami
  would own work with no pane you can take over, which is exactly what the
  surviving rule above forbids; and the billing risk if the pause lifts.
  Channel ② injects into the user's live pane, never spawns.
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

Live (`#[ignore]`) tests need a provider key and run against the real herdr /
provider: `ANTHROPIC_API_KEY=… cargo test sap9_live -- --ignored --nocapture`.
The tool loop has its own, and it needs herdr up with at least one live agent:
`GEMINI_API_KEY=… cargo test gemini_live -- --ignored --nocapture` — it prints the
whole turn, so you can read whether step 1 really returned a `functionCall` and
step 2 really answered from the pane. They relocate the config root to a temp dir,
so they no longer touch your real `memory.jsonl` — the key **file** is carried
over, so `~/.config/shikigami/` keys keep working too. `cargo test` alone can
never prove live behaviour; that is exactly how R-observe's "the brain invents a
source" defect got through, **and the same defect grew back in the tool loop** as
a failed pane read counting as consulted — offline tests were green for it.

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
- **R-observe — depth of what it sees** — **DONE 2026-08-16** (`f352e16`).
  The premise held: the depth was already arriving and being thrown away —
  `cchooks::parse_line` kept kind/session/pane/ts and dropped the payload that
  carried "Claude needs your permission". Fixing that cost zero new IO;
  `pane.read` was only needed for the cold-start gap. Two findings worth
  keeping: the brain will **invent** a source when it has none unless the
  prompt forbids it (only a live test caught that — `cargo test` never can),
  and 3.4% of hook lines carry content but no pane, so they can never be
  attributed. Ticket archived at `pm/shikigami/archive/r-observe-depth.md`.
- **R-memory — memory that survives a restart** — **DONE 2026-08-17**. The
  premise held: durability was never about model tier. What the build actually
  settled is *where memory comes from* — **the fact layer is machine-written**
  (every row from data already present after a confirm), because a model-written
  log reintroduces exactly the invention R-observe closed. Survey of Hermes /
  OpenClaw (`resident-agent-survey`) added the second half: memory is **two
  tiers** — small curated always-injected, plus episodic that is search-only and
  never bulk-injected. Storage is format-stable across the still-open loop
  question, which is why this shipped without it. Ticket
  `r-brain-durable-memory`; follow-ups in `memory-read-and-durability-polish`.
- **R-tool-loop — the loop that makes it an agent** — **increments 1 and 2 DONE
  2026-08-17**, split into four at the interface boundary (a human call: 11
  criteria over 5 files was too much for one review). Increment 1: the `Backend`
  port + two adapters, the 5-step/45s loop, gemini native function calling,
  Speak/Summon as tools with Summon loop-terminal, and a thin `read_pane` — pulled
  forward because a loop whose only tools are terminal can never take a second
  step, so criterion 4 would have had no real receipt. Increment 2: the screen half
  off the prefetch path (an utterance now costs zero pane reads), every context
  budget in `budget.rs`, and the no-write-back proof. Still open: **increment 3**
  `recall` + the `memory` curated writer and its four guardrails; **increment 4**
  `/context`. Findings worth keeping:
  `thinkingBudget: 0` does **not** break gemini-2.5-flash function calling
  (live-verified — no offline test could have said either way), and **R-observe's
  invented-source defect grew back in the new code** as "a failed pane read still
  counted as consulted", caught only by a post-hoc review pass, which is the second
  time this brain has had to be stopped from naming a source it never had. And a
  third thing, from increment 2: **a live test can pass on the wrong half** —
  `vdp_t2` had been standing in as the pane-id join's receipt while only ever
  exercising the screen excerpt, and deleting that half is what revealed the hook
  half was unchecked — the archived ticket said so in writing and it still read as
  covered. Ticket
  `r-brain-tool-loop`.
- **R-event-driven — it wakes on the world, not only on you** (2026-08-16):
  hooks events reach the brain's judgement, with a silence-by-default rule and a
  heartbeat timer **inside the existing Tauri process** — no gateway daemon;
  unlike Hermes/OpenClaw we are already resident. Ticket `r-brain-event-driven`.
- **R-hands — reach the second axis** (2026-08-10, previously absent from the
  roadmap entirely — `bridge`/`MCP` appear 0 times in this repo's code and
  once in this file, under Related repos): an MCP client so the core can
  actually pick up shikigami-bridge's tools, and the contract between the two.
  Tool implementations stay in `../shikigami-bridge` per the repo boundary
  above. Blocked on nothing; leverage here likely exceeds the core choice,
  since any shell+MCP engine can only do what the hands can do.
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
