//! MMU and Page Table Management
//!
//! This module provides:
//! - Architecture-aware page table entry management
//! - Virtual to physical address translation
//! - Memory protection and permissions
//! - User/kernel memory isolation model

#![allow(dead_code)]
#![allow(static_mut_refs)]

#[cfg(not(target_arch = "aarch64"))]
use crate::kernel_lowlevel::memory::PageFrameAllocator;
use crate::kernel_lowlevel::memory::PAGE_SIZE;
use alloc::vec::Vec;
#[cfg(target_arch = "aarch64")]
use core::sync::atomic::{AtomicU64, Ordering};

use super::lowlevel_logic;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressSpaceError {
    OutOfMemory,
    InvalidAddress,
    InvalidPermissions,
    AlreadyMapped,
    NotMapped,
    PermissionDenied,
}

const PTE_VALID: u64 = 1 << 0;

#[cfg(target_arch = "aarch64")]
mod arch_pte {
    pub const TABLE_OR_PAGE: u64 = 1 << 1;
    pub const ATTR_INDEX_SHIFT: u64 = 2;
    pub const ATTR_INDEX_MASK: u64 = 0x7 << ATTR_INDEX_SHIFT;
    pub const AP_EL0: u64 = 1 << 6;
    pub const AP_READ_ONLY: u64 = 1 << 7;
    pub const SH_SHIFT: u64 = 8;
    pub const SH_MASK: u64 = 0x3 << SH_SHIFT;
    pub const AF: u64 = 1 << 10;
    pub const UXN: u64 = 1 << 54;
    pub const PXN: u64 = 1 << 53;
    pub const OUTPUT_ADDR_MASK: u64 = 0x0000_FFFF_FFFF_F000;

    pub fn set_leaf(value: u64) -> u64 {
        value | TABLE_OR_PAGE
    }

    pub fn set_user(value: u64, allow: bool) -> u64 {
        set_flag(value, AP_EL0, allow)
    }

    pub fn set_read_only(value: u64, read_only: bool) -> u64 {
        set_flag(value, AP_READ_ONLY, read_only)
    }

    pub fn set_execute_never(value: u64, execute_never: bool) -> u64 {
        set_flag(value, UXN, execute_never)
    }

    pub fn set_privileged_execute_never(value: u64, execute_never: bool) -> u64 {
        set_flag(value, PXN, execute_never)
    }

    pub fn set_accessed(value: u64) -> u64 {
        value | AF
    }

    pub fn set_attr_idx(value: u64, idx: u64) -> u64 {
        (value & !ATTR_INDEX_MASK) | ((idx << ATTR_INDEX_SHIFT) & ATTR_INDEX_MASK)
    }

    pub fn set_sh(value: u64, sharability: u64) -> u64 {
        (value & !SH_MASK) | ((sharability << SH_SHIFT) & SH_MASK)
    }

    pub fn set_output_address(value: u64, paddr: u64) -> u64 {
        (value & !OUTPUT_ADDR_MASK) | (paddr & OUTPUT_ADDR_MASK)
    }

    pub fn output_address(value: u64) -> u64 {
        value & OUTPUT_ADDR_MASK
    }

    pub fn is_table(value: u64) -> bool {
        value & super::PTE_VALID != 0 && value & TABLE_OR_PAGE == 0
    }

    pub fn is_leaf(value: u64) -> bool {
        value & TABLE_OR_PAGE != 0
    }

    fn set_flag(value: u64, flag: u64, enabled: bool) -> u64 {
        if enabled {
            value | flag
        } else {
            value & !flag
        }
    }
}

#[cfg(target_arch = "riscv64")]
mod arch_pte {
    pub const READ: u64 = 1 << 1;
    pub const WRITE: u64 = 1 << 2;
    pub const EXECUTE: u64 = 1 << 3;
    pub const USER: u64 = 1 << 4;
    pub const GLOBAL: u64 = 1 << 5;
    pub const ACCESSED: u64 = 1 << 6;
    pub const DIRTY: u64 = 1 << 7;
    pub const PPN_SHIFT: u64 = 10;
    pub const PPN_MASK: u64 = 0x003F_FFFF_FFFF_FC00;

    pub fn set_leaf(value: u64) -> u64 {
        value | READ
    }

    pub fn set_user(value: u64, allow: bool) -> u64 {
        set_flag(value, USER, allow)
    }

    pub fn set_read_only(value: u64, read_only: bool) -> u64 {
        set_flag(value, WRITE | DIRTY, !read_only)
    }

