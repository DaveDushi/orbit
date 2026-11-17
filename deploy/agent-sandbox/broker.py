#!/usr/bin/python3 -I
"""Agent sandbox broker: one gVisor sandbox per chat thread (docs/agent-sandbox-design.md, sections 2-6 and 13).

systemd starts one copy per connection (`<name>.socket`, Accept=yes) as root. Setup and test copies: setup.sh.

A thread sandbox (key: agent + thread + driver, the `work` id Orbit signs into the ticket) starts at the thread's
first call and stays alive between calls. It is a `runsc --systemd-cgroup run --detach` in a scope of the agent's
slice, with tini as PID 1, RLIMIT_NPROC, its own netns and veth (/30, only the proxy port open, nft), a read-only
rootfs and /work from a sparse ext4 image (its size is the disk quota). Each call is one `runsc exec` with the call's
own token and proxy password. At most MAX_LIVE sandboxes live; a new one evicts the one quiet the longest (never one
with a running call: a running call holds its thread lock). `gc` deletes idle and too-old sandboxes, and images
unused for IMAGE_IDLE_HOURS. A delete keeps /work.

Request: one JSON line, then `stdin` bytes.
  {"op": "run", "ticket": "v1.…", "script": "...", "args": [...], "stdin": 0, "stdout_max": 65536, "stderr_max": 65536}
  {"op": "stop_agent", "agent": "<id>"}   off switch for one agent: kill its live sandboxes (busy or idle)
  {"op": "stop_all"}                      off switch for all agents
  {"op": "gc"}                            idle / max life / image idle
  {"op": "list"}
Only `run` needs a ticket: the other ops only stop or delete, and the socket is open to root and the sidecar's group.
Answer: frames of 1 type byte, 4 length bytes (big-endian) and the payload: b"o" stdout, b"e" stderr, and last b"x",
a JSON object: {"exit", "signal", "timed_out", "interrupted": null | reason, "stdout_truncated", "stderr_truncated",
"cpu_ms", "egress_pending": [...]} or {"error": "..."}.

CLI (root, from units): `broker.py cleanup` (socket ExecStartPre: kill everything a former broker left), `broker.py gc`.
"""

import base64
import fcntl
import hashlib
import hmac
import json
import os
import re
import shutil
import signal
import socket
import struct
import subprocess
import sys
import threading
import time
import urllib.request

ENV = os.environ.get
NAME = ENV("SBX_NAME", "orbit-sandbox")
RUN = ENV("SBX_RUN", f"/run/{NAME}")
STATE_DIR = ENV("SBX_STATE", f"/var/lib/{NAME}")
ROOTFS = ENV("SBX_ROOTFS", f"{STATE_DIR}/rootfs/current")
RUNSC_BIN = ENV("SBX_RUNSC", f"/usr/local/lib/{NAME}/gvisor/runsc")
KEY_FILE = ENV("SBX_KEY_FILE", "/etc/orbit/sandbox-broker.key")
SLICE_PREFIX = ENV("SBX_SLICE_PREFIX", "orbit-agents")  # agent slice: <prefix>-<agent hex>.slice
SCOPE_PREFIX = ENV("SBX_SCOPE_PREFIX", NAME)  # scope: <prefix>-<work>.scope
NET = ENV("SBX_NET", "10.201")  # thread /30: <NET>.<idx>.0/30; host .1, sandbox .2
NET_PREFIX = ENV("SBX_NET_PREFIX", "osb")  # netns <prefix>-<work>, veth <prefix>h<idx> / <prefix>p<idx>
PROXY_PORT = int(ENV("SBX_PROXY_PORT", "3128"))
ORBIT_URL = ENV("SBX_ORBIT_URL", "")  # where the broker reports call ends; empty: no report (test copies)
MAX_LIVE = int(ENV("SBX_MAX_LIVE", "3"))
MAX_LIFE_S = int(ENV("SBX_MAX_LIFE_S", "14400"))
BUSY_WAIT_S = float(ENV("SBX_BUSY_WAIT_S", "30"))
QUOTA_MB = int(ENV("SBX_QUOTA_MB", "4096"))
MIN_FREE_MB = int(ENV("SBX_MIN_FREE_MB", "8192"))
MAX_IMAGES_MB = int(ENV("SBX_MAX_IMAGES_MB", "16384"))
IMAGE_IDLE_HOURS = float(ENV("SBX_IMAGE_IDLE_HOURS", "72"))
MAX_TIMEOUT_S = int(ENV("SBX_MAX_TIMEOUT_S", "900"))
MAX_MEMORY_MB = int(ENV("SBX_MAX_MEMORY_MB", "1024"))
NPROC = int(ENV("SBX_NPROC", "128"))
UID = 1000
SYS_ENV = {"PATH": "/usr/sbin:/usr/bin:/sbin:/bin"}
WORK_ID = re.compile(r"^[0-9a-f]{32}$")
AGENT_ID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
SANDBOX_ENV = ["HOME=/work", "LANG=C.UTF-8", "PATH=/work/.local/bin:/usr/local/bin:/usr/bin:/bin",
               "PYTHONUSERBASE=/work/.local", "npm_config_prefix=/work/.local", "TMPDIR=/tmp"]


