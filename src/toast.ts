// Typewriter speech bubble for the latest notification, mirroring
// SumVox's Swift toast. Placed between the avatar and the screen
// center: above/below the blob and leaning toward the center side.

import { currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";
import { acquireLarge, releaseLarge } from "./window-frame";

const CHAR_MS = 30;
const HOLD_MS = 3000;

let gen = 0;

// hold the window large while a toast is visible (idle window is too small for it)
let holding = false;
function hold() {
  if (!holding) {
    holding = true;
    acquireLarge();
  }
}
function unhold() {
  if (holding) {
    holding = false;
    releaseLarge();
  }
}

// The whole outer ring shares one direction, so the two classes live on the
// #aura container and every slot inherits them. Recomputed at each show, NOT
// on onFrameChange: dragging the window changes the answer and dragging only
// moves the window, it never flips the size flag.
export async function orient() {
  if (typeof document === "undefined") return;
  const el = document.getElementById("aura");
  if (!el) return;
  try {
    const win = getCurrentWindow();
    const [pos, size, mon] = await Promise.all([
      win.outerPosition(),
      win.outerSize(),
      currentMonitor(),
    ]);
    if (!mon) return;
    const wcx = pos.x + size.width / 2;
    const wcy = pos.y + size.height / 2;
    const scx = mon.position.x + mon.size.width / 2;
    const scy = mon.position.y + mon.size.height / 2;
    el.classList.toggle("flip", wcy > scy); // avatar below center → grow upward
    el.classList.toggle("lean-left", scx < wcx);
  } catch {
    // default placement (below, leaning right)
  }
}

export function toast(text: string) {
  const el = document.getElementById("toast")!;
  const my = ++gen;
  el.textContent = "";
  hold();
  orient().then(() => {
    if (my !== gen) return;
    el.classList.add("show");
    let i = 0;
    const type = () => {
      if (my !== gen) return;
      if (i < text.length) {
        el.textContent = text.slice(0, ++i);
        setTimeout(type, CHAR_MS);
      } else {
        setTimeout(() => {
          if (my === gen) {
            el.classList.remove("show");
            unhold();
          }
        }, HOLD_MS);
      }
    };
    type();
  });
}
