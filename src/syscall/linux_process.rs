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
struct Aarch64LinuxForkOps {
    context: super::linux_syscall_context::LinuxSyscallFrameRef,
    namespace_flags: usize,
    child_exit_signal: usize,
}

#[cfg(target_arch = "aarch64")]
impl Aarch64LinuxForkOps {
    fn new(
        context: super::linux_syscall_context::LinuxSyscallFrameRef,
        namespace_flags: usize,
        child_exit_signal: usize,
    ) -> Self {
        Self {
            context,
            namespace_flags,
            child_exit_signal,
        }
    }
}

#[cfg(target_arch = "aarch64")]
struct Aarch64LinuxForkMemory {
    pid: usize,
    root_paddr: u64,
}

#[cfg(target_arch = "aarch64")]
type LinuxForkReservation = LinuxForkOwnershipCore<Aarch64LinuxForkOps>;

#[cfg(target_arch = "aarch64")]
impl LinuxForkOwnershipOps for Aarch64LinuxForkOps {
    type Error = SysError;
    type Output = usize;
    type SchedulerThread = ThreadId;
    type Parent = LinuxProcessCore;
    type Task = LinuxTaskReservation;
    type Process = LinuxProcessReservation;
    type Resources = LinuxResourceClone;
    type Memory = Aarch64LinuxForkMemory;
    type Configured = Aarch64ProcessStart;
    type Publication = usize;

    fn injected_failure(&self) -> Self::Error {
        SysError::EAGAIN
    }

    fn acquire_scheduler_thread(&mut self) -> Result<Self::SchedulerThread, Self::Error> {
        scheduler::scheduler()
            .create_suspended_thread_on_cpu(linux_fork_child_entry, "linux_process", 0)
            .ok_or(SysError::EAGAIN)
    }

    fn acquire_task(
        &mut self,
        scheduler_thread: &Self::SchedulerThread,
    ) -> Result<(Self::Parent, Self::Task), Self::Error> {
        Ok((current()?, linux_task::reserve_fork_task(*scheduler_thread)?))
    }

    fn acquire_process(
        &mut self,
        parent: &Self::Parent,
        scheduler_thread: &Self::SchedulerThread,
        task: &Self::Task,
    ) -> Result<Self::Process, Self::Error> {
        with_runtime(|runtime| {
            runtime
                .processes
                .reserve_child_with_pid(
                    parent.pid,
                    scheduler_thread.0,
                    task.tid,
                    self.child_exit_signal,
                )
                .map_err(process_error_to_sys_error)
        })
    }

    fn acquire_resources(
        &mut self,
        parent: &Self::Parent,
    ) -> Result<Self::Resources, Self::Error> {
        reserve_resource_clone(parent.pid, self.namespace_flags)
    }

    fn acquire_memory(
        &mut self,
        parent: &Self::Parent,
        process: &Self::Process,
        resources: &mut Self::Resources,
    ) -> Result<Self::Memory, Self::Error> {
        let shared_attachments = resources.take_shared_attachments();
        let root_paddr = super::linux_process_memory::clone_for_fork(
            parent.pid,
            process.pid,
            shared_attachments,
        )?;
        Ok(Aarch64LinuxForkMemory {
            pid: process.pid,
            root_paddr,
        })
    }

