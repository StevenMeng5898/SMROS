# AArch64 POSIX Clock Settime And Timer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make AArch64 `clock_settime` enforce POSIX clock semantics and make process-owned POSIX timers expire through the existing Linux signal-wakeup path, eliminating the reproduced `clock_settime/4-1.c` hang.

**Architecture:** Extend the existing host-included syscall logic with checked realtime-offset arithmetic and a pure POSIX timer record. Keep compatibility-object handles as the lifetime authority, store timer records in each `LinuxProcessResources`, and scan those bounded records from the CPU0 timer interrupt before queuing process-directed signals.

**Tech Stack:** Rust `no_std`, AArch64 Linux syscall ABI, SMROS process/task/signal runtimes, Open POSIX Test Suite, host Rust tests, Verus, QEMU.

---

## File Structure

- Modify `src/syscall/syscall_logic_shared.rs`: pure clock conversion, realtime offset, timer arming/query, and expiry logic shared with host tests.
- Modify `src/syscall/syscall.rs`: Linux ABI structs/copy helpers, atomic realtime offset, process timer ownership, clock syscalls, POSIX timer syscalls, cleanup, and IRQ-facing expiry entry point.
- Modify `src/main.rs`: invoke POSIX timer expiration from the CPU0 Rust timer handler.
- Modify `tests/host/src/lib.rs`: direct unit tests for clock arithmetic and timer state transitions.
- Modify `tests/host/tests/integration_contracts.rs`: source-level contracts for ABI validation, ownership, cleanup, and timer IRQ signal wiring.

### Task 1: Realtime Clock Arithmetic

**Files:**
- Modify: `tests/host/src/lib.rs` in `mod syscall_logic`
- Modify: `src/syscall/syscall_logic_shared.rs` beside the existing Linux clock helpers

- [ ] **Step 1: Write the failing clock-logic test**

Add this test to `mod syscall_logic`:

```rust
#[test]
fn posix_realtime_offset_is_checked_and_monotonic_is_not_settable() {
    assert!(linux_posix_clock_settable(0));
    assert!(!linux_posix_clock_settable(1));
    assert!(!linux_posix_clock_settable(usize::MAX));

    assert_eq!(linux_posix_timespec_nanoseconds(2, 3), Some(2_000_000_003));
    assert_eq!(linux_posix_timespec_nanoseconds(-1, 0), None);
    assert_eq!(linux_posix_timespec_nanoseconds(0, -1), None);
    assert_eq!(linux_posix_timespec_nanoseconds(0, 1_000_000_000), None);

    assert_eq!(linux_realtime_offset_for_set(3_000_000_000, 2, 0), Some(-1_000_000_000));
    assert_eq!(linux_realtime_from_offset(3_000_000_000, -1_000_000_000), Some(2_000_000_000));
    assert_eq!(linux_realtime_from_offset(1, -2), None);
    assert_eq!(linux_realtime_from_offset(u64::MAX, 1), None);
}
```

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
./scripts/run-host-unit-tests.sh --lib posix_realtime_offset_is_checked_and_monotonic_is_not_settable -- --exact
```

Expected: compilation fails because the four `linux_posix_*`/`linux_realtime_*` helpers do not exist.

- [ ] **Step 3: Add the minimal checked clock helpers**

Add to `src/syscall/syscall_logic_shared.rs`:

```rust
const LINUX_POSIX_NANOS_PER_SECOND: u64 = 1_000_000_000;

pub(crate) fn linux_posix_clock_settable(clock_id: usize) -> bool {
    clock_id == 0
}

pub(crate) fn linux_posix_timespec_nanoseconds(
    seconds: i64,
    nanoseconds: i64,
) -> Option<u64> {
    if seconds < 0 || !(0..LINUX_POSIX_NANOS_PER_SECOND as i64).contains(&nanoseconds) {
        return None;
    }
    (seconds as u64)
        .checked_mul(LINUX_POSIX_NANOS_PER_SECOND)?
        .checked_add(nanoseconds as u64)
}

