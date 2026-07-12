#!/usr/bin/env bash
# Boot SMROS in QEMU long enough to prove that the kernel reaches the shell.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

ARCH="${ARCH:-aarch64-unknown-none}"
case "$ARCH" in
    x86_64-unknown-none|x86_64*)
        DEFAULT_QEMU_SYSTEM="qemu-system-x86_64"
        DEFAULT_KERNEL_IMAGE="$REPO_ROOT/target/x86_64-unknown-none/release/smros"
        DEFAULT_QEMU_MACHINE="q35"
        DEFAULT_QEMU_CPU="max"
        DEFAULT_QEMU_BLOCK_DEVICE="virtio-blk-pci,drive=fxfs"
        DEFAULT_QEMU_NET_DEVICE="virtio-net-pci,netdev=smrosnet"
        ;;
    riscv64gc-unknown-none-elf|riscv64*)
        DEFAULT_QEMU_SYSTEM="qemu-system-riscv64"
        DEFAULT_KERNEL_IMAGE="$REPO_ROOT/target/riscv64gc-unknown-none-elf/release/smros"
        DEFAULT_QEMU_MACHINE="virt"
        DEFAULT_QEMU_CPU="rv64"
        DEFAULT_QEMU_BLOCK_DEVICE="virtio-blk-device,drive=fxfs"
        DEFAULT_QEMU_NET_DEVICE="virtio-net-device,netdev=smrosnet"
        ;;
    *)
        DEFAULT_QEMU_SYSTEM="qemu-system-aarch64"
        DEFAULT_KERNEL_IMAGE="$REPO_ROOT/kernel8.img"
        DEFAULT_QEMU_MACHINE="virt,gic-version=4,virtualization=on"
        DEFAULT_QEMU_CPU="cortex-a710"
        DEFAULT_QEMU_BLOCK_DEVICE="virtio-blk-device,drive=fxfs"
        DEFAULT_QEMU_NET_DEVICE="virtio-net-device,netdev=smrosnet"
        ;;
esac

QEMU_SYSTEM="${QEMU_SYSTEM:-$DEFAULT_QEMU_SYSTEM}"
KERNEL_IMAGE="${KERNEL_IMAGE:-$DEFAULT_KERNEL_IMAGE}"
FXFS_DISK="${FXFS_DISK:-$REPO_ROOT/smros-fxfs.img}"
FXFS_DISK_SIZE="${FXFS_DISK_SIZE:-128M}"
QEMU_MACHINE="${QEMU_MACHINE:-$DEFAULT_QEMU_MACHINE}"
QEMU_CPU="${QEMU_CPU:-$DEFAULT_QEMU_CPU}"
QEMU_BLOCK_DEVICE="${QEMU_BLOCK_DEVICE:-$DEFAULT_QEMU_BLOCK_DEVICE}"
QEMU_NET_DEVICE="${QEMU_NET_DEVICE:-$DEFAULT_QEMU_NET_DEVICE}"
QEMU_SMP="${QEMU_SMP:-4}"
QEMU_MEMORY="${QEMU_MEMORY:-512M}"
SMROS_ST_TIMEOUT="${SMROS_ST_TIMEOUT:-45}"
SMROS_ST_LOG="${SMROS_ST_LOG:-$REPO_ROOT/target/smros-smoke-qemu.log}"
SMROS_ST_PROMPT="${SMROS_ST_PROMPT:-smros:/>}"
DEFAULT_SMROS_ST_REQUIRED_PATTERNS="SMROS-A Distributed AI-Native Operating System|[OK] Kernel initialized successfully!|[OK] Serial console initialized|[SYSCALL] Syscall handler initialized|[CHANNEL] Channel subsystem initialized|[INFO] Fast boot complete. Starting shell|[SHELL] Starting shell as scheduled thread...|$SMROS_ST_PROMPT"
SMROS_ST_REQUIRED_PATTERNS="${SMROS_ST_REQUIRED_PATTERNS:-$DEFAULT_SMROS_ST_REQUIRED_PATTERNS}"

if ! command -v "$QEMU_SYSTEM" >/dev/null 2>&1; then
    echo "error: $QEMU_SYSTEM not found" >&2
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

required_patterns_present() {
    local missing=0
    local pattern
    local required_patterns=()

    IFS='|' read -r -a required_patterns <<< "$SMROS_ST_REQUIRED_PATTERNS"
    for pattern in "${required_patterns[@]}"; do
        if [ -z "$pattern" ]; then
            continue
        fi

        if ! grep -Fq "$pattern" "$SMROS_ST_LOG"; then
            echo "missing required serial pattern: $pattern" >&2
            missing=1
        fi
    done

    return "$missing"
}

pass_if_smoke_complete() {
    if ! grep -Fq "$SMROS_ST_PROMPT" "$SMROS_ST_LOG"; then
        return
    fi

    if required_patterns_present; then
        echo "SMROS QEMU smoke test passed: found '$SMROS_ST_PROMPT' and required boot milestones."
        echo "Log: $SMROS_ST_LOG"
        exit 0
    fi

    echo "SMROS QEMU smoke test failed: prompt appeared but required boot milestones were missing." >&2
    echo "Log tail:" >&2
    tail -n 80 "$SMROS_ST_LOG" >&2 || true
    exit 1
}

"$QEMU_SYSTEM" \
    -M "$QEMU_MACHINE" \
    -cpu "$QEMU_CPU" \
    -smp "$QEMU_SMP" \
    -m "$QEMU_MEMORY" \
    -nographic \
    -kernel "$KERNEL_IMAGE" \
    -drive file="$FXFS_DISK",if=none,format=raw,id=fxfs,cache=writethrough \
    -device "$QEMU_BLOCK_DEVICE" \
    -netdev user,id=smrosnet \
    -device "$QEMU_NET_DEVICE" \
    >"$SMROS_ST_LOG" 2>&1 &
qemu_pid=$!

deadline=$((SECONDS + SMROS_ST_TIMEOUT))
status=0
while [ "$SECONDS" -lt "$deadline" ]; do
    pass_if_smoke_complete

    if ! kill -0 "$qemu_pid" >/dev/null 2>&1; then
        wait "$qemu_pid" || status=$?
        break
    fi

    sleep 1
done

pass_if_smoke_complete

echo "SMROS QEMU smoke test failed: did not find '$SMROS_ST_PROMPT'." >&2
echo "QEMU exit status: $status" >&2
required_patterns_present || true
echo "Log tail:" >&2
tail -n 80 "$SMROS_ST_LOG" >&2 || true
exit 1
