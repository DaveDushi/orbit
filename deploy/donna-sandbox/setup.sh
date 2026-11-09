#!/usr/bin/env bash
# Sets up Donna's media sandbox on a systemd host (Ubuntu 24.04). Safe to run again: it only makes what is missing
# or changed. Run as root from the repository root:
#
#   sudo deploy/donna-sandbox/setup.sh                 # production names, then: systemctl restart orbit-agent
#   sudo deploy/donna-sandbox/setup.sh --uninstall     # removes everything it made (all /work data too)
#
# It makes: the user donna-sandbox (no login, no home), orbit-donna.slice (all runs: 2 CPUs, 3 GiB), the socket
# /run/orbit-donna/sandbox.sock (only root and the group orbit-agent may connect), orbit-donna@.service (one run:
# 2 CPUs, 2 GiB, 256 tasks, 15 minutes at most, no network), the broker in /usr/local/lib/orbit-donna, the folder
# /var/lib/orbit-donna (one disk image per conversation, 2 GiB each), and a drop-in that gives orbit-agent.service
# AGENT_SANDBOX_SOCKET. Packages: bubblewrap, ffmpeg, imagemagick, ghostscript, poppler-utils, libimage-exiftool-perl.
#
# Options (for a test copy next to production, use other names for all of them):
#   --name NAME          unit and folder name (orbit-donna)
#   --user USER          the sandbox user (donna-sandbox)
#   --client-group GROUP who may connect to the socket (orbit-agent)
#   --agent-unit UNIT    the agent's service for the drop-in, or "none" (orbit-agent)
#   --quota-mb N         /work size of one conversation (2048)
#   --no-apt             do not install packages
#   --uninstall
set -euo pipefail

NAME=orbit-donna
SANDBOX_USER=donna-sandbox
CLIENT_GROUP=orbit-agent
AGENT_UNIT=orbit-agent
QUOTA_MB=2048
APT=1
UNINSTALL=0
while [ $# -gt 0 ]; do
	case "$1" in
	--name) NAME=$2; shift 2 ;;
	--user) SANDBOX_USER=$2; shift 2 ;;
	--client-group) CLIENT_GROUP=$2; shift 2 ;;
	--agent-unit) AGENT_UNIT=$2; shift 2 ;;
	--quota-mb) QUOTA_MB=$2; shift 2 ;;
	--no-apt) APT=0; shift ;;
	--uninstall) UNINSTALL=1; shift ;;
	*) echo "unknown option $1" >&2; exit 2 ;;
	esac
done
[[ $NAME =~ ^[a-z][a-z0-9-]{1,40}$ ]] || { echo "invalid --name" >&2; exit 2; }
[[ $SANDBOX_USER =~ ^[a-z][a-z0-9-]{1,31}$ ]] || { echo "invalid --user" >&2; exit 2; }
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }

HERE=$(cd "$(dirname "$0")" && pwd)
LIB=/usr/local/lib/$NAME
STATE=/var/lib/$NAME
SOCKET=/run/$NAME/sandbox.sock
UNITS=/etc/systemd/system
DROPIN=$UNITS/$AGENT_UNIT.service.d/$NAME.conf

if [ "$UNINSTALL" = 1 ]; then
	systemctl disable --now "$NAME.socket" 2>/dev/null || true
	systemctl stop "$NAME@*.service" 2>/dev/null || true
	rm -f "$UNITS/$NAME.socket" "$UNITS/$NAME@.service" "$UNITS/$NAME.slice"
	[ "$AGENT_UNIT" = none ] || rm -f "$DROPIN"
	rm -rf "$LIB" "$STATE"
	systemctl daemon-reload
	if id "$SANDBOX_USER" >/dev/null 2>&1; then userdel "$SANDBOX_USER"; fi
	getent group "$SANDBOX_USER" >/dev/null && groupdel "$SANDBOX_USER" || true
	echo "removed $NAME"
	exit 0
fi

