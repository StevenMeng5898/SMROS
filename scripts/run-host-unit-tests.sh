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

# Parallel test compilation only. Do not apply the kernel crate's
# codegen-units override; tests/host keeps its own Cargo profile.
SMROS_BUILD_JOBS="${SMROS_BUILD_JOBS:-4}"
case "$SMROS_BUILD_JOBS" in
    ''|*[!0-9]*|0*)
        echo "SMROS_BUILD_JOBS must be a positive integer" >&2
        exit 1
        ;;
esac
unset CARGO_PROFILE_RELEASE_CODEGEN_UNITS
unset CARGO_PROFILE_DEV_CODEGEN_UNITS

cargo test \
    -j "$SMROS_BUILD_JOBS" \
    --manifest-path "$REPO_ROOT/tests/host/Cargo.toml" \
    --target "$HOST_TARGET" \
    --target-dir "$CARGO_TARGET_DIR" \
    "$@"
