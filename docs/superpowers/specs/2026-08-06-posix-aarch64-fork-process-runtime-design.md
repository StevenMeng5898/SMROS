# AArch64 POSIX Fork Process Runtime Design

Date: 2026-08-06

## Status

Approved direction: implement real AArch64 forked processes with eager private
page copying and explicit shared page backing. Copy-on-write is deferred until
the eager-copy lifecycle is correct and measured.

## Context

The current AArch64 POSIX runtime can execute shared-address-space clone
children, but it does not create a process when libc calls `fork()`:

- `sys_fork()` returns a monotonically allocated synthetic PID;
- `sys_wait4()` immediately returns the requested positive PID and writes a
  zero status;
- `sys_getppid()` always returns zero;
- `run_elf` enters EL0 with `TTBR0_EL1` set to zero;
- Linux mappings, the initial stack, descriptors, shared-memory attachments,
  container attributes, and related state live in one global
  `MemorySyscallState`; and
- the existing `PageTableManager` is a software bring-up model, not a complete
  process-owned AArch64 translation-table implementation.

The last complete 1,598-test campaign at `f39aaf6` reported 19 selected `fork`
tests: 9 passed accidentally against the synthetic return path, 1 failed, and
9 were unresolved. `WIFEXITED/1-3.c` also failed because no real child status
was produced. The four corrected `sched_setparam` tests now reach scheduling
assertions instead of heap corruption, and expose the same absent process
runtime.

This increment establishes the process foundation on AArch64. It is not a
claim that the memory group, all process APIs, or overall POSIX conformance is
complete.

## Goals

This increment must provide:

1. A real PID-bearing process record, parent/child relationships, process
   groups, and a task-to-process binding.
2. A process-owned AArch64 user address space selected whenever one of the
   process's tasks runs.
3. Transactional `fork()` and fork-compatible non-`CLONE_THREAD` `clone()`
   creation with the parent receiving the child PID and the child resuming the
   copied exception frame with register `x0 == 0`.
4. Eager copies of private mappings and the initial stack, with independent
   backing pages immediately after fork.
5. Shared backing for `MAP_SHARED` and existing SysV/POSIX shared-memory
   mappings so writes are visible in both processes.
6. POSIX inheritance rules for descriptors, signal state, timers, and
   process attributes required by fork.
7. Exit, default-signal termination, `SIGCHLD`, zombie retention, blocking and
   nonblocking wait, wait-status encoding, and one-time reaping.
8. Transactional rollback and launch cleanup that leave no process, task,
   page, descriptor reference, mapping, shared-memory attachment, scheduler
   thread, or kernel handle behind.
9. Detailed host and guest evidence with exact source, patch, manifest, build,
   result, serial-log, coverage, Verus, and Coverity provenance.

Every project-required optional POSIX group remains selected. No upstream
assertion, result code, timeout, group assignment, or report disposition may
be weakened to improve a result.

## Non-Goals

The following are separate increments:

- copy-on-write page faults and lazy private-page duplication;
- `vfork()` address-space suspension optimization;
- replacing a process image through `execve()`;
- x86_64 or RISC-V64 process address spaces;
- completing the named `shm_open`/`shm_unlink` namespace, named semaphores,
  message queues, timers, or optional scheduling policies; and
- claiming that all remaining `fork`, memory, base, or full-suite tests pass.

`vfork()` may use the correct eager-copy `fork()` behavior during this
increment. It must not share the parent's private address space.

Non-thread `clone()` accepts the exit signal and the namespace flags already
modeled by SMROS, applying namespace changes to the child rather than the
parent. Requests for `CLONE_VM`, `CLONE_FILES`, or `CLONE_SIGHAND` sharing
without `CLONE_THREAD` return `ENOSYS` in this increment. They must not be
silently treated as a POSIX fork.

## Chosen Architecture

### Process And Task Ownership

Add a fixed-capacity Linux process table alongside the existing Linux task
table. A process record owns:

- PID, parent PID, process-group ID, and launch identity;
- lifecycle state (`Reserved`, `Running`, `Zombie`, or `Reaped`);
- the scheduler task representing the process's initial thread;
- exit reason and encoded wait status;
- its user address-space root and mapping metadata;
- process-local descriptors and per-descriptor flags;
- signal dispositions and process-pending signals;
- process attributes currently stored in the global compatibility state; and
- references to shared kernel objects and shared page backings.

