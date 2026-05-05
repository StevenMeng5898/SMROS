#![allow(dead_code)]
#![allow(static_mut_refs)]
//! Channel Kernel Object
//!
//! Channels are the primary IPC mechanism in Zircon. They provide
//! bidirectional message passing between processes.
//!
//! This module implements:
//! - Channel creation and destruction
//! - Message sending and receiving
//! - Handle management for channels

use crate::kernel_objects::types::{
    default_rights_for_object, ObjectType, Rights, RIGHT_SAME_RIGHTS,
};
use crate::syscall::{syscall_logic, HandleValue, ZxError, ZxResult};
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use super::object_logic;

/// Maximum message size for channels
pub const MAX_CHANNEL_MSG_SIZE: usize = 65536;

/// Maximum number of handles that can be sent in a message
pub const MAX_CHANNEL_MSG_HANDLES: usize = 64;

/// Channel message
#[repr(C)]
#[derive(Debug, Clone)]
pub struct ChannelMessage {
    /// Message data (bytes)
    pub data: Vec<u8>,
    /// Handles being sent (transferred, not copied)
    pub handles: Vec<u32>,
    /// Whether this message is valid
    pub valid: bool,
}

impl ChannelMessage {
    pub fn new(data: Vec<u8>, handles: Vec<u32>) -> Self {
        Self {
            data,
            handles,
            valid: true,
        }
    }

    pub fn empty() -> Self {
        Self {
            data: Vec::new(),
            handles: Vec::new(),
            valid: true,
        }
    }
}

/// Channel states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    /// Channel is active
    Active,
    /// One end is closed
    PeerClosed,
    /// Both ends are closed
    Closed,
}

/// Channel kernel object
pub struct Channel {
    /// Channel handle (first end)
    pub handle0: HandleValue,
    /// Channel handle (second end)
    pub handle1: HandleValue,
    /// Message queue for endpoint 0
    pub queue0: VecDeque<ChannelMessage>,
    /// Message queue for endpoint 1
    pub queue1: VecDeque<ChannelMessage>,
    /// Channel state
    pub state: ChannelState,
    /// Reference count
    pub ref_count: AtomicU32,
    /// Maximum message size
    pub max_msg_size: usize,
    /// Rights for the first endpoint handle
    pub rights0: u32,
    /// Rights for the second endpoint handle
    pub rights1: u32,
}

impl Channel {
    /// Create a new channel
    ///
    /// Returns two handles: one for each endpoint
    pub fn create() -> Option<(HandleValue, HandleValue)> {
        // Allocate handle values (in real implementation, would use handle table)
        let h0 = HandleValue(100); // Placeholder
        let h1 = HandleValue(101); // Placeholder

        Some((h0, h1))
    }

    /// Create a new channel with specific handles
    pub fn new(handle0: HandleValue, handle1: HandleValue) -> Self {
        Self {
            handle0,
            handle1,
            queue0: VecDeque::new(),
            queue1: VecDeque::new(),
            state: ChannelState::Active,
            ref_count: AtomicU32::new(2), // Two endpoints
            max_msg_size: MAX_CHANNEL_MSG_SIZE,
            rights0: default_rights_for_object(ObjectType::Channel),
            rights1: default_rights_for_object(ObjectType::Channel),
        }
    }

    fn endpoint_rights(&self, endpoint: HandleValue) -> Option<u32> {
        if endpoint == self.handle0 {
            Some(self.rights0)
        } else if endpoint == self.handle1 {
            Some(self.rights1)
        } else {
            None
        }
    }

    fn set_endpoint_rights(&mut self, endpoint: HandleValue, rights: u32) -> bool {
        if endpoint == self.handle0 {
            self.rights0 = rights;
            true
        } else if endpoint == self.handle1 {
            self.rights1 = rights;
            true
        } else {
            false
        }
    }

    pub fn has_rights(&self, endpoint: HandleValue, required: u32) -> bool {
        self.endpoint_rights(endpoint)
            .map(|rights| crate::kernel_objects::rights_contain(rights, required))
            .unwrap_or(false)
    }

