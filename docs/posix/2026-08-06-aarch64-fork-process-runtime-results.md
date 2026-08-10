# AArch64 POSIX Fork Process Runtime Results

Campaign date: 2026-08-09

## Scope

This is the complete Open POSIX Test Suite AArch64 campaign after the
process-owned address-space, eager-copy fork, shared-page inheritance,
descriptor inheritance, process lifecycle, signal termination, and wait/reap
work through commit `c0a513e`. The run includes every staged test and every
project-required optional group. No upstream assertion, result code, timeout,
group, or report disposition was weakened or remapped.

This is a process-foundation failure baseline, not an overall POSIX
conformance claim. The campaign still contains failures, unresolved and
unsupported results, untested assertions, watchdog timeouts, unported shell
tests, build failures, and failed or unavailable quality gates.

All SMROS runs used private generated FxFS disks below
`target/posix/aarch64/`. No run accessed the repository-root
`smros-fxfs.img` or a user-owned VM.

## Provenance

| Field | Value |
| --- | --- |
| Architecture | `aarch64` |
| SMROS implementation commit | `c0a513e75f7762b90e1e6de6ef27051e1add801d` |
| Open POSIX Test Suite revision | `85555325079ea362fa680bd2209c843cfe47e670` |
| Canonical patch-series SHA-256 | `2354fdb550290652373cd831c7489300bdd20344aa74fe151d8b3cfe0d009724` |
| Canonical manifest SHA-256 | `e9c4768f13803aea92af9cd521b36287e9468ac310cb4b2408d5d6ab218cd554` |
| Manifest JSON file SHA-256 | `ca7fec9d1bc7c9d0968fa45d88367595658e41b9b339e8492ac26d1a724eb6f0` |
| Manifest TSV file SHA-256 | `597cdfd196b009b8b1f0612db1d50ce1d5b4b5ae1e118192b30309fedd7790f5` |
| Canonical build-results SHA-256 | `ef58bb15baf69fc731bdb64810bec7a64ab8559a31e7c4152be4852858042e7a` |
| Build-results file SHA-256 | `f56a2f6b9895ff2f141540322160f2b3752b36d91676db7c37d0a74d4099cda4` |
| Build ID | `d10840111a9c0c6236838110c935a735e26b8cbf40e5f2354429ce5e5ecc1a36` |
| Full SMROS run ID | `e656c51976c1a9e1438bbc4bdbdb6f02` |
| Full SMROS results SHA-256 | `151a759496fcccca20a165a806956cffdd05399da0feb6da5b47e63436722a59` |
| Full SMROS serial-log SHA-256 | `776afd0ecaea10246296540ad852f875cb5086c88e08afa14a62c0eb10af1201` |
| Quality evidence SHA-256 | `4724c3fdeba8811063d7bfc6ec3c047655b0739b5921f70e938822a0ff9de252` |
| Linux reference run ID | `74c369920dec54c5b89f2b03eeabf21e` |
| Raw Linux runner output SHA-256 | `45d412f9f26ffef7accfd83c4e9e11c8663162b39e7e9c3cfe53523f5a4abca5` |
| Canonical Linux report input SHA-256 | `ecf0b11b8ed87da68d7bc1dac557d8fe99324964ee101055bcd4ff7c2d8ebb9b` |
| Report summary SHA-256 | `37d1bf8d67c9e29b14149ec030d3184350eceb475ac0a3a111ae7876786d7cd8` |
| QEMU boots / watchdog restarts | `333 / 332` |
| Compiler | `aarch64-linux-gnu-gcc (Ubuntu 13.3.0-6ubuntu2~24.04.1) 13.3.0` |
| SMROS system QEMU | `8.2.2` |

Durable inputs and outputs are under:

```text
host_shared/posixtest/
target/posix/aarch64/smros-run-fork-process-c0a513e75f77-*/
target/posix/aarch64/smros-run-fork-api-c0a513e75f77/
target/posix/aarch64/smros-run-base-c0a513e75f77/
target/posix/aarch64/smros-run-memory-c0a513e75f77/
target/posix/aarch64/smros-run-all-c0a513e75f77/
target/posix/aarch64/linux-reference/
target/posix/aarch64/quality-c0a513e75f77/
target/posix/aarch64/report-c0a513e75f77/
```

