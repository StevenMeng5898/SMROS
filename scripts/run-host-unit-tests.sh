#!/usr/bin/env bash
# Run host-side Rust tests for pure SMROS helper logic and contracts.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"

if [ -z "$HOST_TARGET" ]; then
    echo "error: failed to detect rustc host target" >&2
    exit 1
fi

# Cargo discovers configuration from the invocation directory rather than the
# manifest directory. Run outside the repository so the kernel-only build-std
# configuration is not merged into this host build.
cd /

echo "Running SMROS host tests on $HOST_TARGET..."
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target/host-tests}"
export CARGO_TARGET_DIR

cargo test \
    --manifest-path "$REPO_ROOT/tests/host/Cargo.toml" \
    --target "$HOST_TARGET" \
    --target-dir "$CARGO_TARGET_DIR" \
    "$@"
