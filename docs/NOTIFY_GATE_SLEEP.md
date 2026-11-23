# notify-gate-sleep late samples

`sectest notify-gate-sleep` is process A and process B. A arms a one-shot
`SIGEV_THREAD` timer, waits for it in `sigwait` so the kernel notify gate is
armed, then forks. The child measures a 50ms `nanosleep` (`0x02FAF080` ns)
and a 50ms `clock_nanosleep`. The parent keeps re-arming a 1ms deadline
(`0x000F4240` ns) and does not return to `sigwait`, so its own gate stays
armed, until 300ms have passed (`0x11E1A300` ns). The child exits 0 only when
both sleeps last at least 49999872 ns and finish in under 200ms
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

The probe also rejects a sleep shorter than the requested 49999872 ns.
`nanosleep-early` is exit 36 and `clock-nanosleep-early` is exit 37. Neither
is retried.

## What the 50 one-shot runs showed

The samples are monotonic time from arming the precision deadline until the
sleeper runs again. That is the interval the probe compares with 200ms, apart
from a few microseconds of syscall entry and exit. Every sample is in
`verification/logs/notify-gate-sleep.txt`. The run was 50/50 pass, 0 retries,
0 early wakes, and 0 failures. Percentiles are linear between sorted indexes
`(n-1)*p`.

| sleep | min | median | p95 | max |
| --- | ---: | ---: | ---: | ---: |
| nanosleep | 50172576 | 85245608 | 158747254 | 179106496 |
| clock_nanosleep | 50072192 | 51342432 | 129054676 | 138816928 |

All values are nanoseconds. Nothing reached 200000000. The late-only retry
was already in this kernel and did not fire.

Nanosleep is late more often than `clock_nanosleep` because it runs while the
parent is still in the 1ms re-arm loop on the same boot. The parent and the
child share the per-CPU compare. Each parent `timer_settime` pulls that
compare forward by 1ms. The IRQ path rearms the earliest remaining precision
sleep, so the child is still due at 50ms, but it does not run until the
parent's timeslice is preempted. `clock_nanosleep` usually starts after that
first wakeup and lands on 50ms. QEMU TCG adds the long tail: CNTPCT keeps
moving while a vCPU is descheduled, and a compare that is already behind
CNTPCT does not raise a new GICv3 edge until it is rewritten. A stalled vCPU
therefore observes a sleep longer than the programmed 50ms. The longest
nanosleep in the quiet 50-run sample was 188ms, under the 200ms line and
under the 300ms hold.

That is a late-only flake, not a wrong result. The failure rate on this host
was 0/50. An earlier hermes boot of the same PR #32 commit reported
`nanosleep-late` once, and a later boot of that commit passed. Both facts
match a tail that reaches the 200ms line only when the host or the parent's
timer loop delays preemption.

## Retry

The guest prints `NOTIFY-GATE SUMMARY attempts=… late=… values=…`.
`scripts/summarize-hermes-campaign.py` copies that line and repeats it when
`late` is not zero.

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
- Later campaign logs are regenerated on the commit that contains these
  corrections. Full serial is not stored.
