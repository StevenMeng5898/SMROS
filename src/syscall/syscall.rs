#![allow(dead_code)]
#![allow(static_mut_refs)]
//! System Call Interface Layer
//!
//! This module provides comprehensive syscall compatibility with both Linux and Zircon APIs,
//! inspired by the grt-zcore project architecture. It bridges SMROS memory management
//! with standard syscall interfaces for process management, virtual memory, and IPC.
//!
//! # Architecture
//!
//! Based on grt-zcore design patterns:
//! - Linux syscalls: mmap, munmap, mprotect, fork, execve, wait4, etc.
//! - Zircon syscalls: VMO, VMAR, handle management, object operations, channels, etc.
//!
//! # Syscall Categories
//!
//! ## Memory Management (VM)
//! - Linux: sys_mmap, sys_munmap, sys_mprotect, sys_mremap
//! - Zircon: sys_vmo_create, sys_vmo_read, sys_vmo_write, sys_vmar_map, sys_vmar_unmap
//!
//! ## Process/Task Management
//! - Linux: sys_fork, sys_execve, sys_wait4, sys_exit, sys_getpid, sys_kill
//! - Zircon: sys_process_create, sys_thread_create, sys_task_kill, sys_process_exit
//!
//! ## Handle & Object Management (Zircon-style)
//! - Handle operations: create, close, duplicate, replace
//! - Object operations: wait, signal, get_info, get_property
//!
//! ## IPC & Communication
//! - Channels: create, read, write
//! - Sockets: create, read, write, shutdown
//! - FIFOs: create, read, write
//! - Futex: wait, wake, requeue
//!
//! ## Time & Clock
//! - Clock: get, create, read
//! - Timer: create, set, cancel
//! - Sleep: nanosleep, clock_nanosleep

use core::convert::TryFrom;
use alloc::vec::Vec;

use crate::kernel_lowlevel::memory::{
    PAGE_SIZE, PageFrameAllocator, process_manager,
};
use crate::kernel_objects::vmar::Vmar;

// Re-export kernel objects for convenience
pub use crate::kernel_objects::{
    HandleValue, VmOptions, MmuFlags, VmarFlags, VmoOpType, VmoType,
    ZxError, ZxResult, Vmo,
    pages, roundup_pages,
    INVALID_HANDLE,
};

// Simple logging macros (placeholder for real logging)
macro_rules! info {
    ($($arg:tt)*) => {
        // In a real kernel, would write to debug log
        let _ = format_args!($($arg)*);
    };
}

macro_rules! warn {
    ($($arg:tt)*) => {
        // In a real kernel, would write to warning log
        let _ = format_args!($($arg)*);
    };
}

// ============================================================================
// Constants and Types
// ============================================================================

// These are now in kernel_objects.rs and re-exported above

/// Linux syscall error codes
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysError {
    EPERM = 1,
    ENOENT = 2,
    ESRCH = 3,
    EINTR = 4,
    EIO = 5,
    ENXIO = 6,
    E2BIG = 7,
    ENOMEM = 12,
    EACCES = 13,
    EFAULT = 14,
    EBUSY = 16,
    EEXIST = 17,
    ENODEV = 19,
    EINVAL = 22,
    ENOSYS = 38,
}

/// Syscall result type
pub type SysResult = Result<usize, SysError>;

impl From<SysError> for usize {
    fn from(err: SysError) -> Self {
        -(err as i32) as usize
    }
}

const LINUX_MAPPING_BASE: usize = 0x5000_0000;
const LINUX_MAPPING_LIMIT: usize = 0x6000_0000;
const BRK_HEAP_START: usize = 0x4000_0000;
const BRK_HEAP_LIMIT: usize = BRK_HEAP_START + (1024 * 1024);
const ZIRCON_ROOT_VMAR_BASE: usize = 0x7000_0000;
const ZIRCON_ROOT_VMAR_SIZE: usize = 0x1000_0000;
const MEMORY_HANDLE_START: u32 = 0x1000;
const ARM64_SYS_WRITE: u32 = 64;
const ARM64_SYS_EXIT: u32 = 93;
const ARM64_SYS_GETPID: u32 = 172;
const ARM64_SYS_MMAP: u32 = 222;

#[derive(Clone)]
struct LinuxMappingRecord {
    addr: usize,
    len: usize,
    prot: usize,
    flags: usize,
    pfns: Vec<u64>,
}

#[derive(Default)]
struct BrkState {
    start: usize,
    current: usize,
    limit: usize,
    pfns: Vec<u64>,
}

impl BrkState {
    fn new() -> Self {
        Self {
            start: BRK_HEAP_START,
            current: BRK_HEAP_START,
            limit: BRK_HEAP_LIMIT,
            pfns: Vec::new(),
        }
    }

    fn committed_pages(&self) -> usize {
        self.pfns.len()
    }
}

struct VmoRecord {
    handle: u32,
    vmo: Vmo,
}

struct VmarRecord {
    handle: u32,
    vmar: Vmar,
}

#[derive(Clone, Copy, Default)]
pub struct MemorySyscallStats {
    pub linux_mapping_count: usize,
    pub linux_mapped_bytes: usize,
    pub linux_committed_pages: usize,
    pub brk_start: usize,
    pub brk_current: usize,
    pub brk_limit: usize,
    pub brk_committed_pages: usize,
    pub zircon_vmo_count: usize,
    pub zircon_vmo_bytes: usize,
    pub zircon_vmo_committed_pages: usize,
    pub zircon_vmar_count: usize,
    pub zircon_mapping_count: usize,
    pub zircon_root_vmar_handle: u32,
}

struct MemorySyscallState {
    linux_mappings: Vec<LinuxMappingRecord>,
    next_linux_addr: usize,
    brk: BrkState,
    vmos: Vec<VmoRecord>,
    vmars: Vec<VmarRecord>,
    next_handle: u32,
    root_vmar_handle: u32,
}

impl MemorySyscallState {
    fn new() -> Self {
        let root_vmar_handle = MEMORY_HANDLE_START;
        let mut root_vmar = Vmar::new(ZIRCON_ROOT_VMAR_BASE, ZIRCON_ROOT_VMAR_SIZE);
        root_vmar.handle = HandleValue(root_vmar_handle);
        let mut vmars = Vec::new();
        vmars.push(VmarRecord {
            handle: root_vmar_handle,
            vmar: root_vmar,
        });

        Self {
            linux_mappings: Vec::new(),
            next_linux_addr: LINUX_MAPPING_BASE,
            brk: BrkState::new(),
            vmos: Vec::new(),
            vmars,
            next_handle: MEMORY_HANDLE_START + 1,
            root_vmar_handle,
        }
    }

    fn alloc_handle(&mut self) -> u32 {
        let handle = self.next_handle;
        self.next_handle = self.next_handle.wrapping_add(1);
        handle
    }

    fn stats(&self) -> MemorySyscallStats {
        MemorySyscallStats {
            linux_mapping_count: self.linux_mappings.len(),
            linux_mapped_bytes: self.linux_mappings.iter().map(|mapping| mapping.len).sum(),
            linux_committed_pages: self.linux_mappings.iter().map(|mapping| mapping.pfns.len()).sum(),
            brk_start: self.brk.start,
            brk_current: self.brk.current,
            brk_limit: self.brk.limit,
            brk_committed_pages: self.brk.committed_pages(),
            zircon_vmo_count: self.vmos.len(),
            zircon_vmo_bytes: self.vmos.iter().map(|record| record.vmo.len()).sum(),
            zircon_vmo_committed_pages: self.vmos.iter().map(|record| record.vmo.committed_pages()).sum(),
            zircon_vmar_count: self.vmars.len(),
            zircon_mapping_count: self.vmars.iter().map(|record| record.vmar.mappings.len()).sum(),
            zircon_root_vmar_handle: self.root_vmar_handle,
        }
    }

