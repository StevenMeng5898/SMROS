pub(crate) const FUTEX_WAIT: u32 = 0;
pub(crate) const FUTEX_WAKE: u32 = 1;
pub(crate) const FUTEX_WAIT_BITSET: u32 = 9;
pub(crate) const FUTEX_WAKE_BITSET: u32 = 10;
pub(crate) const FUTEX_PRIVATE_FLAG: u32 = 128;
pub(crate) const FUTEX_CLOCK_REALTIME: u32 = 256;
pub(crate) const FUTEX_CMD_MASK: u32 = 0x7f;
pub(crate) const FUTEX_BITSET_MATCH_ANY: u32 = u32::MAX;
pub(crate) const FUTEX_SHARED_KEY_TAG: usize = 1usize << (usize::BITS - 1);

const FUTEX_ALLOWED_OP_BITS: u32 = FUTEX_CMD_MASK | FUTEX_PRIVATE_FLAG | FUTEX_CLOCK_REALTIME;
const NANOS_PER_SECOND: u64 = 1_000_000_000;

macro_rules! smros_linux_futex_command_supported_body {
    ($command:expr) => {{
        $command == 0u32 || $command == 1u32 || $command == 9u32 || $command == 10u32
    }};
}

macro_rules! smros_linux_futex_realtime_allowed_body {
    ($command:expr, $realtime:expr) => {{
        !$realtime || $command == 9u32
    }};
}

macro_rules! smros_linux_futex_bitset_matches_body {
    ($waiter:expr, $requested:expr) => {{
        $waiter != 0 && $requested != 0 && ($waiter & $requested) != 0
    }};
}

