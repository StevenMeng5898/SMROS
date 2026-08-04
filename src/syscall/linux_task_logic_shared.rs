macro_rules! smros_linux_root_tid_body {
    () => {{
        1usize
    }};
}

pub(crate) const LINUX_ROOT_TID: usize = smros_linux_root_tid_body!();
pub(crate) const LINUX_MAX_TID: usize = i32::MAX as usize;

pub(crate) const CLONE_VM: usize = 0x0000_0100;
pub(crate) const CLONE_FS: usize = 0x0000_0200;
pub(crate) const CLONE_FILES: usize = 0x0000_0400;
pub(crate) const CLONE_SIGHAND: usize = 0x0000_0800;
pub(crate) const CLONE_THREAD: usize = 0x0001_0000;
pub(crate) const CLONE_SYSVSEM: usize = 0x0004_0000;
pub(crate) const CLONE_SETTLS: usize = 0x0008_0000;
pub(crate) const CLONE_PARENT_SETTID: usize = 0x0010_0000;
pub(crate) const CLONE_CHILD_CLEARTID: usize = 0x0020_0000;
pub(crate) const CLONE_CHILD_SETTID: usize = 0x0100_0000;

const CLONE_EXIT_SIGNAL_MASK: usize = 0xff;
const CLONE_REQUIRED_THREAD_FLAGS: usize =
    CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM;
