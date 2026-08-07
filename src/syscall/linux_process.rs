use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};

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
static LINUX_FORK_FAILURE_POINT: AtomicUsize = AtomicUsize::new(LinuxForkFailurePoint::COUNT);
static LINUX_FORK_FAILURE_OCCURRENCE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn configure_fork_failure(point: LinuxForkFailurePoint, occurrence: usize) {
    LINUX_FORK_FAILURE_OCCURRENCE.store(occurrence, Ordering::SeqCst);
    LINUX_FORK_FAILURE_POINT.store(point as usize, Ordering::SeqCst);
}

pub(crate) fn clear_fork_failure() {
    LINUX_FORK_FAILURE_POINT.store(LinuxForkFailurePoint::COUNT, Ordering::SeqCst);
    LINUX_FORK_FAILURE_OCCURRENCE.store(0, Ordering::SeqCst);
}

pub(crate) fn fork_failpoint(point: LinuxForkFailurePoint) -> bool {
    if LINUX_FORK_FAILURE_POINT.load(Ordering::SeqCst) != point as usize {
        return false;
    }
    let remaining = LINUX_FORK_FAILURE_OCCURRENCE.load(Ordering::SeqCst);
    if remaining != 0 {
        LINUX_FORK_FAILURE_OCCURRENCE.store(remaining - 1, Ordering::SeqCst);
        return false;
    }
    clear_fork_failure();
    true
}

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
    process: Option<LinuxProcessReservation>,
    task: Option<LinuxTaskReservation>,
    scheduler_thread: Option<ThreadId>,
    child_start: Option<Aarch64ProcessStart>,
    resources: Option<LinuxResourceClone>,
    resources_installed: bool,
    memory_pid: Option<usize>,
    ledger: LinuxForkAcquisitionLedger,
    published: bool,
}

#[cfg(target_arch = "aarch64")]
impl LinuxForkReservation {
    fn new(scheduler_thread: ThreadId) -> Self {
        let mut ledger = LinuxForkAcquisitionLedger::new();
        debug_assert!(ledger.acquire(LinuxForkAcquisition::SchedulerThread));
        Self {
            process: None,
            task: None,
            scheduler_thread: Some(scheduler_thread),
            child_start: None,
            resources: None,
            resources_installed: false,
            memory_pid: None,
            ledger,
            published: false,
        }
    }

    fn acquire_task(&mut self, task: LinuxTaskReservation) -> Result<(), SysError> {
        if self.task.is_some() || !self.ledger.acquire(LinuxForkAcquisition::Task) {
            return Err(SysError::EAGAIN);
        }
        self.task = Some(task);
        Ok(())
    }

    fn acquire_process(&mut self, process: LinuxProcessReservation) -> Result<(), SysError> {
        if self.process.is_some() || !self.ledger.acquire(LinuxForkAcquisition::Process) {
            return Err(SysError::EAGAIN);
        }
        self.process = Some(process);
        Ok(())
    }

    fn acquire_resources(&mut self, resources: LinuxResourceClone) -> Result<(), SysError> {
        if self.resources.is_some() || !self.ledger.acquire(LinuxForkAcquisition::Resources) {
            return Err(SysError::EAGAIN);
        }
        self.resources = Some(resources);
        Ok(())
    }

    fn take_shared_attachments(
        &mut self,
    ) -> Result<Vec<super::linux_process_memory::LinuxSharedAttachmentClone>, SysError> {
        self.resources
            .as_mut()
            .map(LinuxResourceClone::take_shared_attachments)
            .ok_or(SysError::EAGAIN)
    }

    fn acquire_memory(&mut self, pid: usize) -> Result<(), SysError> {
        if self.memory_pid.is_some() || !self.ledger.acquire(LinuxForkAcquisition::Memory) {
            return Err(SysError::EAGAIN);
        }
        self.memory_pid = Some(pid);
        Ok(())
    }

    fn acquire_configured(&mut self, child_start: Aarch64ProcessStart) -> Result<(), SysError> {
        if self.child_start.is_some() || !self.ledger.acquire(LinuxForkAcquisition::Configured) {
            return Err(SysError::EAGAIN);
        }
        self.child_start = Some(child_start);
        Ok(())
    }

