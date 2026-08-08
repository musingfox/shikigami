// Idle window hugs the avatar (small); it grows to fit the radial menu or a
// toast, then shrinks back. Position is compensated on every resize so the
// avatar's on-screen spot never moves. Ref-counted so menu + toast can both
// hold it large without fighting.
// ponytail: two fixed sizes, no per-item measuring — the avatar is centered.

import { getCurrentWindow, LogicalSize, PhysicalPosition } from "@tauri-apps/api/window";

// 176: leaves ~13px of breathing room past the roster arc so dock-magnified
// bubbles don't clip at the window edge
const SMALL = { w: 176, h: 176, cx: 88, cy: 88 };
// 592: the outer ring needs 中心到邊 = 錨點 64 + 確認框 120 + gap 8 + 報告 66
// + gap 8 + 名牌 20 = 286；取 296 留餘裕後 ×2。三個泡泡同時在場才不會被裁掉。
const LARGE = { w: 320, h: 592, cx: 160, cy: 296 };

let large = false;
let refs = 0;
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
  return large ? { x: LARGE.cx, y: LARGE.cy } : { x: SMALL.cx, y: SMALL.cy };
}

export const LARGE_W = LARGE.w;
export const LARGE_H = LARGE.h;

async function applySize(toLarge: boolean) {
  if (toLarge === large) return;
  const target = toLarge ? LARGE : SMALL;
  try {
    const win = getCurrentWindow();
    const sf = await win.scaleFactor();
    const [pos, size] = await Promise.all([win.outerPosition(), win.outerSize()]);
    // keep the avatar (window center) pinned: newTopLeft = center - targetPhys/2
    const tw = target.w * sf;
    const th = target.h * sf;
    const nx = Math.round(pos.x + size.width / 2 - tw / 2);
    const ny = Math.round(pos.y + size.height / 2 - th / 2);
    if (toLarge) {
      await win.setSize(new LogicalSize(target.w, target.h));
      await win.setPosition(new PhysicalPosition(nx, ny));
    } else {
      await win.setPosition(new PhysicalPosition(nx, ny));
      await win.setSize(new LogicalSize(target.w, target.h));
    }
  } catch {
    // browser preview / no Tauri runtime: still flip so geometry uses the right center
  }
  large = toLarge;
  frameListeners.forEach((f) => f());
}

export function acquireLarge(): Promise<void> {
  refs++;
  chain = chain.then(() => applySize(true));
  return chain;
}

export function releaseLarge(): Promise<void> {
  refs = Math.max(0, refs - 1);
  if (refs === 0) chain = chain.then(() => applySize(false));
  return chain;
}

// test hooks
export function __isLargeForTest() { return large; }
export function __refsForTest() { return refs; }
export function __resetFrameForTest() { large = false; refs = 0; chain = Promise.resolve(); }