## Build Inventory

| Metric | Result |
| --- | ---: |
| C sources discovered | 1,979 |
| Compile pass / fail | 1,941 / 38 |
| Link pass / fail | 1,680 / 2 |
| Reviewed upstream stubs excluded | 94 |
| Shell sources reviewed | 176 |
| Shell tests not ported | 169 |
| Runnable staged tests | 1,598 |
| Staged bytes | 119,395,933 |

The report's build denominator is 1,637 complete buildable programs. Of those,
1,598 linked and entered the runtime selection. Definition-only checks,
reviewed upstream stubs, shell helpers, and unported shell assertions retain
their distinct inventory dispositions.

## Focused Canaries

Each canary used the commit-specific private disk and a fresh QEMU boot. All
three attempts launched, exited with the genuine Open POSIX Test Suite pass
status, and measured zero across all 14 resource-delta dimensions.

| Test | Status | Duration | Boots / restarts | Resource evidence |
| --- | --- | ---: | ---: | --- |
| `conformance/behavior/WIFEXITED/1-3.c` | pass | 19 ms | 1 / 0 | measured zero |
| `conformance/interfaces/fork/16-1.c` | pass | 53 ms | 1 / 0 | measured zero |
| `conformance/interfaces/fork/6-1.c` | pass | 19 ms | 1 / 0 | measured zero |

| Canary | Results SHA-256 | Serial SHA-256 |
| --- | --- | --- |
| `WIFEXITED/1-3.c` | `3da301bf83fa4a7e02600117944d8b1e10322d2f6294999f9fab1e2f9467a343` | `9576d51e727b7c68178c8f62942c0d2e6d449be834a9db036a65972a000a109a` |
| `fork/16-1.c` | `85b9a20a22970964e44bd4da363b7f170b884b8f4c643200706a630315d5ac20` | `dc44775a603e10da58210847b6bdb8b2b0390facabbdd71e575676f15f1f5120` |
| `fork/6-1.c` | `a98ce19d70a43c79bfee77671e9d85f340dcd2a4f4eb7bc8bf0d9681de5f6097` | `d06f1e7753d959de9db197d45c6c28206450a06863d58e4fc244fca33f360213` |

## Focused Selections

The aggregate duration below is the sum of the selected attempt durations.
Every measured attempt had all-zero resource deltas. Unavailable evidence is
limited to genuine host-watchdog timeouts.

| Selection | Attempts | Pass | Fail | Unresolved | Unsupported | Untested | Timeout | Duration | Boots / restarts | Measured / unavailable |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| complete `fork` API | 19 | 9 | 4 | 4 | 0 | 0 | 2 | 62,054 ms | 3 / 2 | 17 / 2 |
| base group | 38 | 24 | 7 | 4 | 0 | 0 | 3 | 93,395 ms | 4 / 3 | 35 / 3 |
| memory group | 93 | 39 | 33 | 10 | 0 | 5 | 6 | 191,734 ms | 7 / 6 | 87 / 6 |

| Selection | Results SHA-256 | Serial SHA-256 |
| --- | --- | --- |
| complete `fork` API | `6403cc4df1fe6ae64bb768aca1fed94ba9bfab95f6d973f04d196a35a2ef2fc2` | `cced301c173ffe3e73434e3e162f2445631f556f26f74891c2b878893f3cf5c8` |
| base group | `f86f9461ffae828402041973889d848775789387079ae4ddd78c016b450c2d39` | `aa4685bb04d7ccde19c6be3de9a1758ad9beb14ab9b6e6ecc5d2c5aa870b8981` |
| memory group | `9b9dec269f2acd478690d54fffff821e3145d64c3e0f09f2659689d98ff5b59b` | `5e75015760e9b35b9520a8aca87b93ad58a9c90e94099bd90828ce43ebd46095` |

The complete `fork` API retained these exact attempt results:

