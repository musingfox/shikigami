// Idle window hugs the avatar (small); it grows to fit whatever is on screen,
// then shrinks back. Position is compensated on every resize so the avatar's
// on-screen spot never moves. Ref-counted so several holders can coexist.
// ponytail: three fixed sizes, no per-item measuring — the avatar is centered.

import { getCurrentWindow, LogicalSize, PhysicalPosition } from "@tauri-apps/api/window";

// 176: leaves ~13px of breathing room past the roster arc so dock-magnified
// bubbles don't clip at the window edge
const SMALL = { w: 176, h: 176, cx: 88, cy: 88 };
// A lone report needs 錨點 64 + 泡泡 66 = 130 from the center; 150 leaves room.
// agent:report is pushed from outside, not asked for, so this is the size the
// window sits at most of the time — paying 592 for it means a transparent but
// mouse-blocking 320×592 rectangle for every report, 3s at a time.
const MEDIUM = { w: 240, h: 300, cx: 120, cy: 150 };
// 592: the full outer ring — 錨點 64 + 確認框 120 + gap 8 + 報告 66 + gap 8 +
// 名牌 20 = 286, rounded to 296 and doubled. Also what the radial menu needs.
const LARGE = { w: 320, h: 592, cx: 160, cy: 296 };

// Two counters, not a holder registry: the only question the window asks is
// "does anyone need the full ring?" — a report alone answers no.
let bigRefs = 0;
let midRefs = 0;
let frame = SMALL;
let chain: Promise<void> = Promise.resolve();

// Notified AFTER the size flag flips — the DOM resize event races the flag
// (it fires mid-applySize while currentCenter still reports the old center),
// so geometry consumers subscribe here instead.
const frameListeners: (() => void)[] = [];
export function onFrameChange(fn: () => void) {
  frameListeners.push(fn);
}

// Window center in CSS (logical) px — the coordinate space of clientX/clientY.
export function currentCenter(): { x: number; y: number } {
  return { x: frame.cx, y: frame.cy };
}

export const LARGE_W = LARGE.w;
export const LARGE_H = LARGE.h;

function wanted() {
  return bigRefs > 0 ? LARGE : midRefs > 0 ? MEDIUM : SMALL;
}

async function applySize(target: typeof SMALL) {
  if (target === frame) return;
  const growing = target.w > frame.w;
  try {
    const win = getCurrentWindow();
    const sf = await win.scaleFactor();
    const [pos, size] = await Promise.all([win.outerPosition(), win.outerSize()]);
    // keep the avatar (window center) pinned: newTopLeft = center - targetPhys/2
    const tw = target.w * sf;
    const th = target.h * sf;
    const nx = Math.round(pos.x + size.width / 2 - tw / 2);
    const ny = Math.round(pos.y + size.height / 2 - th / 2);
    if (growing) {
      await win.setSize(new LogicalSize(target.w, target.h));
      await win.setPosition(new PhysicalPosition(nx, ny));
    } else {
      await win.setPosition(new PhysicalPosition(nx, ny));
      await win.setSize(new LogicalSize(target.w, target.h));
    }
  } catch {
    // browser preview / no Tauri runtime: still flip so geometry uses the right center
  }
  frame = target;
  frameListeners.forEach((f) => f());
}

function settle(): Promise<void> {
  chain = chain.then(() => applySize(wanted()));
  return chain;
}

// Full ring: the radial menu and the confirm bubble.
export function acquireLarge(): Promise<void> {
  bigRefs++;
  return settle();
}

export function releaseLarge(): Promise<void> {
  bigRefs = Math.max(0, bigRefs - 1);
  return settle();
}

// One bubble talking: a report only ever needs the first slot.
export function acquireMedium(): Promise<void> {
  midRefs++;
  return settle();
}

export function releaseMedium(): Promise<void> {
  midRefs = Math.max(0, midRefs - 1);
  return settle();
}

// test hooks
export function __sizeForTest() {
  return frame === LARGE ? "large" : frame === MEDIUM ? "medium" : "small";
}
export function __isLargeForTest() { return frame === LARGE; }
export function __refsForTest() { return bigRefs + midRefs; }
export function __resetFrameForTest() {
  bigRefs = 0;
  midRefs = 0;
  frame = SMALL;
  chain = Promise.resolve();
}
