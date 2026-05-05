include!("lowlevel_logic_shared.rs");

pub(crate) fn segment_size(page_count: usize, page_size: usize) -> Option<usize> {
    smros_ll_segment_size_body!(page_count, page_size)
}

pub(crate) fn segment_end(
    valid: bool,
    base: usize,
    page_count: usize,
    page_size: usize,
) -> Option<usize> {
    smros_ll_segment_end_body!(valid, base, page_count, page_size)
}

pub(crate) fn segment_contains(
    valid: bool,
    base: usize,
    page_count: usize,
    page_size: usize,
    vaddr: usize,
) -> bool {
    smros_ll_segment_contains_body!(valid, base, page_count, page_size, vaddr)
}

pub(crate) fn memory_capacity_ok(
    segment_count: usize,
    page_count: usize,
    valid_page_count: usize,
    max_segments: usize,
    max_pages: usize,
) -> bool {
    smros_ll_memory_capacity_ok_body!(
        segment_count,
        page_count,
        valid_page_count,
        max_segments,
        max_pages
    )
}

pub(crate) fn permission_writable<T: Copy + PartialEq>(
    permission: T,
    write: T,
    read_write: T,
) -> bool {
    smros_ll_permission_writable_body!(permission, write, read_write)
}

pub(crate) fn permission_executable<T: Copy + PartialEq>(
    permission: T,
    execute: T,
    read_execute: T,
) -> bool {
    smros_ll_permission_executable_body!(permission, execute, read_execute)
}

pub(crate) fn heap_alloc(
    current: usize,
    max: usize,
    size: usize,
    page_size: usize,
) -> Option<(usize, usize)> {
    smros_ll_heap_alloc_body!(current, max, size, page_size)
}

pub(crate) fn stack_alloc(current: usize, size: usize, page_size: usize) -> Option<usize> {
    smros_ll_stack_alloc_body!(current, size, page_size)
}

pub(crate) fn page_to_vaddr(
    page_idx: usize,
    valid_page_count: usize,
    page_size: usize,
) -> Option<usize> {
    smros_ll_page_to_vaddr_body!(page_idx, valid_page_count, page_size)
}

pub(crate) fn pfn_valid(pfn: u64, total_pages: usize) -> bool {
    smros_ll_pfn_valid_body!(pfn, total_pages)
}

pub(crate) fn bitmap_word_index(pfn: u64) -> usize {
    smros_ll_bitmap_word_index_body!(pfn)
}

pub(crate) fn bitmap_bit_index(pfn: u64) -> usize {
    smros_ll_bitmap_bit_index_body!(pfn)
}

pub(crate) fn bitmap_mask(bit: usize) -> u64 {
    smros_ll_bitmap_mask_body!(bit)
}

pub(crate) fn process_index_valid(index: usize, max_processes: usize) -> bool {
    smros_ll_process_index_valid_body!(index, max_processes)
}

pub(crate) fn thread_state_runnable<T: Copy + PartialEq>(state: T, ready: T, running: T) -> bool {
    smros_ll_thread_state_runnable_body!(state, ready, running)
}

pub(crate) fn thread_id_idle<T: Copy + PartialEq>(id: T, idle: T) -> bool {
    smros_ll_thread_id_idle_body!(id, idle)
}

pub(crate) fn pte_set_flag(value: u64, flag: u64, enabled: bool) -> u64 {
    smros_ll_pte_set_flag_body!(value, flag, enabled)
}

pub(crate) fn pte_output_address(paddr: u64) -> u64 {
    smros_ll_pte_output_address_body!(paddr)
}

pub(crate) fn pte_set_output_address(value: u64, paddr: u64) -> u64 {
    smros_ll_pte_set_output_address_body!(value, paddr)
}

pub(crate) fn pte_attr_idx(value: u64, idx: u64) -> u64 {
    smros_ll_pte_attr_idx_body!(value, idx)
}

pub(crate) fn pte_sh(value: u64, sharability: u64) -> u64 {
    smros_ll_pte_sh_body!(value, sharability)
}

pub(crate) fn pte_table(value: u64) -> bool {
    smros_ll_pte_table_body!(value)
}

pub(crate) fn pt_index(vaddr: usize, entries: usize) -> usize {
    smros_ll_pt_index_body!(vaddr, entries)
}

pub(crate) fn vma_size(start: usize, end: usize) -> usize {
    smros_ll_vma_size_body!(start, end)
}

pub(crate) fn mmio_addr(base: usize, offset: usize) -> Option<usize> {
    smros_ll_mmio_addr_body!(base, offset)
}

pub(crate) fn uart_control(uarten: u32, txe: u32, rxe: u32) -> u32 {
    smros_ll_uart_control_body!(uarten, txe, rxe)
}

