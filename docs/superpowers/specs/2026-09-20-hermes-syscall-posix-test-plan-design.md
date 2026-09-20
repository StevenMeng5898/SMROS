# Hermes Syscall and POSIX API Test Plan

## Goal

Use the native SMROS Hermes agent as the control plane for syscall ABI and
POSIX API verification. Hermes already owns safe guest execution (`hermes exec`,
`hermes random`) and named host jobs (`ut`, `it`, `st`). This plan adds four
first-class Hermes skills — **ut**, **it**, **st**, and **fuzzing** — and aims
them at Linux/Zircon syscall dispatch plus IEEE 1003.1-2001 System Interfaces.

This is a test strategy, not POSIX certification. Pass rates are evidence of
the selected Open POSIX Test Suite campaign on SMROS, not a claim that the
kernel is a complete POSIX implementation.

## Non-Goals

- Do not treat guest `posixtest all` as the official campaign. That path does
  not enforce `timeout_ms` and can hang.
- Do not start a new full ~1600-test `posix-run` unless a focused canary
  regresses below the quality bar.
- Do not give Hermes arbitrary shell, `posixtest all`, `kill`, `reboot`, `rm`,
  or `run`.
- Do not wire RISC-V secondary harts only to make `sched_yield/1-1` pass.
- Do not introduce coverage-guided syzkaller, guest line coverage, or x86_64
  POSIX in this milestone.
- Do not block the plan on repairing the currently uncompilable `tests/host`
  crate (~4092 errors). That restore is a parallel track.

## Control Plane

Hermes is the orchestrator. Gemma text is never executed. Every guest action
goes through `hermes_shell_logic_shared::classify` before dispatch.

```text
operator / Gemma prompt
        |
        v
Hermes skills (ut, it, st, fuzzing, smros-kernel, smros-ops)
        |
        +--> guest allowlist: testsc, fuzzsc, hermes test, posixtest test <canary>
        |
        +--> host launcher jobs: ut, it, st [, posix-tool]
        |
        +--> scheduled/manual host: make posix-run (official POSIX ST)
```

Existing commands stay compatible:

- `hermes test` — native Hermes agent smoke (config, skills, memory, Gemma, /svc, UI)
- `hermes exec <safe-command>` — one allowlisted guest command
- `hermes random seed=<n> iterations=<n>` — catalog of safe guest ops, including `fuzzsc`
- `hermes test-all seed=<n> iterations=<n>` — native test once, then per iteration one
  random guest op plus host jobs `ut`, `it`, `st`
- `hermes skills` — list installed skills, including the four new ones

New mode (implementation milestone):

```text
hermes test-all mode=syscall seed=<n> iterations=<n>
```

`mode=syscall` does not replace the existing test-all loop. It selects the
syscall/POSIX catalog, requires the four skills, and records a combined report
under `/data/hermes/tests/syscall-*`. Default `hermes test-all` remains the
generic ops campaign.

## Skills

Install four native skills under `/data/hermes/skills/`. Bodies live in
`src/user_level/services/hermes_agent.rs` next to the existing one-line
`smros-kernel` / `smros-ops` skills. Keyword matching must hit syscall and
POSIX prompts.

### Skill `ut` — Unit

**Purpose.** Deterministic, no-QEMU checks of syscall policy and POSIX source
contracts.

**Run.**

```bash
make ut                          # host crate, when it compiles
make posix-tool-test             # current working syscall/POSIX unit gate
```

Until `tests/host` compiles, `make posix-tool-test` is the required UT gate
for this plan. `make ut` is recorded as `blocked` rather than skipped silently.

**What it covers.**

- Pure helpers in `src/syscall/*_logic_shared.rs`: address ranges, fcntl/record
  locks, clock_nanosleep flags, hybrid sleep thresholds, sched policy/priority,
  futex wait classification, mqueue attribute checks.
