//! Memory Management Module for Multi-Process Support
//!
//! This module provides:
//! - 4K page-based memory management
//! - Segment management (code, data, heap, stack)
//! - Process address spaces with isolated memory
//! - Safe, stable Rust implementation for bare-metal ARM64
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

#![allow(dead_code)]

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

/// Page size: 4KB (standard ARM64 granule)
pub const PAGE_SIZE: usize = 0x1000;

/// Page size mask (4KB aligned)
pub const PAGE_MASK: usize = !(PAGE_SIZE - 1);

/// Maximum number of processes supported
pub const MAX_PROCESSES: usize = 16;

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

    /// Get segment size in bytes
    pub fn size(&self) -> usize {
        self.page_count * PAGE_SIZE
    }

    /// Get segment end address
    pub fn end_vaddr(&self) -> usize {
        if self.valid {
            self.base_vaddr + self.size()
        } else {
            0
        }
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

        if page_count == 0 || self.valid_page_count + page_count > MAX_PAGES_PER_PROCESS {
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
                    writable: permissions == SegmentPermission::ReadWrite
                        || permissions == SegmentPermission::Write,
                    executable: permissions == SegmentPermission::ReadExecute
                        || permissions == SegmentPermission::Execute,
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

    /// Allocate heap memory (grow heap upward)
    pub fn heap_alloc(&mut self, size: usize) -> Option<usize> {
        // Align to page boundary
        let aligned_size = (size + PAGE_SIZE - 1) & PAGE_MASK;

        if self.heap_current + aligned_size > self.heap_max {
            return None;
        }

        let addr = self.heap_current;
        self.heap_current += aligned_size;
        Some(addr)
    }

    /// Allocate stack space (grow stack downward)
    pub fn stack_alloc(&mut self, size: usize) -> Option<usize> {
        // Align to page boundary
        let aligned_size = (size + PAGE_SIZE - 1) & PAGE_MASK;

        if self.stack_current < aligned_size {
            return None;
        }

        self.stack_current -= aligned_size;
        Some(self.stack_current)
    }

    /// Get virtual address for a page index
    pub fn page_to_vaddr(&self, page_idx: usize) -> Option<usize> {
        if page_idx >= self.valid_page_count {
            return None;
        }

        // Simple linear mapping: page_idx * PAGE_SIZE
        Some(page_idx * PAGE_SIZE)
    }

    /// Find segment containing a virtual address
    pub fn find_segment_for_vaddr(&self, vaddr: usize) -> Option<&MemorySegment> {
        for i in 0..self.valid_segment_count {
            let seg = &self.segments[i];
            if seg.valid && vaddr >= seg.base_vaddr && vaddr < seg.end_vaddr() {
                return Some(seg);
            }
        }
        None
    }

    /// Check if virtual address is valid (mapped)
    pub fn is_valid_vaddr(&self, vaddr: usize) -> bool {
        self.find_segment_for_vaddr(vaddr).is_some()
    }

    /// Print address space information
    pub fn print_info(&self, serial: &mut crate::kernel_lowlevel::serial::Serial) {
        serial.write_str("  Process ");
        crate::kernel_lowlevel::smp::print_number(serial, self.pid as u32);
        serial.write_str(" Address Space:\n");

        serial.write_str("    Segments:\n");
        for i in 0..self.valid_segment_count {
            let seg = &self.segments[i];
            if !seg.valid {
                continue;
            }

            serial.write_str("      [");
            serial.write_str(seg.seg_type.as_str());
            serial.write_str("] 0x");
            print_hex(serial, seg.base_vaddr as u64);
            serial.write_str(" - 0x");
            print_hex(serial, seg.end_vaddr() as u64);
            serial.write_str(" (");
            crate::kernel_lowlevel::smp::print_number(serial, seg.page_count as u32);
            serial.write_str(" pages, ");
            serial.write_str(seg.permissions.as_str());
            serial.write_str(")\n");
        }

        serial.write_str("    Valid Pages: ");
        crate::kernel_lowlevel::smp::print_number(serial, self.valid_page_count as u32);
        serial.write_str("/");
        crate::kernel_lowlevel::smp::print_number(serial, MAX_PAGES_PER_PROCESS as u32);
        serial.write_str("\n");

        serial.write_str("    Heap: 0x");
        print_hex(serial, 0x2000);
        serial.write_str(" - 0x");
        print_hex(serial, self.heap_current as u64);
        serial.write_str(" (used: ");
        crate::kernel_lowlevel::smp::print_number(
            serial,
            ((self.heap_current - 0x2000) / 1024) as u32,
        );
        serial.write_str("KB)\n");

        serial.write_str("    Stack: 0x");
        print_hex(serial, self.stack_current as u64);
        serial.write_str(" - 0x");
        print_hex(serial, self.stack_top as u64);
        serial.write_str("\n");
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
    Blocked = 3,
    Terminated = 4,
}

impl ProcessState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProcessState::Empty => "Empty     ",
            ProcessState::Ready => "Ready     ",
            ProcessState::Running => "Running   ",
            ProcessState::Blocked => "Blocked   ",
            ProcessState::Terminated => "Terminated",
        }
    }
}

