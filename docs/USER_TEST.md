# User Test Harness: Current Behavior

This document explains what the current user test code actually validates.

## Relevant Files

- `src/user_level/user_test.rs`
- `src/user_level/user_process.rs`
- `src/user_level/user_shell.rs`
- `src/main.rs`

## Two Different Test Layers Exist

The tree currently contains both:

1. an active boot-time EL0 syscall smoke test
2. additional shell-level syscall smoke tests

Those are not the same thing.

## Active Boot-Time Test

The live boot path calls:

```rust
crate::user_level::user_test::run_user_test();
```

`run_user_test()` currently:

- prints `[EL0]`-prefixed log lines
- prepares a small EL0 stack
- drops into EL0 with `switch_to_el0()`
- runs `user_test_process_entry()`
- issues Linux-style `svc #0` calls for `write`, `getpid`, `mmap`, and `exit`
- resumes at `el0_test_resume()` and validates the EL0-observed and EL1-observed syscall results

This means the current boot-time test now validates the real EL0-to-EL1 syscall trap path. It still uses a lightweight `ttbr0 = 0` setup, so it is not yet a fully isolated userspace process.

## EL0 Helpers

`src/user_level/user_test.rs` contains:

- `linux_syscall()`
- `test_getpid()`
- `test_mmap()`
- `test_write()`
- `test_exit()`
- `user_test_process_entry()`
- `user_busy_loop_entry()`

These helpers back the active boot-time EL0 test and remain useful for expanding the EL0 coverage.

## What The Current Boot Test Proves

Today the active test proves:

- the active exception vector can enter EL1 from EL0 via `svc #0`
- Linux syscall numbers for `write`, `getpid`, `mmap`, and `exit` route through `handle_syscall_simple()`
- syscall results are observed consistently by the EL0 code and the EL1 validation hook
- boot continues into shell startup afterward

## What The Current Boot Test Does Not Prove

Today the active test does not prove:

- fully isolated user page tables
- a real per-process userspace address space
- a complete Linux ABI
- a complete Zircon ABI
- complete user-space memory isolation

## The Shell's `testsc` Command

The shell exposes a `testsc` command that acts as an additional smoke test.

It currently:

- performs a lightweight write-style syscall helper call
- directly exercises Linux process/time and memory syscall helpers
- directly exercises Zircon VMO/VMAR, handle/object, signal/wait, port, channel, socket, FIFO, futex, process/thread, time/debug/system/exception, and hypervisor helpers
- directly exercises Linux signal, SysV IPC, socket/networking, misc, file, directory, fd, vector I/O, poll, and stat helpers

Treat it as a developer smoke test, not as a full syscall compliance suite.

Current successful shell runs include these group completion markers:

```text
[OK] object signal tests completed
[OK] port tests completed
[OK] socket kernel object tests completed
[OK] FIFO kernel object tests completed
[OK] futex tests completed
[OK] time/debug/system/exception tests completed
[OK] hypervisor tests completed
[OK] Linux signal, IPC, misc, and net tests completed
[OK] Linux file, dir, fd, poll, and stat tests completed
```

## Why The Logs Still Say `[EL0]`

The prefixes in `run_user_test()` reflect the intended direction of the project, not the current execution mode.

As the code stands today:

- the kernel initializes user-process scaffolding
- the boot-time syscall test executes in EL0
- the shell remains in EL1

## What Is Needed For A Full EL0 Process

To convert the current smoke test into a fully isolated user process, the kernel still needs to:

1. build or place executable user code into a user mapping
2. create a real `UserProcess`
3. install TTBR0 page tables for that process
4. set up `SP_EL0`, `ELR_EL1`, and `SPSR_EL1`
5. call `switch_to_el0()`
6. return syscall results through a fully correct EL0 register-frame path

## Bottom Line

The current user test code is useful, but it should be described accurately:

- active boot path: real EL0 syscall smoke test with lightweight address-space setup
- shell `testsc`: broader EL1 developer smoke test for syscall helper behavior

That distinction matters when evaluating boot logs or shell output.
