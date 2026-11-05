#!/usr/bin/env bash
# Installs the pinned scanners as prebuilt release binaries into ~/.cargo/bin when they are missing.
# No compile step, so this is light on the shared VM. Linux x86_64 only.
set -euo pipefail

bin_dir="$HOME/.cargo/bin"
mkdir -p "$bin_dir"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

CARGO_AUDIT=0.22.2
CARGO_DENY=0.20.2
GITLEAKS=8.30.1
OHA=1.16.0

# fetch <url> <file>
fetch() { curl -fsSL --retry 3 -o "$work/$2" "$1"; }

if ! command -v cargo-audit >/dev/null; then
    name="cargo-audit-x86_64-unknown-linux-musl-v$CARGO_AUDIT"
    fetch "https://github.com/rustsec/rustsec/releases/download/cargo-audit%2Fv$CARGO_AUDIT/$name.tgz" audit.tgz
    mkdir "$work/audit" && tar -xzf "$work/audit.tgz" -C "$work/audit"
    install -m 0755 "$(find "$work/audit" -type f -name cargo-audit | head -1)" "$bin_dir/cargo-audit"
fi

if ! command -v cargo-deny >/dev/null; then
    name="cargo-deny-$CARGO_DENY-x86_64-unknown-linux-musl"
    base="https://github.com/EmbarkStudios/cargo-deny/releases/download/$CARGO_DENY/$name.tar.gz"
    fetch "$base" deny.tgz
    fetch "$base.sha256" deny.sha256
    (cd "$work" && echo "$(awk '{print $1}' deny.sha256)  deny.tgz" | sha256sum -c --quiet)
    mkdir "$work/deny" && tar -xzf "$work/deny.tgz" -C "$work/deny"
    install -m 0755 "$(find "$work/deny" -type f -name cargo-deny | head -1)" "$bin_dir/cargo-deny"
fi

if ! command -v gitleaks >/dev/null; then
    name="gitleaks_${GITLEAKS}_linux_x64.tar.gz"
    fetch "https://github.com/gitleaks/gitleaks/releases/download/v$GITLEAKS/$name" "$name"
    fetch "https://github.com/gitleaks/gitleaks/releases/download/v$GITLEAKS/gitleaks_${GITLEAKS}_checksums.txt" sums
    (cd "$work" && grep " $name\$" sums | sha256sum -c --quiet)
    mkdir "$work/gitleaks" && tar -xzf "$work/$name" -C "$work/gitleaks"
    install -m 0755 "$work/gitleaks/gitleaks" "$bin_dir/gitleaks"
fi

if ! command -v oha >/dev/null; then
    fetch "https://github.com/hatoo/oha/releases/download/v$OHA/oha-linux-amd64" oha
    install -m 0755 "$work/oha" "$bin_dir/oha"
fi
