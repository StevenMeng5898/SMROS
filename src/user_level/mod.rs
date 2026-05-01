//! User-Level Processes Module
//!
//! This module contains all user-level process implementations:
//! - User Process Management
//! - User Shell (user-mode shell)
//! - User Test Processes
//!
//! These modules handle processes that run at EL0 (user mode) and
//! their interaction with the kernel via syscalls.

#![allow(dead_code)]

pub(crate) mod user_logic;
pub mod user_process;
pub mod user_shell;
pub mod user_test;

/// Initialize user-level process subsystem
pub fn init() {
    user_process::init();
}
