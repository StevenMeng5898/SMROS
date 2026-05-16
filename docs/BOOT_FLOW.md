# SMROS Boot Flow

This document describes the boot path implemented in the current source tree.

## High-Level Flow

```text
QEMU
  -> _start in src/main.rs
  -> kernel_main()
  -> subsystem initialization
  -> user-level VirtIO driver init
  -> FxFS mount or block-image load
  -> logical SMP bring-up
  -> demo process creation
  -> run_user_test()
  -> switch_to_el0()
  -> user_test_process_entry() at EL0
  -> svc #0
  -> exception_handler in src/main.rs
  -> handle_syscall_simple()
  -> sys_exit()
  -> prepare_el0_test_kernel_return()
  -> el0_test_resume() at EL1
  -> start_user_shell()
  -> start_first_thread()
  -> smros> prompt
```

`run_user_test()` is no longer just a kernel-mode smoke test. The boot path now performs a real EL0 drop, issues Linux-style `svc #0` calls, returns through the active EL1 exception path, and then resumes kernel boot.

## 1. QEMU Handoff

SMROS is normally started with:

```bash
./scripts/setup-qemu-icmp.sh --ensure
```

```bash
qemu-system-aarch64 \
  -M virt \
  -cpu cortex-a57 \
  -smp 4 \
  -m 512M \
  -nographic \
  -kernel kernel8.img \
  -drive file=smros-fxfs.img,if=none,format=raw,id=fxfs,cache=writethrough \
  -device virtio-blk-device,drive=fxfs \
  -netdev user,id=smrosnet \
  -device virtio-net-device,netdev=smrosnet
```

The `make run`, `make debug`, `make gdb`, `scripts/run.sh`, and
`scripts/run-simple.sh` launch paths run that setup step automatically on Linux
hosts so QEMU user networking can pass external ICMP echo traffic.

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
6. Branches to `kernel_main()`.

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

- `timer_interrupt_handler()`
- `check_preemption()`

and then return with `eret`.

The synchronous exception handler:

1. Saves general-purpose registers.
2. Reads `ESR_EL1`.
3. Recognizes `EC = 0x15` as an AArch64 `svc`.
4. Loads the syscall number from saved `x8`.
5. Loads syscall arguments from saved `x0` through `x5`.
6. Calls `handle_syscall_simple()`.
7. Writes the result back to the saved `x0` slot.
8. Calls `syscall_should_advance_elr()`.
9. Returns with `eret`.

By default, `syscall_should_advance_elr()` returns `0`, so the handler does not add 4 to `ELR_EL1`. AArch64 already reports `ELR_EL1` after the `svc` instruction for this path. The hook remains so tests can opt into manual advancement if needed.

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
| 12 | `crate::user_level::init()` | user-process state, user-level VirtIO drivers, FxFS, host-share snapshot, component framework, and `/svc` init |
| 13 | `scheduler().init()` | Creates idle thread and resets scheduler state |
| 14 | `component::start_boot_component_threads()` | Creates scheduler launcher threads for bootstrap components |
| 15 | `interrupt::enable_timer_interrupt()` | Logical enable step |
| 16 | clear `DAIF.I` | Unmasks IRQs on CPU0 |

After this table, `kernel_main()` brings up logical SMP state, creates demo process records, prints process status, and calls `crate::user_level::user_test::run_user_test()`. That call does not return to `kernel_main()`; the user-test module continues the boot flow after the EL0 test completes.

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

The remaining flow is owned by `src/user_level/apps/user_test.rs`:

1. `run_user_test()` prepares EL0 test state.
2. It allocates a dedicated 8 KiB EL0 stack.
3. It records `el0_test_resume` as the EL1 resume address.
4. It marks the EL0 test active.
5. It calls `switch_to_el0(user_test_process_entry, stack_top, 0)`.
6. `user_test_process_entry()` runs at EL0 and issues `write`, `getpid`, `mmap`, and `exit` syscalls.
7. `sys_exit()` calls `prepare_el0_test_kernel_return()`.
8. `prepare_el0_test_kernel_return()` rewrites `ELR_EL1` to `el0_test_resume` and sets `SPSR_EL1` for EL1h with interrupts masked.
9. `eret` resumes at `el0_test_resume()`.
10. `el0_test_resume()` prints the EL0 validation result.
11. `finish_boot_after_user_test()` starts the shell thread and jumps into the scheduler.

