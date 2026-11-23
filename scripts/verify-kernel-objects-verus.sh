#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
"$ROOT_DIR/scripts/verus-recorded-run.sh" \
    "$ROOT_DIR/verification/logs/verus-kernel-objects.log" \
    verification/kernel_objects/src/lib.rs
