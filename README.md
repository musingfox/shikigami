# Shikigami

A desktop-floating AI assistant — an **agent aggregator**.

One face on your desktop, many agents behind it. Speak to it; it summons and
directs the right backend agent (Claude Code, Codex, OpenClaw, Hermes, …) to do
the work, then talks back.

The name: in Onmyōdō a *shikigami* (式神) is a servant-spirit summoned to carry
out its master's will. Here each backend agent is a shikigami you summon; the
app is the interface you summon them from.

## Shape

```
you (voice / hotkey)
   │
   ▼
shikigami — floating avatar + thin agent wrapper
   ├─ brain : adapter + router  (normalize CC/Codex/OpenClaw/Hermes → one agent interface, pick who)
   ├─ mouth : SumVox            (summarize + TTS)              → ../SumVox
   ├─ ears  : local STT
   └─ hands : shikigami-bridge  (browser + macOS control, MCP) → ../shikigami-bridge
```

Status: **M0, empty repo.** Architecture under discussion — see `CLAUDE.md`.