    fn free_linux_pages(pfns: &[u64]) {
        for pfn in pfns {
            PageFrameAllocator::free(*pfn);
        }
    }

    fn alloc_linux_pages(page_count: usize) -> Option<Vec<u64>> {
        let mut pfns = Vec::with_capacity(page_count);

        for _ in 0..page_count {
            if let Some(pfn) = PageFrameAllocator::alloc() {
                pfns.push(pfn);
            } else {
                Self::free_linux_pages(&pfns);
                return None;
            }
        }

        Some(pfns)
    }

    fn sort_linux_mappings(&mut self) {
        self.linux_mappings.sort_by_key(|mapping| mapping.addr);
    }

    fn linux_range_available(&self, addr: usize, len: usize) -> bool {
        range_within_window(addr, len, LINUX_MAPPING_BASE, LINUX_MAPPING_LIMIT)
            && !self.linux_mappings.iter().any(|mapping| {
                range_overlaps(addr, len, mapping.addr, mapping.len)
            })
    }

    fn find_free_linux_region(&mut self, hint: Option<usize>, len: usize) -> Option<usize> {
        if let Some(addr) = hint {
            if self.linux_range_available(addr, len) {
                return Some(addr);
            }
        }

        self.sort_linux_mappings();
        let mut candidate = self.next_linux_addr.max(LINUX_MAPPING_BASE);

        for mapping in &self.linux_mappings {
            if candidate + len <= mapping.addr {
                self.next_linux_addr = candidate + len;
                return Some(candidate);
            }

            candidate = candidate.max(mapping.addr + mapping.len);
        }

        if candidate + len <= LINUX_MAPPING_LIMIT {
            self.next_linux_addr = candidate + len;
            return Some(candidate);
        }

        None
    }

    fn get_vmo(&self, handle: u32) -> Option<&Vmo> {
        self.vmos.iter().find(|record| record.handle == handle).map(|record| &record.vmo)
    }

    fn get_vmo_mut(&mut self, handle: u32) -> Option<&mut Vmo> {
        self.vmos.iter_mut().find(|record| record.handle == handle).map(|record| &mut record.vmo)
    }

    fn get_vmar(&self, handle: u32) -> Option<&Vmar> {
        self.vmars.iter().find(|record| record.handle == handle).map(|record| &record.vmar)
    }

    fn get_vmar_mut(&mut self, handle: u32) -> Option<&mut Vmar> {
        self.vmars.iter_mut().find(|record| record.handle == handle).map(|record| &mut record.vmar)
    }

    fn remove_vmo(&mut self, handle: u32) -> bool {
        if let Some(index) = self.vmos.iter().position(|record| record.handle == handle) {
            let mut record = self.vmos.swap_remove(index);
            record.vmo.release_pages();
            true
        } else {
            false
        }
    }

    fn destroy_vmar_recursive(&mut self, handle: u32) -> bool {
        let Some(index) = self.vmars.iter().position(|record| record.handle == handle) else {
            return false;
        };

        let child_handles = self.vmars[index].vmar.children.clone();
        for child_handle in child_handles {
            self.destroy_vmar_recursive(child_handle as u32);
        }

        if handle == self.root_vmar_handle {
            self.vmars[index].vmar.destroy().ok();
            return true;
        }

        let parent_handle = self.vmars[index].vmar.parent_idx.map(|parent| parent as u32);
        let mut record = self.vmars.swap_remove(index);
        record.vmar.destroy().ok();

        if let Some(parent_handle) = parent_handle {
            if let Some(parent) = self.get_vmar_mut(parent_handle) {
                parent.children.retain(|child| *child != handle as usize);
            }
        }

        true
    }

    fn remove_vmar(&mut self, handle: u32) -> bool {
        self.destroy_vmar_recursive(handle)
    }

    fn release_handle(&mut self, handle: u32) -> bool {
        self.remove_vmo(handle) || self.remove_vmar(handle)
    }
}

static mut MEMORY_SYSCALL_STATE: Option<MemorySyscallState> = None;

fn memory_state() -> &'static mut MemorySyscallState {
    unsafe {
        if MEMORY_SYSCALL_STATE.is_none() {
            MEMORY_SYSCALL_STATE = Some(MemorySyscallState::new());
        }

        MEMORY_SYSCALL_STATE.as_mut().unwrap()
    }
}

fn checked_end(addr: usize, len: usize) -> Option<usize> {
    addr.checked_add(len)
}

fn range_overlaps(start_a: usize, len_a: usize, start_b: usize, len_b: usize) -> bool {
    let end_a = start_a + len_a;
    let end_b = start_b + len_b;
    start_a < end_b && start_b < end_a
}

fn range_within_window(addr: usize, len: usize, base: usize, limit: usize) -> bool {
    checked_end(addr, len).map(|end| addr >= base && end <= limit).unwrap_or(false)
}

fn mmu_flags_from_vm_options(options: VmOptions) -> MmuFlags {
    let mut flags = MmuFlags::USER;

    if options.contains(VmOptions::PERM_READ) {
        flags |= MmuFlags::READ;
    }
    if options.contains(VmOptions::PERM_WRITE) {
        flags |= MmuFlags::WRITE;
    }
    if options.contains(VmOptions::PERM_EXECUTE) {
        flags |= MmuFlags::EXECUTE;
    }
    if flags == MmuFlags::USER {
        flags |= MmuFlags::READ;
    }

    flags
}

fn split_linux_mapping(mapping: LinuxMappingRecord, start: usize, len: usize) -> Vec<LinuxMappingRecord> {
    let mut pieces = Vec::new();
    let end = start + len;
    let mapping_end = mapping.addr + mapping.len;

    if start > mapping.addr {
        let left_pages = (start - mapping.addr) / PAGE_SIZE;
        let left_len = start - mapping.addr;
        pieces.push(LinuxMappingRecord {
            addr: mapping.addr,
            len: left_len,
            prot: mapping.prot,
            flags: mapping.flags,
            pfns: mapping.pfns[..left_pages].to_vec(),
        });
    }

    if end < mapping_end {
        let right_start_page = (end - mapping.addr) / PAGE_SIZE;
        pieces.push(LinuxMappingRecord {
            addr: end,
            len: mapping_end - end,
            prot: mapping.prot,
            flags: mapping.flags,
            pfns: mapping.pfns[right_start_page..].to_vec(),
        });
    }

    pieces
}

pub fn memory_syscall_stats() -> MemorySyscallStats {
    memory_state().stats()
}

pub fn memory_root_vmar_handle() -> u32 {
    memory_state().root_vmar_handle
}

// ============================================================================
// Linux-compatible Memory Syscalls
// ============================================================================

// Linux mmap protection flags
bitflags::bitflags! {
    pub struct MmapProt: usize {
        const READ = 1 << 0;
        const WRITE = 1 << 1;
        const EXEC = 1 << 2;
    }
}

impl MmapProt {
    pub fn to_flags(&self) -> MmuFlags {
        let mut flags = MmuFlags::USER;
        if self.contains(MmapProt::READ) {
            flags |= MmuFlags::READ;
        }
        if self.contains(MmapProt::WRITE) {
            flags |= MmuFlags::WRITE;
        }
        if self.contains(MmapProt::EXEC) {
            flags |= MmuFlags::EXECUTE;
        }
        if self.is_empty() {
            flags |= MmuFlags::READ | MmuFlags::WRITE;
        }
        flags
    }
}

// Linux mmap flags
bitflags::bitflags! {
    pub struct MmapFlags: usize {
        const SHARED = 1 << 0;
        const PRIVATE = 1 << 1;
        const FIXED = 1 << 4;
        const ANONYMOUS = 1 << 5;
    }
}

