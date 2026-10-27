#!/usr/bin/env python3
"""Stage a development build with validated device profiles and a shared keychain.

Internal helper for install-linux.sh. No credential values are printed. The
caller owns a private temporary directory and removes it after installation.
"""
import datetime
import fnmatch
import plistlib
import re
import shutil
import sys
from pathlib import Path


def payload(raw):
    return plistlib.loads(raw[raw.index(b"<?xml"):raw.rindex(b"</plist>") + 8])


def prepare(app, stage, profiles_dir, signing_dir, entitlements_path, udid):
    bundles = [app, *sorted((app / "PlugIns").glob("*.appex"))]
    original_ids = [plistlib.loads((bundle / "Info.plist").read_bytes())["CFBundleIdentifier"] for bundle in bundles]
    now = datetime.datetime.now(datetime.UTC).replace(tzinfo=None)
    profiles = []
    for path in profiles_dir.glob("*.mobileprovision"):
        raw = path.read_bytes()
        profile = payload(raw)
        if udid not in profile.get("ProvisionedDevices", []) or profile["ExpirationDate"] <= now:
            continue
        prefix = profile["ApplicationIdentifierPrefix"][0] + "."
        identifier = profile["Entitlements"]["application-identifier"]
        if not identifier.startswith(prefix):
            continue
        profiles.append((identifier[len(prefix):], prefix, profile, raw))

    choices = sorted(
        (item for item in profiles if re.fullmatch(r"(?:XTL-[A-Z0-9]+\.)?" + re.escape(original_ids[0]), item[0])),
        key=lambda item: item[2]["ExpirationDate"], reverse=True,
    )
    selected = None
    for main in choices:
        signing = signing_dir / main[2]["TeamIdentifier"][0]
        if not signing.is_file():
            continue
        cached = plistlib.loads(signing.read_bytes())
        matched = [main]
        for original in original_ids[1:]:
            if not original.startswith(original_ids[0] + "."):
                raise ValueError("An extension bundle ID must be nested under the application bundle ID")
            expected = main[0] + original[len(original_ids[0]):]
            candidates = [item for item in profiles if item[0] == expected and item[1] == main[1]
                          and item[2]["TeamIdentifier"] == main[2]["TeamIdentifier"]]
            if not candidates:
                break
            matched.append(max(candidates, key=lambda item: item[2]["ExpirationDate"]))
        if len(matched) == len(bundles) and all(cached["certificate"] in item[2]["DeveloperCertificates"] for item in matched):
            selected = matched, cached
            break
    if selected is None:
        print("Current device profiles are needed for Orbit AND its Share Extension.", file=sys.stderr)
        return 75

    selected, cached = selected
    requested = plistlib.loads(entitlements_path.read_bytes())["keychain-access-groups"]
    groups = [group.replace("$(AppIdentifierPrefix)", selected[0][1]) for group in requested]
    if any("$(" in group or "*" in group for group in groups):
        raise ValueError("The shared keychain groups must resolve to explicit group identifiers")
    for _, _, profile, _ in selected:
        allowed = profile["Entitlements"].get("keychain-access-groups", [])
        if not all(any(fnmatch.fnmatchcase(group, pattern) for pattern in allowed) for group in groups):
            raise ValueError("The provisioning profiles do not permit the shared keychain group")

    destination = stage / "Payload" / app.name
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(app, destination)
    for bundle, (identifier, _, profile, raw) in zip(bundles, selected, strict=True):
        target = destination / bundle.relative_to(app)
        info_path = target / "Info.plist"
        info = plistlib.loads(info_path.read_bytes())
        info["CFBundleIdentifier"] = identifier
        info_path.write_bytes(plistlib.dumps(info))
        (target / "embedded.mobileprovision").write_bytes(raw)
        entitlements = dict(profile["Entitlements"])
        # First group is the default on both targets. xtool otherwise replaces
        # this with each bundle's own group, which prevents credential sharing.
        entitlements["keychain-access-groups"] = groups
        (stage / (target.stem + ".entitlements")).write_bytes(plistlib.dumps(entitlements))
    for filename, data in [("key.pem", cached["privateKey"]["data"]), ("cert.der", cached["certificate"])]:
        path = stage / filename
        path.write_bytes(data)
        path.chmod(0o600)
    (stage / "team").write_text(selected[0][2]["TeamIdentifier"][0])
    print("Validated both profiles, device, expiry, certificate and shared keychain permissions.")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 7:
        sys.exit("usage: prepare-device-signing.py APP STAGE PROFILES XTOOL_DATA ENTITLEMENTS UDID")
    try:
        sys.exit(prepare(*(Path(value) for value in sys.argv[1:6]), sys.argv[6]))
    except (OSError, ValueError, KeyError, plistlib.InvalidFileException):
        # Never include a credential-containing plist or private key in errors.
        sys.exit("Could not prepare device signing. Check the profiles and cached xtool signing identity.")
