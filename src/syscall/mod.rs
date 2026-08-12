//! System Call Interface Module
//!
//! This module handles all syscall-related functionality:
//! - Syscall interface layer (Linux and Zircon compatibility)
//! - Syscall dispatch from assembly exception handler
//! - Syscall handling from EL0 processes
//!
//! Each aspect is in its own file for better organization.

pub(crate) mod address_logic;
pub(crate) mod linux_futex;
pub(crate) mod linux_process;
pub(crate) mod linux_process_memory;
#[allow(dead_code)]
pub(crate) mod linux_record_lock;
#[cfg(target_arch = "aarch64")]
pub(crate) mod linux_syscall_context;
pub(crate) mod linux_task;
pub mod syscall;
pub(crate) mod syscall_bridge;
pub mod syscall_dispatch;
pub mod syscall_handler;
pub(crate) mod syscall_logic;

pub use syscall::*;

/// Initialize syscall subsystem
pub fn init() {
    syscall_handler::init();
}