macro_rules! smros_linux_futex_deadline_expired_body {
    ($now:expr, $deadline:expr) => {{
        $now >= $deadline
    }};
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FutexCommand {
    Wait,
    Wake,
    WaitBitset,
    WakeBitset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DecodedFutexOp {
    pub command: FutexCommand,
    pub private: bool,
    pub realtime: bool,
}

pub(crate) fn decode_futex_op(op: u32) -> Option<DecodedFutexOp> {
    if op & !FUTEX_ALLOWED_OP_BITS != 0 {
        return None;
    }
    let command_number = op & FUTEX_CMD_MASK;
    if !smros_linux_futex_command_supported_body!(command_number) {
        return None;
    }
    let command = match command_number {
        FUTEX_WAIT => FutexCommand::Wait,
        FUTEX_WAKE => FutexCommand::Wake,
        FUTEX_WAIT_BITSET => FutexCommand::WaitBitset,
        FUTEX_WAKE_BITSET => FutexCommand::WakeBitset,
        _ => return None,
    };
    let realtime = op & FUTEX_CLOCK_REALTIME != 0;
    if !smros_linux_futex_realtime_allowed_body!(command_number, realtime) {
        return None;
    }
    Some(DecodedFutexOp {
        command,
        private: op & FUTEX_PRIVATE_FLAG != 0,
        realtime,
    })
}

pub(crate) fn futex_address_valid(address: usize) -> bool {
    address != 0
        && address % core::mem::align_of::<u32>() == 0
        && address.checked_add(core::mem::size_of::<u32>()).is_some()
}

pub(crate) fn futex_shared_key(pfn: u64, address: usize) -> usize {
    FUTEX_SHARED_KEY_TAG
        | ((pfn as usize).wrapping_mul(0x1000) & !FUTEX_SHARED_KEY_TAG)
        | (address & 0xfff)
}

pub(crate) fn futex_bitset_valid(bitset: u32) -> bool {
    bitset != 0
}

pub(crate) fn futex_timespec_valid(seconds: i64, nanoseconds: i64) -> bool {
    seconds >= 0 && (0..NANOS_PER_SECOND as i64).contains(&nanoseconds)
}

pub(crate) fn futex_wait_value_matches(observed: u32, expected: u32) -> bool {
    observed == expected
}

pub(crate) fn futex_timespec_to_ticks_ceil(
    seconds: i64,
    nanoseconds: i64,
    tick_nanoseconds: u64,
) -> Option<u64> {
    if tick_nanoseconds == 0 || !futex_timespec_valid(seconds, nanoseconds) {
        return None;
    }
    let total_nanoseconds = (seconds as u64)
        .checked_mul(NANOS_PER_SECOND)?
        .checked_add(nanoseconds as u64)?;
    let ticks = total_nanoseconds / tick_nanoseconds;
    if total_nanoseconds % tick_nanoseconds == 0 {
        Some(ticks)
    } else {
        ticks.checked_add(1)
    }
}

pub(crate) fn futex_relative_deadline(
    now: u64,
    seconds: i64,
    nanoseconds: i64,
    tick_nanoseconds: u64,
) -> Option<u64> {
    let duration_ticks = futex_timespec_to_ticks_ceil(seconds, nanoseconds, tick_nanoseconds)?;
    let phase_guard = if seconds == 0 && nanoseconds == 0 {
        0
    } else {
        1
    };
    now.checked_add(duration_ticks)?.checked_add(phase_guard)
}

pub(crate) fn futex_realtime_deadline_ticks(
    seconds: i64,
    nanoseconds: i64,
    realtime_offset_nanoseconds: i64,
    tick_nanoseconds: u64,
) -> Option<u64> {
    if tick_nanoseconds == 0 || !futex_timespec_valid(seconds, nanoseconds) {
        return None;
    }
    let realtime_nanoseconds = i128::from(seconds)
        .checked_mul(i128::from(NANOS_PER_SECOND))?
        .checked_add(i128::from(nanoseconds))?;
    let monotonic_nanoseconds = realtime_nanoseconds
        .checked_sub(i128::from(realtime_offset_nanoseconds))?;
    let monotonic_nanoseconds = if monotonic_nanoseconds <= 0 {
        0
    } else {
        u128::try_from(monotonic_nanoseconds).ok()?
    };
    let tick_nanoseconds = u128::from(tick_nanoseconds);
    let ticks = monotonic_nanoseconds / tick_nanoseconds;
    ticks
        .checked_add(u128::from(monotonic_nanoseconds % tick_nanoseconds != 0))
        .and_then(|ticks| u64::try_from(ticks).ok())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FutexClock {
    Monotonic,
    Realtime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FutexDeadline {
    pub ticks: u64,
    pub clock: FutexClock,
}

pub(crate) fn futex_deadline_from_timeout(
    command: FutexCommand,
    realtime: bool,
    now_monotonic: u64,
    seconds: i64,
    nanoseconds: i64,
    tick_nanoseconds: u64,
    realtime_offset_nanoseconds: i64,
) -> Option<FutexDeadline> {
    match (command, realtime) {
        (FutexCommand::Wait, false) => Some(FutexDeadline {
            ticks: futex_relative_deadline(now_monotonic, seconds, nanoseconds, tick_nanoseconds)?,
            clock: FutexClock::Monotonic,
        }),
        (FutexCommand::WaitBitset, false) => Some(FutexDeadline {
            ticks: futex_timespec_to_ticks_ceil(seconds, nanoseconds, tick_nanoseconds)?,
            clock: FutexClock::Monotonic,
        }),
        (FutexCommand::WaitBitset, true) => Some(FutexDeadline {
            ticks: futex_realtime_deadline_ticks(
                seconds,
                nanoseconds,
                realtime_offset_nanoseconds,
                tick_nanoseconds,
            )?,
            clock: FutexClock::Monotonic,
        }),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FutexWaitOutcome {
    Waiting,
    Woken,
    TimedOut,
    Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FutexWaiter {
    pub address: usize,
    pub bitset: u32,
    pub tid: usize,
    pub scheduler_thread: usize,
    pub deadline: Option<FutexDeadline>,
    pub sequence: u64,
    pub outcome: FutexWaitOutcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FutexQueueError {
    Capacity,
    Duplicate,
    Exhausted,
    Invalid,
}

pub(crate) struct FutexQueue<const N: usize> {
    waiters: [Option<FutexWaiter>; N],
    next_sequence: u64,
    sequence_exhausted: bool,
}

impl<const N: usize> FutexQueue<N> {
    pub(crate) const fn new() -> Self {
        Self {
            waiters: [None; N],
            next_sequence: 0,
            sequence_exhausted: false,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.waiters
            .iter()
            .filter(|waiter| waiter.is_some())
            .count()
    }

    pub(crate) fn push(&mut self, mut waiter: FutexWaiter) -> Result<(), FutexQueueError> {
        if !futex_address_valid(waiter.address)
            || !futex_bitset_valid(waiter.bitset)
            || waiter.tid == 0
            || waiter.outcome != FutexWaitOutcome::Waiting
        {
            return Err(FutexQueueError::Invalid);
        }
        if self.waiters.iter().flatten().any(|current| {
            current.tid == waiter.tid && current.scheduler_thread == waiter.scheduler_thread
        }) {
            return Err(FutexQueueError::Duplicate);
        }
        if self.sequence_exhausted {
            return Err(FutexQueueError::Exhausted);
        }
        let slot = self
            .waiters
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(FutexQueueError::Capacity)?;
        waiter.sequence = self.next_sequence;
        match self.next_sequence.checked_add(1) {
            Some(next) => self.next_sequence = next,
            None => self.sequence_exhausted = true,
        }
        *slot = Some(waiter);
        Ok(())
    }

    pub(crate) fn wake(
        &mut self,
        address: usize,
        count: usize,
        bitset: u32,
    ) -> [Option<(usize, usize)>; N] {
        let mut identities = [None; N];
        if count == 0 || !futex_bitset_valid(bitset) {
            return identities;
        }
        let limit = core::cmp::min(count, N);
        let mut selected_count = 0usize;
        while selected_count < limit {
            let Some(index) = self.oldest_matching(|waiter| {
                waiter.outcome == FutexWaitOutcome::Waiting
                    && waiter.address == address
                    && smros_linux_futex_bitset_matches_body!(waiter.bitset, bitset)
            }) else {
                break;
            };
            let waiter = self.waiters[index].as_mut().expect("selected futex waiter");
            waiter.outcome = FutexWaitOutcome::Woken;
            identities[selected_count] = Some((waiter.tid, waiter.scheduler_thread));
            selected_count += 1;
        }
        identities
    }

    #[allow(dead_code)]
    pub(crate) fn expire(
        &mut self,
        now_monotonic: u64,
        now_realtime: u64,
    ) -> [Option<(usize, usize)>; N] {
        let mut identities = [None; N];
        let mut selected_count = 0usize;
        while selected_count < N {
            let Some(index) = self.oldest_matching(|waiter| {
                waiter.outcome == FutexWaitOutcome::Waiting
                    && waiter
                        .deadline
                        .is_some_and(|deadline| match deadline.clock {
                            FutexClock::Monotonic => {
                                smros_linux_futex_deadline_expired_body!(
                                    now_monotonic,
                                    deadline.ticks
                                )
                            }
                            FutexClock::Realtime => {
                                smros_linux_futex_deadline_expired_body!(
                                    now_realtime,
                                    deadline.ticks
                                )
                            }
                        })
            }) else {
                break;
            };
            let waiter = self.waiters[index].as_mut().expect("selected futex waiter");
            waiter.outcome = FutexWaitOutcome::TimedOut;
            identities[selected_count] = Some((waiter.tid, waiter.scheduler_thread));
            selected_count += 1;
        }
        identities
    }

    pub(crate) fn expire_one(
        &mut self,
        now_monotonic: u64,
        now_realtime: u64,
    ) -> Option<(usize, usize)> {
        let index = self.oldest_matching(|waiter| {
            waiter.outcome == FutexWaitOutcome::Waiting
                && waiter.deadline.is_some_and(|deadline| match deadline.clock {
                    FutexClock::Monotonic => smros_linux_futex_deadline_expired_body!(
                        now_monotonic,
                        deadline.ticks
                    ),
                    FutexClock::Realtime => smros_linux_futex_deadline_expired_body!(
                        now_realtime,
                        deadline.ticks
                    ),
                })
        })?;
        let waiter = self.waiters[index].as_mut().expect("selected futex waiter");
        waiter.outcome = FutexWaitOutcome::TimedOut;
        Some((waiter.tid, waiter.scheduler_thread))
    }

    pub(crate) fn interrupt(&mut self, tid: usize, scheduler_thread: usize) -> bool {
        let Some(waiter) = self.waiters.iter_mut().flatten().find(|waiter| {
            waiter.tid == tid
                && waiter.scheduler_thread == scheduler_thread
                && waiter.outcome == FutexWaitOutcome::Waiting
        }) else {
            return false;
        };
        waiter.outcome = FutexWaitOutcome::Interrupted;
        true
    }

    pub(crate) fn take_outcome(
        &mut self,
        tid: usize,
        scheduler_thread: usize,
    ) -> Option<FutexWaitOutcome> {
        let slot = self.waiters.iter_mut().find(|slot| {
            slot.is_some_and(|waiter| {
                waiter.tid == tid
                    && waiter.scheduler_thread == scheduler_thread
                    && waiter.outcome != FutexWaitOutcome::Waiting
            })
        })?;
        slot.take().map(|waiter| waiter.outcome)
    }

    pub(crate) fn remove(&mut self, tid: usize, scheduler_thread: usize) -> bool {
        let Some(slot) = self.waiters.iter_mut().find(|slot| {
            slot.is_some_and(|waiter| {
                waiter.tid == tid && waiter.scheduler_thread == scheduler_thread
            })
        }) else {
            return false;
        };
        *slot = None;
        true
    }

    pub(crate) fn remove_task(&mut self, tid: usize, scheduler_thread: usize) -> usize {
        let mut removed = 0usize;
        for slot in &mut self.waiters {
            if slot.is_some_and(|waiter| {
                waiter.tid == tid && waiter.scheduler_thread == scheduler_thread
            }) {
                *slot = None;
                removed += 1;
            }
        }
        removed
    }

    pub(crate) fn reset(&mut self) -> usize {
        let drained = self.len();
        self.waiters.fill(None);
        self.next_sequence = 0;
        self.sequence_exhausted = false;
        drained
    }

    fn oldest_matching(&self, predicate: impl Fn(FutexWaiter) -> bool) -> Option<usize> {
        let mut selected = None;
        let mut selected_sequence = u64::MAX;
        for (index, waiter) in self.waiters.iter().enumerate() {
            let Some(waiter) = waiter else {
                continue;
            };
            if predicate(*waiter) && (selected.is_none() || waiter.sequence < selected_sequence) {
                selected = Some(index);
                selected_sequence = waiter.sequence;
            }
        }
        selected
    }
}
