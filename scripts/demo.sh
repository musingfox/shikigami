#!/bin/sh
# Fire a fake SumVox notification at shikigami: speech toast + RMS lip-sync.
# Usage: scripts/demo.sh [text]
set -e

DIR="$HOME/.config/sumvox"
TEXT="${1:-式神測試通知,泡泡與嘴型應同步出現}"
WAV="$(mktemp -t shiki_demo).wav"

say "$TEXT" -o "$WAV" --data-format=LEF32@22050
printf '%s\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$TEXT" >> "$DIR/history.log"
printf '%s' "$WAV" > "$DIR/now_playing"
afplay "$WAV"
