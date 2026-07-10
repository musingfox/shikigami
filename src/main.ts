import { listen } from "@tauri-apps/api/event";
import { initAvatar } from "./avatar";
import { lipsync } from "./lipsync";

initAvatar(document.getElementById("orb") as HTMLCanvasElement);

// SumVox file-IPC events relayed by the Rust watcher.
listen<string>("sumvox:now-playing", (e) => lipsync(e.payload));
listen<string>("sumvox:history", (e) => console.log("history", e.payload));
listen<boolean>("sumvox:muted", (e) => console.log("muted", e.payload));