/// Helper: check if address is page-aligned
pub fn page_aligned(addr: usize) -> bool {
    addr & (PAGE_SIZE - 1) == 0
}

fn update_linux_protection(
    state: &mut MemorySyscallState,
    addr: usize,
    len: usize,
    prot_bits: usize,
) -> SysResult {
    let end = addr + len;
    let mut touched = false;
    let mappings = core::mem::take(&mut state.linux_mappings);

    for mapping in mappings {
        if !range_overlaps(addr, len, mapping.addr, mapping.len) {
            state.linux_mappings.push(mapping);
            continue;
        }

        touched = true;
        let overlap_start = core::cmp::max(addr, mapping.addr);
        let overlap_end = core::cmp::min(end, mapping.addr + mapping.len);
        let start_page = (overlap_start - mapping.addr) / PAGE_SIZE;
        let end_page = (overlap_end - mapping.addr) / PAGE_SIZE;

        for piece in split_linux_mapping(mapping.clone(), overlap_start, overlap_end - overlap_start) {
            state.linux_mappings.push(piece);
        }

        state.linux_mappings.push(LinuxMappingRecord {
            addr: overlap_start,
            len: overlap_end - overlap_start,
            prot: prot_bits,
            flags: mapping.flags,
            pfns: mapping.pfns[start_page..end_page].to_vec(),
        });
    }

    state.sort_linux_mappings();
    if touched {
        Ok(0)
    } else {
        Err(SysError::EINVAL)
    }
}

fn unmap_linux_range(state: &mut MemorySyscallState, addr: usize, len: usize) -> SysResult {
    let end = addr + len;
    let mut removed = false;
    let mappings = core::mem::take(&mut state.linux_mappings);

    for mapping in mappings {
        if !range_overlaps(addr, len, mapping.addr, mapping.len) {
            state.linux_mappings.push(mapping);
            continue;
        }

        removed = true;
        let overlap_start = core::cmp::max(addr, mapping.addr);
        let overlap_end = core::cmp::min(end, mapping.addr + mapping.len);
        let start_page = (overlap_start - mapping.addr) / PAGE_SIZE;
        let end_page = (overlap_end - mapping.addr) / PAGE_SIZE;
        MemorySyscallState::free_linux_pages(&mapping.pfns[start_page..end_page]);

        for piece in split_linux_mapping(mapping, overlap_start, overlap_end - overlap_start) {
            state.linux_mappings.push(piece);
        }
    }

    state.sort_linux_mappings();
    if removed {
        Ok(0)
    } else {
        Err(SysError::EINVAL)
    }
}

/// Linux sys_mmap implementation
pub fn sys_mmap(
    addr: usize,
    len: usize,
    prot: usize,
    flags: usize,
    fd: usize,
    offset: u64,
) -> SysResult {
    let prot = MmapProt::from_bits_truncate(prot);
    let flags = MmapFlags::from_bits_truncate(flags);

    info!(
        "mmap: addr={:#x}, size={:#x}, prot={:?}, flags={:?}",
        addr, len, prot, flags
    );

    if len == 0 {
        return Err(SysError::EINVAL);
    }
    if !flags.contains(MmapFlags::ANONYMOUS) {
        return Err(SysError::ENOSYS);
    }
    if flags.contains(MmapFlags::SHARED) && flags.contains(MmapFlags::PRIVATE) {
        return Err(SysError::EINVAL);
    }
    if !flags.contains(MmapFlags::SHARED) && !flags.contains(MmapFlags::PRIVATE) {
        return Err(SysError::EINVAL);
    }
    if offset != 0 || fd != 0 {
        return Err(SysError::EINVAL);
    }

    let len = roundup_pages(len);
    let state = memory_state();
    let requested = if flags.contains(MmapFlags::FIXED) {
        if !page_aligned(addr) {
            return Err(SysError::EINVAL);
        }
        if !range_within_window(addr, len, LINUX_MAPPING_BASE, LINUX_MAPPING_LIMIT) {
            return Err(SysError::EINVAL);
        }
        let _ = unmap_linux_range(state, addr, len);
        Some(addr)
    } else if addr != 0 && page_aligned(addr) {
        Some(addr)
    } else {
        None
    };

    let vaddr = state.find_free_linux_region(requested, len).ok_or(SysError::ENOMEM)?;
    let pfns = MemorySyscallState::alloc_linux_pages(pages(len)).ok_or(SysError::ENOMEM)?;

    state.linux_mappings.push(LinuxMappingRecord {
        addr: vaddr,
        len,
        prot: prot.bits(),
        flags: flags.bits(),
        pfns,
    });
    state.sort_linux_mappings();
    Ok(vaddr)
}

/// Linux sys_mprotect implementation
pub fn sys_mprotect(addr: usize, len: usize, prot: usize) -> SysResult {
    let prot = MmapProt::from_bits_truncate(prot);

    info!(
        "mprotect: addr={:#x}, size={:#x}, prot={:?}",
        addr, len, prot
    );

    if !page_aligned(addr) || len == 0 {
        return Err(SysError::EINVAL);
    }

    update_linux_protection(memory_state(), addr, roundup_pages(len), prot.bits())
}

/// Linux sys_munmap implementation
pub fn sys_munmap(addr: usize, len: usize) -> SysResult {
    info!("munmap: addr={:#x}, size={:#x}", addr, len);

    if !page_aligned(addr) || len == 0 {
        return Err(SysError::EINVAL);
    }

    unmap_linux_range(memory_state(), addr, roundup_pages(len))
}

/// Linux sys_brk implementation
///
/// The brk syscall is used to change the program break (heap end).
/// It's the traditional way to implement heap allocation in Linux.
///
/// # Arguments
/// * `new_brk` - The new program break address
///
/// # Returns
/// * On success: The current program break address
/// * On error: Negative error code
pub fn sys_brk(new_brk: usize) -> SysResult {
    info!("brk: new_brk={:#x}", new_brk);

    let state = memory_state();

    if new_brk == 0 {
        return Ok(state.brk.current);
    }
    if new_brk < state.brk.start || new_brk > state.brk.limit {
        return Ok(state.brk.current);
    }

    let old_pages = pages(state.brk.current.saturating_sub(state.brk.start));
    let new_pages = pages(new_brk.saturating_sub(state.brk.start));

    if new_pages > old_pages {
        let mut newly_allocated = Vec::with_capacity(new_pages - old_pages);
        for _ in old_pages..new_pages {
            if let Some(pfn) = PageFrameAllocator::alloc() {
                newly_allocated.push(pfn);
            } else {
                MemorySyscallState::free_linux_pages(&newly_allocated);
                return Ok(state.brk.current);
            }
        }
        state.brk.pfns.extend(newly_allocated);
    } else if new_pages < old_pages {
        for _ in new_pages..old_pages {
            if let Some(pfn) = state.brk.pfns.pop() {
                PageFrameAllocator::free(pfn);
            }
        }
    }

    state.brk.current = new_brk;
    Ok(state.brk.current)
}