/// Page frame allocator - manages physical page frames
///
/// Uses a simple bitmap allocator for physical pages.
/// In a real kernel, you'd use a more sophisticated allocator (buddy, slab).
pub struct PageFrameAllocator {
    /// Bitmap of allocated pages (each bit represents one 4K page)
    bitmap: [u64; 64], // Manages 64 * 64 = 4096 pages = 16MB
    /// Total number of available pages
    total_pages: usize,
    /// Number of allocated pages
    allocated_pages: usize,
}

impl PageFrameAllocator {
    /// Create a new page frame allocator
    const fn new() -> Self {
        PageFrameAllocator {
            bitmap: [0; 64],
            total_pages: 4096,
            allocated_pages: 0,
        }
    }

    /// Allocate a single page frame
    /// Returns the page frame number (PFN)
    pub fn alloc() -> Option<u64> {
        // SAFETY: We use interior mutability with careful synchronization.
        // In a single-threaded kernel context, this is safe.
        let allocator = unsafe { &mut *ALLOCATOR.get() };

        for i in 0..64 {
            if allocator.bitmap[i] == u64::MAX {
                continue;
            }

            for bit in 0..64 {
                let page_idx = i * 64 + bit;
                if page_idx >= allocator.total_pages {
                    return None;
                }

                let mask = 1u64 << bit;
                if allocator.bitmap[i] & mask == 0 {
                    allocator.bitmap[i] |= mask;
                    allocator.allocated_pages += 1;
                    return Some(page_idx as u64);
                }
            }
        }

        None // No free pages
    }

    /// Free a page frame
    pub fn free(pfn: u64) {
        if pfn as usize >= 4096 {
            return;
        }

        let allocator = unsafe { &mut *ALLOCATOR.get() };
        let i = (pfn as usize) / 64;
        let bit = (pfn as usize) % 64;
        let mask = 1u64 << bit;

        if allocator.bitmap[i] & mask != 0 {
            allocator.bitmap[i] &= !mask;
            allocator.allocated_pages -= 1;
        }
    }

    /// Get total number of pages
    pub fn total_pages() -> usize {
        let allocator = unsafe { &*ALLOCATOR.get() };
        allocator.total_pages
    }

    /// Get number of allocated pages
    pub fn allocated_pages() -> usize {
        let allocator = unsafe { &*ALLOCATOR.get() };
        allocator.allocated_pages
    }