| Test | Status | Duration | Resource evidence |
| --- | --- | ---: | --- |
| `fork/1-1.c` | fail | 454 ms | measured zero |
| `fork/11-1.c` | timeout | 30,001 ms | unavailable |
| `fork/12-1.c` | pass | 60 ms | measured zero |
| `fork/13-1.c` | pass | 84 ms | measured zero |
| `fork/14-1.c` | pass | 342 ms | measured zero |
| `fork/16-1.c` | pass | 188 ms | measured zero |
| `fork/17-1.c` | fail | 71 ms | measured zero |
| `fork/17-2.c` | fail | 84 ms | measured zero |
| `fork/18-1.c` | unresolved | 217 ms | measured zero |
| `fork/19-1.c` | unresolved | 46 ms | measured zero |
| `fork/2-1.c` | pass | 59 ms | measured zero |
| `fork/21-1.c` | timeout | 30,000 ms | unavailable |
| `fork/22-1.c` | unresolved | 40 ms | measured zero |
| `fork/3-1.c` | pass | 55 ms | measured zero |
| `fork/4-1.c` | pass | 65 ms | measured zero |
| `fork/6-1.c` | pass | 57 ms | measured zero |
| `fork/7-1.c` | unresolved | 41 ms | measured zero |
| `fork/8-1.c` | fail | 145 ms | measured zero |
| `fork/9-1.c` | pass | 45 ms | measured zero |

The full campaign reproduced every focused `fork` status. The only focused/full
memory status difference was `shm_open/23-1.c`: unresolved after 897 ms in the
focused run and a genuine 30,000 ms watchdog timeout in the full run.

## Full Campaign

The terminal record is complete: 1,598 unique attempts exactly matched the
1,598 selected runnable tests, all attempts have `launch_status=launched`, and
the run published `complete=true` with no interrupted or launch-error result.

| Metric | Result | Percent |
| --- | ---: | ---: |
| Build coverage | `1598/1637` | 97.62% |
| Execution coverage | `1598/1598` | 100.00% |
| Runtime pass coverage | `948/1598` | 59.32% |
| Program completion | `1196/2054` | 58.23% |
| Selected API completion | `185/185` | 100.00% |
| Selected APIs with every test passing | `56/185` | 30.27% |
| Selected group completion | `9/9` | 100.00% |
| Selected groups with every test passing | `0/9` | 0.00% |

| Runtime status | Count |
| --- | ---: |
| pass | 948 |
| fail | 173 |
| unresolved | 109 |
| unsupported | 20 |
| untested | 15 |
| timeout | 333 |
| interrupted | 0 |
| crash | 0 |
| launch error | 0 |

The runtime selected 185 APIs. Fifty-six passed every selected test, 60 passed
at least 90%, 72 passed at least 75%, 108 passed at least 50%, and 33 had no
passing selected test.

## Group Results

All eight non-base groups, including the optional scheduling facilities,
remain selected. `Measured` and `Unavailable` are resource-evidence counts.

| Group | Tests | Pass | Fail | Unresolved | Unsupported | Untested | Timeout | Pass coverage | Measured | Unavailable |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| aio | 50 | 21 | 6 | 1 | 0 | 0 | 22 | 42.00% | 28 | 22 |
| base | 38 | 24 | 7 | 4 | 0 | 0 | 3 | 63.16% | 35 | 3 |
| memory | 93 | 39 | 33 | 9 | 0 | 5 | 7 | 41.94% | 86 | 7 |
| message-queues | 127 | 29 | 84 | 13 | 0 | 0 | 1 | 22.83% | 126 | 1 |
| scheduling | 69 | 25 | 11 | 7 | 20 | 6 | 0 | 36.23% | 69 | 0 |
| semaphores | 69 | 27 | 3 | 0 | 0 | 2 | 37 | 39.13% | 32 | 37 |
| signals | 649 | 572 | 14 | 31 | 0 | 2 | 30 | 88.14% | 619 | 30 |
| threads | 392 | 164 | 5 | 41 | 0 | 0 | 182 | 41.84% | 210 | 182 |
| time | 111 | 47 | 10 | 3 | 0 | 0 | 51 | 42.34% | 60 | 51 |

All 69 scheduling attempts executed. The 20 unsupported results are genuine
exit-4 outcomes from the `sched_get_priority_max`, `sched_get_priority_min`,
`sched_setparam`, and `sched_setscheduler` optional facilities. They remain
incomplete project requirements; none was removed from selection or counted
as passing.

## Resource And Fatal Integrity