## 5. EL0 Transition

The active EL0 transition helper is `switch_to_el0()` in `src/user_level/apps/user_process.rs`.

It performs:

1. `TTBR0_EL1 = ttbr0`
2. TLB invalidation and barriers
3. `SP_EL0 = user_stack`
4. `ELR_EL1 = entry_point`
5. `SPSR_EL1 = 0`, selecting EL0t with interrupts enabled
6. `eret`

The boot-time test currently passes `ttbr0 = 0`, so this validates the exception/syscall path and stack transition, but it does not yet run a fully isolated user address space with a process-specific page table.

## 6. User-Level Drivers, FxFS, And Bootstrap ELF Loading

`user_level::init()` first initializes the user process table and then probes user-level VirtIO-MMIO drivers. On the standard `make run` QEMU command this binds:

- virtio-blk backed by `smros-fxfs.img`
- virtio-net backed by QEMU user networking

The FxFS-shaped store uses the block device when available. It loads the newest valid image slot from `smros-fxfs.img`; if the image is empty, it creates a fresh root tree. `make clean` keeps that image, while `make clean-fxfs` removes it.

During `user_level::init()`, the FxFS-shaped store is mounted and the component manager installs three boot package files:

- `/pkg/bin/component_manager`
- `/pkg/bin/fxfs`
- `/pkg/bin/user-init`

Those files are tiny ELF64 little-endian AArch64 images generated by `src/user_level/services/elf.rs`. The parser validates ELF magic, class/data/version, `ET_EXEC`/`ET_DYN`, `EM_AARCH64`, header size, program-header table bounds, entry address, and PT_LOAD segment bounds. Component start reads the ELF from FxFS, records the parsed entry and segment metadata in `UserProcess`, and exposes that metadata through the `components` shell command.

The current boot ELFs point at the existing SMROS EL0 trampoline. Segment bytes are not yet copied into process-owned TTBR0 mappings, so this is a minimal loader and launch-record stage rather than full external binary execution.

FxFS paths resolve through explicit directory entries backed by object ids. File and directory objects carry mode, uid/gid, size, timestamps, and link count. The shell smoke path also exercises append, truncate, seek/read, attribute lookup, and journal replay metadata.

The repository-local `host_shared/` directory is embedded at build time and installed at `/shared`. It is a snapshot, not a live host mount. On each FxFS mount the snapshot is refreshed while persisted deletion tombstones in `/config/host-share-deleted` keep deleted snapshot files hidden across reboot with the same `smros-fxfs.img`.

The same initialization registers a minimal `/svc` directory with component-manager, ELF-runner, and FxFS services. Connections allocate Zircon channel pairs, and the first IPC layer uses fixed 32-byte request/reply structs rather than full FIDL encoding.

The shell `run` command is separate from the bootstrap component loader. It reads a dynamic PIE ELF from FxFS, resolves `PT_INTERP` and `DT_NEEDED` libraries from `/shared/lib` or `/lib`, maps the main executable and dynamic loader into the Linux mmap window, builds a Linux argv/env/auxv stack, and enters the loader at EL0. This is a working bring-up path for simple external AArch64 binaries, but it still uses the identity-mapped model rather than a process-owned TTBR0 address space.

## 7. Active EL0 Syscall Test

`user_test_process_entry()` runs at EL0 and checks:

- `write(1, banner)` returns the banner length
- `getpid()` returns `1`
- `mmap(4096)` returns an address in `0x5000_0000..0x6000_0000`
- the mapped address is 4 KiB aligned
- final status writes return their expected lengths
- `exit(0)` returns control to EL1 boot code

`handle_syscall_simple()` records kernel-observed results while the EL0 test is active:

- first `write()` result
- `getpid()` result
- `mmap()` result

`el0_test_resume()` compares the EL0-observed exit code with the kernel-observed syscall results and prints either:

```text
[EL0] Real EL0 -> SVC -> EL1 validation: SUCCESS
```

or:

```text
[EL0] Real EL0 -> SVC -> EL1 validation: FAIL
```

## 8. SMP Behavior in the Current Tree

The current SMP code supports two layers:

- PSCI and secondary CPU entry scaffolding in `src/kernel_lowlevel/smp.rs`
- a logical 4-CPU scheduling model used by `boot_all_cpus()`

In the current boot path, `boot_all_cpus()` marks all four logical CPUs online for scheduling and status reporting. That is enough for the current demo flow.

## 9. Scheduler Handoff

The scheduler handoff happens after the EL0 test returns to EL1:

1. `finish_boot_after_user_test()` calls `start_user_shell()`.
2. `start_user_shell()` creates a thread whose entry point is `shell_thread_wrapper`.
3. `finish_boot_after_user_test()` calls `start_first_thread()`.
4. `start_first_thread()` finds the first ready non-idle thread.
5. It marks that thread as running.
6. It jumps into the new thread with `context_switch_start`.

The context switch code lives in `src/kernel_lowlevel/context_switch.S`.

## 10. Timer and Preemption Hooks

The current timer IRQ path calls:

- `timer_interrupt_handler()`
- `scheduler().on_timer_tick()`
- `check_preemption()`

`check_preemption()` asks the scheduler whether the current thread should yield and then uses `schedule_on_cpu()` when needed.

## 11. Active Syscall Entry Point

There are multiple syscall helper files under `src/syscall/`, but the path used by the live exception vectors is:

```text
exception_handler in src/main.rs
  -> handle_syscall_simple()
  -> dispatch_linux_syscall() or dispatch_zircon_syscall()
  -> concrete sys_* implementation
```

Important consequences:

- syscall numbers below `1000` use the Linux dispatch table
- syscall numbers from `1000` through `1000 + u32::MAX` use the Zircon dispatch table after subtracting `1000`
- `handle_svc_exception_from_el0()` exists, but is not the handler used by the current assembly
- `sys_exit()` has a special boot-test hook through `prepare_el0_test_kernel_return()`

## 12. Current User/Kernel Reality

The boot-time syscall test now runs real EL0 code. The shell does not.

Today:

- `run_user_test()` drops into EL0 and validates the active `svc #0` path.
- `user_test_process_entry()` is the active EL0 test payload.
- `start_user_shell()` still creates a normal kernel scheduler thread.
- `UserShell` still interacts with serial and kernel data structures directly.
- `UserProcess` records minimal ELF load metadata for bootstrap components, but mapped user text still uses the lightweight trampoline path.
- `run_elf` can run a dynamic PIE ELF through the dynamic loader, but it is still not a fully isolated process model.

## 13. Observable Boot Milestones

A normal QEMU boot currently prints these milestones before reaching the prompt:

1. Kernel banner
2. GIC and timer initialization
3. SMP initialization
4. Memory, syscall, MMU, channel, user-level, and scheduler initialization
5. VirtIO block/net probe and FxFS mount status
6. Demo process creation
7. EL0 test setup messages
8. EL0 syscall-test output
9. EL1 resume and validation summary
10. Shell startup messages
11. `smros>` prompt

After the prompt, the shell `testsc` command runs the broader developer smoke suite. Current successful runs cover Linux memory/process/time, signal, IPC, networking, misc, file, directory, fd, poll, and stat paths; Zircon VMO/VMAR, handle/object, signal/wait, port, channel, socket, FIFO, futex, process/thread, time/debug/system/exception, and hypervisor paths; and the minimal component framework, ELF loader, FxFS-shaped object-store scaffold, and `/svc` fixed-message IPC layer.

## Known Gaps

- The shell is still an EL1 scheduler thread despite the "User-Mode Shell" banner.
- The boot-time EL0 test validates syscall transition mechanics, but not a fully isolated user address space.
- The active exception path bypasses the more elaborate `handle_svc_exception_from_el0()` helper.
- Zircon calls are routed through the live SVC path, but process/thread ownership and handle rights are still simplified kernel-side models.
- Linux file and directory syscalls are modeled through compatibility objects; FxFS-backed file descriptors now support open/read/write/stat and file-backed `mmap`, but this is not a complete VFS.
- `/shared` is a build-time host snapshot, not live 9p or virtio-fs sharing.
- The Fuchsia-inspired userspace layer parses boot component ELF images, creates scheduler launcher threads that enter EL0 for bootstrap component payloads, and has a fixed-message `/svc` IPC layer, but it is still not a full component-manager/FIDL/package-resolver/FxFS port.