/// Linux sys_mremap implementation
///
/// The mremap syscall is used to resize existing memory mappings.
///
/// # Arguments
/// * `old_address` - Current mapping address
/// * `old_size` - Current mapping size
/// * `new_size` - New desired size
/// * `flags` - Mremap flags (MREMAP_MAYMOVE, MREMAP_FIXED)
/// * `new_address` - New address if MREMAP_FIXED is set
///
/// # Returns
/// * On success: New mapping address
/// * On error: Negative error code
pub fn sys_mremap(
    old_address: usize,
    old_size: usize,
    new_size: usize,
    flags: usize,
    new_address: usize,
) -> SysResult {
    info!(
        "mremap: old_addr={:#x}, old_size={:#x}, new_size={:#x}, flags={:#x}",
        old_address, old_size, new_size, flags
    );
    
    const MREMAP_MAYMOVE: usize = 1 << 0;
    const MREMAP_FIXED: usize = 1 << 1;
    const MREMAP_DONTUNMAP: usize = 1 << 2;

    if old_address == 0 || new_size == 0 {
        return Err(SysError::EINVAL);
    }
    if !page_aligned(old_address) || old_size == 0 {
        return Err(SysError::EINVAL);
    }

    if flags & MREMAP_FIXED != 0 && flags & MREMAP_MAYMOVE == 0 {
        return Err(SysError::EINVAL);
    }
    if flags & MREMAP_DONTUNMAP != 0 && flags & MREMAP_MAYMOVE == 0 {
        return Err(SysError::EINVAL);
    }

    let old_len = roundup_pages(old_size);
    let new_len = roundup_pages(new_size);
    let state = memory_state();
    let Some(index) = state
        .linux_mappings
        .iter()
        .position(|mapping| mapping.addr == old_address && mapping.len == old_len) else {
        return Err(SysError::EINVAL);
    };

    if new_len == old_len {
        return Ok(old_address);
    }

    if new_len < old_len {
        let mapping = state.linux_mappings.remove(index);
        let keep_pages = new_len / PAGE_SIZE;
        let mut keep_pfns = mapping.pfns;
        let tail_pfns = keep_pfns.split_off(keep_pages);
        MemorySyscallState::free_linux_pages(&tail_pfns);
        state.linux_mappings.push(LinuxMappingRecord {
            addr: old_address,
            len: new_len,
            prot: mapping.prot,
            flags: mapping.flags,
            pfns: keep_pfns,
        });
        state.sort_linux_mappings();
        return Ok(old_address);
    }

    let extra_len = new_len - old_len;
    if flags & MREMAP_FIXED == 0 && state.linux_range_available(old_address + old_len, extra_len) {
        let extra_pfns = MemorySyscallState::alloc_linux_pages(extra_len / PAGE_SIZE).ok_or(SysError::ENOMEM)?;
        state.linux_mappings[index].len = new_len;
        state.linux_mappings[index].pfns.extend(extra_pfns);
        state.sort_linux_mappings();
        return Ok(old_address);
    }

    if flags & MREMAP_MAYMOVE == 0 {
        return Err(SysError::ENOMEM);
    }

    let requested_addr = if flags & MREMAP_FIXED != 0 {
        if new_address == 0 || !page_aligned(new_address) {
            return Err(SysError::EINVAL);
        }
        let _ = unmap_linux_range(state, new_address, new_len);
        Some(new_address)
    } else {
        None
    };

    let new_addr = state.find_free_linux_region(requested_addr, new_len).ok_or(SysError::ENOMEM)?;
    let new_pfns = MemorySyscallState::alloc_linux_pages(new_len / PAGE_SIZE).ok_or(SysError::ENOMEM)?;
    let prot = state.linux_mappings[index].prot;
    let flags_bits = state.linux_mappings[index].flags;

    if flags & MREMAP_DONTUNMAP == 0 {
        let old_mapping = state.linux_mappings.swap_remove(index);
        MemorySyscallState::free_linux_pages(&old_mapping.pfns);
    }

    state.linux_mappings.push(LinuxMappingRecord {
        addr: new_addr,
        len: new_len,
        prot,
        flags: flags_bits,
        pfns: new_pfns,
    });
    state.sort_linux_mappings();
    Ok(new_addr)
}

/// Linux sys_write implementation
pub fn sys_write(fd: usize, buf_ptr: usize, len: usize) -> SysResult {
    info!("write: fd={}, buf={:#x}, len={:#x}", fd, buf_ptr, len);

    if len == 0 {
        return Ok(0);
    }
    if buf_ptr == 0 {
        return Err(SysError::EFAULT);
    }

    match fd {
        1 | 2 => {
            let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len) };
            let mut serial = crate::kernel_lowlevel::serial::Serial::new();
            serial.init();
            for byte in buf {
                serial.write_byte(*byte);
            }
            Ok(len)
        }
        _ => Err(SysError::ENODEV),
    }
}

// ============================================================================
// Zircon VMO Syscalls
// ============================================================================

/// Zircon sys_vmo_create implementation
pub fn sys_vmo_create(
    size: u64,
    options: u32,
    out_handle: &mut u32,
) -> ZxResult {
    info!("vmo.create: size={:#x?}, options={:#x?}", size, options);

    // Options flags:
    // bit 0: resizable
    // bit 1: physical (if set, creates physical VMO)
    // bit 2: contiguous (if set, creates contiguous VMO)
    
    let resizable = options & 1 != 0;
    let is_physical = options & 2 != 0;
    let is_contiguous = options & 4 != 0;

    if is_physical && is_contiguous {
        return Err(ZxError::ErrInvalidArgs);
    }

    if resizable && (is_physical || is_contiguous) {
        return Err(ZxError::ErrInvalidArgs);
    }

    let page_count = pages(size as usize);
    let mut vmo = if is_physical {
        let mut vmo = Vmo::new_contiguous(size as usize).ok_or(ZxError::ErrNoMemory)?;
        vmo.vmo_type = VmoType::Physical;
        vmo
    } else if is_contiguous {
        Vmo::new_contiguous(size as usize).ok_or(ZxError::ErrNoMemory)?
    } else {
        Vmo::new_paged_with_resizable(resizable, page_count).ok_or(ZxError::ErrNoMemory)?
    };

    let state = memory_state();
    let handle = state.alloc_handle();
    vmo.handle = HandleValue(handle);
    state.vmos.push(VmoRecord { handle, vmo });
    *out_handle = handle;
    Ok(())
}

/// Zircon sys_vmo_read implementation
pub fn sys_vmo_read(
    handle: u32,
    buf: &mut [u8],
    offset: u64,
) -> ZxResult<usize> {
    info!("vmo.read: handle={:#x?}, offset={:#x?}", handle, offset);

    let state = memory_state();
    let vmo = state.get_vmo(handle).ok_or(ZxError::ErrNotFound)?;
    vmo.read(offset as usize, buf)?;
    Ok(buf.len())
}

/// Zircon sys_vmo_write implementation
pub fn sys_vmo_write(
    handle: u32,
    buf: &[u8],
    offset: u64,
) -> ZxResult<usize> {
    info!("vmo.write: handle={:#x?}, offset={:#x?}", handle, offset);

    let state = memory_state();
    let vmo = state.get_vmo_mut(handle).ok_or(ZxError::ErrNotFound)?;
    vmo.write(offset as usize, buf)?;
    Ok(buf.len())
}

/// Zircon sys_vmo_get_size implementation
pub fn sys_vmo_get_size(handle: u32, out_size: &mut usize) -> ZxResult {
    info!("vmo.get_size: handle={:?}", handle);

    let state = memory_state();
    let vmo = state.get_vmo(handle).ok_or(ZxError::ErrNotFound)?;
    *out_size = vmo.len();
    Ok(())
}

/// Zircon sys_vmo_set_size implementation
pub fn sys_vmo_set_size(handle: u32, size: usize) -> ZxResult {
    info!("vmo.set_size: handle={:#x}, size={:#x}", handle, size);

    let state = memory_state();
    let vmo = state.get_vmo_mut(handle).ok_or(ZxError::ErrNotFound)?;
    vmo.set_len(size)
}

