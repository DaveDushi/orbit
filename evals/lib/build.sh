#!/usr/bin/env bash
# Builds the web app and the release `orbit` binary, and records wall time and peak memory of each step.
# Usage: evals/lib/build.sh. Writes target/evals-build.json next to the binary, so later runs without --build can
# report the numbers of the build that made it. Runs every heavy step through orbit-build when it is installed.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
out="$root/target/evals-build.json"
log_dir=$(mktemp -d)
trap 'rm -rf "$log_dir"' EXIT
export CARGO_INCREMENTAL=0
guard=()
[ -x /usr/local/bin/orbit-build ] && guard=(/usr/local/bin/orbit-build)
revision=$(git -C "$root" rev-parse HEAD)

# Prints "<wall seconds> <peak RSS kB>" from a GNU time -v log.
measure() {
    local wall rss
    wall=$(sed -n 's/.*Elapsed (wall clock) time.*: //p' "$1" | awk -F: '{ s = 0; for (i = 1; i <= NF; i++) s = s * 60 + $i; print s }')
    rss=$(sed -n 's/.*Maximum resident set size (kbytes): //p' "$1")
    echo "$wall $rss"
}

(cd "$root/apps/web" && ORBIT_BUILD_REVISION="$revision" "${guard[@]}" /usr/bin/time -v -o "$log_dir/web" bun run build >"$log_dir/web.out" 2>&1) ||
    { tail -40 "$log_dir/web.out" >&2; exit 1; }
read -r web_wall web_rss < <(measure "$log_dir/web")

# A clean release build, so the numbers do not depend on what an earlier build left.
rm -rf "$root/target/release"
(cd "$root" && ORBIT_BUILD_REVISION="$revision" "${guard[@]}" /usr/bin/time -v -o "$log_dir/cargo" cargo build --release -p orbit-server >"$log_dir/cargo.out" 2>&1) ||
    { tail -40 "$log_dir/cargo.out" >&2; exit 1; }
read -r cargo_wall cargo_rss < <(measure "$log_dir/cargo")

binary="$root/target/release/orbit"
cat >"$out" <<JSON
{
  "revision": "$revision",
  "web_build_seconds": $web_wall,
  "web_build_peak_rss_mb": $((web_rss / 1024)),
  "release_build_seconds": $cargo_wall,
  "release_build_peak_rss_mb": $((cargo_rss / 1024)),
  "release_binary_mb": $(($(stat -c %s "$binary") / 1048576)),
  "cargo_jobs": ${CARGO_BUILD_JOBS:-1}
}
JSON
