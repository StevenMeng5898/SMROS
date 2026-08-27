# AArch64 POSIX Full-Campaign Results

## Scope

This record captures the final AArch64 SMROS run of the pinned Open POSIX Test
Suite manifest. The run used QEMU system emulation with one virtual CPU and a
1024 MiB guest:

```text
PYTHONDONTWRITEBYTECODE=1 POSIX_QEMU_SMP=1 \
  python3 -m scripts.posix.cli run-smros --qemu-memory 1024M
```

The campaign ran from a clean results directory and completed in one QEMU boot.

## Runtime Result

| Metric | Result |
| --- | ---: |
| Selected tests | `1598` |
| Completed tests | `1598` |
| Pass | `1575` |
| Unsupported | `20` |
| Untested | `3` |
| Fail | `0` |
| Unresolved | `0` |
| Timeout | `0` |
| Crash | `0` |
| Launch errors | `0` |
| QEMU restarts | `0` |

The structured result file is `target/posix/aarch64/smros-run/results.ndjson`.
The run terminal record is complete and reports the same status counts.

## Coverage

| Coverage | Result |
| --- | ---: |
| Build coverage | `1598/1637` |
| Execution coverage | `1598/1598` |
| Runtime pass coverage | `1575/1598` |
| Program completion | `1823/2054` |
| Groups represented | `9` |
| APIs represented | `195` |

The detailed generated artifacts are in `target/posix/aarch64/report/`:
`summary.json`, `report.md`, `index.html`, `apis.csv`, `groups.csv`,
`junit.xml`, and `events.ndjson`.

The report was generated from the SMROS results only because no Linux-reference
result directory was present. It does not claim comparative Linux evidence.

## Group Coverage

| Group | Execution | Pass | Unsupported | Untested |
| --- | ---: | ---: | ---: | ---: |
| aio | `50/50` | `50/50` | `0` | `0` |
| base | `38/38` | `38/38` | `0` | `0` |
| memory | `93/93` | `92/93` | `0` | `1` |
| message-queues | `127/127` | `127/127` | `0` | `0` |
| scheduling | `69/69` | `47/69` | `20` | `2` |
| semaphores | `69/69` | `69/69` | `0` | `0` |
| signals | `649/649` | `649/649` | `0` | `0` |
| threads | `392/392` | `392/392` | `0` | `0` |
| time | `111/111` | `111/111` | `0` | `0` |

## Remaining Optional Cases

The 20 unsupported results are explicit POSIX Test Suite exit-4 outcomes, not
SMROS failures:

- 7 cases require process-contention-scope threads (`PTHREAD_SCOPE_PROCESS`).
- 13 cases require the optional sporadic-server policy (`SCHED_SPORADIC`).

The three untested results are:

- `conformance/interfaces/mmap/27-1.c`: the upstream test only reports that
  `MAP_FIXED` is defined.
- `conformance/interfaces/sched_setparam/26-1.c`: upstream requires execution
  as a regular user rather than root.
- `conformance/interfaces/sched_setscheduler/17-6.c`: upstream requires
  execution as a regular user rather than root.

These optional and privilege-gated cases are kept visible in the report and are
not counted as ordinary failures.

## Verification And Quality

- AArch64 release kernel build passed with `make build
  ARCH=aarch64-unknown-none QEMU_SMP=1 SMROS_LOGICAL_CPUS=1`.
- Host unit tests passed: `337` passed, `0` failed.
- Host integration contracts passed: `172` passed, `0` failed.
- POSIX host-tool tests passed: `524` passed, `0` failed.
- The repaired `sem_wait/13-1.c` case passed on five consecutive cold boots;
  the corrected `difftime/1-1.c` case passed on ten consecutive cold boots.
- The report records zero aggregate resource deltas and zero resource leaks for
  every measured resource category.
- Coverity was unavailable: `cov-build`, `cov-analyze`, and
  `cov-format-errors` were not installed, so no Coverity findings or coverage
  are claimed.
- Workspace usage after the campaign was approximately `7.1G`, below the 10G
  limit.

## Provenance

The manifest and staged POSIX runtime used by this campaign carry
`smros_commit=f10011c1cdd9fc51fc8fc07c712537a5682e4ef7`,
`manifest_sha256=2087164360d57724f778502d646938b393e43668ce507ee0ed2e79aa9227edca`,
`build_results_sha256=b01d5e1876d9eece7bd346140fe5d1565d6497b63c776b24ebafb406e83e8c94`,
and `patch_sha256=faf39d0193308e76e192a227222a62b37fb1167eff9f723d873ad5a90485ca93`.
The terminal run record carries build ID
`a3d7c8c592663243d4c2345b0dbd935acfa68542acb183cd0510a9274238798d`
and run ID `9935f568ca6efe4ca68b16a796b7ed35`.
