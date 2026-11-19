#!/usr/bin/env bash
# One command: ./evals/donna-codemode/run.sh — the security tests, then the eval (results.json). Needs Node 22.19+.
# The old mode runs the sidecar of BASELINE (default: the branch base, before code mode was the default).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$here/../.."
agent="$root/apps/agent"
export BASELINE="${BASELINE:-25e535d7}"
[ -d /root/.local/node/bin ] && export PATH="/root/.local/node/bin:$PATH"
[ -d "$agent/node_modules" ] || (cd "$agent" && npm ci --no-audit --no-fund)
# The eval imports the agent's packages: resolve them from apps/agent.
[ -e "$here/node_modules" ] || ln -s ../../apps/agent/node_modules "$here/node_modules"
rm -rf "$here/.baseline" && mkdir -p "$here/.baseline"
git -C "$root" -c safe.directory="$(cd "$root" && pwd)" archive "$BASELINE" apps/agent/src | tar -x -C "$here/.baseline"
ln -s "$(cd "$agent" && pwd)/node_modules" "$here/.baseline/apps/agent/node_modules"
node --test "$here"/*.test.ts
node "$here/run.ts"
