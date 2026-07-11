# Voice Setup

## Brain provider + API key (ApiKeyResolution)

Three providers are supported; the app picks the first one with a key
available, cheapest first: **gemini → openai → anthropic**. Override with
`SHIKIGAMI_BRAIN=gemini|openai|anthropic`.

Per provider, the key resolves env-first, then a key file (contents trimmed):

| Provider | Env | Key file (`~/.config/shikigami/`) | Model |
|---|---|---|---|
| gemini | `GEMINI_API_KEY` | `gemini_api_key` | gemini-2.5-flash |
| openai | `OPENAI_API_KEY` | `openai_api_key` | gpt-4.1-mini |
| anthropic | `ANTHROPIC_API_KEY` | `anthropic_api_key` | claude-haiku-4-5 |

If no key is found anywhere, the error names all three env vars.

Create a key file:
```
mkdir -p ~/.config/shikigami
echo "AIza..." > ~/.config/shikigami/gemini_api_key
chmod 600 ~/.config/shikigami/gemini_api_key
```

Models are constants in `src-tauri/src/brain.rs` (`Provider::model`).

## Microphone Permission (MicUtteranceCapture)

Hold Cmd+Ctrl+M to record. Uses webview getUserMedia (downsampled 16 kHz mono f32 on-device).

macOS: first run may prompt; if dev spike, grant in System Settings > Privacy & Security > Microphone for the dev binary or built app.

The app Info.plist declares NSMicrophoneUsageDescription.

## Whisper Model (UtteranceTranscription)

STT runs on-device via whisper.cpp. Default model = **Breeze-ASR-25 q5_k**
(MediaTek whisper-large-v2 fine-tune for Taiwanese Mandarin + zh/en
code-switching, ~1 GB), one-time manual download:

```
mkdir -p ~/.config/shikigami/models
curl -L -o ~/.config/shikigami/models/breeze-asr-25-q5_k.bin \
  https://huggingface.co/alan314159/Breeze-ASR-25-whispercpp/resolve/main/ggml-model-q5_k.bin
```

Language is pinned to zh (`SHIKIGAMI_STT_LANG` to override). Swap models with
`SHIKIGAMI_STT_MODEL` (filename under `models/`, or absolute path) — e.g.
`ggml-base.bin` for a fast generic multilingual model.

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
