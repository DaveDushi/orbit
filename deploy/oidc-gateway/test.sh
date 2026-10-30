#!/usr/bin/env bash
# Tests the gateway's rules against a fake Orbit that writes down what reaches it. Needs nginx,
# python3 and curl. Usage: deploy/oidc-gateway/test.sh [gateway port] [backend port]
set -euo pipefail
gateway=${1:-4207}
backend=${2:-4208}
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
pids=()
cleanup() { for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done; rm -rf "$work"; }
trap cleanup EXIT

cat >"$work/backend.py" <<PY
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer
class Handler(BaseHTTPRequestHandler):
    def answer(self):
        body = self.rfile.read(int(self.headers.get('content-length') or 0))
        with open('$work/seen.jsonl', 'a') as seen:
            seen.write(json.dumps({'line': self.command + ' ' + self.path, 'headers': {k.lower(): v for k, v in self.headers.items()}, 'body': body.decode()}) + '\n')
        self.send_response(200); self.send_header('Content-Length', '2'); self.end_headers(); self.wfile.write(b'ok')
    do_GET = do_POST = do_PUT = do_DELETE = do_HEAD = do_OPTIONS = answer
    def log_message(self, *args): pass
HTTPServer(('127.0.0.1', $backend), Handler).serve_forever()
PY
python3 "$work/backend.py" & pids+=($!)
grep -q '^        listen 127.0.0.1:8400;$' "$here/nginx.conf" || { echo 'FAILED - the gateway must listen on loopback only'; exit 1; }
[ "$(grep -c '^ *listen ' "$here/nginx.conf")" = 1 ] || { echo 'FAILED - the gateway must have one listener'; exit 1; }
sed -e "s/listen 127.0.0.1:8400;/listen 127.0.0.1:$gateway;/" -e "s#http://127.0.0.1:8080#http://127.0.0.1:$backend#" "$here/nginx.conf" >"$work/nginx.conf"
nginx -e stderr -p "$work/" -c "$work/nginx.conf" 2>"$work/error.log" & pids+=($!)
for _ in $(seq 50); do curl -s -o /dev/null "http://127.0.0.1:$gateway/" && break; sleep 0.1; done

failed=0
touch "$work/seen.jsonl"
status() { curl -s -o /dev/null -w '%{http_code}' --path-as-is "$@"; }
expect() {
    local want=$1 name=$2; shift 2
    local got; got=$(status "$@")
    if [ "$got" = "$want" ]; then echo "ok - $name"; else echo "FAILED - $name: wanted $want, got $got"; failed=1; fi
}
url=http://127.0.0.1:$gateway

expect 200 'POST /oauth/token passes' -X POST -d 'grant_type=x' "$url/oauth/token"
expect 200 'GET /oauth/jwks passes' "$url/oauth/jwks"
allowed=$(wc -l <"$work/seen.jsonl")
[ "$allowed" = 2 ] || { echo "FAILED - the backend saw $allowed requests, not 2"; failed=1; }

for path in / /login /health/ready /api/v1/auth/me /mcp /oauth/authorize /oauth/userinfo \
    /.well-known/openid-configuration /oauth/register /oauth/revoke \
    /oauth/token/ /oauth/jwks/ //oauth/token /oauth//token /oauth/./token /./oauth/token \
    /oauth/../oauth/token /oauth/../api/v1/auth/me /oauth/token/../../api/v1/auth/me \
    /oauth/%2e%2e/api/v1/auth/me /oauth/%2E%2E/api /oauth/..%2fapi /oauth%2ftoken /oauth/%74oken \
    /oauth/token%2f /oauth/token%20 '/oauth/token;x=1' '/oauth/token?x=1' \
    '/oauth/token?' '/oauth/jwks?a=b' /OAUTH/TOKEN /oauth/Token /OAUTH/JWKS /oauth/token.json; do
    expect 404 "POST $path is refused" -X POST -d 'x=1' "$url$path"
    expect 404 "GET $path is refused" "$url$path"
