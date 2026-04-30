//! Handle Table Implementation
//!
//! Manages kernel object handles for processes.

#![allow(dead_code)]

use super::{object_logic, types::*};
use core::sync::atomic::{AtomicU32, Ordering};

/// Handle entry in the handle table
#[derive(Clone)]
pub struct HandleEntry {
    /// Handle value
    pub handle: HandleValue,
    /// Object type
    pub obj_type: ObjectType,
    /// Rights granted to this handle
    pub rights: u32,
    /// Whether this handle is valid
    pub valid: bool,
}

/// Handle table for a process
pub struct HandleTable {
    entries: [HandleEntry; MAX_HANDLES_PER_PROCESS],
    next_handle: AtomicU32,
}

impl HandleTable {
    /// Create a new handle table
    pub const fn new() -> Self {
        const EMPTY_ENTRY: HandleEntry = HandleEntry {
            handle: HandleValue(INVALID_HANDLE),
            obj_type: ObjectType::Vmo,
            rights: 0,
            valid: false,
        };

        Self {
            entries: [EMPTY_ENTRY; MAX_HANDLES_PER_PROCESS],
            next_handle: AtomicU32::new(1),
        }
    }

    /// Add a new handle
    pub fn add(&mut self, obj_type: ObjectType, rights: u32) -> Option<HandleValue> {
        let handle_num = self.next_handle.fetch_add(1, Ordering::Relaxed);

        for i in 0..MAX_HANDLES_PER_PROCESS {
            if !self.entries[i].valid {
                self.entries[i] = HandleEntry {
                    handle: HandleValue(handle_num),
                    obj_type,
                    rights,
                    valid: true,
                };
                return Some(HandleValue(handle_num));
            }
        }

        None
    }

    /// Remove a handle
    pub fn remove(&mut self, handle: HandleValue) -> bool {
        for i in 0..MAX_HANDLES_PER_PROCESS {
            if self.entries[i].valid && self.entries[i].handle == handle {
                self.entries[i].valid = false;
                return true;
            }
        }
        false
    }

    /// Get handle rights
    pub fn get_rights(&self, handle: HandleValue) -> Option<u32> {
        for i in 0..MAX_HANDLES_PER_PROCESS {
            if self.entries[i].valid && self.entries[i].handle == handle {
                return Some(self.entries[i].rights);
            }
        }
        None
    }

    /// Duplicate a handle
    pub fn duplicate(&mut self, handle: HandleValue, rights: u32) -> Option<HandleValue> {
        for i in 0..MAX_HANDLES_PER_PROCESS {
            if self.entries[i].valid && self.entries[i].handle == handle {
                let new_rights = object_logic::intersect_rights(rights, self.entries[i].rights);
                return self.add(self.entries[i].obj_type, new_rights);
            }
        }
        None
    }
}
