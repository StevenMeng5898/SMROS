# SMROS

SMROS is an experimental bare-metal AArch64 kernel written in Rust for QEMU's `virt` machine. The current tree boots to a serial diagnostic shell, initializes low-level platform code plus user-level VirtIO block/net drivers, mounts a small FxFS-shaped store, keeps heavier syscall validation behind shell commands, and can launch a dynamic PIE ELF through the shell `run` command.

## Current Status

- Boots on `qemu-system-aarch64` and reaches the `smros>` shell prompt.
- Uses inline ARM64 boot assembly in `src/main.rs` and context switch routines from `src/kernel_lowlevel/context_switch.S`.
- Initializes PL011 UART, GICv3/v4 on QEMU virt, ARM generic timer, MMU/page-table helpers, SMP bookkeeping, kernel objects, channels, scheduler state, and syscall dispatch.
- Skips the boot-time EL0 smoke test on the fast path; run `testsc` from the shell for syscall validation.
- Keeps the live shell as an EL1 scheduler thread; the banner is aspirational, not proof of an isolated shell process.
- Provides modeled Linux and Zircon syscall coverage for memory, handles, IPC, object, timer/debug, hypervisor, networking, file-descriptor, and compatibility-object paths.
- Initializes a Fuchsia-inspired user-level scaffold with component instances, namespace entries, generated boot ELF metadata, `/svc` fixed-message IPC, an FxFS-shaped object store, and compatibility-app/Docker/runc smoke surfaces.
- Binds QEMU VirtIO-MMIO block and net devices from user-level driver modules.
- Uses `smros-fxfs.img` as a persistent 16 MiB block-backed FxFS image when QEMU provides the virtio-blk device.
- Embeds repository-local `host_shared/` files into the kernel at build time and installs them under `/shared` during FxFS initialization.
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

## Test

Fast local checks:

```bash
make test
```

This runs scoped formatting checks, shell script syntax checks, host-side unit
tests for pure shared logic, and the production kernel build.

Boot-level smoke test:

```bash
make st
```

This starts QEMU non-interactively and passes when the serial log reaches the
`smros:/>` shell prompt. See `docs/TESTING.md` for the full test-layer map,
including `make ut`, `make verus`, and `make verify`.

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
  -M virt,gic-version=4,virtualization=on \
  -cpu cortex-a710 \
  -smp 64 \
  -m 2G \
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
4. Mount or initialize the FxFS-shaped store and install `/pkg`, `/data`, `/tmp`, `/svc`, `/config`, and the build-time `/shared` snapshot.
5. Defer bootstrap component process launch and EL0 syscall validation until requested.
6. Start the shell scheduler thread.
7. Reach the `smros>` prompt.

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
fuzzsc
dockertest
docker images
docker pull smros/hello
docker load /shared/my-image.tar
docker run smros/hello
docker ps -a
docker logs smros0001
gemma info
gemma test
hermes info
hermes test
hermes ui
hui
hermes ask test hermes agent on SMROS
lvgl info
lvgl render
lvgl test
qmlcluster info
qmlcluster render
qmlcluster source
qmlcluster window
qmlcluster test
```

`docker load` accepts SMROS-loadable Docker archive tars already stored in FxFS,
including under `/shared`. The archive must contain `manifest.json`, a config
JSON, and uncompressed layer tar members. It stores the config and layers under
`/docker/images` and extracts regular files into the image rootfs. `docker pull`
can install the built-in sample image by name and can fetch a plain
`http://.../*.tar` archive before feeding the same loader. HTTPS Docker Registry
pulls are still reported as unsupported until TLS and bearer-token auth exist.

`fuzzsc [seed] [iterations]` runs the syzkaller-inspired syscall fuzzer from the
shell. It also accepts named limits such as
`fuzzsc seed=1234 iterations=4 time=2` or `fuzzsc iter 4 ms=500`. It mutates
structured Linux and Zircon syscall arguments against the live dispatch tables,
prints a compact success/error/unsupported summary, and only walks modeled
success-path syscalls. Unsupported ABI entries, non-returning calls, and
destructive calls such as process exit, kill, close-many, and clone-style task
creation are kept out of the interactive run so `err`, `ENOSYS`, and
unsupported counts indicate a harness or coverage gap.
The output separately reports interface syscall coverage and per-iteration
success-path case counts, so lower `calls` totals do not mean dispatcher
coverage was removed.
Explicit iteration values run exactly that many completed rounds unless a
nonzero time budget expires first.

