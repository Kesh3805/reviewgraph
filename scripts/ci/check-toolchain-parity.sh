#!/usr/bin/env bash
# ci_toolchain_matches_dev_image: the pinned toolchain, the active rustc and the dev image agree.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
pinned=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$ROOT/engine/rust-toolchain.toml")
image=$(sed -n 's/.*rustup toolchain install \([0-9.]*\).*/\1/p' "$ROOT/engine/docker/dev.Dockerfile")
active=$(cd "$ROOT/engine" && rustc --version | awk '{print $2}')
echo "pinned=$pinned image=$image active=$active"
[ "$pinned" = "$image" ] || { echo "dev image installs $image but rust-toolchain.toml pins $pinned"; exit 1; }
case "$active" in
  "$pinned"|"$pinned".*) ;;
  *) echo "active rustc $active does not match pinned $pinned"; exit 1 ;;
esac
