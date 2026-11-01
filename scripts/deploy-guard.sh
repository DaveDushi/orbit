#!/usr/bin/env bash
# Installed on the shared build VM as /usr/local/bin/orbit-deploy.
#
# Deploys are manual steps in many separate shells, so no single process can
# hold a lock for the whole deploy. `begin` starts a small detached holder that
# keeps the flock. The kernel releases it if the holder dies or the VM reboots.
# The lock file also stores the holder's record; the flock alone decides whether
# the lock is held, so a stale record cannot block a deploy.
set -euo pipefail

readonly lock_path=/var/lib/orbit-deploys/deploy.lock
readonly bin_dir=/usr/local/bin
readonly deployment_file=/home/exedev/orbit/DEPLOYMENT.local.md
readonly docker_root=/var/lib/docker
# The holder exits after this time with no running step, so that a deploy
# session that crashes cannot keep the lock until the next reboot.
readonly lease_seconds=7200
readonly renew_seconds=60
readonly poll_seconds=2
# A cold build needs about 2.4 GB. The rest is margin for the running services.
readonly min_free_kb=3145728
readonly keep_old=3
readonly status_busy=75 status_low_disk=73

die() {
    printf 'orbit-deploy: %s\n' "$*" >&2
    exit 1
}

refuse() {
    printf 'orbit-deploy: %s\n' "$*" >&2
    exit "$status_busy"
}

usage() {
    cat >&2 <<'USAGE'
Usage: orbit-deploy begin <what is deployed>     take the lock, print the deploy ID
       orbit-deploy status                       show who holds the lock
       orbit-deploy run <id> -- command ...      run one deploy step under the lock
       orbit-deploy build <id> -- command ...    check the disk, then run the build
       orbit-deploy disk                         check free disk only
       orbit-deploy end <id>                     release the lock
       orbit-deploy cleanup [--apply]            show (or do) the old binary and image cleanup
USAGE
    exit 64
}

require_lock_file() {
    [[ -f "$lock_path" && -r "$lock_path" && -w "$lock_path" ]] ||
        die "requires the precreated, shared mode-0666 $lock_path; see deploy/README.md."
}

# The file is world-writable: read fields as text, never source it.
field() {
    sed -n "/^$1=/{s///p;q}" "$lock_path"
}

lock_free() {
    flock -n "$lock_path" true
}

describe_holder() {
    printf 'id=%s, what=%s, user=%s, started=%s' \
        "$(field id)" "$(field what)" "$(field user)" "$(field started)"
}

require_held() {
    local id=$1
    require_lock_file
    lock_free && refuse "the deploy lock is not held (crash, reboot or expired lease). Check production, then begin again."
    [[ "$(field id)" == "$id" ]] || refuse "the deploy lock belongs to another deploy: $(describe_holder)"
}

