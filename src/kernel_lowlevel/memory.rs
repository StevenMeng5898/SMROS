//! Memory Management Module for Multi-Process Support
//!
//! This module provides:
//! - 4K page-based memory management
//! - Segment management (code, data, heap, stack)
//! - Process address spaces with isolated memory
//! - Safe, stable Rust implementation for bare-metal targets
//!
//! # Syscall Compatibility
//!
//! This memory management system is designed to be compatible with both Linux and Zircon
//! system call interfaces, following the architecture of the grt-zcore project:
//! <https://github.com/StevenMeng5898/grt-zcore>
//!
//! ## Linux Syscall Compatibility
//! The following Linux memory syscalls are supported (see `syscall.rs`):
//! - `sys_mmap` - Map files or devices into memory
//! - `sys_munmap` - Unmap files or devices from memory
//! - `sys_mprotect` - Set protection on a region of memory
//! - `sys_brk` - Change program break (heap allocation)
//!
//! ## Zircon Syscall Compatibility
//! The following Zircon memory syscalls are supported (see `syscall.rs`):
//! - `sys_vmo_create` - Create a Virtual Memory Object
//! - `sys_vmo_read` / `sys_vmo_write` - Read/write VMO
//! - `sys_vmo_get_size` / `sys_vmo_set_size` - Query/resize VMO
//! - `sys_vmo_op_range` - Perform operations on VMO range (commit, decommit, zero)
//! - `sys_vmar_map` - Map VMO into Virtual Memory Address Region
//! - `sys_vmar_unmap` - Unmap from VMAR
//! - `sys_vmar_allocate` - Allocate subregion in VMAR
//! - `sys_vmar_protect` - Set protection on VMAR pages
//! - `sys_vmar_destroy` - Destroy VMAR
//!
//! ## Architecture Mapping
//!
//! SMROS Component          | Zircon Equivalent     | Linux Equivalent
//! -------------------------|----------------------|------------------
//! ProcessAddressSpace      | Process + VMAR       | mm_struct
//! MemorySegment            | VMO mapping          | vm_area_struct
//! PageEntry                | Page table entry     | PTE
//! PageFrameAllocator       | PhysAlloc            | buddy allocator
//! ProcessControlBlock      | Process object       | task_struct
//! heap_alloc()             | vmar.allocate()      | brk/mmap
//! stack_alloc()            | vmar.map(stack)      | mmap(MAP_STACK)
//!
//! # Memory Layout per Process
//! ```text
//! 0x0000_0000_0000_0000 - Code Segment (text)
//! 0x0000_0000_0001_0000 - Data Segment
//! 0x0000_0000_0002_0000 - Heap Segment (grows upward)
//! ...
//! 0x0000_0000_FFFF_0000 - Stack Segment (grows downward)
//! ```

use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_arch = "aarch64")]
use super::aarch64_vm_logic_shared as aarch64_vm_logic;
use super::lowlevel_logic;

/// Page size: 4 KiB, matching the currently supported ARM64 granule and
/// RISC-V Sv39/Sv48 page size.
pub const PAGE_SIZE: usize = 0x1000;

const PAGE_FRAME_BITMAP_WORDS: usize = (2 * 1024 * 1024 * 1024 / PAGE_SIZE) / 64;
const PAGE_FRAME_BITS_PER_WORD: usize = 64;
const DEFAULT_PAGE_FRAME_COUNT: usize = 64 * PAGE_FRAME_BITS_PER_WORD;

/// Maximum number of processes supported
pub const MAX_PROCESSES: usize = 16;
const MAX_DYNAMIC_PROCESS_NAMES: usize = 16;
const DYNAMIC_PROCESS_NAME_LEN: usize = 32;

/// Maximum number of pages per process
pub const MAX_PAGES_PER_PROCESS: usize = 64;

/// Maximum number of segments per process
pub const MAX_SEGMENTS: usize = 4;

