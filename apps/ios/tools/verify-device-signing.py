#!/usr/bin/env python3
"""Check the final app AND extension entitlements after recursive signing."""
import plistlib
import struct
import subprocess
import sys
from pathlib import Path


def entitlements(executable):
    raw = subprocess.check_output(["rcodesign", "extract", "signature-raw", str(executable)])
    magic, length, count = struct.unpack_from(">III", raw)
    if magic != 0xFADE0CC0 or length > len(raw):
        raise ValueError("Invalid code signature")
    for index in range(count):
        slot, offset = struct.unpack_from(">II", raw, 12 + index * 8)
        if slot == 5:
            magic, size = struct.unpack_from(">II", raw, offset)
            if magic != 0xFADE7171 or offset + size > len(raw):
                raise ValueError("Invalid entitlements slot")
            return plistlib.loads(raw[offset + 8:offset + size])
    raise ValueError("Code signature has no entitlements")


def verify(stage):
    app = stage / "Payload/Orbit.app"
    for bundle in [app, *sorted((app / "PlugIns").glob("*.appex"))]:
        info = plistlib.loads((bundle / "Info.plist").read_bytes())
        expected = plistlib.loads((stage / (bundle.stem + ".entitlements")).read_bytes())
        actual = entitlements(bundle / info["CFBundleExecutable"])
        for key in ["application-identifier", "com.apple.developer.team-identifier", "keychain-access-groups", "get-task-allow"]:
            if actual.get(key) != expected.get(key):
                raise ValueError(f"{bundle.name}: signing did not preserve {key}")
    print("Verified app and Share Extension code signatures retain their own app IDs and the same shared keychain group.")


if __name__ == "__main__":
    try:
        verify(Path(sys.argv[1]))
    except (OSError, ValueError, KeyError, struct.error, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