pub(crate) fn uart_lcrh(word_len_8: u32, fifo_enable: u32) -> u32 {
    smros_ll_uart_lcrh_body!(word_len_8, fifo_enable)
}

pub(crate) fn uart_has_byte(flags: u32, rx_empty_flag: u32) -> bool {
    smros_ll_uart_has_byte_body!(flags, rx_empty_flag)
}

pub(crate) fn uart_tx_ready(flags: u32, tx_full_flag: u32) -> bool {
    smros_ll_uart_tx_ready_body!(flags, tx_full_flag)
}

pub(crate) fn ascii_printable(byte: u8) -> bool {
    smros_ll_ascii_printable_body!(byte)
}

pub(crate) fn hex_digit(nibble: u8) -> u8 {
    smros_ll_hex_digit_body!(nibble)
}

pub(crate) fn timer_period(frequency: u64) -> u64 {
    smros_ll_timer_period_body!(frequency)
}

pub(crate) fn timer_compare(current: u64, period: u64) -> u64 {
    smros_ll_timer_compare_body!(current, period)
}

pub(crate) fn timer_tick_count(counter: u64, period: u64) -> u64 {
    smros_ll_timer_tick_count_body!(counter, period)
}

pub(crate) fn timer_ctl(enable: u64, imask: u64) -> u64 {
    smros_ll_timer_ctl_body!(enable, imask)
}

pub(crate) fn gic_reg_offset(base_offset: usize, irq: u32, field_width: usize) -> usize {
    smros_ll_gic_reg_offset_body!(base_offset, irq, field_width)
}

pub(crate) fn gic_byte_shift(irq: u32) -> usize {
    smros_ll_gic_byte_shift_body!(irq)
}

pub(crate) fn gic_set_byte_field(value: u32, byte_shift: usize, field: u8) -> u32 {
    smros_ll_gic_set_byte_field_body!(value, byte_shift, field)
}

pub(crate) fn gic_enable_bit(irq: u32) -> u32 {
    smros_ll_gic_enable_bit_body!(irq)
}

pub(crate) fn gic_interrupt_id(iar: u32) -> u32 {
    smros_ll_gic_interrupt_id_body!(iar)
}

pub(crate) fn dt_reg_valid(base: usize, size: usize) -> bool {
    smros_ll_dt_reg_valid_body!(base, size)
}

pub(crate) fn dt_reg_contains(base: usize, size: usize, addr: usize) -> bool {
    smros_ll_dt_reg_contains_body!(base, size, addr)
}

pub(crate) fn dt_irq_valid(irq: u32, max_irqs: u32) -> bool {
    smros_ll_dt_irq_valid_body!(irq, max_irqs)
}

pub(crate) fn dt_platform_index(candidate: usize, platform_count: usize, fallback: usize) -> usize {
    smros_ll_dt_platform_index_body!(candidate, platform_count, fallback)
}

pub(crate) fn fdt_range_valid(offset: usize, len: usize, total: usize) -> bool {
    smros_ll_fdt_range_valid_body!(offset, len, total)
}

pub(crate) fn fdt_align4(offset: usize) -> Option<usize> {
    smros_ll_fdt_align4_body!(offset)
}

pub(crate) fn fdt_cells_to_bytes(cells: usize) -> Option<usize> {
    smros_ll_fdt_cells_to_bytes_body!(cells)
}

pub(crate) fn fdt_reg_tuple_bytes(address_cells: usize, size_cells: usize) -> Option<usize> {
    smros_ll_fdt_reg_tuple_bytes_body!(address_cells, size_cells)
}

pub(crate) fn fdt_reg_tuple_offset(
    index: usize,
    address_cells: usize,
    size_cells: usize,
) -> Option<usize> {
    smros_ll_fdt_reg_tuple_offset_body!(index, address_cells, size_cells)
}

pub(crate) fn dt_gic_irq(kind: u32, hwirq: u32, max_irqs: u32) -> Option<u32> {
    smros_ll_dt_gic_irq_body!(kind, hwirq, max_irqs)
}

pub(crate) fn dt_timer_irq_index(entry_count: usize) -> usize {
    smros_ll_dt_timer_irq_index_body!(entry_count)
}

pub(crate) fn cpu_id_from_mpidr(mpidr: u64) -> u32 {
    smros_ll_cpu_id_from_mpidr_body!(mpidr)
}

pub(crate) fn valid_cpu_id(cpu_id: u32, max_cpus: usize) -> bool {
    smros_ll_valid_cpu_id_body!(cpu_id, max_cpus)
}

pub(crate) fn display_mpidr(cpu_id: u32) -> u64 {
    smros_ll_display_mpidr_body!(cpu_id)
}

pub(crate) fn psci_success(result: i64, success: i64, on_pending: i64) -> bool {
    smros_ll_psci_success_body!(result, success, on_pending)
}
