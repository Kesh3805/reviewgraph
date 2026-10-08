#!/usr/bin/env bash
# Run cargo inside the Linux engine build container (ADR-001).
#   engine/scripts/cargo.sh test --workspace
set -euo pipefail
exec "$(dirname "${BASH_SOURCE[0]}")/run.sh" cargo "$@"
