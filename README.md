# SMROS

SMROS is an experimental bare-metal multi-architecture kernel written in Rust for QEMU machines. The current tree boots on ARM64/AArch64, RISC-V64, and x86_64 to a serial diagnostic shell, initializes architecture-selected low-level platform code, mounts a small FxFS-shaped store, keeps heavier syscall validation behind shell commands, and can launch a dynamic PIE ELF through the shell `run` command on the current AArch64 user-binary path.

## Current Status

- Boots on `qemu-system-aarch64`, `qemu-system-riscv64`, and `qemu-system-x86_64` and reaches the `smros:/>` shell prompt.
- Selects low-level code by Rust target architecture through `src/kernel_lowlevel/mod.rs`: ARM64 code lives under `src/kernel_lowlevel/ARM64/`, RISC-V64 code lives under `src/kernel_lowlevel/RISCV64/`, and x86_64 code lives under `src/kernel_lowlevel/X86_64/`.
- Initializes ARM64 PL011/GIC/generic-timer code, RISC-V64 FDT-discovered NS16550/SBI-timer/supervisor-trap code, or x86_64 16550/IDT/TSC/PVH boot code, then shares the common MMU/page-table scaffolding, SMP bookkeeping, kernel objects, channels, scheduler state, and syscall dispatch.
- Skips the boot-time EL0 smoke test on the fast path; run `testsc` from the shell for syscall validation.
- Keeps the live shell as an EL1 scheduler thread; the banner is aspirational, not proof of an isolated shell process.
- Provides modeled Linux and Zircon syscall coverage for memory, handles, IPC, object, timer/debug, hypervisor, networking, file-descriptor, and compatibility-object paths.
- Initializes a Fuchsia-inspired user-level scaffold with component instances, namespace entries, generated boot ELF metadata, `/svc` fixed-message IPC, an FxFS-shaped object store, and compatibility-app/Docker/runc smoke surfaces.
- Binds QEMU VirtIO-MMIO block and net devices from user-level driver modules on ARM64/RISC-V64, and binds QEMU VirtIO-PCI block and net devices on x86_64. On RISC-V64, UART, hart, timer, and VirtIO-MMIO resources are discovered from the firmware-provided FDT instead of hard-coded board addresses.
- Uses `smros-fxfs.img` as a persistent 128 MiB block-backed FxFS image on ARM64, RISC-V64, and x86_64.
- Embeds repository-local `host_shared/` files into the kernel at build time and installs them under `/shared` during FxFS initialization.
- Supports `run <elf>` for dynamic PIE AArch64 ELF files stored in FxFS. The dynamic loader and C library are resolved from `/shared/lib` or `/lib`. RISC-V64 and x86_64 kernel boot support is present, but external user ELF loading for those ABIs is still future work.
- Maintains standalone Verus harnesses for syscall, kernel-object, low-level, and user-level pure helper logic.

## Toolchain

SMROS currently requires nightly Rust because `.cargo/config.toml` enables `build-std`.

### Required Tools

- `rustup`
- `rust-src`
- `qemu-system-aarch64`
- `qemu-system-riscv64`
- `qemu-system-x86_64`
- `qemu-img`
- `aarch64-linux-gnu-objcopy` from GNU binutils for the ARM64 raw `kernel8.img` path
- `make` for the documented build/run flow

### Recommended Setup

```bash
rustup toolchain install nightly
rustup override set nightly
rustup target add aarch64-unknown-none
rustup target add riscv64gc-unknown-none-elf
rustup target add x86_64-unknown-none
rustup component add rust-src
```

### QEMU Packages

```bash
# Ubuntu / Debian
sudo apt-get install qemu-system-arm qemu-system-misc qemu-utils binutils-aarch64-linux-gnu

# Arch Linux
sudo pacman -S qemu-full

# macOS
brew install qemu
```

## Build

The preferred build entry point is the `Makefile`. The default `ARCH` remains
`aarch64-unknown-none` so existing ARM64 workflows keep working:

```bash
make build
```

That produces:

- `target/aarch64-unknown-none/release/smros`
- `kernel8.img`

Build the RISC-V64 kernel with:

```bash
make build ARCH=riscv64gc-unknown-none-elf
```

That produces `target/riscv64gc-unknown-none-elf/release/smros`, which QEMU
loads directly as an ELF payload.

Build the x86_64 kernel with:

```bash
make build ARCH=x86_64-unknown-none
```

That produces `target/x86_64-unknown-none/release/smros`, which QEMU loads as
a PVH ELF payload.

`build.rs` snapshots files under `host_shared/` into the kernel image. Rebuild after adding files there.

## Test

Fast local checks:

```bash
make test
```

This runs scoped formatting checks, shell script syntax checks, host-side unit
tests for pure shared logic, host-side integration contract tests, and the
production kernel build.

Host-side HTML coverage:

```bash
cargo install --locked cargo-tarpaulin
make coverage-host
```

This wraps `cargo tarpaulin --out Html --fail-under 100 --include-tests` for the
host UT/IT crate and writes the source-highlighted heatmap to
`target/coverage/host/tarpaulin-report.html`. Use `make coverage-ut` or
`make coverage-it` for narrower 100% gated reports, or `make coverage` to
generate host coverage and then run the QEMU system smoke test. `make
coverage-st` writes `target/coverage/st/index.html` and the raw serial log. The
QEMU `st` layer is reported as required serial milestone coverage; it is not
Tarpaulin guest line coverage.

Boot-level smoke test:

```bash
make st
make st ARCH=riscv64gc-unknown-none-elf
make st ARCH=x86_64-unknown-none
make st ARCH=aarch64-unknown-none QEMU_CPU_AARCH64=cortex-a57
```

This starts QEMU non-interactively and passes when the serial log reaches the
required boot milestones and the `smros:/>` shell prompt. See `docs/TESTING.md`
for the full test-layer map, including `make ut`, `make it`, `make verus`, and
`make verify`.

## Run

### Normal Boot

```bash
make run
```

Run RISC-V64:

```bash
make run ARCH=riscv64gc-unknown-none-elf
```

Run x86_64:

```bash
make run ARCH=x86_64-unknown-none
```

Run ARM64 on an older QEMU CPU model:

```bash
make run ARCH=aarch64-unknown-none QEMU_CPU_AARCH64=cortex-a57
```

`make run` builds the kernel, creates `smros-fxfs.img` if missing, and starts QEMU with:

- `virtio-blk-device`/`virtio-net-device` on ARM64 and RISC-V64 `virt`
- `virtio-blk-pci`/`virtio-net-pci` on x86_64 `q35`
- QEMU user networking through the selected virtio network device

The RISC-V64 default is `-M virt -cpu rv64`, which is the portable QEMU path.
For a QEMU build that exposes a Kunminghu/XiangShan-compatible machine or CPU
model, override the RISC-V knobs instead of editing source:

```bash
make run ARCH=riscv64gc-unknown-none-elf \
  QEMU_MACHINE_RISCV64=<machine> \
  QEMU_CPU_RISCV64=<cpu>
```

The x86_64 default is `-M q35 -cpu max`, which is the portable QEMU path for
the current PVH ELF boot path. To approximate a specific Intel Xeon model in
QEMU, override the CPU model without editing source:

```bash
make run ARCH=x86_64-unknown-none QEMU_CPU_X86_64=<cpu>
```

On Linux hosts, `make run`, `make debug`, and `make gdb` first run
`scripts/setup-qemu-icmp.sh --ensure`. The legacy `scripts/run.sh` and
`scripts/run-simple.sh` paths are still ARM64-only helpers. The ICMP setup
persists and applies
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

For RISC-V64, use:

```bash
gdb
(gdb) target remote :1234
(gdb) symbol-file target/riscv64gc-unknown-none-elf/release/smros
```

For x86_64, use:

```bash
gdb
(gdb) target remote :1234
(gdb) symbol-file target/x86_64-unknown-none/release/smros
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
  -smp "${SMROS_CPUS:-8}" \
  -m 2G \
  -nographic \
  -kernel kernel8.img \
  -drive file=smros-fxfs.img,if=none,format=raw,id=fxfs,cache=writethrough \
  -device virtio-blk-device,drive=fxfs \
  -netdev user,id=smrosnet \
  -device virtio-net-device,netdev=smrosnet
```

