# User Shell: Current Integration

This document summarizes how the current shell is wired into the kernel.

For the `src/kernel_objects/` layout and object responsibilities, see `docs/KERNEL_OBJECTS_DIRECTORY.md`.

## Current Shell Status

The shell lives in `src/user_level/services/user_shell.rs`.

Important reality:

- the banner says `SMROS User-Mode Shell v0.5.0`
- the live shell currently runs as an EL1 scheduler thread
- it is not yet an isolated EL0 process

## Shell Startup Path

The current startup path is:

```text
kernel_main()
  -> start_user_shell()
  -> scheduler().create_thread(shell_thread_wrapper, "user_shell")
  -> start_first_thread()
  -> UserShell::run()
```

`start_user_shell()` logs shell startup, creates the thread, and leaves the actual handoff to the scheduler.

## Shell Command Set

The shell currently registers these commands:

- `help`
- `version`
- `ps`
- `top`
- `meminfo`
- `components`
- `fxfs`
- `drivers`
- `ifconfig`
- `dns`
- `dhcp`
- `ping`
- `curl`
- `ftp`
- `tls`
- `pwd`
- `ls`
- `cd`
- `cd..`
- `mkdir`
- `write`
- `cat`
- `cp`
- `mv`
- `rm`
- `run`
- `vi`
- `mount`
- `share`
- `svc`
- `porttest`
- `dockertest`
- `docker`
- `gemma`
- `hermes`
- `hui`
- `qmlcluster`
- `uptime`
- `dhrystone`
- `sched`
- `kill`
- `testsc`
- `fuzzsc`
- `echo`
- `clear`
- `reboot`
- `exit`

## How The Shell Talks To The Kernel

The current shell is tightly coupled to kernel internals.

### Direct Kernel Calls

The shell directly uses:

- `process_manager()` for `ps`, `top`, `kill`
- `scheduler::scheduler()` for `top` and `uptime`
- `PageFrameAllocator` for `meminfo`
- many `crate::syscall::sys_*()` helpers inside `testsc`
- the EL0 `test_write()` helper for the first write smoke call

### Direct Serial Access

The shell:

- writes output through `Serial`
- reads input by polling PL011 MMIO registers directly

This is another reason it should be considered a kernel shell in the current tree.

## Command Behavior Notes

### `testsc`

`testsc` is a smoke test command, not a complete ABI validator.

It currently:

- attempts a write-style smoke call through `test_write()`
- directly exercises Linux process/time calls
- directly exercises Linux memory calls and memory accounting
- directly exercises Zircon VMO/VMAR, handle/object, signal/wait, port, channel, socket, FIFO, futex, process/thread, time/debug/system/exception, and hypervisor helpers
- directly exercises Linux signal, IPC, networking, misc, file, directory, fd, poll, and stat helpers
- directly checks the minimal component framework, FxFS-shaped object-store paths, and `/svc` fixed-message IPC

So it mixes the future-facing syscall helper path with direct kernel function calls.

### `dhrystone`

`dhrystone [runs-per-core]` runs a native shell Dhrystone 2.1 port derived from
the BYTE UnixBench `dhry_1.c`, `dhry_2.c`, and `dhry.h` benchmark code. The
default is `50000` runs per core.

The command reports:

- measured per-core Dhrystones per second
- projected aggregate Dhrystones per second for the currently online logical CPUs
- per-core and aggregate DMIPS, using the usual Dhrystone divisor of `1757`

Current SMP boot marks 64 logical CPUs online for scheduling/status reporting,
so the aggregate score is a logical 64-core projection from one measured
Dhrystone worker. It is not yet a true parallel 64-worker benchmark.

### `fuzzsc`

`fuzzsc [seed] [iterations]` is a syzkaller-inspired syscall fuzzer wired into
the shell. It also accepts named parameters:

```text
fuzzsc seed=<n> iterations=<n> time=<seconds>
fuzzsc iter <n> ms=<milliseconds>
```

It runs at the dispatcher layer and mutates structured arguments for the modeled
Linux ARM64 and Zircon syscall tables.

It is deliberately a safe interactive fuzzer:

- pointer arguments target kernel-owned scratch buffers instead of arbitrary
  invalid addresses
- non-returning and shell-destructive calls are skipped
- clone-style process creation is skipped in the default round
- each run closes tracked handles, fds, and mappings before returning

The command prints call, success, `ENOSYS`, unsupported, error, and object-count
totals, including completed/configured iterations and timeout status when a
time limit is used. The default sweep uses supported success-path syscalls; a
nonzero error, `ENOSYS`, or unsupported counter means the harness selected a bad
argument shape or walked a syscall that is not modeled yet. It also prints the
known Linux and Zircon interface syscall counts plus success-path cases per
iteration, so the interactive `calls` total is not treated as the full
compatibility surface. Explicit iteration values are not clamped; the time
limit, when present, is the early-stop condition. It is not a full external
syzkaller executor with coverage feedback yet; it is the in-kernel fuzzing entry
point for broad syscall-dispatch coverage.

Successful current runs include markers such as:

