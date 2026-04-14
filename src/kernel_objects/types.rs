//! Kernel Object Types and Constants
//!
//! This module contains shared types, constants, and error codes
//! used by all kernel objects.

// ============================================================================
// Constants
// ============================================================================

/// Maximum number of handles per process
#[allow(dead_code)]
pub const MAX_HANDLES_PER_PROCESS: usize = 1024;

/// Invalid handle value
pub const INVALID_HANDLE: u32 = 0xFFFF_FFFF;

// ============================================================================
// Handle Types
// ============================================================================

/// Handle to a kernel object
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HandleValue(pub u32);

impl From<u32> for HandleValue {
    fn from(v: u32) -> Self { Self(v) }
}

impl From<HandleValue> for u32 {
    fn from(h: HandleValue) -> Self { h.0 }
}

/// Kernel object types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    Vmo = 1,
    Vmar = 2,
    Channel = 3,
    Socket = 4,
    Event = 5,
    Process = 6,
    Thread = 7,
    Port = 8,
    Timer = 9,
    Resource = 10,
    Interrupt = 11,
}

/// Handle rights bitmask
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rights {
    None = 0,
    Duplicate = 1 << 0,
    Transfer = 1 << 1,
    Read = 1 << 2,
    Write = 1 << 3,
    Execute = 1 << 4,
    Map = 1 << 5,
    GetProperty = 1 << 6,
    SetProperty = 1 << 7,
    Signal = 1 << 8,
    SignalPeer = 1 << 9,
    Wait = 1 << 10,
    DefaultVmo = Rights::Duplicate as u32 | Rights::Transfer as u32 | Rights::Map as u32
                | Rights::GetProperty as u32 | Rights::SetProperty as u32,
    DefaultVmar = Rights::Duplicate as u32 | Rights::Transfer as u32 | Rights::Map as u32
                 | Rights::Read as u32 | Rights::Write as u32 | Rights::Execute as u32,
}

// ============================================================================
// VM Options and Flags
// ============================================================================

// Virtual Memory Options
bitflags::bitflags! {
    pub struct VmOptions: u32 {
        const PERM_READ = 1 << 0;
        const PERM_WRITE = 1 << 1;
        const PERM_EXECUTE = 1 << 2;
        const PERM_RX = Self::PERM_READ.bits | Self::PERM_EXECUTE.bits;
        const PERM_RW = Self::PERM_READ.bits | Self::PERM_WRITE.bits;
        const PERM_RXW = Self::PERM_READ.bits | Self::PERM_WRITE.bits | Self::PERM_EXECUTE.bits;
        const SPECIFIC = 1 << 3;
        const SPECIFIC_OVERWRITE = 1 << 4;
        const COMPACT = 1 << 5;
        const CAN_MAP_RXW = 1 << 6;
        const CAN_MAP_SPECIFIC = 1 << 7;
        const MAP_RANGE = 1 << 8;
        const REQUIRE_NON_RESIZABLE = 1 << 9;
    }
}

// MMU flags for memory mappings
bitflags::bitflags! {
    pub struct MmuFlags: u32 {
        const READ = 1 << 0;
        const WRITE = 1 << 1;
        const EXECUTE = 1 << 2;
        const USER = 1 << 3;
    }
}

// VMO clone flags
bitflags::bitflags! {
    pub struct VmoCloneFlags: u32 {
        const COPY_ON_WRITE = 1 << 0;
        const RESIZABLE = 1 << 2;
        const COPY_ON_WRITE2 = 1 << 3;
        const SLICE = 1 << 4;
    }
}

// VMAR flags
bitflags::bitflags! {
    pub struct VmarFlags: u32 {
        const SPECIFIC = 1 << 0;
        const CAN_MAP_SPECIFIC = 1 << 1;
        const COMPACT = 1 << 2;
        const CAN_MAP_RXW = 1 << 3;
    }
}

// ============================================================================
// VMO Types
// ============================================================================

/// VMO types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmoType {
    Paged = 0,
    Physical = 1,
    Contiguous = 2,
    Resizable = 3,
}

/// VMO operation types
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmoOpType {
    Commit = 1,
    Decommit = 2,
    Lock = 3,
    Unlock = 4,
    CacheSync = 6,
    CacheInvalidate = 7,
    CacheClean = 8,
    CacheCleanInvalidate = 9,
    Zero = 10,
}

impl core::convert::TryFrom<u32> for VmoOpType {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, ()> {
        match value {
            1 => Ok(VmoOpType::Commit),
            2 => Ok(VmoOpType::Decommit),
            3 => Ok(VmoOpType::Lock),
            4 => Ok(VmoOpType::Unlock),
            6 => Ok(VmoOpType::CacheSync),
            7 => Ok(VmoOpType::CacheInvalidate),
            8 => Ok(VmoOpType::CacheClean),
            9 => Ok(VmoOpType::CacheCleanInvalidate),
            10 => Ok(VmoOpType::Zero),
            _ => Err(()),
        }
    }
}

/// Cache policy for VMO
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePolicy {
    Cached = 0,
    Uncached = 1,
    UncachedDevice = 2,
    WriteCombining = 3,
}

// ============================================================================
// Error Types
// ============================================================================

/// Zircon error codes
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZxError {
    Ok = 0,
    ErrInternal = -1,
    ErrNotSupported = -2,
    ErrNoMemory = -3,
    ErrCallBadState = -4,
    ErrInvalidArgs = -10,
    ErrAccessDenied = -12,
    ErrNotFound = -14,
    ErrAlreadyExists = -16,
    ErrOutOfRange = -21,
    ErrBadState = -24,
    ErrTimedOut = -30,
    ErrShouldWait = -31,
    ErrCanceled = -32,
    ErrPeerClosed = -34,
}

/// Zircon result type
pub type ZxResult<T = ()> = Result<T, ZxError>;

// ============================================================================
// Helper Functions
// ============================================================================

/// Helper: convert bytes to pages
pub fn pages(size: usize) -> usize {
    (size + crate::kernel_lowlevel::memory::PAGE_SIZE - 1) / crate::kernel_lowlevel::memory::PAGE_SIZE
}

/// Round up to page boundary
pub fn roundup_pages(size: usize) -> usize {
    (size + crate::kernel_lowlevel::memory::PAGE_SIZE - 1) & !(crate::kernel_lowlevel::memory::PAGE_SIZE - 1)
}