cmd_begin() {
    (( $# >= 1 )) || usage
    local what="$*" id
    what=${what//[$'\n\r']/ }
    require_lock_file
    exec 9<>"$lock_path"
    flock -n 9 || refuse "another deploy is running: $(describe_holder)"
    if [[ -n "$(field id)" ]]; then
        printf 'orbit-deploy: WARNING: the previous deploy did not end: %s\n' "$(describe_holder)" >&2
        printf 'orbit-deploy: production can be half switched. Check it before you continue.\n' >&2
    fi
    id="$(date -u +%Y%m%dT%H%M%SZ)-$$"
    printf 'id=%s\nwhat=%s\nuser=%s\nstarted=%s\n' "$id" "$what" "$(id -un)" "$(date -u +%FT%TZ)" >"$lock_path"
    # The holder inherits FD 9, so the flock stays held when this process exits.
    setsid "$(readlink -f "$0")" _hold "$id" </dev/null >/dev/null 2>&1 &
    printf 'holder_pid=%s\n' "$!" >>"$lock_path"
    printf '%s\n' "$id"
}

cmd_hold() {
    local id=$1
    while [[ "$(field id)" == "$id" ]] &&
        (( $(date +%s) - $(stat -c %Y "$lock_path") < lease_seconds )); do
        # sleep must not inherit FD 9: it would keep the lock after a crash.
        sleep "$poll_seconds" 9>&-
    done
}

cmd_status() {
    require_lock_file
    if ! lock_free; then
        printf 'held: %s, holder_pid=%s\n' "$(describe_holder)" "$(field holder_pid)"
        exit "$status_busy"
    fi
    if [[ -n "$(field id)" ]]; then
        printf 'free, but the previous deploy did not end: %s\n' "$(describe_holder)"
    else
        printf 'free\n'
    fi
}

check_disk() {
    local path avail
    for path in "$docker_root" "$PWD"; do
        avail=$(df --output=avail -k "$path" | tail -n 1) || die "cannot read free disk for $path."
        avail=${avail//[[:space:]]/}
        [[ "$avail" =~ ^[0-9]+$ ]] || die "cannot read free disk for $path."
        if (( avail < min_free_kb )); then
            printf 'orbit-deploy: low disk: %s has %s KiB free, a build needs %s KiB. Free disk first (orbit-deploy cleanup).\n' \
                "$path" "$avail" "$min_free_kb" >&2
            exit "$status_low_disk"
        fi
    done
}

cmd_run() {
    local id=$1 status=0 renewer
    require_held "$id"
    touch "$lock_path"
    # Keep the lease fresh while the step runs, for any step length.
    (
        while kill -0 "$$" 2>/dev/null && [[ "$(field id)" == "$id" ]]; do
            sleep "$renew_seconds"
            touch "$lock_path"
        done
    ) </dev/null >/dev/null 2>&1 &
    renewer=$!
    trap 'kill "$renewer" 2>/dev/null || true' EXIT
    shift 2
    "$@" || status=$?
    exit "$status"
}

cmd_end() {
    local id=$1 attempt
    require_held "$id"
    : >"$lock_path"
    for (( attempt=0; attempt<(poll_seconds + 5) * 10; attempt++ )); do
        lock_free && return 0
        sleep 0.1
    done
    die "the holder did not exit. Check orbit-deploy status."
}

# Prints the shas of every orbit-<sha> binary that has the same content as $1.
shas_matching() {
    local name
    for name in "${binaries[@]}"; do
        if cmp -s "$1" "$bin_dir/orbit-$name"; then
            printf '%s\n' "$name"
        fi
    done
}

find_protected() {
    local sha pid found rollback
    [[ -e "$bin_dir/orbit" ]] || die "cannot find $bin_dir/orbit."
    found=$(shas_matching "$bin_dir/orbit")
    [[ -n "$found" ]] || die "no orbit-<sha> copy matches $bin_dir/orbit, so the running version is unknown."
    for sha in $found; do reason[$sha]=installed; done

    # After a swap with no restart, the running process differs from the file.
    pid=$(systemctl show -p MainPID --value orbit.service) || die 'cannot ask systemd for the orbit process.'
    if [[ "$pid" != 0 && -n "$pid" ]]; then
        found=$(shas_matching "/proc/$pid/exe")
        [[ -n "$found" ]] || die "no orbit-<sha> copy matches the running process $pid (run this as root)."
        for sha in $found; do reason[$sha]=running; done
    fi

    [[ -r "$deployment_file" ]] || die "cannot read $deployment_file, so the rollback copy is unknown."
    # The newest upgrade is first in the file, so the first match is the current rollback.
    rollback=$(sed -nE '/^(sudo )?install -m 755 \/usr\/local\/bin\/orbit-([0-9a-f]{7,40}) \/usr\/local\/bin\/orbit$/{s//\2/p;q}' "$deployment_file")
    [[ -n "$rollback" ]] || die "no rollback command in $deployment_file."
    [[ -f "$bin_dir/orbit-$rollback" ]] || die "the rollback copy $bin_dir/orbit-$rollback does not exist."
    [[ -z "${reason[$rollback]:-}" ]] || die "the rollback copy $rollback is the running version; $deployment_file is out of date."
    reason[$rollback]=rollback
}

# Input: "sha size" lines, newest first. Fills the remove list given by name.
plan() {
    local kind=$1 sha size kept=0
    local -n removals=$2
    while read -r sha size; do
        if [[ -n "${reason[$sha]:-}" ]]; then
            printf 'keep    %-6s %s (%s)\n' "$kind" "$sha" "${reason[$sha]}"
        elif (( kept < keep_old )); then
            kept=$(( kept + 1 ))
            printf 'keep    %-6s %s (old copy %s of %s)\n' "$kind" "$sha" "$kept" "$keep_old"
        else
            printf 'remove  %-6s %s %s\n' "$kind" "$sha" "$size"
            removals+=("$sha")
        fi
    done
}

cmd_cleanup() {
    local apply=0 listing images sha failed=0
    case "${1:-}" in
        '') ;;
        --apply) apply=1 ;;
        *) usage ;;
    esac
    if (( apply )); then
        # Keep the lock until the end of this process: no deploy can start in the middle.
        require_lock_file
        exec 9<>"$lock_path"
        flock -n 9 || refuse "a deploy is running, cleanup must wait: $(describe_holder)"
    elif [[ -r "$lock_path" ]] && ! lock_free; then
        printf 'note: a deploy is running, so this list can change: %s\n' "$(describe_holder)"
    fi

    listing=$(find "$bin_dir" -maxdepth 1 -type f -regextype posix-extended -regex '.*/orbit-[0-9a-f]{7,40}' \
        -printf '%T@ %f %s\n' | sort -rn)
    local binaries=()
    while read -r _ name _; do
        [[ -n "$name" ]] && binaries+=("${name#orbit-}")
    done <<<"$listing"
    declare -A reason=()
    find_protected

    images=$(docker images --filter 'reference=orbit:*' --format '{{.CreatedAt}}|{{.Tag}}|{{.Size}}') ||
        die 'cannot list docker images.'

    local remove_binaries=() remove_images=()
    plan binary remove_binaries < <(awk '{ sub(/^orbit-/, "", $2); printf "%s %.0fMB\n", $2, $3 / 1000000 }' <<<"$listing")
    plan image remove_images < <(sort -r <<<"$images" | awk -F'|' '$2 ~ /^[0-9a-f]+$/ && length($2) >= 7 && length($2) <= 40 { print $2, $3 }')

    if (( ! apply )); then
        printf 'dry run: nothing removed. %s binaries and %s images would be removed. Use --apply to remove them.\n' \
            "${#remove_binaries[@]}" "${#remove_images[@]}"
        return 0
    fi
    for sha in "${remove_binaries[@]}"; do
        rm -- "$bin_dir/orbit-$sha" || failed=1
    done
    for sha in "${remove_images[@]}"; do
        # No --force: Docker refuses an image that a container uses.
        docker rmi "orbit:$sha" >/dev/null || failed=1
    done
    printf 'removed: %s binaries and %s images.\n' "${#remove_binaries[@]}" "${#remove_images[@]}"
    (( ! failed )) || die 'some removals failed; see the errors above.'
}

for dependency in flock setsid; do
    command -v "$dependency" >/dev/null || die "missing $dependency."
done

(( $# >= 1 )) || usage
command=$1
shift
case "$command" in
    begin) cmd_begin "$@" ;;
    _hold) cmd_hold "$@" ;;
    status) (( $# == 0 )) || usage; cmd_status ;;
    run|build)
        (( $# >= 3 )) && [[ $2 == -- ]] || usage
        if [[ "$command" == build ]]; then
            require_held "$1"
            check_disk
        fi
        cmd_run "$@"
        ;;
    disk) (( $# == 0 )) || usage; check_disk; printf 'disk ok\n' ;;
    end) (( $# == 1 )) || usage; cmd_end "$1" ;;
    cleanup) cmd_cleanup "$@" ;;
    *) usage ;;
esac
