# SMROS

SMROS is an experimental bare-metal AArch64 kernel written in Rust for QEMU's `virt` machine. The current tree boots to a serial diagnostic shell, initializes low-level platform code plus user-level VirtIO block/net drivers, runs an EL0 syscall smoke test, mounts a small FxFS-shaped store, and can launch a dynamic PIE ELF through the shell `run` command.

## Current Status

- Boots on `qemu-system-aarch64` and reaches the `smros>` shell prompt.
- Uses inline ARM64 boot assembly in `src/main.rs` and context switch routines from `src/kernel_lowlevel/context_switch.S`.
- Initializes PL011 UART, GICv2, ARM generic timer, MMU/page-table helpers, SMP bookkeeping, kernel objects, channels, scheduler state, and syscall dispatch.
- Runs a boot-time EL0 `svc #0` smoke test for Linux `write`, `getpid`, `mmap`, and `exit`.
- Keeps the live shell as an EL1 scheduler thread; the banner is aspirational, not proof of an isolated shell process.
- Provides modeled Linux and Zircon syscall coverage for memory, handles, IPC, object, timer/debug, hypervisor, networking, file-descriptor, and compatibility-object paths.
- Initializes a Fuchsia-inspired user-level scaffold with component instances, namespace entries, generated boot ELF metadata, `/svc` fixed-message IPC, an FxFS-shaped object store, and compatibility-app/Docker/runc smoke surfaces.
- Binds QEMU VirtIO-MMIO block and net devices from user-level driver modules.
- Uses `smros-fxfs.img` as a persistent 16 MiB block-backed FxFS image when QEMU provides the virtio-blk device.
- Embeds repository-local `host_shared/` files into the kernel at build time and exposes them in the shell at `/shared`.
- Supports `run <elf>` for dynamic PIE AArch64 ELF files stored in FxFS. The dynamic loader and C library are resolved from `/shared/lib` or `/lib`.
- Maintains standalone Verus harnesses for syscall, kernel-object, low-level, and user-level pure helper logic.

## Toolchain

SMROS currently requires nightly Rust because `.cargo/config.toml` enables `build-std`.

### Required Tools

- `rustup`
- `rust-src`
- `qemu-system-aarch64`
- `qemu-img`
- `aarch64-linux-gnu-objcopy` from GNU binutils
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
sudo apt-get install qemu-system-arm qemu-utils

# Arch Linux
sudo pacman -S qemu-full

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

`build.rs` snapshots files under `host_shared/` into the kernel image. Rebuild after adding files there.

## Run

### Normal Boot

```bash
make run
```

`make run` builds the kernel, creates `smros-fxfs.img` if missing, and starts QEMU with:

- `virtio-blk-device` backed by `smros-fxfs.img`
- QEMU user networking through `virtio-net-device`

On Linux hosts, `make run`, `make debug`, `make gdb`, and the run scripts
first run `scripts/setup-qemu-icmp.sh --ensure`. This persists and applies
`net.ipv4.ping_group_range = 0 2147483647` under `/etc/sysctl.d/` so QEMU user
networking can create unprivileged ICMP echo sockets. Without this host setting,
external `ping` can resolve DNS and still fall back to TCP with an `icmp blocked`
diagnostic.

`make clean` removes build outputs and keeps `smros-fxfs.img`. Use `make clean-fxfs` when you want to reset the persistent FxFS image and `/shared` deletion tombstones.

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

On Linux, run the host ICMP setup once before launching QEMU manually:

```bash
./scripts/setup-qemu-icmp.sh --ensure
```

```bash
qemu-system-aarch64 \
  -M virt \
  -cpu cortex-a57 \
  -smp 4 \
  -m 512M \
  -nographic \
  -kernel kernel8.img \
  -drive file=smros-fxfs.img,if=none,format=raw,id=fxfs,cache=writethrough \
  -device virtio-blk-device,drive=fxfs \
  -netdev user,id=smrosnet \
  -device virtio-net-device,netdev=smrosnet
```

Exit QEMU with `Ctrl+A`, then `X`.

## Expected Boot Sequence

The current release build is expected to:

1. Print the kernel banner and platform initialization logs.
2. Initialize interrupt, timer, SMP, memory, syscall, MMU, channel, user-level, and scheduler subsystems.
3. Bind user-level VirtIO block/net drivers when QEMU provides the devices.
4. Mount or initialize the FxFS-shaped store and install `/pkg`, `/data`, `/tmp`, `/svc`, `/config`, and `/shared`.
5. Create three demo process records: `shell`, `editor`, and `compiler`.
6. Run the boot-time EL0 syscall validation.
7. Start component launcher threads and the shell scheduler thread.
8. Reach the `smros>` prompt.

## Shell Highlights

Useful commands:

```text
help
drivers
ifconfig
dhcp
dns example.com
curl http://example.com/
fxfs
mount
share
ls /shared
vi /shared/test
rm /shared/test
run hello.elf
testsc
dockertest
docker images
docker run smros/hello
docker ps -a
docker logs smros0001
```