/// Zircon sys_vmo_op_range implementation
pub fn sys_vmo_op_range(
    handle: u32,
    op: u32,
    offset: usize,
    len: usize,
) -> ZxResult<usize> {
    info!("vmo.op_range: handle={:#x}, op={:#X}, offset={:#x}, len={:#x}",
          handle, op, offset, len);

    let op = VmoOpType::try_from(op).or(Err(ZxError::ErrInvalidArgs))?;
    let state = memory_state();
    let vmo = state.get_vmo_mut(handle).ok_or(ZxError::ErrNotFound)?;

    if checked_end(offset, len).filter(|end| *end <= vmo.len()).is_none() {
        return Err(ZxError::ErrOutOfRange);
    }

    match op {
        VmoOpType::Commit => {
            if !page_aligned(offset) || !page_aligned(len) {
                return Err(ZxError::ErrInvalidArgs);
            }
            vmo.commit(offset, len)?;
            Ok(0)
        }
        VmoOpType::Decommit => {
            if !page_aligned(offset) || !page_aligned(len) {
                return Err(ZxError::ErrInvalidArgs);
            }
            vmo.decommit(offset, len)?;
            Ok(0)
        }
        VmoOpType::Zero => {
            vmo.zero(offset, len)?;
            Ok(0)
        }
        VmoOpType::Lock
        | VmoOpType::Unlock
        | VmoOpType::CacheSync
        | VmoOpType::CacheInvalidate
        | VmoOpType::CacheClean
        | VmoOpType::CacheCleanInvalidate => Ok(0),
    }
}

// ============================================================================
// Zircon VMAR Syscalls
// ============================================================================

/// Zircon sys_vmar_map implementation
#[allow(clippy::too_many_arguments)]
pub fn sys_vmar_map(
    vmar_handle: u32,
    options: u32,
    vmar_offset: usize,
    vmo_handle: u32,
    vmo_offset: usize,
    len: usize,
    out_addr: &mut usize,
) -> ZxResult {
    info!(
        "vmar.map: vmar={:#x}, offset={:#x}, vmo={:#x}, len={:#x}",
        vmar_handle, vmar_offset, vmo_handle, len
    );

    let options = VmOptions::from_bits(options).ok_or(ZxError::ErrInvalidArgs)?;

    let len = roundup_pages(len);
    if len == 0 || !page_aligned(vmo_offset) {
        return Err(ZxError::ErrInvalidArgs);
    }

    let vmo_len = {
        let state = memory_state();
        let vmo = state.get_vmo(vmo_handle).ok_or(ZxError::ErrNotFound)?;
        vmo.len()
    };

    if checked_end(vmo_offset, len).filter(|end| *end <= vmo_len).is_none() {
        return Err(ZxError::ErrOutOfRange);
    }

    let overwrite = options.contains(VmOptions::SPECIFIC_OVERWRITE);
    let specific = options.contains(VmOptions::SPECIFIC) || overwrite;
    let requested_offset = if specific { Some(vmar_offset) } else { None };
    let flags = mmu_flags_from_vm_options(options);
    let state = memory_state();
    let vmar = state.get_vmar_mut(vmar_handle).ok_or(ZxError::ErrNotFound)?;
    *out_addr = vmar.map_ext(
        requested_offset,
        HandleValue(vmo_handle),
        vmo_offset,
        len,
        flags,
        flags,
        overwrite,
        options.contains(VmOptions::MAP_RANGE),
    )?;
    Ok(())
}

/// Zircon sys_vmar_unmap implementation
pub fn sys_vmar_unmap(vmar_handle: u32, addr: usize, len: usize) -> ZxResult {
    info!("vmar.unmap: vmar={:#x}, addr={:#x}, len={:#x}",
          vmar_handle, addr, len);

    if !page_aligned(addr) || len == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }

    let state = memory_state();
    let vmar = state.get_vmar_mut(vmar_handle).ok_or(ZxError::ErrNotFound)?;
    vmar.unmap(addr, roundup_pages(len))
}

/// Zircon sys_vmar_protect implementation
pub fn sys_vmar_protect(
    vmar_handle: u32,
    options: u32,
    addr: u64,
    len: u64,
) -> ZxResult {
    let raw_options = options;
    let options = VmOptions::from_bits(options).ok_or(ZxError::ErrInvalidArgs)?;

    info!("vmar.protect: vmar={:#x}, options={:#x}, addr={:#x}, len={:#x}",
          vmar_handle, raw_options, addr, len);

    if !page_aligned(addr as usize) || len == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }

    let state = memory_state();
    let vmar = state.get_vmar_mut(vmar_handle).ok_or(ZxError::ErrNotFound)?;
    vmar.protect(addr as usize, roundup_pages(len as usize), mmu_flags_from_vm_options(options))
}

/// Zircon sys_vmar_allocate implementation
pub fn sys_vmar_allocate(
    parent_vmar: u32,
    options: u32,
    offset: u64,
    size: u64,
    out_child_vmar: &mut u32,
    out_child_addr: &mut usize,
) -> ZxResult {
    let flags = VmarFlags::from_bits(options).ok_or(ZxError::ErrInvalidArgs)?;

    info!(
        "vmar.allocate: parent={:#x?}, options={:#x?}, offset={:#x?}, size={:#x?}",
        parent_vmar, options, offset, size,
    );

    let size = roundup_pages(size as usize);
    if size == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }

    let child_handle = {
        let state = memory_state();
        state.alloc_handle()
    };

    let requested_offset = if flags.contains(VmarFlags::SPECIFIC) || offset != 0 {
        Some(offset as usize)
    } else {
        None
    };

    let child_addr = {
        let state = memory_state();
        let parent = state.get_vmar_mut(parent_vmar).ok_or(ZxError::ErrNotFound)?;
        let child_addr = parent.allocate(requested_offset, size, flags, PAGE_SIZE)?;
        parent.children.push(child_handle as usize);
        child_addr
    };

    let mut child_vmar = Vmar::new(child_addr, size);
    child_vmar.handle = HandleValue(child_handle);
    child_vmar.parent_idx = Some(parent_vmar as usize);

    let state = memory_state();
    state.vmars.push(VmarRecord {
        handle: child_handle,
        vmar: child_vmar,
    });

    *out_child_vmar = child_handle;
    *out_child_addr = child_addr;
    Ok(())
}

/// Zircon sys_vmar_destroy implementation
pub fn sys_vmar_destroy(vmar_handle: u32) -> ZxResult {
    info!("vmar.destroy: handle={:#x?}", vmar_handle);

    if memory_state().remove_vmar(vmar_handle) {
        Ok(())
    } else {
        Err(ZxError::ErrNotFound)
    }
}

/// Zircon sys_vmar_unmap_handle_close_thread_exit implementation
///
/// This is a special Zircon syscall that unmaps a region and handles
/// closing threads that are exiting. It's used when a thread is exiting
/// and needs to clean up its stack mapping.
///
/// # Arguments
/// * `vmar_handle` - VMAR handle
/// * `addr` - Address to unmap
/// * `len` - Length of region to unmap
///
/// # Returns
/// * On success: Ok(())
/// * On error: ZxError
pub fn sys_vmar_unmap_handle_close_thread_exit(
    vmar_handle: u32,
    addr: usize,
    len: usize,
) -> ZxResult {
    info!(
        "vmar.unmap_handle_close_thread_exit: vmar={:#x}, addr={:#x}, len={:#x}",
        vmar_handle, addr, len
    );

    if addr == 0 || len == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }

    if !page_aligned(addr) {
        return Err(ZxError::ErrInvalidArgs);
    }

    let state = memory_state();
    let vmar = state.get_vmar_mut(vmar_handle).ok_or(ZxError::ErrNotFound)?;
    vmar.unmap_handle_close_thread_exit(addr, roundup_pages(len))
}

// ============================================================================
// Linux Process/Task Syscalls
// ============================================================================

/// Linux sys_fork implementation
pub fn sys_fork() -> SysResult {
    info!("fork");
    
    let pm = process_manager();
    if let Some(pid) = pm.create_process("forked") {
        Ok(pid)
    } else {
        Err(SysError::ENOMEM)
    }
}

