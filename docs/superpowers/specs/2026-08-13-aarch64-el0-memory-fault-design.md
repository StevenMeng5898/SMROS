# AArch64 EL0 Memory-Fault Delivery Design

**Date:** 2026-08-13

**Status:** Approved design

## Goal

Make AArch64 synchronous memory faults raised by Linux-compatible EL0 processes
follow POSIX signal semantics instead of returning to the unchanged faulting
instruction. The immediate regression is Open POSIX Test Suite
`conformance/interfaces/mmap/6-2.c`, whose child reads a `PROT_NONE` mapping,
fault-loops in the exception vector, and leaves its parent blocked in `waitpid`.

This increment covers the complete AArch64 EL0 memory-fault boundary. It does
not add demand paging or redesign the x86-64 and RISC-V exception paths.

## Evidence and Root Cause

A fresh private-disk reproduction at implementation commit `f243f12` launched
`mmap/6-2.c` and timed out after 30 seconds with no guest output after its
`test_start` event. The saved evidence is under
`target/posix/aarch64/mmap-6-2-repro-f243f12/`.

The test forks. Its child creates a file-backed `PROT_NONE` mapping and reads
from it. SMROS correctly installs an AArch64 page descriptor without EL0 read,
write, or execute permission, so the access raises a lower-EL data abort. The
shared synchronous handler in `src/kernel_lowlevel/ARM64/boot.rs` recognizes
only AArch64 SVC. Every other synchronous exception stores `-ENOSYS` in saved
`x0`, restores the original exception state, and executes `eret`. Because
`ELR_EL1` still names the faulting load, the child repeats the same data abort
forever while the parent remains blocked in `waitpid`.

The existing Linux-compatible runtime already supplies signal dispositions,
AArch64 handler-frame construction, `sigreturn`, process termination by signal,
wait-status encoding, zombie policy, resource release, and parent-waiter wakeup.
The fix must route faults through that runtime rather than create another exit
lifecycle.

## Scope

The implementation will:

- separate lower-EL AArch64 synchronous exceptions from current-EL exceptions;
- classify lower-EL instruction and data abort syndromes in testable Rust code;
- translate EL0 memory faults into synchronous `SIGSEGV` or, when backing
  metadata proves it, `SIGBUS`;
- populate POSIX-compatible `siginfo_t` fields for `SA_SIGINFO` handlers;
- deliver caught signals from the saved fault frame;
- terminate through the existing Linux process lifecycle for fatal delivery;
- ensure no unhandled synchronous fault returns unchanged to EL0; and
- preserve the existing SVC syscall and syscall-return signal behavior.

The implementation will not:

- add lazy allocation, demand paging, copy-on-write fault resolution, or page
  cache faulting;
- infer `SIGBUS` when the process-memory metadata cannot prove the condition;
- recover current-EL/kernel faults as process signals; or
- change x86-64 or RISC-V exception handling in this increment.

## Architecture

### Exception-vector boundary

The AArch64 vector for synchronous exceptions from a lower EL in AArch64 mode
will branch to a dedicated lower-EL entry. That entry will save the same complete
general-purpose, SIMD, FPCR, and FPSR frame used by the syscall path. It will
capture `ESR_EL1`, `FAR_EL1`, and the saved return PC from `ELR_EL1`, then call a
Rust exception-facing function with the frame address and fault metadata.

AArch64 SVC remains on the existing syscall path. Lower-EL instruction-abort and
data-abort exception classes enter memory-fault classification. Other lower-EL
synchronous exceptions are rejected explicitly and cannot fall through to the
ordinary restore-and-`eret` path.

Current-EL synchronous vectors will use a fatal diagnostic path. The diagnostic
will identify the exception class, syndrome, fault address, and exception PC,
then halt. A kernel fault must never be mislabeled as a Linux process signal or
resumed into a fault loop.

### Pure syndrome classification

A small architecture-local Rust module will decode the AArch64 exception
syndrome without accessing global runtime state. It will expose typed results
for:

- AArch64 SVC;
- lower-EL instruction abort;
- lower-EL data abort;
- the data-abort write/not-read bit;
- translation, access-flag, and permission fault status codes; and
- unsupported or invalid synchronous exception classes.

The typed result will state the attempted access as read, write, or execute.
Syndrome decoding is kept independent of signal policy so all architectural
bit-field cases can be covered by host unit tests.

### Process-memory classification

For a decoded EL0 memory abort, the Linux process-memory layer will inspect the
mapping containing `FAR_EL1` and return one of these architecture-independent
outcomes:

- **Unmapped address:** `SIGSEGV` with `SEGV_MAPERR`.
- **Mapped address without the attempted permission:** `SIGSEGV` with
  `SEGV_ACCERR`.
- **Mapped file or shared-object page wholly beyond the recorded backing-object
  length:** `SIGBUS` with `BUS_ADRERR`.
- **Metadata unavailable or inconsistent:** deterministic `SIGSEGV`; never
  retry or resume the instruction.

The mapping record must retain enough backing-object length and offset metadata
to prove the `SIGBUS` case. A partially backed final page remains accessible;
only pages wholly beyond the object produce `SIGBUS`. Anonymous mappings and
ordinary permission failures never produce `SIGBUS`.