`run hello.elf` from `/shared` expects an AArch64 dynamic PIE and resolves its interpreter and needed libraries from `/shared/lib` or `/lib`, for example:

```text
/shared/hello.elf
/shared/lib/ld-linux-aarch64.so.1
/shared/lib/libc.so.6
```

This is a working dynamic-loader handoff for the current identity-mapped EL0 bring-up path. It is not yet a fully isolated per-process address-space implementation.

## Repository Layout

```text
SMROS/
├── .cargo/config.toml          # Target and build-std configuration
├── Cargo.toml                  # Package metadata
├── Makefile                    # Build, run, clean, and Verus entry points
├── build.rs                    # Embeds host_shared/ into the kernel image
├── linker/kernel.ld            # AArch64 linker script
├── src/
│   ├── main.rs                 # Boot assembly, exception vectors, kernel entry
│   ├── main_logic.rs           # Pure runtime wrappers shared with Verus
│   ├── main_logic_shared.rs    # Macro bodies shared by runtime and Verus
│   ├── kernel_lowlevel/        # Platform and low-level kernel code
│   ├── kernel_objects/         # Threads, scheduler, handles, VMO, VMAR, channels, compat objects
│   ├── syscall/                # Syscall definitions, dispatch, and handler helpers
│   └── user_level/
│       ├── apps/               # EL0 process/test scaffolding
│       ├── drivers/            # User-level VirtIO block/net drivers and verified helper logic
│       └── services/           # Component, FxFS, /svc, ELF, run_elf, shell, networking, compat apps
├── docs/                       # Design and status documents
├── host_shared/                # Build-time snapshot exposed as /shared
├── scripts/                    # Helper scripts
└── verification/               # Standalone Verus harnesses
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
- Software Linux mapping registry for `mmap`, `munmap`, `mprotect`, and `mremap`
- Handle-backed VMO and VMAR object models
- ELF launcher maps dynamic PIE segments into the Linux mmap window for the current bring-up path

### User-Level Storage And Drivers

- User-level VirtIO-MMIO block driver for QEMU `virt`
- User-level VirtIO-MMIO network driver and simple IPv4/UDP/DNS/ICMP/TCP/HTTP/FTP service layer
- FxFS-shaped object store with object ids, attributes, directory entries, journal records, read/write/append/truncate/seek support, and block-image persistence
- Build-time `host_shared/` snapshot mounted at `/shared`

### Kernel Objects And Syscalls

- Handle table with rights checks for core modeled operations
- VMO, VMAR, channel, thread, scheduler, and compatibility-object tables
- Linux ARM64 dispatch coverage with modeled behavior for common bring-up calls
- Zircon dispatch path reachable as `1000 + zircon_syscall_number`
- `/svc` services for component manager, ELF runner, and FxFS using fixed 32-byte messages over Zircon channels

## Documentation Map

- `docs/BOOT_FLOW.md`: current boot path from QEMU entry to shell prompt
- `docs/KERNEL_OBJECTS_DIRECTORY.md`: current `src/kernel_objects/` layout
- `docs/MEMORY_SYSCALLS_IMPLEMENTED.md`: status of memory-related syscalls
- `docs/NETWORKING.md`: VirtIO net driver and user-level network service status
- `docs/SYSCALL_COMPATIBILITY.md`: syscall entry points and dispatch reality
- `docs/USER_KERNEL_IMP.md`: current EL0 and user/kernel boundary status
- `docs/USER_SHELL.md`: shell integration and command behavior
- `docs/USER_TEST.md`: current test harness behavior
- `docs/VERUS.md`: standalone Verus verification harnesses and commands

## Verus

Common commands:

```bash
make verus-setup
make verus-syscall
make verus-kernel-objects
make verus-kernel-lowlevel
make verus-user-level
```

The user-level harness now covers pure helper logic for `src/main.rs`, user process layout, shell parsing, FxFS, `/svc`, ELF parsing, dynamic ELF launch arithmetic, DNS/IPv4 validation, and user-level VirtIO driver checks.

## Known Limitations

- The shell banner says "User-Mode Shell", but the shell currently runs as an EL1 kernel thread.
- The boot-time EL0 test uses a lightweight `TTBR0_EL1 = 0` setup, not a fully isolated process address space.
- The shell `testsc` command directly calls most syscall helpers from EL1; it is a developer smoke test, not an external ABI compliance suite.
- The dynamic PIE launcher works for the current mapped bring-up path, but it does not create a process-owned TTBR0 address space.
- The syscall layer is broad but modeled; many paths are interface validation, object bookkeeping, or deterministic placeholders.
- Linux fd objects can bind to FxFS files for open/read/write/stat and file-backed `mmap`, but this is not a complete VFS.
- `/shared` is a build-time snapshot of `host_shared/`, not a live host directory mount. Live sharing still needs a 9p or virtio-fs guest driver.
- TLS is reported as unsupported by the network service layer.
- Component manager, FxFS, and user-init scaffolding are not yet isolated userspace servers, full FIDL bindings, or a package resolver.
