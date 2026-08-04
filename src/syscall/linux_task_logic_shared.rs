macro_rules! smros_linux_root_tid_body {
    () => {{
        1usize
    }};
}

macro_rules! smros_linux_max_signal_body {
    () => {{
        64usize
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

macro_rules! smros_linux_task_tid_allocation_body {
    ($next_tid:expr, $root_tid:expr, $max_tid:expr) => {{
        let next_tid = $next_tid;
        let root_tid = $root_tid;
        let max_tid = $max_tid;
        if next_tid <= root_tid || next_tid > max_tid {
            None
        } else {
            let following_tid = if next_tid < max_tid {
                Some(next_tid + 1)
            } else {
                None
            };
            Some((next_tid, following_tid))
        }
    }};
}

pub(crate) fn linux_task_tid_allocation(next_tid: usize) -> Option<(usize, Option<usize>)> {
    smros_linux_task_tid_allocation_body!(next_tid, LINUX_ROOT_TID, LINUX_MAX_TID)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxChildExitDisposition {
    ScheduleWithoutEl0Return,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxChildExitTransition {
    pub task: LinuxTaskCore,
    pub slot: usize,
    pub clear_child_tid: usize,
    pub disposition: LinuxChildExitDisposition,
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

pub(crate) const LINUX_MAX_SIGNAL: usize = smros_linux_max_signal_body!();
pub(crate) const LINUX_REALTIME_SIGNAL_MIN: usize = 32;
pub(crate) const LINUX_SIGNAL_INFO_BYTES: usize = 128;
pub(crate) const LINUX_RT_QUEUE_LIMIT: usize = 64;
pub(crate) const LINUX_SIGNAL_FRAME_LIMIT: usize = 16;
pub(crate) const LINUX_SS_ONSTACK: u64 = 1;
pub(crate) const LINUX_SS_DISABLE: u64 = 2;

pub(crate) fn linux_signal_bit(signum: usize) -> u64 {
    if !(1..=LINUX_MAX_SIGNAL).contains(&signum) {
        return 0;
    }
    1u64 << (signum - 1)
}

pub(crate) fn linux_signal_info_offset(task_slot: usize, frame_depth: usize) -> Option<usize> {
    if frame_depth >= LINUX_SIGNAL_FRAME_LIMIT {
        return None;
    }
    task_slot
        .checked_mul(LINUX_SIGNAL_FRAME_LIMIT)?
        .checked_add(frame_depth)?
        .checked_mul(LINUX_SIGNAL_INFO_BYTES)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxSignalDisposition {
    Ignore,
    Terminate,
    Handled,
}

const LINUX_DEFAULT_SIGNAL_HANDLER: u64 = 0;
const LINUX_IGNORE_SIGNAL_HANDLER: u64 = 1;

pub(crate) fn linux_signal_disposition(handler: u64, signum: usize) -> LinuxSignalDisposition {
    match handler {
        LINUX_IGNORE_SIGNAL_HANDLER => LinuxSignalDisposition::Ignore,
        LINUX_DEFAULT_SIGNAL_HANDLER => match signum {
            17 | 23 | 28 => LinuxSignalDisposition::Ignore,
            _ => LinuxSignalDisposition::Terminate,
        },
        _ => LinuxSignalDisposition::Handled,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxPendingSignal {
    pub signum: usize,
    pub has_info: bool,
    pub info: [u8; LINUX_SIGNAL_INFO_BYTES],
}

impl LinuxPendingSignal {
    pub(crate) const EMPTY: Self = Self {
        signum: 0,
        has_info: false,
        info: [0; LINUX_SIGNAL_INFO_BYTES],
    };

    pub(crate) const fn standard(signum: usize) -> Self {
        Self {
            signum,
            ..Self::EMPTY
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxPendingSignalSource {
    Task,
    Process,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxPendingSignalReservation {
    Standard(usize),
    Realtime,
}

pub(crate) fn select_linux_pending_signal(
    task: Option<LinuxPendingSignal>,
    process: Option<LinuxPendingSignal>,
) -> Option<(LinuxPendingSignalSource, LinuxPendingSignal)> {
    match (task, process) {
        (Some(task), Some(process)) if task.signum <= process.signum => {
            Some((LinuxPendingSignalSource::Task, task))
        }
        (Some(_), Some(process)) => Some((LinuxPendingSignalSource::Process, process)),
        (Some(task), None) => Some((LinuxPendingSignalSource::Task, task)),
        (None, Some(process)) => Some((LinuxPendingSignalSource::Process, process)),
        (None, None) => None,
    }
}

#[derive(Clone, Copy)]
pub(crate) struct LinuxPendingSignals {
    pub standard_pending: u64,
    pub standard_reserved: u64,
    pub standard_records: [LinuxPendingSignal; LINUX_REALTIME_SIGNAL_MIN],
    pub realtime_pending: [LinuxPendingSignal; LINUX_RT_QUEUE_LIMIT],
    pub realtime_len: usize,
    pub realtime_reserved: usize,
}

impl LinuxPendingSignals {
    pub(crate) const fn new() -> Self {
        Self {
            standard_pending: 0,
            standard_reserved: 0,
            standard_records: [LinuxPendingSignal::EMPTY; LINUX_REALTIME_SIGNAL_MIN],
            realtime_pending: [LinuxPendingSignal::EMPTY; LINUX_RT_QUEUE_LIMIT],
            realtime_len: 0,
            realtime_reserved: 0,
        }
    }

    pub(crate) fn pending_mask(&self) -> u64 {
        let mut pending = self.standard_pending;
        for record in &self.realtime_pending[..self.realtime_len] {
            pending |= linux_signal_bit(record.signum);
        }
        pending
    }

    pub(crate) fn queue(
        &mut self,
        record: LinuxPendingSignal,
    ) -> Result<(), LinuxSignalRouteError> {
        if !(1..=LINUX_MAX_SIGNAL).contains(&record.signum) {
            return Err(LinuxSignalRouteError::InvalidSignal);
        }
        if record.signum < LINUX_REALTIME_SIGNAL_MIN {
            let bit = linux_signal_bit(record.signum);
            if self.standard_pending & bit == 0 {
                self.standard_pending |= bit;
                self.standard_records[record.signum] = record;
            }
            return Ok(());
        }
        if self.realtime_len + self.realtime_reserved >= LINUX_RT_QUEUE_LIMIT {
            return Err(LinuxSignalRouteError::QueueFull);
        }
        self.realtime_pending[self.realtime_len] = record;
        self.realtime_len += 1;
        Ok(())
    }

    pub(crate) fn peek_eligible(
        &self,
        mut eligible: impl FnMut(usize) -> bool,
    ) -> Option<LinuxPendingSignal> {
        let mut standard = self.standard_pending;
        while standard != 0 {
            let signum = standard.trailing_zeros() as usize + 1;
            if self.standard_reserved & linux_signal_bit(signum) == 0 && eligible(signum) {
                return Some(self.standard_records[signum]);
            }
            standard &= !linux_signal_bit(signum);
        }
        lowest_linux_pending_index(&self.realtime_pending[..self.realtime_len], |record| {
            eligible(record.signum)
        })
        .map(|index| self.realtime_pending[index])
    }

    pub(crate) fn take_eligible(
        &mut self,
        mut eligible: impl FnMut(usize) -> bool,
    ) -> Option<LinuxPendingSignal> {
        let mut standard = self.standard_pending;
        while standard != 0 {
            let signum = standard.trailing_zeros() as usize + 1;
            if self.standard_reserved & linux_signal_bit(signum) == 0 && eligible(signum) {
                self.standard_pending &= !linux_signal_bit(signum);
                let record = self.standard_records[signum];
                self.standard_records[signum] = LinuxPendingSignal::EMPTY;
                return Some(record);
            }
            standard &= !linux_signal_bit(signum);
        }
        let index = lowest_linux_pending_index(
            &self.realtime_pending[..self.realtime_len],
            |record| eligible(record.signum),
        )?;
        let record = self.realtime_pending[index];
        for shifted in index..self.realtime_len - 1 {
            self.realtime_pending[shifted] = self.realtime_pending[shifted + 1];
        }
        self.realtime_len -= 1;
        self.realtime_pending[self.realtime_len] = LinuxPendingSignal::EMPTY;
        Some(record)
    }

    pub(crate) fn peek_matching(&self, wait_mask: u64) -> Option<LinuxPendingSignal> {
        self.peek_eligible(|signum| wait_mask & linux_signal_bit(signum) != 0)
    }

    pub(crate) fn take_matching(&mut self, wait_mask: u64) -> Option<LinuxPendingSignal> {
        self.take_eligible(|signum| wait_mask & linux_signal_bit(signum) != 0)
    }

    pub(crate) fn reserve_direct(
        &mut self,
        record: LinuxPendingSignal,
    ) -> Result<Option<LinuxPendingSignalReservation>, LinuxSignalRouteError> {
        if !(1..=LINUX_MAX_SIGNAL).contains(&record.signum) {
            return Err(LinuxSignalRouteError::InvalidSignal);
        }
        if record.signum < LINUX_REALTIME_SIGNAL_MIN {
            let bit = linux_signal_bit(record.signum);
            if self.standard_reserved & bit != 0 {
                self.queue(record)?;
                return Ok(None);
            }
            self.standard_reserved |= bit;
            return Ok(Some(LinuxPendingSignalReservation::Standard(
                record.signum,
            )));
        }
        if self.realtime_len + self.realtime_reserved >= LINUX_RT_QUEUE_LIMIT {
            return Err(LinuxSignalRouteError::QueueFull);
        }
        self.realtime_reserved += 1;
        Ok(Some(LinuxPendingSignalReservation::Realtime))
    }

    pub(crate) fn take_matching_reserved(
        &mut self,
        wait_mask: u64,
    ) -> Option<(LinuxPendingSignal, LinuxPendingSignalReservation)> {
        let record = self.take_matching(wait_mask)?;
        let reservation = if record.signum < LINUX_REALTIME_SIGNAL_MIN {
            let bit = linux_signal_bit(record.signum);
            self.standard_reserved |= bit;
            LinuxPendingSignalReservation::Standard(record.signum)
        } else {
            self.realtime_reserved += 1;
            LinuxPendingSignalReservation::Realtime
        };
        Some((record, reservation))
    }

    pub(crate) fn commit_reservation(
        &mut self,
        reservation: LinuxPendingSignalReservation,
    ) -> Result<(), LinuxSignalRouteError> {
        match reservation {
            LinuxPendingSignalReservation::Standard(signum) => {
                let bit = linux_signal_bit(signum);
                if !(1..LINUX_REALTIME_SIGNAL_MIN).contains(&signum)
                    || self.standard_reserved & bit == 0
                {
                    return Err(LinuxSignalRouteError::InvalidReservation);
                }
                self.standard_reserved &= !bit;
            }
            LinuxPendingSignalReservation::Realtime => {
                if self.realtime_reserved == 0 {
                    return Err(LinuxSignalRouteError::InvalidReservation);
                }
                self.realtime_reserved -= 1;
            }
        }
        Ok(())
    }

    pub(crate) fn rollback_reservation(
        &mut self,
        reservation: LinuxPendingSignalReservation,
        record: LinuxPendingSignal,
    ) -> Result<(), LinuxSignalRouteError> {
        match reservation {
            LinuxPendingSignalReservation::Standard(signum) if signum == record.signum => {
                self.commit_reservation(reservation)?;
                self.requeue_front(record)
            }
            LinuxPendingSignalReservation::Realtime
                if record.signum >= LINUX_REALTIME_SIGNAL_MIN =>
            {
                self.commit_reservation(reservation)?;
                self.requeue_front(record)
            }
            _ => Err(LinuxSignalRouteError::InvalidReservation),
        }
    }

    pub(crate) fn requeue_front(
        &mut self,
        record: LinuxPendingSignal,
    ) -> Result<(), LinuxSignalRouteError> {
        if !(1..=LINUX_MAX_SIGNAL).contains(&record.signum) {
            return Err(LinuxSignalRouteError::InvalidSignal);
        }
        if record.signum < LINUX_REALTIME_SIGNAL_MIN {
            self.standard_pending |= linux_signal_bit(record.signum);
            self.standard_records[record.signum] = record;
            return Ok(());
        }
        if self.realtime_len >= LINUX_RT_QUEUE_LIMIT {
            return Err(LinuxSignalRouteError::QueueFull);
        }
        for index in (0..self.realtime_len).rev() {
            self.realtime_pending[index + 1] = self.realtime_pending[index];
        }
        self.realtime_pending[0] = record;
        self.realtime_len += 1;
        Ok(())
    }

    pub(crate) fn discard(&mut self, signum: usize) {
        self.standard_pending &= !linux_signal_bit(signum);
        if signum < LINUX_REALTIME_SIGNAL_MIN {
            self.standard_records[signum] = LinuxPendingSignal::EMPTY;
        }
        let mut write = 0usize;
        for read in 0..self.realtime_len {
            let record = self.realtime_pending[read];
            if record.signum != signum {
                self.realtime_pending[write] = record;
                write += 1;
            }
        }
        for index in write..self.realtime_len {
            self.realtime_pending[index] = LinuxPendingSignal::EMPTY;
        }
        self.realtime_len = write;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxSignalWaitOutcome {
    Waiting,
    Signal,
    TimedOut,
    Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxSignalWait {
    pub wait_mask: u64,
    pub deadline: Option<u64>,
    pub output_address: usize,
    pub previous_mask: Option<u64>,
    pub outcome: LinuxSignalWaitOutcome,
    pub signal: LinuxPendingSignal,
    pub signal_source: Option<LinuxPendingSignalSource>,
    pub signal_reservation: Option<LinuxPendingSignalReservation>,
}

impl LinuxSignalWait {
    pub(crate) const fn timed(
        wait_mask: u64,
        deadline: Option<u64>,
        output_address: usize,
    ) -> Self {
        Self {
            wait_mask,
            deadline,
            output_address,
            previous_mask: None,
            outcome: LinuxSignalWaitOutcome::Waiting,
            signal: LinuxPendingSignal::EMPTY,
            signal_source: None,
            signal_reservation: None,
        }
    }

    pub(crate) const fn suspend(wait_mask: u64, previous_mask: u64) -> Self {
        Self {
            wait_mask,
            deadline: None,
            output_address: 0,
            previous_mask: Some(previous_mask),
            outcome: LinuxSignalWaitOutcome::Waiting,
            signal: LinuxPendingSignal::EMPTY,
            signal_source: None,
            signal_reservation: None,
        }
    }

    pub(crate) fn accepts(&self, signum: usize) -> bool {
        self.outcome == LinuxSignalWaitOutcome::Waiting
            && self.wait_mask & linux_signal_bit(signum) != 0
    }
}

pub(crate) fn linux_signal_timespec_to_ticks_ceil(
    now: u64,
    seconds: i64,
    nanoseconds: i64,
    tick_nanoseconds: u64,
) -> Option<u64> {
    if seconds < 0 || !(0..1_000_000_000).contains(&nanoseconds) || tick_nanoseconds == 0 {
        return None;
    }
    let total_nanoseconds = (seconds as u64)
        .checked_mul(1_000_000_000)?
        .checked_add(nanoseconds as u64)?;
    let ticks = total_nanoseconds
        .checked_add(tick_nanoseconds - 1)?
        .checked_div(tick_nanoseconds)?;
    now.checked_add(ticks)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxRestartTimeout {
    Unset,
    Infinite,
    Deadline { ticks: u64, realtime: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxRestartBlock {
    pub syscall_number: u64,
    pub arguments: [u64; 6],
    pub svc_address: u64,
    pub timeout: LinuxRestartTimeout,
}

pub(crate) fn lowest_linux_pending_index(
    pending: &[LinuxPendingSignal],
    mut eligible: impl FnMut(LinuxPendingSignal) -> bool,
) -> Option<usize> {
    let mut selected: Option<usize> = None;
    for (index, record) in pending.iter().copied().enumerate() {
        if !eligible(record) {
            continue;
        }
        if selected
            .map(|selected_index| record.signum < pending[selected_index].signum)
            .unwrap_or(true)
        {
            selected = Some(index);
        }
    }
    selected
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxSignalStack {
    pub sp: u64,
    pub flags: u32,
    pub _padding: u32,
    pub size: u64,
}

impl LinuxSignalStack {
    pub(crate) const DISABLED: Self = Self {
        sp: 0,
        flags: LINUX_SS_DISABLE as u32,
        _padding: 0,
        size: 0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxSignalFrame {
    pub regs: [u64; 32],
    pub return_pc: u64,
    pub previous_mask: u64,
    pub user_sp: u64,
    pub previous_stack_flags: u64,
    pub restart: Option<LinuxRestartBlock>,
}

impl LinuxSignalFrame {
    const EMPTY: Self = Self {
        regs: [0; 32],
        return_pc: 0,
        previous_mask: 0,
        user_sp: 0,
        previous_stack_flags: LINUX_SS_DISABLE,
        restart: None,
    };
}

#[derive(Clone, Copy)]
pub(crate) struct LinuxTaskSignalState {
    pub mask: u64,
    pub pending: LinuxPendingSignals,
    pub alt_stack: LinuxSignalStack,
    pub frames: [LinuxSignalFrame; LINUX_SIGNAL_FRAME_LIMIT],
    pub frame_depth: usize,
    pub sigreturn_requested: bool,
    pub signal_wait: Option<LinuxSignalWait>,
    pub restart_block: Option<LinuxRestartBlock>,
    pub suspend_restore_mask: Option<u64>,
}

impl core::ops::Deref for LinuxTaskSignalState {
    type Target = LinuxPendingSignals;

    fn deref(&self) -> &Self::Target {
        &self.pending
    }
}

impl core::ops::DerefMut for LinuxTaskSignalState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.pending
    }
}

impl LinuxTaskSignalState {
    pub(crate) const fn new() -> Self {
        Self {
            mask: 0,
            pending: LinuxPendingSignals::new(),
            alt_stack: LinuxSignalStack::DISABLED,
            frames: [LinuxSignalFrame::EMPTY; LINUX_SIGNAL_FRAME_LIMIT],
            frame_depth: 0,
            sigreturn_requested: false,
            signal_wait: None,
            restart_block: None,
            suspend_restore_mask: None,
        }
    }

    pub(crate) fn peek_unblocked(&self) -> Option<LinuxPendingSignal> {
        self.pending
            .peek_eligible(|signum| self.mask & linux_signal_bit(signum) == 0)
    }

    pub(crate) fn pending_mask(&self) -> u64 {
        self.pending.pending_mask()
    }

    pub(crate) fn queue(
        &mut self,
        record: LinuxPendingSignal,
    ) -> Result<(), LinuxSignalRouteError> {
        self.pending.queue(record)
    }

    pub(crate) fn take_unblocked(&mut self) -> Option<LinuxPendingSignal> {
        self.pending
            .take_eligible(|signum| self.mask & linux_signal_bit(signum) == 0)
    }

    pub(crate) fn matching_signum(&self, wait_mask: u64) -> Option<usize> {
        self.pending
            .peek_matching(wait_mask)
            .map(|record| record.signum)
    }

    pub(crate) fn peek_matching(&self, wait_mask: u64) -> Option<LinuxPendingSignal> {
        self.pending.peek_matching(wait_mask)
    }

    pub(crate) fn take_matching(&mut self, wait_mask: u64) -> Option<LinuxPendingSignal> {
        self.pending.take_matching(wait_mask)
    }

    pub(crate) fn install_signal_wait(&mut self, wait: LinuxSignalWait) -> bool {
        if wait.outcome != LinuxSignalWaitOutcome::Waiting || self.signal_wait.is_some() {
            return false;
        }
        self.signal_wait = Some(wait);
        true
    }

    pub(crate) fn signal_wait_accepts(&self, signum: usize) -> bool {
        self.signal_wait
            .as_ref()
            .map(|wait| wait.accepts(signum))
            .unwrap_or(false)
    }

    pub(crate) fn complete_signal_wait(
        &mut self,
        record: LinuxPendingSignal,
        source: LinuxPendingSignalSource,
        reservation: LinuxPendingSignalReservation,
    ) -> bool {
        let Some(wait) = self.signal_wait.as_mut() else {
            return false;
        };
        if wait.previous_mask.is_some() || !wait.accepts(record.signum) {
            return false;
        }
        wait.signal = record;
        wait.signal_source = Some(source);
        wait.signal_reservation = Some(reservation);
        wait.outcome = LinuxSignalWaitOutcome::Signal;
        true
    }

    pub(crate) fn timed_wait_interrupted_by(&self, signum: usize) -> bool {
        self.signal_wait
            .as_ref()
            .map(|wait| {
                wait.previous_mask.is_none()
                    && wait.outcome == LinuxSignalWaitOutcome::Waiting
                    && !wait.accepts(signum)
                    && self.mask & linux_signal_bit(signum) == 0
            })
            .unwrap_or(false)
    }

    pub(crate) fn interrupt_timed_signal_wait(&mut self, signum: usize) -> bool {
        if !self.timed_wait_interrupted_by(signum) {
            return false;
        }
        let Some(wait) = self.signal_wait.as_mut() else {
            return false;
        };
        wait.outcome = LinuxSignalWaitOutcome::Interrupted;
        true
    }

    pub(crate) fn interrupt_signal_suspend(&mut self, signum: usize) -> bool {
        let Some(wait) = self.signal_wait.as_mut() else {
            return false;
        };
        if wait.previous_mask.is_none() || !wait.accepts(signum) {
            return false;
        }
        wait.outcome = LinuxSignalWaitOutcome::Interrupted;
        true
    }

    pub(crate) fn expire_signal_wait(&mut self, now: u64) -> bool {
        let Some(wait) = self.signal_wait.as_mut() else {
            return false;
        };
        if wait.previous_mask.is_some()
            || wait.outcome != LinuxSignalWaitOutcome::Waiting
            || !wait.deadline.is_some_and(|deadline| deadline <= now)
        {
            return false;
        }
        wait.outcome = LinuxSignalWaitOutcome::TimedOut;
        true
    }

    pub(crate) fn take_signal_wait_outcome(&mut self) -> Option<LinuxSignalWait> {
        if self
            .signal_wait
            .as_ref()
            .map(|wait| wait.outcome == LinuxSignalWaitOutcome::Waiting)
            .unwrap_or(true)
        {
            return None;
        }
        let wait = self.signal_wait.take()?;
        if wait.outcome == LinuxSignalWaitOutcome::Interrupted {
            self.suspend_restore_mask = wait.previous_mask;
        }
        Some(wait)
    }

    pub(crate) fn cancel_signal_wait(&mut self) -> bool {
        let Some(wait) = self.signal_wait.take() else {
            return false;
        };
        if let Some(previous_mask) = wait.previous_mask {
            self.mask = previous_mask;
        }
        true
    }

    pub(crate) fn take_suspend_restore_mask(&mut self) -> Option<u64> {
        self.suspend_restore_mask.take()
    }

    pub(crate) fn install_restart_block(&mut self, restart: LinuxRestartBlock) -> bool {
        if let Some(current) = self.restart_block {
            if current.syscall_number == restart.syscall_number
                && current.arguments == restart.arguments
                && current.svc_address == restart.svc_address
            {
                return true;
            }
        }
        self.restart_block = Some(restart);
        true
    }

    pub(crate) fn set_restart_timeout(&mut self, timeout: LinuxRestartTimeout) -> bool {
        let Some(restart) = self.restart_block.as_mut() else {
            return false;
        };
        restart.timeout = timeout;
        true
    }

    pub(crate) fn restart_timeout(&self) -> Option<LinuxRestartTimeout> {
        self.restart_block.map(|restart| restart.timeout)
    }

    pub(crate) fn clear_restart_block(&mut self) -> bool {
        self.restart_block.take().is_some()
    }

    pub(crate) fn take_restart_for_signal(
        &mut self,
        restart_action: bool,
    ) -> Option<LinuxRestartBlock> {
        let restart = self.restart_block.take();
        restart_action.then_some(restart).flatten()
    }

    pub(crate) fn requeue_front(
        &mut self,
        record: LinuxPendingSignal,
    ) -> Result<(), LinuxSignalRouteError> {
        self.pending.requeue_front(record)
    }

    pub(crate) fn discard(&mut self, signum: usize) {
        self.pending.discard(signum);
    }

    pub(crate) fn push_frame(&mut self, frame: LinuxSignalFrame) -> Option<usize> {
        if self.frame_depth >= LINUX_SIGNAL_FRAME_LIMIT {
            return None;
        }
        let depth = self.frame_depth;
        self.frames[depth] = frame;
        self.frame_depth += 1;
        Some(depth)
    }

    pub(crate) fn request_sigreturn(&mut self) -> bool {
        if self.frame_depth == 0 {
            return false;
        }
        self.sigreturn_requested = true;
        true
    }

    pub(crate) fn take_requested_frame(&mut self) -> Option<LinuxSignalFrame> {
        if !self.sigreturn_requested {
            return None;
        }
        self.sigreturn_requested = false;
        if self.frame_depth == 0 {
            return None;
        }
        self.frame_depth -= 1;
        let frame = self.frames[self.frame_depth];
        self.frames[self.frame_depth] = LinuxSignalFrame::EMPTY;
        Some(frame)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxSignalRouteError {
    NoSuchTask,
    InvalidSignal,
    InvalidReservation,
    QueueFull,
}

pub(crate) struct LinuxTaskTable<const N: usize> {
    tasks: [LinuxTaskCore; N],
    signal_states: [LinuxTaskSignalState; N],
    clear_child_tids: [usize; N],
    next_tid: usize,
    exhausted: bool,
}

impl<const N: usize> LinuxTaskTable<N> {
    pub(crate) const fn new() -> Self {
        Self {
            tasks: [LinuxTaskCore::EMPTY; N],
            signal_states: [LinuxTaskSignalState::new(); N],
            clear_child_tids: [0; N],
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
        let Some(slot) = self
            .tasks
            .iter()
            .position(|task| task.state == LinuxTaskState::Empty)
        else {
            return Err(LinuxTaskError::Capacity);
        };
        self.tasks[slot] = LinuxTaskCore {
            tid: LINUX_ROOT_TID,
            tgid: LINUX_ROOT_TID,
            scheduler_thread,
            state: LinuxTaskState::Runnable,
            block_reason: LinuxBlockReason::None,
        };
        self.signal_states[slot] = LinuxTaskSignalState::new();
        self.clear_child_tids[slot] = 0;
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
        self.signal_states[slot] = LinuxTaskSignalState::new();
        self.clear_child_tids[slot] = 0;
        Some(reservation)
    }

    pub(crate) fn inherit_signal_mask(
        &mut self,
        reservation: LinuxTaskReservation,
        parent_scheduler_thread: usize,
    ) -> bool {
        let Some(parent_slot) = self.tasks.iter().position(|task| {
            Self::is_live(*task) && task.scheduler_thread == parent_scheduler_thread
        }) else {
            return false;
        };
        let Some(child) = self.tasks.get(reservation.slot).copied() else {
            return false;
        };
        if child.state != LinuxTaskState::Starting
            || child.tid != reservation.tid
            || child.scheduler_thread != reservation.scheduler_thread
            || child.tgid != self.tasks[parent_slot].tgid
        {
            return false;
        }

        self.signal_states[reservation.slot] = LinuxTaskSignalState {
            mask: self.signal_states[parent_slot].mask,
            ..LinuxTaskSignalState::new()
        };
        true
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
        self.signal_states[reservation.slot] = LinuxTaskSignalState::new();
        self.clear_child_tids[reservation.slot] = 0;
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

    pub(crate) fn signal_state(
        &self,
        tid: usize,
        scheduler_thread: usize,
    ) -> Option<&LinuxTaskSignalState> {
        let slot = self.task_slot(tid, scheduler_thread)?;
        self.signal_states.get(slot)
    }

    pub(crate) fn signal_state_mut(
        &mut self,
        tid: usize,
        scheduler_thread: usize,
    ) -> Option<&mut LinuxTaskSignalState> {
        let slot = self.task_slot(tid, scheduler_thread)?;
        self.signal_states.get_mut(slot)
    }

    pub(crate) fn signal_state_by_scheduler_mut(
        &mut self,
        scheduler_thread: usize,
    ) -> Option<(usize, &mut LinuxTaskSignalState, LinuxTaskCore)> {
        let slot = self
            .tasks
            .iter()
            .position(|task| Self::is_live(*task) && task.scheduler_thread == scheduler_thread)?;
        Some((slot, &mut self.signal_states[slot], self.tasks[slot]))
    }

    pub(crate) fn route_signal(
        &mut self,
        tgid: Option<usize>,
        tid: usize,
        record: LinuxPendingSignal,
    ) -> Result<LinuxTaskCore, LinuxSignalRouteError> {
        let slot = self
            .tasks
            .iter()
            .position(|task| {
                Self::is_live(*task)
                    && task.tid == tid
                    && tgid.map(|expected| task.tgid == expected).unwrap_or(true)
            })
            .ok_or(LinuxSignalRouteError::NoSuchTask)?;
        let task = self.tasks[slot];
        if record.signum != 0 {
            self.signal_states[slot].queue(record)?;
        }
        Ok(task)
    }

    pub(crate) fn route_signal_and_complete_wait(
        &mut self,
        tgid: Option<usize>,
        tid: usize,
        record: LinuxPendingSignal,
    ) -> Result<(LinuxTaskCore, Option<LinuxBlockReason>), LinuxSignalRouteError> {
        let slot = self
            .tasks
            .iter()
            .position(|task| {
                Self::is_live(*task)
                    && task.tid == tid
                    && tgid.map(|expected| task.tgid == expected).unwrap_or(true)
            })
            .ok_or(LinuxSignalRouteError::NoSuchTask)?;
        let task = self.tasks[slot];
        if record.signum == 0 {
            return Ok((task, None));
        }
        let signal_state = &mut self.signal_states[slot];
        if task.block_reason == LinuxBlockReason::SignalWait
            && signal_state.signal_wait_accepts(record.signum)
        {
            let Some(reservation) = signal_state.pending.reserve_direct(record)? else {
                return Ok((task, None));
            };
            if signal_state.complete_signal_wait(
                record,
                LinuxPendingSignalSource::Task,
                reservation,
            ) {
                return Ok((task, Some(LinuxBlockReason::SignalWait)));
            }
            signal_state
                .pending
                .rollback_reservation(reservation, record)?;
        }
        match task.block_reason {
            LinuxBlockReason::SignalWait => {
                signal_state.queue(record)?;
                if signal_state.interrupt_timed_signal_wait(record.signum) {
                    Ok((task, Some(LinuxBlockReason::SignalWait)))
                } else {
                    Ok((task, None))
                }
            }
            LinuxBlockReason::SignalSuspend if signal_state.signal_wait_accepts(record.signum) => {
                signal_state.queue(record)?;
                if !signal_state.interrupt_signal_suspend(record.signum) {
                    return Ok((task, None));
                }
                Ok((task, Some(LinuxBlockReason::SignalSuspend)))
            }
            _ => {
                signal_state.queue(record)?;
                Ok((task, None))
            }
        }
    }

    pub(crate) fn signal_wait_target(&self, signum: usize) -> Option<LinuxTaskCore> {
        let matching = self
            .tasks
            .iter()
            .zip(self.signal_states.iter())
            .find_map(|(task, signal_state)| {
                (Self::is_live(*task)
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
                        && task.block_reason == LinuxBlockReason::SignalWait
                        && signal_state.timed_wait_interrupted_by(signum))
                    .then_some(*task)
                })
        })
    }

    pub(crate) fn complete_process_signal_wait(
        &mut self,
        tid: usize,
        scheduler_thread: usize,
        record: LinuxPendingSignal,
        reservation: LinuxPendingSignalReservation,
    ) -> Option<LinuxBlockReason> {
        let slot = self.task_slot(tid, scheduler_thread)?;
        let task = self.tasks[slot];
        let signal_state = &mut self.signal_states[slot];
        match task.block_reason {
            LinuxBlockReason::SignalWait
                if signal_state.complete_signal_wait(
                    record,
                    LinuxPendingSignalSource::Process,
                    reservation,
                ) =>
            {
                Some(LinuxBlockReason::SignalWait)
            }
            LinuxBlockReason::SignalSuspend
                if signal_state.interrupt_signal_suspend(record.signum) =>
            {
                Some(LinuxBlockReason::SignalSuspend)
            }
            _ => None,
        }
    }

    pub(crate) fn interrupt_process_signal_wait(
        &mut self,
        tid: usize,
        scheduler_thread: usize,
        signum: usize,
    ) -> Option<LinuxBlockReason> {
        let slot = self.task_slot(tid, scheduler_thread)?;
        let task = self.tasks[slot];
        let signal_state = &mut self.signal_states[slot];
        match task.block_reason {
            LinuxBlockReason::SignalWait
                if signal_state.interrupt_timed_signal_wait(signum) =>
            {
                Some(LinuxBlockReason::SignalWait)
            }
            LinuxBlockReason::SignalSuspend if signal_state.interrupt_signal_suspend(signum) => {
                Some(LinuxBlockReason::SignalSuspend)
            }
            _ => None,
        }
    }

    pub(crate) fn expire_signal_waits(
        &mut self,
        now: u64,
    ) -> [Option<(usize, usize, LinuxBlockReason)>; N] {
        let mut expired = [None; N];
        let mut expired_len = 0usize;
        for (task, signal_state) in self.tasks.iter().zip(self.signal_states.iter_mut()) {
            if task.state == LinuxTaskState::Blocked
                && task.block_reason == LinuxBlockReason::SignalWait
                && signal_state.expire_signal_wait(now)
            {
                expired[expired_len] = Some((task.tid, task.scheduler_thread, task.block_reason));
                expired_len += 1;
            }
        }
        expired
    }

    pub(crate) fn process_signal_target(&self, signum: usize) -> Option<LinuxTaskCore> {
        let bit = linux_signal_bit(signum);
        if bit == 0 {
            return None;
        }
        self.tasks
            .iter()
            .zip(self.signal_states.iter())
            .find_map(|(task, signal_state)| {
                (Self::is_live(*task) && signal_state.mask & bit == 0).then_some(*task)
            })
    }

    pub(crate) fn discard_signal(&mut self, signum: usize) {
        for (task, signal_state) in self.tasks.iter().zip(self.signal_states.iter_mut()) {
            if Self::is_live(*task) {
                signal_state.discard(signum);
            }
        }
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

    pub(crate) fn set_clear_child_tid(
        &mut self,
        tid: usize,
        scheduler_thread: usize,
        address: usize,
    ) -> bool {
        let Some(slot) = self.task_slot(tid, scheduler_thread) else {
            return false;
        };
        self.clear_child_tids[slot] = address;
        true
    }

    pub(crate) fn exit_with_clear_child_tid(
        &mut self,
        tid: usize,
        scheduler_thread: usize,
    ) -> Option<usize> {
        let slot = self.task_slot(tid, scheduler_thread)?;
        let task = &mut self.tasks[slot];
        task.state = LinuxTaskState::Exited;
        task.block_reason = LinuxBlockReason::None;
        self.signal_states[slot] = LinuxTaskSignalState::new();
        Some(core::mem::replace(&mut self.clear_child_tids[slot], 0))
    }

    pub(crate) fn begin_child_exit_by_scheduler(
        &mut self,
        scheduler_thread: usize,
    ) -> Option<LinuxChildExitTransition> {
        let task = self.by_scheduler(scheduler_thread)?;
        let slot = self.task_slot_index(task.tid, scheduler_thread)?;
        let clear_child_tid = self.exit_with_clear_child_tid(task.tid, scheduler_thread)?;
        Some(LinuxChildExitTransition {
            task,
            slot,
            clear_child_tid,
            disposition: LinuxChildExitDisposition::ScheduleWithoutEl0Return,
        })
    }

    pub(crate) fn exit(&mut self, tid: usize, scheduler_thread: usize) -> bool {
        self.exit_with_clear_child_tid(tid, scheduler_thread)
            .is_some()
    }

    pub(crate) fn retire(&mut self, tid: usize, scheduler_thread: usize) -> bool {
        let Some(slot) = self.task_slot_index(tid, scheduler_thread) else {
            return false;
        };
        if self.tasks[slot].state != LinuxTaskState::Exited {
            return false;
        }
        self.tasks[slot] = LinuxTaskCore::EMPTY;
        self.signal_states[slot] = LinuxTaskSignalState::new();
        self.clear_child_tids[slot] = 0;
        true
    }

    pub(crate) fn reset(&mut self) {
        self.tasks.fill(LinuxTaskCore::EMPTY);
        self.signal_states.fill(LinuxTaskSignalState::new());
        self.clear_child_tids.fill(0);
        self.next_tid = LINUX_ROOT_TID + 1;
        self.exhausted = false;
    }

    fn is_published(task: LinuxTaskCore) -> bool {
        task.state != LinuxTaskState::Empty && task.state != LinuxTaskState::Starting
    }

    fn is_live(task: LinuxTaskCore) -> bool {
        task.state == LinuxTaskState::Runnable || task.state == LinuxTaskState::Blocked
    }

    fn task_slot(&self, tid: usize, scheduler_thread: usize) -> Option<usize> {
        self.tasks.iter().position(|task| {
            Self::is_live(*task) && task.tid == tid && task.scheduler_thread == scheduler_thread
        })
    }

    fn task_slot_index(&self, tid: usize, scheduler_thread: usize) -> Option<usize> {
        self.tasks
            .iter()
            .position(|task| task.tid == tid && task.scheduler_thread == scheduler_thread)
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
