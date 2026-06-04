#!/bin/bash
# Simple run script for SMROS ARM64 Kernel on QEMU

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

KERNEL_IMAGE="${1:-kernel8.img}"
FXFS_DISK="${FXFS_DISK:-smros-fxfs.img}"
FXFS_DISK_SIZE="${FXFS_DISK_SIZE:-128M}"
QEMU_MACHINE="${QEMU_MACHINE:-virt,gic-version=4,virtualization=on}"
QEMU_CPU="${QEMU_CPU:-cortex-a710}"

if [ ! -f "$KERNEL_IMAGE" ]; then
    echo "Kernel image not found: $KERNEL_IMAGE"
    echo "Please build first with: ./scripts/build.sh"
    exit 1
fi

if [ ! -f "$FXFS_DISK" ]; then
    echo "Creating persistent FxFS disk image: $FXFS_DISK"
    qemu-img create -f raw "$FXFS_DISK" "$FXFS_DISK_SIZE" >/dev/null
fi

./scripts/setup-qemu-icmp.sh --ensure

echo "Starting QEMU with SMROS ARM64 Kernel..."
echo "Press Ctrl+A, then X to exit QEMU"
echo ""

qemu-system-aarch64 \
    -M "$QEMU_MACHINE" \
    -cpu "$QEMU_CPU" \
    -smp 4 \
    -m 512M \
    -nographic \
    -kernel "$KERNEL_IMAGE" \
    -drive file="$FXFS_DISK",if=none,format=raw,id=fxfs,cache=writethrough \
    -device virtio-blk-device,drive=fxfs \
    -netdev user,id=smrosnet \
    -device virtio-net-device,netdev=smrosnet
