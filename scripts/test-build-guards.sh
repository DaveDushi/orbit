#!/usr/bin/env bash
# No sudo, Docker daemon, or systemd access: all privileged entry points are mocked.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/worktree-a" "$tmp/worktree-b"
touch "$tmp/build.lock" "$tmp/orbit-builds.slice" "$tmp/log"
export GUARD_TEST_TMP="$tmp" MOCK_UID=1000 MOCK_GID=1000
unset ORBIT_DEV_BUILD_GUARD_ACTIVE

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
expect_status() {
    local expected=$1 actual=0
    shift
    "$@" >"$tmp/stdout" 2>"$tmp/stderr" || actual=$?
    [[ "$actual" == "$expected" ]] || fail "expected status $expected, got $actual: $(cat "$tmp/stderr")"
}
wait_for_file() {
    local attempt
    for (( attempt=0; attempt<200; attempt++ )); do
        [[ ! -e "$1" ]] || return 0
        sleep 0.01
    done
    fail "timed out waiting for $1"
}
cat >"$tmp/bin/uname" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "${MOCK_OS:-Linux}"
MOCK
cat >"$tmp/bin/id" <<'MOCK'
#!/usr/bin/env bash
case "$1" in -u) echo "$MOCK_UID";; -g) echo "$MOCK_GID";; *) exit 90;; esac
MOCK
cat >"$tmp/bin/sudo" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
[[ ${ORBIT_DEV_BUILD_GUARD_ACTIVE:-} != 1 ]]
printf 'sudo %s\n' "$*" >>"$GUARD_TEST_TMP/log"
if [[ $1 == -n ]]; then
    [[ $2 == -E ]]
    shift 2
else
    [[ $1 == -u && $2 == "#$MOCK_UID" && $3 == -g && $4 == "#$MOCK_GID" && $5 == --preserve-env && $6 == -- ]]
    shift 6
fi
# Model sudo closing inherited FDs, which must not release the parent's lock.
exec 9>&-
export PATH="$GUARD_TEST_TMP/bin:/usr/bin:/bin"
exec "$@"
MOCK
cat >"$tmp/bin/systemd-run" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
[[ ${ORBIT_DEV_BUILD_GUARD_ACTIVE:-} != 1 ]]
[[ $1 == --scope && $2 == --quiet && $3 == --slice=orbit-builds.slice && $4 == --expand-environment=no && $5 == "--working-directory=$PWD" && $6 == -- ]]
printf 'scope\n' >>"$GUARD_TEST_TMP/log"
shift 6
[[ ${MOCK_SYSTEMD_FAILURE:-0} == 0 ]] || exit "$MOCK_SYSTEMD_FAILURE"
exec "$@"
MOCK
chmod +x "$tmp/bin/"*
export PATH="$tmp/bin:$PATH"
for worktree in worktree-a worktree-b; do
    sed -e "s|/var/lib/orbit-builds/build.lock|$tmp/build.lock|g" \
        -e "s|/etc/systemd/system/orbit-builds.slice|$tmp/orbit-builds.slice|g" \
        "$repo/scripts/build-limited.sh" >"$tmp/$worktree/orbit-build"
    chmod +x "$tmp/$worktree/orbit-build"
done
runner="$tmp/worktree-a/orbit-build"

expect_status 64 "$runner"
expect_status 0 env CARGO_BUILD_JOBS=99 TEST_GUARD_VALUE='hello world' "$runner" bash -c '
    [[ $ORBIT_DEV_BUILD_GUARD_ACTIVE == 1 && $CARGO_BUILD_JOBS == 1 ]]
    [[ $TEST_GUARD_VALUE == "hello world" && $PWD == "$1" && $PATH == "$2" ]]
    [[ $3 == "$literal argument" && $4 == "a b" ]]
    read -r input
    [[ $input == "stdin survives" ]]
' _ "$PWD" "$PATH" '$literal argument' 'a b' <<< 'stdin survives'
expect_status 37 "$runner" bash -c 'exit 37'
expect_status 127 "$runner" orbit-nonexistent-test-command
# Signal only the guard, after its actual command has installed an INT handler.
# timeout also signals the entire process group, causing duplicate INT delivery
# instead of testing the guard's forwarding alone (and sporadic status 255).
python3 - "$runner" "$tmp" <<'PY'
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

