//! Kernel Objects Module
//!
//! This module contains all kernel object implementations:
//! - Thread Management
//! - VMO (Virtual Memory Object)
//! - VMAR (Virtual Memory Address Region)
//! - Handle Table
//! - Channel (IPC mechanism)
//! - Scheduler (Thread scheduling)
//!
//! Each kernel object is in its own file for better organization.

#![allow(dead_code)]
#![allow(static_mut_refs)]

// Module declarations
pub mod channel;
pub mod compat;
pub mod handle;
pub(crate) mod object_logic;
pub mod scheduler;
pub mod socket;
pub(crate) mod socket_logic;
pub mod thread;
pub mod types;
pub mod vmar;
pub mod vmo;

// Re-export all public types
pub use handle::*;
pub use scheduler::*;
pub use types::*;
pub use vmo::*;

// ============================================================================
// Kernel Object Manager
// ============================================================================

/// Global kernel object manager
/// Manages all kernel objects and their handles
pub struct KernelObjectManager {
    /// Global handle table (simplified - in real impl, per-process)
    handle_table: HandleTable,
}

impl KernelObjectManager {
    /// Create a new kernel object manager
    pub const fn new() -> Self {
        Self {
            handle_table: HandleTable::new(),
        }
    }

    /// Get handle table
    pub fn handle_table(&mut self) -> &mut HandleTable {
        &mut self.handle_table
    }
}

/// Global kernel object manager instance
static mut KERNEL_OBJECT_MANAGER: KernelObjectManager = KernelObjectManager::new();

/// Get global kernel object manager
pub fn kernel_object_manager() -> &'static mut KernelObjectManager {
    unsafe { &mut KERNEL_OBJECT_MANAGER }
}

/// Initialize kernel objects
pub fn init() {
    // Kernel objects are statically initialized
}
