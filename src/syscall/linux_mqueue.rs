use crate::kernel_lowlevel::thread;
use crate::kernel_objects::scheduler;

use super::linux_task;
use super::linux_task::LinuxBlockReason;
use super::syscall::{SysError, SysResult};

include!("linux_mqueue_logic_shared.rs");
include!("linux_runtime_lock_shared.rs");

const LINUX_MQUEUE_LIMIT: usize = 64;
const LINUX_MQUEUE_HANDLE_LIMIT: usize = 256;
const LINUX_MQUEUE_WAITER_LIMIT: usize = thread::MAX_THREADS;

static LINUX_MQUEUE_RUNTIME: LinuxRuntimeLock<
    LinuxMqueueState<LINUX_MQUEUE_LIMIT, LINUX_MQUEUE_HANDLE_LIMIT, LINUX_MQUEUE_WAITER_LIMIT>,
> = LinuxRuntimeLock::new(LinuxMqueueState::new());

fn with_state<R>(
    operation: impl FnOnce(
        &mut LinuxMqueueState<
            LINUX_MQUEUE_LIMIT,
            LINUX_MQUEUE_HANDLE_LIMIT,
            LINUX_MQUEUE_WAITER_LIMIT,
        >,
    ) -> R,
) -> R {
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let mut state = LINUX_MQUEUE_RUNTIME.lock();
    let result = operation(&mut state);
    drop(state);
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    result
}

pub(crate) fn linux_mqueue_error(error: LinuxMqueueError) -> SysError {
    match error {
        LinuxMqueueError::BadDescriptor => SysError::EBADF,
        LinuxMqueueError::Capacity => SysError::ENOMEM,
        LinuxMqueueError::Exists => SysError::EEXIST,
        LinuxMqueueError::Invalid => SysError::EINVAL,
        LinuxMqueueError::MessageTooLarge => SysError::EMSGSIZE,
        LinuxMqueueError::NameTooLong => SysError::ENAMETOOLONG,
        LinuxMqueueError::NotFound => SysError::ENOENT,
        LinuxMqueueError::WouldBlock => SysError::EAGAIN,
        LinuxMqueueError::Busy => SysError::EBUSY,
    }
}

fn linux_mqueue_wait_error(error: LinuxMqueueError) -> SysError {
    match error {
        LinuxMqueueError::Capacity | LinuxMqueueError::Busy => SysError::EAGAIN,
        _ => linux_mqueue_error(error),
    }
}

fn wake_identity(identity: Option<(usize, usize)>) {
    let Some((tid, scheduler_thread)) = identity else {
        return;
    };
    if linux_task::wake_blocked(tid, scheduler_thread, LinuxBlockReason::Mqueue) {
        return;
    }
    let _ = with_state(|state| state.take_outcome(tid, scheduler_thread));
}

pub(crate) fn open_named(
    name: &str,
    handle: u32,
    create: bool,
    exclusive: bool,
    attr: Option<LinuxMqueueAttr>,
) -> Result<LinuxMqueueOpen, SysError> {
    with_state(|state| state.open(name, handle, create, exclusive, attr))
        .map_err(linux_mqueue_error)
}

pub(crate) fn unlink(name: &str) -> SysResult {
    with_state(|state| state.unlink(name)).map_err(linux_mqueue_error)?;
    Ok(0)
}

pub(crate) fn getattr(handle: u32, flags: usize) -> Result<LinuxMqueueAttr, SysError> {
    with_state(|state| state.getattr(handle, flags)).map_err(linux_mqueue_error)
}

pub(crate) fn notify(
    handle: u32,
    notification: Option<LinuxMqueueNotification>,
) -> SysResult {
    with_state(|state| state.notify(handle, notification)).map_err(linux_mqueue_error)?;
    Ok(0)
}

pub(crate) fn send(
    handle: u32,
    bytes: &[u8],
    priority: usize,
) -> Result<LinuxMqueueSendOutcome, SysError> {
    let outcome = with_state(|state| state.send(handle, bytes, priority))
        .map_err(linux_mqueue_error)?;
    wake_identity(outcome.receiver);
    Ok(outcome)
}

pub(crate) fn receive(
    handle: u32,
    buffer_len: usize,
) -> Result<LinuxMqueueReceiveOutcome, SysError> {
    let outcome = with_state(|state| state.receive(handle, buffer_len))
        .map_err(linux_mqueue_error)?;
    wake_identity(outcome.sender);
    Ok(outcome)
}

pub(crate) fn wait(
    handle: u32,
    kind: LinuxMqueueWaitKind,
    deadline: Option<LinuxMqueueDeadline>,
) -> Result<LinuxMqueueWaitOutcome, SysError> {
    #[cfg(target_arch = "aarch64")]
    if crate::kernel_lowlevel::smp::current_cpu_id() != 0 {
        return Err(SysError::EINVAL);
    }

    let now = crate::kernel_lowlevel::timer::get_tick_count();
    if deadline.is_some_and(|deadline| deadline.ticks <= now) {
        return Err(SysError::ETIMEDOUT);
    }

    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let result = (|| {
        let scheduler_thread = scheduler::scheduler().current();
        let tid = linux_task::current_tid()?;
        let ready = with_state(|state| {
            state.push_waiter(handle, kind, tid, scheduler_thread.0, deadline)?;
            state.ready(handle, kind)
        })
        .map_err(linux_mqueue_wait_error)?;
        if ready {
            let _ = with_state(|state| state.remove_task(tid, scheduler_thread.0));
            return Ok(LinuxMqueueWaitOutcome::Woken);
        }
        match linux_task::block_current(LinuxBlockReason::Mqueue) {
            Ok(task) if task.tid == tid && task.scheduler_thread == scheduler_thread.0 => {}
            Ok(_) | Err(_) => {
                let _ = with_state(|state| state.remove_task(tid, scheduler_thread.0));
                return Err(SysError::EAGAIN);
            }
        }

        scheduler::schedule();
        let outcome = with_state(|state| state.take_outcome(tid, scheduler_thread.0));
        if let Some(outcome) = outcome {
            return Ok(outcome);
        }
        let _ = with_state(|state| state.remove_task(tid, scheduler_thread.0));
        if linux_task::wake_blocked(tid, scheduler_thread.0, LinuxBlockReason::Mqueue) {
            scheduler::schedule();
        }
        Err(SysError::EAGAIN)
    })();
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    result
}

pub(crate) fn on_timer_tick(now: u64) {
    #[cfg(target_arch = "aarch64")]
    if crate::kernel_lowlevel::smp::current_cpu_id() == 0 {
        let expired = with_state(|state| state.expire(now));
        for identity in expired.into_iter().flatten() {
            wake_identity(Some(identity));
        }
    }
}

pub(crate) fn interrupt_task(tid: usize, scheduler_thread: usize) -> bool {
    let interrupted = with_state(|state| state.interrupt(tid, scheduler_thread));
    if !interrupted {
        return false;
    }
    if linux_task::wake_blocked(tid, scheduler_thread, LinuxBlockReason::Mqueue) {
        return true;
    }
    let _ = with_state(|state| state.take_outcome(tid, scheduler_thread));
    false
}

pub(crate) fn remove_task_waiters(tid: usize, scheduler_thread: usize) -> usize {
    with_state(|state| state.remove_task(tid, scheduler_thread))
}

pub(crate) fn close_handle(handle: u32) -> bool {
    with_state(|state| state.close_handle(handle))
}

pub(crate) fn reset() {
    with_state(|state| state.reset());
}