if [ "$APT" = 1 ]; then
	missing=()
	for package in bubblewrap ffmpeg imagemagick ghostscript poppler-utils libimage-exiftool-perl e2fsprogs python3; do
		dpkg -s "$package" >/dev/null 2>&1 || missing+=("$package")
	done
	if [ ${#missing[@]} -gt 0 ]; then
		apt-get update -qq
		DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends "${missing[@]}"
	fi
fi
for tool in /usr/bin/bwrap /usr/bin/setpriv /usr/sbin/mkfs.ext4 /usr/bin/python3; do
	[ -x "$tool" ] || { echo "missing $tool" >&2; exit 1; }
done
getent group "$CLIENT_GROUP" >/dev/null || { echo "no group $CLIENT_GROUP" >&2; exit 1; }

if ! id "$SANDBOX_USER" >/dev/null 2>&1; then
	useradd --system --user-group --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin "$SANDBOX_USER"
fi

install -d -o root -g root -m 0755 "$LIB"
install -o root -g root -m 0755 "$HERE/broker.py" "$LIB/broker.py"
install -d -o root -g root -m 0700 "$STATE" "$STATE/work"

changed=0
# Writes $2 to $1 when it differs.
put() {
	if [ ! -f "$1" ] || [ "$(cat "$1")" != "$2" ]; then
		mkdir -p "$(dirname "$1")"
		printf '%s\n' "$2" >"$1"
		chmod 0644 "$1"
		changed=1
	fi
}

put "$UNITS/$NAME.slice" "[Unit]
Description=Donna media sandbox runs (all together)

[Slice]
CPUQuota=200%
MemoryAccounting=yes
MemoryHigh=2560M
MemoryMax=3G
CPUWeight=20
IOWeight=20
TasksMax=1024"

put "$UNITS/$NAME.socket" "[Unit]
Description=Donna media sandbox requests

[Socket]
ListenStream=$SOCKET
SocketUser=root
SocketGroup=$CLIENT_GROUP
SocketMode=0660
DirectoryMode=0755
Accept=yes
MaxConnections=8
RemoveOnStop=yes

[Install]
WantedBy=sockets.target"

put "$UNITS/$NAME@.service" "[Unit]
Description=Donna media sandbox run
CollectMode=inactive-or-failed

[Service]
Type=simple
ExecStart=/usr/bin/python3 -I $LIB/broker.py
StandardInput=socket
StandardOutput=socket
StandardError=journal
Environment=DONNA_USER=$SANDBOX_USER DONNA_STATE=$STATE DONNA_QUOTA_MB=$QUOTA_MB DONNA_MAX_TIMEOUT=600
Slice=$NAME.slice
CPUQuota=200%
MemoryMax=2G
MemorySwapMax=0
TasksMax=256
RuntimeMaxSec=900
LimitCORE=0
UMask=0077
NoNewPrivileges=yes
PrivateNetwork=yes
PrivateTmp=yes
ProtectSystem=strict
ReadWritePaths=$STATE
ProtectHome=yes
InaccessiblePaths=-/opt -/srv -/etc/orbit -/var/lib/orbit -/var/lib/orbit-agent -/var/backups
KeyringMode=private"
# No ProtectKernelModules, ProtectKernelLogs or ProtectClock: they mask parts of /proc, and then the kernel does not
# let bwrap mount the sandbox's own /proc.

if [ "$AGENT_UNIT" != none ]; then
	put "$DROPIN" "[Service]
Environment=AGENT_SANDBOX_SOCKET=$SOCKET"
fi

if [ "$changed" = 1 ]; then systemctl daemon-reload; fi
systemctl enable --now "$NAME.socket" >/dev/null
systemctl start "$NAME.slice"
echo "$NAME is ready: socket $SOCKET, user $SANDBOX_USER, /work images in $STATE/work."
if [ "$AGENT_UNIT" != none ] && [ "$changed" = 1 ]; then
	echo "Restart $AGENT_UNIT.service so the agent gets AGENT_SANDBOX_SOCKET."
fi