- Python StagingTests in `scripts/posix/tests/test_build.py` that lock
  production source contracts (compat poll, sleep sigtimedwait, `yield_now`
  Ready-before-schedule, mq_open 16-1 patch, dual-arch guards).
- Hermes policy helpers in `hermes_shell_logic_shared.rs`.

**Pass.** All selected unit tests pass. No QEMU. No network. Runtime budget:
minutes, not hours.

**Fail example that must stay RED until fixed.** A `yield_now()` body that
calls `schedule()` while the current thread is still `Running`.

### Skill `it` — Integration

**Purpose.** Cross-module contracts and POSIX tooling identity, still mostly
offline.

**Run.**

```bash
make it                          # tests/host/tests, when it compiles
make posix-tool-test             # always, even if host IT is blocked
make launcher-test               # SMROS_TEST_RUN job enum
```

**What it covers.**

- Linux vs Zircon syscall-number routing (`syscall_num < 1000` vs `+1000`).
- Compat `.so` dual-GLIBC contracts (`GLIBC_2.17` aliases and `GLIBC_2.27`
  defaults). No architecture-hardcoded paths.
- Manifest/stage identity: `binary_sha256`, `manifest_sha256`, TSV checksum.
- Hermes test-all wiring: jobs remain the closed set `{ut,it,st}` unless a
  protocol bump adds `posix-tool`.
- qemu-user reference canaries via `make posix-baseline --api getpid` only
  when explicitly requested; not part of default `hermes test-all`.

**Pass.** Contracts agree across files. StagingTests green. Launcher rejects
unknown jobs and never uses `shell=True`.

### Skill `st` — System

**Purpose.** Booted guest behavior: syscall helpers, bounded fuzz, and
host-controlled POSIX canaries.

**Fast ST (Hermes / `make st`).**

```bash
make st                          # existing smoke
# serial input must include:
#   testsc
#   fuzzsc seed=1 iterations=1
#   hermes random seed=1 iterations=1
#   hermes exec reboot           # must be denied
```

Required serial milestones already documented in `docs/TESTING.md`, plus:

```text
[OK] futex tests completed
[OK] Linux signal, IPC, misc, and net tests completed
[OK] Linux file, dir, fd, poll, and stat tests completed
fuzzsc: ... completed_iterations=1
Hermes denied forbidden command: reboot
```

**POSIX ST (official, host-controlled, not Hermes guest `all`).**

```bash
# focused canary — default for this plan
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli run-smros \
  --api getpid --api sched_yield --api mq_send --api mq_timedsend \
  --api mq_open --api mmap --api pthread_cond_broadcast \
  --api pthread_create --qemu-memory 1024M

# full campaign — only if focused regresses, or as a scheduled job
make posix-run
make posix-run ARCH=riscv64gc-unknown-none-elf
```

Rules:

- Use isolated disks `target/posix/<arch>/smros-fxfs.img`. Never share
  `smros-fxfs.img` across architectures.
- Host writes `posixtest test {id}` and requires `selected_count == 1`.
- Dedicated output directories so campaign-keep / `results.ndjson` stay intact.
- Guest `posixtest all` is diagnostic only and is **forbidden** to Hermes.

**Quality bar (both AArch64 and RISC-V64).**

- pass rate ≥ 99% of selected tests
- timeout = 0
- unresolved = 0
- untested = 0
- fail count < 2 (0 or 1)
- AArch64 `pthread_spin_trylock/1-1.c` on SMP=4 is the currently allowed
  single fail. Do not paper over it by dropping SMP or patching the test.

x86_64 POSIX remains out of scope. `make st ARCH=x86_64-unknown-none` stays a
boot smoke only.

### Skill `fuzzing` — Syscall Fuzzer

**Purpose.** Broad dispatcher coverage with structured, replayable mutation.

**Guest entry.**

```text
fuzzsc seed=<n> iterations=<n> time=<seconds>
```

