#!/bin/bash
# Build script for SMROS ARM64 Kernel

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "Building SMROS ARM64 Kernel..."
echo "================================"

# Check for required tools
if ! command -v rustc &> /dev/null; then
    echo "Error: rustc not found. Please install Rust."
    exit 1
fi

if ! command -v qemu-system-aarch64 &> /dev/null; then
    echo "Warning: qemu-system-aarch64 not found. Install QEMU to run the kernel."
fi

# Build the kernel
cargo build --release

# Copy the kernel binary to a convenient location
cp target/aarch64-unknown-none/release/smros kernel8.img

echo ""
echo "================================"
echo "Build complete!"
echo "Kernel image: kernel8.img"
echo ""
echo "To run with QEMU:"
echo "  ./scripts/run.sh"
echo ""
echo "Or manually:"
echo "  qemu-img create -f raw smros-fxfs.img 16M"
echo "  qemu-system-aarch64 -M virt -cpu cortex-a57 -nographic -kernel kernel8.img \\"
echo "    -drive file=smros-fxfs.img,if=none,format=raw,id=fxfs \\"
echo "    -device virtio-blk-device,drive=fxfs"
