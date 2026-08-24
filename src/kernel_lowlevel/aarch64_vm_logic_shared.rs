pub(crate) const AARCH64_PAGE_SIZE: usize = 0x1000;
pub(crate) const AARCH64_VA_BITS: usize = 39;
pub(crate) const AARCH64_TABLE_ENTRIES: usize = 512;
pub(crate) const AARCH64_USER_BASE: usize = 0x1000_0000;
pub(crate) const AARCH64_USER_LIMIT: usize = 0x2000_0000;
pub(crate) const AARCH64_L2_BLOCK_SIZE: usize = 1 << 21;
pub(crate) const AARCH64_DESC_VALID: u64 = 1;
pub(crate) const AARCH64_DESC_TABLE_OR_PAGE: u64 = 2;
pub(crate) const AARCH64_DESC_AF: u64 = 1 << 10;
pub(crate) const AARCH64_DESC_AP_USER: u64 = 1 << 6;
pub(crate) const AARCH64_DESC_AP_READ_ONLY: u64 = 1 << 7;
pub(crate) const AARCH64_DESC_INNER_SHAREABLE: u64 = 3 << 8;
pub(crate) const AARCH64_DESC_PXN: u64 = 1 << 53;
pub(crate) const AARCH64_DESC_UXN: u64 = 1 << 54;
pub(crate) const AARCH64_DESC_ADDR_MASK: u64 = 0x0000_ffff_ffff_f000;
pub(crate) const AARCH64_L2_BLOCK_ADDR_MASK: u64 = 0x0000_ffff_ffe0_0000;

pub(crate) fn aarch64_table_descriptor(paddr: usize) -> u64 {
    (paddr as u64 & AARCH64_DESC_ADDR_MASK) | AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE
}

pub(crate) fn aarch64_l3_page_descriptor_valid(descriptor: u64) -> bool {
    descriptor & (AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE)
        == AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE
}

pub(crate) fn aarch64_user_page_descriptor(
    paddr: usize,
    readable: bool,
    writable: bool,
    executable: bool,
) -> u64 {
    let user_access = if readable || writable || executable {
        AARCH64_DESC_AP_USER
    } else {
        0
    };
    let read_only = if writable {
        0
    } else {
        AARCH64_DESC_AP_READ_ONLY
    };
    let execute_never = if executable { 0 } else { AARCH64_DESC_UXN };
    (paddr as u64 & AARCH64_DESC_ADDR_MASK)
        | AARCH64_DESC_VALID
        | AARCH64_DESC_TABLE_OR_PAGE
        | AARCH64_DESC_AF
        | user_access
        | AARCH64_DESC_INNER_SHAREABLE
        | read_only
        | execute_never
        | AARCH64_DESC_PXN
}

pub(crate) fn aarch64_supervisor_block_descriptor(
    paddr: usize,
    device: bool,
    executable: bool,
) -> u64 {
    let attr_index = if device { 1u64 << 2 } else { 0 };
    let execute_never = AARCH64_DESC_UXN | if executable { 0 } else { AARCH64_DESC_PXN };
    (paddr as u64 & AARCH64_L2_BLOCK_ADDR_MASK)
        | AARCH64_DESC_VALID
        | AARCH64_DESC_AF
        | AARCH64_DESC_INNER_SHAREABLE
        | attr_index
        | execute_never
}

pub(crate) fn aarch64_table_indices(vaddr: usize) -> Option<[usize; 3]> {
    if vaddr >= 1usize.checked_shl(AARCH64_VA_BITS as u32)? {
        return None;
    }
    Some([
        (vaddr >> 30) & (AARCH64_TABLE_ENTRIES - 1),
        (vaddr >> 21) & (AARCH64_TABLE_ENTRIES - 1),
        (vaddr >> 12) & (AARCH64_TABLE_ENTRIES - 1),
    ])
}

pub(crate) fn aarch64_user_range_valid(start: usize, len: usize) -> bool {
    start & (AARCH64_PAGE_SIZE - 1) == 0
        && len != 0
        && len & (AARCH64_PAGE_SIZE - 1) == 0
        && start >= AARCH64_USER_BASE
        && start
            .checked_add(len)
            .map(|end| end <= AARCH64_USER_LIMIT)
            .unwrap_or(false)
}

pub(crate) fn aarch64_frame_range(
    kernel_end: usize,
    ram_base: usize,
    ram_end: usize,
) -> Option<(usize, usize)> {
    let start = kernel_end.checked_add(AARCH64_PAGE_SIZE - 1)? & !(AARCH64_PAGE_SIZE - 1);
    let start = core::cmp::max(start, ram_base);
    let end = ram_end & !(AARCH64_PAGE_SIZE - 1);
    if start >= end {
        return None;
    }
    if end <= AARCH64_USER_BASE || start >= AARCH64_USER_LIMIT {
        return Some((start, end));
    }

    let before_end = core::cmp::min(end, AARCH64_USER_BASE);
    let before_len = before_end.saturating_sub(start);
    let after_start = core::cmp::max(start, AARCH64_USER_LIMIT);
    let after_len = end.saturating_sub(after_start);
    if after_len >= before_len && after_len != 0 {
        Some((after_start, end))
    } else if before_len != 0 {
        Some((start, before_end))
    } else {
        None
    }
}

