//! MMU and Page Table Management
//!
//! This module provides:
//! - Page table management (4-level ARM64 page tables)
//! - Virtual to physical address translation
//! - Memory protection and permissions
//! - EL0/EL1 memory isolation

#![allow(dead_code)]
#![allow(static_mut_refs)]

use crate::kernel_lowlevel::memory::{PageFrameAllocator, PAGE_SIZE};
use alloc::vec::Vec;

use super::lowlevel_logic;

// Page table entry flags
bitflags::bitflags! {
    pub struct PageAttr: u64 {
        const VALID = 1 << 0;
        const BLOCK = 1 << 1; // Block mapping (larger pages)
        const ATTR_INDX_0 = 0 << 2;
        const ATTR_INDX_1 = 1 << 2;
        const ATTR_INDX_2 = 2 << 2;
        const NS = 1 << 5; // Non-secure
        const AP_EL0 = 1 << 6; // Accessible from EL0
        const AP_READ_ONLY = 1 << 7; // Read-only
        const SH_INNER = 3 << 8; // Inner shareable
        const AF = 1 << 10; // Access flag
        const N_G = 1 << 11; // Not global
        const UXN = 1 << 54; // Unprivileged execute-never
        const PXN = 1 << 53; // Privileged execute-never
    }
}

// Memory attributes for MAIR_EL1
pub const ATTR_NORMAL_CACHED: u8 = 0xFF; // Normal memory, write-back
pub const ATTR_NORMAL_UNCACHED: u8 = 0x44; // Device memory
pub const ATTR_DEVICE: u8 = 0x00; // Device memory

// Translation Control Register flags
pub const TCR_T0SZ: u64 = 16; // TTBR0 region size (2^(64-16) = 2^48)
pub const TCR_T1SZ: u64 = 16 << 16; // TTBR1 region size
pub const TCR_TG0_4K: u64 = 0 << 14; // TTBR0 4KB granule
pub const TCR_TG1_4K: u64 = 2 << 30; // TTBR1 4KB granule
pub const TCR_SH0_INNER: u64 = 3 << 12; // TTBR0 inner shareable
pub const TCR_SH1_INNER: u64 = 3 << 28; // TTBR1 inner shareable
pub const TCR_ORGN0_WB: u64 = 1 << 10; // TTBR0 outer write-back
pub const TCR_IRGN0_WB: u64 = 1 << 8; // TTBR0 inner write-back
pub const TCR_ORGN1_WB: u64 = 1 << 26; // TTBR1 outer write-back
pub const TCR_IRGN1_WB: u64 = 1 << 24; // TTBR1 inner write-back
pub const TCR_IPS_4GB: u64 = 0 << 32; // 4GB physical address space

// System Control Register flags
pub const SCTLR_M: u64 = 1 << 0; // MMU enable
pub const SCTLR_A: u64 = 1 << 1; // Alignment check
pub const SCTLR_C: u64 = 1 << 2; // Data cache enable
pub const SCTLR_I: u64 = 1 << 12; // Instruction cache enable

/// Page table levels for ARM64 (4KB granule, 4 levels)
const PT_LEVEL_COUNT: usize = 4;
const PT_ENTRIES: usize = 512; // 2^9 entries per table
const PT_ENTRY_SIZE: usize = 8; // 64-bit entries
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
        self.value & 1 != 0
    }

    pub fn set_valid(&mut self, valid: bool) {
        self.value = lowlevel_logic::pte_set_flag(self.value, 1, valid);
    }

    pub fn set_block(&mut self, block: bool) {
        self.value = lowlevel_logic::pte_set_flag(self.value, 1 << 1, block);
    }

    pub fn set_ap_el0(&mut self, allow: bool) {
        self.value = lowlevel_logic::pte_set_flag(self.value, 1 << 6, allow);
    }

    pub fn set_ap_read_only(&mut self, read_only: bool) {
        self.value = lowlevel_logic::pte_set_flag(self.value, 1 << 7, read_only);
    }

    pub fn set_xn(&mut self, execute_never: bool) {
        self.value = lowlevel_logic::pte_set_flag(self.value, 1 << 54, execute_never);
    }

    pub fn set_pxn(&mut self, privileged_execute_never: bool) {
        self.value = lowlevel_logic::pte_set_flag(self.value, 1 << 53, privileged_execute_never);
    }

    pub fn set_output_address(&mut self, paddr: u64) {
        // Output address is bits [47:12] for level 3
        self.value = lowlevel_logic::pte_set_output_address(self.value, paddr);
    }

    pub fn get_output_address(&self) -> u64 {
        lowlevel_logic::pte_output_address(self.value)
    }

    pub fn is_table(&self) -> bool {
        lowlevel_logic::pte_table(self.value)
    }

    pub fn is_block(&self) -> bool {
        self.value & (1 << 1) != 0
    }

    pub fn set_attr_idx(&mut self, idx: u64) {
        self.value = lowlevel_logic::pte_attr_idx(self.value, idx);
    }

    pub fn set_af(&mut self) {
        self.value |= 1 << 10;
    }

    pub fn set_sh(&mut self, sharability: u64) {
        self.value = lowlevel_logic::pte_sh(self.value, sharability);
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
    /// Root page table (TTBR0)
    pub root_ttbr0: *mut PageTableEntry,
    /// Root page table (TTBR1 - kernel space)
    pub root_ttbr1: *mut PageTableEntry,
    /// Current ASID (Address Space ID)
    pub asid: u16,
    /// VMAs for this address space
    pub vmas: Vec<Vma>,
}

