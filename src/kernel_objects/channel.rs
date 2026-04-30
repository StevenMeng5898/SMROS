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

use crate::syscall::{HandleValue, ZxError, ZxResult};
use alloc::collections::VecDeque;
use alloc::vec;
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
        }
    }

    /// Write a message to the channel
    pub fn write(&mut self, endpoint: HandleValue, data: &[u8], handles: &[u32]) -> ZxResult {
        if self.state != ChannelState::Active {
            return Err(ZxError::ErrPeerClosed);
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
    _bytes_ptr: usize,
    _bytes_len: usize,
    _handles_ptr: usize,
    _handles_count: usize,
    out_bytes_actual: &mut usize,
    out_handles_actual: &mut usize,
) -> crate::syscall::ZxResult {
    if options != 0 {
        return Err(ZxError::ErrInvalidArgs);
    }

    let h = HandleValue(handle);
    let mut data = Vec::new();
    let mut handles = Vec::new();

    match channel_table().get_channel(h) {
        Some(channel) => {
            match channel.read(h, &mut data, &mut handles) {
                Ok(()) => {
                    *out_bytes_actual = data.len();
                    *out_handles_actual = handles.len();
                    // In real implementation, would copy to user buffers
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        None => Err(ZxError::ErrNotFound),
    }
}

/// Zircon sys_channel_write implementation
pub fn sys_channel_write(
    handle: u32,
    options: u32,
    _bytes_ptr: usize,
    bytes_count: usize,
    _handles_ptr: usize,
    _handles_count: usize,
) -> crate::syscall::ZxResult {
    if options != 0 {
        return Err(ZxError::ErrInvalidArgs);
    }

    let h = HandleValue(handle);
    let data = vec![0u8; bytes_count]; // Placeholder - would copy from user

    match channel_table().get_channel(h) {
        Some(channel) => channel.write(h, &data, &[]),
        None => Err(ZxError::ErrNotFound),
    }
}

/// Zircon sys_channel_call_noretry implementation
pub fn sys_channel_call_noretry(
    handle: u32,
    options: u32,
    wr_bytes_ptr: usize,
    wr_num_bytes: usize,
    _wr_num_handles: usize,
    rd_bytes_ptr: usize,
    rd_num_bytes: usize,
    _rd_num_handles: usize,
    out_actual_bytes: &mut usize,
    out_actual_handles: &mut usize,
) -> crate::syscall::ZxResult {
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