runner, directory = sys.argv[1:]
signal.signal(signal.SIGINT, signal.SIG_DFL)
child = """
from pathlib import Path
import signal
import sys
def interrupted(signum, frame):
    print("INT forwarded", flush=True)
    sys.exit(130)
signal.signal(signal.SIGINT, interrupted)
Path(sys.argv[1]).touch()
signal.pause()
"""
for uid in ("1000", "0"):
    for attempt in range(int(os.environ.get("ORBIT_TEST_GUARD_INT_REPETITIONS", "1"))):
        ready = Path(directory) / f"int-ready-{uid}-{attempt}"
        env = dict(os.environ, MOCK_UID=uid, MOCK_GID=uid)
        process = subprocess.Popen(
            [runner, sys.executable, "-c", child, str(ready)],
            env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            restore_signals=True, start_new_session=True,
        )
        try:
            deadline = time.monotonic() + 5
            while not ready.exists():
                if process.poll() is not None:
                    out, err = process.communicate()
                    raise AssertionError(f"guard exited before INT readiness: {process.returncode}, {out!r}, {err!r}")
                if time.monotonic() >= deadline:
                    raise AssertionError("timed out waiting for INT child readiness")
                time.sleep(.005)
            os.kill(process.pid, signal.SIGINT)
            out, err = process.communicate(timeout=5)
            assert process.returncode == 130, (uid, process.returncode, out, err)
            assert out == "INT forwarded\n", (uid, out, err)
        finally:
            # Also clean up any descendant if an assertion/timeout failed.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
PY
expect_status 72 env MOCK_SYSTEMD_FAILURE=72 "$runner" touch "$tmp/must-not-exist"
[[ ! -e "$tmp/must-not-exist" ]] || fail 'failed scope executed command'
expect_status 1 env MOCK_OS=Darwin "$runner" true
grep -q 'Linux/systemd' "$tmp/stderr" || fail 'missing actionable platform error'
mv "$tmp/orbit-builds.slice" "$tmp/slice-away"
expect_status 1 "$runner" true
grep -q 'install deploy/orbit-builds.slice' "$tmp/stderr" || fail 'missing actionable setup error'
mv "$tmp/slice-away" "$tmp/orbit-builds.slice"
mv "$tmp/build.lock" "$tmp/lock-away"
expect_status 1 "$runner" true
[[ ! -e "$tmp/build.lock" ]] || fail 'runner created its own lock'
mv "$tmp/lock-away" "$tmp/build.lock"

: >"$tmp/log"
expect_status 0 timeout 5 "$runner" "$tmp/worktree-b/orbit-build" bash -c '[[ $CARGO_BUILD_JOBS == 1 ]]'
[[ $(grep -c '^scope$' "$tmp/log") == 1 ]] || fail 'nested invocation created another scope'
: >"$tmp/log"
expect_status 0 env MOCK_UID=0 MOCK_GID=0 "$runner" true
! grep -q '^sudo ' "$tmp/log" || fail 'root caller unnecessarily used sudo'

# Two different worktrees/users must contend for the same lock even though sudo
# drops FD 9 in both children. Handshakes keep the test deterministic.
"$runner" bash -c 'touch "$1/first-started"; while [[ ! -e "$1/release-first" ]]; do sleep .01; done; touch "$1/first-ended"' _ "$tmp" &
first=$!
wait_for_file "$tmp/first-started"
"$tmp/worktree-b/orbit-build" touch "$tmp/canceled-waiter-ran" &
canceled=$!
sleep .05
kill -TERM "$canceled"
status=0
wait "$canceled" || status=$?
[[ "$status" == 143 && ! -e "$tmp/canceled-waiter-ran" ]] || fail 'canceling lock waiter ran a build'
env MOCK_UID=1001 MOCK_GID=1001 "$tmp/worktree-b/orbit-build" bash -c '[[ -e "$1/first-ended" ]]; touch "$1/second-started"' _ "$tmp" &
second=$!
sleep .1
[[ ! -e "$tmp/second-started" ]] || fail 'heavy builds overlapped'
touch "$tmp/release-first"
wait "$first"
wait "$second"
[[ -e "$tmp/second-started" ]] || fail 'second build never ran'