`gemma` exposes the native SMROS Gemma model service. It installs model
metadata, prompt formatting, bounded generation, and generation logs under
`/data/gemma`. Full Google Gemma weights are still too large for the default
512 MiB SMROS/QEMU profile, so this is the SMROS-native backend boundary that a
future full-weight runner can replace.

`hermes` is a native SMROS compatibility port of
`NousResearch/hermes-agent`. Upstream Hermes is a Python 3.11 application, so
SMROS does not execute the original package directly yet. Hermes now routes
`ask` through the SMROS Gemma provider (`gemma/gemma-3n-e2b-smros`) and validates
config, provider/model routing, skills, memory, tool calls, delegated subagents,
cron metadata, `/svc`, Gemma generation, and transcript persistence under
`/data/hermes`. `gemma test`, `hermes test`, and `testsc` cover the path.
Use `hermes ui` or `hui` for the LVGL-styled full-screen keyboard/mouse
terminal UI.

`lvgl` exposes the SMROS-native LVGL-style porting layer. It models the LVGL
display, input, tick, and widget seams with a CPU renderer, serial
pointer/keypad input mapping, scheduler ticks, and an FxFS-backed PPM display
flush at `/data/lvgl/workbench.ppm`. Use `lvgl render` for the ANSI preview
and generated bounded preview image, and `lvgl test` to validate the port.

`qmlcluster` ports a Qt/QML vehicle instrument cluster into SMROS. It installs
`/data/qml-cluster/InstrumentCluster.qml` as an embeddable `Item` component and
`/data/qml-cluster/ClusterWindow.qml` as the direct Qt window wrapper, parses
the cluster properties (`speedKph`, `rpm`, `gear`, battery, range, turn
indicators, and warning text), and renders the dashboard through the SMROS LVGL
widget layer into a bounded `/data/qml-cluster/cluster.ppm` preview sized for
the current kernel heap. Use `qmlcluster render` for the serial preview and
generated PPM path, `qmlcluster source` to inspect the component QML, and
`qmlcluster window` to inspect the host-runnable window QML.
On a Qt host, run `qmlscene host_shared/qml-cluster/ClusterWindow.qml` to open
the cluster directly.

For registry images today, use the host helper. It pulls the `linux/arm64`
image with Docker, exports a single uncompressed layer, and writes the archive
shape SMROS can load:

```bash
./scripts/pull-docker-image.sh docker.1ms.run/library/alpine:latest host_shared/alpine.tar
make clean-fxfs
make run
```

```text
docker load /shared/alpine.tar
```

After `/shared/alpine.tar` is present, the same registry-shaped command also
uses that staged archive as a fallback:

```text
docker pull docker.1ms.run/library/alpine:latest
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
- GICv3/v4 interrupt controller on QEMU virt
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
make verus-services
```

The user-level harness now covers pure helper logic for `src/main.rs`, user process layout, shell parsing, FxFS, `/svc`, ELF parsing, dynamic ELF launch arithmetic, DNS/IPv4 validation, and user-level VirtIO driver checks. The services harness covers proof slices for every file under `src/user_level/services`, including Gemma/Hermes prompt-routing predicates, Docker/path/archive validation, network sizing checks, FxFS/ELF/service predicates, and shell command input checks.

## Known Limitations

- The shell banner says "User-Mode Shell", but the shell currently runs as an EL1 kernel thread.
- The explicit EL0 smoke helper uses a lightweight `TTBR0_EL1 = 0` setup when run, not a fully isolated process address space.
- The shell `testsc` command directly calls most syscall helpers from EL1; it is a developer smoke test, not an external ABI compliance suite.
- The dynamic PIE launcher works for the current mapped bring-up path, but it does not create a process-owned TTBR0 address space.
- The syscall layer is broad but modeled; many paths are interface validation, object bookkeeping, or deterministic placeholders.
- Linux fd objects can bind to FxFS files for open/read/write/stat and file-backed `mmap`, but this is not a complete VFS.
- `/shared` is a build-time snapshot of `host_shared/`, not a live host directory mount. Live sharing still needs a 9p or virtio-fs guest driver.
- TLS is reported as unsupported by the network service layer.
- Component manager, FxFS, and user-init scaffolding are not yet isolated userspace servers, full FIDL bindings, or a package resolver.
