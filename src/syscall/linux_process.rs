use alloc::vec::Vec;

#[cfg(target_arch = "aarch64")]
use crate::kernel_lowlevel::thread::Aarch64ExceptionFrame;
use crate::kernel_lowlevel::thread::{self, ThreadId};
use crate::kernel_objects::scheduler;

pub(crate) use super::linux_process_memory::{LinuxDescriptorEntry, LinuxOpenDescription};
use super::linux_process_memory::{
    LinuxForkAcquisition, LinuxForkAcquisitionLedger, LinuxForkFailurePoint,
};
#[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
use super::linux_task::LinuxTaskReservation;
use super::linux_task::{
    LinuxBlockReason, LinuxPendingSignal, LinuxPendingSignals, LINUX_MAX_SIGNAL,
};
use super::{linux_task, SysError};

include!("linux_process_logic_shared.rs");
include!("linux_fork_logic_shared.rs");
include!("linux_runtime_lock_shared.rs");

pub(crate) const LINUX_PROCESS_LIMIT: usize = thread::MAX_THREADS;
pub(crate) const LINUX_SA_NOCLDWAIT: u64 = 0x0000_0002;
const LINUX_SIG_IGN: u64 = 1;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct LinuxKernelSigaction {
    pub handler: u64,
    pub flags: u64,
    pub restorer: u64,
    pub mask: u64,
}

impl LinuxKernelSigaction {
    pub(crate) const DEFAULT: Self = Self {
        handler: 0,
        flags: 0,
        restorer: 0,
        mask: 0,
    };
}

type LinuxProcessSignalState = LinuxProcessSignalStateCore<
    LinuxKernelSigaction,
    LinuxPendingSignals,
    { LINUX_MAX_SIGNAL + 1 },
>;

