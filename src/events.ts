// Core event contract — TS mirror of src-tauri/src/events.rs. Keep in sync.

export const AGENT_SPEECH = "agent:speech"; // payload: string (audio path)
export const AGENT_REPORT = "agent:report"; // payload: Report
export const VOICE_MUTED = "voice:muted"; // payload: boolean

export type Report = {
  source: string;
  ts: string;
  text: string;
};