This classification does not resolve faults. SMROS continues to map all
currently supported pages eagerly.

## POSIX Signal Delivery

The exception-facing Rust function will construct a task-directed synchronous
signal record for the faulting Linux thread. Its `siginfo_t` payload will set:

- `si_signo` to `SIGSEGV` or `SIGBUS`;
- `si_code` to `SEGV_MAPERR`, `SEGV_ACCERR`, or `BUS_ADRERR`; and
- `si_addr` to `FAR_EL1`.

The record will enter the existing signal-delivery machinery immediately using
the saved exception frame and the faulting `ELR_EL1` as its return PC.

For a caught signal, the existing trampoline and signal-frame code installs the
handler. `sigreturn` restores the faulting PC. A handler that neither terminates
nor changes the invalid mapping will fault again, matching synchronous-fault
semantics.

For a default, ignored, blocked, or otherwise undeliverable fatal synchronous
fault, SMROS will terminate the process rather than resume it. Signal queue or
handler-frame construction failure has the same fail-closed behavior. Fatal
delivery reuses `terminate_linux_process_by_signal`,
`linux_process::terminate_by_signal`, and the existing no-EL0-return task finish
path. That lifecycle records the terminating signal in wait status, applies the
existing `SIGCHLD` and `SA_NOCLDWAIT` zombie rules, releases process resources,
wakes a matching parent waiter, and switches away without returning to the
faulting instruction.

The handler must expose a narrow exception-facing API instead of making the
assembly entry depend on internal signal tables. Signal disposition and process
lifecycle policy remain in Rust.

## Error Containment

No lower-EL abort path may write an errno into saved `x0` and return unchanged.
Every recognized memory abort must either install a handler frame or finish the
faulting process. Every unrecognized lower-EL synchronous exception must take a
deterministic diagnostic/fatal path.

Current-EL exceptions always remain kernel-fatal. The implementation must not
turn invalid kernel memory accesses into `SIGSEGV` for whichever process happens
to be current.

## Test Strategy

Implementation will follow test-driven development.

### Host unit tests

Tests will first fail against the missing behavior, then lock:

- AArch64 ESR exception-class decoding;
- instruction versus data aborts;
- read, write, and execute access extraction;
- translation, access-flag, and permission fault decoding;
- `SEGV_MAPERR` versus `SEGV_ACCERR` mapping;
- proven file-backed beyond-object `BUS_ADRERR` mapping;
- partial-final-page and anonymous-mapping behavior; and
- the deterministic `SIGSEGV` fallback for incomplete metadata.

### Integration contracts

Source-level integration contracts will require:

- separate current-EL and lower-EL synchronous vector entries;
- full saved-frame forwarding with `ESR_EL1`, `FAR_EL1`, and `ELR_EL1`;
- immediate routing into synchronous Linux signal delivery;
- reuse of the existing signal termination and no-EL0-return lifecycle; and
- absence of any unhandled lower-EL abort fallthrough to ordinary `eret`.

### AArch64 runtime tests

Every runtime test will use a fresh private disk under
`target/posix/aarch64/`. The repository-root `smros-fxfs.img` and any QEMU
process owned by the user are out of bounds.

The focused acceptance set is:

- `conformance/interfaces/mmap/6-1.c`;
- `conformance/interfaces/mmap/6-2.c`;
- `conformance/interfaces/mmap/6-3.c`;
- `conformance/interfaces/mmap/11-2.c`;
- `conformance/interfaces/mmap/11-3.c`; and
- fork, signal-termination wait status, and blocked-parent-wakeup canaries.

The `mmap/6-*` cases will run repeatedly to rule out another wait/fault race.
After focused acceptance, the complete `mmap` API selection will run.

Each result must show the expected manifest, build, patch, and implementation
provenance; `launch_status=launched`; no timeout or guest restart; no kernel
panic or repeated synchronous-fault marker; and non-positive terminal deltas for
Linux descriptors, mappings, processes, zombies, private/shared pages,
page-table pages, scheduler threads, handles, IPC objects, AIO requests, and
timers.

### Repository gates

The final verification will include:

- focused and complete host Rust tests;
- POSIX Python tooling tests relevant to building, running, and reporting;
- a warning-denied AArch64 release build; and
- applicable Verus verification checks.

## Acceptance Criteria

The work is complete when:

1. `mmap/6-2.c` terminates without watchdog intervention and reports pass after
   observing child termination by `SIGSEGV`.
2. `mmap/6-1.c` and `mmap/6-3.c` also report the required protection fault.
3. Installed synchronous fault handlers receive the correct signal, fault
   address, and usable saved context.
4. Proven beyond-object file-mapping accesses use `SIGBUS`, while ordinary
   permission and mapping faults use `SIGSEGV`.
5. The parent wait lifecycle completes and all measured resources return to
   their expected baseline.
6. Current-EL faults are fatal and no synchronous exception can resume into an
   unchanged fault loop.
7. All specified repository and AArch64 runtime verification gates pass.
