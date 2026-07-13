// Composition root: wires core events to the presentation modules.
// Mic capture (getUserMedia/Audio downsample) initialized via initMic(); see mic.ts for process_utterance bytes path.
// Only core event names appear here — adapter-specific formats stay in Rust.

import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { initAvatar } from "./avatar";
import { initMic, toggleTalk } from "./mic";
import { AGENT_REPORT, AGENT_SPEECH, VOICE_MUTED, type Report } from "./events";
import { lipsync } from "./lipsync";
import { toast } from "./toast";
import { initMenu, handleContextMenu, onMutedEvent, isOrbHit, isMenuOpen } from "./menu";

initAvatar(document.getElementById("orb") as HTMLCanvasElement);

listen<string>(AGENT_SPEECH, (e) => lipsync(e.payload));
initMic();
listen<Report>(AGENT_REPORT, (e) => toast(e.payload.text));
listen<boolean>(VOICE_MUTED, (e) => onMutedEvent(e.payload));

initMenu();
document.addEventListener("contextmenu", (e) => {
  handleContextMenu(e);
});

// Orb left press: the canvas has no drag-region attribute, so we route by hand —
// move >4px = window drag, still release on the orb = talk toggle (start/stop+send).
function beginDrag() {
  try {
    getCurrentWindow().startDragging().catch(() => {});
  } catch {
    // browser preview: no Tauri runtime
  }
}

const orb = document.getElementById("orb")!;
orb.addEventListener("mousedown", (e) => {
  if (e.button !== 0) return;
  const menuWasOpen = isMenuOpen(); // this press is a menu dismiss, not a talk toggle
  if (!isOrbHit(e.clientX, e.clientY)) {
    beginDrag();
    return;
  }
  const sx = e.clientX;
  const sy = e.clientY;
  let dragging = false;
  const onMove = (me: MouseEvent) => {
    if (!dragging && Math.hypot(me.clientX - sx, me.clientY - sy) > 4) {
      dragging = true;
      cleanup();
      beginDrag();
    }
  };
  const onUp = () => {
    cleanup();
    if (!dragging && !menuWasOpen) toggleTalk();
  };
  const cleanup = () => {
    document.removeEventListener("mousemove", onMove);
    document.removeEventListener("mouseup", onUp);
  };
  document.addEventListener("mousemove", onMove);
  document.addEventListener("mouseup", onUp);
});
