# AArch64 Clone Address-Space Repair Design

## Problem

`conformance/interfaces/aio_cancel/5-1.c` submits 128 asynchronous writes. The
glibc AIO implementation creates one worker with `clone(2)`, cancels 127 queued
requests, and waits for the running request to finish. On SMROS the worker never
executes its first libc instruction, leaving one request in `EINPROGRESS`
forever.

Live AArch64 GDB evidence at the hang showed:

- `ELR_EL1=0x120fb3f8`, the `cmp x0, #0` instruction immediately after glibc's
  clone syscall;
- `ESR_EL1=0x82000006`, an instruction-abort translation fault;
- `FAR_EL1=0x120fb3f8`, matching the attempted libc instruction fetch; and
- `x0=-38`, written by SMROS's generic synchronous-exception fallback before it
  retries the same faulting address.

The clone child is created as a fresh scheduler thread. Its initial
`CpuContext.ttbr0_el1` therefore contains the kernel bootstrap root. Unlike the
working fork child path, the clone path neither configures the scheduler thread
with the owning Linux process root nor installs that root in its final EL0
startup assembly. The child consequently attempts to fetch process code through
the bootstrap translation root and faults indefinitely.

## Scope

This change repairs AArch64 `CLONE_THREAD` address-space ownership and proves the
specific POSIX AIO hang is removed. It does not redesign the scheduler, change
glibc, weaken Open POSIX Test Suite assertions, or add AIO-specific behavior to
the kernel.

The production change is limited to the existing clone reservation/start path,
the AArch64 clone startup image, and its assembly transfer. Existing diagnostic
serial probes and the diagnostic POSIX stage are removed before verification.

## Selected Approach

Mirror the established AArch64 fork child invariant at both ownership
boundaries:

1. The suspended scheduler thread is configured with the owning process's user
   stack, TLS, and translation root before it can be published.
2. The clone startup image carries the same translation root and the final EL0
   transfer installs it with the required TLB maintenance immediately before
   restoring the copied register frame and executing `eret`.

The two installations are intentional. Scheduler metadata is correct from the
thread's first scheduled entry, while the final transfer reasserts the process
root at the architecture boundary just as the fork path does.

## Components And Data Flow

### Clone reservation

`linux_task::reserve_clone` obtains the current process identity and
`linux_process_memory::current_root_paddr()` while the parent still owns the
active syscall frame. A zero or unavailable root is rejected.

After reserving the Linux task and inheriting its signal mask, reservation
configures the unpublished scheduler thread through
`CpuContext::set_linux_process_start(user_sp, tls, root_paddr)` and binds its
scheduler metadata to the current process. Only then does it publish the
`LinuxCloneSlot` startup record.

The slot's `Aarch64CloneStart` gains a `root_paddr: u64` field after `tls`. Its
compile-time offset assertion becomes part of the Rust/assembly ABI contract.

### Clone publication

The existing transaction order remains unchanged:

1. validate the syscall and user TID destinations;
2. allocate a suspended scheduler thread;
3. reserve and fully configure the clone child;
4. copy parent/child TIDs transactionally; and
5. publish the Linux task and then the scheduler thread.

No partially configured child becomes runnable.

### AArch64 EL0 transfer

`start_linux_clone_child` loads `root_paddr` from the startup image and performs
the same architectural sequence used by `start_linux_process_child`:

```asm
msr ttbr0_el1, x17
dsb ish
tlbi vmalle1is
dsb ish
isb
```

This sequence occurs with IRQs masked, before the copied general-purpose
registers are restored and before `eret`. The existing user SP, return PC,
PSTATE, TLS, floating-point state, SIMD state, and general-purpose register
restore order is otherwise preserved.

## Error Handling And Rollback

Failure to obtain a valid process root, configure the suspended thread, or bind
its process identity fails clone reservation before publication. The reserved
Linux task is rolled back. `sys_clone` then terminates the still-suspended
scheduler thread through its existing failure path.

Failures after TID copyout continue to restore the original user values before
rolling back the task and scheduler thread. The change adds no cleanup after a
child becomes runnable and does not convert kernel failures into successful
clone results.

## Test-Driven Implementation

The first production-facing change is a host integration regression contract
that requires all of the following:

- clone reservation resolves `current_root_paddr()` before publication;
- the suspended thread receives `set_linux_process_start` and process binding;
- `Aarch64CloneStart` contains the asserted `root_paddr` layout;
- clone assembly loads and writes `ttbr0_el1`; and
- the `dsb`/`tlbi`/`dsb`/`isb` order occurs before register restore and `eret`.

The contract must fail against the current implementation for the missing clone
root ownership. Only after observing that failure is the minimal production fix
implemented. The focused contract and the surrounding host suites must then
pass.

## AArch64 Acceptance Matrix

The fix is accepted only with fresh evidence from all applicable gates:

- host unit and integration suites;
- offline POSIX harness tests;
- AArch64 warning-as-error release build and link-layout check;
- all wired Verus proof harnesses;
- three independent fresh-disk runs of
  `conformance/interfaces/aio_cancel/5-1.c`;
- the complete `aio_cancel` API selection on a fresh disk;
- the complete AIO group on a fresh disk;
- clone, pthread, fork, and process-lifecycle canaries selected from the staged
  Open POSIX Test Suite;
- host coverage gates and the project quality-evidence report; and
- Coverity capture/analysis when the local Coverity tools are available.

Each runtime campaign retains its manifest identity, result NDJSON, and raw
serial log. Focused runs must terminate normally without watchdog timeout,
instruction-abort repetition, unresolved result, or positive residual task,
mapping, page, descriptor, handle, timer, IPC, or AIO resource deltas. Missing
external analysis tools are reported as unavailable rather than represented as
passing.

## Completion Criteria

The original upstream `aio_cancel/5-1.c` passes three fresh-disk runs, the full
`aio_cancel` and AIO selections complete without this hang, related clone/thread
canaries remain functional, all repository gates pass, diagnostic probes are
absent from tracked production files, and the normal Open POSIX Test Suite stage
is restored.
