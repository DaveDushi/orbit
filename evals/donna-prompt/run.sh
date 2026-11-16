#!/usr/bin/env bash
# One command: ./evals/donna-prompt/run.sh [label] — runs the 35 cases in today's (direct) mode and writes
# results-<label>.json (default label: current). Needs Node 22.19+.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
agent="$here/../../apps/agent"
[ -d /root/.local/node/bin ] && export PATH="/root/.local/node/bin:$PATH"
[ -d "$agent/node_modules" ] || (cd "$agent" && npm ci --no-audit --no-fund)
# The eval imports the agent's packages: resolve them from apps/agent.
[ -e "$here/node_modules" ] || ln -s ../../apps/agent/node_modules "$here/node_modules"
node "$here/run.ts" "${1:-current}"
