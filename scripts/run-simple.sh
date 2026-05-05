#!/bin/bash
# Simple run script for SMROS ARM64 Kernel on QEMU

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

KERNEL_IMAGE="${1:-kernel8.img}"
FXFS_DISK="${FXFS_DISK:-smros-fxfs.img}"

if [ ! -f "$KERNEL_IMAGE" ]; then
    echo "Kernel image not found: $KERNEL_IMAGE"
    echo "Please build first with: ./scripts/build.sh"
    exit 1
fi

if [ ! -f "$FXFS_DISK" ]; then
    echo "Creating persistent FxFS disk image: $FXFS_DISK"
    qemu-img create -f raw "$FXFS_DISK" 16M >/dev/null
fi

echo "Starting QEMU with SMROS ARM64 Kernel..."
echo "Press Ctrl+A, then X to exit QEMU"
echo ""

qemu-system-aarch64 \
    -M virt \
    -cpu cortex-a57 \
    -smp 4 \
    -m 512M \
    -nographic \
    -kernel "$KERNEL_IMAGE" \
    -drive file="$FXFS_DISK",if=none,format=raw,id=fxfs \
    -device virtio-blk-device,drive=fxfs
