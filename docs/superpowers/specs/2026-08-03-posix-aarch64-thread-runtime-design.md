# AArch64 POSIX Thread Runtime Design

Date: 2026-08-03

## Status

Approved direction: implement a real AArch64 `CLONE_THREAD` runtime before
building forked address spaces and parent/child wait state.

This design is an incremental part of the POSIX userspace conformance
architecture. It must improve conformance through reusable task, scheduler,
futex, TLS, and signal semantics. It must not special-case Open POSIX test IDs,
remap result statuses, or weaken assertions.

## Evidence And Problem

The commit-matched AArch64 `signals` campaign completed all 649 selected tests
with 527 passes, 79 failures, 41 unresolved results, two untested results, and
no timeouts. The largest remaining thread-dependent cluster is:

- 26 `sigaction/16-*` tests where `pthread_create` receives a synthetic clone
  identifier but no child thread executes, so `pthread_kill` returns `ESRCH`;
- blocking signal APIs whose callers and target threads cannot sleep and wake;
- pthread and semaphore tests that need a real futex wait queue; and
- thread joins that need `CLONE_CHILD_CLEARTID` and a wake on exit.

SMROS currently returns a synthetic PID from `sys_clone`, returns immediately
from Linux futex operations, returns constant PID/TID values, and stores Linux
signal masks, pending state, alternate stacks, and signal frames globally. The
scheduler switches kernel stacks but does not preserve the AArch64 EL0 return
PC, user stack, TLS register, processor state, or FP/SIMD state for each
thread. `sched_yield` is also a no-op.

## Scope

This increment provides executable, shared-address-space Linux threads for the
AArch64 runtime:

- support the glibc `pthread_create` clone flag combination;
- give each live task a unique TID in one thread group;
- start a child at the instruction after `clone`, with `x0=0`, the requested
  stack, and the requested `TPIDR_EL0` value;
- preserve each task's EL0 return state across yield, futex blocking, and timer
  preemption;
- implement `CLONE_PARENT_SETTID`, `CLONE_CHILD_SETTID`, and
  `CLONE_CHILD_CLEARTID` copy-out and exit behavior;
- make `gettid`, `tgkill`, `tkill`, and thread-directed queued signals use live
  task records;
- move masks, task-pending signals, alternate stacks, and nested signal frames
  into per-task state while retaining process-wide signal dispositions;
- implement bounded Linux futex wait and wake queues backed by scheduler thread
  states; and
- make `sched_yield`, child thread exit, join, semaphore waits, and signal
  wakeups schedule real threads.

All Linux tasks in this increment are bound to the active run's physical CPU0.
SMROS currently exposes logical CPU affinity but does not maintain independent
per-CPU scheduler-current state. POSIX does not require more than one online
processor, and restricting the first implementation to CPU0 avoids claiming
unsafe SMP execution. Multi-CPU Linux task scheduling requires a separate
scheduler ownership design.

## Explicit Non-Goals

This increment does not implement non-`CLONE_THREAD` process creation, forked
address spaces, copy-on-write, `vfork`, `execve`, parent/child relationships,
`SIGCHLD`, or wait statuses. Those remain real failures until the next process
milestone. It does not make x86_64 or RISC-V64 signal/thread return paths
functional; their architecture ports follow AArch64 conformance.

The existing non-thread clone path may remain temporarily synthetic only to
avoid widening this increment. It must not be used to classify any test as a
pass, and the results documentation must continue to identify fork/wait as
unimplemented.

## Architecture

### Linux Task State

A dedicated Linux task module owns a fixed-capacity task table. A task record
contains:

- Linux TID and thread-group ID;
- the backing scheduler `ThreadId`;
- lifecycle state (`empty`, `starting`, `runnable`, `blocked`, or `exited`);
- saved clone register image, child entry PC, child stack, and TLS value while
  startup is pending;
- parent-TID and child-TID addresses and the clear-child-TID address;
- blocked signal mask and standard/real-time task-pending signals;
- alternate signal stack state; and
- a bounded stack of nested AArch64 signal frames.

The root ELF task is registered when the loader transfers to EL0. It attaches
to the scheduler thread and kernel stack already running the ELF launcher; it
does not allocate a second scheduler thread. Its TID and thread-group ID are 1
for compatibility with the current single-process runtime. Child TIDs are
monotonically allocated within a launch and do not change when a task-table
slot is reused after deferred scheduler retirement. Allocation fails with
`EAGAIN` before integer wrap could make an old TID live again. Per-launch
cleanup terminates or detaches every child scheduler thread, clears futex
waiters, clears task signal state, and resets the allocator.

