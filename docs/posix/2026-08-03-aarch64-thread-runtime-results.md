# AArch64 POSIX Thread Runtime And Resource Results

Campaign date: 2026-08-05

## Scope

This is the complete Open POSIX Test Suite AArch64 campaign after the Linux
thread, signal-wait, sleep, standard-descriptor, and SysV shared-memory
lifecycle work through commit `f39aaf6`. The run includes every staged test and
every project-required optional group. No result was remapped and no assertion
was weakened.

This is not an overall POSIX conformance claim. The campaign still contains
failures, unresolved results, unsupported optional facilities, untested
assertions, watchdog timeouts, unported shell tests, build failures, and two
user-space heap-corruption markers.

The campaign used private generated FxFS disks below `target/posix/aarch64/`.
It did not access the repository-root `smros-fxfs.img` or the user-owned VM.

## Provenance

| Field | Value |
| --- | --- |
| Architecture | `aarch64` |
| SMROS commit | `f39aaf6bf2c84637c80e6552aa72b489c979d44d` |
| Open POSIX Test Suite revision | `85555325079ea362fa680bd2209c843cfe47e670` |
| Run ID | `f569b2fc3716d9afd9f40d487a9ab4e6` |
| Build ID | `fa7e42897b76da1fff4e6b7f8a7d22016eaf0795b85a7a682bfab847e5a0c7dc` |
| Canonical manifest SHA-256 | `fac283801f43f037d2e49788832f242f8e2d6ffaae418f9cf92b104ec4f67c50` |
| Manifest file SHA-256 | `11795c11a9fa429c9337fe013827c4b162baba421e128720c19cd12735a7f5ce` |
| Canonical build-results SHA-256 | `9635ad7ff3f550822640edb73866662894b7f0dbf28fda80d1f50f24f5ee8323` |
| Build-results file SHA-256 | `1bef4af9ab355a21ce6976c3fa5b69fa376cbb20fce08505030ffa68ff7954a9` |
| Patch SHA-256 | `3d6aea89fcaac1becb52cf168b7150825853f52a275c8edaf0dcb06d32086db6` |
| Results SHA-256 | `32b3a96b82efc364a56af890b742dcdb81a0d30fa6477aec14ebe8b2ffe2b8b6` |
| Serial-log SHA-256 | `0e63bafc9618b77085173be525f5fee75394370425186f495f6f8cc92b22b528` |
| Report summary SHA-256 | `906d8534e85588a4fd932a0a9bde5e17f2fa3d37e39144a418e1988019ef6128` |
| Quality evidence SHA-256 | `f521050a96822d1fe98468514ff51244fd8c13f9bd500b44e92c7859444eb266` |
| QEMU boots / watchdog restarts | `86 / 85` |
| Compiler | `aarch64-linux-gnu-gcc 13.3.0` |
| QEMU | `8.2.2` |

Durable generated evidence is under:

```text
target/posix/aarch64/smros-run-all-f39aaf6/
target/posix/aarch64/smros-run-resource-f39aaf6-1/
target/posix/aarch64/smros-run-resource-f39aaf6-2/
target/posix/aarch64/smros-run-resource-f39aaf6-3/
target/posix/aarch64/smros-run-resource-f39aaf6-4/
target/posix/aarch64/smros-run-scheduling-f39aaf6/
target/posix/aarch64/report-f39aaf6/
target/posix/aarch64/quality-f39aaf6/
```

The focused `results.ndjson` files have these exact hashes:

| Evidence directory | Results SHA-256 |
| --- | --- |
| `target/posix/aarch64/smros-run-resource-f39aaf6-1/` | `20143281ad21e01206fe220128ab4ad70ea48d0d4be9fd75788bfac4c5107bf3` |
| `target/posix/aarch64/smros-run-resource-f39aaf6-2/` | `9ccc9c30a8dd960e95bf00a054edc28fd3db87561fdcc12cbdf52581378a5edc` |
| `target/posix/aarch64/smros-run-resource-f39aaf6-3/` | `df3bd7380cd1ad03e41d07ffb278acf2add35db1911a97aba6da4c1d510743e9` |
| `target/posix/aarch64/smros-run-resource-f39aaf6-4/` | `817ef83a450e62fb9f3807dc31e10d8be3cfdf40905674ee93821a93a9cee10a` |
| `target/posix/aarch64/smros-run-scheduling-f39aaf6/` | `6280c3ff2c1c4d4f63e6be39d7f280b8098d2a69ba7e48450b650129161d07e3` |

The report atomically publishes `events.ndjson`, `summary.json`, `junit.xml`,
`groups.csv`, `apis.csv`, `report.md`, and `index.html`.

## Build Inventory

| Metric | Result |
| --- | ---: |
| C sources discovered | 1,979 |
| Compile pass / fail | 1,941 / 38 |
| Link pass / fail | 1,680 / 2 |
| Shell sources reviewed | 176 |
| Shell tests not ported | 169 |
| Runnable staged tests | 1,598 |
| Staged bytes | 119,397,443 |

The host report counts 1,637 buildable complete programs. Of those, 1,598
linked and entered the runtime selection. Definition-only checks, reviewed
upstream stubs, shell helpers, and unported shell assertions retain their
separate dispositions.

## Coverage And Runtime Status

| Metric | Result | Percent |
| --- | ---: | ---: |
| Build coverage | `1598/1637` | 97.62% |
| Execution coverage | `1598/1598` | 100.00% |
| Runtime pass coverage | `1094/1598` | 68.46% |
| Program completion | `1342/2054` | 65.34% |
| Selected API completion | `185/185` | 100.00% |
| Selected APIs with every test passing | `84/185` | 45.41% |
| Selected group completion | `9/9` | 100.00% |
| Selected groups with every test passing | `0/9` | 0.00% |

API pass-rate distribution is also available per row in `apis.csv`. Of the
185 executed APIs, 84 passed every selected test, 86 passed at least 90%, 111
passed at least 75%, 146 passed at least 50%, and 10 had no passing selected
test. The two APIs between 90% and 100% were `pthread_sigmask` at `13/14`
(92.86%) and `pthread_exit` at `9/10` (90.00%).

| Runtime status | Count |
| --- | ---: |
| pass | 1,094 |
| fail | 232 |
| unresolved | 151 |
| unsupported | 20 |
| untested | 15 |
| timeout | 86 |
| crash | 0 |
| launch error | 0 |

All 1,598 attempts have `launch_status=launched`. The 1,512 non-timeout
attempts contain measured guest resource snapshots. The 86 host-watchdog
timeouts retain unavailable resource evidence instead of inventing a guest
snapshot.

## Group Results

All eight non-base suite groups are included; none was removed because an
option is unsupported. A group passes only when every selected runtime test in
that group passes.

| Group | Tests | Pass | Fail | Unresolved | Unsupported | Untested | Timeout | Pass coverage |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| aio | 50 | 37 | 10 | 3 | 0 | 0 | 0 | 74.00% |
| base | 38 | 23 | 6 | 9 | 0 | 0 | 0 | 60.53% |
| memory | 93 | 19 | 25 | 44 | 0 | 5 | 0 | 20.43% |
| message-queues | 127 | 28 | 85 | 13 | 0 | 0 | 1 | 22.05% |
| scheduling | 69 | 26 | 11 | 4 | 20 | 6 | 2 | 37.68% |
| semaphores | 69 | 23 | 6 | 37 | 0 | 2 | 1 | 33.33% |
| signals | 649 | 579 | 59 | 8 | 0 | 2 | 1 | 89.21% |
| threads | 392 | 325 | 21 | 16 | 0 | 0 | 30 | 82.91% |
| time | 111 | 34 | 9 | 17 | 0 | 0 | 51 | 30.63% |

## Baseline Comparison

