// Voice loop coordinator (PTT -> mic capture -> stt -> brain -> sumvox say/report).
// For BrainReply contract: exposes reply step.
// ponytail: macOS cfg only for hotkeys; core logic cross.

use crate::brain;

pub async fn reply_to_transcript(transcript: &str) -> Result<String, String> {
    if transcript.trim().is_empty() {
        return Err("empty transcript".to_string());
    }
    brain::ask_claude(transcript).await
}