    /// Write a message to the channel
    pub fn write(&mut self, endpoint: HandleValue, data: &[u8], handles: &[u32]) -> ZxResult {
        if self.state != ChannelState::Active {
            return Err(ZxError::ErrPeerClosed);
        }
        if !self.has_rights(endpoint, Rights::Write as u32) {
            return Err(ZxError::ErrAccessDenied);
        }

        if !object_logic::channel_message_fits(
            data.len(),
            handles.len(),
            self.max_msg_size,
            MAX_CHANNEL_MSG_HANDLES,
        ) {
            return Err(ZxError::ErrInvalidArgs);
        }

        let msg = ChannelMessage::new(data.to_vec(), handles.to_vec());

        // Deliver to the opposite endpoint's queue
        if endpoint == self.handle0 {
            self.queue1.push_back(msg);
        } else if endpoint == self.handle1 {
            self.queue0.push_back(msg);
        } else {
            return Err(ZxError::ErrInvalidArgs);
        }

        Ok(())
    }

    /// Read a message from the channel
    pub fn read(
        &mut self,
        endpoint: HandleValue,
        out_data: &mut Vec<u8>,
        out_handles: &mut Vec<u32>,
    ) -> ZxResult {
        if self.state != ChannelState::Active {
            return Err(ZxError::ErrPeerClosed);
        }
        if !self.has_rights(endpoint, Rights::Read as u32) {
            return Err(ZxError::ErrAccessDenied);
        }

        // Get the queue for this endpoint
        let queue = if endpoint == self.handle0 {
            &mut self.queue0
        } else if endpoint == self.handle1 {
            &mut self.queue1
        } else {
            return Err(ZxError::ErrInvalidArgs);
        };

        // Pop the first message
        if let Some(msg) = queue.pop_front() {
            *out_data = msg.data;
            *out_handles = msg.handles;
            Ok(())
        } else {
            Err(ZxError::ErrShouldWait)
        }
    }

    /// Close one endpoint of the channel
    pub fn close_endpoint(&mut self, endpoint: HandleValue) -> ZxResult {
        if endpoint == self.handle0 {
            self.queue0.clear();
            self.ref_count.fetch_sub(1, Ordering::Relaxed);
        } else if endpoint == self.handle1 {
            self.queue1.clear();
            self.ref_count.fetch_sub(1, Ordering::Relaxed);
        } else {
            return Err(ZxError::ErrInvalidArgs);
        }

        if self.ref_count.load(Ordering::Relaxed) == 0 {
            self.state = ChannelState::Closed;
        } else {
            self.state = ChannelState::PeerClosed;
        }

        Ok(())
    }

    /// Close both endpoints
    pub fn close(&mut self) -> ZxResult {
        self.close_endpoint(self.handle0)?;
        self.close_endpoint(self.handle1)?;
        self.state = ChannelState::Closed;
        Ok(())
    }

    /// Check if channel is valid
    pub fn is_valid(&self) -> bool {
        self.state == ChannelState::Active
    }

    /// Get signal state (for wait operations)
    pub fn get_signal_state(&self, endpoint: HandleValue) -> u32 {
        let queue = if endpoint == self.handle0 {
            &self.queue0
        } else {
            &self.queue1
        };

        object_logic::channel_signal_state(
            !queue.is_empty(),
            self.state == ChannelState::PeerClosed,
            CHANNEL_SIGNAL_READABLE,
            CHANNEL_SIGNAL_PEER_CLOSED,
        )
    }
}

/// Channel signal flags
pub const CHANNEL_SIGNAL_READABLE: u32 = 1 << 0;
pub const CHANNEL_SIGNAL_WRITABLE: u32 = 1 << 1;
pub const CHANNEL_SIGNAL_PEER_CLOSED: u32 = 1 << 2;

/// Channel table - manages all channels in the system
pub struct ChannelTable {
    /// Active channels
    pub channels: Vec<Channel>,
    /// Next channel ID
    pub next_id: AtomicU32,
}