The comparison baseline is commit `7982807`, the first complete campaign after
runtime isolation and terminal-event framing.

| Status | Post-isolation baseline | Current | Change |
| --- | ---: | ---: | ---: |
| pass | 674 | 1,094 | +420 |
| fail | 478 | 232 | -246 |
| timeout | 172 | 86 | -86 |
| unresolved | 239 | 151 | -88 |
| unsupported | 20 | 20 | 0 |
| untested | 15 | 15 | 0 |

Runtime pass coverage increased from 42.18% to 68.46%, a gain of 26.28
percentage points. Program completion increased from `922/2054` to
`1342/2054`. This does not convert the remaining non-pass results into
conformance.

## Resource Integrity

The resource-reclamation change was verified with four focused canaries, the
complete scheduling group, and the full campaign.

| Test | Status | Duration | Restart | Resource evidence |
| --- | --- | ---: | ---: | --- |
| `sched_setparam/10-1.c` | timeout | 30,001 ms | 0 | unavailable |
| `sched_setparam/2-1.c` | unresolved | 53 ms | 0 | measured zero |
| `sched_setparam/2-2.c` | unresolved | 54 ms | 0 | measured zero |
| `sched_setparam/9-1.c` | timeout | 30,001 ms | 0 | unavailable |

The scheduling group completed `69/69` with 26 pass, 11 fail, 4 unresolved,
20 unsupported, 6 untested, 2 timeout, 2 restarts, 67 measured-zero attempts,
and 2 unavailable timeout attempts. All 1,512 measured attempts in the full
campaign also had zero positive and zero nonzero deltas across file
descriptors, mappings, shared memory, processes, scheduler threads, kernel
handles, IPC objects, timers, and AIO requests. The 86 timeout attempts retain
unavailable resource evidence.

`Kernel panic`, `Fatal glibc error`, `failed to map segment`, and `cannot
create shared object descriptor` are absent from the serial log.

## Heap-Corruption Evidence

The serial log contains two `malloc(): corrupted top size` markers. They occur
in:

- `conformance/interfaces/sched_setparam/9-1.c`
- `conformance/interfaces/sched_setparam/10-1.c`

Both sources allocate `child_pid` with `malloc(nb_cpu)` and then write
`nb_cpu` values through an `int *`. The same undersized allocation exists in
`sched_setparam/2-1.c` and `sched_setparam/2-2.c` as `malloc(nb_child)`.
The suite already contains the correct pattern in `sched_yield/1-1.c`:
allocation is multiplied by `sizeof(int)`.

This establishes a four-file upstream test-porting defect as the immediate
heap-corruption cause. It is distinct from SMROS retained-resource behavior
and does not excuse the tests' timeout or unresolved statuses. The next suite
patch must correct the allocations without weakening any scheduling
assertion, then rerun these four canaries before deeper fork/scheduling work.

## Largest Remaining API Clusters

| API | Non-pass | Breakdown |
| --- | ---: | --- |
| `sigaction` | 54 | 54 fail, 472 pass |
| `shm_open` | 28 | 4 fail, 22 unresolved, 2 untested, 1 pass |
| `mmap` | 23 | 14 fail, 8 unresolved, 1 untested, 10 pass |
| `mq_timedsend` | 23 | 16 fail, 6 unresolved, 1 timeout, 2 pass |
| `timer_settime` | 19 | 19 timeout, 1 pass |
| `mq_send` | 18 | 15 fail, 3 unresolved |
| `sched_setscheduler` | 18 | 4 fail, 2 unresolved, 10 unsupported, 2 untested, 3 pass |
| `mq_open` | 16 | 16 fail, 12 pass |
| `sched_setparam` | 16 | 2 fail, 2 unresolved, 8 unsupported, 2 untested, 2 timeout, 5 pass |
| `mq_timedreceive` | 14 | 14 fail, 5 pass |
| `clock_settime` | 12 | 5 fail, 1 unresolved, 6 timeout, 3 pass |
| `fork` | 10 | 1 fail, 9 unresolved, 9 pass |
| `shm_unlink` | 10 | 3 fail, 7 unresolved |
| `mq_receive` | 9 | 9 fail, 1 pass |
| `sem_open` | 9 | 2 fail, 6 unresolved, 1 untested, 3 pass |
| `sem_unlink` | 9 | 2 fail, 7 unresolved, 1 pass |
| `timer_gettime` | 9 | 9 timeout, 1 pass |

