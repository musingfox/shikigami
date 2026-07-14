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

// Orb left press: double-click = talk, single press+hold = drag.
// startDragging() must fire on mousedown or macOS won't latch the native drag,
// and it then consumes the click — so talk can't be a single click. Instead we
// branch on e.detail: the 2nd mousedown of a double-click (detail===2) toggles
// talk; any other press starts the drag (a plain click just no-ops in place).
// A press that dismisses an open menu must not drag or talk.
const orb = document.getElementById("orb")!;
orb.addEventListener("mousedown", (e) => {
  if (e.button !== 0) return;
  if (!isOrbHit(e.clientX, e.clientY)) return;
  if (isMenuOpen()) return; // let the menu dismiss instead
  if (e.detail === 2) {
    toggleTalk();
    return;
  }
  try {
    getCurrentWindow().startDragging().catch(() => {});
  } catch {
    // browser preview: no Tauri runtime
  }
});
