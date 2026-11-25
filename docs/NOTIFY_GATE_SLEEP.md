# notify-gate-sleep late samples

`sectest notify-gate-sleep` is process A and process B. A arms a one-shot
`SIGEV_THREAD` timer, waits for it in `sigwait` so the kernel notify gate is
armed, then forks. The child measures a 50ms `nanosleep` (`0x02FAF080` ns)
and a 50ms `clock_nanosleep`. The parent keeps re-arming a 1ms deadline
(`0x000F4240` ns) and does not return to `sigwait`, so its own gate stays
armed, until 300ms have passed (`0x11E1A300` ns). The child exits 0 only when
both sleeps last at least 50000000 ns and finish in under 200ms
(`0x0BEBC200` ns). Exit 33 is `nanosleep-late`. Exit 35 is
`clock-nanosleep-late`. Exit 36 is `nanosleep-early`. Exit 37 is
`clock-nanosleep-early`. Other exits are setup or syscall failures.

The 200ms line is not a guess at scheduler slack. It sits under the parent's
300ms hold. `TimerNotifyGate::blocks_sleep` defers only the arming process.
If the child kept the parent's thread-group id, or the gate were global, the
child's 50ms sleep would stay blocked until that 300ms hold ended and the
case would fail late every time. A sleep that returns on its own deadline
stays well under 200ms. Raising the bound would also let a real cross-process
hold pass.

The probe also rejects a sleep shorter than the requested 50000000 ns.
`nanosleep-early` is exit 36 and `clock-nanosleep-early` is exit 37. Neither
is retried.

## What the 50 in-guest runs showed

The samples are monotonic time from arming the precision deadline until the
sleeper runs again. That is the interval the probe compares with 200ms, apart
from a few microseconds of syscall entry and exit. Every sample is in
`verification/logs/notify-gate-sleep.txt`. That file records the run that
booted `ed2f346` (`git_head=ed2f3464e116b72bd12e59f8d3c7904eaec65a6f`,
`utc_start=2026-11-23T09:00:55Z`). It was one QEMU process whose command
line requested `-smp 4`, then 50 sequential `sectest` commands, not 50 boots
and not a parallel run. The kernel still prints `Hardware-online CPUs: 1`.
Only CPU0 comes online. The `-smp 4` flag does not exercise a cross-CPU path.
Final verdicts were 50/50 pass and 0 early
wakes. One first-attempt nanosleep was 201478000 ns, so that run retried:
`attempts=2 late=1 values=201478000,50443072,50165680`. The second attempt
passed. No sample exceeded 300000000 ns. Nanosleep has 51 samples because the
retried run contributes the late first attempt and the passing second
attempt. Percentiles are linear between sorted indexes `(n-1)*p`.

| sleep | n | min | median | p95 | max |
| --- | ---: | ---: | ---: | ---: | ---: |
| nanosleep | 51 | 50128272 | 84091424 | 172085160 | 201478000 |
| clock_nanosleep | 50 | 50085520 | 50321360 | 95209047 | 130205072 |

All values are nanoseconds. The passing attempts stay under 200000000. The
one late sample is the case the retry rule exists for.

Nanosleep is late more often than `clock_nanosleep` because it runs while the
parent is still in the 1ms re-arm loop on the same boot. The parent and the
child share the per-CPU compare. Each parent `timer_settime` pulls that
compare forward by 1ms. The IRQ path rearms the earliest remaining precision
sleep, so the child is still due at 50ms, but it does not run until the
parent's timeslice is preempted. `clock_nanosleep` usually starts after that
first wakeup and lands on 50ms. QEMU TCG adds the long tail: CNTPCT keeps
moving while a vCPU is descheduled, and a compare that is already behind
CNTPCT does not raise a new GICv3 edge until it is rewritten. A stalled vCPU
therefore observes a sleep longer than the programmed 50000000 ns. On
this 50-run the longest nanosleep was 201478000 ns. That is past 200ms and
short of the 300ms hold, so the guest retried once and then passed.

The final-verdict failure rate on this host was 0/50. One of the 50 runs was
late on the first attempt. An earlier hermes boot of the same PR #32 commit
reported `nanosleep-late` once, and a later boot of that commit passed. Both
facts match a tail that reaches the 200ms line only when the host or the
parent's timer loop delays preemption.

## Retry

The guest prints `NOTIFY-GATE SUMMARY attempts=… late=… values=…`.
`scripts/summarize-hermes-campaign.py` copies that line verbatim, so a
non-zero `late` stays in the summary.

