# AArch64 POSIX Full-Campaign Results

## Scope

This record captures the final AArch64 SMROS run of the pinned Open POSIX Test
Suite manifest. The run used QEMU system emulation with one virtual CPU and a
1024 MiB guest:

```text
PYTHONDONTWRITEBYTECODE=1 POSIX_QEMU_SMP=1 \
  python3 -m scripts.posix.cli run-smros --qemu-memory 1024M --resume
```

The campaign resumed from a checkpoint after an interactive interruption. The
runner performed one controlled restart and retained the completed attempts.

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
| QEMU restarts | `1` |

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
- Host integration contracts passed: `171` passed, `0` failed.
- POSIX host-tool tests passed: `523` passed, `0` failed.
- Coverity was unavailable: `cov-build`, `cov-analyze`, and
  `cov-format-errors` were not installed, so no Coverity findings or coverage
  are claimed.
- Workspace usage after the campaign was approximately `7.1G`, below the 10G
  limit.

## Provenance

The manifest and staged POSIX runtime used by this campaign carry
`smros_commit=6cdee1062780b92c69c9910eb66c165fe37a4a6e` and
`manifest_sha256=d755ecc1d2c3ba90c3925ad8a06055b24236f761d8b3f1d340c618552e4db44b`.
The local branch is currently at `9bff87a4846b72871c7d67fa402a0d481d352aec`
with relevant uncommitted changes, so a future clean-stage run should be made
after those changes are committed. This distinction is preserved here rather
than silently relabeling the existing result.
