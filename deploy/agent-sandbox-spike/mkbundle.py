#!/usr/bin/python3 -I
"""Make an OCI bundle for one thread sandbox (spike only).

Usage: mkbundle.py <bundle-dir> <work-dir> [netns-path] [pids-limit]
"""
import json
import os
import sys

bundle, work = sys.argv[1], sys.argv[2]
netns = sys.argv[3] if len(sys.argv) > 3 and sys.argv[3] != "-" else None
pids = int(sys.argv[4]) if len(sys.argv) > 4 else 256

os.makedirs(bundle, exist_ok=True)
namespaces = [{"type": t} for t in ("pid", "ipc", "uts", "mount")]
namespaces.append({"type": "network", "path": netns} if netns else {"type": "network"})
config = {
    "ociVersion": "1.0.0",
    "process": {
        "user": {"uid": 1000, "gid": 1000},
        "args": ["/sbin/init"],
        "env": ["PATH=/usr/local/bin:/usr/bin:/bin", "HOME=/work", "LANG=C.UTF-8"],
        "cwd": "/work",
        "capabilities": {k: [] for k in ("bounding", "effective", "inheritable", "permitted", "ambient")},
        "noNewPrivileges": True,
        "rlimits": [{"type": "RLIMIT_NOFILE", "hard": 1024, "soft": 1024}, {"type": "RLIMIT_NPROC", "hard": pids, "soft": pids}],
    },
    "root": {"path": "/opt/orbit-spike/rootfs", "readonly": True},
    "hostname": "sandbox",
    "mounts": [
        {"destination": "/proc", "type": "proc", "source": "proc"},
        {"destination": "/dev", "type": "tmpfs", "source": "tmpfs"},
        {"destination": "/tmp", "type": "tmpfs", "source": "tmpfs", "options": ["size=256m", "nosuid", "nodev"]},
        {"destination": "/etc/inittab", "type": "bind", "source": "/opt/orbit-spike/init/inittab", "options": ["rbind", "ro"]},
        {"destination": "/work", "type": "bind", "source": work, "options": ["rbind", "rw", "nosuid", "nodev"]},
    ],
    "linux": {
        "namespaces": namespaces,
        "resources": {"pids": {"limit": pids}},
    },
}
with open(os.path.join(bundle, "config.json"), "w") as f:
    json.dump(config, f, indent=1)
