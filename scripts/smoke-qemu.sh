#!/usr/bin/env bash
# Boot SMROS in QEMU long enough to prove that the kernel reaches the shell.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

KERNEL_IMAGE="${KERNEL_IMAGE:-$REPO_ROOT/kernel8.img}"
FXFS_DISK="${FXFS_DISK:-$REPO_ROOT/smros-fxfs.img}"
FXFS_DISK_SIZE="${FXFS_DISK_SIZE:-128M}"
QEMU_MACHINE="${QEMU_MACHINE:-virt,gic-version=4,virtualization=on}"
QEMU_CPU="${QEMU_CPU:-cortex-a710}"
QEMU_SMP="${QEMU_SMP:-4}"
QEMU_MEMORY="${QEMU_MEMORY:-512M}"
SMROS_ST_TIMEOUT="${SMROS_ST_TIMEOUT:-45}"
SMROS_ST_LOG="${SMROS_ST_LOG:-$REPO_ROOT/target/smros-smoke-qemu.log}"
SMROS_ST_PROMPT="${SMROS_ST_PROMPT:-smros:/>}"

if ! command -v qemu-system-aarch64 >/dev/null 2>&1; then
    echo "error: qemu-system-aarch64 not found" >&2
    exit 1
fi

if ! command -v qemu-img >/dev/null 2>&1; then
    echo "error: qemu-img not found" >&2
    exit 1
fi

if [ ! -f "$KERNEL_IMAGE" ]; then
    echo "error: kernel image not found: $KERNEL_IMAGE" >&2
    echo "hint: run make build first" >&2
    exit 1
fi

mkdir -p "$(dirname "$SMROS_ST_LOG")"

if [ ! -f "$FXFS_DISK" ]; then
    echo "Creating persistent FxFS disk image: $FXFS_DISK"
    qemu-img create -f raw "$FXFS_DISK" "$FXFS_DISK_SIZE" >/dev/null
fi

rm -f "$SMROS_ST_LOG"
echo "Booting SMROS smoke test for up to ${SMROS_ST_TIMEOUT}s..."

qemu_pid=""
cleanup() {
    if [ -n "$qemu_pid" ] && kill -0 "$qemu_pid" >/dev/null 2>&1; then
        kill "$qemu_pid" >/dev/null 2>&1 || true
        wait "$qemu_pid" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT INT TERM

qemu-system-aarch64 \
    -M "$QEMU_MACHINE" \
    -cpu "$QEMU_CPU" \
    -smp "$QEMU_SMP" \
    -m "$QEMU_MEMORY" \
    -nographic \
    -kernel "$KERNEL_IMAGE" \
    -drive file="$FXFS_DISK",if=none,format=raw,id=fxfs,cache=writethrough \
    -device virtio-blk-device,drive=fxfs \
    -netdev user,id=smrosnet \
    -device virtio-net-device,netdev=smrosnet \
    >"$SMROS_ST_LOG" 2>&1 &
qemu_pid=$!

deadline=$((SECONDS + SMROS_ST_TIMEOUT))
status=0
while [ "$SECONDS" -lt "$deadline" ]; do
    if grep -Fq "$SMROS_ST_PROMPT" "$SMROS_ST_LOG"; then
        echo "SMROS QEMU smoke test passed: found '$SMROS_ST_PROMPT'."
        echo "Log: $SMROS_ST_LOG"
        exit 0
    fi

    if ! kill -0 "$qemu_pid" >/dev/null 2>&1; then
        wait "$qemu_pid" || status=$?
        break
    fi

    sleep 1
done

if grep -Fq "$SMROS_ST_PROMPT" "$SMROS_ST_LOG"; then
    echo "SMROS QEMU smoke test passed: found '$SMROS_ST_PROMPT'."
    echo "Log: $SMROS_ST_LOG"
    exit 0
fi

echo "SMROS QEMU smoke test failed: did not find '$SMROS_ST_PROMPT'." >&2
echo "QEMU exit status: $status" >&2
echo "Log tail:" >&2
tail -n 80 "$SMROS_ST_LOG" >&2 || true
exit 1