pub(crate) fn linux_realtime_offset_for_set(
    monotonic_nanoseconds: u64,
    seconds: i64,
    nanoseconds: i64,
) -> Option<i64> {
    let requested = linux_posix_timespec_nanoseconds(seconds, nanoseconds)?;
    let offset = i128::from(requested) - i128::from(monotonic_nanoseconds);
    i64::try_from(offset).ok()
}

pub(crate) fn linux_realtime_from_offset(
    monotonic_nanoseconds: u64,
    offset_nanoseconds: i64,
) -> Option<u64> {
    if offset_nanoseconds >= 0 {
        monotonic_nanoseconds.checked_add(offset_nanoseconds as u64)
    } else {
        monotonic_nanoseconds.checked_sub(offset_nanoseconds.unsigned_abs())
    }
}
```

- [ ] **Step 4: Run the focused and complete host library tests**

Run:

```bash
./scripts/run-host-unit-tests.sh --lib posix_realtime_offset_is_checked_and_monotonic_is_not_settable -- --exact
make ut
```

Expected: the focused test passes and all host library tests report zero failures.

- [ ] **Step 5: Commit the clock arithmetic**

```bash
git add src/syscall/syscall_logic_shared.rs tests/host/src/lib.rs
git commit -m "test: define POSIX realtime clock arithmetic"
```

### Task 2: Pure POSIX Timer State

**Files:**
- Modify: `tests/host/src/lib.rs` in `mod syscall_logic`
- Modify: `src/syscall/syscall_logic_shared.rs`

- [ ] **Step 1: Write failing timer-state tests**

Add these tests to `mod syscall_logic`:

```rust
#[test]
fn posix_relative_and_absolute_timers_use_the_correct_clock_domain() {
    let mut relative = LinuxPosixTimerCore::new(7, LinuxPosixClock::Realtime, 14);
    relative
        .arm(false, 100, LinuxPosixTimerSpec { interval: 0, value: 50 })
        .unwrap();
    assert_eq!(relative.snapshot(120, 20_000).value, 30);
    assert!(!relative.expire(149, 30_000));
    assert!(relative.expire(150, 30_000));
    assert_eq!(relative.snapshot(150, 30_000).value, 0);

    let mut absolute = LinuxPosixTimerCore::new(8, LinuxPosixClock::Realtime, 14);
    absolute
        .arm(true, 100, LinuxPosixTimerSpec { interval: 0, value: 1_050 })
        .unwrap();
    assert!(!absolute.expire(200, 1_049));
    assert!(absolute.expire(200, 1_050));
}

#[test]
fn posix_timer_disarm_query_and_periodic_reschedule_are_one_shot_per_scan() {
    let mut timer = LinuxPosixTimerCore::new(9, LinuxPosixClock::Monotonic, 10);
    timer
        .arm(false, 100, LinuxPosixTimerSpec { interval: 20, value: 10 })
        .unwrap();
    assert!(timer.expire(150, 900));
    let snapshot = timer.snapshot(150, 900);
    assert_eq!(snapshot.interval, 20);
    assert_eq!(snapshot.value, 20);
    assert!(!timer.expire(150, 900));

    timer
        .arm(false, 150, LinuxPosixTimerSpec { interval: 20, value: 0 })
        .unwrap();
    assert_eq!(timer.snapshot(150, 900), LinuxPosixTimerSpec::DISARMED);
}
```

- [ ] **Step 2: Run the timer tests and verify RED**

Run:

```bash
./scripts/run-host-unit-tests.sh --lib posix_relative_and_absolute_timers_use_the_correct_clock_domain
```

Expected: compilation fails because the timer clock, spec, and record types do not exist.

- [ ] **Step 3: Implement the pure timer record**

Add the following public surface to `src/syscall/syscall_logic_shared.rs`; implement every method with checked arithmetic:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxPosixClock {
    Realtime,
    Monotonic,
}

impl LinuxPosixClock {
    pub(crate) fn from_id(clock_id: usize) -> Option<Self> {
        match clock_id {
            0 => Some(Self::Realtime),
            1 => Some(Self::Monotonic),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxPosixTimerSpec {
    pub interval: u64,
    pub value: u64,
}

impl LinuxPosixTimerSpec {
    pub(crate) const DISARMED: Self = Self { interval: 0, value: 0 };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxPosixTimerCore {
    pub handle: u32,
    pub clock: LinuxPosixClock,
    pub signal: usize,
    deadline_clock: LinuxPosixClock,
    deadline: Option<u64>,
    interval: u64,
}
```