    pub fn set_execute_never(value: u64, execute_never: bool) -> u64 {
        set_flag(value, EXECUTE, !execute_never)
    }

    pub fn set_privileged_execute_never(value: u64, _execute_never: bool) -> u64 {
        value
    }

    pub fn set_accessed(value: u64) -> u64 {
        value | ACCESSED
    }

    pub fn set_attr_idx(value: u64, _idx: u64) -> u64 {
        value
    }

    pub fn set_sh(value: u64, _sharability: u64) -> u64 {
        value
    }

    pub fn set_output_address(value: u64, paddr: u64) -> u64 {
        (value & !PPN_MASK) | (((paddr >> 12) << PPN_SHIFT) & PPN_MASK)
    }

    pub fn output_address(value: u64) -> u64 {
        ((value & PPN_MASK) >> PPN_SHIFT) << 12
    }

    pub fn is_table(value: u64) -> bool {
        value & super::PTE_VALID != 0 && value & (READ | WRITE | EXECUTE) == 0
    }

    pub fn is_leaf(value: u64) -> bool {
        value & (READ | WRITE | EXECUTE) != 0
    }

    fn set_flag(value: u64, flag: u64, enabled: bool) -> u64 {
        if enabled {
            value | flag
        } else {
            value & !flag
        }
    }
}

#[cfg(target_arch = "x86_64")]
mod arch_pte {
    pub const WRITE: u64 = 1 << 1;
    pub const USER: u64 = 1 << 2;
    pub const ACCESSED: u64 = 1 << 5;
    pub const DIRTY: u64 = 1 << 6;
    pub const EXECUTE_DISABLE: u64 = 1 << 63;
    pub const OUTPUT_ADDR_MASK: u64 = 0x000f_ffff_ffff_f000;

    pub fn set_leaf(value: u64) -> u64 {
        value
    }

    pub fn set_user(value: u64, allow: bool) -> u64 {
        set_flag(value, USER, allow)
    }

    pub fn set_read_only(value: u64, read_only: bool) -> u64 {
        set_flag(value, WRITE | DIRTY, !read_only)
    }

    pub fn set_execute_never(value: u64, execute_never: bool) -> u64 {
        set_flag(value, EXECUTE_DISABLE, execute_never)
    }

    pub fn set_privileged_execute_never(value: u64, execute_never: bool) -> u64 {
        set_execute_never(value, execute_never)
    }

    pub fn set_accessed(value: u64) -> u64 {
        value | ACCESSED
    }

    pub fn set_attr_idx(value: u64, _idx: u64) -> u64 {
        value
    }

    pub fn set_sh(value: u64, _sharability: u64) -> u64 {
        value
    }

    pub fn set_output_address(value: u64, paddr: u64) -> u64 {
        (value & !OUTPUT_ADDR_MASK) | (paddr & OUTPUT_ADDR_MASK)
    }

    pub fn output_address(value: u64) -> u64 {
        value & OUTPUT_ADDR_MASK
    }

    pub fn is_table(value: u64) -> bool {
        value & super::PTE_VALID != 0 && value & DIRTY == 0
    }

    pub fn is_leaf(value: u64) -> bool {
        value & super::PTE_VALID != 0
    }

    fn set_flag(value: u64, flag: u64, enabled: bool) -> u64 {
        if enabled {
            value | flag
        } else {
            value & !flag
        }
    }
}

bitflags::bitflags! {
    pub struct PageAttr: u64 {
        const VALID = 1 << 0;
        const USER = 1 << 1;
        const READ = 1 << 2;
        const WRITE = 1 << 3;
        const EXECUTE = 1 << 4;
        const READ_ONLY = 1 << 5;
        const EXECUTE_NEVER = 1 << 6;
    }
}

/// Page table slot count for one 4 KiB table page.
const PT_ENTRIES: usize = 512; // 2^9 entries per table
const MAX_PAGE_TABLE_PAGES: usize = 32;

/// Page table entry
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct PageTableEntry {
    pub value: u64,
}

impl PageTableEntry {
    pub const fn new() -> Self {
        Self { value: 0 }
    }

    pub fn is_valid(&self) -> bool {
        self.value & PTE_VALID != 0
    }

    pub fn set_valid(&mut self, valid: bool) {
        self.value = set_flag(self.value, PTE_VALID, valid);
    }

    pub fn set_leaf(&mut self, leaf: bool) {
        if leaf {
            self.value = arch_pte::set_leaf(self.value);
        }
    }

