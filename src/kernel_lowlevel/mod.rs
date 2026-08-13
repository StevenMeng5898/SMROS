//! Kernel Low-Level Module
//!
//! This module contains all low-level kernel implementations:
//! - Memory Management (page frames, segments, process address spaces)
//! - MMU and Page Table Management
//! - Architecture-selected serial, timer, interrupt, SMP, and context code
//! - SMP Support (Symmetric Multi-Processing)
//! - Thread Context Switching
//! - Hardware Drivers
//!
//! These modules handle low-level operations that form the foundation of the kernel.

#[cfg(target_arch = "aarch64")]
#[path = "ARM64/mod.rs"]
mod arch;

#[cfg(target_arch = "riscv64")]
#[path = "RISCV64/mod.rs"]
mod arch;

#[cfg(target_arch = "x86_64")]
#[path = "X86_64/mod.rs"]
mod arch;

#[cfg(target_arch = "aarch64")]
pub(crate) mod aarch64_exception_logic_shared;
#[cfg(target_arch = "aarch64")]
#[path = "aarch64_vm_logic_shared.rs"]
pub(crate) mod aarch64_vm_logic_shared;
pub(crate) mod lowlevel_logic;
pub mod memory;
pub mod mmu;

#[cfg(target_arch = "aarch64")]
pub use arch::user_address_space::Aarch64AddressSpace;

pub use arch::{cpu, drivers, interrupt, serial, smp, thread, timer};
