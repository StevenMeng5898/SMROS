#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
"$ROOT_DIR/scripts/verus-recorded-run.sh" \
    "$ROOT_DIR/verification/logs/verus-user-level.log" \
    verification/user_level/src/lib.rs
