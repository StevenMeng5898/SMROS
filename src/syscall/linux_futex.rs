use core::cell::UnsafeCell;

use crate::kernel_lowlevel::thread;
use crate::kernel_objects::scheduler;

use super::linux_task;
use super::linux_task::LinuxBlockReason;
use super::syscall::{SysError, SysResult};
use super::syscall_logic;

include!("linux_futex_logic_shared.rs");

const LINUX_FUTEX_LIMIT: usize = thread::MAX_THREADS;
const LINUX_FUTEX_TICK_NANOS: u64 = 10_000_000;

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxFutexTimespec {
    seconds: i64,
    nanoseconds: i64,
}

struct LinuxFutexRuntimeCell(UnsafeCell<FutexQueue<LINUX_FUTEX_LIMIT>>);

// SAFETY: AArch64 Linux tasks and their futex queue are confined to CPU0.
// Every access masks local interrupts before borrowing the queue.
unsafe impl Sync for LinuxFutexRuntimeCell {}

static LINUX_FUTEX_RUNTIME: LinuxFutexRuntimeCell =
    LinuxFutexRuntimeCell(UnsafeCell::new(FutexQueue::new()));

fn queue_mut() -> &'static mut FutexQueue<LINUX_FUTEX_LIMIT> {
    // SAFETY: callers hold the CPU0 interrupt critical section described above.
    unsafe { &mut *LINUX_FUTEX_RUNTIME.0.get() }
}

fn with_queue<R>(operation: impl FnOnce(&mut FutexQueue<LINUX_FUTEX_LIMIT>) -> R) -> R {
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let result = operation(queue_mut());
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
        if !futex_address_valid(uaddr)
            || !syscall_logic::user_buffer_valid(uaddr, core::mem::size_of::<u32>())
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
    let observed = unsafe { core::ptr::read(uaddr as *const u32) };
    if !futex_wait_value_matches(observed, expected) {
        crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
        return Err(SysError::EAGAIN);
    }

    let now = crate::kernel_lowlevel::timer::get_tick_count();
    let deadline = match read_deadline(timeout_pointer, now, command, realtime) {
        Ok(deadline) => deadline,
        Err(error) => {
            crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
            return Err(error);
        }
    };
    if deadline.is_some_and(|deadline| deadline.ticks <= now) {
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
    if queue_mut().push(waiter).is_err() {
        crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
        return Err(SysError::EAGAIN);
    }
    match linux_task::block_current(LinuxBlockReason::Futex) {
        Ok(task) if task.tid == tid && task.scheduler_thread == scheduler_thread.0 => {}
        Ok(_) | Err(_) => {
            let _ = queue_mut().remove(tid, scheduler_thread.0);
            crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
            return Err(SysError::EAGAIN);
        }
    }

    scheduler::schedule();
    let outcome = queue_mut().take_outcome(tid, scheduler_thread.0);
    if outcome.is_none() {
        let _ = queue_mut().remove(tid, scheduler_thread.0);
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
    if !syscall_logic::user_buffer_valid(
        timeout_pointer,
        core::mem::size_of::<LinuxFutexTimespec>(),
    ) {
        return Err(SysError::EFAULT);
    }
    let timeout = unsafe { core::ptr::read(timeout_pointer as *const LinuxFutexTimespec) };
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
        let Some((tid, scheduler_thread)) = queue_mut().wake(address, 1, bitset)[0] else {
            break;
        };
        if linux_task::wake_blocked(tid, scheduler_thread, LinuxBlockReason::Futex) {
            woken += 1;
        } else {
            let _ = queue_mut().take_outcome(tid, scheduler_thread);
        }
    }
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    Ok(woken)
}

pub(crate) fn on_timer_tick(now_monotonic: u64, now_realtime: u64) {
    #[cfg(target_arch = "aarch64")]
    if crate::kernel_lowlevel::smp::current_cpu_id() == 0 {
        with_queue(|queue| {
            for identity in queue
                .expire(now_monotonic, now_realtime)
                .into_iter()
                .flatten()
            {
                let (tid, scheduler_thread) = identity;
                if !linux_task::wake_blocked(tid, scheduler_thread, LinuxBlockReason::Futex) {
                    let _ = queue.take_outcome(tid, scheduler_thread);
                }
            }
        });
    }
}

pub(crate) fn reset() {
    with_queue(|queue| {
        let _ = queue.reset();
    });
}
