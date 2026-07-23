// Roster strip + report attribution (R1c).
// agent:roster / agent:status → breathing status bubbles arced around the
// orb's rim (hover grows one + shows a "name · status" label clamped inside
// the window; click toasts); agent:activity remembers who acted last so the
// next SumVox report toast gets a name prefix. Hidden while the radial menu
// is open (body.menu-open, toggled by menu.ts).
// ponytail: attribution is a 30s time-window correlation, not a join — hooks
// were deliberately kept toast-free (R1b decision); test hooks mirror mic.ts.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  AGENT_ACTIVITY,
  AGENT_ROSTER,
  AGENT_STATUS,
  type Activity,
  type AgentEntry,
  type AgentStatusChange,
} from "./events";
import { startTargetedTalk } from "./command";
import { toast } from "./toast";
import { currentCenter } from "./window-frame";

export const ATTRIBUTION_WINDOW_MS = 30_000;

let roster: AgentEntry[] = [];
let lastActivity: { name: string; at: number } | null = null;
let testMode = false;

export const testStripRenders: AgentEntry[][] = [];
export function __setTestModeForTest(v: boolean) { testMode = v; }
export function __resetForTest() {
  roster = [];
  lastActivity = null;
  selectedId = null;
  testStripRenders.length = 0;
}

export function setRoster(r: AgentEntry[]) {
  roster = [...r].sort((a, b) => a.name.localeCompare(b.name));
  renderStrip();
}

export function applyStatus(c: AgentStatusChange) {
  const a = roster.find((x) => x.id === c.id);
  if (!a || a.status === c.status) return;
  a.status = c.status;
  renderStrip();
}

export function nameForPane(pane: string): string | null {
  if (!pane) return null;
  return roster.find((a) => a.pane === pane)?.name ?? null;
}

export function noteActivity(act: Activity, now: number = Date.now()) {
  const name =
    nameForPane(act.pane) ?? (act.session ? act.session.slice(0, 8) : null);
  if (name) lastActivity = { name, at: now };
}

/// Prefix a report with the last actor's name when it plausibly caused it.
export function attribute(text: string, now: number = Date.now()): string {
  if (lastActivity && now - lastActivity.at <= ATTRIBUTION_WINDOW_MS) {
    return `${lastActivity.name}：${text}`;
  }
  return text;
}

// Bubbles sit on an arc hugging the orb's rim, centered straight up (-90°).
export function arcPositions(
  n: number,
  center: { x: number; y: number },
  radius = 66,
  stepDeg = 24,
): { x: number; y: number }[] {
  if (n <= 0) return [];
  const start = -90 - (stepDeg * (n - 1)) / 2;
  const out: { x: number; y: number }[] = [];
  for (let i = 0; i < n; i++) {
    const theta = ((start + stepDeg * i) * Math.PI) / 180;
    out.push({
      x: Math.round((center.x + radius * Math.cos(theta)) * 100) / 100,
      y: Math.round((center.y + radius * Math.sin(theta)) * 100) / 100,
    });
  }
  return out;
}

// macOS-dock magnification: scale by cursor distance, cosine falloff.
// max 1.6 — with the 176px idle frame, edge bubbles stay inside the window.
export function magnifyScale(dist: number, influence = 48, maxScale = 1.6): number {
  if (dist >= influence) return 1;
  return 1 + ((maxScale - 1) * (Math.cos((dist / influence) * Math.PI) + 1)) / 2;
}

const DOT = 20; // button hit size

export function labelText(a: AgentEntry): string {
  const base = `${a.name} · ${a.status}`;
  return a.title ? `${base} — ${a.title}` : base;
}

// Click-selected bubble (sticky caption + action buttons); null = hover mode.
let selectedId: string | null = null;

function labelEl(): HTMLDivElement {
  let el = document.getElementById("roster-label") as HTMLDivElement | null;
  if (!el) {
    el = document.createElement("div");
    el.id = "roster-label";
    document.body.appendChild(el);
  }
  return el;
}

