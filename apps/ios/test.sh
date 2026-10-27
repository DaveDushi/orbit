#!/usr/bin/env bash
# Foundation-only connection, sharing and delivery tests; no simulator required.
set -euo pipefail
cd "$(dirname "$0")"
build=$(mktemp -d)
trap 'rm -rf "$build"' EXIT
swiftc Orbit/ServerAddress.swift Shared/*.swift Tests/ShareTests.swift Tests/main.swift -o "$build/ios-tests"
"$build/ios-tests"
PYTHONDONTWRITEBYTECODE=1 python3 Tests/test_signing.py
