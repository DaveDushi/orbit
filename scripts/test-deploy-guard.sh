#!/usr/bin/env bash
# No sudo, Docker daemon, or systemd access: all privileged entry points are mocked.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'kill "${running_pid:-}" 2>/dev/null || true; rm -rf "$tmp"' EXIT
mkdir -p "$tmp/mock" "$tmp/bins" "$tmp/guards" "$tmp/repo"
export GUARD_TEST_TMP="$tmp" MOCK_DF_KB=99999999 MOCK_MAIN_PID=0
touch "$tmp/deploy.lock" "$tmp/log" "$tmp/images" "$tmp/labels"

fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
pass() { printf 'ok: %s\n' "$*"; }
expect_status() {
    local expected=$1 actual=0
    shift
    "$@" >"$tmp/stdout" 2>"$tmp/stderr" || actual=$?
    [[ "$actual" == "$expected" ]] || fail "expected status $expected, got $actual for $*: $(cat "$tmp/stderr")"
}
stderr_has() { grep -q -- "$1" "$tmp/stderr" || fail "stderr has no '$1': $(cat "$tmp/stderr")"; }
wait_free() {
    local attempt
    for (( attempt=0; attempt<$1; attempt++ )); do
        flock -n "$tmp/deploy.lock" true && return 0
        sleep 0.1
    done
    fail "lock not released after $1 tenths of a second"
}
field() { sed -n "s/^$1=//p" "$tmp/deploy.lock"; }

cat >"$tmp/mock/df" <<'MOCK'
#!/usr/bin/env bash
[[ $1 == --output=avail && $2 == -k ]]
[[ ${MOCK_DF_FAIL:-0} == 0 ]] || exit 1
printf '  Avail\n'
if [[ -n ${MOCK_DF_LOW_PATH:-} && $3 == "$MOCK_DF_LOW_PATH" ]]; then echo 1024; else echo "$MOCK_DF_KB"; fi
MOCK
cat >"$tmp/mock/systemctl" <<'MOCK'
#!/usr/bin/env bash
[[ "$*" == 'show -p MainPID --value orbit.service' ]]
echo "$MOCK_MAIN_PID"
MOCK
cat >"$tmp/mock/docker" <<'MOCK'
#!/usr/bin/env bash
case "$1" in
    images) cat "$GUARD_TEST_TMP/images" ;;
    image)
        # The label of the tag, from a "tag label" file, as docker image inspect prints it.
        [[ $2 == inspect && $3 == --format ]] || exit 90
        [[ $4 == *org.orbit.clean-revision* ]] || exit 90
        line=$(grep "^$5 " "$GUARD_TEST_TMP/labels") || exit 1
        printf '%s\n' "${line#* }"
        ;;
    buildx)
        printf 'docker %s\n' "$*" >>"$GUARD_TEST_TMP/log"
        context=${!#}
        [[ -f "$context/source.txt" && ! -e "$context/dirty.txt" ]] || exit 91
        cat "$context/source.txt" >>"$GUARD_TEST_TMP/log"
        [[ ${MOCK_BUILD_FAIL:-0} == 0 ]] || exit 7
        ;;
    rmi) printf 'rmi %s\n' "$2" >>"$GUARD_TEST_TMP/log" ;;
    *) exit 90 ;;
esac
MOCK
cat >"$tmp/mock/orbit-build" <<'MOCK'
#!/usr/bin/env bash
printf 'orbit-build\n' >>"$GUARD_TEST_TMP/log"
exec "$@"
MOCK
chmod +x "$tmp/mock/"*
export PATH="$tmp/mock:$PATH"

