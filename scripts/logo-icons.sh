#!/usr/bin/env bash
# Makes every copy of the Orbit mark from its one source, apps/web/public/logo.svg (the pixel "W" on an 11x11
# grid): the web favicons, the PWA and Apple icons, the desktop tray PNG and the Windows .ico (exe, installer).
# The mark sits centered at whole pixels per grid unit, so edges stay crisp, on a transparent background (RGBA).
# Only the Apple touch icon keeps an opaque black square: iOS shows transparent pixels as black anyway.
# (No icon in manifest.webmanifest is "maskable"; a maskable one would need an opaque background too.)
# Needs ImageMagick (`convert`). Run from anywhere: scripts/logo-icons.sh
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
public="$root/apps/web/public"
desktop="$root/apps/desktop"
source_svg="$public/logo.svg"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# The two paths of the mark: the shadow, then the pink.
shadow=$(sed -n 's/.*fill="#742f4d" d="\([^"]*\)".*/\1/p' "$source_svg")
pink=$(sed -n 's/.*fill="#f2458f" d="\([^"]*\)".*/\1/p' "$source_svg")
[[ -n "$shadow" && -n "$pink" ]] || { echo "logo-icons: cannot read the two paths of $source_svg" >&2; exit 1; }

head='<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" viewBox="0 0 11 11" shape-rendering="crispEdges">'
mark="<path fill=\"#742f4d\" d=\"$shadow\"/><path fill=\"#f2458f\" d=\"$pink\"/>"
printf '%s%s</svg>\n' "$head" "$mark" > "$public/favicon.svg"
# Unread: the bottom-right corner is cut away and a blue square sits in it (lib/favicon.ts).
printf '%s<mask id="cut"><path fill="#fff" d="M0 0h11v11H0z"/><path d="M6 6h5v5H6z"/></mask><g mask="url(#cut)">%s</g><path fill="#38a8ff" d="M7 7h4v4H7z"/></svg>\n' \
  "$head" "$mark" > "$public/favicon-unread.svg"

# The mark at 1 pixel per grid unit (11x11), transparent around it: the center of each unit of the 256px render,
# so no blended edge pixels.
convert -background none "$source_svg" -filter point -resize '11x11!' -depth 8 "$tmp/mark.png"

# icon <size> <pixels per unit> <out>: transparent square, mark centered, 8-bit RGBA.
icon() {
  convert "$tmp/mark.png" -sample "$(( 11 * $2 ))x$(( 11 * $2 ))" -background none -gravity center \
    -extent "$1x$1" -depth 8 -strip "PNG32:$3"
}

# opaque_icon <size> <pixels per unit> <out>: the same on a black square, no alpha channel.
opaque_icon() {
  convert "$tmp/mark.png" -sample "$(( 11 * $2 ))x$(( 11 * $2 ))" -background black -gravity center \
    -extent "$1x$1" -flatten -alpha off -depth 8 -strip "PNG24:$3"
}

opaque_icon 180 10 "$public/apple-touch-icon.png"
icon 192 10 "$public/icon-192.png"
icon 512 28 "$public/icon-512.png"
icon 64 4 "$desktop/crates/orbit-desktop/assets/orbit-64.png"

for spec in 16:1 24:2 32:2 48:4 64:4 256:14; do
  icon "${spec%%:*}" "${spec##*:}" "$tmp/ico-${spec%%:*}.png"
done
convert "$tmp"/ico-{16,24,32,48,64,256}.png "$desktop/packaging/windows/orbit.ico"

echo "logo-icons: wrote the favicons, app icons, orbit-64.png and orbit.ico from $source_svg"