A Linux task record gains a process identity. `CLONE_THREAD` children retain
their parent's process identity and address-space root. `fork()` creates a new
process identity and one initial task. `getpid()` returns the process PID,
`gettid()` returns the task TID, and `getppid()` reads the live parent
relationship. PID and TID values remain monotonically allocated and are not
reused during one boot.

The pure lifecycle and selection rules belong in a host-testable shared logic
module. Scheduler, page allocator, raw user-memory access, and architecture
entry remain in the kernel-backed runtime module.

### System State Versus Process State

The current global compatibility state is divided by ownership rather than
blindly cloned.

System-wide state retains named IPC objects, shared page backings, kernel
object registries, global allocation counters, and persistent namespaces.
Each process owns its mapping list, program break, initial stack, descriptor
table, FxFS descriptor bindings, shared-memory attachment list, mount/chroot
view, capability state, signal actions, and launch accounting.

On fork:

- descriptor numbers and close-on-exec flags are copied;
- each copied descriptor references the same open description, file offset,
  pipe endpoint, socket, message queue, or shared-memory object as its parent;
- signal dispositions are copied, the calling thread's signal mask is copied,
  and child pending-signal queues start empty;
- interval timers and POSIX timers start disarmed in the child;
- outstanding AIO requests and scheduler wait registrations are not copied;
- process group, credentials, root/current directory view, capabilities, and
  namespace view are inherited; and
- other parent threads are not copied.

Reference increments occur before publication. Rollback decrements only the
references acquired by the reservation, so a failed fork cannot close or
unmap a parent resource.

### AArch64 User Address Spaces

Introduce a process-owned AArch64 translation-table type backed by page frames
from `PageFrameAllocator`. It must build valid 4 KiB translation-table walks
for the configured EL0 virtual-address range, map individual data pages with
read/write/execute/user permissions, unmap them, translate a user virtual
address for checked kernel copy operations, and release every table page on
destruction.

The existing single-table `PageTableManager` model must not be treated as the
fork implementation merely because it exposes a root pointer. The new tests
must prove that two different virtual pages do not alias one table entry, that
permissions survive mapping splits, and that destroying one root cannot
invalidate another root.

`CpuContext` gains the active `TTBR0_EL1` value. Both AArch64 context-switch
entry points save and restore that field and issue the required barrier and
TLB invalidation sequence before returning to a different user address space.
Kernel threads use the bootstrap TTBR0 value; Linux threads use their owning
process root. Complete Rust layout checks and assembly offset checks are
mandatory because an offset mismatch corrupts every scheduled context.

The first version may flush the local EL1 TLB on every address-space change.
ASID allocation and targeted invalidation are performance work, not a
correctness prerequisite for this increment. Linux process tasks remain
pinned to the existing AArch64 runtime CPU, so cross-CPU TLB shootdown is not
introduced here.

### Launcher And User Memory

`run_elf` creates the root process and its address space before loading the
main executable, interpreter, and initial stack. ELF segments and stack bytes
are written through checked address-space copy functions that resolve backing
pages; they are no longer written to arbitrary identity-mapped virtual
addresses with raw pointers. The launcher enters EL0 with the process's actual
translation root.

Linux `mmap`, `munmap`, `mprotect`, `mremap`, `brk`, and checked syscall
copy-in/copy-out operate on the current process selected through the current
Linux task. Mapping metadata and hardware page tables change in one
transaction. A failed mapping operation releases newly allocated pages and
leaves the old mapping and page table unchanged.

Private anonymous, file-backed private, ELF, stack, and brk pages have one
owner. Shared mappings reference a system-wide backing object whose page list
is reference counted. An unmap removes only that process's mapping and frees a
shared page only after the final mapping/object reference is gone.

### Eager Fork Transaction

`fork()` executes these ordered stages while the child scheduler thread is
suspended and invisible:

1. Validate that the caller has a live process binding and a complete AArch64
   syscall exception frame.
2. Reserve a PID, process-table slot, task-table slot, and suspended scheduler
   thread.
