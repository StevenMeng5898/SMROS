# AArch64 Linux Sleep Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement scheduler-backed AArch64 Linux `nanosleep` and `clock_nanosleep` with checked deadlines, timer expiry, signal interruption, and remaining-time copyout.

**Architecture:** Extend the fixed Linux task table with one identity-bound sleep record per task and a `Sleep` block reason. The existing CPU0 timer path expires records and wakes exact scheduler identities; signal routing interrupts only unmasked sleeping targets. A unified syscall helper validates userspace timespecs, publishes the wait atomically, blocks through the scheduler, and maps terminal outcomes to Linux results.

**Tech Stack:** Rust `no_std`, fixed-capacity task tables, AArch64 userspace mapping checks, scheduler task transitions, 100 Hz timer IRQs, Cargo host tests, source integration contracts, Verus, Open POSIX Test Suite, and QEMU AArch64 system emulation.

---

## File Map

- Modify `src/syscall/linux_task_logic_shared.rs`: define sleep deadlines, outcomes, records, task-table ownership, expiry, interruption, and cleanup rules.
- Modify `src/syscall/linux_task.rs`: expose current-task sleep operations, wake expired sleepers, and handle failed wakes.
- Modify `src/syscall/syscall_logic_shared.rs`: define the pure `clock_nanosleep` flag rule.
- Modify `src/syscall/syscall_logic.rs`: expose the production wrapper for that flag rule.
- Modify `src/syscall/syscall.rs`: validate timespecs and mappings, block relative/absolute sleeps, copy remaining time, and interrupt sleeps during signal routing.
- Modify `tests/host/src/lib.rs`: test deadline arithmetic, one-shot task sleep state, signal masks, cleanup, and flag validation.
- Modify `tests/host/tests/integration_contracts.rs`: lock syscall publication order, timer expiry, exact wake identity, and signal interruption wiring.
- Generated `host_shared/posixtest/`: rebuild the commit-matched AArch64 POSIX stage.
- Generated `target/posix/aarch64/thread-runtime-quality.json`: retain exact coverage/Coverity outcomes with the implementation commit.
- Generated `target/posix/aarch64/smros-fxfs-sleep-canary.img`: private canary disk.
- Generated `target/posix/aarch64/smros-run-sleep-canary-*`: canary evidence directories.

### Task 1: Add Pure Per-Task Sleep State And Deadline Rules

**Files:**
- Modify: `src/syscall/linux_task_logic_shared.rs:158-173,716-733,1118-1134,1137-1201,1249-1262,1525-1651`
- Modify: `tests/host/src/lib.rs:353-590`

- [ ] **Step 1: Write failing deadline tests**

Add these tests inside `mod linux_task_logic` in `tests/host/src/lib.rs`:

```rust
#[test]
fn linux_sleep_deadlines_round_up_and_remaining_time_never_goes_negative() {
    const TICK_NANOS: u64 = 10_000_000;

    assert_eq!(
        linux_sleep_relative_deadline_ticks(40, 0, 0, TICK_NANOS),
        Some(40)
    );
    assert_eq!(
        linux_sleep_relative_deadline_ticks(40, 0, 1, TICK_NANOS),
        Some(42)
    );
    assert_eq!(
        linux_sleep_relative_deadline_ticks(40, 1, 0, TICK_NANOS),
        Some(141)
    );
    assert_eq!(
        linux_sleep_absolute_deadline_ticks(0, 1, TICK_NANOS),
        Some(1)
    );
    assert_eq!(
        linux_sleep_absolute_deadline_ticks(1, 0, TICK_NANOS),
        Some(100)
    );
    assert_eq!(
        linux_sleep_remaining_timespec(141, 41, TICK_NANOS),
        Some((1, 0))
    );
    assert_eq!(
        linux_sleep_remaining_timespec(41, 141, TICK_NANOS),
        Some((0, 0))
    );
    assert_eq!(linux_sleep_relative_deadline_ticks(0, -1, 0, TICK_NANOS), None);
    assert_eq!(linux_sleep_absolute_deadline_ticks(0, 1_000_000_000, TICK_NANOS), None);
    assert_eq!(linux_sleep_remaining_timespec(1, 0, 0), None);
}
```

- [ ] **Step 2: Write failing task-state tests**

Add this second test in the same module:

```rust
#[test]
fn linux_sleep_waits_expire_or_interrupt_once_and_reset_with_their_task() {
    let mut tasks = LinuxTaskTable::<3>::new();
    tasks.register_root(7).unwrap();
    let child = tasks.reserve_child(8).unwrap();
    assert!(tasks.publish(child));

    assert!(tasks.install_sleep(child.tid, 8, LinuxSleepWait::waiting(50)));
    assert!(!tasks.install_sleep(child.tid, 8, LinuxSleepWait::waiting(60)));
    assert!(tasks.block(child.tid, 8, LinuxBlockReason::Sleep));
    assert_eq!(tasks.expire_sleeps(49), [None, None, None]);
    let expired = tasks.expire_sleeps(50);
    assert_eq!(expired[0], Some((child.tid, 8, LinuxBlockReason::Sleep)));
    assert!(expired[1..].iter().all(Option::is_none));
    assert_eq!(tasks.expire_sleeps(51), [None, None, None]);
    assert!(tasks.wake(child.tid, 8));
    assert_eq!(
        tasks.take_sleep_outcome(child.tid, 8),
        Some(LinuxSleepWait {
            deadline: 50,
            outcome: LinuxSleepOutcome::Completed,
        })
    );
    assert_eq!(tasks.take_sleep_outcome(child.tid, 8), None);

    assert!(tasks.install_sleep(child.tid, 8, LinuxSleepWait::waiting(80)));
    assert!(tasks.block(child.tid, 8, LinuxBlockReason::Sleep));
    tasks.signal_state_mut(child.tid, 8).unwrap().mask = linux_signal_bit(6);
    assert!(!tasks.interrupt_sleep(child.tid, 8, 6));
    tasks.signal_state_mut(child.tid, 8).unwrap().mask = 0;
    assert!(tasks.interrupt_sleep(child.tid, 8, 6));
    assert!(!tasks.interrupt_sleep(child.tid, 8, 6));
    assert!(tasks.wake(child.tid, 8));
    assert_eq!(
        tasks.take_sleep_outcome(child.tid, 8).map(|wait| wait.outcome),
        Some(LinuxSleepOutcome::Interrupted)
    );

    assert!(tasks.install_sleep(child.tid, 8, LinuxSleepWait::waiting(90)));
    assert!(tasks.exit(child.tid, 8));
    assert!(tasks.retire(child.tid, 8));
    assert_eq!(tasks.take_sleep_outcome(child.tid, 8), None);
    let replacement = tasks.reserve_child(9).unwrap();
    assert!(tasks.publish(replacement));
    assert!(!tasks.interrupt_sleep(child.tid, 8, 6));
    assert_eq!(tasks.expire_sleeps(90), [None, None, None]);

    let rollback = tasks.reserve_child(10).unwrap();
    tasks.sleep_waits[rollback.slot] = Some(LinuxSleepWait::waiting(95));
    assert!(tasks.rollback(rollback));
    assert_eq!(tasks.sleep_waits[rollback.slot], None);

    tasks.reset();
    assert_eq!(tasks.expire_sleeps(u64::MAX), [None, None, None]);
}
```

