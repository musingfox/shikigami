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

// Clamp the label's left edge so its full width stays inside the window.
export function clampLabelX(centerX: number, width: number, winW: number, margin = 4): number {
  return Math.min(winW - margin - width, Math.max(margin, centerX - width / 2));
}

const DOT = 20; // button hit size

function labelEl(): HTMLDivElement {
  let el = document.getElementById("roster-label") as HTMLDivElement | null;
  if (!el) {
    el = document.createElement("div");
    el.id = "roster-label";
    document.body.appendChild(el);
  }
  return el;
}

function showLabel(a: AgentEntry, at: { x: number; y: number }) {
  const el = labelEl();
  el.textContent = `${a.name} · ${a.status}`;
  const winW = document.documentElement.clientWidth || 150;
  // place first so offsetWidth is measurable, then clamp into the window
  el.style.top = `${at.y + DOT / 2 + 4}px`;
  el.style.left = `${clampLabelX(at.x, el.offsetWidth, winW)}px`;
  el.classList.add("show");
}

function hideLabel() {
  document.getElementById("roster-label")?.classList.remove("show");
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
  hideLabel();
  const pos = arcPositions(roster.length, currentCenter());
  roster.forEach((a, i) => {
    const p = pos[i];
    const dot = document.createElement("button");
    dot.className = "roster-dot";
    dot.dataset.status = a.status;
    dot.style.left = `${p.x - DOT / 2}px`;
    dot.style.top = `${p.y - DOT / 2}px`;
    // stagger the breathing so the flock feels alive, not metronomic
    dot.style.animationDelay = `${i * 0.45}s`;
    dot.onmouseenter = () => showLabel(a, p);
    dot.onmouseleave = hideLabel;
    dot.onclick = () => toast(`${a.name}: ${a.status}`);
    el!.appendChild(dot);
  });
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
}
