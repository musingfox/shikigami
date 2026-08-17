# Shikigami

A desktop-floating AI companion — an **agent aggregator**.

One face on your desktop, many agents behind it. It watches your coding
agents and reports their progress with a voice and a living avatar; speak to
it and it directs the right agent to act, then talks back.

The name: in Onmyōdō a *shikigami* (式神) is a servant-spirit summoned to carry
out its master's will. Here each backend agent is a shikigami you summon; the
app is the interface you summon them from.

## Shape

```
① observe   agent event ─▶ avatar reaction + spoken report      (done)
② command   you speak ─▶ STT ─▶ brain ─▶ act ─▶ speak back      (done)

shikigami — floating avatar (Tauri 2)
   ├─ mouth : SumVox            (summarize + TTS)              → ../SumVox
   ├─ ears  : local STT (M2)
   └─ hands : shikigami-bridge  (browser + macOS control, MCP) → ../shikigami-bridge
```

## Run

```sh
bun install
bun run tauri dev        # floating avatar + tray + Cmd+Ctrl+S toggle
scripts/demo.sh "hello"  # fire a fake notification: toast + lip-sync
```

Status: **R0–R2, R-observe, R-memory done**, plus the tool-use loop
(`R-tool-loop` increments 1–2) — both channels closed, memory survives a restart,
and the brain can look something up before it answers.
Architecture: `ARCHITECTURE.md`. Context for agents: `CLAUDE.md`.
