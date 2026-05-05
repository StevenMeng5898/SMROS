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
    fn from(v: u32) -> Self {
        Self(v)
    }
}

impl From<HandleValue> for u32 {
    fn from(h: HandleValue) -> Self {
        h.0
    }
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
    EventPair = 12,
    Fifo = 13,
    Stream = 14,
    DebugLog = 15,
    Clock = 16,
    Job = 17,
    SuspendToken = 18,
    Exception = 19,
    Iommu = 20,
    Bti = 21,
    Pmt = 22,
    PciDevice = 23,
    Guest = 24,
    Vcpu = 25,
    Semaphore = 26,
    SharedMemory = 27,
    Profile = 28,
    Pager = 29,
    Framebuffer = 30,
    Ktrace = 31,
    Mtrace = 32,
    MessageQueue = 33,
    EventFd = 34,
    SignalFd = 35,
    TimerFd = 36,
    Inotify = 37,
    IoUring = 38,
    MemFd = 39,
    PidFd = 40,
    Futex = 41,
    LinuxFile = 42,
    LinuxPipe = 43,
    LinuxTcpSocket = 44,
    LinuxUdpSocket = 45,
    LinuxRawSocket = 46,
    LinuxNetlinkSocket = 47,
    LinuxProcess = 48,
    LinuxThread = 49,
    LinuxSignal = 50,
    LinuxEventBus = 51,
    LinuxDevice = 52,
    LinuxDir = 53,
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
    Enumerate = 1 << 8,
    Destroy = 1 << 9,
    SetPolicy = 1 << 10,
    GetPolicy = 1 << 11,
    Signal = 1 << 12,
    SignalPeer = 1 << 13,
    Wait = 1 << 14,
    Inspect = 1 << 15,
    ManageJob = 1 << 16,
    ManageProcess = 1 << 17,
    ManageThread = 1 << 18,
    ApplyProfile = 1 << 19,
    ManageSocket = 1 << 20,
    OpChildren = 1 << 21,
    Resize = 1 << 22,
    AttachVmo = 1 << 23,
    ManageVmo = 1 << 24,
    DefaultVmo = Rights::Duplicate as u32
        | Rights::Transfer as u32
        | Rights::Read as u32
        | Rights::Write as u32
        | Rights::Map as u32
        | Rights::GetProperty as u32
        | Rights::SetProperty as u32
        | Rights::Signal as u32
        | Rights::Inspect as u32
        | Rights::Resize as u32,
    DefaultVmar = Rights::Duplicate as u32
        | Rights::Transfer as u32
        | Rights::Read as u32
        | Rights::Write as u32
        | Rights::Execute as u32
        | Rights::Map as u32
        | Rights::GetProperty as u32
        | Rights::SetProperty as u32
        | Rights::Destroy as u32
        | Rights::Inspect as u32
        | Rights::OpChildren as u32,
    DefaultChannel = Rights::Duplicate as u32
        | Rights::Transfer as u32
        | Rights::Read as u32
        | Rights::Write as u32
        | Rights::Signal as u32
        | Rights::SignalPeer as u32
        | Rights::Wait as u32,
    DefaultJob = Rights::Duplicate as u32
        | Rights::Transfer as u32
        | Rights::Read as u32
        | Rights::Write as u32
        | Rights::GetProperty as u32
        | Rights::SetProperty as u32
        | Rights::Enumerate as u32
        | Rights::Destroy as u32
        | Rights::SetPolicy as u32
        | Rights::GetPolicy as u32
        | Rights::Signal as u32
        | Rights::Wait as u32
        | Rights::Inspect as u32
        | Rights::ManageJob as u32
        | Rights::ManageProcess as u32
        | Rights::ManageThread as u32,
    DefaultProcess = Rights::Duplicate as u32
        | Rights::Transfer as u32
        | Rights::Read as u32
        | Rights::Write as u32
        | Rights::GetProperty as u32
        | Rights::SetProperty as u32
        | Rights::Enumerate as u32
        | Rights::Destroy as u32
        | Rights::Signal as u32
        | Rights::Wait as u32
        | Rights::Inspect as u32
        | Rights::ManageProcess as u32
        | Rights::ManageThread as u32,
    DefaultThread = Rights::Duplicate as u32
        | Rights::Transfer as u32
        | Rights::Read as u32
        | Rights::Write as u32
        | Rights::GetProperty as u32
        | Rights::SetProperty as u32
        | Rights::Destroy as u32
        | Rights::Signal as u32
        | Rights::Wait as u32
        | Rights::Inspect as u32
        | Rights::ManageThread as u32,
}

