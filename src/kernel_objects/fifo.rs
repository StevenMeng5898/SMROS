#![allow(dead_code)]
#![allow(static_mut_refs)]
//! Zircon FIFO kernel object.
//!
//! FIFOs are bidirectional endpoint pairs that move fixed-size elements. A
//! write to one endpoint appends whole elements to the peer endpoint's queue.

use super::{fifo_logic, object_logic, HandleValue, ZxError, ZxResult, INVALID_HANDLE};

const FIFO_HANDLE_START: u32 = 0x9100_0000;
const MAX_FIFO_OBJECTS: usize = 16;
pub const FIFO_MAX_ELEMS: usize = 64;
pub const FIFO_MAX_ELEM_SIZE: usize = 64;
pub const FIFO_BUFFER_SIZE: usize = FIFO_MAX_ELEMS * FIFO_MAX_ELEM_SIZE;
const FIFO_CREATE_OPTIONS_MASK: u32 = 0;

#[derive(Clone, Copy)]
pub struct FifoEndpoint {
    pub handle: HandleValue,
    pub peer: Option<HandleValue>,
    pub signals: u32,
    pub elem_count: usize,
    pub elem_size: usize,
    pub data: [u8; FIFO_BUFFER_SIZE],
    pub read_pos: usize,
    pub len: usize,
    pub active: bool,
}

impl FifoEndpoint {
    pub const fn empty() -> Self {
        Self {
            handle: HandleValue(INVALID_HANDLE),
            peer: None,
            signals: 0,
            elem_count: 0,
            elem_size: 0,
            data: [0; FIFO_BUFFER_SIZE],
            read_pos: 0,
            len: 0,
            active: false,
        }
    }

    fn activate(
        &mut self,
        handle: HandleValue,
        peer: HandleValue,
        elem_count: usize,
        elem_size: usize,
    ) {
        self.handle = handle;
        self.peer = Some(peer);
        self.signals = crate::kernel_objects::channel::CHANNEL_SIGNAL_WRITABLE;
        self.elem_count = elem_count;
        self.elem_size = elem_size;
        self.read_pos = 0;
        self.len = 0;
        self.active = true;
    }

    fn clear(&mut self) {
        self.handle = HandleValue(INVALID_HANDLE);
        self.peer = None;
        self.signals = 0;
        self.elem_count = 0;
        self.elem_size = 0;
        self.read_pos = 0;
        self.len = 0;
        self.active = false;
    }

    fn elem_index(&self, offset: usize) -> usize {
        fifo_logic::ring_index(self.read_pos, offset, self.elem_count)
    }

    fn data_index(&self, elem_offset: usize, byte_offset: usize) -> ZxResult<usize> {
        let elem_index = self.elem_index(elem_offset);
        let elem_start = elem_index
            .checked_mul(self.elem_size)
            .ok_or(ZxError::ErrOutOfRange)?;
        elem_start
            .checked_add(byte_offset)
            .filter(|index| *index < FIFO_BUFFER_SIZE)
            .ok_or(ZxError::ErrOutOfRange)
    }

    fn push_element(&mut self, elem: &[u8]) -> ZxResult {
        if elem.len() != self.elem_size {
            return Err(ZxError::ErrInvalidArgs);
        }
        if self.len >= self.elem_count {
            return Err(ZxError::ErrShouldWait);
        }

        for (byte_offset, byte) in elem.iter().enumerate() {
            let index = self.data_index(self.len, byte_offset)?;
            self.data[index] = *byte;
        }
        self.len += 1;
        Ok(())
    }

    fn pop_element(&mut self, out: &mut [u8]) -> ZxResult {
        if out.len() != self.elem_size {
            return Err(ZxError::ErrInvalidArgs);
        }
        if self.len == 0 {
            return Err(ZxError::ErrShouldWait);
        }

        for (byte_offset, slot) in out.iter_mut().enumerate() {
            let index = self.data_index(0, byte_offset)?;
            *slot = self.data[index];
        }

        self.read_pos = fifo_logic::ring_index(self.read_pos, 1, self.elem_count);
        self.len -= 1;
        if self.len == 0 {
            self.read_pos = 0;
        }
        Ok(())
    }
}

pub struct FifoTable {
    endpoints: [FifoEndpoint; MAX_FIFO_OBJECTS],
    next_handle: u32,
}

