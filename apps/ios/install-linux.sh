#!/usr/bin/env bash
# Install the app + Share Extension without losing their shared keychain group.
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")"
udid=${1:?usage: install-linux.sh IPHONE_UDID}
installer=${XTOOL_INSTALLER:-xtool-1.22.2}
device_tool=${PYMOBILEDEVICE3:-pymobiledevice3}
for tool in "$device_tool" rcodesign python3 zip; do
    command -v "$tool" >/dev/null || { echo "Required tool not found: $tool" >&2; exit 1; }
done
[ -d xtool/Orbit.app/PlugIns/OrbitShare.appex ] || {
    echo "Run ./build-linux.sh first (the Share Extension must be included)." >&2; exit 1;
}
umask 077
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir "$stage/profiles"
"$device_tool" provision dump "$stage/profiles" --udid "$udid"
prepare() {
    python3 tools/prepare-device-signing.py xtool/Orbit.app "$stage" "$stage/profiles" \
        "${XTOOL_DATA_DIR:-$HOME/.config/xtool/data}" Config/Sharing.entitlements "$udid"
}
if prepare; then
    :
else
    result=$?
    [ "$result" -eq 75 ] || exit "$result"
    command -v "$installer" >/dev/null || { echo "Install xtool 1.22.2 and sign in privately first." >&2; exit 1; }
    # Provision new/expired extension profiles with the fixed authenticator. It
    # also installs once, but overwrites shared keychain groups. Re-sign below
    # before launching. Never use the older builder for authentication/signing.
    "$installer" install --usb --udid "$udid" "$(pwd)/xtool/Orbit.app"
    "$device_tool" provision dump "$stage/profiles" --udid "$udid"
    prepare
fi
app="$stage/Payload/Orbit.app"
team=$(<"$stage/team")
[[ "$team" =~ ^[A-Z0-9]{10}$ ]] || { echo "Invalid signing team" >&2; exit 1; }
sign=(rcodesign sign --pem-file "$stage/key.pem" --certificate-der-file "$stage/cert.der" --team-name "$team" --timestamp-url none)
shopt -s nullglob
for framework in "$app"/Frameworks/*.framework "$app"/Frameworks/*.dylib; do
    "${sign[@]}" "$framework"
done
for extension in "$app"/PlugIns/*.appex; do
    "${sign[@]}" --entitlements-xml-file "$stage/$(basename "$extension" .appex).entitlements" "$extension"
done
"${sign[@]}" --entitlements-xml-file "$stage/Orbit.entitlements" "$app"
python3 tools/verify-device-signing.py "$stage"
(cd "$stage" && zip -qry Orbit.ipa Payload)
"$device_tool" apps install "$stage/Orbit.ipa" --udid "$udid"
echo "Installed Orbit and its Share Extension. Open Orbit once to refresh sharing sign-in, then use Photos > Share > Orbit."
