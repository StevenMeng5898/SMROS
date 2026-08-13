# AArch64 POSIX Fork Record-Lock Results

Campaign date: 2026-08-12

## Scope

This increment replaces the defective pinned Open POSIX
`conformance/interfaces/fork/11-1.c` stream-lock test with Linux Test
Project's maintained process-associated record-lock assertion. It implements
regular-file `fcntl(F_GETLK)`, `fcntl(F_SETLK)`, and `fcntl(F_SETLKW)` record
locks, including blocking, signal interruption, descriptor-close cleanup,
process-exit cleanup, fork ownership, and launch reset.

The corrected assertion now reaches a genuine guest terminal result instead
of hanging at `test_start`. This document records evidence for that affected
surface. It does not establish whole-suite POSIX compliance: the complete
`fork` API selection still contains four failures and four unresolved tests,
and no new all-API campaign was performed for this increment.

All QEMU attempts used separate generated FxFS disks under
`target/posix/aarch64/`. They did not use the repository-root
`smros-fxfs.img` or interact with the user-owned VM using that disk.

## Provenance

| Field | Value |
| --- | --- |
| Architecture | `aarch64` |
| Branch | `fix/posix-aio-fxfs-persistence-batching` |
| Tested SMROS implementation commit | `881880e86b6ac553eef8385136432a7cbda01302` |
| Open POSIX Test Suite revision | `85555325079ea362fa680bd2209c843cfe47e670` |
| Canonical patch-series SHA-256 | `e54668f51377ac9493d0bbbc607e4fae0d74ba60106dac11aeaa312b59c0f4f6` |
| Canonical manifest SHA-256 | `436c058778b363a43bf390b4c3a6a8c4138ca14817987a432bef5e7e77265173` |
| Manifest JSON file SHA-256 | `c676434ccddbae1f9104b5eea1d8470faf230ce516c4330457b1488d913c8591` |
| Manifest TSV file SHA-256 | `3f52247eb5efb451a822c0c16d865c3364c7dfe2e9666b59b5ccd7bab6ee0cf6` |
| Canonical build-results SHA-256 | `e51e42f9f81bb3deb1d3a84b75390b1a5951d3767fe2f4c79f338c5ba491d901` |
| Build-results file SHA-256 | `3f7456ec878ce8770139d06db8e92d25a04a38a41bb047a63ba6815e01b066fd` |
| Build ID | `d3f182e2a1accf971e4def50ae78aa054145ac677ca73ecfe15a30cc3d0e9cfd` |
| Corrected staged binary SHA-256 | `1b116d0e134a59a32f5c3edc58f152aff2ef8da9b5aae2b74ce3e83464012bc0` |
| Native executable SHA-256 | `b92faf78945a9ff58b80824ad63c0380c487659d92d8ecd229931f8f55471537` |
| Quality evidence SHA-256 | `15e7e5ba638c24981ccb888cc79bfb9902c77b85e7a5b924990e684ec05de6de` |

The verified stage contains 1,598 complete runnable tests. Its inventory also
retains 248 definition-only entries, 169 not-built shell tests, 94 reviewed
upstream stubs, 37 compile failures, and two link failures.

The replacement comes from Linux Test Project commit
`0b69550e055b5385822f001e2a27fedfbef31816`:

```text
https://raw.githubusercontent.com/linux-test-project/ltp/0b69550e055b5385822f001e2a27fedfbef31816/testcases/open_posix_testsuite/conformance/interfaces/fork/11-1.c
sha256: fcf9b794dd054586f65625ee6dd9a5daee61b98c1a43887de57e8c230a7d1626
```

The audited compatibility adaptation changes LTP's `test_main` to the pinned
suite's `main` entry point. The maintained record-lock checks and child status
propagation are unchanged.

## Native Check

The patched source was compiled as a native x86-64 Linux executable before
the SMROS runs. It printed:

```text
PASSED: Child locked file already locked by parent
```

The process exited with status `0`.

## Repository Gates

| Gate | Result |
| --- | --- |
| Host formatting | pass |
| Script checks | pass, 4 tests |
| Launcher checks | pass, 4 tests |
| Linker layout checks | pass, 8 tests |
| Host Rust unit tests | pass, 275 tests |
| Integration contracts | pass, 99 tests |
| POSIX tooling tests | pass, 474 tests |
| Stage build and verify-only rebuild | pass, 1,598 complete runnable tests |
| AArch64 warning-as-error production build | pass, no Rust warning; entry `0x40200000` |
| Git whitespace check | pass |

The five wired Verus suites all completed with zero errors:

| Verus suite | Verified | Errors |
| --- | ---: | ---: |
| 1 | 279 | 0 |
| 2 | 266 | 0 |
| 3 | 132 | 0 |
| 4 | 172 | 0 |
| 5 | 140 | 0 |

