use crate::kernel_lowlevel::aarch64_vm_logic_shared::{
    Aarch64AddressSpaceBackend, Aarch64AddressSpaceCore, Aarch64AddressSpaceCoreError,
    AARCH64_TABLE_ENTRIES,
};
use crate::kernel_lowlevel::cpu;
use crate::kernel_lowlevel::drivers;
use crate::kernel_lowlevel::memory::PageFrameAllocator;
use crate::kernel_lowlevel::mmu::AddressSpaceError;

type TableAllocationFailureHook = fn(usize) -> bool;

struct PageFrameBackend {
    allocation_count: usize,
    failure_hook: Option<TableAllocationFailureHook>,
    defer_user_updates: bool,
}

impl PageFrameBackend {
    const fn new(failure_hook: Option<TableAllocationFailureHook>) -> Self {
        Self {
            allocation_count: 0,
            failure_hook,
            defer_user_updates: false,
        }
    }
}

impl Aarch64AddressSpaceBackend for PageFrameBackend {
    fn allocate_table(&mut self) -> Result<u64, Aarch64AddressSpaceCoreError> {
        let pfn = PageFrameAllocator::alloc_table()
            .ok_or(Aarch64AddressSpaceCoreError::OutOfMemory)?;
        let address = match PageFrameAllocator::pfn_address(pfn) {
            Some(address) => address,
            None => {
                PageFrameAllocator::release_table(pfn);
                return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
            }
        };
        unsafe { core::ptr::write_bytes(address as *mut u64, 0, AARCH64_TABLE_ENTRIES) };
        let allocation = self.allocation_count;
        self.allocation_count = self.allocation_count.saturating_add(1);
        if self.failure_hook.is_some_and(|hook| hook(allocation)) {
            PageFrameAllocator::release_table(pfn);
            return Err(Aarch64AddressSpaceCoreError::OutOfMemory);
        }
        Ok(pfn)
    }

    fn free_table(&mut self, pfn: u64) {
        PageFrameAllocator::release_table(pfn);
    }

    fn pfn_address(&self, pfn: u64) -> Option<usize> {
        PageFrameAllocator::pfn_address(pfn)
    }

    fn read_table_entry(&self, pfn: u64, index: usize) -> Option<u64> {
        let address = PageFrameAllocator::pfn_address(pfn)?;
        let table = unsafe { &*(address as *const [u64; AARCH64_TABLE_ENTRIES]) };
        table.get(index).copied()
    }

    fn write_table_entry(&mut self, pfn: u64, index: usize, descriptor: u64) -> bool {
        let Some(address) = PageFrameAllocator::pfn_address(pfn) else {
            return false;
        };
        let table = unsafe { &mut *(address as *mut [u64; AARCH64_TABLE_ENTRIES]) };
        let Some(entry) = table.get_mut(index) else {
            return false;
        };
        *entry = descriptor;
        true
    }

    fn copy_to_physical(&self, physical: usize, bytes: &[u8]) -> bool {
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), physical as *mut u8, bytes.len());
        }
        true
    }

    fn copy_from_physical(&self, physical: usize, out: &mut [u8]) -> bool {
        unsafe {
            core::ptr::copy_nonoverlapping(physical as *const u8, out.as_mut_ptr(), out.len());
        }
        true
    }

    #[cfg(not(target_arch = "aarch64"))]
    fn copy_table_from(&mut self, source: &Self, source_pfn: u64, destination_pfn: u64) -> bool {
        let Some(source_address) = source.pfn_address(source_pfn) else {
            return false;
        };
        let Some(destination_address) = self.pfn_address(destination_pfn) else {
            return false;
        };
        unsafe {
            core::ptr::copy_nonoverlapping(
                source_address as *const u8,
                destination_address as *mut u8,
                crate::kernel_lowlevel::aarch64_vm_logic_shared::AARCH64_PAGE_SIZE,
            );
        }
        true
    }

    fn copy_table_within(&mut self, source_pfn: u64, destination_pfn: u64) -> bool {
        let Some(source_address) = self.pfn_address(source_pfn) else {
            return false;
        };
        let Some(destination_address) = self.pfn_address(destination_pfn) else {
            return false;
        };
        unsafe {
            core::ptr::copy_nonoverlapping(
                source_address as *const u8,
                destination_address as *mut u8,
                crate::kernel_lowlevel::aarch64_vm_logic_shared::AARCH64_PAGE_SIZE,
            );
        }
        true
    }

    fn retain_table(&mut self, pfn: u64) -> bool {
        PageFrameAllocator::retain_table(pfn)
    }

    fn release_table(&mut self, pfn: u64) {
        PageFrameAllocator::release_table(pfn);
    }

    fn table_is_shared(&self, pfn: u64) -> bool {
        PageFrameAllocator::table_is_shared(pfn)
    }

    fn publish_user_mapping(&mut self, _vaddr: usize) {
        if !self.defer_user_updates {
            cpu::complete_user_page_update();
        }
    }

    fn break_user_mapping(&mut self, vaddr: usize) {
        cpu::invalidate_user_page(vaddr);
    }

    fn complete_user_mapping(&mut self) {
        cpu::complete_user_page_update();
    }

    fn begin_deferred_user_updates(&mut self) {
        self.defer_user_updates = true;
    }

    fn end_deferred_user_updates(&mut self) {
        self.defer_user_updates = false;
    }
}