const LINUX_PROCESS_SIGNAL_STATE_EMPTY: LinuxProcessSignalState =
    LinuxProcessSignalState::new(LinuxKernelSigaction::DEFAULT, LinuxPendingSignals::new());

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LinuxProcessResourceCounts {
    pub linux_processes: usize,
    pub linux_zombies: usize,
    pub private_pages: usize,
    pub shared_pages: usize,
    pub page_table_pages: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxProcessExitOutcome {
    TaskOnly,
    Descendant,
    LaunchRoot,
}

struct LinuxProcessRuntime {
    processes: LinuxProcessTable<LINUX_PROCESS_LIMIT>,
    signal_states: [LinuxProcessSignalState; LINUX_PROCESS_LIMIT],
    #[cfg(target_arch = "aarch64")]
    fork_starts: [Option<Aarch64ProcessStart>; LINUX_PROCESS_LIMIT],
    #[cfg(target_arch = "riscv64")]
    riscv_fork_starts: [Option<RiscvProcessStart>; LINUX_PROCESS_LIMIT],
}

impl LinuxProcessRuntime {
    const fn new() -> Self {
        Self {
            processes: LinuxProcessTable::new(),
            signal_states: [LINUX_PROCESS_SIGNAL_STATE_EMPTY; LINUX_PROCESS_LIMIT],
            #[cfg(target_arch = "aarch64")]
            fork_starts: [None; LINUX_PROCESS_LIMIT],
            #[cfg(target_arch = "riscv64")]
            riscv_fork_starts: [None; LINUX_PROCESS_LIMIT],
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
        let pid = runtime
            .processes
            .register_root(scheduler_thread.0)
            .map_err(process_error_to_sys_error)?;
        let slot = runtime
            .processes
            .processes
            .iter()
            .position(|process| process.pid == pid && process.state == LinuxProcessState::Running)
            .ok_or(SysError::ESRCH)?;
        runtime.signal_states[slot] = LINUX_PROCESS_SIGNAL_STATE_EMPTY;
        Ok(pid)
    })
}

pub(crate) fn by_pid(pid: usize) -> Option<LinuxProcessCore> {
    with_runtime(|runtime| runtime.processes.by_pid(pid))
}

const LINUX_DISPATCH_PID: usize = 0x1_0000;

pub(crate) fn ensure_dispatch_process(pid: usize, scheduler_thread: usize) -> Result<usize, SysError> {
    with_runtime(|runtime| {
        if runtime
            .processes
            .processes
            .iter()
            .any(|process| process.pid == pid && process.state == LinuxProcessState::Running)
        {
            return Ok(pid);
        }
        let slot = runtime
            .processes
            .processes
            .iter()
            .position(|process| process.state == LinuxProcessState::Empty)
            .ok_or(SysError::EAGAIN)?;
        runtime.processes.processes[slot] = LinuxProcessCore {
            pid,
            parent_pid: 0,
            process_group: pid,
            root_scheduler_thread: scheduler_thread,
            state: LinuxProcessState::Running,
            wait_status: 0,
            exit_signal: 0,
        };
        runtime.signal_states[slot] = LINUX_PROCESS_SIGNAL_STATE_EMPTY;
        Ok(pid)
    })
}

pub(crate) fn ensure_dispatch_identity() -> Result<usize, SysError> {
    if let Ok(pid) = current_pid() {
        return Ok(pid);
    }
    let scheduler_thread = scheduler::scheduler().current().0;
    let pid = match linux_task::current_task() {
        Ok(task) => task.tgid,
        Err(_) => {
            linux_task::ensure_dispatch_task(LINUX_DISPATCH_PID, scheduler_thread)?;
            LINUX_DISPATCH_PID
        }
    };
    ensure_dispatch_process(pid, scheduler_thread)?;
    Ok(pid)
}

fn running_parent_pid(pid: usize) -> Result<usize, SysError> {
    with_runtime(|runtime| {
        runtime
            .processes
            .processes
            .iter()
            .find(|process| process.pid == pid && process.state == LinuxProcessState::Running)
            .map(|process| process.parent_pid)
            .ok_or(SysError::ESRCH)
    })
}

pub(crate) fn pids_in_process_group(process_group: usize) -> Vec<usize> {
    with_runtime(|runtime| {
        runtime
            .processes
            .processes
            .iter()
            .copied()
            .filter(|process| {
                matches!(
                    process.state,
                    LinuxProcessState::Running | LinuxProcessState::Zombie
                ) && process.process_group == process_group
            })
            .map(|process| process.pid)
            .collect()
    })
}

pub(crate) fn visible_pids() -> Vec<usize> {
    with_runtime(|runtime| {
        runtime
            .processes
            .processes
            .iter()
            .copied()
            .filter(|process| {
                matches!(
                    process.state,
                    LinuxProcessState::Running | LinuxProcessState::Zombie
                )
            })
            .map(|process| process.pid)
            .collect()
    })
}

pub(crate) fn set_process_group(pid: usize, process_group: usize) -> Result<(), SysError> {
    if process_group == 0 {
        return Err(SysError::EINVAL);
    }
    with_runtime(|runtime| {
        let process = runtime
            .processes
            .processes
            .iter_mut()
            .find(|process| process.pid == pid && process.state == LinuxProcessState::Running)
            .ok_or(SysError::ESRCH)?;
        process.process_group = process_group;
        Ok(())
    })
}

pub(crate) fn with_signal_state<R>(
    pid: usize,
    operation: impl FnOnce(
        &mut [LinuxKernelSigaction; LINUX_MAX_SIGNAL + 1],
        &mut LinuxPendingSignals,
    ) -> R,
) -> Result<R, SysError> {
    with_runtime(|runtime| {
        let slot = runtime
            .processes
            .processes
            .iter()
            .position(|process| {
                process.pid == pid
                    && matches!(
                        process.state,
                        LinuxProcessState::Reserved
                            | LinuxProcessState::Publishing
                            | LinuxProcessState::Running
                    )
            })
            .ok_or(SysError::ESRCH)?;
        let state = &mut runtime.signal_states[slot];
        Ok(operation(
            &mut state.signal_actions,
            &mut state.process_pending,
        ))
    })
}

pub(crate) fn reset_current_signal_state() -> Result<(), SysError> {
    let pid = current_pid()?;
    with_signal_state(pid, |signal_actions, process_pending| {
        signal_actions.fill(LinuxKernelSigaction::DEFAULT);
        process_pending.reset_in_place();
    })
}

pub(crate) fn clone_signal_state_for_fork(
    parent_pid: usize,
    child_pid: usize,
) -> Result<(), SysError> {
    with_runtime(|runtime| {
        let parent_slot = runtime
            .processes
            .processes
            .iter()
            .position(|process| {
                process.pid == parent_pid && process.state == LinuxProcessState::Running
            })
            .ok_or(SysError::ESRCH)?;
        let child_slot = runtime
            .processes
            .processes
            .iter()
            .position(|process| {
                process.pid == child_pid && process.state == LinuxProcessState::Reserved
            })
            .ok_or(SysError::ESRCH)?;
        runtime.signal_states[child_slot] =
            runtime.signal_states[parent_slot].fork_child(LinuxPendingSignals::new());
        Ok(())
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
    current().map(|process| linux_visible_parent_pid(process.pid, process.parent_pid))
}

#[cfg(target_arch = "riscv64")]
pub(crate) fn switch_riscv_process_address_space(scheduler_thread: usize) {
    let root_pid = super::linux_task::by_scheduler(scheduler_thread).map(|task| task.tgid);
    let root_pid = root_pid.or_else(|| with_runtime(|runtime| {
        runtime
            .processes
            .processes
            .iter()
            .find(|process| {
                process.state == LinuxProcessState::Running
                    && process.root_scheduler_thread == scheduler_thread
            })
            .map(|process| process.pid)
    }));
    // fuzzsc/hermes dispatch a fake Linux pid on the kernel shell thread.
    // RISC-V has one satp; installing that process root unmaps UART/virtio
    // MMIO and hangs after the first schedule. Keep dispatch in bare mode.
    if root_pid == Some(LINUX_DISPATCH_PID) {
        crate::kernel_lowlevel::cpu::switch_user_address_space(0);
        return;
    }
    let root = root_pid
        .and_then(super::linux_process_memory::root_paddr_for_pid)
        .unwrap_or(0);
    crate::kernel_lowlevel::cpu::switch_user_address_space(root);
}

pub(crate) fn wait_current(
    selector: LinuxWaitSelector,
    nohang: bool,
    include_stopped: bool,
) -> Result<LinuxWaitOutcome, SysError> {
    let parent = current()?;
    loop {
        let outcome = with_runtime(|runtime| {
            runtime
                .processes
                .wait_outcome_with_options(parent.pid, selector, include_stopped)
        });
        crate::kobj_debug!(
            "posix-wait",
            "riscv wait probe parent={} selector={:?} outcome={:?} current={}",
            parent.pid,
            selector,
            outcome,
            scheduler::scheduler().current().0
        );
        if outcome != LinuxWaitOutcome::WouldBlock || nohang {
            return Ok(outcome);
        }

        crate::kobj_debug!(
            "posix-wait",
            "block parent={} selector={:?} parent_addr={:#x} selector_addr={:#x}",
            parent.pid,
            selector,
            &parent as *const LinuxProcessCore as usize,
            &selector as *const LinuxWaitSelector as usize
        );
        let blocked = linux_task::block_current(LinuxBlockReason::ChildWait)?;
        let rechecked = with_runtime(|runtime| {
            runtime
                .processes
                .wait_outcome_with_options(parent.pid, selector, include_stopped)
        });
        if rechecked != LinuxWaitOutcome::WouldBlock {
            let _ = linux_task::wake_blocked(
                blocked.tid,
                blocked.scheduler_thread,
                LinuxBlockReason::ChildWait,
            );
            return Ok(rechecked);
        }
        scheduler::schedule();
        crate::kobj_debug!(
            "posix-wait",
            "riscv wait resumed parent={} current={} parent_addr={:#x} selector_addr={:#x}",
            parent.pid,
            scheduler::scheduler().current().0,
            &parent as *const LinuxProcessCore as usize,
            &selector as *const LinuxWaitSelector as usize
        );
    }
}

pub(crate) fn complete_wait_current(
    selector: LinuxWaitSelector,
    pid: usize,
    status: i32,
    wstatus: usize,
    include_stopped: bool,
) -> Result<Option<usize>, SysError> {
    let parent = current()?;
    with_runtime(|runtime| {
        match complete_linux_wait_with_options(
            &mut runtime.processes,
            parent.pid,
            selector,
            pid,
            status,
            include_stopped,
            |status| {
                if wstatus != 0 {
                    super::linux_process_memory::copy_to_process(
                        parent.pid,
                        wstatus,
                        &status.to_ne_bytes(),
                    )?;
                }
                Ok(())
            },
        ) {
            Ok(completed) => Ok(completed),
            Err(LinuxWaitCompletionError::Copy(error)) => Err(error),
            Err(LinuxWaitCompletionError::Reap) => Err(SysError::ECHILD),
        }
    })
}

pub(crate) fn stop_current_child(
    signum: usize,
) -> Result<Option<LinuxChildStateTransition>, SysError> {
    let process = current()?;
    let transition = with_runtime(|runtime| runtime.processes.stop_child(process.pid, signum));
    if let Some(transition) = transition {
        let _ = linux_task::wake_process_waiters(transition.parent_pid);
    }
    Ok(transition)
}

pub(crate) fn continue_child(pid: usize) -> Result<Option<LinuxChildStateTransition>, SysError> {
    let transition = with_runtime(|runtime| runtime.processes.continue_child(pid));
    if let Some(transition) = transition {
        let _ = linux_task::wake_process_waiters(transition.parent_pid);
    }
    Ok(transition)
}

pub(crate) fn exit_current_process(
    wait_status: i32,
    entire_group: bool,
) -> Result<LinuxProcessExitOutcome, SysError> {
    let process = current()?;
    crate::kobj_debug!(
        "posix-exit",
        "begin pid={} parent={} status={:#x} group={}",
        process.pid,
        process.parent_pid,
        wait_status,
        entire_group
    );
    super::linux_process_memory::deactivate_current_address_space()?;
    let process_empty = linux_task::retire_process_tasks(process.pid, entire_group)?;
    crate::kobj_debug!(
        "posix-exit",
        "retired pid={} process_empty={}",
        process.pid,
        process_empty
    );
    if !process_empty {
        return Ok(LinuxProcessExitOutcome::TaskOnly);
    }

    let _ = super::linux_process_memory::unregister(process.pid);
    super::record_linux_child_cpu_usage(process.parent_pid, process.pid);
    let _ = super::release_linux_process_resources(process.pid);
    finish_terminal_process(process.pid, wait_status)
}

fn finish_terminal_process(
    pid: usize,
    wait_status: i32,
) -> Result<LinuxProcessExitOutcome, SysError> {
    crate::kobj_debug!(
        "posix-exit",
        "finish pid={} status={:#x}",
        pid,
        wait_status
    );
    if pid != LINUX_ROOT_PID {
        let transition = with_runtime(|runtime| {
            let process = runtime
                .processes
                .by_pid(pid)
                .filter(|process| process.state == LinuxProcessState::Running)
                .ok_or(SysError::ESRCH)?;
            let parent_pid = process.parent_pid;
            let child_slot = process_slot(runtime, pid)?;
            if parent_pid == LINUX_LAUNCH_REAPER_PID {
                if !runtime.processes.exit(pid, wait_status) {
                    return Err(SysError::ESRCH);
                }
                runtime.signal_states[child_slot] = LINUX_PROCESS_SIGNAL_STATE_EMPTY;
                let _ = runtime.processes.reparent_children_to_launch_reaper(pid);
                return Ok(None);
            }
            let policy = if process.exit_signal == LINUX_SIGCHLD {
                let parent_slot = runtime
                    .processes
                    .processes
                    .iter()
                    .position(|candidate| {
                        candidate.pid == parent_pid && candidate.state == LinuxProcessState::Running
                    })
                    .ok_or(SysError::ESRCH)?;
                let sigchld_action =
                    runtime.signal_states[parent_slot].signal_actions[LINUX_SIGCHLD];
                linux_sigchld_exit_policy(
                    sigchld_action.handler == LINUX_SIG_IGN,
                    sigchld_action.flags & LINUX_SA_NOCLDWAIT != 0,
                )
            } else {
                LinuxSigchldExitPolicy::RetainZombieAndNotify
            };
            let transition = runtime
                .processes
                .terminate_child(pid, wait_status, policy)
                .ok_or(SysError::ESRCH)?;
            runtime.signal_states[child_slot] = LINUX_PROCESS_SIGNAL_STATE_EMPTY;
            let _ = runtime.processes.reparent_children_to_launch_reaper(pid);
            Ok(Some(transition))
        })?;
        if let Some(transition) = transition {
            crate::kobj_debug!(
                "posix-exit",
                "terminal pid={} parent={} notify={:?}",
                pid,
                transition.parent_pid,
                transition.notification_signal
            );
            let _ = apply_linux_terminal_child_transition(
                transition,
                |parent_pid, notification_signal| {
                    super::queue_process_linux_signal_and_wake(
                        parent_pid,
                        LinuxPendingSignal::standard(notification_signal),
                    )
                },
                |parent_pid| {
                    let count = linux_task::wake_process_waiters(parent_pid);
                    crate::kobj_debug!(
                        "posix-wait",
                        "wake parent={} waiters={}",
                        parent_pid,
                        count
                    );
                },
            );
        }
        return Ok(LinuxProcessExitOutcome::Descendant);
    }

    let mut descendant_pids = [0usize; LINUX_PROCESS_LIMIT];
    let descendant_count = with_runtime(|runtime| {
        let slot = process_slot(runtime, pid)?;
        if !runtime.processes.exit(pid, wait_status) {
            return Err(SysError::ESRCH);
        }
        runtime.signal_states[slot] = LINUX_PROCESS_SIGNAL_STATE_EMPTY;
        let _ = runtime.processes.adopt_launch_descendants(LINUX_ROOT_PID);
        let mut count = 0usize;
        for descendant in runtime.processes.processes.iter().copied() {
            if descendant.pid == LINUX_ROOT_PID
                || !LinuxProcessTable::<LINUX_PROCESS_LIMIT>::is_visible(descendant)
            {
                continue;
            }
            descendant_pids[count] = descendant.pid;
            count += 1;
        }
        Ok(count)
    })?;
    linux_task::retire_launch_descendants(LINUX_ROOT_PID);
    for pid in descendant_pids.into_iter().take(descendant_count) {
        let _ = super::linux_process_memory::unregister(pid);
        let _ = super::release_linux_process_resources(pid);
    }
    with_runtime(|runtime| {
        let _ = runtime.processes.reap_launch_descendants();
    });
    Ok(LinuxProcessExitOutcome::LaunchRoot)
}

pub(crate) fn terminate_by_signal(
    tgid: usize,
    signum: usize,
) -> Result<LinuxProcessExitOutcome, SysError> {
    let parent_pid = running_parent_pid(tgid)?;
    if linux_task::current_task().is_ok_and(|task| task.tgid == tgid) {
        super::linux_process_memory::deactivate_current_address_space()?;
    }
    if !linux_task::terminate_process_tasks(tgid) {
        return Err(SysError::ESRCH);
    }
    let _ = super::linux_process_memory::unregister(tgid);
    super::record_linux_child_cpu_usage(parent_pid, tgid);
    let _ = super::release_linux_process_resources(tgid);
    let wait_status = linux_wait_status_signal(signum, false).ok_or(SysError::EINVAL)?;
    finish_terminal_process(tgid, wait_status)
}

fn process_slot(runtime: &LinuxProcessRuntime, pid: usize) -> Result<usize, SysError> {
    runtime
        .processes
        .processes
        .iter()
        .position(|process| process.pid == pid && process.state != LinuxProcessState::Empty)
        .ok_or(SysError::ESRCH)
}

pub(crate) fn resource_counts() -> LinuxProcessResourceCounts {
    loop {
        // Cross-runtime snapshots take the process lock before the memory lock.
        let counts = with_runtime(|runtime| {
            let linux_memory_counts = super::linux_process_memory::resource_counts();
            if !runtime.processes.running_pids_match(
                &linux_memory_counts.process_pids[..linux_memory_counts.process_count],
            ) {
                return None;
            }
            let (linux_processes, linux_zombies) = runtime.processes.resource_counts();
            Some(LinuxProcessResourceCounts {
                linux_processes,
                linux_zombies,
                private_pages: linux_memory_counts.private_pages,
                shared_pages: linux_memory_counts.shared_pages,
                page_table_pages: linux_memory_counts.page_table_pages,
            })
        });
        if let Some(counts) = counts {
            return counts;
        }
        core::hint::spin_loop();
    }
}

pub(crate) fn reset_launch() {
    with_runtime(|runtime| {
        runtime.processes.reset();
        runtime.signal_states.fill(LINUX_PROCESS_SIGNAL_STATE_EMPTY);
        #[cfg(target_arch = "aarch64")]
        runtime.fork_starts.fill(None);
        #[cfg(target_arch = "riscv64")]
        runtime.riscv_fork_starts.fill(None);
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
    pub(crate) fn take_shared_attachments(
        &mut self,
    ) -> Vec<super::linux_process_memory::LinuxSharedAttachmentClone> {
        core::mem::take(&mut self.shared_attachments)
    }

    pub(crate) fn commit(mut self, child_pid: usize) -> Result<(), SysError> {
        let process_state = self.process_state.take().ok_or(SysError::EAGAIN)?;
        crate::kobj_debug!(
            "posix-fork",
            "riscv resource commit begin child={} descriptors={} objects={}",
            child_pid,
            self.descriptors.len(),
            self.objects.len()
        );
        if !super::install_linux_resource_clone(
            child_pid,
            &mut self.descriptors,
            &mut self.objects,
            process_state,
        ) {
            crate::kobj_err!("posix-fork", "riscv resource commit rejected child={}", child_pid);
            return Err(SysError::EBUSY);
        }
        crate::kobj_debug!("posix-fork", "riscv resource commit installed child={}", child_pid);
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

#[cfg(target_arch = "riscv64")]
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub(crate) struct RiscvProcessStart {
    /// The trap vector saves thirty registers plus sepc, user sp, and sstatus.
    pub frame: [u64; 34],
    pub return_pc: u64,
    pub pstate: u64,
    pub user_sp: u64,
    pub root_paddr: u64,
}

#[cfg(target_arch = "riscv64")]
const _: () = {
    assert!(core::mem::offset_of!(RiscvProcessStart, frame) == 0x000);
    assert!(core::mem::offset_of!(RiscvProcessStart, return_pc) == 0x110);
    assert!(core::mem::offset_of!(RiscvProcessStart, pstate) == 0x118);
    assert!(core::mem::offset_of!(RiscvProcessStart, user_sp) == 0x120);
    assert!(core::mem::offset_of!(RiscvProcessStart, root_paddr) == 0x128);
};

#[cfg(target_arch = "aarch64")]
struct Aarch64LinuxForkOps {
    context: super::linux_syscall_context::LinuxSyscallFrameRef,
    namespace_flags: usize,
    child_exit_signal: usize,
    set_child_tid: Option<usize>,
    clear_child_tid: usize,
}

#[cfg(target_arch = "aarch64")]
impl Aarch64LinuxForkOps {
    fn new(
        context: super::linux_syscall_context::LinuxSyscallFrameRef,
        namespace_flags: usize,
        child_exit_signal: usize,
        set_child_tid: Option<usize>,
        clear_child_tid: usize,
    ) -> Self {
        Self {
            context,
            namespace_flags,
            child_exit_signal,
            set_child_tid,
            clear_child_tid,
        }
    }

}

#[cfg(target_arch = "aarch64")]
struct Aarch64LinuxForkMemory {
    pid: usize,
    root_paddr: u64,
    child_tid_write: Option<(usize, u32)>,
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
        let parent = current()?;
        let task = linux_task::reserve_fork_task(*scheduler_thread)?;
        Ok((parent, task))
    }

    fn acquire_process(
        &mut self,
        parent: &Self::Parent,
        scheduler_thread: &Self::SchedulerThread,
        task: &Self::Task,
    ) -> Result<Self::Process, Self::Error> {
        let process = with_runtime(|runtime| {
            runtime
                .processes
                .reserve_child_with_pid(
                    parent.pid,
                    scheduler_thread.0,
                    task.tid,
                    self.child_exit_signal,
                )
                .map_err(process_error_to_sys_error)
        })?;
        if let Err(error) = clone_signal_state_for_fork(parent.pid, process.pid) {
            with_runtime(|runtime| {
                runtime.signal_states[process.slot] = LINUX_PROCESS_SIGNAL_STATE_EMPTY;
                let _ = runtime.processes.rollback(process);
            });
            return Err(error);
        }
        Ok(process)
    }

    fn acquire_resources(&mut self, parent: &Self::Parent) -> Result<Self::Resources, Self::Error> {
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
        let child_tid_write = if let Some(child_tid) = self.set_child_tid {
            let Some(tid) = linux_task::linux_tid_to_user_value(process.pid) else {
                let _ = super::linux_process_memory::unregister(process.pid);
                return Err(SysError::EAGAIN);
            };
            let mut original = [0u8; core::mem::size_of::<u32>()];
            if let Err(error) = super::linux_process_memory::copy_from_process(
                process.pid,
                child_tid,
                &mut original,
            ) {
                let _ = super::linux_process_memory::unregister(process.pid);
                return Err(error);
            }
            if let Err(error) = super::linux_process_memory::copy_to_process(
                process.pid,
                child_tid,
                &tid.to_ne_bytes(),
            ) {
                let _ =
                    super::linux_process_memory::copy_to_process(process.pid, child_tid, &original);
                let _ = super::linux_process_memory::unregister(process.pid);
                return Err(error);
            }
            Some((child_tid, u32::from_ne_bytes(original)))
        } else {
            None
        };
        Ok(Aarch64LinuxForkMemory {
            pid: process.pid,
            root_paddr,
            child_tid_write,
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
        let scheduler_thread = with_runtime(|runtime| {
            let process_record = runtime.processes.processes.get(process.slot)?;
            if process_record.pid == process.pid && process_record.parent_pid == process.parent_pid
            {
                Some(process_record.root_scheduler_thread)
            } else {
                None
            }
        })
        .ok_or(SysError::EAGAIN)?;
        let resources = resources.take().ok_or(SysError::EAGAIN)?;
        resources.commit(process.pid)?;
        if let Err(error) =
            super::apply_linux_resource_scheduler_priority(process.pid, scheduler_thread)
        {
            let _ = super::rollback_linux_fork_process_resources(process.pid);
            return Err(error);
        }
        Ok(())
    }

    fn begin_publication(&mut self) -> Result<Self::Publication, Self::Error> {
        let state = crate::kernel_lowlevel::cpu::mask_interrupts();
        Ok(state)
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
        if linux_task::publish_fork_task(*task, self.clear_child_tid) {
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
        if let Some((address, original)) = memory.child_tid_write {
            assert!(super::linux_process_memory::copy_to_process(
                memory.pid,
                address,
                &original.to_ne_bytes(),
            )
            .is_ok());
        }
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
                runtime.signal_states[process.slot] = LINUX_PROCESS_SIGNAL_STATE_EMPTY;
                return true;
            }
            let removed = runtime.processes.exit(process.pid, 0)
                && runtime
                    .processes
                    .reap(process.parent_pid, process.pid)
                    .is_some();
            if removed {
                runtime.signal_states[process.slot] = LINUX_PROCESS_SIGNAL_STATE_EMPTY;
            }
            removed
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
    set_child_tid: Option<usize>,
    clear_child_tid: usize,
) -> Result<usize, SysError> {
    let result = run_linux_fork_transaction(
        LinuxForkReservation::new(Aarch64LinuxForkOps::new(
            context,
            namespace_flags,
            child_exit_signal,
            set_child_tid,
            clear_child_tid,
        )),
        fork_failpoint,
    );
    result
}

#[cfg(target_arch = "riscv64")]
struct RiscvLinuxForkOps {
    context: super::linux_riscv_syscall_context::LinuxRiscvSyscallFrameRef,
    namespace_flags: usize,
    child_exit_signal: usize,
    set_child_tid: Option<usize>,
    clear_child_tid: usize,
    child_scheduler_thread: Option<ThreadId>,
}

#[cfg(target_arch = "riscv64")]
impl RiscvLinuxForkOps {
    fn new(
        context: super::linux_riscv_syscall_context::LinuxRiscvSyscallFrameRef,
        namespace_flags: usize,
        child_exit_signal: usize,
        set_child_tid: Option<usize>,
        clear_child_tid: usize,
    ) -> Self {
        Self {
            context,
            namespace_flags,
            child_exit_signal,
            set_child_tid,
            clear_child_tid,
            child_scheduler_thread: None,
        }
    }
}

#[cfg(target_arch = "riscv64")]
struct RiscvLinuxForkMemory {
    pid: usize,
    root_paddr: u64,
    child_tid_write: Option<(usize, u32)>,
}

#[cfg(target_arch = "riscv64")]
type RiscvForkReservation = LinuxForkOwnershipCore<RiscvLinuxForkOps>;

#[cfg(target_arch = "riscv64")]
impl LinuxForkOwnershipOps for RiscvLinuxForkOps {
    type Error = SysError;
    type Output = usize;
    type SchedulerThread = ThreadId;
    type Parent = LinuxProcessCore;
    type Task = LinuxTaskReservation;
    type Process = LinuxProcessReservation;
    type Resources = LinuxResourceClone;
    type Memory = RiscvLinuxForkMemory;
    type Configured = RiscvProcessStart;
    type Publication = usize;

    fn injected_failure(&self) -> Self::Error {
        SysError::EAGAIN
    }

    fn acquire_scheduler_thread(&mut self) -> Result<Self::SchedulerThread, Self::Error> {
        scheduler::scheduler()
            .create_suspended_thread_on_cpu(riscv_linux_fork_child_entry, "linux_process", 0)
            .ok_or(SysError::EAGAIN)
    }

    fn acquire_task(
        &mut self,
        scheduler_thread: &Self::SchedulerThread,
    ) -> Result<(Self::Parent, Self::Task), Self::Error> {
        let parent = current()?;
        crate::kobj_debug!(
            "posix-fork",
            "riscv acquire task current={} parent={} child-scheduler={}",
            scheduler::scheduler().current().0,
            parent.pid,
            scheduler_thread.0
        );
        let task = linux_task::reserve_fork_task(*scheduler_thread)?;
        Ok((parent, task))
    }

    fn acquire_process(
        &mut self,
        parent: &Self::Parent,
        scheduler_thread: &Self::SchedulerThread,
        task: &Self::Task,
    ) -> Result<Self::Process, Self::Error> {
        let process = with_runtime(|runtime| {
            runtime
                .processes
                .reserve_child_with_pid(
                    parent.pid,
                    scheduler_thread.0,
                    task.tid,
                    self.child_exit_signal,
                )
                .map_err(process_error_to_sys_error)
        })?;
        if let Err(error) = clone_signal_state_for_fork(parent.pid, process.pid) {
            with_runtime(|runtime| {
                runtime.signal_states[process.slot] = LINUX_PROCESS_SIGNAL_STATE_EMPTY;
                let _ = runtime.processes.rollback(process);
            });
            return Err(error);
        }
        Ok(process)
    }

    fn acquire_resources(&mut self, parent: &Self::Parent) -> Result<Self::Resources, Self::Error> {
        reserve_resource_clone(parent.pid, self.namespace_flags)
    }

    fn acquire_memory(
        &mut self,
        parent: &Self::Parent,
        process: &Self::Process,
        resources: &mut Self::Resources,
    ) -> Result<Self::Memory, Self::Error> {
        crate::kobj_debug!(
            "posix-fork",
            "riscv acquire memory begin current={}",
            scheduler::scheduler().current().0
        );
        let shared_attachments = resources.take_shared_attachments();
        let root_paddr = super::linux_process_memory::clone_for_fork(
            parent.pid,
            process.pid,
            shared_attachments,
        )?;
        crate::kobj_debug!(
            "posix-fork",
            "riscv acquire memory cloned root={:#x} current={}",
            root_paddr,
            scheduler::scheduler().current().0
        );
        let child_tid_write = if let Some(child_tid) = self.set_child_tid {
            let Some(tid) = linux_task::linux_tid_to_user_value(process.pid) else {
                let _ = super::linux_process_memory::unregister(process.pid);
                return Err(SysError::EAGAIN);
            };
            let mut original = [0u8; core::mem::size_of::<u32>()];
            if let Err(error) = super::linux_process_memory::copy_from_process(
                process.pid,
                child_tid,
                &mut original,
            ) {
                let _ = super::linux_process_memory::unregister(process.pid);
                return Err(error);
            }
            if let Err(error) = super::linux_process_memory::copy_to_process(
                process.pid,
                child_tid,
                &tid.to_ne_bytes(),
            ) {
                let _ = super::linux_process_memory::copy_to_process(
                    process.pid,
                    child_tid,
                    &original,
                );
                let _ = super::linux_process_memory::unregister(process.pid);
                return Err(error);
            }
            Some((child_tid, u32::from_ne_bytes(original)))
        } else {
            None
        };
        Ok(RiscvLinuxForkMemory {
            pid: process.pid,
            root_paddr,
            child_tid_write,
        })
    }

    fn configure_child(
        &mut self,
        process: &Self::Process,
        scheduler_thread: &Self::SchedulerThread,
        memory: &Self::Memory,
    ) -> Result<Self::Configured, Self::Error> {
        crate::kobj_debug!(
            "posix-fork",
            "riscv configure begin current={}",
            scheduler::scheduler().current().0
        );
        let mut frame = [0u64; 34];
        unsafe {
            core::ptr::copy_nonoverlapping(self.context.frame, frame.as_mut_ptr(), frame.len());
        }
        // fork returns zero in the child. The trap restore code reads a0 from
        // slot 8 before returning to the instruction after ecall.
        frame[8] = 0;
        let configured = RiscvProcessStart {
            frame,
            return_pc: self.context.return_pc,
            pstate: self.context.pstate,
            user_sp: self.context.user_sp,
            root_paddr: memory.root_paddr,
        };
        let configured_thread = scheduler::scheduler()
            .get_thread_mut(*scheduler_thread)
            .is_some_and(|thread| {
                thread.context.set_entry_stack(
                    riscv_linux_fork_child_entry as *const () as u64,
                    thread.context.sp,
                );
                true
            })
            && scheduler::scheduler().bind_thread_process(*scheduler_thread, process.pid);
        if !configured_thread {
            return Err(SysError::EAGAIN);
        }
        crate::kobj_debug!(
            "posix-fork",
            "riscv configure done current={} child-thread={}",
            scheduler::scheduler().current().0,
            scheduler_thread.0
        );
        self.child_scheduler_thread = Some(*scheduler_thread);
        Ok(configured)
    }

    fn install_resources(
        &mut self,
        process: &Self::Process,
        resources: &mut Option<Self::Resources>,
    ) -> Result<(), Self::Error> {
        crate::kobj_debug!(
            "posix-fork",
            "riscv install resources begin current={}",
            scheduler::scheduler().current().0
        );
        let scheduler_thread = self.child_scheduler_thread.ok_or(SysError::EAGAIN)?.0;
        let resources = resources.take().ok_or(SysError::EAGAIN)?;
        resources.commit(process.pid)?;
        crate::kobj_debug!(
            "posix-fork",
            "riscv install resources committed current={}",
            scheduler::scheduler().current().0
        );
        if let Err(error) = super::apply_linux_resource_scheduler_priority(process.pid, scheduler_thread)
        {
            let _ = super::rollback_linux_fork_process_resources(process.pid);
            return Err(error);
        }
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
        let published = with_runtime(|runtime| {
            let Some(start) = runtime.riscv_fork_starts.get_mut(process.slot) else {
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
        if published { Ok(()) } else { Err(SysError::EAGAIN) }
    }

    fn publish_task(&mut self, task: &Self::Task) -> Result<(), Self::Error> {
        if linux_task::publish_fork_task(*task, self.clear_child_tid) {
            Ok(())
        } else {
            Err(SysError::EAGAIN)
        }
    }

    fn publish_scheduler_thread(
        &mut self,
        scheduler_thread: &Self::SchedulerThread,
    ) -> Result<(), Self::Error> {
        let published = scheduler::scheduler().publish_suspended_thread(*scheduler_thread);
        crate::kobj_debug!(
            "posix-fork",
            "riscv publish scheduler={} published={} state={:?} current={}",
            scheduler_thread.0,
            published,
            scheduler::scheduler()
                .get_thread(*scheduler_thread)
                .map(|thread| thread.state),
            scheduler::scheduler().current().0
        );
        if published {
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
        if let Some((address, original)) = memory.child_tid_write {
            assert!(super::linux_process_memory::copy_to_process(
                memory.pid,
                address,
                &original.to_ne_bytes(),
            )
            .is_ok());
        }
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
            if let Some(start) = runtime.riscv_fork_starts.get_mut(process.slot) {
                *start = None;
            }
            if runtime.processes.rollback_fork(process) {
                runtime.signal_states[process.slot] = LINUX_PROCESS_SIGNAL_STATE_EMPTY;
                return true;
            }
            let removed = runtime.processes.exit(process.pid, 0)
                && runtime.processes.reap(process.parent_pid, process.pid).is_some();
            if removed {
                runtime.signal_states[process.slot] = LINUX_PROCESS_SIGNAL_STATE_EMPTY;
            }
            removed
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

#[cfg(target_arch = "riscv64")]
pub(crate) fn run_riscv_fork_transaction(
    context: super::linux_riscv_syscall_context::LinuxRiscvSyscallFrameRef,
    namespace_flags: usize,
    child_exit_signal: usize,
    set_child_tid: Option<usize>,
    clear_child_tid: usize,
) -> Result<usize, SysError> {
    crate::kobj_debug!("posix-fork", "riscv transaction begin");
    let result = run_linux_fork_transaction(
        RiscvForkReservation::new(RiscvLinuxForkOps::new(
            context,
            namespace_flags,
            child_exit_signal,
            set_child_tid,
            clear_child_tid,
        )),
        fork_failpoint,
    );
    crate::kobj_debug!("posix-fork", "riscv transaction end result={:?}", result);
    result
}

#[cfg(target_arch = "riscv64")]
fn take_riscv_fork_start() -> Option<RiscvProcessStart> {
    with_runtime(|runtime| {
        let scheduler_thread = scheduler::scheduler().current().0;
        let process = runtime.processes.processes.iter().position(|process| {
            process.state == LinuxProcessState::Running
                && process.root_scheduler_thread == scheduler_thread
        })?;
        runtime.riscv_fork_starts[process].take()
    })
}

#[cfg(target_arch = "riscv64")]
pub(crate) extern "C" fn riscv_linux_fork_child_entry() -> ! {
    let Some(start) = take_riscv_fork_start() else {
        crate::kobj_err!("posix-fork", "riscv child start image missing");
        scheduler::scheduler().finish_current_without_stack_free();
        scheduler::schedule();
        loop {
            crate::kernel_lowlevel::cpu::wait_for_interrupt();
        }
    };
    crate::kobj_debug!(
        "posix-fork",
        "riscv child entering user pc={:#x} sp={:#x} ra={:#x} gp={:#x} a0={:#x} a1={:#x}",
        start.return_pc,
        start.user_sp,
        start.frame[0],
        start.frame[1],
        start.frame[8],
        start.frame[9]
    );
    unsafe { thread::start_linux_process_child(&start as *const RiscvProcessStart as *const u8) }
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