/// Segment types for process memory
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SegmentType {
    /// Code segment (read-only, executable)
    Code = 0,
    /// Data segment (read-write, initialized)
    Data = 1,
    /// Heap segment (read-write, grows upward)
    Heap = 2,
    /// Stack segment (read-write, grows downward)
    Stack = 3,
}

impl SegmentType {
    /// Get string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            SegmentType::Code => "Code",
            SegmentType::Data => "Data",
            SegmentType::Heap => "Heap",
            SegmentType::Stack => "Stack",
        }
    }
}

/// Segment permissions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SegmentPermission {
    Read = 0b001,
    Write = 0b010,
    Execute = 0b100,
    ReadWrite = 0b011,
    ReadExecute = 0b101,
}

impl SegmentPermission {
    pub fn as_str(&self) -> &'static str {
        match self {
            SegmentPermission::Read => "r--",
            SegmentPermission::Write => "-w-",
            SegmentPermission::Execute => "--x",
            SegmentPermission::ReadWrite => "rw-",
            SegmentPermission::ReadExecute => "r-x",
        }
    }
}

/// Memory segment descriptor
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MemorySegment {
    /// Segment type
    pub seg_type: SegmentType,
    /// Base virtual address
    pub base_vaddr: usize,
    /// Number of pages in this segment
    pub page_count: usize,
    /// Segment permissions
    pub permissions: SegmentPermission,
    /// Whether this segment is valid
    pub valid: bool,
}

impl MemorySegment {
    /// Create a new memory segment
    pub const fn new() -> Self {
        MemorySegment {
            seg_type: SegmentType::Code,
            base_vaddr: 0,
            page_count: 0,
            permissions: SegmentPermission::Read,
            valid: false,
        }
    }

    /// Get segment end address
    pub fn end_vaddr(&self) -> usize {
        lowlevel_logic::segment_end(self.valid, self.base_vaddr, self.page_count, PAGE_SIZE)
            .unwrap_or(usize::MAX)
    }
}

/// Page table entry for a process
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PageEntry {
    /// Physical page frame number
    pub pfn: u64,
    /// Whether this page is valid (mapped)
    pub valid: bool,
    /// Whether this page is writable
    pub writable: bool,
    /// Whether this page is executable
    pub executable: bool,
    /// Whether this page is user-accessible
    pub user_accessible: bool,
}

impl PageEntry {
    /// Create an invalid page entry
    pub const fn invalid() -> Self {
        PageEntry {
            pfn: 0,
            valid: false,
            writable: false,
            executable: false,
            user_accessible: false,
        }
    }
}

/// Process address space - manages virtual memory for a single process
#[repr(C)]
pub struct ProcessAddressSpace {
    /// Process ID this address space belongs to
    pub pid: usize,
    /// Page table entries
    pub pages: [PageEntry; MAX_PAGES_PER_PROCESS],
    /// Number of valid pages
    pub valid_page_count: usize,
    /// Memory segments
    pub segments: [MemorySegment; MAX_SEGMENTS],
    /// Number of valid segments
    pub valid_segment_count: usize,
    /// Heap current top (relative to heap base)
    pub heap_current: usize,
    /// Heap maximum size
    pub heap_max: usize,
    /// Stack top (highest address)
    pub stack_top: usize,
    /// Stack current pointer (grows downward)
    pub stack_current: usize,
}

impl ProcessAddressSpace {
    /// Create a new empty address space
    pub const fn new() -> Self {
        ProcessAddressSpace {
            pid: 0,
            pages: [const { PageEntry::invalid() }; MAX_PAGES_PER_PROCESS],
            valid_page_count: 0,
            segments: [const { MemorySegment::new() }; MAX_SEGMENTS],
            valid_segment_count: 0,
            heap_current: 0,
            heap_max: 0,
            stack_top: 0,
            stack_current: 0,
        }
    }

