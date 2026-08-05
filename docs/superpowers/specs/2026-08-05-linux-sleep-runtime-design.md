# AArch64 Linux Sleep Runtime Design

Date: 2026-08-05

## Status

Approved direction: implement scheduler-backed per-task Linux sleeps using the
existing Linux task lifecycle, CPU0 timer expiry, and signal-delivery paths.

This is a focused extension of the AArch64 POSIX thread runtime. It must
implement reusable Linux sleep semantics and must not special-case Open POSIX
test IDs, alter test results, or hide unsupported behavior.

## Evidence And Problem

Commit `528b34686706905f55d083da3af539c0194f0e52` fixed FxFS device/inode
identity, allowing glibc to load `libgcc_s.so.1` correctly. On the matching
AArch64 kernel and POSIX stage:

- `pthread_create/1-1.c`, `pthread_getspecific/1-1.c`, and
  `pthread_join/1-1.c` pass with zero positive resource deltas;
- `pthread_join/1-1.c` no longer reports a glibc unwind assertion or timeout;
- `pthread_kill/1-1.c` consistently calls its signal handler but then fails
  with `Test FAILED: Kill request timed out`; and
- the same `pthread_kill` result reproduces when it is the only test on a new
  private disk.

The signal reaches the requested child. The failure occurs afterward because
Linux `nanosleep` currently validates only a non-null request pointer and then
returns success immediately. `clock_nanosleep` delegates to that stub while
ignoring flags and the remaining-time pointer. The upstream test's child
therefore advances through its nominal five-second delay without blocking and
overwrites the handler result before the main task can observe it.

This is a general POSIX runtime defect, not a `pthread_kill` routing defect.

## Scope

This increment implements AArch64 Linux userspace sleep behavior for:

- relative `nanosleep`;
- relative `clock_nanosleep` for supported Linux clock IDs;
- absolute `clock_nanosleep` with `TIMER_ABSTIME`;
- checked `timespec` input and overflow-safe deadline conversion;
- scheduler blocking until the selected deadline;
- interruption by a deliverable signal targeted at the sleeping task;
- `EINTR` and remaining-time copyout for interrupted relative sleeps; and
- bounded reset and task-exit cleanup without stale wakeups.

All Linux tasks remain pinned to physical CPU0. The implementation uses the
existing 100 Hz kernel tick, whose duration is 10,000,000 nanoseconds. Positive
relative durations round up and include the existing phase guard so they
cannot expire early.

## Explicit Non-Goals

This increment does not add high-resolution hardware timers, change the timer
frequency, implement multi-CPU Linux task scheduling, or redesign POSIX timer
objects. It does not implement sleep restart inside the kernel. Userspace may
retry a relative sleep using the copied remaining duration, as Linux permits.

It does not change the existing futex or signal-wait deadline definitions
except to reuse their checked conversion style. It does not claim x86_64 or
RISC-V64 runtime support and does not claim overall POSIX conformance.

## Architecture

### Per-Task Sleep State

Each live Linux task receives at most one bounded sleep record:

```rust
pub(crate) enum LinuxSleepOutcome {
    Waiting,
    Completed,
    Interrupted,
}

pub(crate) struct LinuxSleepWait {
    pub deadline: u64,
    pub outcome: LinuxSleepOutcome,
}
```

`LinuxTaskTable` owns a distinct fixed `sleep_waits` array indexed by the same
slot as its task record. Clone rollback, task retirement, and process reset
clear that slot together with the task's signal state. A sleep record contains
no raw task, scheduler, or userspace pointers. A new
`LinuxBlockReason::Sleep` binds the record to the existing Linux task and
scheduler blocked states.

Pure shared logic provides one-shot operations to begin, expire, interrupt,
take, cancel, and reset a sleep. Beginning a second sleep while one is active
fails closed. Expiry and interruption change only `Waiting` records, so a
stale timer tick or repeated signal cannot complete the same wait twice.

### Deadline Conversion

The syscall layer copies a `LinuxTimespec` only after validating one complete
readable userspace range. It rejects negative seconds, nanoseconds outside
`0..1_000_000_000`, arithmetic overflow, unsupported clock IDs, and unknown
flags with the appropriate Linux errno.

Relative sleeps convert the requested duration to ticks using checked
round-up division. A nonzero sub-tick duration consumes at least one full tick,
and the existing phase guard prevents expiration in the partially elapsed
current tick. A zero relative duration returns immediately without entering a
blocked state.

Absolute sleeps convert the requested clock value to the first tick at or
after that value. A deadline at or before the current selected clock returns
immediately. SMROS currently exposes monotonic and realtime through the same
tick source; the API keeps the clock selection explicit so their
implementations can diverge later without changing task state.

### Blocking And Timer Expiry

For a positive future deadline, the syscall path masks interrupts, installs
the current task's sleep record, transitions the Linux task with
`LinuxBlockReason::Sleep`, transitions the backing scheduler thread to
`Blocked`, and calls the scheduler. Publication order matches futex waits: no
timer or signal path can observe an unblocked task with an active sleep record.

