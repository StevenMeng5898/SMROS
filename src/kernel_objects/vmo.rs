//! Virtual Memory Object (VMO) Implementation
//!
//! VMOs are the basic building blocks for memory management in Zircon.

#![allow(dead_code)]

use core::sync::atomic::AtomicU32;
use alloc::vec;
use alloc::vec::Vec;
use super::types::*;
use crate::kernel_lowlevel::memory::PageFrameAllocator;

/// Virtual Memory Object
pub struct Vmo {
    /// VMO handle
    pub handle: HandleValue,
    /// VMO type
    pub vmo_type: VmoType,
    /// Size in bytes
    pub size: usize,
    /// Number of pages
    pub page_count: usize,
    /// Physical page frame numbers for each page. `None` means decommitted.
    pub pfns: Option<Vec<Option<u64>>>,
    /// Software copy of the VMO contents used by read/write tests.
    pub data: Vec<u8>,
    /// Cache policy
    pub cache_policy: CachePolicy,
    /// Reference count
    pub ref_count: AtomicU32,
    /// Whether VMO is resizable
    pub resizable: bool,
}

impl Vmo {
    fn alloc_paged_pfns(page_count: usize) -> Option<Vec<Option<u64>>> {
        let mut pfns = Vec::with_capacity(page_count);

        for _ in 0..page_count {
            if let Some(pfn) = PageFrameAllocator::alloc() {
                pfns.push(Some(pfn));
            } else {
                for pfn in pfns.into_iter().flatten() {
                    PageFrameAllocator::free(pfn);
                }
                return None;
            }
        }

        Some(pfns)
    }

    fn alloc_physical_pfns(paddr: u64, page_count: usize) -> Vec<Option<u64>> {
        let mut pfns = Vec::with_capacity(page_count);

        for i in 0..page_count {
            pfns.push(Some((paddr >> 12) + i as u64));
        }

        pfns
    }

    fn checked_end(&self, offset: usize, len: usize) -> ZxResult<usize> {
        offset
            .checked_add(len)
            .filter(|end| *end <= self.size)
            .ok_or(ZxError::ErrOutOfRange)
    }

    fn ensure_range_committed(&mut self, offset: usize, len: usize) -> ZxResult {
        if len == 0 {
            return Ok(());
        }

        self.checked_end(offset, len)?;
        let start_page = offset / crate::kernel_lowlevel::memory::PAGE_SIZE;
        let end_page = (offset + len + crate::kernel_lowlevel::memory::PAGE_SIZE - 1)
            / crate::kernel_lowlevel::memory::PAGE_SIZE;

        if let Some(ref mut pfns) = self.pfns {
            for page in start_page..end_page {
                if page >= pfns.len() {
                    return Err(ZxError::ErrOutOfRange);
                }

                if pfns[page].is_none() {
                    pfns[page] = Some(PageFrameAllocator::alloc().ok_or(ZxError::ErrNoMemory)?);
                }
            }
        }

        Ok(())
    }

    /// Create a new paged VMO
    pub fn new_paged(page_count: usize) -> Option<Self> {
        let pfns = Self::alloc_paged_pfns(page_count)?;
        let size = page_count * crate::kernel_lowlevel::memory::PAGE_SIZE;

        Some(Self {
            handle: HandleValue(INVALID_HANDLE),
            vmo_type: VmoType::Paged,
            size,
            page_count,
            pfns: Some(pfns),
            data: vec![0; size],
            cache_policy: CachePolicy::Cached,
            ref_count: AtomicU32::new(1),
            resizable: false,
        })
    }

    /// Create a resizable VMO
    pub fn new_paged_with_resizable(resizable: bool, page_count: usize) -> Option<Self> {
        let mut vmo = Self::new_paged(page_count)?;
        vmo.resizable = resizable;
        if resizable {
            vmo.vmo_type = VmoType::Resizable;
        }
        Some(vmo)
    }

    /// Create a physical VMO (backed by specific physical memory)
    pub fn new_physical(paddr: u64, size: usize) -> Option<Self> {
        let page_count = pages(size);
        let rounded_size = page_count * crate::kernel_lowlevel::memory::PAGE_SIZE;
        let pfns = Self::alloc_physical_pfns(paddr, page_count);

        Some(Self {
            handle: HandleValue(INVALID_HANDLE),
            vmo_type: VmoType::Physical,
            size: rounded_size,
            page_count,
            pfns: Some(pfns),
            data: vec![0; rounded_size],
            cache_policy: CachePolicy::Cached,
            ref_count: AtomicU32::new(1),
            resizable: false,
        })
    }

    /// Create a contiguous VMO (physically contiguous memory)
    pub fn new_contiguous(size: usize) -> Option<Self> {
        let page_count = pages(size);
        let rounded_size = page_count * crate::kernel_lowlevel::memory::PAGE_SIZE;
        let pfns = Self::alloc_paged_pfns(page_count)?;

        Some(Self {
            handle: HandleValue(INVALID_HANDLE),
            vmo_type: VmoType::Contiguous,
            size: rounded_size,
            page_count,
            pfns: Some(pfns),
            data: vec![0; rounded_size],
            cache_policy: CachePolicy::Cached,
            ref_count: AtomicU32::new(1),
            resizable: false,
        })
    }

