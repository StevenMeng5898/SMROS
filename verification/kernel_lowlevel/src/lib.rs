use vstd::prelude::*;

verus! {

include!("../../../src/kernel_lowlevel/lowlevel_logic_shared.rs");

pub const PAGE_SIZE: usize = 4096;
pub const MAX_PROCESSES: usize = 16;
pub const MAX_PAGES_PER_PROCESS: usize = 64;
pub const MAX_SEGMENTS: usize = 4;
pub const TOTAL_PHYSICAL_PAGES: usize = 4096;
pub const PT_ENTRIES: usize = 512;
pub const TIMER_IRQ: u32 = 30;
pub const MAX_CPUS: usize = 4;

pub const SEGMENT_CODE: u8 = 0;
pub const SEGMENT_DATA: u8 = 1;
pub const SEGMENT_HEAP: u8 = 2;
pub const SEGMENT_STACK: u8 = 3;

pub const PERM_READ: u8 = 0b001;
pub const PERM_WRITE: u8 = 0b010;
pub const PERM_EXECUTE: u8 = 0b100;
pub const PERM_READ_WRITE: u8 = 0b011;
pub const PERM_READ_EXECUTE: u8 = 0b101;

pub const PROCESS_EMPTY: u8 = 0;
pub const PROCESS_READY: u8 = 1;
pub const PROCESS_RUNNING: u8 = 2;
pub const PROCESS_BLOCKED: u8 = 3;
pub const PROCESS_TERMINATED: u8 = 4;

pub const THREAD_EMPTY: u8 = 0;
pub const THREAD_READY: u8 = 1;
pub const THREAD_RUNNING: u8 = 2;
pub const THREAD_BLOCKED: u8 = 3;
pub const THREAD_TERMINATED: u8 = 4;
pub const THREAD_IDLE_ID: usize = 0;

pub const UART_BASE: usize = 0x0900_0000;
pub const UART_FR: usize = 0x18;
pub const UART_CR: usize = 0x30;
pub const FR_TXFF: u32 = 1 << 5;
pub const FR_RXFE: u32 = 1 << 4;
pub const CR_UARTEN: u32 = 1 << 0;
pub const CR_TXE: u32 = 1 << 8;
pub const CR_RXE: u32 = 1 << 9;
pub const LCRH_WLEN_8: u32 = 3 << 5;
pub const LCRH_FEN: u32 = 1 << 4;

pub const CNTP_CTL_ENABLE: u64 = 1 << 0;
pub const CNTP_CTL_IMASK: u64 = 1 << 1;

pub const GICD_IPRIORITYR: usize = 0x400;
pub const GICD_ITARGETSR: usize = 0x800;
pub const GICD_ISENABLER: usize = 0x100;
pub const PRIORITY_HIGH: u8 = 0x50;
pub const MAX_PLATFORM_IRQS: u32 = 1024;
pub const DEFAULT_PLATFORM_INDEX: usize = 0;
pub const PLATFORM_COUNT: usize = 2;
pub const QEMU_VIRT_UART_BASE: usize = 0x0900_0000;
pub const QEMU_VIRT_UART_SIZE: usize = 0x1000;
pub const QEMU_VIRT_UART_IRQ: u32 = 33;
pub const QEMU_VIRT_GICD_BASE: usize = 0x0800_0000;
pub const QEMU_VIRT_GICD_SIZE: usize = 0x10000;
pub const QEMU_VIRT_GICC_BASE: usize = 0x0801_0000;
pub const QEMU_VIRT_GICC_SIZE: usize = 0x10000;

pub const PSCI_RET_SUCCESS: i64 = 0;
pub const PSCI_RET_ON_PENDING: i64 = -1;
pub const PSCI_RET_INTERNAL_FAILURE: i64 = -2;

#[derive(Copy, Clone)]
struct SegmentModel {
    base: usize,
    page_count: usize,
    valid: bool,
}

#[derive(Copy, Clone)]
struct ProcessModel {
    pid: usize,
    state: u8,
}

spec fn checked_end_spec(addr: int, len: int) -> Option<int> {
    if 0 <= addr && 0 <= len && addr <= usize::MAX as int - len {
        Some(addr + len)
    } else {
        Option::<int>::None
    }
}

spec fn align_up_spec(size: int, align: int) -> Option<int> {
    if size < 0 || align <= 0 {
        Option::<int>::None
    } else if size % align == 0 {
        Some(size)
    } else {
        let whole_units = size / align;
        if whole_units < usize::MAX as int && whole_units + 1 <= usize::MAX as int / align {
            let units = whole_units + 1;
            Some(units * align)
        } else {
            Option::<int>::None
        }
    }
}

spec fn segment_size_spec(page_count: int, page_size: int) -> Option<int> {
    if page_count < 0 || page_size < 0 {
        Option::<int>::None
    } else if page_size == 0 {
        Some(0)
    } else if page_count <= usize::MAX as int / page_size {
        Some(page_count * page_size)
    } else {
        Option::<int>::None
    }
}

spec fn segment_end_spec(valid: bool, base: int, page_count: int, page_size: int) -> Option<int> {
    if !valid {
        Some(0)
    } else {
        match segment_size_spec(page_count, page_size) {
            Some(size) => checked_end_spec(base, size),
            None => Option::<int>::None,
        }
    }
}

spec fn segment_contains_spec(valid: bool, base: int, page_count: int, page_size: int, vaddr: int) -> bool {
    match segment_end_spec(valid, base, page_count, page_size) {
        Some(end) => valid && vaddr >= base && vaddr < end,
        None => false,
    }
}

spec fn memory_capacity_ok_spec(
    segment_count: int,
    page_count: int,
    valid_page_count: int,
    max_segments: int,
    max_pages: int,
) -> bool {
    segment_count < max_segments
        && valid_page_count <= max_pages
        && page_count != 0
        && page_count <= max_pages - valid_page_count
}

spec fn heap_alloc_spec(current: int, max: int, size: int, page_size: int) -> Option<(int, int)> {
    match align_up_spec(size, page_size) {
        Some(aligned_size) => match checked_end_spec(current, aligned_size) {
            Some(next) => if next <= max { Some((current, next)) } else { Option::<(int, int)>::None },
            None => Option::<(int, int)>::None,
        },
        None => Option::<(int, int)>::None,
    }
}

spec fn stack_alloc_spec(current: int, size: int, page_size: int) -> Option<int> {
    match align_up_spec(size, page_size) {
        Some(aligned_size) => if current >= aligned_size { Some(current - aligned_size) } else { Option::<int>::None },
        None => Option::<int>::None,
    }
}

spec fn page_to_vaddr_spec(page_idx: int, valid_page_count: int, page_size: int) -> Option<int> {
    if page_idx >= valid_page_count {
        Option::<int>::None
    } else {
        segment_size_spec(page_idx, page_size)
    }
}

spec fn process_state_live_spec(state: int) -> bool {
    state != PROCESS_EMPTY as int && state != PROCESS_TERMINATED as int
}

spec fn dt_reg_valid_spec(base: int, size: int) -> bool {
    size > 0 && match checked_end_spec(base, size) {
        Some(_) => true,
        None => false,
    }
}

spec fn dt_reg_contains_spec(base: int, size: int, addr: int) -> bool {
    if size <= 0 {
        false
    } else {
        match checked_end_spec(base, size) {
            Some(end) => addr >= base && addr < end,
            None => false,
        }
    }
}

spec fn dt_irq_valid_spec(irq: int, max_irqs: int) -> bool {
    max_irqs > 0 && irq < max_irqs
}

spec fn dt_platform_index_spec(candidate: int, platform_count: int, fallback: int) -> int {
    if candidate < platform_count {
        candidate
    } else if fallback < platform_count {
        fallback
    } else {
        0
    }
}

spec fn fdt_range_valid_spec(offset: int, len: int, total: int) -> bool {
    offset <= total && len <= total - offset
}

spec fn fdt_align4_spec(offset: int) -> Option<int> {
    align_up_spec(offset, 4)
}

spec fn fdt_cells_to_bytes_spec(cells: int) -> Option<int> {
    if cells < 0 || cells > usize::MAX as int / 4 {
        Option::<int>::None
    } else {
        Some(cells * 4)
    }
}

spec fn fdt_reg_tuple_bytes_spec(address_cells: int, size_cells: int) -> Option<int> {
    if address_cells < 0
        || size_cells < 0
        || address_cells > usize::MAX as int - size_cells
    {
        Option::<int>::None
    } else {
        let cells = address_cells + size_cells;
        if cells == 0 {
            Option::<int>::None
        } else {
            fdt_cells_to_bytes_spec(cells)
        }
    }
}

spec fn fdt_reg_tuple_offset_spec(
    index: int,
    address_cells: int,
    size_cells: int,
) -> Option<int> {
    if index < 0 {
        Option::<int>::None
    } else {
        match fdt_reg_tuple_bytes_spec(address_cells, size_cells) {
            Some(tuple_bytes) => {
                if tuple_bytes > 0 && index <= usize::MAX as int / tuple_bytes {
                    Some(index * tuple_bytes)
                } else {
                    Option::<int>::None
                }
            },
            None => Option::<int>::None,
        }
    }
}

spec fn dt_gic_irq_spec(kind: int, hwirq: int, max_irqs: int) -> Option<int> {
    if kind == 0 {
        if hwirq >= 0 && hwirq <= 4294967295int - 32 && hwirq + 32 < max_irqs {
            Some(hwirq + 32)
        } else {
            Option::<int>::None
        }
    } else if kind == 1 {
        if hwirq >= 0 && hwirq <= 4294967295int - 16 && hwirq + 16 < max_irqs {
            Some(hwirq + 16)
        } else {
            Option::<int>::None
        }
    } else {
        Option::<int>::None
    }
}

spec fn dt_timer_irq_index_spec(entry_count: int) -> int {
    if entry_count >= 4 {
        1
    } else {
        0
    }
}

fn ll_checked_end(addr: usize, len: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(end) => checked_end_spec(addr as int, len as int) == Some(end as int),
            None => checked_end_spec(addr as int, len as int) == Option::<int>::None,
        },
{
    smros_ll_checked_end_body!(addr, len)
}

fn ll_segment_end(valid: bool, base: usize, page_count: usize, page_size: usize) -> (out: Option<usize>)
    requires
        valid ==> page_size > 0,
        valid ==> page_count <= usize::MAX / page_size,
        valid ==> base <= usize::MAX - (page_count * page_size),
    ensures
        match out {
            Some(end) => segment_end_spec(valid, base as int, page_count as int, page_size as int) == Some(end as int),
            None => segment_end_spec(valid, base as int, page_count as int, page_size as int) == Option::<int>::None,
        },
{
    smros_ll_segment_end_body!(valid, base, page_count, page_size)
}

fn ll_segment_contains(valid: bool, base: usize, page_count: usize, page_size: usize, vaddr: usize) -> (out: bool)
    requires
        valid ==> page_size > 0,
        valid ==> page_count <= usize::MAX / page_size,
        valid ==> base <= usize::MAX - (page_count * page_size),
    ensures
        out == segment_contains_spec(valid, base as int, page_count as int, page_size as int, vaddr as int),
{
    smros_ll_segment_contains_body!(valid, base, page_count, page_size, vaddr)
}

fn ll_memory_capacity_ok(
    segment_count: usize,
    page_count: usize,
    valid_page_count: usize,
    max_segments: usize,
    max_pages: usize,
) -> (out: bool)
    ensures
        out == memory_capacity_ok_spec(
            segment_count as int,
            page_count as int,
            valid_page_count as int,
            max_segments as int,
            max_pages as int,
        ),
{
    smros_ll_memory_capacity_ok_body!(
        segment_count,
        page_count,
        valid_page_count,
        max_segments,
        max_pages
    )
}

fn ll_permission_writable(permission: u8) -> (out: bool)
    ensures
        out == (permission == PERM_READ_WRITE || permission == PERM_WRITE),
{
    smros_ll_permission_writable_body!(permission, PERM_WRITE, PERM_READ_WRITE)
}

fn ll_permission_executable(permission: u8) -> (out: bool)
    ensures
        out == (permission == PERM_READ_EXECUTE || permission == PERM_EXECUTE),
{
    smros_ll_permission_executable_body!(permission, PERM_EXECUTE, PERM_READ_EXECUTE)
}

fn ll_heap_alloc(current: usize, max: usize, size: usize) -> (out: Option<(usize, usize)>)
    requires
        size <= usize::MAX - (PAGE_SIZE - 1),
    ensures
        match out {
            Some((addr, next)) => heap_alloc_spec(current as int, max as int, size as int, PAGE_SIZE as int)
                == Some((addr as int, next as int)),
            None => heap_alloc_spec(current as int, max as int, size as int, PAGE_SIZE as int)
                == Option::<(int, int)>::None,
        },
{
    smros_ll_heap_alloc_body!(current, max, size, PAGE_SIZE)
}

fn ll_stack_alloc(current: usize, size: usize) -> (out: Option<usize>)
    requires
        size <= usize::MAX - (PAGE_SIZE - 1),
    ensures
        match out {
            Some(next) => stack_alloc_spec(current as int, size as int, PAGE_SIZE as int) == Some(next as int),
            None => stack_alloc_spec(current as int, size as int, PAGE_SIZE as int) == Option::<int>::None,
        },
{
    smros_ll_stack_alloc_body!(current, size, PAGE_SIZE)
}

fn ll_page_to_vaddr(page_idx: usize, valid_page_count: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(vaddr) => page_to_vaddr_spec(page_idx as int, valid_page_count as int, PAGE_SIZE as int) == Some(vaddr as int),
            None => page_to_vaddr_spec(page_idx as int, valid_page_count as int, PAGE_SIZE as int) == Option::<int>::None,
        },
{
    smros_ll_page_to_vaddr_body!(page_idx, valid_page_count, PAGE_SIZE)
}

fn find_segment_model(segments: &Vec<SegmentModel>, vaddr: usize) -> (out: Option<usize>)
    requires
        forall|i: int|
            0 <= i < segments@.len() ==> !segments@[i].valid
                || (segments@[i].page_count as int <= usize::MAX as int / PAGE_SIZE as int
                    && segments@[i].base as int
                        <= usize::MAX as int - segments@[i].page_count as int * PAGE_SIZE as int),
    ensures
        match out {
            Some(i) => i < segments.len()
                && segment_contains_spec(
                    segments@[i as int].valid,
                    segments@[i as int].base as int,
                    segments@[i as int].page_count as int,
                    PAGE_SIZE as int,
                    vaddr as int,
                ),
            None => forall|i: int|
                0 <= i < segments@.len() ==> !segment_contains_spec(
                    segments@[i].valid,
                    segments@[i].base as int,
                    segments@[i].page_count as int,
                    PAGE_SIZE as int,
                    vaddr as int,
                ),
        },
{
    let mut i = 0usize;
    while i < segments.len()
        invariant
            i <= segments.len(),
            forall|k: int|
                0 <= k < segments@.len() ==> !segments@[k].valid
                    || (segments@[k].page_count as int <= usize::MAX as int / PAGE_SIZE as int
                        && segments@[k].base as int
                            <= usize::MAX as int - segments@[k].page_count as int * PAGE_SIZE as int),
            forall|j: int|
                0 <= j < i as int ==> !segment_contains_spec(
                    segments@[j].valid,
                    segments@[j].base as int,
                    segments@[j].page_count as int,
                    PAGE_SIZE as int,
                    vaddr as int,
                ),
        decreases segments.len() - i,
    {
        let seg = &segments[i];
        if ll_segment_contains(seg.valid, seg.base, seg.page_count, PAGE_SIZE, vaddr) {
            return Some(i);
        }
        i += 1;
    }

    assert forall|j: int|
        0 <= j < segments@.len() implies !segment_contains_spec(
            segments@[j].valid,
            segments@[j].base as int,
            segments@[j].page_count as int,
            PAGE_SIZE as int,
            vaddr as int,
        ) by {
        assert(j < i as int);
    };
    None
}

fn find_process_by_pid_model(processes: &Vec<ProcessModel>, pid: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(i) => i < processes.len()
                && processes@[i as int].pid == pid
                && processes@[i as int].state != PROCESS_EMPTY,
            None => forall|i: int|
                0 <= i < processes@.len()
                    ==> !(processes@[i].pid == pid && processes@[i].state != PROCESS_EMPTY),
        },
{
    let mut i = 0usize;
    while i < processes.len()
        invariant
            i <= processes.len(),
            forall|j: int|
                0 <= j < i as int
                    ==> !(processes@[j].pid == pid && processes@[j].state != PROCESS_EMPTY),
        decreases processes.len() - i,
    {
        let process = &processes[i];
        if process.pid == pid && process.state != PROCESS_EMPTY {
            return Some(i);
        }
        i += 1;
    }

    assert forall|j: int|
        0 <= j < processes@.len() implies !(processes@[j].pid == pid && processes@[j].state != PROCESS_EMPTY) by {
        assert(j < i as int);
    };
    None
}

fn ll_pfn_valid(pfn: u64) -> (out: bool)
    requires
        pfn <= usize::MAX as u64,
    ensures
        out == ((pfn as int) < TOTAL_PHYSICAL_PAGES as int),
{
    smros_ll_pfn_valid_body!(pfn, TOTAL_PHYSICAL_PAGES)
}

fn ll_bitmap_indices(pfn: u64) -> (out: (usize, usize))
    ensures
        out.0 == (pfn as usize) / 64,
        out.1 == (pfn as usize) % 64,
        out.1 < 64,
{
    (
        smros_ll_bitmap_word_index_body!(pfn),
        smros_ll_bitmap_bit_index_body!(pfn),
    )
}

fn ll_bitmap_mask(bit: usize) -> (out: u64)
    requires
        bit < 64,
    ensures
        out == 1u64 << bit,
{
    smros_ll_bitmap_mask_body!(bit)
}

fn ll_process_index_valid(index: usize) -> (out: bool)
    ensures
        out == (index < MAX_PROCESSES),
{
    smros_ll_process_index_valid_body!(index, MAX_PROCESSES)
}

fn ll_thread_state_runnable(state: u8) -> (out: bool)
    ensures
        out == (state == THREAD_READY || state == THREAD_RUNNING),
{
    smros_ll_thread_state_runnable_body!(state, THREAD_READY, THREAD_RUNNING)
}

fn ll_thread_id_idle(id: usize) -> (out: bool)
    ensures
        out == (id == THREAD_IDLE_ID),
{
    smros_ll_thread_id_idle_body!(id, THREAD_IDLE_ID)
}

fn ll_pte_set_flag(value: u64, flag: u64, enabled: bool) -> (out: u64)
    ensures
        enabled ==> out == (value | flag),
        !enabled ==> out == (value & !flag),
{
    smros_ll_pte_set_flag_body!(value, flag, enabled)
}

fn ll_pte_output_address(paddr: u64) -> (out: u64)
    ensures
        out == (paddr & 0x0000_FFFF_FFFF_F000u64),
{
    smros_ll_pte_output_address_body!(paddr)
}

fn ll_pte_set_output_address(value: u64, paddr: u64) -> (out: u64)
    ensures
        out == ((value & 0xFFFu64) | (paddr & 0x0000_FFFF_FFFF_F000u64)),
{
    smros_ll_pte_set_output_address_body!(value, paddr)
}

fn ll_pte_attr_idx(value: u64, idx: u64) -> (out: u64)
    ensures
        out == ((value & !0x1Cu64) | ((idx << 2) & 0x1Cu64)),
{
    smros_ll_pte_attr_idx_body!(value, idx)
}

fn ll_pte_sh(value: u64, sharability: u64) -> (out: u64)
    ensures
        out == ((value & !0x300u64) | ((sharability << 8) & 0x300u64)),
{
    smros_ll_pte_sh_body!(value, sharability)
}

fn ll_pte_table(value: u64) -> (out: bool)
    ensures
        out == ((value & 1u64) != 0 && (value & (1u64 << 1)) == 0),
{
    smros_ll_pte_table_body!(value)
}

fn ll_pt_index(vaddr: usize) -> (out: usize)
    ensures
        out == ((vaddr >> 21) & 511usize),
{
    smros_ll_pt_index_body!(vaddr, PT_ENTRIES)
}

fn ll_vma_size(start: usize, end: usize) -> (out: usize)
    ensures
        end >= start ==> out == end - start,
        end < start ==> out == 0,
{
    smros_ll_vma_size_body!(start, end)
}

fn ll_mmio_addr(base: usize, offset: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(addr) => checked_end_spec(base as int, offset as int) == Some(addr as int),
            None => checked_end_spec(base as int, offset as int) == Option::<int>::None,
        },
{
    smros_ll_mmio_addr_body!(base, offset)
}

fn ll_uart_control() -> (out: u32)
    ensures
        out == (CR_UARTEN | CR_TXE | CR_RXE),
{
    smros_ll_uart_control_body!(CR_UARTEN, CR_TXE, CR_RXE)
}

fn ll_uart_lcrh() -> (out: u32)
    ensures
        out == (LCRH_WLEN_8 | LCRH_FEN),
{
    smros_ll_uart_lcrh_body!(LCRH_WLEN_8, LCRH_FEN)
}

fn ll_uart_has_byte(flags: u32) -> (out: bool)
    ensures
        out == ((flags & FR_RXFE) == 0),
{
    smros_ll_uart_has_byte_body!(flags, FR_RXFE)
}

fn ll_uart_tx_ready(flags: u32) -> (out: bool)
    ensures
        out == ((flags & FR_TXFF) == 0),
{
    smros_ll_uart_tx_ready_body!(flags, FR_TXFF)
}

fn ll_ascii_printable(byte: u8) -> (out: bool)
    ensures
        out == (byte >= 0x20 && byte <= 0x7e),
{
    smros_ll_ascii_printable_body!(byte)
}

fn ll_hex_digit(nibble: u8) -> (out: u8)
    requires
        nibble < 16,
    ensures
        nibble < 10 ==> out == 48u8 + nibble,
        nibble >= 10 ==> out == 97u8 + (nibble - 10),
{
    smros_ll_hex_digit_body!(nibble)
}

fn ll_timer_period(frequency: u64) -> (out: u64)
    ensures
        out == frequency / 100,
{
    smros_ll_timer_period_body!(frequency)
}

fn ll_timer_compare(current: u64, period: u64) -> (out: u64)
    ensures
        out == current.wrapping_add(period),
{
    smros_ll_timer_compare_body!(current, period)
}

fn ll_timer_tick_count(counter: u64, period: u64) -> (out: u64)
    ensures
        period == 0 ==> out == 0,
        period != 0 ==> out == counter / period,
{
    smros_ll_timer_tick_count_body!(counter, period)
}

fn ll_timer_ctl() -> (out: u64)
    ensures
        out == (CNTP_CTL_ENABLE & !CNTP_CTL_IMASK),
{
    smros_ll_timer_ctl_body!(CNTP_CTL_ENABLE, CNTP_CTL_IMASK)
}

fn ll_gic_reg_offset(base_offset: usize, irq: u32, field_width: usize) -> (out: usize)
    requires
        field_width > 0,
        ((irq as usize) / field_width) <= usize::MAX / 4,
        base_offset <= usize::MAX - (((irq as usize) / field_width) * 4),
    ensures
        out == base_offset + (((irq as usize) / field_width) * 4),
{
    smros_ll_gic_reg_offset_body!(base_offset, irq, field_width)
}

fn ll_gic_byte_shift(irq: u32) -> (out: usize)
    ensures
        out == ((irq % 4) as usize) * 8,
        out <= 24,
{
    smros_ll_gic_byte_shift_body!(irq)
}

fn ll_gic_set_byte_field(value: u32, byte_shift: usize, field: u8) -> (out: u32)
    requires
        byte_shift <= 24,
    ensures
        out == ((value & !(0xFFu32 << byte_shift)) | ((field as u32) << byte_shift)),
{
    smros_ll_gic_set_byte_field_body!(value, byte_shift, field)
}

fn ll_gic_enable_bit(irq: u32) -> (out: u32)
    ensures
        out == 1u32 << (irq % 32),
{
    smros_ll_gic_enable_bit_body!(irq)
}

fn ll_gic_interrupt_id(iar: u32) -> (out: u32)
    ensures
        out == (iar & 0x3FFu32),
{
    smros_ll_gic_interrupt_id_body!(iar)
}

fn ll_dt_reg_valid(base: usize, size: usize) -> (out: bool)
    ensures
        out == dt_reg_valid_spec(base as int, size as int),
{
    smros_ll_dt_reg_valid_body!(base, size)
}

fn ll_dt_reg_contains(base: usize, size: usize, addr: usize) -> (out: bool)
    ensures
        out == dt_reg_contains_spec(base as int, size as int, addr as int),
{
    smros_ll_dt_reg_contains_body!(base, size, addr)
}

fn ll_dt_irq_valid(irq: u32, max_irqs: u32) -> (out: bool)
    ensures
        out == dt_irq_valid_spec(irq as int, max_irqs as int),
{
    smros_ll_dt_irq_valid_body!(irq, max_irqs)
}

fn ll_dt_platform_index(candidate: usize, platform_count: usize, fallback: usize) -> (out: usize)
    ensures
        out as int == dt_platform_index_spec(candidate as int, platform_count as int, fallback as int),
        platform_count > 0 ==> out < platform_count,
{
    smros_ll_dt_platform_index_body!(candidate, platform_count, fallback)
}

fn ll_fdt_range_valid(offset: usize, len: usize, total: usize) -> (out: bool)
    ensures
        out == fdt_range_valid_spec(offset as int, len as int, total as int),
{
    smros_ll_fdt_range_valid_body!(offset, len, total)
}

fn ll_fdt_align4(offset: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(aligned) => fdt_align4_spec(offset as int) == Some(aligned as int),
            None => fdt_align4_spec(offset as int) == Option::<int>::None,
        },
{
    smros_ll_fdt_align4_body!(offset)
}

fn ll_fdt_cells_to_bytes(cells: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(bytes) => fdt_cells_to_bytes_spec(cells as int) == Some(bytes as int),
            None => fdt_cells_to_bytes_spec(cells as int) == Option::<int>::None,
        },
{
    smros_ll_fdt_cells_to_bytes_body!(cells)
}

fn ll_fdt_reg_tuple_bytes(address_cells: usize, size_cells: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(bytes) => fdt_reg_tuple_bytes_spec(address_cells as int, size_cells as int)
                == Some(bytes as int),
            None => fdt_reg_tuple_bytes_spec(address_cells as int, size_cells as int)
                == Option::<int>::None,
        },
{
    smros_ll_fdt_reg_tuple_bytes_body!(address_cells, size_cells)
}

fn ll_fdt_reg_tuple_offset(
    index: usize,
    address_cells: usize,
    size_cells: usize,
) -> (out: Option<usize>)
    ensures
        match out {
            Some(offset) => fdt_reg_tuple_offset_spec(
                index as int,
                address_cells as int,
                size_cells as int,
            ) == Some(offset as int),
            None => fdt_reg_tuple_offset_spec(
                index as int,
                address_cells as int,
                size_cells as int,
            ) == Option::<int>::None,
        },
{
    let _macro_use = smros_ll_fdt_reg_tuple_offset_body!(0usize, 1usize, 0usize);
    match ll_fdt_reg_tuple_bytes(address_cells, size_cells) {
        Some(tuple_bytes) => {
            if index <= usize::MAX / tuple_bytes {
                assert((index as int) * (tuple_bytes as int) <= usize::MAX as int) by(nonlinear_arith)
                    requires
                        tuple_bytes as int > 0,
                        index as int <= usize::MAX as int / tuple_bytes as int,
                ;
                let offset = index * tuple_bytes;
                assert(offset as int == (index as int) * (tuple_bytes as int)) by(nonlinear_arith)
                    requires
                        tuple_bytes as int > 0,
                        index as int <= usize::MAX as int / tuple_bytes as int,
                        offset as int == (index as int) * (tuple_bytes as int),
                ;
                Some(offset)
            } else {
                assert(fdt_reg_tuple_offset_spec(
                    index as int,
                    address_cells as int,
                    size_cells as int,
                ) == Option::<int>::None);
                None
            }
        },
        None => {
            assert(fdt_reg_tuple_offset_spec(
                index as int,
                address_cells as int,
                size_cells as int,
            ) == Option::<int>::None);
            None
        },
    }
}

fn ll_dt_gic_irq(kind: u32, hwirq: u32, max_irqs: u32) -> (out: Option<u32>)
    ensures
        match out {
            Some(irq) => dt_gic_irq_spec(kind as int, hwirq as int, max_irqs as int)
                == Some(irq as int),
            None => dt_gic_irq_spec(kind as int, hwirq as int, max_irqs as int)
                == Option::<int>::None,
        },
{
    smros_ll_dt_gic_irq_body!(kind, hwirq, max_irqs)
}

fn ll_dt_timer_irq_index(entry_count: usize) -> (out: usize)
    ensures
        out as int == dt_timer_irq_index_spec(entry_count as int),
{
    smros_ll_dt_timer_irq_index_body!(entry_count)
}

fn ll_cpu_id_from_mpidr(mpidr: u64) -> (out: u32)
    ensures
        out == (mpidr & 0xFFu64) as u32,
{
    smros_ll_cpu_id_from_mpidr_body!(mpidr)
}

fn ll_valid_cpu_id(cpu_id: u32) -> (out: bool)
    ensures
        out == ((cpu_id as int) < MAX_CPUS as int),
{
    smros_ll_valid_cpu_id_body!(cpu_id, MAX_CPUS)
}

fn ll_display_mpidr(cpu_id: u32) -> (out: u64)
    ensures
        out == (0x8000_0000u64 | (cpu_id as u64)),
{
    smros_ll_display_mpidr_body!(cpu_id)
}

fn ll_psci_success(result: i64) -> (out: bool)
    ensures
        out == (result == PSCI_RET_SUCCESS || result == PSCI_RET_ON_PENDING),
{
    smros_ll_psci_success_body!(result, PSCI_RET_SUCCESS, PSCI_RET_ON_PENDING)
}

proof fn memory_layout_smoke() {
    assert(segment_contains_spec(true, 0x2000, 4, PAGE_SIZE as int, 0x2000));
    assert(!segment_contains_spec(true, 0x2000, 4, PAGE_SIZE as int, 0x6000));
    assert(memory_capacity_ok_spec(3, 1, 63, MAX_SEGMENTS as int, MAX_PAGES_PER_PROCESS as int));
    assert(!memory_capacity_ok_spec(4, 1, 0, MAX_SEGMENTS as int, MAX_PAGES_PER_PROCESS as int));
}

proof fn mmu_constants_smoke() {
    assert(PT_ENTRIES == 512);
    assert(PAGE_SIZE == 4096);
}

proof fn serial_timer_interrupt_smp_smoke() {
    assert(CR_UARTEN | CR_TXE | CR_RXE == 0x301) by (bit_vector);
    assert(CNTP_CTL_ENABLE & !CNTP_CTL_IMASK == 1) by (bit_vector);
    assert(TIMER_IRQ == 30);
    assert(PRIORITY_HIGH == 0x50);
    assert(PSCI_RET_INTERNAL_FAILURE == -2);
}

proof fn thread_context_smoke() {
    assert(THREAD_EMPTY == PROCESS_EMPTY);
    assert(THREAD_READY == PROCESS_READY);
    assert(THREAD_RUNNING == PROCESS_RUNNING);
    assert(THREAD_BLOCKED == PROCESS_BLOCKED);
    assert(THREAD_TERMINATED == PROCESS_TERMINATED);
    assert(THREAD_IDLE_ID == 0);
}

proof fn driver_platform_table_smoke() {
    assert(DEFAULT_PLATFORM_INDEX < PLATFORM_COUNT);
    assert(dt_reg_valid_spec(QEMU_VIRT_UART_BASE as int, QEMU_VIRT_UART_SIZE as int));
    assert(dt_reg_valid_spec(QEMU_VIRT_GICD_BASE as int, QEMU_VIRT_GICD_SIZE as int));
    assert(dt_reg_valid_spec(QEMU_VIRT_GICC_BASE as int, QEMU_VIRT_GICC_SIZE as int));
    assert(dt_reg_contains_spec(
        QEMU_VIRT_UART_BASE as int,
        QEMU_VIRT_UART_SIZE as int,
        (QEMU_VIRT_UART_BASE + UART_FR) as int,
    ));
    assert(dt_irq_valid_spec(QEMU_VIRT_UART_IRQ as int, MAX_PLATFORM_IRQS as int));
    assert(dt_irq_valid_spec(TIMER_IRQ as int, MAX_PLATFORM_IRQS as int));
    assert(dt_platform_index_spec(0, PLATFORM_COUNT as int, DEFAULT_PLATFORM_INDEX as int) == 0);
    assert(dt_platform_index_spec(1, PLATFORM_COUNT as int, DEFAULT_PLATFORM_INDEX as int) == 1);
    assert(dt_platform_index_spec(99, PLATFORM_COUNT as int, DEFAULT_PLATFORM_INDEX as int) == DEFAULT_PLATFORM_INDEX as int);
    assert(fdt_range_valid_spec(40, 64, 4096));
    assert(!fdt_range_valid_spec(4090, 16, 4096));
    assert(fdt_align4_spec(5) == Some(8int));
    assert(fdt_cells_to_bytes_spec(3) == Some(12int));
    assert(fdt_reg_tuple_bytes_spec(2, 1) == Some(12int));
    assert(fdt_reg_tuple_offset_spec(1, 2, 1) == Some(12int));
    assert(dt_gic_irq_spec(0, 1, MAX_PLATFORM_IRQS as int) == Some(33int));
    assert(dt_gic_irq_spec(1, 14, MAX_PLATFORM_IRQS as int) == Some(30int));
    assert(dt_timer_irq_index_spec(4) == 1);
    assert(dt_timer_irq_index_spec(1) == 0);
}

} // verus!
