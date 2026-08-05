#!/usr/bin/env bash
# Generate cargo-tarpaulin HTML coverage for the host-side SMROS test crate.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
HOST_TARGET="$(rustc -vV | sed -n 's/^host: //p')"
MODE="${1:-host}"

if [ -z "$HOST_TARGET" ]; then
    echo "error: failed to detect rustc host target" >&2
    exit 1
fi

# Cargo discovers configuration from the invocation directory rather than the
# manifest directory. Run outside the repository so the kernel-only build-std
# configuration is not merged into this host build.
cd /

if ! cargo tarpaulin --version >/dev/null 2>&1; then
    echo "error: cargo-tarpaulin is not installed" >&2
    echo "hint: cargo install --locked cargo-tarpaulin" >&2
    exit 1
fi

shift || true

COVERAGE_ROOT="${SMROS_COVERAGE_DIR:-$REPO_ROOT/target/coverage}"
TARPAULIN_TARGET_ROOT="${SMROS_TARPAULIN_TARGET_DIR:-$REPO_ROOT/target/tarpaulin}"
TARPAULIN_ENGINE="${SMROS_TARPAULIN_ENGINE:-llvm}"
TARPAULIN_TIMEOUT="${SMROS_TARPAULIN_TIMEOUT:-120}"
TARPAULIN_FAIL_UNDER="${SMROS_TARPAULIN_FAIL_UNDER:-100}"
TARGET_ARGS=()

case "$MODE" in
    ut)
        OUTPUT_DIR="$COVERAGE_ROOT/ut"
        TARGET_ARGS=(--lib)
        DESCRIPTION="host unit tests"
        ;;
    it)
        OUTPUT_DIR="$COVERAGE_ROOT/it"
        TARGET_ARGS=(--test integration_contracts)
        DESCRIPTION="host integration tests"
        ;;
    host|all)
        OUTPUT_DIR="$COVERAGE_ROOT/host"
        DESCRIPTION="host unit and integration tests"
        ;;
    *)
        echo "usage: $0 [ut|it|host] [extra cargo-tarpaulin args...]" >&2
        exit 2
        ;;
esac

mkdir -p "$OUTPUT_DIR" "$TARPAULIN_TARGET_ROOT"

echo "Generating Tarpaulin HTML coverage for $DESCRIPTION on $HOST_TARGET..."
cargo tarpaulin \
    --manifest-path "$REPO_ROOT/tests/host/Cargo.toml" \
    --target "$HOST_TARGET" \
    --target-dir "$TARPAULIN_TARGET_ROOT/$MODE" \
    --engine "$TARPAULIN_ENGINE" \
    --timeout "$TARPAULIN_TIMEOUT" \
    --fail-under "$TARPAULIN_FAIL_UNDER" \
    --include-tests \
    --out Html \
    --output-dir "$OUTPUT_DIR" \
    --skip-clean \
    "${TARGET_ARGS[@]}" \
    "$@"

REPORT="$OUTPUT_DIR/tarpaulin-report.html"
if [ -f "$REPORT" ]; then
    echo "Tarpaulin HTML report: $REPORT"
else
    echo "warning: expected Tarpaulin HTML report was not found at $REPORT" >&2
fi
