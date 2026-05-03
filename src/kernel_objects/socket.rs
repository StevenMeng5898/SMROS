#![allow(dead_code)]
#![allow(static_mut_refs)]
//! Zircon socket kernel object.
//!
//! Sockets are bidirectional byte transports. Stream sockets can perform short
//! writes, while datagram sockets preserve packet boundaries and discard the
//! unread tail of a packet on a non-peek read.

use super::{object_logic, socket_logic, HandleValue, ZxError, ZxResult, INVALID_HANDLE};

const SOCKET_HANDLE_START: u32 = 0x9000_0000;
const MAX_SOCKET_OBJECTS: usize = 16;
const MAX_SOCKET_SHARED: usize = 16;
const MAX_SOCKET_DATAGRAMS: usize = 16;
pub const SOCKET_SIZE: usize = 128 * 2048;

pub const SOCKET_SHUTDOWN_WRITE: u32 = 1;
pub const SOCKET_SHUTDOWN_READ: u32 = 1 << 1;
const SOCKET_SHUTDOWN_MASK: u32 = SOCKET_SHUTDOWN_WRITE | SOCKET_SHUTDOWN_READ;

pub const SOCKET_DATAGRAM: u32 = 1;
const SOCKET_CREATE_MASK: u32 = SOCKET_DATAGRAM;

pub const SOCKET_PEEK: u32 = 1 << 3;
const SOCKET_READ_OPTIONS_MASK: u32 = SOCKET_PEEK;

pub const SOCKET_SIGNAL_PEER_WRITE_DISABLED: u32 = 1 << 4;
pub const SOCKET_SIGNAL_WRITE_DISABLED: u32 = 1 << 5;
pub const SOCKET_SIGNAL_SHARE: u32 = 1 << 9;
pub const SOCKET_SIGNAL_READ_THRESHOLD: u32 = 1 << 10;
pub const SOCKET_SIGNAL_WRITE_THRESHOLD: u32 = 1 << 11;

pub const SOCKET_PROPERTY_RX_THRESHOLD: u32 = 12;
pub const SOCKET_PROPERTY_TX_THRESHOLD: u32 = 13;
pub const OBJECT_INFO_TOPIC_SOCKET: u32 = 22;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SocketInfo {
    pub options: u32,
    pub padding1: u32,
    pub rx_buf_max: u64,
    pub rx_buf_size: u64,
    pub rx_buf_available: u64,
    pub tx_buf_max: u64,
    pub tx_buf_size: u64,
}

#[derive(Clone, Copy)]
pub struct SocketEndpoint {
    pub handle: HandleValue,
    pub peer: Option<HandleValue>,
    pub signals: u32,
    pub options: u32,
    pub data: [u8; SOCKET_SIZE],
    pub read_pos: usize,
    pub len: usize,
    pub datagram_lens: [usize; MAX_SOCKET_DATAGRAMS],
    pub datagram_head: usize,
    pub datagram_count: usize,
    pub read_threshold: usize,
    pub write_threshold: usize,
    pub read_disabled: bool,
    pub write_disabled: bool,
    pub shared: [HandleValue; MAX_SOCKET_SHARED],
    pub shared_head: usize,
    pub shared_count: usize,
    pub active: bool,
}

impl SocketEndpoint {
    pub const fn empty() -> Self {
        Self {
            handle: HandleValue(INVALID_HANDLE),
            peer: None,
            signals: 0,
            options: 0,
            data: [0; SOCKET_SIZE],
            read_pos: 0,
            len: 0,
            datagram_lens: [0; MAX_SOCKET_DATAGRAMS],
            datagram_head: 0,
            datagram_count: 0,
            read_threshold: 0,
            write_threshold: 0,
            read_disabled: false,
            write_disabled: false,
            shared: [HandleValue(INVALID_HANDLE); MAX_SOCKET_SHARED],
            shared_head: 0,
            shared_count: 0,
            active: false,
        }
    }

    fn activate(&mut self, handle: HandleValue, peer: HandleValue, options: u32) {
        self.handle = handle;
        self.peer = Some(peer);
        self.signals = crate::kernel_objects::channel::CHANNEL_SIGNAL_WRITABLE;
        self.options = options;
        self.read_pos = 0;
        self.len = 0;
        self.datagram_head = 0;
        self.datagram_count = 0;
        self.read_threshold = 0;
        self.write_threshold = 0;
        self.read_disabled = false;
        self.write_disabled = false;
        self.shared_head = 0;
        self.shared_count = 0;
        self.active = true;
    }

