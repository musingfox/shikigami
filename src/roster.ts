// Roster strip + report attribution (R1c).
// agent:roster / agent:status → status dots above the orb (click a dot to
// toast "name: status"); agent:activity remembers who acted last so the next
// SumVox report toast gets a name prefix.
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
  for (const a of roster) {
    const dot = document.createElement("button");
    dot.className = "roster-dot";
    dot.dataset.status = a.status;
    dot.title = a.name;
    dot.onclick = () => toast(`${a.name}: ${a.status}`);
    el.appendChild(dot);
  }
}

export function initRoster() {
  listen<AgentEntry[]>(AGENT_ROSTER, (e) => setRoster(e.payload));
  listen<AgentStatusChange>(AGENT_STATUS, (e) => applyStatus(e.payload));
  listen<Activity>(AGENT_ACTIVITY, (e) => noteActivity(e.payload));
  // late-subscriber cover: the watcher may have emitted before we listened
  invoke<AgentEntry[]>("get_roster")
    .then((r) => setRoster(r ?? []))
    .catch(() => {});
}