    /// Initialize address space for a process
    ///
    /// Sets up standard memory layout:
    /// - Code segment: 1 page at 0x0
    /// - Data segment: 1 page at 0x1000
    /// - Heap segment: 4 pages at 0x2000 (grows upward)
    /// - Stack segment: 2 pages at 0xF000 (grows downward)
    pub fn init(&mut self, pid: usize) -> bool {
        self.pid = pid;
        self.valid_page_count = 0;
        self.valid_segment_count = 0;
        self.heap_current = 0;
        self.stack_current = 0;

        // Allocate code segment (1 page)
        if !self.add_segment(SegmentType::Code, 0x0000, 1, SegmentPermission::ReadExecute) {
            return false;
        }

        // Allocate data segment (1 page)
        if !self.add_segment(SegmentType::Data, 0x1000, 1, SegmentPermission::ReadWrite) {
            return false;
        }

        // Allocate heap segment (4 pages, 16KB)
        if !self.add_segment(SegmentType::Heap, 0x2000, 4, SegmentPermission::ReadWrite) {
            return false;
        }
        self.heap_current = 0x2000;
        self.heap_max = 0x2000 + (4 * PAGE_SIZE);

        // Allocate stack segment (2 pages, 8KB)
        if !self.add_segment(SegmentType::Stack, 0xF000, 2, SegmentPermission::ReadWrite) {
            return false;
        }
        self.stack_top = 0xF000 + (2 * PAGE_SIZE);
        self.stack_current = self.stack_top;

        true
    }

    /// Add a memory segment to this address space
    pub fn add_segment(
        &mut self,
        seg_type: SegmentType,
        base_vaddr: usize,
        page_count: usize,
        permissions: SegmentPermission,
    ) -> bool {
        if self.valid_segment_count >= MAX_SEGMENTS {
            return false;
        }

        if !lowlevel_logic::memory_capacity_ok(
            self.valid_segment_count,
            page_count,
            self.valid_page_count,
            MAX_SEGMENTS,
            MAX_PAGES_PER_PROCESS,
        ) {
            return false;
        }

        // Allocate physical pages for this segment
        let start_page_idx = self.valid_page_count;
        for i in 0..page_count {
            let page_idx = start_page_idx + i;
            if let Some(pfn) = PageFrameAllocator::alloc() {
                self.pages[page_idx] = PageEntry {
                    pfn,
                    valid: true,
                    writable: lowlevel_logic::permission_writable(
                        permissions,
                        SegmentPermission::Write,
                        SegmentPermission::ReadWrite,
                    ),
                    executable: lowlevel_logic::permission_executable(
                        permissions,
                        SegmentPermission::Execute,
                        SegmentPermission::ReadExecute,
                    ),
                    user_accessible: true,
                };
            } else {
                // Rollback on failure
                for j in 0..i {
                    let page_idx = start_page_idx + j;
                    if self.pages[page_idx].valid {
                        PageFrameAllocator::free(self.pages[page_idx].pfn);
                        self.pages[page_idx] = PageEntry::invalid();
                    }
                }
                return false;
            }
        }

        // Add segment descriptor
        let seg_idx = self.valid_segment_count;
        self.segments[seg_idx] = MemorySegment {
            seg_type,
            base_vaddr,
            page_count,
            permissions,
            valid: true,
        };

        self.valid_page_count += page_count;
        self.valid_segment_count += 1;

        true
    }
}

/// Process Control Block (PCB) - represents a process
#[repr(C)]
pub struct ProcessControlBlock {
    /// Process ID
    pub pid: usize,
    /// Process state
    pub state: ProcessState,
    /// Process address space
    pub address_space: ProcessAddressSpace,
    /// Parent process ID (0 if init)
    pub parent_pid: usize,
    /// Process name
    pub name: &'static str,
    /// Number of threads in this process
    pub thread_count: usize,
}

impl ProcessControlBlock {
    /// Create a new empty PCB
    pub const fn new() -> Self {
        ProcessControlBlock {
            pid: 0,
            state: ProcessState::Empty,
            address_space: ProcessAddressSpace::new(),
            parent_pid: 0,
            name: "",
            thread_count: 0,
        }
    }

