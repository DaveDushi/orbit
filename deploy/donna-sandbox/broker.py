#!/usr/bin/python3 -I
"""Donna's media sandbox broker. systemd starts one copy per connection (orbit-donna.socket, Accept=yes) as root, in
its own service instance with the CPU, memory, task and time limits of the unit. Setup: setup.sh in this folder.

It reads one request from the socket, mounts the conversation's disk image (a sparse ext4 file: its size is the hard
disk quota of /work), and runs bash as the sandbox user in bubblewrap: an empty environment, no network, a read-only
system, only /work writable. Every file of /work goes in and out through such a run, so the agent never touches the
files itself.

Request: one JSON line, then `stdin` bytes.
  {"op": "run", "work": "<id>", "script": "...", "args": [...], "timeout": 120, "stdin": 0, "stdout_max": 65536,
   "stderr_max": 65536}
  {"op": "drop", "work": "<id>"}            deletes the conversation's /work
  {"op": "gc", "idle_seconds": 86400}       deletes every /work unused for that long
Answer: frames of 1 type byte, 4 length bytes (big-endian) and the payload: b"o" stdout, b"e" stderr, and last b"x",
a JSON object: {"exit": int | null, "timed_out": bool, "stdout_truncated": bool, "stderr_truncated": bool}
or {"error": "..."}.
"""

import fcntl
import json
import os
import pwd
import re
import resource
import shutil
import signal
import socket
import struct
import subprocess
import sys
import threading
import time

USER = os.environ.get("DONNA_USER", "donna-sandbox")
STATE = os.environ.get("DONNA_STATE", "/var/lib/orbit-donna")
QUOTA_MB = int(os.environ.get("DONNA_QUOTA_MB", "2048"))
MAX_TIMEOUT = int(os.environ.get("DONNA_MAX_TIMEOUT", "600"))
# A new /work is refused while the disk has less free space than this.
MIN_FREE_MB = int(os.environ.get("DONNA_MIN_FREE_MB", "4096"))
MAX_OUTPUT = 1024 * 1024 * 1024
WORK_ID = re.compile(r"^[A-Za-z0-9_-]{1,100}$")
MOUNT = "/tmp/work"  # In the unit's private /tmp: other runs and the host never see this mount.
ETC = ["alternatives", "ld.so.cache", "ld.so.conf", "ld.so.conf.d", "ImageMagick-6", "ImageMagick-7", "fonts",
       "localtime", "mime.types", "magic"]

conn = socket.socket(fileno=0)
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


def image(work: str) -> str:
    return os.path.join(STATE, "work", f"{work}.img")


def open_image(path: str, uid: int, gid: int) -> int:
    """The image, locked for this run (runs of one conversation take turns); made on first use."""
    fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    fcntl.flock(fd, fcntl.LOCK_EX)
    if os.fstat(fd).st_size == 0:
        if shutil.disk_usage(os.path.dirname(path)).free < MIN_FREE_MB * 1024 * 1024:
            os.close(fd)
            os.unlink(path)
            raise RuntimeError("the server has too little free disk for a new work folder")
        os.ftruncate(fd, QUOTA_MB * 1024 * 1024)
        subprocess.run(
            ["mkfs.ext4", "-q", "-F", "-m", "0", "-O", "^has_journal", "-L", "work",
             "-E", f"root_owner={uid}:{gid},nodiscard", path],
            check=True, env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin"}, stdout=subprocess.DEVNULL,
        )
    os.utime(fd)
    return fd


def bwrap(uid: int, gid: int, script: str, args: list[str]) -> list[str]:
    passwd = os.path.join("/tmp", "passwd")
    group = os.path.join("/tmp", "group")
    with open(passwd, "w") as file:
        file.write(f"donna:x:{uid}:{gid}:Donna:/work:/usr/bin/bash\n")
    with open(group, "w") as file:
        file.write(f"donna:x:{gid}:\n")
    os.chmod(passwd, 0o644)
    os.chmod(group, 0o644)
    command = [
        "/usr/bin/setpriv", f"--reuid={uid}", f"--regid={gid}", "--clear-groups", "--no-new-privs",
        "--inh-caps=-all", "--bounding-set=-all", "--",
        "/usr/bin/bwrap", "--unshare-all", "--unshare-user", "--disable-userns", "--die-with-parent", "--new-session", "--cap-drop", "ALL",
        "--hostname", "sandbox",
        "--ro-bind", "/usr", "/usr", "--tmpfs", "/usr/local", "--remount-ro", "/usr/local",
    ]
    for name in ("bin", "sbin", "lib", "lib64", "lib32", "libx32"):
        target = os.path.join("/", name)
        if os.path.islink(target):
            command += ["--symlink", os.readlink(target), target]
    for name in ETC:
        command += ["--ro-bind-try", f"/etc/{name}", f"/etc/{name}"]
    command += [
        "--ro-bind", passwd, "/etc/passwd", "--ro-bind", group, "/etc/group",
        "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp", "--bind", MOUNT, "/work", "--remount-ro", "/",
        "--chdir", "/work",
        "--clearenv", "--setenv", "PATH", "/usr/bin:/bin", "--setenv", "HOME", "/work", "--setenv", "LANG", "C.UTF-8",
        "--setenv", "TMPDIR", "/tmp", "--setenv", "USER", "donna",
        "--", "/usr/bin/bash", "--noprofile", "--norc", "-c", script, "sandbox", *args,
    ]
    return command


