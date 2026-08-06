use alloc::vec::Vec;

use crate::kernel_lowlevel::aarch64_vm_logic_shared::{
    aarch64_supervisor_block_descriptor, aarch64_table_descriptor, aarch64_table_indices,
    aarch64_user_page_descriptor, aarch64_user_range_valid, AARCH64_DESC_ADDR_MASK,
    AARCH64_DESC_AP_READ_ONLY, AARCH64_DESC_AP_USER, AARCH64_DESC_TABLE_OR_PAGE,
    AARCH64_DESC_VALID, AARCH64_PAGE_SIZE, AARCH64_TABLE_ENTRIES,
};
use crate::kernel_lowlevel::drivers;
use crate::kernel_lowlevel::memory::PageFrameAllocator;
use crate::kernel_lowlevel::mmu::AddressSpaceError;

const AARCH64_L2_BLOCK_SIZE: usize = 1 << 21;

pub struct Aarch64AddressSpace {
    root_pfn: u64,
    table_pfns: Vec<u64>,
}

impl Aarch64AddressSpace {
    pub fn new_with_kernel_map() -> Result<Self, AddressSpaceError> {
        let root_pfn = Self::allocate_table()?;
        let mut address_space = Self {
            root_pfn,
            table_pfns: alloc::vec![root_pfn],
        };

        let memory = drivers::memory_reg().ok_or(AddressSpaceError::InvalidAddress)?;
        address_space.map_supervisor_range(memory.base, memory.size, false, true)?;
        address_space.map_supervisor_range(
            drivers::uart_base(),
            drivers::uart_size(),
            true,
            false,
        )?;
        address_space.map_supervisor_range(
            drivers::gicd_base(),
            drivers::gicd_size(),
            true,
            false,
        )?;
        address_space.map_supervisor_range(
            drivers::gicr_base(),
            drivers::gicr_size(),
            true,
            false,
        )?;
        let mut index = 0;
        while let Some(reg) = drivers::virtio_mmio_reg(index) {
            address_space.map_supervisor_range(reg.base, reg.size, true, false)?;
            index += 1;
        }
        Ok(address_space)
    }

    pub fn root_paddr(&self) -> u64 {
        self.root_pfn * AARCH64_PAGE_SIZE as u64
    }

    pub fn map_user_page(
        &mut self,
        vaddr: usize,
        pfn: u64,
        readable: bool,
        writable: bool,
        executable: bool,
    ) -> Result<(), AddressSpaceError> {
        Self::validate_page_operation(vaddr, readable, writable)?;
        PageFrameAllocator::pfn_address(pfn).ok_or(AddressSpaceError::InvalidAddress)?;
        let indices = aarch64_table_indices(vaddr).ok_or(AddressSpaceError::InvalidAddress)?;
        self.ensure_leaf_table(indices).and_then(|table_pfn| {
            let entry = unsafe { &mut Self::table_mut(table_pfn)?[indices[2]] };
            if *entry & AARCH64_DESC_VALID != 0 {
                return Err(AddressSpaceError::AlreadyMapped);
            }
            *entry = aarch64_user_page_descriptor(
                usize::try_from(
                    pfn.checked_mul(AARCH64_PAGE_SIZE as u64)
                        .ok_or(AddressSpaceError::InvalidAddress)?,
                )
                .map_err(|_| AddressSpaceError::InvalidAddress)?,
                readable,
                writable,
                executable,
            );
            Ok(())
        })
    }

    pub fn protect_user_page(
        &mut self,
        vaddr: usize,
        readable: bool,
        writable: bool,
        executable: bool,
    ) -> Result<(), AddressSpaceError> {
        Self::validate_page_operation(vaddr, readable, writable)?;
        let entry = self.leaf_entry_mut(vaddr)?;
        let descriptor = *entry;
        if descriptor & AARCH64_DESC_VALID == 0 {
            return Err(AddressSpaceError::NotMapped);
        }
        *entry = aarch64_user_page_descriptor(
            (descriptor & AARCH64_DESC_ADDR_MASK) as usize,
            readable,
            writable,
            executable,
        );
        Ok(())
    }

