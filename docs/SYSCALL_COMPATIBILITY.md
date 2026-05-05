# SMROS Syscall Compatibility Status

This document describes the syscall layer as it exists in the current source tree. It is intentionally conservative: the goal is to document what is actually wired up today, not the full Linux or Zircon target architecture.

## Relevant Files

- `src/syscall/mod.rs`
- `src/syscall/syscall.rs`
- `src/syscall/syscall_dispatch.rs`
- `src/syscall/syscall_handler.rs`
- `src/kernel_objects/channel.rs`
- `src/kernel_objects/compat.rs`
- `src/main.rs`

## Current Architecture

There are three distinct syscall-facing layers in the repo:

### 1. Direct Rust Calls

Kernel code can directly call:

- `sys_*` functions in `src/syscall/syscall.rs`
- `dispatch_linux_syscall()`
- `dispatch_zircon_syscall()`

This is how much of the current boot-time testing works.

### 2. Active `svc` Path

The live exception vector path in `src/main.rs` uses:

```text
exception_handler
  -> handle_syscall_simple()
  -> dispatch_linux_syscall() or dispatch_zircon_syscall()
```

Important detail:

- if `syscall_num < 1000`, the active bridge treats it as a Linux syscall
- if `1000 <= syscall_num <= 1000 + u32::MAX`, the bridge treats it as a Zircon syscall and subtracts `1000` before dispatch

So the active `svc` path now exposes both the Linux dispatcher and the Zircon dispatcher.

### 3. Alternative EL0 Handler Scaffolding

`src/syscall/syscall_handler.rs` contains `handle_svc_exception_from_el0()` and result helpers, and `src/syscall/syscall_dispatch.rs` contains another bridge layer. Those files are real scaffolding, but they are not the code path used by the current assembly exception handler in `src/main.rs`.

## Linux Side

## Linux Syscalls Currently Dispatched By `dispatch_linux_syscall()`

The dispatcher now imports the ARM64 Linux syscall interface from the sample tree as named `ARM64_SYS_*` constants. All 301 non-commented sample syscall numbers are represented in the dispatcher. Implemented entries are routed to modeled SMROS behavior; recognized but unsupported entries return `ENOSYS` explicitly instead of falling through as unknown numbers.

Implemented groups:

- Memory: `mmap`, `munmap`, `mprotect`, `mremap`, `brk`, `madvise`
- Process/task: `clone`, `fork`, `vfork`, `execve`, `wait4`, `exit`, `exit_group`, `kill`, `tkill`, `tgkill`
- Identity/scheduler: `getpid`, `getppid`, `gettid`, `getuid`, `geteuid`, `getgid`, `getegid`, `sched_yield`, `set_tid_address`
- Time/resource: `nanosleep`, `clock_gettime`, `clock_getres`, `clock_nanosleep`, `gettimeofday`, `times`, `getrusage`, `prlimit64`, `sysinfo`
- Basic I/O and fd management: `read`, `write`, `close`, `close_range`, `pipe2`, `dup`, `dup2`, `dup3`, `fcntl`, `ioctl`, `flock`
- File and directory compatibility: `openat`, `openat2`, `getdents64`, `readlinkat`, `faccessat`, `faccessat2`, `mknodat`, `mkdirat`, `unlinkat`, `symlinkat`, `linkat`, `renameat`, `renameat2`, `chdir`, `fchdir`, `chroot`, `fchmod*`, `fchown*`
- Stat and sync paths: `fstat`, `newfstatat`, `statfs`, `fstatfs`, `statx`, `truncate`, `ftruncate`, `fallocate`, `fsync`, `fdatasync`, `sync`, `sync_file_range`, `utimensat`
- Vector and copy I/O: `readv`, `writev`, `pread64`, `pwrite64`, `preadv`, `pwritev`, `sendfile`, `copy_file_range`, `splice`, `tee`, `vmsplice`
- Poll/event helpers: `pselect6`, `ppoll`, `eventfd2`, `epoll_create1`, `epoll_ctl`, `epoll_pwait`, `epoll_pwait2`, `signalfd4`, `timerfd_*`, `inotify_*`
- Signals: `rt_sigaction`, `rt_sigprocmask`, `rt_sigpending`, `rt_sigtimedwait`, `rt_sigqueueinfo`, `rt_tgsigqueueinfo`, `rt_sigsuspend`, `rt_sigreturn`, `sigaltstack`
- Linux IPC: `semget`, `semctl`, `semop`, `semtimedop`, `msgget`, `msgctl`, `msgsnd`, `msgrcv`, `shmget`, `shmctl`, `shmat`, `shmdt`
- Networking: `socket`, `socketpair`, `bind`, `listen`, `accept`, `accept4`, `connect`, `getsockname`, `getpeername`, `sendto`, `recvfrom`, `sendmsg`, `recvmsg`, `recvmmsg`, `setsockopt`, `getsockopt`, `shutdown`
- Modeled Linux objects: file descriptors backed by `LinuxFile`, `LinuxDir`, `LinuxPipe`, TCP/UDP/raw/netlink socket categories, `eventfd`, `signalfd`, `timerfd`, `inotify`, `memfd`, semaphores, shared memory, and message queues
- Misc bring-up helpers: `getrandom`, `memfd_create`, `membarrier`, `uname`, `umask`, robust-list stubs, xattr validation stubs

## Linux Compatibility Caveats

