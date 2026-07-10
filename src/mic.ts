// Mic capture + voice state (PTT driven).
// ListeningIndicator: on voice:listening drives setSpeaking + levels; transcript -> toast("你：...")
// ponytail: reuse setSpeaking/setLevel/toast exactly, no new avatar API.

import { listen } from "@tauri-apps/api/event";
import { setLevel, setSpeaking } from "./avatar";
import { toast } from "./toast";
import {
  VOICE_LISTENING,
  VOICE_TRANSCRIPT,
} from "./events";

let listening = false;
let testMode = false;

// test observable records (populated when verbs called via this module's wiring)
export const testSpeakCalls: boolean[] = [];
export const testLevelCalls: number[] = [];

function recordSpeak(on: boolean) {
  testSpeakCalls.push(on);
  if (!testMode) setSpeaking(on);
}
function recordLevel(v: number) {
  testLevelCalls.push(v);
  if (!testMode) setLevel(v);
}

export function resetTestRecords() {
  testMode = true;
  testSpeakCalls.length = 0;
  testLevelCalls.length = 0;
}

export function __setTestModeForTest(v: boolean) { testMode = v; }

export function initMic() {
  listen<boolean>(VOICE_LISTENING, (e) => {
    listening = e.payload;
    recordSpeak(listening);
    if (!listening) recordLevel(0);
  });

  listen<string>(VOICE_TRANSCRIPT, (e) => {
    toast("你：" + e.payload);
  });
}

// for capture rms during listening (called from audio worklet path later)
export function onAudioLevel(level: number) {
  if (listening) {
    recordLevel(level);
  }
}

// resample etc implemented in MicUtteranceCapture contract
export function resampleTo16k(samples: Float32Array, fromRate: number): Float32Array {
  if (fromRate === 16000) return samples;
  // naive for now; full impl in later contract with tests
  const ratio = fromRate / 16000;
  const outLen = Math.floor(samples.length / ratio);
  const out = new Float32Array(outLen);
  for (let i = 0; i < outLen; i++) {
    out[i] = samples[Math.floor(i * ratio)];
  }
  return out;
}

export function rms(samples: Float32Array): number {
  if (samples.length === 0) return 0;
  let sum = 0;
  for (let i = 0; i < samples.length; i++) sum += samples[i] * samples[i];
  return Math.sqrt(sum / samples.length);
}

// test-only hook for driving state without tauri listen (used by voice.test.ts T1)
export function __setListeningForTest(on: boolean) {
  listening = on;
  recordSpeak(on);
  if (!on) recordLevel(0);
}
