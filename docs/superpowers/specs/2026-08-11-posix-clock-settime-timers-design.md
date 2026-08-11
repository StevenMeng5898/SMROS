# AArch64 POSIX Clock Settime And Timer Design

Date: 2026-08-11

## Status

Approved direction: implement a settable realtime clock and functional
per-process POSIX timers using the existing CPU0 timer tick and Linux signal
delivery paths.

This is a focused correction for the reproduced Open POSIX failures in
`clock_settime/20-1.c` and `clock_settime/4-1.c`. It must implement reusable
clock and timer semantics; it must not special-case test IDs, report a timeout
as a pass, or change the test harness to conceal missing runtime behavior.

## Evidence And Problem

SMROS currently accepts both `CLOCK_REALTIME` and `CLOCK_MONOTONIC` in
`clock_settime`. The implementation checks only that the clock ID is supported
for reads, checks for a null pointer, and returns success without reading or
applying the requested value. POSIX requires `CLOCK_MONOTONIC` to be
non-settable, so `clock_settime/20-1.c` reports that the call did not fail.

The POSIX timer syscalls currently create an owned handle, but
`timer_settime` does not read or arm the supplied `itimerspec`, `timer_gettime`
always reports zero, and no timer-expiration path exists. In
`clock_settime/4-1.c`, the process arms an absolute realtime timer, moves the
realtime clock backward, and waits for `SIGALRM`. Because the timer is never
armed, the wait has no completion event and the full campaign hangs.

## Scope

This increment implements AArch64 Linux userspace behavior for:

- distinct monotonic and realtime clock values;
- setting `CLOCK_REALTIME` while rejecting `CLOCK_MONOTONIC` and invalid IDs;
- checked `timespec` input and overflow-safe realtime offset calculation;
- per-process POSIX timer ownership and bounded timer state;
- relative and absolute one-shot or periodic timer arming;
- `SIGEV_SIGNAL` notification, including the Linux default when `sevp` is null;
- CPU0 timer-tick expiration, periodic rescheduling, and process-directed
  signal wakeup;
- meaningful `timer_gettime`, timer disarm, replacement, and deletion; and
- process-exit/reset cleanup with no stale timer delivery.

The timer model supports both exposed clock IDs. Relative timers retain their
monotonic deadline when realtime is changed. Absolute realtime timers retain
their requested realtime deadline and therefore follow realtime clock changes.

## Explicit Non-Goals

This increment does not add high-resolution hardware timers, change the 100 Hz
kernel tick, implement `SIGEV_THREAD`, or redesign Linux task scheduling. It
does not add runner timeouts as a substitute for timer behavior. Timer overrun
reporting remains zero unless an existing POSIX test demonstrates that a
nonzero count is required; periodic expiration itself must still be correct.

The change is implemented and verified on AArch64 first. It must preserve host
tests and the source/build contracts for x86_64 and RISC-V64, but it does not
claim that those architectures can execute the POSIX campaign yet.

## Architecture

### Clock Domains

The monotonic clock remains derived directly from `monotonic_nanos()` and can
never move backward through `clock_settime`. A system-wide signed realtime
offset represents:

```text
realtime = monotonic + realtime_offset
```

`clock_gettime(CLOCK_REALTIME)` applies the offset with checked arithmetic;
`clock_gettime(CLOCK_MONOTONIC)` returns the underlying monotonic value.
`clock_settime` accepts only `CLOCK_REALTIME`, copies and validates the complete
userspace `timespec`, and atomically replaces the offset. Nanoseconds outside
`0..1_000_000_000`, negative seconds, overflow, unsupported clock IDs, and bad
userspace ranges return the corresponding Linux error without changing state.

### POSIX Timer State

The existing process-owned timer-handle list remains the ownership authority
so fork, cleanup, and resource-accounting contracts stay intact. Each owned
handle gains a bounded timer record containing:

- timer handle and selected clock ID;
- notification signal;
- armed/disarmed state;
- absolute deadline in the selected clock domain; and
- periodic interval in nanoseconds.

No raw userspace pointer is retained. Creation validates the selected clock
and notification mode before publishing the handle and record. Deletion
removes both together. Forked children do not inherit parent POSIX timers, as
required by POSIX.

### Arming And Querying