done
expect 400 'a NUL in the path is refused by nginx' "$url/oauth/token%00"
for method in GET PUT DELETE PATCH OPTIONS; do
    expect 404 "$method /oauth/token is refused" -X "$method" "$url/oauth/token"
done
for method in POST PUT DELETE PATCH OPTIONS; do
    expect 404 "$method /oauth/jwks is refused" -X "$method" "$url/oauth/jwks"
done
expect 404 'HEAD /oauth/jwks is refused' -I "$url/oauth/jwks"
expect 413 'a large body is refused' -X POST -H 'Expect:' -d "$(head -c 20000 /dev/zero | tr '\0' a)" "$url/oauth/token"
seen=$(wc -l <"$work/seen.jsonl")
[ "$seen" = "$allowed" ] || { echo "FAILED - a refused request reached the backend"; tail -n +3 "$work/seen.jsonl"; failed=1; }

# What reaches Orbit: only the allow-list, whatever the proxy and the client add.
curl -s -o /dev/null -X POST "$url/oauth/token" -u 'client:secret-value' -d 'code=abc' \
    -H 'X-ExeDev-UserID: forged-user' -H 'X-ExeDev-Email: forged@example.com' -H 'X-ExeDev-Anything: x' \
    -H 'Cookie: __Host-orbit_session=stolen' -H 'Origin: https://evil.example' -H 'X-Request-Id: forged' \
    -H 'X-Forwarded-Host: evil.example' -H 'X-Forwarded-Proto: http' -H 'Forwarded: for=1.1.1.1' \
    -H 'X-Real-IP: 1.1.1.1' -H 'Upgrade: websocket' -H 'User-Agent: secret-agent' \
    -H 'X-Forwarded-For: 198.51.100.7, 198.51.100.8' -H 'CF-Connecting-IP: 203.0.113.9' \
    -H 'CF-Ray: abc' -H 'Cf-Access-Jwt-Assertion: forged'
python3 - "$work/seen.jsonl" <<'PY' || failed=1
import base64, json, sys
last = json.loads(open(sys.argv[1]).read().splitlines()[-1])
h = last['headers']
want = {'host': 'orbit-oidc-gateway', 'connection': 'close', 'content-type': 'application/x-www-form-urlencoded',
        'content-length': '8', 'authorization': 'Basic ' + base64.b64encode(b'client:secret-value').decode(),
        'x-forwarded-for': '203.0.113.9', 'x-forwarded-proto': 'https'}
want.pop('connection'); h.pop('connection', None)
assert h == want, f'headers at Orbit: {h}'
assert last['body'] == 'code=abc' and last['line'] == 'POST /oauth/token', last
print('ok - only the allow-listed headers reach Orbit, and the client is CF-Connecting-IP, not X-Forwarded-For')
PY
# A forged X-Forwarded-For never counts, with or without a valid CF-Connecting-IP.
for value in 'not-an-address' '203.0.113.9, 203.0.113.10' '<script>' ''; do
    curl -s -o /dev/null "$url/oauth/jwks" -H "CF-Connecting-IP: $value" -H 'X-Forwarded-For: 198.51.100.7'
    python3 - "$work/seen.jsonl" <<'PY' || failed=1
import json, sys
last = json.loads(open(sys.argv[1]).read().splitlines()[-1])
assert last['headers']['x-forwarded-for'] == '127.0.0.1', last['headers']
print('ok - without one valid CF-Connecting-IP the client is the connection, not a forged value')
PY
done

# Nothing secret in the logs.
curl -s -o /dev/null -X POST "$url/oauth/token" -d 'client_secret=topsecretvalue&code=topsecretcode'
curl -s -o /dev/null "$url/x?client_secret=topsecretvalue"
if grep -rq topsecret "$work" --include='*.log' --exclude=seen.jsonl; then echo 'FAILED - a secret is in a log'; failed=1; else echo 'ok - no secret in the logs'; fi
if find "$work" -path '*_temp*' -type f | grep -q .; then echo 'FAILED - a body went to a temporary file'; failed=1; else echo 'ok - no body on disk'; fi

[ "$failed" = 0 ] && echo 'all checks passed'
exit "$failed"
