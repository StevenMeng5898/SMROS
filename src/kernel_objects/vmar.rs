//! Virtual Memory Address Region (VMAR) Implementation
//!
//! VMARs manage virtual address space layout and mappings.

#![allow(dead_code)]

use alloc::vec::Vec;
use alloc::sync::Arc;
use super::types::*;
use super::vmo::Vmo;

/// Memory mapping in a VMAR
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

    /// Map a VMO into this VMAR
    pub fn map(
        &mut self,
        vmar_offset: Option<usize>,
        _vmo: Arc<Vmo>,
        vmo_offset: usize,
        len: usize,
        mmu_flags: MmuFlags,
    ) -> ZxResult<usize> {
        let vaddr = match vmar_offset {
            Some(offset) => self.base_addr + offset,
            None => self.find_free_region(len)?,
        };

        let mapping = VmarMapping {
            vaddr,
            size: len,
            vmo_handle: HandleValue(0),
            vmo_offset,
            mmu_flags,
            valid: true,
        };

        self.mappings.push(mapping);
        Ok(vaddr)
    }

    /// Extended map with more options
    #[allow(clippy::too_many_arguments)]
    pub fn map_ext(
        &mut self,
        vmar_offset: Option<usize>,
        vmo: Arc<Vmo>,
        vmo_offset: usize,
        len: usize,
        _permissions: MmuFlags,
        mapping_flags: MmuFlags,
        overwrite: bool,
        _map_range: bool,
    ) -> ZxResult<usize> {
        if overwrite {
            if let Some(offset) = vmar_offset {
                self.unmap(self.base_addr + offset, len)?;
            }
        }

        self.map(vmar_offset, vmo, vmo_offset, len, mapping_flags)
    }

    /// Unmap a region
    pub fn unmap(&mut self, vaddr: usize, _len: usize) -> ZxResult {
        self.mappings.retain(|m| {
            !(m.valid && vaddr >= m.vaddr && vaddr < m.vaddr + m.size)
        });
        Ok(())
    }

    /// Unmap and handle thread exit (special Zircon syscall)
    pub fn unmap_handle_close_thread_exit(&mut self, vaddr: usize, _len: usize) -> ZxResult {
        self.mappings.retain(|m| {
            !(m.valid && vaddr >= m.vaddr && vaddr < m.vaddr + m.size)
        });
        Ok(())
    }

    /// Protect a region (change permissions)
    pub fn protect(&mut self, vaddr: usize, _len: usize, new_flags: MmuFlags) -> ZxResult {
        for mapping in &mut self.mappings {
            if mapping.valid && vaddr >= mapping.vaddr && vaddr < mapping.vaddr + mapping.size {
                mapping.mmu_flags = new_flags;
            }
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
        let vaddr = match offset {
            Some(off) => self.base_addr + off,
            None => self.find_free_region_aligned(size, align)?,
        };

        Ok(vaddr)
    }

    /// Find a free region
    fn find_free_region(&self, size: usize) -> ZxResult<usize> {
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
    fn find_free_region_aligned(&self, size: usize, align: usize) -> ZxResult<usize> {
        let mut candidate = (self.base_addr + align - 1) & !(align - 1);

        for mapping in &self.mappings {
            if !mapping.valid {
                continue;
            }

            if candidate + size <= mapping.vaddr {
                return Ok(candidate);
            }

            candidate = ((mapping.vaddr + mapping.size + align - 1) / align) * align;
        }

        if candidate + size <= self.base_addr + self.size {
            return Ok(candidate);
        }

        Err(ZxError::ErrNoMemory)
    }
}
