# SMROS POSIX Userspace Conformance Design

## Objective

Make SMROS run real POSIX user processes and pass every complete test in the
Open POSIX Test Suite, including all optional API groups. Deliver AArch64
first, then x86_64, then RISC-V64 without duplicating POSIX subsystem
semantics.

The public Open POSIX Test Suite targets IEEE 1003.1-2001 System Interfaces.
It is not a formal certification program and does not establish certified
POSIX conformance by itself. In this project, "100%" means that every complete
test applicable to the pinned suite revision builds and passes under SMROS.

Tests that upstream deliberately ships as incomplete stubs are excluded from
the pass-rate denominator only through a reviewed, file-level allowlist. The
report must still show every excluded test and its reason. `UNSUPPORTED`,
`UNRESOLVED`, timeouts, crashes, and non-stub `UNTESTED` results prevent 100%
completion.

## Chosen Approach

SMROS will expose a Linux-compatible userspace ABI and use an existing AArch64
glibc runtime. Test programs will call POSIX APIs through glibc, which will use
the Linux syscall ABI to enter SMROS. This tests real user-visible behavior and
avoids creating and validating a new libc at the same time as the kernel
subsystems.

The rejected alternatives are:

- A new SMROS-native POSIX libc, because it adds a separate libc port before
  kernel behavior can be measured.
- Kernel-side API emulation, because direct Rust calls and modeled objects do
  not demonstrate the behavior of isolated user processes.

## Architecture

The conformance path is:

```text
Open POSIX Test Suite C test
  -> architecture glibc
  -> architecture Linux syscall ABI
  -> SMROS architecture trap adapter
  -> shared SMROS POSIX subsystem
  -> process, VFS, signal, thread, time, AIO, and IPC implementations
```

Architecture adapters decode syscall registers and numbers, validate trap
origin, and encode the result according to the Linux ABI. They do not own
POSIX semantics. A shared POSIX layer owns process and thread state, VFS
objects, signal state, scheduling policy, timers, asynchronous operations, and
IPC objects. The existing Zircon syscall route remains separate.

Each process owns:

- an architecture-backed user address space;
- credentials, process-group, and session state;
- a file-descriptor table referencing shared open-file descriptions;
- signal dispositions and process-pending signals;
- a thread group whose members have independent registers, masks, TLS, and
  thread-pending signals;
- timers, asynchronous I/O requests, and IPC references;
- parent/child relationships, wait state, and exit status.

User binaries must not depend on identity mappings. All syscall pointers use
checked copy-in/copy-out operations against the current process address space.
Syscalls return Linux-compatible values and negative errno results. Exiting a
process or thread releases every owned reference and wakes the relevant waiters.

## Suite Provenance And Build

Use a pinned GPLv2 Open POSIX Test Suite revision or immutable release archive.
The fetch step verifies a cryptographic checksum before extracting sources.
The repository stores the fetch metadata, license notice, SMROS patch series,
stub allowlist, and build scripts. Generated source trees and test binaries do
not need to be committed.

The host cross-build produces one executable per runnable test plus a manifest.
For every test, the manifest records:

- stable test ID and source path;
- POSIX API and feature group;
- assertion identifier or source description;
- executable path and checksum;
- timeout and resource limits;
- complete, definition-only, or audited-upstream-stub disposition;
- required runtime files and launch arguments.

The build metadata records the suite revision/archive checksum, patch-series
checksum, compiler, libc and binutils versions, target architecture, flags,
and SMROS commit. The same source revision and patch series produce the Linux
reference run and the SMROS run.

Suite patches may fix cross-compilation, paths, deterministic timeouts, or
demonstrable upstream defects. A patch must not weaken an assertion, suppress
a valid failure, or change a nonzero result into a pass. Each patch carries a
reason and upstream reference in the patch ledger.

## Shell Runner

SMROS does not build the tests internally. Host-built test binaries and their
manifest are packaged into the SMROS shared filesystem. The kernel-resident
diagnostic shell provides these commands:

```text
posixtest all
posixtest group <group>
posixtest api <api>
posixtest test <test-id>
posixtest status
```

The command launches every selected binary as a fresh EL0 process. It applies
the manifest timeout and quotas, captures bounded stdout/stderr, waits for the
process, validates cleanup, and proceeds to the next test. A test cannot share
unintended process state with its successor.

The runner emits versioned machine-readable events over serial. A run has a
stable ID and records its manifest checksum, filter, architecture, boot ID,
start/end times, and completion state. Interrupted runs can resume at the
first manifest entry without a terminal result when the manifest and SMROS
build IDs still match.

## Results And API Coverage

A host collector converts the serial event stream into:

- raw NDJSON events;
- a canonical JSON summary;
- JUnit XML for continuous integration;
- CSV tables for API and feature-group analysis;
- human-readable Markdown and HTML reports.

