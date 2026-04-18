# User / Kernel Boundary: Current Implementation

This document describes the actual state of user/kernel separation in the current tree.

The short version is:

- EL0 scaffolding exists
- MMU and page-table helpers exist
- an `svc` bridge exists
- the live boot path still runs the shell and test harness from EL1

## Relevant Files

- `src/kernel_lowlevel/mmu.rs`
- `src/syscall/syscall_handler.rs`
- `src/syscall/syscall_dispatch.rs`
- `src/user_level/user_process.rs`
- `src/user_level/user_shell.rs`
- `src/user_level/user_test.rs`
- `src/main.rs`

## What Exists Today

### MMU and Page-Table Helpers

`src/kernel_lowlevel/mmu.rs` provides:

- page-table entry definitions
- TTBR0 and TTBR1 root allocation
- `PageTableManager`
- user-region mapping helpers
- kernel-region mapping helpers
- address-space bookkeeping through VMAs

This is the kernel's main scaffolding for eventual EL0 isolation.

### User Process Structure

`src/user_level/user_process.rs` defines `UserProcess`, which carries:

- a base `ProcessControlBlock`
- a `PageTableManager`
- user stack address and size
- user entry point
- placeholder process and VMAR handles
- an `initialized` flag

The same file also provides:

- `create_user_process()`
- lookup helpers
- `switch_to_el0()`

### EL0 Test and Shell Entry Points

`src/user_level/user_test.rs` contains:

- `linux_syscall()` using `svc #0`
- `user_test_process_entry()`
- `user_busy_loop_entry()`

`src/user_level/user_shell.rs` contains:

- `user_shell_entry()`
- `start_user_shell()`
- the live shell implementation

These files are the current staging area for user-mode work.

### Exception Handling

The live synchronous exception handler is assembled in `src/main.rs`. For `svc` exceptions it currently calls `handle_syscall_simple()`.

`src/syscall/syscall_handler.rs` also contains `handle_svc_exception_from_el0()`, but that is not the handler the current assembly vectors use.

## What The Boot Path Actually Does

During a normal boot:

1. `kernel_main()` calls `user_process::init()`.
2. `kernel_main()` calls `run_user_test()`.
3. `kernel_main()` calls `start_user_shell()`.
4. `start_user_shell()` creates a normal scheduler thread.
5. `start_first_thread()` jumps into that EL1 thread.

No part of the normal boot path:

- creates a real EL0 process and runs it
- calls `switch_to_el0()`
- executes `user_test_process_entry()` in EL0
- executes `user_shell_entry()` after an EL transition

## Current State By Area

| Area | Current State | Notes |
|------|---------------|-------|
| Page-table manager | present | real scaffolding in `mmu.rs` |
| User-process data model | present | `UserProcess` exists |
| EL0 transition helper | present | `switch_to_el0()` exists |
| Live shell in EL0 | not active | shell runs as EL1 thread |
| Live test process in EL0 | not active | boot test runs in kernel mode |
| Full register-frame EL0 syscall handler | not active | current vectors use `handle_syscall_simple()` |
| Zircon-on-SVC path | not active | active `svc` path routes Linux-only |

## Why The Shell Is Not Yet A Real User Process

The shell source still contains future-facing comments about EL0. In the current implementation:

- `start_user_shell()` uses `scheduler().create_thread(...)`
- the shell thread executes with normal kernel thread context
- shell commands call kernel services directly
- serial input is polled directly from PL011 registers

So the shell is currently a kernel-resident diagnostic shell, not an isolated user process.

## Why `run_user_test()` Is Not Yet A Real EL0 Test

`run_user_test()` in `src/user_level/user_test.rs` currently:

- prints `[EL0]` log prefixes
- directly calls `sys_getpid()`
- directly calls `sys_mmap()`
- prints TODO steps for real EL0 bring-up

That makes it a boot-time syscall smoke test, not a real exception-level transition test.

## What Is Already Useful

Even though the live boot path stays in EL1, the existing scaffolding is still valuable:

- `UserProcess` defines the shape of a future EL0 process object
- `switch_to_el0()` captures the intended register transition
- `linux_syscall()` provides a future EL0-side calling convention
- `PageTableManager` already has the necessary mapping helpers

## What Still Needs To Happen For True EL0 Execution

To move from scaffolding to real user-mode execution, the kernel still needs to:

1. create an actual user process during boot
2. map user code/data/heap/stack into TTBR0-backed tables
3. load a real EL0 entry point and stack
4. call `switch_to_el0()`
5. route `svc` exceptions through a fully correct EL0 handler
6. make syscall numbering and return handling consistent across the tree
7. enforce process-owned handles and memory rather than direct kernel calls

## Bottom Line

The current tree has meaningful EL0 groundwork, but the live system boundary is still:

```text
kernel code
  -> scheduler thread
  -> shell and test helpers
```

not:

```text
EL0 process
  -> SVC
  -> EL1 syscall dispatch
  -> ERET back to EL0
```