pub(crate) fn aarch64_frame_range_cap(
    start: usize,
    end: usize,
    capacity_bytes: usize,
) -> Option<(usize, usize)> {
    let capped_end = core::cmp::min(end, start.checked_add(capacity_bytes).unwrap_or(usize::MAX));
    (start < capped_end).then_some((start, capped_end))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Aarch64AddressSpaceCoreError {
    OutOfMemory,
    InvalidAddress,
    AlreadyMapped,
    NotMapped,
    PermissionDenied,
}

pub(crate) trait Aarch64AddressSpaceBackend {
    fn allocate_table(&mut self) -> Result<u64, Aarch64AddressSpaceCoreError>;
    fn free_table(&mut self, pfn: u64);
    fn pfn_address(&self, pfn: u64) -> Option<usize>;
    fn read_table_entry(&self, pfn: u64, index: usize) -> Option<u64>;
    fn write_table_entry(&mut self, pfn: u64, index: usize, descriptor: u64) -> bool;
    fn copy_to_physical(&self, physical: usize, bytes: &[u8]) -> bool;
    fn copy_from_physical(&self, physical: usize, out: &mut [u8]) -> bool;

    #[cfg(not(target_arch = "aarch64"))]
    fn copy_table_from(&mut self, source: &Self, source_pfn: u64, destination_pfn: u64) -> bool {
        for index in 0..AARCH64_TABLE_ENTRIES {
            let Some(descriptor) = source.read_table_entry(source_pfn, index) else {
                return false;
            };
            if !self.write_table_entry(destination_pfn, index, descriptor) {
                return false;
            }
        }
        true
    }

    fn copy_table_within(&mut self, source_pfn: u64, destination_pfn: u64) -> bool {
        let mut entries = [0u64; AARCH64_TABLE_ENTRIES];
        for (index, entry) in entries.iter_mut().enumerate() {
            let Some(descriptor) = self.read_table_entry(source_pfn, index) else {
                return false;
            };
            *entry = descriptor;
        }
        for (index, descriptor) in entries.into_iter().enumerate() {
            if !self.write_table_entry(destination_pfn, index, descriptor) {
                return false;
            }
        }
        true
    }

    fn retain_table(&mut self, _pfn: u64) -> bool {
        true
    }

    fn release_table(&mut self, pfn: u64) {
        self.free_table(pfn);
    }

    fn table_is_shared(&self, _pfn: u64) -> bool {
        false
    }

    fn physical_page_accessible(&self, pfn: u64) -> bool {
        self.pfn_address(pfn).is_some()
    }

    fn publish_user_mapping(&mut self, _vaddr: usize) {}
    fn break_user_mapping(&mut self, _vaddr: usize) {}
    fn complete_user_mapping(&mut self) {}

    fn begin_deferred_user_updates(&mut self) {}

    fn end_deferred_user_updates(&mut self) {}
}

pub(crate) struct Aarch64AddressSpaceCore<B: Aarch64AddressSpaceBackend> {
    root_pfn: u64,
    table_pfns: alloc::vec::Vec<u64>,
    backend: B,
}

impl<B: Aarch64AddressSpaceBackend> Aarch64AddressSpaceCore<B> {
    pub(crate) fn new(mut backend: B) -> Result<Self, Aarch64AddressSpaceCoreError> {
        let mut table_pfns = alloc::vec::Vec::new();
        table_pfns
            .try_reserve(1)
            .map_err(|_| Aarch64AddressSpaceCoreError::OutOfMemory)?;
        let root_pfn = backend.allocate_table()?;
        table_pfns.push(root_pfn);
        Ok(Self {
            root_pfn,
            table_pfns,
            backend,
        })
    }

    #[cfg(not(target_os = "none"))]
    pub(crate) fn root_pfn(&self) -> u64 {
        self.root_pfn
    }

    pub(crate) fn root_paddr(&self) -> Option<usize> {
        self.backend.pfn_address(self.root_pfn)
    }

    pub(crate) fn table_page_count(&self) -> usize {
        self.table_pfns.len()
    }

    pub(crate) fn begin_deferred_user_updates(&mut self) {
        self.backend.begin_deferred_user_updates();
    }

    pub(crate) fn end_deferred_user_updates(&mut self) {
        self.backend.end_deferred_user_updates();
    }

    pub(crate) fn map_user_page(
        &mut self,
        vaddr: usize,
        pfn: u64,
        readable: bool,
        writable: bool,
        executable: bool,
    ) -> Result<(), Aarch64AddressSpaceCoreError> {
        if !aarch64_user_range_valid(vaddr, AARCH64_PAGE_SIZE) {
            return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
        }
        self.ensure_private_existing_path(vaddr)?;
        let paddr = self
            .backend
            .pfn_address(pfn)
            .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        if let Ok((table_pfn, index)) = self.leaf_location(vaddr) {
            let descriptor = self
                .backend
                .read_table_entry(table_pfn, index)
                .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
            if descriptor != 0 {
                return Err(Aarch64AddressSpaceCoreError::AlreadyMapped);
            }
            let descriptor = aarch64_user_page_descriptor(paddr, readable, writable, executable);
            if !self.backend.write_table_entry(table_pfn, index, descriptor) {
                return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
            }
            self.backend.publish_user_mapping(vaddr);
            return Ok(());
        }

        let mut created = alloc::vec::Vec::new();
        let mut mapped = alloc::vec::Vec::new();
        let result = self.map_user_page_recorded(
            vaddr,
            pfn,
            readable,
            writable,
            executable,
            &mut created,
            &mut mapped,
        );
        if result.is_err() {
            self.rollback_mapping_transaction(&mapped, &created);
        }
        result
    }

    pub(crate) fn map_user_region(
        &mut self,
        vaddr: usize,
        first_pfn: u64,
        page_count: usize,
        readable: bool,
        writable: bool,
        executable: bool,
    ) -> Result<(), Aarch64AddressSpaceCoreError> {
        let len = page_count
            .checked_mul(AARCH64_PAGE_SIZE)
            .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        if !aarch64_user_range_valid(vaddr, len) {
            return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
        }
        first_pfn
            .checked_add(page_count.saturating_sub(1) as u64)
            .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;

        let mut created = alloc::vec::Vec::new();
        let mut mapped = alloc::vec::Vec::new();
        for page_index in 0..page_count {
            let page_vaddr = vaddr
                .checked_add(page_index * AARCH64_PAGE_SIZE)
                .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
            let pfn = first_pfn
                .checked_add(page_index as u64)
                .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
            if let Err(error) = self.map_user_page_recorded(
                page_vaddr,
                pfn,
                readable,
                writable,
                executable,
                &mut created,
                &mut mapped,
            ) {
                self.rollback_mapping_transaction(&mapped, &created);
                return Err(error);
            }
        }
        Ok(())
    }

    /// Map a contiguous virtual range while reusing the page-table walk for
    /// adjacent pages. The callback form keeps fork cloning from allocating a
    /// temporary PFN or permission vector for every mapping.
    pub(crate) fn map_user_pages_with_protection<P, F>(
        &mut self,
        vaddr: usize,
        page_count: usize,
        mut pfn_at: P,
        mut protection_at: F,
    ) -> Result<(), Aarch64AddressSpaceCoreError>
    where
        P: FnMut(usize) -> u64,
        F: FnMut(usize) -> (bool, bool, bool),
    {
        let len = page_count
            .checked_mul(AARCH64_PAGE_SIZE)
            .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        if !aarch64_user_range_valid(vaddr, len) {
            return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
        }
        if page_count == 0 {
            return Ok(());
        }

        let mut created = alloc::vec::Vec::new();
        created
            .try_reserve(2)
            .map_err(|_| Aarch64AddressSpaceCoreError::OutOfMemory)?;
        let mut mapped = alloc::vec::Vec::new();
        mapped
            .try_reserve_exact(page_count)
            .map_err(|_| Aarch64AddressSpaceCoreError::OutOfMemory)?;
        let mut cached_indices = None;
        let mut cached_leaf = 0;

        for page_index in 0..page_count {
            let page_vaddr = vaddr
                .checked_add(page_index * AARCH64_PAGE_SIZE)
                .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
            self.ensure_private_existing_path(page_vaddr)?;
            let indices =
                aarch64_table_indices(page_vaddr).ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
            let table_pfn = if cached_indices == Some([indices[0], indices[1]]) {
                cached_leaf
            } else {
                let leaf = self.ensure_leaf_table(indices, &mut created)?;
                cached_indices = Some([indices[0], indices[1]]);
                cached_leaf = leaf;
                leaf
            };
            let descriptor = self
                .backend
                .read_table_entry(table_pfn, indices[2])
                .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
            if descriptor != 0 {
                self.rollback_mapping_transaction(&mapped, &created);
                return Err(Aarch64AddressSpaceCoreError::AlreadyMapped);
            }
            let paddr = self
                .backend
                .pfn_address(pfn_at(page_index))
                .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
            let (readable, writable, executable) = protection_at(page_index);
            let descriptor = aarch64_user_page_descriptor(
                paddr,
                readable,
                writable,
                executable,
            );
            if !self
                .backend
                .write_table_entry(table_pfn, indices[2], descriptor)
            {
                self.rollback_mapping_transaction(&mapped, &created);
                return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
            }
            self.backend.publish_user_mapping(page_vaddr);
            mapped.push(page_vaddr);
        }
        Ok(())
    }

    fn map_user_page_recorded(
        &mut self,
        vaddr: usize,
        pfn: u64,
        readable: bool,
        writable: bool,
        executable: bool,
        created: &mut alloc::vec::Vec<(u64, usize, u64)>,
        mapped: &mut alloc::vec::Vec<usize>,
    ) -> Result<(), Aarch64AddressSpaceCoreError> {
        if !aarch64_user_range_valid(vaddr, AARCH64_PAGE_SIZE) {
            return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
        }
        self.ensure_private_existing_path(vaddr)?;
        mapped
            .try_reserve(1)
            .map_err(|_| Aarch64AddressSpaceCoreError::OutOfMemory)?;
        let paddr = self
            .backend
            .pfn_address(pfn)
            .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        let indices =
            aarch64_table_indices(vaddr).ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        let table_pfn = self.ensure_leaf_table(indices, created)?;
        let descriptor = self
            .backend
            .read_table_entry(table_pfn, indices[2])
            .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        if descriptor != 0 {
            return Err(Aarch64AddressSpaceCoreError::AlreadyMapped);
        }
        let descriptor = aarch64_user_page_descriptor(paddr, readable, writable, executable);
        if !self
            .backend
            .write_table_entry(table_pfn, indices[2], descriptor)
        {
            return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
        }
        self.backend.publish_user_mapping(vaddr);
        mapped.push(vaddr);
        Ok(())
    }

    pub(crate) fn protect_user_page(
        &mut self,
        vaddr: usize,
        readable: bool,
        writable: bool,
        executable: bool,
    ) -> Result<(), Aarch64AddressSpaceCoreError> {
        if !aarch64_user_range_valid(vaddr, AARCH64_PAGE_SIZE) {
            return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
        }
        self.ensure_private_existing_path(vaddr)?;
        let (table_pfn, index) = self.leaf_location(vaddr)?;
        let descriptor = self
            .backend
            .read_table_entry(table_pfn, index)
            .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        if !aarch64_l3_page_descriptor_valid(descriptor) {
            return Err(Aarch64AddressSpaceCoreError::NotMapped);
        }
        if !self.backend.write_table_entry(table_pfn, index, 0) {
            return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
        }
        self.backend.break_user_mapping(vaddr);
        let replacement = aarch64_user_page_descriptor(
            (descriptor & AARCH64_DESC_ADDR_MASK) as usize,
            readable,
            writable,
            executable,
        );
        if !self
            .backend
            .write_table_entry(table_pfn, index, replacement)
        {
            return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
        }
        self.backend.complete_user_mapping();
        Ok(())
    }

    pub(crate) fn unmap_user_page(
        &mut self,
        vaddr: usize,
    ) -> Result<u64, Aarch64AddressSpaceCoreError> {
        if !aarch64_user_range_valid(vaddr, AARCH64_PAGE_SIZE) {
            return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
        }
        self.ensure_private_existing_path(vaddr)?;
        let (table_pfn, index) = self.leaf_location(vaddr)?;
        let descriptor = self
            .backend
            .read_table_entry(table_pfn, index)
            .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        if !aarch64_l3_page_descriptor_valid(descriptor) {
            return Err(Aarch64AddressSpaceCoreError::NotMapped);
        }
        if !self.backend.write_table_entry(table_pfn, index, 0) {
            return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
        }
        self.backend.break_user_mapping(vaddr);
        Ok((descriptor & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64)
    }

    pub(crate) fn translate_user(&self, vaddr: usize, write: bool) -> Option<usize> {
        if !(AARCH64_USER_BASE..AARCH64_USER_LIMIT).contains(&vaddr) {
            return None;
        }
        let (table_pfn, index) = self.leaf_location(vaddr).ok()?;
        let descriptor = self.backend.read_table_entry(table_pfn, index)?;
        if !aarch64_l3_page_descriptor_valid(descriptor)
            || descriptor & AARCH64_DESC_AP_USER == 0
            || (write && descriptor & AARCH64_DESC_AP_READ_ONLY != 0)
        {
            return None;
        }
        Some((descriptor & AARCH64_DESC_ADDR_MASK) as usize | (vaddr & (AARCH64_PAGE_SIZE - 1)))
    }

    pub(crate) fn copy_to_user(
        &self,
        vaddr: usize,
        bytes: &[u8],
    ) -> Result<(), Aarch64AddressSpaceCoreError> {
        self.validate_copy(vaddr, bytes.len(), true)?;
        let mut current = vaddr;
        let mut offset = 0;
        while offset < bytes.len() {
            let physical = self
                .translate_user(current, true)
                .ok_or(Aarch64AddressSpaceCoreError::PermissionDenied)?;
            let count = core::cmp::min(
                AARCH64_PAGE_SIZE - (physical & (AARCH64_PAGE_SIZE - 1)),
                bytes.len() - offset,
            );
            if !self
                .backend
                .copy_to_physical(physical, &bytes[offset..offset + count])
            {
                return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
            }
            current += count;
            offset += count;
        }
        Ok(())
    }

    pub(crate) fn copy_from_user(
        &self,
        vaddr: usize,
        out: &mut [u8],
    ) -> Result<(), Aarch64AddressSpaceCoreError> {
        self.validate_copy(vaddr, out.len(), false)?;
        let mut current = vaddr;
        let mut offset = 0;
        while offset < out.len() {
            let physical = self
                .translate_user(current, false)
                .ok_or(Aarch64AddressSpaceCoreError::PermissionDenied)?;
            let count = core::cmp::min(
                AARCH64_PAGE_SIZE - (physical & (AARCH64_PAGE_SIZE - 1)),
                out.len() - offset,
            );
            if !self
                .backend
                .copy_from_physical(physical, &mut out[offset..offset + count])
            {
                return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
            }
            current += count;
            offset += count;
        }
        Ok(())
    }

    fn validate_copy(
        &self,
        vaddr: usize,
        len: usize,
        write: bool,
    ) -> Result<(), Aarch64AddressSpaceCoreError> {
        let end = vaddr
            .checked_add(len)
            .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        let mut current = vaddr;
        while current < end {
            let physical = self
                .translate_user(current, write)
                .ok_or(Aarch64AddressSpaceCoreError::PermissionDenied)?;
            if !self
                .backend
                .physical_page_accessible((physical / AARCH64_PAGE_SIZE) as u64)
            {
                return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
            }
            let count = core::cmp::min(
                AARCH64_PAGE_SIZE - (current & (AARCH64_PAGE_SIZE - 1)),
                end - current,
            );
            current += count;
        }
        Ok(())
    }

    pub(crate) fn map_supervisor_range(
        &mut self,
        start: usize,
        len: usize,
        device: bool,
        executable: bool,
    ) -> Result<(), Aarch64AddressSpaceCoreError> {
        let requested_end = start
            .checked_add(len)
            .filter(|_| len != 0)
            .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        let block_start = start & !(AARCH64_L2_BLOCK_SIZE - 1);
        let block_end = requested_end
            .checked_add(AARCH64_L2_BLOCK_SIZE - 1)
            .map(|end| end & !(AARCH64_L2_BLOCK_SIZE - 1))
            .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        if block_end > (1usize << AARCH64_VA_BITS) {
            return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
        }
        if block_start < AARCH64_USER_LIMIT && block_end > AARCH64_USER_BASE {
            return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
        }

        let mut current = block_start;
        while current < block_end {
            let indices = aarch64_table_indices(current)
                .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
            let table_pfn = self.ensure_level_two_table(indices[0])?;
            let descriptor = aarch64_supervisor_block_descriptor(current, device, executable);
            let existing = self
                .backend
                .read_table_entry(table_pfn, indices[1])
                .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
            if existing == 0 {
                if !self
                    .backend
                    .write_table_entry(table_pfn, indices[1], descriptor)
                {
                    return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
                }
            } else if existing != descriptor {
                return Err(Aarch64AddressSpaceCoreError::AlreadyMapped);
            }
            current += AARCH64_L2_BLOCK_SIZE;
        }
        Ok(())
    }

    pub(crate) fn map_supervisor_ram_range(
        &mut self,
        start: usize,
        len: usize,
        executable: bool,
    ) -> Result<(), Aarch64AddressSpaceCoreError> {
        let end = start
            .checked_add(len)
            .filter(|_| len != 0)
            .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        if end > (1usize << AARCH64_VA_BITS) {
            return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
        }

        let low_end = core::cmp::min(end, AARCH64_USER_BASE);
        if start < low_end {
            self.map_supervisor_range(start, low_end - start, false, executable)?;
        }
        let high_start = core::cmp::max(start, AARCH64_USER_LIMIT);
        if high_start < end {
            self.map_supervisor_range(high_start, end - high_start, false, executable)?;
        }
        Ok(())
    }

    /// Copy the supervisor portion of an address space without copying user mappings.
    ///
    /// Kernel mappings are immutable after boot, so each fork can copy the small
    /// page-table hierarchy that describes them instead of rebuilding every RAM
    /// block descriptor from the physical-memory range.
    #[cfg(not(target_arch = "aarch64"))]
    pub(crate) fn clone_kernel_mappings_from(
        &mut self,
        source: &Self,
    ) -> Result<(), Aarch64AddressSpaceCoreError> {
        self.clone_mappings_from_mode(source, false)
    }

    /// Clone the complete page-table hierarchy, including user mappings.
    ///
    /// Fork uses this path so the child gets an independent set of table pages
    /// without walking and rewriting every mapped user page individually.
    #[cfg(not(target_arch = "aarch64"))]
    pub(crate) fn clone_mappings_from(
        &mut self,
        source: &Self,
    ) -> Result<(), Aarch64AddressSpaceCoreError> {
        self.clone_mappings_from_mode(source, true)
    }

    pub(crate) fn clone_shared_mappings_from(
        &mut self,
        source: &Self,
    ) -> Result<(), Aarch64AddressSpaceCoreError> {
        let original_len = self.table_pfns.len();
        let result = (|| {
            for root_index in 0..AARCH64_TABLE_ENTRIES {
                let descriptor = source
                    .backend
                    .read_table_entry(source.root_pfn, root_index)
                    .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
                if descriptor == 0 {
                    continue;
                }
                if descriptor
                    & (AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE)
                    != AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE
                {
                    if !self
                        .backend
                        .write_table_entry(self.root_pfn, root_index, descriptor)
                    {
                        return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
                    }
                    continue;
                }
                let source_child = (descriptor & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64;
                if !self.backend.retain_table(source_child) {
                    return Err(Aarch64AddressSpaceCoreError::OutOfMemory);
                }
                if self.table_pfns.try_reserve(1).is_err() {
                    self.backend.release_table(source_child);
                    return Err(Aarch64AddressSpaceCoreError::OutOfMemory);
                }
                self.table_pfns.push(source_child);
                if !self.backend.write_table_entry(self.root_pfn, root_index, descriptor) {
                    return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
                }
            }
            Ok(())
        })();
        if result.is_err() {
            self.rollback_shared_tables(original_len);
        }
        result
    }

    fn rollback_shared_tables(&mut self, original_len: usize) {
        while self.table_pfns.len() > original_len {
            if let Some(pfn) = self.table_pfns.pop() {
                self.backend.release_table(pfn);
            }
        }
    }

    fn ensure_private_existing_path(
        &mut self,
        vaddr: usize,
    ) -> Result<(), Aarch64AddressSpaceCoreError> {
        let indices = aarch64_table_indices(vaddr)
            .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        let level_one = self
            .backend
            .read_table_entry(self.root_pfn, indices[0])
            .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        if level_one == 0 {
            return Ok(());
        }
        if level_one & (AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE)
            != AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE
        {
            return Err(Aarch64AddressSpaceCoreError::AlreadyMapped);
        }
        let level_one_pfn = self.detach_shared_table(
            self.root_pfn,
            indices[0],
            (level_one & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64,
            1,
        )?;
        let level_two = self
            .backend
            .read_table_entry(level_one_pfn, indices[1])
            .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        if level_two == 0 {
            return Ok(());
        }
        if level_two & (AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE)
            != AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE
        {
            return Err(Aarch64AddressSpaceCoreError::AlreadyMapped);
        }
        let level_two_pfn = (level_two & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64;
        let _ = self.detach_shared_table(level_one_pfn, indices[1], level_two_pfn, 2)?;
        Ok(())
    }

    fn detach_shared_table(
        &mut self,
        parent_pfn: u64,
        parent_index: usize,
        source_pfn: u64,
        level: usize,
    ) -> Result<u64, Aarch64AddressSpaceCoreError> {
        if !self.backend.table_is_shared(source_pfn) {
            return Ok(source_pfn);
        }
        self.table_pfns
            .try_reserve(1)
            .map_err(|_| Aarch64AddressSpaceCoreError::OutOfMemory)?;
        let destination_pfn = self.backend.allocate_table()?;
        let retained_start = self.table_pfns.len();
        if !self
            .backend
            .copy_table_within(source_pfn, destination_pfn)
        {
            self.backend.release_table(destination_pfn);
            return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
        }
        if level == 1 {
            for index in 0..AARCH64_TABLE_ENTRIES {
                let descriptor = match self.backend.read_table_entry(destination_pfn, index) {
                    Some(descriptor) => descriptor,
                    None => {
                        self.rollback_detached_table(destination_pfn, retained_start);
                        return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
                    }
                };
                if descriptor
                    & (AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE)
                    != AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE
                {
                    continue;
                }
                let child_pfn =
                    (descriptor & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64;
                if !self.backend.retain_table(child_pfn) {
                    self.rollback_detached_table(destination_pfn, retained_start);
                    return Err(Aarch64AddressSpaceCoreError::OutOfMemory);
                }
                if self.table_pfns.try_reserve(1).is_err() {
                    self.backend.release_table(child_pfn);
                    self.rollback_detached_table(destination_pfn, retained_start);
                    return Err(Aarch64AddressSpaceCoreError::OutOfMemory);
                }
                self.table_pfns.push(child_pfn);
            }
        }
        let destination_paddr = match self.backend.pfn_address(destination_pfn) {
            Some(address) => address,
            None => {
                self.rollback_detached_table(destination_pfn, retained_start);
                return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
            }
        };
        let Some(position) = self.table_pfns.iter().position(|pfn| *pfn == source_pfn) else {
            self.rollback_detached_table(destination_pfn, retained_start);
            return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
        };
        if !self.backend.write_table_entry(
            parent_pfn,
            parent_index,
            aarch64_table_descriptor(destination_paddr),
        ) {
            self.rollback_detached_table(destination_pfn, retained_start);
            return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
        }
        self.table_pfns.swap_remove(position);
        self.backend.release_table(source_pfn);
        self.table_pfns.push(destination_pfn);
        Ok(destination_pfn)
    }

    fn rollback_detached_table(&mut self, destination_pfn: u64, retained_start: usize) {
        self.backend.release_table(destination_pfn);
        while self.table_pfns.len() > retained_start {
            if let Some(pfn) = self.table_pfns.pop() {
                self.backend.release_table(pfn);
            }
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    fn clone_mappings_from_mode(
        &mut self,
        source: &Self,
        preserve_user: bool,
    ) -> Result<(), Aarch64AddressSpaceCoreError> {
        for root_index in 0..AARCH64_TABLE_ENTRIES {
            let Some(descriptor) = source.backend.read_table_entry(source.root_pfn, root_index)
            else {
                return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
            };
            if descriptor == 0 {
                continue;
            }

            let base = root_index
                .checked_mul(1usize << 30)
                .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
            let child = if descriptor
                & (AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE)
                == AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE
            {
                let source_child = (descriptor & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64;
                self.clone_table_from(source, source_child, 1, base, preserve_user)?
            } else {
                None
            };
            if let Some(child_pfn) = child {
                let child_paddr = self
                    .backend
                    .pfn_address(child_pfn)
                    .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
                if !self.backend.write_table_entry(
                    self.root_pfn,
                    root_index,
                    aarch64_table_descriptor(child_paddr),
                ) {
                    return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
                }
            }
        }
        Ok(())
    }

    #[cfg(not(target_arch = "aarch64"))]
    fn clone_table_from(
        &mut self,
        source: &Self,
        source_pfn: u64,
        level: usize,
        base: usize,
        preserve_user: bool,
    ) -> Result<Option<u64>, Aarch64AddressSpaceCoreError> {
        let span = match level {
            1 => 1usize << 21,
            2 => AARCH64_PAGE_SIZE,
            _ => return Err(Aarch64AddressSpaceCoreError::InvalidAddress),
        };
        if preserve_user && level == 2 {
            let mut populated = false;
            for index in 0..AARCH64_TABLE_ENTRIES {
                if source
                    .backend
                    .read_table_entry(source_pfn, index)
                    .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?
                    != 0
                {
                    populated = true;
                    break;
                }
            }
            if !populated {
                return Ok(None);
            }
        }
        self.table_pfns
            .try_reserve(1)
            .map_err(|_| Aarch64AddressSpaceCoreError::OutOfMemory)?;
        let destination_pfn = self.backend.allocate_table()?;
        self.table_pfns.push(destination_pfn);
        if !self
            .backend
            .copy_table_from(&source.backend, source_pfn, destination_pfn)
        {
            return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
        }
        // A level-3 table contains only user page descriptors when a full
        // fork hierarchy is being cloned. Its entries already refer to the
        // shared COW backings, so the copied table needs no further filtering.
        if preserve_user && level == 2 {
            return Ok(Some(destination_pfn));
        }
        let mut copied = false;

        for index in 0..AARCH64_TABLE_ENTRIES {
            let entry_base = base
                .checked_add(
                    index
                        .checked_mul(span)
                        .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?,
                )
                .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
            let entry_end = entry_base
                .checked_add(span)
                .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
            if !preserve_user && entry_base < AARCH64_USER_LIMIT && entry_end > AARCH64_USER_BASE {
                if !self.backend.write_table_entry(destination_pfn, index, 0) {
                    return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
                }
                continue;
            }
            let descriptor = source
                .backend
                .read_table_entry(source_pfn, index)
                .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
            if descriptor == 0 {
                continue;
            }

            let descriptor = if level == 1
                && descriptor
                    & (AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE)
                    == AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE
            {
                let source_child =
                    (descriptor & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64;
                let Some(child_pfn) = self.clone_table_from(
                    source,
                    source_child,
                    2,
                    entry_base,
                    preserve_user,
                )?
                else {
                    continue;
                };
                let child_paddr = self
                    .backend
                    .pfn_address(child_pfn)
                    .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
                aarch64_table_descriptor(child_paddr)
            } else {
                descriptor
            };
            if !self.backend.write_table_entry(destination_pfn, index, descriptor) {
                return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
            }
            copied = true;
        }

        if !copied {
            debug_assert_eq!(self.table_pfns.last(), Some(&destination_pfn));
            self.table_pfns.pop();
            self.backend.release_table(destination_pfn);
            return Ok(None);
        }
        Ok(Some(destination_pfn))
    }

    fn ensure_leaf_table(
        &mut self,
        indices: [usize; 3],
        created: &mut alloc::vec::Vec<(u64, usize, u64)>,
    ) -> Result<u64, Aarch64AddressSpaceCoreError> {
        let mut table_pfn = self.root_pfn;
        for index in indices[..2].iter().copied() {
            let descriptor = self
                .backend
                .read_table_entry(table_pfn, index)
                .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
            if descriptor == 0 {
                self.table_pfns
                    .try_reserve(1)
                    .map_err(|_| Aarch64AddressSpaceCoreError::OutOfMemory)?;
                created
                    .try_reserve(1)
                    .map_err(|_| Aarch64AddressSpaceCoreError::OutOfMemory)?;
                let child_pfn = self.backend.allocate_table()?;
                let child_paddr = match self.backend.pfn_address(child_pfn) {
                    Some(address) => address,
                    None => {
                        self.backend.release_table(child_pfn);
                        return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
                    }
                };
                self.table_pfns.push(child_pfn);
                created.push((table_pfn, index, child_pfn));
                if !self.backend.write_table_entry(
                    table_pfn,
                    index,
                    aarch64_table_descriptor(child_paddr),
                ) {
                    return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
                }
                table_pfn = child_pfn;
            } else if descriptor & (AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE)
                == AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE
            {
                table_pfn = (descriptor & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64;
            } else {
                return Err(Aarch64AddressSpaceCoreError::AlreadyMapped);
            }
        }
        Ok(table_pfn)
    }

    fn ensure_level_two_table(
        &mut self,
        index: usize,
    ) -> Result<u64, Aarch64AddressSpaceCoreError> {
        let descriptor = self
            .backend
            .read_table_entry(self.root_pfn, index)
            .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        if descriptor == 0 {
            self.table_pfns
                .try_reserve(1)
                .map_err(|_| Aarch64AddressSpaceCoreError::OutOfMemory)?;
            let child_pfn = self.backend.allocate_table()?;
            let child_paddr = match self.backend.pfn_address(child_pfn) {
                Some(address) => address,
                None => {
                    self.backend.release_table(child_pfn);
                    return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
                }
            };
            if !self.backend.write_table_entry(
                self.root_pfn,
                index,
                aarch64_table_descriptor(child_paddr),
            ) {
                self.backend.release_table(child_pfn);
                return Err(Aarch64AddressSpaceCoreError::InvalidAddress);
            }
            self.table_pfns.push(child_pfn);
            Ok(child_pfn)
        } else if descriptor & (AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE)
            == AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE
        {
            Ok((descriptor & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64)
        } else {
            Err(Aarch64AddressSpaceCoreError::AlreadyMapped)
        }
    }

    fn leaf_location(&self, vaddr: usize) -> Result<(u64, usize), Aarch64AddressSpaceCoreError> {
        let indices =
            aarch64_table_indices(vaddr).ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        let mut table_pfn = self.root_pfn;
        for index in indices[..2].iter().copied() {
            let descriptor = self
                .backend
                .read_table_entry(table_pfn, index)
                .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
            if descriptor & (AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE)
                != AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE
            {
                return Err(Aarch64AddressSpaceCoreError::NotMapped);
            }
            table_pfn = (descriptor & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64;
        }
        Ok((table_pfn, indices[2]))
    }

    fn rollback_mapping_transaction(&mut self, mapped: &[usize], created: &[(u64, usize, u64)]) {
        for &vaddr in mapped.iter().rev() {
            if let Ok((table_pfn, index)) = self.leaf_location(vaddr) {
                if self.backend.write_table_entry(table_pfn, index, 0) {
                    self.backend.break_user_mapping(vaddr);
                }
            }
        }
        for &(parent_pfn, index, child_pfn) in created.iter().rev() {
            let _ = self.backend.write_table_entry(parent_pfn, index, 0);
            debug_assert_eq!(self.table_pfns.last(), Some(&child_pfn));
            if self.table_pfns.last() == Some(&child_pfn) {
                self.table_pfns.pop();
            }
            self.backend.release_table(child_pfn);
        }
    }
}

impl<B: Aarch64AddressSpaceBackend> Drop for Aarch64AddressSpaceCore<B> {
    fn drop(&mut self) {
        for pfn in self.table_pfns.drain(..) {
            self.backend.release_table(pfn);
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Aarch64MaintenanceEvent {
    Publish(usize),
    Break(usize),
    Make,
}

#[cfg(test)]
struct Aarch64TestAllocatorState {
    next_pfn: u64,
    remaining_table_allocations: Option<usize>,
    pages: alloc::collections::BTreeMap<u64, [u64; AARCH64_TABLE_ENTRIES]>,
    table_references: alloc::collections::BTreeMap<u64, usize>,
    data_pages: alloc::collections::BTreeMap<u64, alloc::boxed::Box<[u8; AARCH64_PAGE_SIZE]>>,
    maintenance_events: alloc::vec::Vec<Aarch64MaintenanceEvent>,
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct Aarch64TestAllocator {
    state: alloc::rc::Rc<core::cell::RefCell<Aarch64TestAllocatorState>>,
}

#[cfg(test)]
impl Aarch64TestAllocator {
    pub(crate) fn new(next_pfn: u64) -> Self {
        Self {
            state: alloc::rc::Rc::new(core::cell::RefCell::new(Aarch64TestAllocatorState {
                next_pfn,
                remaining_table_allocations: None,
                pages: alloc::collections::BTreeMap::new(),
                table_references: alloc::collections::BTreeMap::new(),
                data_pages: alloc::collections::BTreeMap::new(),
                maintenance_events: alloc::vec::Vec::new(),
            })),
        }
    }

    pub(crate) fn allocated_pages(&self) -> usize {
        self.state.borrow().pages.len()
    }

    pub(crate) fn fail_after_table_allocations(&mut self, successful_allocations: usize) {
        self.state.borrow_mut().remaining_table_allocations = Some(successful_allocations);
    }

    pub(crate) fn allow_table_allocations(&mut self) {
        self.state.borrow_mut().remaining_table_allocations = None;
    }

    pub(crate) fn insert_data_page(&mut self, pfn: u64) {
        self.state
            .borrow_mut()
            .data_pages
            .insert(pfn, alloc::boxed::Box::new([0; AARCH64_PAGE_SIZE]));
    }

    pub(crate) fn maintenance_events(&self) -> alloc::vec::Vec<Aarch64MaintenanceEvent> {
        self.state.borrow().maintenance_events.clone()
    }

    pub(crate) fn clear_maintenance_events(&mut self) {
        self.state.borrow_mut().maintenance_events.clear();
    }

    pub(crate) fn set_l3_descriptor(&mut self, root_pfn: u64, vaddr: usize, descriptor: u64) {
        let indices = aarch64_table_indices(vaddr).expect("test address indices");
        let mut state = self.state.borrow_mut();
        let level_two_pfn = (state.pages[&root_pfn][indices[0]] & AARCH64_DESC_ADDR_MASK)
            / AARCH64_PAGE_SIZE as u64;
        let level_three_pfn = (state.pages[&level_two_pfn][indices[1]] & AARCH64_DESC_ADDR_MASK)
            / AARCH64_PAGE_SIZE as u64;
        state
            .pages
            .get_mut(&level_three_pfn)
            .expect("level three table")[indices[2]] = descriptor;
    }

    pub(crate) fn set_root_descriptor(&mut self, root_pfn: u64, index: usize, descriptor: u64) {
        self.state
            .borrow_mut()
            .pages
            .get_mut(&root_pfn)
            .expect("test root table")[index] = descriptor;
    }

    pub(crate) fn set_level_one_descriptor(
        &mut self,
        root_pfn: u64,
        vaddr: usize,
        descriptor: u64,
    ) {
        let indices = aarch64_table_indices(vaddr).expect("test address indices");
        let mut state = self.state.borrow_mut();
        let level_one_pfn = (state.pages[&root_pfn][indices[0]] & AARCH64_DESC_ADDR_MASK)
            / AARCH64_PAGE_SIZE as u64;
        state
            .pages
            .get_mut(&level_one_pfn)
            .expect("test level-one table")[indices[1]] = descriptor;
    }
}

#[cfg(test)]
#[derive(Clone)]
struct Aarch64TestBackend {
    allocator: Aarch64TestAllocator,
}

#[cfg(test)]
impl Aarch64AddressSpaceBackend for Aarch64TestBackend {
    fn allocate_table(&mut self) -> Result<u64, Aarch64AddressSpaceCoreError> {
        let mut state = self.allocator.state.borrow_mut();
        if let Some(remaining) = &mut state.remaining_table_allocations {
            if *remaining == 0 {
                return Err(Aarch64AddressSpaceCoreError::OutOfMemory);
            }
            *remaining -= 1;
        }
        let pfn = state.next_pfn;
        state.next_pfn = state
            .next_pfn
            .checked_add(1)
            .ok_or(Aarch64AddressSpaceCoreError::InvalidAddress)?;
        state.pages.insert(pfn, [0; AARCH64_TABLE_ENTRIES]);
        state.table_references.insert(pfn, 1);
        Ok(pfn)
    }

    fn free_table(&mut self, pfn: u64) {
        let mut state = self.allocator.state.borrow_mut();
        state.pages.remove(&pfn);
        state.table_references.remove(&pfn);
    }

    fn retain_table(&mut self, pfn: u64) -> bool {
        let mut state = self.allocator.state.borrow_mut();
        let Some(references) = state.table_references.get_mut(&pfn) else {
            return false;
        };
        *references = references.saturating_add(1);
        true
    }

    fn release_table(&mut self, pfn: u64) {
        let mut state = self.allocator.state.borrow_mut();
        let Some(references) = state.table_references.get_mut(&pfn) else {
            return;
        };
        if *references > 1 {
            *references -= 1;
            return;
        }
        state.table_references.remove(&pfn);
        state.pages.remove(&pfn);
    }

    fn table_is_shared(&self, pfn: u64) -> bool {
        self.allocator
            .state
            .borrow()
            .table_references
            .get(&pfn)
            .copied()
            .is_some_and(|references| references > 1)
    }

    fn pfn_address(&self, pfn: u64) -> Option<usize> {
        usize::try_from(pfn.checked_mul(AARCH64_PAGE_SIZE as u64)?).ok()
    }

    fn read_table_entry(&self, pfn: u64, index: usize) -> Option<u64> {
        self.allocator
            .state
            .borrow()
            .pages
            .get(&pfn)
            .and_then(|table| table.get(index))
            .copied()
    }

    fn write_table_entry(&mut self, pfn: u64, index: usize, descriptor: u64) -> bool {
        let mut state = self.allocator.state.borrow_mut();
        let Some(entry) = state
            .pages
            .get_mut(&pfn)
            .and_then(|table| table.get_mut(index))
        else {
            return false;
        };
        *entry = descriptor;
        true
    }

    fn copy_to_physical(&self, physical: usize, bytes: &[u8]) -> bool {
        let pfn = (physical / AARCH64_PAGE_SIZE) as u64;
        let offset = physical & (AARCH64_PAGE_SIZE - 1);
        let mut state = self.allocator.state.borrow_mut();
        let Some(page) = state.data_pages.get_mut(&pfn) else {
            return false;
        };
        let Some(destination) = page.get_mut(offset..offset + bytes.len()) else {
            return false;
        };
        destination.copy_from_slice(bytes);
        true
    }

    fn copy_from_physical(&self, physical: usize, out: &mut [u8]) -> bool {
        let pfn = (physical / AARCH64_PAGE_SIZE) as u64;
        let offset = physical & (AARCH64_PAGE_SIZE - 1);
        let state = self.allocator.state.borrow();
        let Some(source) = state
            .data_pages
            .get(&pfn)
            .and_then(|page| page.get(offset..offset + out.len()))
        else {
            return false;
        };
        out.copy_from_slice(source);
        true
    }

    fn physical_page_accessible(&self, pfn: u64) -> bool {
        self.allocator.state.borrow().data_pages.contains_key(&pfn)
    }

    fn publish_user_mapping(&mut self, vaddr: usize) {
        self.allocator
            .state
            .borrow_mut()
            .maintenance_events
            .push(Aarch64MaintenanceEvent::Publish(vaddr));
    }

    fn break_user_mapping(&mut self, vaddr: usize) {
        self.allocator
            .state
            .borrow_mut()
            .maintenance_events
            .push(Aarch64MaintenanceEvent::Break(vaddr));
    }

    fn complete_user_mapping(&mut self) {
        self.allocator
            .state
            .borrow_mut()
            .maintenance_events
            .push(Aarch64MaintenanceEvent::Make);
    }
}

#[cfg(test)]
pub(crate) struct Aarch64AddressSpaceModel {
    core: Aarch64AddressSpaceCore<Aarch64TestBackend>,
}

#[cfg(test)]
impl Aarch64AddressSpaceModel {
    pub(crate) fn new(allocator: &mut Aarch64TestAllocator) -> Option<Self> {
        Some(Self {
            core: Aarch64AddressSpaceCore::new(Aarch64TestBackend {
                allocator: allocator.clone(),
            })
            .ok()?,
        })
    }

    pub(crate) fn root_pfn(&self) -> u64 {
        self.core.root_pfn()
    }

    pub(crate) fn table_page_count(&self) -> usize {
        self.core.table_page_count()
    }

    pub(crate) fn map_supervisor_ram_range(&mut self, start: usize, len: usize) -> Result<(), ()> {
        self.core
            .map_supervisor_ram_range(start, len, true)
            .map_err(|_| ())
    }

    pub(crate) fn map_user_page(
        &mut self,
        _allocator: &mut Aarch64TestAllocator,
        vaddr: usize,
        pfn: u64,
        readable: bool,
        writable: bool,
        executable: bool,
    ) -> Result<(), ()> {
        self.core
            .map_user_page(vaddr, pfn, readable, writable, executable)
            .map_err(|_| ())
    }

    pub(crate) fn map_user_region(
        &mut self,
        _allocator: &mut Aarch64TestAllocator,
        vaddr: usize,
        first_pfn: u64,
        page_count: usize,
        readable: bool,
        writable: bool,
        executable: bool,
    ) -> Result<(), ()> {
        self.core
            .map_user_region(vaddr, first_pfn, page_count, readable, writable, executable)
            .map_err(|_| ())
    }

    pub(crate) fn map_user_pages_with_protection(
        &mut self,
        _allocator: &mut Aarch64TestAllocator,
        vaddr: usize,
        pages: &[u64],
        mut protection: impl FnMut(usize) -> (bool, bool, bool),
    ) -> Result<(), ()> {
        self.core
            .map_user_pages_with_protection(
                vaddr,
                pages.len(),
                |index| pages[index],
                |index| protection(index),
            )
            .map_err(|_| ())
    }

    pub(crate) fn clone_mappings_from(
        &mut self,
        source: &Self,
    ) -> Result<(), ()> {
        self.core.clone_mappings_from(&source.core).map_err(|_| ())
    }

    pub(crate) fn clone_shared_mappings_from(
        &mut self,
        source: &Self,
    ) -> Result<(), ()> {
        self.core
            .clone_shared_mappings_from(&source.core)
            .map_err(|_| ())
    }

    pub(crate) fn protect_user_page(
        &mut self,
        _allocator: &mut Aarch64TestAllocator,
        vaddr: usize,
        readable: bool,
        writable: bool,
        executable: bool,
    ) -> Result<(), ()> {
        self.core
            .protect_user_page(vaddr, readable, writable, executable)
            .map_err(|_| ())
    }

    pub(crate) fn translate_user(
        &self,
        _allocator: &Aarch64TestAllocator,
        vaddr: usize,
        write: bool,
    ) -> Option<usize> {
        self.core.translate_user(vaddr, write)
    }

    pub(crate) fn unmap_user_page(
        &mut self,
        _allocator: &mut Aarch64TestAllocator,
        vaddr: usize,
    ) -> Result<u64, ()> {
        self.core.unmap_user_page(vaddr).map_err(|_| ())
    }

    pub(crate) fn copy_to_user(
        &self,
        _allocator: &mut Aarch64TestAllocator,
        vaddr: usize,
        bytes: &[u8],
    ) -> Result<(), ()> {
        self.core.copy_to_user(vaddr, bytes).map_err(|_| ())
    }

    pub(crate) fn copy_from_user(
        &self,
        _allocator: &Aarch64TestAllocator,
        vaddr: usize,
        out: &mut [u8],
    ) -> Result<(), ()> {
        self.core.copy_from_user(vaddr, out).map_err(|_| ())
    }
}