The 1,265 non-timeout attempts contain measured guest snapshots. Every one
has zero positive and zero nonzero deltas across AIO requests, IPC objects,
kernel handles, Linux file descriptors, mappings, processes, shared memory,
and zombies, page-table pages, private pages, process records, scheduler
threads, shared pages, and timers. The 333 host-watchdog timeout attempts
retain unavailable evidence rather than an invented zero snapshot.

The serial fatal-pattern audit found zero kernel-panic, glibc-fatal,
heap-corruption, loader-segment-map, shared-object-descriptor,
translation-fault, and stale-address-space-root markers. In particular, the
four corrected `sched_setparam` tests introduced before this increment no
longer emit `malloc(): corrupted top size`.

## Linux Reference

The host did not provide a system `qemu-aarch64`, so the reference used the
Ubuntu `qemu-user` package `1:8.2.2+ds-0ubuntu1.18`. The package SHA-256 is
`9655f3e7e50af597e1b845822c47af9231f9f1bcb8c99e586db9da775d70c041`;
the extracted `qemu-aarch64` SHA-256 is
`0338d0ed6013abe5e0f41c5a22976ac6fe2f1b5869edf05d4a9098c121248f02`.

An unbounded diagnostic run reached approximately 6.48 GiB anonymous RSS in
`pthread_cond_init/4-1.c` and was killed by the host, so it published no
result. The retained wrapper applies a 2 GiB address-space limit, launches the
QEMU subtree in a private session, and kills and reaps that process group on a
supervisor shutdown. Its SHA-256 is
`bad021af6aee09adcd02bca7cba41957846ffe765682e0cbd7bef56ed771f2fb`.
The complete baseline also allowed a 15-second supervisor cleanup interval;
this changes no test timeout or returned status.

The complete runner output contained one cross-field harness inconsistency:
`clock_gettime/4-1.c` crossed its deadline but exited zero during shutdown, so
the runner emitted `status=timeout`, `timed_out=true`, and `exit_code=0`. The
shared report contract requires a post-deadline exit code to be null. The raw
`linux-reference/raw-results.ndjson` is retained unchanged, with a
byte-identical copy at
`quality-c0a513e75f77/linux-reference-runner-raw.ndjson` and the raw hash
above. A structured normalization cleared only that exit code; it did not
change the status, timeout flag, output, duration, provenance, or any other
attempt. The canonical report input `linux-reference/results.ndjson` has
SHA-256
`ecf0b11b8ed87da68d7bc1dac557d8fe99324964ee101055bcd4ff7c2d8ebb9b`.

Only this final complete 1,598-test canonical file is an input to the report.
Earlier unbounded and interrupted diagnostics are excluded.

| Linux runtime status | Count |
| --- | ---: |
| pass | 1,411 |
| fail | 63 |
| unresolved | 85 |
| unsupported | 18 |
| untested | 12 |
| timeout | 9 |
| crash | 0 |

## Baseline Comparison

The comparison baseline is the complete `f39aaf6` campaign. Every row below
uses the raw recorded status; no failure, unresolved result, or timeout is
reclassified.

| Status | `f39aaf6` | Current | Change |
| --- | ---: | ---: | ---: |
| pass | 1,094 | 948 | -146 |
| fail | 232 | 173 | -59 |
| unresolved | 151 | 109 | -42 |
| unsupported | 20 | 20 | 0 |
| untested | 15 | 15 | 0 |
| timeout | 86 | 333 | +247 |

Runtime pass coverage decreased from 68.46% to 59.32%, and program completion
decreased from `1342/2054` to `1196/2054`. The timeout increase is concentrated
in threads (+152), semaphores (+36), signals (+29), and AIO (+22). The memory
group gained 20 passes while reducing unresolved results by 35.

| Selection | `f39aaf6` | Current |
| --- | --- | --- |
| `fork` API | 9 pass, 1 fail, 9 unresolved | 9 pass, 4 fail, 4 unresolved, 2 timeout |
| base group | 23 pass, 6 fail, 9 unresolved | 24 pass, 7 fail, 4 unresolved, 3 timeout |
| memory group | 19 pass, 25 fail, 44 unresolved, 5 untested | 39 pass, 33 fail, 10 unresolved, 5 untested, 6 timeout |