`new` initializes a disarmed record. `arm(absolute, now_monotonic, spec)` must:

```rust
if spec.value == 0 {
    self.deadline = None;
    self.interval = 0;
    return Some(());
}
let (deadline_clock, deadline) = if absolute {
    (self.clock, spec.value)
} else {
    (LinuxPosixClock::Monotonic, now_monotonic.checked_add(spec.value)?)
};
self.deadline_clock = deadline_clock;
self.deadline = Some(deadline);
self.interval = spec.interval;
Some(())
```

`snapshot` selects `now_monotonic` or `now_realtime` from `deadline_clock` and returns a `LinuxPosixTimerSpec` with a saturating remaining `value`. `expire` returns `false` when disarmed or not due. For a due one-shot it clears `deadline`; for a due periodic timer computes the next value with checked operations:

```rust
let next = now
    .saturating_sub(deadline)
    .checked_div(self.interval)
    .and_then(|periods| periods.checked_add(1))
    .and_then(|periods| periods.checked_mul(self.interval))
    .and_then(|advance| deadline.checked_add(advance));
self.deadline = next;
```

If periodic arithmetic overflows, disarm after returning one expiration. Never return `true` twice for the same `now` value.

- [ ] **Step 4: Run focused and complete host library tests**

Run:

```bash
./scripts/run-host-unit-tests.sh --lib posix_relative_and_absolute_timers_use_the_correct_clock_domain
./scripts/run-host-unit-tests.sh --lib posix_timer_disarm_query_and_periodic_reschedule_are_one_shot_per_scan
make ut
```

Expected: both focused tests and the complete host library suite pass.

- [ ] **Step 5: Commit the pure timer state**

```bash
git add src/syscall/syscall_logic_shared.rs tests/host/src/lib.rs
git commit -m "feat: model POSIX timer deadlines"
```

### Task 3: Clock Syscall Runtime

**Files:**
- Modify: `tests/host/tests/integration_contracts.rs`
- Modify: `src/syscall/syscall.rs`

- [ ] **Step 1: Write the failing clock-runtime contract**

Add a test named `posix_clock_timer_clock_runtime_applies_checked_realtime_offsets` that extracts the bodies of `sys_clock_settime`, `sys_clock_gettime`, `sys_time`, `sys_gettimeofday`, and `reset_linux_signal_timer_state`, then asserts:

```rust
assert!(syscall.contains("static LINUX_REALTIME_OFFSET_NANOS: AtomicI64"));
assert!(settime.contains("linux_posix_clock_settable(clockid)"));
assert!(settime.contains("linux_read_user_timespec(tp)?"));
assert!(settime.contains("linux_realtime_offset_for_set("));
assert!(settime.contains("LINUX_REALTIME_OFFSET_NANOS.store("));
assert!(gettime.contains("linux_clock_nanoseconds(clock)?"));
assert!(time.contains("linux_realtime_nanos()?"));
assert!(gettimeofday.contains("linux_realtime_nanos()?"));
assert!(reset.contains("LINUX_REALTIME_OFFSET_NANOS.store(0"));
```

- [ ] **Step 2: Run the contract and verify RED**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts posix_clock_timer_clock_runtime_applies_checked_realtime_offsets -- --exact
```

Expected: the test fails because `clock_settime` still returns success without copying or applying the requested time.

- [ ] **Step 3: Implement the realtime clock runtime**

In `src/syscall/syscall.rs`:

1. Import `AtomicI64` beside the existing atomics and add `SysError::EOVERFLOW = 75`.
2. Add `#[derive(Clone, Copy)]` to `LinuxTimespec` and rename `linux_read_sleep_timespec` to `linux_read_user_timespec` at all callers.
3. Add the clock state and helpers:

