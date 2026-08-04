include!("address_logic_shared.rs");

pub(crate) fn checked_end(addr: usize, len: usize) -> Option<usize> {
    smros_checked_end_body!(addr, len)
}

pub(crate) fn range_overlaps(start_a: usize, len_a: usize, start_b: usize, len_b: usize) -> bool {
    smros_range_overlaps_body!(start_a, len_a, start_b, len_b)
}

pub(crate) fn range_within_window(addr: usize, len: usize, base: usize, limit: usize) -> bool {
    smros_range_within_window_body!(addr, len, base, limit)
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

pub(crate) fn linux_user_range_writable(
    addr: usize,
    len: usize,
    ranges: impl IntoIterator<Item = (usize, usize, bool)>,
) -> bool {
    smros_linux_user_range_writable_body!(addr, len, ranges)
}

pub(crate) fn linux_user_range_readable(
    addr: usize,
    len: usize,
    ranges: impl IntoIterator<Item = (usize, usize, bool, bool)>,
) -> bool {
    smros_linux_user_range_readable_body!(addr, len, ranges)
}
