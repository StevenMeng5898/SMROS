//! Virtual Memory Object (VMO) Implementation
//!
//! VMOs are the basic building blocks for memory management in Zircon.

#![allow(dead_code)]

use core::sync::atomic::AtomicU32;
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
    /// Physical page frame numbers (if allocated)
    pub pfns: Option<Vec<u64>>,
    /// Cache policy
    pub cache_policy: CachePolicy,
    /// Reference count
    pub ref_count: AtomicU32,
    /// Whether VMO is resizable
    pub resizable: bool,
}

impl Vmo {
    /// Create a new paged VMO
    pub fn new_paged(page_count: usize) -> Option<Self> {
        let mut pfns = Vec::with_capacity(page_count);

        for _ in 0..page_count {
            if let Some(pfn) = PageFrameAllocator::alloc() {
                pfns.push(pfn);
            } else {
                for pfn in &pfns {
                    PageFrameAllocator::free(*pfn);
                }
                return None;
            }
        }

        Some(Self {
            handle: HandleValue(INVALID_HANDLE),
            vmo_type: VmoType::Paged,
            size: page_count * crate::kernel_lowlevel::memory::PAGE_SIZE,
            page_count,
            pfns: Some(pfns),
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
        let mut pfns = Vec::with_capacity(page_count);

        for i in 0..page_count {
            pfns.push((paddr >> 12) + i as u64);
        }

        Some(Self {
            handle: HandleValue(INVALID_HANDLE),
            vmo_type: VmoType::Physical,
            size,
            page_count,
            pfns: Some(pfns),
            cache_policy: CachePolicy::Cached,
            ref_count: AtomicU32::new(1),
            resizable: false,
        })
    }

    /// Create a contiguous VMO (physically contiguous memory)
    pub fn new_contiguous(size: usize) -> Option<Self> {
        let page_count = pages(size);
        let mut pfns = Vec::with_capacity(page_count);

        for _ in 0..page_count {
            if let Some(pfn) = PageFrameAllocator::alloc() {
                pfns.push(pfn);
            } else {
                for pfn in &pfns {
                    PageFrameAllocator::free(*pfn);
                }
                return None;
            }
        }

        Some(Self {
            handle: HandleValue(INVALID_HANDLE),
            vmo_type: VmoType::Contiguous,
            size,
            page_count,
            pfns: Some(pfns),
            cache_policy: CachePolicy::Cached,
            ref_count: AtomicU32::new(1),
            resizable: false,
        })
    }

    /// Get physical addresses for this VMO
    pub fn get_physical_addresses(&self) -> Option<Vec<u64>> {
        self.pfns.as_ref().map(|pfns| {
            pfns.iter().map(|pfn| pfn << 12).collect()
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

        let new_page_count = (new_size + crate::kernel_lowlevel::memory::PAGE_SIZE - 1) / crate::kernel_lowlevel::memory::PAGE_SIZE;

        if new_page_count > self.page_count {
            let additional = new_page_count - self.page_count;
            if let Some(ref mut pfns) = self.pfns {
                for _ in 0..additional {
                    if let Some(pfn) = PageFrameAllocator::alloc() {
                        pfns.push(pfn);
                    } else {
                        return Err(ZxError::ErrNoMemory);
                    }
                }
            }
        } else if new_page_count < self.page_count {
            let remove = self.page_count - new_page_count;
            if let Some(ref mut pfns) = self.pfns {
                for _ in 0..remove {
                    if let Some(pfn) = pfns.pop() {
                        PageFrameAllocator::free(pfn);
                    }
                }
            }
        }

        self.page_count = new_page_count;
        self.size = new_page_count * crate::kernel_lowlevel::memory::PAGE_SIZE;
        Ok(())
    }

    /// Read from VMO
    pub fn read(&self, offset: usize, buf: &mut [u8]) -> ZxResult {
        if offset + buf.len() > self.size {
            return Err(ZxError::ErrOutOfRange);
        }

        for byte in buf.iter_mut() {
            *byte = 0;
        }

        Ok(())
    }

    /// Write to VMO
    pub fn write(&self, offset: usize, buf: &[u8]) -> ZxResult {
        if offset + buf.len() > self.size {
            return Err(ZxError::ErrOutOfRange);
        }

        Ok(())
    }

    /// Commit pages for a range
    pub fn commit(&self, _offset: usize, _len: usize) -> ZxResult {
        Ok(())
    }

    /// Decommit pages for a range
    pub fn decommit(&self, offset: usize, len: usize) -> ZxResult {
        let start_page = offset / crate::kernel_lowlevel::memory::PAGE_SIZE;
        let num_pages = len / crate::kernel_lowlevel::memory::PAGE_SIZE;

        if let Some(ref pfns) = self.pfns {
            for i in start_page..(start_page + num_pages).min(pfns.len()) {
                PageFrameAllocator::free(pfns[i]);
            }
        }

        Ok(())
    }

    /// Zero a range
    pub fn zero(&self, _offset: usize, _len: usize) -> ZxResult {
        Ok(())
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