impl ChannelTable {
    pub const fn new() -> Self {
        Self {
            channels: Vec::new(),
            next_id: AtomicU32::new(1),
        }
    }

    /// Create a new channel
    pub fn create_channel(&mut self) -> Option<(HandleValue, HandleValue)> {
        let h0 = HandleValue(self.next_id.fetch_add(1, Ordering::Relaxed));
        let h1 = HandleValue(self.next_id.fetch_add(1, Ordering::Relaxed));

        let channel = Channel::new(h0, h1);
        self.channels.push(channel);

        Some((h0, h1))
    }

    /// Get channel by handle
    pub fn get_channel(&mut self, handle: HandleValue) -> Option<&mut Channel> {
        self.channels
            .iter_mut()
            .find(|c| c.handle0 == handle || c.handle1 == handle)
    }

    /// Remove a channel
    pub fn remove_channel(&mut self, handle: HandleValue) -> bool {
        if let Some(pos) = self
            .channels
            .iter()
            .position(|c| c.handle0 == handle || c.handle1 == handle)
        {
            self.channels.remove(pos);
            true
        } else {
            false
        }
    }

    pub fn write_message(&mut self, handle: HandleValue, data: &[u8]) -> ZxResult {
        match self.get_channel(handle) {
            Some(channel) => channel.write(handle, data, &[]),
            None => Err(ZxError::ErrNotFound),
        }
    }

    pub fn read_message(&mut self, handle: HandleValue, out: &mut Vec<u8>) -> ZxResult {
        let mut handles = Vec::new();
        match self.get_channel(handle) {
            Some(channel) => channel.read(handle, out, &mut handles),
            None => Err(ZxError::ErrNotFound),
        }
    }

    pub fn rights(&mut self, handle: HandleValue) -> Option<u32> {
        self.get_channel(handle)
            .and_then(|channel| channel.endpoint_rights(handle))
    }

    pub fn duplicate_endpoint_rights(
        &mut self,
        handle: HandleValue,
        requested_rights: u32,
        replace: bool,
    ) -> ZxResult {
        let index = self
            .channels
            .iter()
            .position(|channel| channel.handle0 == handle || channel.handle1 == handle)
            .ok_or(ZxError::ErrNotFound)?;
        let existing_rights = self.channels[index]
            .endpoint_rights(handle)
            .ok_or(ZxError::ErrNotFound)?;
        let allowed = if replace {
            object_logic::replace_rights_allowed(
                existing_rights,
                requested_rights,
                RIGHT_SAME_RIGHTS,
                crate::kernel_objects::RIGHTS_ALL,
            )
        } else {
            object_logic::duplicate_rights_allowed(
                existing_rights,
                requested_rights,
                Rights::Duplicate as u32,
                RIGHT_SAME_RIGHTS,
                crate::kernel_objects::RIGHTS_ALL,
            )
        };
        if !allowed {
            return Err(ZxError::ErrAccessDenied);
        }
        let rights = if requested_rights == RIGHT_SAME_RIGHTS {
            existing_rights
        } else {
            requested_rights
        };

        if replace {
            if !self.channels[index].set_endpoint_rights(handle, rights) {
                return Err(ZxError::ErrNotFound);
            }
        }
        Ok(())
    }
}

/// Global channel table
static mut CHANNEL_TABLE: ChannelTable = ChannelTable::new();

/// Initialize channel subsystem
pub fn init() {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    serial.write_str("[CHANNEL] Channel subsystem initialized\n");
}

/// Get global channel table
pub fn channel_table() -> &'static mut ChannelTable {
    unsafe { &mut CHANNEL_TABLE }
}

// ============================================================================
// Channel Syscalls (Zircon)
// ============================================================================

/// Zircon sys_channel_create implementation
pub fn sys_channel_create(
    options: u32,
    out_handle0: &mut u32,
    out_handle1: &mut u32,
) -> crate::syscall::ZxResult {
    if options != 0 {
        return Err(ZxError::ErrInvalidArgs);
    }

    if let Some((h0, h1)) = channel_table().create_channel() {
        *out_handle0 = h0.0;
        *out_handle1 = h1.0;
        Ok(())
    } else {
        Err(ZxError::ErrNoMemory)
    }
}