One late sample at or under 300ms may be retried once. A second late, or any
sample over 300ms, fails immediately. Early wakes fail immediately. The 200ms
bound is unchanged. A gate bug that holds the child past the parent's 300ms
window still fails on the first attempt.

## Log commits

- `2a7abfa` is the kernel and mutation-gate code: duration samples, the first
  retry, and the mutation script. It does not include later log republishes.
- `324061b` only republishes `notify-gate-sleep.txt` and
  `verus-mutation-summary.txt` headers onto that code.
- `6dff7d8` records host `ut`/`it`/`skt`, the five Verus harness totals, CBMC,
  and futex-stress. It does not change kernel or proof sources.
- `9db5d55` moves the mutated body to the real function body, requires both
  Boolean and `Option` sides, and adds the early-wake exits plus the retry
  predicate. `security_case_logic_shared.rs` gained the retry macro, and
  `verification/services` proves it as `verified_notify_gate_allow_retry`.
  `tests/host/src/lib.rs` gained the retry tests in the same commit.
- `1f1e31d` publishes a campaign summary and also edits
  `scripts/summarize-hermes-campaign.py`. It does not only add the script.
- `ed2f346` proves `fifo_capacity_valid` with the nonlinear multiplication
  lemma and makes the mutation gate reject one-sided integer and tuple specs.
  Its ring-index postcondition was only one point
  (`read_pos == 0 && offset == 1 && capacity == 2 ==> out == 1`). That point
  does not constrain wrap, a non-zero `read_pos`, or any other capacity.
  It edits `summarize-hermes-campaign.py` again so every count line is kept.
  The 50-run above booted this commit. The run was one QEMU process whose
  command line requested `-smp 4`, then 50 sequential commands in that same
  shell. It was not 50 boots and it was not parallel, which is why the clock
  time was about 41 seconds. Only CPU0 was hardware-online.

The host header `git_head` is `git describe --always --dirty` of the tree that
launched QEMU. The shell prints the same string as `smros_commit=`, baked in
by `build.rs`. A dirty tree gets a `-dirty` suffix. `build.rs` watches the
ref `git rev-parse --git-path` names, including a worktree whose `.git` is a
file, and it rewrites a stamp on every run so the next build recomputes
`--dirty` even when the branch ref did not move. The campaign summary fails
when those two strings differ or either ends in `-dirty`.

POSIX event `smros_commit` is the full SHA recorded by `posix-build`. The
summary prefers that SHA to be the same commit as `git_head`. An older stage
is accepted only when `git diff` from that SHA to `git_head` touches
`verification/logs/`, `docs/`, `scripts/posix/baseline.py`, or
`scripts/posix/cli.py`. `scripts/smros-vm-launcher.py`,
`scripts/run-hermes-campaign.sh`, and `scripts/measure-notify-gate-sleep.sh`
choose the QEMU command, CPU count, image path, and how the campaign or the
sleep measurement starts, so a lag that changes them is rejected. A re-run
can record `script_sha256 <path>=<blob sha256 at git_head>` in the preamble;
the summary accepts that lag only when every changed one of those three files
has exactly one `script_sha256` line and that line matches the blob. A second
line for the same path, including a wrong digest before or after the right
one, is rejected. A diff that includes `src/`, `build.rs`, a Verus
harness, or a gate script is rejected.

The 31 status changes between the `cf68ddc` baseline (1365 pass) and the
`8eb3bda` baseline (1357 pass) are not only mqueue. They are 8
`clock_settime`, 3 `mq_close`, 2 `aio_fsync`, 2 `sched_setparam`, 2
`sched_setscheduler`, 2 `shm_open`, 2 `sigqueue`, and one each of
`aio_error`, `fork`, `mlock`, `mlockall`, `mq_timedsend`,
`pthread_cond_init`, `pthread_detach`, `sched_getparam`,
`sched_getscheduler`, and `sem_open`. The `8eb3bda` ndjson has no literal
`EMFILE` token and was reported as `emfile=0`, but 40 mqueue attempts contain
the text `Too many open files`. The counter now includes that text and
`ENFILE`.

The `751b308` qemu-user reference (`verification/logs/posix-baseline-results.ndjson.gz`,
pass=1357) differs from the `500c948` reference (pass=1358) in seven attempts.
`git diff 500c9486773717d901acb11636874d6d9a3512f2 751b308560119ceb3de3571cfaf90787cd8de44d`
has no changes under `src/` or `scripts/posix/baseline.py`. These rows are
the qemu-user host reference. They are not the guest kernel futex path.
`pthread_cond_init/1-3.c` is tracked in `docs/KNOWN_ISSUES.md`.