`timer_settime` copies and validates a complete `itimerspec`. A zero
`it_value` disarms the timer. With `TIMER_ABSTIME`, the supplied value is stored
as a deadline in the selected clock domain. Without it, the duration is added
to the current selected clock with checked arithmetic. Re-arming atomically
replaces the previous deadline and interval. If requested, `old_value` is
filled from the prior state before replacement.

`timer_gettime` reports a nonnegative remaining value and the configured
interval. An expired one-shot timer reports zero. Copyin and copyout ranges are
validated before state mutation, and unknown flags, invalid time fields,
overflow, unowned handles, and inaccessible buffers return real errors.

### Expiration And Signal Delivery

The existing CPU0 Rust timer handler scans the fixed process/timer capacity
after the scheduler tick. For each armed timer whose selected clock has
reached its deadline, it records one pending process-directed signal. One-shot
timers become disarmed. Periodic timers advance by whole intervals until their
next deadline is strictly in the future, avoiding repeated IRQ delivery for
the same elapsed period while preserving phase.

Signal delivery uses `queue_process_linux_signal_and_wake` with a standard
pending signal. That existing path completes a matching `sigwait`, wakes the
backing scheduler thread, or leaves the signal pending for normal delivery.
Expiration is keyed by process identity and owned handle, so a deleted timer or
reused process slot cannot receive a stale notification.

Realtime clock changes do not require walking or rewriting timer records.
Absolute realtime deadlines naturally follow the new clock value because the
expiry scan compares against current realtime. Relative timers use a monotonic
deadline so setting realtime cannot shorten or extend them.

### Concurrency And Cleanup

Clock offset updates and timer-state mutation follow the existing interrupt
masking rules used by Linux process resources. The IRQ path performs no
allocation and collects only bounded signal targets before calling the signal
wakeup layer. Process teardown and runtime reset disarm and remove all timer
records before process identities can be reused.

## Error Behavior

The implementation returns:

- `EINVAL` for setting a non-settable or unknown clock, invalid nanoseconds,
  negative time, unknown timer flags, unsupported notification mode, or an
  invalid/unowned timer ID;
- `EFAULT` for unreadable input or unwritable output ranges;
- `EOVERFLOW` where a valid representation cannot be converted or added; and
- success only after the requested clock or timer state has been applied.

No validation failure partially updates the realtime offset, timer ownership,
or armed state.

## Testing

Development follows red-green-refactor:

1. Pure host tests fail first for realtime-only settable-clock validation,
   timespec validation, offset calculation, relative versus absolute deadline
   selection, one-shot expiry, periodic rescheduling, and disarming.
2. Source integration contracts fail first unless `clock_settime` copies and
   applies its input, the CPU0 tick invokes POSIX timer expiration, and timer
   expiry uses the existing process-signal wake path.
3. Focused tests turn green before the full host unit and integration suites.
4. Formatting, `git diff --check`, Verus syscall checks, and the warning-free
   AArch64 release build remain mandatory.
5. A commit-matched AArch64 kernel and POSIX stage rerun
   `clock_settime/20-1.c` and `clock_settime/4-1.c`. The former must pass with
   `EINVAL` for `CLOCK_MONOTONIC`; the latter must terminate and pass after
   receiving its timer signal.
6. The complete `clock_settime` API group then runs without a hang, followed by
   the broader time group and full campaign when execution time permits.

Coverage and static-analysis results must report the exact commands, tool
availability, failures, and captured artifacts. No coverage percentage or
finding count is inferred when Tarpaulin or Coverity cannot complete.

## Success Criteria

This increment is complete only when:

- `CLOCK_MONOTONIC` cannot be set and remains independent of realtime changes;
- realtime reads reflect a successfully validated `clock_settime` value;
- relative timers are unaffected by later realtime changes;
- absolute realtime timers react correctly to backward and forward changes;
- due timers queue the configured signal and wake a matching `sigwait`;
- one-shot, periodic, disarm, delete, exit, and reset paths leave consistent
  bounded state;
- the two reproduced Open POSIX tests pass on the matching AArch64 artifacts;
  and
- host, Verus, formatting, and warning-free AArch64 build gates remain green.

Passing these gates removes the observed campaign hang and improves the time
API group. It does not by itself establish full POSIX compliance.