    pub(crate) fn commit(mut self) -> Result<usize, SysError> {
        let process = self.process.ok_or(SysError::EAGAIN)?;
        let task = self.task.ok_or(SysError::EAGAIN)?;
        let scheduler_thread = self.scheduler_thread.ok_or(SysError::EAGAIN)?;
        let child_start = self.child_start.ok_or(SysError::EAGAIN)?;
        let resources = self.resources.take().ok_or(SysError::EAGAIN)?;
        resources.commit(process.pid)?;
        self.resources_installed = true;

        let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
        let publication = (|| {
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
            if !process_published {
                return Err(SysError::EAGAIN);
            }
            if fork_failpoint(LinuxForkFailurePoint::ProcessPublication) {
                return Err(SysError::EAGAIN);
            }
            if !linux_task::publish_fork_task(task) {
                return Err(SysError::EAGAIN);
            }
            if fork_failpoint(LinuxForkFailurePoint::TaskPublication) {
                return Err(SysError::EAGAIN);
            }
            if !scheduler::scheduler().publish_suspended_thread(scheduler_thread) {
                return Err(SysError::EAGAIN);
            }
            if fork_failpoint(LinuxForkFailurePoint::SchedulerPublication) {
                return Err(SysError::EAGAIN);
            }
            if !with_runtime(|runtime| runtime.processes.complete_fork_publish(process)) {
                return Err(SysError::EAGAIN);
            }
            Ok(())
        })();

        if let Err(error) = publication {
            drop(self);
            crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
            return Err(error);
        }
        let pid = process.pid;
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
        if self.child_start.take().is_some() {
            debug_assert!(self.ledger.release(LinuxForkAcquisition::Configured));
        }
        if let Some(pid) = self.memory_pid.take() {
            let _ = super::linux_process_memory::unregister(pid);
            debug_assert!(self.ledger.release(LinuxForkAcquisition::Memory));
        }
        let had_resources = self.resources_installed || self.resources.is_some();
        if self.resources_installed {
            if let Some(process) = self.process {
                let _ = release_resources(process.pid);
            }
        } else {
            drop(self.resources.take());
        }
        if had_resources {
            debug_assert!(self.ledger.release(LinuxForkAcquisition::Resources));
        }
        if let Some(process) = self.process.take() {
            with_runtime(|runtime| {
                if let Some(start) = runtime.fork_starts.get_mut(process.slot) {
                    *start = None;
                }
                if !runtime.processes.rollback_fork(process)
                    && runtime.processes.exit(process.pid, 0)
                {
                    let _ = runtime.processes.reap(process.parent_pid, process.pid);
                }
            });
            debug_assert!(self.ledger.release(LinuxForkAcquisition::Process));
        }
        if let Some(task) = self.task.take() {
            linux_task::rollback_fork_task(task);
            debug_assert!(self.ledger.release(LinuxForkAcquisition::Task));
        }
        if let Some(scheduler_thread) = self.scheduler_thread.take() {
            let _ = scheduler::scheduler().terminate_thread(scheduler_thread);
            debug_assert!(self.ledger.release(LinuxForkAcquisition::SchedulerThread));
        }
        debug_assert!(self.ledger.is_empty());
    }
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn reserve_fork(
    scheduler_thread: ThreadId,
    context: super::linux_syscall_context::LinuxSyscallFrameRef,
    namespace_flags: usize,
    child_exit_signal: usize,
) -> Result<LinuxForkReservation, SysError> {
    let mut reservation = LinuxForkReservation::new(scheduler_thread);
    if fork_failpoint(LinuxForkFailurePoint::SchedulerThread) {
        return Err(SysError::EAGAIN);
    }
    let parent = current()?;
    let task = linux_task::reserve_fork_task(scheduler_thread)?;
    reservation.acquire_task(task)?;
    if fork_failpoint(LinuxForkFailurePoint::Task) {
        return Err(SysError::EAGAIN);
    }
    let process = with_runtime(|runtime| {
        runtime
            .processes
            .reserve_child_with_pid(parent.pid, scheduler_thread.0, task.tid, child_exit_signal)
            .map_err(process_error_to_sys_error)
    })?;
    reservation.acquire_process(process)?;
    if fork_failpoint(LinuxForkFailurePoint::Process) {
        return Err(SysError::EAGAIN);
    }

    let resources = reserve_resource_clone(parent.pid, namespace_flags)?;
    reservation.acquire_resources(resources)?;
    let shared_attachments = reservation.take_shared_attachments()?;
    reservation.acquire_memory(process.pid)?;
    let root_paddr =
        super::linux_process_memory::clone_for_fork(parent.pid, process.pid, shared_attachments)?;
    if fork_failpoint(LinuxForkFailurePoint::Memory) {
        return Err(SysError::EAGAIN);
    }
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
        return Err(SysError::EAGAIN);
    }
    reservation.acquire_configured(Aarch64ProcessStart {
        frame,
        return_pc: context.return_pc,
        pstate: context.pstate,
        root_paddr,
    })?;
    if fork_failpoint(LinuxForkFailurePoint::Configured) {
        return Err(SysError::EAGAIN);
    }
    Ok(reservation)
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