- [ ] **Step 3: Run focused tests and verify RED**

Run:

```bash
./scripts/run-host-unit-tests.sh --lib linux_task_logic::linux_sleep
```

Expected: compilation fails because `LinuxSleepWait`, `LinuxSleepOutcome`,
`LinuxBlockReason::Sleep`, the deadline helpers, and the task-table sleep
methods do not exist.

- [ ] **Step 4: Add checked deadline helpers and sleep types**

In `src/syscall/linux_task_logic_shared.rs`, add `Sleep` to
`LinuxBlockReason`, then define:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxSleepOutcome {
    Waiting,
    Completed,
    Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxSleepWait {
    pub deadline: u64,
    pub outcome: LinuxSleepOutcome,
}

impl LinuxSleepWait {
    pub(crate) const fn waiting(deadline: u64) -> Self {
        Self {
            deadline,
            outcome: LinuxSleepOutcome::Waiting,
        }
    }
}

pub(crate) fn linux_sleep_relative_deadline_ticks(
    now: u64,
    seconds: i64,
    nanoseconds: i64,
    tick_nanoseconds: u64,
) -> Option<u64> {
    linux_signal_timespec_to_ticks_ceil(now, seconds, nanoseconds, tick_nanoseconds)
}

pub(crate) fn linux_sleep_absolute_deadline_ticks(
    seconds: i64,
    nanoseconds: i64,
    tick_nanoseconds: u64,
) -> Option<u64> {
    if seconds < 0 || !(0..1_000_000_000).contains(&nanoseconds) || tick_nanoseconds == 0 {
        return None;
    }
    let total_nanoseconds = (seconds as u64)
        .checked_mul(1_000_000_000)?
        .checked_add(nanoseconds as u64)?;
    total_nanoseconds
        .checked_add(tick_nanoseconds - 1)?
        .checked_div(tick_nanoseconds)
}

pub(crate) fn linux_sleep_remaining_timespec(
    deadline: u64,
    now: u64,
    tick_nanoseconds: u64,
) -> Option<(i64, i64)> {
    if tick_nanoseconds == 0 {
        return None;
    }
    let remaining_nanoseconds = deadline
        .saturating_sub(now)
        .checked_mul(tick_nanoseconds)?;
    let seconds = i64::try_from(remaining_nanoseconds / 1_000_000_000).ok()?;
    let nanoseconds = i64::try_from(remaining_nanoseconds % 1_000_000_000).ok()?;
    Some((seconds, nanoseconds))
}
```

- [ ] **Step 5: Store and transition one sleep record per task**

Add `sleep_waits: [Option<LinuxSleepWait>; N]` to `LinuxTaskTable<N>` and
initialize it with `[None; N]`. Clear the matching slot in `register_root`,
`reserve_child`, `rollback`, `exit_with_clear_child_tid`, and `retire`; call
`self.sleep_waits.fill(None)` in `reset`.

Add these methods to `LinuxTaskTable<N>`:

```rust
pub(crate) fn install_sleep(
    &mut self,
    tid: usize,
    scheduler_thread: usize,
    wait: LinuxSleepWait,
) -> bool {
    let Some(slot) = self.task_slot(tid, scheduler_thread) else {
        return false;
    };
    if self.sleep_waits[slot].is_some() {
        return false;
    }
    self.sleep_waits[slot] = Some(wait);
    true
}

pub(crate) fn take_sleep_outcome(
    &mut self,
    tid: usize,
    scheduler_thread: usize,
) -> Option<LinuxSleepWait> {
    let slot = self.task_slot(tid, scheduler_thread)?;
    match self.sleep_waits[slot] {
        Some(wait) if wait.outcome != LinuxSleepOutcome::Waiting => {
            self.sleep_waits[slot].take()
        }
        Some(_) | None => None,
    }
}

pub(crate) fn cancel_sleep(&mut self, tid: usize, scheduler_thread: usize) -> bool {
    let Some(slot) = self.task_slot(tid, scheduler_thread) else {
        return false;
    };
    self.sleep_waits[slot].take().is_some()
}

pub(crate) fn interrupt_sleep(
    &mut self,
    tid: usize,
    scheduler_thread: usize,
    signum: usize,
) -> bool {
    let Some(slot) = self.task_slot(tid, scheduler_thread) else {
        return false;
    };
    let task = self.tasks[slot];
    let bit = linux_signal_bit(signum);
    if task.state != LinuxTaskState::Blocked
        || task.block_reason != LinuxBlockReason::Sleep
        || bit == 0
        || self.signal_states[slot].mask & bit != 0
    {
        return false;
    }
    let Some(wait) = self.sleep_waits[slot].as_mut() else {
        return false;
    };
    if wait.outcome != LinuxSleepOutcome::Waiting {
        return false;
    }
    wait.outcome = LinuxSleepOutcome::Interrupted;
    true
}

pub(crate) fn expire_sleeps(
    &mut self,
    now: u64,
) -> [Option<(usize, usize, LinuxBlockReason)>; N] {
    let mut expired = [None; N];
    let mut expired_len = 0usize;
    for index in 0..N {
        let task = self.tasks[index];
        let Some(wait) = self.sleep_waits[index].as_mut() else {
            continue;
        };
        if task.state == LinuxTaskState::Blocked
            && task.block_reason == LinuxBlockReason::Sleep
            && wait.outcome == LinuxSleepOutcome::Waiting
            && wait.deadline <= now
        {
            wait.outcome = LinuxSleepOutcome::Completed;
            expired[expired_len] = Some((task.tid, task.scheduler_thread, task.block_reason));
            expired_len += 1;
        }
    }
    expired
}
```

- [ ] **Step 6: Run focused tests and verify GREEN**

Run:

```bash
./scripts/run-host-unit-tests.sh --lib linux_task_logic::linux_sleep
```

Expected: both new sleep tests pass with zero failures.

- [ ] **Step 7: Run syscall verification and commit**

Run:

```bash
make verus-syscall
git diff --check
```

Expected: Verus reports zero errors and the diff check exits zero.

Commit only the task logic and its tests:

```bash
git add src/syscall/linux_task_logic_shared.rs tests/host/src/lib.rs
git commit -m "feat: add per-task Linux sleep state"
```

### Task 2: Wire Timer Expiry And Signal Interruption

**Files:**
- Modify: `src/syscall/linux_task.rs:204-255,289-349,351-433`
- Modify: `src/syscall/syscall.rs:2891-2993`
- Modify: `tests/host/tests/integration_contracts.rs:2416-2696`

- [ ] **Step 1: Write the failing production integration contract**

Add this test to `tests/host/tests/integration_contracts.rs`:

```rust
#[test]
fn linux_sleeps_expire_or_interrupt_only_the_matching_task() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let task_logic =
        std::fs::read_to_string(repository.join("src/syscall/linux_task_logic_shared.rs"))
            .expect("read Linux task logic");
    let task = std::fs::read_to_string(repository.join("src/syscall/linux_task.rs"))
        .expect("read Linux task runtime");
    let syscall = std::fs::read_to_string(repository.join("src/syscall/syscall.rs"))
        .expect("read Linux syscall runtime");

    assert!(task_logic.contains("sleep_waits: [Option<LinuxSleepWait>; N]"));
    assert!(task_logic.contains("pub(crate) fn expire_sleeps("));
    assert!(task_logic.contains("pub(crate) fn interrupt_sleep("));
    assert!(task_logic.contains("self.signal_states[slot].mask & bit != 0"));
    assert!(task.contains("pub(crate) fn install_current_sleep("));
    assert!(task.contains("pub(crate) fn take_current_sleep_outcome("));
    assert!(task.contains("pub(crate) fn cancel_current_sleep("));

    let timer_start = task
        .find("pub(crate) fn on_timer_tick(")
        .expect("Linux task timer hook");
    let timer = braced_body(&task[timer_start..]);
    let expire = timer.find("expire_sleeps(now)").expect("sleep expiry");
    let wake = timer
        .find("wake_blocked(tid, scheduler_thread, reason)")
        .expect("exact scheduler wake");
    let cancel = timer
        .find("cancel_sleep(tid, scheduler_thread)")
        .expect("failed wake cleanup");
    assert!(expire < wake && wake < cancel);

    let interrupt_start = syscall
        .find("fn interrupt_linux_signal_target(")
        .expect("signal interruption helper");
    let interrupt = braced_body(&syscall[interrupt_start..]);
    assert!(interrupt.contains("LinuxBlockReason::Sleep"));
    assert!(interrupt.contains("linux_task::interrupt_sleep("));
    assert!(interrupt.contains("linux_task::wake_blocked("));
    assert!(interrupt.contains("linux_task::cancel_sleep("));

    for caller in [
        "fn queue_process_linux_signal_and_wake(",
        "fn queue_directed_linux_signal(",
    ] {
        let start = syscall.find(caller).expect("signal routing caller");
        let body = braced_body(&syscall[start..]);
        assert!(body.contains("interrupt_linux_signal_target("));
        assert!(body.contains("record.signum"));
    }
}
```

- [ ] **Step 2: Run the contract and verify RED**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts linux_sleeps_expire_or_interrupt_only_the_matching_task -- --exact
```

