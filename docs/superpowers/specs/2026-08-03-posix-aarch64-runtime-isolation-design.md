# POSIX AArch64 Runtime Isolation Design

## Goal

Remove cross-test Linux process-state leakage from serialized `run_elf` launches,
correct the Open POSIX Test Suite's obsolete asynchronous-I/O option guard, and
rerun all 1,598 selected AArch64 tests to obtain an honest semantic failure
inventory. The project-level acceptance criterion remains 1,598 genuine
`PTS_PASS` results with no skipped tests, result remapping, or test-specific
kernel behavior.

This is the first bounded delivery in the staged conformance-closure approach.
It does not claim POSIX compliance merely because tests can load after the
leak is removed.

## Evidence And Root Cause

The completed AArch64 run produced 4 passes, 1,591 failures, and 3 unsupported
results. Strict event parsing divides the failures into:

- 1,471 dynamic-loader failures reporting that `libc.so.6` could not create a
  shared-object descriptor because of `ENOMEM`;
- 119 dynamic-loader segment mapping failures;
- one real `WIFEXITED/1-3.c` failure; and
- three `aio_cancel` tests returning `PTS_UNSUPPORTED`.

Each early launch leaves six or more mappings in the global
`MemorySyscallState`. `LinuxMappingRecord` has no process or launch owner,
`sys_exit` does not release mappings, and the fixed 256 MiB Linux mapping
window is exhausted after the eighth test. Therefore 1,590 results describe a
single cascading loader failure rather than the behavior of their POSIX APIs.

The AIO cases contain an independent suite bug. They require
`_POSIX_ASYNCHRONOUS_IO == 200112L`, while the AArch64 glibc toolchain correctly
advertises the newer `200809L` value. A newer supported option version must not
be classified as unsupported.

## Scope

This delivery contains three changes:

1. Add a process-state reset operation for the currently serialized Linux ELF
   execution model.
2. Apply a provenance-tracked upstream test patch that accepts asynchronous-I/O
   option versions at least as new as POSIX.1-2001.
3. Perform canary and complete AArch64 runs and produce a strict failure
   inventory for the next conformance delivery.

Real multi-process ownership, `fork`, signal delivery, and `wait` semantics are
not simulated in this delivery. The existing `WIFEXITED/1-3.c` failure remains
until the process-foundation delivery implements those semantics. Any further
failures exposed by the complete rerun are grouped by root cause and handled in
subsequent bounded specifications.

## Process-State Boundary

Add `MemorySyscallState::reset_linux_process_state()` and a public
`reset_linux_process_state()` entry point. The operation is idempotent and
cleans only state owned by the serialized Linux ELF process:

- unmap every Linux mapping and free every mapping-owned page frame;
- clear all shared-memory attachment records associated with those mappings;
- free committed `brk` frames and restore a new `BrkState`;
- close every Linux descriptor above standard input/output/error through the
  normal descriptor and handle teardown rules, including duplicate-descriptor
  handling and FxFS cursor cleanup;
- restore the next mapping address and next descriptor allocators;
- reset per-process container, namespace, credential, and security state;
- cancel and remove transient AIO requests when request storage exists; and
- reset signal dispositions, pending state, and process timers.

The reset preserves:

- FxFS file contents and directory entries;
- named or explicitly persistent IPC objects, while removing this process's
  attachments and open references;
- the permanent root VMAR handle and kernel-global object tables;
- the monotonic synthetic PID allocator; and
- POSIX runner, manifest, and reporting state.

Descriptor cleanup must use the same ownership rules as `sys_close`: duplicated
descriptors release their shared handle only after the final reference closes.
The implementation must not clear the entire `MemorySyscallState`, because it
also contains permanent and non-Linux kernel state.

## Lifecycle Integration

The existing launch-ID checks remain the authority for whether cleanup may
run. Replace the signal/timer-only reset hook used by matched `run_elf`
transitions with the complete process reset.

The data flow is:

1. A matched launch admission resets stale transient process state before ELF
   loader preparation.
2. Loader mappings, descriptors, heap pages, timers, and other resources are
   created for that launch.
3. Normal exit, loader failure, launch-thread failure, and matched explicit
   clearing all converge on the reset operation.
4. Terminal cleanup runs before observer dispatch, so the runner's post-test
   resource snapshot measures leaks after teardown.
5. Repeated or stale completion IDs do not reset anything and cannot affect a
   newer launch.

Running cleanup at both matched admission and matched termination provides a
clean recovery boundary after an interrupted older launch while terminal
cleanup remains the normal ownership path. Idempotence prevents a successful
terminal cleanup followed by a new admission from double-freeing resources.

## Open POSIX Test Suite Patch

Add one reviewed patch to `third_party/posixtest/patches/series`. Across the
suite's asynchronous-I/O tests, change the obsolete guard from exact equality:

```c
#if _POSIX_ASYNCHRONOUS_IO != 200112L
```

to a minimum supported version check:

```c
#if _POSIX_ASYNCHRONOUS_IO < 200112L
```

Undefined option macros evaluate as zero in the preprocessor and remain
unsupported. `-1` remains unsupported. Values for POSIX.1-2001 and newer enter
the original test body without changing its assertions or return handling.

The existing source pipeline applies the patch, includes its bytes and name in
`patch_sha256`, derives the expected Git tree, and rejects an untracked or
modified checkout. No test result is rewritten, and no assertion is weakened.

## Error Handling

Reset must make forward progress even if an individual descriptor handle has
already disappeared. It drains a snapshot of descriptor numbers and applies
normal close semantics to each remaining entry. Mapping and `brk` page frames
are moved out of state before being freed, leaving an empty state if cleanup is
reentered.

If source patch validation, source-tree identity, build provenance, staging,
event parsing, or suite completion fails, the run is an infrastructure failure
and cannot contribute to the compliance result. Loader failures after the
cleanup change remain genuine failures and are retained in the report.

## Verification

Development follows test-driven changes:

1. Host logic tests first demonstrate that transient state is released,
   duplicated descriptor handles close once, allocator state resets, persistent
   state survives, and a second reset is harmless.
2. Lifecycle tests first demonstrate that all matched terminal paths reset once
   before observer dispatch and stale launch IDs do not reset a newer launch.
3. Source-pipeline tests first demonstrate the AIO patch is applied, changes the
   patch digest/tree identity, and leaves unsupported older option values
   unsupported.
4. Existing host, integration-contract, POSIX Python, AArch64 kernel build, and
   staging verification must remain green.
5. A private-disk QEMU canary runs enough sequential dynamically linked tests to
   cross the previous eight-test exhaustion point. Every completed test must
   return to its baseline transient resource counts.
6. A private-disk `posixtest all` run must contain 1,598 terminal attempts and a
   valid `suite_end`. Strict parsing must report zero loader failures caused by
   stale mappings and zero unsupported results caused solely by the obsolete
   AIO option-version guard.

The complete rerun's real failures are then grouped by API and root-cause
signature. The next delivery starts with the highest-impact semantic cluster,
with process/fork/signal/wait behavior taking precedence for the already known
`WIFEXITED/1-3.c` failure. This loop continues until the project-level gate is
1,598 genuine passes and all 185 APIs and 9 groups pass.

## Safety And Ownership

All QEMU validation uses a fresh disk below `target/posix/aarch64/`. The active
user-owned VM and `smros-fxfs.img` are not inspected, reused, or terminated.
Generated checkout and run artifacts remain outside commits; only production
code, tests, the reviewed patch series, and documentation are committed.