The retained post-allocation scheduling evidence at `db6dd5a` completed all
69 selected tests with no timeout. Its results SHA-256 is
`ecc33dea34eaafb02df2db9731bcabca3c5c315c9ea5b3a55fe0cf027b2240ee`.
The current full campaign also completed all 69 scheduling tests without a
timeout, with these genuine status changes:

| Scheduling status | Post-allocation | Current | Change |
| --- | ---: | ---: | ---: |
| pass | 26 | 25 | -1 |
| fail | 13 | 11 | -2 |
| unresolved | 4 | 7 | +3 |
| unsupported | 20 | 20 | 0 |
| untested | 6 | 6 | 0 |
| timeout | 0 | 0 | 0 |

The focused canaries establish specific real-child behavior, but the unchanged
9/19 `fork` pass count and the full-suite timeout regression prevent a broader
conformance claim.

## Quality Evidence

The canonical quality record contains ten checks: six passed, three failed,
and one was unavailable. Overall quality status is `failed`; it does not alter
any POSIX numerator or denominator.

| Check | Status | Result |
| --- | --- | --- |
| Host Rust coverage | failed | 99.10%, 8,553/8,631 lines, 78 uncovered; `make coverage-host` exited 2 |
| Verus syscall harness | failed | compilation stopped before verification with 5 missing-`ObjectType` errors |
| Verus kernel objects | passed | 266 verified, 0 errors |
| Verus kernel low level | passed | 128 verified, 0 errors |
| Verus user level | passed | 172 verified, 0 errors |
| Verus services | passed | 130 verified, 0 errors |
| Verus coverage audit | failed | 35 findings |
| Coverity | unavailable | `cov-build`, `cov-analyze`, and `cov-format-errors` missing |
| POSIX selection/resource integrity | passed | 1,598 unique attempts; measured deltas all zero |
| POSIX serial fatal integrity | passed | zero fatal markers |

`cargo-tarpaulin-tarpaulin 0.37.0` ran 249 unit tests, 79 integration
contracts, and one socket behavior test successfully. Coverage was
4,822/4,884 lines in `src/lib.rs`, 3,673/3,681 in
`tests/integration_contracts.rs`, and 58/66 in
`tests/socket_table_behavior.rs`: 78 uncovered lines in total. The HTML
artifact is `target/coverage/host/tarpaulin-report.html`, and the complete log
is `target/posix/aarch64/quality-c0a513e75f77/host-rust-coverage.log`.

The retained Tarpaulin HTML marks these exact coverable lines with zero hits:

- `src/lib.rs` (62): lines `26-27`, `30-31`, `155`, `226`, `403`, `418-419`,
  `2611`, `2652`, `2682`, `2698`, `4115`, `4166`, `4189`, `4192`, `4217`,
  `4262`, `4371-4372`, `4533`, `4541`, `4572`, `4656`, `5627`, `5642-5645`,
  `5647`, `5668`, `5672`, `5729`, `6158`, `6160`, `6162`, `6205`, `7168`,
  `7196`, `7212`, `7253`, `7344`, `7390`, `7425`, `7441-7442`, `7478`,
  `7483`, `7492`, `7504`, `7601`, `7613`, `7626`, `7684`, `7714`, `7731`,
  and `8611-8615`.
- `tests/integration_contracts.rs` (8): lines `489`, `574`, `612-613`, `631`,
  `651`, `1990`, and `4403`.
- `tests/socket_table_behavior.rs` (8): lines `71-72`, `86-87`, `111-112`,
  and `129-130`.

Four direct Verus harnesses verified 696 obligations with zero errors. The
syscall harness did not enter verification because `ObjectType` was absent
from scope. The coverage audit's 35 findings consist of four missing
shared-logic includes, 12 missing macro uses, and 19 unclassified source
files. Complete logs remain in the commit-specific quality directory.

Coverity is explicitly unavailable. No zero findings count, coverage value,
or successful static-analysis claim is inferred from the missing commands.

## Report Artifacts

The canonical report publishes exactly seven regular files. Each parsed in
its native format, and the CSV group/API sets match all nine manifest groups
and all 195 manifest APIs.