pub struct Aarch64AddressSpace {
    core: Aarch64AddressSpaceCore<PageFrameBackend>,
}

impl Aarch64AddressSpace {
    pub fn new_with_kernel_map() -> Result<Self, AddressSpaceError> {
        Self::new_with_kernel_map_backend(PageFrameBackend::new(None))
    }

    pub(crate) fn new_for_fork(
        parent: &Self,
        failure_hook: TableAllocationFailureHook,
    ) -> Result<Self, AddressSpaceError> {
        let mut address_space = Aarch64AddressSpaceCore::new(PageFrameBackend::new(Some(failure_hook)))
            .map_err(map_core_error)?;
        address_space
            .clone_shared_mappings_from(&parent.core)
            .map_err(map_core_error)?;
        Ok(Self {
            core: address_space,
        })
    }

    fn new_with_kernel_map_backend(backend: PageFrameBackend) -> Result<Self, AddressSpaceError> {
        let mut address_space = Aarch64AddressSpaceCore::new(backend)?;
        let memory = drivers::memory_reg().ok_or(AddressSpaceError::InvalidAddress)?;
        address_space.map_supervisor_ram_range(memory.base, memory.size, true)?;
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
        Ok(Self {
            core: address_space,
        })
    }

    pub fn root_paddr(&self) -> u64 {
        self.core.root_paddr().unwrap_or(0) as u64
    }

    pub(crate) fn table_page_count(&self) -> usize {
        self.core.table_page_count()
    }

    pub(crate) fn begin_deferred_user_updates(&mut self) {
        self.core.begin_deferred_user_updates();
    }

    pub(crate) fn end_deferred_user_updates(&mut self) {
        self.core.end_deferred_user_updates();
    }

    pub fn map_user_page(
        &mut self,
        vaddr: usize,
        pfn: u64,
        readable: bool,
        writable: bool,
        executable: bool,
    ) -> Result<(), AddressSpaceError> {
        self.core
            .map_user_page(vaddr, pfn, readable, writable, executable)
            .map_err(map_core_error)
    }

    pub(crate) fn map_user_pages_with_protection(
        &mut self,
        vaddr: usize,
        page_count: usize,
        pfn_at: impl FnMut(usize) -> u64,
        protection_at: impl FnMut(usize) -> (bool, bool, bool),
    ) -> Result<(), AddressSpaceError> {
        self.core
            .map_user_pages_with_protection(vaddr, page_count, pfn_at, protection_at)
            .map_err(map_core_error)
    }

    pub(crate) fn map_user_region(
        &mut self,
        vaddr: usize,
        first_pfn: u64,
        page_count: usize,
        readable: bool,
        writable: bool,
        executable: bool,
    ) -> Result<(), AddressSpaceError> {
        self.core
            .map_user_region(vaddr, first_pfn, page_count, readable, writable, executable)
            .map_err(map_core_error)
    }

    pub fn protect_user_page(
        &mut self,
        vaddr: usize,
        readable: bool,
        writable: bool,
        executable: bool,
    ) -> Result<(), AddressSpaceError> {
        self.core
            .protect_user_page(vaddr, readable, writable, executable)
            .map_err(map_core_error)
    }

    pub fn unmap_user_page(&mut self, vaddr: usize) -> Result<u64, AddressSpaceError> {
        self.core.unmap_user_page(vaddr).map_err(map_core_error)
    }

    pub fn translate_user(&self, vaddr: usize, write: bool) -> Option<usize> {
        self.core.translate_user(vaddr, write)
    }

    pub fn copy_to_user(&self, vaddr: usize, bytes: &[u8]) -> Result<(), AddressSpaceError> {
        self.core.copy_to_user(vaddr, bytes).map_err(map_core_error)
    }

    pub fn copy_from_user(&self, vaddr: usize, out: &mut [u8]) -> Result<(), AddressSpaceError> {
        self.core.copy_from_user(vaddr, out).map_err(map_core_error)
    }
}

fn map_core_error(error: Aarch64AddressSpaceCoreError) -> AddressSpaceError {
    error.into()
}

impl From<Aarch64AddressSpaceCoreError> for AddressSpaceError {
    fn from(error: Aarch64AddressSpaceCoreError) -> Self {
        match error {
            Aarch64AddressSpaceCoreError::OutOfMemory => AddressSpaceError::OutOfMemory,
            Aarch64AddressSpaceCoreError::InvalidAddress => AddressSpaceError::InvalidAddress,
            Aarch64AddressSpaceCoreError::AlreadyMapped => AddressSpaceError::AlreadyMapped,
            Aarch64AddressSpaceCoreError::NotMapped => AddressSpaceError::NotMapped,
            Aarch64AddressSpaceCoreError::PermissionDenied => AddressSpaceError::PermissionDenied,
        }
    }
}
