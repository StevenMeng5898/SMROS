//! User-Level Processes Module
//!
//! This module contains all user-level process implementations:
//! - User Process Management
//! - Minimal component framework
//! - Minimal FxFS-shaped object store
//! - Minimal ELF image loader
//! - Minimal Fuchsia-style service directory and fixed-message IPC
//! - User Shell (user-mode shell)
//! - User Test Processes
//!
//! These modules handle processes that run at EL0 (user mode) and
//! their interaction with the kernel via syscalls.

#![allow(dead_code)]

pub mod apps;
pub mod drivers;
pub mod services;

pub use apps::{user_process, user_test};
pub(crate) use services::user_logic;
pub use services::{
    compat_apps, component, docker_compat, elf, fxfs, host_share, net, svc, user_shell,
};

/// Initialize user-level process subsystem
pub fn init() {
    user_process::init();
    let drivers_ready = drivers::init();
    if component::init() && svc::init() {
        let mut serial = crate::kernel_lowlevel::serial::Serial::new();
        serial.init();
        if drivers_ready {
            serial.write_str(
                "[USER] Driver framework, component framework, FxFS, and /svc initialized\n",
            );
        } else {
            serial.write_str(
                "[USER] Component framework, FxFS, and /svc initialized without block driver\n",
            );
        }
    }
}
