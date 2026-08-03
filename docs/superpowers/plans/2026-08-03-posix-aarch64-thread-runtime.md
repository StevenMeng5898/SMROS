# AArch64 POSIX Thread Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Execute real AArch64 `CLONE_THREAD` children with independent TLS and signal state, scheduler-backed futex blocking, directed signal delivery, and clean pthread exit/join semantics.

**Architecture:** A fixed-capacity Linux task table binds monotonically allocated TIDs to existing scheduler `ThreadId` values while all tasks share the active ELF address space on physical CPU0. AArch64 exception frames and scheduler contexts preserve complete EL0 state; suspended clone children are published only after validation and TID copy-out. Separate fixed-capacity futex and signal queues move blocked tasks through one task/scheduler lifecycle API.

**Tech Stack:** Rust `no_std`, AArch64 assembly, fixed-capacity kernel tables, Cargo host tests, source-level integration contracts, Verus-compatible shared logic, Open POSIX Test Suite, AArch64 GNU cross-toolchain, FxFS, QEMU system emulation, host coverage, and optional Coverity analysis.

---

## File Map

- Create `src/syscall/linux_task_logic_shared.rs`: pure clone validation, TID allocation, task lifecycle, and lookup rules shared with host tests and Verus.
- Create `src/syscall/linux_task.rs`: production fixed-capacity task records, clone startup state, per-task signal state, and task/scheduler transitions.
- Create `src/syscall/linux_futex_logic_shared.rs`: pure futex command decoding, waiter matching, FIFO selection, and deadline rules.
- Create `src/syscall/linux_futex.rs`: production futex queues, blocking, waking, timeout, interruption, and clear-child-TID integration.
- Create `src/syscall/linux_syscall_context.rs`: per-CPU bounded ownership of the active AArch64 syscall frame.
- Create `src/kernel_lowlevel/ARM64/context_shared.rs`: C-layout exception and scheduler context structures plus assembly offsets.
- Modify `src/kernel_lowlevel/ARM64/thread.rs`: consume the shared context layout and expose clone-child transfer.
- Modify `src/kernel_lowlevel/ARM64/context_switch.S`: save and restore system registers, FP status, and all SIMD registers; add clone-child EL0 transfer.
- Modify `src/kernel_lowlevel/ARM64/boot.rs`: save full exception frames, pass their address to syscall dispatch, and schedule at the lower-EL timer return boundary.
- Modify `src/kernel_lowlevel/ARM64/cpu.rs`: expose checked accessors for `SPSR_EL1`, `TPIDR_EL0`, and exception return state.
- Modify `src/kernel_objects/scheduler_logic_shared.rs`: pure suspended/blocked/wake/termination transition rules.
- Modify `src/kernel_objects/scheduler.rs`: suspended thread creation, publication, targeted block/wake/termination, and deferred retirement.
- Modify `src/main.rs`: expire Linux task waits on CPU0 before the timer preemption decision.
- Modify `src/syscall/mod.rs`: register the new task, futex, and syscall-context modules.
- Modify `src/syscall/syscall_dispatch.rs`: bind each Linux syscall dispatch to its saved AArch64 frame.
- Modify `src/syscall/syscall.rs`: replace synthetic thread, futex, TID, directed-signal, signal-wait, and child-exit behavior.
- Modify `src/user_level/services/run_elf.rs`: register the root Linux task immediately before entering EL0.
- Modify `tests/host/src/lib.rs`: exercise all production-shared task, futex, scheduler, and AArch64 layout logic.
- Modify `tests/host/tests/integration_contracts.rs`: lock cross-module assembly, syscall, cleanup, and signal ownership contracts.
- Modify `verification/syscall/src/lib.rs`: include and verify the new pure task and futex rules.
- Create `docs/posix/2026-08-03-aarch64-thread-runtime-results.md`: record commit-matched canary, group, full-suite, coverage, and static-analysis evidence.

This plan changes only AArch64 shared-address-space threads. It does not add
fork/COW/wait semantics or claim x86_64 and RISC-V64 thread support.

### Task 1: Add The Shared Linux Task Lifecycle

**Files:**
- Create: `src/syscall/linux_task_logic_shared.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `verification/syscall/src/lib.rs`

- [ ] **Step 1: Write failing task-table host tests**

Add a `linux_task_logic` module to `tests/host/src/lib.rs` that includes the new
shared file. Add tests for root registration, monotonically allocated child
TIDs, scheduler-ID lookup, suspended publication, blocked/wake transitions,
rollback, slot reuse without TID reuse, stale identity rejection, and
fail-closed allocator exhaustion.

```rust
mod linux_task_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/syscall/linux_task_logic_shared.rs"
    ));

    #[test]
    fn task_slots_publish_atomically_and_tid_values_do_not_reuse() {
        let mut tasks = LinuxTaskTable::<3>::new();
        assert_eq!(tasks.register_root(7), Ok(LINUX_ROOT_TID));

        let first = tasks.reserve_child(8).expect("first child reservation");
        assert_eq!(first.tid, 2);
        assert_eq!(tasks.by_tid(first.tid), None);
        assert!(tasks.publish(first));
        assert_eq!(tasks.by_scheduler(8).map(|task| task.tid), Some(2));

        assert!(tasks.exit(first.tid, 8));
        assert!(tasks.retire(first.tid, 8));
        let second = tasks.reserve_child(9).expect("reused table slot");
        assert_eq!(second.tid, 3);
        assert_ne!(first.tid, second.tid);
        assert!(!tasks.publish(first), "stale reservation must not publish");
    }

    #[test]
    fn task_state_and_scheduler_identity_move_together() {
        let mut tasks = LinuxTaskTable::<3>::new();
        tasks.register_root(7).unwrap();
        let child = tasks.reserve_child(8).unwrap();
        assert!(tasks.publish(child));
        assert!(tasks.block(child.tid, 8, LinuxBlockReason::Futex));
        assert_eq!(tasks.by_tid(child.tid).unwrap().state, LinuxTaskState::Blocked);
        assert!(tasks.wake(child.tid, 8));
        assert_eq!(tasks.by_tid(child.tid).unwrap().state, LinuxTaskState::Runnable);
        assert!(!tasks.wake(child.tid, 99));
    }
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

```bash
./scripts/run-host-unit-tests.sh --lib linux_task_logic
```

Expected: compilation fails because `linux_task_logic_shared.rs` and its task
types do not exist.

- [ ] **Step 3: Implement the pure fixed-capacity model**

Define these exact public shared types in
`src/syscall/linux_task_logic_shared.rs`:

