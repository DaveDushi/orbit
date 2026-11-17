#!/usr/bin/env bash
# Sets up the agent sandbox broker (broker.py in this folder; docs/agent-sandbox-design.md) on a systemd host. Safe to
# run again. Run as root from the repository root. Phase 3: TEST COPIES ONLY; production needs the owner's yes.
#
#   sudo deploy/agent-sandbox/setup.sh --name orbit-sbx-test --slice-prefix orbit-sbx-test --net 10.202 \
#     --net-prefix sbt --proxy-port 13128 --quota-mb 256 --min-free-mb 4096 --client-group root \
#     --image orbit-sbx-test-image:p3
#   sudo deploy/agent-sandbox/setup.sh --name orbit-sbx-test --slice-prefix orbit-sbx-test --uninstall
#
# It makes: gVisor (pinned release, SHA-512 checked) in /usr/local/lib/NAME/gvisor, the image's read-only rootfs in
# /var/lib/NAME/rootfs (exported from a local Docker image built from the Dockerfile here), the broker key, the socket
# /run/NAME/sandbox.sock (root and --client-group), NAME@.service (one broker per connection), NAME-gc.timer (gc every
# minute), and the per-agent slice limits (drop-in SLICE-.slice.d). With a test --slice-prefix it also makes the parent
# slice with the limits of orbit-agents.slice. Starting or stopping the socket deletes every live sandbox.
#
# Options:
#   --name NAME            units, folders, scopes, nft table (orbit-sandbox)
#   --slice-prefix PREFIX  parent slice PREFIX.slice; per agent PREFIX-<agent>.slice (orbit-agents)
#   --net A.B              thread /30s in A.B.0.0/16 (10.201)
#   --net-prefix P         netns P-<work>, veth Ph<n>/Pp<n> (osb)
#   --proxy-port N         egress proxy port on each /30's host address (3128)
#   --quota-mb N           /work size per thread (4096);  --min-free-mb N  refuse new images below (8192)
#   --image REF            local Docker image of the Dockerfile here (orbit-agent-sandbox:latest)
#   --client-group GROUP   who may connect (orbit-agent);  --key PATH  (/etc/orbit/sandbox-broker.key)
#   --orbit-url URL        where call ends are reported (none)
#   --tarball PATH         a local copy of the pinned gVisor tarball (else it is downloaded)
#   --uninstall            removes all of it, /work images included (not the key under /etc/orbit)
set -euo pipefail

GVISOR_VERSION=20261109.0
GVISOR_SHA512=0a0041fd862f4ea3042a9db8d41c742f48a01f909699adc2a0256e46d85e53f97b82b9fd154b3dce636f62b8d557677639f91c77d13f3f875ff9fa535275393c
NAME=orbit-sandbox SLICE_PREFIX=orbit-agents NET=10.201 NET_PREFIX=osb PROXY_PORT=3128 QUOTA_MB=4096
MIN_FREE_MB=8192 IMAGE=orbit-agent-sandbox:latest CLIENT_GROUP=orbit-agent KEY=/etc/orbit/sandbox-broker.key
ORBIT_URL="" TARBALL="" UNINSTALL=0
while [ $# -gt 0 ]; do
	case "$1" in
	--name) NAME=$2; shift 2 ;;
	--slice-prefix) SLICE_PREFIX=$2; shift 2 ;;
	--net) NET=$2; shift 2 ;;
	--net-prefix) NET_PREFIX=$2; shift 2 ;;
	--proxy-port) PROXY_PORT=$2; shift 2 ;;
	--quota-mb) QUOTA_MB=$2; shift 2 ;;
	--min-free-mb) MIN_FREE_MB=$2; shift 2 ;;
	--image) IMAGE=$2; shift 2 ;;
	--client-group) CLIENT_GROUP=$2; shift 2 ;;
	--key) KEY=$2; shift 2 ;;
	--orbit-url) ORBIT_URL=$2; shift 2 ;;
	--tarball) TARBALL=$2; shift 2 ;;
	--uninstall) UNINSTALL=1; shift ;;
	*) echo "unknown option $1" >&2; exit 2 ;;
	esac