Expected: the contract fails because production wrappers, timer expiry, and
sleep-aware signal interruption are absent.

- [ ] **Step 3: Add production task wrappers**

Add these functions in `src/syscall/linux_task.rs`:

```rust
pub(crate) fn install_current_sleep(wait: LinuxSleepWait) -> Result<bool, SysError> {
    with_runtime(|runtime| {
        let scheduler_thread = scheduler::scheduler().current();
        let task = runtime
            .tasks
            .by_scheduler(scheduler_thread.0)
            .ok_or(SysError::ESRCH)?;
        Ok(runtime
            .tasks
            .install_sleep(task.tid, scheduler_thread.0, wait))
    })
}

pub(crate) fn take_current_sleep_outcome() -> Result<Option<LinuxSleepWait>, SysError> {
    with_runtime(|runtime| {
        let scheduler_thread = scheduler::scheduler().current();
        let task = runtime
            .tasks
            .by_scheduler(scheduler_thread.0)
            .ok_or(SysError::ESRCH)?;
        Ok(runtime
            .tasks
            .take_sleep_outcome(task.tid, scheduler_thread.0))
    })
}

pub(crate) fn cancel_current_sleep() -> Result<bool, SysError> {
    with_runtime(|runtime| {
        let scheduler_thread = scheduler::scheduler().current();
        let task = runtime
            .tasks
            .by_scheduler(scheduler_thread.0)
            .ok_or(SysError::ESRCH)?;
        Ok(runtime.tasks.cancel_sleep(task.tid, scheduler_thread.0))
    })
}

pub(crate) fn cancel_sleep(tid: usize, scheduler_thread: usize) -> bool {
    with_runtime(|runtime| runtime.tasks.cancel_sleep(tid, scheduler_thread))
}

pub(crate) fn interrupt_sleep(tid: usize, scheduler_thread: usize, signum: usize) -> bool {
    with_runtime(|runtime| runtime.tasks.interrupt_sleep(tid, scheduler_thread, signum))
}
```

