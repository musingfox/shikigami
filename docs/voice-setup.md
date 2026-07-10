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

macOS note: file perms should be 600.