    pub fn unmap_user_page(&mut self, vaddr: usize) -> Result<u64, AddressSpaceError> {
        if !aarch64_user_range_valid(vaddr, AARCH64_PAGE_SIZE) {
            return Err(AddressSpaceError::InvalidAddress);
        }
        let entry = self.leaf_entry_mut(vaddr)?;
        let descriptor = core::mem::take(entry);
        if descriptor & AARCH64_DESC_VALID == 0 {
            return Err(AddressSpaceError::NotMapped);
        }
        Ok((descriptor & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64)
    }

    pub fn translate_user(&self, vaddr: usize, write: bool) -> Option<usize> {
        if vaddr < crate::kernel_lowlevel::aarch64_vm_logic_shared::AARCH64_USER_BASE
            || vaddr >= crate::kernel_lowlevel::aarch64_vm_logic_shared::AARCH64_USER_LIMIT
        {
            return None;
        }
        let indices = aarch64_table_indices(vaddr)?;
        let mut table_pfn = self.root_pfn;
        for index in indices[..2].iter().copied() {
            let descriptor = unsafe { Self::table(table_pfn).ok()?[index] };
            if descriptor & (AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE)
                != AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE
            {
                return None;
            }
            table_pfn = (descriptor & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64;
        }
        let descriptor = unsafe { Self::table(table_pfn).ok()?[indices[2]] };
        if descriptor & AARCH64_DESC_VALID == 0
            || descriptor & AARCH64_DESC_AP_USER == 0
            || (write && descriptor & AARCH64_DESC_AP_READ_ONLY != 0)
        {
            return None;
        }
        Some((descriptor & AARCH64_DESC_ADDR_MASK) as usize | (vaddr & (AARCH64_PAGE_SIZE - 1)))
    }

    pub fn copy_to_user(&self, vaddr: usize, bytes: &[u8]) -> Result<(), AddressSpaceError> {
        self.copy_user(vaddr, bytes.len(), true, |physical, offset, count| unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr().add(offset), physical as *mut u8, count);
        })
    }

    pub fn copy_from_user(&self, vaddr: usize, out: &mut [u8]) -> Result<(), AddressSpaceError> {
        self.copy_user(vaddr, out.len(), false, |physical, offset, count| unsafe {
            core::ptr::copy_nonoverlapping(
                physical as *const u8,
                out.as_mut_ptr().add(offset),
                count,
            );
        })
    }

    pub(crate) fn map_supervisor_range(
        &mut self,
        start: usize,
        len: usize,
        device: bool,
        executable: bool,
    ) -> Result<(), AddressSpaceError> {
        let requested_end = start
            .checked_add(len)
            .filter(|_| len != 0)
            .ok_or(AddressSpaceError::InvalidAddress)?;
        let block_start = start & !(AARCH64_L2_BLOCK_SIZE - 1);
        let block_end = requested_end
            .checked_add(AARCH64_L2_BLOCK_SIZE - 1)
            .map(|end| end & !(AARCH64_L2_BLOCK_SIZE - 1))
            .ok_or(AddressSpaceError::InvalidAddress)?;
        if block_end > (1usize << 39) {
            return Err(AddressSpaceError::InvalidAddress);
        }

        let mut current = block_start;
        while current < block_end {
            let indices = aarch64_table_indices(current).ok_or(AddressSpaceError::InvalidAddress)?;
            let table_pfn = self.ensure_level_two_table(indices[0])?;
            let descriptor = aarch64_supervisor_block_descriptor(current, device, executable);
            let entry = unsafe { &mut Self::table_mut(table_pfn)?[indices[1]] };
            if *entry == 0 {
                *entry = descriptor;
            } else if *entry != descriptor {
                return Err(AddressSpaceError::AlreadyMapped);
            }
            current += AARCH64_L2_BLOCK_SIZE;
        }
        Ok(())
    }

    fn copy_user(
        &self,
        vaddr: usize,
        len: usize,
        write: bool,
        mut copy: impl FnMut(usize, usize, usize),
    ) -> Result<(), AddressSpaceError> {
        let end = vaddr
            .checked_add(len)
            .ok_or(AddressSpaceError::InvalidAddress)?;
        let mut current = vaddr;
        while current < end {
            self.translate_user(current, write)
                .ok_or(AddressSpaceError::PermissionDenied)?;
            let count = core::cmp::min(
                AARCH64_PAGE_SIZE - (current & (AARCH64_PAGE_SIZE - 1)),
                end - current,
            );
            current += count;
        }

        current = vaddr;
        let mut offset = 0;
        while current < end {
            let physical = self
                .translate_user(current, write)
                .ok_or(AddressSpaceError::PermissionDenied)?;
            let count = core::cmp::min(
                AARCH64_PAGE_SIZE - (current & (AARCH64_PAGE_SIZE - 1)),
                end - current,
            );
            copy(physical, offset, count);
            current += count;
            offset += count;
        }
        Ok(())
    }

    fn validate_page_operation(
        vaddr: usize,
        _readable: bool,
        _writable: bool,
    ) -> Result<(), AddressSpaceError> {
        if !aarch64_user_range_valid(vaddr, AARCH64_PAGE_SIZE) {
            return Err(AddressSpaceError::InvalidAddress);
        }
        Ok(())
    }