    pub fn set_user_accessible(&mut self, allow: bool) {
        self.value = arch_pte::set_user(self.value, allow);
    }

    pub fn set_read_only(&mut self, read_only: bool) {
        self.value = arch_pte::set_read_only(self.value, read_only);
    }

    pub fn set_xn(&mut self, execute_never: bool) {
        self.value = arch_pte::set_execute_never(self.value, execute_never);
    }

    pub fn set_pxn(&mut self, privileged_execute_never: bool) {
        self.value = arch_pte::set_privileged_execute_never(self.value, privileged_execute_never);
    }

    pub fn set_output_address(&mut self, paddr: u64) {
        self.value = arch_pte::set_output_address(self.value, paddr);
    }

    pub fn get_output_address(&self) -> u64 {
        arch_pte::output_address(self.value)
    }

    pub fn is_table(&self) -> bool {
        arch_pte::is_table(self.value)
    }

    pub fn is_leaf(&self) -> bool {
        arch_pte::is_leaf(self.value)
    }

    pub fn set_attr_idx(&mut self, idx: u64) {
        self.value = arch_pte::set_attr_idx(self.value, idx);
    }

    pub fn set_af(&mut self) {
        self.value = arch_pte::set_accessed(self.value);
    }

    pub fn set_sh(&mut self, sharability: u64) {
        self.value = arch_pte::set_sh(self.value, sharability);
    }
}

fn set_flag(value: u64, flag: u64, enabled: bool) -> u64 {
    if enabled {
        value | flag
    } else {
        value & !flag
    }
}

#[repr(C, align(4096))]
struct PageTablePage {
    entries: [PageTableEntry; PT_ENTRIES],
}

impl PageTablePage {
    const fn new() -> Self {
        Self {
            entries: [PageTableEntry::new(); PT_ENTRIES],
        }
    }
}

/// Virtual memory region descriptor
#[repr(C)]
#[derive(Debug, Clone)]
pub struct Vma {
    /// Start virtual address
    pub start: usize,
    /// End virtual address
    pub end: usize,
    /// Page table permissions
    pub flags: PageAttr,
    /// Physical address (for mapped regions)
    pub paddr: Option<u64>,
    /// Whether this VMA is valid
    pub valid: bool,
}

impl Vma {
    pub fn new(start: usize, end: usize, flags: PageAttr) -> Self {
        Self {
            start,
            end,
            flags,
            paddr: None,
            valid: true,
        }
    }

    pub fn size(&self) -> usize {
        lowlevel_logic::vma_size(self.start, self.end)
    }
}

/// Page table manager
pub struct PageTableManager {
    /// User address-space root table.
    pub user_root: *mut PageTableEntry,
    /// Kernel address-space root table.
    pub kernel_root: *mut PageTableEntry,
    /// Current ASID (Address Space ID)
    pub asid: u16,
    /// VMAs for this address space
    pub vmas: Vec<Vma>,
    #[cfg(target_arch = "aarch64")]
    user_address_space: crate::kernel_lowlevel::Aarch64AddressSpace,
}

impl PageTableManager {
    /// Create a new page table manager
    pub fn new() -> Option<Self> {
        #[cfg(target_arch = "aarch64")]
        let user_address_space =
            crate::kernel_lowlevel::Aarch64AddressSpace::new_with_kernel_map().ok()?;
        #[cfg(target_arch = "aarch64")]
        let user_root_vaddr = user_address_space.root_paddr() as *mut PageTableEntry;

        #[cfg(not(target_arch = "aarch64"))]
        let user_root_pfn = allocate_page_table_root()?;
        #[cfg(not(target_arch = "aarch64"))]
        let user_root_vaddr = map_page_table(user_root_pfn)?;
        #[cfg(not(target_arch = "aarch64"))]
        unsafe {
            core::ptr::write_bytes(user_root_vaddr, 0, PT_ENTRIES);
        }

        #[cfg(target_arch = "aarch64")]
        let kernel_root_vaddr = core::ptr::null_mut();
        #[cfg(not(target_arch = "aarch64"))]
        let kernel_root_pfn = PageFrameAllocator::alloc()?;
        #[cfg(not(target_arch = "aarch64"))]
        let kernel_root_vaddr = map_page_table(kernel_root_pfn)?;
        #[cfg(not(target_arch = "aarch64"))]
        unsafe {
            core::ptr::write_bytes(kernel_root_vaddr, 0, PT_ENTRIES);
        }

        Some(Self {
            user_root: user_root_vaddr,
            kernel_root: kernel_root_vaddr,
            asid: 0,
            vmas: Vec::new(),
            #[cfg(target_arch = "aarch64")]
            user_address_space,
        })
    }