def runsc(*args: str) -> list[str]:
    return [RUNSC_BIN, f"--root={RUN}/runsc", "--systemd-cgroup", "--network=sandbox", "--overlay2=none", *args]


def sh(*args: str, check: bool = True) -> subprocess.CompletedProcess:
    # Never the client's socket as stdin: `runsc exec` would forward it and shut it down.
    return subprocess.run(list(args), check=check, text=True, capture_output=True, env=SYS_ENV,
                          stdin=subprocess.DEVNULL)


def log(message: str) -> None:
    """Journal: short reasons only, never scripts, tokens or tickets."""
    print(f"{NAME}: {message}", file=sys.stderr, flush=True)


# ---------------------------------------------------------------------------------------------------------------
# Tickets (Orbit signs them: apps/server/src/agent/sandbox.rs). Keys are derived from the one broker key file.

def derived(label: bytes) -> bytes:
    with open(KEY_FILE, "rb") as file:
        key = file.read().strip()
    if len(key) < 32:
        raise RuntimeError("the broker key is too short")
    return hmac.new(key, label, hashlib.sha256).digest()


def b64d(text: str) -> bytes:
    return base64.urlsafe_b64decode(text + "=" * (-len(text) % 4))


def unseal(sealed: str) -> dict:
    """Encrypt-then-MAC: the ticket MAC covers the sealed bytes; here only the HMAC-SHA256 counter keystream."""
    raw = b64d(sealed)
    nonce, data = raw[:16], raw[16:]
    key = derived(b"orbit-sandbox/seal")
    stream = b"".join(hmac.new(key, nonce + struct.pack(">I", i), hashlib.sha256).digest()
                      for i in range((len(data) + 31) // 32))
    return json.loads(bytes(a ^ b for a, b in zip(data, stream)))


def verify_ticket(ticket: str) -> dict:
    parts = ticket.split(".") if isinstance(ticket, str) else []
    if len(parts) != 3 or parts[0] != "v1":
        raise PermissionError("invalid ticket")
    expected = hmac.new(derived(b"orbit-sandbox/ticket"), f"v1.{parts[1]}".encode(), hashlib.sha256).digest()
    if not hmac.compare_digest(expected, b64d(parts[2])):
        raise PermissionError("invalid ticket")
    claims = json.loads(b64d(parts[1]))
    if not (isinstance(claims.get("exp"), int) and claims["exp"] > time.time() * 1000):
        raise PermissionError("the ticket expired")
    if not (WORK_ID.match(str(claims.get("work", ""))) and AGENT_ID.match(str(claims.get("agent", "")))
            and isinstance(claims.get("call"), str) and isinstance(claims.get("limits"), dict)):
        raise PermissionError("invalid ticket")
    claims.update(unseal(claims.pop("sealed")))
    return claims


def service_token() -> str:
    return derived(b"orbit-sandbox/service").hex()


# ---------------------------------------------------------------------------------------------------------------
# State: {"sandboxes": {work: {...}}, "used": {call: exp}, "ended": {gen: reason}}, under the global lock.

class Locked:
    def __enter__(self):
        os.makedirs(f"{RUN}/locks", mode=0o700, exist_ok=True)
        self.lock = open(f"{RUN}/state.lock", "w")
        fcntl.flock(self.lock, fcntl.LOCK_EX)
        try:
            with open(f"{RUN}/state.json") as file:
                self.state = json.load(file)
        except FileNotFoundError:
            self.state = {}
        for key in ("sandboxes", "used", "ended"):
            self.state.setdefault(key, {})
        return self.state

    def __exit__(self, *_):
        with open(f"{RUN}/state.json.tmp", "w") as file:
            json.dump(self.state, file)
        os.replace(f"{RUN}/state.json.tmp", f"{RUN}/state.json")
        self.lock.close()


def thread_lock(work: str, block: bool):
    """The thread's lock: held for the whole call. `None` when busy (non-blocking)."""
    lock = open(f"{RUN}/locks/{work}.lock", "w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | (0 if block else fcntl.LOCK_NB))
    except BlockingIOError:
        lock.close()
        return None
    return lock


def agent_slice(agent: str) -> str:
    return f"{SLICE_PREFIX}-{agent.replace('-', '')}.slice"


def image_path(agent: str, work: str) -> str:
    return f"{STATE_DIR}/agents/{agent}/work/{work}.img"


def ensure_nft() -> None:
    table = NAME.replace("-", "_")
    if sh("nft", "list", "table", "inet", table, check=False).returncode == 0:
        return
    # From a sandbox veth: only TCP to the proxy port on its own host address; nothing is forwarded.
    rules = f"""table inet {table} {{
  chain input {{
    type filter hook input priority -10; policy accept;
    iifname "{NET_PREFIX}h*" ip daddr {NET}.0.0/16 tcp dport {PROXY_PORT} accept
    iifname "{NET_PREFIX}h*" counter drop
  }}
  chain forward {{
    type filter hook forward priority -10; policy accept;
    iifname "{NET_PREFIX}h*" counter drop
    oifname "{NET_PREFIX}h*" counter drop
  }}
}}"""
    subprocess.run(["nft", "-f", "-"], input=rules, text=True, check=True, env=SYS_ENV, capture_output=True)


def net_up(work: str, idx: int) -> str:
    ns, host, peer = f"{NET_PREFIX}-{work}", f"{NET_PREFIX}h{idx}", f"{NET_PREFIX}p{idx}"
    sh("ip", "netns", "add", ns)
    sh("ip", "link", "add", host, "type", "veth", "peer", "name", peer, "netns", ns)
    # IPv6 off before the links go up, else gVisor copies a link-local address.
    sh("sysctl", "-qw", f"net.ipv6.conf.{host}.disable_ipv6=1")
    for name in (peer, "lo", "all", "default"):
        sh("ip", "netns", "exec", ns, "sysctl", "-qw", f"net.ipv6.conf.{name}.disable_ipv6=1")
    sh("ip", "addr", "add", f"{NET}.{idx}.1/30", "dev", host)
    sh("ip", "link", "set", host, "up")
    sh("ip", "-n", ns, "addr", "add", f"{NET}.{idx}.2/30", "dev", peer)
    sh("ip", "-n", ns, "link", "set", peer, "up")
    sh("ip", "-n", ns, "link", "set", "lo", "up")
    return f"/run/netns/{ns}"


def net_down(work: str, idx) -> None:
    if idx is not None:
        sh("ip", "link", "del", f"{NET_PREFIX}h{idx}", check=False)
    sh("ip", "netns", "del", f"{NET_PREFIX}-{work}", check=False)


def unmount(path: str) -> None:
    for _ in range(20):
        if not os.path.ismount(path) or sh("umount", path, check=False).returncode == 0:
            return
        time.sleep(0.25)
    sh("umount", "-l", path, check=False)


def kill_sandbox(state: dict, work: str, reason: str) -> None:
    """Deletes a thread sandbox (all its processes, scope, netns, mount); /work's image stays."""
    info = state["sandboxes"].pop(work, None) or {}
    sh(*runsc("kill", "--all", work, "KILL"), check=False)
    sh(*runsc("delete", "--force", work), check=False)
    sh("systemctl", "stop", f"{SCOPE_PREFIX}-{work}.scope", check=False)
    unmount(f"{RUN}/mnt/{work}")
    net_down(work, info.get("netidx"))
    shutil.rmtree(f"{RUN}/bundles/{work}", ignore_errors=True)
    if info.get("gen"):
        state["ended"][info["gen"]] = {"reason": reason, "at": time.time()}
    log(f"deleted sandbox {work[:8]} ({reason})")


def images_mb() -> float:
    total = 0
    for root, _, files in os.walk(f"{STATE_DIR}/agents"):
        total += sum(os.lstat(os.path.join(root, name)).st_blocks * 512 for name in files)
    return total / 1024 / 1024


def make_image(path: str, quota_mb: int) -> None:
    if os.path.exists(path):
        return
    if shutil.disk_usage(STATE_DIR).free < MIN_FREE_MB * 1024 * 1024 or images_mb() > MAX_IMAGES_MB:
        raise RuntimeError("the server has too little free disk for a new work folder")
    os.makedirs(os.path.dirname(path), mode=0o700, exist_ok=True)
    fd = os.open(path + ".new", os.O_RDWR | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
    os.ftruncate(fd, quota_mb * 1024 * 1024)
    os.close(fd)
    sh("mkfs.ext4", "-q", "-F", "-m", "0", "-O", "^has_journal", "-L", "work",
       "-E", f"root_owner={UID}:{UID},nodiscard", path + ".new")
    os.replace(path + ".new", path)


def bundle(work: str, netns: str, rootfs: str, agent: str, limits: dict) -> str:
    path = f"{RUN}/bundles/{work}"
    os.makedirs(path, mode=0o700, exist_ok=True)
    memory = min(int(limits.get("memory_mb", MAX_MEMORY_MB)), MAX_MEMORY_MB) * 1024 * 1024
    fsize = QUOTA_MB * 1024 * 1024
    config = {
        "ociVersion": "1.0.0",
        "process": {
            "user": {"uid": UID, "gid": UID},
            "args": ["/usr/bin/tini", "--", "/bin/sleep", "infinity"],
            "env": SANDBOX_ENV,
            "cwd": "/work",
            "capabilities": {k: [] for k in ("bounding", "effective", "inheritable", "permitted", "ambient")},
            "noNewPrivileges": True,
            "oomScoreAdj": -900,
            "rlimits": [{"type": "RLIMIT_NPROC", "hard": NPROC, "soft": NPROC},
                        {"type": "RLIMIT_NOFILE", "hard": 1024, "soft": 1024},
                        {"type": "RLIMIT_FSIZE", "hard": fsize, "soft": fsize},
                        {"type": "RLIMIT_CORE", "hard": 0, "soft": 0}],
        },
        "root": {"path": rootfs, "readonly": True},
        "hostname": "sandbox",
        "mounts": [
            {"destination": "/proc", "type": "proc", "source": "proc"},
            {"destination": "/tmp", "type": "tmpfs", "source": "tmpfs", "options": ["size=256m", "nosuid", "nodev"]},
            {"destination": "/work", "type": "bind", "source": f"{RUN}/mnt/{work}",
             "options": ["rbind", "rw", "nosuid", "nodev"]},
        ],
        "linux": {
            "namespaces": [{"type": t} for t in ("pid", "ipc", "uts", "mount")] + [{"type": "network", "path": netns}],
            # The scope runsc makes in the agent slice. No pids limit: a host task limit crashes the Sentry; the
            # guest limit is RLIMIT_NPROC (section 13).
            "cgroupsPath": f"{agent_slice(agent)}:{SCOPE_PREFIX}:{work}",
            "resources": {
                "memory": {"limit": memory, "swap": memory},
                "cpu": {"quota": int(float(limits.get("cpu", 1)) * 100000), "period": 100000},
            },
        },
    }
    with open(f"{path}/config.json", "w") as file:
        json.dump(config, file)
    return path


def start(state: dict, work: str, agent: str, limits: dict) -> None:
    ensure_nft()
    image = image_path(agent, work)
    quota = min(int(limits.get("disk_mb", QUOTA_MB)), QUOTA_MB)
    make_image(image, quota)
    mount = f"{RUN}/mnt/{work}"
    os.makedirs(mount, mode=0o700, exist_ok=True)
    sh("mount", "-o", "loop,nodev,nosuid,noexec,discard", image, mount)
    used = {v.get("netidx") for v in state["sandboxes"].values()}
    idx = next(i for i in range(1, 250) if i not in used)
    gen = os.urandom(8).hex()
    state["sandboxes"][work] = {"agent": agent, "gen": gen, "started": time.time(), "last_call": time.time(),
                                "idle_s": 60 * max(5, min(int(limits.get("idle_minutes", 30)), 120)), "netidx": idx}
    try:
        netns = net_up(work, idx)
        rootfs = os.path.realpath(ROOTFS)
        # No pipes: the sandbox keeps its stdio open for its whole life.
        subprocess.run(runsc("run", "--detach", "--bundle", bundle(work, netns, rootfs, agent, limits), work),
                       check=True, env=SYS_ENV, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL)
        deadline = time.monotonic() + 15
        while sh(*runsc("exec", "--user", f"{UID}:{UID}", work, "/bin/true"), check=False).returncode != 0:
            if time.monotonic() > deadline:
                raise RuntimeError("the sandbox did not start")
            time.sleep(0.02)
        cgroup = sh("systemctl", "show", "-P", "ControlGroup", f"{SCOPE_PREFIX}-{work}.scope").stdout.strip()
        state["sandboxes"][work]["cgroup"] = cgroup
    except Exception:
        kill_sandbox(state, work, "start failed")
        raise


def place(work: str, agent: str, limits: dict) -> str:
    """Makes sure the thread's sandbox lives (evicts the quietest when full). Returns its generation."""
    deadline = time.monotonic() + BUSY_WAIT_S
    while True:
        with Locked() as state:
            gc(state)
            sandboxes = state["sandboxes"]
            if work in sandboxes:
                sandboxes[work]["last_call"] = time.time()
                return sandboxes[work]["gen"]
            for other in sorted((k for k in sandboxes if k != work), key=lambda k: sandboxes[k]["last_call"]):
                if len(sandboxes) < MAX_LIVE:
                    break
                quiet = thread_lock(other, block=False)  # None: a call runs there; never evict it
                if quiet:
                    kill_sandbox(state, other, "evicted: quiet the longest")
                    quiet.close()
            if len(sandboxes) < MAX_LIVE:
                start(state, work, agent, limits)
                return sandboxes[work]["gen"]
        if time.monotonic() > deadline:
            raise RuntimeError("all sandboxes are busy, try again")
        time.sleep(0.5)


def gc(state: dict) -> None:
    now = time.time()
    for work, info in list(state["sandboxes"].items()):
        if now - info["started"] > MAX_LIFE_S:
            kill_sandbox(state, work, "the sandbox reached its maximum life")
        elif now - info["last_call"] > info["idle_s"]:
            lock = thread_lock(work, block=False)
            if lock:
                kill_sandbox(state, work, "idle")
                lock.close()
    state["used"] = {call: exp for call, exp in state["used"].items() if exp > now * 1000}
    state["ended"] = {gen: e for gen, e in state["ended"].items() if now - e["at"] < 3600}


def gc_images() -> int:
    removed = 0
    live = set()
    with Locked() as state:
        live = set(state["sandboxes"])
    for root, _, files in os.walk(f"{STATE_DIR}/agents"):
        for name in files:
            path = os.path.join(root, name)
            work = name.removesuffix(".img")
            if work in live or time.time() - os.lstat(path).st_mtime < IMAGE_IDLE_HOURS * 3600:
                continue
            lock = thread_lock(work, block=False) if WORK_ID.match(work) else None
            if lock:
                os.unlink(path)
                removed += 1
                lock.close()
    return removed


def cleanup() -> None:
    """At socket start: a new broker cannot trust old state, so every sandbox goes (images stay)."""
    os.makedirs(f"{RUN}/locks", mode=0o700, exist_ok=True)
    with Locked() as state:
        listed = sh(*runsc("list", "-quiet"), check=False).stdout.split()
        works = set(state["sandboxes"]) | {w for w in listed if WORK_ID.match(w)}
        if os.path.isdir(f"{RUN}/mnt"):
            works |= {w for w in os.listdir(f"{RUN}/mnt") if WORK_ID.match(w)}
        for work in works:
            state["sandboxes"].setdefault(work, {})
            kill_sandbox(state, work, "broker start")
        for ns in sh("ip", "netns", "list", check=False).stdout.split("\n"):
            name = ns.split(" ")[0]
            if name.startswith(f"{NET_PREFIX}-"):
                sh("ip", "netns", "del", name, check=False)
        for link in sh("ip", "-o", "link", "show", check=False).stdout.split("\n"):
            match = re.match(rf"^\d+: ({NET_PREFIX}h\d+)[@:]", link)
            if match:
                sh("ip", "link", "del", match.group(1), check=False)
        for unit in sh("systemctl", "list-units", "--plain", "--no-legend", f"{SCOPE_PREFIX}-*.scope",
                       check=False).stdout.split("\n"):
            if unit.strip():
                sh("systemctl", "stop", unit.split()[0], check=False)
        sh("nft", "delete", "table", "inet", NAME.replace("-", "_"), check=False)
        state["sandboxes"] = {}
    unmount(f"{RUN}/runsc/null-netns")  # runsc leaves this nsfs mount; the state folder cannot go without its umount


# ---------------------------------------------------------------------------------------------------------------
# One connection.

conn = None
write_lock = threading.Lock()


def frame(kind: bytes, payload: bytes) -> None:
    with write_lock:
        conn.sendall(kind + struct.pack(">I", len(payload)) + payload)


def finish(result: dict) -> None:
    frame(b"x", json.dumps(result).encode())


def read_line(limit: int = 4 * 1024 * 1024) -> tuple[bytes, bytes]:
    data = b""
    while b"\n" not in data:
        chunk = conn.recv(65536)
        if not chunk:
            raise ValueError("no request")
        data += chunk
        if len(data) > limit:
            raise ValueError("request too long")
    line, rest = data.split(b"\n", 1)
    return line, rest


def pump(source, kind: bytes, cap: int, result: dict, key: str) -> None:
    sent = 0
    while chunk := source.read1(65536):
        room = cap - sent
        if room > 0:
            frame(kind, chunk[:room])
            sent += min(room, len(chunk))
        if len(chunk) > room:
            result[key] = True  # Keep reading so the program does not block on a full pipe.


def feed(child, size: int, first: bytes) -> None:
    try:
        left = size
        data = first[:left]
        while True:
            if data:
                child.stdin.write(data)
                left -= len(data)
            if left <= 0:
                break
            data = conn.recv(min(65536, left))
            if not data:
                break
    except OSError:
        pass
    finally:
        try:
            child.stdin.close()
        except OSError:
            pass


def cpu_usec(cgroup: str) -> int:
    try:
        with open(f"/sys/fs/cgroup{cgroup}/cpu.stat") as file:
            return int(file.readline().split()[1])
    except (OSError, ValueError, IndexError):
        return 0


def kill_call(work: str, pidfile: str) -> None:
    """Kills the call's process group (the call is its own session). The sandbox and what earlier calls left running
    in other sessions stay. Killing the `runsc exec` client alone stops nothing inside. The kill runs as root in the
    sandbox with only CAP_KILL: the sandbox user may be at its RLIMIT_NPROC (a fork bomb) and could not start it."""
    try:
        with open(pidfile) as file:
            pid = int(file.read().strip())
        sh(*runsc("exec", "--user", "0:0", "--cap", "CAP_KILL", work, "/bin/bash", "--noprofile", "--norc", "-c",
                  'kill -KILL -- "-$1" 2>/dev/null', "kill", str(pid)), check=False)
    except (OSError, ValueError):
        pass


def report_end(call: str, result: dict) -> list:
    """Tells Orbit the call ended (it deletes the call token); returns the egress requests waiting for a yes."""
    if not ORBIT_URL:
        return []
    body = {k: result.get(k) for k in ("exit", "signal", "timed_out", "interrupted", "cpu_ms")}
    request = urllib.request.Request(
        f"{ORBIT_URL}/api/internal/agent/sandbox/calls/{call}/end", data=json.dumps(body).encode(), method="POST",
        headers={"Authorization": f"Bearer {service_token()}", "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            return json.load(response).get("egress_pending", [])
    except Exception as error:  # Orbit closes a call without an end as lost; the token dies then.
        log(f"call end report failed: {type(error).__name__}")
        return []


def run(request: dict, rest: bytes) -> None:
    script, args = request.get("script"), request.get("args", [])
    if not isinstance(script, str) or "\0" in script or not isinstance(args, list) or not all(
            isinstance(a, str) and "\0" not in a for a in args):
        return finish({"error": "invalid request"})
    claims = verify_ticket(request.get("ticket"))
    work, agent, call, limits = claims["work"], claims["agent"], claims["call"], claims["limits"]
    with Locked() as state:
        if call in state["used"]:
            raise PermissionError("the ticket was used")
        state["used"][call] = claims["exp"]
    timeout = max(1, min(int(limits.get("timeout_s", 120)), MAX_TIMEOUT_S))
    stdin_size = max(0, int(request.get("stdin", 0)))
    stdout_max = max(0, min(int(request.get("stdout_max", 65536)), 64 * 1024 * 1024))
    stderr_max = max(0, min(int(request.get("stderr_max", 65536)), 4 * 1024 * 1024))
    lock = thread_lock(work, block=True)  # calls of one thread take turns
    try:
        gen = place(work, agent, limits)
        with Locked() as state:
            cgroup, idx = state["sandboxes"][work].get("cgroup", ""), state["sandboxes"][work]["netidx"]
        os.utime(image_path(agent, work))
        env = [f"ORBIT_TOKEN={claims['token']}", "ORBIT_URL=http://orbit.internal",
               f"HTTPS_PROXY=http://call:{claims['proxy_password']}@{NET}.{idx}.1:{PROXY_PORT}"]
        pidfile = f"{RUN}/locks/{work}.pid"
        result = {"stdout_truncated": False, "stderr_truncated": False, "timed_out": False, "interrupted": None}
        cpu_before = cpu_usec(cgroup)
        # The unit's stop (RuntimeMaxSec, systemctl stop) kills this client, which alone stops nothing inside.
        signal.signal(signal.SIGTERM, lambda *_: (kill_call(work, pidfile), os._exit(143)))
        child = subprocess.Popen(
            runsc("exec", "--user", f"{UID}:{UID}", "--cwd", "/work", "--internal-pid-file", pidfile,
                  *(x for e in SANDBOX_ENV + env for x in ("--env", e)), work,
                  "/bin/bash", "--noprofile", "--norc", "-c", script, "sandbox", *args),
            env=SYS_ENV, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, close_fds=True)
        # Daemon threads: a background process of the call may keep the pipes open for hours.
        threads = [threading.Thread(target=feed, args=(child, stdin_size, rest), daemon=True),
                   threading.Thread(target=pump, args=(child.stdout, b"o", stdout_max, result, "stdout_truncated"),
                                    daemon=True),
                   threading.Thread(target=pump, args=(child.stderr, b"e", stderr_max, result, "stderr_truncated"),
                                    daemon=True)]
        for thread in threads:
            thread.start()
        try:
            child.wait(timeout)
        except subprocess.TimeoutExpired:
            result["timed_out"] = True
            kill_call(work, pidfile)
            try:
                child.wait(10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
        # What the call left in its group goes too (also a fork bomb's children); `setsid` keeps a process alive for
        # later calls of the thread.
        kill_call(work, pidfile)
        for thread in threads[1:]:
            thread.join(2)
        code = child.returncode
        result.update(exit=code if code >= 0 else None, signal=-code if code < 0 else None,
                      cpu_ms=max(0, cpu_usec(cgroup) - cpu_before) // 1000)
        with Locked() as state:
            live = state["sandboxes"].get(work)
            if live and live["gen"] == gen:
                live["last_call"] = time.time()
            else:
                result["interrupted"] = state["ended"].get(gen, {}).get("reason", "the sandbox was deleted")
        result["egress_pending"] = report_end(call, result)
        finish(result)
    finally:
        lock.close()


def stop(match) -> int:
    with Locked() as state:
        works = [w for w, info in state["sandboxes"].items() if match(info)]
        for work in works:
            kill_sandbox(state, work, "off switch")
    return len(works)


def main() -> None:
    global conn
    conn = socket.socket(fileno=0)
    try:
        line, rest = read_line()
        request = json.loads(line)
        op = request.get("op")
        if op == "run":
            run(request, rest)
        elif op == "stop_agent":
            agent = str(request.get("agent", ""))
            if not AGENT_ID.match(agent):
                return finish({"error": "invalid request"})
            finish({"stopped": stop(lambda info: info.get("agent") == agent)})
        elif op == "stop_all":
            finish({"stopped": stop(lambda info: True)})
        elif op == "gc":
            with Locked() as state:
                gc(state)
            finish({"images_removed": gc_images()})
        elif op == "list":
            with Locked() as state:
                now = time.time()
                finish({"sandboxes": [{"work": w, "agent": i["agent"], "age_s": int(now - i["started"]),
                                       "quiet_s": int(now - i["last_call"])} for w, i in state["sandboxes"].items()]})
        else:
            finish({"error": "unknown op"})
    except Exception as error:  # The agent gets a short reason; the journal keeps the type.
        log(f"request failed: {type(error).__name__}: {str(error)[:200]}")
        try:
            finish({"error": str(error)[:300] if isinstance(error, (PermissionError, RuntimeError)) else
                    "the sandbox failed"})
        except OSError:
            pass


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "cleanup":
        cleanup()
    elif len(sys.argv) > 1 and sys.argv[1] == "gc":
        with Locked() as state:
            gc(state)
        print(json.dumps({"images_removed": gc_images()}))
    else:
        main()
