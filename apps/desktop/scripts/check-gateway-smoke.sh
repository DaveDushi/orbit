#!/usr/bin/env bash
# Runs `--check-gateway` of a built or packaged app (BIN) against the fake server without a
# gateway (`fake-gateway.py … nogateway`): probe, Orbit sign-in, /auth/me, both sockets,
# sign-out. No display and no person needed, so CI runs it on the macOS and Windows packages.
# It proves the packaged binary runs the check end to end; the real risk-4b answer still needs
# a person and the real gateway (apps/desktop/README.md).
set -euo pipefail
bin=$1
port=${PORT:-18765}
python=${PYTHON:-python3}
dir=$(mktemp -d)
printf 'smoke-%s%s\n' "$RANDOM" "$RANDOM" > "$dir/password"
"$python" -I "$(dirname "$0")/fake-gateway.py" "$port" "$dir/password" nogateway 2> "$dir/server.log" &
server=$!
trap 'kill $server 2>/dev/null || true; rm -rf "$dir"' EXIT
for _ in $(seq 50); do
    curl -fs "http://127.0.0.1:$port/api/v1/setup/status" > /dev/null && break
    sleep 0.2
done
ORBIT_CHECK_EMAIL=ada@example.test ORBIT_CHECK_PASSWORD_FILE="$dir/password" \
    "$bin" --check-gateway "http://127.0.0.1:$port" | tee "$dir/out.txt"
grep -q "Risk 4b: FORWARDED" "$dir/out.txt"
echo "check-gateway smoke: ok"