```rust
pub(crate) const LINUX_ROOT_TID: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxTaskError {
    Capacity,
    DuplicateRoot,
    Exhausted,
    InvalidTransition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxTaskState {
    Empty,
    Starting,
    Runnable,
    Blocked,
    Exited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxBlockReason {
    None,
    Futex,
    SignalWait,
    SignalSuspend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxTaskReservation {
    pub slot: usize,
    pub tid: usize,
    pub scheduler_thread: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxTaskCore {
    pub tid: usize,
    pub tgid: usize,
    pub scheduler_thread: usize,
    pub state: LinuxTaskState,
    pub block_reason: LinuxBlockReason,
}
```

Implement `LinuxTaskTable<const N: usize>` with `new`, `register_root`,
`reserve_child`, `publish`, `rollback`, `by_tid`, `by_scheduler`, `block`,
`wake`, `exit`, `retire`, and `reset`. A reserved slot remains invisible to
`by_tid` until `publish`. Every transition requires both TID and scheduler ID.
`next_tid.checked_add(1)` failure permanently exhausts allocation and returns
`None`; retiring a slot never decrements or rewinds the allocator.

- [ ] **Step 4: Include the rules in the syscall verification crate**

Add this include beside the existing syscall shared-logic includes:

```rust
include!("../../../src/syscall/linux_task_logic_shared.rs");
```

Add Verus assertions for nonzero root/child TIDs, checked monotonic allocation,
and the rule that only `Starting` tasks can become `Runnable`.

- [ ] **Step 5: Run GREEN and commit**

```bash
./scripts/run-host-unit-tests.sh --lib linux_task_logic
make verus-syscall
git add src/syscall/linux_task_logic_shared.rs tests/host/src/lib.rs verification/syscall/src/lib.rs
git commit -m "feat: add Linux task lifecycle model"
```

Expected: focused host tests and syscall verification pass with zero failures.

### Task 2: Make The AArch64 Context ABI Complete

**Files:**
- Create: `src/kernel_lowlevel/ARM64/context_shared.rs`
- Modify: `src/kernel_lowlevel/ARM64/thread.rs`
- Modify: `src/kernel_lowlevel/ARM64/context_switch.S`
- Modify: `src/kernel_lowlevel/ARM64/boot.rs`
- Modify: `src/kernel_lowlevel/ARM64/cpu.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing layout and assembly contracts**

Include `context_shared.rs` in a host test module and assert the exact ABI:

```rust
#[test]
fn aarch64_exception_and_context_layouts_are_locked() {
    use core::mem::{offset_of, size_of};

    assert_eq!(offset_of!(Aarch64ExceptionFrame, regs), 0x000);
    assert_eq!(offset_of!(Aarch64ExceptionFrame, simd), 0x100);
    assert_eq!(offset_of!(Aarch64ExceptionFrame, fpcr), 0x300);
    assert_eq!(offset_of!(Aarch64ExceptionFrame, fpsr), 0x308);
    assert_eq!(size_of::<Aarch64ExceptionFrame>(), 0x310);

    assert_eq!(offset_of!(CpuContext, sp_el0), 0x110);
    assert_eq!(offset_of!(CpuContext, elr_el1), 0x118);
    assert_eq!(offset_of!(CpuContext, spsr_el1), 0x120);
    assert_eq!(offset_of!(CpuContext, tpidr_el0), 0x128);
    assert_eq!(offset_of!(CpuContext, fpcr), 0x130);
    assert_eq!(offset_of!(CpuContext, fpsr), 0x138);
    assert_eq!(offset_of!(CpuContext, simd), 0x140);
    assert_eq!(size_of::<CpuContext>(), 0x340);
}
```

Add an integration contract requiring `boot.rs` to allocate `0x310` bytes,
save/restore `q0` through `q31`, `fpcr`, and `fpsr` before any Rust call, and
requiring `context_switch.S` to save/restore `sp_el0`, `elr_el1`, `spsr_el1`,
`tpidr_el0`, `fpcr`, `fpsr`, and all SIMD pairs.

- [ ] **Step 2: Run the contracts and verify RED**

```bash
./scripts/run-host-unit-tests.sh --lib aarch64_context_logic
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_el0_context_abi_is_complete -- --exact
```

Expected: the shared layouts are missing and the existing assembly saves only
kernel callee-saved general registers.

- [ ] **Step 3: Define the shared C layouts**

Create `context_shared.rs` with `#[repr(C, align(16))]` structures. Preserve the
existing general-register field order in `CpuContext`, then append the fields
shown below so all asserted offsets remain exact.

```rust
#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default)]
pub struct Aarch64ExceptionFrame {
    pub regs: [u64; 32],
    pub simd: [u128; 32],
    pub fpcr: u64,
    pub fpsr: u64,
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuContext {
    pub x0: u64,
    pub x1: u64,
    pub x2: u64,
    pub x3: u64,
    pub x4: u64,
    pub x5: u64,
    pub x6: u64,
    pub x7: u64,
    pub x8: u64,
    pub x9: u64,
    pub x10: u64,
    pub x11: u64,
    pub x12: u64,
    pub x13: u64,
    pub x14: u64,
    pub x15: u64,
    pub x16: u64,
    pub x17: u64,
    pub x18: u64,
    pub x19: u64,
    pub x20: u64,
    pub x21: u64,
    pub x22: u64,
    pub x23: u64,
    pub x24: u64,
    pub x25: u64,
    pub x26: u64,
    pub x27: u64,
    pub x28: u64,
    pub fp: u64,
    pub lr: u64,
    pub sp: u64,
    pub pc: u64,
    pub pstate: u64,
    pub sp_el0: u64,
    pub elr_el1: u64,
    pub spsr_el1: u64,
    pub tpidr_el0: u64,
    pub fpcr: u64,
    pub fpsr: u64,
    pub simd: [u128; 32],
}
```

Use `include!("context_shared.rs")` from `thread.rs`, remove its duplicate
`CpuContext`, and initialize every appended field to zero.

- [ ] **Step 4: Save complete state before Rust and across switches**

Change every AArch64 IRQ and synchronous exception frame from `0x100` to
`0x310` bytes. Keep GPR offsets `0x000..0x0f0`; save SIMD at
`0x100..0x2f0`, then `fpcr` and `fpsr` at `0x300` and `0x308`. Restore those
values before the GPR restore and `eret`.

Extend `context_switch.S` using the `CpuContext` offsets from Step 3. The save
side must execute these system-register operations before changing TCBs:

```asm
mrs     x17, sp_el0
str     x17, [x16, #0x110]
mrs     x17, elr_el1
str     x17, [x16, #0x118]
mrs     x17, spsr_el1
str     x17, [x16, #0x120]
mrs     x17, tpidr_el0
str     x17, [x16, #0x128]
mrs     x17, fpcr
str     x17, [x16, #0x130]
mrs     x17, fpsr
str     x17, [x16, #0x138]
stp     q0, q1, [x16, #0x140]
stp     q30, q31, [x16, #0x320]
```

Restore all intermediate SIMD pairs and system registers symmetrically. Add
`read_exception_return_state`, `read_user_tls`, and `set_user_tls` helpers in
`cpu.rs` using `mrs/msr spsr_el1` and `mrs/msr tpidr_el0`.

