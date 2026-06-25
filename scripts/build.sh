#!/bin/bash
# Build script for SMROS ARM64 Kernel

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "Building SMROS ARM64 Kernel..."
echo "================================"

SMROS_CPUS="${SMROS_CPUS:-8}"
QEMU_SMP="${QEMU_SMP:-$SMROS_CPUS}"
SMROS_LOGICAL_CPUS="${SMROS_LOGICAL_CPUS:-$QEMU_SMP}"

# Check for required tools
if ! command -v rustc &> /dev/null; then
    echo "Error: rustc not found. Please install Rust."
    exit 1
fi

if ! command -v qemu-system-aarch64 &> /dev/null; then
    echo "Warning: qemu-system-aarch64 not found. Install QEMU to run the kernel."
fi

# Build the kernel
SMROS_LOGICAL_CPUS="$SMROS_LOGICAL_CPUS" cargo build --release

# Emit a raw AArch64 Linux Image for QEMU's -kernel loader.
aarch64-linux-gnu-objcopy -O binary target/aarch64-unknown-none/release/smros kernel8.img

echo ""
echo "================================"
echo "Build complete!"
echo "Kernel image: kernel8.img"
echo ""
echo "To run with QEMU:"
echo "  ./scripts/run.sh"
echo ""
echo "Or manually:"
echo "  qemu-img create -f raw smros-fxfs.img 128M"
echo "  qemu-system-aarch64 -M virt,gic-version=4,virtualization=on -cpu cortex-a710 -smp $QEMU_SMP -m 2G -nographic -kernel kernel8.img \\"
echo "    -drive file=smros-fxfs.img,if=none,format=raw,id=fxfs,cache=writethrough \\"
echo "    -device virtio-blk-device,drive=fxfs \\"
echo "    -netdev user,id=smrosnet \\"
echo "    -device virtio-net-device,netdev=smrosnet"
