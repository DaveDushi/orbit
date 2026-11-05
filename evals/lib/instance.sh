# Starts and stops the isolated test instance. Sourced by run.sh, which sets EVALS_WORK, EVALS_PORT, ORBIT_BIN and
# APP_KEY. The instance has its own port and its own data directory under $EVALS_WORK; it never touches other data.

data_dir() { echo "$EVALS_WORK/data"; }

# orbit_command <mode> [ORBIT_X=y ...]: sets ORBIT_COMMAND to the orbit binary on the test data directory, with a clean
# environment (Orbit refuses unknown ORBIT_* variables; ORBIT_DEV_* is ignored and allowed) plus the given settings.
orbit_command() {
    local mode=$1
    shift
    local unset_args=()
    for name in $(env | sed -n 's/^\(ORBIT_[A-Z0-9_]*\)=.*/\1/p' | grep -v '^ORBIT_DEV_'); do
        unset_args+=(-u "$name")
    done
    ORBIT_COMMAND=(env "${unset_args[@]}" ORBIT_APP_KEY="$APP_KEY" ORBIT_ENV="$mode"
        ORBIT_DATABASE_PATH="$(data_dir)/orbit.sqlite" ORBIT_ATTACHMENTS_PATH="$(data_dir)/attachments"
        ORBIT_BACKUPS_PATH="$(data_dir)/backups" "$@" "$ORBIT_BIN")
}

# orbit_cli <mode> <orbit arguments...>: runs one CLI command on the test data directory.
orbit_cli() {
    local mode=$1
    shift
    orbit_command "$mode"
    "${ORBIT_COMMAND[@]}" "$@"
}

# start_instance <development|production> [ORBIT_X=y ...]: starts the server, waits until it is ready and sets
# STARTUP_MS to the milliseconds from spawn to the first healthy response. Call it directly, not in $(...).
start_instance() {
    local mode=$1
    shift
    local origin cookie
    if [[ $mode == production ]]; then
        # Production refuses an http origin; the client acts as the trusted TLS proxy on 127.0.0.1.
        origin=https://orbit.test
        cookie=__Host-orbit_session
        set -- ORBIT_TRUSTED_PROXIES=127.0.0.1/32 "$@"
    else
        origin="http://127.0.0.1:$EVALS_PORT"
        cookie=orbit_session_dev
    fi
    mkdir -p "$(data_dir)"
    if curl -s -o /dev/null "http://127.0.0.1:$EVALS_PORT/"; then
        echo "port $EVALS_PORT is already in use" >&2
        return 1
    fi
    local start
    start=$(date +%s%3N)
    orbit_command "$mode" ORBIT_BIND=127.0.0.1 ORBIT_PORT="$EVALS_PORT" ORBIT_PUBLIC_ORIGIN="$origin" "$@"
    # A plain background command: $! is env's pid, and env execs orbit, so it is the server's pid.
    "${ORBIT_COMMAND[@]}" serve >>"$EVALS_WORK/server.log" 2>&1 &
    SERVER_PID=$!
    local ready=0
    for _ in $(seq 1 600); do
        if curl -fs -o /dev/null -H 'x-forwarded-proto: https' "http://127.0.0.1:$EVALS_PORT/health/ready"; then
            ready=1
            break
        fi
        kill -0 "$SERVER_PID" 2>/dev/null || break
        sleep 0.05
    done
    if [[ $ready != 1 ]]; then
        echo "test instance did not become ready; last log lines:" >&2
        tail -20 "$EVALS_WORK/server.log" >&2
        return 1
    fi
    local elapsed=$(($(date +%s%3N) - start))
    cat >"$EVALS_WORK/instance.json" <<JSON
{"base": "http://127.0.0.1:$EVALS_PORT", "origin": "$origin", "mode": "$mode", "cookie": "$cookie", "pid": $SERVER_PID, "dataDir": "$(data_dir)"}
JSON
    STARTUP_MS=$elapsed
}

stop_instance() {
    [[ -n ${SERVER_PID:-} ]] || return 0
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
    SERVER_PID=
}

# Resident memory of the server in MB: "<current> <peak>".
server_rss() {
    awk '/^VmRSS/ { rss = $2 } /^VmHWM/ { hwm = $2 } END { printf "%d %d\n", rss / 1024, hwm / 1024 }' "/proc/$SERVER_PID/status"
}