Each test result contains build/link status, launch status, PTS return status,
exit code or terminating signal, timeout status, duration, bounded output,
kernel diagnostics, and resource deltas. It also links the test to its POSIX
API, group, assertion source, relevant syscall surface, and Linux-reference
result.

The report presents these counts globally and for every API and feature group:

- discovered;
- complete;
- excluded upstream stub;
- build attempted and built;
- execution attempted and executed;
- passed and failed;
- unresolved, unsupported, untested, timed out, crashed, and flaky;
- leaked process, thread, mapping, fd, timer, AIO, and IPC resources.

The metrics are calculated as follows:

```text
build coverage = successfully built complete tests / buildable complete tests
execution coverage = executed complete tests / successfully built complete tests
pass coverage = passed complete tests / executed complete tests
program completion = passed complete tests / all complete tests
```

No excluded stub appears in these denominators. Definition-only compile tests
participate in program completion through their successful compilation rather
than execution. A report labels a run incomplete if its event stream lacks the
terminal run record.

## Delivery Milestones

### 1. Harness And Reference Baseline

Pin and fetch the suite, cross-build AArch64 artifacts, audit upstream stubs,
run an AArch64 Linux reference baseline, package the manifest and binaries, add
the `posixtest` shell command, collect events, and generate all report formats.
The initial SMROS report is expected to expose failures; it establishes the
honest baseline for later milestones.

### 2. Real Process Foundation

Replace identity-mapped execution with process-owned user page tables and
checked user-memory access. Complete process and thread lifecycle behavior,
including `fork`, `execve`, wait operations, exit, credentials, process groups,
sessions, environment, auxiliary vectors, and TLS setup.

### 3. VFS And File APIs

Make FxFS-backed state available through a process-visible VFS. Implement path
resolution, directories, metadata, permissions, links, shared open-file
descriptions, offsets, pipes, advisory locking, synchronization, and shared
file mappings with correct lifetime rules.

### 4. Signals

Implement process and thread pending sets, masks, dispositions, user signal
frames, alternate stacks, synchronous faults, queued delivery, child and timer
notifications, syscall interruption/restart behavior, and architecture-specific
signal return.

### 5. Threads And Scheduling

Implement shared-address-space thread groups, `clone` behavior, TLS, futex
wait/wake, robust lists, scheduling policies and priorities, cancellation
prerequisites, and process-shared pthread synchronization.

### 6. Realtime, AIO, And IPC

Complete clocks, timers, sleeps, POSIX message queues, named semaphores, shared
memory, memory locking, asynchronous I/O, notification, and cleanup behavior.

### 7. Conformance Closure

Resolve remaining API-specific failures, add functional and stress coverage,
repeat the full suite to identify nondeterminism, and enforce the AArch64
release gate.

### 8. x86_64 And RISC-V64

Add native x86_64 ELF loading, syscall/trap return, TLS, and signal frames, then
reach the same release gate. Repeat for RISC-V64. Neither port forks the shared
POSIX semantics.

## Failure Handling

Each test has wall-clock and resource limits. A timeout terminates the complete
test process tree before cleanup validation. Launch, manifest, checksum, or
collector errors are infrastructure failures and cannot be represented as
test failures or passes.

A kernel fault records the active test and fault context, marks the result as a
kernel crash, terminates the suite run, and leaves the event stream incomplete.
The next clean boot can resume the run. Continuing within a corrupted kernel
would make later results unreliable.

Any result that alternates between pass and another state across required
repetitions is `flaky` and blocks completion. The report retains every attempt
rather than selecting the successful one.

## Verification Strategy

Verification has four layers:

1. Host tests cover source verification, manifest generation, classification,
   event parsing, resume rules, denominator calculations, regression diffs,
   and each report renderer.
2. Kernel unit and contract tests cover process, memory, VFS, signal, futex,
   scheduler, timer, AIO, and IPC semantics without contributing to the POSIX
   pass rate.
3. Focused QEMU tests boot SMROS and execute selected APIs through real EL0
   traps, including negative and cleanup cases.
4. Full QEMU runs execute every complete suite test, repeat runs to detect
   flakes, and compare results with the matching Linux reference baseline.

Every defect found by a conformance test receives the smallest useful kernel
or host regression test before its fix. Direct Rust syscall tests remain
developer checks and never count as suite results.

## Release Gates

An architecture reaches the conformance target only when:

- every complete test and definition test builds successfully;
- every complete runnable test passes in all required repetitions;
- every optional feature group is implemented, so none reports unsupported;
- there are no unresolved or non-stub untested results;
- there are no timeouts, crashes, flaky tests, infrastructure errors, or
  resource leaks;
- every excluded test matches the reviewed stub allowlist and pinned source
  checksum;
- reports and their raw event input are archived with complete provenance.

A passing Open POSIX Test Suite report is evidence for this defined test scope,
not a claim of formal IEEE or The Open Group certification.
