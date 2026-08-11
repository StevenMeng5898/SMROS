# POSIX ELF FxFS Persistence Batching Design

## Problem

The AArch64 Open POSIX Test Suite AIO group intermittently exceeds the host
runner's 30-second per-test watchdog. The affected tests are not deadlocked.
They eventually complete when run directly in the guest, including
`conformance/interfaces/aio_return/3-1.c`, which took about 28.6 seconds in an
unmodified diagnostic run.

GDB initially sampled CPU0 in `compiler_builtins::mem::memcpy`, called by
`Vec<u8>::extend_from_slice` during `FxfsState::persist`. Instruction stepping
proved that this loop advances by eight bytes per iteration and reaches its
exit with the current pointer equal to the end pointer. The copy routine is not
the root cause.

The actual problem is persistence amplification. Every FxFS create, write,
append, truncate, attribute update, and delete calls `persist()`. Each call
serializes the complete selected filesystem image, writes the image to the
alternate block slot, and flushes the block device. A GDB counter observed 106
full `FxfsState::persist` entries by test 49 of the 80-test AIO group. Files and
journal records accumulate across the campaign, so later mutations repeatedly
copy and flush a progressively larger image.

## Scope

This change batches ordinary FxFS mutations made by one launched ELF process
and preserves explicit POSIX synchronization semantics. AArch64 is the first
runtime acceptance architecture, consistent with the current porting order.

The change does not alter Open POSIX Test Suite programs, weaken their
assertions, hide non-pass results, increase the watchdog, change the on-disk
FxFS format, or implement incremental journaling. Existing AIO functional
failures remain separate conformance work and must stay visible in reports.

## Considered Approaches

### Selected: one persistence transaction per ELF lifecycle

Attach the existing `FxfsPersistGuard` to the accepted `run_elf` lifecycle.
Ordinary mutations set `persist_pending` while the guard is active. Releasing
the lifecycle resource after process teardown resumes persistence and commits
the final image once.

This approach uses existing ownership and rollback mechanisms, changes no disk
format, and reduces a mutation-heavy test from many full-image flushes to one
normal completion flush plus any synchronization explicitly requested by the
program.

### Deferred: incremental journal or dirty-object persistence

An incremental on-disk design would scale better for general workloads, but it
requires a new recovery protocol, compatibility rules, corruption handling,
and substantially broader verification. It is not needed to remove this
watchdog defect safely.

### Rejected: increase or retry the watchdog

A larger timeout would only mask repeated synchronous full-image commits. Its
cost would continue to grow with campaign state, and host retries would rerun
tests after an avoidable infrastructure timeout.

## Lifecycle Ownership

`run_elf::ActiveRun` changes its resource type from `()` to
`fxfs::FxfsPersistGuard`. After `run_elf_start_transition` accepts a launch, the
launcher suspends ordinary persistence and attaches the guard through the
existing launch-ID-aware resource transition before binding or creating the
scheduler thread.

The resource remains owned by the exact accepted launch ID through all of the
following paths:

- normal process exit;
- dynamic-loader or mapping failure;
- CPU-binding failure;
- scheduler-thread creation failure;
- stale or repeated completion attempts; and
- explicit launch-state cleanup.

An attach failure returns ownership of the guard to the caller, where dropping
it immediately balances the suspension. A later launch failure clears the
accepted request, and its owned guard is released by the existing completion
object. No guard may be leaked, attached to a successor launch, or released by
a stale launch ID.

On normal completion, `complete_active_run` takes the accepted request, drops
its persistence resource after Linux process teardown, and only then captures
the end tick and dispatches the observer outcome. Consequently, the measured
test duration includes its single final persistence commit, and the next test
does not begin while the previous test has uncommitted ordinary mutations.

## FxFS Persistence Semantics

FxFS retains nested suspension through `persist_suspended` and coalescing
through `persist_pending`:

1. An ordinary mutation while no guard is active commits as it does today.
2. An ordinary mutation while one or more guards are active sets
   `persist_pending` and returns without serializing or flushing.
3. Releasing an inner nested guard does not commit.
4. Releasing the outermost guard commits once when work is pending.
5. Releasing a guard with no pending mutation performs no block write.

The existing two-slot full-image write remains the commit operation. Storage
success and failure continue to update `last_sync_ok` and
`last_storage_error`; batching must not report a failed block operation as a
successful internal sync.

## Explicit Synchronization

