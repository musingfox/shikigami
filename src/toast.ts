// Typewriter speech bubble for the latest notification, mirroring
// SumVox's Swift toast. Placed between the avatar and the screen
// center: above/below the blob and leaning toward the center side.

import { currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";

const CHAR_MS = 30;
const HOLD_MS = 3000;

let gen = 0;

async function place(el: HTMLElement) {
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
    el.classList.toggle("above", wcy > scy); // avatar below center → bubble above
    el.classList.toggle("lean-left", scx < wcx);
  } catch {
    // default placement (below, centered)
  }
}

export function toast(text: string) {
  const el = document.getElementById("toast")!;
  const my = ++gen;
  el.textContent = "";
  place(el).then(() => {
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
          if (my === gen) el.classList.remove("show");
        }, HOLD_MS);
      }
    };
    type();
  });
}
