#!/bin/sh
# Claude Code → shikigami activity signal (channel ①, R1b).
# Appends one NDJSON line per hook firing to ~/.config/shikigami/hooks.ndjson;
# shikigami tails that spool and emits agent:activity (identity only — it
# never toasts; SumVox keeps owning the spoken/toasted report).
# Must never block or fail the agent: always exits 0.
#
# Install — add to ~/.claude/settings.json:
#   "hooks": {
#     "Stop": [{ "hooks": [{ "type": "command",
#       "command": "/ABS/PATH/TO/shikigami/scripts/cc-hook.sh stop" }] }],
#     "Notification": [{ "hooks": [{ "type": "command",
#       "command": "/ABS/PATH/TO/shikigami/scripts/cc-hook.sh notification" }] }]
#   }
dir="$HOME/.config/shikigami"
mkdir -p "$dir" 2>/dev/null
payload=$(cat 2>/dev/null)
[ -n "$payload" ] || payload='{}'
printf '{"kind":"%s","pane":"%s","ts":"%s","payload":%s}\n' \
  "${1:-unknown}" "${HERDR_PANE_ID:-${TMUX_PANE:-}}" \
  "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$payload" \
  >> "$dir/hooks.ndjson" 2>/dev/null
exit 0