- [ ] **Step 4: Expire sleeps from the existing timer hook**

After the signal-wait expiry loop in `linux_task::on_timer_tick`, add:

```rust
let expired_sleeps = with_runtime(|runtime| runtime.tasks.expire_sleeps(now));
for identity in expired_sleeps.into_iter().flatten() {
    let (tid, scheduler_thread, reason) = identity;
    if !wake_blocked(tid, scheduler_thread, reason) {
        let _ = cancel_sleep(tid, scheduler_thread);
    }
}
```

Keep this inside the existing AArch64 CPU0 guard. Do not add another timer
hook in `main.rs`.

- [ ] **Step 5: Interrupt only an unmasked sleeping target**

Change the helper in `src/syscall/syscall.rs` to accept a signal number and
handle both futex and sleep waits:

```rust
fn interrupt_linux_signal_target(target: linux_task::LinuxTaskCore, signum: usize) {
    if target.state != linux_task::LinuxTaskState::Blocked {
        return;
    }
    match target.block_reason {
        linux_task::LinuxBlockReason::Futex => {
            let _ = linux_futex::interrupt_task(target.tid, target.scheduler_thread);
        }
        linux_task::LinuxBlockReason::Sleep => {
            if linux_task::interrupt_sleep(target.tid, target.scheduler_thread, signum)
                && !linux_task::wake_blocked(
                    target.tid,
                    target.scheduler_thread,
                    LinuxBlockReason::Sleep,
                )
            {
                let _ = linux_task::cancel_sleep(target.tid, target.scheduler_thread);
            }
        }
        _ => {}
    }
}
```

Pass `record.signum` from `queue_process_linux_signal_and_wake` and
`queue_directed_linux_signal`. Keep record queueing before interruption.

- [ ] **Step 6: Run the contract and focused logic tests**

Run:

```bash
./scripts/run-host-unit-tests.sh --test integration_contracts linux_sleeps_expire_or_interrupt_only_the_matching_task -- --exact
./scripts/run-host-unit-tests.sh --lib linux_task_logic::linux_sleep
```

Expected: the integration contract and both sleep logic tests pass.

- [ ] **Step 7: Build AArch64 and commit**

Run:

```bash
rustfmt --edition 2021 --check src/syscall/linux_task.rs src/syscall/syscall.rs
make build-test ARCH=aarch64-unknown-none
git diff --check
```

Expected: formatting, the AArch64 release/link-layout build, and diff checks
exit zero.

Commit only the production timer/signal wiring and its contract:

```bash
git add src/syscall/linux_task.rs src/syscall/syscall.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: wake Linux sleeps on timers and signals"
```

### Task 3: Implement `nanosleep` And `clock_nanosleep`

**Files:**
- Modify: `src/syscall/syscall_logic_shared.rs:90-94`
- Modify: `src/syscall/syscall_logic.rs:50-60`
- Modify: `src/syscall/syscall.rs:462-475,772-775,6093-6108,6825-6885,9672-9680,10263-10277`
- Modify: `tests/host/src/lib.rs:340-365`
- Modify: `tests/host/tests/integration_contracts.rs`

- [ ] **Step 1: Write failing flag-validation test**

Add to `mod syscall_logic` in `tests/host/src/lib.rs`:

```rust
#[test]
fn clock_nanosleep_accepts_only_relative_or_timer_abstime_flags() {
    const TIMER_ABSTIME: usize = 1;
    assert!(smros_linux_clock_nanosleep_flags_valid_body!(0, TIMER_ABSTIME));
    assert!(smros_linux_clock_nanosleep_flags_valid_body!(
        TIMER_ABSTIME,
        TIMER_ABSTIME
    ));
    assert!(!smros_linux_clock_nanosleep_flags_valid_body!(
        2,
        TIMER_ABSTIME
    ));
    assert!(!smros_linux_clock_nanosleep_flags_valid_body!(
        usize::MAX,
        TIMER_ABSTIME
    ));
}
```

- [ ] **Step 2: Extend the integration contract with syscall requirements**

In `linux_sleeps_expire_or_interrupt_only_the_matching_task`, add:

```rust
assert!(syscall.contains("const LINUX_TIMER_ABSTIME: usize = 1"));
assert!(syscall.contains("pub fn sys_nanosleep_linux(req: usize, rem: usize)"));
assert!(syscall.contains("linux_task::install_current_sleep("));
assert!(syscall.contains("linux_task::block_current(LinuxBlockReason::Sleep)"));
assert!(syscall.contains("scheduler::schedule();"));
assert!(syscall.contains("LinuxSleepOutcome::Completed => Ok(0)"));
assert!(syscall.contains("LinuxSleepOutcome::Interrupted"));
assert!(syscall.contains("Err(SysError::EINTR)"));
assert!(syscall.contains("linux_sleep_remaining_timespec("));
assert!(syscall.contains("ARM64_SYS_NANOSLEEP => sys_nanosleep_linux(args[0], args[1])"));

let sleep_until_start = syscall
    .find("fn linux_sleep_until(")
    .expect("blocking Linux sleep helper");
let sleep_until = braced_body(&syscall[sleep_until_start..]);
let validate = sleep_until
    .find("linux_sleep_user_range_writable(")
    .expect("remaining buffer validation");
let mask = sleep_until.find("mask_interrupts()").expect("IRQ mask");
let install = sleep_until
    .find("install_current_sleep(")
    .expect("sleep publication");
let block = sleep_until
    .find("block_current(LinuxBlockReason::Sleep)")
    .expect("task block");
let schedule = sleep_until
    .find("scheduler::schedule();")
    .expect("schedule");
assert!(validate < mask && mask < install && install < block && block < schedule);
assert!(sleep_until.contains("let _ = linux_task::cancel_current_sleep();"));

let nanosleep_start = syscall
    .find("pub fn sys_nanosleep_linux(")
    .expect("Linux nanosleep syscall");
let nanosleep = braced_body(&syscall[nanosleep_start..]);
assert!(nanosleep.contains("linux_sleep_relative_deadline_ticks("));
assert!(nanosleep.contains("linux_sleep_until(deadline, rem, false)"));
assert!(!nanosleep.contains("if req == 0"));
```

