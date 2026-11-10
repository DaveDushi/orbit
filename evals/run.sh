#!/usr/bin/env bash
# Orbit perf + security evals. See evals/README.md.
#   evals/run.sh [--perf] [--security] [--quick] [--build] [--no-prod] [--noise[=N]]
# Without --perf/--security it runs both. Writes evals/results/<date>-<sha>.json and exits non-zero on a budget
# overrun, a perf regression beyond noise, a new high/critical security finding, or a step that failed.
# --noise[=N]: N perf runs (default 5) of this commit, kept out of evals/results; writes evals/noise.json.
set -euo pipefail

usage() { sed -n '2,5p' "$0" | sed 's/^# \{0,1\}//'; }
perf=0 security=0 quick=0 build=0 prod=1 noise=0
for argument in "$@"; do
    case $argument in
        --perf) perf=1 ;;
        --security) security=1 ;;
        --quick) quick=1 ;;
        --build) build=1 ;;
        --no-prod) prod=0 ;;
        --noise) noise=5 ;;
        --noise=[2-9] | --noise=[1-9][0-9]) noise=${argument#--noise=} ;;
        -h | --help) usage; exit 0 ;;
        *) usage >&2; exit 64 ;;
    esac
done
[[ $perf == 0 && $security == 0 ]] && perf=1 security=1

root=$(git rev-parse --show-toplevel)
cd "$root"

# The whole run (build, server, browser, scanners) holds the shared build lock and runs in the build slice.
if [[ -x /usr/local/bin/orbit-build && ${ORBIT_DEV_BUILD_GUARD_ACTIVE:-} != 1 ]]; then
    exec /usr/local/bin/orbit-build "$0" "$@"
fi

export CARGO_INCREMENTAL=0

# Noise: the same perf run N times in a row under this one lock, then the spread of every metric.
if [[ $noise -gt 0 ]]; then
    [[ $build == 1 ]] && "$root/evals/lib/build.sh"
    runs=$(mktemp -d /tmp/orbit-evals-noise.XXXXXX)
    trap 'rm -rf "$runs"' EXIT
    for run in $(seq 1 "$noise"); do
        echo "=== noise run $run/$noise"
        # Exit 1 (over budget, worse than the run before) is fine here; 2 is a failed step.
        status=0
        EVALS_RESULTS_DIR=$runs "$0" --perf --no-prod $([[ $quick == 1 ]] && echo --quick) || status=$?
        if [[ $status -gt 1 ]]; then
            echo "noise run $run failed ($status)" >&2
            exit 2
        fi
    done
    bun "$root/evals/src/noise.ts" "$runs"/*.json
    exit 0
fi
export PATH="$HOME/.cargo/bin:$PATH"
# On the shared VM node/npm live in ~/.local/node/bin (npm audit of apps/agent needs both).
if ! command -v npm >/dev/null && [[ -x $HOME/.local/node/bin/npm ]]; then export PATH="$HOME/.local/node/bin:$PATH"; fi
if [[ -z ${PLAYWRIGHT_CHROMIUM_EXECUTABLE:-} ]]; then
    PLAYWRIGHT_CHROMIUM_EXECUTABLE=$(find "$HOME/.cache/ms-playwright" -maxdepth 3 -name chrome-headless-shell -type f 2>/dev/null | sort | tail -1)
    export PLAYWRIGHT_CHROMIUM_EXECUTABLE
fi

EVALS_WORK=$(mktemp -d /tmp/orbit-evals.XXXXXX)
EVALS_QUICK=$quick
EVALS_PORT=$(bun -e 'const s = Bun.listen({ hostname: "127.0.0.1", port: 0, socket: { data() {} } }); console.log(s.port); s.stop(true)')
# A fresh key for the throwaway data only; it never leaves $EVALS_WORK's process environment.
APP_KEY=$(openssl rand -base64 32)
ORBIT_BIN="$root/target/release/orbit"
export EVALS_WORK EVALS_QUICK
SERVER_PID=
source "$root/evals/lib/instance.sh"
cleanup() {
    stop_instance
    rm -rf "$EVALS_WORK"
}
trap cleanup EXIT
started=$(date +%s)
mkdir -p "$EVALS_WORK/parts"

# step <name> <command...>: runs one step; a failure is recorded and the run goes on. `set -e` does not apply inside
# `if`, so step functions add `|| return 1` to each command that must not fail.
step() {
    local name=$1
    shift
    echo "== $name"
    local begin=$(date +%s)
    if "$@"; then
        echo "   $name: $(($(date +%s) - begin)) s"
    else
        echo "   $name FAILED" >&2
        echo "$name" >>"$EVALS_WORK/errors.txt"
        stop_instance
    fi
}

"$root/evals/lib/install-tools.sh"
[[ $build == 1 ]] && step build "$root/evals/lib/build.sh"
if [[ ! -x $ORBIT_BIN ]]; then
    echo "no release binary at target/release/orbit: run evals/run.sh --build once" >&2
    exit 2
fi
# Build numbers of the binary under test, if a build of this commit measured them.
if [[ -f $root/target/evals-build.json ]] && grep -q "\"$(git rev-parse HEAD)\"" "$root/target/evals-build.json"; then
    bun -e "const b = require('$root/target/evals-build.json'); b.measured_in_this_run = $build === 1; require('fs').writeFileSync('$EVALS_WORK/parts/build.json', JSON.stringify(b))"
fi

seed() {
    local url
    url=$(orbit_cli development setup-token init --origin "http://127.0.0.1:$EVALS_PORT" | grep -o 'token=[A-Za-z0-9_-]*' | tail -1)
    start_instance development ORBIT_RATE_LIMIT_GENERAL_PER_MINUTE=1000000 || return 1
    echo "{\"fresh_start_ms\": $STARTUP_MS}" >"$EVALS_WORK/fresh-start.json"
    bun "$root/evals/src/seed.ts" "${url#token=}" || return 1
    stop_instance
}

perf_server() {
    # Cold start on the seeded database, then memory idle, after the latency run and after the page loads.
    start_instance development ORBIT_RATE_LIMIT_GENERAL_PER_MINUTE=1000000 || return 1
    local cold=$STARTUP_MS idle after_latency after_pages
    sleep 5
    idle=$(server_rss)
    bun "$root/evals/src/latency.ts" || return 1
    after_latency=$(server_rss)
    bun "$root/evals/src/pageload.ts" || return 1
    after_pages=$(server_rss)
    stop_instance
    local fresh
    fresh=$(sed 's/[^0-9]//g' "$EVALS_WORK/fresh-start.json")
    cat >"$EVALS_WORK/parts/server.json" <<JSON
{"cold_start_ms": $cold, "fresh_start_ms": $fresh, "rss_idle_mb": ${idle% *}, "rss_after_latency_mb": ${after_latency% *}, "rss_after_pageload_mb": ${after_pages% *}, "rss_peak_mb": ${after_pages#* }}
JSON
}

security_local() {
    start_instance production || return 1
    bun "$root/evals/src/security-local.ts" || return 1
    stop_instance
}

step seed seed
if [[ $perf == 1 ]]; then
    step bundle bun "$root/evals/src/bundle.ts"
    step server-latency-pageload perf_server
fi
if [[ $security == 1 ]]; then
    step supply-chain bun "$root/evals/src/supply-chain.ts"
    [[ $prod == 1 ]] && step security-prod bun "$root/evals/src/security-prod.ts"
    step security-local security_local
fi

bun "$root/evals/src/report.ts" --duration "$(($(date +%s) - started))" --perf "$perf" --security "$security" --quick "$quick" --build "$build" --prod "$prod"
