use vstd::prelude::*;

verus! {

include!("../../../src/syscall/address_logic_shared.rs");
include!("../../../src/syscall/syscall_bridge_shared.rs");
include!("../../../src/syscall/syscall_logic_shared.rs");

pub const PAGE_SIZE: usize = 4096;
pub const LINUX_MAPPING_BASE: usize = 0x5000_0000;
pub const LINUX_MAPPING_LIMIT: usize = 0x6000_0000;
pub const SMROS_SYSCALL_LINUX_THRESHOLD_U64: u64 =
    smros_syscall_linux_threshold_u64!();
pub const SMROS_ZIRCON_SYSCALL_BASE_U64: u64 = SMROS_SYSCALL_LINUX_THRESHOLD_U64;
pub const SMROS_INVALID_HANDLE_U32: u32 = 0xFFFF_FFFF;
pub const SMROS_SAVED_REG_FRAME_BYTES: usize = smros_saved_reg_frame_bytes!();
pub const SMROS_SAVED_REG_WORDS: usize = smros_saved_reg_words!();
pub const SMROS_SAVED_REG_COUNT: usize = smros_saved_reg_count!();
pub const SMROS_SYSCALL_NUMBER_REG_INDEX: usize = smros_syscall_number_reg_index!();
pub const SMROS_SYSCALL_ARG_COUNT_LINUX: usize = smros_syscall_arg_count_linux!();
pub const SMROS_SYSCALL_ARG_COUNT_ZIRCON: usize = smros_syscall_arg_count_zircon!();

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

spec fn linux_syscall_number_spec(syscall_num: int) -> bool {
    0 <= syscall_num && syscall_num < SMROS_SYSCALL_LINUX_THRESHOLD_U64 as int
}

spec fn linux_errno_return_spec(errno: int) -> int {
    if errno == 0 {
        0
    } else {
        u64::MAX as int + 1 - errno
    }
}

spec fn linux_sys_result_return_spec(is_ok: bool, value: int, errno: int) -> int {
    if is_ok {
        value
    } else {
        linux_errno_return_spec(errno)
    }
}

spec fn saved_reg_arg_spec(regs: Seq<u64>, idx: int) -> int {
    regs[idx] as int
}

spec fn zircon_syscall_number_spec(syscall_num: int) -> bool {
    SMROS_ZIRCON_SYSCALL_BASE_U64 as int <= syscall_num
        && syscall_num <= SMROS_ZIRCON_SYSCALL_BASE_U64 as int + u32::MAX as int
}

spec fn zircon_syscall_from_raw_spec(syscall_num: int) -> int {
    if zircon_syscall_number_spec(syscall_num) {
        syscall_num - SMROS_ZIRCON_SYSCALL_BASE_U64 as int
    } else {
        u32::MAX as int
    }
}

spec fn handle_invalid_spec(handle: int, invalid_handle: int) -> bool {
    handle == 0 || handle == invalid_handle
}

spec fn user_buffer_valid_spec(ptr: int, len: int) -> bool {
    len == 0 || ptr != 0
}

spec fn channel_buffers_valid_spec(
    bytes_ptr: int,
    bytes_len: int,
    handles_ptr: int,
    handles_len: int,
) -> bool {
    user_buffer_valid_spec(bytes_ptr, bytes_len)
        && user_buffer_valid_spec(handles_ptr, handles_len)
}

spec fn signal_update_spec(current: u32, clear_mask: u32, set_mask: u32) -> u32 {
    (current & !clear_mask) | set_mask
}

spec fn wait_satisfied_spec(observed: u32, requested: u32) -> bool {
    requested == 0 || (observed & requested) != 0
}

spec fn linux_clock_id_supported_spec(clock_id: int) -> bool {
    0 <= clock_id && clock_id <= 1
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

fn is_linux_syscall_number(syscall_num: u64) -> (out: bool)
    ensures
        out == linux_syscall_number_spec(syscall_num as int),
{
    smros_is_linux_syscall_number_u64_body!(syscall_num)
}

fn is_zircon_syscall_number(syscall_num: u64) -> (out: bool)
    ensures
        out == zircon_syscall_number_spec(syscall_num as int),
{
    smros_is_zircon_syscall_number_body!(syscall_num, SMROS_ZIRCON_SYSCALL_BASE_U64)
}

fn zircon_syscall_from_raw(syscall_num: u64) -> (out: u32)
    ensures
        out as int == zircon_syscall_from_raw_spec(syscall_num as int),
{
    smros_zircon_syscall_from_raw_body!(syscall_num, SMROS_ZIRCON_SYSCALL_BASE_U64)
}

fn handle_invalid(handle: u32, invalid_handle: u32) -> (out: bool)
    ensures
        out == handle_invalid_spec(handle as int, invalid_handle as int),
{
    smros_syscall_handle_invalid_body!(handle, invalid_handle)
}

fn user_buffer_valid(ptr: usize, len: usize) -> (out: bool)
    ensures
        out == user_buffer_valid_spec(ptr as int, len as int),
{
    smros_syscall_user_buffer_valid_body!(ptr, len)
}

fn channel_buffers_valid(
    bytes_ptr: usize,
    bytes_len: usize,
    handles_ptr: usize,
    handles_len: usize,
) -> (out: bool)
    ensures
        out == channel_buffers_valid_spec(
            bytes_ptr as int,
            bytes_len as int,
            handles_ptr as int,
            handles_len as int,
        ),
{
    smros_syscall_channel_buffers_valid_body!(bytes_ptr, bytes_len, handles_ptr, handles_len)
}

fn signal_update(current: u32, clear_mask: u32, set_mask: u32) -> (out: u32)
    ensures
        out == signal_update_spec(current, clear_mask, set_mask),
{
    smros_syscall_signal_update_body!(current, clear_mask, set_mask)
}

fn wait_satisfied(observed: u32, requested: u32) -> (out: bool)
    ensures
        out == wait_satisfied_spec(observed, requested),
{
    smros_syscall_wait_satisfied_body!(observed, requested)
}

fn linux_clock_id_supported(clock_id: usize) -> (out: bool)
    ensures
        out == linux_clock_id_supported_spec(clock_id as int),
{
    smros_linux_clock_id_supported_body!(clock_id)
}

fn linux_errno_code_to_u64(errno: u32) -> (out: u64)
    ensures
        out as int == linux_errno_return_spec(errno as int),
{
    smros_linux_errno_to_u64_body!(errno)
}

fn linux_sys_result_to_u64_model(is_ok: bool, value: usize, errno: u32) -> (out: u64)
    ensures
        out as int == linux_sys_result_return_spec(is_ok, value as int, errno as int),
{
    if is_ok {
        value as u64
    } else {
        linux_errno_code_to_u64(errno)
    }
}

fn syscall_num_from_regs(regs: &Vec<u64>) -> (out: u64)
    requires
        regs.len() == SMROS_SAVED_REG_COUNT,
    ensures
        out == regs@[SMROS_SYSCALL_NUMBER_REG_INDEX as int],
{
    assert(SMROS_SYSCALL_NUMBER_REG_INDEX < regs.len());
    smros_syscall_num_from_regs_body!(regs)
}

fn syscall_arg_from_reg(regs: &Vec<u64>, idx: usize) -> (out: usize)
    requires
        regs.len() == SMROS_SAVED_REG_COUNT,
        idx < SMROS_SYSCALL_ARG_COUNT_ZIRCON,
        regs@[idx as int] as int <= usize::MAX as int,
    ensures
        out as int == saved_reg_arg_spec(regs@, idx as int),
{
    assert(SMROS_SYSCALL_ARG_COUNT_ZIRCON <= SMROS_SAVED_REG_COUNT);
    assert(idx < regs.len());
    smros_syscall_arg_from_reg_body!(regs, idx)
}

fn syscall_arg_from_u64(arg: u64) -> (out: usize)
    requires
        arg as int <= usize::MAX as int,
    ensures
        out as int == arg as int,
{
    smros_syscall_arg_from_u64_body!(arg)
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

proof fn syscall_bridge_layout_matches_exception_frame()
    ensures
        SMROS_SAVED_REG_FRAME_BYTES == SMROS_SAVED_REG_COUNT * 8,
        SMROS_SAVED_REG_WORDS == SMROS_SAVED_REG_COUNT,
        SMROS_SYSCALL_ARG_COUNT_LINUX == 6,
        SMROS_SYSCALL_ARG_COUNT_ZIRCON == 8,
        SMROS_SYSCALL_ARG_COUNT_LINUX <= SMROS_SYSCALL_ARG_COUNT_ZIRCON,
        SMROS_SYSCALL_ARG_COUNT_ZIRCON <= SMROS_SYSCALL_NUMBER_REG_INDEX,
        SMROS_SYSCALL_NUMBER_REG_INDEX < SMROS_SAVED_REG_COUNT,
{
}

proof fn syscall_bridge_route_smoke() {
    assert(linux_syscall_number_spec(0));
    assert(linux_syscall_number_spec(999));
    assert(!linux_syscall_number_spec(1000));
    assert(!linux_syscall_number_spec(u64::MAX as int));

    assert(linux_errno_return_spec(0) == 0);
    assert(linux_errno_return_spec(38) == u64::MAX as int + 1 - 38);
}

proof fn syscall_zircon_logic_smoke() {
    assert(!zircon_syscall_number_spec(999));
    assert(zircon_syscall_number_spec(1000));
    assert(zircon_syscall_number_spec(1000 + u32::MAX as int));
    assert(!zircon_syscall_number_spec(1001 + u32::MAX as int));

    assert(zircon_syscall_from_raw_spec(1000) == 0);
    assert(zircon_syscall_from_raw_spec(1001) == 1);
    assert(zircon_syscall_from_raw_spec(999) == u32::MAX as int);

    assert(handle_invalid_spec(0, SMROS_INVALID_HANDLE_U32 as int));
    assert(handle_invalid_spec(
        SMROS_INVALID_HANDLE_U32 as int,
        SMROS_INVALID_HANDLE_U32 as int,
    ));
    assert(!handle_invalid_spec(1, SMROS_INVALID_HANDLE_U32 as int));

    assert(user_buffer_valid_spec(0, 0));
    assert(!user_buffer_valid_spec(0, 1));
    assert(user_buffer_valid_spec(0x1000, 1));
    assert(channel_buffers_valid_spec(0x1000, 8, 0, 0));
    assert(!channel_buffers_valid_spec(0x1000, 8, 0, 1));

    assert(wait_satisfied_spec(0, 0));

    assert(linux_clock_id_supported_spec(0));
    assert(linux_clock_id_supported_spec(1));
    assert(!linux_clock_id_supported_spec(2));
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