- [ ] **Step 3: Run focused tests and verify RED**

Run:

```bash
./scripts/run-host-unit-tests.sh --lib syscall_logic::clock_nanosleep
./scripts/run-host-unit-tests.sh --test integration_contracts linux_sleeps_expire_or_interrupt_only_the_matching_task -- --exact
```

Expected: the unit test fails because the flag helper is missing, and the
contract fails because the Linux sleep syscalls still return immediately.

- [ ] **Step 4: Add the shared flag rule**

In `src/syscall/syscall_logic_shared.rs`, add:

```rust
macro_rules! smros_linux_clock_nanosleep_flags_valid_body {
    ($flags:expr, $timer_abstime:expr) => {{
        ($flags & !$timer_abstime) == 0
    }};
}
```

Expose it from `src/syscall/syscall_logic.rs`:

```rust
pub(crate) fn linux_clock_nanosleep_flags_valid(flags: usize, timer_abstime: usize) -> bool {
    smros_linux_clock_nanosleep_flags_valid_body!(flags, timer_abstime)
}
```

- [ ] **Step 5: Add checked timespec and remaining-time helpers**

In `src/syscall/syscall.rs`, define `LINUX_TIMER_ABSTIME` beside the Linux
clock constants and add these helpers near the existing signal userspace range
helpers:

```rust
fn linux_sleep_user_range_readable(address: usize, len: usize) -> bool {
    if !syscall_logic::user_buffer_valid(address, len) {
        return false;
    }
    #[cfg(target_arch = "aarch64")]
    {
        return linux_user_range_readable(address, len);
    }
    #[cfg(not(target_arch = "aarch64"))]
    true
}

fn linux_sleep_user_range_writable(address: usize, len: usize) -> bool {
    if !syscall_logic::user_buffer_valid(address, len) {
        return false;
    }
    #[cfg(target_arch = "aarch64")]
    {
        return linux_user_range_writable(address, len);
    }
    #[cfg(not(target_arch = "aarch64"))]
    true
}

fn linux_read_sleep_timespec(address: usize) -> Result<LinuxTimespec, SysError> {
    let size = core::mem::size_of::<LinuxTimespec>();
    if !linux_sleep_user_range_readable(address, size) {
        return Err(SysError::EFAULT);
    }
    Ok(unsafe { core::ptr::read_unaligned(address as *const LinuxTimespec) })
}

fn linux_write_sleep_remaining(address: usize, wait: LinuxSleepWait) -> SysResult {
    if address == 0 {
        return Ok(0);
    }
    let size = core::mem::size_of::<LinuxTimespec>();
    if !linux_sleep_user_range_writable(address, size) {
        return Err(SysError::EFAULT);
    }
    let now = crate::kernel_lowlevel::timer::get_tick_count();
    let (tv_sec, tv_nsec) = linux_task::linux_sleep_remaining_timespec(
        wait.deadline,
        now,
        LINUX_SIGNAL_TICK_NANOS,
    )
    .ok_or(SysError::EINVAL)?;
    unsafe {
        core::ptr::write_unaligned(address as *mut LinuxTimespec, LinuxTimespec {
            tv_sec,
            tv_nsec,
        });
    }
    Ok(0)
}
```

Import `LinuxSleepOutcome` and `LinuxSleepWait` through the existing
`linux_task` include/re-export path used for `LinuxBlockReason`.

- [ ] **Step 6: Implement the unified blocking sleep path**

Add this private helper in `src/syscall/syscall.rs`:

```rust
fn linux_sleep_until(deadline: u64, rem: usize, absolute: bool) -> SysResult {
    let now = crate::kernel_lowlevel::timer::get_tick_count();
    if deadline <= now {
        return Ok(0);
    }
    if !absolute
        && rem != 0
        && !linux_sleep_user_range_writable(rem, core::mem::size_of::<LinuxTimespec>())
    {
        return Err(SysError::EFAULT);
    }

    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let wait = LinuxSleepWait::waiting(deadline);
    let installed = match linux_task::install_current_sleep(wait) {
        Ok(installed) => installed,
        Err(error) => {
            crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
            return Err(error);
        }
    };
    if !installed {
        crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
        return Err(SysError::EAGAIN);
    }
    if linux_task::block_current(LinuxBlockReason::Sleep).is_err() {
        let _ = linux_task::cancel_current_sleep();
        crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
        return Err(SysError::EAGAIN);
    }

    scheduler::schedule();
    let outcome = match linux_task::take_current_sleep_outcome() {
        Ok(outcome) => outcome,
        Err(error) => {
            let _ = linux_task::cancel_current_sleep();
            crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
            return Err(error);
        }
    };
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    match outcome {
        Some(wait) => match wait.outcome {
            LinuxSleepOutcome::Completed => Ok(0),
            LinuxSleepOutcome::Interrupted => {
                if !absolute {
                    linux_write_sleep_remaining(rem, wait)?;
                }
                Err(SysError::EINTR)
            }
            LinuxSleepOutcome::Waiting => {
                let _ = linux_task::cancel_current_sleep();
                Err(SysError::EAGAIN)
            }
        },
        None => {
            let _ = linux_task::cancel_current_sleep();
            Err(SysError::EAGAIN)
        }
    }
}
```

- [ ] **Step 7: Replace both Linux sleep stubs and preserve the ABI**

Replace `sys_nanosleep_linux` and `sys_clock_nanosleep` with:

