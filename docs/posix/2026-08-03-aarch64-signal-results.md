# AArch64 POSIX Signal Results

Date: 2026-08-03

## Scope

This is a complete filtered run of the Open POSIX Test Suite `signals` group
after adding AArch64 signal actions, masks, pending state, handler delivery,
signal return, alternate stacks, and queued signal information. It is not a
POSIX conformance claim. No status was remapped and no test assertion was
weakened.

The campaign used a fresh private FxFS disk under `target/posix/aarch64/`. It
did not access the user-owned VM or the repository-root `smros-fxfs.img`.

## Provenance

| Field | Value |
| --- | --- |
| Architecture | `aarch64` |
| SMROS commit | `94fed31215102473892b2d2c656a8a58f058dbf6` |
| Open POSIX Test Suite revision | `85555325079ea362fa680bd2209c843cfe47e670` |
| Run ID | `17333f097954e6c1f48596719a62d7f8` |
| Build ID | `fd11a4a9acc76f714d9282fea9e32884133f00746bb0f42644d4c0d242752f27` |
| Manifest SHA-256 | `d8d56c06fb4346d459bd2ad25e0fed24a2018aa098273dc8992ff452a82c973d` |
| Build-results SHA-256 | `9635ad7ff3f550822640edb73866662894b7f0dbf28fda80d1f50f24f5ee8323` |
| Patch SHA-256 | `3d6aea89fcaac1becb52cf168b7150825853f52a275c8edaf0dcb06d32086db6` |
| Results SHA-256 | `ec8e37d61bf736bc65c356651d3f64bcb786e050a6a9e07da4b76b4c922b0f0d` |
| Serial-log SHA-256 | `1643515ee593e20f1f45d19d148d76693f70d3d2c2bb016cb45bf513efce1c59` |
| QEMU boots / watchdog restarts | `1 / 0` |

Durable runtime evidence is under:

```text
target/posix/aarch64/smros-run-signals-group-94fed31/
```

## Integrity Gate

| Check | Result |
| --- | --- |
| Attempt records | `649`, exactly matching the filtered selection |
| Terminal run records | `1`, with `complete=true` |
| Infrastructure error | none |
| Loader descriptor exhaustion | absent |
| Loader segment-map failure | absent |
| Kernel panic | absent |
| Watchdog timeout | none |
| Attempts with residual Linux mappings | `0` |

One test, `conformance/interfaces/signal/1-1.c`, reported `processes=-1`.
This is cleanup beyond its initial snapshot, not a positive resource residual.
Every other resource delta was zero.

## Coverage And Runtime Status

| Metric | Result | Percent |
| --- | ---: | ---: |
| Execution coverage | `649/649` | 100.00% |
| Runtime pass coverage | `527/649` | 81.20% |
| API completion | `23/23` | 100.00% |
| APIs with every selected test passing | `12/23` | 52.17% |
| Group completion | `1/1` | 100.00% |
| Groups with every selected test passing | `0/1` | 0.00% |

| Runtime status | Count |
| --- | ---: |
| pass | 527 |
| fail | 79 |
| unresolved | 41 |
| untested | 2 |
| timeout | 0 |
| unsupported | 0 |
| launch error | 0 |

The twelve completely passing APIs are `raise`, `sigaddset`, `sigaltstack`,
`sigdelset`, `sigemptyset`, `sigfillset`, `sighold`, `sigignore`,
`sigismember`, `signal`, `sigprocmask`, and `sigrelse`.

## Baseline Comparison

| Status | Post-isolation baseline | Signal implementation | Change |
| --- | ---: | ---: | ---: |
| pass | 284 | 527 | +243 |
| fail | 279 | 79 | -200 |
| timeout | 26 | 0 | -26 |
| unresolved | 58 | 41 | -17 |
| untested | 2 | 2 | 0 |

The pass rate increased from 43.76% to 81.20%. The new run used the rebuilt
stage and kernel whose manifest records the signal implementation commit.

## Remaining API Clusters

| API | Non-pass | Breakdown |
| --- | ---: | --- |
| `sigaction` | 80 | 54 fail, 26 unresolved, 446 pass |
| `sigwait` | 8 | 6 fail, 2 unresolved |
| `sigqueue` | 6 | 4 fail, 2 untested, 7 pass |
| `sigwaitinfo` | 6 | 4 fail, 2 unresolved, 2 pass |
| `sigtimedwait` | 5 | 3 fail, 2 unresolved |
| `sigpause` | 4 | 4 unresolved, 1 pass |
| `sigpending` | 4 | 4 fail |
| `kill` | 3 | 2 fail, 1 unresolved, 2 pass |
| `sigset` | 3 | 1 fail, 2 unresolved, 7 pass |
| `killpg` | 2 | 1 fail, 1 unresolved, 5 pass |
| `sigsuspend` | 1 | 1 unresolved, 3 pass |

The dominant causes are shared process and thread foundations:

- 52 `sigaction/4-*` tests require a real forked child, default `SIGKILL`
  termination, and a wait status that reports signal termination;
- 26 `sigaction/16-*` tests require an executable pthread and
  thread-directed delivery rather than a synthetic clone identifier;
- `sigaction/10-1.c` and `sigaction/21-1.c` require child stop, continue,
  exit, `SIGCHLD`, and `SA_NOCLDWAIT` lifecycle semantics;
- `sigpause`, `sigsuspend`, `sigwait`, `sigwaitinfo`, and `sigtimedwait`
  require real blocking and wakeup, including per-thread masks and pending
  state; and
- the remaining `kill`, `killpg`, and `sigqueue` failures require real task
  existence, ownership, process-group, credential, and errno checks.

## Quality Evidence

The offline gate passed 92 host unit tests, 30 integration contracts, 462
POSIX-tool tests, 8 launcher tests, 4 linker-layout tests, formatting, shell
syntax, and the AArch64 release layout check before the stage was rebuilt.

Coverity is explicitly unavailable: `cov-build`, `cov-analyze`, and
`cov-format-errors` are not installed. No Coverity findings or coverage value
can therefore be claimed. This missing quality tool does not alter the POSIX
statuses or denominators above.

## Next Root Cause

The next increment is shared AArch64 task execution, not another
signal-specific special case. It must preserve per-thread EL0 return state,
create executable `CLONE_THREAD` children with TLS and child-TID semantics,
block and wake real threads through futexes, and route thread-directed signals
to live task IDs. Real forked address spaces and parent/child wait status then
build on the same task model.