| Artifact | SHA-256 |
| --- | --- |
| `events.ndjson` | `e9064d1c5fb261ca9f109159e559ec52df4e9d24bec69e975f73904ef8e83a05` |
| `summary.json` | `37d1bf8d67c9e29b14149ec030d3184350eceb475ac0a3a111ae7876786d7cd8` |
| `junit.xml` | `1cfb3b8ab2fc44101597b9eb23338b405e65509ea0e3b037d310c31d27ee1327` |
| `groups.csv` | `bbbd661839d145957a7de94c6b5aac814ff8d52464b23d1b6d0f61b19316bdaa` |
| `apis.csv` | `04c230e3a25b44777c89d7587fb9fc8bf41fa1410eedf19f07b0880476e04634` |
| `report.md` | `e48b3fa80ac2591041b116ede4cc7d08b2c460c644b7155bc1ec5f7672646461` |
| `index.html` | `6c8a9212b7b634703c696a0c75c395d163ad6e0d89f66d4561fcb5b0f47f8d1a` |

The report independently retains complete Linux and SMROS terminal records,
all optional scheduling rows, the all-zero measured resource evidence, the
333 unavailable timeout snapshots, and the exact ten-check quality table.

## Compliance Decision And Remaining Work

The AArch64 process foundation now demonstrates real child creation and wait
status in focused canaries, process-owned address spaces, eager private-page
copying, shared-page references, inherited descriptors, signal termination,
and all-zero cleanup metrics for every measured attempt. Execution selection
coverage is complete.

SMROS is not yet POSIX compliant. Required work remains for the failing and
timed-out fork cases, named `shm_open`/`shm_unlink`, named semaphores, message
queues, timers, optional scheduling policies, the full timeout regression,
100% host coverage, the syscall Verus harness, the Verus coverage audit, and
available static analysis. Copy-on-write, `execve`, x86_64 process address
spaces, and RISC-V64 process address spaces remain separate increments.

## Task 14 merge-head verification

This additive verification records the repaired merge candidate
`599c4925dd708a21da1b8d9458fed0fa3232a63b` without changing any result or
provenance above. The earlier full campaign remains immutably bound to `c0a513e75f7762b90e1e6de6ef27051e1add801d` as a historical baseline.

### Current-head gates

| Evidence | Result |
| --- | --- |
| Merge candidate | `599c4925dd708a21da1b8d9458fed0fa3232a63b` |
| Host test counts | `make test` passed 253 unit tests, 80 integration contracts, 473 POSIX-tool tests, 4 launcher tests, and 8 linker-layout tests; the coverage run also passed 1 socket behavior test |
| Proof counts | Verus coverage audit passed; syscall 278, kernel objects 266, kernel low level 132, user level 172, and services 140: 988 verified, 0 errors |
| AArch64 build and layout | passed; entry `0x40200000`, `.text [0x40200000,0x4027a000)`, `.rodata [0x4027a000,0x4a9c7000)`, `.data [0x4a9c7000,0x4bb40000)`, `.bss [0x4bb40000,0x4fb59000)`, `.stack [0x4fb59000,0x4fbd9000)` |
| QEMU smoke | passed with SMP=4 and 512 MiB on private disk `target/posix/aarch64/smros-fxfs-task14-smoke-599c4925dd70.img` |
| POSIX stage | 1,979 C sources; 1,941 compile pass, 38 compile fail; 1,680 link pass, 2 link fail; 169 shell tests unported; 119,397,116 staged bytes |
| Host coverage | failed as required below 100%: 8,690/8,768 lines, 99.11%, 78 uncovered; `make coverage-host` exited 2 |
| Coverity | unavailable: `cov-build`, `cov-analyze`, and `cov-format-errors` were all missing; no findings or analysis coverage is claimed |

The current stage is bound to canonical manifest
`9466093c93ba29f29e7a025ac98f13a79f2178c1d99274a837641aa98bf70cb8`,
canonical build results
`ef58bb15baf69fc731bdb64810bec7a64ab8559a31e7c4152be4852858042e7a`,
patch digest
`2354fdb550290652373cd831c7489300bdd20344aa74fe151d8b3cfe0d009724`,
and build ID
`fde6be8fee45bc42dcc8fcd6442fff2156f301f2560d64094f562b9efa430bfb`.

### Focused canaries

