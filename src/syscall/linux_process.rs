use alloc::vec::Vec;

#[cfg(target_arch = "aarch64")]
use crate::kernel_lowlevel::thread::Aarch64ExceptionFrame;
use crate::kernel_lowlevel::thread::{self, ThreadId};
use crate::kernel_objects::scheduler;

pub(crate) use super::linux_process_memory::{LinuxDescriptorEntry, LinuxOpenDescription};
use super::linux_process_memory::{
    LinuxForkAcquisition, LinuxForkAcquisitionLedger, LinuxForkFailurePoint,
};
#[cfg(target_arch = "aarch64")]
use super::linux_task::LinuxTaskReservation;
use super::{linux_task, SysError};

include!("linux_process_logic_shared.rs");
include!("linux_fork_logic_shared.rs");
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
    process_state: Option<super::LinuxProcessForkState>,
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
        let process_state = self.process_state.take().ok_or(SysError::EAGAIN)?;
        if !super::install_linux_resource_clone(
            child_pid,
            &mut self.descriptors,
            &mut self.objects,
            process_state,
        ) {
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

pub(crate) fn reserve_resource_clone(
    parent_pid: usize,
    namespace_flags: usize,
) -> Result<LinuxResourceClone, SysError> {
    let (descriptors, objects, process_state) =
        super::reserve_linux_resource_clone(parent_pid, namespace_flags)?;
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
        process_state: Some(process_state),
        shared_attachments,
        committed: false,
    })
}

pub(crate) fn release_resources(pid: usize) -> bool {
    super::rollback_linux_fork_process_resources(pid)
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
    context: super::linux_syscall_context::LinuxSyscallFrameRef,
    namespace_flags: usize,
    child_exit_signal: usize,
    parent: Option<LinuxProcessCore>,
    process: Option<LinuxProcessReservation>,
    task: Option<LinuxTaskReservation>,
    scheduler_thread: Option<ThreadId>,
    child_start: Option<Aarch64ProcessStart>,
    resources: Option<LinuxResourceClone>,
    resources_installed: bool,
    memory_pid: Option<usize>,
    root_paddr: Option<u64>,
    publication_interrupt_state: Option<usize>,
}

#[cfg(target_arch = "aarch64")]
impl LinuxForkReservation {
    fn new(
        context: super::linux_syscall_context::LinuxSyscallFrameRef,
        namespace_flags: usize,
        child_exit_signal: usize,
    ) -> Self {
        Self {
            context,
            namespace_flags,
            child_exit_signal,
            parent: None,
            process: None,
            task: None,
            scheduler_thread: None,
            child_start: None,
            resources: None,
            resources_installed: false,
            memory_pid: None,
            root_paddr: None,
            publication_interrupt_state: None,
        }
    }

    fn restore_publication_interrupts(&mut self) {
        if let Some(state) = self.publication_interrupt_state.take() {
            crate::kernel_lowlevel::cpu::restore_interrupts(state);
        }
    }
}

#[cfg(target_arch = "aarch64")]
impl LinuxForkTransactionBackend for LinuxForkReservation {
    type Error = SysError;
    type Output = usize;

    fn injected_failure(&self) -> Self::Error {
        SysError::EAGAIN
    }

    fn acquire_scheduler_thread(&mut self) -> Result<(), Self::Error> {
        let scheduler_thread = scheduler::scheduler()
            .create_suspended_thread_on_cpu(linux_fork_child_entry, "linux_process", 0)
            .ok_or(SysError::EAGAIN)?;
        self.scheduler_thread = Some(scheduler_thread);
        Ok(())
    }

    fn acquire_task(&mut self) -> Result<(), Self::Error> {
        let scheduler_thread = self.scheduler_thread.ok_or(SysError::EAGAIN)?;
        self.parent = Some(current()?);
        self.task = Some(linux_task::reserve_fork_task(scheduler_thread)?);
        Ok(())
    }

    fn acquire_process(&mut self) -> Result<(), Self::Error> {
        let parent = self.parent.ok_or(SysError::EAGAIN)?;
        let scheduler_thread = self.scheduler_thread.ok_or(SysError::EAGAIN)?;
        let task = self.task.ok_or(SysError::EAGAIN)?;
        self.process = Some(with_runtime(|runtime| {
            runtime
                .processes
                .reserve_child_with_pid(
                    parent.pid,
                    scheduler_thread.0,
                    task.tid,
                    self.child_exit_signal,
                )
                .map_err(process_error_to_sys_error)
        })?);
        Ok(())
    }

    fn acquire_resources(&mut self) -> Result<(), Self::Error> {
        let parent = self.parent.ok_or(SysError::EAGAIN)?;
        self.resources = Some(reserve_resource_clone(parent.pid, self.namespace_flags)?);
        Ok(())
    }

    fn acquire_memory(&mut self) -> Result<(), Self::Error> {
        let parent = self.parent.ok_or(SysError::EAGAIN)?;
        let process = self.process.ok_or(SysError::EAGAIN)?;
        let shared_attachments = self
            .resources
            .as_mut()
            .map(LinuxResourceClone::take_shared_attachments)
            .ok_or(SysError::EAGAIN)?;
        self.memory_pid = Some(process.pid);
        self.root_paddr = Some(super::linux_process_memory::clone_for_fork(
            parent.pid,
            process.pid,
            shared_attachments,
        )?);
        Ok(())
    }