- [ ] **Step 5: Run GREEN, build, and commit**

```bash
./scripts/run-host-unit-tests.sh --lib aarch64_context_logic
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_el0_context_abi_is_complete -- --exact
rustfmt --edition 2021 --check src/kernel_lowlevel/ARM64/thread.rs src/kernel_lowlevel/ARM64/cpu.rs
make build-test ARCH=aarch64-unknown-none
git add src/kernel_lowlevel/ARM64/context_shared.rs src/kernel_lowlevel/ARM64/thread.rs src/kernel_lowlevel/ARM64/context_switch.S src/kernel_lowlevel/ARM64/boot.rs src/kernel_lowlevel/ARM64/cpu.rs tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: preserve complete AArch64 EL0 thread state"
```

Expected: layout tests, assembly contracts, release build, and link-layout
validation pass.

### Task 3: Add Atomic Scheduler Task Transitions

**Files:**
- Modify: `src/kernel_objects/scheduler_logic_shared.rs`
- Modify: `src/kernel_objects/scheduler.rs`
- Modify: `src/kernel_lowlevel/ARM64/boot.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing scheduler transition tests**

Add pure tests proving that only `Blocked` becomes `Ready` on wake, only a
suspended `Blocked` child can be published, an exited task cannot be revived,
and a non-current terminated stack is reclaimable immediately while the
current stack uses deferred retirement.

```rust
assert_eq!(
    smros_sched_wake_transition_body!(3u8, 3u8, 1u8),
    Some(1u8)
);
assert_eq!(smros_sched_wake_transition_body!(1u8, 3u8, 1u8), None);
assert_eq!(smros_sched_wake_transition_body!(4u8, 3u8, 1u8), None);
assert_eq!(
    smros_sched_publish_transition_body!(3u8, true, 3u8, 1u8),
    Some(1u8)
);
assert_eq!(
    smros_sched_publish_transition_body!(3u8, false, 3u8, 1u8),
    None
);
```

Add a source contract for production APIs named
`create_suspended_thread_on_cpu`, `publish_suspended_thread`, `block_thread`,
`wake_thread`, and `terminate_thread`. Require the lower-EL timer IRQ path to
call `check_preemption` only after timer handling and signal delivery.

- [ ] **Step 2: Run RED**

```bash
./scripts/run-host-unit-tests.sh --lib scheduler_logic
./scripts/run-host-unit-tests.sh --test integration_contracts scheduler_exposes_atomic_linux_task_transitions -- --exact
```

Expected: the new helpers and production APIs are absent.

- [ ] **Step 3: Implement suspended creation and targeted transitions**

Refactor `create_thread_on_cpu` through one private allocator that accepts an
initial `ThreadState`. Add these signatures:

```rust
pub fn create_suspended_thread_on_cpu(
    &mut self,
    entry: extern "C" fn() -> !,
    name: &'static str,
    cpu_affinity: usize,
) -> Option<ThreadId>;

pub fn publish_suspended_thread(&mut self, id: ThreadId) -> bool;
pub fn block_thread(&mut self, id: ThreadId) -> bool;
pub fn wake_thread(&mut self, id: ThreadId) -> bool;
pub fn terminate_thread(&mut self, id: ThreadId) -> bool;
```

`publish_suspended_thread` and `wake_thread` accept only `Blocked`.
`block_thread` accepts only `Running` or `Ready`. `terminate_thread` refuses
idle, decrements `active_threads` exactly once, and uses existing deferred
retirement only when `id == current_thread`; a non-current stack is freed after
its TCB is made unreachable.

Keep the shared transition layer type-agnostic by implementing
`smros_sched_wake_transition_body!` and
`smros_sched_publish_transition_body!` macros that receive the concrete state
constants. `scheduler.rs` supplies `ThreadState` values; host and Verus
harnesses supply their existing numeric model constants.

- [ ] **Step 4: Enable safe lower-EL timer preemption**

In `irq_handler_lower`, keep the full exception frame on the current kernel
stack and order calls as follows:

```asm
bl      timer_interrupt_handler
mov     x0, sp
bl      deliver_linux_timer_signal_from_irq
bl      check_preemption
```

Do not call `check_preemption` from the current-EL handlers in this increment.
Linux tasks are pinned to CPU0; other scheduler ownership remains unchanged.

- [ ] **Step 5: Run GREEN and commit**

```bash
./scripts/run-host-unit-tests.sh --lib scheduler_logic
./scripts/run-host-unit-tests.sh --test integration_contracts scheduler_exposes_atomic_linux_task_transitions -- --exact
make build-test ARCH=aarch64-unknown-none
git add src/kernel_objects/scheduler_logic_shared.rs src/kernel_objects/scheduler.rs src/kernel_lowlevel/ARM64/boot.rs tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: add scheduler transitions for Linux tasks"
```

### Task 4: Bind Root Tasks And Saved Syscall Frames

**Files:**
- Create: `src/syscall/linux_task.rs`
- Create: `src/syscall/linux_syscall_context.rs`
- Modify: `src/syscall/mod.rs`
- Modify: `src/syscall/syscall_dispatch.rs`
- Modify: `src/syscall/syscall.rs`
- Modify: `src/kernel_lowlevel/ARM64/boot.rs`
- Modify: `src/user_level/services/run_elf.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing ownership contracts**

Require the AArch64 synchronous handler to pass `sp` as the first argument to
`handle_syscall_simple`, followed by syscall number and six arguments. Require
the dispatcher to install and clear a per-CPU `LinuxSyscallContext` around only
Linux dispatch. Require `run_elf_launcher_entry` to register scheduler current
as TID/TGID 1 immediately before `switch_to_el0`, and require full process
reset to clear Linux tasks before freeing userspace mappings.