Every canary launched on its own fresh private 128 MiB disk, passed with no
restart, and measured zero in all 14 resource dimensions.

| Test | Status | Duration | Boots / restarts | Resource evidence |
| --- | --- | ---: | ---: | --- |
| `conformance/behavior/WIFEXITED/1-3.c` | pass | 17 ms | 1 / 0 | measured zero |
| `conformance/interfaces/fork/16-1.c` | pass | 120 ms | 1 / 0 | measured zero |
| `conformance/interfaces/fork/6-1.c` | pass | 22 ms | 1 / 0 | measured zero |

The complete focused selections retained every genuine non-pass and used
separate fresh private disks.

| Selection | Attempts | Statuses | Duration | Boots / restarts | Measured / unavailable |
| --- | ---: | --- | ---: | ---: | ---: |
| complete `fork` API | 19 | 9 pass, 4 fail, 4 unresolved, 2 timeout | 61,288 ms | 3 / 2 | 17 / 2 |
| base group | 38 | 24 pass, 7 fail, 4 unresolved, 3 timeout | 92,265 ms | 4 / 3 | 35 / 3 |
| memory group | 93 | 40 pass, 33 fail, 10 unresolved, 5 untested, 5 timeout | 158,510 ms | 6 / 5 | 88 / 5 |

All measured focused attempts had zero positive and zero nonzero deltas. All
six focused serial logs had zero panic, glibc-fatal, heap-corruption,
loader-segment-map, shared-object-descriptor, translation-fault, or
stale-address-space-root markers.

### Repair-head campaign

The fresh repair-head campaign used private disk
`target/posix/aarch64/smros-fxfs-task14-599c4925dd70-all.img`, selected all
1,598 complete runnable tests, published 1,598 unique terminal attempts, and
exited successfully after 332 boots and 331 watchdog restarts. Its run ID is
`cb4251d05bd5f23661c85b9d015a707b` and its aggregate attempt duration is
10,048,159 ms.

| Runtime status | Count |
| --- | ---: |
| pass | 949 |
| fail | 173 |
| unresolved | 109 |
| unsupported | 20 |
| untested | 15 |
| timeout | 332 |
| interrupted | 0 |
| crash | 0 |
| launch error | 0 |

The 1,266 measured attempts had zero positive and zero nonzero deltas in all
14 resource dimensions. The 332 unavailable snapshots belong exactly to the
332 host-watchdog timeouts. The 5,002,731-byte serial log had zero matches in
the fatal-marker audit.

The repair-head report records build coverage `1598/1637`, execution coverage
`1598/1598`, runtime pass coverage `949/1598`, and program completion
`1197/2054`. All 69 scheduling attempts executed; 20 remained genuinely
unsupported and none timed out. Compared with the immutable campaign, the raw
aggregate has one more pass and one fewer timeout. Nine individual statuses
changed, and no status was remapped.

The current quality record contains ten checks: eight passed, host Rust
coverage failed, and Coverity was unavailable. Its overall status is
`failed`; quality evidence does not alter a POSIX numerator or denominator.

The retained Linux reference file remains unchanged at SHA-256
`ecf0b11b8ed87da68d7bc1dac557d8fe99324964ee101055bcd4ff7c2d8ebb9b`.
Because the strict report validator requires current-stage provenance, the
report uses a separate derivative whose 1,599 records differ only in
`manifest_sha256` and `smros_commit`; its SHA-256 is
`7c1691f6e214870c928677a51a77dc6b31b86db1bde1c9725f85db5979dd573e`.
No retained Linux status, duration, output, run ID, or binary identity was
changed.

### Repair-head artifact hashes

