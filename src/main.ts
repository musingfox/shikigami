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
import { orbGesture } from "./orb-gesture";
import { initRoster, attribute } from "./roster";

initAvatar(document.getElementById("orb") as HTMLCanvasElement);

listen<string>(AGENT_SPEECH, (e) => lipsync(e.payload));
initMic();
initRoster();
listen<Report>(AGENT_REPORT, (e) => toast(attribute(e.payload.text)));
listen<boolean>(VOICE_MUTED, (e) => onMutedEvent(e.payload));

initMenu();
document.addEventListener("contextmenu", (e) => {
  handleContextMenu(e);
});

// Orb left press: double-click = talk, single press+hold = drag, menu-dismiss
// = neither. startDragging() must fire on mousedown or macOS won't latch the
// native drag, and it then consumes the click — so talk can't be a single
// click. orbGesture() owns the routing (and the sequence-latched menu guard);
// see orb-gesture.ts.
const orb = document.getElementById("orb")!;
orb.addEventListener("mousedown", (e) => {
  if (e.button !== 0) return;
  if (!isOrbHit(e.clientX, e.clientY)) return;
  const action = orbGesture(e.detail, isMenuOpen());
  if (action === "talk") {
    toggleTalk();
  } else if (action === "drag") {
    try {
      getCurrentWindow().startDragging().catch(() => {});
    } catch {
      // browser preview: no Tauri runtime
    }
  }
});
