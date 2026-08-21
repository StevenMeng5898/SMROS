include!("address_logic_shared.rs");

pub(crate) fn checked_end(addr: usize, len: usize) -> Option<usize> {
    smros_checked_end_body!(addr, len)
}

pub(crate) fn page_aligned(addr: usize, page_size: usize) -> bool {
    smros_page_aligned_body!(addr, page_size)
}

pub(crate) fn fixed_linux_mmap_request_ok(
    addr: usize,
    len: usize,
    page_size: usize,
    base: usize,
    limit: usize,
) -> bool {
    smros_fixed_linux_mmap_request_ok_body!(addr, len, page_size, base, limit)
}

pub(crate) fn regular_file_mmap_span_ok(offset: u64, len: usize, offset_max: u64) -> bool {
    smros_regular_file_mmap_span_ok_body!(offset, len, offset_max)
}