impl FifoTable {
    pub const fn new() -> Self {
        Self {
            endpoints: [FifoEndpoint::empty(); MAX_FIFO_OBJECTS],
            next_handle: FIFO_HANDLE_START,
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

    fn index(&self, handle: HandleValue) -> Option<usize> {
        self.endpoints
            .iter()
            .position(|endpoint| endpoint.active && endpoint.handle == handle)
    }

    fn free_pair_indices(&self) -> Option<(usize, usize)> {
        let mut first = None;
        for (index, endpoint) in self.endpoints.iter().enumerate() {
            if endpoint.active {
                continue;
            }
            if let Some(first) = first {
                return Some((first, index));
            }
            first = Some(index);
        }
        None
    }

    fn active_count(&self) -> usize {
        self.endpoints
            .iter()
            .filter(|endpoint| endpoint.active)
            .count()
    }

    pub fn contains(&self, handle: HandleValue) -> bool {
        self.index(handle).is_some()
    }

    pub fn create_pair(
        &mut self,
        elem_count: usize,
        elem_size: usize,
        options: u32,
    ) -> ZxResult<(HandleValue, HandleValue)> {
        if !fifo_logic::options_valid(options, FIFO_CREATE_OPTIONS_MASK) {
            return Err(ZxError::ErrInvalidArgs);
        }
        if elem_count == 0 || elem_size == 0 {
            return Err(ZxError::ErrInvalidArgs);
        }
        if !fifo_logic::capacity_valid(
            elem_count,
            elem_size,
            FIFO_MAX_ELEMS,
            FIFO_MAX_ELEM_SIZE,
            FIFO_BUFFER_SIZE,
        ) {
            return Err(ZxError::ErrOutOfRange);
        }
        if self.active_count() + 2 > MAX_FIFO_OBJECTS {
            return Err(ZxError::ErrNoMemory);
        }

        let (left_index, right_index) = self.free_pair_indices().ok_or(ZxError::ErrNoMemory)?;
        let left = self.alloc_handle().ok_or(ZxError::ErrNoMemory)?;
        let right = self.alloc_handle().ok_or(ZxError::ErrNoMemory)?;

        self.endpoints[left_index].activate(left, right, elem_count, elem_size);
        self.endpoints[right_index].activate(right, left, elem_count, elem_size);
        Ok((left, right))
    }

    pub fn close(&mut self, handle: HandleValue) -> bool {
        let Some(index) = self.index(handle) else {
            return false;
        };

        let peer = self.endpoints[index].peer;
        if let Some(peer) = peer {
            if let Some(peer_index) = self.index(peer) {
                self.endpoints[peer_index].signals &=
                    !crate::kernel_objects::channel::CHANNEL_SIGNAL_WRITABLE;
                self.endpoints[peer_index].signals |=
                    crate::kernel_objects::channel::CHANNEL_SIGNAL_PEER_CLOSED;
                self.endpoints[peer_index].peer = None;
            }
        }

        self.endpoints[index].clear();
        true
    }

    pub fn signals(&self, handle: HandleValue) -> Option<u32> {
        self.index(handle)
            .map(|index| self.endpoints[index].signals)
    }

    pub fn update_signals(
        &mut self,
        handle: HandleValue,
        clear_mask: u32,
        set_mask: u32,
    ) -> Option<u32> {
        let index = self.index(handle)?;
        self.endpoints[index].signals =
            object_logic::signal_update(self.endpoints[index].signals, clear_mask, set_mask);
        Some(self.endpoints[index].signals)
    }

    pub fn signal_peer(
        &mut self,
        handle: HandleValue,
        clear_mask: u32,
        set_mask: u32,
    ) -> ZxResult<u32> {
        let index = self.index(handle).ok_or(ZxError::ErrNotFound)?;
        let peer = self.endpoints[index].peer.ok_or(ZxError::ErrPeerClosed)?;
        self.update_signals(peer, clear_mask, set_mask)
            .ok_or(ZxError::ErrPeerClosed)
    }

    fn refresh_read_signals(endpoint: &mut FifoEndpoint) {
        endpoint.signals = fifo_logic::refresh_read_signals(
            endpoint.signals,
            endpoint.len,
            crate::kernel_objects::channel::CHANNEL_SIGNAL_READABLE,
        );
    }

    fn refresh_write_signals(&mut self, writer_index: usize) {
        let Some(peer) = self.endpoints[writer_index].peer else {
            self.endpoints[writer_index].signals &=
                !crate::kernel_objects::channel::CHANNEL_SIGNAL_WRITABLE;
            return;
        };

        let Some(peer_index) = self.index(peer) else {
            self.endpoints[writer_index].signals &=
                !crate::kernel_objects::channel::CHANNEL_SIGNAL_WRITABLE;
            return;
        };

        let remaining = fifo_logic::remaining_capacity(
            self.endpoints[peer_index].len,
            self.endpoints[peer_index].elem_count,
        );
        self.endpoints[writer_index].signals = fifo_logic::refresh_write_signals(
            self.endpoints[writer_index].signals,
            remaining,
            crate::kernel_objects::channel::CHANNEL_SIGNAL_WRITABLE,
        );
    }

    pub fn write(
        &mut self,
        handle: HandleValue,
        elem_size: usize,
        bytes: &[u8],
    ) -> ZxResult<usize> {
        let writer_index = self.index(handle).ok_or(ZxError::ErrNotFound)?;
        if elem_size == 0 || elem_size != self.endpoints[writer_index].elem_size {
            return Err(ZxError::ErrInvalidArgs);
        }
        if bytes.len() % elem_size != 0 {
            return Err(ZxError::ErrInvalidArgs);
        }
        let count = bytes.len() / elem_size;
        if count == 0 {
            return Ok(0);
        }

        let peer = self.endpoints[writer_index]
            .peer
            .ok_or(ZxError::ErrPeerClosed)?;
        let receiver_index = self.index(peer).ok_or(ZxError::ErrPeerClosed)?;
        let remaining = fifo_logic::remaining_capacity(
            self.endpoints[receiver_index].len,
            self.endpoints[receiver_index].elem_count,
        );
        if remaining == 0 {
            return Err(ZxError::ErrShouldWait);
        }

        let actual = fifo_logic::min_count(count, remaining);
        for element in 0..actual {
            let start = element
                .checked_mul(elem_size)
                .ok_or(ZxError::ErrOutOfRange)?;
            let end = start.checked_add(elem_size).ok_or(ZxError::ErrOutOfRange)?;
            self.endpoints[receiver_index].push_element(&bytes[start..end])?;
        }

        Self::refresh_read_signals(&mut self.endpoints[receiver_index]);
        self.refresh_write_signals(writer_index);
        Ok(actual)
    }

    pub fn read(
        &mut self,
        handle: HandleValue,
        elem_size: usize,
        out: &mut [u8],
    ) -> ZxResult<usize> {
        let reader_index = self.index(handle).ok_or(ZxError::ErrNotFound)?;
        if elem_size == 0 || elem_size != self.endpoints[reader_index].elem_size {
            return Err(ZxError::ErrInvalidArgs);
        }
        if out.len() % elem_size != 0 {
            return Err(ZxError::ErrInvalidArgs);
        }
        let count = out.len() / elem_size;
        if count == 0 {
            return Ok(0);
        }
        if self.endpoints[reader_index].len == 0 {
            if self.endpoints[reader_index].peer.is_none() {
                return Err(ZxError::ErrPeerClosed);
            }
            return Err(ZxError::ErrShouldWait);
        }

        let actual = fifo_logic::min_count(count, self.endpoints[reader_index].len);
        for element in 0..actual {
            let start = element
                .checked_mul(elem_size)
                .ok_or(ZxError::ErrOutOfRange)?;
            let end = start.checked_add(elem_size).ok_or(ZxError::ErrOutOfRange)?;
            self.endpoints[reader_index].pop_element(&mut out[start..end])?;
        }

        Self::refresh_read_signals(&mut self.endpoints[reader_index]);
        if let Some(peer) = self.endpoints[reader_index].peer {
            if let Some(peer_index) = self.index(peer) {
                self.refresh_write_signals(peer_index);
            }
        }
        Ok(actual)
    }
}

static mut FIFO_TABLE: FifoTable = FifoTable::new();

pub fn fifo_table() -> &'static mut FifoTable {
    unsafe { &mut FIFO_TABLE }
}
