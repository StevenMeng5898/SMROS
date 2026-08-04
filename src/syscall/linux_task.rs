use crate::kernel_lowlevel::thread::{self, ThreadId};
use crate::kernel_objects::scheduler;

use super::SysError;

include!("linux_task_logic_shared.rs");
include!("linux_runtime_lock_shared.rs");

const LINUX_TASK_LIMIT: usize = thread::MAX_THREADS;

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

pub(crate) fn block_current(reason: LinuxBlockReason) -> Result<LinuxTaskCore, SysError> {
    if reason == LinuxBlockReason::None {
        return Err(SysError::EINVAL);
    }
    with_runtime(|runtime| {
        let scheduler_thread = scheduler::scheduler().current();
        let task = runtime
            .tasks
            .by_scheduler(scheduler_thread.0)
            .ok_or(SysError::ESRCH)?;
        if !runtime.tasks.block(task.tid, scheduler_thread.0, reason) {
            return Err(SysError::EAGAIN);
        }
        if !scheduler::scheduler().block_thread(scheduler_thread) {
            let _ = runtime.tasks.wake(task.tid, scheduler_thread.0);
            return Err(SysError::EAGAIN);
        }
        Ok(task)
    })
}

pub(crate) fn wake_blocked(tid: usize, scheduler_thread: usize, reason: LinuxBlockReason) -> bool {
    if reason == LinuxBlockReason::None {
        return false;
    }
    with_runtime(|runtime| {
        let Some(task) = runtime.tasks.by_tid(tid) else {
            return false;
        };
        if task.scheduler_thread != scheduler_thread
            || task.state != LinuxTaskState::Blocked
            || task.block_reason != reason
        {
            return false;
        }
        let scheduler_id = ThreadId(scheduler_thread);
        if scheduler::scheduler()
            .get_thread(scheduler_id)
            .map(|thread| thread.state)
            != Some(thread::ThreadState::Blocked)
            || !runtime.tasks.wake(tid, scheduler_thread)
        {
            return false;
        }
        if scheduler::scheduler().wake_thread(scheduler_id) {
            true
        } else {
            let _ = runtime.tasks.block(tid, scheduler_thread, reason);
            false
        }
    })
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
    }

    const _: () = {
        assert!(core::mem::offset_of!(Aarch64CloneStart, frame) == 0x000);
        assert!(core::mem::offset_of!(Aarch64CloneStart, user_sp) == 0x310);
        assert!(core::mem::offset_of!(Aarch64CloneStart, return_pc) == 0x318);
        assert!(core::mem::offset_of!(Aarch64CloneStart, pstate) == 0x320);
        assert!(core::mem::offset_of!(Aarch64CloneStart, tls) == 0x328);
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
        with_runtime(|runtime| {
            let current = scheduler::scheduler().current();
            if runtime.tasks.by_scheduler(current.0).is_none()
                || scheduler_id == ThreadId::IDLE
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
                .reserve_child(scheduler_id.0)
                .ok_or(SysError::EAGAIN)?;
            let mut frame = unsafe { context.frame.read() };
            frame.regs[0] = 0;
            let tls = request
                .tls
                .map(|tls| tls as u64)
                .unwrap_or_else(crate::kernel_lowlevel::cpu::read_user_tls);
            runtime.clone_slots[reservation.slot] = LinuxCloneSlot {
                reservation,
                start: Some(Aarch64CloneStart {
                    frame,
                    user_sp: request.user_sp as u64,
                    return_pc: context.return_pc,
                    pstate: context.pstate,
                    tls,
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
        with_runtime(|runtime| {
            let slot = runtime
                .clone_slots
                .get_mut(reservation.slot)
                .filter(|slot| slot.matches(reservation) && !slot.committed)
                .ok_or(SysError::EAGAIN)?;
            let tid = linux_tid_to_user_value(reservation.tid).ok_or(SysError::EAGAIN)?;

            for destination in [&slot.parent_tid, &slot.child_tid] {
                if destination.address != 0
                    && !crate::syscall::syscall::linux_clone_tid_destination_valid(
                        destination.address,
                    )
                {
                    return Err(SysError::EFAULT);
                }
            }
            for destination in [&mut slot.parent_tid, &mut slot.child_tid] {
                if destination.address != 0 {
                    destination.original =
                        unsafe { core::ptr::read(destination.address as *const u32) };
                }
            }
            for destination in [&mut slot.parent_tid, &mut slot.child_tid] {
                if destination.address != 0 {
                    unsafe {
                        core::ptr::write(destination.address as *mut u32, tid);
                    }
                    destination.written = true;
                }
            }
            Ok(())
        })
    }

    pub(crate) fn restore_clone_tid_destinations(reservation: LinuxTaskReservation) {
        with_runtime(|runtime| {
            let Some(slot) = runtime
                .clone_slots
                .get_mut(reservation.slot)
                .filter(|slot| slot.matches(reservation) && !slot.committed)
            else {
                return;
            };
            for destination in [&mut slot.parent_tid, &mut slot.child_tid] {
                if destination.written {
                    unsafe {
                        core::ptr::write(destination.address as *mut u32, destination.original);
                    }
                    destination.written = false;
                }
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
