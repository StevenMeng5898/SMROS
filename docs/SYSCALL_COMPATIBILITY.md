# SMROS Syscall Compatibility Status

This document describes the syscall layer as it exists in the current source tree. It is intentionally conservative: the goal is to document what is actually wired up today, not the full Linux or Zircon target architecture.

## Relevant Files

- `src/syscall/mod.rs`
- `src/syscall/syscall.rs`
- `src/syscall/syscall_dispatch.rs`
- `src/syscall/syscall_handler.rs`
- `src/kernel_objects/channel.rs`
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
  -> dispatch_linux_syscall()
```

Important detail:

- if `syscall_num < 1000`, the active bridge treats it as a Linux syscall
- if `syscall_num >= 1000`, `handle_syscall_simple()` currently returns `ENOSYS`

So the active `svc` path does not currently expose the Zircon dispatcher.

### 3. Alternative EL0 Handler Scaffolding

`src/syscall/syscall_handler.rs` contains `handle_svc_exception_from_el0()` and result helpers, and `src/syscall/syscall_dispatch.rs` contains another bridge layer. Those files are real scaffolding, but they are not the code path used by the current assembly exception handler in `src/main.rs`.

## Linux Side

## Linux Syscalls Currently Dispatched By `dispatch_linux_syscall()`

| Syscall | Current Behavior |
|---------|------------------|
| `mmap` | partial anonymous mapping support |
| `munmap` | validation plus placeholder success |
| `mprotect` | placeholder success |
| `brk` | partial global heap tracking |
| `mremap` | partial relocate-or-return-old behavior |
| `fork` | creates a demo process through the process manager |
| `exit` | placeholder success |
| `exit_group` | placeholder success |
| `getpid` | returns `1` |
| `getppid` | returns `0` |
| `kill` | terminates a process through the process manager |
| `gettid` | placeholder `1` from the dispatcher |

These are the only Linux-style syscalls reachable through the current active `svc` bridge.

## Linux Functions Present But Not In The Active Dispatch Table

`src/syscall/syscall.rs` also contains functions such as:

- `sys_vfork()`
- `sys_clone()`
- `sys_execve()`
- `sys_wait4()`
- `sys_clock_gettime()`
- `sys_nanosleep_linux()`

Those functions exist in source, but they are not wired into the active `dispatch_linux_syscall()` match table in the current tree.

## Linux Compatibility Caveats

- The Linux syscall surface is not complete.
- The numbering is still experimental and should not be treated as a stable ARM64 ABI promise.
- There is no general file-descriptor layer behind `open/read/write/close`.
- The current user-level smoke tests do not validate a full Linux userspace ABI.

## Zircon Side

## Zircon Syscalls Currently Dispatched By `dispatch_zircon_syscall()`

| Syscall | Current Behavior |
|---------|------------------|
| `HandleClose` | validates handle and returns placeholder success |
| `HandleDuplicate` | returns a duplicated placeholder handle value |
| `VmoCreate` | partial VMO construction |
| `VmoRead` | placeholder read path |
| `VmoWrite` | placeholder write path |
| `VmoGetSize` | placeholder size reporting |
| `VmarMap` | bookkeeping-only map path |
| `VmarUnmap` | placeholder unmap path |
| `VmarUnmapHandleCloseThreadExit` | validation plus placeholder cleanup |
| `ProcessCreate` | creates a demo process and returns placeholder handles |

These are reachable only through direct Rust calls to `dispatch_zircon_syscall()` today. They are not reachable from the active `svc` handler used by the booted kernel.

## Zircon Functions Present But Not Exposed By `dispatch_zircon_syscall()`

The tree also contains source implementations for:

- `sys_vmo_set_size()`
- `sys_vmo_op_range()`
- `sys_vmar_protect()`
- `sys_vmar_allocate()`
- `sys_vmar_destroy()`
- `sys_process_exit()`
- `sys_thread_create()`
- `sys_thread_start()`
- `sys_thread_exit()`
- `sys_task_kill()`
- `sys_handle_close_many()`
- `sys_handle_replace()`
- `sys_object_wait_one()`
- `sys_object_wait_many()`
- `sys_object_signal()`
- `sys_object_get_info()`
- `sys_object_get_property()`
- `sys_object_set_property()`
- `sys_clock_get_monotonic()`
- `sys_nanosleep()`

These functions are part of the compatibility scaffold, but they are not all part of the currently exposed Zircon dispatch table.

## Channel Syscalls

Channel syscall wrappers live in `src/kernel_objects/channel.rs`:

- `sys_channel_create()`
- `sys_channel_read()`
- `sys_channel_write()`
- `sys_channel_call_noretry()`

The channel object layer is present and initialized during boot, but these wrappers are not currently routed through `dispatch_zircon_syscall()`.

## Practical Compatibility Model

The current syscall layer is best understood as:

- a partially functional Linux-style dispatch path for bring-up
- a partially exposed Zircon-style object API for kernel-side experimentation
- a larger amount of future-facing scaffolding for EL0 work

It is not yet:

- a complete Linux userspace ABI
- a complete Zircon syscall ABI
- a stable compatibility contract for external binaries

## Live Boot Reality

During normal boot:

- `run_user_test()` directly calls syscall functions from kernel mode
- the shell runs as an EL1 thread
- the active `svc` bridge only targets the Linux dispatch table

That means the current boot flow exercises the syscall layer mainly as an internal kernel interface, not as a fully isolated user/kernel boundary.

## Known Limitations

- Active `svc` routing bypasses `dispatch_zircon_syscall()`.
- Many syscall handlers return placeholder success values.
- Several functions exist in source but are not wired into dispatch.
- File-descriptor style Linux syscalls are not implemented as a general subsystem.
- Handle ownership and lifetime tracking are still simplified.