- [ ] **Step 2: Run RED**

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts linux_root_task_and_syscall_frame_have_bounded_owners -- --exact
```

Expected: the task module, context owner, frame argument, and root registration
are missing.

- [ ] **Step 3: Implement per-CPU syscall-frame ownership**

Define a copyable context:

```rust
#[derive(Clone, Copy)]
pub(crate) struct LinuxSyscallFrameRef {
    pub frame: *mut Aarch64ExceptionFrame,
    pub return_pc: u64,
    pub pstate: u64,
}
```

Back it with fixed atomic arrays sized by `scheduler::MAX_CPUS`. Implement
`with_linux_syscall_frame(frame, return_pc, pstate, dispatch)` using
compare-exchange from zero, clear it on every return path, and reject nested or
out-of-range installation with `SysError::EINVAL`. `current()` returns `None`
outside the dynamic dispatch extent.

Change `handle_syscall_simple` to this exact ABI:

```rust
pub extern "C" fn handle_syscall_simple(
    saved_frame: usize,
    syscall_num: u64,
    arg0: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
) -> u64;
```

- [ ] **Step 4: Implement the production task runtime and root lifecycle**

Use an `UnsafeCell<LinuxTaskRuntime>` behind a private `Sync` wrapper. Every
public mutation masks interrupts, resolves current identity from
`scheduler().current()`, and delegates lifecycle legality to
`LinuxTaskTable`. Provide `register_root`, `current_tid`, `current_tgid`,
`lookup_tid`, and `reset`.

In `run_elf_launcher_entry`, after loader preparation succeeds and before
entering EL0, register the current scheduler thread:

```rust
let scheduler_thread = scheduler::scheduler().current();
if syscall::linux_task::register_root(scheduler_thread).is_err() {
    complete_active_run(cpu, launch_id, |_| RunTermination::LaunchError(RunElfError::Thread));
    finish_launcher_thread();
}
unsafe {
    user_process::switch_to_el0(entry, stack_top, 0);
}
```

Call `linux_task::reset()` at the start of `reset_linux_process_state`, before
Linux pages, descriptors, or signal trampolines are reclaimed. Reset terminates
all non-current Linux scheduler threads and clears the table; it never frees
the currently executing kernel stack.

Change `sys_getpid` to current TGID and `sys_gettid` to current TID. Outside an
active Linux task both return `ESRCH`, rather than fabricated identity.

- [ ] **Step 5: Run GREEN and commit**

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts linux_root_task_and_syscall_frame_have_bounded_owners -- --exact
./scripts/run-host-unit-tests.sh --lib linux_task_logic
make build-test ARCH=aarch64-unknown-none
git add src/syscall/linux_task.rs src/syscall/linux_syscall_context.rs src/syscall/mod.rs src/syscall/syscall_dispatch.rs src/syscall/syscall.rs src/kernel_lowlevel/ARM64/boot.rs src/user_level/services/run_elf.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: bind Linux root tasks to syscall frames"
```

### Task 5: Execute Real AArch64 Clone Children

**Files:**
- Modify: `src/syscall/linux_task_logic_shared.rs`
- Modify: `src/syscall/linux_task.rs`
- Modify: `src/syscall/syscall.rs`
- Modify: `src/kernel_lowlevel/ARM64/thread.rs`
- Modify: `src/kernel_lowlevel/ARM64/context_switch.S`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing clone validation tests**

Define exact Linux constants in the shared file and test the glibc flag set:

```rust
const CLONE_VM: usize = 0x0000_0100;
const CLONE_FS: usize = 0x0000_0200;
const CLONE_FILES: usize = 0x0000_0400;
const CLONE_SIGHAND: usize = 0x0000_0800;
const CLONE_THREAD: usize = 0x0001_0000;
const CLONE_SYSVSEM: usize = 0x0004_0000;
const CLONE_SETTLS: usize = 0x0008_0000;
const CLONE_PARENT_SETTID: usize = 0x0010_0000;
const CLONE_CHILD_CLEARTID: usize = 0x0020_0000;
const CLONE_CHILD_SETTID: usize = 0x0100_0000;
```

Tests must accept the required base flags plus optional TID flags and reject a
nonzero exit signal, unknown bits, `CLONE_THREAD` without `CLONE_VM` and
`CLONE_SIGHAND`, a null/misaligned stack, missing TLS under `CLONE_SETTLS`, and
missing TID pointers under their corresponding flags.

Return `LinuxCloneValidationError::{Flags, Stack, Tls, ParentTid, ChildTid}`
from shared validation. Production maps `Flags`, `Stack`, and `Tls` to
`EINVAL`, and pointer variants to `EFAULT`.

- [ ] **Step 2: Write failing clone publication contracts**

Require `sys_clone` to read `linux_syscall_context::current`, copy the complete
parent exception frame, create a CPU0 suspended scheduler child, reserve a task
slot, perform checked parent/child TID stores, publish the task, and only then
publish the scheduler thread. Require every error path to roll back both
reservations and return `EINVAL`, `EFAULT`, or `EAGAIN`. Require `sys_clone3`
to return `ENOSYS` until its full versioned argument structure is implemented.

- [ ] **Step 3: Run RED**

```bash
./scripts/run-host-unit-tests.sh --lib linux_task_logic
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_clone_child_is_validated_before_publication -- --exact
```

Expected: clone flag validation and executable child publication are absent.

- [ ] **Step 4: Add clone startup state and EL0 transfer**

Add this task-owned startup image:

```rust
#[derive(Clone, Copy)]
pub(crate) struct Aarch64CloneStart {
    pub frame: Aarch64ExceptionFrame,
    pub user_sp: u64,
    pub return_pc: u64,
    pub pstate: u64,
    pub tls: u64,
}
```

`linux_clone_child_entry` resolves its task from scheduler current, takes the
startup image exactly once, and calls a non-returning assembly routine. Before
publication set `frame.regs[0] = 0`; retain all other copied GPR, SIMD, FPCR,
and FPSR values.

Add `start_linux_clone_child` to `context_switch.S`. It programs `sp_el0`,
`elr_el1`, `spsr_el1`, `tpidr_el0`, `fpcr`, and `fpsr`, restores `q0..q31`,
restores all GPRs from `Aarch64ExceptionFrame` with `x16` as the frame pointer,
loads saved `x17` before overwriting `x16`, and executes `eret`.

- [ ] **Step 5: Replace only the `CLONE_THREAD` synthetic path**

Keep non-thread clone behavior visibly synthetic for the separate process
milestone. For `CLONE_THREAD`, use this publication order:

```rust
let context = linux_syscall_context::current().ok_or(SysError::EINVAL)?;
let request = LinuxCloneRequest::validate(flags, newsp, parent_tid, newtls, child_tid)?;
let scheduler_id = scheduler::scheduler()
    .create_suspended_thread_on_cpu(linux_clone_child_entry, "linux_thread", 0)
    .ok_or(SysError::EAGAIN)?;
let reservation = linux_task::reserve_clone(scheduler_id, request, context)
    .map_err(|error| {
        scheduler::scheduler().terminate_thread(scheduler_id);
        error
    })?;
if let Err(error) = linux_task::copy_clone_tids(reservation) {
    linux_task::rollback_clone(reservation);
    scheduler::scheduler().terminate_thread(scheduler_id);
    return Err(error);
}
if let Err(error) = linux_task::commit_clone(reservation) {
    linux_task::restore_clone_tid_destinations(reservation);
    linux_task::rollback_clone(reservation);
    scheduler::scheduler().terminate_thread(scheduler_id);
    return Err(error);
}
Ok(reservation.tid)
```

`copy_clone_tids` snapshots the original destination values before writing.
`commit_clone` masks interrupts, revalidates both suspended identities, and
publishes the task and scheduler thread in one critical section. Its invariant
failure restores both destinations before returning `EAGAIN`.

