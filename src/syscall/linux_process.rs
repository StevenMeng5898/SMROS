use alloc::vec::Vec;

#[cfg(target_arch = "aarch64")]
use crate::kernel_lowlevel::thread::Aarch64ExceptionFrame;
use crate::kernel_lowlevel::thread::{self, ThreadId};
use crate::kernel_objects::scheduler;

pub(crate) use super::linux_process_memory::{LinuxDescriptorEntry, LinuxOpenDescription};
#[cfg(target_arch = "aarch64")]
use super::linux_task::LinuxTaskReservation;
use super::{linux_task, SysError};

include!("linux_process_logic_shared.rs");
include!("linux_runtime_lock_shared.rs");

pub(crate) const LINUX_PROCESS_LIMIT: usize = thread::MAX_THREADS;

struct LinuxProcessRuntime {
    processes: LinuxProcessTable<LINUX_PROCESS_LIMIT>,
    #[cfg(target_arch = "aarch64")]
    fork_starts: [Option<Aarch64ProcessStart>; LINUX_PROCESS_LIMIT],
}

impl LinuxProcessRuntime {
    const fn new() -> Self {
        Self {
            processes: LinuxProcessTable::new(),
            #[cfg(target_arch = "aarch64")]
            fork_starts: [None; LINUX_PROCESS_LIMIT],
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
    with_runtime(|runtime| {
        runtime.processes.reset();
        #[cfg(target_arch = "aarch64")]
        runtime.fork_starts.fill(None);
    });
}

pub(crate) struct LinuxResourceClone {
    descriptors: Vec<LinuxDescriptorEntry>,
    objects: Vec<u32>,
    shared_attachments: Vec<super::linux_process_memory::LinuxSharedAttachmentClone>,
    committed: bool,
}

impl LinuxResourceClone {
    pub(crate) fn descriptors(&self) -> &[LinuxDescriptorEntry] {
        &self.descriptors
    }

    pub(crate) fn shared_attachments(
        &self,
    ) -> &[super::linux_process_memory::LinuxSharedAttachmentClone] {
        &self.shared_attachments
    }

    pub(crate) fn take_shared_attachments(
        &mut self,
    ) -> Vec<super::linux_process_memory::LinuxSharedAttachmentClone> {
        core::mem::take(&mut self.shared_attachments)
    }

    pub(crate) fn commit(mut self, child_pid: usize) -> Result<(), SysError> {
        if !super::install_linux_resource_clone(child_pid, &mut self.descriptors, &mut self.objects)
        {
            return Err(SysError::EBUSY);
        }
        self.committed = true;
        Ok(())
    }
}

impl Drop for LinuxResourceClone {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        super::linux_process_memory::release_shared_attachments(&self.shared_attachments);
        super::release_linux_resource_clone(&self.descriptors, &self.objects);
    }
}

pub(crate) fn reserve_resource_clone(parent_pid: usize) -> Result<LinuxResourceClone, SysError> {
    let (descriptors, objects) = super::reserve_linux_resource_clone(parent_pid)?;
    let shared_attachments =
        match super::linux_process_memory::reserve_shared_attachments(parent_pid) {
            Ok(attachments) => attachments,
            Err(error) => {
                super::release_linux_resource_clone(&descriptors, &objects);
                return Err(error);
            }
        };
    Ok(LinuxResourceClone {
        descriptors,
        objects,
        shared_attachments,
        committed: false,
    })
}

pub(crate) fn release_resources(pid: usize) -> bool {
    super::release_linux_process_resources(pid)
}

#[cfg(target_arch = "aarch64")]
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub(crate) struct Aarch64ProcessStart {
    pub frame: Aarch64ExceptionFrame,
    pub return_pc: u64,
    pub pstate: u64,
    pub root_paddr: u64,
}

#[cfg(target_arch = "aarch64")]
const _: () = {
    assert!(core::mem::offset_of!(Aarch64ProcessStart, frame) == 0x000);
    assert!(core::mem::offset_of!(Aarch64ProcessStart, return_pc) == 0x310);
    assert!(core::mem::offset_of!(Aarch64ProcessStart, pstate) == 0x318);
    assert!(core::mem::offset_of!(Aarch64ProcessStart, root_paddr) == 0x320);
};

#[cfg(target_arch = "aarch64")]
pub(crate) struct LinuxForkReservation {
    pub process: LinuxProcessReservation,
    pub task: LinuxTaskReservation,
    pub scheduler_thread: ThreadId,
    pub child_start: Aarch64ProcessStart,
    resources: Option<LinuxResourceClone>,
    resources_installed: bool,
    memory_installed: bool,
    namespace_flags: usize,
    published: bool,
}

#[cfg(target_arch = "aarch64")]
impl LinuxForkReservation {
    pub(crate) fn commit(mut self) -> Result<usize, SysError> {
        let resources = self.resources.take().ok_or(SysError::EAGAIN)?;
        resources.commit(self.process.pid)?;
        self.resources_installed = true;
        if !super::inherit_linux_fork_namespace_flags(
            self.process.parent_pid,
            self.process.pid,
            self.namespace_flags,
        ) {
            return Err(SysError::EAGAIN);
        }

        let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
        let publication = (|| {
            let process_published = with_runtime(|runtime| {
                let Some(start) = runtime.fork_starts.get_mut(self.process.slot) else {
                    return false;
                };
                if start.is_some() {
                    return false;
                }
                *start = Some(self.child_start);
                if !runtime.processes.publish_fork(self.process) {
                    *start = None;
                    return false;
                }
                true
            });
            if !process_published {
                return Err(SysError::EAGAIN);
            }
            if !linux_task::publish_fork_task(self.task) {
                return Err(SysError::EAGAIN);
            }
            if !scheduler::scheduler().publish_suspended_thread(self.scheduler_thread) {
                return Err(SysError::EAGAIN);
            }
            if !with_runtime(|runtime| runtime.processes.complete_fork_publish(self.process)) {
                return Err(SysError::EAGAIN);
            }
            Ok(())
        })();

        if let Err(error) = publication {
            drop(self);
            crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
            return Err(error);
        }
        let pid = self.process.pid;
        self.published = true;
        crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
        Ok(pid)
    }
}

#[cfg(target_arch = "aarch64")]
impl Drop for LinuxForkReservation {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        with_runtime(|runtime| {
            if let Some(start) = runtime.fork_starts.get_mut(self.process.slot) {
                *start = None;
            }
        });
        linux_task::rollback_fork_task(self.task);
        if self.resources_installed {
            let _ = release_resources(self.process.pid);
        } else {
            drop(self.resources.take());
        }
        if self.memory_installed {
            let _ = super::linux_process_memory::unregister(self.process.pid);
        }
        let _ = scheduler::scheduler().terminate_thread(self.scheduler_thread);
        with_runtime(|runtime| {
            if !runtime.processes.rollback_fork(self.process)
                && runtime.processes.exit(self.process.pid, 0)
            {
                let _ = runtime
                    .processes
                    .reap(self.process.parent_pid, self.process.pid);
            }
        });
    }
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn reserve_fork(
    scheduler_thread: ThreadId,
    context: super::linux_syscall_context::LinuxSyscallFrameRef,
    namespace_flags: usize,
    child_exit_signal: usize,
) -> Result<LinuxForkReservation, SysError> {
    let parent = match current() {
        Ok(parent) => parent,
        Err(error) => {
            let _ = scheduler::scheduler().terminate_thread(scheduler_thread);
            return Err(error);
        }
    };
    let task = match linux_task::reserve_fork_task(scheduler_thread) {
        Ok(task) => task,
        Err(error) => {
            let _ = scheduler::scheduler().terminate_thread(scheduler_thread);
            return Err(error);
        }
    };
    let process = match with_runtime(|runtime| {
        runtime
            .processes
            .reserve_child_with_pid(parent.pid, scheduler_thread.0, task.tid, child_exit_signal)
            .map_err(process_error_to_sys_error)
    }) {
        Ok(process) => process,
        Err(error) => {
            linux_task::rollback_fork_task(task);
            let _ = scheduler::scheduler().terminate_thread(scheduler_thread);
            return Err(error);
        }
    };

    let mut resources = match reserve_resource_clone(parent.pid) {
        Ok(resources) => resources,
        Err(error) => {
            linux_task::rollback_fork_task(task);
            let _ = scheduler::scheduler().terminate_thread(scheduler_thread);
            with_runtime(|runtime| {
                let _ = runtime.processes.rollback_fork(process);
            });
            return Err(error);
        }
    };
    let shared_attachments = resources.take_shared_attachments();
    let root_paddr = match super::linux_process_memory::clone_for_fork(
        parent.pid,
        process.pid,
        shared_attachments,
    ) {
        Ok(root_paddr) => root_paddr,
        Err(error) => {
            drop(resources);
            linux_task::rollback_fork_task(task);
            let _ = scheduler::scheduler().terminate_thread(scheduler_thread);
            with_runtime(|runtime| {
                let _ = runtime.processes.rollback_fork(process);
            });
            return Err(error);
        }
    };
    let mut frame = unsafe { context.frame.read() };
    frame.regs[0] = 0;
    let user_sp = crate::kernel_lowlevel::cpu::read_user_stack_pointer();
    let tls = crate::kernel_lowlevel::cpu::read_user_tls();
    let configured = scheduler::scheduler()
        .get_thread_mut(scheduler_thread)
        .map(|thread| {
            thread
                .context
                .set_linux_process_start(user_sp, tls, root_paddr)
        })
        .unwrap_or(false)
        && scheduler::scheduler().bind_thread_process(scheduler_thread, process.pid);
    if !configured {
        linux_task::rollback_fork_task(task);
        let _ = super::linux_process_memory::unregister(process.pid);
        drop(resources);
        let _ = scheduler::scheduler().terminate_thread(scheduler_thread);
        with_runtime(|runtime| {
            let _ = runtime.processes.rollback_fork(process);
        });
        return Err(SysError::EAGAIN);
    }

    Ok(LinuxForkReservation {
        process,
        task,
        scheduler_thread,
        child_start: Aarch64ProcessStart {
            frame,
            return_pc: context.return_pc,
            pstate: context.pstate,
            root_paddr,
        },
        resources: Some(resources),
        resources_installed: false,
        memory_installed: true,
        namespace_flags,
        published: false,
    })
}

#[cfg(target_arch = "aarch64")]
fn take_fork_start() -> Option<Aarch64ProcessStart> {
    with_runtime(|runtime| {
        let scheduler_thread = scheduler::scheduler().current().0;
        let process = runtime.processes.processes.iter().position(|process| {
            process.state == LinuxProcessState::Running
                && process.root_scheduler_thread == scheduler_thread
        })?;
        runtime.fork_starts[process].take()
    })
}

#[cfg(target_arch = "aarch64")]
pub(crate) extern "C" fn linux_fork_child_entry() -> ! {
    let Some(start) = take_fork_start() else {
        scheduler::scheduler().finish_current_without_stack_free();
        scheduler::schedule();
        loop {
            crate::kernel_lowlevel::cpu::wait_for_interrupt();
        }
    };
    unsafe { thread::start_linux_process_child(&start as *const Aarch64ProcessStart as *const u8) }
}

fn process_error_to_sys_error(error: LinuxProcessError) -> SysError {
    match error {
        LinuxProcessError::Capacity | LinuxProcessError::Exhausted => SysError::EAGAIN,
        LinuxProcessError::DuplicateRoot => SysError::EBUSY,
        LinuxProcessError::NoSuchParent => SysError::ESRCH,
    }
}
