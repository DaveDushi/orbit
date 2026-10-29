#!/usr/bin/env bash
# Installed on the shared build VM as /usr/local/bin/orbit-build.
set -euo pipefail

die() {
    printf 'orbit-build: %s\n' "$*" >&2
    exit 1
}

if (( $# == 0 )); then
    printf 'Usage: %s command [argument ...]\n' "$0" >&2
    exit 64
fi

export CARGO_BUILD_JOBS=1
# Only the command inside the scope receives this marker. In particular, a Cargo
# rustc-wrapper invocation must not try to reacquire its parent's global lock.
# Use Orbit's ignored ORBIT_DEV_* namespace so child app configuration accepts it.
if [[ ${ORBIT_DEV_BUILD_GUARD_ACTIVE:-} == 1 ]]; then
    exec "$@"
fi

readonly lock_path=/var/lib/orbit-builds/build.lock
readonly slice_path=/etc/systemd/system/orbit-builds.slice
[[ $(uname -s) == Linux ]] || die 'requires the Linux/systemd build VM; run the command directly on other developer machines.'
for dependency in flock systemd-run; do
    command -v "$dependency" >/dev/null || die "missing $dependency; ask the VM operator to complete deploy/README.md build-worker setup."
done
[[ -f "$slice_path" ]] || die "missing $slice_path; ask the VM operator to install deploy/orbit-builds.slice."
[[ -f "$lock_path" && -r "$lock_path" && -w "$lock_path" ]] ||
    die "requires the precreated, shared mode-0666 $lock_path (root-owned parent directory); see deploy/README.md."

uid=$(id -u)
gid=$(id -g)
scope=(systemd-run --scope --quiet --slice=orbit-builds.slice
    --expand-environment=no --working-directory="$PWD" --)
if (( uid != 0 )); then
    command -v sudo >/dev/null || die 'requires passwordless sudo for systemd-run; see deploy/README.md.'
    scope=(sudo -n -E "${scope[@]}" sudo -u "#$uid" -g "#$gid" --preserve-env --)
fi
# sudo's secure_path can replace PATH even with -E. Restore the caller's PATH
# after dropping privileges; all other exported variables are inherited via -E.
scope+=(env "PATH=$PATH" ORBIT_DEV_BUILD_GUARD_ACTIVE=1 CARGO_BUILD_JOBS=1 "$@")

child_pid=
wait_interrupted=0
forward_signal() {
    local signal=$1 status=$2
    if [[ -n "$child_pid" ]]; then
        wait_interrupted=1
        kill -s "$signal" "$child_pid" 2>/dev/null || true
    else
        exit "$status"
    fi
}
trap 'forward_signal HUP 129' HUP
trap 'forward_signal INT 130' INT
trap 'forward_signal QUIT 131' QUIT
trap 'forward_signal TERM 143' TERM

run_child() {
    local status
    # Background jobs otherwise inherit ignored SIGINT/SIGQUIT from Bash. Reset
    # those dispositions, keep stdin, and forward signals while wait is active.
    (trap - HUP INT QUIT TERM; exec "$@") <&0 &
    child_pid=$!
    while :; do
        wait_interrupted=0
        if wait "$child_pid"; then status=0; else status=$?; fi
        (( wait_interrupted )) || break
    done
    child_pid=
    return "$status"
}

# Hold the lock in this parent, not just the child: sudo closes extra FDs.
# Never create, replace, truncate, or unlink the shared lock file.
exec 9<>"$lock_path"
run_child flock --exclusive 9
run_child "${scope[@]}"