RISC-V64:

```bash
qemu-system-riscv64 \
  -M virt \
  -cpu rv64 \
  -smp "${SMROS_CPUS:-4}" \
  -m 2G \
  -nographic \
  -kernel target/riscv64gc-unknown-none-elf/release/smros \
  -drive file=smros-fxfs.img,if=none,format=raw,id=fxfs,cache=writethrough \
  -device virtio-blk-device,drive=fxfs \
  -netdev user,id=smrosnet \
  -device virtio-net-device,netdev=smrosnet
```

x86_64:

```bash
qemu-system-x86_64 \
  -M q35 \
  -cpu max \
  -smp "${SMROS_CPUS:-4}" \
  -m 2G \
  -nographic \
  -kernel target/x86_64-unknown-none/release/smros \
  -drive file=smros-fxfs.img,if=none,format=raw,id=fxfs,cache=writethrough \
  -device virtio-blk-pci,drive=fxfs \
  -netdev user,id=smrosnet \
  -device virtio-net-pci,netdev=smrosnet
```

Exit QEMU with `Ctrl+A`, then `X`.

## Expected Boot Sequence

The current release build is expected to:

1. Print the kernel banner and platform initialization logs.
2. Initialize interrupt, timer, SMP, memory, syscall, MMU, channel, user-level, and scheduler subsystems.
3. Bind user-level VirtIO-MMIO block/net drivers on ARM64/RISC-V64 when QEMU provides the devices, or bind VirtIO-PCI block/net drivers on x86_64.
4. Mount or initialize the FxFS-shaped store and install `/pkg`, `/data`, `/tmp`, `/svc`, `/config`, and the build-time `/shared` snapshot.
5. Defer bootstrap component process launch and EL0 syscall validation until requested.
6. Start the shell scheduler thread.
7. Reach the `smros:/>` prompt.

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
sched set fair
sched sample 8
sched perfetto
docker images
docker pull smros/hello
docker load /shared/my-image.tar
docker run smros/hello
docker ps -a
docker logs smros0001
hermes info
hermes test
hermes exec meminfo
hermes random seed=1234 iterations=8
hermes test-all seed=1234 iterations=8
hermes ui
hermes ask test hermes agent on SMROS
lvgl info
lvgl render
lvgl test
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

The native SMROS Gemma model service installs model
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
`/data/hermes`. `hermes test` and `testsc` cover the path. Use `hermes ui` for
the LVGL-styled full-screen keyboard/mouse terminal UI.

Hermes can execute explicitly allowlisted guest commands with `hermes exec`,
or run a deterministic safe campaign with `hermes random`. Each campaign
prints a replay seed and writes its bounded report to
`/data/hermes/tests/latest.log`. `hermes test-all` combines the native Hermes
test, a guest campaign, and fixed host-assisted `make ut`, `make it`, and
`make st` jobs. The host jobs require `scripts/smros-vm-launcher.py`; the guest
can request only those three jobs and cannot supply host command text.
Destructive commands remain unavailable to Hermes.

`lvgl` exposes the SMROS-native LVGL-style porting layer. It models the LVGL
display, input, tick, and widget seams with a CPU renderer, serial
pointer/keypad input mapping, scheduler ticks, and an FxFS-backed PPM display
flush at `/data/lvgl/workbench.ppm`. Use `lvgl render` for the ANSI preview
and generated bounded preview image, and `lvgl test` to validate the port.

`sched perfetto [samples]` exports the scheduler ring to `/shared/trace.pftrace`
as a native Perfetto protobuf trace file with CPU tracks and slices named after
SMROS threads. Open `host_shared/trace.pftrace` in `https://ui.perfetto.dev`.