    /// Initialize a new process
    pub fn init(&mut self, pid: usize, parent_pid: usize, name: &'static str) -> bool {
        self.pid = pid;
        self.parent_pid = parent_pid;
        self.name = name;
        self.state = ProcessState::Ready;
        self.thread_count = 0;

        self.address_space.init(pid)
    }
}

/// Process states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ProcessState {
    Empty = 0,
    Ready = 1,
    Running = 2,
    Terminated = 4,
}

impl ProcessState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProcessState::Empty => "Empty     ",
            ProcessState::Ready => "Ready     ",
            ProcessState::Running => "Running   ",
            ProcessState::Terminated => "Terminated",
        }
    }
}

/// Page frame allocator - manages physical page frames
///
/// Uses a simple bitmap allocator for physical pages.
/// In a real kernel, you'd use a more sophisticated allocator (buddy, slab).
pub struct PageFrameAllocator {
    core: lowlevel_logic::PageFrameAllocatorCore<PAGE_FRAME_BITMAP_WORDS>,
}

impl PageFrameAllocator {
    /// Create a new page frame allocator
    const fn new() -> Self {
        PageFrameAllocator {
            core: lowlevel_logic::PageFrameAllocatorCore::new(DEFAULT_PAGE_FRAME_COUNT),
        }
    }

    pub fn init_range(start: usize, end: usize) -> bool {
        let allocator = unsafe { &mut *ALLOCATOR.get() };
        allocator.core.init_range(start, end, PAGE_SIZE)
    }

    /// Allocate a single page frame
    /// Returns the page frame number (PFN)
    pub fn alloc() -> Option<u64> {
        // SAFETY: We use interior mutability with careful synchronization.
        // In a single-threaded kernel context, this is safe.
        let allocator = unsafe { &mut *ALLOCATOR.get() };

        allocator.core.alloc()
    }

    /// Free a page frame
    pub fn free(pfn: u64) {
        let allocator = unsafe { &mut *ALLOCATOR.get() };
        let _ = allocator.core.free(pfn);
    }

    pub fn pfn_address(pfn: u64) -> Option<usize> {
        let allocator = unsafe { &*ALLOCATOR.get() };
        allocator.core.pfn_address(pfn, PAGE_SIZE)
    }

    /// Get total number of pages
    pub fn total_pages() -> usize {
        let allocator = unsafe { &*ALLOCATOR.get() };
        allocator.core.total_pages()
    }

    /// Get number of allocated pages
    pub fn allocated_pages() -> usize {
        let allocator = unsafe { &*ALLOCATOR.get() };
        allocator.core.allocated_pages()
    }

    /// Get number of free pages
    pub fn free_pages() -> usize {
        let allocator = unsafe { &*ALLOCATOR.get() };
        allocator.core.free_pages()
    }
}

/// Global page frame allocator with interior mutability
struct AllocatorCell(core::cell::UnsafeCell<PageFrameAllocator>);
unsafe impl Sync for AllocatorCell {}
impl AllocatorCell {
    fn get(&self) -> *mut PageFrameAllocator {
        self.0.get()
    }
}

static ALLOCATOR: AllocatorCell =
    AllocatorCell(core::cell::UnsafeCell::new(PageFrameAllocator::new()));

/// Process manager - manages all processes in the system
pub struct ProcessManager {
    /// Process control blocks
    processes: [ProcessControlBlock; MAX_PROCESSES],
    dynamic_names: [[u8; DYNAMIC_PROCESS_NAME_LEN]; MAX_DYNAMIC_PROCESS_NAMES],
    dynamic_name_used: [bool; MAX_DYNAMIC_PROCESS_NAMES],
    /// Number of active processes
    active_processes: usize,
    /// Next PID to allocate
    next_pid: AtomicU64,
}