The existing CPU0 `linux_task::on_timer_tick(now)` path expires due sleep
records after scheduler tick accounting. It collects bounded `(tid,
scheduler_thread)` identities, then uses `wake_blocked` with the exact `Sleep`
reason. A stale identity or failed scheduler wake removes the completed record
instead of leaving reusable task state armed.

When the task resumes, the syscall takes exactly one terminal sleep outcome.
`Completed` returns success. Missing or inconsistent state returns a real
error and cancels the residual record rather than fabricating success.

### Signal Interruption

Signal routing continues to queue the complete signal record before waking a
target. `interrupt_linux_signal_target` gains a `Sleep` case beside the
existing futex case. A task-table helper checks the queued signal number
against the addressed task's current mask. Only when the signal is unmasked
and the exact task is blocked in a waiting sleep does it mark that record
`Interrupted` and wake the matching scheduler identity.

The resumed sleep syscall returns `EINTR`. Normal syscall-return signal
delivery then builds the handler frame for that same task. A blocked signal
remains pending without waking the task, and an ignored signal is discarded by
the existing disposition path.

For relative sleeps, an interrupted call calculates the nonnegative remaining
duration from the saved absolute deadline and current tick. If the caller
provided a non-null remaining pointer, the syscall writes a complete checked
`LinuxTimespec`. `clock_nanosleep` with `TIMER_ABSTIME` never writes remaining
time, matching Linux behavior.

### Reset And Exit

Clone rollback, child exit, task retirement, and Linux process reset clear the
sleep record in place. Process reset terminates blocked child scheduler threads
through the existing lifecycle before task slots can be reused. Since timer
expiry validates both TID and scheduler identity, a stale deadline cannot wake
a successor occupying the same task-table or scheduler slot.

## Syscall Behavior

`nanosleep(req, rem)` uses a relative monotonic duration. `clock_nanosleep`
accepts flags `0` and `TIMER_ABSTIME` only and uses the requested supported
clock. The AArch64 dispatcher passes both `nanosleep` arguments instead of
discarding `rem`.

The implementation returns:

- `0` after a zero duration, an already reached absolute deadline, or timer
  completion;
- `EFAULT` for an unreadable request or an unwritable non-null remaining
  buffer;
- `EINVAL` for an invalid `timespec`, unsupported clock ID, or unknown flag;
- `EAGAIN` when task/scheduler state cannot publish the bounded sleep; and
- `EINTR` only after a deliverable signal interrupts the sleeping task.

Remaining-buffer writability is checked before blocking so an invalid output
cannot turn a completed asynchronous transition into an ambiguous result.

## Concurrency And Safety

Sleep record mutations and task/scheduler transitions run with interrupts
masked, following the existing futex and signal-wait ownership rules. Timer IRQ
code performs no allocation and handles at most the fixed Linux task capacity.
Every wake verifies TID, scheduler identity, task state, and block reason.

All deadline arithmetic uses checked operations. All userspace access uses the
existing AArch64 readable/writable mapping checks. No userspace pointer is
retained across the blocked interval.

## Testing

Development follows red-green-refactor:

1. Pure host tests fail first for relative deadline rounding, absolute
   conversion, one-shot expiry, one-shot interruption, reset, and stale task
   identity rejection.
2. A source integration contract fails first unless `nanosleep` carries both
   pointers, publishes `LinuxBlockReason::Sleep`, timer expiry wakes the exact
   task, and directed signal delivery interrupts a sleep.
3. Focused host tests and the integration contract turn green before broader
   verification.
4. Full host library and integration suites, `make verus-syscall`, formatting,
   `git diff --check`, and the AArch64 release/link-layout build remain
   mandatory.
5. A commit-matched stage and kernel rerun `pthread_kill/1-1.c` first on a new
   disk. It must pass once, with no restart, timeout, panic, loader failure, or
   positive resource delta.
6. The five thread/signal canaries then run sequentially on another new disk,
   followed by the complete `threads` and `signals` campaigns and the full
   1,598-test campaign.

Existing Tarpaulin and Coverity evidence remains exact: coverage is failed
with the recorded duplicate `core::sized` error, and Coverity is unavailable
because its commands are absent. No coverage percentage or finding count is
invented.

## Success Criteria

This increment is complete only when:

- positive sleeps block the calling Linux task until deadline or a deliverable
  signal;
- timer expiry and signal interruption are one-shot and identity checked;
- relative interruption returns `EINTR` with a correct nonnegative remaining
  duration when requested;
- absolute `clock_nanosleep` does not report remaining time;
- reset and exit leave no armed sleep record or blocked scheduler task;
- the previously reproducible `pthread_kill/1-1.c` failure passes for the
  correct runtime reason; and
- all campaign evidence identifies the exact implementation commit, stage,
  manifest, build, runtime, and result hashes.

Passing these gates improves the AArch64 thread runtime but does not establish
overall POSIX compliance.
