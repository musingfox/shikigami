// Radial settings menu (DOM+CSS, zero new deps).
// Box labels fan out from the orb toward the screen center (ring 96px,
// 45° apart). Geometry uses the CURRENT window center: idle the window hugs
// the avatar, so opening the menu first grows it (window-frame.ts) — the
// menu renders against the enlarged center 160/180.
// Mute state via get_muted/toggle_mute + VOICE_MUTED; open via open_config.
// ponytail: hand-rolled DOM, no framework; test hooks mirror mic.ts exactly.

import { invoke } from "@tauri-apps/api/core";
import { currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";
import { toast } from "./toast";
import { acquireLarge, releaseLarge, currentCenter, LARGE_W, LARGE_H } from "./window-frame";

export type MenuItemDef = {
  id: string;
  label: (muted: boolean) => string;
  action: string; // invoke name
};
export type TestRender = { action: "open" | "rerender"; items?: { id: string; label: string; active: boolean }[]; muted?: boolean };

let registry: MenuItemDef[] = [
  { id: "mute", label: (m) => (m ? "Unmute" : "Mute"), action: "toggle_mute" },
  { id: "open-config", label: () => "Open Config", action: "open_config" },
  { id: "close", label: () => "Close", action: "quit_app" },
];

let mutedCache = false;
let menuOpen = false;
let currentItems: HTMLElement[] = [];
let testMode = false;

export const testInvokeCalls: string[] = [];
export const testMenuRenders: TestRender[] = [];
export const testDismissCalls: string[] = [];
export const testToastCalls: string[] = [];
// Promise-wrapped so a missing Tauri runtime (browser preview) rejects instead of throwing
const defaultInvoke = (cmd: string): Promise<unknown> => Promise.resolve().then(() => invoke(cmd));
let invokeImpl = defaultInvoke;

export function resetTestRecords() {
  testInvokeCalls.length = 0;
  testMenuRenders.length = 0;
  testDismissCalls.length = 0;
  testToastCalls.length = 0;
  invokeImpl = defaultInvoke;
}

export function __setTestModeForTest(v: boolean) { testMode = v; }
export function __setInvokeForTest(fn: ((cmd: string) => Promise<unknown>) | null) { invokeImpl = fn ?? defaultInvoke; }
// FanLayout: items fan around centerDeg (direction toward screen center), 45° apart
const RING = 96;
const FAN_STEP = 45;
const EDGE = 10; // keep box + glow inside the window

export function fanPositions(
  n: number,
  centerDeg: number,
  center: { x: number; y: number } = { x: 160, y: 180 },
): { x: number; y: number }[] {
  if (n <= 0) return [];
  const start = centerDeg - (FAN_STEP * (n - 1)) / 2;
  const positions: { x: number; y: number }[] = [];
  for (let i = 0; i < n; i++) {
    const theta = (start + FAN_STEP * i) * (Math.PI / 180);
    const x = center.x + RING * Math.cos(theta);
    const y = center.y + RING * Math.sin(theta);
    positions.push({ x: Math.round(x * 100) / 100, y: Math.round(y * 100) / 100 });
  }
  return positions;
}

let fanAngle = -90; // default: fan upward until the real direction is known

// degrees from the avatar (window center) toward the screen center; -90 (up) as fallback
async function fanAngleToScreenCenter(): Promise<number> {
  const win = getCurrentWindow();
  const [pos, size, mon] = await Promise.all([
    win.outerPosition(),
    win.outerSize(),
    currentMonitor(),
  ]);
  if (!mon) return -90;
  const dx = mon.position.x + mon.size.width / 2 - (pos.x + size.width / 2);
  const dy = mon.position.y + mon.size.height / 2 - (pos.y + size.height / 2);
  if (dx === 0 && dy === 0) return -90;
  return Math.atan2(dy, dx) * (180 / Math.PI);
}

// hit test against the current window center (default = live center)
export function isOrbHit(
  x: number,
  y: number,
  center: { x: number; y: number } = currentCenter(),
): boolean {
  const dx = x - center.x;
  const dy = y - center.y;
  return dx * dx + dy * dy <= 48 * 48;
}

let menuEl: HTMLDivElement | null = null;

function ensureMenuEl(): HTMLDivElement {
  if (menuEl) return menuEl;
  menuEl = document.createElement("div");
  menuEl.id = "radial-menu";
  menuEl.style.position = "fixed";
  menuEl.style.left = "0";
  menuEl.style.top = "0";
  menuEl.style.width = `${LARGE_W}px`;
  menuEl.style.height = `${LARGE_H}px`;
  menuEl.style.pointerEvents = "none";
  menuEl.style.zIndex = "1000"; // above toast
  document.body.appendChild(menuEl);
  return menuEl;
}

export function buildMenuItems(reg: MenuItemDef[], cacheMuted: boolean): { id: string; label: string; active: boolean }[] {
  return reg.map((d) => ({
    id: d.id,
    label: d.label(cacheMuted),
    active: d.id === "mute" ? cacheMuted : false,
  }));
}

export function openMenu(angleDeg?: number) {
  if (menuOpen) return;
  if (testMode) {
    testMenuRenders.push({ action: "open", items: buildMenuItems(registry, mutedCache) });
    menuOpen = true;
    return;
  }
  menuOpen = true;
  // grow the window first so the menu has room and geometry uses the large center
  acquireLarge().then(() => {
    if (!menuOpen) return;
    const angleP = angleDeg !== undefined
      ? Promise.resolve(angleDeg)
      : fanAngleToScreenCenter().catch(() => -90);
    return angleP.then((deg) => {
      if (!menuOpen) return;
      fanAngle = deg;
      renderMenu(buildMenuItems(registry, mutedCache));
      invokeImpl("get_muted").then((v) => {
        const m = !!v;
        if (m !== mutedCache) {
          mutedCache = m;
          if (menuOpen) renderMenu(buildMenuItems(registry, mutedCache));
        }
      }).catch(() => {});
    });
  });
}

function renderMenu(items: { id: string; label: string; active: boolean }[]) {
  const el = ensureMenuEl();
  el.innerHTML = "";
  const c = currentCenter();
  const pos = fanPositions(items.length, fanAngle, c);
  items.forEach((it, i) => {
    const p = pos[i] || { x: c.x, y: c.y };
    const btn = document.createElement("button");
    btn.setAttribute("role", "menuitem");
    btn.dataset.id = it.id;
    btn.textContent = it.label;
    btn.classList.toggle("active", it.active);
    btn.style.opacity = "0";
    el.appendChild(btn);
    // box anchored (centered) at its fan slot, clamped inside the window by its real size
    const x = Math.min(LARGE_W - EDGE - btn.offsetWidth / 2, Math.max(EDGE + btn.offsetWidth / 2, p.x));
    const y = Math.min(LARGE_H - EDGE - btn.offsetHeight / 2, Math.max(EDGE + btn.offsetHeight / 2, p.y));
    btn.style.left = `${x}px`;
    btn.style.top = `${y}px`;
    // launched from the orb center
    btn.style.transform = `translate(calc(-50% + ${c.x - x}px), calc(-50% + ${c.y - y}px)) scale(0.3)`;
    setTimeout(() => {
      if (btn.isConnected) {
        btn.style.opacity = "1";
        btn.style.transform = "translate(-50%, -50%) scale(1)";
      }
    }, 50 * i);
    btn.onclick = () => activateItem(it.id);
    currentItems.push(btn);
  });
}

export function dismissMenu(reason: "escape" | "outside") {
  if (!menuOpen) return;
  if (testMode) {
    testDismissCalls.push(reason);
    currentItems = [];
    menuOpen = false;
    return;
  }
  const el = menuEl;
  if (el) el.innerHTML = "";
  currentItems = [];
  menuOpen = false;
  releaseLarge(); // shrink back to hug the avatar
}

export function activateItem(id: string) {
  const def = registry.find((d) => d.id === id);
  if (!def) return;
  if (testMode) testInvokeCalls.push(def.action);
  return invokeImpl(def.action).then((res: unknown) => {
    if (id === "mute") {
      mutedCache = !!res;
    }
    dismissMenu("outside");
  }).catch((e: unknown) => {
    const msg = e instanceof Error ? e.message : String(e);
    if (testMode) testToastCalls.push("⚠ " + msg);
    else toast("⚠ " + msg);
    dismissMenu("outside");
  });
}

export function onMutedEvent(m: boolean) {
  mutedCache = m;
  if (testMode) {
    if (menuOpen) {
      testMenuRenders.push({ action: "rerender", muted: m });
    }
    return;
  }
  if (menuOpen) {
    const fresh = buildMenuItems(registry, mutedCache);
    renderMenu(fresh);
  }
}

export function initMenu() {
  // context menu routing + VOICE_MUTED subscription live in main.ts (composition
  // root) — subscribing here too made every mute event re-render the menu twice
  // MenuDismiss triggers: Escape and left-click outside menu items
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") dismissMenu("escape");
  });
  document.addEventListener("mousedown", (e) => {
    if (e.button !== 0 || !menuOpen) return;
    const t = e.target as Element | null;
    if (!t || !t.closest("#radial-menu button")) dismissMenu("outside");
  });
  // initial cache
  if (!testMode) {
    invoke<boolean>("get_muted").then((v) => { mutedCache = !!v; }).catch(() => {});
  }
}

// ContextMenuRouting helpers (for main.ts to call)
export function handleContextMenu(e: { clientX: number; clientY: number; preventDefault(): void }): boolean {
  const x = e.clientX;
  const y = e.clientY;
  e.preventDefault();
  if (!isOrbHit(x, y)) {
    return false;
  }
  if (menuOpen) {
    dismissMenu("outside");
  } else {
    if (registry.length > 0) {
      openMenu();
    }
  }
  return true;
}

export function setRegistryForTest(r: MenuItemDef[]) {
  registry = r;
}

export function getMutedCacheForTest() { return mutedCache; }
export function setMutedCacheForTest(v: boolean) { mutedCache = v; }
export function isMenuOpen() { return menuOpen; }
export function isMenuOpenForTest() { return menuOpen; }