impl ProcessManager {
    /// Create a new process manager
    pub const fn new() -> Self {
        ProcessManager {
            processes: [const { ProcessControlBlock::new() }; MAX_PROCESSES],
            dynamic_names: [[0; DYNAMIC_PROCESS_NAME_LEN]; MAX_DYNAMIC_PROCESS_NAMES],
            dynamic_name_used: [false; MAX_DYNAMIC_PROCESS_NAMES],
            active_processes: 0,
            next_pid: AtomicU64::new(1),
        }
    }

    /// Initialize the process manager
    pub fn init(&mut self) {
        // Create init process (PID 1)
        if let Some(ref mut pcb) = self.get_process_mut(0) {
            if pcb.init(1, 0, "init") {
                self.active_processes = 1;
                self.next_pid.store(2, Ordering::Relaxed);
            }
        }
    }

    /// Create a new process
    pub fn create_process(&mut self, name: &'static str) -> Option<usize> {
        // Find an empty slot
        for i in 0..MAX_PROCESSES {
            if self.processes[i].state == ProcessState::Empty
                || self.processes[i].state == ProcessState::Terminated
            {
                if self.processes[i].state == ProcessState::Terminated {
                    self.release_dynamic_name(self.processes[i].name);
                }
                let pid = self.next_pid.load(Ordering::Relaxed) as usize;
                let parent_pid = 1; // Init is parent

                if self.processes[i].init(pid, parent_pid, name) {
                    self.next_pid.fetch_add(1, Ordering::Relaxed);
                    self.active_processes += 1;
                    return Some(pid);
                }
            }
        }

        None // No available slots
    }

    /// Create a VM process with a stable dynamic name visible to ps/top.
    pub fn create_vm_process(&mut self, vm_name: &str) -> Option<usize> {
        let name = self.alloc_vm_process_name(vm_name)?;
        match self.create_process(name) {
            Some(pid) => {
                if let Some(pcb) = self.get_process_by_pid_mut(pid) {
                    pcb.state = ProcessState::Running;
                    pcb.thread_count = 1;
                }
                Some(pid)
            }
            None => {
                self.release_dynamic_name(name);
                None
            }
        }
    }

    /// Get a process by index
    pub fn get_process(&self, index: usize) -> Option<&ProcessControlBlock> {
        if lowlevel_logic::process_index_valid(index, MAX_PROCESSES) {
            Some(&self.processes[index])
        } else {
            None
        }
    }

    /// Get a mutable reference to a process by index
    pub fn get_process_mut(&mut self, index: usize) -> Option<&mut ProcessControlBlock> {
        if lowlevel_logic::process_index_valid(index, MAX_PROCESSES) {
            Some(&mut self.processes[index])
        } else {
            None
        }
    }

    /// Get a mutable reference to a process by PID
    pub fn get_process_by_pid_mut(&mut self, pid: usize) -> Option<&mut ProcessControlBlock> {
        for i in 0..MAX_PROCESSES {
            if self.processes[i].pid == pid && self.processes[i].state != ProcessState::Empty {
                return Some(&mut self.processes[i]);
            }
        }
        None
    }

    /// Terminate a process
    pub fn terminate_process(&mut self, pid: usize) -> bool {
        if let Some(pcb) = self.get_process_by_pid_mut(pid) {
            if pcb.state == ProcessState::Terminated {
                return true;
            }
            if pcb.state == ProcessState::Empty {
                return false;
            }
            // Free all pages
            for i in 0..pcb.address_space.valid_page_count {
                if pcb.address_space.pages[i].valid {
                    PageFrameAllocator::free(pcb.address_space.pages[i].pfn);
                    pcb.address_space.pages[i] = PageEntry::invalid();
                }
            }

            pcb.state = ProcessState::Terminated;
            self.active_processes = self.active_processes.saturating_sub(1);
            true
        } else {
            false
        }
    }

    fn alloc_vm_process_name(&mut self, vm_name: &str) -> Option<&'static str> {
        let slot = self.dynamic_name_used.iter().position(|used| !*used)?;
        let out = &mut self.dynamic_names[slot];
        out.fill(0);

