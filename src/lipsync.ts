// now_playing audio → 40ms RMS envelope → avatar mouth, mirroring
// SumVox's Swift menubar animation.

import { invoke } from "@tauri-apps/api/core";
import { setLevel, setSpeaking } from "./avatar";

const BUCKET_S = 0.04;

let audioCtx: AudioContext | null = null;
let gen = 0; // newer clip cancels the running animation

export async function lipsync(path: string) {
  const my = ++gen;
  let env: number[];
  try {
    const bytes = await invoke<ArrayBuffer>("read_file", { path });
    audioCtx ??= new AudioContext();
    const buf = await audioCtx.decodeAudioData(bytes);
    env = rmsEnvelope(buf);
  } catch (e) {
    console.warn("lipsync: decode failed", path, e);
    return;
  }
  if (my !== gen) return;

  setSpeaking(true);
  const t0 = performance.now();
  const step = () => {
    if (my !== gen) return;
    const i = Math.floor((performance.now() - t0) / (BUCKET_S * 1000));
    if (i >= env.length) {
      setSpeaking(false);
      return;
    }
    setLevel(env[i]);
    requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

function rmsEnvelope(buf: AudioBuffer): number[] {
  const ch = buf.getChannelData(0);
  const bucket = Math.round(BUCKET_S * buf.sampleRate);
  const env: number[] = [];
  for (let i = 0; i < ch.length; i += bucket) {
    const end = Math.min(i + bucket, ch.length);
    let s = 0;
    for (let j = i; j < end; j++) s += ch[j] * ch[j];
    env.push(Math.sqrt(s / (end - i)));
  }
  let peak = 1e-6;
  for (const v of env) if (v > peak) peak = v;
  return env.map((v) => v / peak);
}
