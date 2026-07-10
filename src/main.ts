import { listen } from "@tauri-apps/api/event";
import { initAvatar } from "./avatar";

initAvatar(document.getElementById("orb") as HTMLCanvasElement);

// SumVox file-IPC events relayed by the Rust watcher.
// ponytail: console-only for now — lip-sync and toast tasks consume these next.
listen<string>("sumvox:now-playing", (e) => console.log("now-playing", e.payload));
listen<string>("sumvox:history", (e) => console.log("history", e.payload));
listen<boolean>("sumvox:muted", (e) => console.log("muted", e.payload));