        let prefix = b"vm:";
        out[..prefix.len()].copy_from_slice(prefix);
        let max_name_len = DYNAMIC_PROCESS_NAME_LEN.saturating_sub(prefix.len() + 1);
        let mut written = 0usize;
        for byte in vm_name.bytes() {
            if written >= max_name_len {
                break;
            }
            out[prefix.len() + written] = if matches!(
                byte,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.'
            ) {
                byte
            } else {
                b'_'
            };
            written += 1;
        }
        if written == 0 {
            return None;
        }

        self.dynamic_name_used[slot] = true;
        let len = prefix.len() + written;
        if core::str::from_utf8(&out[..len]).is_err() {
            self.dynamic_name_used[slot] = false;
            return None;
        }
        let ptr = out.as_ptr();
        let name: &'static str =
            unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len)) };
        Some(name)
    }

    fn release_dynamic_name(&mut self, name: &'static str) {
        let ptr = name.as_ptr() as usize;
        for index in 0..MAX_DYNAMIC_PROCESS_NAMES {
            let start = self.dynamic_names[index].as_ptr() as usize;
            let end = start + DYNAMIC_PROCESS_NAME_LEN;
            if ptr >= start && ptr < end {
                self.dynamic_name_used[index] = false;
                self.dynamic_names[index].fill(0);
                return;
            }
        }
    }

    /// Get active process count
    pub fn active_processes(&self) -> usize {
        self.active_processes
    }
}

/// Global process manager
struct ProcessManagerCell(core::cell::UnsafeCell<ProcessManager>);
unsafe impl Sync for ProcessManagerCell {}

static PROCESS_MANAGER: ProcessManagerCell =
    ProcessManagerCell(core::cell::UnsafeCell::new(ProcessManager::new()));

/// Get a mutable reference to the global process manager
pub fn process_manager() -> &'static mut ProcessManager {
    unsafe { &mut *PROCESS_MANAGER.0.get() }
}

/// Initialize the memory management subsystem
pub fn init() {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();

    serial.write_str("[MEM] Initializing memory management...\n");

    #[cfg(target_arch = "aarch64")]
    {
        unsafe extern "C" {
            static __kernel_end: u8;
        }

        let memory = crate::kernel_lowlevel::drivers::memory_reg()
            .expect("AArch64 RAM range must be available");
        let ram_end = memory
            .base
            .checked_add(memory.size)
            .expect("AArch64 RAM range must not overflow");
        let kernel_end = core::ptr::addr_of!(__kernel_end) as usize;
        let (frame_start, detected_frame_end) =
            aarch64_vm_logic::aarch64_frame_range(kernel_end, memory.base, ram_end)
                .expect("AArch64 RAM must contain frames after the kernel");
        let (_, frame_end) = aarch64_vm_logic::aarch64_frame_range_cap(
            frame_start,
            detected_frame_end,
            PAGE_FRAME_BITMAP_WORDS * PAGE_SIZE * PAGE_FRAME_BITS_PER_WORD,
        )
        .expect("AArch64 RAM must contain frames within allocator capacity");
        assert!(PageFrameAllocator::init_range(frame_start, frame_end));
    }

    // Initialize process manager
    process_manager().init();

    serial.write_str("[MEM] Process manager initialized with init process (PID 1)\n");
    serial.write_str("[MEM] Page size: 4KB (");
    crate::kernel_lowlevel::smp::print_number(&mut serial, (PAGE_SIZE / 1024) as u32);
    serial.write_str(" KB)\n");
    serial.write_str("[MEM] Max processes: ");
    crate::kernel_lowlevel::smp::print_number(&mut serial, MAX_PROCESSES as u32);
    serial.write_str(", Max pages per process: ");
    crate::kernel_lowlevel::smp::print_number(&mut serial, MAX_PAGES_PER_PROCESS as u32);
    serial.write_str("\n");
}
