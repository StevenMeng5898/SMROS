# AArch64 POSIX Results After Runtime Isolation

Date: 2026-08-03

## Scope

This is the first complete Open POSIX Test Suite AArch64 campaign after
per-launch Linux process cleanup and terminal-event framing were added. It is a
failure baseline, not a POSIX conformance claim. No status was remapped and no
test assertion was weakened.

The campaign used a private FxFS disk and did not access the user-owned VM or
the repository-root `smros-fxfs.img`.

## Provenance

| Field | Value |
| --- | --- |
| Architecture | `aarch64` |
| SMROS commit | `7982807ab47ebaee2688592b22a6b8433407da2f` |
| Open POSIX Test Suite revision | `85555325079ea362fa680bd2209c843cfe47e670` |
| Run ID | `3fdda15c34451cb433c46296cba6b5a5` |
| Build ID | `9dfb7c88c629b4bdf655f812427e9e052ca32d6c7c5ae6cb8199e071002d57ee` |
| Manifest SHA-256 | `be4a9c2dc98ff0d2bb0ac1774068867416b1de3fa12839e84b44b7098268d0e6` |
| Build-results SHA-256 | `9635ad7ff3f550822640edb73866662894b7f0dbf28fda80d1f50f24f5ee8323` |
| Patch SHA-256 | `3d6aea89fcaac1becb52cf168b7150825853f52a275c8edaf0dcb06d32086db6` |
| Results SHA-256 | `01adb7e8e5656ee1e48f68b70d956825e91bfa30b868f518d8676c8b56af151b` |
| Serial-log SHA-256 | `dd903513bd17ea9268adb49cfc203560cfa8e218d6cf6384147a23f0450f888d` |
| Report summary SHA-256 | `eed9b3db3fb057c026d0c05a8a9c052f22e875ac772e3980ee5144df4b5bae28` |
| QEMU boots / watchdog restarts | `172 / 171` |

Durable input and rendered output are under:

```text
target/posix/aarch64/smros-run-runtime-isolation-all-framed/
target/posix/aarch64/report-runtime-isolation-framed/
```

The report contains `events.ndjson`, `summary.json`, `junit.xml`, `groups.csv`,
`apis.csv`, `report.md`, and `index.html`.

## Integrity Gate

| Check | Result |
| --- | --- |
| Attempt records | `1598`, exactly matching selection |
| Terminal run records | `1`, with `complete=true` |
| Infrastructure error | none |
| Loader descriptor exhaustion | absent |
| Loader segment-map failure | absent |
| Measured resource snapshots | `1426` |
| Watchdog attempts without guest snapshot | `172` |
| Measured attempts with residual Linux mappings | `0` |

Three tests reported other positive residuals before lifecycle cleanup:

| Test | Residual |
| --- | --- |
| `conformance/interfaces/sched_setparam/10-1.c` | `linux_shared_memory=1` |
| `conformance/interfaces/sched_setparam/2-1.c` | `linux_fds=1` |
| `conformance/interfaces/sched_setparam/9-1.c` | `linux_shared_memory=1` |

## Coverage And Runtime Status

| Metric | Result | Percent |
| --- | ---: | ---: |
| Build coverage | `1598/1637` | 97.62% |
| Execution coverage | `1598/1598` | 100.00% |
| Runtime pass coverage | `674/1598` | 42.18% |
| Inventory program completion | `922/2054` | 44.89% |

`922/2054` includes completed non-runtime inventory dispositions. The actual
SMROS runtime pass count is `674/1598`; it must not be reported as 922 runtime
passes.

| Runtime status | Count |
| --- | ---: |
| pass | 674 |
| fail | 478 |
| timeout | 172 |
| unresolved | 239 |
| unsupported | 20 |
| untested | 15 |

Exit-code evidence is consistent with Open POSIX Test Suite status values:
674 attempts exited 0, 475 exited 1, 239 exited 2, 20 exited 4, 15 exited 5,
three exited 255, and 172 watchdog timeouts had no exit code.

## Group Results

| Group | Pass | Fail | Timeout | Unresolved | Unsupported | Untested |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| aio | 22 | 9 | 18 | 1 | 0 | 0 |
| base | 21 | 7 | 1 | 9 | 0 | 0 |
| memory | 19 | 25 | 0 | 44 | 0 | 5 |
| message-queues | 28 | 85 | 1 | 13 | 0 | 0 |
| scheduling | 26 | 11 | 0 | 6 | 20 | 6 |
| semaphores | 19 | 6 | 5 | 37 | 0 | 2 |
| signals | 284 | 279 | 26 | 58 | 0 | 2 |
| threads | 229 | 40 | 70 | 53 | 0 | 0 |
| time | 26 | 16 | 51 | 18 | 0 | 0 |

## Largest API Failure Clusters

| API | Non-pass | Breakdown |
| --- | ---: | --- |
| `sigaction` | 288 | 236 fail, 26 unresolved, 26 timeout |
| `shm_open` | 28 | 4 fail, 22 unresolved, 2 untested |
| `mq_timedsend` | 23 | 16 fail, 6 unresolved, 1 timeout |
| `mmap` | 23 | 14 fail, 8 unresolved, 1 untested |
| `timer_settime` | 19 | 19 timeout |
| `sched_setscheduler` | 18 | 4 fail, 2 unresolved, 10 unsupported, 2 untested |
| `mq_send` | 18 | 15 fail, 3 unresolved |
| `sched_setparam` | 16 | 2 fail, 4 unresolved, 8 unsupported, 2 untested |
| `mq_open` | 16 | 16 fail |
| `mq_timedreceive` | 14 | 14 fail |

Normalized diagnostics confirm shared missing semantics rather than loader
contamination. The leading clusters are 230 generic `Test FAILED` results, 172
watchdog timeouts, 52 `sigaction` handler-not-called failures, 29 missing
`shm_open` objects, 26 failed thread-directed signal deliveries, 26 restored
signal-handler failures, and 25 old-action mismatches.

## Unsupported Tests

All unsupported results are real exit-4 outcomes from scheduling optional
groups; none is a harness classification.

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

The report includes a separate quality-evidence record. Coverity is marked
`unavailable`, with null findings and null coverage, because Coverity tools are
not installed in the validation environment. This makes overall quality
evidence incomplete and does not change any POSIX result or denominator.

## Root-Cause Selection

`sys_rt_sigaction` currently persists only the SIGALRM handler, zero-fills old
actions for every other signal, and stores no flags or mask. `sys_kill` treats
nonzero signals as process termination requests, while `sys_tgkill` delegates
to that path. General registered-handler delivery, pending masks,
thread-directed delivery, `SA_SIGINFO`, and handler return restoration are not
modeled.

That implementation directly explains the highest-impact cluster:

- all 26 assertion-1 handler-delivery failures;
- 25 of 26 assertion-2 old-handler failures, with SIGALRM the lone pass;
- all assertion-3 handler-delivery failures;
- `SA_SIGINFO` and restored-handler failures;
- all 26 assertion-16 thread-directed signal unresolved results; and
- all 26 assertion-23 pending-signal timeouts.

The next implementation increment must therefore add process signal-action
state and AArch64 handler delivery/restoration under regression tests. Fork,
real pthread execution, named shared memory/semaphores, message queues, timers,
and optional scheduler groups remain independent later increments.