Implementation: `src/syscall/fuzz.rs` via `fuzz_syscalls` /
`fuzz_syscalls_with_config`. Hermes already allowlists `fuzzsc` with
`iterations<=16` and `time<=5`. Campaign catalog currently uses
`fuzzsc seed=1 iterations=1`.

**Invariants the fuzzer must keep.**

- Pointer arguments use kernel-owned scratch, not arbitrary user addresses.
- Non-returning, shell-destructive, and default-round clone/fork/exec are
  skipped.
- Each run closes tracked handles, fds, and mappings.
- Reports include seed, completed vs requested iterations, timeout flag,
  Linux/Zircon call/success/error/ENOSYS/unsupported totals.
- Replay is `fuzzsc seed=<same> iterations=<same>`.

**Syscall families this plan requires in the corpus** (today futex is
present; mqueue/kill/yield/sleep must be added):

| Family | Linux numbers / helpers | Why |
| --- | --- | --- |
| sched_yield | 124 | Ready-enqueue before `schedule()` |
| kill / tkill / tgkill | 129 / 130 / 131 | signal delivery + post-kill yield |
| nanosleep / clock_nanosleep | 101 / 115 | interrupt vs timeout |
| futex | 98 | private wait/wake, any-CPU |
| mq_open / mq_timedsend / mq_receive | 180+ | O_NONBLOCK EAGAIN, EINTR, EEXIST |
| mmap / munmap / mprotect | 222 / 215 / 226 | MAP_FIXED, permissions |
| clone (named, not default round) | 220 | opt-in only, never in Hermes random |

**Pass for a bounded fuzz round.** Completes requested iterations or hits the
time limit cleanly; does not panic, hang, or leak objects; `ENOSYS` on
explicitly unsupported numbers is allowed; unexpected success on a skipped
destructive call is a fail.

**Not in this milestone.** Coverage feedback, external syzkaller executor,
or mutating POSIX C tests on disk.

## Layer Mapping

| Layer | Skill | Where | Syscall focus | POSIX focus | Official? |
| --- | --- | --- | --- | --- | --- |
| Hygiene | — | `make host-fmt-check script-check` | n/a | n/a | yes for `make test` |
| UT | `ut` | host + python | `*_logic_shared.rs`, StagingTests | source contracts, patches | yes |
| IT | `it` | host + launcher | number routing, ABI guards | stage identity, glibc versions | yes |
| Fuzz | `fuzzing` | guest `fuzzsc` | dispatcher corpus | none (syscall ABI, not PTS) | yes, bounded |
| Fast ST | `st` | QEMU `make st` | `testsc` + `fuzzsc` | none | yes |
| POSIX canary ST | `st` | host `run-smros --api ...` | via glibc + compat `.so` | selected APIs | yes |
| POSIX campaign ST | `st` | host `make posix-run` | full selected suite | ~1600 tests | scheduled / on regression |
| Verus | — | `make verus` | proof harnesses | n/a | separate |

`make test` stays the fast offline suite and must not boot QEMU or fetch POSIX
sources. `make verify` remains UT+IT+build+ST+Verus.

## POSIX / Syscall Family Matrix

Execute families in this order. Each family follows TDD: RED contract, GREEN
minimal code, focused guest evidence. Do not open a full 1600-test campaign
per family.

1. **Identity / trivial** — `getpid`, `getppid`, `gettid`. Canary that the
   ELF, loader, and event schema work.
2. **Scheduling** — `sched_yield`. `yield_now` must mark current `Ready`, set
   the ready bit, and restore `Running` on the no-switch path. Do not patch
   `sched_yield/1-1.c`. Do not change `LINUX_SCHED_ONLINE_CPU_COUNT`.
3. **Signals** — `kill` / `raise` / `sigaction`. Keep the post-successful
   `sys_kill` yield. Compat sleep may leave SIGABRT blocked; mq poll must
   then watch SIGABRT and SIGUSR1 via `sigtimedwait`.
