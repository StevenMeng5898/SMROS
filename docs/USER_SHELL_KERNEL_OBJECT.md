# Shell and Kernel Objects: Current Integration

This document summarizes how the current shell is wired into the kernel and how that relates to the `kernel_objects/` refactor.

## Current Shell Status

The shell lives in `src/user_level/user_shell.rs`.

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
- `uptime`
- `kill`
- `testsc`
- `echo`
- `clear`
- `exit`

## How The Shell Talks To The Kernel

The current shell is tightly coupled to kernel internals.

### Direct Kernel Calls

The shell directly uses:

- `process_manager()` for `ps`, `top`, `kill`
- `scheduler::scheduler()` for `top` and `uptime`
- `PageFrameAllocator` for `meminfo`
- `crate::syscall::sys_getpid()` and `crate::syscall::sys_mmap()` inside `testsc`

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
- directly calls `sys_getpid()`
- directly calls `sys_mmap()`

So it mixes the future-facing syscall helper path with direct kernel function calls.

### `clear`

`clear` is currently a stub. The ANSI clear-sequence call is commented out.

### `exit`

`exit` does not tear down the shell process. It simply parks the current thread in a `wfi()` loop.

## Relationship To `kernel_objects/`

The `kernel_objects/` split is the right source layout for the current codebase:

- thread and scheduler code live under `src/kernel_objects/`
- handle, VMO, VMAR, and channel code also live there
- syscall code lives separately under `src/syscall/`

This means the shell now sits above a clearer layering:

```text
user_level/user_shell.rs
  -> kernel_objects/scheduler.rs
  -> kernel_lowlevel/memory.rs
  -> syscall/syscall.rs
  -> kernel_objects/* for object abstractions
```

## Kernel Objects That Matter To The Shell Today

### Scheduler and Threads

These are the most important live kernel objects for shell execution:

- the shell is created as a scheduler thread
- the shell starts only after `start_first_thread()`
- thread context is restored by `context_switch_start`

### Process Manager

The shell surfaces process-manager state through commands like:

- `ps`
- `top`
- `kill`

### Memory Subsystem

The shell exposes allocator and process-memory state through:

- `meminfo`
- `top`

### VMO / VMAR / Handle Objects

These objects are present in the tree and used by the syscall layer, but the normal shell command set does not manage them directly yet.

### Channels

The channel subsystem is initialized during boot, but the shell does not currently expose commands for channel creation or message passing.

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
- `clear` and `exit` are placeholders.
- The "user-mode" label reflects the intended direction, not the current runtime mode.
