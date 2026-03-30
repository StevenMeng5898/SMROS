# SMROS ARM64 Kernel Makefile

ARCH = aarch64-unknown-none
TARGET = $(ARCH)
KERNEL = kernel8.img
BUILD_DIR = target/$(TARGET)/release

.PHONY: all build run clean debug help

all: build

# Build the kernel
build:
	@echo "Building SMROS ARM64 Kernel..."
	@cargo build --release
	@cp $(BUILD_DIR)/smros $(KERNEL)
	@echo "Build complete: $(KERNEL)"

# Run with QEMU (simple mode)
run: build
	@echo "Starting QEMU..."
	@qemu-system-aarch64 \
		-M virt \
		-cpu cortex-a57 \
		-smp 4 \
		-m 512M \
		-nographic \
		-kernel $(KERNEL)

# Run with QEMU (debug mode with logging)
debug: build
	@echo "Starting QEMU in debug mode..."
	@qemu-system-aarch64 \
		-M virt \
		-cpu cortex-a57 \
		-smp 4 \
		-m 512M \
		-nographic \
		-kernel $(KERNEL) \
		-serial mon:stdio \
		-d int,cpu_reset \
		-D qemu.log

# Run with GDB server
gdb: build
	@echo "Starting QEMU with GDB server on port 1234..."
	@qemu-system-aarch64 \
		-M virt \
		-cpu cortex-a57 \
		-smp 4 \
		-m 512M \
		-nographic \
		-kernel $(KERNEL) \
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
	@echo "  help      - Show this help message"
	@echo ""
	@echo "Usage:"
	@echo "  make          - Build the kernel"
	@echo "  make run      - Build and run in QEMU"
	@echo "  make debug    - Run with debug logging"
	@echo "  make gdb      - Run with GDB server"
	@echo "  make clean    - Clean everything"