```text
[OK] time/debug/system/exception tests completed
[OK] hypervisor tests completed
[OK] Linux signal, IPC, misc, and net tests completed
[OK] Linux file, dir, fd, poll, and stat tests completed
[OK] component framework, FxFS, and /svc IPC returned
```

The file/fd section creates modeled `LinuxFile` and `LinuxDir` compatibility objects, tests fd duplication and `fcntl`, moves bytes through the file object's queue, checks directory-only `getdents64`, validates stat/statx buffers, checks `writev`, `poll`, `lseek`, `ftruncate`, and sync-style calls, then closes the fds.

The component/FxFS/`/svc` section verifies that the boot topology is installed, `/bootstrap/fxfs` has a modeled process and launcher thread, `/pkg/bin` entries exist, a small `/data` file can be written, appended, truncated, seek-read, checked for attributes, and replayed through the journal model, and fixed component-manager, runner, and filesystem service messages round-trip over Zircon channels.

The `components` command also shows the minimal ELF loader state. A successful boot currently reports three loaded ELF images, zero load errors, one PT_LOAD segment per bootstrap image, and an entry address for `/`, `/bootstrap/fxfs`, and `/bootstrap/user-init`.

The `fxfs` command shows object-store statistics, directory-entry count, journal replay count, and the generated boot ELF files in `/pkg/bin`. The listing includes object id, size, mode, link count, uid/gid owner, and name; the current trampoline images are 120 bytes each.

### Host Shared Snapshot

Files placed in the repository's `host_shared/` directory are embedded at build time and exposed inside the shell as `/shared`.

Useful commands:

```text
mount
mount share
share
ls /shared
cd /shared
vi /shared/test
rm /shared/test
run hello.elf
run /shared/hello.elf
docker images
docker pull smros/hello
docker pull http://10.0.2.2:8000/my-image.tar
docker load /shared/my-image.tar
docker run my/image:latest
gemma info
gemma test
gemma ask test gemma on smros
hermes info
hermes test
hermes skills
hermes ui
hui
hermes web
hermes web text
hermes ask test hermes on smros
qmlcluster info
qmlcluster render
qmlcluster source
qmlcluster window
qmlcluster test
```

FxFS installs `/shared` from the snapshot compiled into the current kernel image during boot. `share` lists that snapshot, and `mount share` refreshes `/shared` from the same embedded snapshot. They do not read the host directory live while QEMU is already running. To see files added to `host_shared/` after boot, rebuild and restart with `make run`; then use `share` or `ls /shared`.

The current implementation is a build-time FxFS snapshot because the guest has virtio block and net drivers, but no 9p or virtio-fs filesystem driver yet. Files larger than 64 MiB are skipped by the build script and reported in the `share` command's skipped list. Shell-created files and edits under `/shared` are FxFS-local changes. Deleting a snapshot file such as `/shared/test` records a persisted tombstone in `/config/host-share-deleted`, so the file stays deleted across reboot while the same `smros-fxfs.img` is used. Remove `smros-fxfs.img` with `make clean-fxfs` to reset those tombstones.

### Hermes Agent Port

`gemma` exposes the native SMROS Gemma model service. It installs model
metadata, prompt formatting, bounded generation, and generation logs under
`/data/gemma`. Full Google Gemma weights are still too large for the default
512 MiB SMROS/QEMU profile, so this is the SMROS-native backend boundary that a
future full-weight runner can replace.

`hermes` exposes a native SMROS compatibility port of `NousResearch/hermes-agent`.
The upstream Hermes project is a Python 3.11 application, while SMROS does not
yet host a Python runtime. Hermes now routes `ask` through the SMROS Gemma
provider (`gemma/gemma-3n-e2b-smros`) and validates provider/model routing,
skill lookup, memory updates, tool calls, subagent delegation, cron metadata,
`/svc`, Gemma generation, the generated web UI, and transcript persistence
under `/data/hermes`.

Hermes installs four native SMROS skills under `/data/hermes/skills`:
`smros-kernel`, `hermes-web-ui`, `smros-ops`, and `hermes-memory`. Prompt
routing reports the matched skills in the shell response. The web UI renderer
writes a static HTML model to `/data/hermes/web/index.html`. By default,
`hermes web` parses that HTML and renders a richer CPU-drawn native UI surface:
panels, status tiles, buttons, text, and skill rows are rasterized into
`/data/hermes/web/hermes-ui.ppm`, with an ANSI color preview printed in the
serial shell. Use `hermes web text` for the older text-only shell view, and
`hermes web source` only when you explicitly want to inspect the HTML source.
For interaction, use `hermes ui` or the shorter `hui` entry. That opens a
full-screen Hermes UI on the serial terminal with a focused prompt field,
buttons, response panel, keyboard navigation, and xterm mouse tracking. Type in
the prompt field, press Enter or click Send to submit, Tab between controls,
use arrow keys or the mouse wheel to move through the response panel, click
Clear to reset the prompt, and press Esc or click Exit to return to the normal
`smros>` shell.

Useful commands:

```text
gemma info
gemma test
gemma ask test gemma agent on SMROS
hermes info
hermes test
hermes skills
hermes ui
hui
hermes web
hermes web text
hermes web source
hermes ask test hermes agent on SMROS
qmlcluster info
qmlcluster render
qmlcluster source
qmlcluster window
qmlcluster test
```

### Qt/QML Vehicle Cluster Port

`qmlcluster` ports a Qt/QML car instrument cluster into SMROS. Because SMROS
does not host Qt yet, the service stores an embeddable component at
`/data/qml-cluster/InstrumentCluster.qml`, a direct Qt window wrapper at
`/data/qml-cluster/ClusterWindow.qml`, parses the dashboard properties, and
renders the cluster with a native CPU rasterizer into
`/data/qml-cluster/cluster.ppm`. The default cluster shows speed, rpm, gear,
drive mode, battery, range, turn indicators, lane/vehicle visualization, and
warning text. On a Qt host, the same component opens directly with
`qmlscene host_shared/qml-cluster/ClusterWindow.qml`.

Useful commands:

```text
qmlcluster info
qmlcluster render
qmlcluster source
qmlcluster window
qmlcluster test
```

The shell `testsc` suite includes the Gemma, Hermes, and Qt/QML cluster full
smoke tests.

The `run` command loads a dynamic PIE ELF from FxFS, parses `PT_INTERP` and `DT_NEEDED`, resolves the dynamic loader and C library from `/shared/lib` or `/lib`, builds an argv/env/auxv stack, and enters the loader from an EL0 launcher thread. For example, `run hello.elf` from `/shared` uses `hello.elf`, `/shared/lib/ld-linux-aarch64.so.1`, and `/shared/lib/libc.so.6` and returns to the shell after the program calls `exit`.

The launcher currently supports dynamic PIE binaries (`ET_DYN`) with a dynamic interpreter. Static `ET_EXEC` execution is still reported as unsupported. The implementation maps PT_LOAD bytes for the main executable and interpreter into the Linux mmap window and uses the current identity-mapped EL0 bring-up model, not a process-owned TTBR0 address space.

### Docker Image Pull And Load

The `docker` command now has a persistent local image table under `/docker/images`.
`docker images` lists all image metadata records, not only the built-in sample.

`docker load <archive.tar>` reads a SMROS-loadable Docker archive from FxFS,
parses `manifest.json`, stores the config and layer tar files, extracts regular
files and directories from uncompressed layers into the image rootfs, and writes
an `image.meta` record. The loaded image can be used by `docker run`,
`docker create`, and `docker start` through the existing modeled runc path. Use
`scripts/pull-docker-image.sh` to convert a registry image into this archive
shape on the host.

`docker pull <image-or-http-url>` supports two current paths: built-in aliases
such as `smros/hello`, and plain HTTP URLs that point directly at an
uncompressed image archive, for example a tar served from the host to QEMU user
networking. Full Docker Registry pulls remain intentionally unsupported because
this kernel stack does not yet implement TLS or registry bearer-token auth.

Use `scripts/pull-docker-image.sh <image> host_shared/<name>.tar` on the host
for HTTPS registries such as `docker.1ms.run`. The helper defaults to
`DOCKER_PLATFORM=linux/arm64`, matching the QEMU guest. Rebuild afterward so the
new host_shared snapshot is embedded, and run `make clean-fxfs` once if an older
small `smros-fxfs.img` already exists. Then run `docker load /shared/<name>.tar`
inside SMROS.
For `docker.1ms.run/library/alpine:latest`, `docker pull` also checks the staged
fallback path `/shared/alpine.tar`.

### Block-Backed FxFS

The default QEMU targets attach `smros-fxfs.img` through virtio-blk. FxFS loads from that image when it exists and writes metadata/data changes back to it after mutating non-`/shared` paths. `make clean` keeps the image; use `make clean-fxfs` to reset it.

The `mount` command shows whether FxFS is block-backed and whether the last sync succeeded.

### `svc`

The `svc` command shows registered services, connection count, request/reply counters, and the last fixed-message status. A clean boot starts with three services and zero connections; after `testsc`, the smoke path has three connections, three requests, and three replies.

### `clear`

`clear` is currently a stub. The ANSI clear-sequence call is commented out.

### `exit`

`exit` does not tear down the shell process. It simply parks the current thread in a `wfi()` loop.

## Practical Interpretation

The current shell should be treated as:

- a diagnostic shell
- a scheduler/demo workload
- a convenient place to inspect process and memory state

It should not yet be treated as:

- a protected user shell
- a proof of complete EL0 support
- a proof of complete Linux or Zircon syscall compatibility

## Known Limitations

- Shell execution is still EL1-only.
- Shell input/output bypasses any future user-space I/O abstraction.
- `/shared` is a build-time snapshot of `host_shared/`, not a live two-way host mount.
- FxFS is persistent only when the virtio-blk-backed `smros-fxfs.img` is present.
- `clear` and `exit` are placeholders.
- The "user-mode" label reflects the intended direction, not the current runtime mode.