pub const RIGHT_SAME_RIGHTS: u32 = 0x8000_0000;
pub const RIGHTS_BASIC: u32 = Rights::Duplicate as u32
    | Rights::Transfer as u32
    | Rights::Wait as u32
    | Rights::Inspect as u32;
pub const RIGHTS_PROPERTY: u32 = Rights::GetProperty as u32 | Rights::SetProperty as u32;
pub const RIGHTS_POLICY: u32 = Rights::GetPolicy as u32 | Rights::SetPolicy as u32;
pub const RIGHTS_SIGNAL: u32 = Rights::Signal as u32 | Rights::SignalPeer as u32;
pub const RIGHTS_IO: u32 = Rights::Read as u32 | Rights::Write as u32 | Rights::Execute as u32;
pub const RIGHTS_MEMORY: u32 = Rights::Map as u32;
pub const RIGHTS_TASK: u32 = Rights::Enumerate as u32
    | Rights::Destroy as u32
    | Rights::ManageJob as u32
    | Rights::ManageProcess as u32
    | Rights::ManageThread as u32
    | Rights::ApplyProfile as u32;
pub const RIGHTS_SOCKET: u32 = Rights::ManageSocket as u32;
pub const RIGHTS_EXTENDED: u32 = Rights::OpChildren as u32
    | Rights::Resize as u32
    | Rights::AttachVmo as u32
    | Rights::ManageVmo as u32;
pub const RIGHTS_ALL: u32 =
    RIGHTS_BASIC
        | RIGHTS_PROPERTY
        | RIGHTS_POLICY
        | RIGHTS_SIGNAL
        | RIGHTS_IO
        | RIGHTS_MEMORY
        | RIGHTS_TASK
        | RIGHTS_SOCKET
        | RIGHTS_EXTENDED;

pub const DEFAULT_EVENT_RIGHTS: u32 = Rights::Duplicate as u32
    | Rights::Transfer as u32
    | Rights::GetProperty as u32
    | Rights::SetProperty as u32
    | Rights::Signal as u32
    | Rights::Wait as u32
    | Rights::Inspect as u32;

pub const DEFAULT_EVENTPAIR_RIGHTS: u32 = DEFAULT_EVENT_RIGHTS | Rights::SignalPeer as u32;

pub const DEFAULT_PORT_RIGHTS: u32 = Rights::Duplicate as u32
    | Rights::Transfer as u32
    | Rights::Read as u32
    | Rights::Write as u32
    | Rights::GetProperty as u32
    | Rights::SetProperty as u32
    | Rights::Inspect as u32;

pub const DEFAULT_SOCKET_RIGHTS: u32 = Rights::DefaultChannel as u32
    | Rights::GetProperty as u32
    | Rights::SetProperty as u32
    | Rights::Inspect as u32;

pub const DEFAULT_RESOURCE_RIGHTS: u32 = Rights::Duplicate as u32
    | Rights::Transfer as u32
    | Rights::Read as u32
    | Rights::Write as u32
    | Rights::Inspect as u32;

pub const DEFAULT_INTERRUPT_RIGHTS: u32 =
    RIGHTS_BASIC | RIGHTS_IO | Rights::Signal as u32;

pub const DEFAULT_STREAM_RIGHTS: u32 = RIGHTS_BASIC | RIGHTS_PROPERTY | Rights::Signal as u32;

