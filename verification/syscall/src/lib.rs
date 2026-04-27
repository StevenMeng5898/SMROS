use vstd::prelude::*;

verus! {

include!("../../../src/syscall/address_logic_shared.rs");

pub const PAGE_SIZE: usize = 4096;
pub const LINUX_MAPPING_BASE: usize = 0x5000_0000;
pub const LINUX_MAPPING_LIMIT: usize = 0x6000_0000;

#[derive(Copy, Clone)]
struct LinuxRange {
    addr: usize,
    len: usize,
}

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

spec fn range_ends_before_spec(addr: int, len: int, bound: int) -> bool {
    match checked_end_spec(addr, len) {
        Some(end) => end <= bound,
        None => false,
    }
}

spec fn no_overlap_with_mappings_spec(addr: int, len: int, mappings: Seq<LinuxRange>) -> bool {
    forall|i: int|
        0 <= i < mappings.len() ==> !range_overlaps_spec(
            addr,
            len,
            mappings[i].addr as int,
            mappings[i].len as int,
        )
}

spec fn linux_range_available_spec(addr: int, len: int, mappings: Seq<LinuxRange>) -> bool {
    range_within_window_spec(
        addr,
        len,
        LINUX_MAPPING_BASE as int,
        LINUX_MAPPING_LIMIT as int,
    ) && no_overlap_with_mappings_spec(addr, len, mappings)
}

fn checked_end(addr: usize, len: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(end) => checked_end_spec(addr as int, len as int) == Some(end as int),
            None => checked_end_spec(addr as int, len as int) == Option::<int>::None,
        },
{
    smros_checked_end_body!(addr, len)
}

fn range_overlaps(start_a: usize, len_a: usize, start_b: usize, len_b: usize) -> (out: bool)
    ensures
        out == range_overlaps_spec(start_a as int, len_a as int, start_b as int, len_b as int),
{
    smros_range_overlaps_body!(start_a, len_a, start_b, len_b)
}

fn range_within_window(addr: usize, len: usize, base: usize, limit: usize) -> (out: bool)
    ensures
        out == range_within_window_spec(addr as int, len as int, base as int, limit as int),
{
    smros_range_within_window_body!(addr, len, base, limit)
}

fn page_aligned(addr: usize) -> (out: bool)
    ensures
        out == page_aligned_spec(addr as int),
{
    smros_page_aligned_body!(addr, PAGE_SIZE)
}

fn fixed_linux_mmap_request_ok(addr: usize, len: usize) -> (out: bool)
    ensures
        out == fixed_linux_mmap_request_ok_spec(addr as int, len as int),
{
    smros_fixed_linux_mmap_request_ok_body!(
        addr,
        len,
        PAGE_SIZE,
        LINUX_MAPPING_BASE,
        LINUX_MAPPING_LIMIT
    )
}

fn no_overlap_with_mappings(addr: usize, len: usize, mappings: &Vec<LinuxRange>) -> (out: bool)
    ensures
        out == no_overlap_with_mappings_spec(addr as int, len as int, mappings@),
{
    let mut i = 0usize;
    while i < mappings.len()
        invariant
            i <= mappings.len(),
            forall|j: int|
                0 <= j < i as int ==> !range_overlaps_spec(
                    addr as int,
                    len as int,
                    mappings@[j].addr as int,
                    mappings@[j].len as int,
                ),
        decreases mappings.len() - i,
    {
        let mapping = &mappings[i];
        if range_overlaps(addr, len, mapping.addr, mapping.len) {
            assert(range_overlaps_spec(
                addr as int,
                len as int,
                mappings@[i as int].addr as int,
                mappings@[i as int].len as int,
            ));
            assert(!no_overlap_with_mappings_spec(addr as int, len as int, mappings@)) by {
                assert(range_overlaps_spec(
                    addr as int,
                    len as int,
                    mappings@[i as int].addr as int,
                    mappings@[i as int].len as int,
                ));
            }
            return false;
        }
        i += 1;
    }

    assert(i == mappings.len());
    assert(no_overlap_with_mappings_spec(addr as int, len as int, mappings@)) by {
        assert forall|j: int|
            0 <= j < mappings@.len() implies !range_overlaps_spec(
                addr as int,
                len as int,
                mappings@[j].addr as int,
                mappings@[j].len as int,
            ) by {
            assert(j < i as int);
        }
    };
    true
}

fn linux_range_available(addr: usize, len: usize, mappings: &Vec<LinuxRange>) -> (out: bool)
    ensures
        out == linux_range_available_spec(addr as int, len as int, mappings@),
{
    range_within_window(addr, len, LINUX_MAPPING_BASE, LINUX_MAPPING_LIMIT)
        && no_overlap_with_mappings(addr, len, mappings)
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

proof fn range_end_before_start_means_no_overlap(
    start_a: int,
    len_a: int,
    start_b: int,
    len_b: int,
)
    requires
        0 <= start_a,
        0 <= len_a,
        0 <= start_b,
        0 <= len_b,
        range_ends_before_spec(start_a, len_a, start_b),
    ensures
        !range_overlaps_spec(start_a, len_a, start_b, len_b),
{
}

proof fn range_start_after_end_means_no_overlap(
    start_a: int,
    len_a: int,
    start_b: int,
    len_b: int,
)
    requires
        0 <= start_a,
        0 <= len_a,
        0 <= start_b,
        0 <= len_b,
        range_ends_before_spec(start_b, len_b, start_a),
    ensures
        !range_overlaps_spec(start_a, len_a, start_b, len_b),
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

proof fn linux_range_availability_smoke() {
    let first = LinuxRange {
        addr: 0x5000_1000,
        len: PAGE_SIZE,
    };
    let second = LinuxRange {
        addr: 0x5000_3000,
        len: PAGE_SIZE,
    };
    let mappings = seq![first, second];

    range_end_before_start_means_no_overlap(
        LINUX_MAPPING_BASE as int,
        PAGE_SIZE as int,
        first.addr as int,
        first.len as int,
    );
    range_end_before_start_means_no_overlap(
        LINUX_MAPPING_BASE as int,
        PAGE_SIZE as int,
        second.addr as int,
        second.len as int,
    );
    assert(linux_range_available_spec(
        LINUX_MAPPING_BASE as int,
        PAGE_SIZE as int,
        mappings,
    ));

    assert(!linux_range_available_spec(
        first.addr as int,
        PAGE_SIZE as int,
        mappings,
    ));

    range_start_after_end_means_no_overlap(
        0x5000_2000 as int,
        PAGE_SIZE as int,
        first.addr as int,
        first.len as int,
    );
    range_end_before_start_means_no_overlap(
        0x5000_2000 as int,
        PAGE_SIZE as int,
        second.addr as int,
        second.len as int,
    );
    assert(linux_range_available_spec(
        0x5000_2000 as int,
        PAGE_SIZE as int,
        mappings,
    ));

    assert(!linux_range_available_spec(
        0x4fff_f000 as int,
        PAGE_SIZE as int,
        mappings,
    ));
}

} // verus!
