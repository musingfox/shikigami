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
    const was = listening;
    listening = e.payload;
    recordSpeak(listening);
    if (!listening) recordLevel(0);
    if (listening && !was) {
      startMicIfListening();
    } else if (!listening && was) {
      stopMicAndSend();
    }
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

// === MicUtteranceCapture: webview getUserMedia + Audio downsample to 16k mono f32, send bytes via invoke ===
// dev-mode permission is manual step (do not block); cpal fallback not in this pass

import { invoke } from "@tauri-apps/api/core";

let mediaStream: MediaStream | null = null;
let audioCtx: AudioContext | null = null;
let processor: ScriptProcessorNode | null = null;
let source: MediaStreamAudioSourceNode | null = null;
let capturedSamples: number[] = [];
let nativeSampleRate = 48000;
async function startCapture() {
  if (typeof navigator === "undefined" || !navigator.mediaDevices) return; // test / no mic
  try {
    mediaStream = await navigator.mediaDevices.getUserMedia({ audio: { sampleRate: 48000, channelCount: 1 } });
    const W = window as unknown as { AudioContext?: typeof AudioContext; webkitAudioContext?: typeof AudioContext };
    const AC = W.AudioContext || W.webkitAudioContext || AudioContext;
    audioCtx = new AC();
    nativeSampleRate = audioCtx.sampleRate || 48000;
    source = audioCtx.createMediaStreamSource(mediaStream!);
    processor = audioCtx.createScriptProcessor(4096, 1, 1);
    capturedSamples = [];
    processor.onaudioprocess = (e) => {
      const input = e.inputBuffer.getChannelData(0);
      const level = rms(input);
      onAudioLevel(level);
      for (let i = 0; i < input.length; i++) capturedSamples.push(input[i]);
    };
    source.connect(processor);
    processor.connect(audioCtx.destination);
  } catch (err) {
    console.warn("mic getUserMedia failed (manual perm may be needed in dev)", err);
  }
}

async function stopCaptureAndSend() {
  if (processor) {
    processor.disconnect();
    processor.onaudioprocess = null;
  }
  if (source) source.disconnect();
  if (mediaStream) mediaStream.getTracks().forEach((t) => t.stop());
  if (audioCtx) {
    await audioCtx.close().catch(() => {});
  }
  const samples = capturedSamples;
  capturedSamples = [];
  mediaStream = null;
  source = null;
  processor = null;
  audioCtx = null;

  if (samples.length === 0) return;
  const native = Float32Array.from(samples);
  const down = resampleTo16k(native, nativeSampleRate);
  const bytes = new Uint8Array(down.buffer);
  try {
    await invoke("process_utterance", { pcm: Array.from(bytes) });
  } catch (e) {
    console.warn("process_utterance invoke failed", e);
  }
}

export async function startMicIfListening() {
  if (listening) await startCapture();
}

export async function stopMicAndSend() {
  await stopCaptureAndSend();
}