# make_guard <name> <lease> <renew> <poll>: a test copy with local paths and short times.
make_guard() {
    sed -e "s|^readonly lock_path=.*|readonly lock_path=$tmp/deploy.lock|" \
        -e "s|^readonly bin_dir=.*|readonly bin_dir=$tmp/bins|" \
        -e "s|^readonly repo_path=.*|readonly repo_path=$tmp/repo|" \
        -e "s|^readonly deployment_file=.*|readonly deployment_file=$tmp/DEPLOYMENT.local.md|" \
        -e "s|^readonly docker_root=.*|readonly docker_root=$tmp/docker-root|" \
        -e "s|^readonly lease_seconds=.*|readonly lease_seconds=$2|" \
        -e "s|^readonly renew_seconds=.*|readonly renew_seconds=$3|" \
        -e "s|^readonly poll_seconds=.*|readonly poll_seconds=$4|" \
        "$repo/scripts/deploy-guard.sh" >"$tmp/guards/$1"
    chmod +x "$tmp/guards/$1"
}
make_guard normal 7200 60 1
make_guard short 3 1 1
make_guard slow_poll 7200 60 20
guard="$tmp/guards/normal"

# --- A second deploy is refused ---
first=$("$guard" begin 'first deploy')
[[ -n "$first" ]] || fail 'begin printed no ID'
expect_status 75 "$guard" begin 'second deploy'
stderr_has 'another deploy is running'
stderr_has "$first"
expect_status 75 "$guard" status
pass 'a second begin is refused and names the first deploy, also from a new shell'

expect_status 75 "$guard" run wrong-id -- touch "$tmp/wrong-ran"
[[ ! -e "$tmp/wrong-ran" ]] || fail 'a step ran with the wrong ID'
expect_status 75 "$guard" end wrong-id
flock -n "$tmp/deploy.lock" true && fail 'end with the wrong ID released the lock'
pass 'the wrong ID cannot run a step or release the lock'

expect_status 0 "$guard" run "$first" -- touch "$tmp/right-ran"
[[ -e "$tmp/right-ran" ]] || fail 'the step did not run'
expect_status 42 "$guard" run "$first" -- bash -c 'exit 42'
flock -n "$tmp/deploy.lock" true && fail 'a failed step released the lock'
pass 'a step runs under the lock, keeps its status, and a failed step keeps the lock'

expect_status 0 "$guard" end "$first"
flock -n "$tmp/deploy.lock" true || fail 'end did not release the lock'
second=$("$guard" begin 'next deploy' 2>"$tmp/stderr")
[[ ! -s "$tmp/stderr" ]] || fail "clean end gave a warning: $(cat "$tmp/stderr")"
expect_status 0 "$guard" end "$second"
expect_status 0 "$guard" status
grep -qx free "$tmp/stdout" || fail "status after a clean end: $(cat "$tmp/stdout")"
pass 'end releases the lock, with no warning for the next deploy'

# --- The lock releases after a crash ---
# A 20 second poll proves that the holder's sleep child does not keep the lock.
crash_guard="$tmp/guards/slow_poll"
crashed=$("$crash_guard" begin 'crashed deploy')
kill -9 "$(field holder_pid)"
wait_free 10
pass 'SIGKILL of the holder (same as a reboot) releases the lock at once'

expect_status 75 "$guard" run "$crashed" -- touch "$tmp/after-crash-ran"
[[ ! -e "$tmp/after-crash-ran" ]] || fail 'a step ran after the lock was lost'
stderr_has 'not held'
pass 'a step of the crashed deploy is refused'

"$guard" status >"$tmp/stdout"
grep -q 'did not end' "$tmp/stdout" || fail "status does not report the crash: $(cat "$tmp/stdout")"
third=$("$guard" begin 'after crash' 2>"$tmp/stderr")
stderr_has 'previous deploy did not end'
stderr_has 'crashed deploy'
expect_status 0 "$guard" end "$third"
pass 'the next deploy can start and gets a warning about the crashed one'

# A session that crashes leaves the holder alive: only the lease releases it.
short="$tmp/guards/short"
lease=$("$short" begin 'long step')
expect_status 0 "$short" run "$lease" -- sleep 5
flock -n "$tmp/deploy.lock" true && fail 'the lease expired during a running step'
pass 'a step longer than the lease keeps the lock'
wait_free 60
expect_status 75 "$short" run "$lease" -- true
pass 'an idle lease expires after a session crash'
: >"$tmp/deploy.lock"