Prevalidate every requested 32-bit TID destination with existing checked user
helpers before the first write. `sys_sched_yield` calls
`scheduler::yield_now()` and returns zero only after control resumes.

- [ ] **Step 6: Run GREEN, build, and commit**

```bash
./scripts/run-host-unit-tests.sh --lib linux_task_logic
./scripts/run-host-unit-tests.sh --test integration_contracts aarch64_clone_child_is_validated_before_publication -- --exact
make build-test ARCH=aarch64-unknown-none
git add src/syscall/linux_task_logic_shared.rs src/syscall/linux_task.rs src/syscall/syscall.rs src/kernel_lowlevel/ARM64/thread.rs src/kernel_lowlevel/ARM64/context_switch.S tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: execute AArch64 clone thread children"
```

### Task 6: Implement Scheduler-Backed Linux Futexes

**Files:**
- Create: `src/syscall/linux_futex_logic_shared.rs`
- Create: `src/syscall/linux_futex.rs`
- Modify: `src/syscall/mod.rs`
- Modify: `src/syscall/syscall.rs`
- Modify: `src/main.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`
- Modify: `verification/syscall/src/lib.rs`

- [ ] **Step 1: Write failing futex decoder and queue tests**

Test commands `WAIT=0`, `WAKE=1`, `WAIT_BITSET=9`, and `WAKE_BITSET=10`,
with `PRIVATE_FLAG=128`, `CLOCK_REALTIME=256`, and command mask `0x7f`.
Reject the realtime flag on `WAIT`/`WAKE`, unknown commands, unaligned
addresses, zero bitsets, and invalid timespec nanoseconds. Test compare
mismatch as `EAGAIN`, FIFO wake counts, bitset intersection, timeout rounding
without early expiry, stale scheduler identity rejection, and reset draining.

```rust
let wait = decode_futex_op(9 | 128 | 256).expect("WAIT_BITSET realtime");
assert_eq!(wait.command, FutexCommand::WaitBitset);
assert!(wait.private);
assert!(wait.realtime);
assert!(decode_futex_op(0 | 256).is_none());

let mut queue = FutexQueue::<4>::new();
queue.push(waiter(2, 8, 0x1000, 0x1)).unwrap();
queue.push(waiter(3, 9, 0x1000, 0x2)).unwrap();
assert_eq!(queue.wake(0x1000, 1, 0x2), [Some((3, 9)), None, None, None]);
```

- [ ] **Step 2: Run RED**

```bash
./scripts/run-host-unit-tests.sh --lib linux_futex_logic
```

Expected: the shared futex types and rules do not exist.

- [ ] **Step 3: Implement the fixed-capacity futex table**

Define `FutexWaiter` with address, bitset, TID, scheduler ID, optional absolute
tick deadline, FIFO sequence, and `FutexWaitOutcome::{Waiting, Woken,
TimedOut, Interrupted}`. The production table contains no raw task pointers.

For `FUTEX_WAIT` and `FUTEX_WAIT_BITSET`, mask interrupts, read the aligned
32-bit userspace value, compare it with `val`, enqueue the current task, and
transition both task and scheduler to blocked before restoring interrupts and
calling `scheduler::schedule()`. On resume remove the completed waiter and map
outcomes to zero, `ETIMEDOUT`, or `EINTR`.

`FUTEX_WAIT` converts its relative timespec to an absolute monotonic tick.
`FUTEX_WAIT_BITSET` treats its deadline as absolute monotonic or realtime.
Round positive sub-tick durations upward. `FUTEX_WAKE` uses match-all bitset;
`FUTEX_WAKE_BITSET` uses `val3` and returns the actual wake count.

- [ ] **Step 4: Connect timeout and reset boundaries**

Add `linux_futex::on_timer_tick(now_monotonic, now_realtime)` to the CPU0 timer
path after scheduler tick accounting and before `check_preemption`. It marks
expired waiters `TimedOut` and wakes only matching blocked scheduler threads.
Call `linux_futex::reset()` before `linux_task::reset()` during Linux process
cleanup so no queue retains a retiring identity.

Replace `sys_futex` with a direct call to `linux_futex::sys_futex`. Include the
shared file in the host and syscall verification crates.

- [ ] **Step 5: Run GREEN and commit**

```bash
./scripts/run-host-unit-tests.sh --lib linux_futex_logic
./scripts/run-host-unit-tests.sh --test integration_contracts linux_futex_waits_block_and_wake_scheduler_tasks -- --exact
make verus-syscall
make build-test ARCH=aarch64-unknown-none
git add src/syscall/linux_futex_logic_shared.rs src/syscall/linux_futex.rs src/syscall/mod.rs src/syscall/syscall.rs src/main.rs tests/host/src/lib.rs tests/host/tests/integration_contracts.rs verification/syscall/src/lib.rs
git commit -m "feat: block Linux threads on futex queues"
```

### Task 7: Move Signal State Into Linux Tasks

**Files:**
- Modify: `src/syscall/linux_task.rs`
- Modify: `src/syscall/linux_futex.rs`
- Modify: `src/syscall/syscall.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing isolation and routing tests**

Add task-table tests with root and two children. Give them distinct masks,
alternate stacks, standard pending bits, real-time queue entries, and nested
signal frames. Assert mutations and sigreturn pop only the addressed task.
Assert process-pending delivery chooses an unmasked live task, while
`tgkill(1, tid, sig)` and `rt_tgsigqueueinfo(1, tid, sig, info)` never deliver
on the caller. Assert missing TID or wrong TGID returns `ESRCH`, and signal zero
performs existence checking without queueing.

- [ ] **Step 2: Run RED**

```bash
./scripts/run-host-unit-tests.sh --lib linux_task_logic
./scripts/run-host-unit-tests.sh --test integration_contracts linux_signal_state_is_owned_by_each_live_task -- --exact
```

Expected: masks, pending queues, alt stacks, and signal frames still use global
atomics in `syscall.rs`.

- [ ] **Step 3: Add bounded per-task signal state**

Move task-owned state behind `linux_task` operations:

```rust
pub(crate) struct LinuxTaskSignalState {
    pub mask: u64,
    pub standard_pending: u64,
    pub realtime_pending: [LinuxPendingSignal; LINUX_RT_QUEUE_LIMIT],
    pub realtime_len: usize,
    pub alt_stack: LinuxSignalStack,
    pub frames: [LinuxSignalFrame; LINUX_SIGNAL_FRAME_LIMIT],
    pub frame_depth: usize,
    pub sigreturn_requested: bool,
}
```

Keep signal dispositions and process-pending queues process-wide. Convert
`rt_sigprocmask`, `rt_sigpending`, `sigaltstack`, `rt_sigreturn`, frame
push/pop, and pending selection to resolve the current task by scheduler ID.
Standard signals coalesce by bit; realtime records remain FIFO and preserve
all 128 `siginfo_t` bytes. Queue exhaustion returns `EAGAIN`.

- [ ] **Step 4: Implement directed delivery and blocked-task wakeup**

`kill(1, sig)` queues process-wide. `tkill` and `tgkill` validate and queue to
the target task. `complete_linux_signal_syscall_return` and timer delivery may
consume only current-task or process-pending signals allowed by the current
mask. A target blocked in futex receives `linux_futex::interrupt_task(tid,
scheduler_id)`, changes its waiter outcome to `Interrupted`, and becomes ready;
the handler still runs only when that target resumes.

Retain process-wide action semantics for `SIG_IGN`, `SA_RESETHAND`, and action
masks. Reject actions for SIGKILL/SIGSTOP as before.

- [ ] **Step 5: Run GREEN and commit**

```bash
./scripts/run-host-unit-tests.sh --lib linux_task_logic
./scripts/run-host-unit-tests.sh --test integration_contracts linux_signal_state_is_owned_by_each_live_task -- --exact
make build-test ARCH=aarch64-unknown-none
git add src/syscall/linux_task.rs src/syscall/linux_futex.rs src/syscall/syscall.rs tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: route Linux signals to live tasks"
```

### Task 8: Implement Child Exit And Pthread Join

**Files:**
- Modify: `src/syscall/linux_task_logic_shared.rs`
- Modify: `src/syscall/linux_task.rs`
- Modify: `src/syscall/linux_futex.rs`
- Modify: `src/syscall/syscall.rs`
- Modify: `src/kernel_objects/scheduler.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing clear-child-TID and exit tests**

