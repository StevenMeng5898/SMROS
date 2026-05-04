# User / Kernel Boundary: Current Implementation

This document describes the actual state of user/kernel separation in the current tree.

The short version is:

- EL0 scaffolding exists
- MMU and page-table helpers exist
- an `svc` bridge exists
- the boot path runs a real EL0 syscall smoke test
- the live shell still runs from EL1

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
3. `run_user_test()` allocates a small EL0 stack and calls `switch_to_el0()`.
4. `user_test_process_entry()` runs at EL0 and issues Linux `svc #0` calls.
5. `sys_exit()` redirects the return path to `el0_test_resume()` in EL1.
6. `finish_boot_after_user_test()` calls `start_user_shell()`.
7. `start_user_shell()` creates a normal scheduler thread.
8. `start_first_thread()` jumps into that EL1 shell thread.

No part of the normal boot path:

- creates a real EL0 process and runs it
- executes `user_shell_entry()` after an EL transition
- installs a process-specific TTBR0 page table for the EL0 test
- enforces per-process handle ownership for the shell

## Current State By Area

| Area | Current State | Notes |
|------|---------------|-------|
| Page-table manager | present | real scaffolding in `mmu.rs` |
| User-process data model | present | `UserProcess` exists |
| EL0 transition helper | present | `switch_to_el0()` exists |
| Live shell in EL0 | not active | shell runs as EL1 thread |
| Live test process in EL0 | active | boot test drops to EL0 and returns through the active exception path |
| Full register-frame EL0 syscall handler | not active | current vectors use `handle_syscall_simple()` |
| Zircon-on-SVC path | active | raw syscall numbers `1000 + zircon_number` route through `dispatch_zircon_syscall()` |

## Why The Shell Is Not Yet A Real User Process

The shell source still contains future-facing comments about EL0. In the current implementation:

- `start_user_shell()` uses `scheduler().create_thread(...)`
- the shell thread executes with normal kernel thread context
- shell commands call kernel services directly
- serial input is polled directly from PL011 registers

So the shell is currently a kernel-resident diagnostic shell, not an isolated user process.

## What The Boot-Time EL0 Test Does

`run_user_test()` in `src/user_level/user_test.rs` currently:

- prints `[EL0]` log prefixes
- prepares a dedicated 8 KiB EL0 stack
- marks the EL0 test active for syscall-result recording
- calls `switch_to_el0(user_test_process_entry, stack_top, 0)`
- uses `svc #0` from EL0 for Linux `write`, `getpid`, `mmap`, and `exit`
- resumes kernel boot through `prepare_el0_test_kernel_return()` and `el0_test_resume()`

That makes it a real exception-level transition test. It is still not a fully isolated user process because the test uses the lightweight `ttbr0 = 0` setup and does not install a process-owned address space.

## What Is Already Useful

The existing scaffolding is useful for the next step toward a real userspace:

- `UserProcess` defines the shape of a future EL0 process object
- `switch_to_el0()` captures the intended register transition
- `linux_syscall()` is already used by the boot-time EL0 smoke test
- `PageTableManager` already has the necessary mapping helpers
- the active exception vector already routes Linux numbers below `1000` and Zircon numbers at `1000 + zircon_number`

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

The current tree has a working EL0 syscall smoke path, but not a fully isolated userspace runtime. The shell boundary is still:

```text
kernel code
  -> scheduler thread
  -> shell and shell-level test helpers
```

The boot-time smoke path is:

```text
EL0 test payload
  -> SVC
  -> EL1 syscall dispatch
  -> EL1 resume hook
```

What is still missing is the full process model:

```text
EL0 process
  -> SVC
  -> EL1 syscall dispatch
  -> ERET back to EL0
```
