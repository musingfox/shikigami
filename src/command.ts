// Channel ② targeted talk (R2a): bubble 🎙 → capture → transcript confirm →
// inject via herdr agent.prompt. Confirm-before-inject is policy, not UX
// sugar — nothing reaches an agent's pane without an explicit ✓.
// Target comes from the clicked bubble, so there is no voice name-resolution
// here (that's a later R2 enhancement).

import { invoke } from "@tauri-apps/api/core";
import type { AgentEntry } from "./events";
import { isListening, setCaptureReadyListener, setUtteranceInterceptor, toggleTalk } from "./mic";
import { toast } from "./toast";
import { acquireLarge, releaseLarge } from "./window-frame";

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
  if (isListening()) {
    // a capture is already running — this click means "finish it"; the
    // flush routes through onUtterance and pops the confirm
    await toggleTalk();
    return;
  }
  target = { pane: a.pane, name: a.name };
  // words spoken before the mic actually streams are LOST (this clipped
  // leading Chinese and left English-only transcripts) — don't show the red
  // dot until capture-ready fires
  showStage("🎙 麥克風準備中…");
  await toggleTalk();
}

// Registered as mic.ts's utterance interceptor: claims the utterance only
// while a target is armed; everything else falls through to Q&A as before.
export async function onUtterance(pcm: number[]): Promise<boolean> {
  if (!target) return false;
  const t = target;
  target = null; // one-shot: next utterance is ordinary Q&A again
  try {
    showStage("辨識中…"); // STT takes a beat — show we heard them
    const text = String(await invokeImpl("transcribe_utterance", { pcm })).trim();
    if (!text) {
      hideConfirm();
      if (!testMode) toast("⚠ 沒聽到內容");
      return true;
    }
    showConfirm(t, text);
  } catch (e) {
    hideConfirm();
    if (!testMode) toast("⚠ " + String(e));
  }
  return true;
}

// One visible state at every step of a targeted take: the confirm bubble
// doubles as the stage indicator (錄音中 → 辨識中 → confirm), so a running
// capture always has an on-screen stop affordance.
function showStage(text: string, buttonText?: string, onButton?: () => void) {
  if (testMode) return;
  const el = confirmEl();
  el.innerHTML = "";
  const span = document.createElement("span");
  span.textContent = text;
  el.appendChild(span);
  if (buttonText && onButton) {
    const actions = document.createElement("div");
    actions.className = "actions";
    const btn = document.createElement("button");
    btn.textContent = buttonText;
    btn.onclick = onButton;
    actions.appendChild(btn);
    el.appendChild(actions);
  }
  if (!confirmShown) {
    confirmShown = true;
    acquireLarge().then(() => el.classList.add("show"));
  } else {
    el.classList.add("show");
  }
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

// Toast-style bubble (multi-line, full text visible), amber accent. Holds the
// window large while pending, same ref-count dance as menu/toast.
let confirmShown = false;

function showConfirm(t: Target, text: string) {
  if (testMode) {
    testConfirms.push({ name: t.name, text });
    return;
  }
  const el = confirmEl();
  el.innerHTML = "";
  const span = document.createElement("span");
  span.textContent = `→ ${t.name}：${text}`;
  const actions = document.createElement("div");
  actions.className = "actions";
  const ok = document.createElement("button");
  ok.textContent = "✓ 送出";
  ok.onclick = () => {
    inject(t, text);
    hideConfirm();
  };
  const no = document.createElement("button");
  no.textContent = "✕ 取消";
  no.onclick = hideConfirm;
  actions.append(no, ok);
  el.append(span, actions);
  if (!confirmShown) {
    confirmShown = true;
    acquireLarge().then(() => el.classList.add("show"));
  } else {
    el.classList.add("show");
  }
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
  if (testMode || typeof document === "undefined") {
    confirmShown = false;
    return;
  }
  document.getElementById("confirm-bar")?.classList.remove("show");
  if (confirmShown) {
    confirmShown = false;
    releaseLarge();
  }
}

export function initCommand() {
  setUtteranceInterceptor(onUtterance);
  setCaptureReadyListener((ready) => {
    if (ready && target) {
      showStage(`🔴 對 ${target.name} 說話中…`, "■ 結束", () => { toggleTalk(); });
    }
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") hideConfirm();
  });
}