| test | `500c948` | `751b308` campaign | class |
| --- | --- | --- | --- |
| `conformance/interfaces/aio_error/2-1.c` | pass | unresolved, exit 2, empty output, 199 ms | timing |
| `conformance/interfaces/aio_fsync/8-1.c` | crash, signal 1 | pass | timing |
| `conformance/interfaces/mq_send/5-1.c` | fail, "didn't appear to complete", 7203 ms | pass, 4205 ms | timing |
| `conformance/interfaces/mq_timedsend/5-1.c` | pass, 4199 ms | fail, "mq_send() didn't appear to complete", 7232 ms | timing |
| `conformance/interfaces/pthread_cond_init/1-3.c` | pass | fail, "behave differently across processes", about 200 ms, no timeout token | out of scope — host glibc + qemu-user |
| `conformance/interfaces/pthread_cond_init/4-1.c` | unresolved, child killed, EBADF, 6387 ms | timeout, signal 15, 33689 ms | address-space limit exhaustion, inferred |
| `conformance/interfaces/pthread_mutex_init/5-1.c` | unresolved, child killed, 6460 ms | timeout, signal 15, 31367 ms | address-space limit exhaustion, inferred |

`aio_error/2-1.c` returns unresolved when none of 128 `aio_write` calls is
still `EINPROGRESS`. Three isolated reruns were pass, pass, unresolved.
`mq_timedsend/5-1.c` passes only if the blocked send interrupts the parent
inside `sleep(3)` after `mq_receive`. The campaign fail printed that the send
did not complete; three isolated reruns all passed. `mq_send/5-1.c` is the
same sleep race and moved from fail to pass. `aio_fsync/8-1.c` moved from one
SIGHUP crash to pass; it was not repeated because it did not leave pass.
`pthread_cond_init/1-3.c` compares wakeup counts for a NULL condattr, a
default condattr, and a process-shared reference. It is not a sleep timer.
The staged campaign binary (`a1f87d30…`) does not print those counts. The
`-DVERBOSE=2` binary (`8e0fdcdf…`) is a different counting probe, not a timing
reproduction. Its 20 runs are evidence only that no side is 0 and that
pass/fail is an exact-equality check: run 04 is 200/199/169 and passes, and
run 15 is 200/199/197 and passes, while a shortfall of 1 to 7 fails. They do
not show why a count differed. The entry is closed as out of scope for host
glibc under qemu-user. The binding log is
`verification/logs/posix-baseline-status-rerun/pthread_cond_init-1-3-ld-debug/`.
`docs/KNOWN_ISSUES.md` records that this host baseline does not cover the
compat library's `pthread_cond_*`. `pthread_cond_init/4-1.c` and
`pthread_mutex_init/5-1.c` fork, lower `RLIMIT_AS`, and allocate until that
address-space limit is exhausted. That reading is inferred: the saved stdout
does not contain `ENOMEM`. Both archives are non-pass; the later run hit the
harness timeout instead of recording a killed child.

The three tests that left pass were repeated with `run_baseline` on
`host_shared/posixtest` (manifest `smros_commit`
`751b308560119ceb3de3571cfaf90787cd8de44d`), sysroot
`/usr/aarch64-linux-gnu`, and qemu `/usr/bin/qemu-aarch64-static`. That is
the campaign host stage's launch path, including the private `/dev/mqueue`
mount. The CLI `baseline --test` entry point was not used: it requires
`git rev-parse HEAD` to equal the staged `smros_commit`, and a later
logs-only commit does not. Raw ndjson and `SHA256SUMS` are in
`verification/logs/posix-baseline-status-rerun/`.

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -c '
from pathlib import Path
from scripts.posix.baseline import run_baseline
from scripts.posix.cli import posix_stage_path
run_baseline(
    posix_stage_path("aarch64"),
    Path("/usr/aarch64-linux-gnu"),
    Path("out.ndjson"),
    test_id="conformance/interfaces/aio_error/2-1.c",
    qemu="/usr/bin/qemu-aarch64-static",
)
'
```

Replace `test_id` with `conformance/interfaces/mq_timedsend/5-1.c` or
`conformance/interfaces/pthread_cond_init/1-3.c` for the other two.

- A later commit whose diff is only `verification/logs/` republishes the host
  Verus, CBMC, and futex logs for the documentation commit that contains this
  table. The hermes serial, with `utc`, `git_head`, and `git_dirty` on the
  first lines, is gzipped at `verification/logs/hermes-campaign-serial.log.gz`.
  `scripts/summarize-hermes-campaign.py` reads that gzip and rebuilds
  `verification/logs/campaign-summary.txt`, including a failing count line
  instead of dropping it. `serial_sha256` is the sha256 of the gzip bytes.
