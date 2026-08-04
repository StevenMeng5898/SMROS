use core::cell::UnsafeCell;

use crate::kernel_lowlevel::thread::{self, ThreadId};
use crate::kernel_objects::scheduler;

use super::SysError;

include!("linux_task_logic_shared.rs");

const LINUX_TASK_LIMIT: usize = thread::MAX_THREADS;

struct LinuxTaskRuntime {
    tasks: LinuxTaskTable<LINUX_TASK_LIMIT>,
}

impl LinuxTaskRuntime {
    const fn new() -> Self {
        Self {
            tasks: LinuxTaskTable::new(),
        }
    }
}

struct LinuxTaskRuntimeCell(UnsafeCell<LinuxTaskRuntime>);

// SAFETY: every access is serialized with local interrupts masked. Linux tasks
// are pinned to CPU0 during the AArch64 thread-runtime milestone.
unsafe impl Sync for LinuxTaskRuntimeCell {}

static LINUX_TASK_RUNTIME: LinuxTaskRuntimeCell =
    LinuxTaskRuntimeCell(UnsafeCell::new(LinuxTaskRuntime::new()));

fn with_runtime<R>(operation: impl FnOnce(&mut LinuxTaskRuntime) -> R) -> R {
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    // SAFETY: local interrupts are masked and Linux task mutations are confined
    // to CPU0, so no concurrent reference to the runtime can exist.
    let result = operation(unsafe { &mut *LINUX_TASK_RUNTIME.0.get() });
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    result
}

pub(crate) fn register_root(scheduler_thread: ThreadId) -> Result<usize, SysError> {
    with_runtime(|runtime| {
        let current = scheduler::scheduler().current();
        if scheduler_thread == ThreadId::IDLE || scheduler_thread != current {
            return Err(SysError::ESRCH);
        }
        runtime
            .tasks
            .register_root(scheduler_thread.0)
            .map_err(|error| match error {
                LinuxTaskError::DuplicateRoot => SysError::EBUSY,
                LinuxTaskError::Capacity | LinuxTaskError::Exhausted => SysError::EAGAIN,
                LinuxTaskError::InvalidTransition => SysError::EINVAL,
            })
    })
}

pub(crate) fn current_tid() -> Result<usize, SysError> {
    with_runtime(|runtime| {
        let scheduler_thread = scheduler::scheduler().current();
        runtime
            .tasks
            .by_scheduler(scheduler_thread.0)
            .map(|task| task.tid)
            .ok_or(SysError::ESRCH)
    })
}

pub(crate) fn current_tgid() -> Result<usize, SysError> {
    with_runtime(|runtime| {
        let scheduler_thread = scheduler::scheduler().current();
        runtime
            .tasks
            .by_scheduler(scheduler_thread.0)
            .map(|task| task.tgid)
            .ok_or(SysError::ESRCH)
    })
}

pub(crate) fn lookup_tid(tid: usize) -> Option<LinuxTaskCore> {
    with_runtime(|runtime| runtime.tasks.by_tid(tid))
}

pub(crate) fn reset() {
    with_runtime(|runtime| {
        let current = scheduler::scheduler().current();
        for scheduler_id in 1..thread::MAX_THREADS {
            if scheduler_id != current.0 && runtime.tasks.by_scheduler(scheduler_id).is_some() {
                let _ = scheduler::scheduler().terminate_thread(ThreadId(scheduler_id));
            }
        }
        runtime.tasks.reset();
    });
}
