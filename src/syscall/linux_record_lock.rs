use crate::kernel_objects::scheduler;

use super::linux_task;
use super::linux_task::LinuxBlockReason;
use super::syscall::SysError;

include!("linux_record_lock_logic_shared.rs");
include!("linux_runtime_lock_shared.rs");

const LINUX_RECORD_LOCK_LIMIT: usize = 64;
const LINUX_RECORD_LOCK_WAITER_LIMIT: usize = linux_task::LINUX_TASK_LIMIT;

type LinuxRecordLockRuntimeState =
    LinuxRecordLockState<LINUX_RECORD_LOCK_LIMIT, LINUX_RECORD_LOCK_WAITER_LIMIT>;

static LINUX_RECORD_LOCK_RUNTIME: LinuxRuntimeLock<LinuxRecordLockRuntimeState> =
    LinuxRuntimeLock::new(LinuxRecordLockState::new());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxRecordLockRuntimeError {
    Conflict,
    Capacity,
}

fn wake_ready_tasks() {
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let mut runtime = LINUX_RECORD_LOCK_RUNTIME.lock();
    let identities = runtime.wake_ready();
    drop(runtime);
    for (tid, scheduler_thread) in identities.into_iter().flatten() {
        if !linux_task::wake_blocked(tid, scheduler_thread, LinuxBlockReason::RecordLock) {
            let mut runtime = LINUX_RECORD_LOCK_RUNTIME.lock();
            let _ = runtime.take_outcome(tid, scheduler_thread);
        }
    }
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
}

pub(crate) fn first_conflict(
    file_id: u64,
    owner: usize,
    kind: LinuxRecordLockKind,
    range: LinuxRecordLockRange,
) -> Option<LinuxRecordLock> {
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let runtime = LINUX_RECORD_LOCK_RUNTIME.lock();
    let conflict = runtime.locks.first_conflict(file_id, owner, kind, range);
    drop(runtime);
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    conflict
}

pub(crate) fn set_nonblocking(
    file_id: u64,
    owner: usize,
    kind: Option<LinuxRecordLockKind>,
    range: LinuxRecordLockRange,
) -> Result<(), LinuxRecordLockRuntimeError> {
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let mut runtime = LINUX_RECORD_LOCK_RUNTIME.lock();
    let result = match kind {
        Some(kind)
            if runtime
                .locks
                .first_conflict(file_id, owner, kind, range)
                .is_some() =>
        {
            Err(LinuxRecordLockRuntimeError::Conflict)
        }
        Some(kind) => runtime
            .locks
            .set(file_id, owner, kind, range)
            .map_err(|_| LinuxRecordLockRuntimeError::Capacity),
        None => runtime
            .locks
            .unlock(file_id, owner, range)
            .map_err(|_| LinuxRecordLockRuntimeError::Capacity),
    };
    drop(runtime);
    if result.is_ok() {
        wake_ready_tasks();
    }
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    result
}

pub(crate) fn set_blocking(
    file_id: u64,
    owner: usize,
    kind: LinuxRecordLockKind,
    range: LinuxRecordLockRange,
) -> Result<(), SysError> {
    if crate::kernel_lowlevel::smp::current_cpu_id() != 0 {
        return Err(SysError::EINVAL);
    }

    loop {
        let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
        let scheduler_thread = scheduler::scheduler().current();
        let tid = match linux_task::current_tid() {
            Ok(tid) => tid,
            Err(error) => {
                crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
                return Err(error);
            }
        };
        let waiter =
            LinuxRecordLockWaiter::new(file_id, owner, kind, range, tid, scheduler_thread.0);
        let mut runtime = LINUX_RECORD_LOCK_RUNTIME.lock();
        if runtime
            .locks
            .first_conflict(file_id, owner, kind, range)
            .is_none()
        {
            let result = runtime
                .locks
                .set(file_id, owner, kind, range)
                .map_err(|_| SysError::ENOLCK);
            drop(runtime);
            if result.is_ok() {
                wake_ready_tasks();
            }
            crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
            return result;
        }
        if runtime.push(waiter).is_err() {
            drop(runtime);
            crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
            return Err(SysError::ENOLCK);
        }
        drop(runtime);

        match linux_task::block_current(LinuxBlockReason::RecordLock) {
            Ok(task) if task.tid == tid && task.scheduler_thread == scheduler_thread.0 => {}
            Ok(_) | Err(_) => {
                let mut runtime = LINUX_RECORD_LOCK_RUNTIME.lock();
                let _ = runtime.remove_task(tid, scheduler_thread.0);
                drop(runtime);
                crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
                return Err(SysError::EAGAIN);
            }
        }

        scheduler::schedule();
        let mut runtime = LINUX_RECORD_LOCK_RUNTIME.lock();
        let outcome = runtime.take_outcome(tid, scheduler_thread.0);
        drop(runtime);
        match outcome {
            Some(LinuxRecordLockWaitOutcome::Woken) => {
                crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
            }
            Some(LinuxRecordLockWaitOutcome::Interrupted) => {
                crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
                return Err(SysError::EINTR);
            }
            Some(LinuxRecordLockWaitOutcome::Waiting) => unreachable!(),
            None => {
                let mut runtime = LINUX_RECORD_LOCK_RUNTIME.lock();
                let _ = runtime.remove_task(tid, scheduler_thread.0);
                drop(runtime);
                if linux_task::wake_blocked(tid, scheduler_thread.0, LinuxBlockReason::RecordLock) {
                    scheduler::schedule();
                }
                crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
                return Err(SysError::EAGAIN);
            }
        }
    }
}

pub(crate) fn release_owner_file(owner: usize, file_id: u64) {
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let mut runtime = LINUX_RECORD_LOCK_RUNTIME.lock();
    runtime.locks.release_owner_file(owner, file_id);
    drop(runtime);
    wake_ready_tasks();
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
}

pub(crate) fn release_owner(owner: usize) {
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let mut runtime = LINUX_RECORD_LOCK_RUNTIME.lock();
    runtime.locks.release_owner(owner);
    drop(runtime);
    wake_ready_tasks();
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
}

pub(crate) fn interrupt_task(tid: usize, scheduler_thread: usize) -> bool {
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let mut runtime = LINUX_RECORD_LOCK_RUNTIME.lock();
    let interrupted = runtime.interrupt(tid, scheduler_thread);
    drop(runtime);
    if !interrupted {
        crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
        return false;
    }
    if linux_task::wake_blocked(tid, scheduler_thread, LinuxBlockReason::RecordLock) {
        crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
        return true;
    }
    let mut runtime = LINUX_RECORD_LOCK_RUNTIME.lock();
    let _ = runtime.take_outcome(tid, scheduler_thread);
    drop(runtime);
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    false
}

pub(crate) fn remove_task_waiters(tid: usize, scheduler_thread: usize) -> usize {
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let mut runtime = LINUX_RECORD_LOCK_RUNTIME.lock();
    let removed = runtime.remove_task(tid, scheduler_thread);
    drop(runtime);
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    removed
}

pub(crate) fn reset() {
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let mut runtime = LINUX_RECORD_LOCK_RUNTIME.lock();
    runtime.reset();
    drop(runtime);
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
}
