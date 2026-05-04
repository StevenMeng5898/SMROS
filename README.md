# SMROS

SMROS is an experimental bare-metal AArch64 kernel written in Rust for QEMU's `virt` machine. The current tree boots to a serial shell, initializes the low-level platform drivers, brings up a simple process manager, and carries an in-progress Linux/Zircon-flavored syscall layer.

## Current Status

- Boots on `qemu-system-aarch64` and reaches the `smros>` shell prompt.
- Uses inline ARM64 boot assembly in `src/main.rs`.
- Links the thread switch routines from `src/kernel_lowlevel/context_switch.S`.
- Initializes PL011 UART, GICv2, the ARM generic timer, MMU/page-table helpers, SMP bookkeeping, kernel objects, and the scheduler.
- Provides a simple process manager with fixed code, data, heap, and stack segments per process.
- Organizes kernel objects under `src/kernel_objects/`.
- Splits syscall code under `src/syscall/`.
- Runs a boot-time EL0 `svc #0` smoke test for Linux `write`, `getpid`, `mmap`, and `exit`.
- Includes EL0 process scaffolding under `src/user_level/`; the live shell still runs as an EL1 scheduled thread.
- Provides modeled Linux and Zircon syscall compatibility for bring-up tests, including memory, IPC, object, timer/debug, hypervisor, networking, and file-descriptor paths.

## Toolchain

SMROS currently requires nightly Rust because `.cargo/config.toml` enables `build-std`.

### Required Tools

- `rustup`
- `rust-src`
- `qemu-system-aarch64`
- `make` for the documented build/run flow

### Recommended Setup

```bash
rustup toolchain install nightly
rustup override set nightly
rustup target add aarch64-unknown-none
rustup component add rust-src
```

### QEMU Packages

```bash
# Ubuntu / Debian
sudo apt-get install qemu-system-arm

# Arch Linux
sudo pacman -S qemu

# macOS
brew install qemu
```

## Build

The preferred build entry point is the `Makefile`:

```bash
make build
```

That produces:

- `target/aarch64-unknown-none/release/smros`
- `kernel8.img`

You can also build manually:

```bash
cargo build --release
cp target/aarch64-unknown-none/release/smros kernel8.img
```

## Run

### Normal Boot

```bash
make run
```

### Debug Logging

```bash
make debug
```

This writes QEMU diagnostics to `qemu.log`.

### GDB Stub

```bash
make gdb
```

Then from another terminal:

```bash
gdb
(gdb) target remote :1234
(gdb) symbol-file target/aarch64-unknown-none/release/smros
```

### Manual QEMU Command

```bash
qemu-system-aarch64 \
  -M virt \
  -cpu cortex-a57 \
  -smp 4 \
  -m 512M \
  -nographic \
  -kernel kernel8.img
```

Exit QEMU with `Ctrl+A`, then `X`.

## Expected Boot Sequence

The current release build is expected to:

1. Print the kernel banner and platform initialization logs.
2. Initialize interrupt, timer, SMP, memory, MMU, syscall, channel, and scheduler subsystems.
3. Create three demo processes: `shell`, `editor`, and `compiler`.
4. Run the boot-time user test harness.
5. Start the shell thread and transfer control to the scheduler.
6. Reach the `smros>` prompt.

## Repository Layout

```text
SMROS/
├── .cargo/config.toml          # Target and build-std configuration
├── Cargo.toml                  # Package metadata
├── Makefile                    # Preferred build and run entry points
├── build.rs                    # Empty build script; assembly is linked via global_asm!
├── linker/kernel.ld            # AArch64 linker script
├── src/
│   ├── main.rs                 # Boot assembly, exception vectors, kernel entry
│   ├── kernel_lowlevel/        # Platform and low-level kernel code
│   │   ├── context_switch.S
│   │   ├── drivers.rs
│   │   ├── interrupt.rs
│   │   ├── memory.rs
│   │   ├── mmu.rs
│   │   ├── mod.rs
│   │   ├── serial.rs
│   │   ├── smp.rs
│   │   └── timer.rs
│   ├── kernel_objects/         # Threads, scheduler, handles, VMO, VMAR, channels
│   ├── syscall/                # Syscall definitions, dispatch, and handler helpers
│   └── user_level/             # User-process scaffolding, shell, and test helpers
├── docs/                       # Design and status documents
└── scripts/                    # Helper scripts (Makefile remains the documented flow)
```

## Key Subsystems

### Low-Level Platform

- PL011 serial console
- GICv2 interrupt controller
- ARM generic timer
- ARM64 exception vectors
- ARM64 context switch assembly

### Scheduling and Threads

- Fixed maximum of 16 threads
- Idle thread plus scheduled worker threads
- Round-robin scheduler with per-thread time-slice bookkeeping
- CPU affinity support in the scheduler data model

### Process and Memory Model

- Fixed maximum of 16 processes
- 4 KiB pages
- 4096 physical page frames tracked by a bitmap allocator
- Per-process address space model with four fixed segments:
  - code at `0x0000`
  - data at `0x1000`
  - heap at `0x2000`
  - stack at `0xF000`

### Kernel Objects

- Handle table
- VMO
- VMAR
- Channel
- Thread and scheduler objects
- Lightweight compatibility objects for modeled Linux and Zircon handles, including files, directories, pipes, sockets, IPC objects, timers, clocks, ports, guests, and VCPUs

## Documentation Map

- `docs/BOOT_FLOW.md`: current boot path from QEMU entry to shell prompt
- `docs/KERNEL_OBJECTS_DIRECTORY.md`: current `src/kernel_objects/` layout
- `docs/MEMORY_SYSCALLS_IMPLEMENTED.md`: status of memory-related syscalls
- `docs/SYSCALL_COMPATIBILITY.md`: syscall entry points and dispatch reality
- `docs/USER_KERNEL_IMP.md`: current EL0 and user/kernel boundary status
- `docs/USER_SHELL.md`: shell integration and command behavior
- `docs/USER_TEST.md`: current test harness behavior
- `docs/VERUS.md`: standalone Verus verification harnesses and commands

## Known Limitations

- The shell banner says "User-Mode Shell", but the shell currently runs as an EL1 kernel thread.
- The boot-time EL0 test uses a lightweight `TTBR0_EL1 = 0` setup, not a fully isolated process address space.
- The shell `testsc` command directly calls most syscall helpers from EL1; it is a developer smoke test, not an external ABI compliance suite.
- The syscall layer is broad but modeled; many paths are interface validation, object bookkeeping, or deterministic placeholders.
- The Linux file model uses compatibility objects and byte queues. It does not yet provide a persistent namespace, inode layer, or disk-backed filesystem.
- The active SVC bridge is not yet a full Linux/Zircon ABI implementation with per-process handles and memory isolation.
- Some boot-time status output still contains garbled or NUL characters.
