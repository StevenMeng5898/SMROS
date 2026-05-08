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
- `uptime`
- `kill`
- `testsc`
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
```

`mount share` refreshes `/shared` from the snapshot compiled into the current kernel image. It does not read the host directory live while QEMU is already running. To see files added to `host_shared/` after boot, rebuild and restart with `make run`; then use `share` or `ls /shared`.

The current implementation is a build-time FxFS snapshot because the guest has virtio block and net drivers, but no 9p or virtio-fs filesystem driver yet. Files larger than 4 MiB are skipped by the build script and reported in the `share` command's skipped list. Shell-created files and edits under `/shared` are FxFS-local changes. Deleting a snapshot file such as `/shared/test` records a persisted tombstone in `/config/host-share-deleted`, so the file stays deleted across reboot while the same `smros-fxfs.img` is used. Remove `smros-fxfs.img` with `make clean-fxfs` to reset those tombstones.

The `run` command loads a dynamic PIE ELF from FxFS, parses `PT_INTERP` and `DT_NEEDED`, resolves the dynamic loader and C library from `/shared/lib` or `/lib`, builds an argv/env/auxv stack, and enters the loader from an EL0 launcher thread. For example, `run hello.elf` from `/shared` uses `hello.elf`, `/shared/lib/ld-linux-aarch64.so.1`, and `/shared/lib/libc.so.6` and returns to the shell after the program calls `exit`.

The launcher currently supports dynamic PIE binaries (`ET_DYN`) with a dynamic interpreter. Static `ET_EXEC` execution is still reported as unsupported. The implementation maps PT_LOAD bytes for the main executable and interpreter into the Linux mmap window and uses the current identity-mapped EL0 bring-up model, not a process-owned TTBR0 address space.

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
