#![allow(dead_code)]
#![allow(static_mut_refs)]
//! Zircon futex wait queue model.
//!
//! Zircon futexes are addressed by aligned userspace pointers rather than
//! handles. SMROS keeps a small table of address-keyed queues so futex syscalls
//! have real validation, owner bookkeeping, wake, and requeue behavior.

use super::{futex_logic, ZxError, ZxResult};

const FUTEX_ALIGN: usize = core::mem::align_of::<i32>();
const MAX_FUTEX_ENTRIES: usize = 32;

#[derive(Clone, Copy)]
pub struct FutexEntry {
    pub value_ptr: usize,
    pub waiters: u32,
    pub owner: u32,
    pub active: bool,
}

impl FutexEntry {
    pub const fn empty() -> Self {
        Self {
            value_ptr: 0,
            waiters: 0,
            owner: 0,
            active: false,
        }
    }

    fn activate(&mut self, value_ptr: usize) {
        self.value_ptr = value_ptr;
        self.waiters = 0;
        self.owner = 0;
        self.active = true;
    }

    fn clear(&mut self) {
        self.value_ptr = 0;
        self.waiters = 0;
        self.owner = 0;
        self.active = false;
    }
}

pub struct FutexTable {
    entries: [FutexEntry; MAX_FUTEX_ENTRIES],
}

impl FutexTable {
    pub const fn new() -> Self {
        Self {
            entries: [FutexEntry::empty(); MAX_FUTEX_ENTRIES],
        }
    }

    fn index(&self, value_ptr: usize) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.active && entry.value_ptr == value_ptr)
    }

    fn free_index(&self) -> Option<usize> {
        self.entries.iter().position(|entry| !entry.active)
    }

    fn ensure_index(&mut self, value_ptr: usize) -> ZxResult<usize> {
        if let Some(index) = self.index(value_ptr) {
            return Ok(index);
        }

        let index = self.free_index().ok_or(ZxError::ErrNoMemory)?;
        self.entries[index].activate(value_ptr);
        Ok(index)
    }

    fn cleanup_index(&mut self, index: usize) {
        if self.entries[index].waiters == 0 && self.entries[index].owner == 0 {
            self.entries[index].clear();
        }
    }

    pub fn validate_ptr(value_ptr: usize) -> ZxResult {
        if futex_logic::ptr_valid(value_ptr, FUTEX_ALIGN) {
            Ok(())
        } else {
            Err(ZxError::ErrInvalidArgs)
        }
    }

    fn observed_value(value_ptr: usize) -> i32 {
        unsafe { core::ptr::read_volatile(value_ptr as *const i32) }
    }

    pub fn wait(
        &mut self,
        value_ptr: usize,
        current_value: i32,
        new_owner: u32,
        deadline: u64,
    ) -> ZxResult {
        Self::validate_ptr(value_ptr)?;
        let observed = Self::observed_value(value_ptr);
        if !futex_logic::value_matches(observed, current_value) {
            return Err(ZxError::ErrBadState);
        }

        let index = self.ensure_index(value_ptr)?;
        self.entries[index].waiters = futex_logic::saturating_add(self.entries[index].waiters, 1);
        self.entries[index].owner = new_owner;

        if deadline == 0 {
            self.entries[index].waiters = self.entries[index].waiters.saturating_sub(1);
            self.cleanup_index(index);
            Err(ZxError::ErrTimedOut)
        } else {
            Ok(())
        }
    }

    pub fn wake(&mut self, value_ptr: usize, count: u32) -> ZxResult<u32> {
        Self::validate_ptr(value_ptr)?;
        let Some(index) = self.index(value_ptr) else {
            return Ok(0);
        };

        let actual = futex_logic::min_count(count, self.entries[index].waiters);
        self.entries[index].waiters -= actual;
        if self.entries[index].waiters == 0 {
            self.entries[index].owner = 0;
        }
        self.cleanup_index(index);
        Ok(actual)
    }

    pub fn requeue(
        &mut self,
        value_ptr: usize,
        wake_count: u32,
        current_value: i32,
        requeue_ptr: usize,
        requeue_count: u32,
        new_requeue_owner: u32,
    ) -> ZxResult<(u32, u32)> {
        Self::validate_ptr(value_ptr)?;
        Self::validate_ptr(requeue_ptr)?;
        if value_ptr == requeue_ptr {
            return Err(ZxError::ErrInvalidArgs);
        }

        let observed = Self::observed_value(value_ptr);
        if !futex_logic::value_matches(observed, current_value) {
            return Err(ZxError::ErrBadState);
        }

        let Some(source_index) = self.index(value_ptr) else {
            return Ok((0, 0));
        };

        let wake_actual = futex_logic::min_count(wake_count, self.entries[source_index].waiters);
        self.entries[source_index].waiters -= wake_actual;
        let requeue_actual =
            futex_logic::min_count(requeue_count, self.entries[source_index].waiters);
        self.entries[source_index].waiters -= requeue_actual;

        if requeue_actual != 0 {
            let target_index = self.ensure_index(requeue_ptr)?;
            self.entries[target_index].waiters =
                futex_logic::saturating_add(self.entries[target_index].waiters, requeue_actual);
            self.entries[target_index].owner = new_requeue_owner;
        }

        if self.entries[source_index].waiters == 0 {
            self.entries[source_index].owner = 0;
        }
        self.cleanup_index(source_index);
        Ok((wake_actual, requeue_actual))
    }

    pub fn get_owner(&self, value_ptr: usize) -> ZxResult<u32> {
        Self::validate_ptr(value_ptr)?;
        Ok(self
            .index(value_ptr)
            .map(|index| self.entries[index].owner)
            .unwrap_or(0))
    }

    pub fn waiter_count(&self, value_ptr: usize) -> ZxResult<u32> {
        Self::validate_ptr(value_ptr)?;
        Ok(self
            .index(value_ptr)
            .map(|index| self.entries[index].waiters)
            .unwrap_or(0))
    }
}

static mut FUTEX_TABLE: FutexTable = FutexTable::new();

pub fn futex_table() -> &'static mut FutexTable {
    unsafe { &mut FUTEX_TABLE }
}
