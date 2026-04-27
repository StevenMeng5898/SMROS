//! Virtual Memory Address Region (VMAR) Implementation
//!
//! VMARs manage virtual address space layout and mappings.

#![allow(dead_code)]

use super::types::*;
use alloc::vec::Vec;

fn page_aligned(addr: usize) -> bool {
    addr & (crate::kernel_lowlevel::memory::PAGE_SIZE - 1) == 0
}

/// Memory mapping in a VMAR
#[derive(Clone, Copy)]
pub struct VmarMapping {
    /// Virtual address
    pub vaddr: usize,
    /// Size
    pub size: usize,
    /// VMO reference
    pub vmo_handle: HandleValue,
    /// Offset in VMO
    pub vmo_offset: usize,
    /// MMU flags
    pub mmu_flags: MmuFlags,
    /// Whether mapping is valid
    pub valid: bool,
}

/// Virtual Memory Address Region
pub struct Vmar {
    /// VMAR handle
    pub handle: HandleValue,
    /// Base virtual address
    pub base_addr: usize,
    /// Size
    pub size: usize,
    /// Memory mappings
    pub mappings: Vec<VmarMapping>,
    /// Child VMARs
    pub children: Vec<usize>,
    /// Parent VMAR index
    pub parent_idx: Option<usize>,
    /// Rights
    pub rights: u32,
}

impl Vmar {
    /// Create a new VMAR
    pub fn new(base_addr: usize, size: usize) -> Self {
        Self {
            handle: HandleValue(INVALID_HANDLE),
            base_addr,
            size,
            mappings: Vec::new(),
            children: Vec::new(),
            parent_idx: None,
            rights: Rights::DefaultVmar as u32,
        }
    }

    /// Get VMAR address
    pub fn addr(&self) -> usize {
        self.base_addr
    }

    fn end_addr(&self) -> usize {
        self.base_addr + self.size
    }

    fn sort_mappings(&mut self) {
        self.mappings.sort_by_key(|mapping| mapping.vaddr);
    }

    fn contains_range(&self, vaddr: usize, len: usize) -> bool {
        let Some(end) = vaddr.checked_add(len) else {
            return false;
        };

        vaddr >= self.base_addr && end <= self.end_addr()
    }

    fn ranges_overlap(start_a: usize, len_a: usize, start_b: usize, len_b: usize) -> bool {
        let end_a = start_a + len_a;
        let end_b = start_b + len_b;
        start_a < end_b && start_b < end_a
    }

    fn range_available(&self, vaddr: usize, len: usize) -> bool {
        if !self.contains_range(vaddr, len) {
            return false;
        }

        !self.mappings.iter().any(|mapping| {
            mapping.valid && Self::ranges_overlap(vaddr, len, mapping.vaddr, mapping.size)
        })
    }

    fn align_up(addr: usize, align: usize) -> usize {
        (addr + align - 1) & !(align - 1)
    }

    /// Map a VMO into this VMAR
    pub fn map(
        &mut self,
        vmar_offset: Option<usize>,
        vmo_handle: HandleValue,
        vmo_offset: usize,
        len: usize,
        mmu_flags: MmuFlags,
    ) -> ZxResult<usize> {
        if len == 0 || !page_aligned(vmo_offset) {
            return Err(ZxError::ErrInvalidArgs);
        }

        let len = roundup_pages(len);
        let vaddr = match vmar_offset {
            Some(offset) => {
                if !page_aligned(offset) {
                    return Err(ZxError::ErrInvalidArgs);
                }
                self.base_addr + offset
            }
            None => self.find_free_region(len)?,
        };

        if !self.range_available(vaddr, len) {
            return Err(ZxError::ErrAlreadyExists);
        }

        let mapping = VmarMapping {
            vaddr,
            size: len,
            vmo_handle,
            vmo_offset,
            mmu_flags,
            valid: true,
        };

        self.mappings.push(mapping);
        self.sort_mappings();
        Ok(vaddr)
    }

    /// Extended map with more options
    #[allow(clippy::too_many_arguments)]
    pub fn map_ext(
        &mut self,
        vmar_offset: Option<usize>,
        vmo_handle: HandleValue,
        vmo_offset: usize,
        len: usize,
        _permissions: MmuFlags,
        mapping_flags: MmuFlags,
        overwrite: bool,
        _map_range: bool,
    ) -> ZxResult<usize> {
        if overwrite {
            let offset = vmar_offset.ok_or(ZxError::ErrInvalidArgs)?;
            match self.unmap(self.base_addr + offset, roundup_pages(len)) {
                Ok(()) | Err(ZxError::ErrNotFound) => {}
                Err(err) => return Err(err),
            }
        }

        self.map(vmar_offset, vmo_handle, vmo_offset, len, mapping_flags)
    }

