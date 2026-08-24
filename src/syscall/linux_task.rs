use alloc::vec::Vec;

use crate::kernel_lowlevel::thread::{self, ThreadId};
use crate::kernel_objects::scheduler;

use super::SysError;

include!("linux_task_logic_shared.rs");

include!("linux_runtime_lock_shared.rs");

pub(crate) const LINUX_TASK_LIMIT: usize = thread::MAX_THREADS;

struct LinuxTaskRuntime {
    tasks: LinuxTaskTable<LINUX_TASK_LIMIT>,
    #[cfg(target_arch = "aarch64")]
    clone_slots: [aarch64_clone::LinuxCloneSlot; LINUX_TASK_LIMIT],
}

impl LinuxTaskRuntime {
    const fn new() -> Self {
        Self {
            tasks: LinuxTaskTable::new(),
            #[cfg(target_arch = "aarch64")]
            clone_slots: [aarch64_clone::LinuxCloneSlot::EMPTY; LINUX_TASK_LIMIT],
        }
    }
}

static LINUX_TASK_RUNTIME: LinuxRuntimeLock<LinuxTaskRuntime> =
    LinuxRuntimeLock::new(LinuxTaskRuntime::new());

fn with_runtime<R>(operation: impl FnOnce(&mut LinuxTaskRuntime) -> R) -> R {
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let mut runtime = LINUX_TASK_RUNTIME.lock();
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
    let process_pid = super::linux_process::register_root(scheduler_thread)?;
    let task = with_runtime(|runtime| {
        runtime
            .tasks
            .register_root(scheduler_thread.0)
            .map_err(|error| match error {
                LinuxTaskError::DuplicateRoot => SysError::EBUSY,
                LinuxTaskError::Capacity | LinuxTaskError::Exhausted => SysError::EAGAIN,
                LinuxTaskError::InvalidTransition => SysError::EINVAL,
            })
    });
    match task {
        Ok(tid) if tid == process_pid => Ok(tid),
        Ok(_) => {
            super::linux_process::reset_launch();
            Err(SysError::EINVAL)
        }
        Err(error) => {
            super::linux_process::reset_launch();
            Err(error)
        }
    }
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

pub(crate) fn current_task() -> Result<LinuxTaskCore, SysError> {
    with_runtime(|runtime| {
        let scheduler_thread = scheduler::scheduler().current();
        runtime
            .tasks
            .by_scheduler(scheduler_thread.0)
            .ok_or(SysError::ESRCH)
    })
}

pub(crate) fn by_tid(tid: usize) -> Option<LinuxTaskCore> {
    with_runtime(|runtime| runtime.tasks.by_tid(tid))
}

pub(crate) fn sched_param(
    tid: usize,
    scheduler_thread: usize,
) -> Option<LinuxTaskSchedParam> {
    with_runtime(|runtime| runtime.tasks.sched_param(tid, scheduler_thread))
}

pub(crate) fn set_sched_param(
    tid: usize,
    scheduler_thread: usize,
    param: LinuxTaskSchedParam,
) -> bool {
    with_runtime(|runtime| runtime.tasks.set_sched_param(tid, scheduler_thread, param))
}

pub(crate) fn set_current_clear_child_tid(address: usize) -> Result<usize, SysError> {
    with_runtime(|runtime| {
        let scheduler_thread = scheduler::scheduler().current();
        let task = runtime
            .tasks
            .by_scheduler(scheduler_thread.0)
            .ok_or(SysError::ESRCH)?;
        if !runtime
            .tasks
            .set_clear_child_tid(task.tid, scheduler_thread.0, address)
        {
            return Err(SysError::ESRCH);
        }
        Ok(task.tid)
    })
}

pub(crate) fn with_current_signal_state<R>(
    operation: impl FnOnce(&mut LinuxTaskSignalState) -> R,
) -> Result<R, SysError> {
    with_runtime(|runtime| {
        let scheduler_thread = scheduler::scheduler().current();
        let (_, signal_state, _) = runtime
            .tasks
            .signal_state_by_scheduler_mut(scheduler_thread.0)
            .ok_or(SysError::ESRCH)?;
        Ok(operation(signal_state))
    })
}

pub(crate) fn peek_current_matching_signal(
    wait_mask: u64,
) -> Result<Option<LinuxPendingSignal>, SysError> {
    with_current_signal_state(|signal_state| signal_state.peek_matching(wait_mask))
}

pub(crate) fn take_current_matching_signal(
    wait_mask: u64,
) -> Result<Option<(LinuxPendingSignal, LinuxPendingSignalReservation)>, SysError> {
    with_current_signal_state(|signal_state| signal_state.pending.take_matching_reserved(wait_mask))
}

pub(crate) fn take_current_unblocked_signal(
) -> Result<Option<(LinuxPendingSignal, LinuxPendingSignalReservation)>, SysError> {
    with_current_signal_state(|signal_state| signal_state.take_unblocked_reserved())
}

pub(crate) fn current_matching_signum(wait_mask: u64) -> Result<Option<usize>, SysError> {
    with_current_signal_state(|signal_state| signal_state.matching_signum(wait_mask))
}

pub(crate) fn install_current_signal_wait(
    wait: LinuxSignalWait,
    replacement_mask: Option<u64>,
) -> Result<bool, SysError> {
    with_current_signal_state(|signal_state| {
        let previous_mask = signal_state.mask;
        if let Some(mask) = replacement_mask {
            signal_state.mask = mask;
        }
        if signal_state.install_signal_wait(wait) {
            return true;
        }
        signal_state.mask = previous_mask;
        false
    })
}

pub(crate) fn interrupt_current_signal_suspend(signum: usize) -> Result<bool, SysError> {
    with_current_signal_state(|signal_state| signal_state.interrupt_signal_suspend(signum))
}

pub(crate) fn take_current_signal_wait_outcome() -> Result<Option<LinuxSignalWait>, SysError> {
    with_current_signal_state(|signal_state| signal_state.take_signal_wait_outcome())
}

pub(crate) fn cancel_current_signal_wait() -> Result<bool, SysError> {
    with_current_signal_state(|signal_state| signal_state.cancel_signal_wait())
}

pub(crate) fn install_current_sleep(wait: LinuxSleepWait) -> Result<bool, SysError> {
    with_runtime(|runtime| {
        let scheduler_thread = scheduler::scheduler().current();
        let task = runtime
            .tasks
            .by_scheduler(scheduler_thread.0)
            .ok_or(SysError::ESRCH)?;
        Ok(runtime
            .tasks
            .install_sleep(task.tid, scheduler_thread.0, wait))
    })
}

pub(crate) fn take_current_sleep_outcome() -> Result<Option<LinuxSleepWait>, SysError> {
    with_runtime(|runtime| {
        let scheduler_thread = scheduler::scheduler().current();
        let task = runtime
            .tasks
            .by_scheduler(scheduler_thread.0)
            .ok_or(SysError::ESRCH)?;
        Ok(runtime
            .tasks
            .take_sleep_outcome(task.tid, scheduler_thread.0))
    })
}

pub(crate) fn cancel_current_sleep() -> Result<bool, SysError> {
    with_runtime(|runtime| {
        let scheduler_thread = scheduler::scheduler().current();
        let task = runtime
            .tasks
            .by_scheduler(scheduler_thread.0)
            .ok_or(SysError::ESRCH)?;
        Ok(runtime.tasks.cancel_sleep(task.tid, scheduler_thread.0))
    })
}

pub(crate) fn cancel_sleep(tid: usize, scheduler_thread: usize) -> bool {
    with_runtime(|runtime| runtime.tasks.cancel_sleep(tid, scheduler_thread))
}

pub(crate) fn refresh_realtime_sleep_deadlines(
    now: u64,
    realtime_offset_nanoseconds: i64,
    tick_nanoseconds: u64,
) -> [Option<(usize, usize, LinuxBlockReason)>; LINUX_TASK_LIMIT] {
    with_runtime(|runtime| {
        runtime.tasks.refresh_realtime_sleep_deadlines(
            now,
            realtime_offset_nanoseconds,
            tick_nanoseconds,
        )
    })
}

pub(crate) fn interrupt_sleep(tid: usize, scheduler_thread: usize, signum: usize) -> bool {
    with_runtime(|runtime| runtime.tasks.interrupt_sleep(tid, scheduler_thread, signum))
}

pub(crate) fn install_current_restart_block(restart: LinuxRestartBlock) -> Result<bool, SysError> {
    with_current_signal_state(|signal_state| signal_state.install_restart_block(restart))
}

pub(crate) fn set_current_restart_timeout(timeout: LinuxRestartTimeout) -> Result<bool, SysError> {
    with_current_signal_state(|signal_state| signal_state.set_restart_timeout(timeout))
}

pub(crate) fn current_restart_timeout() -> Option<LinuxRestartTimeout> {
    with_current_signal_state(|signal_state| signal_state.restart_timeout())
        .ok()
        .flatten()
}

pub(crate) fn clear_current_restart_block() -> bool {
    with_current_signal_state(|signal_state| signal_state.clear_restart_block()).unwrap_or(false)
}

pub(crate) fn queue_task_signal(
    tgid: Option<usize>,
    tid: usize,
    record: LinuxPendingSignal,
) -> Result<LinuxTaskCore, SysError> {
    with_runtime(|runtime| {
        runtime
            .tasks
            .route_signal(tgid, tid, record)
            .map_err(|error| match error {
                LinuxSignalRouteError::NoSuchTask => SysError::ESRCH,
                LinuxSignalRouteError::InvalidSignal => SysError::EINVAL,
                LinuxSignalRouteError::InvalidReservation => SysError::EINVAL,
                LinuxSignalRouteError::QueueFull => SysError::EAGAIN,
            })
    })
}

pub(crate) fn route_signal_and_complete_wait(
    tgid: Option<usize>,
    tid: usize,
    record: LinuxPendingSignal,
) -> Result<(LinuxTaskCore, Option<LinuxBlockReason>), SysError> {
    with_runtime(|runtime| {
        runtime
            .tasks
            .route_signal_and_complete_wait(tgid, tid, record)
            .map_err(|error| match error {
                LinuxSignalRouteError::NoSuchTask => SysError::ESRCH,
                LinuxSignalRouteError::InvalidSignal => SysError::EINVAL,
                LinuxSignalRouteError::InvalidReservation => SysError::EINVAL,
                LinuxSignalRouteError::QueueFull => SysError::EAGAIN,
            })
    })
}

pub(crate) fn signal_wait_target(tgid: usize, signum: usize) -> Option<LinuxTaskCore> {
    with_runtime(|runtime| runtime.tasks.signal_wait_target_for(tgid, signum))
}

pub(crate) fn handoff_process_pending_signal(
    tgid: usize,
    pending: &mut LinuxPendingSignals,
) -> Result<Option<(LinuxTaskCore, LinuxBlockReason)>, LinuxSignalRouteError> {
    with_runtime(|runtime| {
        runtime
            .tasks
            .handoff_process_pending_signal_for(tgid, pending)
    })
}

pub(crate) fn complete_process_signal_wait(
    tid: usize,
    scheduler_thread: usize,
    record: LinuxPendingSignal,
    reservation: LinuxPendingSignalReservation,
) -> Option<LinuxBlockReason> {
    with_runtime(|runtime| {
        runtime
            .tasks
            .complete_process_signal_wait(tid, scheduler_thread, record, reservation)
    })
}

pub(crate) fn interrupt_process_signal_wait(
    tid: usize,
    scheduler_thread: usize,
    signum: usize,
) -> Option<LinuxBlockReason> {
    with_runtime(|runtime| {
        runtime
            .tasks
            .interrupt_process_signal_wait(tid, scheduler_thread, signum)
    })
}

pub(crate) fn process_signal_target(tgid: usize, signum: usize) -> Option<LinuxTaskCore> {
    with_runtime(|runtime| runtime.tasks.process_signal_target_for(tgid, signum))
}

impl<const N: usize> LinuxTaskTable<N> {
    fn signal_wait_target_for(&self, tgid: usize, signum: usize) -> Option<LinuxTaskCore> {
        let matching =
            self.tasks
                .iter()
                .zip(self.signal_states.iter())
                .find_map(|(task, signal_state)| {
                    (Self::is_live(*task)
                        && task.tgid == tgid
                        && matches!(
                            task.block_reason,
                            LinuxBlockReason::SignalWait | LinuxBlockReason::SignalSuspend
                        )
                        && signal_state.signal_wait_accepts(signum))
                    .then_some(*task)
                });
        matching.or_else(|| {
            self.tasks
                .iter()
                .zip(self.signal_states.iter())
                .find_map(|(task, signal_state)| {
                    (Self::is_live(*task)
                        && task.tgid == tgid
                        && task.block_reason == LinuxBlockReason::SignalWait
                        && signal_state.timed_wait_interrupted_by(signum))
                    .then_some(*task)
                })
        })
    }

    fn accepting_signal_wait_target_for(
        &self,
        tgid: usize,
        signum: usize,
    ) -> Option<LinuxTaskCore> {
        self.tasks
            .iter()
            .zip(self.signal_states.iter())
            .find_map(|(task, signal_state)| {
                (Self::is_live(*task)
                    && task.tgid == tgid
                    && task.block_reason == LinuxBlockReason::SignalWait
                    && signal_state.signal_wait_accepts(signum))
                .then_some(*task)
            })
    }

    fn handoff_process_pending_signal_for(
        &mut self,
        tgid: usize,
        pending: &mut LinuxPendingSignals,
    ) -> Result<Option<(LinuxTaskCore, LinuxBlockReason)>, LinuxSignalRouteError> {
        let Some(record) = pending.peek_eligible(|signum| {
            self.accepting_signal_wait_target_for(tgid, signum)
                .is_some()
        }) else {
            return Ok(None);
        };
        let Some(target) = self.accepting_signal_wait_target_for(tgid, record.signum) else {
            return Ok(None);
        };
        let Some((record, reservation)) =
            pending.take_matching_reserved(linux_signal_bit(record.signum))
        else {
            return Ok(None);
        };
        let Some(reason) = self.complete_process_signal_wait(
            target.tid,
            target.scheduler_thread,
            record,
            reservation,
        ) else {
            pending.rollback_reservation(reservation, record)?;
            return Ok(None);
        };
        Ok(Some((target, reason)))
    }

    fn process_signal_target_for(&self, tgid: usize, signum: usize) -> Option<LinuxTaskCore> {
        super::linux_process::select_linux_process_signal_target(
            &self.tasks,
            tgid,
            linux_signal_bit(signum),
            |task| Self::is_live(task),
            |task| task.tgid,
            |slot| self.signal_states[slot].mask,
        )
    }
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn reserve_fork_task(scheduler_id: ThreadId) -> Result<LinuxTaskReservation, SysError> {
    with_runtime(|runtime| {
        let current = scheduler::scheduler().current();
        let parent_slot = runtime
            .tasks
            .tasks
            .iter()
            .position(|task| {
                task.state == LinuxTaskState::Runnable && task.scheduler_thread == current.0
            })
            .ok_or(SysError::ESRCH)?;
        if scheduler_id == ThreadId::IDLE
            || scheduler_id == current
            || scheduler::scheduler()
                .get_thread(scheduler_id)
                .map(|thread| thread.state)
                != Some(thread::ThreadState::Blocked)
        {
            return Err(SysError::EAGAIN);
        }

        let parent_mask = runtime.tasks.signal_states[parent_slot].mask;
        let reservation = runtime
            .tasks
            .reserve_child(0, scheduler_id.0)
            .ok_or(SysError::EAGAIN)?;
        let task = &mut runtime.tasks.tasks[reservation.slot];
        task.tgid = reservation.tid;
        super::linux_process::prepare_linux_fork_task_signal_state(
            &mut runtime.tasks.signal_states[reservation.slot],
            parent_mask,
            |signal_state| signal_state.reset_in_place(),
            |signal_state, mask| signal_state.mask = mask,
        );
        Ok(reservation)
    })
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn publish_fork_task(reservation: LinuxTaskReservation, clear_child_tid: usize) -> bool {
    with_runtime(|runtime| {
        scheduler::scheduler()
            .get_thread(ThreadId(reservation.scheduler_thread))
            .map(|thread| thread.state)
            == Some(thread::ThreadState::Blocked)
            && runtime.tasks.publish(reservation)
            && runtime.tasks.set_clear_child_tid(
                reservation.tid,
                reservation.scheduler_thread,
                clear_child_tid,
            )
    })
}

#[cfg(target_arch = "aarch64")]
pub(crate) fn rollback_fork_task(reservation: LinuxTaskReservation) {
    with_runtime(|runtime| {
        if runtime.tasks.rollback(reservation) {
            return;
        }
        if runtime
            .tasks
            .exit(reservation.tid, reservation.scheduler_thread)
        {
            let _ = runtime
                .tasks
                .retire(reservation.tid, reservation.scheduler_thread);
        }
    });
}

pub(crate) fn discard_signal(tgid: usize, signum: usize) {
    with_runtime(|runtime| {
        for (task, signal_state) in runtime
            .tasks
            .tasks
            .iter()
            .zip(runtime.tasks.signal_states.iter_mut())
        {
            if LinuxTaskTable::<LINUX_TASK_LIMIT>::is_live(*task) && task.tgid == tgid {
                signal_state.discard(signum);
            }
        }
    });
}

pub(crate) fn block_current(reason: LinuxBlockReason) -> Result<LinuxTaskCore, SysError> {
    if reason == LinuxBlockReason::None {
        return Err(SysError::EINVAL);
    }
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let result = (|| {
        let scheduler_thread = scheduler::scheduler().current();
        let task = with_runtime(|runtime| {
            let task = runtime
                .tasks
                .by_scheduler(scheduler_thread.0)
                .ok_or(SysError::ESRCH)?;
            if !runtime.tasks.block(task.tid, scheduler_thread.0, reason) {
                return Err(SysError::EAGAIN);
            }
            Ok(task)
        })?;
        if scheduler::scheduler().block_thread(scheduler_thread) {
            return Ok(task);
        }
        let _ = with_runtime(|runtime| runtime.tasks.wake(task.tid, scheduler_thread.0));
        Err(SysError::EAGAIN)
    })();
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    result
}

pub(crate) fn wake_blocked(tid: usize, scheduler_thread: usize, reason: LinuxBlockReason) -> bool {
    if reason == LinuxBlockReason::None {
        return false;
    }
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let result = (|| {
        let scheduler_id = ThreadId(scheduler_thread);
        if scheduler::scheduler()
            .get_thread(scheduler_id)
            .map(|thread| thread.state)
            != Some(thread::ThreadState::Blocked)
        {
            return false;
        }
        let woken = with_runtime(|runtime| {
            let Some(task) = runtime.tasks.by_tid(tid) else {
                return false;
            };
            if task.scheduler_thread != scheduler_thread
                || task.state != LinuxTaskState::Blocked
                || task.block_reason != reason
            {
                return false;
            }
            runtime.tasks.wake(tid, scheduler_thread)
        });
        if !woken {
            return false;
        }
        if scheduler::scheduler().wake_thread(scheduler_id) {
            return true;
        }
        let _ = with_runtime(|runtime| runtime.tasks.block(tid, scheduler_thread, reason));
        false
    })();
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    result
}

pub(crate) fn wake_process_waiters(tgid: usize) -> usize {
    let waiters = with_runtime(|runtime| runtime.tasks.child_waiters(tgid));
    let count = waiters
        .into_iter()
        .filter(|task| wake_blocked(task.tid, task.scheduler_thread, LinuxBlockReason::ChildWait))
        .count();
    if count != 0 {
        crate::kobj_info!(
            "posix-wait",
            "wake-process-waiters parent={} count={}",
            tgid,
            count
        );
    }
    count
}

#[derive(Clone, Copy)]
enum LinuxTaskRetirementScope {
    Process {
        tgid: usize,
        entire_group: bool,
        current_scheduler: usize,
    },
    LaunchDescendants {
        root_tgid: usize,
    },
}

fn retire_tasks(
    scope: LinuxTaskRetirementScope,
) -> (Vec<LinuxChildExitTransition>, usize, bool) {
    let mut retired = Vec::new();
    let (retired_count, process_empty) = with_runtime(|runtime| {
        let candidates: Vec<(usize, LinuxTaskCore)> = runtime
            .tasks
            .tasks
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, task)| match scope {
                LinuxTaskRetirementScope::Process {
                    tgid,
                    entire_group,
                    current_scheduler,
                } => {
                    LinuxTaskTable::<LINUX_TASK_LIMIT>::is_live(*task)
                        && task.tgid == tgid
                        && (entire_group || task.scheduler_thread == current_scheduler)
                }
                LinuxTaskRetirementScope::LaunchDescendants { root_tgid } => {
                    task.tgid != root_tgid
                        && LinuxTaskTable::<LINUX_TASK_LIMIT>::is_live(*task)
                }
            })
            .collect();
        let mut retire_task = |slot: usize, task: LinuxTaskCore| {
            let Some(clear_child_tid) = runtime
                .tasks
                .exit_with_clear_child_tid(task.tid, task.scheduler_thread)
            else {
                return false;
            };
            if !runtime.tasks.retire(task.tid, task.scheduler_thread) {
                return false;
            }
            #[cfg(target_arch = "aarch64")]
            if let Some(clone_slot) = runtime.clone_slots.get_mut(slot) {
                *clone_slot = aarch64_clone::LinuxCloneSlot::EMPTY;
            }
            retired.push(LinuxChildExitTransition {
                task,
                slot,
                clear_child_tid,
                disposition: LinuxChildExitDisposition::ScheduleWithoutEl0Return,
            });
            true
        };
        let mut retired_count = 0usize;
        for (slot, task) in candidates {
            if retire_task(slot, task) {
                retired_count += 1;
            }
        }
        let process_empty = match scope {
            LinuxTaskRetirementScope::Process { tgid, .. } => {
                !runtime.tasks.tasks.iter().copied().any(|task| {
                    task.tgid == tgid && LinuxTaskTable::<LINUX_TASK_LIMIT>::is_live(task)
                })
            }
            LinuxTaskRetirementScope::LaunchDescendants { .. } => true,
        };
        (retired_count, process_empty)
    });
    (retired, retired_count, process_empty)
}

fn complete_task_retirements(
    retired: Vec<LinuxChildExitTransition>,
    _retired_count: usize,
    current_scheduler: ThreadId,
) {
    for transition in retired {
        let _ = super::linux_futex::remove_task_waiters(
            transition.task.tid,
            transition.task.scheduler_thread,
        );
        let _ = super::linux_mqueue::remove_task_waiters(
            transition.task.tid,
            transition.task.scheduler_thread,
        );
        let _ = super::linux_record_lock::remove_task_waiters(
            transition.task.tid,
            transition.task.scheduler_thread,
        );
        if transition.clear_child_tid != 0
            && super::linux_process_memory::copy_to_process(
                transition.task.tgid,
                transition.clear_child_tid,
                &0u32.to_ne_bytes(),
            )
            .is_ok()
        {
            let _ = super::linux_futex::wake_address(
                transition.clear_child_tid,
                1,
                super::linux_futex::FUTEX_BITSET_MATCH_ANY,
            );
        }
        if transition.task.scheduler_thread != current_scheduler.0 {
            let _ = scheduler::scheduler()
                .terminate_thread(ThreadId(transition.task.scheduler_thread));
        }
    }
}

pub(crate) fn retire_process_tasks(tgid: usize, entire_group: bool) -> Result<bool, SysError> {
    let current_scheduler = scheduler::scheduler().current();
    let current = current_task()?;
    if current.tgid != tgid {
        return Err(SysError::ESRCH);
    }

    let (retired, retired_count, process_empty) = retire_tasks(LinuxTaskRetirementScope::Process {
        tgid,
        entire_group,
        current_scheduler: current_scheduler.0,
    });
    complete_task_retirements(retired, retired_count, current_scheduler);
    Ok(process_empty)
}

pub(crate) fn terminate_process_tasks(tgid: usize) -> bool {
    let current_scheduler = scheduler::scheduler().current();
    let (retired, retired_count, process_empty) = retire_tasks(LinuxTaskRetirementScope::Process {
        tgid,
        entire_group: true,
        current_scheduler: current_scheduler.0,
    });
    complete_task_retirements(retired, retired_count, current_scheduler);
    process_empty && retired_count != 0
}

pub(crate) fn retire_launch_descendants(root_tgid: usize) {
    let current_scheduler = scheduler::scheduler().current();
    let (retired, retired_count, _) =
        retire_tasks(LinuxTaskRetirementScope::LaunchDescendants { root_tgid });
    complete_task_retirements(retired, retired_count, current_scheduler);
}

pub(crate) fn finish_current_without_el0_return() -> ! {
    scheduler::scheduler().finish_current_without_stack_free();
    scheduler::schedule();
    loop {
        crate::kernel_lowlevel::cpu::wait_for_interrupt();
        scheduler::schedule();
    }
}

pub(crate) fn on_timer_tick(now: u64) {
    #[cfg(target_arch = "aarch64")]
    if crate::kernel_lowlevel::smp::current_cpu_id() == 0 {
        while let Some(identity) = with_runtime(|runtime| runtime.tasks.expire_one_signal_wait(now))
        {
            let (tid, scheduler_thread, reason) = identity;
            if !wake_blocked(tid, scheduler_thread, reason) {
                let _ = with_runtime(|runtime| {
                    runtime
                        .tasks
                        .signal_state_mut(tid, scheduler_thread)
                        .and_then(|state| state.take_signal_wait_outcome())
                });
            }
        }

        while let Some(identity) = with_runtime(|runtime| runtime.tasks.expire_one_sleep(now)) {
            let (tid, scheduler_thread, reason) = identity;
            if !wake_blocked(tid, scheduler_thread, reason) {
                let _ = cancel_sleep(tid, scheduler_thread);
            }
        }
    }
}

pub(crate) fn reset() {
    with_runtime(|runtime| {
        let current = scheduler::scheduler().current();
        for slot in 0..LINUX_TASK_LIMIT {
            if let Some(scheduler_id) = runtime.tasks.scheduler_thread_for_reset(slot) {
                if scheduler_id != current.0 {
                    let _ = scheduler::scheduler().terminate_thread(ThreadId(scheduler_id));
                }
            }
        }
        #[cfg(target_arch = "aarch64")]
        runtime
            .clone_slots
            .fill(aarch64_clone::LinuxCloneSlot::EMPTY);
        runtime.tasks.reset();
    });
    super::linux_process::reset_launch();
    #[cfg(target_arch = "aarch64")]
    super::linux_syscall_context::reset();
}

#[cfg(target_arch = "aarch64")]
mod aarch64_clone {
    use crate::kernel_lowlevel::thread::{Aarch64ExceptionFrame, ThreadState};

    use super::*;
    use crate::syscall::linux_syscall_context::LinuxSyscallFrameRef;

    #[repr(C, align(16))]
    #[derive(Clone, Copy)]
    pub(crate) struct Aarch64CloneStart {
        pub frame: Aarch64ExceptionFrame,
        pub user_sp: u64,
        pub return_pc: u64,
        pub pstate: u64,
        pub tls: u64,
        pub root_paddr: u64,
    }

    const _: () = {
        assert!(core::mem::offset_of!(Aarch64CloneStart, frame) == 0x000);
        assert!(core::mem::offset_of!(Aarch64CloneStart, user_sp) == 0x310);
        assert!(core::mem::offset_of!(Aarch64CloneStart, return_pc) == 0x318);
        assert!(core::mem::offset_of!(Aarch64CloneStart, pstate) == 0x320);
        assert!(core::mem::offset_of!(Aarch64CloneStart, tls) == 0x328);
        assert!(core::mem::offset_of!(Aarch64CloneStart, root_paddr) == 0x330);
    };

    #[derive(Clone, Copy)]
    struct TidDestination {
        address: usize,
        original: u32,
        written: bool,
    }

    impl TidDestination {
        const EMPTY: Self = Self {
            address: 0,
            original: 0,
            written: false,
        };
    }

    #[derive(Clone, Copy)]
    pub(super) struct LinuxCloneSlot {
        reservation: LinuxTaskReservation,
        start: Option<Aarch64CloneStart>,
        parent_tid: TidDestination,
        child_tid: TidDestination,
        clear_child_tid: usize,
        committed: bool,
    }

    impl LinuxCloneSlot {
        pub(super) const EMPTY: Self = Self {
            reservation: LinuxTaskReservation {
                slot: usize::MAX,
                tid: 0,
                scheduler_thread: usize::MAX,
            },
            start: None,
            parent_tid: TidDestination::EMPTY,
            child_tid: TidDestination::EMPTY,
            clear_child_tid: 0,
            committed: false,
        };

        fn matches(&self, reservation: LinuxTaskReservation) -> bool {
            self.reservation == reservation && self.start.is_some()
        }
    }

    pub(crate) fn reserve_clone(
        scheduler_id: ThreadId,
        request: LinuxCloneRequest,
        context: LinuxSyscallFrameRef,
    ) -> Result<LinuxTaskReservation, SysError> {
        let root_paddr = crate::syscall::linux_process_memory::current_root_paddr()?;
        if root_paddr == 0 {
            return Err(SysError::EAGAIN);
        }
        with_runtime(|runtime| {
            let current = scheduler::scheduler().current();
            let Some(parent) = runtime.tasks.by_scheduler(current.0) else {
                return Err(SysError::EAGAIN);
            };
            if scheduler_id == ThreadId::IDLE
                || scheduler_id == current
                || scheduler::scheduler()
                    .get_thread(scheduler_id)
                    .map(|thread| thread.state)
                    != Some(ThreadState::Blocked)
            {
                return Err(SysError::EAGAIN);
            }

            let reservation = runtime
                .tasks
                .reserve_child(parent.tgid, scheduler_id.0)
                .ok_or(SysError::EAGAIN)?;
            if !runtime.tasks.inherit_signal_mask(reservation, current.0) {
                let _ = runtime.tasks.rollback(reservation);
                return Err(SysError::EAGAIN);
            }
            if !runtime.tasks.inherit_sched_param(reservation, current.0) {
                let _ = runtime.tasks.rollback(reservation);
                return Err(SysError::EAGAIN);
            }
            let mut frame = unsafe { context.frame.read() };
            frame.regs[0] = 0;
            let tls = request
                .tls
                .map(|tls| tls as u64)
                .unwrap_or_else(crate::kernel_lowlevel::cpu::read_user_tls);
            let configured = scheduler::scheduler()
                .get_thread_mut(scheduler_id)
                .map(|thread| {
                    thread
                        .context
                        .set_linux_process_start(request.user_sp as u64, tls, root_paddr)
                })
                .unwrap_or(false)
                && scheduler::scheduler().bind_thread_process(scheduler_id, parent.tgid);
            if !configured {
                let _ = runtime.tasks.rollback(reservation);
                return Err(SysError::EAGAIN);
            }
            runtime.clone_slots[reservation.slot] = LinuxCloneSlot {
                reservation,
                start: Some(Aarch64CloneStart {
                    frame,
                    user_sp: request.user_sp as u64,
                    return_pc: context.return_pc,
                    pstate: context.pstate,
                    tls,
                    root_paddr,
                }),
                parent_tid: TidDestination {
                    address: request.parent_tid.unwrap_or(0),
                    ..TidDestination::EMPTY
                },
                child_tid: TidDestination {
                    address: if request.flags & CLONE_CHILD_SETTID != 0 {
                        request.child_tid.unwrap_or(0)
                    } else {
                        0
                    },
                    ..TidDestination::EMPTY
                },
                clear_child_tid: if request.clear_child_tid {
                    request.child_tid.unwrap_or(0)
                } else {
                    0
                },
                committed: false,
            };
            Ok(reservation)
        })
    }

    pub(crate) fn copy_clone_tids(reservation: LinuxTaskReservation) -> Result<(), SysError> {
        let pid = current_tgid()?;
        let (addresses, tid) = with_runtime(|runtime| {
            let slot = runtime
                .clone_slots
                .get(reservation.slot)
                .filter(|slot| slot.matches(reservation) && !slot.committed)
                .ok_or(SysError::EAGAIN)?;
            let tid = linux_tid_to_user_value(reservation.tid).ok_or(SysError::EAGAIN)?;
            Ok(([slot.parent_tid.address, slot.child_tid.address], tid))
        })?;
        for address in addresses.into_iter().filter(|address| *address != 0) {
            if !crate::syscall::syscall::linux_clone_tid_destination_valid(address) {
                return Err(SysError::EFAULT);
            }
        }
        let mut originals = [0u32; 2];
        for (index, address) in addresses.into_iter().enumerate() {
            if address != 0 {
                let mut bytes = [0u8; core::mem::size_of::<u32>()];
                crate::syscall::linux_process_memory::copy_from_process(pid, address, &mut bytes)?;
                originals[index] = u32::from_ne_bytes(bytes);
            }
        }
        let mut written = [false; 2];
        for (index, address) in addresses.into_iter().enumerate() {
            if address != 0 {
                if let Err(error) = crate::syscall::linux_process_memory::copy_to_process(
                    pid,
                    address,
                    &tid.to_ne_bytes(),
                ) {
                    for rollback in 0..index {
                        if written[rollback] {
                            let _ = crate::syscall::linux_process_memory::copy_to_process(
                                pid,
                                addresses[rollback],
                                &originals[rollback].to_ne_bytes(),
                            );
                        }
                    }
                    return Err(error);
                }
                written[index] = true;
            }
        }
        let committed = with_runtime(|runtime| {
            let Some(slot) = runtime
                .clone_slots
                .get_mut(reservation.slot)
                .filter(|slot| slot.matches(reservation) && !slot.committed)
            else {
                return false;
            };
            for (index, destination) in [&mut slot.parent_tid, &mut slot.child_tid]
                .into_iter()
                .enumerate()
            {
                destination.original = originals[index];
                destination.written = written[index];
            }
            true
        });
        if committed {
            Ok(())
        } else {
            for index in 0..2 {
                if written[index] {
                    let _ = crate::syscall::linux_process_memory::copy_to_process(
                        pid,
                        addresses[index],
                        &originals[index].to_ne_bytes(),
                    );
                }
            }
            Err(SysError::EAGAIN)
        }
    }

    pub(crate) fn restore_clone_tid_destinations(reservation: LinuxTaskReservation) {
        let Ok(pid) = current_tgid() else {
            return;
        };
        let Some(destinations) = with_runtime(|runtime| {
            runtime
                .clone_slots
                .get(reservation.slot)
                .filter(|slot| slot.matches(reservation) && !slot.committed)
                .map(|slot| [slot.parent_tid, slot.child_tid])
        }) else {
            return;
        };
        for destination in destinations {
            if destination.written {
                let _ = crate::syscall::linux_process_memory::copy_to_process(
                    pid,
                    destination.address,
                    &destination.original.to_ne_bytes(),
                );
            }
        }
        with_runtime(|runtime| {
            if let Some(slot) = runtime
                .clone_slots
                .get_mut(reservation.slot)
                .filter(|slot| slot.matches(reservation) && !slot.committed)
            {
                slot.parent_tid.written = false;
                slot.child_tid.written = false;
            }
        });
    }

    pub(crate) fn rollback_clone(reservation: LinuxTaskReservation) {
        with_runtime(|runtime| {
            let Some(slot) = runtime.clone_slots.get_mut(reservation.slot) else {
                return;
            };
            if !slot.matches(reservation) || slot.committed {
                return;
            }
            let _ = runtime.tasks.rollback(reservation);
            *slot = LinuxCloneSlot::EMPTY;
        });
    }

    pub(crate) fn commit_clone(reservation: LinuxTaskReservation) -> Result<(), SysError> {
        with_runtime(|runtime| {
            let valid_slot = runtime
                .clone_slots
                .get(reservation.slot)
                .map(|slot| slot.matches(reservation) && !slot.committed)
                .unwrap_or(false);
            let scheduler_id = ThreadId(reservation.scheduler_thread);
            let suspended = scheduler::scheduler()
                .get_thread(scheduler_id)
                .map(|thread| thread.state)
                == Some(ThreadState::Blocked);
            if !valid_slot || !suspended || !runtime.tasks.publish(reservation) {
                return Err(SysError::EAGAIN);
            }
            let clear_child_tid = runtime.clone_slots[reservation.slot].clear_child_tid;
            if !runtime.tasks.set_clear_child_tid(
                reservation.tid,
                reservation.scheduler_thread,
                clear_child_tid,
            ) {
                let _ = runtime
                    .tasks
                    .exit(reservation.tid, reservation.scheduler_thread);
                let _ = runtime
                    .tasks
                    .retire(reservation.tid, reservation.scheduler_thread);
                return Err(SysError::EAGAIN);
            }
            if !scheduler::scheduler().publish_suspended_thread(scheduler_id) {
                let _ = runtime
                    .tasks
                    .exit(reservation.tid, reservation.scheduler_thread);
                let _ = runtime
                    .tasks
                    .retire(reservation.tid, reservation.scheduler_thread);
                return Err(SysError::EAGAIN);
            }
            runtime.clone_slots[reservation.slot].committed = true;
            Ok(())
        })
    }

    fn take_clone_start() -> Option<Aarch64CloneStart> {
        with_runtime(|runtime| {
            let scheduler_id = scheduler::scheduler().current();
            let task = runtime.tasks.by_scheduler(scheduler_id.0)?;
            runtime
                .clone_slots
                .iter_mut()
                .find(|slot| {
                    slot.committed
                        && slot.reservation.tid == task.tid
                        && slot.reservation.scheduler_thread == scheduler_id.0
                })?
                .start
                .take()
        })
    }

    pub(crate) extern "C" fn linux_clone_child_entry() -> ! {
        let Some(start) = take_clone_start() else {
            scheduler::scheduler().finish_current_without_stack_free();
            scheduler::schedule();
            loop {
                crate::kernel_lowlevel::cpu::wait_for_interrupt();
            }
        };
        unsafe { thread::start_linux_clone_child(&start as *const Aarch64CloneStart as *const u8) }
    }
}

#[cfg(target_arch = "aarch64")]
pub(crate) use aarch64_clone::{
    commit_clone, copy_clone_tids, linux_clone_child_entry, reserve_clone,
    restore_clone_tid_destinations, rollback_clone,
};
