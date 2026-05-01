include!("user_logic_shared.rs");

pub(crate) const USER_PROCESS_CAPACITY: usize = 16;
pub(crate) const USER_INIT_PARENT_PID: usize = 1;
pub(crate) const USER_CODE_VADDR: usize = 0x0000_0000;
pub(crate) const USER_DATA_VADDR: usize = 0x0000_1000;
pub(crate) const USER_HEAP_VADDR: usize = 0x0000_2000;
pub(crate) const USER_HEAP_PAGES: usize = 4;
pub(crate) const USER_STACK_VADDR: usize = 0xFFFF_0000;
pub(crate) const USER_STACK_PAGES: usize = 2;
pub(crate) const USER_THREAD_TIME_SLICE: u32 = 10;
pub(crate) const USER_MMAP_BASE: u64 = 0x5000_0000;
pub(crate) const USER_MMAP_LIMIT: u64 = 0x6000_0000;

pub(crate) fn page_offset_vaddr(base: usize, page_index: usize, page_size: usize) -> Option<usize> {
    smros_user_page_offset_body!(base, page_index, page_size)
}

pub(crate) fn pfn_to_paddr(pfn: u64, page_size: usize) -> Option<u64> {
    smros_user_pfn_to_paddr_body!(pfn, page_size as u64)
}

pub(crate) fn stack_top_u64(stack_base: u64, stack_size: usize) -> Option<u64> {
    smros_user_stack_top_u64_body!(stack_base, stack_size)
}

pub(crate) fn el0_thread_pstate() -> u64 {
    smros_user_el0_thread_pstate_body!()
}

pub(crate) fn el0_spsr() -> u64 {
    smros_user_el0_spsr_body!()
}

pub(crate) fn el1h_spsr_masked() -> u64 {
    smros_user_el1h_spsr_masked_body!()
}

pub(crate) fn syscall_should_advance_elr() -> u64 {
    smros_user_syscall_should_advance_elr_body!()
}

pub(crate) fn ascii_shell_input(byte: u8) -> bool {
    smros_user_ascii_shell_input_body!(byte)
}

pub(crate) fn decimal_digit_value(byte: u8) -> Option<usize> {
    smros_user_decimal_digit_value_body!(byte)
}

pub(crate) fn parse_digit_step(result: usize, digit: usize) -> Option<usize> {
    smros_user_parse_digit_step_body!(result, digit)
}

pub(crate) fn saturating_sub(lhs: usize, rhs: usize) -> usize {
    smros_user_saturating_sub_body!(lhs, rhs)
}

pub(crate) fn pages_to_kb(pages: usize, page_size: usize) -> usize {
    smros_user_pages_to_kb_body!(pages, page_size)
}

pub(crate) fn usage_percent(used_pages: usize, total_pages: usize) -> usize {
    smros_user_usage_percent_body!(used_pages, total_pages)
}

pub(crate) fn uptime_parts(ticks: u64) -> (u64, u64, u64, u64) {
    smros_user_uptime_parts_body!(ticks)
}

pub(crate) fn mmap_result_ok(addr: u64) -> bool {
    smros_user_mmap_result_ok_body!(addr, 4096u64, USER_MMAP_BASE, USER_MMAP_LIMIT)
}

pub(crate) fn kernel_success(
    kernel_entered: bool,
    kernel_finished: bool,
    exit_code: i32,
    kernel_write: u64,
    kernel_pid: u64,
    kernel_mmap: u64,
    banner_len: usize,
) -> bool {
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
