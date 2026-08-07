pub(crate) const LINUX_ROOT_PID: usize = 1;
pub(crate) const LINUX_MAX_PID: usize = i32::MAX as usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxProcessState {
    Empty,
    Reserved,
    Publishing,
    Running,
    Zombie,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxProcessCore {
    pub pid: usize,
    pub parent_pid: usize,
    pub process_group: usize,
    pub root_scheduler_thread: usize,
    pub state: LinuxProcessState,
    pub wait_status: i32,
    pub exit_signal: usize,
}

impl LinuxProcessCore {
    const EMPTY: Self = Self {
        pid: 0,
        parent_pid: 0,
        process_group: 0,
        root_scheduler_thread: 0,
        state: LinuxProcessState::Empty,
        wait_status: 0,
        exit_signal: 0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxProcessReservation {
    pub slot: usize,
    pub pid: usize,
    pub parent_pid: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxWaitSelector {
    Pid(usize),
    Any,
    ProcessGroup(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxProcessError {
    Capacity,
    DuplicateRoot,
    Exhausted,
    NoSuchParent,
}

pub(crate) fn linux_wait_status_exit(code: i32) -> i32 {
    ((code as u32 & 0xff) << 8) as i32
}

pub(crate) fn linux_wait_status_signal(signum: usize, core_dumped: bool) -> Option<i32> {
    (1..=127)
        .contains(&signum)
        .then_some(signum as i32 | if core_dumped { 0x80 } else { 0 })
}

pub(crate) struct LinuxProcessTable<const N: usize> {
    processes: [LinuxProcessCore; N],
    next_pid: usize,
    exhausted: bool,
}

impl<const N: usize> LinuxProcessTable<N> {
    pub(crate) const fn new() -> Self {
        Self::with_next_pid(LINUX_ROOT_PID + 1)
    }

    pub(crate) const fn with_next_pid(next_pid: usize) -> Self {
        Self {
            processes: [LinuxProcessCore::EMPTY; N],
            next_pid,
            exhausted: false,
        }
    }

    pub(crate) fn register_root(
        &mut self,
        scheduler_thread: usize,
    ) -> Result<usize, LinuxProcessError> {
        if self.processes.iter().any(|process| {
            process.state != LinuxProcessState::Empty && process.pid == LINUX_ROOT_PID
        }) {
            return Err(LinuxProcessError::DuplicateRoot);
        }
        let Some(slot) = self
            .processes
            .iter()
            .position(|process| process.state == LinuxProcessState::Empty)
        else {
            return Err(LinuxProcessError::Capacity);
        };
        self.processes[slot] = LinuxProcessCore {
            pid: LINUX_ROOT_PID,
            parent_pid: 0,
            process_group: LINUX_ROOT_PID,
            root_scheduler_thread: scheduler_thread,
            state: LinuxProcessState::Running,
            wait_status: 0,
            exit_signal: 0,
        };
        Ok(LINUX_ROOT_PID)
    }

    pub(crate) fn reserve_child(
        &mut self,
        parent_pid: usize,
        scheduler_thread: usize,
    ) -> Result<LinuxProcessReservation, LinuxProcessError> {
        if self.exhausted || !(LINUX_ROOT_PID + 1..=LINUX_MAX_PID).contains(&self.next_pid) {
            self.exhausted = true;
            return Err(LinuxProcessError::Exhausted);
        }
        self.reserve_child_with_pid(parent_pid, scheduler_thread, self.next_pid, 0)
    }

    pub(crate) fn reserve_child_with_pid(
        &mut self,
        parent_pid: usize,
        scheduler_thread: usize,
        pid: usize,
        exit_signal: usize,
    ) -> Result<LinuxProcessReservation, LinuxProcessError> {
        let parent = self
            .processes
            .iter()
            .copied()
            .find(|process| {
                process.pid == parent_pid && process.state == LinuxProcessState::Running
            })
            .ok_or(LinuxProcessError::NoSuchParent)?;
        let slot = self
            .processes
            .iter()
            .position(|process| process.state == LinuxProcessState::Empty)
            .ok_or(LinuxProcessError::Capacity)?;
        if !(LINUX_ROOT_PID + 1..=LINUX_MAX_PID).contains(&pid)
            || self.processes.iter().any(|process| {
                process.state != LinuxProcessState::Empty && process.pid == pid
            })
        {
            return Err(LinuxProcessError::Exhausted);
        }

        if pid == LINUX_MAX_PID {
            self.exhausted = true;
        } else if pid >= self.next_pid {
            self.next_pid = pid + 1;
        }
        self.processes[slot] = LinuxProcessCore {
            pid,
            parent_pid,
            process_group: parent.process_group,
            root_scheduler_thread: scheduler_thread,
            state: LinuxProcessState::Reserved,
            wait_status: 0,
            exit_signal,
        };
        Ok(LinuxProcessReservation {
            slot,
            pid,
            parent_pid,
        })
    }

    pub(crate) fn publish(&mut self, reservation: LinuxProcessReservation) -> bool {
        let Some(process) = self.processes.get_mut(reservation.slot) else {
            return false;
        };
        if process.state != LinuxProcessState::Reserved
            || process.pid != reservation.pid
            || process.parent_pid != reservation.parent_pid
        {
            return false;
        }
        process.state = LinuxProcessState::Running;
        true
    }

    pub(crate) fn publish_fork(&mut self, reservation: LinuxProcessReservation) -> bool {
        let Some(process) = self.processes.get_mut(reservation.slot) else {
            return false;
        };
        if process.state != LinuxProcessState::Reserved
            || process.pid != reservation.pid
            || process.parent_pid != reservation.parent_pid
        {
            return false;
        }
        process.state = LinuxProcessState::Publishing;
        true
    }

    pub(crate) fn complete_fork_publish(&mut self, reservation: LinuxProcessReservation) -> bool {
        let Some(process) = self.processes.get_mut(reservation.slot) else {
            return false;
        };
        if process.state != LinuxProcessState::Publishing
            || process.pid != reservation.pid
            || process.parent_pid != reservation.parent_pid
        {
            return false;
        }
        process.state = LinuxProcessState::Running;
        true
    }

    pub(crate) fn rollback_fork(&mut self, reservation: LinuxProcessReservation) -> bool {
        let Some(process) = self.processes.get_mut(reservation.slot) else {
            return false;
        };
        if !matches!(
            process.state,
            LinuxProcessState::Reserved | LinuxProcessState::Publishing
        ) || process.pid != reservation.pid
            || process.parent_pid != reservation.parent_pid
        {
            return false;
        }
        *process = LinuxProcessCore::EMPTY;
        true
    }

    pub(crate) fn rollback(&mut self, reservation: LinuxProcessReservation) -> bool {
        let Some(process) = self.processes.get_mut(reservation.slot) else {
            return false;
        };
        if process.state != LinuxProcessState::Reserved
            || process.pid != reservation.pid
            || process.parent_pid != reservation.parent_pid
        {
            return false;
        }
        *process = LinuxProcessCore::EMPTY;
        true
    }

    pub(crate) fn by_pid(&self, pid: usize) -> Option<LinuxProcessCore> {
        self.processes
            .iter()
            .copied()
            .find(|process| Self::is_visible(*process) && process.pid == pid)
    }

    pub(crate) fn by_scheduler(&self, scheduler_thread: usize) -> Option<LinuxProcessCore> {
        self.processes.iter().copied().find(|process| {
            Self::is_visible(*process) && process.root_scheduler_thread == scheduler_thread
        })
    }

    pub(crate) fn exit(&mut self, pid: usize, wait_status: i32) -> bool {
        let Some(process) = self
            .processes
            .iter_mut()
            .find(|process| process.pid == pid && process.state == LinuxProcessState::Running)
        else {
            return false;
        };
        process.state = LinuxProcessState::Zombie;
        process.wait_status = wait_status;
        true
    }

    pub(crate) fn select_waitable(
        &self,
        parent_pid: usize,
        selector: LinuxWaitSelector,
    ) -> Option<LinuxProcessCore> {
        self.processes
            .iter()
            .copied()
            .filter(|process| {
                process.state == LinuxProcessState::Zombie
                    && process.parent_pid == parent_pid
                    && Self::selector_matches(*process, selector)
            })
            .min_by_key(|process| process.pid)
    }

    pub(crate) fn has_matching_child(
        &self,
        parent_pid: usize,
        selector: LinuxWaitSelector,
    ) -> bool {
        self.processes.iter().copied().any(|process| {
            Self::is_visible(process)
                && process.parent_pid == parent_pid
                && Self::selector_matches(process, selector)
        })
    }

    pub(crate) fn reap(&mut self, parent_pid: usize, pid: usize) -> Option<LinuxProcessCore> {
        let slot = self.processes.iter().position(|process| {
            process.state == LinuxProcessState::Zombie
                && process.parent_pid == parent_pid
                && process.pid == pid
        })?;
        let reaped = self.processes[slot];
        self.processes[slot] = LinuxProcessCore::EMPTY;
        Some(reaped)
    }

    pub(crate) fn reparent_children(&mut self, parent_pid: usize, new_parent_pid: usize) -> usize {
        if !self
            .processes
            .iter()
            .any(|process| Self::is_visible(*process) && process.pid == new_parent_pid)
        {
            return 0;
        }
        let mut reparented = 0;
        for process in &mut self.processes {
            if Self::is_visible(*process) && process.parent_pid == parent_pid {
                process.parent_pid = new_parent_pid;
                reparented += 1;
            }
        }
        reparented
    }

    pub(crate) fn reset(&mut self) {
        self.processes.fill(LinuxProcessCore::EMPTY);
        self.next_pid = LINUX_ROOT_PID + 1;
        self.exhausted = false;
    }

    fn is_visible(process: LinuxProcessCore) -> bool {
        matches!(
            process.state,
            LinuxProcessState::Running | LinuxProcessState::Zombie
        )
    }

    fn selector_matches(process: LinuxProcessCore, selector: LinuxWaitSelector) -> bool {
        match selector {
            LinuxWaitSelector::Pid(pid) => process.pid == pid,
            LinuxWaitSelector::Any => true,
            LinuxWaitSelector::ProcessGroup(process_group) => {
                process.process_group == process_group
            }
        }
    }
}