The next SMROS semantic foundation remains real forked process state,
copy-on-write or equivalent address-space separation, shared-memory visibility
between processes, child lifecycle, and wait status. Timers, named shared
memory/semaphores, message queues, `mmap`, and optional scheduler facilities
remain independent later clusters.

## Unsupported Tests

Every unsupported result is a real exit-4 scheduling result. Because every
optional group is required by the project target, these remain incomplete
results.

| Test | Diagnostic |
| --- | --- |
| `sched_get_priority_max/1-3.c` | sporadic server not supported |
| `sched_get_priority_min/1-3.c` | sporadic server not supported |
| `sched_setparam/20-1.c` | process contention scope not supported |
| `sched_setparam/21-1.c` | process contention scope not supported |
| `sched_setparam/21-2.c` | process contention scope not supported |
| `sched_setparam/23-2.c` | sporadic server not supported |
| `sched_setparam/23-3.c` | sporadic server not supported |
| `sched_setparam/23-4.c` | sporadic server not supported |
| `sched_setparam/23-5.c` | sporadic server not supported |
| `sched_setparam/25-2.c` | sporadic server not supported |
| `sched_setscheduler/15-1.c` | process contention scope not supported |
| `sched_setscheduler/15-2.c` | process contention scope not supported |
| `sched_setscheduler/17-2.c` | sporadic server not supported |
| `sched_setscheduler/17-3.c` | sporadic server not supported |
| `sched_setscheduler/17-4.c` | sporadic server not supported |
| `sched_setscheduler/19-2.c` | sporadic server not supported |
| `sched_setscheduler/19-3.c` | sporadic server not supported |
| `sched_setscheduler/19-4.c` | sporadic server not supported |
| `sched_setscheduler/22-1.c` | process contention scope not supported |
| `sched_setscheduler/22-2.c` | process contention scope not supported |

## Quality Evidence

The report embeds 18 exact quality checks: 14 passed, 3 failed, and 1 was
unavailable. Overall quality status is therefore `failed` and does not alter
any POSIX numerator or denominator.

Passed checks include 161 host unit tests, 51 integration contracts, 468 POSIX
host-tool tests, stage verification, Rust formatting, 8 linker-layout tests,
the AArch64 release build, the latest-commit whitespace check, campaign
selection/resource integrity, and five direct Verus harnesses totaling 971
verified obligations with zero errors.

The Verus coverage audit failed with 23 findings: 3 missing shared-logic
includes, 8 missing macro uses, and 12 unclassified source files. The serial
heap-integrity check failed with the two allocation-corruption markers above.

`cargo-tarpaulin-tarpaulin 0.37.0` ran all 161 unit and 51 integration tests,
then reported 99.09% host Rust line coverage: 4,988 of 5,034 lines covered and
46 uncovered. The `make coverage-host` gate failed with exit 2 because the
result was below the required 100%. The HTML artifact is
`target/coverage/host/tarpaulin-report.html`; complete command output is in
`target/posix/aarch64/quality-f39aaf6/host-rust-coverage.log`. Coverity is
`unavailable` because `cov-build`, `cov-analyze`, and `cov-format-errors` are
not installed, so no Coverity findings count or analysis coverage is claimed.

## Compliance Decision

Execution selection coverage is complete and resource retention is clean for
all measured attempts, but the required pass, build, shell, optional-group,
heap-integrity, and quality gates are not complete. SMROS is therefore not yet
POSIX compliant, and this evidence must be used as the next failure baseline
rather than as certification.
