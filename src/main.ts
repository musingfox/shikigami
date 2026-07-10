import { listen } from "@tauri-apps/api/event";
import { initAvatar } from "./avatar";
import { lipsync } from "./lipsync";
import { toast } from "./toast";

initAvatar(document.getElementById("orb") as HTMLCanvasElement);

// SumVox file-IPC events relayed by the Rust watcher.
listen<string>("sumvox:now-playing", (e) => lipsync(e.payload));
// history.log lines are "RFC3339\ttext"
listen<string>("sumvox:history", (e) => toast(e.payload.split("\t").slice(1).join("\t") || e.payload));
listen<boolean>("sumvox:muted", (e) => console.log("muted", e.payload));
