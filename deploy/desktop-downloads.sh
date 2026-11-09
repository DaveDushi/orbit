#!/usr/bin/env bash
# Copies the Windows installer, the zip and latest.json of a "Desktop" workflow run (.github/workflows/desktop.yml)
# to the folder that nginx serves as https://<server>/downloads/desktop/ (the download link and the app's update
# check). The folder is outside the deploy, so deploys keep it. Old Orbit-* files are removed after the copy.
#
#   deploy/desktop-downloads.sh [<run id>]     # default: the newest successful Desktop run
#
# nginx (in the server block of /etc/nginx/conf.d/orbit.conf, before `location /`):
#   location /downloads/desktop/ {
#       alias /srv/orbit-downloads/desktop/;
#       autoindex on;
#       types { application/json json; }
#       default_type application/octet-stream;
#       add_header Cache-Control "no-cache" always;
#       access_log off;
#   }
set -euo pipefail

gh=${GH:-/usr/local/bin/gh}
repo=${REPO:-DaveDushi/orbit}
dest=${DEST:-/srv/orbit-downloads/desktop}

run=${1:-$("$gh" run list -R "$repo" -w Desktop -s success -L 1 --json databaseId -q '.[0].databaseId')}
[[ -n "$run" ]] || { echo "desktop-downloads: no successful Desktop run in $repo" >&2; exit 1; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
"$gh" run download "$run" -R "$repo" -n desktop-windows -n desktop-latest-json -D "$tmp"
latest="$tmp/desktop-latest-json/latest.json"
files="$tmp/desktop-windows"

# Every file latest.json names must be here, with its sha256.
version=$(jq -er .version "$latest")
for key in windows windows_zip; do
  name=$(basename "$(jq -er ".$key" "$latest")")
  want=$(jq -er ".sha256.$key" "$latest")
  got=$(sha256sum "$files/$name" | cut -d' ' -f1)
  [[ "$got" == "$want" ]] || { echo "desktop-downloads: $name: sha256 $got, latest.json says $want" >&2; exit 1; }
done

install -d -m 755 "$dest"
install -m 644 "$files"/Orbit-* "$dest/"
# latest.json last, so the update check never names a file that is not there yet.
install -m 644 "$latest" "$dest/latest.json.tmp"
mv -f "$dest/latest.json.tmp" "$dest/latest.json"
for old in "$dest"/Orbit-*; do
  [[ -e "$files/$(basename "$old")" ]] || rm -f "$old"
done

echo "desktop-downloads: Orbit $version from run $run in $dest:"
ls -l "$dest"