# --- Disk check ---
disk_id=$("$guard" begin 'disk test')
expect_status 0 "$guard" disk
MOCK_DF_KB=3145728 expect_status 0 "$guard" build "$disk_id" -- touch "$tmp/at-limit-ran"
[[ -e "$tmp/at-limit-ran" ]] || fail 'the build did not run at the limit'
MOCK_DF_KB=3145727 expect_status 73 "$guard" build "$disk_id" -- touch "$tmp/low-ran"
stderr_has 'low disk'
[[ ! -e "$tmp/low-ran" ]] || fail 'the build ran with low disk'
MOCK_DF_LOW_PATH="$tmp/docker-root" expect_status 73 "$guard" build "$disk_id" -- touch "$tmp/low-ran"
MOCK_DF_LOW_PATH="$PWD" expect_status 73 "$guard" build "$disk_id" -- touch "$tmp/low-ran"
MOCK_DF_FAIL=1 expect_status 1 "$guard" build "$disk_id" -- touch "$tmp/low-ran"
[[ ! -e "$tmp/low-ran" ]] || fail 'the build ran with low or unknown disk'
flock -n "$tmp/deploy.lock" true && fail 'low disk released the lock'
MOCK_DF_KB=1 expect_status 73 "$guard" disk
expect_status 0 "$guard" end "$disk_id"
expect_status 75 "$guard" build "$disk_id" -- touch "$tmp/low-ran"
[[ ! -e "$tmp/low-ran" ]] || fail 'the build ran without the lock'
pass 'the disk check stops a build below 3 GiB, on either path or if df fails, and a build needs the lock'

# --- Image reuse ---
git -C "$tmp/repo" init -q -b main
printf 'first\n' >"$tmp/repo/source.txt"
git -C "$tmp/repo" add source.txt
git -C "$tmp/repo" -c user.name=t -c user.email=t@t commit -qm first
first_sha=$(git -C "$tmp/repo" rev-parse HEAD)
printf 'second\n' >"$tmp/repo/source.txt"
git -C "$tmp/repo" -c user.name=t -c user.email=t@t commit -qam second
second_sha=$(git -C "$tmp/repo" rev-parse HEAD)
# Uncommitted work must not reach the build.
printf 'dirty\n' >"$tmp/repo/source.txt"
touch "$tmp/repo/dirty.txt"
second_tag=orbit:${second_sha:0:7}
image_id=$("$guard" begin 'image test')
built() { grep -q '^docker buildx' "$tmp/log"; }

: >"$tmp/log"
expect_status 0 "$guard" image "$image_id" main
built || fail 'no image: it did not build'
grep -q '^orbit-build$' "$tmp/log" || fail 'the build did not use orbit-build'
grep -q -- '--builder=orbit-limited' "$tmp/log" || fail 'the build did not use the limited builder'
grep -q -- "ORBIT_BUILD_REVISION=$second_sha" "$tmp/log" || fail 'wrong revision argument'
grep -q -- "--label org.orbit.clean-revision=$second_sha" "$tmp/log" || fail 'no clean label'
grep -q -- "-t $second_tag" "$tmp/log" || fail 'wrong tag'
grep -qx second "$tmp/log" || fail 'the build did not use the committed source'
pass 'an image that does not exist is built from a clean archive of the commit'

printf '%s %s\n' "$second_tag" "$second_sha" >"$tmp/labels"
: >"$tmp/log"
MOCK_DF_KB=1 expect_status 0 "$guard" image "$image_id" main
built && fail 'a clean image of the exact commit was built again'
stderr_has 'still run the smoke test'
pass 'a clean image of the exact commit is reused, with no disk need'

# A real merge commit has another sha, so another tag.
: >"$tmp/log"
expect_status 0 "$guard" image "$image_id" "$first_sha"
built || fail 'another commit did not build'
grep -q -- "-t orbit:${first_sha:0:7}" "$tmp/log" || fail 'another commit used the wrong tag'
grep -qx first "$tmp/log" || fail 'another commit used the wrong source'
pass 'another commit still builds'