- The Linux syscall interface is covered at the dispatcher level, but many filesystem, networking, IPC, io_uring, module, and namespace syscalls intentionally return `ENOSYS`.
- The file-descriptor layer is a compatibility table. Linux fd numbers point to modeled kernel object handles with readable/writable bits.
- `LinuxFile` and `LinuxDir` objects are not backed by a persistent namespace. Regular file writes append to a bounded byte queue; reads consume from that queue. Directory fds validate directory-only operations such as `getdents64`, but directory entries are currently returned as an empty zeroed buffer.
- Stat, statfs, and statx paths validate arguments and zero output buffers; they do not yet expose real inode or filesystem metadata.
- Socket and IPC syscalls are modeled enough for deterministic syscall tests, not for a complete network stack or SysV IPC implementation.
- The current user-level smoke tests do not validate a full Linux userspace ABI.

## Zircon Side

## Zircon Syscalls Currently Dispatched By `dispatch_zircon_syscall()`

The dispatcher uses the sample `zx-syscall-numbers.h` numbering. All 167 non-commented sample syscall defines, including `ZX_SYS_COUNT`, are represented in the enum and explicit dispatch table. Implemented entries are routed to concrete or modeled behavior; recognized but unsupported hardware/platform entries return `ERR_NOT_SUPPORTED`.

Implemented groups:

- Handles/objects: close, close-many, duplicate, replace, get/set property, get-info, signal, signal-peer, wait-one, wait-many, wait-async validation
- Tasks: process/thread create/start/exit, process read/write-memory placeholders, task kill, suspend-token, job create/policy/critical placeholders
- Memory objects: VMO create/read/write/get-size/set-size/op-range/create-child/create-physical/create-contiguous/cache-policy/replace-as-executable
- VMAR: allocate, map, unmap, protect, destroy, unmap-handle-close-thread-exit
- IPC/lightweight objects: channels, sockets, FIFOs, events, eventpairs, ports, timers, debug logs, resources, streams and stream vector I/O
- Time/debug/system/exception: clock get/create/read/update/adjust, monotonic clock, nanosleep, timers, debuglog create/read/write, debug read/write/send-command, system event handles, exception channel create/get-thread/get-process/finish
- Hypervisor: guest create, memory and I/O trap registration, VCPU create/resume/interrupt/read-state/write-state, SMC call modeling
- Platform and device-shaped objects: IOMMU, BTI, PMT, interrupts, PCI devices, guests, VCPUs, profiles, pagers, framebuffer, ktrace, and mtrace interfaces
- Random/debug: CPRNG draw/add-entropy plus debug command helpers

Unsupported but recognized calls remain explicit for operations that cannot be meaningfully modeled yet, such as `ioports_request`, privileged power-control details, and hardware effects behind PCI/interrupt/hypervisor calls.

These are reachable through direct Rust calls to `dispatch_zircon_syscall()` and through the active `svc` bridge by using raw syscall numbers starting at `1000`.

## Channel Syscalls

Channel syscall wrappers live in `src/kernel_objects/channel.rs`:

- `sys_channel_create()`
- `sys_channel_read()`
- `sys_channel_write()`
- `sys_channel_call_noretry()`

The channel object layer is present, initialized during boot, and routed through `dispatch_zircon_syscall()`.

## Practical Compatibility Model

The current syscall layer is best understood as:

- a Linux ARM64 dispatch path with explicit interface coverage and modeled behavior for common bring-up calls
- a Zircon dispatch path aligned to the sample syscall numbers
- a Linux fd table where fd records point to compatibility object handles
- a lightweight compatibility object table for object types that do not yet have full subsystems
- a first Fuchsia-inspired userspace scaffold with component instances, namespace entries, minimal ELF64/AArch64 loading from FxFS, ELF-runner-shaped process launch, a `/svc` service directory using Zircon channels with fixed request/reply structs, and an in-memory FxFS-shaped object store with attributes, object-id directory entries, file-position semantics, and journal replay metadata
- a larger amount of future-facing scaffolding for EL0 work

It is not yet:

- a complete Linux userspace implementation
- a complete Zircon rights/security model
- a stable compatibility contract for external binaries

## Live Boot Reality

During normal boot:

- `run_user_test()` drops to EL0 and validates the active Linux `svc` path
- the shell runs as an EL1 thread
- the active `svc` bridge targets Linux syscall numbers below `1000` and Zircon syscall numbers at `1000 + zircon_number`

That means the current boot flow exercises the syscall layer mainly as an internal kernel interface, not as a fully isolated user/kernel boundary.

## Known Limitations

- The shell still calls most syscall tests directly from EL1 rather than as isolated EL0 userspace.
- Some process/thread semantics are modeled records, not full scheduler integration.
- File-descriptor style Linux syscalls are modeled for files, directories, pipes, sockets, event-like objects, IPC objects, and memfd, but not with a real VFS or persistent filesystem.
- The component/FxFS/`/svc` scaffold is internal kernel-side state today. It parses boot ELF metadata from FxFS, models object metadata plus journal replay in memory, and exchanges fixed service messages over Zircon channels, but it is not yet a userspace component manager, full FIDL runtime, package resolver, copied-segment ELF runtime, or block-backed FxFS server.
- Handle ownership and lifetime tracking are still simplified.
- Platform/hardware-heavy Zircon calls are interface-covered but intentionally return `ERR_NOT_SUPPORTED`.