The Verus source-classification audit also passed.

## Corrected Test

Each attempt used a new 128 MiB private disk and a fresh QEMU boot. All three
attempts launched once, published a genuine `pts_status=pass`, exited `0`, and
finished without a timeout, restart, fatal marker, or positive resource delta.

| Attempt | Status | Duration | Boots / restarts | Resource evidence |
| --- | --- | ---: | ---: | --- |
| 1 | pass | 87 ms | 1 / 0 | all 14 measured deltas zero |
| 2 | pass | 91 ms | 1 / 0 | all 14 measured deltas zero |
| 3 | pass | 82 ms | 1 / 0 | all 14 measured deltas zero |

The measured fields were AIO requests, IPC objects, kernel handles, Linux file
descriptors, mappings, processes, shared memory, zombies, page-table pages,
private pages, process records, scheduler threads, shared pages, and timers.

## Complete Fork API

The fresh-disk `api=fork` selection completed all 19 selected tests in one
boot with no restart, timeout, launch error, fatal marker, or positive resource
delta. The corrected `fork/11-1.c` passed in 62 ms. Aggregate results were:

| Status | Count |
| --- | ---: |
| pass | 11 |
| fail | 4 |
| unresolved | 4 |
| unsupported | 0 |
| untested | 0 |
| timeout | 0 |

The eight non-passes remain truthful and are unrelated conformance gaps or
test-environment requirements:

| Test | Status | Diagnostic |
| --- | --- | --- |
| `conformance/interfaces/fork/1-1.c` | fail | semaphore concurrency: `The new process does not execute` |
| `conformance/interfaces/fork/17-1.c` | fail | scheduling policy was not inherited |
| `conformance/interfaces/fork/17-2.c` | fail | scheduling policy was not inherited |
| `conformance/interfaces/fork/8-1.c` | fail | child `tms` state was not reset during fork |
| `conformance/interfaces/fork/18-1.c` | unresolved | timer creation returned `EINVAL` |
| `conformance/interfaces/fork/19-1.c` | unresolved | message-queue information did not show the new message |
| `conformance/interfaces/fork/22-1.c` | unresolved | process CPU clock lookup returned `ESRCH` |
| `conformance/interfaces/fork/7-1.c` | unresolved | required `mess.cat` catalog was absent |

## Adjacent Canaries

Every canary used its own fresh disk and QEMU process. All seven terminated
without a watchdog, restart, launch error, fatal marker, or positive resource
delta.

| Test | Status | Duration | Diagnostic |
| --- | --- | ---: | --- |
| `conformance/interfaces/fork/1-1.c` | fail | 394 ms | semaphore concurrency assertion |
| `conformance/interfaces/fork/12-1.c` | pass | 52 ms | pending/blocked signal fork state |
| `conformance/behavior/WIFEXITED/1-3.c` | pass | 47 ms | wait/exit status |
| `conformance/interfaces/pthread_kill/1-1.c` | pass | 842 ms | thread signal delivery |
| `conformance/interfaces/shm_open/11-1.c` | pass | 80 ms | shared-memory descriptor/process behavior |
| `conformance/interfaces/sched_setparam/2-1.c` | unresolved | 736 ms | fork capacity returned `EAGAIN` |
| `conformance/interfaces/sched_setparam/2-2.c` | unresolved | 792 ms | fork capacity returned `EAGAIN` |

## Quality Evidence

Tarpaulin 0.37.0 measured `9,651/9,731` host Rust lines, or **99.18%**.
`make coverage-host` exited nonzero because the repository threshold is
100.00%, so this check is recorded as **failed**, not passed. The retained
HTML report has SHA-256
`9faaf20f557ecb21203ef06f9ad3d91c7c1502f9ad297b3d6816d9299893a7b9`.

Coverity was **unavailable** because `cov-build`, `cov-analyze`, and
`cov-format-errors` were not installed. Its finding count therefore remains
`null`; unavailable analysis is not represented as a pass.

The canonical quality JSON validates with the repository parser and records
one failed coverage check and one unavailable static-analysis check. Its
overall quality status is failed. These quality dispositions do not alter any
POSIX test status.

## Remaining Limitations

Close-on-exec record-lock cleanup awaits a real SMROS `execve` image
transition. The current `sys_execve` stub does not close `FD_CLOEXEC`
descriptors, so the implementation deliberately makes no close-on-exec
conformance claim.

The four failed and four unresolved `fork` assertions above remain open work.
More broadly, this focused campaign does not demonstrate that all 1,598 staged
tests pass or that SMROS is 100% POSIX compliant.