Lifecycle batching must not turn `sync`, `fsync`, or `fdatasync` into no-ops.
FxFS therefore exposes a forced persistence operation that bypasses
`persist_suspended`, commits the current complete image immediately, and
clears `persist_pending` only for mutations included in that successful
commit. Mutations after the forced commit become pending again and are
committed when the outer lifecycle guard is released.

The syscall behavior is:

- `sync` requests a forced global FxFS commit and retains its existing
  no-error-return syscall contract;
- `fsync` performs existing descriptor validation, forces the commit, and
  returns `EIO` if FxFS cannot serialize, write, or flush the image;
- `fdatasync` continues to share `fsync` behavior because FxFS persists data
  and required metadata as one image; and
- `sync_file_range` retains its existing delegation to `fsync`.

No explicit synchronization call releases the lifecycle guard. It creates a
durability point inside the still-active process transaction.

## Concurrency And Ordering

SMROS currently permits one observed `run_elf` request at a time and pins the
Linux runtime to CPU0. The global FxFS state and the run lifecycle therefore
retain their existing serialization model. This change adds ownership, not a
new lock or parallel filesystem execution path.

The persistence guard is attached before the launch thread can run. Process
teardown occurs while the guard is still active so descriptor, shared-memory,
and exit cleanup mutations are included. The guard is released before the
POSIX harness receives the terminal outcome, preventing overlap with the next
selected test.

## Error Handling

If the guard cannot attach to the accepted launch ID, launch setup fails closed
and both the launch state and persistence suspension are balanced. Existing
busy, thread, mapping, and infrastructure errors keep their current external
classification.

Ordinary deferred mutations preserve their current FxFS return behavior. The
final guard-triggered commit records any storage failure in FxFS state even
though Rust `Drop` cannot return it to an already exited process. Explicit
`fsync` and `fdatasync` calls can still observe commit failure as `EIO` while
the process is alive.

Arithmetic, image-capacity, allocation, block-write, and block-flush errors
remain real errors. The repair must not convert them into a successful commit
or clear a pending mutation after a failed forced commit.

## Test-Driven Implementation

The first tracked change is a focused host integration regression that fails
against the current launcher. It requires:

- `ActiveRun` to own an `FxfsPersistGuard`;
- the accepted launch ID to receive a guard before thread publication;
- every setup failure path to clear or return the owned resource;
- completion to release the resource before measuring and dispatching the
  outcome; and
- `sync`, `fsync`, and `fdatasync` to reach forced FxFS persistence.

Existing host lifecycle tests already prove exact-ID attachment, stale-launch
isolation, balanced cleanup, and release-before-callback ordering. They remain
part of the focused regression gate. Production code is changed only after the
new contract has been observed failing for the missing persistence ownership.

## AArch64 Runtime Acceptance

Acceptance requires fresh disks and the unmodified pinned Open POSIX Test Suite
stage. The following evidence is mandatory:

- three focused runs of `aio_cancel/5-1.c`, retaining the clone-address-space
  regression coverage;
- focused runs of previously slow `aio_read/7-1.c`, `aio_error/2-1.c`,
  `aio_return/3-1.c`, and `aio_return/3-2.c`;
- the complete `aio_cancel` API selection;
- the complete 80-test AIO group with zero watchdog timeouts, zero runner
  restarts, unique terminal results, and no positive resource deltas;
- pthread create, TLS, join, fork, and `WIFEXITED` canaries; and
- a reboot/readback check demonstrating that the final batched filesystem
  image remains loadable.

Focused tests that are expected by upstream to fail must still terminate with
their truthful PTS result. The performance repair is successful when every
launched test reaches a terminal event without watchdog recovery; it does not
claim that unrelated AIO assertions now pass.

## Repository And Quality Gates

The implementation must pass:

- `make host-fmt-check`;
- `make script-check`;
- `make ut`;
- `make it`;
- `make posix-tool-test`;
- `make aarch64-warning-check`;
- `make verus`; and
- `git diff --check`.

Host coverage and Coverity are captured using the repository's existing
quality-evidence format. Missing external tools are recorded as unavailable;
no coverage percentage or finding count is invented. The detailed POSIX report
must contain the documented seven artifacts and agree on manifest, build,
commit, architecture, selection, terminal statuses, resource deltas, coverage,
and static-analysis availability.

## Completion Criteria

The repair is complete when ordinary per-process mutations are coalesced,
explicit synchronization still forces durable persistence, focused host and
AArch64 regressions pass, the full AIO group completes without timeout or
restart on a fresh disk, all repository gates pass, and the detailed report
truthfully preserves every upstream non-pass result and unavailable quality
tool.
