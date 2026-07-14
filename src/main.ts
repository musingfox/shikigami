// Composition root: wires core events to the presentation modules.
// Mic capture (getUserMedia/Audio downsample) initialized via initMic(); see mic.ts for process_utterance bytes path.
// Only core event names appear here — adapter-specific formats stay in Rust.

import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { initAvatar } from "./avatar";
import { initMic } from "./mic";
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

// Orb left press = window drag, started immediately so it feels native (macOS
// takes over the mouse the moment startDragging is called; a deferred call
// wouldn't latch onto the drag). The canvas has no drag-region attribute so we
// call it by hand. A press that dismisses an open menu must not also drag.
// Talk lives on the Cmd+Ctrl+M push-to-talk hotkey.
const orb = document.getElementById("orb")!;
orb.addEventListener("mousedown", (e) => {
  if (e.button !== 0) return;
  if (!isOrbHit(e.clientX, e.clientY)) return;
  if (isMenuOpen()) return; // let the menu dismiss instead of dragging
  try {
    getCurrentWindow().startDragging().catch(() => {});
  } catch {
    // browser preview: no Tauri runtime
  }
});
