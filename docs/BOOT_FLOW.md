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
  -> start_user_shell()
  -> start_first_thread()
  -> smros> prompt
```

Normal boot is intentionally short: it no longer creates demo process records,
starts bootstrap EL0 launcher threads, or runs the boot-time EL0 syscall smoke
test before the shell. Run `testsc` from the shell when you want the heavier
syscall and service validation.

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
| 8 | `crate::kernel_objects::init()` | Installs kernel object rights config |
| 9 | `crate::syscall::init()` | Logged as "syscall interface" |
| 10 | `kernel_lowlevel::mmu::init()` | MMU/page-table helper initialization |
| 11 | `crate::syscall::init()` | Called again, logged as "syscall handler" |
| 12 | `crate::kernel_objects::channel::init()` | Channel subsystem init log |
| 13 | `crate::user_level::init()` | user-process state, user-level VirtIO drivers, FxFS, build-time `/shared` snapshot, component topology, and `/svc` init |
| 14 | `scheduler().init()` | Creates idle thread and resets scheduler state |
| 15 | defer bootstrap component launchers | Keeps normal boot on the fast path |
| 16 | `interrupt::enable_timer_interrupt()` | Logical enable step |
| 17 | clear `DAIF.I` | Unmasks IRQs on CPU0 |

After this table, `kernel_main()` brings up logical SMP state, starts the shell
scheduler thread, and jumps into the scheduler. Demo process creation and the
boot-time EL0 syscall test are no longer part of normal boot.

## 4. Runtime Setup After Initialization

After the initial subsystem bring-up, `kernel_main()` continues with:

1. `boot_all_cpus()`
2. `smp_print_status()`
3. `start_user_shell()`
4. `start_first_thread()`

The shell reaches the prompt first. Broader syscall, component, FxFS, `/svc`,
Gemma, Hermes, Docker, and QML cluster checks are explicit shell commands,
primarily through `testsc` and the service-specific `test` commands.

## 5. EL0 Transition

The active EL0 transition helper is `switch_to_el0()` in
`src/user_level/apps/user_process.rs`. It is still used by shell-launched ELF
programs and component launcher paths, but not by default boot.

It performs:

1. `TTBR0_EL1 = ttbr0`
2. TLB invalidation and barriers
3. `SP_EL0 = user_stack`
4. `ELR_EL1 = entry_point`
5. `SPSR_EL1 = 0`, selecting EL0t with interrupts enabled
6. `eret`

The old boot-time test passed `ttbr0 = 0`; keeping that validation out of the
default boot avoids paying its startup cost before the shell.

## 6. User-Level Drivers, FxFS, And Bootstrap ELF Loading

`user_level::init()` first initializes the user process table and then probes user-level VirtIO-MMIO drivers. On the standard `make run` QEMU command this binds:

- virtio-blk backed by `smros-fxfs.img`
- virtio-net backed by QEMU user networking

The FxFS-shaped store uses the block device when available. It loads the newest valid image slot from `smros-fxfs.img`; if the image is empty, it creates a fresh root tree. New run targets create a 128 MiB image, large enough for staged Docker archives and extracted rootfs metadata. `make clean` keeps that image, while `make clean-fxfs` removes it.

During `user_level::init()`, the FxFS-shaped store is mounted and the component manager installs three boot package files:

- `/pkg/bin/component_manager`
- `/pkg/bin/fxfs`
- `/pkg/bin/user-init`

Those files are tiny ELF64 little-endian AArch64 images generated by `src/user_level/services/elf.rs`. The parser validates ELF magic, class/data/version, `ET_EXEC`/`ET_DYN`, `EM_AARCH64`, header size, program-header table bounds, entry address, and PT_LOAD segment bounds. Component start reads the ELF from FxFS, records the parsed entry and segment metadata in `UserProcess`, and exposes that metadata through the `components` shell command.

The current boot ELFs point at the existing SMROS EL0 trampoline. Segment bytes are not yet copied into process-owned TTBR0 mappings, so this is a minimal loader and launch-record stage rather than full external binary execution.

FxFS paths resolve through explicit directory entries backed by object ids. File and directory objects carry mode, uid/gid, size, timestamps, and link count. The shell smoke path also exercises append, truncate, seek/read, attribute lookup, and journal replay metadata.

The repository-local `host_shared/` directory is embedded at build time and installed under `/shared` during FxFS initialization. `share` lists that snapshot, and `mount share` refreshes it from the embedded data; neither command uses a live host mount. Persisted deletion tombstones in `/config/host-share-deleted` keep deleted snapshot files hidden across reboot with the same `smros-fxfs.img`.

The same initialization registers a minimal `/svc` directory with component-manager, ELF-runner, and FxFS services. Connections allocate Zircon channel pairs, and the first IPC layer uses fixed 32-byte request/reply structs rather than full FIDL encoding.

The shell `run` command is separate from the bootstrap component loader. It reads a dynamic PIE ELF from FxFS, resolves `PT_INTERP` and `DT_NEEDED` libraries from `/shared/lib` or `/lib`, maps the main executable and dynamic loader into the Linux mmap window, builds a Linux argv/env/auxv stack, and enters the loader at EL0. This is a working bring-up path for simple external AArch64 binaries, but it still uses the identity-mapped model rather than a process-owned TTBR0 address space.

## 7. Explicit EL0 And Syscall Tests

The previous default boot path ran `user_test_process_entry()` at EL0 before
starting the shell. That test is now kept out of normal boot for latency. The
EL0 helper code remains in `src/user_level/apps/user_test.rs`, and the broader
developer validation path is the shell `testsc` command.

`testsc` exercises Linux and Zircon syscall helpers, memory objects, IPC
objects, component metadata, FxFS, `/svc`, compatibility apps, Docker/runc
surfaces, Gemma, Hermes, and the QML cluster service.

## 8. SMP Behavior in the Current Tree

The current SMP code supports two layers:

- PSCI and secondary CPU entry scaffolding in `src/kernel_lowlevel/smp.rs`
- a logical 4-CPU scheduling model used by `boot_all_cpus()`

In the current boot path, `boot_all_cpus()` marks all four logical CPUs online for scheduling and status reporting. That is enough for the current demo flow.

## 9. Scheduler Handoff

The scheduler handoff happens directly from `kernel_main()`:

1. `start_user_shell()` creates a thread whose entry point is `shell_thread_wrapper`.
2. `start_first_thread()` finds the first ready non-idle thread.
3. It marks that thread as running.
4. It jumps into the new thread with `context_switch_start`.

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
- `sys_exit()` still has an EL0 test hook through `prepare_el0_test_kernel_return()`, but normal boot does not activate it

## 12. Current User/Kernel Reality

Normal boot starts the shell without running the EL0 syscall test. The shell itself still does not run in EL0.

Today:

- `start_user_shell()` creates a normal kernel scheduler thread.
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
6. Shell startup messages
7. `smros>` prompt

After the prompt, the shell `testsc` command runs the broader developer smoke suite. Current successful runs cover Linux memory/process/time, signal, IPC, networking, misc, file, directory, fd, poll, and stat paths; Zircon VMO/VMAR, handle/object, signal/wait, port, channel, socket, FIFO, futex, process/thread, time/debug/system/exception, and hypervisor paths; and the minimal component framework, ELF loader, FxFS-shaped object-store scaffold, and `/svc` fixed-message IPC layer.

## Known Gaps

- The shell is still an EL1 scheduler thread despite the "User-Mode Shell" banner.
- The EL0 syscall smoke helpers validate transition mechanics when run explicitly, but they are not part of the normal fast boot and do not provide a fully isolated user address space.
- The active exception path bypasses the more elaborate `handle_svc_exception_from_el0()` helper.
- Zircon calls are routed through the live SVC path, but process/thread ownership and handle rights are still simplified kernel-side models.
- Linux file and directory syscalls are modeled through compatibility objects; FxFS-backed file descriptors now support open/read/write/stat and file-backed `mmap`, but this is not a complete VFS.
- `/shared` is a build-time host snapshot, not live 9p or virtio-fs sharing.
- The Fuchsia-inspired userspace layer parses boot component ELF images and can create scheduler launcher threads that enter EL0 for bootstrap component payloads when requested; it has a fixed-message `/svc` IPC layer, but it is still not a full component-manager/FIDL/package-resolver/FxFS port.
