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
`utc_start=2026-11-23T09:00:55Z`). It was one QEMU process with `-smp 4` and
50 sequential `sectest` commands, not 50 boots and not a parallel run.
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
  The 50-run above booted this commit. The run was one QEMU process
  (`-smp 4`) and then 50 sequential `sectest notify-gate-sleep` commands in
  that same shell. It was not 50 boots and it was not parallel, which is why
  the clock time was about 41 seconds.
The host header `git_head` is the commit that launched QEMU. The shell prints
`smros_commit=` from `SMROS_GIT_COMMIT`, which `build.rs` sets to the commit
compiled into that image. POSIX events use the same compiled commit, the one
`posix-build` embedded. Those three match after a clean rebuild. They differ
when an older image is booted from a later checkout. The campaign that
reported `smros_commit=6cdcfe9` under host `git_head=86a6927` booted an image
whose embedded stage had not been rebuilt.

- A later commit whose diff is only `verification/logs/` republishes the host
  Verus, CBMC, and futex logs for the documentation commit that contains this
  table. The hermes serial, with `utc`, `git_head`, and `git_dirty` on the
  first lines, is gzipped at `verification/logs/hermes-campaign-serial.log.gz`.
  `scripts/summarize-hermes-campaign.py` reads that gzip and rebuilds
  `verification/logs/campaign-summary.txt`, including a failing count line
  instead of dropping it. `serial_sha256` is the sha256 of the gzip bytes.