done
[[ $NAME =~ ^[a-z][a-z0-9-]{1,30}$ ]] || { echo "invalid --name" >&2; exit 2; }
[[ $SLICE_PREFIX =~ ^[a-z][a-z0-9-]{1,30}$ ]] || { echo "invalid --slice-prefix" >&2; exit 2; }
[[ $NET =~ ^10\.[0-9]{1,3}$ ]] || { echo "invalid --net" >&2; exit 2; }
[[ $NET_PREFIX =~ ^[a-z]{2,6}$ ]] || { echo "invalid --net-prefix" >&2; exit 2; }
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }

HERE=$(cd "$(dirname "$0")" && pwd)
LIB=/usr/local/lib/$NAME
STATE=/var/lib/$NAME
CONF=/etc/$NAME
UNITS=/etc/systemd/system
DROPIN=$UNITS/$SLICE_PREFIX-.slice.d/50-agent.conf

if [ "$UNINSTALL" = 1 ]; then
	systemctl disable --now "$NAME-gc.timer" "$NAME.socket" 2>/dev/null || true  # the socket's stop deletes sandboxes
	systemctl stop "$NAME@*.service" "$NAME-gc.service" 2>/dev/null || true
	for f in "$NAME.socket" "$NAME@.service" "$NAME-gc.service" "$NAME-gc.timer"; do rm -f "$UNITS/$f"; done
	rm -f "$DROPIN"; rmdir "$(dirname "$DROPIN")" 2>/dev/null || true
	if [ -f "$CONF/made-parent-slice" ]; then
		systemctl stop "$SLICE_PREFIX.slice" 2>/dev/null || true
		rm -f "$UNITS/$SLICE_PREFIX.slice"
	fi
	nft delete table inet "${NAME//-/_}" 2>/dev/null || true
	for m in /run/$NAME/mnt/* /run/$NAME/runsc/null-netns; do mountpoint -q "$m" 2>/dev/null && umount -l "$m"; done
	rm -rf "$LIB" "$STATE" "$CONF" "/run/$NAME"
	case "$KEY" in "$CONF"/*) ;; *) echo "kept the key $KEY" ;; esac
	systemctl daemon-reload
	echo "removed $NAME"
	exit 0
fi

for tool in /usr/sbin/mkfs.ext4 /usr/bin/python3 /usr/sbin/nft /usr/sbin/ip /usr/bin/zstd; do
	[ -x "$tool" ] || { echo "missing $tool" >&2; exit 1; }
done
getent group "$CLIENT_GROUP" >/dev/null || { echo "no group $CLIENT_GROUP" >&2; exit 1; }

# gVisor: the whole release tarball (runsc alone no longer works), checked against the pinned SHA-512.
if [ "$("$LIB/gvisor/runsc" --version 2>/dev/null | head -1)" != "runsc version release-$GVISOR_VERSION" ]; then
	download=$(mktemp -d)
	if [ -z "$TARBALL" ]; then
		TARBALL=$download/gvisor.tar.zstd
		curl -fsSL -o "$TARBALL" "https://storage.googleapis.com/gvisor/releases/release/$GVISOR_VERSION/x86_64/gvisor.tar.zstd"
	fi
	echo "$GVISOR_SHA512  $TARBALL" | sha512sum -c --quiet - || { echo "gVisor tarball: wrong SHA-512" >&2; exit 1; }
	rm -rf "$LIB/gvisor"
	install -d -m 0755 "$LIB/gvisor"
	tar --zstd -xf "$TARBALL" -C "$LIB/gvisor" --no-same-owner
	rm -f "$LIB/gvisor/containerd-shim-runsc-v1"
	rm -rf "$download"
fi
install -o root -g root -m 0755 "$HERE/broker.py" "$LIB/broker.py"
install -d -o root -g root -m 0700 "$STATE" "$STATE/agents" "$STATE/rootfs" "$CONF"
[ -d "$(dirname "$KEY")" ] || install -d -o root -g root -m 0700 "$(dirname "$KEY")"

if [ ! -s "$KEY" ]; then
	(umask 077; head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' >"$KEY")
fi
chmod 0600 "$KEY"

# The rootfs: exported once per image id; `current` points to the pinned one.
image_id=$(docker image inspect --format '{{.Id}}' "$IMAGE")
rootfs=$STATE/rootfs/${image_id#sha256:}
if [ ! -d "$rootfs" ]; then
	container=$(docker create "$IMAGE" /bin/true)
	mkdir -p "$rootfs.new"
	docker export "$container" | tar -x -C "$rootfs.new" --numeric-owner
	docker rm "$container" >/dev/null
	mv "$rootfs.new" "$rootfs"
fi
ln -sfn "$rootfs" "$STATE/rootfs/current"
echo "$image_id" >"$STATE/rootfs/current.id"

changed=0
put() {
	if [ ! -f "$1" ] || [ "$(cat "$1")" != "$2" ]; then
		mkdir -p "$(dirname "$1")"
		printf '%s\n' "$2" >"$1"
		chmod 0644 "$1"
		changed=1
	fi
}

put "$CONF/broker.env" "SBX_NAME=$NAME
SBX_SLICE_PREFIX=$SLICE_PREFIX
SBX_NET=$NET
SBX_NET_PREFIX=$NET_PREFIX
SBX_PROXY_PORT=$PROXY_PORT
SBX_QUOTA_MB=$QUOTA_MB
SBX_MIN_FREE_MB=$MIN_FREE_MB
SBX_KEY_FILE=$KEY
SBX_ORBIT_URL=$ORBIT_URL"

if [ ! -f "$UNITS/$SLICE_PREFIX.slice" ]; then
	put "$UNITS/$SLICE_PREFIX.slice" "# Made by deploy/agent-sandbox/setup.sh --name $NAME: the limits of orbit-agents.slice.
[Unit]
Description=Agent sandboxes ($NAME)

[Slice]
CPUQuota=150%
CPUWeight=20
MemoryAccounting=yes
MemoryHigh=2G
MemoryMax=2560M
MemorySwapMax=0
TasksMax=1536
IOWeight=20"
	touch "$CONF/made-parent-slice"
fi

put "$DROPIN" "# One agent's sandboxes together (docs/agent-sandbox-design.md, section 6).
[Slice]
CPUQuota=100%
MemoryMax=1536M
MemorySwapMax=0
TasksMax=1536"

put "$UNITS/$NAME.socket" "[Unit]
Description=Agent sandbox requests ($NAME)

[Socket]
ListenStream=/run/$NAME/sandbox.sock
SocketUser=root
SocketGroup=$CLIENT_GROUP
SocketMode=0660
DirectoryMode=0755
Accept=yes
MaxConnections=16
RemoveOnStop=yes
EnvironmentFile=$CONF/broker.env
# A new broker cannot trust old state; a stop is the operator's off switch. Both delete every sandbox.
ExecStartPre=/usr/bin/python3 -I $LIB/broker.py cleanup
ExecStopPost=/usr/bin/python3 -I $LIB/broker.py cleanup

[Install]
WantedBy=sockets.target"

# No mount namespacing (PrivateTmp, Protect*): the loop mounts and /run/netns must be the host's, so every broker
# copy and the cleanup see them.
put "$UNITS/$NAME@.service" "[Unit]
Description=Agent sandbox broker ($NAME)
CollectMode=inactive-or-failed

[Service]
Type=simple
ExecStart=/usr/bin/python3 -I $LIB/broker.py
StandardInput=socket
StandardOutput=socket
StandardError=journal
EnvironmentFile=$CONF/broker.env
Slice=$SLICE_PREFIX.slice
RuntimeMaxSec=1000
TimeoutStopSec=20
NoNewPrivileges=yes
LimitCORE=0
UMask=0077"

put "$UNITS/$NAME-gc.service" "[Unit]
Description=Agent sandbox gc ($NAME): idle and max-life sandboxes, idle images

[Service]
Type=oneshot
ExecStart=/usr/bin/python3 -I $LIB/broker.py gc
EnvironmentFile=$CONF/broker.env
Slice=$SLICE_PREFIX.slice"

put "$UNITS/$NAME-gc.timer" "[Unit]
Description=Agent sandbox gc every minute ($NAME)

[Timer]
OnActiveSec=1min
OnUnitActiveSec=1min
AccuracySec=5s

[Install]
WantedBy=timers.target"

if [ "$changed" = 1 ]; then systemctl daemon-reload; fi
systemctl enable --now "$NAME.socket" "$NAME-gc.timer" >/dev/null
echo "$NAME is ready: socket /run/$NAME/sandbox.sock, rootfs ${image_id:7:12}, key $KEY."