```rust
static LINUX_REALTIME_OFFSET_NANOS: AtomicI64 = AtomicI64::new(0);

fn linux_realtime_nanos() -> Result<u64, SysError> {
    syscall_logic::linux_realtime_from_offset(
        monotonic_nanos(),
        LINUX_REALTIME_OFFSET_NANOS.load(Ordering::SeqCst),
    )
    .ok_or(SysError::EOVERFLOW)
}

fn linux_clock_nanoseconds(clock_id: usize) -> Result<u64, SysError> {
    match syscall_logic::LinuxPosixClock::from_id(clock_id).ok_or(SysError::EINVAL)? {
        syscall_logic::LinuxPosixClock::Realtime => linux_realtime_nanos(),
        syscall_logic::LinuxPosixClock::Monotonic => Ok(monotonic_nanos()),
    }
}
```

4. Make `sys_clock_settime` reject every non-realtime clock before user copyin, read and validate the complete timespec, calculate the checked offset from the same monotonic sample, and store it only after all checks pass.
5. Make `sys_clock_gettime` select the requested clock. Make `sys_time` and `sys_gettimeofday` use realtime.
6. Reset the offset in `reset_linux_signal_timer_state` so one POSIX test cannot leak a changed clock into the next process launch.

- [ ] **Step 4: Run the focused contract and host suites**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts posix_clock_timer_clock_runtime_applies_checked_realtime_offsets -- --exact
make ut
make it
```

Expected: the focused contract and all host tests pass.

- [ ] **Step 5: Commit the clock runtime**

```bash
git add src/syscall/syscall.rs tests/host/tests/integration_contracts.rs
git commit -m "fix: implement settable realtime clock"
```

### Task 4: POSIX Timer ABI And Process Ownership

**Files:**
- Modify: `tests/host/tests/integration_contracts.rs`
- Modify: `src/syscall/syscall.rs`

- [ ] **Step 1: Write failing timer ABI and ownership contracts**

Add `posix_clock_timer_syscalls_copy_validate_and_publish_owned_state`. It must assert that:

```rust
assert!(syscall.contains("struct LinuxItimerspec"));
assert!(syscall.contains("struct LinuxSigevent"));
assert!(syscall.contains("posix_timers: Vec<LinuxPosixTimerCore>"));
assert!(timer_create.contains("LinuxPosixClock::from_id(clockid)"));
assert!(timer_create.contains("linux_read_user_sigevent(sevp)?"));
assert!(timer_create.contains("register_linux_timer(pid, handle.0, clock, signal)"));
assert!(timer_settime.contains("linux_read_user_itimerspec(new_value)?"));
assert!(timer_settime.contains("linux_posix_timespec_nanoseconds("));
assert!(timer_settime.contains("timer.arm("));
assert!(timer_gettime.contains("timer.snapshot("));
assert!(timer_delete.contains("remove_linux_timer(pid, timerid as u32)"));
assert!(reset.contains("resources.posix_timers"));
```

Also retain the existing create/copyout rollback ordering assertions, updated for the new registration signature.

- [ ] **Step 2: Run the contract and verify RED**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts posix_clock_timer_syscalls_copy_validate_and_publish_owned_state -- --exact
```

Expected: failure because timer syscalls still ignore `clockid`, `sevp`, flags, and `itimerspec`.

- [ ] **Step 3: Add checked Linux timer ABI helpers**

Add these C-layout types:

```rust
#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxItimerspec {
    it_interval: LinuxTimespec,
    it_value: LinuxTimespec,
}

#[repr(C)]
struct LinuxSigevent {
    sigev_value: usize,
    sigev_signo: i32,
    sigev_notify: i32,
    padding: [u8; 48],
}
```

