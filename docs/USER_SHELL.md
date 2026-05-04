# User Shell: Current Integration

This document summarizes how the current shell is wired into the kernel.

For the `src/kernel_objects/` layout and object responsibilities, see `docs/KERNEL_OBJECTS_DIRECTORY.md`.

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

Successful current runs include markers such as:

```text
[OK] time/debug/system/exception tests completed
[OK] hypervisor tests completed
[OK] Linux signal, IPC, misc, and net tests completed
[OK] Linux file, dir, fd, poll, and stat tests completed
```

The file/fd section creates modeled `LinuxFile` and `LinuxDir` compatibility objects, tests fd duplication and `fcntl`, moves bytes through the file object's queue, checks directory-only `getdents64`, validates stat/statx buffers, checks `writev`, `poll`, `lseek`, `ftruncate`, and sync-style calls, then closes the fds.

So it mixes the future-facing syscall helper path with direct kernel function calls.

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
- `clear` and `exit` are placeholders.
- The "user-mode" label reflects the intended direction, not the current runtime mode.
