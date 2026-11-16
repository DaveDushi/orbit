#!/usr/bin/python3 -I
"""Spike broker for per-thread gVisor sandboxes (prototype, test copy only).

  sbx.py call <key> <script>   run one call in the thread sandbox <key> (start or evict as needed)
  sbx.py gc                    delete sandboxes idle > IDLE_S or older than MAX_LIFE_S
  sbx.py stop-all              off switch: kill every live sandbox now
  sbx.py list

<key> stands for agent+thread+driver. /work/<key> stays after a delete.

Prototype from the phase 1 spike (docs/agent-sandbox-design.md, section 13). It runs each sandbox as a
transient service with --ignore-cgroups. The design uses --systemd-cgroup with memory and CPU in the spec
instead, so the sandbox sees its real limits; the spike tested that mode by hand.
"""
import fcntl
import json
import os
import secrets
import subprocess
import sys
import time

BASE = "/opt/orbit-spike"
RUNSC = [f"{BASE}/runsc", "--root=/run/orbit-spike/runsc", "--ignore-cgroups"]
STATE = "/run/orbit-spike/state.json"
SLICE = "sandboxspike.slice"
IDLE_S = int(os.environ.get("IDLE_S", 1800))
MAX_LIFE_S = int(os.environ.get("MAX_LIFE_S", 14400))
MAX_LIVE = int(os.environ.get("MAX_LIVE", 3))
CALL_TIMEOUT_S = int(os.environ.get("CALL_TIMEOUT_S", 900))
NETWORK = os.environ.get("NETWORK", "none")  # "none" or "proxy"


def sh(*args, check=True, **kw):
    return subprocess.run(list(args), check=check, text=True, capture_output=True, **kw)


def unit(key):
    return f"sbxspike-{key}.service"


def load():
    try:
        with open(STATE) as f:
            return json.load(f)
    except FileNotFoundError:
        return {}


def save(state):
    with open(STATE + ".tmp", "w") as f:
        json.dump(state, f)
    os.replace(STATE + ".tmp", STATE)


def netns_up(key, idx):
    """One netns + veth /30 per sandbox. Host side 10.200.<idx>.1, sandbox side .2, only route = the /30."""
    ns, host_if, sb_if = f"sbxs-{key}", f"sbxh{idx}", f"sbxp{idx}"
    sh("ip", "netns", "add", ns)
    sh("ip", "link", "add", host_if, "type", "veth", "peer", "name", sb_if, "netns", ns)
    sh("ip", "addr", "add", f"10.200.{idx}.1/30", "dev", host_if)
    sh("ip", "link", "set", host_if, "up")
    sh("ip", "-n", ns, "addr", "add", f"10.200.{idx}.2/30", "dev", sb_if)
    sh("ip", "-n", ns, "link", "set", sb_if, "up")
    sh("ip", "-n", ns, "link", "set", "lo", "up")
    return f"/var/run/netns/{ns}"


def netns_down(key, idx):
    sh("ip", "link", "del", f"sbxh{idx}", check=False)
    sh("ip", "netns", "del", f"sbxs-{key}", check=False)


def kill_sandbox(state, key, reason):
    info = state.pop(key)
    sh("systemctl", "stop", unit(key), check=False)
    sh(*RUNSC, "delete", "--force", key, check=False)
    sh("systemctl", "reset-failed", unit(key), check=False)
    if info.get("netidx") is not None:
        netns_down(key, info["netidx"])
    print(f"deleted {key} ({reason})", file=sys.stderr)