# TERM reaches the child, its custom exit status is preserved, and the parent
# does not unlock until that child has completed its shutdown.
cat >"$tmp/term-child" <<'CHILD'
#!/usr/bin/env bash
trap 'touch "$1/term-received"; while [[ ! -e "$1/release-term" ]]; do sleep .01; done; exit 42' TERM
touch "$1/term-started"
while :; do sleep .01; done
CHILD
chmod +x "$tmp/term-child"
"$runner" "$tmp/term-child" "$tmp" &
signaled=$!
wait_for_file "$tmp/term-started"
kill -TERM "$signaled"
wait_for_file "$tmp/term-received"
if flock -n "$tmp/build.lock" true; then fail 'lock released during signal cleanup'; fi
touch "$tmp/release-term"
status=0
wait "$signaled" || status=$?
[[ "$status" == 42 ]] || fail "signal child status was $status, not 42"
flock -n "$tmp/build.lock" true || fail 'lock not released after completion'

# Docker wrapper: replace only installed absolute executables in the test copy.
cat >"$tmp/docker-real" <<'MOCK'
#!/usr/bin/env bash
printf 'builder=%s\n' "${BUILDX_BUILDER:-unset}"
printf '<%s>\n' "$@"
exit "${MOCK_DOCKER_STATUS:-0}"
MOCK
cat >"$tmp/orbit-build-mock" <<'MOCK'
#!/usr/bin/env bash
printf 'guarded\n'
exec "$@"
MOCK
chmod +x "$tmp/docker-real" "$tmp/orbit-build-mock"
sed -e "s|/usr/bin/docker|$tmp/docker-real|g" \
    -e "s|/usr/local/bin/orbit-build|$tmp/orbit-build-mock|g" \
    "$repo/scripts/docker-build-guard.sh" >"$tmp/docker"
chmod +x "$tmp/docker"
expect_status 0 "$tmp/docker" ps --all
! grep -q '^guarded$' "$tmp/stdout" || fail 'docker ps was guarded'
grep -q '^<ps>$' "$tmp/stdout" || fail 'passthrough lost arguments'
expect_status 0 "$tmp/docker" build -t 'name with space' .
grep -q '^guarded$' "$tmp/stdout" || fail 'docker build was not guarded'
grep -q '^builder=orbit-limited$' "$tmp/stdout" || fail 'builder env missing'
grep -q '^<--load>$' "$tmp/stdout" || fail 'docker build did not load image'
grep -q '^<name with space>$' "$tmp/stdout" || fail 'argument quoting lost'
expect_status 0 "$tmp/docker" buildx build --push --builder orbit-limited .
! grep -q '^<--load>$' "$tmp/stdout" || fail 'buildx export options were changed'
grep -q '^<--push>$' "$tmp/stdout" || fail 'buildx push was lost'
expect_status 0 "$tmp/docker" build --builder=orbit-limited .
for exporter in --output=type=oci,dest=image.tar -oimage.tar --push --load=false; do
    expect_status 0 "$tmp/docker" build "$exporter" .
    ! grep -q '^<--load>$' "$tmp/stdout" || fail "extra --load added with $exporter"
done
expect_status 0 "$tmp/docker" build --output type=oci,dest=image.tar .
! grep -q '^<--load>$' "$tmp/stdout" || fail 'extra --load added with --output'
expect_status 0 "$tmp/docker" build -o image.tar .
! grep -q '^<--load>$' "$tmp/stdout" || fail 'extra --load added with -o'
expect_status 64 "$tmp/docker" build --builder=default .
expect_status 64 "$tmp/docker" buildx build --builder default .
expect_status 64 "$tmp/docker" buildx build --builder
expect_status 64 "$tmp/docker" build --builder default --builder orbit-limited .
expect_status 29 env MOCK_DOCKER_STATUS=29 "$tmp/docker" build .
printf 'PASS: build runner and Docker guard mock tests\n'