```rust
pub fn sys_nanosleep_linux(req: usize, rem: usize) -> SysResult {
    let requested = linux_read_sleep_timespec(req)?;
    let now = crate::kernel_lowlevel::timer::get_tick_count();
    let deadline = linux_task::linux_sleep_relative_deadline_ticks(
        now,
        requested.tv_sec,
        requested.tv_nsec,
        LINUX_SIGNAL_TICK_NANOS,
    )
    .ok_or(SysError::EINVAL)?;
    linux_sleep_until(deadline, rem, false)
}

pub fn sys_clock_nanosleep(clockid: usize, flags: usize, req: usize, rem: usize) -> SysResult {
    if !syscall_logic::linux_clock_id_supported(clockid)
        || !syscall_logic::linux_clock_nanosleep_flags_valid(flags, LINUX_TIMER_ABSTIME)
    {
        return Err(SysError::EINVAL);
    }
    let requested = linux_read_sleep_timespec(req)?;
    let absolute = flags & LINUX_TIMER_ABSTIME != 0;
    let now = crate::kernel_lowlevel::timer::get_tick_count();
    let deadline = if absolute {
        linux_task::linux_sleep_absolute_deadline_ticks(
            requested.tv_sec,
            requested.tv_nsec,
            LINUX_SIGNAL_TICK_NANOS,
        )
    } else {
        linux_task::linux_sleep_relative_deadline_ticks(
            now,
            requested.tv_sec,
            requested.tv_nsec,
            LINUX_SIGNAL_TICK_NANOS,
        )
    }
    .ok_or(SysError::EINVAL)?;
    linux_sleep_until(deadline, rem, absolute)
}
```

Change the AArch64 dispatch arm to:

```rust
ARM64_SYS_NANOSLEEP => sys_nanosleep_linux(args[0], args[1]),
```

- [ ] **Step 8: Run focused GREEN tests**

Run:

```bash
./scripts/run-host-unit-tests.sh --lib syscall_logic::clock_nanosleep
./scripts/run-host-unit-tests.sh --lib linux_task_logic::linux_sleep
./scripts/run-host-unit-tests.sh --test integration_contracts linux_sleeps_expire_or_interrupt_only_the_matching_task -- --exact
```

Expected: all focused unit tests and the exact integration contract pass.

- [ ] **Step 9: Run broad offline gates and commit**

Run:

```bash
./scripts/run-host-unit-tests.sh --lib
./scripts/run-host-unit-tests.sh --test integration_contracts
make verus-syscall
cargo fmt --manifest-path tests/host/Cargo.toml --check
rustfmt --edition 2021 --check src/syscall/linux_task_logic_shared.rs src/syscall/linux_task.rs src/syscall/syscall_logic_shared.rs src/syscall/syscall_logic.rs src/syscall/syscall.rs tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
make build-test ARCH=aarch64-unknown-none
git diff --check
```

Expected: every command exits zero, all host tests pass, Verus reports zero
errors, and the AArch64 layout check succeeds.

Commit only the flag rule, syscall implementation, and tests not already
committed:

```bash
git add src/syscall/syscall_logic_shared.rs src/syscall/syscall_logic.rs src/syscall/syscall.rs tests/host/src/lib.rs tests/host/tests/integration_contracts.rs
git commit -m "feat: block AArch64 Linux sleep syscalls"
```

### Task 4: Verify The Commit-Matched AArch64 Runtime

**Files:**
- Generated: `host_shared/posixtest/`
- Generated: `target/posix/aarch64/thread-runtime-quality.json`
- Generated: `target/posix/aarch64/smros-fxfs-sleep-canary.img`
- Generated: `target/posix/aarch64/smros-run-sleep-canary-pthread-kill/`
- Generated: `target/posix/aarch64/smros-fxfs-sleep-five.img`
- Generated: `target/posix/aarch64/smros-run-sleep-five-*/`
- Modify later with complete campaign evidence: `docs/posix/2026-08-03-aarch64-thread-runtime-results.md`

- [ ] **Step 1: Advance the detached clean campaign worktree**

Resolve and validate the implementation commit from the feature worktree:

```bash
feature=/home/steven/workspace/SMROS/.worktrees/posix-runtime-isolation
commit=$(git -C "$feature" rev-parse HEAD)
test "${#commit}" -eq 40
git -C "$feature" status --short --branch
```

Expected: only the protected user-owned Python cache and service changes remain
unstaged in the feature worktree.

In `/home/steven/workspace/SMROS/.worktrees/task10-clean-0a6049a`, require a
clean tracked state and switch the detached worktree to that exact commit:

```bash
feature=/home/steven/workspace/SMROS/.worktrees/posix-runtime-isolation
commit=$(git -C "$feature" rev-parse HEAD)
git status --short --branch
test -z "$(git status --porcelain --untracked-files=no)"
git switch --detach "$commit"
test "$(git rev-parse HEAD)" = "$commit"
```

- [ ] **Step 2: Refresh exact quality evidence**

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
import json
import re
import shutil
import subprocess
from pathlib import Path

commit = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
root = Path("target/posix/aarch64")
root.mkdir(parents=True, exist_ok=True)
checks = []

tarpaulin = shutil.which("cargo-tarpaulin")
coverage_log = root / "thread-runtime-coverage.log"
coverage_artifact = Path("target/coverage/host/tarpaulin-report.html")
if tarpaulin is None:
    checks.append({
        "artifact": None,
        "command": None,
        "coverage_percent": None,
        "findings": None,
        "kind": "coverage",
        "name": "host-rust-coverage",
        "status": "unavailable",
        "summary": "cargo-tarpaulin is not installed",
        "version": None,
    })
