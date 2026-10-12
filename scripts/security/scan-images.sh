#!/usr/bin/env sh
# Scans images with grype and fails on HIGH or CRITICAL. Usage: scan-images.sh <image>...
# Accepted advisories live in security/exceptions.toml (see check-exceptions.mjs).
set -eu
command -v grype >/dev/null 2>&1 || { echo "install grype: https://github.com/anchore/grype" >&2; exit 1; }
node scripts/security/check-exceptions.mjs
for image in "$@"; do
  grype "$image" --fail-on high
done