Linux task lifecycle changes pass through one task-module transition API. That
API updates the task record and backing scheduler state while interrupts are
masked, so the duplicate lifecycle fields cannot independently drift. The
running task is resolved from the current scheduler `ThreadId`; no syscall or
signal path relies on a global "current Linux TID" value.

Process-wide signal dispositions and process-pending signals remain shared by
the thread group. `kill` targets the process pending queue; `tkill`, `tgkill`,
and `rt_tgsigqueueinfo` target a specific live task. Delivery considers the
current task queue before the process queue and applies the current task's
mask. A signal intended for another task never runs on the caller merely
because the caller next returns from a syscall.

### AArch64 Context Boundary

The AArch64 scheduler context and context-switch assembly are extended as one
ABI change. In addition to the existing kernel callee-saved registers and
kernel stack, each TCB preserves:

- `SP_EL0`, `ELR_EL1`, and `SPSR_EL1`;
- `TPIDR_EL0`;
- `FPCR` and `FPSR`; and
- all 32 128-bit SIMD registers.

The Rust `CpuContext` offsets and assembly offsets are locked by host tests and
source-level integration contracts. Context switching masks interrupts until
the old state is complete and the new state is restored.

The synchronous exception entry passes the saved 32-register frame address to
the syscall dispatcher. The dispatcher exposes that address only for the
duration of the active syscall and indexes it by physical CPU. `sys_clone`
copies the frame before the child is published as runnable. Caller-saved EL0
registers remain in each task's exception frame on its kernel stack; the
extended `CpuContext` owns the register and system-register state that is not
already preserved by that frame. A task switch never points two tasks at the
same exception frame.

A new AArch64 transfer routine starts the clone child by restoring its copied
general registers, setting `x0=0`, programming its user stack and TLS, and
returning to the saved post-`svc` PC at EL0. The parent receives the live child
TID only after all requested TID stores and the startup record are valid.

### Clone Validation And Publication

The thread path accepts the known Linux flags needed by AArch64 glibc:
`CLONE_VM`, `CLONE_FS`, `CLONE_FILES`, `CLONE_SIGHAND`, `CLONE_THREAD`,
`CLONE_SYSVSEM`, `CLONE_SETTLS`, `CLONE_PARENT_SETTID`,
`CLONE_CHILD_SETTID`, and `CLONE_CHILD_CLEARTID`, plus the required zero exit
signal for a thread-group child.

Validation rejects unknown flags, a null or misaligned child stack, invalid TID
pointers, missing TLS when `CLONE_SETTLS` is requested, and inconsistent Linux
relationships such as `CLONE_THREAD` without `CLONE_SIGHAND` and `CLONE_VM`.
All copy-out and scheduler allocation checks happen before publication. Any
failure rolls back the task slot and scheduler thread and returns the precise
Linux errno without leaving a runnable child.

The initial implementation supports the legacy AArch64 `clone` ABI used by the
pinned glibc. `clone3` is accepted for thread creation only after its complete
versioned argument structure is safely copied and validated; otherwise it
continues returning a real error rather than guessing missing fields.

### Scheduling And Futexes

`sched_yield` calls the scheduler's yield path. Timer IRQ return invokes the
preemption check after timer-signal work and after all interrupt-owned mutable
state has been released, allowing another CPU0-bound ready thread to run when
the time slice expires. Because the TCB and its kernel stack jointly own the
full EL0 return state, a switch from a syscall or IRQ resumes the correct
kernel exception frame and then the correct userspace context.

The Linux futex table stores bounded address-keyed FIFO queues of Linux TID and
scheduler `ThreadId` pairs. The dispatcher separates the futex command from
`FUTEX_PRIVATE_FLAG` and `FUTEX_CLOCK_REALTIME`; private and non-private keys
are equivalent while this runtime has only one shared address space, and the
clock flag is rejected on commands for which Linux does not allow it.
`FUTEX_WAIT` and `FUTEX_WAIT_BITSET` validate alignment, atomically compare the
userspace value with the expected value, register the current live task, mark
it blocked, and schedule. A mismatch returns `EAGAIN`. `FUTEX_WAIT` interprets
its timeout as relative; `FUTEX_WAIT_BITSET` interprets it as an absolute
monotonic deadline, or an absolute realtime deadline when the clock flag is
present. The existing tick clocks round deadlines up so a wait cannot expire
early and return `ETIMEDOUT` when the deadline is reached. A zero bitset is
invalid. `FUTEX_WAKE` and `FUTEX_WAKE_BITSET` remove up to the requested number
of matching waiters, mark their scheduler threads ready, and return the actual
wake count.