/// Linux sys_vfork implementation
pub async fn sys_vfork() -> SysResult {
    info!("vfork");
    sys_fork()
}

/// Linux sys_clone implementation
pub fn sys_clone(
    flags: usize,
    newsp: usize,
    _parent_tid: usize,
    _newtls: usize,
    _child_tid: usize,
) -> SysResult {
    info!("clone: flags={:#x}, newsp={:#x}", flags, newsp);
    sys_fork()
}

/// Linux sys_execve implementation
pub fn sys_execve(_path: usize, _argv: usize, _envp: usize) -> SysResult {
    info!("execve: path={:#x}", _path);
    
    // TODO: Implement execve
    warn!("execve: unimplemented");
    Err(SysError::ENOSYS)
}

/// Linux sys_wait4 implementation
pub async fn sys_wait4(_pid: i32, _wstatus: usize, _options: u32) -> SysResult {
    info!("wait4: pid={}, options={:#x}", _pid, _options);
    
    // TODO: Implement wait4
    warn!("wait4: unimplemented");
    Err(SysError::ENOSYS)
}

/// Linux sys_exit implementation
pub fn sys_exit(exit_code: i32) -> SysResult {
    info!("exit: code={}", exit_code);

    if crate::user_level::user_test::prepare_el0_test_kernel_return(exit_code) {
        return Ok(0);
    }

    // TODO: Terminate current process
    Ok(0)
}

/// Linux sys_exit_group implementation
pub fn sys_exit_group(exit_code: i32) -> SysResult {
    info!("exit_group: code={}", exit_code);
    
    // TODO: Terminate all threads in process group
    Ok(0)
}

/// Linux sys_getpid implementation
pub fn sys_getpid() -> SysResult {
    // TODO: Return current process PID
    Ok(1) // Placeholder: init process
}

/// Linux sys_getppid implementation
pub fn sys_getppid() -> SysResult {
    // TODO: Return parent process PID
    Ok(0) // Placeholder
}

/// Linux sys_kill implementation
pub fn sys_kill(pid: isize, signum: usize) -> SysResult {
    info!("kill: pid={}, signal={}", pid, signum);
    
    if pid <= 0 {
        return Err(SysError::ESRCH);
    }
    
    let pm = process_manager();
    if pm.terminate_process(pid as usize) {
        Ok(0)
    } else {
        Err(SysError::ESRCH)
    }
}

// ============================================================================
// Zircon Process/Task Syscalls
// ============================================================================

/// Zircon sys_process_create implementation
pub fn sys_process_create(
    job_handle: u32,
    _name_ptr: usize,
    name_len: usize,
    _options: u32,
    out_proc_handle: &mut u32,
    out_vmar_handle: &mut u32,
) -> ZxResult {
    info!("process.create: job={:#x}, name_len={}", job_handle, name_len);
    
    let pm = process_manager();
    if let Some(_pid) = pm.create_process("zircon_proc") {
        *out_proc_handle = 1;
        *out_vmar_handle = memory_root_vmar_handle();
        Ok(())
    } else {
        Err(ZxError::ErrNoMemory)
    }
}

/// Zircon sys_process_exit implementation
pub fn sys_process_exit(handle: u32, exit_code: i32) -> ZxResult {
    info!("process.exit: handle={:#x}, code={}", handle, exit_code);
    
    // TODO: Terminate process
    Ok(())
}

/// Zircon sys_thread_create implementation
pub fn sys_thread_create(
    proc_handle: u32,
    _name_ptr: usize,
    name_len: usize,
    entry_point: usize,
    _stack_size: usize,
    out_thread_handle: &mut u32,
) -> ZxResult {
    info!(
        "thread.create: proc={:#x}, name_len={}, entry={:#x}",
        proc_handle, name_len, entry_point
    );
    
    // TODO: Create thread
    *out_thread_handle = 0;
    Ok(())
}

/// Zircon sys_thread_start implementation
pub fn sys_thread_start(
    thread_handle: u32,
    entry_point: usize,
    _stack_top: usize,
    _arg1: usize,
    _arg2: usize,
) -> ZxResult {
    info!("thread.start: handle={:#x}, entry={:#x}", thread_handle, entry_point);
    
    // TODO: Start thread execution
    Ok(())
}

/// Zircon sys_thread_exit implementation
pub fn sys_thread_exit() -> ZxResult {
    info!("thread.exit");
    
    // TODO: Terminate current thread
    Ok(())
}

/// Zircon sys_task_kill implementation
pub fn sys_task_kill(task_handle: u32) -> ZxResult {
    info!("task.kill: handle={:#x}", task_handle);
    
    // TODO: Kill task
    Ok(())
}

// ============================================================================
// Handle Syscalls (Zircon)
// ============================================================================

/// Zircon sys_handle_close implementation
pub fn sys_handle_close(handle: u32) -> ZxResult {
    info!("handle.close: handle={:#x}", handle);

    if handle == INVALID_HANDLE {
        return Err(ZxError::ErrInvalidArgs);
    }

    let _ = memory_state().release_handle(handle);
    Ok(())
}

/// Zircon sys_handle_close_many implementation
pub fn sys_handle_close_many(handles_ptr: usize, num_handles: usize) -> ZxResult {
    info!("handle.close_many: ptr={:#x}, count={}", handles_ptr, num_handles);

    if num_handles == 0 {
        return Ok(());
    }
    if handles_ptr == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }

    let handles = unsafe { core::slice::from_raw_parts(handles_ptr as *const u32, num_handles) };
    for handle in handles {
        sys_handle_close(*handle)?;
    }

    Ok(())
}

/// Zircon sys_handle_duplicate implementation
pub fn sys_handle_duplicate(
    handle: u32,
    rights: u32,
    out_handle: &mut u32,
) -> ZxResult {
    info!("handle.duplicate: handle={:#x}, rights={:#x}", handle, rights);
    
    // TODO: Duplicate handle with new rights
    *out_handle = handle; // Placeholder
    Ok(())
}

/// Zircon sys_handle_replace implementation
pub fn sys_handle_replace(
    handle: u32,
    rights: u32,
    out_handle: &mut u32,
) -> ZxResult {
    info!("handle.replace: handle={:#x}, rights={:#x}", handle, rights);
    
    // TODO: Replace handle with new rights
    *out_handle = handle; // Placeholder
    Ok(())
}

// ============================================================================
// Object Syscalls (Zircon)
// ============================================================================

/// Zircon sys_object_wait_one implementation
pub async fn sys_object_wait_one(
    handle: u32,
    signals: u32,
    _deadline: u64,
    out_pending: &mut u32,
) -> ZxResult {
    info!("object.wait_one: handle={:#x}, signals={:#x}", handle, signals);
    
    // TODO: Wait for signals
    *out_pending = 0;
    Ok(())
}

/// Zircon sys_object_wait_many implementation
pub async fn sys_object_wait_many(
    _items_ptr: usize,
    count: usize,
    _deadline: u64,
) -> ZxResult {
    info!("object.wait_many: count={}, deadline={:#x}", count, _deadline);
    
    // TODO: Wait for multiple objects
    Ok(())
}

/// Zircon sys_object_signal implementation
pub fn sys_object_signal(handle: u32, clear_mask: u32, set_mask: u32) -> ZxResult {
    info!("object.signal: handle={:#x}, clear={:#x}, set={:#x}",
          handle, clear_mask, set_mask);
    
    // TODO: Signal object
    Ok(())
}

/// Zircon sys_object_get_info implementation
pub fn sys_object_get_info(
    handle: u32,
    topic: u32,
    _buffer: usize,
    _buffer_size: usize,
    out_actual_size: &mut usize,
) -> ZxResult {
    info!("object.get_info: handle={:#x}, topic={:#x}", handle, topic);
    
    // TODO: Get object info
    *out_actual_size = 0;
    Ok(())
}

