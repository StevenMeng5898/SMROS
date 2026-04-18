# SMROS Boot Flow

This document describes the boot path implemented in the current source tree.

## High-Level Flow

```text
QEMU
  -> _start in src/main.rs
  -> kernel_main()
  -> subsystem initialization
  -> logical SMP bring-up
  -> demo process creation
  -> run_user_test()
  -> start_user_shell()
  -> start_first_thread()
  -> smros> prompt
```

## 1. QEMU Handoff

SMROS is normally started with:

```bash
qemu-system-aarch64 \
  -M virt \
  -cpu cortex-a57 \
  -smp 4 \
  -m 512M \
  -nographic \
  -kernel kernel8.img
```

QEMU enters the kernel at `_start`, which is emitted by the `global_asm!` block in `src/main.rs`.

## 2. Early Assembly Path

The boot assembly in `src/main.rs` performs two different paths.

### CPU0 Path

CPU0:

1. Reads `MPIDR_EL1`.
2. Masks interrupts through `DAIF`.
3. Sets `SP` to `__stack_top`.
4. Clears the `.bss` range.
5. Loads `VBAR_EL1` with `exception_vectors`.
6. Branches to `kernel_main`.

### Secondary CPU Path

Non-zero CPUs:

1. Read `MPIDR_EL1`.
2. Reuse the stack pointer passed from PSCI context.
3. Load `VBAR_EL1`.
4. Branch to `secondary_cpu_entry`.

### Exception Vectors

The same `global_asm!` block defines:

- `exception_vectors`
- IRQ handlers for current EL and lower EL
- the synchronous exception handler

The IRQ handlers save caller-saved registers, call:

- `timer_interrupt_handler`
- `check_preemption`

and then return with `eret`.

The synchronous exception handler:

1. Saves general-purpose registers.
2. Reads `ESR_EL1`.
3. Recognizes `EC = 0x15` as an AArch64 `svc`.
4. Calls `handle_syscall_simple`.
5. Writes the result back to the saved `x0` slot.
6. Advances `ELR_EL1` by 4 bytes.
7. Returns with `eret`.

## 3. `kernel_main()` Initialization Order

`kernel_main()` in `src/main.rs` currently performs initialization in this order:

| Order | Call | Notes |
|------:|------|-------|
| 1 | `Serial::new().init()` | Enables the PL011 console |
| 2 | banner + version print | Kernel version is `0.2.0` |
| 3 | `print_system_info()` | Prints `MPIDR_EL1` and `SCTLR_EL1` |
| 4 | `kernel_lowlevel::interrupt::init()` | GICv2 setup |
| 5 | `kernel_lowlevel::timer::init()` | ARM generic timer setup |
| 6 | `kernel_lowlevel::smp::init()` | SMP bookkeeping and CPU0 registration |
| 7 | `kernel_lowlevel::memory::init()` | Process manager and page allocator setup |
| 8 | `crate::syscall::init()` | Logged as "syscall interface" |
| 9 | `kernel_lowlevel::mmu::init()` | MMU/page-table helper initialization |
| 10 | `crate::syscall::init()` | Called again, logged as "syscall handler" |
| 11 | `crate::kernel_objects::channel::init()` | Channel subsystem init log |
| 12 | `crate::user_level::user_process::init()` | User-process scaffolding init log |
| 13 | `scheduler().init()` | Creates idle thread and resets scheduler state |
| 14 | `interrupt::enable_timer_interrupt()` | Logical enable step |
| 15 | clear `DAIF.I` | Unmasks IRQs on CPU0 |

## 4. Runtime Setup After Initialization

After the initial subsystem bring-up, `kernel_main()` continues with:

1. `boot_all_cpus()`
2. `smp_print_status()`
3. demo process creation through `process_manager()`:
   - `shell`
   - `editor`
   - `compiler`
4. `process_manager().print_status(...)`
5. `run_user_test()`
6. `start_user_shell()`
7. `start_first_thread()`

## 5. SMP Behavior in the Current Tree

The current SMP code supports two layers:

- PSCI and secondary CPU entry scaffolding in `src/kernel_lowlevel/smp.rs`
- a logical 4-CPU scheduling model used by `boot_all_cpus()`

In the current boot path, `boot_all_cpus()` marks all four logical CPUs online for scheduling and status reporting. That is enough for the current demo flow, even though the documented EL1/EL0 separation work is still in progress.

## 6. Scheduler Handoff

The scheduler handoff is:

1. `start_user_shell()` creates a thread whose entry point is `shell_thread_wrapper`.
2. `start_first_thread()` finds the first ready non-idle thread.
3. It marks that thread as running.
4. It jumps into the new thread with `context_switch_start`.

The context switch code lives in `src/kernel_lowlevel/context_switch.S`.

## 7. Timer and Preemption Hooks

The current timer IRQ path calls:

- `timer_interrupt_handler()`
- `scheduler().on_timer_tick()`
- `check_preemption()`

`check_preemption()` asks the scheduler whether the current thread should yield and then uses `schedule_on_cpu()` when needed.

That means the codebase contains the machinery for time-slice driven scheduling, even though the broader user-mode story is still incomplete.

## 8. Current User/Kernel Reality

The live boot path is not yet a true EL0 boot path.

Today:

- `run_user_test()` executes from kernel mode and directly calls `sys_getpid()` and `sys_mmap()`.
- `start_user_shell()` creates a normal kernel thread.
- `UserShell` interacts with serial and kernel data structures directly.

Scaffolding for EL0 exists in `src/user_level/user_process.rs` and `src/user_level/user_test.rs`, but the normal boot path does not currently call `switch_to_el0()`.

## 9. Active Syscall Entry Point

There are multiple syscall helper files under `src/syscall/`, but the path used by the live exception vectors is the simple one:

```text
exception_handler in src/main.rs
  -> handle_syscall_simple()
  -> dispatch_linux_syscall()
```

Important consequences:

- the active `svc` bridge is Linux-style only
- syscall numbers `>= 1000` are not routed to Zircon in the live SVC path
- `handle_svc_exception_from_el0()` exists, but is not the handler used by the current assembly

## 10. Observable Boot Milestones

A normal QEMU boot currently prints these milestones before reaching the prompt:

1. Kernel banner
2. GIC and timer initialization
3. SMP initialization
4. Memory, syscall, MMU, channel, and scheduler initialization
5. Demo process creation
6. Boot-time user test output
7. Shell startup messages
8. `smros>` prompt

## Known Gaps

- The shell is still an EL1 thread despite the "User-Mode Shell" banner.
- `run_user_test()` is a kernel-mode smoke test, not a full EL0 execution path.
- The active exception path bypasses the more elaborate `handle_svc_exception_from_el0()` helper.
- Some status output still contains garbled characters.
