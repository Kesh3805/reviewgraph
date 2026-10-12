#!/usr/bin/env sh
# Generates CycloneDX SBOMs into artifacts/sbom (not committed). Requires cargo-cyclonedx and pnpm.
set -eu
out="artifacts/sbom"
mkdir -p "$out"
command -v cargo-cyclonedx >/dev/null 2>&1 || { echo "install: cargo install cargo-cyclonedx" >&2; exit 1; }
(cd engine && cargo cyclonedx --format json --override-filename rust-workspace)
find engine -name 'rust-workspace.json' -exec cp {} "$out/" \; -quit
pnpm dlx @cyclonedx/cyclonedx-npm --output-file "$out/node.json" --omit dev
