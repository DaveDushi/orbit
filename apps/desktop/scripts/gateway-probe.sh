#!/usr/bin/env bash
# Read-only checks of the exe.dev gateway with a per-user VM token (M0 questions 3 and 4).
# Usage: gateway-probe.sh TOKEN_FILE [HOST]   (HOST defaults to orbit.wellrox.com)
# Sends GETs only, never prints the token, and uses no Orbit session.
set -euo pipefail

token_file=${1:?usage: gateway-probe.sh TOKEN_FILE [HOST]}
host=${2:-orbit.wellrox.com}
gateway=(-H "X-Exedev-Authorization: Bearer $(cat "$token_file")")
upgrade=(--http1.1 -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13"
    -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==")
# A UUIDv7 no workspace has: Orbit checks the origin rule before it looks the workspace up.
workspace=00000000-0000-7000-8000-000000000000

status() { curl -sS -m 15 -o /tmp/gateway-probe.body -w '%{http_code} %{content_type}' "$@"; }
body() { head -c 160 /tmp/gateway-probe.body | tr '\n' ' '; echo; }

echo "1. Lifetime: the token's signed payload (exp, nbf; absent exp = never expires)"
payload=$(cut -d. -f2 "$token_file" | tr '_-' '/+')
while (( ${#payload} % 4 )); do payload+='='; done
printf '%s' "$payload" | base64 -d | jq -c '{exp, nbf, exp_utc: (if .exp then (.exp | todate) else null end)}'

echo "2. HTTP with the token (want 200 application/json from Orbit)"
status "${gateway[@]}" "https://$host/api/v1/setup/status"; echo; body

for socket in live events; do
    path="/api/v1/workspaces/$workspace/$socket"
    echo "3. $socket upgrade with the token, no Orbit session (want Orbit's 403 origin_forbidden: the upgrade reached Orbit)"
    status "${upgrade[@]}" "${gateway[@]}" "https://$host$path"; echo; body
    echo "4. $socket upgrade with the token and Authorization: Bearer (not 403 origin_forbidden: Orbit got the bearer header)"
    status "${upgrade[@]}" "${gateway[@]}" -H "Authorization: Bearer not-a-session" "https://$host$path"; echo; body
done
