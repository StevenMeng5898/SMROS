//! Hardware Drivers Module
//! 
//! This module contains various hardware drivers for the kernel.

pub mod serial {
    pub use crate::serial::Serial;
}

// Future drivers can be added here:
// pub mod timer;
// pub mod interrupt;
// pub mod memory;