4. **Time** — `sleep`, `nanosleep`, `clock_nanosleep`, `timer_settime`.
   Expiry comparisons that userspace does with `time()` must not use a
   diverging `CLOCK_REALTIME`.
5. **Message queues** — `mq_open`, `mq_send`, `mq_timedsend`, `mq_receive`.
   O_NONBLOCK → EAGAIN, not a poll livelock. Parent `mq_open` O_EXCL EEXIST
   after a child-won race counts as success in 16-1. Dual-arch poll
   (`__aarch64__` \|\| `__riscv`).
6. **Futex / cond** — `FUTEX_WAIT_PRIVATE` / `FUTEX_WAKE_PRIVATE`. Any-CPU
   wait. No CPU0-only EINVAL on the production futex path.
7. **Memory** — `mmap` MAP_FIXED, `brk`, file-backed maps. 27-1 stays patched
   for supported MAP_FIXED.
8. **Threads / process** — `pthread_create`, `clone`, `fork`, `execve`/`execl`.
   Keep kill-yield because `pthread_create/14-1` depends on it.
9. **Files / fd / poll** — already in `testsc`; POSIX canaries only when a
   contract changes.

Recent focused evidence (do not treat as a new full campaign): RISC-V
identification 1596 pass / 4 fail on the hang-fix kernel, then TDD fixes for
those four plus sleep/poll side-effects; focused-c 10/10 PASS. Overlay of
identification + focused fixes is the current RISC-V expectation, not a
fresh 1600 on the newest embed. AArch64 leftover fail is `pthread_spin_trylock/1-1`.

## Dual-Architecture Rules

- Architectures in scope: AArch64 (default) and RISC-V64. x86_64 is boot-ST
  only.
- `PosixToolchain` is the single source of truth for compiler, qemu, kernel
  path, boot timeout, qemu-user, and baseline packages.
- No hardcoded architecture-specific paths or constants in new test code.
- Isolated per-arch disks and stages: `target/posix/<arch>/`.
- Guest `host_shared` is a **build-time embed**. Changing `.so` or test ELFs
  requires a kernel rebuild or the guest still runs the old snapshot.
- GLIBC: AArch64 tests bind `@GLIBC_2.17`; RISC-V binds `@GLIBC_2.27`. Keep
  dual `smros_*_glibc217` + default 2.27 version-script entries.
- RISC-V SBI HSM is not wired; `online_count` starts at 1. Yield-on-CPU0 is
  the dual-arch-correct sched_yield fix.

## Hermes Allowlist Changes

Today `classify()` allows `testsc` and bounded `fuzzsc`, and **forbids**
`posixtest` (default arm). Keep `posixtest all`, `group`, and unknown forms
forbidden.

Allow only:

```text
posixtest test getpid/1-1.c
posixtest status
```

plus a closed canary table owned by `hermes_shell_logic_shared.rs` (exact
test ids, max one id, no globs). Anything else, including `posixtest all`,
stays `Forbidden`.

Do not add `kill` to the allowlist even though signal tests need it; those
tests run as staged POSIX ELFs under the host runner, not as Hermes exec.

## Host Launcher

Keep the closed job enum. Default stays `{ut, it, st}`.

Optional protocol bump (versioned, TDD the Python parser first):

| job | argv | timeout |
| --- | --- | --- |
| `ut` | `make ut` | 300s |
| `it` | `make it` | 300s |
| `st` | `make st` with hermes-tests disk/log | existing ST timeout |
| `posix-tool` | `make posix-tool-test` | 300s |

Never add `posix-run` as a Hermes host job. A full campaign exceeds the
launcher timeout and must be a human/scheduled Make invocation.

If `make ut` / `make it` fail because `tests/host` does not compile, the
launcher must return `ERROR` with a bounded summary. The syscall plan still
passes its own UT/IT gate via `posix-tool-test` until the host crate is
restored.

## Reports And Evidence

Persist bounded reports only.