    fn configure_child(
        &mut self,
        process: &Self::Process,
        scheduler_thread: &Self::SchedulerThread,
        memory: &Self::Memory,
    ) -> Result<Self::Configured, Self::Error> {
        let frame = unsafe { self.context.frame.read() };
        let child_context = prepare_linux_fork_context(
            frame,
            self.context.return_pc,
            self.context.pstate,
            crate::kernel_lowlevel::cpu::read_user_stack_pointer(),
            crate::kernel_lowlevel::cpu::read_user_tls(),
            memory.root_paddr,
            |frame| frame.regs[0] = 0,
        );
        let configured = scheduler::scheduler()
            .get_thread_mut(*scheduler_thread)
            .map(|thread| {
                thread.context.set_linux_process_start(
                    child_context.user_sp,
                    child_context.tls,
                    child_context.root_paddr,
                )
            })
            .unwrap_or(false)
            && scheduler::scheduler().bind_thread_process(*scheduler_thread, process.pid);
        if !configured {
            return Err(SysError::EAGAIN);
        }
        Ok(Aarch64ProcessStart {
            frame: child_context.frame,
            return_pc: child_context.return_pc,
            pstate: child_context.pstate,
            root_paddr: child_context.root_paddr,
        })
    }

    fn install_resources(
        &mut self,
        process: &Self::Process,
        resources: &mut Option<Self::Resources>,
    ) -> Result<(), Self::Error> {
        let resources = resources.take().ok_or(SysError::EAGAIN)?;
        resources.commit(process.pid)?;
        Ok(())
    }

    fn begin_publication(&mut self) -> Result<Self::Publication, Self::Error> {
        Ok(crate::kernel_lowlevel::cpu::mask_interrupts())
    }

    fn publish_process(
        &mut self,
        process: &Self::Process,
        configured: &Self::Configured,
    ) -> Result<(), Self::Error> {
        let process_published = with_runtime(|runtime| {
            let Some(start) = runtime.fork_starts.get_mut(process.slot) else {
                return false;
            };
            if start.is_some() {
                return false;
            }
            *start = Some(*configured);
            if !runtime.processes.publish_fork(*process) {
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

    fn publish_task(&mut self, task: &Self::Task) -> Result<(), Self::Error> {
        if linux_task::publish_fork_task(*task) {
            Ok(())
        } else {
            Err(SysError::EAGAIN)
        }
    }

    fn publish_scheduler_thread(
        &mut self,
        scheduler_thread: &Self::SchedulerThread,
    ) -> Result<(), Self::Error> {
        if scheduler::scheduler().publish_suspended_thread(*scheduler_thread) {
            Ok(())
        } else {
            Err(SysError::EAGAIN)
        }
    }

    fn complete_publication(&mut self, process: &Self::Process) -> Result<(), Self::Error> {
        if with_runtime(|runtime| runtime.processes.complete_fork_publish(*process)) {
            Ok(())
        } else {
            Err(SysError::EAGAIN)
        }
    }

    fn finish(
        &mut self,
        process: &Self::Process,
        _configured: &Self::Configured,
    ) -> Result<Self::Output, Self::Error> {
        Ok(process.pid)
    }

    fn restore_publication(&mut self, publication: Self::Publication) {
        crate::kernel_lowlevel::cpu::restore_interrupts(publication);
    }

    fn rollback_configured(&mut self, _configured: Self::Configured) {}

    fn rollback_memory(&mut self, memory: Self::Memory) {
        assert!(super::linux_process_memory::unregister(memory.pid));
    }

    fn rollback_reserved_resources(&mut self, resources: Self::Resources) {
        drop(resources);
    }

    fn rollback_installed_resources(&mut self, process: &Self::Process) {
        assert!(release_resources(process.pid));
    }

    fn rollback_process(&mut self, process: Self::Process) {
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

    fn rollback_task(&mut self, task: Self::Task) {
        linux_task::rollback_fork_task(task);
    }

    fn rollback_scheduler_thread(&mut self, scheduler_thread: Self::SchedulerThread) {
        assert!(scheduler::scheduler().terminate_thread(scheduler_thread));
    }
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn run_fork_transaction(
    context: super::linux_syscall_context::LinuxSyscallFrameRef,
    namespace_flags: usize,
    child_exit_signal: usize,
) -> Result<usize, SysError> {
    run_linux_fork_transaction(
        LinuxForkReservation::new(Aarch64LinuxForkOps::new(
            context,
            namespace_flags,
            child_exit_signal,
        )),
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
