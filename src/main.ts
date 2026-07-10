// Composition root: wires core events to the presentation modules.
// Only core event names appear here — adapter-specific formats stay in Rust.

import { listen } from "@tauri-apps/api/event";
import { initAvatar } from "./avatar";
import { AGENT_REPORT, AGENT_SPEECH, VOICE_MUTED, type Report } from "./events";
import { lipsync } from "./lipsync";
import { toast } from "./toast";

initAvatar(document.getElementById("orb") as HTMLCanvasElement);

listen<string>(AGENT_SPEECH, (e) => lipsync(e.payload));
listen<Report>(AGENT_REPORT, (e) => toast(e.payload.text));
listen<boolean>(VOICE_MUTED, (e) => console.log("muted", e.payload));
