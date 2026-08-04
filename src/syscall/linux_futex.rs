use crate::kernel_lowlevel::thread;
use crate::kernel_objects::scheduler;

use super::linux_task;
use super::linux_task::LinuxBlockReason;
#[cfg(target_arch = "aarch64")]
use super::syscall::linux_user_range_readable;
use super::syscall::{SysError, SysResult};

include!("linux_futex_logic_shared.rs");
include!("linux_runtime_lock_shared.rs");

const LINUX_FUTEX_LIMIT: usize = thread::MAX_THREADS;
const LINUX_FUTEX_TICK_NANOS: u64 = 10_000_000;

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxFutexTimespec {
    seconds: i64,
    nanoseconds: i64,
}

static LINUX_FUTEX_RUNTIME: LinuxRuntimeLock<FutexQueue<LINUX_FUTEX_LIMIT>> =
    LinuxRuntimeLock::new(FutexQueue::new());

fn with_queue<R>(operation: impl FnOnce(&mut FutexQueue<LINUX_FUTEX_LIMIT>) -> R) -> R {
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let mut queue = LINUX_FUTEX_RUNTIME.lock();
    let result = operation(&mut queue);
    drop(queue);
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    result
}

pub(crate) fn sys_futex(
    uaddr: usize,
    op: u32,
    val: u32,
    timeout: usize,
    _uaddr2: usize,
    val3: u32,
) -> SysResult {
    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (uaddr, op, val, timeout, val3);
        return Err(SysError::ENOSYS);
    }

    #[cfg(target_arch = "aarch64")]
    {
        if crate::kernel_lowlevel::smp::current_cpu_id() != 0 {
            return Err(SysError::EINVAL);
        }
        let decoded = decode_futex_op(op).ok_or(SysError::EINVAL)?;
        if uaddr % core::mem::align_of::<u32>() != 0 {
            return Err(SysError::EINVAL);
        }
        if !futex_address_valid(uaddr)
            || !linux_user_range_readable(uaddr, core::mem::size_of::<u32>())
        {
            return Err(SysError::EFAULT);
        }
        match decoded.command {
            FutexCommand::Wait => wait(
                uaddr,
                val,
                timeout,
                FUTEX_BITSET_MATCH_ANY,
                FutexCommand::Wait,
                false,
            ),
            FutexCommand::WaitBitset => wait(
                uaddr,
                val,
                timeout,
                val3,
                FutexCommand::WaitBitset,
                decoded.realtime,
            ),
            FutexCommand::Wake => wake(uaddr, val as usize, FUTEX_BITSET_MATCH_ANY),
            FutexCommand::WakeBitset => wake(uaddr, val as usize, val3),
        }
    }
}

#[cfg(target_arch = "aarch64")]
fn wait(
    uaddr: usize,
    expected: u32,
    timeout_pointer: usize,
    bitset: u32,
    command: FutexCommand,
    realtime: bool,
) -> SysResult {
    if !futex_bitset_valid(bitset) {
        return Err(SysError::EINVAL);
    }

    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    if !linux_user_range_readable(uaddr, core::mem::size_of::<u32>()) {
        crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
        return Err(SysError::EFAULT);
    }
    let observed = unsafe { core::ptr::read(uaddr as *const u32) };
    if !futex_wait_value_matches(observed, expected) {
        crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
        return Err(SysError::EAGAIN);
    }

    let now_monotonic = crate::kernel_lowlevel::timer::get_tick_count();
    let now_realtime = now_monotonic;
    let deadline = match read_deadline(timeout_pointer, now_monotonic, command, realtime) {
        Ok(deadline) => deadline,
        Err(error) => {
            crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
            return Err(error);
        }
    };
    if deadline.is_some_and(|deadline| match deadline.clock {
        FutexClock::Monotonic => deadline.ticks <= now_monotonic,
        FutexClock::Realtime => deadline.ticks <= now_realtime,
    }) {
        crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
        return Err(SysError::ETIMEDOUT);
    }

    let scheduler_thread = scheduler::scheduler().current();
    let tid = match linux_task::current_tid() {
        Ok(tid) => tid,
        Err(error) => {
            crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
            return Err(error);
        }
    };
    let waiter = FutexWaiter {
        address: uaddr,
        bitset,
        tid,
        scheduler_thread: scheduler_thread.0,
        deadline,
        sequence: 0,
        outcome: FutexWaitOutcome::Waiting,
    };
    if with_queue(|queue| queue.push(waiter)).is_err() {
        crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
        return Err(SysError::EAGAIN);
    }
    match linux_task::block_current(LinuxBlockReason::Futex) {
        Ok(task) if task.tid == tid && task.scheduler_thread == scheduler_thread.0 => {}
        Ok(_) | Err(_) => {
            let _ = with_queue(|queue| queue.remove(tid, scheduler_thread.0));
            crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
            return Err(SysError::EAGAIN);
        }
    }

    scheduler::schedule();
    let outcome = with_queue(|queue| queue.take_outcome(tid, scheduler_thread.0));
    if outcome.is_none() {
        let _ = with_queue(|queue| queue.remove(tid, scheduler_thread.0));
        if linux_task::wake_blocked(tid, scheduler_thread.0, LinuxBlockReason::Futex) {
            scheduler::schedule();
        }
        crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
        return Err(SysError::EAGAIN);
    }
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    match outcome {
        Some(FutexWaitOutcome::Woken) => Ok(0),
        Some(FutexWaitOutcome::TimedOut) => Err(SysError::ETIMEDOUT),
        Some(FutexWaitOutcome::Interrupted) => Err(SysError::EINTR),
        Some(FutexWaitOutcome::Waiting) | None => unreachable!(),
    }
}

