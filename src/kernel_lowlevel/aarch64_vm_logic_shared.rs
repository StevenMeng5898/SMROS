pub(crate) const AARCH64_PAGE_SIZE: usize = 0x1000;
pub(crate) const AARCH64_VA_BITS: usize = 39;
pub(crate) const AARCH64_TABLE_ENTRIES: usize = 512;
pub(crate) const AARCH64_USER_BASE: usize = 0x1000_0000;
pub(crate) const AARCH64_USER_LIMIT: usize = 0x2000_0000;
pub(crate) const AARCH64_DESC_VALID: u64 = 1;
pub(crate) const AARCH64_DESC_TABLE_OR_PAGE: u64 = 2;
pub(crate) const AARCH64_DESC_AF: u64 = 1 << 10;
pub(crate) const AARCH64_DESC_AP_USER: u64 = 1 << 6;
pub(crate) const AARCH64_DESC_AP_READ_ONLY: u64 = 1 << 7;
pub(crate) const AARCH64_DESC_INNER_SHAREABLE: u64 = 3 << 8;
pub(crate) const AARCH64_DESC_PXN: u64 = 1 << 53;
pub(crate) const AARCH64_DESC_UXN: u64 = 1 << 54;
pub(crate) const AARCH64_DESC_ADDR_MASK: u64 = 0x0000_ffff_ffff_f000;

pub(crate) fn aarch64_table_descriptor(paddr: u64) -> u64 {
    (paddr & AARCH64_DESC_ADDR_MASK) | AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE
}

pub(crate) fn aarch64_user_page_descriptor(
    paddr: u64,
    readable: bool,
    writable: bool,
    executable: bool,
) -> u64 {
    let mut descriptor = (paddr & AARCH64_DESC_ADDR_MASK)
        | AARCH64_DESC_VALID
        | AARCH64_DESC_TABLE_OR_PAGE
        | AARCH64_DESC_AF
        | AARCH64_DESC_AP_USER
        | AARCH64_DESC_INNER_SHAREABLE
        | AARCH64_DESC_PXN;
    if readable && !writable {
        descriptor |= AARCH64_DESC_AP_READ_ONLY;
    }
    if !executable {
        descriptor |= AARCH64_DESC_UXN;
    }
    descriptor
}

pub(crate) fn aarch64_supervisor_block_descriptor(
    paddr: u64,
    device: bool,
    executable: bool,
) -> u64 {
    let mut descriptor = (paddr & AARCH64_DESC_ADDR_MASK) | AARCH64_DESC_VALID | AARCH64_DESC_AF;
    if device {
        descriptor |= 1 << 2;
    } else {
        descriptor |= AARCH64_DESC_INNER_SHAREABLE;
    }
    if executable {
        descriptor |= AARCH64_DESC_UXN;
    } else {
        descriptor |= AARCH64_DESC_PXN | AARCH64_DESC_UXN;
    }
    descriptor
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
    (start < end).then_some((start, end))
}

pub(crate) fn aarch64_frame_range_cap(
    start: usize,
    end: usize,
    capacity_bytes: usize,
) -> Option<(usize, usize)> {
    let capped_end = core::cmp::min(end, start.checked_add(capacity_bytes).unwrap_or(usize::MAX));
    (start < capped_end).then_some((start, capped_end))
}

#[cfg(test)]
pub(crate) struct Aarch64TestAllocator {
    next_pfn: u64,
    pages: alloc::collections::BTreeMap<u64, [u64; AARCH64_TABLE_ENTRIES]>,
}

#[cfg(test)]
impl Aarch64TestAllocator {
    pub(crate) fn new(next_pfn: u64) -> Self {
        Self {
            next_pfn,
            pages: alloc::collections::BTreeMap::new(),
        }
    }

    fn alloc(&mut self) -> u64 {
        let pfn = self.next_pfn;
        self.next_pfn += 1;
        self.pages.insert(pfn, [0; AARCH64_TABLE_ENTRIES]);
        pfn
    }

    fn free(&mut self, pfn: u64) {
        self.pages.remove(&pfn);
    }

    fn table(&self, pfn: u64) -> &[u64; AARCH64_TABLE_ENTRIES] {
        self.pages.get(&pfn).expect("allocated test table")
    }