def start(state, key):
    live = [k for k in state if k != key]
    while len(live) >= MAX_LIVE:
        quiet = min(live, key=lambda k: state[k]["last_call"])
        kill_sandbox(state, quiet, "evicted: quiet the longest")
        live.remove(quiet)
    work = f"{BASE}/work/{key}"
    os.makedirs(work, exist_ok=True)
    os.chown(work, 1000, 1000)
    os.chmod(work, 0o700)
    netidx, netns = None, "-"
    if NETWORK == "proxy":
        used = {v.get("netidx") for v in state.values()}
        netidx = next(i for i in range(1, 250) if i not in used)
        netns = netns_up(key, netidx)
    sh(f"{BASE}/bin/mkbundle.py", f"{BASE}/b/{key}", work, netns, "128")
    t0 = time.monotonic()
    sh("systemd-run", "--quiet", f"--unit={unit(key)}", f"--slice={SLICE}",
       "-p", "CPUQuota=100%", "-p", "MemoryMax=1G", "-p", "MemorySwapMax=0", "-p", "TasksMax=1024",
       "-p", f"RuntimeMaxSec={MAX_LIFE_S + 60}",
       *RUNSC, f"--network={'sandbox' if netidx else 'none'}", "run", "--bundle", f"{BASE}/b/{key}", key)
    while sh(*RUNSC, "exec", key, "/bin/true", check=False).returncode != 0:
        if time.monotonic() - t0 > 10:
            raise SystemExit("sandbox did not start")
        time.sleep(0.01)
    now = time.time()
    state[key] = {"started": now, "last_call": now, "netidx": netidx}
    print(f"started {key} in {time.monotonic() - t0:.3f}s", file=sys.stderr)


def call(key, script):
    lock = open("/run/orbit-spike/broker.lock", "w")
    fcntl.flock(lock, fcntl.LOCK_EX)
    state = load()
    gc(state)
    if key not in state:
        start(state, key)
    state[key]["last_call"] = time.time()
    save(state)
    fcntl.flock(lock, fcntl.LOCK_UN)  # other threads may run; one thread is serialized by its own lock
    tlock = open(f"/run/orbit-spike/{key}.lock", "w")
    fcntl.flock(tlock, fcntl.LOCK_EX)
    token = "orbit-agent." + secrets.token_urlsafe(24)  # new per-call token; Orbit would delete it at call end
    pidfile = f"/run/orbit-spike/{key}.callpid"
    env = ["-env", f"ORBIT_TOKEN={token}"]
    if state[key]["netidx"]:
        env += ["-env", f"HTTPS_PROXY=http://call:{secrets.token_urlsafe(8)}@10.200.{state[key]['netidx']}.1:3128"]
    proc = subprocess.Popen([*RUNSC, "exec", "--internal-pid-file", pidfile, *env, key,
                             "/bin/sh", "-c", script])
    try:
        rc = proc.wait(timeout=CALL_TIMEOUT_S)
    except subprocess.TimeoutExpired:
        pid = open(pidfile).read().strip()
        # The call is its own session and process group (runsc exec makes it a session leader): kill the whole group, keep the sandbox.
        sh(*RUNSC, "exec", key, "/bin/kill", "-s", "KILL", f"-{pid}", check=False)
        proc.wait(timeout=10)
        rc = "timeout"
    print(f"call exit={rc}", file=sys.stderr)


def gc(state):
    now = time.time()
    for key in list(state):
        if now - state[key]["last_call"] > IDLE_S:
            kill_sandbox(state, key, "idle")
        elif now - state[key]["started"] > MAX_LIFE_S:
            kill_sandbox(state, key, "max life")


def locked(fn):
    with open("/run/orbit-spike/broker.lock", "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        state = load()
        fn(state)
        save(state)


if __name__ == "__main__":
    op = sys.argv[1]
    if op == "call":
        call(sys.argv[2], sys.argv[3])
    elif op == "gc":
        locked(gc)
    elif op == "stop-all":
        locked(lambda s: [kill_sandbox(s, k, "off switch") for k in list(s)])
    elif op == "list":
        now = time.time()
        for k, v in load().items():
            print(f"{k} age={now - v['started']:.0f}s quiet={now - v['last_call']:.0f}s net={v['netidx']}")