/// Zircon sys_object_get_property implementation
pub fn sys_object_get_property(
    handle: u32,
    prop_id: u32,
    _buffer: usize,
    _buffer_size: usize,
) -> ZxResult {
    info!("object.get_property: handle={:#x}, prop={:#x}", handle, prop_id);
    
    // TODO: Get property
    Ok(())
}

/// Zircon sys_object_set_property implementation
pub fn sys_object_set_property(
    handle: u32,
    prop_id: u32,
    _buffer: usize,
    _buffer_size: usize,
) -> ZxResult {
    info!("object.set_property: handle={:#x}, prop={:#x}", handle, prop_id);
    
    // TODO: Set property
    Ok(())
}

// ============================================================================
// Time Syscalls
// ============================================================================

/// Zircon sys_clock_get_monotonic implementation
pub fn sys_clock_get_monotonic() -> ZxResult<u64> {
    // TODO: Return monotonic clock value
    Ok(0)
}

/// Zircon sys_nanosleep implementation
pub async fn sys_nanosleep(deadline: u64) -> ZxResult {
    info!("nanosleep: deadline={:#x}", deadline);
    
    // TODO: Implement sleep
    Ok(())
}

/// Linux sys_clock_gettime implementation
pub fn sys_clock_gettime(_clock: usize, _buf: usize) -> SysResult {
    info!("clock_gettime: clock={}", _clock);
    
    // TODO: Implement clock_gettime
    Ok(0)
}

/// Linux sys_nanosleep implementation
pub async fn sys_nanosleep_linux(_req: usize) -> SysResult {
    info!("nanosleep (linux)");
    
    // TODO: Implement nanosleep
    Ok(0)
}

// ============================================================================
// Syscall Number Definitions
// ============================================================================

/// Linux syscall numbers
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxSyscall {
    IoSetup = 0,
    IoDestroy = 1,
    IoSubmit = 2,
    IoCancel = 3,
    IoGetevents = 4,
    Setxattr = 5,
    Lsetxattr = 6,
    Fsetxattr = 7,
    Getxattr = 8,
    Lgetxattr = 9,
    Fgetxattr = 10,
    Listxattr = 11,
    Llistxattr = 12,
    Flistxattr = 13,
    Removexattr = 14,
    Lremovexattr = 15,
    Fremovexattr = 16,
    Getcwd = 17,
    LookupDcookie = 18,
    Eventfd2 = 19,
    EpollCreate1 = 20,
    EpollCtl = 21,
    EpollPwait = 22,
    Dup = 23,
    Dup3 = 24,
    Fcntl = 25,
    InotifyInit1 = 26,
    InotifyAddWatch = 27,
    InotifyRmWatch = 28,
    Ioctl = 29,
    IoprioSet = 30,
    IoprioGet = 31,
    Flock = 32,
    Mknodat = 33,
    Mkdirat = 34,
    Unlinkat = 35,
    Symlinkat = 36,
    Linkat = 37,
    Renameat = 38,
    Umount = 39,
    Mount = 40,
    PivotRoot = 41,
    Nfsservctl = 42,
    Statfs = 43,
    Fstatfs = 44,
    Truncate = 45,
    Ftruncate = 46,
    Fallocate = 47,
    Faccessat = 48,
    Chdir = 49,
    Fchdir = 50,
    Chroot = 51,
    Fchmod = 52,
    Fchmodat = 53,
    Fchownat = 54,
    Fchown = 55,
    Openat = 56,
    Close = 57,
    Vhangup = 58,
    Pipe2 = 59,
    Quotactl = 60,
    Getdents64 = 61,
    Lseek = 62,
    Read = 63,
    Write = 64,
    Readv = 65,
    Writev = 66,
    Pread64 = 67,
    Pwrite64 = 68,
    Preadv = 69,
    Pwritev = 70,
    Sendfile = 71,
    Pselect6 = 72,
    Ppoll = 73,
    Signalfd4 = 74,
    Vmsplice = 75,
    Splice = 76,
    Tee = 77,
    Readlinkat = 78,
    Fstatat = 79,
    Fstat = 80,
    Sync = 81,
    Fsync = 82,
    Fdatasync = 83,
    SyncFileRange = 84,
    TimerfdCreate = 85,
    TimerfdSettime = 86,
    TimerfdGettime = 87,
    Utimensat = 88,
    Acct = 89,
    Capget = 90,
    Capset = 91,
    Personality = 92,
    Exit = 93,
    ExitGroup = 94,
    Waitid = 95,
    SetTidAddress = 96,
    Unshare = 97,
    Futex = 98,
    SetRobustList = 99,
    GetRobustList = 100,
    Nanosleep = 101,
    Getitimer = 102,
    Setitimer = 103,
    KexecLoad = 104,
    InitModule = 105,
    DeleteModule = 106,
    TimerCreate = 107,
    TimerGettime = 108,
    TimerGetoverrun = 109,
    TimerDelete = 110,
    ClockSettime = 111,
    ClockGettime = 112,
    ClockGetres = 113,
    ClockNanosleep = 114,
    Syslog = 115,
    Ptrace = 116,
    SchedSetparam = 117,
    SchedSetscheduler = 118,
    SchedGetscheduler = 119,
    SchedGetparam = 120,
    SchedSetaffinity = 121,
    SchedAffinity = 122,
    SchedYield = 123,
    SchedGetPriorityMax = 124,
    SchedGetPriorityMin = 125,
    SchedRrGetInterval = 126,
    RestartSyscall = 127,
    Kill = 128,
    Tkill = 129,
    Tgkill = 130,
    Sigaltstack = 131,
    RtSigaction = 132,
    RtSigprocmask = 133,
    RtSigpending = 134,
    RtSigtimedwait = 135,
    RtSigqueueinfo = 136,
    RtSigreturn = 137,
    Setpriority = 138,
    Getpriority = 139,
    Reboot = 140,
    Setregid = 141,
    Setgid = 142,
    Setreuid = 143,
    Setuid = 144,
    Setresuid = 145,
    Getresuid = 146,
    Setresgid = 147,
    Getresgid = 148,
    Setfsuid = 149,
    Setfsgid = 150,
    Times = 151,
    Setpgid = 152,
    Getpgid = 153,
    Getsid = 154,
    Setsid = 155,
    Getgroups = 156,
    Setgroups = 157,
    Uname = 158,
    Sethostname = 159,
    Setdomainname = 160,
    Getrlimit = 161,
    Setrlimit = 162,
    Getrusage = 163,
    Umask = 164,
    Prctl = 165,
    Getcpu = 166,
    Gettimeofday = 167,
    Settimeofday = 168,
    Adjtimex = 169,
    Getpid = 170,
    Getppid = 171,
    Getuid = 172,
    Geteuid = 173,
    Getgid = 174,
    Getegid = 175,
    Gettid = 176,
    Sysinfo = 177,
    MqOpen = 178,
    MqUnlink = 179,
    MqTimedsend = 180,
    MqTimedreceive = 181,
    MqNotify = 182,
    MqGetsetattr = 183,
    Msgget = 184,
    Msgctl = 185,
    Msgsnd = 186,
    Msgrcv = 187,
    Semget = 188,
    Semctl = 189,
    Semtimedop = 190,
    Semop = 191,
    Shmget = 192,
    Shmctl = 193,
    Shmat = 194,
    Shmdt = 195,
    Socket = 196,
    Socketpair = 197,
    Bind = 198,
    Listen = 199,
    Accept = 200,
    Connect = 201,
    Getsockname = 202,
    Getpeername = 203,
    Sendto = 204,
    Recvfrom = 205,
    Setsockopt = 206,
    Getsockopt = 207,
    Shutdown = 208,
    Sendmsg = 209,
    Recvmsg = 210,
    Readahead = 211,
    Brk = 212,
    Munmap = 213,
    Clone = 214,
    Execve = 215,
    Mmap = 216,  // Note: actual number may differ
    Mprotect = 217,
    Mremap = 218,
    Msync = 219,
    Mincore = 220,
    Madvise = 221,
    Accept4 = 222,
    Recvmsg2 = 223,
    // ARM64 specific
    Fork = 1000,
    Vfork = 1001,
}

