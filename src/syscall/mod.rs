//! System Call Interface Module
//!
//! This module handles all syscall-related functionality:
//! - Syscall interface layer (Linux and Zircon compatibility)
//! - Syscall dispatch from assembly exception handler
//! - Syscall handling from EL0 processes
//!
//! Each aspect is in its own file for better organization.

pub(crate) mod address_logic;
pub mod syscall;
pub mod syscall_dispatch;
pub mod syscall_handler;

pub use syscall::*;

/// Initialize syscall subsystem
pub fn init() {
    syscall_handler::init();
}
