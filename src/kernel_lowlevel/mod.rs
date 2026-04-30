//! Kernel Low-Level Module
//!
//! This module contains all low-level kernel implementations:
//! - Memory Management (page frames, segments, process address spaces)
//! - MMU and Page Table Management
//! - Serial Driver (PL011 UART)
//! - Timer Driver (ARM Generic Timer)
//! - Interrupt Controller (GICv2)
//! - SMP Support (Symmetric Multi-Processing)
//! - Hardware Drivers
//!
//! These modules handle low-level operations that form the foundation of the kernel.

pub mod drivers;
pub mod interrupt;
pub(crate) mod lowlevel_logic;
pub mod memory;
pub mod mmu;
pub mod serial;
pub mod smp;
pub mod timer;
