use vstd::prelude::*;

verus! {

include!("../../../src/main_logic_shared.rs");
include!("../../../src/user_level/user_logic_shared.rs");

pub const KERNEL_HEAP_SIZE: usize = 0x100000;
pub const PAGE_SIZE: usize = 4096;
pub const USER_PROCESS_CAPACITY: usize = 16;
pub const USER_INIT_PARENT_PID: usize = 1;
pub const USER_CODE_VADDR: usize = 0x0000_0000;
pub const USER_DATA_VADDR: usize = 0x0000_1000;
pub const USER_HEAP_VADDR: usize = 0x0000_2000;
pub const USER_HEAP_PAGES: usize = 4;
pub const USER_STACK_VADDR: usize = 0xFFFF_0000;
pub const USER_STACK_PAGES: usize = 2;
pub const USER_THREAD_TIME_SLICE: u32 = 10;
pub const DEFAULT_STACK_SIZE: usize = 0x4000;
pub const USER_MMAP_BASE: u64 = 0x5000_0000;
pub const USER_MMAP_LIMIT: u64 = 0x6000_0000;

pub const SYS_WRITE: u32 = 64;
pub const SYS_EXIT: u32 = 93;
pub const SYS_GETPID: u32 = 172;
pub const SYS_MMAP: u32 = 222;
pub const EL0_TEST_STACK_SIZE: usize = 0x2000;
pub const EL0_TEST_BANNER_LEN: usize = 33;
pub const EL0_TEST_INFO_GETPID_LEN: usize = 27;
pub const EL0_TEST_INFO_MMAP_LEN: usize = 25;
pub const EL0_TEST_COMPLETE_LEN: usize = 35;
pub const EL0_TEST_EXIT_OK: i32 = 0;
pub const EL0_TEST_EXIT_WRITE_RESULT_MISMATCH: i32 = 10;
pub const EL0_TEST_EXIT_GETPID_RESULT_MISMATCH: i32 = 11;
pub const EL0_TEST_EXIT_GETPID_LOG_WRITE_MISMATCH: i32 = 12;
pub const EL0_TEST_EXIT_MMAP_RESULT_MISMATCH: i32 = 13;
pub const EL0_TEST_EXIT_MMAP_LOG_WRITE_MISMATCH: i32 = 14;
pub const EL0_TEST_EXIT_COMPLETE_LOG_WRITE_MISMATCH: i32 = 15;

#[derive(Copy, Clone)]
struct UserProcessSlotModel {
    pid: usize,
    occupied: bool,
}

#[derive(Copy, Clone)]
struct El0KernelObservation {
    entered: bool,
    write_result: u64,
    pid: u64,
    mmap_addr: u64,
}

spec fn checked_end_spec(addr: int, len: int) -> Option<int> {
    if 0 <= addr && 0 <= len && addr <= usize::MAX as int - len {
        Some(addr + len)
    } else {
        Option::<int>::None
    }
}

spec fn align_up_spec(pos: int, align: int) -> Option<int> {
    if pos < 0 || align <= 0 {
        Option::<int>::None
    } else {
        let offset = pos % align;
        if offset == 0 {
            Some(pos)
        } else if pos <= usize::MAX as int - (align - offset) {
            Some(pos + align - offset)
        } else {
            Option::<int>::None
        }
    }
}

spec fn bump_alloc_next_spec(pos: int, size: int, align: int, heap_size: int) -> Option<(int, int)> {
    match align_up_spec(pos, align) {
        Some(aligned_pos) => match checked_end_spec(aligned_pos, size) {
            Some(next_pos) => if next_pos <= heap_size {
                Some((aligned_pos, next_pos))
            } else {
                Option::<(int, int)>::None
            },
            None => Option::<(int, int)>::None,
        },
        None => Option::<(int, int)>::None,
    }
}

spec fn page_offset_spec(base: int, page_index: int, page_size: int) -> Option<int> {
    if 0 <= base
        && 0 <= page_index
        && 0 <= page_size
        && (page_size == 0 || page_index <= usize::MAX as int / page_size)
    {
        checked_end_spec(base, page_index * page_size)
    } else {
        Option::<int>::None
    }
}

spec fn pfn_to_paddr_spec(pfn: int, page_size: int) -> Option<int> {
    if 0 <= pfn && 0 <= page_size && (page_size == 0 || pfn <= u64::MAX as int / page_size) {
        Some(pfn * page_size)
    } else {
        Option::<int>::None
    }
}

spec fn stack_top_u64_spec(stack_base: int, stack_size: int) -> Option<int> {
    if 0 <= stack_base && 0 <= stack_size && stack_base <= u64::MAX as int - stack_size {
        Some(stack_base + stack_size)
    } else {
        Option::<int>::None
    }
}

spec fn ascii_shell_input_spec(byte: int) -> bool {
    0x20 <= byte && byte <= 0x7e
}

spec fn decimal_digit_value_spec(byte: int) -> Option<int> {
    if 48 <= byte && byte <= 57 {
        Some(byte - 48)
    } else {
        Option::<int>::None
    }
}

spec fn parse_digit_step_spec(result: int, digit: int) -> Option<int> {
    if 0 <= result
        && 0 <= digit
        && result <= usize::MAX as int / 10
        && result * 10 <= usize::MAX as int - digit
    {
        Some(result * 10 + digit)
    } else {
        Option::<int>::None
    }
}

spec fn saturating_sub_spec(lhs: int, rhs: int) -> int {
    if lhs >= rhs {
        lhs - rhs
    } else {
        0
    }
}

spec fn pages_to_kb_spec(pages: int, page_size: int) -> int {
    if 0 <= pages && 0 <= page_size && (page_size == 0 || pages <= usize::MAX as int / page_size) {
        (pages * page_size) / 1024
    } else {
        usize::MAX as int
    }
}

spec fn usage_percent_spec(used_pages: int, total_pages: int) -> int {
    if total_pages == 0 {
        0
    } else if 0 <= used_pages && used_pages <= usize::MAX as int / 100 {
        (used_pages * 100) / total_pages
    } else {
        usize::MAX as int
    }
}

spec fn uptime_parts_spec(ticks: int) -> (int, int, int, int) {
    let seconds = ticks / 100;
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    (seconds, minutes, hours, days)
}

spec fn mmap_result_ok_spec(addr: int, page_size: int, base: int, limit: int) -> bool {
    page_size != 0 && addr >= base && addr < limit && addr % page_size == 0
}

spec fn kernel_success_spec(
    kernel_entered: bool,
    kernel_finished: bool,
    exit_code: int,
    kernel_write: int,
    kernel_pid: int,
    kernel_mmap: int,
    banner_len: int,
) -> bool {
    kernel_entered
        && kernel_finished
        && exit_code == EL0_TEST_EXIT_OK as int
        && kernel_write == banner_len
        && kernel_pid == 1
        && kernel_mmap > 0
        && kernel_mmap < 0xFFFF_FFFF_FFFF_F000u64 as int
}

spec fn user_test_exit_code_spec(
    write_result: int,
    pid: int,
    getpid_log_write: int,
    mmap_addr: int,
    mmap_log_write: int,
    complete_log_write: int,
) -> int {
    if write_result != EL0_TEST_BANNER_LEN as int {
        EL0_TEST_EXIT_WRITE_RESULT_MISMATCH as int
    } else if pid != 1 {
        EL0_TEST_EXIT_GETPID_RESULT_MISMATCH as int
    } else if getpid_log_write != EL0_TEST_INFO_GETPID_LEN as int {
        EL0_TEST_EXIT_GETPID_LOG_WRITE_MISMATCH as int
    } else if !mmap_result_ok_spec(
        mmap_addr,
        PAGE_SIZE as int,
        USER_MMAP_BASE as int,
        USER_MMAP_LIMIT as int,
    ) {
        EL0_TEST_EXIT_MMAP_RESULT_MISMATCH as int
    } else if mmap_log_write != EL0_TEST_INFO_MMAP_LEN as int {
        EL0_TEST_EXIT_MMAP_LOG_WRITE_MISMATCH as int
    } else if complete_log_write != EL0_TEST_COMPLETE_LEN as int {
        EL0_TEST_EXIT_COMPLETE_LOG_WRITE_MISMATCH as int
    } else {
        EL0_TEST_EXIT_OK as int
    }
}

fn main_align_up(pos: usize, align: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(aligned) => align_up_spec(pos as int, align as int) == Some(aligned as int),
            None => align_up_spec(pos as int, align as int) == Option::<int>::None,
        },
{
    smros_main_align_up_body!(pos, align)
}

fn main_bump_alloc_next(
    pos: usize,
    size: usize,
    align: usize,
    heap_size: usize,
) -> (out: Option<(usize, usize)>)
    ensures
        match out {
            Some((aligned_pos, next_pos)) => bump_alloc_next_spec(
                pos as int,
                size as int,
                align as int,
                heap_size as int,
            ) == Some((aligned_pos as int, next_pos as int)),
            None => bump_alloc_next_spec(
                pos as int,
                size as int,
                align as int,
                heap_size as int,
            ) == Option::<(int, int)>::None,
        },
{
    smros_main_bump_alloc_next_body!(pos, size, align, heap_size)
}

fn user_checked_end(addr: usize, len: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(end) => checked_end_spec(addr as int, len as int) == Some(end as int),
            None => checked_end_spec(addr as int, len as int) == Option::<int>::None,
        },
{
    smros_user_checked_end_body!(addr, len)
}

fn user_page_offset(base: usize, page_index: usize, page_size: usize) -> (out: Option<usize>)
    requires
        page_size > 0,
        page_index <= usize::MAX / page_size,
    ensures
        match out {
            Some(vaddr) => page_offset_spec(base as int, page_index as int, page_size as int)
                == Some(vaddr as int),
            None => page_offset_spec(base as int, page_index as int, page_size as int)
                == Option::<int>::None,
        },
{
    smros_user_page_offset_body!(base, page_index, page_size)
}

fn user_pfn_to_paddr(pfn: u64, page_size: u64) -> (out: Option<u64>)
    requires
        page_size > 0,
        pfn as int <= u64::MAX as int / page_size as int,
    ensures
        match out {
            Some(paddr) => pfn_to_paddr_spec(pfn as int, page_size as int)
                == Some(paddr as int),
            None => pfn_to_paddr_spec(pfn as int, page_size as int) == Option::<int>::None,
        },
{
    let out = smros_user_pfn_to_paddr_body!(pfn, page_size);
    assert((pfn as int) * (page_size as int) <= u64::MAX as int) by (nonlinear_arith)
        requires
            page_size as int > 0,
            pfn as int <= u64::MAX as int / page_size as int,
    ;
    out
}

fn user_stack_top_u64(stack_base: u64, stack_size: usize) -> (out: Option<u64>)
    ensures
        match out {
            Some(stack_top) => stack_top_u64_spec(stack_base as int, stack_size as int)
                == Some(stack_top as int),
            None => stack_top_u64_spec(stack_base as int, stack_size as int)
                == Option::<int>::None,
        },
{
    smros_user_stack_top_u64_body!(stack_base, stack_size)
}

fn user_el0_thread_pstate() -> (out: u64)
    ensures
        out == 0x3C0u64,
{
    smros_user_el0_thread_pstate_body!()
}

fn user_el0_spsr() -> (out: u64)
    ensures
        out == 0u64,
{
    smros_user_el0_spsr_body!()
}

fn user_el1h_spsr_masked() -> (out: u64)
    ensures
        out == 0x3C5u64,
{
    smros_user_el1h_spsr_masked_body!()
}

fn user_syscall_should_advance_elr() -> (out: u64)
    ensures
        out == 0u64,
{
    smros_user_syscall_should_advance_elr_body!()
}

fn user_ascii_shell_input(byte: u8) -> (out: bool)
    ensures
        out == ascii_shell_input_spec(byte as int),
{
    smros_user_ascii_shell_input_body!(byte)
}

fn user_decimal_digit_value(byte: u8) -> (out: Option<usize>)
    ensures
        match out {
            Some(digit) => decimal_digit_value_spec(byte as int) == Some(digit as int),
            None => decimal_digit_value_spec(byte as int) == Option::<int>::None,
        },
{
    smros_user_decimal_digit_value_body!(byte)
}

fn user_parse_digit_step(result: usize, digit: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(next) => parse_digit_step_spec(result as int, digit as int) == Some(next as int),
            None => parse_digit_step_spec(result as int, digit as int) == Option::<int>::None,
        },
{
    smros_user_parse_digit_step_body!(result, digit)
}

fn user_saturating_sub(lhs: usize, rhs: usize) -> (out: usize)
    ensures
        out as int == saturating_sub_spec(lhs as int, rhs as int),
{
    smros_user_saturating_sub_body!(lhs, rhs)
}

fn user_pages_to_kb(pages: usize, page_size: usize) -> (out: usize)
    requires
        page_size > 0,
        pages as int <= usize::MAX as int / page_size as int,
    ensures
        out as int == pages_to_kb_spec(pages as int, page_size as int),
{
    let out = smros_user_pages_to_kb_body!(pages, page_size);
    assert((pages as int) * (page_size as int) <= usize::MAX as int) by (nonlinear_arith)
        requires
            page_size as int > 0,
            pages as int <= usize::MAX as int / page_size as int,
    ;
    out
}

fn user_usage_percent(used_pages: usize, total_pages: usize) -> (out: usize)
    ensures
        out as int == usage_percent_spec(used_pages as int, total_pages as int),
{
    smros_user_usage_percent_body!(used_pages, total_pages)
}

fn user_uptime_parts(ticks: u64) -> (out: (u64, u64, u64, u64))
    ensures
        out.0 as int == uptime_parts_spec(ticks as int).0,
        out.1 as int == uptime_parts_spec(ticks as int).1,
        out.2 as int == uptime_parts_spec(ticks as int).2,
        out.3 as int == uptime_parts_spec(ticks as int).3,
{
    smros_user_uptime_parts_body!(ticks)
}

fn user_mmap_result_ok(addr: u64) -> (out: bool)
    ensures
        out == mmap_result_ok_spec(
            addr as int,
            PAGE_SIZE as int,
            USER_MMAP_BASE as int,
            USER_MMAP_LIMIT as int,
        ),
{
    smros_user_mmap_result_ok_body!(addr, PAGE_SIZE as u64, USER_MMAP_BASE, USER_MMAP_LIMIT)
}

fn user_kernel_success(
    kernel_entered: bool,
    kernel_finished: bool,
    exit_code: i32,
    kernel_write: u64,
    kernel_pid: u64,
    kernel_mmap: u64,
    banner_len: usize,
) -> (out: bool)
    ensures
        out == kernel_success_spec(
            kernel_entered,
            kernel_finished,
            exit_code as int,
            kernel_write as int,
            kernel_pid as int,
            kernel_mmap as int,
            banner_len as int,
        ),
{
    smros_user_kernel_success_body!(
        kernel_entered,
        kernel_finished,
        exit_code,
        kernel_write,
        kernel_pid,
        kernel_mmap,
        banner_len
    )
}

fn find_empty_user_process_slot(slots: &Vec<UserProcessSlotModel>) -> (out: Option<usize>)
    ensures
        match out {
            Some(i) => i < slots.len()
                && !slots@[i as int].occupied
                && forall|j: int| 0 <= j < i as int ==> slots@[j].occupied,
            None => forall|i: int| 0 <= i < slots@.len() ==> slots@[i].occupied,
        },
{
    let mut i = 0usize;
    while i < slots.len()
        invariant
            i <= slots.len(),
            forall|j: int| 0 <= j < i as int ==> slots@[j].occupied,
        decreases slots.len() - i,
    {
        if !slots[i].occupied {
            return Some(i);
        }
        i += 1;
    }

    assert forall|j: int| 0 <= j < slots@.len() implies slots@[j].occupied by {
        assert(j < i as int);
    };
    None
}

fn find_user_process_by_pid(slots: &Vec<UserProcessSlotModel>, pid: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(i) => i < slots.len() && slots@[i as int].occupied && slots@[i as int].pid == pid,
            None => forall|i: int|
                0 <= i < slots@.len() ==> !(slots@[i].occupied && slots@[i].pid == pid),
        },
{
    let mut i = 0usize;
    while i < slots.len()
        invariant
            i <= slots.len(),
            forall|j: int|
                0 <= j < i as int ==> !(slots@[j].occupied && slots@[j].pid == pid),
        decreases slots.len() - i,
    {
        if slots[i].occupied && slots[i].pid == pid {
            return Some(i);
        }
        i += 1;
    }

    assert forall|j: int|
        0 <= j < slots@.len() implies !(slots@[j].occupied && slots@[j].pid == pid) by {
        assert(j < i as int);
    };
    None
}

fn user_test_exit_code_model(
    write_result: u64,
    pid: u64,
    getpid_log_write: u64,
    mmap_addr: u64,
    mmap_log_write: u64,
    complete_log_write: u64,
) -> (out: i32)
    ensures
        out as int == user_test_exit_code_spec(
            write_result as int,
            pid as int,
            getpid_log_write as int,
            mmap_addr as int,
            mmap_log_write as int,
            complete_log_write as int,
        ),
{
    if write_result != EL0_TEST_BANNER_LEN as u64 {
        EL0_TEST_EXIT_WRITE_RESULT_MISMATCH
    } else if pid != 1 {
        EL0_TEST_EXIT_GETPID_RESULT_MISMATCH
    } else if getpid_log_write != EL0_TEST_INFO_GETPID_LEN as u64 {
        EL0_TEST_EXIT_GETPID_LOG_WRITE_MISMATCH
    } else if !user_mmap_result_ok(mmap_addr) {
        EL0_TEST_EXIT_MMAP_RESULT_MISMATCH
    } else if mmap_log_write != EL0_TEST_INFO_MMAP_LEN as u64 {
        EL0_TEST_EXIT_MMAP_LOG_WRITE_MISMATCH
    } else if complete_log_write != EL0_TEST_COMPLETE_LEN as u64 {
        EL0_TEST_EXIT_COMPLETE_LOG_WRITE_MISMATCH
    } else {
        EL0_TEST_EXIT_OK
    }
}

fn record_el0_kernel_syscall_result_model(
    active: bool,
    syscall_num: u32,
    result: u64,
    old: El0KernelObservation,
) -> (out: El0KernelObservation)
    ensures
        !active ==> out.entered == old.entered
            && out.write_result == old.write_result
            && out.pid == old.pid
            && out.mmap_addr == old.mmap_addr,
        active ==> out.entered,
        active && syscall_num == SYS_WRITE && old.write_result == 0 ==> out.write_result == result,
        active && syscall_num == SYS_WRITE && old.write_result != 0 ==> out.write_result == old.write_result,
        active && syscall_num == SYS_GETPID ==> out.pid == result,
        active && syscall_num == SYS_MMAP ==> out.mmap_addr == result,
{
    if !active {
        return old;
    }

    let mut next = old;
    next.entered = true;
    if syscall_num == SYS_WRITE {
        if next.write_result == 0 {
            next.write_result = result;
        }
    } else if syscall_num == SYS_GETPID {
        next.pid = result;
    } else if syscall_num == SYS_MMAP {
        next.mmap_addr = result;
    }
    next
}

proof fn main_rs_allocator_smoke() {
    assert(align_up_spec(0int, 16int) == Some(0int));
    assert(align_up_spec(1int, 16int) == Some(16int));
    assert(bump_alloc_next_spec(1int, 8int, 16int, KERNEL_HEAP_SIZE as int) == Some((16int, 24int)));
    assert(bump_alloc_next_spec(
        KERNEL_HEAP_SIZE as int,
        1int,
        16int,
        KERNEL_HEAP_SIZE as int,
    ) == Option::<(int, int)>::None);
}

proof fn user_level_mod_has_no_extra_pure_runtime_obligation()
    ensures
        USER_PROCESS_CAPACITY == 16,
        USER_INIT_PARENT_PID == 1,
{
}

proof fn user_process_layout_smoke() {
    assert(USER_CODE_VADDR == 0);
    assert(USER_DATA_VADDR == PAGE_SIZE);
    assert(USER_HEAP_VADDR == 2 * PAGE_SIZE);
    assert(USER_HEAP_PAGES == 4);
    assert(USER_STACK_PAGES == 2);
    assert(page_offset_spec(USER_STACK_VADDR as int, USER_STACK_PAGES as int, PAGE_SIZE as int)
        == Some((USER_STACK_VADDR + USER_STACK_PAGES * PAGE_SIZE) as int));
    assert(USER_THREAD_TIME_SLICE == 10);
    assert(0x3C0u64 == smros_user_el0_thread_pstate_body!());
    assert(0u64 == smros_user_el0_spsr_body!());
}

proof fn user_shell_logic_smoke() {
    assert(ascii_shell_input_spec(0x20));
    assert(ascii_shell_input_spec(0x7e));
    assert(!ascii_shell_input_spec(0x1f));
    assert(decimal_digit_value_spec(48int) == Some(0int));
    assert(decimal_digit_value_spec(57int) == Some(9int));
    assert(decimal_digit_value_spec(65int) == Option::<int>::None);
    assert(parse_digit_step_spec(12int, 3int) == Some(123int));
    assert(saturating_sub_spec(3int, 5int) == 0);
    assert(saturating_sub_spec(8int, 3int) == 5);
    assert(pages_to_kb_spec(2int, PAGE_SIZE as int) == 8);
    assert(usage_percent_spec(25int, 100int) == 25);
    assert(uptime_parts_spec(86400int * 100int).3 == 1);
}

proof fn user_test_syscall_smoke() {
    assert(SYS_WRITE == 64);
    assert(SYS_EXIT == 93);
    assert(SYS_GETPID == 172);
    assert(SYS_MMAP == 222);
    assert(EL0_TEST_STACK_SIZE == 2 * PAGE_SIZE);
    assert(mmap_result_ok_spec(
        USER_MMAP_BASE as int,
        PAGE_SIZE as int,
        USER_MMAP_BASE as int,
        USER_MMAP_LIMIT as int,
    ));
    assert(!mmap_result_ok_spec(
        USER_MMAP_LIMIT as int,
        PAGE_SIZE as int,
        USER_MMAP_BASE as int,
        USER_MMAP_LIMIT as int,
    ));
    assert(user_test_exit_code_spec(
        EL0_TEST_BANNER_LEN as int,
        1,
        EL0_TEST_INFO_GETPID_LEN as int,
        USER_MMAP_BASE as int,
        EL0_TEST_INFO_MMAP_LEN as int,
        EL0_TEST_COMPLETE_LEN as int,
    ) == EL0_TEST_EXIT_OK as int);
    assert(kernel_success_spec(
        true,
        true,
        EL0_TEST_EXIT_OK as int,
        EL0_TEST_BANNER_LEN as int,
        1,
        USER_MMAP_BASE as int,
        EL0_TEST_BANNER_LEN as int,
    ));
    assert(!kernel_success_spec(
        false,
        true,
        EL0_TEST_EXIT_OK as int,
        EL0_TEST_BANNER_LEN as int,
        1,
        USER_MMAP_BASE as int,
        EL0_TEST_BANNER_LEN as int,
    ));
    assert(0x3C5u64 == smros_user_el1h_spsr_masked_body!());
    assert(0u64 == smros_user_syscall_should_advance_elr_body!());
}

} // verus!