pub const DEFAULT_CLOCK_RIGHTS: u32 = RIGHTS_BASIC | RIGHTS_IO;

pub const DEFAULT_SUSPEND_TOKEN_RIGHTS: u32 = Rights::Transfer as u32 | Rights::Inspect as u32;

pub const DEFAULT_EXCEPTION_RIGHTS: u32 =
    Rights::Transfer as u32 | RIGHTS_PROPERTY | Rights::Inspect as u32;

pub fn rights_are_valid(rights: u32) -> bool {
    super::object_logic::rights_valid(rights, RIGHTS_ALL)
}

pub fn rights_contain(rights: u32, required: u32) -> bool {
    super::object_logic::rights_has(rights, required)
}

pub fn rights_are_subset(requested: u32, existing: u32) -> bool {
    super::object_logic::rights_subset(requested, existing)
}

pub fn default_rights_for_object(obj_type: ObjectType) -> u32 {
    match obj_type {
        ObjectType::Vmo => Rights::DefaultVmo as u32,
        ObjectType::Vmar => Rights::DefaultVmar as u32,
        ObjectType::Channel => Rights::DefaultChannel as u32,
        ObjectType::Job => Rights::DefaultJob as u32,
        ObjectType::Process | ObjectType::LinuxProcess => Rights::DefaultProcess as u32,
        ObjectType::Thread | ObjectType::LinuxThread => Rights::DefaultThread as u32,
        ObjectType::Fifo => Rights::DefaultChannel as u32,
        ObjectType::Socket => DEFAULT_SOCKET_RIGHTS,
        ObjectType::Event => DEFAULT_EVENT_RIGHTS,
        ObjectType::EventPair => DEFAULT_EVENTPAIR_RIGHTS,
        ObjectType::Port => DEFAULT_PORT_RIGHTS,
        ObjectType::Timer => DEFAULT_EVENT_RIGHTS,
        ObjectType::Resource => DEFAULT_RESOURCE_RIGHTS,
        ObjectType::Interrupt => DEFAULT_INTERRUPT_RIGHTS,
        ObjectType::Stream => DEFAULT_STREAM_RIGHTS,
        ObjectType::DebugLog => DEFAULT_SOCKET_RIGHTS,
        ObjectType::Clock => DEFAULT_CLOCK_RIGHTS,
        ObjectType::SuspendToken => DEFAULT_SUSPEND_TOKEN_RIGHTS,
        ObjectType::Exception => DEFAULT_EXCEPTION_RIGHTS,
        ObjectType::Iommu
        | ObjectType::Bti
        | ObjectType::Pmt
        | ObjectType::PciDevice
        | ObjectType::Guest
        | ObjectType::Vcpu
        | ObjectType::Semaphore
        | ObjectType::SharedMemory
        | ObjectType::Profile
        | ObjectType::Pager
        | ObjectType::Framebuffer
        | ObjectType::Ktrace
        | ObjectType::Mtrace
        | ObjectType::MessageQueue
        | ObjectType::EventFd
        | ObjectType::SignalFd
        | ObjectType::TimerFd
        | ObjectType::Inotify
        | ObjectType::IoUring
        | ObjectType::MemFd
        | ObjectType::PidFd
        | ObjectType::Futex
        | ObjectType::LinuxFile
        | ObjectType::LinuxPipe
        | ObjectType::LinuxTcpSocket
        | ObjectType::LinuxUdpSocket
        | ObjectType::LinuxRawSocket
        | ObjectType::LinuxNetlinkSocket
        | ObjectType::LinuxSignal
        | ObjectType::LinuxEventBus
        | ObjectType::LinuxDevice
        | ObjectType::LinuxDir => DEFAULT_EVENT_RIGHTS | RIGHTS_IO,
    }
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
    super::object_logic::pages(size, crate::kernel_lowlevel::memory::PAGE_SIZE)
}

/// Round up to page boundary
pub fn roundup_pages(size: usize) -> usize {
    super::object_logic::roundup_pages(size, crate::kernel_lowlevel::memory::PAGE_SIZE)
}
