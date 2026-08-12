# POSIX Fork Record-Lock Conformance Design

## Problem

`conformance/interfaces/fork/11-1.c` reaches `test_start` and then remains
silent until the 30-second host watchdog classifies it as a timeout. A focused
fresh-disk run at SMROS commit `caf658f` reproduced the timeout in one boot with
no guest `test_end`, restart, or fatal kernel marker.

The hang is not evidence that SMROS fails to schedule the fork child. A
checkpoint executable that performs the same fork, child `pthread_create`,
worker call, `pthread_join`, child exit, and parent `waitpid` completed every
operation under SMROS. It showed that the child worker's
`ftrylockfile(stdout)` returned busy.

The pinned 2004 test is itself defective:

- it uses a libc `FILE` stream lock while claiming to test process-associated
  file-lock inheritance;
- its worker calls the test framework's `FAILED` macro while `stdout` is busy;
- `FAILED` reports through `output`, which calls `printf` and `vprintf` on the
  same `stdout` stream; and
- the resulting self-deadlock prevents the child from exiting and the parent
  from completing `waitpid`.

The exact unmodified pinned source also timed out after five seconds on native
x86-64 Linux with no output. Linux Test Project replaced this test in 2017 with
a direct `fcntl(F_GETLK/F_SETLK)` record-lock test. Its maintained version tests
the intended POSIX rule without the stream-lock/reporting deadlock.

SMROS also has a genuine conformance gap behind the defective test. Its
`fcntl` implementation currently supports descriptor duplication and
descriptor/status flags only. It rejects `F_GETLK`, `F_SETLK`, and `F_SETLKW`,
so the maintained test cannot yet execute its assertion.

## Scope

This change replaces the defective pinned test through the repository's
audited patch series and implements the process-associated advisory record-lock
behavior required by that replacement on regular FxFS files.

The scope includes:

- the pinned Open POSIX Test Suite patch ledger and its non-weakening audit;
- Linux AArch64 `fcntl` commands `F_GETLK`, `F_SETLK`, and `F_SETLKW`;
- Linux `struct flock` copy-in and copy-out validation;
- process-owned byte-range read/write/unlock records keyed by stable FxFS file
  identity;
- fork, descriptor-close, process-exit, and launch-reset lock lifecycle; and
- focused and complete `fork` runtime evidence.

The scope does not include BSD `flock`, open-file-description locks, leases,
mandatory locking, signal-driven I/O ownership, remote filesystem locking, or
an unbounded general-purpose lock manager. The change must not special-case the
test binary, lower its timeout, synthesize a pass, or hide a truthful failure.

## Selected Approach

Port Linux Test Project's maintained `fork/11-1.c` replacement into the pinned
patch series, preserving its direct record-lock assertion. Implement a bounded,
allocation-free record-lock core for SMROS and connect it to the existing Linux
descriptor and process lifecycle.

The maintained assertion is:

1. the parent creates a temporary regular file;
2. the parent places a write lock over bytes `[0, 100)`;
3. the process forks;
4. the child asks `F_GETLK` about an overlapping subrange and must observe the
   parent's conflicting write lock;
5. the child attempts `F_SETLK` over that subrange and must receive `EACCES` or
   `EAGAIN`; and
6. the child returns a real PTS status that the parent propagates.

This tests the POSIX property at the kernel API boundary. The child inherits
descriptors and shared open descriptions, but it does not inherit the parent's
process-owned record locks.

## Test-Suite Patch And Provenance

Add a dedicated patch after the existing AIO and `sched_setparam` corrections
in `third_party/posixtest/patches/series`. The patch replaces only
`conformance/interfaces/fork/11-1.c` with the maintained LTP test logic.

The repository audit must prove that the patch:

- remains listed exactly once in the ordered series;
- removes `flockfile`, `ftrylockfile`, and the local `testfrmw` inclusion from
  this test;
- adds `F_GETLK` and `F_SETLK` checks with a real overlapping range;
- accepts only `EACCES` or `EAGAIN` as the expected conflicting-lock result;
- retains `fork` and `waitpid` status propagation;
- contains no unconditional `PTS_PASS`, timeout reduction, SMROS branch, or
  assertion bypass; and
- is included in the computed patch checksum and staged manifest identity.

The pinned source checkout remains generated state. Tracked behavior lives in
the patch file and audit test, not in an ad hoc edit under `target/posix/src`.

## Record-Lock Model

### File identity

Locks are keyed by a stable regular-file identity, not by descriptor number or
open-description ID. Descriptor numbers are process-local and may be reused.
Open descriptions are shared by `dup` and inherited across `fork`; keying by
them would incorrectly make the child's operations appear to own the parent's
locks.

The FxFS-backed Linux file record therefore carries the existing stable
`FxfsCursor::object_id()` value for the underlying file. All descriptors that
refer to the same FxFS object ID resolve to the same lock identity even when
they use different open descriptions.

### Owner identity

Each record lock is owned by the Linux process ID (`tgid`) that requested it.
Threads in one process share lock ownership. A fork child receives a distinct
PID and starts with no lock records, while the parent's records remain active.

### Ranges and types

The core models `F_RDLCK`, `F_WRLCK`, and `F_UNLCK` over normalized half-open
byte ranges. `l_whence` values `SEEK_SET`, `SEEK_CUR`, and `SEEK_END` resolve
through the descriptor cursor and current file size. A positive `l_len`
describes `[start, start + len)`. A zero `l_len` extends through end-of-file and
future growth. A negative `l_len` describes `[start + len, start)`. An invalid
lock type or `l_whence`, or a resolved endpoint below zero, returns `EINVAL`.
Overflow while converting the descriptor position or file size to `off_t`, or
while adding `l_start` and `l_len`, returns `EOVERFLOW`.