/// Zircon syscall numbers
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZirconSyscall {
    HandleClose = 0,
    HandleCloseMany = 1,
    HandleDuplicate = 2,
    HandleReplace = 3,
    ObjectGetInfo = 4,
    ObjectGetProperty = 5,
    ObjectSetProperty = 6,
    ObjectSignal = 7,
    ObjectSignalPeer = 8,
    ObjectWaitOne = 9,
    ObjectWaitMany = 10,
    ObjectWaitAsync = 11,
    ThreadCreate = 12,
    ThreadStart = 13,
    ThreadWriteState = 14,
    ThreadReadState = 15,
    TaskKill = 16,
    ThreadExit = 17,
    ProcessCreate = 18,
    ProcessStart = 19,
    ProcessReadMemory = 20,
    ProcessWriteMemory = 21,
    ProcessExit = 22,
    JobCreate = 23,
    JobSetPolicy = 24,
    JobSetCritical = 25,
    TaskSuspendToken = 26,
    ChannelCreate = 27,
    ChannelRead = 28,
    ChannelWrite = 29,
    ChannelWriteEtc = 30,
    ChannelCallNoretry = 31,
    ChannelCallFinish = 32,
    SocketCreate = 33,
    SocketWrite = 34,
    SocketRead = 35,
    SocketShutdown = 36,
    StreamCreate = 37,
    StreamWritev = 38,
    StreamReadv = 39,
    StreamSeek = 40,
    FifoCreate = 41,
    FifoRead = 42,
    FifoWrite = 43,
    EventCreate = 44,
    EventpairCreate = 45,
    PortCreate = 46,
    PortWait = 47,
    PortQueue = 48,
    FutexWait = 49,
    FutexWake = 50,
    FutexRequeue = 51,
    VmoCreate = 52,
    VmoRead = 53,
    VmoWrite = 54,
    VmoGetSize = 55,
    VmoSetSize = 56,
    VmoOpRange = 57,
    VmarMap = 58,
    VmarUnmap = 59,
    VmarAllocate = 60,
    VmarProtect = 61,
    VmarDestroy = 62,
    VmarUnmapHandleCloseThreadExit = 75,
    CprngDrawOnce = 63,
    Nanosleep = 64,
    ClockCreate = 65,
    ClockGet = 66,
    ClockGetMonotonic = 67,
    TimerCreate = 68,
    TimerSet = 69,
    TimerCancel = 70,
    DebugWrite = 71,
    DebuglogCreate = 72,
    ResourceCreate = 73,
    SystemGetEvent = 74,
}

// ============================================================================
// Syscall Dispatcher
// ============================================================================

/// Dispatch a Linux syscall
pub fn dispatch_linux_syscall(syscall_num: u32, args: [usize; 6]) -> SysResult {
    match syscall_num {
        ARM64_SYS_WRITE => {
            sys_write(args[0], args[1], args[2])
        }
        ARM64_SYS_EXIT => {
            sys_exit(args[0] as i32)
        }
        ARM64_SYS_GETPID => {
            sys_getpid()
        }
        ARM64_SYS_MMAP => {
            sys_mmap(args[0], args[1], args[2], args[3], args[4], args[5] as u64)
        }
        num if num == LinuxSyscall::Mmap as u32 => {
            sys_mmap(args[0], args[1], args[2], args[3], args[4], args[5] as u64)
        }
        num if num == LinuxSyscall::Munmap as u32 => {
            sys_munmap(args[0], args[1])
        }
        num if num == LinuxSyscall::Mprotect as u32 => {
            sys_mprotect(args[0], args[1], args[2])
        }
        num if num == LinuxSyscall::Brk as u32 => {
            sys_brk(args[0])
        }
        num if num == LinuxSyscall::Mremap as u32 => {
            sys_mremap(args[0], args[1], args[2], args[3], args[4])
        }
        num if num == LinuxSyscall::Fork as u32 => {
            sys_fork()
        }
        num if num == LinuxSyscall::Exit as u32 => {
            sys_exit(args[0] as i32)
        }
        num if num == LinuxSyscall::ExitGroup as u32 => {
            sys_exit_group(args[0] as i32)
        }
        num if num == LinuxSyscall::Getpid as u32 => {
            sys_getpid()
        }
        num if num == LinuxSyscall::Getppid as u32 => {
            sys_getppid()
        }
        num if num == LinuxSyscall::Kill as u32 => {
            sys_kill(args[0] as isize, args[1])
        }
        num if num == LinuxSyscall::Gettid as u32 => {
            Ok(1) // Placeholder
        }
        _ => {
            warn!("Unimplemented Linux syscall: {}", syscall_num);
            Err(SysError::ENOSYS)
        }
    }
}

/// Dispatch a Zircon syscall
pub fn dispatch_zircon_syscall(syscall_num: u32, args: [usize; 8]) -> ZxResult<usize> {
    match syscall_num {
        num if num == ZirconSyscall::HandleClose as u32 => {
            sys_handle_close(args[0] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::HandleDuplicate as u32 => {
            let mut out = 0u32;
            sys_handle_duplicate(args[0] as u32, args[1] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::VmoCreate as u32 => {
            let mut out = 0u32;
            sys_vmo_create(args[0] as u64, args[1] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::VmoRead as u32 => {
            // Simplified: would need buffer handling
            sys_vmo_read(args[0] as u32, &mut [], args[2] as u64)
        }
        num if num == ZirconSyscall::VmoWrite as u32 => {
            sys_vmo_write(args[0] as u32, &[], args[2] as u64)
        }
        num if num == ZirconSyscall::VmoGetSize as u32 => {
            let mut size = 0usize;
            sys_vmo_get_size(args[0] as u32, &mut size).map(|_| size)
        }
        num if num == ZirconSyscall::VmarMap as u32 => {
            let mut addr = 0usize;
            sys_vmar_map(args[0] as u32, args[1] as u32, args[2], args[3] as u32,
                        args[4], args[5], &mut addr).map(|_| addr)
        }
        num if num == ZirconSyscall::VmarUnmap as u32 => {
            sys_vmar_unmap(args[0] as u32, args[1], args[2]).map(|_| 0)
        }
        num if num == ZirconSyscall::VmarUnmapHandleCloseThreadExit as u32 => {
            sys_vmar_unmap_handle_close_thread_exit(args[0] as u32, args[1], args[2]).map(|_| 0)
        }
        num if num == ZirconSyscall::ProcessCreate as u32 => {
            let mut proc_h = 0u32;
            let mut vmar_h = 0u32;
            sys_process_create(args[0] as u32, args[1], args[2], args[3] as u32,
                             &mut proc_h, &mut vmar_h).map(|_| proc_h as usize)
        }
        _ => {
            warn!("Unimplemented Zircon syscall: {}", syscall_num);
            Err(ZxError::ErrNotSupported)
        }
    }
}

// ============================================================================
// Initialization
// ============================================================================

/// Initialize syscall subsystem
pub fn init() {
    info!("Initializing syscall interface layer...");
    info!("  - Linux syscall interface: ready");
    info!("  - Zircon syscall interface: ready");
    info!("  - Handle management: ready");
    info!("  - VMO/VMAR support: ready");
}