`CLONE_CHILD_CLEARTID` exit writes zero to the registered address and performs
a timeout-free futex wake of one waiter. This is the primitive used by glibc
`pthread_join`. Futex queues store no raw references into task records, and
per-launch reset removes all entries before task IDs can be reset.

### Thread Exit

`exit` distinguishes the root task from a clone child. A child exit:

1. records the exit transition once;
2. clears and wakes the clear-child-TID futex when registered;
3. removes task-directed pending signals and futex wait registrations;
4. marks the backing scheduler thread terminated without freeing its active
   kernel stack; and
5. schedules another runnable thread without returning to EL0.

The scheduler's existing deferred retirement mechanism reclaims the kernel
stack only after another stack is confirmed active. `exit_group` terminates all
child tasks and then follows the existing root ELF completion path.

## Concurrency And Safety

Task, futex, and signal state mutations run with interrupts masked and use
fixed-capacity tables. No allocator is called from timer IRQ signal delivery.
The child is not visible to scheduling or signal lookup until its complete
startup state and requested userspace TID writes exist.

Scheduler wake operations transition only `Blocked` threads to `Ready` and do
not revive an exited or stale task. Every lookup verifies both Linux TID and
the backing scheduler thread identity. Cleanup is idempotent and bounded.

All userspace reads and writes use the existing checked user-buffer helpers.
Arithmetic for stacks, futex pointers, table counts, timeouts, and register
frame offsets is checked before dereference.

## Error Behavior

The implementation uses Linux errno values:

- `EINVAL` for unsupported/inconsistent clone flags or futex operations;
- `EFAULT` for invalid userspace pointers;
- `EAGAIN` for task-table, scheduler-thread, or futex-waiter exhaustion and
  futex compare mismatch;
- `ESRCH` for missing or exited target TIDs;
- `ETIMEDOUT` for an expired supported futex wait; and
- `EINTR` only when a blocking operation is genuinely interrupted by a signal
  whose action does not require restart.

No failure is converted to success for compatibility with a test.

## Testing

Development follows red-green-refactor in bounded increments:

1. Host logic tests fail first for clone flag validation, TID allocation,
   publication rollback, task lookup, futex FIFO wake selection, and lifecycle
   cleanup.
2. Integration contracts fail first unless the AArch64 `CpuContext` and
   assembly save/restore offsets agree and the syscall exception path passes
   its saved register frame.
3. Scheduler tests fail first for waking only blocked threads and retiring a
   clone stack only after switching away.
4. Existing `sigaction/16-1.c` is retained as the initial guest RED result:
   `pthread_kill` currently receives `ESRCH` because no child task exists.
5. A small AArch64 guest canary demonstrates clone child execution, distinct
   TIDs, TLS isolation, futex block/wake, directed signal delivery, clean child
   exit, and join.
6. All 26 `sigaction/16-*` cases run on a fresh private disk. Their outcomes
   must be genuine Open POSIX results and every terminal resource delta must be
   non-positive.
7. The complete `threads` and `signals` groups run on separate fresh private
   disks, followed by the full 1,598-case campaign when those gates are stable.

Every existing host, integration, POSIX-tool, launcher, linker-layout,
formatting, shell-syntax, AArch64 release-build, stage-verification, event
integrity, and resource-cleanup gate remains mandatory. Coverity is recorded as
unavailable until its tools are installed; no finding or coverage number is
invented.

## Success Criteria

This increment is complete only when:

- an AArch64 glibc pthread child executes with a distinct live TID and correct
  stack/TLS state;
- yield, timer preemption, futex block/wake, directed signal delivery, child
  exit, and join all operate on that real child;
- per-task signal masks, pending queues, alternate stacks, and signal frames do
  not leak between threads;
- invalid clone/futex/task operations return the specified errno;
- per-launch cleanup leaves no child scheduler thread, futex waiter, signal
  frame, Linux mapping, descriptor, or kernel handle;
- the 26 `sigaction/16-*` tests no longer report `ESRCH` due to a synthetic
  clone identifier; and
- all verification artifacts retain exact commit and manifest provenance.

Passing this gate does not claim fork, process, or overall POSIX conformance.
The next design increment adds real forked address spaces, termination by
default signals, `SIGCHLD`, and parent wait status on top of the same task
lifecycle model.