    /// Get physical addresses for this VMO
    pub fn get_physical_addresses(&self) -> Option<Vec<u64>> {
        self.pfns.as_ref().map(|pfns| {
            pfns.iter().filter_map(|pfn| pfn.map(|val| val << 12)).collect()
        })
    }

    /// Get VMO type
    pub fn get_type(&self) -> VmoType {
        self.vmo_type
    }

    /// Get VMO size
    pub fn len(&self) -> usize {
        self.size
    }

    /// Get the number of committed pages.
    pub fn committed_pages(&self) -> usize {
        self.pfns
            .as_ref()
            .map(|pfns| pfns.iter().filter(|pfn| pfn.is_some()).count())
            .unwrap_or(0)
    }

    /// Check if VMO is empty
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Check if VMO is resizable
    pub fn is_resizable(&self) -> bool {
        self.resizable
    }

    /// Set VMO size (for resizable VMOs)
    pub fn set_len(&mut self, new_size: usize) -> ZxResult {
        if !self.resizable {
            return Err(ZxError::ErrNotSupported);
        }

        let new_page_count = pages(new_size);
        let rounded_size = new_page_count * crate::kernel_lowlevel::memory::PAGE_SIZE;

        if new_page_count > self.page_count {
            let additional = new_page_count - self.page_count;
            if let Some(ref mut pfns) = self.pfns {
                for _ in 0..additional {
                    if let Some(pfn) = PageFrameAllocator::alloc() {
                        pfns.push(Some(pfn));
                    } else {
                        return Err(ZxError::ErrNoMemory);
                    }
                }
            }
        } else if new_page_count < self.page_count {
            let remove = self.page_count - new_page_count;
            if let Some(ref mut pfns) = self.pfns {
                for _ in 0..remove {
                    if let Some(Some(pfn)) = pfns.pop() {
                        PageFrameAllocator::free(pfn);
                    }
                }
            }
        }

        self.page_count = new_page_count;
        self.size = rounded_size;
        self.data.resize(rounded_size, 0);
        Ok(())
    }

    /// Read from VMO
    pub fn read(&self, offset: usize, buf: &mut [u8]) -> ZxResult {
        let end = self.checked_end(offset, buf.len())?;
        buf.copy_from_slice(&self.data[offset..end]);

        Ok(())
    }

    /// Write to VMO
    pub fn write(&mut self, offset: usize, buf: &[u8]) -> ZxResult {
        let end = self.checked_end(offset, buf.len())?;
        self.ensure_range_committed(offset, buf.len())?;
        self.data[offset..end].copy_from_slice(buf);

        Ok(())
    }

    /// Commit pages for a range
    pub fn commit(&mut self, offset: usize, len: usize) -> ZxResult {
        self.ensure_range_committed(offset, len)?;
        Ok(())
    }

    /// Decommit pages for a range
    pub fn decommit(&mut self, offset: usize, len: usize) -> ZxResult {
        self.checked_end(offset, len)?;
        let start_page = offset / crate::kernel_lowlevel::memory::PAGE_SIZE;
        let end_page = (offset + len) / crate::kernel_lowlevel::memory::PAGE_SIZE;

        if let Some(ref mut pfns) = self.pfns {
            for i in start_page..end_page.min(pfns.len()) {
                if let Some(pfn) = pfns[i].take() {
                    PageFrameAllocator::free(pfn);
                }
            }
        }

        for byte in &mut self.data[offset..offset + len] {
            *byte = 0;
        }

        Ok(())
    }

    /// Zero a range
    pub fn zero(&mut self, offset: usize, len: usize) -> ZxResult {
        self.checked_end(offset, len)?;

        for byte in &mut self.data[offset..offset + len] {
            *byte = 0;
        }

        Ok(())
    }

    /// Release all committed page frames held by this VMO.
    pub fn release_pages(&mut self) {
        if let Some(ref mut pfns) = self.pfns {
            for pfn in pfns.iter_mut() {
                if let Some(frame) = pfn.take() {
                    PageFrameAllocator::free(frame);
                }
            }
        }
    }

    /// Create a child VMO
    pub fn create_child(&self, resizable: bool, _offset: usize, size: usize) -> ZxResult<Self> {
        let page_count = (size + crate::kernel_lowlevel::memory::PAGE_SIZE - 1) / crate::kernel_lowlevel::memory::PAGE_SIZE;
        if let Some(mut child) = Self::new_paged(page_count) {
            child.resizable = resizable;
            if resizable {
                child.vmo_type = VmoType::Resizable;
            }
            Ok(child)
        } else {
            Err(ZxError::ErrNoMemory)
        }
    }

    /// Create a slice of this VMO
    pub fn create_slice(&self, _offset: usize, size: usize) -> ZxResult<Self> {
        let page_count = (size + crate::kernel_lowlevel::memory::PAGE_SIZE - 1) / crate::kernel_lowlevel::memory::PAGE_SIZE;
        if let Some(mut child) = Self::new_paged(page_count) {
            child.vmo_type = VmoType::Paged;
            Ok(child)
        } else {
            Err(ZxError::ErrNoMemory)
        }
    }

    /// Set cache policy
    pub fn set_cache_policy(&mut self, policy: CachePolicy) -> ZxResult {
        self.cache_policy = policy;
        Ok(())
    }
}