    fn table_mut(&mut self, pfn: u64) -> &mut [u64; AARCH64_TABLE_ENTRIES] {
        self.pages.get_mut(&pfn).expect("allocated test table")
    }

    pub(crate) fn allocated_pages(&self) -> usize {
        self.pages.len()
    }
}

#[cfg(test)]
pub(crate) struct Aarch64AddressSpaceModel {
    root_pfn: u64,
    table_pfns: alloc::vec::Vec<u64>,
}

#[cfg(test)]
impl Aarch64AddressSpaceModel {
    pub(crate) fn new(allocator: &mut Aarch64TestAllocator) -> Option<Self> {
        let root_pfn = allocator.alloc();
        Some(Self {
            root_pfn,
            table_pfns: alloc::vec![root_pfn],
        })
    }

    pub(crate) fn root_pfn(&self) -> u64 {
        self.root_pfn
    }

    pub(crate) fn map_user_page(
        &mut self,
        allocator: &mut Aarch64TestAllocator,
        vaddr: usize,
        pfn: u64,
        readable: bool,
        writable: bool,
        executable: bool,
    ) -> Result<(), ()> {
        let indices = aarch64_table_indices(vaddr).ok_or(())?;
        if !aarch64_user_range_valid(vaddr, AARCH64_PAGE_SIZE)
            || !readable
            || (writable && !readable)
        {
            return Err(());
        }
        let mut table_pfn = self.root_pfn;
        for index in indices[..2].iter().copied() {
            let descriptor = allocator.table(table_pfn)[index];
            if descriptor == 0 {
                let child_pfn = allocator.alloc();
                self.table_pfns.push(child_pfn);
                allocator.table_mut(table_pfn)[index] =
                    aarch64_table_descriptor(child_pfn * AARCH64_PAGE_SIZE as u64);
                table_pfn = child_pfn;
            } else {
                table_pfn = (descriptor & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64;
            }
        }
        if allocator.table(table_pfn)[indices[2]] != 0 {
            return Err(());
        }
        allocator.table_mut(table_pfn)[indices[2]] = aarch64_user_page_descriptor(
            pfn * AARCH64_PAGE_SIZE as u64,
            readable,
            writable,
            executable,
        );
        Ok(())
    }

    pub(crate) fn translate_user(
        &self,
        allocator: &Aarch64TestAllocator,
        vaddr: usize,
        write: bool,
    ) -> Option<usize> {
        let indices = aarch64_table_indices(vaddr)?;
        let mut table_pfn = self.root_pfn;
        for index in indices[..2].iter().copied() {
            let descriptor = allocator.table(table_pfn)[index];
            if descriptor & (AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE)
                != AARCH64_DESC_VALID | AARCH64_DESC_TABLE_OR_PAGE
            {
                return None;
            }
            table_pfn = (descriptor & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64;
        }
        let descriptor = allocator.table(table_pfn)[indices[2]];
        if descriptor & AARCH64_DESC_VALID == 0
            || descriptor & AARCH64_DESC_AP_USER == 0
            || (write && descriptor & AARCH64_DESC_AP_READ_ONLY != 0)
        {
            return None;
        }
        Some((descriptor & AARCH64_DESC_ADDR_MASK) as usize | (vaddr & (AARCH64_PAGE_SIZE - 1)))
    }

    pub(crate) fn unmap_user_page(
        &mut self,
        allocator: &mut Aarch64TestAllocator,
        vaddr: usize,
    ) -> Result<u64, ()> {
        let indices = aarch64_table_indices(vaddr).ok_or(())?;
        let mut table_pfn = self.root_pfn;
        for index in indices[..2].iter().copied() {
            let descriptor = allocator.table(table_pfn)[index];
            if descriptor & AARCH64_DESC_VALID == 0 {
                return Err(());
            }
            table_pfn = (descriptor & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64;
        }
        let descriptor = core::mem::take(&mut allocator.table_mut(table_pfn)[indices[2]]);
        if descriptor & AARCH64_DESC_VALID == 0 {
            return Err(());
        }
        Ok((descriptor & AARCH64_DESC_ADDR_MASK) / AARCH64_PAGE_SIZE as u64)
    }

    pub(crate) fn destroy(mut self, allocator: &mut Aarch64TestAllocator) {
        for pfn in self.table_pfns.drain(..) {
            allocator.free(pfn);
        }
    }
}
