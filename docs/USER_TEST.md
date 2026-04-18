# User Test Harness: Current Behavior

This document explains what the current user test code actually validates.

## Relevant Files

- `src/user_level/user_test.rs`
- `src/user_level/user_process.rs`
- `src/user_level/user_shell.rs`
- `src/main.rs`

## Two Different Test Layers Exist

The tree currently contains both:

1. an active boot-time smoke test
2. future-facing EL0 test entry points

Those are not the same thing.

## Active Boot-Time Test

The live boot path calls:

```rust
crate::user_level::user_test::run_user_test();
```

`run_user_test()` currently:

- prints `[EL0]`-prefixed log lines
- directly calls `sys_getpid()`
- directly calls `sys_mmap()`
- prints TODO steps for real EL0 bring-up

This means the current boot-time test is a kernel-mode smoke test of syscall functions, not a real EL0-to-EL1 trap path.

## Future-Facing EL0 Helpers

`src/user_level/user_test.rs` also contains:

- `linux_syscall()`
- `test_getpid()`
- `test_mmap()`
- `test_write()`
- `test_exit()`
- `user_test_process_entry()`
- `user_busy_loop_entry()`

These helpers are intended for a future boot path that actually drops into EL0.

## What The Current Boot Test Proves

Today the active test proves:

- the syscall module is linked and callable
- `sys_getpid()` returns a sensible placeholder result
- `sys_mmap()` returns an address-like success result
- boot continues into shell startup afterward

## What The Current Boot Test Does Not Prove

Today the active test does not prove:

- real EL0 execution
- real `svc` exception entry from a user process
- correct `eret` return to EL0
- stable Linux ABI numbering
- Zircon syscall reachability through the active exception vectors
- complete user-space memory isolation

## The Shell's `testsc` Command

The shell exposes a `testsc` command that acts as an additional smoke test.

It currently:

- performs a lightweight write-style syscall helper call
- directly calls `sys_getpid()`
- directly calls `sys_mmap()`

Treat it as a developer smoke test, not as a full syscall compliance suite.

## Why The Logs Still Say `[EL0]`

The prefixes in `run_user_test()` reflect the intended direction of the project, not the current execution mode.

As the code stands today:

- the kernel initializes user-process scaffolding
- the test harness remains in EL1
- the shell remains in EL1

## What Is Needed For A Real EL0 Test

To convert the current scaffolding into a real user-mode test path, the kernel still needs to:

1. build or place executable user code into a user mapping
2. create a real `UserProcess`
3. install TTBR0 page tables for that process
4. set up `SP_EL0`, `ELR_EL1`, and `SPSR_EL1`
5. call `switch_to_el0()`
6. return syscall results through a fully correct EL0 register-frame path

## Bottom Line

The current user test code is useful, but it should be described accurately:

- active test path: kernel-mode smoke test
- inactive but present scaffolding: future EL0 syscall test path

That distinction matters when evaluating boot logs or shell output.
