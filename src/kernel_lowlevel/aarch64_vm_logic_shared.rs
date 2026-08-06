pub(crate) const AARCH64_PAGE_SIZE: usize = 0x1000;
pub(crate) const AARCH64_VA_BITS: usize = 39;
pub(crate) const AARCH64_TABLE_ENTRIES: usize = 512;
pub(crate) const AARCH64_USER_BASE: usize = 0x1000_0000;
pub(crate) const AARCH64_USER_LIMIT: usize = 0x2000_0000;

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