    /// Get number of free pages
    pub fn free_pages() -> usize {
        let allocator = unsafe { &*ALLOCATOR.get() };
        allocator.total_pages - allocator.allocated_pages
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
            if self.processes[i].state == ProcessState::Empty {
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

    /// Get a process by index
    pub fn get_process(&self, index: usize) -> Option<&ProcessControlBlock> {
        if index < MAX_PROCESSES {
            Some(&self.processes[index])
        } else {
            None
        }
    }

    /// Get a mutable reference to a process by index
    pub fn get_process_mut(&mut self, index: usize) -> Option<&mut ProcessControlBlock> {
        if index < MAX_PROCESSES {
            Some(&mut self.processes[index])
        } else {
            None
        }
    }

    /// Get a process by PID
    pub fn get_process_by_pid(&self, pid: usize) -> Option<&ProcessControlBlock> {
        for i in 0..MAX_PROCESSES {
            if self.processes[i].pid == pid && self.processes[i].state != ProcessState::Empty {
                return Some(&self.processes[i]);
            }
        }
        None
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
            // Free all pages
            for i in 0..pcb.address_space.valid_page_count {
                if pcb.address_space.pages[i].valid {
                    PageFrameAllocator::free(pcb.address_space.pages[i].pfn);
                    pcb.address_space.pages[i] = PageEntry::invalid();
                }
            }

            pcb.state = ProcessState::Terminated;
            self.active_processes -= 1;
            true
        } else {
            false
        }
    }

    /// Get active process count
    pub fn active_processes(&self) -> usize {
        self.active_processes
    }

    /// Print process manager status
    pub fn print_status(&self, serial: &mut crate::kernel_lowlevel::serial::Serial) {
        serial.write_str("\n=== Process & Memory Management Status ===\n");
        serial.write_str("Active Processes: ");
        crate::kernel_lowlevel::smp::print_number(serial, self.active_processes as u32);
        serial.write_str("/");
        crate::kernel_lowlevel::smp::print_number(serial, MAX_PROCESSES as u32);
        serial.write_str("\n\n");

        serial.write_str("Physical Memory:\n");
        serial.write_str("  Total Pages: ");
        crate::kernel_lowlevel::smp::print_number(serial, PageFrameAllocator::total_pages() as u32);
        serial.write_str(" (");
        crate::kernel_lowlevel::smp::print_number(
            serial,
            (PageFrameAllocator::total_pages() * PAGE_SIZE / 1024) as u32,
        );
        serial.write_str(" KB)\n");
        serial.write_str("  Allocated: ");
        crate::kernel_lowlevel::smp::print_number(
            serial,
            PageFrameAllocator::allocated_pages() as u32,
        );
        serial.write_str(" pages\n");
        serial.write_str("  Free: ");
        crate::kernel_lowlevel::smp::print_number(serial, PageFrameAllocator::free_pages() as u32);
        serial.write_str(" pages\n\n");

        serial.write_str("Process Table:\n");
        serial.write_str("PID  State      Name         Threads  Segments  Pages\n");

        for i in 0..MAX_PROCESSES {
            let pcb = &self.processes[i];
            if pcb.state != ProcessState::Empty {
                crate::kernel_lowlevel::smp::print_number(serial, pcb.pid as u32);
                serial.write_str("    ");
                serial.write_str(pcb.state.as_str());
                serial.write_str("  ");
                serial.write_str(pcb.name);
                for _ in 0..(12usize.saturating_sub(pcb.name.len())) {
                    serial.write_byte(b' ');
                }
                crate::kernel_lowlevel::smp::print_number(serial, pcb.thread_count as u32);
                serial.write_str("         ");
                crate::kernel_lowlevel::smp::print_number(
                    serial,
                    pcb.address_space.valid_segment_count as u32,
                );
                serial.write_str("        ");
                crate::kernel_lowlevel::smp::print_number(
                    serial,
                    pcb.address_space.valid_page_count as u32,
                );
                serial.write_str("\n");
            }
        }

        serial.write_str("\n");

        // Print detailed memory layout for each process
        for i in 0..MAX_PROCESSES {
            let pcb = &self.processes[i];
            if pcb.state != ProcessState::Empty {
                pcb.address_space.print_info(serial);
                serial.write_str("\n");
            }
        }

        serial.write_str("========================================\n");
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

/// Helper function to print hex number
fn print_hex(serial: &mut crate::kernel_lowlevel::serial::Serial, mut num: u64) {
    if num == 0 {
        serial.write_str("0000");
        return;
    }

    let hex_chars = b"0123456789abcdef";
    let mut buf = [0u8; 16];
    let mut i = 0;

    while num > 0 && i < 16 {
        buf[i] = hex_chars[(num & 0xF) as usize];
        num >>= 4;
        i += 1;
    }

    // Pad with zeros to at least 4 digits
    while i < 4 {
        buf[i] = b'0';
        i += 1;
    }

    // Print in reverse order
    for j in (0..i).rev() {
        serial.write_byte(buf[j]);
    }
}

/// Initialize the memory management subsystem
pub fn init() {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();

    serial.write_str("[MEM] Initializing memory management...\n");

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

/// Demo: Create multiple processes and show memory layout
pub fn demo_processes() {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();

    serial.write_str("\n--- Multi-Process Memory Management Demo ---\n");
    serial.write_str("Creating sample processes...\n");

    let pm = process_manager();

    // Create some sample processes
    if let Some(pid1) = pm.create_process("shell") {
        serial.write_str("[MEM] Created process 'shell' with PID ");
        crate::kernel_lowlevel::smp::print_number(&mut serial, pid1 as u32);
        serial.write_str("\n");
    }

    if let Some(pid2) = pm.create_process("editor") {
        serial.write_str("[MEM] Created process 'editor' with PID ");
        crate::kernel_lowlevel::smp::print_number(&mut serial, pid2 as u32);
        serial.write_str("\n");
    }

    if let Some(pid3) = pm.create_process("compiler") {
        serial.write_str("[MEM] Created process 'compiler' with PID ");
        crate::kernel_lowlevel::smp::print_number(&mut serial, pid3 as u32);
        serial.write_str("\n");
    }

    serial.write_str("\nProcess creation complete. Printing memory layout:\n");

    // Print process and memory status
    pm.print_status(&mut serial);
}

/// Shell command handler
pub struct ShellCommand {
    pub name: &'static str,
    pub description: &'static str,
    pub handler: fn(&mut crate::kernel_lowlevel::serial::Serial, &[&str]),
}

/// Shell context
pub struct Shell {
    pub serial: crate::kernel_lowlevel::serial::Serial,
    pub input_buf: [u8; 256],
    pub command_history: [&'static str; 10],
    pub history_index: usize,
}

impl Shell {
    /// Create a new shell
    pub fn new() -> Self {
        Shell {
            serial: crate::kernel_lowlevel::serial::Serial::new(),
            input_buf: [0; 256],
            command_history: [""; 10],
            history_index: 0,
        }
    }

    /// Initialize the shell
    pub fn init(&mut self) {
        self.serial.init();
    }

    /// Print shell prompt
    pub fn print_prompt(&mut self) {
        self.serial.write_str("\nsmros$ ");
    }

    /// Run the shell main loop
    pub fn run(&mut self) -> ! {
        self.print_welcome();

        loop {
            self.print_prompt();

            // Read a line of input
            let len = self.serial.read_line(&mut self.input_buf);

            if len == 0 {
                continue;
            }

            // Parse the command - create a copy to avoid borrow issues
            let mut cmd_buf = [0u8; 256];
            cmd_buf[..len].copy_from_slice(&self.input_buf[..len]);
            let command_str = core::str::from_utf8(&cmd_buf[..len]).unwrap_or("");
            let args = Self::parse_command_static(command_str);

            if args.is_empty() {
                continue;
            }

            // Execute the command
            self.execute_command(&args);
        }
    }

    /// Print welcome message
    fn print_welcome(&mut self) {
        self.serial.write_str("\n");
        self.serial
            .write_str("╔═══════════════════════════════════════════════════════════╗\n");
        self.serial
            .write_str("║                                                           ║\n");
        self.serial
            .write_str("║          SMROS Shell v0.3.0 - Process Management          ║\n");
        self.serial
            .write_str("║                                                           ║\n");
        self.serial
            .write_str("╚═══════════════════════════════════════════════════════════╝\n");
        self.serial.write_str("\n");
        self.serial
            .write_str("Type 'help' for available commands.\n\n");
    }

    /// Parse command line into arguments (static version to avoid borrow issues)
    fn parse_command_static<'a>(input: &'a str) -> Vec<&'a str> {
        let mut args: Vec<&'a str> = Vec::new();
        for part in input.split_whitespace() {
            if !part.is_empty() {
                args.push(part);
            }
        }
        args
    }

    /// Execute a command
    fn execute_command(&mut self, args: &[&str]) {
        if args.is_empty() {
            return;
        }

        let cmd = args[0];

        // Built-in commands
        match cmd {
            "help" => cmd_help(&mut self.serial, args),
            "ps" => cmd_ps(&mut self.serial, args),
            "top" => cmd_top(&mut self.serial, args),
            "meminfo" => cmd_meminfo(&mut self.serial, args),
            "clear" => cmd_clear(&mut self.serial, args),
            "version" => cmd_version(&mut self.serial, args),
            "echo" => cmd_echo(&mut self.serial, args),
            "uptime" => cmd_uptime(&mut self.serial, args),
            "kill" => cmd_kill(&mut self.serial, args),
            "info" => cmd_info(&mut self.serial, args),
            "heap" => cmd_heap(&mut self.serial, args),
            "pages" => cmd_pages(&mut self.serial, args),
            "tree" => cmd_tree(&mut self.serial, args),
            "whoami" => cmd_whoami(&mut self.serial, args),
            "date" => cmd_date(&mut self.serial, args),
            "cat" => cmd_cat(&mut self.serial, args),
            _ => {
                self.serial.write_str("Unknown command: ");
                self.serial.write_str(cmd);
                self.serial
                    .write_str("\nType 'help' for available commands.\n");
            }
        }
    }
}

/// Command: help - Show available commands
fn cmd_help(serial: &mut crate::kernel_lowlevel::serial::Serial, _args: &[&str]) {
    serial.write_str("\nAvailable Commands:\n");
    serial.write_str("═══════════════════════════════════════════════════════\n");
    serial.write_str("  Command       Description\n");
    serial.write_str("───────────────────────────────────────────────────────\n");
    serial.write_str("  help          Show this help message\n");
    serial.write_str("  ps            List all processes\n");
    serial.write_str("  top           Show process status (like top)\n");
    serial.write_str("  meminfo       Show memory information\n");
    serial.write_str("  pages         Show page allocation details\n");
    serial.write_str("  heap          Show heap usage for processes\n");
    serial.write_str("  tree          Show process tree\n");
    serial.write_str("  kill <pid>    Terminate a process\n");
    serial.write_str("  info [pid]    Show detailed process info\n");
    serial.write_str("  uptime        Show system uptime\n");
    serial.write_str("  version       Show kernel version\n");
    serial.write_str("  whoami        Show current user\n");
    serial.write_str("  date          Show current date/time\n");
    serial.write_str("  echo <text>   Print text\n");
    serial.write_str("  cat <file>    Display file contents (stub)\n");
    serial.write_str("  clear         Clear the screen\n");
    serial.write_str("═══════════════════════════════════════════════════════\n");
}

/// Command: ps - List all processes
fn cmd_ps(serial: &mut crate::kernel_lowlevel::serial::Serial, _args: &[&str]) {
    let pm = process_manager();

    serial.write_str("\n  PID  State      Name         Threads  Parent\n");
    serial.write_str("  ─────────────────────────────────────────────\n");

    let mut count = 0;
    for i in 0..MAX_PROCESSES {
        if let Some(pcb) = pm.get_process(i) {
            if pcb.state != ProcessState::Empty {
                crate::kernel_lowlevel::smp::print_number(serial, pcb.pid as u32);
                serial.write_str("    ");
                serial.write_str(pcb.state.as_str());
                serial.write_str("  ");
                serial.write_str(pcb.name);
                for _ in 0..(12usize.saturating_sub(pcb.name.len())) {
                    serial.write_byte(b' ');
                }
                crate::kernel_lowlevel::smp::print_number(serial, pcb.thread_count as u32);
                serial.write_str("         ");
                crate::kernel_lowlevel::smp::print_number(serial, pcb.parent_pid as u32);
                serial.write_str("\n");
                count += 1;
            }
        }
    }

    serial.write_str("  ─────────────────────────────────────────────\n");
    serial.write_str("  Total: ");
    crate::kernel_lowlevel::smp::print_number(serial, count as u32);
    serial.write_str(" process(es)\n");
}

/// Command: top - Show process status (interactive-like display)
fn cmd_top(serial: &mut crate::kernel_lowlevel::serial::Serial, _args: &[&str]) {
    let pm = process_manager();

    serial.write_str("\n┌─────────────────────────────────────────────────────────────┐\n");
    serial.write_str("│              SMROS Process Monitor (top)                    │\n");
    serial.write_str("├─────────────────────────────────────────────────────────────┤\n");

    // Header
    serial.write_str("│  PID  │ State    │ Name       │ Segments │ Pages  │ Heap     │\n");
    serial.write_str("│───────┼──────────┼────────────┼──────────┼────────┼──────────│\n");

    for i in 0..MAX_PROCESSES {
        if let Some(pcb) = pm.get_process(i) {
            if pcb.state != ProcessState::Empty {
                serial.write_str("│  ");
                print_padded_number(serial, pcb.pid as u32, 3);
                serial.write_str(" │ ");
                serial.write_str(pcb.state.as_str().trim());
                for _ in 0..(8usize.saturating_sub(pcb.state.as_str().trim().len())) {
                    serial.write_byte(b' ');
                }
                serial.write_str("│ ");
                serial.write_str(pcb.name);
                for _ in 0..(10usize.saturating_sub(pcb.name.len())) {
                    serial.write_byte(b' ');
                }
                serial.write_str(" │    ");
                crate::kernel_lowlevel::smp::print_number(
                    serial,
                    pcb.address_space.valid_segment_count as u32,
                );
                serial.write_str("   │   ");
                crate::kernel_lowlevel::smp::print_number(
                    serial,
                    pcb.address_space.valid_page_count as u32,
                );
                serial.write_str("   │  ");

                // Show heap usage
                let heap_used = pcb.address_space.heap_current - 0x2000;
                crate::kernel_lowlevel::smp::print_number(serial, (heap_used / 1024) as u32);
                serial.write_str("KB   │\n");
            }
        }
    }

    serial.write_str("├─────────────────────────────────────────────────────────────┤\n");

    // Memory summary
    serial.write_str("│ Memory: ");
    crate::kernel_lowlevel::smp::print_number(serial, PageFrameAllocator::allocated_pages() as u32);
    serial.write_str(" used / ");
    crate::kernel_lowlevel::smp::print_number(serial, PageFrameAllocator::total_pages() as u32);
    serial.write_str(" total pages           │\n");

    serial.write_str("│ Free: ");
    crate::kernel_lowlevel::smp::print_number(serial, PageFrameAllocator::free_pages() as u32);
    serial.write_str(" pages (");
    crate::kernel_lowlevel::smp::print_number(
        serial,
        (PageFrameAllocator::free_pages() * PAGE_SIZE / 1024) as u32,
    );
    serial.write_str(" KB)                        │\n");

    serial.write_str("└─────────────────────────────────────────────────────────────┘\n");
}

/// Command: meminfo - Show memory information
fn cmd_meminfo(serial: &mut crate::kernel_lowlevel::serial::Serial, _args: &[&str]) {
    let total_pages = PageFrameAllocator::total_pages();
    let used_pages = PageFrameAllocator::allocated_pages();
    let free_pages = PageFrameAllocator::free_pages();
    let total_kb = total_pages * PAGE_SIZE / 1024;
    let used_kb = used_pages * PAGE_SIZE / 1024;
    let free_kb = free_pages * PAGE_SIZE / 1024;
    let usage_pct = if total_pages > 0 {
        (used_pages * 100) / total_pages
    } else {
        0
    };

    serial.write_str("\n┌─────────────────────────────────────────┐\n");
    serial.write_str("│           Memory Information            │\n");
    serial.write_str("├─────────────────────────────────────────┤\n");
    serial.write_str("│  Total Memory:                          │\n");
    serial.write_str("│    Pages: ");
    crate::kernel_lowlevel::smp::print_number(serial, total_pages as u32);
    serial.write_str("                            │\n");
    serial.write_str("│    Size:  ");
    crate::kernel_lowlevel::smp::print_number(serial, total_kb as u32);
    serial.write_str(" KB (");
    crate::kernel_lowlevel::smp::print_number(serial, (total_kb / 1024) as u32);
    serial.write_str(" MB)                   │\n");
    serial.write_str("│                                         │\n");
    serial.write_str("│  Used Memory:                           │\n");
    serial.write_str("│    Pages: ");
    crate::kernel_lowlevel::smp::print_number(serial, used_pages as u32);
    serial.write_str("                            │\n");
    serial.write_str("│    Size:  ");
    crate::kernel_lowlevel::smp::print_number(serial, used_kb as u32);
    serial.write_str(" KB                          │\n");
    serial.write_str("│    Usage: ");
    crate::kernel_lowlevel::smp::print_number(serial, usage_pct as u32);
    serial.write_str("%                             │\n");
    serial.write_str("│                                         │\n");
    serial.write_str("│  Free Memory:                           │\n");
    serial.write_str("│    Pages: ");
    crate::kernel_lowlevel::smp::print_number(serial, free_pages as u32);
    serial.write_str("                            │\n");
    serial.write_str("│    Size:  ");
    crate::kernel_lowlevel::smp::print_number(serial, free_kb as u32);
    serial.write_str(" KB                          │\n");
    serial.write_str("│                                         │\n");
    serial.write_str("│  Page Size: 4 KB (4096 bytes)           │\n");
    serial.write_str("└─────────────────────────────────────────┘\n");
}

/// Command: pages - Show page allocation details
fn cmd_pages(serial: &mut crate::kernel_lowlevel::serial::Serial, _args: &[&str]) {
    let pm = process_manager();

    serial.write_str("\n  Page Allocation Details:\n");
    serial.write_str("  ════════════════════════════════════════\n\n");

    for i in 0..MAX_PROCESSES {
        if let Some(pcb) = pm.get_process(i) {
            if pcb.state != ProcessState::Empty {
                serial.write_str("  Process: ");
                serial.write_str(pcb.name);
                serial.write_str(" (PID ");
                crate::kernel_lowlevel::smp::print_number(serial, pcb.pid as u32);
                serial.write_str(")\n");
                serial.write_str("  ──────────────────────────────────────\n");
                serial.write_str("    Total Pages: ");
                crate::kernel_lowlevel::smp::print_number(
                    serial,
                    pcb.address_space.valid_page_count as u32,
                );
                serial.write_str("\n");
                serial.write_str("    Page Table:\n");

                for j in 0..pcb.address_space.valid_page_count {
                    let page = &pcb.address_space.pages[j];
                    if page.valid {
                        serial.write_str("      Page[");
                        crate::kernel_lowlevel::smp::print_number(serial, j as u32);
                        serial.write_str("] PFN=");
                        print_hex_u64(serial, page.pfn);
                        if page.writable {
                            serial.write_str(" [RW]");
                        } else {
                            serial.write_str(" [RO]");
                        }
                        if page.executable {
                            serial.write_str(" [X]");
                        }
                        serial.write_str("\n");
                    }
                }

                serial.write_str("\n");
            }
        }
    }

    serial.write_str("  Summary:\n");
    serial.write_str("    Total system pages: ");
    crate::kernel_lowlevel::smp::print_number(serial, PageFrameAllocator::total_pages() as u32);
    serial.write_str("\n");
    serial.write_str("    Allocated pages: ");
    crate::kernel_lowlevel::smp::print_number(serial, PageFrameAllocator::allocated_pages() as u32);
    serial.write_str("\n");
    serial.write_str("    Free pages: ");
    crate::kernel_lowlevel::smp::print_number(serial, PageFrameAllocator::free_pages() as u32);
    serial.write_str("\n");
}

/// Command: heap - Show heap usage for processes
fn cmd_heap(serial: &mut crate::kernel_lowlevel::serial::Serial, _args: &[&str]) {
    let pm = process_manager();

    serial.write_str("\n  Process Heap Usage:\n");
    serial.write_str("  ═══════════════════════════════════════════════════\n");
    serial.write_str("  Name         Heap Used    Heap Max     Free\n");
    serial.write_str("  ─────────────────────────────────────────────────\n");

    for i in 0..MAX_PROCESSES {
        if let Some(pcb) = pm.get_process(i) {
            if pcb.state != ProcessState::Empty {
                serial.write_str("  ");
                serial.write_str(pcb.name);
                for _ in 0..(12usize.saturating_sub(pcb.name.len())) {
                    serial.write_byte(b' ');
                }

                let heap_used = pcb.address_space.heap_current - 0x2000;
                let heap_max = pcb.address_space.heap_max - 0x2000;
                let heap_free = heap_max - heap_used;

                crate::kernel_lowlevel::smp::print_number(serial, (heap_used / 1024) as u32);
                serial.write_str(" KB        ");
                crate::kernel_lowlevel::smp::print_number(serial, (heap_max / 1024) as u32);
                serial.write_str(" KB       ");
                crate::kernel_lowlevel::smp::print_number(serial, (heap_free / 1024) as u32);
                serial.write_str(" KB\n");
            }
        }
    }

    serial.write_str("  ─────────────────────────────────────────────────\n");
}

/// Command: tree - Show process tree
fn cmd_tree(serial: &mut crate::kernel_lowlevel::serial::Serial, _args: &[&str]) {
    serial.write_str("\n  Process Tree:\n");
    serial.write_str("  ════════════════════════════════\n\n");

    // Find init process (PID 1, parent 0)
    let pm = process_manager();
    for i in 0..MAX_PROCESSES {
        if let Some(pcb) = pm.get_process(i) {
            if pcb.pid == 1 && pcb.parent_pid == 0 && pcb.state != ProcessState::Empty {
                print_process_tree(serial, pcb, "", true);
            }
        }
    }

    serial.write_str("\n");
}

/// Helper to print process tree recursively
fn print_process_tree(
    serial: &mut crate::kernel_lowlevel::serial::Serial,
    pcb: &ProcessControlBlock,
    _prefix: &str,
    is_last: bool,
) {
    // Print current process
    if is_last {
        serial.write_str("└─ ");
    } else {
        serial.write_str("├─ ");
    }

    serial.write_str("[");
    crate::kernel_lowlevel::smp::print_number(serial, pcb.pid as u32);
    serial.write_str("] ");
    serial.write_str(pcb.name);
    serial.write_str(" (");
    serial.write_str(pcb.state.as_str().trim());
    serial.write_str(")\n");

    // Find children
    let pm = process_manager();
    let mut last_child_idx = 0;

    // First pass: find last child index
    for j in 0..MAX_PROCESSES {
        if let Some(child) = pm.get_process(j) {
            if child.parent_pid == pcb.pid && child.state != ProcessState::Empty {
                last_child_idx = j;
            }
        }
    }

    // Second pass: print children
    for j in 0..MAX_PROCESSES {
        if let Some(child) = pm.get_process(j) {
            if child.parent_pid == pcb.pid && child.state != ProcessState::Empty {
                let is_last_child = j == last_child_idx;
                print_process_tree(serial, child, "", is_last_child);
            }
        }
    }
}

/// Command: kill - Terminate a process
fn cmd_kill(serial: &mut crate::kernel_lowlevel::serial::Serial, args: &[&str]) {
    if args.len() < 2 {
        serial.write_str("Usage: kill <pid>\n");
        return;
    }

    // Parse PID (simple parser for small numbers)
    let pid_str = args[1];
    let mut pid: usize = 0;
    for ch in pid_str.chars() {
        if ch >= '0' && ch <= '9' {
            pid = pid * 10 + (ch as usize - '0' as usize);
        } else {
            serial.write_str("Invalid PID: ");
            serial.write_str(pid_str);
            serial.write_str("\n");
            return;
        }
    }

    if pid == 0 || pid == 1 {
        serial.write_str("Cannot kill init process (PID 1)\n");
        return;
    }

    let pm = process_manager();
    if pm.terminate_process(pid) {
        serial.write_str("Process ");
        crate::kernel_lowlevel::smp::print_number(serial, pid as u32);
        serial.write_str(" terminated.\n");
    } else {
        serial.write_str("Process ");
        crate::kernel_lowlevel::smp::print_number(serial, pid as u32);
        serial.write_str(" not found.\n");
    }
}

/// Command: info - Show detailed process info
fn cmd_info(serial: &mut crate::kernel_lowlevel::serial::Serial, args: &[&str]) {
    let pm = process_manager();

    if args.len() < 2 {
        // Show info for all processes
        for i in 0..MAX_PROCESSES {
            if let Some(pcb) = pm.get_process(i) {
                if pcb.state != ProcessState::Empty {
                    pcb.address_space.print_info(serial);
                    serial.write_str("\n");
                }
            }
        }
    } else {
        // Show info for specific PID
        let pid_str = args[1];
        let mut pid: usize = 0;
        for ch in pid_str.chars() {
            if ch >= '0' && ch <= '9' {
                pid = pid * 10 + (ch as usize - '0' as usize);
            }
        }

        if let Some(pcb) = pm.get_process_by_pid(pid) {
            pcb.address_space.print_info(serial);
        } else {
            serial.write_str("Process ");
            crate::kernel_lowlevel::smp::print_number(serial, pid as u32);
            serial.write_str(" not found.\n");
        }
    }
}

/// Command: uptime - Show system uptime
fn cmd_uptime(serial: &mut crate::kernel_lowlevel::serial::Serial, _args: &[&str]) {
    // In a real kernel, you'd read from a timer
    // For now, show a placeholder
    serial.write_str("System uptime: (timer integration pending)\n");
    serial.write_str("Scheduler tick count: ");
    crate::kernel_lowlevel::smp::print_number(
        serial,
        crate::kernel_objects::scheduler::scheduler().get_tick_count() as u32,
    );
    serial.write_str("\n");
}

/// Command: version - Show kernel version
fn cmd_version(serial: &mut crate::kernel_lowlevel::serial::Serial, _args: &[&str]) {
    serial.write_str("SMROS ARM64 Kernel v0.3.0\n");
    serial.write_str("Features:\n");
    serial.write_str("  - Preemptive Round-Robin Scheduler\n");
    serial.write_str("  - SMP Multi-Core Support (4 CPUs)\n");
    serial.write_str("  - Multi-Process Memory Management\n");
    serial.write_str("  - 4K Page-based Memory Allocation\n");
    serial.write_str("  - Segment-based Memory Management\n");
    serial.write_str("  - Interactive Shell\n");
}

/// Command: whoami - Show current user
fn cmd_whoami(serial: &mut crate::kernel_lowlevel::serial::Serial, _args: &[&str]) {
    serial.write_str("root\n");
}

/// Command: date - Show current date/time (stub)
fn cmd_date(serial: &mut crate::kernel_lowlevel::serial::Serial, _args: &[&str]) {
    serial.write_str("Date/Time: (RTC not yet implemented)\n");
}

/// Command: echo - Print text
fn cmd_echo(serial: &mut crate::kernel_lowlevel::serial::Serial, args: &[&str]) {
    if args.len() < 2 {
        serial.write_str("\n");
        return;
    }

    for (i, arg) in args.iter().enumerate().skip(1) {
        if i > 1 {
            serial.write_str(" ");
        }
        serial.write_str(arg);
    }
    serial.write_str("\n");
}

/// Command: cat - Display file contents (stub)
fn cmd_cat(serial: &mut crate::kernel_lowlevel::serial::Serial, args: &[&str]) {
    if args.len() < 2 {
        serial.write_str("Usage: cat <file>\n");
        return;
    }

    serial.write_str("cat: ");
    serial.write_str(args[1]);
    serial.write_str(": File system not yet implemented\n");
}

/// Command: clear - Clear the screen
fn cmd_clear(serial: &mut crate::kernel_lowlevel::serial::Serial, _args: &[&str]) {
    serial.write_str("\x1B[2J\x1B[H");
}

/// Helper: Print hex number
fn print_hex_u64(serial: &mut crate::kernel_lowlevel::serial::Serial, num: u64) {
    serial.write_str("0x");
    let hex_chars = b"0123456789abcdef";
    if num == 0 {
        serial.write_byte(b'0');
        return;
    }

    let mut buf = [0u8; 16];
    let mut i = 0;
    let mut n = num;

    while n > 0 && i < 16 {
        buf[i] = hex_chars[(n & 0xF) as usize];
        n >>= 4;
        i += 1;
    }

    for j in (0..i).rev() {
        serial.write_byte(buf[j]);
    }
}

/// Helper: Print a padded number with fixed width
fn print_padded_number(
    serial: &mut crate::kernel_lowlevel::serial::Serial,
    num: u32,
    width: usize,
) {
    // Count digits
    let mut temp = num;
    let mut digits = 0;
    if temp == 0 {
        digits = 1;
    } else {
        while temp > 0 {
            temp /= 10;
            digits += 1;
        }
    }

    // Pad with spaces
    for _ in 0..(width.saturating_sub(digits)) {
        serial.write_byte(b' ');
    }

    crate::kernel_lowlevel::smp::print_number(serial, num);
}

/// Start the shell (called from kernel_main)
pub fn start_shell() -> ! {
    let mut shell = Shell::new();
    shell.init();
    shell.run();
}
