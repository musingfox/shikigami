# Voice Setup

## Anthropic API Key (ApiKeyResolution)

The app resolves the key as follows:

1. `ANTHROPIC_API_KEY` environment variable (preferred)
2. `~/.config/shikigami/anthropic_api_key` (file contents trimmed)

If neither, error message includes both locations.

Create the file with your key:
```
mkdir -p ~/.config/shikigami
echo "sk-ant-..." > ~/.config/shikigami/anthropic_api_key
```

macOS note: `chmod 600 ~/.config/shikigami/anthropic_api_key`.

## Microphone Permission (MicUtteranceCapture)

Hold Cmd+Ctrl+M to record. Uses webview getUserMedia (downsampled 16 kHz mono f32 on-device).

macOS: first run may prompt; if dev spike, grant in System Settings > Privacy & Security > Microphone for the dev binary or built app.

The app Info.plist declares NSMicrophoneUsageDescription.

## Whisper Model (UtteranceTranscription)

STT runs on-device via whisper.cpp; the model is a one-time manual download
(multilingual base, ~142 MB — chosen for zh-TW/English mixed speech):

```
mkdir -p ~/.config/shikigami/models
curl -L -o ~/.config/shikigami/models/ggml-base.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin
```

Accuracy not good enough? Drop in `ggml-small.bin` (~466 MB, slower) and
change `MODEL_NAME` in `src-tauri/src/stt.rs` — one constant.

## Manual regression (end-to-end voice loop)

Hermetic tests cover every stage boundary; this checklist verifies the glue.
Prereqs: model file, API key, `sumvox` on PATH, `bun run tauri dev` running.

1. Hold Cmd+Ctrl+M — avatar brightens and pulses with your voice (mic
   permission prompt on first run).
2. Speak a short question (zh or en), release — toast shows 「你：<辨識文字>」.
3. Within a few seconds the reply appears as a toast, the avatar lip-syncs,
   and tray Recent gains the entry.
4. Tray → Mute, repeat — reply text still appears, no audio. Unmute after.
5. Rename the model file, repeat — toast shows `⚠ stt: whisper model not
   found: …/ggml-base.bin`, avatar returns to idle. Restore the file.
6. Ignored tests (need model/key): `cargo test -- --ignored` in `src-tauri/`.
