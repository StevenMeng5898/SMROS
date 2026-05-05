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
        if !rights_are_valid(rights) {
            return None;
        }

        let handle_num = self.next_handle.fetch_add(1, Ordering::Relaxed);
        if !object_logic::handle_is_valid(handle_num, INVALID_HANDLE) {
            return None;
        }

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

    /// Add a specific handle value when importing an object managed by another table.
    pub fn add_existing(&mut self, handle: HandleValue, obj_type: ObjectType, rights: u32) -> bool {
        if !object_logic::handle_is_valid(handle.0, INVALID_HANDLE)
            || !rights_are_valid(rights)
            || self.contains(handle)
        {
            return false;
        }

        for i in 0..MAX_HANDLES_PER_PROCESS {
            if !self.entries[i].valid {
                self.entries[i] = HandleEntry {
                    handle,
                    obj_type,
                    rights,
                    valid: true,
                };
                return true;
            }
        }

        false
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

    /// Get handle object type.
    pub fn get_object_type(&self, handle: HandleValue) -> Option<ObjectType> {
        for i in 0..MAX_HANDLES_PER_PROCESS {
            if self.entries[i].valid && self.entries[i].handle == handle {
                return Some(self.entries[i].obj_type);
            }
        }
        None
    }

    /// Check whether a handle has all required rights.
    pub fn has_rights(&self, handle: HandleValue, required: u32) -> bool {
        self.get_rights(handle)
            .map(|rights| rights_contain(rights, required))
            .unwrap_or(false)
    }

    /// Check whether a handle exists in the table.
    pub fn contains(&self, handle: HandleValue) -> bool {
        self.get_rights(handle).is_some()
    }

    /// Duplicate a handle
    pub fn duplicate(&mut self, handle: HandleValue, rights: u32) -> Option<HandleValue> {
        for i in 0..MAX_HANDLES_PER_PROCESS {
            if self.entries[i].valid && self.entries[i].handle == handle {
                let existing_rights = self.entries[i].rights;
                if !object_logic::duplicate_rights_allowed(
                    existing_rights,
                    rights,
                    Rights::Duplicate as u32,
                    RIGHT_SAME_RIGHTS,
                    RIGHTS_ALL,
                ) {
                    return None;
                }
                let new_rights = if rights == RIGHT_SAME_RIGHTS {
                    existing_rights
                } else {
                    rights
                };
                let obj_type = self.entries[i].obj_type;
                return self.add(obj_type, new_rights);
            }
        }
        None
    }

    /// Replace a handle with a new one and remove the source on success.
    pub fn replace(&mut self, handle: HandleValue, rights: u32) -> Option<HandleValue> {
        for i in 0..MAX_HANDLES_PER_PROCESS {
            if self.entries[i].valid && self.entries[i].handle == handle {
                let existing_rights = self.entries[i].rights;
                if !object_logic::replace_rights_allowed(
                    existing_rights,
                    rights,
                    RIGHT_SAME_RIGHTS,
                    RIGHTS_ALL,
                ) {
                    return None;
                }
                let new_rights = if rights == RIGHT_SAME_RIGHTS {
                    existing_rights
                } else {
                    rights
                };
                let obj_type = self.entries[i].obj_type;
                let new_handle = self.add(obj_type, new_rights)?;
                if self.remove(handle) {
                    return Some(new_handle);
                }
                let _ = self.remove(new_handle);
                return None;
            }
        }
        None
    }
}
