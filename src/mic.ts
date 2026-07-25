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

// VoiceErrorSurface: every pipeline failure becomes a readable toast.
// Rust rejections arrive stage-labeled ("stt:"/"brain:"/"speak:").
export function errorText(msg: string): string {
  return "⚠ " + msg;
}

function surfaceError(msg: string) {
  toast(errorText(msg));
  recordSpeak(false);
  recordLevel(0);
}

export function isListening() {
  return listening;
}

// Targeted talk (R2a): command.ts registers an interceptor that gets first
// claim on a finished utterance; returning true means "consumed, skip the
// default process_utterance pipeline".
let utteranceInterceptor: ((pcm: number[]) => Promise<boolean>) | null = null;
export function setUtteranceInterceptor(fn: ((pcm: number[]) => Promise<boolean>) | null) {
  utteranceInterceptor = fn;
}

// Summon (R2d): process_utterance now answers with a tagged outcome — either
// the reply was already spoken by Rust, or the brain asked to summon a new
// agent, which must reach the confirm bar before anything is created.
export type SummonProposal = { project: string; task: string; cwd: string };

let summonHandler: ((p: SummonProposal) => void) | null = null;
export function setSummonHandler(fn: ((p: SummonProposal) => void) | null) {
  summonHandler = fn;
}

// true = a well-formed summon proposal was routed to onSummon. Anything else
// (spoken, older unit returns, missing fields) is false and stays silent —
// a half-filled proposal must never become a pane.
export function handleUtteranceOutcome(
  outcome: unknown,
  onSummon: (p: SummonProposal) => void,
): boolean {
  if (!outcome || typeof outcome !== "object") return false;
  const o = outcome as Record<string, unknown>;
  if (o.kind !== "summon") return false;
  const filled = (v: unknown) => typeof v === "string" && v.trim().length > 0;
  if (!filled(o.project) || !filled(o.task) || !filled(o.cwd)) return false;
  onSummon(outcome as SummonProposal);
  return true;
}

// Fired when audio frames actually start flowing — mic spin-up takes a few
// hundred ms after toggleTalk, and words spoken before that are lost. UI
// should not invite the user to speak until this fires.
let captureReadyListener: ((ready: boolean) => void) | null = null;
export function setCaptureReadyListener(fn: ((ready: boolean) => void) | null) {
  captureReadyListener = fn;
}

// Double-click the orb to talk: first call starts capture, second stops+sends.
// Rust owns the listening state (voice::LISTENING) — we just flip it there and
// let the VOICE_LISTENING event drive capture, the exact same path as the PTT
// hotkey. Local flip remains as the no-Tauri (test / browser preview) fallback.
export async function toggleTalk(): Promise<boolean> {
  if (!testMode) {
    try {
      return !!(await invoke("toggle_listening"));
    } catch {
      // browser preview: no Tauri runtime — fall through to the local flip
    }
  }
  if (!listening) {
    listening = true;
    recordSpeak(true);
    await startCaptureOrRollback();
  } else {
    listening = false;
    recordSpeak(false);
    recordLevel(0);
    await stopCaptureAndSend();
  }
  return listening;
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
    mediaStream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, noiseSuppression: true, echoCancellation: true, autoGainControl: true },
    });
    const W = window as unknown as { AudioContext?: typeof AudioContext; webkitAudioContext?: typeof AudioContext };
    const AC = W.AudioContext || W.webkitAudioContext || AudioContext;
    // Ask WebKit for a 16kHz context so it does the (properly anti-aliased)
    // resample. resampleTo16k below then no-ops. If the UA ignores the request,
    // nativeSampleRate reflects the real rate and the naive fallback still runs.
    audioCtx = new AC({ sampleRate: 16000 });
    nativeSampleRate = audioCtx.sampleRate || 16000;
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
    captureReadyListener?.(true);
  } catch (err) {
    surfaceError("mic: " + String(err));
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
  const pcm = Array.from(bytes);
  try {
    if (utteranceInterceptor && (await utteranceInterceptor(pcm))) return;
    const outcome = await invoke("process_utterance", { pcm });
    if (summonHandler) handleUtteranceOutcome(outcome, summonHandler);
  } catch (e) {
    surfaceError(String(e));
  }
}

// Start capture, and if the mic couldn't be acquired roll `listening` back —
// surfaceError has already toasted and reset the avatar, so a leftover
// listening===true would desync state (the next toggleTalk would take the
// "stop" branch and silently no-op). Shared by the PTT hotkey path and the
// double-click toggle so both recover identically.
async function startCaptureOrRollback() {
  await startCapture();
  if (!testMode && !mediaStream) {
    listening = false;
    recordSpeak(false);
    // reset the Rust side too, or the next PTT press-release cycle is swallowed
    invoke("set_listening", { on: false }).catch(() => {});
  }
}

export async function startMicIfListening() {
  if (listening) await startCaptureOrRollback();
}

export async function stopMicAndSend() {
  await stopCaptureAndSend();
}
