//! Lightweight compatibility kernel objects.
//!
//! This module backs syscall interfaces whose full kernel subsystem is not
//! present yet. The objects still have real handle lifetime, signal bits,
//! properties, and optional byte queues, so interface-level tests do not rely
//! on silent fallthrough behavior.

#![allow(dead_code)]
#![allow(static_mut_refs)]

use super::{object_logic, HandleValue, ObjectType, ZxError, ZxResult, INVALID_HANDLE};
use alloc::collections::VecDeque;
use alloc::vec::Vec;

const COMPAT_HANDLE_START: u32 = 0x8000_0000;
const MAX_COMPAT_OBJECTS: usize = 256;
const MAX_COMPAT_QUEUE_BYTES: usize = 65536;

#[derive(Clone)]
pub struct CompatObject {
    pub handle: HandleValue,
    pub obj_type: ObjectType,
    pub peer: Option<HandleValue>,
    pub signals: u32,
    pub property_value: u64,
    pub queue: VecDeque<u8>,
    pub closed: bool,
}

impl CompatObject {
    fn new(handle: HandleValue, obj_type: ObjectType, peer: Option<HandleValue>) -> Self {
        Self {
            handle,
            obj_type,
            peer,
            signals: 0,
            property_value: 0,
            queue: VecDeque::new(),
            closed: false,
        }
    }
}

pub struct CompatObjectTable {
    objects: Vec<CompatObject>,
    next_handle: u32,
}

impl CompatObjectTable {
    pub const fn new() -> Self {
        Self {
            objects: Vec::new(),
            next_handle: COMPAT_HANDLE_START,
        }
    }

    fn alloc_handle(&mut self) -> Option<HandleValue> {
        if self.next_handle == INVALID_HANDLE {
            return None;
        }

        let handle = self.next_handle;
        self.next_handle = self.next_handle.checked_add(1)?;

        if object_logic::handle_is_valid(handle, INVALID_HANDLE) {
            Some(HandleValue(handle))
        } else {
            None
        }
    }

    pub fn create(&mut self, obj_type: ObjectType) -> ZxResult<HandleValue> {
        if self.objects.len() >= MAX_COMPAT_OBJECTS {
            return Err(ZxError::ErrNoMemory);
        }

        let handle = self.alloc_handle().ok_or(ZxError::ErrNoMemory)?;
        self.objects.push(CompatObject::new(handle, obj_type, None));
        Ok(handle)
    }

    pub fn create_pair(&mut self, obj_type: ObjectType) -> ZxResult<(HandleValue, HandleValue)> {
        if self.objects.len() + 2 > MAX_COMPAT_OBJECTS {
            return Err(ZxError::ErrNoMemory);
        }

        let left = self.alloc_handle().ok_or(ZxError::ErrNoMemory)?;
        let right = self.alloc_handle().ok_or(ZxError::ErrNoMemory)?;
        self.objects
            .push(CompatObject::new(left, obj_type, Some(right)));
        self.objects
            .push(CompatObject::new(right, obj_type, Some(left)));
        Ok((left, right))
    }

    pub fn contains(&self, handle: HandleValue) -> bool {
        self.objects
            .iter()
            .any(|object| object.handle == handle && !object.closed)
    }

    pub fn object_type(&self, handle: HandleValue) -> Option<ObjectType> {
        self.objects
            .iter()
            .find(|object| object.handle == handle && !object.closed)
            .map(|object| object.obj_type)
    }

    pub fn signal_mask(&self, handle: HandleValue) -> Option<u32> {
        let object_type = self.object_type(handle)?;
        let mask = match object_type {
            ObjectType::Event => crate::syscall::syscall_logic::event_signal_mask(),
            ObjectType::EventPair => crate::syscall::syscall_logic::eventpair_signal_mask(),
            _ => crate::syscall::syscall_logic::user_signal_mask(),
        };
        Some(mask)
    }

    pub fn close(&mut self, handle: HandleValue) -> bool {
        let Some(index) = self
            .objects
            .iter()
            .position(|object| object.handle == handle && !object.closed)
        else {
            return false;
        };

        let peer = self.objects[index].peer;
        self.objects[index].closed = true;
        self.objects[index].queue.clear();

        if let Some(peer) = peer {
            if let Some(peer_object) = self
                .objects
                .iter_mut()
                .find(|object| object.handle == peer && !object.closed)
            {
                peer_object.signals |= crate::kernel_objects::channel::CHANNEL_SIGNAL_PEER_CLOSED;
                peer_object.peer = None;
            }
        }

        self.objects.swap_remove(index);
        true
    }

    pub fn signals(&self, handle: HandleValue) -> Option<u32> {
        self.objects
            .iter()
            .find(|object| object.handle == handle && !object.closed)
            .map(|object| object.signals)
    }

