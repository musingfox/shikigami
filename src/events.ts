// Core event contract — TS mirror of src-tauri/src/events.rs. Keep in sync.

export const AGENT_SPEECH = "agent:speech"; // payload: string (audio path)
export const AGENT_REPORT = "agent:report"; // payload: Report
export const VOICE_MUTED = "voice:muted"; // payload: boolean
export const VOICE_LISTENING = "voice:listening"; // payload: boolean
export const VOICE_TRANSCRIPT = "voice:transcript"; // payload: string (user text)
export const AGENT_ROSTER = "agent:roster"; // payload: AgentEntry[]
export const AGENT_STATUS = "agent:status"; // payload: AgentStatusChange

export type Report = {
  source: string;
  ts: string;
  text: string;
};

export type AgentEntry = {
  id: string; // stable identity (herdr terminal_id today)
  name: string;
  pane: string; // injection target for channel ② (R2)
  status: string; // idle | working | blocked | done | unknown
};

export type AgentStatusChange = {
  id: string;
  status: string;
};
// MicUtteranceCapture uses core voice:* events + process_utterance invoke (bytes f32).
