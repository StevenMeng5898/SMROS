# SMROS ARM64 Kernel Makefile

ARCH = aarch64-unknown-none
TARGET = $(ARCH)
KERNEL = kernel8.img
FXFS_DISK = smros-fxfs.img
BUILD_DIR = target/$(TARGET)/release

.PHONY: all build run clean debug help verus-setup verus-syscall verus-kernel-objects verus-kernel-lowlevel verus-user-level

all: build

# Build the kernel
build:
	@echo "Building SMROS ARM64 Kernel..."
	@cargo build --release
	@aarch64-linux-gnu-objcopy -O binary $(BUILD_DIR)/smros $(KERNEL)
	@echo "Build complete: $(KERNEL)"

$(FXFS_DISK):
	@echo "Creating persistent FxFS disk image: $(FXFS_DISK)"
	@qemu-img create -f raw $(FXFS_DISK) 16M >/dev/null

# Run with QEMU (simple mode)
run: build $(FXFS_DISK)
	@echo "Starting QEMU..."
	@qemu-system-aarch64 \
		-M virt \
		-cpu cortex-a57 \
		-smp 4 \
		-m 512M \
		-nographic \
		-kernel $(KERNEL) \
		-drive file=$(FXFS_DISK),if=none,format=raw,id=fxfs,cache=writethrough \
		-device virtio-blk-device,drive=fxfs

# Run with QEMU (debug mode with logging)
debug: build $(FXFS_DISK)
	@echo "Starting QEMU in debug mode..."
	@qemu-system-aarch64 \
		-M virt \
		-cpu cortex-a57 \
		-smp 4 \
		-m 512M \
		-nographic \
		-kernel $(KERNEL) \
		-drive file=$(FXFS_DISK),if=none,format=raw,id=fxfs,cache=writethrough \
		-device virtio-blk-device,drive=fxfs \
		-serial mon:stdio \
		-d int,cpu_reset \
		-D qemu.log

# Run with GDB server
gdb: build $(FXFS_DISK)
	@echo "Starting QEMU with GDB server on port 1234..."
	@qemu-system-aarch64 \
		-M virt \
		-cpu cortex-a57 \
		-smp 4 \
		-m 512M \
		-nographic \
		-kernel $(KERNEL) \
		-drive file=$(FXFS_DISK),if=none,format=raw,id=fxfs,cache=writethrough \
		-device virtio-blk-device,drive=fxfs \
		-S -s

# Clean build artifacts
clean:
	@echo "Cleaning..."
	@cargo clean
	@rm -f $(KERNEL)
	@rm -f qemu.log
	@echo "Clean complete"

# Install ARM64 target
install-target:
	@echo "Installing ARM64 target..."
	@rustup target add $(TARGET)

# Install local Verus toolchain used by the verification harness
verus-setup:
	@./scripts/setup-verus.sh

# Verify the first syscall proof harness with Verus
verus-syscall:
	@./scripts/verify-syscall-verus.sh

# Verify the kernel object proof harness with Verus
verus-kernel-objects:
	@./scripts/verify-kernel-objects-verus.sh

# Verify the kernel low-level proof harness with Verus
verus-kernel-lowlevel:
	@./scripts/verify-kernel-lowlevel-verus.sh

# Verify main.rs and user-level proof harness with Verus
verus-user-level:
	@./scripts/verify-user-level-verus.sh

# Show help
help:
	@echo "SMROS ARM64 Kernel Makefile"
	@echo ""
	@echo "Targets:"
	@echo "  all       - Build the kernel (default)"
	@echo "  build     - Build the kernel"
	@echo "  run       - Build and run with QEMU"
	@echo "  debug     - Run with QEMU in debug mode"
	@echo "  gdb       - Run with QEMU GDB server"
	@echo "  clean     - Clean build artifacts"
	@echo "  verus-setup   - Install the pinned Verus toolchain locally"
	@echo "  verus-syscall - Verify the syscall proof harness with Verus"
	@echo "  verus-kernel-objects - Verify the kernel object proof harness with Verus"
	@echo "  verus-kernel-lowlevel - Verify the kernel low-level proof harness with Verus"
	@echo "  verus-user-level - Verify main.rs and user-level proof harness with Verus"
	@echo "  help      - Show this help message"
	@echo ""
	@echo "Usage:"
	@echo "  make          - Build the kernel"
	@echo "  make run      - Build and run in QEMU"
	@echo "  make debug    - Run with debug logging"
	@echo "  make gdb      - Run with GDB server"
	@echo "  make clean    - Clean everything"
