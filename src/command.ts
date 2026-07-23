// Channel ② targeted talk (R2a): bubble 🎙 → capture → transcript confirm →
// inject via herdr agent.prompt. Confirm-before-inject is policy, not UX
// sugar — nothing reaches an agent's pane without an explicit ✓.
// Target comes from the clicked bubble, so there is no voice name-resolution
// here (that's a later R2 enhancement).

import { invoke } from "@tauri-apps/api/core";
import type { AgentEntry } from "./events";
import { setUtteranceInterceptor, toggleTalk } from "./mic";
import { toast } from "./toast";

type Target = { pane: string; name: string };

let target: Target | null = null;
let testMode = false;

export const testConfirms: { name: string; text: string }[] = [];
export const testInjects: { pane: string; text: string }[] = [];
// Promise-wrapped so a missing Tauri runtime rejects instead of throwing
const defaultInvoke = (cmd: string, args?: Record<string, unknown>): Promise<unknown> =>
  Promise.resolve().then(() => invoke(cmd, args));
let invokeImpl = defaultInvoke;

export function __setTestModeForTest(v: boolean) { testMode = v; }
export function __setInvokeForTest(fn: typeof defaultInvoke | null) { invokeImpl = fn ?? defaultInvoke; }
export function __resetForTest() {
  target = null;
  testConfirms.length = 0;
  testInjects.length = 0;
  invokeImpl = defaultInvoke;
}
export function __getTargetForTest() { return target; }

export async function startTargetedTalk(a: AgentEntry) {
  target = { pane: a.pane, name: a.name };
  if (!testMode) toast(`對 ${a.name} 說話…`);
  await toggleTalk();
}

// Registered as mic.ts's utterance interceptor: claims the utterance only
// while a target is armed; everything else falls through to Q&A as before.
export async function onUtterance(pcm: number[]): Promise<boolean> {
  if (!target) return false;
  const t = target;
  target = null; // one-shot: next utterance is ordinary Q&A again
  try {
    const text = String(await invokeImpl("transcribe_utterance", { pcm })).trim();
    if (!text) {
      if (!testMode) toast("⚠ 沒聽到內容");
      return true;
    }
    showConfirm(t, text);
  } catch (e) {
    if (!testMode) toast("⚠ " + String(e));
  }
  return true;
}

function confirmEl(): HTMLDivElement {
  let el = document.getElementById("confirm-bar") as HTMLDivElement | null;
  if (!el) {
    el = document.createElement("div");
    el.id = "confirm-bar";
    document.body.appendChild(el);
  }
  return el;
}

function showConfirm(t: Target, text: string) {
  if (testMode) {
    testConfirms.push({ name: t.name, text });
    return;
  }
  const el = confirmEl();
  el.innerHTML = "";
  const span = document.createElement("span");
  span.textContent = `→ ${t.name}：${text}`;
  span.title = text;
  const ok = document.createElement("button");
  ok.textContent = "✓";
  ok.title = "送出";
  ok.onclick = () => {
    inject(t, text);
    hideConfirm();
  };
  const no = document.createElement("button");
  no.textContent = "✕";
  no.title = "取消";
  no.onclick = hideConfirm;
  el.append(span, ok, no);
  el.classList.add("show");
}

function inject(t: Target, text: string) {
  if (testMode) {
    testInjects.push({ pane: t.pane, text });
    return;
  }
  invokeImpl("prompt_agent", { pane: t.pane, text })
    .then(() => toast(`已送給 ${t.name}`))
    .catch((e) => toast("⚠ " + String(e)));
}

function hideConfirm() {
  document.getElementById("confirm-bar")?.classList.remove("show");
}

export function initCommand() {
  setUtteranceInterceptor(onUtterance);
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") hideConfirm();
  });
}