def limits() -> None:
    """In the child, before setpriv: no core files, one file at most as large as /work."""
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    quota = QUOTA_MB * 1024 * 1024
    resource.setrlimit(resource.RLIMIT_FSIZE, (quota, quota))
    resource.setrlimit(resource.RLIMIT_NOFILE, (1024, 1024))


def pump(source, kind: bytes, cap: int, state: dict, key: str) -> None:
    sent = 0
    while True:
        chunk = source.read1(65536) if hasattr(source, "read1") else source.read(65536)
        if not chunk:
            break
        room = cap - sent
        if room > 0:
            frame(kind, chunk[:room])
            sent += min(room, len(chunk))
        if len(chunk) > room:
            state[key] = True  # Keep reading so the program does not block on a full pipe.


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
    except (BrokenPipeError, OSError):
        pass
    finally:
        try:
            child.stdin.close()
        except OSError:
            pass


def run(request: dict, rest: bytes) -> None:
    work = str(request.get("work", ""))
    script = request.get("script")
    args = request.get("args", [])
    if not WORK_ID.match(work) or not isinstance(script, str) or not isinstance(args, list):
        return finish({"error": "invalid request"})
    if not all(isinstance(arg, str) and "\0" not in arg for arg in args) or "\0" in script:
        return finish({"error": "invalid request"})
    timeout = max(1, min(int(request.get("timeout", 120)), MAX_TIMEOUT))
    stdin_size = max(0, int(request.get("stdin", 0)))
    stdout_max = max(0, min(int(request.get("stdout_max", 65536)), MAX_OUTPUT))
    stderr_max = max(0, min(int(request.get("stderr_max", 65536)), 4 * 1024 * 1024))
    user = pwd.getpwnam(USER)
    os.makedirs(os.path.join(STATE, "work"), mode=0o700, exist_ok=True)
    fd = open_image(image(work), user.pw_uid, user.pw_gid)
    os.makedirs(MOUNT, mode=0o755, exist_ok=True)
    subprocess.run(["mount", "-o", "loop,nodev,nosuid,discard", image(work), MOUNT], check=True,
                   env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin"})
    state = {"stdout_truncated": False, "stderr_truncated": False}
    timed_out = False
    try:
        # An empty environment: bwrap's first process keeps the one it was started with (/proc/1/environ).
        child = subprocess.Popen(bwrap(user.pw_uid, user.pw_gid, script, args), env={}, stdin=subprocess.PIPE,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True,
                                 preexec_fn=limits, close_fds=True)
        threads = [
            threading.Thread(target=feed, args=(child, stdin_size, rest), daemon=True),
            threading.Thread(target=pump, args=(child.stdout, b"o", stdout_max, state, "stdout_truncated")),
            threading.Thread(target=pump, args=(child.stderr, b"e", stderr_max, state, "stderr_truncated")),
        ]
        for thread in threads:
            thread.start()
        try:
            child.wait(timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            # The whole session: bwrap dies, and with it every process in the sandbox's PID namespace.
            os.killpg(child.pid, signal.SIGKILL)
            child.wait()
        for thread in threads[1:]:
            thread.join(10)
        code = child.returncode
        finish({"exit": code if code >= 0 else None, "signal": -code if code < 0 else None, "timed_out": timed_out,
                **state})
    finally:
        for _ in range(20):
            if subprocess.run(["umount", MOUNT], env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin"},
                              stderr=subprocess.DEVNULL).returncode == 0:
                break
            time.sleep(0.25)
        else:
            subprocess.run(["umount", "-l", MOUNT], env={"PATH": "/usr/sbin:/usr/bin:/sbin:/bin"})
        os.close(fd)


def remove(path: str, idle_seconds: float | None) -> bool:
    """Deletes an image that no run uses (and, with `idle_seconds`, that none used for that long)."""
    try:
        fd = os.open(path, os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC)
    except FileNotFoundError:
        return False
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        os.close(fd)
        return False
    try:
        if idle_seconds is not None and time.time() - os.fstat(fd).st_mtime < idle_seconds:
            return False
        os.unlink(path)
        return True
    finally:
        os.close(fd)


def main() -> None:
    try:
        line, rest = read_line()
        request = json.loads(line)
        op = request.get("op")
        if op == "run":
            run(request, rest)
        elif op == "drop":
            work = str(request.get("work", ""))
            if not WORK_ID.match(work):
                return finish({"error": "invalid request"})
            finish({"removed": remove(image(work), None)})
        elif op == "gc":
            idle = max(60, int(request.get("idle_seconds", 86400)))
            folder = os.path.join(STATE, "work")
            names = os.listdir(folder) if os.path.isdir(folder) else []
            removed = sum(remove(os.path.join(folder, name), idle) for name in names if name.endswith(".img"))
            finish({"removed": removed})
        else:
            finish({"error": "unknown op"})
    except Exception as error:  # The agent gets a reason; the journal keeps the details.
        print(f"donna sandbox: {error!r}", file=sys.stderr)
        try:
            finish({"error": str(error)[:300]})
        except OSError:
            pass


if __name__ == "__main__":
    main()