else:
    result = subprocess.run(
        ["make", "coverage-host"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        check=False,
    )
    coverage_log.write_text(result.stdout)
    version = " ".join(subprocess.check_output(
        [tarpaulin, "--version"], text=True
    ).split())
    if result.returncode == 0 and coverage_artifact.is_file():
        html = coverage_artifact.read_text(errors="replace")
        match = re.search(r"([0-9]+(?:\.[0-9]+)?)%\s+coverage", html, re.IGNORECASE)
        coverage = None if match is None else float(match.group(1))
        checks.append({
            "artifact": str(coverage_artifact),
            "command": "make coverage-host",
            "coverage_percent": coverage,
            "findings": None,
            "kind": "coverage",
            "name": "host-rust-coverage",
            "status": "passed" if coverage is not None else "failed",
            "summary": (
                "Tarpaulin host coverage completed"
                if coverage is not None
                else "Tarpaulin completed without a parseable coverage percentage"
            ),
            "version": version,
        })
    else:
        lines = [line.strip() for line in result.stdout.splitlines() if line.strip()]
        detail = next(
            (line for line in lines if "E0152" in line or "duplicate lang item" in line),
            lines[-1] if lines else f"exit code {result.returncode}",
        )
        checks.append({
            "artifact": str(coverage_log),
            "command": "make coverage-host",
            "coverage_percent": None,
            "findings": None,
            "kind": "coverage",
            "name": "host-rust-coverage",
            "status": "failed",
            "summary": "make coverage-host failed: " + detail,
            "version": version,
        })

coverity_names = ("cov-build", "cov-analyze", "cov-format-errors")
coverity_tools = [shutil.which(name) for name in coverity_names]
if not all(coverity_tools):
    missing = [
        name for name, tool in zip(coverity_names, coverity_tools) if tool is None
    ]
    checks.append({
        "artifact": None,
        "command": None,
        "coverage_percent": None,
        "findings": None,
        "kind": "static-analysis",
        "name": "coverity",
        "status": "unavailable",
        "summary": "Missing Coverity commands: " + ", ".join(missing),
        "version": None,
    })
else:
    cov_build, cov_analyze, cov_format = coverity_tools
    coverity_dir = Path(f"target/coverity-aarch64-thread-runtime-{commit[:12]}")
    coverity_artifact = root / "thread-runtime-coverity.json"
    coverity_log = root / "thread-runtime-coverity.log"
    if coverity_dir.exists() or coverity_artifact.exists():
        raise FileExistsError("Coverity output must be fresh")
    commands = (
        [cov_build, "--dir", str(coverity_dir), "make", "build-test", "ARCH=aarch64-unknown-none"],
        [cov_analyze, "--dir", str(coverity_dir), "--all"],
        [cov_format, "--dir", str(coverity_dir), "--json-output-v7", str(coverity_artifact)],
    )
    output = []
    failure = None
    for command in commands:
        result = subprocess.run(
            command,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
        )
        output.append(result.stdout)
        if result.returncode != 0:
            failure = (command[0], result.returncode)
            break
    coverity_log.write_text("\n".join(output))
    version = " ".join(subprocess.check_output(
        [cov_build, "--version"], text=True, stderr=subprocess.STDOUT
    ).split())
    if failure is not None:
        checks.append({
            "artifact": str(coverity_log),
            "command": "cov-build; cov-analyze --all; cov-format-errors --json-output-v7",
            "coverage_percent": None,
            "findings": None,
            "kind": "static-analysis",
            "name": "coverity",
            "status": "failed",
            "summary": f"{failure[0]} failed with exit code {failure[1]}",
            "version": version,
        })
    else:
        value = json.loads(coverity_artifact.read_text())
        issues = value.get("issues")
        if not isinstance(issues, list):
            raise ValueError("Coverity JSON does not contain an issues list")
        checks.append({
            "artifact": str(coverity_artifact),
            "command": "cov-build; cov-analyze --all; cov-format-errors --json-output-v7",
            "coverage_percent": None,
            "findings": len(issues),
            "kind": "static-analysis",
            "name": "coverity",
            "status": "passed" if not issues else "failed",
            "summary": f"Coverity analysis completed with {len(issues)} findings",
            "version": version,
        })

evidence = {
    "architecture": "aarch64",
    "checks": checks,
    "schema": 1,
    "smros_commit": commit,
}
path = root / "thread-runtime-quality.json"
path.write_text(json.dumps(evidence, sort_keys=True, separators=(",", ":")) + "\n")
print(path)
PY

PYTHONDONTWRITEBYTECODE=1 python3 -m json.tool \
  target/posix/aarch64/thread-runtime-quality.json >/dev/null
```

Expected in the current environment: Tarpaulin remains `failed` with the
captured E0152 duplicate-lang-item detail, and Coverity remains `unavailable`
with all three missing command names. If the tools behave differently, retain
their actual captured status, coverage, findings, version, and artifact paths.

- [ ] **Step 3: Rebuild and verify the POSIX stage and kernel**

Run in the clean worktree:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli build --arch aarch64 --stage host_shared/posixtest --verify-only
make build-test ARCH=aarch64-unknown-none
```

Expected: 1,979 discovered sources, 1,941 compile passes, 38 compile failures,
1,680 link passes, two link failures, 169 unported shell tests, 1,598 runnable
tests, and an AArch64 kernel whose embedded manifest names the exact
implementation commit.

- [ ] **Step 4: Rerun the previously failing `pthread_kill` canary first**

Create a new private disk and run only the reproduced failure:

```bash
test ! -e target/posix/aarch64/smros-fxfs-sleep-canary.img
test ! -e target/posix/aarch64/smros-run-sleep-canary-pthread-kill
qemu-img create -f raw target/posix/aarch64/smros-fxfs-sleep-canary.img 128M
```

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

test_id = "conformance/interfaces/pthread_kill/1-1.c"
result = run_smros(
    Path("host_shared/posixtest"),
    Path("target/posix/aarch64/smros-run-sleep-canary-pthread-kill"),
    kernel=Path("kernel8.img"),
    disk=Path("target/posix/aarch64/smros-fxfs-sleep-canary.img"),
    memory="1024M",
    test_id=test_id,
)
assert result.complete
assert result.restart_count == 0
assert len(result.attempts) == 1
attempt = result.attempts[0]
assert attempt.test_id == test_id
assert attempt.status == "pass"
assert attempt.pts_status == "pass"
assert attempt.exit_code == 0
assert not attempt.timed_out
assert not attempt.resource_deltas.has_positive()
serial = result.raw_log_path.read_text(errors="replace")
for forbidden in (
    "Kernel panic",
    "Fatal glibc error",
    "failed to map segment",
    "cannot create shared object descriptor",
):
    assert forbidden not in serial
print(test_id, attempt.status, attempt.duration_ms, attempt.resource_deltas.to_dict())
PY
```

Expected: one genuine pass, no timeout or restart, and no positive resource
delta. If the handler still prints but the test fails, stop and inspect the
sleep deadline/wake trace before running more canaries.

- [ ] **Step 5: Run the five thread/signal canaries on another new disk**

```bash
test ! -e target/posix/aarch64/smros-fxfs-sleep-five.img
qemu-img create -f raw target/posix/aarch64/smros-fxfs-sleep-five.img 128M
```

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

tests = (
    "conformance/interfaces/pthread_create/1-1.c",
    "conformance/interfaces/pthread_getspecific/1-1.c",
    "conformance/interfaces/pthread_join/1-1.c",
    "conformance/interfaces/pthread_kill/1-1.c",
    "conformance/interfaces/sigaction/16-1.c",
)
disk = Path("target/posix/aarch64/smros-fxfs-sleep-five.img")
for index, test_id in enumerate(tests, start=1):
    result = run_smros(
        Path("host_shared/posixtest"),
        Path(f"target/posix/aarch64/smros-run-sleep-five-{index}"),
        kernel=Path("kernel8.img"),
        disk=disk,
        memory="1024M",
        test_id=test_id,
    )
    assert result.complete
    assert result.restart_count == 0
    assert len(result.attempts) == 1
    attempt = result.attempts[0]
    assert attempt.test_id == test_id
    assert attempt.status == "pass"
    assert attempt.pts_status == "pass"
    assert attempt.exit_code == 0
    assert not attempt.timed_out
    assert not attempt.resource_deltas.has_positive()
    print(index, test_id, attempt.status, attempt.duration_ms)
PY
```

Expected: all five tests pass on sequential fresh boots using the same private
disk, with one attempt each and zero positive resource deltas.

- [ ] **Step 6: Resume the complete campaign**

Create one new private disk for the `threads` and `signals` groups:

```bash
test ! -e target/posix/aarch64/smros-fxfs-sleep-groups.img
qemu-img create -f raw target/posix/aarch64/smros-fxfs-sleep-groups.img 128M
```

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

results = {}
disk = Path("target/posix/aarch64/smros-fxfs-sleep-groups.img")
for group in ("threads", "signals"):
    result = run_smros(
        Path("host_shared/posixtest"),
        Path(f"target/posix/aarch64/smros-run-sleep-groups-{group}"),
        kernel=Path("kernel8.img"),
        disk=disk,
        memory="1024M",
        group=group,
    )
    assert result.complete
    assert len({attempt.test_id for attempt in result.attempts}) == len(result.attempts)
    assert all(not attempt.resource_deltas.has_positive() for attempt in result.attempts)
    results[group] = result
    print(group, len(result.attempts), sum(a.status == "pass" for a in result.attempts))

sig16 = [
    attempt for attempt in results["signals"].attempts
    if attempt.test_id.startswith("conformance/interfaces/sigaction/16-")
]
assert len(sig16) == 26
assert all(attempt.pts_status == "pass" for attempt in sig16)
assert all("ESRCH" not in (attempt.stdout + attempt.stderr) for attempt in sig16)
PY
```

Create another new private disk and run all 1,598 selected tests:

```bash
test ! -e target/posix/aarch64/smros-fxfs-sleep-all.img
qemu-img create -f raw target/posix/aarch64/smros-fxfs-sleep-all.img 128M
```

```bash
PYTHONDONTWRITEBYTECODE=1 python3 - <<'PY'
from pathlib import Path
from scripts.posix.qemu_runner import run_smros

result = run_smros(
    Path("host_shared/posixtest"),
    Path("target/posix/aarch64/smros-run-sleep-all"),
    kernel=Path("kernel8.img"),
    disk=Path("target/posix/aarch64/smros-fxfs-sleep-all.img"),
    memory="1024M",
)
assert result.complete
assert len(result.attempts) == 1598
assert all(not attempt.resource_deltas.has_positive() for attempt in result.attempts)
serial = result.raw_log_path.read_text(errors="replace")
for forbidden in (
    "Kernel panic",
    "failed to map segment",
    "cannot create shared object descriptor",
):
    assert forbidden not in serial
print(
    "attempts=", len(result.attempts),
    "pass=", sum(a.status == "pass" for a in result.attempts),
    "restarts=", result.restart_count,
)
PY
```

Render the detailed report with quality evidence:

```bash
PYTHONDONTWRITEBYTECODE=1 python3 -m scripts.posix.cli report \
  --manifest host_shared/posixtest/manifest.json \
  --smros-results target/posix/aarch64/smros-run-sleep-all/results.ndjson \
  --quality-evidence target/posix/aarch64/thread-runtime-quality.json \
  --out target/posix/aarch64/report-thread-runtime
```

Write `docs/posix/2026-08-03-aarch64-thread-runtime-results.md` with the exact
implementation commit, suite revision, build ID, manifest/build-results/patch/
results/serial SHA-256 values, offline test counts, canary rows, group and full
totals, API/group/optional-group coverage, non-pass clusters, timeout/restart
counts, maximum resource deltas, Tarpaulin status, Coverity status/findings,
artifact paths, and the next fork/COW/wait root cause. State explicitly that
the result is not overall POSIX conformance.

Commit only the evidence document:

```bash
git add docs/posix/2026-08-03-aarch64-thread-runtime-results.md
git commit -m "docs: record AArch64 thread runtime campaign"
git status --short --branch
```

Expected: JSON, NDJSON, JUnit, CSV, Markdown, and HTML reports agree on all
test/API/group/optional-group and quality totals. Generated stages, images,
logs, coverage, Coverity, and reports remain uncommitted.
