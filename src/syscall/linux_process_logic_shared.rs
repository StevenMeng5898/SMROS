pub(crate) const LINUX_ROOT_PID: usize = 1;
pub(crate) const LINUX_LAUNCH_REAPER_PID: usize = usize::MAX;
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
pub(crate) enum LinuxWaitOutcome {
    Ready { pid: usize, status: i32 },
    WouldBlock,
    NoChildren,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxSigchldExitPolicy {
    RetainZombieAndNotify,
    ReapAndNotify,
    ReapWithoutNotify,
}

impl LinuxSigchldExitPolicy {
    fn reap_immediately(self) -> bool {
        matches!(self, Self::ReapAndNotify | Self::ReapWithoutNotify)
    }

    fn notify_parent(self) -> bool {
        matches!(self, Self::RetainZombieAndNotify | Self::ReapAndNotify)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxTerminalChildTransition {
    pub parent_pid: usize,
    pub notify_parent: bool,
}

pub(crate) fn apply_linux_terminal_child_transition<E>(
    transition: LinuxTerminalChildTransition,
    notify_parent: impl FnOnce(usize) -> Result<(), E>,
    wake_parent_waiters: impl FnOnce(usize),
) -> Result<(), E> {
    if transition.notify_parent {
        notify_parent(transition.parent_pid)?;
    }
    wake_parent_waiters(transition.parent_pid);
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxProcessSignalStateCore<Action, Pending, const N: usize> {
    pub signal_actions: [Action; N],
    pub process_pending: Pending,
}

impl<Action: Copy, Pending: Copy, const N: usize> LinuxProcessSignalStateCore<Action, Pending, N> {
    pub(crate) const fn new(default_action: Action, empty_pending: Pending) -> Self {
        Self {
            signal_actions: [default_action; N],
            process_pending: empty_pending,
        }
    }

    pub(crate) fn fork_child(&self, empty_pending: Pending) -> Self {
        Self {
            signal_actions: self.signal_actions,
            process_pending: empty_pending,
        }
    }
}

pub(crate) fn prepare_linux_fork_task_signal_state<State>(
    child_state: &mut State,
    parent_mask: u64,
    reset: impl FnOnce(&mut State),
    set_mask: impl FnOnce(&mut State, u64),
) {
    reset(child_state);
    set_mask(child_state, parent_mask);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxSignalDeliveryRoute {
    Ignore,
    TerminateProcess,
    Handle,
}

pub(crate) fn linux_signal_delivery_route<Disposition: PartialEq>(
    disposition: Disposition,
    ignored: Disposition,
    terminate: Disposition,
) -> LinuxSignalDeliveryRoute {
    if disposition == ignored {
        LinuxSignalDeliveryRoute::Ignore
    } else if disposition == terminate {
        LinuxSignalDeliveryRoute::TerminateProcess
    } else {
        LinuxSignalDeliveryRoute::Handle
    }
}

pub(crate) fn select_linux_process_signal_target<T: Copy>(
    tasks: &[T],
    target_tgid: usize,
    signal_bit: u64,
    mut is_live: impl FnMut(T) -> bool,
    mut task_tgid: impl FnMut(T) -> usize,
    mut signal_mask: impl FnMut(usize) -> u64,
) -> Option<T> {
    if signal_bit == 0 {
        return None;
    }
    tasks.iter().copied().enumerate().find_map(|(slot, task)| {
        (is_live(task) && task_tgid(task) == target_tgid && signal_mask(slot) & signal_bit == 0)
            .then_some(task)
    })
}

pub(crate) fn for_each_linux_process_task<T: Copy>(
    tasks: &[T],
    target_tgid: usize,
    scheduler_thread: Option<usize>,
    mut is_live: impl FnMut(T) -> bool,
    mut task_tgid: impl FnMut(T) -> usize,
    mut task_scheduler_thread: impl FnMut(T) -> usize,
    mut operation: impl FnMut(usize, T) -> bool,
) -> usize {
    tasks
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, task)| {
            is_live(*task)
                && task_tgid(*task) == target_tgid
                && scheduler_thread
                    .map(|scheduler| task_scheduler_thread(*task) == scheduler)
                    .unwrap_or(true)
        })
        .filter(|(slot, task)| operation(*slot, *task))
        .count()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxWaitCompletionError<E> {
    Copy(E),
    Reap,
}

pub(crate) fn complete_linux_wait<const N: usize, E>(
    processes: &mut LinuxProcessTable<N>,
    parent_pid: usize,
    selector: LinuxWaitSelector,
    pid: usize,
    status: i32,
    copy_status: impl FnOnce(i32) -> Result<(), E>,
) -> Result<Option<usize>, LinuxWaitCompletionError<E>> {
    let revalidated = processes.wait_outcome(parent_pid, selector);
    if revalidated != (LinuxWaitOutcome::Ready { pid, status }) {
        return Ok(None);
    }
    copy_status(status).map_err(LinuxWaitCompletionError::Copy)?;
    if processes.reap(parent_pid, pid).is_none() {
        return Err(LinuxWaitCompletionError::Reap);
    }
    Ok(Some(pid))
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

pub(crate) fn linux_sigchld_exit_policy(
    sigchld_ignored: bool,
    no_child_wait: bool,
) -> LinuxSigchldExitPolicy {
    if sigchld_ignored {
        LinuxSigchldExitPolicy::ReapWithoutNotify
    } else if no_child_wait {
        LinuxSigchldExitPolicy::ReapAndNotify
    } else {
        LinuxSigchldExitPolicy::RetainZombieAndNotify
    }
}

pub(crate) fn linux_wait_selector(
    pid: i32,
    current_process_group: usize,
) -> Option<LinuxWaitSelector> {
    match pid {
        value if value > 0 => usize::try_from(value).ok().map(LinuxWaitSelector::Pid),
        -1 => Some(LinuxWaitSelector::Any),
        0 => Some(LinuxWaitSelector::ProcessGroup(current_process_group)),
        value => usize::try_from(value.unsigned_abs())
            .ok()
            .map(LinuxWaitSelector::ProcessGroup),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LinuxLaunchReaperRecord {
    active: bool,
}

impl LinuxLaunchReaperRecord {
    const EMPTY: Self = Self { active: false };
}

pub(crate) struct LinuxProcessTable<const N: usize> {
    processes: [LinuxProcessCore; N],
    next_pid: usize,
    exhausted: bool,
    launch_reaper: LinuxLaunchReaperRecord,
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
            launch_reaper: LinuxLaunchReaperRecord::EMPTY,
        }
    }

    pub(crate) fn register_root(
        &mut self,
        scheduler_thread: usize,
    ) -> Result<usize, LinuxProcessError> {
        if self.launch_reaper.active
            || self.processes.iter().any(|process| {
                process.state != LinuxProcessState::Empty && process.pid == LINUX_ROOT_PID
            })
        {
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
        self.launch_reaper.active = true;
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
            || self
                .processes
                .iter()
                .any(|process| process.state != LinuxProcessState::Empty && process.pid == pid)
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

    pub(crate) fn terminate_child(
        &mut self,
        pid: usize,
        wait_status: i32,
        policy: LinuxSigchldExitPolicy,
    ) -> Option<LinuxTerminalChildTransition> {
        let process =
            self.processes.iter().copied().find(|process| {
                process.pid == pid && process.state == LinuxProcessState::Running
            })?;
        if !self.exit(pid, wait_status) {
            return None;
        }
        if policy.reap_immediately() && self.reap(process.parent_pid, pid).is_none() {
            return None;
        }
        Some(LinuxTerminalChildTransition {
            parent_pid: process.parent_pid,
            notify_parent: policy.notify_parent(),
        })
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

    pub(crate) fn wait_outcome(
        &self,
        parent_pid: usize,
        selector: LinuxWaitSelector,
    ) -> LinuxWaitOutcome {
        if let Some(process) = self.select_waitable(parent_pid, selector) {
            return LinuxWaitOutcome::Ready {
                pid: process.pid,
                status: process.wait_status,
            };
        }
        if self.has_matching_child(parent_pid, selector) {
            LinuxWaitOutcome::WouldBlock
        } else {
            LinuxWaitOutcome::NoChildren
        }
    }

    pub(crate) fn resource_counts(&self) -> (usize, usize) {
        self.processes
            .iter()
            .fold((0, 0), |(running, zombies), process| match process.state {
                LinuxProcessState::Running => (running + 1, zombies),
                LinuxProcessState::Zombie => (running, zombies + 1),
                _ => (running, zombies),
            })
    }

    pub(crate) fn running_pids_match(&self, memory_pids: &[usize]) -> bool {
        let running_count = self
            .processes
            .iter()
            .filter(|process| process.state == LinuxProcessState::Running)
            .count();
        running_count == memory_pids.len()
            && self.processes.iter().all(|process| {
                process.state != LinuxProcessState::Running || memory_pids.contains(&process.pid)
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

    pub(crate) fn reparent_children_to_launch_reaper(&mut self, parent_pid: usize) -> usize {
        if !self.launch_reaper.active {
            return 0;
        }
        let mut reparented = 0;
        for process in &mut self.processes {
            if Self::is_visible(*process) && process.parent_pid == parent_pid {
                process.parent_pid = LINUX_LAUNCH_REAPER_PID;
                reparented += 1;
            }
        }
        reparented
    }

    pub(crate) fn adopt_launch_descendants(&mut self, root_pid: usize) -> usize {
        if !self.launch_reaper.active {
            return 0;
        }
        let mut adopted = 0;
        for process in &mut self.processes {
            if Self::is_visible(*process) && process.pid != root_pid {
                process.parent_pid = LINUX_LAUNCH_REAPER_PID;
                adopted += 1;
            }
        }
        adopted
    }

    pub(crate) fn reap_launch_descendants(&mut self) -> usize {
        if !self.launch_reaper.active {
            return 0;
        }
        let mut reaped = 0;
        for process in &mut self.processes {
            if Self::is_visible(*process) && process.parent_pid == LINUX_LAUNCH_REAPER_PID {
                *process = LinuxProcessCore::EMPTY;
                reaped += 1;
            }
        }
        reaped
    }

    pub(crate) fn reset(&mut self) {
        self.processes.fill(LinuxProcessCore::EMPTY);
        self.next_pid = LINUX_ROOT_PID + 1;
        self.exhausted = false;
        self.launch_reaper = LinuxLaunchReaperRecord::EMPTY;
    }

    pub(crate) fn launch_reaper_active(&self) -> bool {
        self.launch_reaper.active
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
