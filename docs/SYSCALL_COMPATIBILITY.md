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
| `nanosleep` | validates request pointer and returns success |
| `clock_gettime` | writes a simple realtime/monotonic `timespec` |
| `clone` | creates a demo process through the process manager |
| `execve` | validates path pointer and returns success |
| `wait4` | writes a zero wait status when provided |
| `getpid` | returns `1` |
| `getppid` | returns `0` |
| `kill` | terminates a process through the process manager |
| `gettid` | returns `1` |

These are the only Linux-style syscalls reachable through the current active `svc` bridge.

## Linux Compatibility Caveats

- The Linux syscall surface is not complete.
- The numbering is still experimental and should not be treated as a stable ARM64 ABI promise.
- There is no general file-descriptor layer behind `open/read/write/close`.
- The current user-level smoke tests do not validate a full Linux userspace ABI.

## Zircon Side

## Zircon Syscalls Currently Dispatched By `dispatch_zircon_syscall()`

| Syscall | Current Behavior |
|---------|------------------|
| `HandleClose` | validates and releases known memory/channel/task handles |
| `HandleCloseMany` | closes a user-provided handle array |
| `HandleDuplicate` | returns a duplicated placeholder handle value |
| `HandleReplace` | validates and returns the handle value |
| `ObjectGetInfo` | validates object handle and reports a small metadata word |
| `ObjectGetProperty` | reads the modeled object property value |
| `ObjectSetProperty` | writes the modeled object property value |
| `ObjectSignal` | updates modeled object signal bits |
| `ObjectWaitOne` | checks modeled pending signals |
| `ObjectWaitMany` | checks a user-provided wait item array |
| `ThreadCreate` | creates a modeled thread handle under a modeled process |
| `ThreadStart` | marks a modeled thread as started |
| `ThreadExit` | returns success for the current modeled thread |
| `TaskKill` | marks modeled process/thread handles terminated or closes other known handles |
| `VmoCreate` | partial VMO construction |
| `VmoRead` | copies VMO bytes into the supplied buffer |
| `VmoWrite` | copies supplied bytes into the VMO |
| `VmoGetSize` | reports tracked VMO size |
| `VmoSetSize` | resizes tracked VMO state |
| `VmoOpRange` | validates and applies supported VMO range operations |
| `VmarMap` | bookkeeping-only map path |
| `VmarUnmap` | removes tracked VMAR mappings |
| `VmarAllocate` | creates a tracked child VMAR |
| `VmarProtect` | updates tracked VMAR mapping permissions |
| `VmarDestroy` | destroys tracked VMAR state |
| `VmarUnmapHandleCloseThreadExit` | validation plus placeholder cleanup |
| `ProcessCreate` | creates modeled process and root VMAR handles |
| `ProcessExit` | marks modeled process handles terminated |
| `ChannelCreate` | creates a pair of channel endpoint handles |
| `ChannelRead` | copies queued channel message bytes/handles into user buffers |
| `ChannelWrite` | copies user buffers into a queued channel message |
| `ChannelCallNoretry` | modeled write-then-read channel call without handle transfer |
| `Nanosleep` | returns success |
| `ClockGetMonotonic` | returns scheduler ticks as nanoseconds |

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

- a partially functional Linux-style dispatch path for bring-up
- a routed Zircon-style object API for kernel-side experimentation
- a larger amount of future-facing scaffolding for EL0 work

It is not yet:

- a complete Linux userspace ABI
- a complete Zircon syscall ABI or rights model
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
- File-descriptor style Linux syscalls are not implemented as a general subsystem.
- Handle ownership and lifetime tracking are still simplified.