    fn clear(&mut self) {
        self.handle = HandleValue(INVALID_HANDLE);
        self.peer = None;
        self.signals = 0;
        self.options = 0;
        self.read_pos = 0;
        self.len = 0;
        self.datagram_head = 0;
        self.datagram_count = 0;
        self.read_threshold = 0;
        self.write_threshold = 0;
        self.read_disabled = false;
        self.write_disabled = false;
        self.shared_head = 0;
        self.shared_count = 0;
        self.active = false;
    }

    fn data_index(&self, offset: usize) -> usize {
        socket_logic::ring_index(self.read_pos, offset, SOCKET_SIZE)
    }

    fn push_byte(&mut self, byte: u8) -> ZxResult {
        if self.len >= SOCKET_SIZE {
            return Err(ZxError::ErrShouldWait);
        }
        let index = self.data_index(self.len);
        self.data[index] = byte;
        self.len += 1;
        Ok(())
    }

    fn pop_byte(&mut self) -> Option<u8> {
        if self.len == 0 {
            return None;
        }
        let byte = self.data[self.read_pos];
        self.read_pos = (self.read_pos + 1) % SOCKET_SIZE;
        self.len -= 1;
        if self.len == 0 {
            self.read_pos = 0;
        }
        Some(byte)
    }

    fn peek_byte(&self, offset: usize) -> Option<u8> {
        if offset >= self.len {
            return None;
        }
        Some(self.data[self.data_index(offset)])
    }

    fn push_datagram_len(&mut self, len: usize) -> ZxResult {
        if self.datagram_count >= MAX_SOCKET_DATAGRAMS {
            return Err(ZxError::ErrShouldWait);
        }
        let index = socket_logic::ring_index(
            self.datagram_head,
            self.datagram_count,
            MAX_SOCKET_DATAGRAMS,
        );
        self.datagram_lens[index] = len;
        self.datagram_count += 1;
        Ok(())
    }

    fn front_datagram_len(&self) -> Option<usize> {
        if self.datagram_count == 0 {
            None
        } else {
            Some(self.datagram_lens[self.datagram_head])
        }
    }

    fn pop_datagram_len(&mut self) -> Option<usize> {
        if self.datagram_count == 0 {
            return None;
        }
        let len = self.datagram_lens[self.datagram_head];
        self.datagram_lens[self.datagram_head] = 0;
        self.datagram_head = socket_logic::ring_index(self.datagram_head, 1, MAX_SOCKET_DATAGRAMS);
        self.datagram_count -= 1;
        if self.datagram_count == 0 {
            self.datagram_head = 0;
        }
        Some(len)
    }

    fn push_shared(&mut self, handle: HandleValue) -> ZxResult {
        if self.shared_count >= MAX_SOCKET_SHARED {
            return Err(ZxError::ErrShouldWait);
        }
        let index =
            socket_logic::ring_index(self.shared_head, self.shared_count, MAX_SOCKET_SHARED);
        self.shared[index] = handle;
        self.shared_count += 1;
        Ok(())
    }

    fn pop_shared(&mut self) -> Option<HandleValue> {
        if self.shared_count == 0 {
            return None;
        }
        let handle = self.shared[self.shared_head];
        self.shared[self.shared_head] = HandleValue(INVALID_HANDLE);
        self.shared_head = socket_logic::ring_index(self.shared_head, 1, MAX_SOCKET_SHARED);
        self.shared_count -= 1;
        if self.shared_count == 0 {
            self.shared_head = 0;
        }
        Some(handle)
    }

    fn remove_shared(&mut self, handle: HandleValue) {
        let mut compact = [HandleValue(INVALID_HANDLE); MAX_SOCKET_SHARED];
        let mut count = 0;
        for offset in 0..self.shared_count {
            let index = socket_logic::ring_index(self.shared_head, offset, MAX_SOCKET_SHARED);
            let shared = self.shared[index];
            if shared != handle {
                compact[count] = shared;
                count += 1;
            }
        }
        self.shared = compact;
        self.shared_head = 0;
        self.shared_count = count;
    }
}

pub struct SocketTable {
    endpoints: [SocketEndpoint; MAX_SOCKET_OBJECTS],
    next_handle: u32,
}