/// Zircon sys_channel_read implementation
pub fn sys_channel_read(
    handle: u32,
    options: u32,
    bytes_ptr: usize,
    bytes_len: usize,
    handles_ptr: usize,
    handles_count: usize,
    out_bytes_actual: &mut usize,
    out_handles_actual: &mut usize,
) -> crate::syscall::ZxResult {
    if options != 0 {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !syscall_logic::channel_buffers_valid(bytes_ptr, bytes_len, handles_ptr, handles_count) {
        return Err(ZxError::ErrInvalidArgs);
    }

    let h = HandleValue(handle);

    match channel_table().get_channel(h) {
        Some(channel) => {
            let queue = if h == channel.handle0 {
                &mut channel.queue0
            } else if h == channel.handle1 {
                &mut channel.queue1
            } else {
                return Err(ZxError::ErrInvalidArgs);
            };

            let Some(message) = queue.front() else {
                return Err(ZxError::ErrShouldWait);
            };

            *out_bytes_actual = message.data.len();
            *out_handles_actual = message.handles.len();

            if message.data.len() > bytes_len || message.handles.len() > handles_count {
                return Err(ZxError::ErrOutOfRange);
            }

            let message = queue.pop_front().ok_or(ZxError::ErrShouldWait)?;

            if !message.data.is_empty() {
                let out = unsafe {
                    core::slice::from_raw_parts_mut(bytes_ptr as *mut u8, message.data.len())
                };
                out.copy_from_slice(&message.data);
            }

            if !message.handles.is_empty() {
                let out = unsafe {
                    core::slice::from_raw_parts_mut(handles_ptr as *mut u32, message.handles.len())
                };
                out.copy_from_slice(&message.handles);
            }

            Ok(())
        }
        None => Err(ZxError::ErrNotFound),
    }
}

/// Zircon sys_channel_write implementation
pub fn sys_channel_write(
    handle: u32,
    options: u32,
    bytes_ptr: usize,
    bytes_count: usize,
    handles_ptr: usize,
    handles_count: usize,
) -> crate::syscall::ZxResult {
    if options != 0 {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !syscall_logic::channel_buffers_valid(bytes_ptr, bytes_count, handles_ptr, handles_count) {
        return Err(ZxError::ErrInvalidArgs);
    }

    let h = HandleValue(handle);
    let data = if bytes_count == 0 {
        &[][..]
    } else {
        unsafe { core::slice::from_raw_parts(bytes_ptr as *const u8, bytes_count) }
    };
    let handles = if handles_count == 0 {
        &[][..]
    } else {
        unsafe { core::slice::from_raw_parts(handles_ptr as *const u32, handles_count) }
    };

    match channel_table().get_channel(h) {
        Some(channel) => channel.write(h, data, handles),
        None => Err(ZxError::ErrNotFound),
    }
}

/// Zircon sys_channel_call_noretry implementation
pub fn sys_channel_call_noretry(
    handle: u32,
    options: u32,
    wr_bytes_ptr: usize,
    wr_num_bytes: usize,
    wr_num_handles: usize,
    rd_bytes_ptr: usize,
    rd_num_bytes: usize,
    rd_num_handles: usize,
    out_actual_bytes: &mut usize,
    out_actual_handles: &mut usize,
) -> crate::syscall::ZxResult {
    if wr_num_handles != 0 || rd_num_handles != 0 {
        return Err(ZxError::ErrInvalidArgs);
    }

    // Atomic write+read operation
    // Write first, then read
    sys_channel_write(handle, options, wr_bytes_ptr, wr_num_bytes, 0, 0)?;
    sys_channel_read(
        handle,
        options,
        rd_bytes_ptr,
        rd_num_bytes,
        0,
        0,
        out_actual_bytes,
        out_actual_handles,
    )
}
