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
/// Roster of observed agents changed; payload = Vec<AgentEntry>.
pub const AGENT_ROSTER: &str = "agent:roster";
/// One agent's coarse status changed; payload = AgentStatusChange.
pub const AGENT_STATUS: &str = "agent:status";
/// Precise identity signal from an agent hook (never toasts); payload = Activity.
pub const AGENT_ACTIVITY: &str = "agent:activity";

#[derive(Serialize, Clone)]
pub struct Report {
    /// which adapter produced this, e.g. "sumvox"
    pub source: &'static str,
    /// RFC3339 timestamp ("" when the source line carried none)
    pub ts: String,
    pub text: String,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct AgentEntry {
    /// stable identity (herdr terminal_id today)
    pub id: String,
    /// display name: agent-declared name, else cwd basename, else agent kind
    pub name: String,
    /// pane address, the injection target for channel ② (R2)
    pub pane: String,
    /// idle | working | blocked | done | unknown
    pub status: String,
    /// what the agent is doing right now (terminal title, "" if unknown)
    pub title: String,
    /// working directory ("" if unknown)
    pub cwd: String,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct AgentStatusChange {
    pub id: String,
    pub status: String,
}

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Activity {
    /// which adapter produced this, e.g. "cchooks"
    pub source: &'static str,
    /// agent session id ("" when the hook payload carried none)
    pub session: String,
    /// pane address (HERDR_PANE_ID / TMUX_PANE, "" outside a multiplexer)
    pub pane: String,
    /// hook kind, e.g. "stop" | "notification"
    pub kind: String,
    /// RFC3339 timestamp stamped by the hook script
    pub ts: String,
}
