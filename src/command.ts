// Channel ② targeted talk (R2a): bubble 🎙 → capture → transcript confirm →
// inject via herdr agent.prompt. Confirm-before-inject is policy, not UX
// sugar — nothing reaches an agent's pane without an explicit ✓.
// Target comes from the clicked bubble, so there is no voice name-resolution
// here (that's a later R2 enhancement).

import { invoke } from "@tauri-apps/api/core";
import type { AgentEntry } from "./events";
import type { SummonProposal } from "./mic";
import {
  isListening,
  setCaptureReadyListener,
  setSummonHandler,
  setUtteranceInterceptor,
  toggleTalk,
} from "./mic";
import { toast } from "./toast";
import { acquireLarge, releaseLarge } from "./window-frame";

type Target = { pane: string; name: string };

let target: Target | null = null;
let pendingSummon: SummonProposal | null = null;
let testMode = false;

export const testConfirms: { name: string; text: string }[] = [];
export const testInjects: { pane: string; text: string }[] = [];
export const testSummonConfirms: { project: string; task: string }[] = [];
export const testSummonSends: { cmd: string; args: Record<string, unknown> }[] = [];
export const testSummonOk: { project: string; task: string }[] = [];
// Promise-wrapped so a missing Tauri runtime rejects instead of throwing
const defaultInvoke = (cmd: string, args?: Record<string, unknown>): Promise<unknown> =>
  Promise.resolve().then(() => invoke(cmd, args));
let invokeImpl = defaultInvoke;

export function __setTestModeForTest(v: boolean) { testMode = v; }
export function __setInvokeForTest(fn: typeof defaultInvoke | null) { invokeImpl = fn ?? defaultInvoke; }
export function __resetForTest() {
  target = null;
  pendingSummon = null;
  testConfirms.length = 0;
  testInjects.length = 0;
  testSummonConfirms.length = 0;
  testSummonSends.length = 0;
  testSummonOk.length = 0;
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
  const head = document.createElement("span");
  head.textContent = `→ ${t.name}`;
  // transcript is editable before sending — STT is good, not perfect
  const input = document.createElement("textarea");
  input.value = text;
  input.rows = 2;
  const autosize = () => {
    input.style.height = "auto";
    // 64 = 3 行（12px × 1.5 ＋ 內距）；248px 寬下那是 55 個中文字
    input.style.height = `${Math.min(input.scrollHeight, 64)}px`;
  };
  input.oninput = autosize;
  const actions = document.createElement("div");
  actions.className = "actions";
  const ok = document.createElement("button");
  ok.textContent = "✓ 送出";
  ok.onclick = () => {
    const edited = input.value.trim();
    if (edited) inject(t, edited);
    hideConfirm();
  };
  const no = document.createElement("button");
  no.textContent = "✕ 取消";
  no.onclick = hideConfirm;
  input.onkeydown = (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      ok.click();
    }
  };
  actions.append(no, ok);
  el.append(head, input, actions);
  setTimeout(() => {
    autosize();
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
  }, 0);
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

// === Summon (R2d) ===
// Same confirm-before-act policy as inject, one step earlier: the brain only
// ever *proposes* a new agent; the pane, the agent and the task are created by
// summon_agent, and only ✓ calls it.
export function onSummonProposal(p: SummonProposal) {
  pendingSummon = p;
  if (testMode) {
    testSummonConfirms.push({ project: p.project, task: p.task });
    return;
  }
  showSummonConfirm(p);
}

export async function confirmSummon(task: string): Promise<void> {
  const p = pendingSummon;
  pendingSummon = null;
  const edited = task.trim();
  if (!p || !edited) {
    hideConfirm(); // empty task = same as cancel, nothing is created
    return;
  }
  const args = { project: p.project, task: edited, cwd: p.cwd };
  if (testMode) testSummonSends.push({ cmd: "summon_agent", args });
  // the chain (tab.create → agent.start → wait → prompt) takes ~10s; keep the
  // bubble up so the window isn't silently idle-looking meanwhile
  showStage(`⚡ 召喚 ${p.project} 中…`);
  try {
    await invokeImpl("summon_agent", args);
    if (testMode) testSummonOk.push({ project: args.project, task: args.task });
    hideConfirm();
    if (!testMode) toast(`已召喚 ${p.project}`);
  } catch (e) {
    // the herdr chain can fail at any of tab.create/agent.start/wait/prompt —
    // surface it, never let it escape into the caller's await
    hideConfirm();
    if (!testMode) toast("⚠ " + String(e));
  }
}

export function cancelSummon() {
  pendingSummon = null;
  hideConfirm();
}

function showSummonConfirm(p: SummonProposal) {
  const el = confirmEl();
  el.innerHTML = "";
  const head = document.createElement("span");
  head.className = "summon-target";
  head.textContent = `⚡ 召喚 ${p.project}`;
  // the task is editable before summoning — the brain paraphrases
  const input = document.createElement("textarea");
  input.value = p.task;
  input.rows = 2;
  const autosize = () => {
    input.style.height = "auto";
    // 64 = 3 行（12px × 1.5 ＋ 內距）；248px 寬下那是 55 個中文字
    input.style.height = `${Math.min(input.scrollHeight, 64)}px`;
  };
  input.oninput = autosize;
  const actions = document.createElement("div");
  actions.className = "actions";
  const ok = document.createElement("button");
  ok.textContent = "✓ 召喚";
  ok.onclick = () => { confirmSummon(input.value); };
  const no = document.createElement("button");
  no.textContent = "✕ 取消";
  no.onclick = cancelSummon;
  input.onkeydown = (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      ok.click();
    }
  };
  actions.append(no, ok);
  el.append(head, input, actions);
  setTimeout(() => {
    autosize();
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
  }, 0);
  if (!confirmShown) {
    confirmShown = true;
    acquireLarge().then(() => el.classList.add("show"));
  } else {
    el.classList.add("show");
  }
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
  setSummonHandler(onSummonProposal);
  setCaptureReadyListener((ready) => {
    if (ready && target) {
      showStage(`🔴 對 ${target.name} 說話中…`, "■ 結束", () => { toggleTalk(); });
    }
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") cancelSummon();
  });
}
