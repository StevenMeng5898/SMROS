#!/usr/bin/env bash
# Run host-side unit tests for pure SMROS helper logic.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"

if [ -z "$HOST_TARGET" ]; then
    echo "error: failed to detect rustc host target" >&2
    exit 1
fi

cd "$REPO_ROOT"

echo "Running SMROS host unit tests on $HOST_TARGET..."
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target/host-tests}"
export CARGO_TARGET_DIR

cargo test \
    --manifest-path tests/host/Cargo.toml \
    --target "$HOST_TARGET" \
    --target-dir "$CARGO_TARGET_DIR" \
    --config 'unstable.build-std=[]' \
    "$@"
