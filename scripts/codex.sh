#!/usr/bin/env bash
# Run the Codex plugin companion (codex@openai-codex for Claude Code) from a plain shell.
#
# The plugin's slash commands resolve ${CLAUDE_PLUGIN_ROOT} themselves, but an agent calling
# Bash does not have that variable, and the install path contains the plugin version. This
# wrapper finds the installed companion and forwards every argument to it.
#
#   scripts/codex.sh task --write --prompt-file brief.md   # delegated implementation
#   scripts/codex.sh task --prompt-file brief.md           # read-only investigation
#   scripts/codex.sh review --wait                         # native review of the working tree
#   scripts/codex.sh adversarial-review --wait "<focus>"   # challenge review
#   scripts/codex.sh status | result [job-id] | cancel [job-id]
set -euo pipefail

registry="${HOME}/.claude/plugins/installed_plugins.json"
root=""
if [[ -f "$registry" ]]; then
  root="$(python3 -I -c 'import json,sys
d = json.load(open(sys.argv[1]))
entries = d.get("plugins", {}).get("codex@openai-codex", [])
print(entries[-1]["installPath"] if entries else "")' "$registry" 2>/dev/null || true)"
fi
if [[ -z "$root" || ! -f "$root/scripts/codex-companion.mjs" ]]; then
  # Fall back to the newest cached version.
  root="$( (ls -d "${HOME}"/.claude/plugins/cache/openai-codex/codex/*/ 2>/dev/null || true) | sort -V | tail -1)"
  root="${root%/}"
fi
companion="$root/scripts/codex-companion.mjs"
if [[ ! -f "$companion" ]]; then
  echo "codex.sh: Codex plugin not found. Install it in Claude Code, then run /codex:setup." >&2
  exit 127
fi
exec node "$companion" "$@"