#[cfg(target_arch = "aarch64")]
fn read_deadline(
    timeout_pointer: usize,
    now_monotonic: u64,
    command: FutexCommand,
    realtime: bool,
) -> Result<Option<FutexDeadline>, SysError> {
    if timeout_pointer == 0 {
        return Ok(None);
    }
    let timeout_size = core::mem::size_of::<LinuxFutexTimespec>();
    if timeout_pointer.checked_add(timeout_size).is_none()
        || !linux_user_range_readable(timeout_pointer, timeout_size)
    {
        return Err(SysError::EFAULT);
    }
    let timeout =
        unsafe { core::ptr::read_unaligned(timeout_pointer as *const LinuxFutexTimespec) };
    futex_deadline_from_timeout(
        command,
        realtime,
        now_monotonic,
        timeout.seconds,
        timeout.nanoseconds,
        LINUX_FUTEX_TICK_NANOS,
    )
    .map(Some)
    .ok_or(SysError::EINVAL)
}

#[cfg(target_arch = "aarch64")]
fn wake(address: usize, requested: usize, bitset: u32) -> SysResult {
    if !futex_bitset_valid(bitset) {
        return Err(SysError::EINVAL);
    }
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let requested = core::cmp::min(requested, LINUX_FUTEX_LIMIT);
    let mut woken = 0usize;
    while woken < requested {
        let identity = with_queue(|queue| queue.wake(address, 1, bitset)[0]);
        let Some((tid, scheduler_thread)) = identity else {
            break;
        };
        if linux_task::wake_blocked(tid, scheduler_thread, LinuxBlockReason::Futex) {
            woken += 1;
        } else {
            let _ = with_queue(|queue| queue.take_outcome(tid, scheduler_thread));
        }
    }
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    Ok(woken)
}

pub(crate) fn on_timer_tick(now_monotonic: u64, now_realtime: u64) {
    #[cfg(target_arch = "aarch64")]
    if crate::kernel_lowlevel::smp::current_cpu_id() == 0 {
        let expired = with_queue(|queue| queue.expire(now_monotonic, now_realtime));
        for identity in expired.into_iter().flatten() {
            let (tid, scheduler_thread) = identity;
            if !linux_task::wake_blocked(tid, scheduler_thread, LinuxBlockReason::Futex) {
                let _ = with_queue(|queue| queue.take_outcome(tid, scheduler_thread));
            }
        }
    }
}

pub(crate) fn interrupt_task(tid: usize, scheduler_thread: usize) -> bool {
    let interrupted = with_queue(|queue| queue.interrupt(tid, scheduler_thread));
    if !interrupted {
        return false;
    }
    if linux_task::wake_blocked(tid, scheduler_thread, LinuxBlockReason::Futex) {
        return true;
    }
    let _ = with_queue(|queue| queue.take_outcome(tid, scheduler_thread));
    false
}

pub(crate) fn remove_task_waiters(tid: usize, scheduler_thread: usize) -> usize {
    with_queue(|queue| queue.remove_task(tid, scheduler_thread))
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn wake_address(address: usize, requested: usize, bitset: u32) -> SysResult {
    wake(address, requested, bitset)
}

pub(crate) fn reset() {
    with_queue(|queue| {
        let _ = queue.reset();
    });
}