    fn configure_child(&mut self) -> Result<(), Self::Error> {
        let process = self.process.ok_or(SysError::EAGAIN)?;
        let scheduler_thread = self.scheduler_thread.ok_or(SysError::EAGAIN)?;
        let root_paddr = self.root_paddr.ok_or(SysError::EAGAIN)?;
        let frame = unsafe { self.context.frame.read() };
        let child_context = prepare_linux_fork_context(
            frame,
            self.context.return_pc,
            self.context.pstate,
            crate::kernel_lowlevel::cpu::read_user_stack_pointer(),
            crate::kernel_lowlevel::cpu::read_user_tls(),
            root_paddr,
            |frame| frame.regs[0] = 0,
        );
        let configured = scheduler::scheduler()
            .get_thread_mut(scheduler_thread)
            .map(|thread| {
                thread.context.set_linux_process_start(
                    child_context.user_sp,
                    child_context.tls,
                    child_context.root_paddr,
                )
            })
            .unwrap_or(false)
            && scheduler::scheduler().bind_thread_process(scheduler_thread, process.pid);
        if !configured {
            return Err(SysError::EAGAIN);
        }
        self.child_start = Some(Aarch64ProcessStart {
            frame: child_context.frame,
            return_pc: child_context.return_pc,
            pstate: child_context.pstate,
            root_paddr: child_context.root_paddr,
        });
        Ok(())
    }

    fn install_resources(&mut self) -> Result<(), Self::Error> {
        let process = self.process.ok_or(SysError::EAGAIN)?;
        let resources = self.resources.take().ok_or(SysError::EAGAIN)?;
        resources.commit(process.pid)?;
        self.resources_installed = true;
        Ok(())
    }

    fn begin_publication(&mut self) -> Result<(), Self::Error> {
        if self.publication_interrupt_state.is_some() {
            return Err(SysError::EAGAIN);
        }
        self.publication_interrupt_state = Some(crate::kernel_lowlevel::cpu::mask_interrupts());
        Ok(())
    }

    fn publish_process(&mut self) -> Result<(), Self::Error> {
        let process = self.process.ok_or(SysError::EAGAIN)?;
        let child_start = self.child_start.ok_or(SysError::EAGAIN)?;
        let process_published = with_runtime(|runtime| {
            let Some(start) = runtime.fork_starts.get_mut(process.slot) else {
                return false;
            };
            if start.is_some() {
                return false;
            }
            *start = Some(child_start);
            if !runtime.processes.publish_fork(process) {
                *start = None;
                return false;
            }
            true
        });
        if process_published {
            Ok(())
        } else {
            Err(SysError::EAGAIN)
        }
    }

    fn publish_task(&mut self) -> Result<(), Self::Error> {
        if linux_task::publish_fork_task(self.task.ok_or(SysError::EAGAIN)?) {
            Ok(())
        } else {
            Err(SysError::EAGAIN)
        }
    }

    fn publish_scheduler_thread(&mut self) -> Result<(), Self::Error> {
        if scheduler::scheduler()
            .publish_suspended_thread(self.scheduler_thread.ok_or(SysError::EAGAIN)?)
        {
            Ok(())
        } else {
            Err(SysError::EAGAIN)
        }
    }

    fn complete_publication(&mut self) -> Result<(), Self::Error> {
        let process = self.process.ok_or(SysError::EAGAIN)?;
        if with_runtime(|runtime| runtime.processes.complete_fork_publish(process)) {
            Ok(())
        } else {
            Err(SysError::EAGAIN)
        }
    }

    fn finish(&mut self) -> Result<Self::Output, Self::Error> {
        let pid = self.process.ok_or(SysError::EAGAIN)?.pid;
        self.restore_publication_interrupts();
        Ok(pid)
    }

    fn rollback(&mut self, acquisition: LinuxForkAcquisition) {
        match acquisition {
            LinuxForkAcquisition::Configured => {
                self.child_start = None;
            }
            LinuxForkAcquisition::Memory => {
                let memory_installed = self.root_paddr.take().is_some();
                if let Some(pid) = self.memory_pid.take() {
                    assert_eq!(
                        super::linux_process_memory::unregister(pid),
                        memory_installed
                    );
                } else {
                    assert!(!memory_installed);
                }
            }
            LinuxForkAcquisition::Resources => {
                if self.resources_installed {
                    if let Some(process) = self.process {
                        assert!(release_resources(process.pid));
                    }
                    self.resources_installed = false;
                } else {
                    drop(self.resources.take());
                }
            }
            LinuxForkAcquisition::Process => {
                if let Some(process) = self.process.take() {
                    let removed = with_runtime(|runtime| {
                        if let Some(start) = runtime.fork_starts.get_mut(process.slot) {
                            *start = None;
                        }
                        if runtime.processes.rollback_fork(process) {
                            return true;
                        }
                        runtime.processes.exit(process.pid, 0)
                            && runtime
                                .processes
                                .reap(process.parent_pid, process.pid)
                                .is_some()
                    });
                    assert!(removed);
                }
            }
            LinuxForkAcquisition::Task => {
                if let Some(task) = self.task.take() {
                    linux_task::rollback_fork_task(task);
                }
            }
            LinuxForkAcquisition::SchedulerThread => {
                if let Some(scheduler_thread) = self.scheduler_thread.take() {
                    assert!(scheduler::scheduler().terminate_thread(scheduler_thread));
                }
                self.restore_publication_interrupts();
            }
        }
    }
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn run_fork_transaction(
    context: super::linux_syscall_context::LinuxSyscallFrameRef,
    namespace_flags: usize,
    child_exit_signal: usize,
) -> Result<usize, SysError> {
    run_linux_fork_transaction(
        LinuxForkReservation::new(context, namespace_flags, child_exit_signal),
        fork_failpoint,
    )
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