Add copy helpers that use `linux_copy_from_user`/`linux_copy_to_user` for the complete 32-byte `itimerspec` and 64-byte `sigevent`. Convert both timespec fields with `linux_posix_timespec_nanoseconds`; reject a signal outside `1..=LINUX_MAX_SIGNAL`, and accept `SIGEV_SIGNAL` (`0`) only. A null `sevp` means `SIGALRM`.

- [ ] **Step 4: Add process-owned timer records and syscall behavior**

Extend `LinuxProcessResources` with:

```rust
posix_timers: Vec<syscall_logic::LinuxPosixTimerCore>,
```

Initialize it empty for root and forked child resources. `register_linux_timer` must reserve both vectors before pushing the handle and matching record. `remove_linux_timer` must remove both. Rollback, release, fork transient-state checks, and reset must require or produce empty `posix_timers`; forked children receive no records.

Implement the syscalls in this order:

1. `timer_create`: validate timer-ID output, clock, and sigevent; create the compatibility object; register record; copy out; rollback both record and handle on copyout failure.
2. `timer_settime`: validate ownership and flags (`0` or `TIMER_ABSTIME`), copy and validate the new spec, preflight non-null old output, snapshot and copy the old state, then arm/disarm using one monotonic sample. No output failure may leave new timer state installed.
3. `timer_gettime`: validate ownership and output, snapshot using current monotonic/realtime values, and write a full `LinuxItimerspec`.
4. `timer_delete`: close the compatibility handle and remove the process timer record atomically with interrupts masked.
5. `timer_getoverrun`: retain zero for a valid owned timer and `EINVAL` otherwise.

- [ ] **Step 5: Run focused and complete host tests**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts posix_clock_timer_syscalls_copy_validate_and_publish_owned_state -- --exact
make ut
make it
```

Expected: all commands pass with zero failures.

- [ ] **Step 6: Commit timer ABI and ownership**

```bash
git add src/syscall/syscall.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: arm process-owned POSIX timers"
```

### Task 5: CPU0 Expiration And Signal Wakeup

**Files:**
- Modify: `tests/host/tests/integration_contracts.rs`
- Modify: `src/syscall/syscall.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write the failing IRQ wiring contract**

Add `posix_clock_timer_cpu0_expiry_queues_process_signals`. Extract the timer interrupt body and the new syscall expiry entry point, then assert:

```rust
assert!(timer.contains("if current_cpu_id() == 0"));
assert!(timer.contains("deliver_linux_posix_timer_signals_from_irq()"));
assert!(expiry.contains("linux_realtime_nanos()"));
assert!(expiry.contains("timer.expire(now_monotonic, now_realtime)"));
assert!(expiry.contains("queue_process_linux_signal_and_wake("));
assert!(expiry.contains("LinuxPendingSignal::standard(signal)"));
```

Assert ordering: scheduler accounting first, Linux task/sleep expiry next, POSIX timer signal delivery next, futex expiry next, and interrupt completion last.

- [ ] **Step 2: Run the contract and verify RED**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts posix_clock_timer_cpu0_expiry_queues_process_signals -- --exact
```

Expected: failure because the Rust timer handler has no POSIX timer expiration call.

- [ ] **Step 3: Implement bounded IRQ expiration**

Add this public IRQ-facing shape to `src/syscall/syscall.rs`:

```rust
pub fn deliver_linux_posix_timer_signals_from_irq() {
    let now_monotonic = monotonic_nanos();
    let Ok(now_realtime) = linux_realtime_nanos() else {
        return;
    };
    let state = memory_state();
    for resources in &mut state.linux_process_resources {
        for timer in &mut resources.posix_timers {
            if timer.expire(now_monotonic, now_realtime) {
                let signal = timer.signal;
                let _ = queue_process_linux_signal_and_wake(
                    resources.pid,
                    LinuxPendingSignal::standard(signal),
                );
            }
        }
    }
}
```

The implementation must not allocate, must advance/disarm a record before queuing its signal, and must execute only inside the existing CPU0 guard. Iterate the process-resource and timer-record vectors in place; signal routing does not access `MemorySyscallState`, so it cannot invalidate the scan.

Call it from `src/main.rs` after `linux_task::on_timer_tick(now)` and before `linux_futex::on_timer_tick(...)`.

- [ ] **Step 4: Run host tests and formatting**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts posix_clock_timer_cpu0_expiry_queues_process_signals -- --exact
make ut
make it
cargo fmt --all -- --check
git diff --check
```

