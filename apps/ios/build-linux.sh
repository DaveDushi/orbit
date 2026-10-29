#!/usr/bin/env bash
# Build an unsigned device app with the installed omarchy-apple-dev toolchain.
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")"

apple_dev="${OMARCHY_APPLE_DEV:-$HOME/omarchy-apple-dev}"
if [ ! -x "$apple_dev/ship.sh" ]; then
    echo "Set OMARCHY_APPLE_DEV to your omarchy-apple-dev checkout." >&2
    exit 1
fi

# Keep runtime metadata in project.yml, shared with the Xcode build. xtool does
# not expand Xcode build settings, so resolve the version fields before packing.
python3 - <<'PY'
import plistlib
import re
from pathlib import Path

import yaml

project = yaml.safe_load(Path("project.yml").read_text())
settings = project["settings"]["base"]


def resolve(value):
    if isinstance(value, str):
        return re.sub(r"\$\(([^)]+)\)", lambda match: str(settings[match[1]]), value)
    if isinstance(value, dict):
        return {key: resolve(item) for key, item in value.items()}
    if isinstance(value, list):
        return [resolve(item) for item in value]
    return value


for target in project["targets"].values():
    if "info" not in target:
        continue
    info = target["info"]
    properties = resolve(info["properties"])
    families = target["settings"]["base"].get("TARGETED_DEVICE_FAMILY", "1")
    properties["UIDeviceFamily"] = [int(family) for family in families.split(",")]
    path = Path(info["path"])
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(plistlib.dumps(properties))
PY

# Older xtool builds initialize an incompatible authentication provider even
# during unsigned builds. Give Foundation an isolated home so they cannot clear
# the login used by xtool-1.22.2. Keep the real Swift SDK registry available.
build_home=$(mktemp -d)
trap 'rm -rf "$build_home"' EXIT
if [ -z "${XDG_CONFIG_HOME:-}" ]; then
    ln -s "$HOME/.swiftpm" "$build_home/.swiftpm"
fi
ulimit -n 65536
CFFIXED_USER_HOME="$build_home" CONFIGURATION="${CONFIGURATION:-debug}" "$apple_dev/ship.sh" --device
printf '\nInstall app + Share Extension with: %s/install-linux.sh IPHONE_UDID\n' "$PWD"
