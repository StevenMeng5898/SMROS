macro_rules! smros_linux_record_lock_ranges_overlap_body {
    ($left_start:expr, $left_end:expr, $right_start:expr, $right_end:expr) => {{
        $left_start < $right_end && $right_start < $left_end
    }};
}

macro_rules! smros_linux_record_lock_types_conflict_body {
    ($left_is_write:expr, $right_is_write:expr) => {{
        $left_is_write || $right_is_write
    }};
}

pub(crate) const LINUX_RECORD_LOCK_END_OF_FILE: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxRecordLockRangeError {
    Invalid,
    Overflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxRecordLockRange {
    pub start: u64,
    pub end: u64,
}

impl LinuxRecordLockRange {
    pub(crate) fn finite(start: u64, end: u64) -> Option<Self> {
        (start < end && end != LINUX_RECORD_LOCK_END_OF_FILE).then_some(Self { start, end })
    }

    pub(crate) const fn to_eof(start: u64) -> Self {
        Self {
            start,
            end: LINUX_RECORD_LOCK_END_OF_FILE,
        }
    }
}

pub(crate) fn normalize_linux_record_lock_range(
    whence: i16,
    l_start: i64,
    l_len: i64,
    cursor: u64,
    file_size: u64,
) -> Result<LinuxRecordLockRange, LinuxRecordLockRangeError> {
    let base = match whence {
        0 => 0,
        1 => i64::try_from(cursor).map_err(|_| LinuxRecordLockRangeError::Overflow)?,
        2 => i64::try_from(file_size).map_err(|_| LinuxRecordLockRangeError::Overflow)?,
        _ => return Err(LinuxRecordLockRangeError::Invalid),
    };
    let anchor = base
        .checked_add(l_start)
        .ok_or(LinuxRecordLockRangeError::Overflow)?;
    if anchor < 0 {
        return Err(LinuxRecordLockRangeError::Invalid);
    }

    if l_len == 0 {
        return Ok(LinuxRecordLockRange::to_eof(anchor as u64));
    }

    let endpoint = anchor
        .checked_add(l_len)
        .ok_or(LinuxRecordLockRangeError::Overflow)?;
    if endpoint < 0 {
        return Err(LinuxRecordLockRangeError::Invalid);
    }

    let (start, end) = if l_len > 0 {
        (anchor as u64, endpoint as u64)
    } else {
        (endpoint as u64, anchor as u64)
    };
    LinuxRecordLockRange::finite(start, end).ok_or(LinuxRecordLockRangeError::Invalid)
}

pub(crate) fn linux_record_lock_ranges_overlap(
    left: LinuxRecordLockRange,
    right: LinuxRecordLockRange,
) -> bool {
    smros_linux_record_lock_ranges_overlap_body!(left.start, left.end, right.start, right.end)
}

pub(crate) fn linux_record_lock_types_conflict(left_is_write: bool, right_is_write: bool) -> bool {
    smros_linux_record_lock_types_conflict_body!(left_is_write, right_is_write)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxRecordLockKind {
    Read,
    Write,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxRecordLock {
    pub file_id: u64,
    pub owner: usize,
    pub kind: LinuxRecordLockKind,
    pub range: LinuxRecordLockRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxRecordLockTableError {
    Capacity,
}

#[derive(Clone, Copy)]
pub(crate) struct LinuxRecordLockTable<const N: usize> {
    records: [Option<LinuxRecordLock>; N],
}

impl<const N: usize> LinuxRecordLockTable<N> {
    pub(crate) const fn new() -> Self {
        Self { records: [None; N] }
    }

    pub(crate) fn first_conflict(
        &self,
        file_id: u64,
        owner: usize,
        kind: LinuxRecordLockKind,
        range: LinuxRecordLockRange,
    ) -> Option<LinuxRecordLock> {
        self.records.iter().flatten().copied().find(|record| {
            record.file_id == file_id
                && record.owner != owner
                && linux_record_lock_ranges_overlap(record.range, range)
                && linux_record_lock_types_conflict(
                    record.kind == LinuxRecordLockKind::Write,
                    kind == LinuxRecordLockKind::Write,
                )
        })
    }

    pub(crate) fn set(
        &mut self,
        file_id: u64,
        owner: usize,
        kind: LinuxRecordLockKind,
        range: LinuxRecordLockRange,
    ) -> Result<(), LinuxRecordLockTableError> {
        let mut replacement = range;
        loop {
            let before = replacement;
            for record in self.records.iter().flatten() {
                if record.file_id == file_id
                    && record.owner == owner
                    && record.kind == kind
                    && ranges_touch_or_overlap(record.range, replacement)
                {
                    replacement.start = core::cmp::min(replacement.start, record.range.start);
                    replacement.end = core::cmp::max(replacement.end, record.range.end);
                }
            }
            if replacement == before {
                break;
            }
        }

        let mut candidate = Self::new();
        for record in self.records.iter().flatten().copied() {
            if record.file_id != file_id || record.owner != owner {
                candidate.push(record)?;
                continue;
            }
            if record.kind == kind && ranges_touch_or_overlap(record.range, replacement) {
                continue;
            }
            if !linux_record_lock_ranges_overlap(record.range, replacement) {
                candidate.push(record)?;
                continue;
            }
            candidate.push_left_piece(record, replacement)?;
            candidate.push_right_piece(record, replacement)?;
        }
        candidate.push(LinuxRecordLock {
            file_id,
            owner,
            kind,
            range: replacement,
        })?;
        candidate.sort_and_coalesce();
        *self = candidate;
        Ok(())
    }

    pub(crate) fn unlock(
        &mut self,
        file_id: u64,
        owner: usize,
        range: LinuxRecordLockRange,
    ) -> Result<(), LinuxRecordLockTableError> {
        let mut candidate = Self::new();
        for record in self.records.iter().flatten().copied() {
            if record.file_id != file_id
                || record.owner != owner
                || !linux_record_lock_ranges_overlap(record.range, range)
            {
                candidate.push(record)?;
                continue;
            }
            candidate.push_left_piece(record, range)?;
            candidate.push_right_piece(record, range)?;
        }
        candidate.sort_and_coalesce();
        *self = candidate;
        Ok(())
    }

    pub(crate) fn release_owner_file(&mut self, owner: usize, file_id: u64) {
        self.retain(|record| record.owner != owner || record.file_id != file_id);
    }

    pub(crate) fn release_owner(&mut self, owner: usize) {
        self.retain(|record| record.owner != owner);
    }

    pub(crate) const fn snapshot(&self) -> [Option<LinuxRecordLock>; N] {
        self.records
    }

    pub(crate) fn reset(&mut self) {
        self.records = [None; N];
    }

    fn push(&mut self, record: LinuxRecordLock) -> Result<(), LinuxRecordLockTableError> {
        let slot = self
            .records
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(LinuxRecordLockTableError::Capacity)?;
        *slot = Some(record);
        Ok(())
    }

    fn push_left_piece(
        &mut self,
        record: LinuxRecordLock,
        removed: LinuxRecordLockRange,
    ) -> Result<(), LinuxRecordLockTableError> {
        if record.range.start < removed.start {
            self.push(LinuxRecordLock {
                range: LinuxRecordLockRange::finite(record.range.start, removed.start)
                    .expect("overlap leaves a finite left record-lock range"),
                ..record
            })?;
        }
        Ok(())
    }

    fn push_right_piece(
        &mut self,
        record: LinuxRecordLock,
        removed: LinuxRecordLockRange,
    ) -> Result<(), LinuxRecordLockTableError> {
        if removed.end < record.range.end {
            let range = if record.range.end == LINUX_RECORD_LOCK_END_OF_FILE {
                LinuxRecordLockRange::to_eof(removed.end)
            } else {
                LinuxRecordLockRange::finite(removed.end, record.range.end)
                    .expect("overlap leaves a finite right record-lock range")
            };
            self.push(LinuxRecordLock { range, ..record })?;
        }
        Ok(())
    }

    fn retain(&mut self, keep: impl Fn(LinuxRecordLock) -> bool) {
        let mut retained = [None; N];
        let mut next = 0usize;
        for record in self.records.iter().flatten().copied() {
            if keep(record) {
                retained[next] = Some(record);
                next += 1;
            }
        }
        self.records = retained;
    }

    fn sort_and_coalesce(&mut self) {
        for left in 0..N {
            for right in (left + 1)..N {
                let should_swap = match (self.records[left], self.records[right]) {
                    (None, Some(_)) => true,
                    (Some(left_record), Some(right_record)) => {
                        record_less(right_record, left_record)
                    }
                    _ => false,
                };
                if should_swap {
                    self.records.swap(left, right);
                }
            }
        }

        let mut coalesced: [Option<LinuxRecordLock>; N] = [None; N];
        let mut count = 0usize;
        for record in self.records.iter().flatten().copied() {
            if count != 0 {
                let previous = coalesced[count - 1]
                    .as_mut()
                    .expect("coalesced prefix contains record locks");
                if previous.file_id == record.file_id
                    && previous.owner == record.owner
                    && previous.kind == record.kind
                    && ranges_touch_or_overlap(previous.range, record.range)
                {
                    previous.range.end = core::cmp::max(previous.range.end, record.range.end);
                    continue;
                }
            }
            coalesced[count] = Some(record);
            count += 1;
        }
        self.records = coalesced;
    }
}

fn ranges_touch_or_overlap(left: LinuxRecordLockRange, right: LinuxRecordLockRange) -> bool {
    linux_record_lock_ranges_overlap(left, right)
        || left.end == right.start
        || right.end == left.start
}

fn record_less(left: LinuxRecordLock, right: LinuxRecordLock) -> bool {
    let left_kind = match left.kind {
        LinuxRecordLockKind::Read => 0u8,
        LinuxRecordLockKind::Write => 1u8,
    };
    let right_kind = match right.kind {
        LinuxRecordLockKind::Read => 0u8,
        LinuxRecordLockKind::Write => 1u8,
    };
    (
        left.file_id,
        left.owner,
        left.range.start,
        left.range.end,
        left_kind,
    ) < (
        right.file_id,
        right.owner,
        right.range.start,
        right.range.end,
        right_kind,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxRecordLockWaitOutcome {
    Waiting,
    Woken,
    Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxRecordLockWaiter {
    pub file_id: u64,
    pub owner: usize,
    pub kind: LinuxRecordLockKind,
    pub range: LinuxRecordLockRange,
    pub tid: usize,
    pub scheduler_thread: usize,
    pub sequence: u64,
    pub outcome: LinuxRecordLockWaitOutcome,
}

impl LinuxRecordLockWaiter {
    pub(crate) const fn new(
        file_id: u64,
        owner: usize,
        kind: LinuxRecordLockKind,
        range: LinuxRecordLockRange,
        tid: usize,
        scheduler_thread: usize,
    ) -> Self {
        Self {
            file_id,
            owner,
            kind,
            range,
            tid,
            scheduler_thread,
            sequence: 0,
            outcome: LinuxRecordLockWaitOutcome::Waiting,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxRecordLockWaiterError {
    Capacity,
    Duplicate,
    Exhausted,
}

pub(crate) struct LinuxRecordLockState<const L: usize, const W: usize> {
    pub locks: LinuxRecordLockTable<L>,
    waiters: [Option<LinuxRecordLockWaiter>; W],
    next_sequence: u64,
}

impl<const L: usize, const W: usize> LinuxRecordLockState<L, W> {
    pub(crate) const fn new() -> Self {
        Self {
            locks: LinuxRecordLockTable::new(),
            waiters: [None; W],
            next_sequence: 0,
        }
    }

    pub(crate) fn push(
        &mut self,
        mut waiter: LinuxRecordLockWaiter,
    ) -> Result<(), LinuxRecordLockWaiterError> {
        if self.waiters.iter().flatten().any(|current| {
            current.tid == waiter.tid && current.scheduler_thread == waiter.scheduler_thread
        }) {
            return Err(LinuxRecordLockWaiterError::Duplicate);
        }
        let next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(LinuxRecordLockWaiterError::Exhausted)?;
        let slot = self
            .waiters
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(LinuxRecordLockWaiterError::Capacity)?;
        waiter.sequence = self.next_sequence;
        waiter.outcome = LinuxRecordLockWaitOutcome::Waiting;
        *slot = Some(waiter);
        self.next_sequence = next_sequence;
        Ok(())
    }

    pub(crate) fn wake_ready(&mut self) -> [Option<(usize, usize)>; W] {
        let mut identities = [None; W];
        let mut count = 0usize;
        loop {
            let mut selected: Option<(usize, u64)> = None;
            for (index, waiter) in self.waiters.iter().enumerate() {
                let Some(waiter) = waiter else {
                    continue;
                };
                if waiter.outcome != LinuxRecordLockWaitOutcome::Waiting
                    || self
                        .locks
                        .first_conflict(waiter.file_id, waiter.owner, waiter.kind, waiter.range)
                        .is_some()
                {
                    continue;
                }
                if selected
                    .map(|(_, sequence)| waiter.sequence < sequence)
                    .unwrap_or(true)
                {
                    selected = Some((index, waiter.sequence));
                }
            }
            let Some((index, _)) = selected else {
                break;
            };
            let waiter = self.waiters[index]
                .as_mut()
                .expect("selected record-lock waiter remains published");
            waiter.outcome = LinuxRecordLockWaitOutcome::Woken;
            identities[count] = Some((waiter.tid, waiter.scheduler_thread));
            count += 1;
        }
        identities
    }

    pub(crate) fn interrupt(&mut self, tid: usize, scheduler_thread: usize) -> bool {
        let Some(waiter) = self
            .waiters
            .iter_mut()
            .flatten()
            .find(|waiter| waiter.tid == tid && waiter.scheduler_thread == scheduler_thread)
        else {
            return false;
        };
        if waiter.outcome != LinuxRecordLockWaitOutcome::Waiting {
            return false;
        }
        waiter.outcome = LinuxRecordLockWaitOutcome::Interrupted;
        true
    }

    pub(crate) fn remove_task(&mut self, tid: usize, scheduler_thread: usize) -> usize {
        let mut removed = 0usize;
        for slot in &mut self.waiters {
            if slot
                .map(|waiter| waiter.tid == tid && waiter.scheduler_thread == scheduler_thread)
                .unwrap_or(false)
            {
                *slot = None;
                removed += 1;
            }
        }
        removed
    }

    pub(crate) fn take_outcome(
        &mut self,
        tid: usize,
        scheduler_thread: usize,
    ) -> Option<LinuxRecordLockWaitOutcome> {
        let slot = self.waiters.iter_mut().find(|slot| {
            slot.map(|waiter| {
                waiter.tid == tid
                    && waiter.scheduler_thread == scheduler_thread
                    && waiter.outcome != LinuxRecordLockWaitOutcome::Waiting
            })
            .unwrap_or(false)
        })?;
        slot.take().map(|waiter| waiter.outcome)
    }

    pub(crate) const fn waiter_snapshot(&self) -> [Option<LinuxRecordLockWaiter>; W] {
        self.waiters
    }

    pub(crate) fn reset(&mut self) {
        self.locks.reset();
        self.waiters = [None; W];
        self.next_sequence = 0;
    }
}
