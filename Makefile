# SMROS ARM64 Kernel Makefile

ARCH = aarch64-unknown-none
TARGET = $(ARCH)
KERNEL = kernel8.img
FXFS_DISK = smros-fxfs.img
FXFS_DISK_SIZE = 128M
BUILD_DIR = target/$(TARGET)/release
SHELL_SCRIPTS = $(sort $(wildcard scripts/*.sh))
QEMU_MACHINE ?= virt,gic-version=4,virtualization=on
QEMU_CPU ?= cortex-a710
# Top-level CPU knob. QEMU_SMP controls QEMU vCPUs; SMROS_LOGICAL_CPUS controls
# the kernel's logical scheduler model. By default they move together.
SMROS_CPUS ?= 4
QEMU_SMP ?= $(SMROS_CPUS)
SMROS_LOGICAL_CPUS ?= $(QEMU_SMP)
QEMU_MEMORY ?= 2G
SMOKE_QEMU_SMP ?= 4
SMOKE_QEMU_MEMORY ?= 512M

.PHONY: all build build-test host-fmt-check script-check ut st test verify run clean clean-fxfs debug gdb qemu-icmp vm-launcher help verus verus-coverage verus-setup verus-syscall verus-kernel-objects verus-kernel-lowlevel verus-user-level verus-services

all: build

# Build the kernel
build:
	@echo "Building SMROS ARM64 Kernel..."
	@SMROS_LOGICAL_CPUS='$(SMROS_LOGICAL_CPUS)' cargo build --release
	@aarch64-linux-gnu-objcopy -O binary $(BUILD_DIR)/smros $(KERNEL)
	@echo "Build complete: $(KERNEL)"

# Production build check used by the local test suite
build-test: build

# Formatting check for the host-side unit-test crate
host-fmt-check:
	@cargo fmt --manifest-path tests/host/Cargo.toml --check

# Shell syntax check for project scripts
script-check:
	@bash -n $(SHELL_SCRIPTS)

# Host-side unit tests for pure helper logic
ut:
	@./scripts/run-host-unit-tests.sh

# QEMU system smoke test: boot until the shell prompt appears
st: $(FXFS_DISK)
	@$(MAKE) build QEMU_SMP='$(SMOKE_QEMU_SMP)'
	@QEMU_MACHINE='$(QEMU_MACHINE)' QEMU_CPU='$(QEMU_CPU)' QEMU_SMP='$(SMOKE_QEMU_SMP)' QEMU_MEMORY='$(SMOKE_QEMU_MEMORY)' ./scripts/smoke-qemu.sh

# Fast local confidence suite; intentionally does not boot QEMU
test: host-fmt-check script-check ut build-test

$(FXFS_DISK):
	@echo "Creating persistent FxFS disk image: $(FXFS_DISK)"
	@qemu-img create -f raw $(FXFS_DISK) $(FXFS_DISK_SIZE) >/dev/null

qemu-icmp:
	@./scripts/setup-qemu-icmp.sh --ensure

vm-launcher:
	@./scripts/start-smros-vm-launcher.sh

# Run with QEMU (simple mode)
run: build $(FXFS_DISK) qemu-icmp vm-launcher
	@echo "Starting QEMU..."
	@qemu-system-aarch64 \
		-M $(QEMU_MACHINE) \
		-cpu $(QEMU_CPU) \
		-smp $(QEMU_SMP) \
		-m $(QEMU_MEMORY) \
		-nographic \
		-kernel $(KERNEL) \
		-drive file=$(FXFS_DISK),if=none,format=raw,id=fxfs,cache=writethrough \
		-device virtio-blk-device,drive=fxfs \
		-netdev user,id=smrosnet \
		-device virtio-net-device,netdev=smrosnet
	@if [ "$${SMROS_SYNC_HOST_SHARED:-1}" != "0" ]; then ./scripts/sync-host-shared.py $(FXFS_DISK) host_shared || true; fi

# Run with QEMU (debug mode with logging)
debug: build $(FXFS_DISK) qemu-icmp vm-launcher
	@echo "Starting QEMU in debug mode..."
	@qemu-system-aarch64 \
		-M $(QEMU_MACHINE) \
		-cpu $(QEMU_CPU) \
		-smp $(QEMU_SMP) \
		-m $(QEMU_MEMORY) \
		-nographic \
		-kernel $(KERNEL) \
		-drive file=$(FXFS_DISK),if=none,format=raw,id=fxfs,cache=writethrough \
		-device virtio-blk-device,drive=fxfs \
		-netdev user,id=smrosnet \
		-device virtio-net-device,netdev=smrosnet \
		-serial mon:stdio \
		-d int,cpu_reset \
		-D qemu.log
	@if [ "$${SMROS_SYNC_HOST_SHARED:-1}" != "0" ]; then ./scripts/sync-host-shared.py $(FXFS_DISK) host_shared || true; fi

# Run with GDB server
gdb: build $(FXFS_DISK) qemu-icmp vm-launcher
	@echo "Starting QEMU with GDB server on port 1234..."
	@qemu-system-aarch64 \
		-M $(QEMU_MACHINE) \
		-cpu $(QEMU_CPU) \
		-smp $(QEMU_SMP) \
		-m $(QEMU_MEMORY) \
		-nographic \
		-kernel $(KERNEL) \
		-drive file=$(FXFS_DISK),if=none,format=raw,id=fxfs,cache=writethrough \
		-device virtio-blk-device,drive=fxfs \
		-netdev user,id=smrosnet \
		-device virtio-net-device,netdev=smrosnet \
		-S -s
	@if [ "$${SMROS_SYNC_HOST_SHARED:-1}" != "0" ]; then ./scripts/sync-host-shared.py $(FXFS_DISK) host_shared || true; fi

# Clean build artifacts
clean:
	@echo "Cleaning..."
	@cargo clean
	@rm -f $(KERNEL)
	@rm -f qemu.log
	@echo "Clean complete (kept $(FXFS_DISK))"

# Reset persistent FxFS disk image
clean-fxfs:
	@echo "Removing persistent FxFS disk image: $(FXFS_DISK)"
	@rm -f $(FXFS_DISK)

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

# Verify the user-level services proof harness with Verus
verus-services:
	@./scripts/verify-services-verus.sh

# Audit src-to-Verus coverage classification and shared logic wiring
verus-coverage:
	@./scripts/audit-verus-coverage.sh

# Verify all currently wired Verus proof harnesses
verus: verus-coverage verus-syscall verus-kernel-objects verus-kernel-lowlevel verus-user-level verus-services

# Full local confidence suite, including QEMU smoke and Verus
verify: test st verus

# Show help
help:
	@echo "SMROS ARM64 Kernel Makefile"
	@echo ""
	@echo "Targets:"
	@echo "  all       - Build the kernel (default)"
	@echo "  build     - Build the kernel"
	@echo "  build-test - Build the production kernel image as a test"
	@echo "  host-fmt-check - Check formatting for the host unit-test crate"
	@echo "  script-check - Check shell script syntax"
	@echo "  ut        - Run host-side unit tests for pure shared logic"
	@echo "  st        - Build and boot QEMU until the smros:/> prompt appears"
	@echo "  test      - Run fast local tests (format + scripts + ut + build-test)"
	@echo "  verify    - Run test + st + all Verus proof harnesses"
	@echo "  run       - Build and run with QEMU"
	@echo "  debug     - Run with QEMU in debug mode"
	@echo "  gdb       - Run with QEMU GDB server"
	@echo "  qemu-icmp - Persist/apply Linux host ICMP setup for QEMU user networking"
	@echo "  vm-launcher - Start the host daemon used by shell vm -c Linux launches"
	@echo "  clean     - Clean build artifacts, keeping $(FXFS_DISK)"
	@echo "  clean-fxfs - Remove the persistent FxFS disk image"
	@echo "  verus-setup   - Install the pinned Verus toolchain locally"
	@echo "  verus-syscall - Verify the syscall proof harness with Verus"
	@echo "  verus-kernel-objects - Verify the kernel object proof harness with Verus"
	@echo "  verus-kernel-lowlevel - Verify the kernel low-level proof harness with Verus"
	@echo "  verus-user-level - Verify main.rs and user-level proof harness with Verus"
	@echo "  verus-services - Verify src/user_level/services proof slices with Verus"
	@echo "  verus-coverage - Audit src-to-Verus coverage classification"
	@echo "  verus     - Run all currently wired Verus proof harnesses"
	@echo "  help      - Show this help message"
	@echo ""
	@echo "Usage:"
	@echo "  make          - Build the kernel"
	@echo "  make test     - Run unit tests and production build test"
	@echo "  make st       - Run QEMU boot smoke test"
	@echo "  make verify   - Run unit, build, QEMU smoke, and Verus checks"
	@echo "  make run      - Build and run in QEMU"
	@echo "  make debug    - Run with debug logging"
	@echo "  make gdb      - Run with GDB server"
	@echo "  make clean    - Clean build outputs, keeping $(FXFS_DISK)"
	@echo "  make clean-fxfs - Remove $(FXFS_DISK)"