    pub fn update_signals(
        &mut self,
        handle: HandleValue,
        clear_mask: u32,
        set_mask: u32,
    ) -> Option<u32> {
        self.objects
            .iter_mut()
            .find(|object| object.handle == handle && !object.closed)
            .map(|object| {
                object.signals = object_logic::signal_update(object.signals, clear_mask, set_mask);
                object.signals
            })
    }

    pub fn update_signals_checked(
        &mut self,
        handle: HandleValue,
        clear_mask: u32,
        set_mask: u32,
    ) -> ZxResult<u32> {
        let allowed_mask = self.signal_mask(handle).ok_or(ZxError::ErrNotFound)?;
        if !crate::syscall::syscall_logic::signal_mask_allowed(clear_mask, set_mask, allowed_mask) {
            return Err(ZxError::ErrInvalidArgs);
        }
        self.update_signals(handle, clear_mask, set_mask)
            .ok_or(ZxError::ErrNotFound)
    }

    pub fn signal_peer(
        &mut self,
        handle: HandleValue,
        clear_mask: u32,
        set_mask: u32,
    ) -> ZxResult<u32> {
        let (peer, allowed_mask) = self
            .objects
            .iter()
            .find(|object| object.handle == handle && !object.closed)
            .map(|object| {
                let allowed_mask = match object.obj_type {
                    ObjectType::EventPair => crate::syscall::syscall_logic::eventpair_signal_mask(),
                    _ => crate::syscall::syscall_logic::user_signal_mask(),
                };
                (object.peer, allowed_mask)
            })
            .ok_or(ZxError::ErrNotFound)?;
        if !crate::syscall::syscall_logic::signal_mask_allowed(clear_mask, set_mask, allowed_mask) {
            return Err(ZxError::ErrInvalidArgs);
        }
        let peer = peer.ok_or(ZxError::ErrPeerClosed)?;

        self.update_signals(peer, clear_mask, set_mask)
            .ok_or(ZxError::ErrPeerClosed)
    }

    pub fn property(&self, handle: HandleValue) -> Option<u64> {
        self.objects
            .iter()
            .find(|object| object.handle == handle && !object.closed)
            .map(|object| object.property_value)
    }

    pub fn set_property(&mut self, handle: HandleValue, value: u64) -> bool {
        if let Some(object) = self
            .objects
            .iter_mut()
            .find(|object| object.handle == handle && !object.closed)
        {
            object.property_value = value;
            true
        } else {
            false
        }
    }

    pub fn write_bytes(&mut self, handle: HandleValue, bytes: &[u8]) -> ZxResult<usize> {
        let target = {
            let object = self
                .objects
                .iter()
                .find(|object| object.handle == handle && !object.closed)
                .ok_or(ZxError::ErrNotFound)?;
            object.peer.unwrap_or(handle)
        };

        let object = self
            .objects
            .iter_mut()
            .find(|object| object.handle == target && !object.closed)
            .ok_or(ZxError::ErrPeerClosed)?;

        if object.queue.len().saturating_add(bytes.len()) > MAX_COMPAT_QUEUE_BYTES {
            return Err(ZxError::ErrNoMemory);
        }

        for byte in bytes {
            object.queue.push_back(*byte);
        }
        if !bytes.is_empty() {
            object.signals |= crate::kernel_objects::channel::CHANNEL_SIGNAL_READABLE;
        }

        Ok(bytes.len())
    }

    pub fn read_bytes(&mut self, handle: HandleValue, out: &mut [u8]) -> ZxResult<usize> {
        let object = self
            .objects
            .iter_mut()
            .find(|object| object.handle == handle && !object.closed)
            .ok_or(ZxError::ErrNotFound)?;

        if object.queue.is_empty() {
            return Err(ZxError::ErrShouldWait);
        }

        let mut read = 0usize;
        while read < out.len() {
            let Some(byte) = object.queue.pop_front() else {
                break;
            };
            out[read] = byte;
            read += 1;
        }

        if object.queue.is_empty() {
            object.signals &= !crate::kernel_objects::channel::CHANNEL_SIGNAL_READABLE;
        }

        Ok(read)
    }
}

static mut COMPAT_OBJECT_TABLE: CompatObjectTable = CompatObjectTable::new();

pub fn table() -> &'static mut CompatObjectTable {
    unsafe { &mut COMPAT_OBJECT_TABLE }
}

pub fn create_object(obj_type: ObjectType) -> ZxResult<HandleValue> {
    table().create(obj_type)
}

pub fn create_pair(obj_type: ObjectType) -> ZxResult<(HandleValue, HandleValue)> {
    table().create_pair(obj_type)
}

pub fn handle_known(handle: HandleValue) -> bool {
    table().contains(handle)
}

pub fn close_handle(handle: HandleValue) -> bool {
    table().close(handle)
}