Test that child exit is accepted once, removes all wait registrations and
task-pending signals, writes zero to the clear-child-TID address, wakes exactly
one matching futex waiter, marks the scheduler thread terminated without
freeing its active stack, and never returns to EL0. Repeated or stale exit must
not clear memory or wake again. Test that `set_tid_address` replaces the
current task's clear address and returns its real TID.

- [ ] **Step 2: Run RED**

```bash
./scripts/run-host-unit-tests.sh --lib linux_task_logic
./scripts/run-host-unit-tests.sh --test integration_contracts linux_child_exit_clears_tid_and_uses_deferred_stack_retirement -- --exact
```

Expected: `sys_exit` still completes every ELF launcher rather than exiting a
clone child.

- [ ] **Step 3: Implement non-returning child exit**

At the top of `sys_exit`, distinguish a clone child from the root task. The
child path calls a non-returning `linux_task::exit_current`:

```rust
if linux_task::current_is_clone_child() {
    linux_task::exit_current(exit_code);
}
```

`exit_current` masks interrupts, transitions once to exited, takes its
clear-child-TID address, removes task signal/wait state, restores interrupts,
performs a checked zero write, calls `linux_futex::wake_address(addr, 1,
FUTEX_BITSET_MATCH_ANY)`, marks the current scheduler thread through
`finish_current_without_stack_free`, then calls `scheduler::schedule()` and
waits for interrupt only if no runnable thread exists.

`sys_set_tid_address` validates a nonzero writable 32-bit address, stores it in
the current task, and returns current TID. A zero pointer clears the address.

- [ ] **Step 4: Make group exit terminate every peer**

`sys_exit_group` first asks task reset to terminate every non-current Linux
scheduler thread, clear all futex and signal state, and leave the current stack
alive. It then follows the existing `prepare_run_elf_return` path. This works
whether root or a child issued group exit because the CPU-bound launch identity
selects the same launcher completion; the current scheduler thread becomes the
resume carrier and is retired after completion.

- [ ] **Step 5: Run GREEN and commit**

```bash
./scripts/run-host-unit-tests.sh --lib linux_task_logic
./scripts/run-host-unit-tests.sh --test integration_contracts linux_child_exit_clears_tid_and_uses_deferred_stack_retirement -- --exact
make build-test ARCH=aarch64-unknown-none
git add src/syscall/linux_task_logic_shared.rs src/syscall/linux_task.rs src/syscall/linux_futex.rs src/syscall/syscall.rs src/kernel_objects/scheduler.rs tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: complete Linux thread exit and join"
```

### Task 9: Block Signal Waiters And Honor Restart Rules

**Files:**
- Modify: `src/syscall/linux_task.rs`
- Modify: `src/syscall/linux_futex.rs`
- Modify: `src/syscall/linux_syscall_context.rs`
- Modify: `src/syscall/syscall.rs`
- Modify: `tests/host/src/lib.rs`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing signal-wait and restart tests**

Test immediate matching dequeue, zero-timeout `EAGAIN`, timed wait expiry,
directed-signal wake, `sigsuspend` temporary mask restoration, non-restartable
`EINTR`, and `SA_RESTART` restoration of original syscall number, six argument
registers, and SVC PC. Require `rt_sigtimedwait` to copy complete queued
`siginfo_t` and return the signal number without running its handler.

- [ ] **Step 2: Run RED**

```bash
./scripts/run-host-unit-tests.sh --lib linux_task_logic
./scripts/run-host-unit-tests.sh --test integration_contracts linux_signal_waits_block_and_restart_from_the_original_svc -- --exact
```

Expected: `rt_sigtimedwait` and `rt_sigsuspend` return immediately and no
restart block exists.

- [ ] **Step 3: Implement task-owned blocking signal waits**

Add `LinuxSignalWait` containing wait mask, optional absolute deadline,
optional output address, previous mask for suspend, and outcome. Before
blocking, consume a matching task or process pending signal if available.
Otherwise transition the current task and scheduler to blocked and schedule.
Timer expiry wakes a timed waiter; `rt_sigtimedwait` maps expiry to `EAGAIN`.
`rt_sigsuspend` restores its previous mask after a caught signal and returns
`EINTR` after the handler completes.

- [ ] **Step 4: Preserve restartable syscall inputs**

At syscall-frame installation, copy syscall number, `x0..x5`, and the SVC
instruction address (`return_pc - 4`) into a task-owned `LinuxRestartBlock` for
blocking futex calls. If a caught action lacks `SA_RESTART`, resume futex wait
with `EINTR`. If it has `SA_RESTART`, attach the restart block to the nested
signal frame. On `rt_sigreturn`, restore `x0..x5`, `x8`, and set `ELR_EL1` to
the saved SVC address so the kernel rechecks the futex value and deadline.
Never restart `rt_sigsuspend` or `rt_sigtimedwait`.

- [ ] **Step 5: Run GREEN and commit**

```bash
./scripts/run-host-unit-tests.sh --lib linux_task_logic
./scripts/run-host-unit-tests.sh --test integration_contracts linux_signal_waits_block_and_restart_from_the_original_svc -- --exact
make build-test ARCH=aarch64-unknown-none
git add src/syscall/linux_task.rs src/syscall/linux_futex.rs src/syscall/linux_syscall_context.rs src/syscall/syscall.rs tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: block Linux signal waiters"
```