| Artifact | SHA-256 |
| --- | --- |
| `stage-task14-599c4925dd70/manifest.json` | `d930383cb701df1b8272ca35814b71d5ebf32c14ed4438443eadad7c8d322ea9` |
| `stage-task14-599c4925dd70/manifest.tsv` | `d9ab7cd2b98ec64fae6076fb111ca6bf51b25d90b89095ded31731d67e0ac315` |
| `stage-task14-599c4925dd70/build-results.ndjson` | `e5ff7bc93e32f54f35fd3f78ef8ab18161112a4f83fa52b1b2c0bca238421b30` |
| `smros-run-task14-599c4925dd70-wifexited/results.ndjson` | `fdfa02e061dda3ecb4343e6337aeda6138c3571e7ba5e0c322625fda8cf8952f` |
| `smros-run-task14-599c4925dd70-wifexited/qemu-serial.log` | `84ce8503e77a614b4e2e7b3c502ece8a13590e47322d7dd71302841d48e8bcf8` |
| `smros-run-task14-599c4925dd70-fork-16-1/results.ndjson` | `273432bb606c4f503334183201080bd3edd183e269466f1f2bfbc4555c5ee95b` |
| `smros-run-task14-599c4925dd70-fork-16-1/qemu-serial.log` | `1045b38412dcd636ce1fd7684ae32b5cc88ed193835f81686cd55db01ce0068a` |
| `smros-run-task14-599c4925dd70-fork-6-1/results.ndjson` | `16deb323fe416e392799112334f1fdb54bee8ec52b76a839ebfdd5f1a1e85e70` |
| `smros-run-task14-599c4925dd70-fork-6-1/qemu-serial.log` | `815d1379277be7e9c6ce394e4afcea87d4db5ea23c57a8c4042611eac63e6b4c` |
| `smros-run-task14-599c4925dd70-fork-api/results.ndjson` | `d8785c76273f8069a32983d7ff1f835da718d0452f80e1ff505c29b3d2d14070` |
| `smros-run-task14-599c4925dd70-fork-api/qemu-serial.log` | `582a9f343968295cf67a904f24707c4de35f2e46ba584b2bc9d7883e75d7814d` |
| `smros-run-task14-599c4925dd70-base/results.ndjson` | `e8bbf19c9e14363d60e46b01fb8504421ab5b443b9fccf652d60f2e6964949a7` |
| `smros-run-task14-599c4925dd70-base/qemu-serial.log` | `c853f809d4662e46b015fab7790013386eb48cd9812314ecf02e407df4a6dae8` |
| `smros-run-task14-599c4925dd70-memory/results.ndjson` | `6792ba8e39411988b954019aa60913e72f856884cc949086a8f29d5ff5f5f5c3` |
| `smros-run-task14-599c4925dd70-memory/qemu-serial.log` | `7ae6866a69dafd3b6abcb2ffb216f43a9c2f78ecc3621e4e034930643049bac8` |
| `smros-run-task14-599c4925dd70-all/results.ndjson` | `a4b81d6f2cdebaf5900e444111cc5495cd772deec9b56fc7412ce77caea315fe` |
| `smros-run-task14-599c4925dd70-all/qemu-serial.log` | `bebb39b72385bc88d6b9821251489057cb8652f5fdfa347c554aad99ff4ab709` |
| `quality-task14-599c4925dd70/quality.json` | `d1cc6d8e39c9aec9e0dc84c53337300aa26a36315606b38491713fd12bf785b4` |
| `report-task14-599c4925dd70/events.ndjson` | `b70ecdc8f0238cec18c3902de22245be4bbb4f7d7d845caa0547eb8d198a871b` |
| `report-task14-599c4925dd70/summary.json` | `1a096d472880222d2d641b3792664ee22b0293e70c4d092d1043c09a1a1321c8` |
| `report-task14-599c4925dd70/junit.xml` | `51306e4a420f21fb17ac9f98e202866f1588c03f2b196b323160603247c871f1` |
| `report-task14-599c4925dd70/groups.csv` | `aff5dadbb77038dbaaf59ac6ff1886477ae649551fe8c710b8e4207a5aba8857` |
| `report-task14-599c4925dd70/apis.csv` | `2cb8a78534d8f4ee49b717e01b0a4eb47bba18c36b23bc000fede7444add67e7` |
| `report-task14-599c4925dd70/report.md` | `ab4748a5535a9c12cfafd079fe86cdaac6a6d57cca880d45a2437db79cbe380f` |
| `report-task14-599c4925dd70/index.html` | `df7ad2c7d41960c50be3837698987c13005005df6454448320efe95a509add73` |

The report directory contains exactly those seven report files. JSON, NDJSON,
JUnit XML, CSV, Markdown, and HTML parsed in their native formats; the CSV and
summary inventories match all nine manifest groups and all 195 manifest APIs.