const CLONE_ALLOWED_THREAD_FLAGS: usize = CLONE_REQUIRED_THREAD_FLAGS
    | CLONE_SETTLS
    | CLONE_PARENT_SETTID
    | CLONE_CHILD_CLEARTID
    | CLONE_CHILD_SETTID;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxCloneValidationError {
    Flags,
    Stack,
    Tls,
    ParentTid,
    ChildTid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxCloneRequest {
    pub flags: usize,
    pub user_sp: usize,
    pub parent_tid: Option<usize>,
    pub tls: Option<usize>,
    pub child_tid: Option<usize>,
    pub clear_child_tid: bool,
}

impl LinuxCloneRequest {
    pub(crate) fn validate(
        flags: usize,
        user_sp: usize,
        parent_tid: usize,
        tls: usize,
        child_tid: usize,
    ) -> Result<Self, LinuxCloneValidationError> {
        if flags & CLONE_EXIT_SIGNAL_MASK != 0
            || flags & !CLONE_ALLOWED_THREAD_FLAGS != 0
            || flags & CLONE_REQUIRED_THREAD_FLAGS != CLONE_REQUIRED_THREAD_FLAGS
            || flags & CLONE_THREAD != 0 && (flags & CLONE_VM == 0 || flags & CLONE_SIGHAND == 0)
        {
            return Err(LinuxCloneValidationError::Flags);
        }
        if user_sp == 0 || user_sp & 0xf != 0 {
            return Err(LinuxCloneValidationError::Stack);
        }
        let tls = if flags & CLONE_SETTLS != 0 {
            if tls == 0 {
                return Err(LinuxCloneValidationError::Tls);
            }
            Some(tls)
        } else {
            None
        };
        let parent_tid = if flags & CLONE_PARENT_SETTID != 0 {
            if !valid_clone_tid_pointer(parent_tid) {
                return Err(LinuxCloneValidationError::ParentTid);
            }
            Some(parent_tid)
        } else {
            None
        };
        let needs_child_tid = flags & (CLONE_CHILD_SETTID | CLONE_CHILD_CLEARTID) != 0;
        let child_tid = if needs_child_tid {
            if !valid_clone_tid_pointer(child_tid) {
                return Err(LinuxCloneValidationError::ChildTid);
            }
            Some(child_tid)
        } else {
            None
        };

        Ok(Self {
            flags,
            user_sp,
            parent_tid,
            tls,
            child_tid,
            clear_child_tid: flags & CLONE_CHILD_CLEARTID != 0,
        })
    }
}

fn valid_clone_tid_pointer(pointer: usize) -> bool {
    pointer != 0 && pointer & 0x3 == 0 && pointer.checked_add(core::mem::size_of::<u32>()).is_some()
}

pub(crate) fn linux_task_tid_allocation(next_tid: usize) -> Option<(usize, Option<usize>)> {
    if next_tid <= LINUX_ROOT_TID || next_tid > LINUX_MAX_TID {
        return None;
    }
    let following_tid = (next_tid < LINUX_MAX_TID).then_some(next_tid + 1);
    Some((next_tid, following_tid))
}

pub(crate) fn linux_tid_to_user_value(tid: usize) -> Option<u32> {
    if !(LINUX_ROOT_TID..=LINUX_MAX_TID).contains(&tid) {
        return None;
    }
    <u32 as core::convert::TryFrom<usize>>::try_from(tid).ok()
}

macro_rules! smros_linux_task_publish_transition_allowed_body {
    ($from:expr, $to:expr, $starting:expr, $runnable:expr) => {{
        $from == $starting && $to == $runnable
    }};
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxTaskError {
    Capacity,
    DuplicateRoot,
    Exhausted,
    InvalidTransition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxTaskState {
    Empty,
    Starting,
    Runnable,
    Blocked,
    Exited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxBlockReason {
    None,
    Futex,
    SignalWait,
    SignalSuspend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxTaskReservation {
    pub slot: usize,
    pub tid: usize,
    pub scheduler_thread: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxTaskCore {
    pub tid: usize,
    pub tgid: usize,
    pub scheduler_thread: usize,
    pub state: LinuxTaskState,
    pub block_reason: LinuxBlockReason,
}

impl LinuxTaskCore {
    const EMPTY: Self = Self {
        tid: 0,
        tgid: 0,
        scheduler_thread: 0,
        state: LinuxTaskState::Empty,
        block_reason: LinuxBlockReason::None,
    };
}

pub(crate) struct LinuxTaskTable<const N: usize> {
    tasks: [LinuxTaskCore; N],
    next_tid: usize,
    exhausted: bool,
}

impl<const N: usize> LinuxTaskTable<N> {
    pub(crate) const fn new() -> Self {
        Self {
            tasks: [LinuxTaskCore::EMPTY; N],
            next_tid: LINUX_ROOT_TID + 1,
            exhausted: false,
        }
    }

    pub(crate) fn register_root(
        &mut self,
        scheduler_thread: usize,
    ) -> Result<usize, LinuxTaskError> {
        if self
            .tasks
            .iter()
            .any(|task| task.state != LinuxTaskState::Empty && task.tid == LINUX_ROOT_TID)
        {
            return Err(LinuxTaskError::DuplicateRoot);
        }
        let Some(task) = self
            .tasks
            .iter_mut()
            .find(|task| task.state == LinuxTaskState::Empty)
        else {
            return Err(LinuxTaskError::Capacity);
        };
        *task = LinuxTaskCore {
            tid: LINUX_ROOT_TID,
            tgid: LINUX_ROOT_TID,
            scheduler_thread,
            state: LinuxTaskState::Runnable,
            block_reason: LinuxBlockReason::None,
        };
        Ok(LINUX_ROOT_TID)
    }

    pub(crate) fn reserve_child(
        &mut self,
        scheduler_thread: usize,
    ) -> Option<LinuxTaskReservation> {
        if self.exhausted {
            return None;
        }
        let slot = self
            .tasks
            .iter()
            .position(|task| task.state == LinuxTaskState::Empty)?;
        let Some((tid, following_tid)) = linux_task_tid_allocation(self.next_tid) else {
            self.exhausted = true;
            return None;
        };
        let reservation = LinuxTaskReservation {
            slot,
            tid,
            scheduler_thread,
        };
        if let Some(next_tid) = following_tid {
            self.next_tid = next_tid;
        } else {
            self.exhausted = true;
        }
        self.tasks[slot] = LinuxTaskCore {
            tid: reservation.tid,
            tgid: LINUX_ROOT_TID,
            scheduler_thread,
            state: LinuxTaskState::Starting,
            block_reason: LinuxBlockReason::None,
        };
        Some(reservation)
    }

    pub(crate) fn publish(&mut self, reservation: LinuxTaskReservation) -> bool {
        let Some(task) = self.tasks.get_mut(reservation.slot) else {
            return false;
        };
        if !smros_linux_task_publish_transition_allowed_body!(
            task.state,
            LinuxTaskState::Runnable,
            LinuxTaskState::Starting,
            LinuxTaskState::Runnable
        ) || task.tid != reservation.tid
            || task.scheduler_thread != reservation.scheduler_thread
        {
            return false;
        }
        task.state = LinuxTaskState::Runnable;
        true
    }

    pub(crate) fn rollback(&mut self, reservation: LinuxTaskReservation) -> bool {
        let Some(task) = self.tasks.get_mut(reservation.slot) else {
            return false;
        };
        if task.state != LinuxTaskState::Starting
            || task.tid != reservation.tid
            || task.scheduler_thread != reservation.scheduler_thread
        {
            return false;
        }
        *task = LinuxTaskCore::EMPTY;
        true
    }

    pub(crate) fn by_tid(&self, tid: usize) -> Option<LinuxTaskCore> {
        self.tasks
            .iter()
            .copied()
            .find(|task| Self::is_published(*task) && task.tid == tid)
    }

    pub(crate) fn by_scheduler(&self, scheduler_thread: usize) -> Option<LinuxTaskCore> {
        self.tasks
            .iter()
            .copied()
            .find(|task| Self::is_published(*task) && task.scheduler_thread == scheduler_thread)
    }

    pub(crate) fn scheduler_thread_for_reset(&self, slot: usize) -> Option<usize> {
        self.tasks
            .get(slot)
            .and_then(|task| (task.state != LinuxTaskState::Empty).then_some(task.scheduler_thread))
    }

    pub(crate) fn block(
        &mut self,
        tid: usize,
        scheduler_thread: usize,
        reason: LinuxBlockReason,
    ) -> bool {
        if reason == LinuxBlockReason::None {
            return false;
        }
        let Some(task) = self.task_for_transition(tid, scheduler_thread) else {
            return false;
        };
        if task.state != LinuxTaskState::Runnable {
            return false;
        }
        task.state = LinuxTaskState::Blocked;
        task.block_reason = reason;
        true
    }

    pub(crate) fn wake(&mut self, tid: usize, scheduler_thread: usize) -> bool {
        let Some(task) = self.task_for_transition(tid, scheduler_thread) else {
            return false;
        };
        if task.state != LinuxTaskState::Blocked {
            return false;
        }
        task.state = LinuxTaskState::Runnable;
        task.block_reason = LinuxBlockReason::None;
        true
    }

    pub(crate) fn exit(&mut self, tid: usize, scheduler_thread: usize) -> bool {
        let Some(task) = self.task_for_transition(tid, scheduler_thread) else {
            return false;
        };
        if task.state != LinuxTaskState::Runnable && task.state != LinuxTaskState::Blocked {
            return false;
        }
        task.state = LinuxTaskState::Exited;
        task.block_reason = LinuxBlockReason::None;
        true
    }

    pub(crate) fn retire(&mut self, tid: usize, scheduler_thread: usize) -> bool {
        let Some(task) = self.task_for_transition(tid, scheduler_thread) else {
            return false;
        };
        if task.state != LinuxTaskState::Exited {
            return false;
        }
        *task = LinuxTaskCore::EMPTY;
        true
    }

    pub(crate) fn reset(&mut self) {
        self.tasks.fill(LinuxTaskCore::EMPTY);
        self.next_tid = LINUX_ROOT_TID + 1;
        self.exhausted = false;
    }

    fn is_published(task: LinuxTaskCore) -> bool {
        task.state != LinuxTaskState::Empty && task.state != LinuxTaskState::Starting
    }

    fn task_for_transition(
        &mut self,
        tid: usize,
        scheduler_thread: usize,
    ) -> Option<&mut LinuxTaskCore> {
        self.tasks.iter_mut().find(|task| {
            Self::is_published(**task)
                && task.tid == tid
                && task.scheduler_thread == scheduler_thread
        })
    }
}