Expected: all commands pass, formatting is unchanged after `cargo fmt --all`, and `git diff --check` emits no errors.

- [ ] **Step 5: Commit IRQ signal delivery**

```bash
git add src/main.rs src/syscall/syscall.rs tests/host/tests/integration_contracts.rs
git commit -m "fix: deliver expired POSIX timer signals"
```

### Task 6: Static, Build, And AArch64 Guest Verification

**Files:**
- Modify only if a verification failure exposes a defect in files already in scope.
- Record generated evidence under `target/posix/aarch64/`; do not commit generated artifacts.

- [ ] **Step 1: Run complete local static and build gates**

Run:

```bash
make ut
make it
make verus-syscall
make build-test ARCH=aarch64-unknown-none
cargo fmt --all -- --check
git diff --check
```

Expected: host suites and Verus pass; the AArch64 release/link-layout build exits zero with warnings denied and prints no Rust warning; formatting and diff checks pass.

- [ ] **Step 2: Capture honest coverage and Coverity availability**

Run `make coverage-host`, retain its complete log under `target/posix/aarch64/clock-timer-quality/`, and record the real exit status. Check:

```bash
command -v cov-build
command -v cov-analyze
command -v cov-format-errors
```

If all Coverity commands exist, run a fresh `cov-build --dir <commit-specific-dir> make build-test ARCH=aarch64-unknown-none`, followed by `cov-analyze --all` and `cov-format-errors --json-output-v7`; retain the log and JSON. If any command is absent, record status `unavailable`, the exact missing commands, and no invented finding count. If coverage fails, record `failed`, the exact error, and no invented percentage.

- [ ] **Step 3: Build commit-matched POSIX and kernel artifacts**

Run:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest --verify-only
make build-test ARCH=aarch64-unknown-none
```

Expected: POSIX stage verification passes and the embedded manifest/build metadata names the implementation commit.

- [ ] **Step 4: Run `clock_settime/20-1.c` on a fresh disk**

Create a new private disk, then invoke `scripts.posix.qemu_runner.run_smros` with:

```python
test_id = "conformance/interfaces/clock_settime/20-1.c"
```

Assert `complete`, `restart_count == 0`, one attempt, `status == "pass"`, `pts_status == "pass"`, `exit_code == 0`, `timed_out == False`, and no positive resource delta. Retain the raw serial log and result JSON.

- [ ] **Step 5: Run the formerly hanging `clock_settime/4-1.c` on another fresh disk**

Invoke the same runner with:

```python
test_id = "conformance/interfaces/clock_settime/4-1.c"
```

Assert the same pass conditions. The log must contain `Test PASSED`, must advance past the `test_start` event to a matching `test_end`, and must contain no kernel panic, fatal glibc error, timeout, restart, or loader failure.

- [ ] **Step 6: Run the complete `clock_settime` selection**

Run the API filter on a third fresh private disk and retain structured output. Every selected test must reach `test_end`; specifically, the campaign must not stop after `4-1.c`. Record individual pass/fail/unresolved results exactly rather than claiming group compliance from only the two canaries.

- [ ] **Step 7: Review and commit any verification-only repair**

If verification required an in-scope repair, repeat its focused RED/GREEN test and all Step 1 gates, then commit only that repair. Finish with:

```bash
git status --short --branch
git log --oneline --decorate master..HEAD
```

Expected: no uncommitted source changes and a linear series of focused design, test, implementation, and repair commits.
