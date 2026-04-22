use vstd::prelude::*;

verus! {

pub const PAGE_SIZE: usize = 4096;
pub const LINUX_MAPPING_BASE: usize = 0x5000_0000;
pub const LINUX_MAPPING_LIMIT: usize = 0x6000_0000;

spec fn checked_end_spec(addr: int, len: int) -> Option<int> {
    if 0 <= addr && 0 <= len && addr <= usize::MAX as int - len {
        Some(addr + len)
    } else {
        Option::<int>::None
    }
}

spec fn range_overlaps_spec(start_a: int, len_a: int, start_b: int, len_b: int) -> bool {
    match (checked_end_spec(start_a, len_a), checked_end_spec(start_b, len_b)) {
        (Some(end_a), Some(end_b)) => start_a < end_b && start_b < end_a,
        _ => false,
    }
}

spec fn range_within_window_spec(addr: int, len: int, base: int, limit: int) -> bool {
    match checked_end_spec(addr, len) {
        Some(end) => addr >= base && end <= limit,
        None => false,
    }
}

spec fn page_aligned_spec(addr: int) -> bool {
    addr % PAGE_SIZE as int == 0
}

spec fn fixed_linux_mmap_request_ok_spec(addr: int, len: int) -> bool {
    page_aligned_spec(addr)
        && range_within_window_spec(
            addr,
            len,
            LINUX_MAPPING_BASE as int,
            LINUX_MAPPING_LIMIT as int,
        )
}

fn checked_end(addr: usize, len: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(end) => checked_end_spec(addr as int, len as int) == Some(end as int),
            None => checked_end_spec(addr as int, len as int) == Option::<int>::None,
        },
{
    if addr <= usize::MAX - len {
        Some(addr + len)
    } else {
        None
    }
}

fn range_overlaps(start_a: usize, len_a: usize, start_b: usize, len_b: usize) -> (out: bool)
    ensures
        out == range_overlaps_spec(start_a as int, len_a as int, start_b as int, len_b as int),
{
    match (checked_end(start_a, len_a), checked_end(start_b, len_b)) {
        (Some(end_a), Some(end_b)) => start_a < end_b && start_b < end_a,
        _ => false,
    }
}

fn range_within_window(addr: usize, len: usize, base: usize, limit: usize) -> (out: bool)
    ensures
        out == range_within_window_spec(addr as int, len as int, base as int, limit as int),
{
    match checked_end(addr, len) {
        Some(end) => addr >= base && end <= limit,
        None => false,
    }
}

fn page_aligned(addr: usize) -> (out: bool)
    ensures
        out == page_aligned_spec(addr as int),
{
    addr % PAGE_SIZE == 0
}

fn fixed_linux_mmap_request_ok(addr: usize, len: usize) -> (out: bool)
    ensures
        out == fixed_linux_mmap_request_ok_spec(addr as int, len as int),
{
    page_aligned(addr) && range_within_window(addr, len, LINUX_MAPPING_BASE, LINUX_MAPPING_LIMIT)
}

proof fn checked_end_spec_returns_sum_when_it_fits(addr: int, len: int)
    requires
        0 <= addr,
        0 <= len,
        addr <= usize::MAX as int - len,
    ensures
        checked_end_spec(addr, len) == Some(addr + len),
{
}

proof fn checked_end_spec_rejects_overflow(addr: int, len: int)
    requires
        0 <= addr,
        0 <= len,
        addr > usize::MAX as int - len,
    ensures
        checked_end_spec(addr, len) == Option::<int>::None,
{
}

proof fn range_overlaps_is_symmetric(start_a: int, len_a: int, start_b: int, len_b: int)
    requires
        0 <= start_a,
        0 <= len_a,
        0 <= start_b,
        0 <= len_b,
    ensures
        range_overlaps_spec(start_a, len_a, start_b, len_b) == range_overlaps_spec(start_b, len_b, start_a, len_a),
{
}

proof fn overflowing_ranges_are_not_treated_as_window_members(addr: int, len: int, base: int, limit: int)
    requires
        0 <= addr,
        0 <= len,
        addr > usize::MAX as int - len,
    ensures
        !range_within_window_spec(addr, len, base, limit),
{
}

proof fn fixed_linux_mmap_request_stays_inside_linux_window(addr: int, len: int)
    requires
        fixed_linux_mmap_request_ok_spec(addr, len),
    ensures
        addr >= LINUX_MAPPING_BASE as int,
{
}

proof fn syscall_address_logic_smoke() {
    assert(page_aligned_spec(0));
    assert(page_aligned_spec(PAGE_SIZE as int));
    assert(!page_aligned_spec(4095));

    assert(range_within_window_spec(
        LINUX_MAPPING_BASE as int,
        PAGE_SIZE as int,
        LINUX_MAPPING_BASE as int,
        LINUX_MAPPING_LIMIT as int,
    ));
    assert(!range_within_window_spec(
        usize::MAX as int,
        PAGE_SIZE as int,
        LINUX_MAPPING_BASE as int,
        LINUX_MAPPING_LIMIT as int,
    ));

    assert(range_overlaps_spec(
        LINUX_MAPPING_BASE as int,
        PAGE_SIZE as int,
        LINUX_MAPPING_BASE as int,
        PAGE_SIZE as int,
    ));
    assert(!range_overlaps_spec(
        LINUX_MAPPING_BASE as int,
        PAGE_SIZE as int,
        0x5000_1000,
        PAGE_SIZE as int,
    ));
}

} // verus!
