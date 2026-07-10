// Core event contract — TS mirror of src-tauri/src/events.rs. Keep in sync.

export const AGENT_SPEECH = "agent:speech"; // payload: string (audio path)
export const AGENT_REPORT = "agent:report"; // payload: Report
export const VOICE_MUTED = "voice:muted"; // payload: boolean
export const VOICE_LISTENING = "voice:listening"; // payload: boolean
export const VOICE_TRANSCRIPT = "voice:transcript"; // payload: string (user text)

export type Report = {
  source: string;
  ts: string;
  text: string;
};