3. Allocate an empty child address-space root and copy process metadata.
4. For every private page, allocate a child page and copy its bytes. For every
   shared mapping, acquire references to the existing shared backing pages.
5. Clone descriptor entries and acquire their open-description/object
   references according to the inheritance rules above.
6. Copy the calling thread's exception frame, set child `x0` to zero, retain
   the post-syscall return PC and stack pointer, clear pending signals, and bind
   the start image to the child root.
7. Publish the process and task records, then publish the scheduler thread.
8. Return the child PID in the parent.

Any failure before the final publication unwinds stages in reverse order. A
publication failure also removes the task and process records before the
scheduler thread is terminated. The parent sees `EAGAIN` for exhausted task,
process, or scheduler capacity and `ENOMEM` for address-space/page-allocation
failure. It never receives a PID for an unstartable child.

`sys_exit()` retires only the calling task and changes process state when that
task was the final task. `sys_exit_group()` terminates only tasks in the
caller's process; it no longer resets the complete Linux runtime. A descendant
process exit never invokes the `run_elf` completion hook. Only final exit of
the launch root publishes the test program's outcome, after descendant
cleanup has been initiated.

### Exit, SIGCHLD, And Wait

When the final task in a process exits, process-owned execution resources are
released and the process becomes a zombie containing only identity,
relationship, accounting, and wait status. Normal exit status is encoded as
`(code & 0xff) << 8`. Signal termination records the terminating signal in the
low seven bits and records the core-dump bit only if a core was actually
created.

If `SIGCHLD` is ignored, the child is fully reaped on exit and no `SIGCHLD` is
queued. If `SA_NOCLDWAIT` is set with a non-ignored disposition, the child is
fully reaped and `SIGCHLD` is queued without retaining a zombie. In both
cases, a blocked waiter is woken and returns `ECHILD` once no matching live
child remains. Otherwise the parent receives `SIGCHLD` and the child remains
a zombie until one successful wait reaps it.

`wait4`/`waitpid` supports exact PID, any child (`-1`), the caller's process
group (`0`), and a selected process group (`pid < -1`). `WNOHANG` returns zero
only when a matching live child exists but none is waitable. No matching child
returns `ECHILD`; invalid option bits return `EINVAL`; a bad status pointer
returns `EFAULT` without reaping. A successful wait copies status first and
then reaps exactly one zombie. Repeated waits cannot observe it again.

If a parent exits first, its live children are reparented to an internal
per-launch reaper record, not to an already exiting user process. End-of-launch
cleanup forcibly retires any surviving descendant tasks and releases all
process resources before the next POSIX test starts.

### Signals And Process Termination

Signal routing distinguishes task-directed and process-directed delivery.
Process-directed signals select a live task in the target process. Default
terminate actions exit the whole process, preserve the signal-derived wait
status, wake a waiting parent, and queue `SIGCHLD`. `SIGKILL` and `SIGSTOP`
remain unmaskable. Fork inheritance copies signal actions and the calling
thread's mask but not pending signals or active signal frames.

This work extends the current thread signal machinery; it does not introduce a
second independent signal table.

## Error Handling And Invariants

The runtime maintains these invariants:

- every published task refers to exactly one published process;
- every running Linux process has exactly one live address-space root;
- threads in one process use the same root, and different processes use
  different roots;
- a private page has one process owner, while shared pages have counted
  references;
- a zombie has no scheduler thread and no private execution pages;
- a reaped process cannot be selected by wait or signal lookup;
- parent-visible PID publication is the final fallible fork step;
- user copy faults return `EFAULT` and do not partially reap, publish, or
  replace state; and
- launch reset returns all process, mapping, descriptor, page-table, task,
  scheduler, IPC-attachment, timer, and handle counts to their pre-launch
  values.

Capacity limits are explicit and checked. Integer overflow in address ranges,
page counts, PID conversion, wait selectors, or status destinations is an
error rather than a wrapping operation.

## Testing Strategy

Implementation follows red-green-refactor in bounded commits.

### Host Logic Tests

Pure tests fail first for:

- PID allocation and parent/process-group relationships;
- task-to-process binding and thread sharing of one process;
- exact-PID, any-child, and process-group wait selection;
- `WNOHANG`, `ECHILD`, invalid options, and one-time reaping;
- normal and signal wait-status encoding;
- signal inheritance, pending-signal clearing, and `SIGCHLD` selection;
- descriptor/open-description reference inheritance;
- eager private-page clone versus shared-page reference clone; and
- reverse-order rollback at every fork reservation stage.

### Integration And Architecture Contracts

Integration tests fail first unless:

- Rust `CpuContext` layout and both assembly context-switch paths agree on the
  TTBR0 offset;
- the launcher creates and installs a nonzero process root;
- `sys_fork`, non-thread `sys_clone`, `sys_exit`, and `sys_wait4` route through
  the process runtime rather than synthetic results;
- memory syscalls resolve the current process state;
- raw ELF and stack writes are replaced by checked address-space copies; and
- cleanup metrics include live processes, zombies, private/shared pages, and
  page-table pages.

Low-level tests cover page-table walk boundaries, permissions, translation,
unmap, independent roots, complete destruction, and injected allocation
failures.

### AArch64 Guest Canaries

Small guest canaries run on fresh private FxFS disks and prove, in order:

1. parent receives a positive PID, child receives zero, and both execute;
2. a child private write does not change the parent's value;
3. a child `MAP_SHARED` write is visible to the parent;
4. inherited pipe/file descriptors refer to the same open descriptions;
5. normal child exit produces `WIFEXITED` and the exact `WEXITSTATUS`;
6. signal termination produces `WIFSIGNALED` and the exact `WTERMSIG`;
7. `WNOHANG`, `ECHILD`, and repeated wait behavior are correct; and
8. every terminal attempt has measured non-positive resource deltas.

The initial upstream canaries include `WIFEXITED/1-3.c` and selected `fork`
tests that exercise private memory, shared mappings, descriptors, signal
inheritance, and child status. Selection is based on reading each pinned
source, not on its previous synthetic pass status.

### Campaign Gates

After canaries, run the complete selected `fork` and base groups, then the
affected memory/shared-memory tests, and finally the full 1,598-test AArch64
campaign. A result is reported exactly as emitted by the pinned Open POSIX Test
Suite. Independent named-shared-memory failures remain failures or unresolved
until their own implementation increment.

Mandatory regression gates remain:

- host unit tests and integration contracts;
- all POSIX tooling tests;
- AArch64 release build and linker-layout verification;
- staged-source and manifest verification;
- event-stream integrity and report generation;
- per-attempt resource evidence and fatal-marker scans;
- Tarpaulin line coverage with uncovered-line detail;
- Verus coverage audit and proof results; and
- Coverity analysis when installed, otherwise an explicit unavailable result
  containing the missing-command evidence.

No unavailable timeout snapshot, coverage result, or static-analysis result is
reported as zero or passing.

## Delivery Boundaries

The implementation is developed on a dedicated branch in a new isolated
worktree. Existing worktrees, ignored campaign artifacts, the repository-root
disk, and user-owned service-file edits are preserved. Generated disks and run
directories stay below that worktree's `target/posix/aarch64/` hierarchy.

The branch is eligible for a local fast-forward merge to `master` only after
the focused guest behavior, full host regression suite, AArch64 build/layout,
stage verification, resource cleanup, and evidence document all pass. A full
campaign regression remains visible even when unrelated later clusters still
fail.

## Success Criteria

This increment is complete only when:

- the AArch64 root ELF runs from a nonzero process-owned TTBR0 root;
- `fork()` launches a real child with a distinct PID and address-space root;
- private data diverges and shared data remains visible across parent/child;
- inherited descriptors and signal state follow the specified rules;
- normal and signal child termination produce exact wait status;
- blocking wait, `WNOHANG`, child selection, `ECHILD`, and one-time reaping
  behave correctly;
- allocation or publication failure leaves the parent unchanged;
- no focused or campaign run leaks process, task, mapping, page, descriptor,
  IPC, timer, scheduler, or handle resources;
- all existing host and POSIX tooling tests pass; and
- the results document distinguishes this process-foundation improvement from
  remaining POSIX failures and unavailable quality tools.

Passing these gates does not claim copy-on-write, `execve`, named IPC,
optional scheduling policies, another architecture, or 100% POSIX compliance.
