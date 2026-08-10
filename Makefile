# SMROS multi-architecture kernel Makefile

comma := ,
ARCH ?= aarch64-unknown-none
TARGET = $(ARCH)
KERNEL_AARCH64 = kernel8.img
KERNEL_RISCV64_ELF = $(BUILD_DIR)/smros
KERNEL_RISCV64_IMG = kernel-riscv64.img
KERNEL_RISCV64 = $(KERNEL_RISCV64_ELF)
KERNEL_X86_64 = $(BUILD_DIR)/smros
KERNEL = $(if $(filter riscv64gc-unknown-none-elf,$(TARGET)),$(KERNEL_RISCV64),$(if $(filter x86_64-unknown-none,$(TARGET)),$(KERNEL_X86_64),$(KERNEL_AARCH64)))
FXFS_DISK = smros-fxfs.img
FXFS_DISK_SIZE = 128M
BUILD_DIR = target/$(TARGET)/release
SHELL_SCRIPTS = $(sort $(wildcard scripts/*.sh))
QEMU_SYSTEM_AARCH64 ?= qemu-system-aarch64
QEMU_SYSTEM_RISCV64 ?= qemu-system-riscv64
QEMU_SYSTEM_X86_64 ?= qemu-system-x86_64
QEMU_SYSTEM = $(if $(filter riscv64gc-unknown-none-elf,$(TARGET)),$(QEMU_SYSTEM_RISCV64),$(if $(filter x86_64-unknown-none,$(TARGET)),$(QEMU_SYSTEM_X86_64),$(QEMU_SYSTEM_AARCH64)))
QEMU_MACHINE_AARCH64 ?= virt,gic-version=4,virtualization=on
QEMU_MACHINE_RISCV64 ?= virt
QEMU_MACHINE_X86_64 ?= q35
QEMU_MACHINE ?= $(if $(filter riscv64gc-unknown-none-elf,$(TARGET)),$(QEMU_MACHINE_RISCV64),$(if $(filter x86_64-unknown-none,$(TARGET)),$(QEMU_MACHINE_X86_64),$(QEMU_MACHINE_AARCH64)))
QEMU_CPU_AARCH64 ?= cortex-a710
QEMU_CPU_RISCV64 ?= rv64
QEMU_CPU_X86_64 ?= max
QEMU_CPU ?= $(if $(filter riscv64gc-unknown-none-elf,$(TARGET)),$(QEMU_CPU_RISCV64),$(if $(filter x86_64-unknown-none,$(TARGET)),$(QEMU_CPU_X86_64),$(QEMU_CPU_AARCH64)))
QEMU_BLOCK_DEVICE ?= $(if $(filter x86_64-unknown-none,$(TARGET)),virtio-blk-pci$(comma)drive=fxfs,virtio-blk-device$(comma)drive=fxfs)
QEMU_NET_DEVICE ?= $(if $(filter x86_64-unknown-none,$(TARGET)),virtio-net-pci$(comma)netdev=smrosnet,virtio-net-device$(comma)netdev=smrosnet)
OBJCOPY_AARCH64 ?= aarch64-linux-gnu-objcopy
OBJCOPY_RISCV64 ?= riscv64-linux-gnu-objcopy
OBJCOPY = $(if $(filter riscv64gc-unknown-none-elf,$(TARGET)),$(OBJCOPY_RISCV64),$(OBJCOPY_AARCH64))
# Top-level CPU knob. QEMU_SMP controls QEMU vCPUs; SMROS_LOGICAL_CPUS controls
# the kernel's logical scheduler model. By default they move together.
SMROS_CPUS ?= 4
QEMU_SMP ?= $(SMROS_CPUS)
SMROS_LOGICAL_CPUS ?= $(QEMU_SMP)
QEMU_MEMORY ?= 2G
SMOKE_QEMU_SMP ?= 4
SMOKE_QEMU_MEMORY ?= 512M
SMROS_ST_LOG ?= target/smros-smoke-qemu.log
ST_COVERAGE_DIR ?= target/coverage/st
POSIX_QEMU_MEMORY ?= 1024M
AARCH64_SYSROOT ?= /usr/aarch64-linux-gnu
POSIX_QUALITY_EVIDENCE ?=
AARCH64_RUSTFLAGS = $(strip $(RUSTFLAGS) -D warnings)
override POSIX_QEMU_MEMORY := $(value POSIX_QEMU_MEMORY)
override AARCH64_SYSROOT := $(value AARCH64_SYSROOT)
override POSIX_QUALITY_EVIDENCE := $(value POSIX_QUALITY_EVIDENCE)
export POSIX_QEMU_MEMORY
export AARCH64_SYSROOT
export POSIX_QUALITY_EVIDENCE

.PHONY: all build build-test aarch64-warning-check host-fmt-check script-check launcher-test linker-layout-test ut it posix-tool-test posix-fetch posix-audit posix-build posix-stage posix-baseline posix-run posix-report coverage-ut coverage-it coverage-host coverage-st coverage st test verify run clean clean-fxfs debug gdb qemu-icmp vm-launcher help verus verus-coverage verus-setup verus-syscall verus-kernel-objects verus-kernel-lowlevel verus-user-level verus-services

all: build

# Build the kernel
build:
	@echo "Building SMROS kernel for $(TARGET)..."
	@if [ "$(TARGET)" = "aarch64-unknown-none" ]; then \
		RUSTFLAGS='$(AARCH64_RUSTFLAGS)' SMROS_LOGICAL_CPUS='$(SMROS_LOGICAL_CPUS)' cargo build --release --target $(TARGET); \
	else \
		SMROS_LOGICAL_CPUS='$(SMROS_LOGICAL_CPUS)' cargo build --release --target $(TARGET); \
	fi
	@if [ "$(TARGET)" = "riscv64gc-unknown-none-elf" ]; then \
		echo "Using RISC-V ELF payload directly for QEMU: $(KERNEL)"; \
	elif [ "$(TARGET)" = "x86_64-unknown-none" ]; then \
		echo "Using x86_64 PVH ELF payload directly for QEMU: $(KERNEL)"; \
	else \
		$(OBJCOPY) -O binary $(BUILD_DIR)/smros $(KERNEL); \
	fi
	@echo "Build complete: $(KERNEL)"

# Production build check used by the local test suite
build-test: build
	@if [ "$(TARGET)" = "aarch64-unknown-none" ]; then \
		python3 scripts/check-aarch64-link-layout.py '$(BUILD_DIR)/smros'; \
	fi

# AArch64 release build with every Rust warning promoted to an error
aarch64-warning-check:
	@$(MAKE) build-test ARCH=aarch64-unknown-none

# Formatting check for the host-side unit-test crate
host-fmt-check:
	@cargo fmt --manifest-path tests/host/Cargo.toml --check

# Shell syntax check for project scripts
script-check:
	@bash -n $(SHELL_SCRIPTS)

# Fixed-protocol host launcher tests
launcher-test:
	@python3 scripts/test-smros-vm-launcher.py

# Structured AArch64 ELF layout checker tests
linker-layout-test:
	@python3 scripts/test-check-aarch64-link-layout.py

# Host-side unit tests for pure helper logic
ut:
	@./scripts/run-host-unit-tests.sh --lib

# Host-side integration tests for cross-module test contracts
it:
	@./scripts/run-host-unit-tests.sh --test integration_contracts

# Offline unit tests for the POSIX host tooling
posix-tool-test:
	@PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts/posix/tests -v

# Fetch and validate the pinned Open POSIX Test Suite checkout
posix-fetch:
	@PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli fetch

# Audit the pinned source inventory and reviewed exclusions
posix-audit: posix-fetch
	@PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli audit --check

# Cross-build the reviewed AArch64 test inventory and publish its stage
posix-build: posix-audit
	@PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest

# Verify the published guest stage against current pinned inputs
posix-stage: posix-build
	@PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest --verify-only

# Run the staged AArch64 Linux reference under qemu-user
posix-baseline: posix-stage
	@PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli baseline --sysroot "$${AARCH64_SYSROOT}"

# Run the staged suite in SMROS under QEMU system emulation
posix-run: posix-stage $(FXFS_DISK)
	@$(MAKE) build ARCH=aarch64-unknown-none
	@PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli run-smros --qemu-memory "$${POSIX_QEMU_MEMORY}"

# Publish all seven report artifacts; quality evidence is optional and separate
posix-report:
	@set -- --manifest host_shared/posixtest/manifest.json \
		--smros-results target/posix/aarch64/smros-run/results.ndjson \
		--linux-results target/posix/aarch64/linux-reference/results.ndjson; \
	if [ -n "$${POSIX_QUALITY_EVIDENCE}" ]; then \
		set -- "$${@}" --quality-evidence "$${POSIX_QUALITY_EVIDENCE}"; \
	fi; \
	set -- "$${@}" --out target/posix/aarch64/report; \
	PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli report "$${@}"

# cargo-tarpaulin HTML coverage for host unit tests
coverage-ut:
	@./scripts/run-host-coverage.sh ut

# cargo-tarpaulin HTML coverage for host integration tests
coverage-it:
	@./scripts/run-host-coverage.sh it

# cargo-tarpaulin HTML coverage for all host Rust tests
coverage-host:
	@./scripts/run-host-coverage.sh host

# HTML report for the QEMU system smoke layer
coverage-st:
	@mkdir -p '$(ST_COVERAGE_DIR)'
	@$(MAKE) st SMROS_ST_LOG='$(ST_COVERAGE_DIR)/smros-smoke-qemu.log'
	@./scripts/write-smoke-html-report.sh '$(ST_COVERAGE_DIR)/smros-smoke-qemu.log' '$(ST_COVERAGE_DIR)/index.html'

# Coverage and smoke summary for UT/IT/ST layers. Tarpaulin measures host UT/IT;
# ST remains a QEMU milestone smoke because guest line coverage is not wired.
coverage: coverage-host coverage-st

# QEMU system smoke test: boot until required milestones and the shell prompt appear
st: $(FXFS_DISK)
	@$(MAKE) build ARCH='$(TARGET)' QEMU_SMP='$(SMOKE_QEMU_SMP)'
	@ARCH='$(TARGET)' QEMU_SYSTEM='$(QEMU_SYSTEM)' KERNEL_IMAGE='$(KERNEL)' QEMU_MACHINE='$(QEMU_MACHINE)' QEMU_CPU='$(QEMU_CPU)' QEMU_SMP='$(SMOKE_QEMU_SMP)' QEMU_MEMORY='$(SMOKE_QEMU_MEMORY)' QEMU_BLOCK_DEVICE='$(QEMU_BLOCK_DEVICE)' QEMU_NET_DEVICE='$(QEMU_NET_DEVICE)' SMROS_ST_LOG='$(SMROS_ST_LOG)' ./scripts/smoke-qemu.sh

# Fast local confidence suite; intentionally does not boot QEMU
test: host-fmt-check script-check launcher-test linker-layout-test ut it posix-tool-test build-test

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
	@$(QEMU_SYSTEM) \
		-M $(QEMU_MACHINE) \
		-cpu $(QEMU_CPU) \
		-smp $(QEMU_SMP) \
		-m $(QEMU_MEMORY) \
		-nographic \
		-kernel $(KERNEL) \
		-drive file=$(FXFS_DISK),if=none,format=raw,id=fxfs,cache=writethrough \
		-device $(QEMU_BLOCK_DEVICE) \
		-netdev user,id=smrosnet \
		-device $(QEMU_NET_DEVICE)
	@if [ "$${SMROS_SYNC_HOST_SHARED:-1}" != "0" ]; then ./scripts/sync-host-shared.py $(FXFS_DISK) host_shared || true; fi

# Run with QEMU (debug mode with logging)
debug: build $(FXFS_DISK) qemu-icmp vm-launcher
	@echo "Starting QEMU in debug mode..."
	@$(QEMU_SYSTEM) \
		-M $(QEMU_MACHINE) \
		-cpu $(QEMU_CPU) \
		-smp $(QEMU_SMP) \
		-m $(QEMU_MEMORY) \
		-nographic \
		-kernel $(KERNEL) \
		-drive file=$(FXFS_DISK),if=none,format=raw,id=fxfs,cache=writethrough \
		-device $(QEMU_BLOCK_DEVICE) \
		-netdev user,id=smrosnet \
		-device $(QEMU_NET_DEVICE) \
		-serial mon:stdio \
		-d int,cpu_reset \
		-D qemu.log
	@if [ "$${SMROS_SYNC_HOST_SHARED:-1}" != "0" ]; then ./scripts/sync-host-shared.py $(FXFS_DISK) host_shared || true; fi

# Run with GDB server
gdb: build $(FXFS_DISK) qemu-icmp vm-launcher
	@echo "Starting QEMU with GDB server on port 1234..."
	@$(QEMU_SYSTEM) \
		-M $(QEMU_MACHINE) \
		-cpu $(QEMU_CPU) \
		-smp $(QEMU_SMP) \
		-m $(QEMU_MEMORY) \
		-nographic \
		-kernel $(KERNEL) \
		-drive file=$(FXFS_DISK),if=none,format=raw,id=fxfs,cache=writethrough \
		-device $(QEMU_BLOCK_DEVICE) \
		-netdev user,id=smrosnet \
		-device $(QEMU_NET_DEVICE) \
		-S -s
	@if [ "$${SMROS_SYNC_HOST_SHARED:-1}" != "0" ]; then ./scripts/sync-host-shared.py $(FXFS_DISK) host_shared || true; fi

# Clean build artifacts
clean:
	@echo "Cleaning..."
	@cargo clean
	@rm -f $(KERNEL_AARCH64) $(KERNEL_RISCV64_IMG)
	@rm -f qemu.log
	@echo "Clean complete (kept $(FXFS_DISK))"

# Reset persistent FxFS disk image
clean-fxfs:
	@echo "Removing persistent FxFS disk image: $(FXFS_DISK)"
	@rm -f $(FXFS_DISK)

install-target:
	@echo "Installing Rust target $(TARGET)..."
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
	@echo "SMROS multi-architecture Kernel Makefile"
	@echo ""
	@echo "Targets:"
	@echo "  all       - Build the kernel (default)"
	@echo "  build     - Build the kernel"
	@echo "  build-test - Build the production kernel image as a test"
	@echo "  aarch64-warning-check - Build and link-check AArch64 with Rust warnings denied"
	@echo "  host-fmt-check - Check formatting for the host unit-test crate"
	@echo "  script-check - Check shell script syntax"
	@echo "  ut        - Run host-side unit tests for pure shared logic"
	@echo "  it        - Run host-side integration contract tests"
	@echo "  posix-tool-test - Run offline POSIX host-tool unit tests"
	@echo "  posix-fetch - Fetch and validate the pinned POSIX source (network)"
	@echo "  posix-audit - Audit pinned POSIX inventory and review ledgers"
	@echo "  posix-build - Cross-build and publish the AArch64 POSIX stage"
	@echo "  posix-stage - Build and verify the generated AArch64 POSIX stage"
	@echo "  posix-baseline - Run the AArch64 Linux reference under qemu-user"
	@echo "  posix-run - Run the staged POSIX suite under QEMU/SMROS"
	@echo "  posix-report - Render seven POSIX report artifacts (optional POSIX_QUALITY_EVIDENCE=path)"
	@echo "  coverage-ut - Generate cargo-tarpaulin HTML for unit tests"
	@echo "  coverage-it - Generate cargo-tarpaulin HTML for integration tests"
	@echo "  coverage-host - Generate cargo-tarpaulin HTML for all host tests"
	@echo "  coverage-st - Generate QEMU smoke HTML/log report"
	@echo "  coverage  - Generate host HTML coverage and run QEMU smoke"
	@echo "  st        - Build and boot QEMU until required milestones and the smros:/> prompt appear"
	@echo "  test      - Run fast local tests (format + scripts + ut + it + offline POSIX tools + build-test)"
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
	@echo "  make verify   - Run unit, integration, build, QEMU smoke, and Verus checks"
	@echo "  make run      - Build and run in QEMU"
	@echo "  make run ARCH=riscv64gc-unknown-none-elf - Run RISC-V64"
	@echo "  make run ARCH=x86_64-unknown-none - Run x86_64"
	@echo "  make debug    - Run with debug logging"
	@echo "  make gdb      - Run with GDB server"
	@echo "  make clean    - Clean build outputs, keeping $(FXFS_DISK)"
	@echo "  make clean-fxfs - Remove $(FXFS_DISK)"