for label in '<no value>' "$first_sha" ''; do
    printf '%s %s\n' "$second_tag" "$label" >"$tmp/labels"
    : >"$tmp/log"
    expect_status 0 "$guard" image "$image_id" "$second_sha"
    built || fail "a tag with the label '$label' was reused"
done
pass 'a tag with no label, an empty label or the label of another commit is not reused'

: >"$tmp/labels"
MOCK_DF_KB=1 expect_status 73 "$guard" image "$image_id" main
MOCK_BUILD_FAIL=1 expect_status 7 "$guard" image "$image_id" main
expect_status 1 "$guard" image "$image_id" no-such-commit
stderr_has 'unknown commit'
expect_status 0 "$guard" end "$image_id"
: >"$tmp/log"
printf '%s %s\n' "$second_tag" "$second_sha" >"$tmp/labels"
expect_status 75 "$guard" image "$image_id" main
built && fail 'a build ran without the lock'
pass 'image stops on low disk, keeps a build failure status, and needs the lock'

# --- Cleanup ---
# Newest to oldest. aaaaaaa is installed, bbbbbbb is the old process that has not
# restarted, and the rollback copy ccccccc is the oldest, so age alone would remove it.
shas=(aaaaaaa bbbbbbb 1111111 2222222 3333333 4444444 5555555 ccccccc)
hour=0
for sha in "${shas[@]}"; do
    printf 'binary %s\n' "$sha" >"$tmp/bins/orbit-$sha"
    touch -d "2026-10-20 00:00:00 UTC - $hour hours" "$tmp/bins/orbit-$sha"
    printf '2026-10-%02d 00:00:00 +0000 UTC|%s|100MB\n' "$(( 28 - hour ))" "$sha" >>"$tmp/images"
    hour=$(( hour + 1 ))
done
cp "$tmp/bins/orbit-aaaaaaa" "$tmp/bins/orbit"
# bbbbbbb is the running process: a real process needs a real executable.
rm "$tmp/bins/orbit-bbbbbbb"
cp "$(command -v sleep)" "$tmp/bins/orbit-bbbbbbb"
touch -d '2026-10-19 23:00:00 UTC' "$tmp/bins/orbit-bbbbbbb"
"$tmp/bins/orbit-bbbbbbb" 300 &
running_pid=$!
export MOCK_MAIN_PID=$running_pid
# Not a release: these names must stay untouched.
printf 'x\n' >"$tmp/bins/orbit-build"
printf 'x\n' >"$tmp/bins/orbit-abc"
printf 'x\n' >"$tmp/bins/orbit-oidc-gateway"
touch -d '2020-01-01' "$tmp/bins/orbit-build" "$tmp/bins/orbit-abc" "$tmp/bins/orbit-oidc-gateway"
ln -s "$tmp/bins/orbit-aaaaaaa" "$tmp/bins/orbit-ddddddd"
{
    printf '2020-01-01 00:00:00 +0000 UTC|latest|1MB\n'
    printf '2020-01-01 00:00:00 +0000 UTC|<none>|1MB\n'
} >>"$tmp/images"
cat >"$tmp/DEPLOYMENT.local.md" <<'DOC'
sudo install -m 755 /usr/local/bin/orbit-ccccccc /usr/local/bin/orbit
later section
sudo install -m 755 /usr/local/bin/orbit-1111111 /usr/local/bin/orbit
DOC

list_of() { sed -nE "s/^$1 +$2 +([^ ]+).*/\1/p" "$tmp/stdout" | sort | tr '\n' ' '; }
expected_keep='1111111 2222222 3333333 aaaaaaa bbbbbbb ccccccc '
expected_remove='4444444 5555555 '

