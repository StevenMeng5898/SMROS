use crate::kernel_lowlevel::thread::{self, ThreadId};
use crate::kernel_objects::scheduler;

use super::{linux_task, SysError};

include!("linux_process_logic_shared.rs");
include!("linux_runtime_lock_shared.rs");

pub(crate) const LINUX_PROCESS_LIMIT: usize = thread::MAX_THREADS;

struct LinuxProcessRuntime {
    processes: LinuxProcessTable<LINUX_PROCESS_LIMIT>,
}

impl LinuxProcessRuntime {
    const fn new() -> Self {
        Self {
            processes: LinuxProcessTable::new(),
        }
    }
}

static LINUX_PROCESS_RUNTIME: LinuxRuntimeLock<LinuxProcessRuntime> =
    LinuxRuntimeLock::new(LinuxProcessRuntime::new());

fn with_runtime<R>(operation: impl FnOnce(&mut LinuxProcessRuntime) -> R) -> R {
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let mut runtime = LINUX_PROCESS_RUNTIME.lock();
    let result = operation(&mut runtime);
    drop(runtime);
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    result
}

pub(crate) fn register_root(scheduler_thread: ThreadId) -> Result<usize, SysError> {
    let current = scheduler::scheduler().current();
    if scheduler_thread == ThreadId::IDLE || scheduler_thread != current {
        return Err(SysError::ESRCH);
    }
    with_runtime(|runtime| {
        runtime
            .processes
            .register_root(scheduler_thread.0)
            .map_err(process_error_to_sys_error)
    })
}

pub(crate) fn current() -> Result<LinuxProcessCore, SysError> {
    let task = linux_task::current_task()?;
    with_runtime(|runtime| {
        runtime
            .processes
            .by_pid(task.tgid)
            .filter(|process| process.state == LinuxProcessState::Running)
            .ok_or(SysError::ESRCH)
    })
}

pub(crate) fn current_pid() -> Result<usize, SysError> {
    current().map(|process| process.pid)
}

pub(crate) fn current_parent_pid() -> Result<usize, SysError> {
    current().map(|process| {
        if process.pid == LINUX_ROOT_PID {
            0
        } else {
            process.parent_pid
        }
    })
}

pub(crate) fn reset_launch() {
    with_runtime(|runtime| runtime.processes.reset());
}

fn process_error_to_sys_error(error: LinuxProcessError) -> SysError {
    match error {
        LinuxProcessError::Capacity | LinuxProcessError::Exhausted => SysError::EAGAIN,
        LinuxProcessError::DuplicateRoot => SysError::EBUSY,
        LinuxProcessError::NoSuchParent => SysError::ESRCH,
    }
}