impl SocketTable {
    pub const fn new() -> Self {
        Self {
            endpoints: [SocketEndpoint::empty(); MAX_SOCKET_OBJECTS],
            next_handle: SOCKET_HANDLE_START,
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

    fn free_index(&self) -> Option<usize> {
        self.endpoints.iter().position(|endpoint| !endpoint.active)
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

    pub fn create_pair(&mut self, options: u32) -> ZxResult<(HandleValue, HandleValue)> {
        if !socket_logic::options_valid(options, SOCKET_CREATE_MASK) {
            return Err(ZxError::ErrInvalidArgs);
        }
        if self.active_count() + 2 > MAX_SOCKET_OBJECTS {
            return Err(ZxError::ErrNoMemory);
        }

        let (left_index, right_index) = self.free_pair_indices().ok_or(ZxError::ErrNoMemory)?;

        let left = self.alloc_handle().ok_or(ZxError::ErrNoMemory)?;
        let right = self.alloc_handle().ok_or(ZxError::ErrNoMemory)?;
        self.endpoints[left_index].activate(left, right, options);
        self.endpoints[right_index].activate(right, left, options);
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

        for endpoint in &mut self.endpoints {
            if endpoint.active {
                endpoint.remove_shared(handle);
                if endpoint.shared_count == 0 {
                    endpoint.signals &= !SOCKET_SIGNAL_SHARE;
                }
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

    pub fn property(&self, handle: HandleValue, prop_id: u32) -> Option<u64> {
        let endpoint = &self.endpoints[self.index(handle)?];
        match prop_id {
            SOCKET_PROPERTY_RX_THRESHOLD => Some(endpoint.read_threshold as u64),
            SOCKET_PROPERTY_TX_THRESHOLD => Some(endpoint.write_threshold as u64),
            _ => None,
        }
    }

    pub fn set_property(
        &mut self,
        handle: HandleValue,
        prop_id: u32,
        value: u64,
    ) -> ZxResult<bool> {
        let Some(index) = self.index(handle) else {
            return Ok(false);
        };

        match prop_id {
            SOCKET_PROPERTY_RX_THRESHOLD => {
                let threshold = usize::try_from(value).map_err(|_| ZxError::ErrOutOfRange)?;
                if threshold > SOCKET_SIZE {
                    return Err(ZxError::ErrInvalidArgs);
                }
                self.endpoints[index].read_threshold = threshold;
                Self::refresh_read_signals(&mut self.endpoints[index]);
                Ok(true)
            }
            SOCKET_PROPERTY_TX_THRESHOLD => {
                let threshold = usize::try_from(value).map_err(|_| ZxError::ErrOutOfRange)?;
                if threshold > SOCKET_SIZE {
                    return Err(ZxError::ErrInvalidArgs);
                }
                self.endpoints[index].write_threshold = threshold;
                self.refresh_write_signals(index);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn refresh_read_signals(endpoint: &mut SocketEndpoint) {
        endpoint.signals = socket_logic::refresh_read_signals(
            endpoint.signals,
            endpoint.len,
            endpoint.read_threshold,
            crate::kernel_objects::channel::CHANNEL_SIGNAL_READABLE,
            SOCKET_SIGNAL_READ_THRESHOLD,
        );
    }

    fn refresh_write_signals(&mut self, writer_index: usize) {
        let Some(peer) = self.endpoints[writer_index].peer else {
            self.endpoints[writer_index].signals &=
                !crate::kernel_objects::channel::CHANNEL_SIGNAL_WRITABLE;
            self.endpoints[writer_index].signals &= !SOCKET_SIGNAL_WRITE_THRESHOLD;
            return;
        };

        let Some(peer_index) = self.index(peer) else {
            self.endpoints[writer_index].signals &=
                !crate::kernel_objects::channel::CHANNEL_SIGNAL_WRITABLE;
            self.endpoints[writer_index].signals &= !SOCKET_SIGNAL_WRITE_THRESHOLD;
            return;
        };

        let remaining =
            socket_logic::remaining_capacity(self.endpoints[peer_index].len, SOCKET_SIZE);
        self.endpoints[writer_index].signals = socket_logic::refresh_write_signals(
            self.endpoints[writer_index].signals,
            self.endpoints[writer_index].write_disabled,
            remaining,
            self.endpoints[writer_index].write_threshold,
            crate::kernel_objects::channel::CHANNEL_SIGNAL_WRITABLE,
            SOCKET_SIGNAL_WRITE_THRESHOLD,
        );
    }

    pub fn write(&mut self, handle: HandleValue, bytes: &[u8]) -> ZxResult<usize> {
        let writer_index = self.index(handle).ok_or(ZxError::ErrNotFound)?;
        if self.endpoints[writer_index].write_disabled {
            return Err(ZxError::ErrBadState);
        }
        let peer = self.endpoints[writer_index]
            .peer
            .ok_or(ZxError::ErrPeerClosed)?;
        let receiver_index = self.index(peer).ok_or(ZxError::ErrPeerClosed)?;
        if self.endpoints[receiver_index].read_disabled {
            return Err(ZxError::ErrBadState);
        }

        let remaining =
            socket_logic::remaining_capacity(self.endpoints[receiver_index].len, SOCKET_SIZE);
        if remaining == 0 {
            return Err(ZxError::ErrShouldWait);
        }

        let datagram = self.endpoints[writer_index].options & SOCKET_DATAGRAM != 0;
        let actual = if datagram {
            if bytes.is_empty() {
                return Err(ZxError::ErrInvalidArgs);
            }
            if bytes.len() > SOCKET_SIZE {
                return Err(ZxError::ErrOutOfRange);
            }
            if bytes.len() > remaining {
                return Err(ZxError::ErrShouldWait);
            }
            if self.endpoints[receiver_index].datagram_count >= MAX_SOCKET_DATAGRAMS {
                return Err(ZxError::ErrShouldWait);
            }
            bytes.len()
        } else {
            socket_logic::min_count(bytes.len(), remaining)
        };

        for byte in &bytes[..actual] {
            self.endpoints[receiver_index].push_byte(*byte)?;
        }
        if datagram && actual != 0 {
            self.endpoints[receiver_index].push_datagram_len(actual)?;
        }

        Self::refresh_read_signals(&mut self.endpoints[receiver_index]);
        self.refresh_write_signals(writer_index);
        Ok(actual)
    }

    pub fn read(&mut self, handle: HandleValue, options: u32, out: &mut [u8]) -> ZxResult<usize> {
        if !socket_logic::options_valid(options, SOCKET_READ_OPTIONS_MASK) {
            return Err(ZxError::ErrInvalidArgs);
        }

        let index = self.index(handle).ok_or(ZxError::ErrNotFound)?;
        if self.endpoints[index].len == 0 {
            if self.endpoints[index].peer.is_none() {
                return Err(ZxError::ErrPeerClosed);
            }
            if self.endpoints[index].read_disabled {
                return Err(ZxError::ErrBadState);
            }
            return Err(ZxError::ErrShouldWait);
        }

        let peek = options & SOCKET_PEEK != 0;
        let datagram = self.endpoints[index].options & SOCKET_DATAGRAM != 0;
        let was_full = self.endpoints[index].len == SOCKET_SIZE;
        let actual = if datagram {
            self.read_datagram(index, peek, out)?
        } else {
            self.read_stream(index, peek, out)?
        };

        if !peek && actual > 0 {
            Self::refresh_read_signals(&mut self.endpoints[index]);
            if let Some(peer) = self.endpoints[index].peer {
                if let Some(peer_index) = self.index(peer) {
                    if was_full {
                        self.endpoints[peer_index].signals |=
                            crate::kernel_objects::channel::CHANNEL_SIGNAL_WRITABLE;
                    }
                    self.refresh_write_signals(peer_index);
                }
            }
        }

        Ok(actual)
    }

    fn read_datagram(&mut self, index: usize, peek: bool, out: &mut [u8]) -> ZxResult<usize> {
        if out.is_empty() {
            return Ok(0);
        }

        let datagram_len = self.endpoints[index]
            .front_datagram_len()
            .ok_or(ZxError::ErrInternal)?;
        let read = socket_logic::min_count(out.len(), datagram_len);
        for (offset, slot) in out.iter_mut().enumerate().take(read) {
            *slot = self.endpoints[index]
                .peek_byte(offset)
                .ok_or(ZxError::ErrInternal)?;
        }

        if !peek {
            self.endpoints[index]
                .pop_datagram_len()
                .ok_or(ZxError::ErrInternal)?;
            for _ in 0..datagram_len {
                self.endpoints[index]
                    .pop_byte()
                    .ok_or(ZxError::ErrInternal)?;
            }
        }

        Ok(read)
    }

    fn read_stream(&mut self, index: usize, peek: bool, out: &mut [u8]) -> ZxResult<usize> {
        let read = socket_logic::min_count(out.len(), self.endpoints[index].len);
        if peek {
            for (offset, slot) in out.iter_mut().enumerate().take(read) {
                *slot = self.endpoints[index]
                    .peek_byte(offset)
                    .ok_or(ZxError::ErrInternal)?;
            }
        } else {
            for slot in out.iter_mut().take(read) {
                *slot = self.endpoints[index]
                    .pop_byte()
                    .ok_or(ZxError::ErrInternal)?;
            }
        }
        Ok(read)
    }

    pub fn shutdown(&mut self, handle: HandleValue, options: u32) -> ZxResult {
        let index = self.index(handle).ok_or(ZxError::ErrNotFound)?;
        let options = socket_logic::mask_options(options, SOCKET_SHUTDOWN_MASK);
        let read = options & SOCKET_SHUTDOWN_READ != 0;
        let write = options & SOCKET_SHUTDOWN_WRITE != 0;
        if !read && !write {
            return Ok(());
        }

        if read {
            self.endpoints[index].read_disabled = true;
            self.endpoints[index].signals |= SOCKET_SIGNAL_PEER_WRITE_DISABLED;
        }
        if write {
            self.endpoints[index].write_disabled = true;
            self.endpoints[index].signals &=
                !crate::kernel_objects::channel::CHANNEL_SIGNAL_WRITABLE;
            self.endpoints[index].signals |= SOCKET_SIGNAL_WRITE_DISABLED;
        }

        if let Some(peer) = self.endpoints[index].peer {
            if let Some(peer_index) = self.index(peer) {
                if read {
                    self.endpoints[peer_index].write_disabled = true;
                    self.endpoints[peer_index].signals &=
                        !crate::kernel_objects::channel::CHANNEL_SIGNAL_WRITABLE;
                    self.endpoints[peer_index].signals |= SOCKET_SIGNAL_WRITE_DISABLED;
                }
                if write {
                    self.endpoints[peer_index].read_disabled = true;
                    self.endpoints[peer_index].signals |= SOCKET_SIGNAL_PEER_WRITE_DISABLED;
                }
                self.refresh_write_signals(peer_index);
            }
        }

        self.refresh_write_signals(index);
        Ok(())
    }

    pub fn share(&mut self, handle: HandleValue, shared: HandleValue) -> ZxResult {
        let index = self.index(handle).ok_or(ZxError::ErrNotFound)?;
        if self.index(shared).is_none() {
            return Err(ZxError::ErrNotFound);
        }
        let peer = self.endpoints[index].peer.ok_or(ZxError::ErrPeerClosed)?;
        let peer_index = self.index(peer).ok_or(ZxError::ErrPeerClosed)?;
        self.endpoints[peer_index].push_shared(shared)?;
        self.endpoints[peer_index].signals |= SOCKET_SIGNAL_SHARE;
        Ok(())
    }

    pub fn accept(&mut self, handle: HandleValue) -> ZxResult<HandleValue> {
        let index = self.index(handle).ok_or(ZxError::ErrNotFound)?;
        let shared = self.endpoints[index]
            .pop_shared()
            .ok_or(ZxError::ErrShouldWait)?;
        if self.endpoints[index].shared_count == 0 {
            self.endpoints[index].signals &= !SOCKET_SIGNAL_SHARE;
        }
        Ok(shared)
    }

    pub fn info(&self, handle: HandleValue) -> Option<SocketInfo> {
        let index = self.index(handle)?;
        let endpoint = &self.endpoints[index];
        let rx_size = endpoint.len;
        let rx_available = if endpoint.options & SOCKET_DATAGRAM != 0 {
            endpoint.front_datagram_len().unwrap_or(0)
        } else {
            rx_size
        };
        let (tx_size, tx_max) = endpoint
            .peer
            .and_then(|peer| self.index(peer))
            .map(|peer_index| (self.endpoints[peer_index].len as u64, SOCKET_SIZE as u64))
            .unwrap_or((0, 0));

        Some(SocketInfo {
            options: endpoint.options,
            padding1: 0,
            rx_buf_max: SOCKET_SIZE as u64,
            rx_buf_size: rx_size as u64,
            rx_buf_available: rx_available as u64,
            tx_buf_max: tx_max,
            tx_buf_size: tx_size,
        })
    }
}

static mut SOCKET_TABLE: SocketTable = SocketTable::new();

pub fn socket_table() -> &'static mut SocketTable {
    unsafe { &mut SOCKET_TABLE }
}
