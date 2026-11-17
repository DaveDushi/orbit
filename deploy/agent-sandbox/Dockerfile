# The agent sandbox image (docs/agent-sandbox-design.md, sections 2 and 11, question 9). It is never run by Docker:
# setup.sh exports it to a read-only rootfs for gVisor. No secrets; the base is pinned by digest.
# The broker sets the environment (an exported rootfs has no ENV); installs go to /work (pip --user, npm --prefix).
# Media tools (ffmpeg, ImageMagick) come with the media_shell merge (phase 11); media_shell keeps its own sandbox.
FROM ubuntu:24.04@sha256:73b999393d07f2bf65a40373778a44dc9a6d54ad7718d29247d8ff9413c4d6a8

RUN apt-get update \
	&& DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
		tini ca-certificates python3 python3-venv nodejs npm git curl jq ripgrep procps less \
	&& rm -rf /var/lib/apt/lists/* /var/cache/apt/* /usr/share/doc/* /usr/share/man/* \
	&& (userdel --remove ubuntu 2>/dev/null || true) \
	&& groupadd --gid 1000 agent && useradd --uid 1000 --gid 1000 --home-dir /work --no-create-home --shell /bin/bash agent \
	&& mkdir -p /work && rm -f /etc/machine-id

# No setuid or setgid programs: the sandbox user never changes its identity (the spec also sets noNewPrivileges).
RUN find / -xdev -perm /6000 -type f -exec chmod a-s {} +
