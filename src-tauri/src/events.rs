// Core event contract (channel ①). Inbound adapters (SumVox files today;
// Codex notify, `shikigami notify`, herdr later) normalize agent activity
// into these events; the frontend subscribes to these names only and never
// to adapter-specific ones. TS mirror: src/events.ts — keep in sync.

use serde::Serialize;

/// An agent/notifier is speaking; payload = path to the audio being played.
pub const AGENT_SPEECH: &str = "agent:speech";
/// A textual progress report from an agent; payload = `Report`.
pub const AGENT_REPORT: &str = "agent:report";
/// Voice output mute state changed; payload = bool.
pub const VOICE_MUTED: &str = "voice:muted";
/// PTT is active; payload = bool (true while held).
pub const VOICE_LISTENING: &str = "voice:listening";
/// Live transcript text for display; payload = string.
pub const VOICE_TRANSCRIPT: &str = "voice:transcript";

#[derive(Serialize, Clone)]
pub struct Report {
    /// which adapter produced this, e.g. "sumvox"
    pub source: &'static str,
    /// RFC3339 timestamp ("" when the source line carried none)
    pub ts: String,
    pub text: String,
}