| Source | Path | Contents |
| --- | --- | --- |
| Hermes campaign | `/data/hermes/tests/` | seed, iterations, pass/fail/denied, ≤64 round details |
| Host jobs | `target/hermes-tests/<job>.log` | capped make output |
| Fast ST | `target/smros-smoke-qemu.log` or `SMROS_ST_LOG` | serial milestones |
| Fuzz | serial `SyscallFuzzReport` | seed, counts, timeout flag |
| POSIX official | `target/posix/<arch>/` seven-artifact report | events, ndjson, provenance |
| POSIX focused | `target/posix/<arch>/focused-*` | never overwrite campaign-keep |

A result is not evidence unless it names architecture, kernel identity
(path + embed hashes), stage `manifest_sha256`, and the command that produced
it. Overlay math across kernel generations is invalid.

## TDD Promotion Path

For every syscall/POSIX defect or new contract:

1. **RED** — add a StagingTest (or host unit test if that crate compiles)
   that names the production change which would make it fail.
2. **GREEN** — minimal kernel / compat / patch change.
3. **Fuzz** — if the bug is in dispatch argument handling, add a corpus case
   and run `fuzzsc seed=<n> iterations=8`.
4. **Focused ST** — host-controlled `posixtest test {id}` on an isolated disk
   for both arches that implement the API.
5. **Stop.** Do not launch a full 1600. Update the family matrix and
   campaign-keep only when a scheduled campaign is explicitly run.

Python StagingTests are the current RED/GREEN vehicle because `tests/host`
does not compile. Do not add new scheduler TDD to the broken host crate.

## Safety And Resource Limits

- Hermes positive allowlist; unknown → Forbidden.
- Permanent forbidden list unchanged: `rm`, `kill`, `reboot`, `exit`, `clear`,
  `vi`, `run`, `write`, `mkdir`, `mv`, `cp`, `mount`, `vm -k`, `docker rm`,
  `docker stop`.
- `fuzzsc` iterations ≤ 16 and time ≤ 5 s through Hermes. Interactive shell
  `fuzzsc` may use larger values; `make st` uses 1 iteration.
- Workspace/disk under 10 GB. Replace kernels in place. No extra worktrees,
  stages, or `posix-build` on a dirty tree (`posix-build` embeds
  `smros_commit`).
- Do not kill an in-use AArch64 QEMU that owns `kernel8.img` and the
  repo-root disk.

## Operator Recipes

```bash
# Fast offline (no QEMU)
make posix-tool-test
make test                         # includes posix-tool-test; ut/it currently blocked if host crate broken

# Hermes fast ST
make st
make st ARCH=riscv64gc-unknown-none-elf

# Syscall fuzz canary inside an already-booted shell
fuzzsc seed=1 iterations=8 time=5
testsc

# Official POSIX canary (host-controlled)
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli run-smros --api getpid --qemu-memory 1024M
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli run-smros \
  --api getpid --qemu-memory 1024M --arch riscv64

# Full POSIX only on regression or schedule
make posix-run
make posix-run ARCH=riscv64gc-unknown-none-elf
```

## Success Criteria For This Plan (once implemented)

- Four Hermes skills `ut`, `it`, `st`, `fuzzing` are installed and documented.
- `hermes test-all mode=syscall` (or equivalent documented sequence) runs
  bounded guest `testsc`/`fuzzsc` and requests host `ut`/`it`/`st`.
- `make st` on AArch64 and RISC-V64 requires `testsc` and `fuzzsc` markers
  and still denies `reboot`.
- `make posix-tool-test` stays green and is the syscall/POSIX UT/IT gate
  while `tests/host` is uncompilable.
- Focused POSIX canaries for the family matrix pass on both arches with
  0 timeout / 0 unresolved / 0 untested.
- Full campaign bar unchanged: ≥99% pass, fail < 2, both arches, host-controlled.
- No architecture-hardcoded paths added. Disk usage stays under 10 GB.