Read locks conflict with write locks from another process. Write locks conflict
with read or write locks from another process. A process never conflicts with
itself. Setting a lock replaces that owner's overlapping region; unlocking
removes only the requested owner/range and may split an existing record. The
bounded core coalesces adjacent compatible records and returns `ENOLCK` rather
than silently dropping state if capacity is exhausted.

### `F_GETLK`

SMROS copies in a complete Linux AArch64 `struct flock`, validates and
normalizes its request, and searches for the first conflicting lock owned by a
different process. If no conflict exists it copies out `l_type=F_UNLCK`. If a
conflict exists it copies out the conflicting type, normalized `SEEK_SET`
range, and owner PID. User-memory validation or copy failure returns `EFAULT`
without changing lock state.

### `F_SETLK` and `F_SETLKW`

`F_SETLK` installs or removes the normalized owner range atomically. A conflict
returns `EAGAIN` without changing state.

`F_SETLKW` uses the existing Linux task block/wake framework. On conflict it
registers a bounded waiter for the exact file/range/type, blocks the current
task, and retries after a relevant unlock, close, or process exit. Signal
interruption returns `EINTR`. Waiter registration failure returns `ENOLCK`.
No task blocks while holding the global syscall state lock.

Although the maintained `fork/11-1.c` uses only nonblocking operations, all
three standard commands are implemented together so the advertised `fcntl`
surface does not treat `F_SETLKW` as an unsupported alias.

## Descriptor And Process Lifecycle

Record locks are process-associated rather than open-description-associated:

- `dup`, `dup3`, and `F_DUPFD*` do not duplicate lock records;
- `fork` clones descriptors and open-description references but creates no
  child-owned lock records;
- closing any descriptor for a file releases all locks that the closing
  process owns on that file, even when another descriptor remains open;
- process exit releases all locks owned by that process and wakes affected
  waiters;
- fork rollback does not remove parent locks;
- failed lock operations leave the complete prior lock table unchanged; and
- launch reset clears records and waiters after Linux tasks have been retired.

Cleanup integrates with existing descriptor-close and process-resource release
paths so normal close, group exit, signal termination, and test isolation share
one release operation. Close-on-exec lock release is deferred until SMROS has a
real descriptor-closing `execve` path; the current `sys_execve` stub does not
perform an executable-image transition or close `FD_CLOEXEC` descriptors.

## Error Handling And Atomicity

The syscall validates the descriptor as a regular seekable file before reading
the request. Invalid descriptors return `EBADF`; unsupported descriptor types
return `EINVAL`. A null, unreadable, or unwritable `struct flock` pointer
returns `EFAULT` according to the command's copy direction.

Range arithmetic is checked. Conflict, capacity, and memory errors occur before
publication. Operations that split or replace records compute their complete
bounded result before committing it, so a failure cannot partially unlock or
install a range. Wakeups occur only after the committed table no longer blocks
the waiter.

## Test-Driven Implementation

The RED phase consists of independent failures:

1. a source-audit test fails because the maintained fork replacement is absent;
2. lock-core unit tests fail because range normalization, conflict lookup,
   replacement, split, coalescing, and lifecycle APIs do not exist;
3. integration contracts fail because `sys_fcntl` does not route the three
   record-lock commands or connect close/exit cleanup; and
4. the current staged `fork/11-1.c` reproduces the 30-second timeout.

After the test-suite patch, native Linux must complete the replacement with a
real PTS result. After the kernel implementation, focused SMROS runs must reach
guest `test_end` without watchdog intervention. The runtime result is not
required to be forced to pass during intermediate work: any assertion failure
must remain visible until the corresponding kernel semantics are corrected.

## Verification Matrix

Acceptance requires fresh evidence from:

- the source patch audit and complete POSIX host-tool suite;
- focused lock-core host unit tests and Linux lifecycle integration contracts;
- all host unit and integration tests;
- script and formatting checks;
- pinned AArch64 POSIX stage rebuild and verification;
- warning-as-error optimized AArch64 kernel build and layout validation;
- all wired Verus proof suites;
- a native Linux run of the corrected `fork/11-1.c`;
- three independent fresh-disk SMROS runs of corrected `fork/11-1.c`;
- the complete staged `fork` API selection on a fresh disk; and
- adjacent fork, descriptor, close, `fcntl`, thread, and wait/exit canaries.

Each SMROS runtime attempt must retain manifest provenance and raw serial logs.
Focused runs must finish below the 30-second watchdog with no restart, fatal
marker, host-watchdog result, or positive resource delta. The complete API run
must preserve truthful pass, fail, unresolved, unsupported, and untested
statuses for unrelated assertions.

Coverage and quality evidence are regenerated after the implementation.
Tarpaulin and Coverity are run when installed; otherwise their unavailability
is reported with null metrics rather than represented as passing.

## Completion Criteria

The corrected maintained assertion is part of the signed staged suite,
`fork/11-1.c` reaches a truthful guest terminal result in three fresh-disk runs,
the complete `fork` selection has no `fork/11-1.c` watchdog timeout, record-lock
ownership and lifecycle tests pass, all repository gates pass, and no
diagnostic executable or production test special case remains.