// Fixed caption slot under the orb (CSS-positioned) — showing it never moves
// the bubbles, and long names just ellipsize inside the window. Selecting a
// bubble makes it sticky and appends the action buttons (R1.5).
function showLabel(a: AgentEntry, sticky = false) {
  const el = labelEl();
  el.innerHTML = "";
  const text = document.createElement("span");
  text.textContent = labelText(a);
  el.appendChild(text);
  if (sticky) {
    const jump = document.createElement("button");
    jump.textContent = "↗";
    jump.title = "跳過去";
    jump.onclick = () => {
      invoke("focus_agent", { pane: a.pane }).catch((e) => toast("⚠ " + String(e)));
      deselect();
    };
    const talk = document.createElement("button");
    talk.textContent = "🎙";
    talk.title = "對它說話";
    talk.onclick = () => {
      deselect();
      startTargetedTalk(a);
    };
    el.append(jump, talk);
  }
  el.classList.toggle("selected", sticky);
  el.classList.add("show");
}

function hideLabel() {
  const el = document.getElementById("roster-label");
  el?.classList.remove("show", "selected");
}

function deselect() {
  selectedId = null;
  hideLabel();
}

// live geometry for the dock effect (rebuilt on every render)
let dotEls: HTMLButtonElement[] = [];
let dotPos: { x: number; y: number }[] = [];

function applyMagnify(mx: number, my: number) {
  dotEls.forEach((dot, i) => {
    const p = dotPos[i];
    if (!p || !dot.isConnected) return;
    const s = magnifyScale(Math.hypot(mx - p.x, my - p.y));
    dot.style.transform = s === 1 ? "" : `scale(${s.toFixed(3)})`;
  });
}

function renderStrip() {
  if (testMode) {
    testStripRenders.push(roster.map((a) => ({ ...a })));
    return;
  }
  if (typeof document === "undefined") return;
  let el = document.getElementById("roster-strip");
  if (!el) {
    el = document.createElement("div");
    el.id = "roster-strip";
    document.body.appendChild(el);
  }
  el.innerHTML = "";
  dotEls = [];
  dotPos = arcPositions(roster.length, currentCenter());
  roster.forEach((a, i) => {
    const p = dotPos[i];
    const dot = document.createElement("button");
    dot.className = "roster-dot";
    dot.dataset.status = a.status;
    dot.style.left = `${p.x - DOT / 2}px`;
    dot.style.top = `${p.y - DOT / 2}px`;
    // stagger the breathing so the flock feels alive, not metronomic
    dot.style.animationDelay = `${i * 0.45}s`;
    dot.onmouseenter = () => { if (!selectedId) showLabel(a); };
    dot.onmouseleave = () => { if (!selectedId) hideLabel(); };
    dot.onclick = () => {
      if (selectedId === a.id) deselect();
      else { selectedId = a.id; showLabel(a, true); }
    };
    el!.appendChild(dot);
    dotEls.push(dot);
  });
  // selection survives re-renders (status polls) as long as the agent exists
  const sel = roster.find((a) => a.id === selectedId);
  if (sel) showLabel(sel, true);
  else deselect();
}

export function initRoster() {
  listen<AgentEntry[]>(AGENT_ROSTER, (e) => setRoster(e.payload));
  listen<AgentStatusChange>(AGENT_STATUS, (e) => applyStatus(e.payload));
  listen<Activity>(AGENT_ACTIVITY, (e) => noteActivity(e.payload));
  // late-subscriber cover: the watcher may have emitted before we listened
  invoke<AgentEntry[]>("get_roster")
    .then((r) => setRoster(r ?? []))
    .catch(() => {});
  // arc geometry depends on the window center — re-render on grow/shrink
  window.addEventListener("resize", renderStrip);
  // dock magnification tracks the cursor everywhere (5 dots, cheap)
  document.addEventListener("mousemove", (e) => applyMagnify(e.clientX, e.clientY));
  // click anywhere outside the bubbles/caption clears the selection
  document.addEventListener("mousedown", (e) => {
    if (!selectedId) return;
    const t = e.target as Element | null;
    if (t && (t.closest(".roster-dot") || t.closest("#roster-label"))) return;
    deselect();
  });
}