impl PageTableManager {
    /// Create a new page table manager
    pub fn new() -> Option<Self> {
        // Allocate page for root TTBR0 (user space)
        let ttbr0_pfn = PageFrameAllocator::alloc()?;
        let ttbr0_vaddr = map_page_table(ttbr0_pfn)?;
        // Zero out the page table
        unsafe {
            core::ptr::write_bytes(ttbr0_vaddr, 0, PT_ENTRIES);
        }

        // Allocate page for root TTBR1 (kernel space)
        let ttbr1_pfn = PageFrameAllocator::alloc()?;
        let ttbr1_vaddr = map_page_table(ttbr1_pfn)?;
        // Zero out the page table
        unsafe {
            core::ptr::write_bytes(ttbr1_vaddr, 0, PT_ENTRIES);
        }

        Some(Self {
            root_ttbr0: ttbr0_vaddr,
            root_ttbr1: ttbr1_vaddr,
            asid: 0,
            vmas: Vec::new(),
        })
    }

    /// Map a region in user space (TTBR0)
    pub fn map_user_region(
        &mut self,
        vaddr: usize,
        paddr: u64,
        size: usize,
        _readable: bool,
        writable: bool,
        executable: bool,
    ) -> bool {
        let mut addr = vaddr;
        let mut paddr = paddr;
        let end = vaddr + size;

        while addr < end {
            let pte = self.walk_page_tables_ttbr0(addr);
            if pte.is_null() {
                return false;
            }

            unsafe {
                (*pte).set_valid(true);
                (*pte).set_block(true); // Use block mapping for simplicity
                (*pte).set_output_address(paddr);
                (*pte).set_ap_el0(true); // Allow EL0 access
                (*pte).set_af(); // Set access flag
                (*pte).set_sh(3); // Inner shareable
                (*pte).set_attr_idx(0); // Use cached attribute

                // Set permissions
                if !writable {
                    (*pte).set_ap_read_only(true);
                }
                if !executable {
                    (*pte).set_xn(true);
                }
            }

            // Add VMA
            self.vmas
                .push(Vma::new(addr, addr + PAGE_SIZE, PageAttr::empty()));

            addr += PAGE_SIZE;
            paddr += PAGE_SIZE as u64;
        }

        true
    }

    /// Map a region in kernel space (TTBR1)
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
        let mut addr = vaddr;
        let mut paddr = paddr;
        let end = vaddr + size;

        while addr < end {
            let pte = self.walk_page_tables_ttbr1(addr);
            if pte.is_null() {
                return false;
            }

            unsafe {
                (*pte).set_valid(true);
                (*pte).set_block(true);
                (*pte).set_output_address(paddr);
                if user_accessible {
                    (*pte).set_ap_el0(true);
                }
                (*pte).set_af();
                (*pte).set_sh(3);
                (*pte).set_attr_idx(0);

                if !writable {
                    (*pte).set_ap_read_only(true);
                }
                if !executable {
                    (*pte).set_xn(true);
                }
                if !user_accessible && executable {
                    (*pte).set_pxn(false); // Allow privileged execution
                }
            }

            addr += PAGE_SIZE;
            paddr += PAGE_SIZE as u64;
        }

        true
    }

    /// Walk page tables for TTBR0 (user space)
    fn walk_page_tables_ttbr0(&mut self, vaddr: usize) -> *mut PageTableEntry {
        // For simplicity, we use a single-level table with 1MB block mappings
        // In a real kernel, you'd walk all 4 levels
        let idx = lowlevel_logic::pt_index(vaddr, PT_ENTRIES); // 2MB blocks

        if idx >= PT_ENTRIES {
            return core::ptr::null_mut();
        }

        unsafe { self.root_ttbr0.add(idx) }
    }

    /// Walk page tables for TTBR1 (kernel space)
    fn walk_page_tables_ttbr1(&mut self, vaddr: usize) -> *mut PageTableEntry {
        let idx = lowlevel_logic::pt_index(vaddr, PT_ENTRIES);

        if idx >= PT_ENTRIES {
            return core::ptr::null_mut();
        }

        unsafe { self.root_ttbr1.add(idx) }
    }

    /// Switch to this address space
    pub fn switch_to(&self) {
        unsafe {
            // Set TTBR0
            let ttbr0 = self.root_ttbr0 as u64;
            core::arch::asm!(
                "msr ttbr0_el1, {ttbr0}",
                ttbr0 = in(reg) ttbr0,
                options(nostack),
            );

            // Set TTBR1
            let ttbr1 = self.root_ttbr1 as u64;
            core::arch::asm!(
                "msr ttbr1_el1, {ttbr1}",
                ttbr1 = in(reg) ttbr1,
                options(nostack),
            );

            // TLB invalidate
            core::arch::asm!("tlbi vmalle1is", "dsb ish", "isb", options(nostack),);
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

/// Global page table manager for kernel
static mut KERNEL_PAGETABLE_MANAGER: Option<PageTableManager> = None;
static mut PAGE_TABLE_POOL: [PageTablePage; MAX_PAGE_TABLE_PAGES] =
    [const { PageTablePage::new() }; MAX_PAGE_TABLE_PAGES];
static mut NEXT_PAGE_TABLE_SLOT: usize = 0;

/// Initialize MMU subsystem
pub fn init() {
    unsafe {
        KERNEL_PAGETABLE_MANAGER = PageTableManager::new();
    }
    crate::kernel_lowlevel::serial::Serial::new().init();
    crate::kernel_lowlevel::serial::Serial::new()
        .write_str("[MMU] Page table manager initialized\n");
}

/// Get kernel page table manager
pub fn get_kernel_manager() -> Option<&'static mut PageTableManager> {
    unsafe { KERNEL_PAGETABLE_MANAGER.as_mut() }
}