    /// Map a region in user space.
    pub fn map_user_region(
        &mut self,
        vaddr: usize,
        paddr: u64,
        size: usize,
        readable: bool,
        writable: bool,
        executable: bool,
    ) -> bool {
        #[cfg(target_arch = "aarch64")]
        {
            if paddr & (PAGE_SIZE as u64 - 1) != 0 || size & (PAGE_SIZE - 1) != 0 {
                return false;
            }
            let end = match vaddr.checked_add(size) {
                Some(end) => end,
                None => return false,
            };
            let first_pfn = paddr / PAGE_SIZE as u64;
            let page_count = size / PAGE_SIZE;
            if self
                .user_address_space
                .map_user_region(vaddr, first_pfn, page_count, readable, writable, executable)
                .is_err()
            {
                return false;
            }

            let mut addr = vaddr;
            while addr < end {
                self.vmas
                    .push(Vma::new(addr, addr + PAGE_SIZE, PageAttr::empty()));
                addr += PAGE_SIZE;
            }
            return true;
        }

        #[cfg(not(target_arch = "aarch64"))]
        {
            let mut addr = vaddr;
            let mut paddr = paddr;
            let end = vaddr + size;

            while addr < end {
                let pte = self.walk_user_page_table(addr);
                if pte.is_null() {
                    return false;
                }

                unsafe {
                    (*pte).set_valid(true);
                    (*pte).set_leaf(true);
                    (*pte).set_output_address(paddr);
                    (*pte).set_user_accessible(true);
                    (*pte).set_af();
                    (*pte).set_sh(3); // Inner shareable
                    (*pte).set_attr_idx(0);

                    (*pte).set_read_only(!writable);
                    (*pte).set_xn(!executable);
                }

                // Add VMA
                self.vmas
                    .push(Vma::new(addr, addr + PAGE_SIZE, PageAttr::empty()));

                addr += PAGE_SIZE;
                paddr += PAGE_SIZE as u64;
            }

            true
        }
    }

    /// Map a region in kernel space.
    pub fn map_kernel_region(
        &mut self,
        vaddr: usize,
        paddr: u64,
        size: usize,
        _readable: bool,
        writable: bool,
        executable: bool,
        user_accessible: bool,
    ) -> bool {
        #[cfg(target_arch = "aarch64")]
        {
            let _ = (
                vaddr,
                paddr,
                size,
                _readable,
                writable,
                executable,
                user_accessible,
            );
            return false; // AArch64 kernel mappings live in the shared root.
        }

        #[cfg(not(target_arch = "aarch64"))]
        {
            let mut addr = vaddr;
            let mut paddr = paddr;
            let end = vaddr + size;

            while addr < end {
                let pte = self.walk_kernel_page_table(addr);
                if pte.is_null() {
                    return false;
                }

                unsafe {
                    (*pte).set_valid(true);
                    (*pte).set_leaf(true);
                    (*pte).set_output_address(paddr);
                    if user_accessible {
                        (*pte).set_user_accessible(true);
                    }
                    (*pte).set_af();
                    (*pte).set_sh(3);
                    (*pte).set_attr_idx(0);

                    (*pte).set_read_only(!writable);
                    (*pte).set_xn(!executable);
                    if !user_accessible && executable {
                        (*pte).set_pxn(false); // Allow privileged execution
                    }
                }

                addr += PAGE_SIZE;
                paddr += PAGE_SIZE as u64;
            }

            true
        }
    }

    /// Select the user-space page-table slot for this virtual address.
    fn walk_user_page_table(&mut self, vaddr: usize) -> *mut PageTableEntry {
        #[cfg(target_arch = "aarch64")]
        {
            let _ = vaddr;
            return core::ptr::null_mut();
        }

        #[cfg(not(target_arch = "aarch64"))]
        {
            let idx = legacy_page_table_slot(vaddr);

            if idx >= PT_ENTRIES {
                return core::ptr::null_mut();
            }

            unsafe { self.user_root.add(idx) }
        }
    }

    /// Select the kernel-space page-table slot for this virtual address.
    fn walk_kernel_page_table(&mut self, vaddr: usize) -> *mut PageTableEntry {
        let idx = legacy_page_table_slot(vaddr);

        if idx >= PT_ENTRIES {
            return core::ptr::null_mut();
        }

        unsafe { self.kernel_root.add(idx) }
    }