### Task 10: Verify And Measure The AArch64 Thread Runtime

**Files:**
- Generated: `host_shared/posixtest/`
- Generated: `target/posix/aarch64/thread-runtime-quality.json`
- Generated: `target/posix/aarch64/smros-fxfs-thread-canary.img`
- Generated: `target/posix/aarch64/smros-fxfs-thread-groups.img`
- Generated: `target/posix/aarch64/smros-fxfs-thread-all.img`
- Generated: `target/posix/aarch64/smros-run-thread-canary-*/`
- Generated: `target/posix/aarch64/smros-run-thread-groups-*/`
- Generated: `target/posix/aarch64/smros-run-thread-all/`
- Generated: `target/posix/aarch64/report-thread-runtime/`
- Create: `docs/posix/2026-08-03-aarch64-thread-runtime-results.md`

- [ ] **Step 1: Run the complete offline gate**

```bash
cargo fmt --manifest-path tests/host/Cargo.toml --check
rustfmt --edition 2021 --check src/syscall/linux_task.rs src/syscall/linux_syscall_context.rs src/syscall/linux_futex.rs src/syscall/syscall.rs src/kernel_objects/scheduler.rs src/kernel_lowlevel/ARM64/thread.rs src/kernel_lowlevel/ARM64/cpu.rs src/main.rs src/user_level/services/run_elf.rs
make script-check launcher-test linker-layout-test ut it posix-tool-test
make verus-syscall verus-kernel-objects verus-kernel-lowlevel
git diff --check
```

Expected: every command exits zero. Record exact test counts and Verus results;
do not infer counts from earlier commits.

- [ ] **Step 2: Capture host coverage and Coverity evidence**

Run host coverage when `cargo-tarpaulin` is available:

```bash
if command -v cargo-tarpaulin >/dev/null 2>&1; then make coverage-host; fi
```

When all three Coverity commands exist, create a fresh capture and analyze it:

```bash
if command -v cov-build >/dev/null 2>&1 && command -v cov-analyze >/dev/null 2>&1 && command -v cov-format-errors >/dev/null 2>&1; then
  test ! -e target/coverity-aarch64-thread-runtime
  cov-build --dir target/coverity-aarch64-thread-runtime make build-test ARCH=aarch64-unknown-none
  cov-analyze --dir target/coverity-aarch64-thread-runtime --all
  cov-format-errors --dir target/coverity-aarch64-thread-runtime --json-output-v7 target/coverity-aarch64-thread-runtime-results.json
fi
```

If `cargo-tarpaulin` or any Coverity command is absent, do not fabricate a
number. Write a canonical schema-1 quality check with status `unavailable`,
null findings/coverage, the missing command names, and current 40-hex commit.
For available tools record command, version, artifact path, findings, and
coverage percentage in
`target/posix/aarch64/thread-runtime-quality.json` using the schema documented
in `docs/POSIX_CONFORMANCE.md`. Generate the canonical file with:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import json
import re
import shutil
import subprocess
from pathlib import Path

commit = subprocess.check_output(
    ["git", "rev-parse", "HEAD"], text=True
).strip()
checks = []

tarpaulin = shutil.which("cargo-tarpaulin")
coverage_artifact = Path("target/coverage/host/tarpaulin-report.html")
if tarpaulin and coverage_artifact.is_file():
    html = coverage_artifact.read_text(errors="replace")
    match = re.search(r"([0-9]+(?:\.[0-9]+)?)%\s+coverage", html, re.IGNORECASE)
    coverage = None if match is None else float(match.group(1))
    checks.append({
        "artifact": str(coverage_artifact),
        "command": "make coverage-host",
        "coverage_percent": coverage,
        "findings": None,
        "kind": "coverage",
        "name": "host-rust-coverage",
        "status": "passed",
        "summary": "Tarpaulin host unit and integration coverage completed",
        "version": " ".join(subprocess.check_output(
            [tarpaulin, "--version"], text=True
        ).split()),
    })
elif not tarpaulin:
    checks.append({
        "artifact": None,
        "command": None,
        "coverage_percent": None,
        "findings": None,
        "kind": "coverage",
        "name": "host-rust-coverage",
        "status": "unavailable",
        "summary": "cargo-tarpaulin is not installed",
        "version": None,
    })
else:
    checks.append({
        "artifact": None,
        "command": "make coverage-host",
        "coverage_percent": None,
        "findings": None,
        "kind": "coverage",
        "name": "host-rust-coverage",
        "status": "failed",
        "summary": "Tarpaulin completed without the required HTML artifact",
        "version": " ".join(subprocess.check_output(
            [tarpaulin, "--version"], text=True
        ).split()),
    })

coverity_tools = [shutil.which(name) for name in (
    "cov-build", "cov-analyze", "cov-format-errors"
)]
coverity_artifact = Path("target/coverity-aarch64-thread-runtime-results.json")
if all(coverity_tools) and coverity_artifact.is_file():
    value = json.loads(coverity_artifact.read_text())
    issues = value.get("issues")
    if not isinstance(issues, list):
        raise ValueError("Coverity JSON does not contain an issues list")
    checks.append({
        "artifact": str(coverity_artifact),
        "command": "cov-build; cov-analyze --all; cov-format-errors --json-output-v7",
        "coverage_percent": None,
        "findings": len(issues),
        "kind": "static-analysis",
        "name": "coverity",
        "status": "passed" if not issues else "failed",
        "summary": f"Coverity analysis completed with {len(issues)} findings",
        "version": " ".join(subprocess.check_output(
            [coverity_tools[0], "--version"], text=True, stderr=subprocess.STDOUT
        ).split()),
    })
elif not all(coverity_tools):
    missing = [name for name, path in zip(
        ("cov-build", "cov-analyze", "cov-format-errors"), coverity_tools
    ) if path is None]
    checks.append({
        "artifact": None,
        "command": None,
        "coverage_percent": None,
        "findings": None,
        "kind": "static-analysis",
        "name": "coverity",
        "status": "unavailable",
        "summary": "Missing Coverity commands: " + ", ".join(missing),
        "version": None,
    })
else:
    checks.append({
        "artifact": None,
        "command": "cov-build; cov-analyze --all; cov-format-errors --json-output-v7",
        "coverage_percent": None,
        "findings": None,
        "kind": "static-analysis",
        "name": "coverity",
        "status": "failed",
        "summary": "Coverity completed without the required JSON artifact",
        "version": " ".join(subprocess.check_output(
            [coverity_tools[0], "--version"], text=True, stderr=subprocess.STDOUT
        ).split()),
    })

evidence = {
    "architecture": "aarch64",
    "checks": checks,
    "schema": 1,
    "smros_commit": commit,
}
path = Path("target/posix/aarch64/thread-runtime-quality.json")
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(
    json.dumps(evidence, sort_keys=True, separators=(",", ":")) + "\n"
)
print(path)
PY
```

- [ ] **Step 3: Rebuild the commit-matched stage and kernel**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest --verify-only
make build-test ARCH=aarch64-unknown-none
```