    fn allocate_table() -> Result<u64, AddressSpaceError> {
        let pfn = PageFrameAllocator::alloc().ok_or(AddressSpaceError::OutOfMemory)?;
        let address = match PageFrameAllocator::pfn_address(pfn) {
            Some(address) => address,
            None => {
                PageFrameAllocator::free(pfn);
                return Err(AddressSpaceError::InvalidAddress);
            }
        };
        unsafe { core::ptr::write_bytes(address as *mut u64, 0, AARCH64_TABLE_ENTRIES) };
        Ok(pfn)
    }

    fn ensure_leaf_table(&mut self, indices: [usize; 3]) -> Result<u64, AddressSpaceError> {
        let mut table_pfn = self.root_pfn;
        let mut created = Vec::new();
        for index in indices[..2].iter().copied() {
            let descriptor = unsafe { Self::table(table_pfn)?[index] };
            if descriptor == 0 {
                let child_pfn = match Self::allocate_table() {
                    Ok(child_pfn) => child_pfn,
                    Err(error) => {
                        self.rollback_created_tables(&created);
                        return Err(error);
                    }
                };
                self.table_pfns.push(child_pfn);
                unsafe {
                    Self::table_mut(table_pfn)?[index] = aarch64_table_descriptor(
                        PageFrameAllocator::pfn_address(child_pfn)
                            .ok_or(AddressSpaceError::InvalidAddress)?,
                    );
                }
                created.push((table_pfn, index, child_pfn));
                table_pfn = child_pfn;
            } else if descriptor & (AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE)
                == AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE
            {
                table_pfn = (descriptor & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64;
            } else {
                self.rollback_created_tables(&created);
                return Err(AddressSpaceError::AlreadyMapped);
            }
        }
        Ok(table_pfn)
    }

    fn ensure_level_two_table(&mut self, index: usize) -> Result<u64, AddressSpaceError> {
        let descriptor = unsafe { Self::table(self.root_pfn)?[index] };
        if descriptor == 0 {
            let child_pfn = Self::allocate_table()?;
            let child_paddr = match PageFrameAllocator::pfn_address(child_pfn) {
                Some(address) => address,
                None => {
                    PageFrameAllocator::free(child_pfn);
                    return Err(AddressSpaceError::InvalidAddress);
                }
            };
            unsafe {
                Self::table_mut(self.root_pfn)?[index] =
                    aarch64_table_descriptor(child_paddr);
            }
            self.table_pfns.push(child_pfn);
            Ok(child_pfn)
        } else if descriptor & (AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE)
            == AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE
        {
            Ok((descriptor & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64)
        } else {
            Err(AddressSpaceError::AlreadyMapped)
        }
    }

    fn leaf_entry_mut(&mut self, vaddr: usize) -> Result<&mut u64, AddressSpaceError> {
        let indices = aarch64_table_indices(vaddr).ok_or(AddressSpaceError::InvalidAddress)?;
        let mut table_pfn = self.root_pfn;
        for index in indices[..2].iter().copied() {
            let descriptor = unsafe { Self::table(table_pfn)?[index] };
            if descriptor & (AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE)
                != AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE
            {
                return Err(AddressSpaceError::NotMapped);
            }
            table_pfn = (descriptor & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64;
        }
        unsafe { Ok(&mut Self::table_mut(table_pfn)?[indices[2]]) }
    }

    fn rollback_created_tables(&mut self, created: &[(u64, usize, u64)]) {
        for &(parent_pfn, index, child_pfn) in created.iter().rev() {
            if let Ok(parent) = unsafe { Self::table_mut(parent_pfn) } {
                parent[index] = 0;
            }
            if self.table_pfns.last() == Some(&child_pfn) {
                self.table_pfns.pop();
            }
            PageFrameAllocator::free(child_pfn);
        }
    }

    unsafe fn table(pfn: u64) -> Result<&'static [u64; AARCH64_TABLE_ENTRIES], AddressSpaceError> {
        let address =
            PageFrameAllocator::pfn_address(pfn).ok_or(AddressSpaceError::InvalidAddress)?;
        Ok(&*(address as *const [u64; AARCH64_TABLE_ENTRIES]))
    }

    unsafe fn table_mut(
        pfn: u64,
    ) -> Result<&'static mut [u64; AARCH64_TABLE_ENTRIES], AddressSpaceError> {
        let address =
            PageFrameAllocator::pfn_address(pfn).ok_or(AddressSpaceError::InvalidAddress)?;
        Ok(&mut *(address as *mut [u64; AARCH64_TABLE_ENTRIES]))
    }
}

impl Drop for Aarch64AddressSpace {
    fn drop(&mut self) {
        for pfn in self.table_pfns.drain(..) {
            PageFrameAllocator::free(pfn);
        }
    }
}
