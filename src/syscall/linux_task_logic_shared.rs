macro_rules! smros_linux_root_tid_body {
    () => {{
        1usize
    }};
}

pub(crate) const LINUX_ROOT_TID: usize = smros_linux_root_tid_body!();

macro_rules! smros_linux_task_next_tid_body {
    ($next_tid:expr) => {{
        $next_tid.checked_add(1)
    }};
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
        let Some(next_tid) = smros_linux_task_next_tid_body!(self.next_tid) else {
            self.exhausted = true;
            return None;
        };
        let reservation = LinuxTaskReservation {
            slot,
            tid: self.next_tid,
            scheduler_thread,
        };
        self.next_tid = next_tid;
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
