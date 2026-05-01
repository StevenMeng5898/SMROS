#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "$ROOT_DIR/verification/verus-toolchain.env"

"$ROOT_DIR/scripts/setup-verus.sh" >/dev/null

export PATH="$ROOT_DIR/.tools/verus/current:$PATH"
export RUSTUP_TOOLCHAIN="$VERUS_RUST_TOOLCHAIN"

tmp_dir="$(mktemp -d)"
cleanup() {
    rm -rf "$tmp_dir"
}
trap cleanup EXIT

(
    cd "$tmp_dir"
    verus \
        "$ROOT_DIR/verification/user_level/src/lib.rs" \
        --crate-type=lib \
        --triggers-mode=silent \
        --no-report-long-running
)