    /// Switch to this address space
    pub fn switch_to(&self) {
        arch_switch_to(self.user_root as u64, self.kernel_root as u64);
    }
}

fn legacy_page_table_slot(vaddr: usize) -> usize {
    (vaddr >> 21) & (PT_ENTRIES - 1)
}

#[cfg(target_arch = "aarch64")]
fn arch_switch_to(ttbr0: u64, ttbr1: u64) {
    unsafe {
        core::arch::asm!("msr ttbr0_el1, {ttbr0}", ttbr0 = in(reg) ttbr0, options(nostack));
        core::arch::asm!("msr ttbr1_el1, {ttbr1}", ttbr1 = in(reg) ttbr1, options(nostack));
        core::arch::asm!("tlbi vmalle1is", "dsb ish", "isb", options(nostack));
    }
}

#[cfg(target_arch = "riscv64")]
fn arch_switch_to(root: u64, _kernel_root: u64) {
    let satp = (8usize << 60) | ((root as usize) >> 12);
    unsafe {
        core::arch::asm!(
            "csrw satp, {satp}",
            "sfence.vma",
            satp = in(reg) satp,
            options(nostack),
        );
    }
}

#[cfg(target_arch = "x86_64")]
fn arch_switch_to(root: u64, _kernel_root: u64) {
    if root != 0 {
        unsafe {
            core::arch::asm!("mov cr3, {root}", root = in(reg) root, options(nostack));
        }
    }
}

/// Map a page table page to a virtual address
fn map_page_table(pfn: u64) -> Option<*mut PageTableEntry> {
    let _ = pfn;

    unsafe {
        if NEXT_PAGE_TABLE_SLOT >= MAX_PAGE_TABLE_PAGES {
            return None;
        }

        let slot = NEXT_PAGE_TABLE_SLOT;
        NEXT_PAGE_TABLE_SLOT += 1;
        Some(PAGE_TABLE_POOL[slot].entries.as_mut_ptr())
    }
}

#[cfg(not(target_arch = "aarch64"))]
fn allocate_page_table_root() -> Option<u64> {
    PageFrameAllocator::alloc()
}

/// Global page table manager for kernel
static mut KERNEL_PAGETABLE_MANAGER: Option<PageTableManager> = None;
static mut PAGE_TABLE_POOL: [PageTablePage; MAX_PAGE_TABLE_PAGES] =
    [const { PageTablePage::new() }; MAX_PAGE_TABLE_PAGES];
static mut NEXT_PAGE_TABLE_SLOT: usize = 0;
#[cfg(target_arch = "aarch64")]
static BOOTSTRAP_ROOT: AtomicU64 = AtomicU64::new(0);

#[cfg(target_arch = "aarch64")]
pub fn bootstrap_root() -> u64 {
    BOOTSTRAP_ROOT.load(Ordering::Acquire)
}

#[cfg(not(target_arch = "aarch64"))]
pub fn bootstrap_root() -> u64 {
    0
}

#[cfg(target_arch = "aarch64")]
pub fn activate_bootstrap_on_current_cpu() -> bool {
    let root = bootstrap_root();
    if root == 0 {
        return false;
    }
    unsafe {
        crate::kernel_lowlevel::cpu::install_stage1_translation(root);
    }
    true
}

#[cfg(not(target_arch = "aarch64"))]
pub fn activate_bootstrap_on_current_cpu() -> bool {
    false
}

/// Initialize MMU subsystem
pub fn init() {
    let manager = PageTableManager::new();
    #[cfg(target_arch = "aarch64")]
    if let Some(manager) = manager.as_ref() {
        BOOTSTRAP_ROOT.store(manager.user_address_space.root_paddr(), Ordering::Release);
    }
    unsafe {
        KERNEL_PAGETABLE_MANAGER = manager;
    }
    #[cfg(target_arch = "aarch64")]
    let activated = activate_bootstrap_on_current_cpu();
    crate::kernel_lowlevel::serial::Serial::new().init();
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    #[cfg(target_arch = "aarch64")]
    if !activated {
        serial.write_str("[MMU] Bootstrap activation failed\n");
        return;
    }
    serial.write_str("[MMU] Page table manager initialized\n");
}

/// Get kernel page table manager
pub fn get_kernel_manager() -> Option<&'static mut PageTableManager> {
    unsafe { KERNEL_PAGETABLE_MANAGER.as_mut() }
}