The Qt/QML vehicle instrument cluster port installs
`/data/qml-cluster/InstrumentCluster.qml` as an embeddable `Item` component and
`/data/qml-cluster/ClusterWindow.qml` as the direct Qt window wrapper, parses
the cluster properties (`speedKph`, `rpm`, `gear`, battery, range, turn
indicators, and warning text), and renders the dashboard through the SMROS LVGL
widget layer into a bounded `/data/qml-cluster/cluster.ppm` preview sized for
the current kernel heap. `testsc` exercises the renderer and stored QML assets.
On a Qt host, run `qmlscene host_shared/qml-cluster/ClusterWindow.qml` to open
the cluster directly.

For registry images today, use the host helper. It currently defaults to a
`linux/arm64` image because the external compatibility payloads are still
AArch64-oriented, exports a single uncompressed layer, and writes the archive
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
├── linker/kernel.ld            # ARM64 linker script
├── linker/kernel-riscv64.ld    # RISC-V64 linker script
├── linker/kernel-x86_64.ld     # x86_64 PVH linker script
├── src/
│   ├── main.rs                 # Shared kernel entry and architecture-neutral init
│   ├── main_logic.rs           # Pure runtime wrappers shared with Verus
│   ├── main_logic_shared.rs    # Macro bodies shared by runtime and Verus
│   ├── kernel_lowlevel/        # Shared low-level code plus ARM64/, RISCV64/, and X86_64/ backends
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

- ARM64 backend: PL011 serial console, GICv3/v4 interrupt controller on QEMU `virt`, ARM generic timer, exception vectors, and context-switch assembly.
- RISC-V64 backend: FDT-discovered NS16550-compatible serial console, SBI timer, supervisor trap/interrupt path, hart bookkeeping, and context-switch assembly.
- x86_64 backend: PVH ELF entry, 16550 COM1 serial console, IDT/PIC mask setup, invariant-TSC timer model, logical APIC/SMP bookkeeping, and context-switch assembly.
- Shared low-level modules: page-frame allocator, architecture-aware page-table entries in `mmu.rs`, process-address-space scaffolding, and pure helper logic used by tests and Verus.

### Scheduling and Threads

- Fixed maximum of 16 threads
- Idle thread plus scheduled worker threads
- Round-robin, EDF, credit, and weighted fair scheduler policies
- Multi-thread logical SMP scheduler sample workers
- Per-thread time-slice bookkeeping with an LVGL-rendered CPU trace view
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
- Linux ARM64-numbered dispatch coverage with modeled behavior for common bring-up calls
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

The user-level harness now covers pure helper logic for `src/main.rs`, user process layout, shell parsing, FxFS, `/svc`, ELF parsing, dynamic ELF launch arithmetic, DNS/IPv4 validation, and user-level VirtIO driver checks. The services harness covers the current service proof slices under `src/user_level/services`, including Gemma/Hermes prompt-routing predicates, LVGL/QML UI sizing checks, Docker/path/archive validation, network sizing checks, FxFS/ELF/service predicates, and shell command input checks.

## Known Limitations

- The shell banner says "User-Mode Shell", but the shell currently runs as an EL1 kernel thread.
- The explicit EL0 smoke helper uses a lightweight architecture-specific user-address-space setup when run, not a fully isolated process address space.
- The shell `testsc` command directly calls most syscall helpers from EL1; it is a developer smoke test, not an external ABI compliance suite.
- The dynamic PIE launcher works for the current mapped bring-up path, but it does not create a process-owned hardware user address space.
- RISC-V64 boots the kernel and shell, but external RISC-V64 user ELF loading, RISC-V Linux syscall numbering, and full SBI HSM secondary-hart startup are not complete yet.
- x86_64 boots the kernel and shell with VirtIO-PCI block/networking, but external x86_64 user ELF loading, Linux x86_64 syscall numbering, and LAPIC timer programming are not complete yet.
- The syscall layer is broad but modeled; many paths are interface validation, object bookkeeping, or deterministic placeholders.
- Linux fd objects can bind to FxFS files for open/read/write/stat and file-backed `mmap`, but this is not a complete VFS.
- `/shared` is a build-time snapshot of `host_shared/`, not a live host directory mount. Live sharing still needs a 9p or virtio-fs guest driver.
- TLS is reported as unsupported by the network service layer.
- Component manager, FxFS, and user-init scaffolding are not yet isolated userspace servers, full FIDL bindings, or a package resolver.