before=$(ls "$tmp/bins" | wc -l)
: >"$tmp/log"
expect_status 0 "$guard" cleanup
[[ "$(list_of keep binary)" == "$expected_keep" ]] || fail "binaries kept: $(list_of keep binary)"
[[ "$(list_of remove binary)" == "$expected_remove" ]] || fail "binaries removed: $(list_of remove binary)"
[[ "$(list_of keep image)" == "$expected_keep" ]] || fail "images kept: $(list_of keep image)"
[[ "$(list_of remove image)" == "$expected_remove" ]] || fail "images removed: $(list_of remove image)"
grep -q 'bbbbbbb (running)' "$tmp/stdout" || fail 'running process is not marked'
grep -q 'aaaaaaa (installed)' "$tmp/stdout" || fail 'installed binary is not marked'
grep -q 'ccccccc (rollback)' "$tmp/stdout" || fail 'first rollback command is not used'
pass 'cleanup keeps installed, running, rollback and 3 old copies of binaries and images'

[[ "$(ls "$tmp/bins" | wc -l)" == "$before" && ! -s "$tmp/log" ]] || fail 'dry run removed something'
grep -q '^dry run: nothing removed' "$tmp/stdout" || fail 'no dry-run line'
pass 'cleanup removes nothing by default'

busy=$("$guard" begin 'busy')
expect_status 75 "$guard" cleanup --apply
stderr_has 'cleanup must wait'
[[ -e "$tmp/bins/orbit-5555555" && ! -s "$tmp/log" ]] || fail 'cleanup --apply ran during a deploy'
expect_status 0 "$guard" end "$busy"
pass 'cleanup --apply is refused during a deploy'

check_refuses() {
    expect_status 1 "$guard" cleanup --apply
    stderr_has "$1"
    [[ -e "$tmp/bins/orbit-5555555" && ! -s "$tmp/log" ]] || fail "cleanup removed files after: $1"
}
cp "$tmp/DEPLOYMENT.local.md" "$tmp/doc.saved"
: >"$tmp/DEPLOYMENT.local.md"
check_refuses 'no rollback command'
rm "$tmp/DEPLOYMENT.local.md"
check_refuses 'rollback copy is unknown'
printf 'sudo install -m 755 /usr/local/bin/orbit-eeeeeee /usr/local/bin/orbit\n' >"$tmp/DEPLOYMENT.local.md"
check_refuses 'does not exist'
printf 'sudo install -m 755 /usr/local/bin/orbit-aaaaaaa /usr/local/bin/orbit\n' >"$tmp/DEPLOYMENT.local.md"
check_refuses 'is the running version'
mv "$tmp/doc.saved" "$tmp/DEPLOYMENT.local.md"
cp "$tmp/bins/orbit" "$tmp/orbit.saved"
printf 'unknown\n' >"$tmp/bins/orbit"
check_refuses 'running version is unknown'
mv "$tmp/orbit.saved" "$tmp/bins/orbit"
real_pid=$MOCK_MAIN_PID
MOCK_MAIN_PID=$$
check_refuses 'matches the running process'
MOCK_MAIN_PID=$real_pid
pass 'cleanup removes nothing if it cannot identify the running or rollback version'

expect_status 0 "$guard" cleanup --apply
for sha in 4444444 5555555; do
    [[ ! -e "$tmp/bins/orbit-$sha" ]] || fail "orbit-$sha was not removed"
    grep -qx "rmi orbit:$sha" "$tmp/log" || fail "image $sha was not removed"
done
for name in orbit orbit-aaaaaaa orbit-bbbbbbb orbit-ccccccc orbit-1111111 orbit-2222222 orbit-3333333 \
    orbit-build orbit-abc orbit-oidc-gateway orbit-ddddddd; do
    [[ -e "$tmp/bins/$name" ]] || fail "$name was removed"
done
[[ "$(wc -l <"$tmp/log")" == 2 ]] || fail "unexpected docker calls: $(cat "$tmp/log")"
flock -n "$tmp/deploy.lock" true || fail 'cleanup --apply kept the lock'
pass 'cleanup --apply removes only the listed files and images'

printf 'All deploy guard tests passed.\n'