Expected: manifest metadata names the current implementation commit, stage
verification succeeds, and `kernel8.img` embeds that exact stage.

- [ ] **Step 4: Run upstream guest canaries on a fresh private disk**

```bash
test ! -e target/posix/aarch64/smros-fxfs-thread-canary.img
qemu-img create -f raw target/posix/aarch64/smros-fxfs-thread-canary.img 128M
```

Run these exact upstream tests with a separate output directory for each
result:

```text
conformance/interfaces/pthread_create/1-1.c
conformance/interfaces/pthread_getspecific/1-1.c
conformance/interfaces/pthread_join/1-1.c
conformance/interfaces/pthread_kill/1-1.c
conformance/interfaces/sigaction/16-1.c
```

Use the same private canary disk for the five sequential boots. Assert one
complete terminal attempt per test, `pts_status == "pass"`, no timeout/restart,
no panic/loader failure in serial, and every resource delta is non-positive.
These upstream tests jointly cover executable clone, distinct TLS, directed
signal delivery, futex join, and child exit without adding a nonstandard test
to the POSIX denominator.

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

tests = (
    "conformance/interfaces/pthread_create/1-1.c",
    "conformance/interfaces/pthread_getspecific/1-1.c",
    "conformance/interfaces/pthread_join/1-1.c",
    "conformance/interfaces/pthread_kill/1-1.c",
    "conformance/interfaces/sigaction/16-1.c",
)
for index, test_id in enumerate(tests, start=1):
    result = run_smros(
        Path("host_shared/posixtest"),
        Path(f"target/posix/aarch64/smros-run-thread-canary-{index}"),
        kernel=Path("kernel8.img"),
        disk=Path("target/posix/aarch64/smros-fxfs-thread-canary.img"),
        memory="1024M",
        test_id=test_id,
    )
    assert result.complete
    assert result.restart_count == 0
    assert len(result.attempts) == 1
    attempt = result.attempts[0]
    assert attempt.test_id == test_id
    assert attempt.pts_status == "pass"
    assert not attempt.timed_out
    assert not attempt.resource_deltas.has_positive()
    serial = result.raw_log_path.read_text(errors="replace")
    for forbidden in ("Kernel panic", "failed to map segment", "cannot create shared object descriptor"):
        assert forbidden not in serial
    print(test_id, attempt.status)
PY
```

- [ ] **Step 5: Run complete `threads` and `signals` groups**

Create one new private disk for both group campaigns:

```bash
test ! -e target/posix/aarch64/smros-fxfs-thread-groups.img
qemu-img create -f raw target/posix/aarch64/smros-fxfs-thread-groups.img 128M
```

Run `run_smros` once with `group="threads"` and once with
`group="signals"`, using distinct output directories and the same private
disk. Assert each selected runnable test has exactly one terminal attempt and
each suite has `complete=true`. In the signals results, assert exactly 26 test
IDs beginning `conformance/interfaces/sigaction/16-` and require all 26 to
pass without `ESRCH`. Preserve all unrelated upstream non-passes.

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

results = {}
for group in ("threads", "signals"):
    result = run_smros(
        Path("host_shared/posixtest"),
        Path(f"target/posix/aarch64/smros-run-thread-groups-{group}"),
        kernel=Path("kernel8.img"),
        disk=Path("target/posix/aarch64/smros-fxfs-thread-groups.img"),
        memory="1024M",
        group=group,
    )
    assert result.complete
    assert len({attempt.test_id for attempt in result.attempts}) == len(result.attempts)
    assert all(not attempt.resource_deltas.has_positive() for attempt in result.attempts)
    results[group] = result
    print(group, len(result.attempts), sum(a.status == "pass" for a in result.attempts))

sig16 = [
    attempt for attempt in results["signals"].attempts
    if attempt.test_id.startswith("conformance/interfaces/sigaction/16-")
]
assert len(sig16) == 26
assert all(attempt.pts_status == "pass" for attempt in sig16)
assert all("ESRCH" not in (attempt.stdout + attempt.stderr) for attempt in sig16)
PY
```

- [ ] **Step 6: Run all 1,598 selected tests on a third private disk**

```bash
test ! -e target/posix/aarch64/smros-fxfs-thread-all.img
qemu-img create -f raw target/posix/aarch64/smros-fxfs-thread-all.img 128M
```

Call `run_smros` without a filter, write to
`target/posix/aarch64/smros-run-thread-all`, and assert `complete=true`, exactly
1,598 terminal attempts, no loader exhaustion, no kernel panic, and no positive
resource delta. This milestone does not convert the real fork/COW/wait
failures to passes.

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

result = run_smros(
    Path("host_shared/posixtest"),
    Path("target/posix/aarch64/smros-run-thread-all"),
    kernel=Path("kernel8.img"),
    disk=Path("target/posix/aarch64/smros-fxfs-thread-all.img"),
    memory="1024M",
)
assert result.complete
assert len(result.attempts) == 1598
assert all(not attempt.resource_deltas.has_positive() for attempt in result.attempts)
serial = result.raw_log_path.read_text(errors="replace")
for forbidden in ("Kernel panic", "failed to map segment", "cannot create shared object descriptor"):
    assert forbidden not in serial
print(
    "attempts=", len(result.attempts),
    "pass=", sum(a.status == "pass" for a in result.attempts),
    "restarts=", result.restart_count,
)
PY
```

- [ ] **Step 7: Render detailed reports with quality evidence**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli report \
  --manifest host_shared/posixtest/manifest.json \
  --smros-results target/posix/aarch64/smros-run-thread-all/results.ndjson \
  --quality-evidence target/posix/aarch64/thread-runtime-quality.json \
  --out target/posix/aarch64/report-thread-runtime
```

Expected: JSON, NDJSON, JUnit, CSV, Markdown, and HTML agree on build,
execution, pass, API, group, optional-group, resource, and quality evidence.

- [ ] **Step 8: Record and commit the evidence summary**

Write `docs/posix/2026-08-03-aarch64-thread-runtime-results.md` with exact
commit, suite revision, build ID, manifest/build-results/patch/results/serial
SHA-256 values, offline test counts, canary rows, group/full totals, API and
group coverage, all non-pass clusters, timeout/restart counts, maximum resource
deltas, host coverage, Coverity status/findings, artifact paths, and the next
fork/COW/wait root cause. State explicitly that this is not overall POSIX
conformance.

```bash
git add docs/posix/2026-08-03-aarch64-thread-runtime-results.md
git commit -m "docs: record AArch64 thread runtime campaign"
git status --short --branch
```

Expected: only the known user-owned Python cache changes remain unstaged;
private images, stages, logs, coverage, Coverity, and reports remain generated
artifacts and are not committed.