    /// Unmap a region
    pub fn unmap(&mut self, vaddr: usize, len: usize) -> ZxResult {
        if len == 0 || !page_aligned(vaddr) || !page_aligned(len) {
            return Err(ZxError::ErrInvalidArgs);
        }
        if !self.contains_range(vaddr, len) {
            return Err(ZxError::ErrOutOfRange);
        }

        let end = vaddr + len;
        let mut removed_any = false;
        let mappings = core::mem::take(&mut self.mappings);

        for mapping in mappings {
            if !mapping.valid || !Self::ranges_overlap(vaddr, len, mapping.vaddr, mapping.size) {
                self.mappings.push(mapping);
                continue;
            }

            removed_any = true;
            let mapping_end = mapping.vaddr + mapping.size;

            if vaddr > mapping.vaddr {
                self.mappings.push(VmarMapping {
                    vaddr: mapping.vaddr,
                    size: vaddr - mapping.vaddr,
                    ..mapping
                });
            }

            if end < mapping_end {
                self.mappings.push(VmarMapping {
                    vaddr: end,
                    size: mapping_end - end,
                    vmo_offset: mapping.vmo_offset + (end - mapping.vaddr),
                    ..mapping
                });
            }
        }

        self.sort_mappings();

        if !removed_any {
            return Err(ZxError::ErrNotFound);
        }

        Ok(())
    }

    /// Unmap and handle thread exit (special Zircon syscall)
    pub fn unmap_handle_close_thread_exit(&mut self, vaddr: usize, len: usize) -> ZxResult {
        self.unmap(vaddr, len)
    }

    /// Protect a region (change permissions)
    pub fn protect(&mut self, vaddr: usize, len: usize, new_flags: MmuFlags) -> ZxResult {
        if len == 0 || !page_aligned(vaddr) || !page_aligned(len) {
            return Err(ZxError::ErrInvalidArgs);
        }
        if !self.contains_range(vaddr, len) {
            return Err(ZxError::ErrOutOfRange);
        }

        let end = vaddr + len;
        let mut updated_any = false;
        let mappings = core::mem::take(&mut self.mappings);

        for mapping in mappings {
            if !mapping.valid || !Self::ranges_overlap(vaddr, len, mapping.vaddr, mapping.size) {
                self.mappings.push(mapping);
                continue;
            }

            updated_any = true;
            let mapping_end = mapping.vaddr + mapping.size;

            if vaddr > mapping.vaddr {
                self.mappings.push(VmarMapping {
                    vaddr: mapping.vaddr,
                    size: vaddr - mapping.vaddr,
                    ..mapping
                });
            }

            let protected_start = core::cmp::max(vaddr, mapping.vaddr);
            let protected_end = core::cmp::min(end, mapping_end);
            self.mappings.push(VmarMapping {
                vaddr: protected_start,
                size: protected_end - protected_start,
                vmo_offset: mapping.vmo_offset + (protected_start - mapping.vaddr),
                mmu_flags: new_flags,
                ..mapping
            });

            if protected_end < mapping_end {
                self.mappings.push(VmarMapping {
                    vaddr: protected_end,
                    size: mapping_end - protected_end,
                    vmo_offset: mapping.vmo_offset + (protected_end - mapping.vaddr),
                    ..mapping
                });
            }
        }

        self.sort_mappings();

        if !updated_any {
            return Err(ZxError::ErrNotFound);
        }

        Ok(())
    }

    /// Destroy this VMAR and all subregions
    pub fn destroy(&mut self) -> ZxResult {
        self.mappings.clear();
        self.children.clear();
        Ok(())
    }

    /// Allocate a subregion
    pub fn allocate(
        &mut self,
        offset: Option<usize>,
        size: usize,
        _flags: VmarFlags,
        align: usize,
    ) -> ZxResult<usize> {
        if size == 0 {
            return Err(ZxError::ErrInvalidArgs);
        }

        let size = roundup_pages(size);
        let align = if align == 0 {
            crate::kernel_lowlevel::memory::PAGE_SIZE
        } else {
            align
        };

        if !align.is_power_of_two() || !page_aligned(align) {
            return Err(ZxError::ErrInvalidArgs);
        }

        let vaddr = match offset {
            Some(off) => {
                if !page_aligned(off) {
                    return Err(ZxError::ErrInvalidArgs);
                }
                self.base_addr + off
            }
            None => self.find_free_region_aligned(size, align)?,
        };

        if !self.range_available(vaddr, size) {
            return Err(ZxError::ErrAlreadyExists);
        }

        Ok(vaddr)
    }

    /// Find a free region
    fn find_free_region(&mut self, size: usize) -> ZxResult<usize> {
        self.sort_mappings();
        let mut candidate = self.base_addr;

        for mapping in &self.mappings {
            if !mapping.valid {
                continue;
            }

            if candidate + size <= mapping.vaddr {
                return Ok(candidate);
            }

            candidate = mapping.vaddr + mapping.size;
        }

        if candidate + size <= self.base_addr + self.size {
            return Ok(candidate);
        }

        Err(ZxError::ErrNoMemory)
    }

    /// Find a free region with alignment
    fn find_free_region_aligned(&mut self, size: usize, align: usize) -> ZxResult<usize> {
        self.sort_mappings();
        let mut candidate = Self::align_up(self.base_addr, align);

        for mapping in &self.mappings {
            if !mapping.valid {
                continue;
            }

            if candidate + size <= mapping.vaddr {
                return Ok(candidate);
            }

            candidate = Self::align_up(mapping.vaddr + mapping.size, align);
        }

        if candidate + size <= self.base_addr + self.size {
            return Ok(candidate);
        }

        Err(ZxError::ErrNoMemory)
    }
}
