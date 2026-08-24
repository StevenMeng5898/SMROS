macro_rules! smros_ll_checked_end_body {
    ($addr:expr, $len:expr) => {{
        if $addr <= usize::MAX - $len {
            Some($addr + $len)
        } else {
            None
        }
    }};
}

macro_rules! smros_ll_align_up_body {
    ($size:expr, $align:expr) => {{
        if $align == 0 {
            None
        } else {
            let whole_units = $size / $align;
            let units = if $size % $align == 0 {
                Some(whole_units)
            } else {
                whole_units.checked_add(1)
            };
            match units {
                Some(units) => units.checked_mul($align),
                None => None,
            }
        }
    }};
}

macro_rules! smros_ll_segment_size_body {
    ($page_count:expr, $page_size:expr) => {{
        $page_count.checked_mul($page_size)
    }};
}

macro_rules! smros_ll_segment_end_body {
    ($valid:expr, $base:expr, $page_count:expr, $page_size:expr) => {{
        if !$valid {
            Some(0)
        } else {
            match smros_ll_segment_size_body!($page_count, $page_size) {
                Some(size) => smros_ll_checked_end_body!($base, size),
                None => None,
            }
        }
    }};
}

#[cfg(not(target_os = "none"))]
macro_rules! smros_ll_segment_contains_body {
    ($valid:expr, $base:expr, $page_count:expr, $page_size:expr, $vaddr:expr) => {{
        match smros_ll_segment_end_body!($valid, $base, $page_count, $page_size) {
            Some(end) => $valid && $vaddr >= $base && $vaddr < end,
            None => false,
        }
    }};
}

macro_rules! smros_ll_memory_capacity_ok_body {
    ($segment_count:expr, $page_count:expr, $valid_page_count:expr, $max_segments:expr, $max_pages:expr) => {{
        $segment_count < $max_segments
            && $valid_page_count <= $max_pages
            && $page_count != 0
            && $page_count <= $max_pages - $valid_page_count
    }};
}

macro_rules! smros_ll_permission_writable_body {
    ($permission:expr, $write:expr, $read_write:expr) => {{
        $permission == $read_write || $permission == $write
    }};
}

macro_rules! smros_ll_permission_executable_body {
    ($permission:expr, $execute:expr, $read_execute:expr) => {{
        $permission == $read_execute || $permission == $execute
    }};
}

#[cfg(not(target_os = "none"))]
macro_rules! smros_ll_heap_alloc_body {
    ($current:expr, $max:expr, $size:expr, $page_size:expr) => {{
        match smros_ll_align_up_body!($size, $page_size) {
            Some(aligned_size) => match smros_ll_checked_end_body!($current, aligned_size) {
                Some(next) if next <= $max => Some(($current, next)),
                _ => None,
            },
            None => None,
        }
    }};
}

#[cfg(not(target_os = "none"))]
macro_rules! smros_ll_stack_alloc_body {
    ($current:expr, $size:expr, $page_size:expr) => {{
        match smros_ll_align_up_body!($size, $page_size) {
            Some(aligned_size) if $current >= aligned_size => Some($current - aligned_size),
            _ => None,
        }
    }};
}

#[cfg(not(target_os = "none"))]
macro_rules! smros_ll_page_to_vaddr_body {
    ($page_idx:expr, $valid_page_count:expr, $page_size:expr) => {{
        if $page_idx >= $valid_page_count {
            None
        } else {
            $page_idx.checked_mul($page_size)
        }
    }};
}

#[cfg(not(target_os = "none"))]
macro_rules! smros_ll_pfn_valid_body {
    ($pfn:expr, $total_pages:expr) => {{
        ($pfn as usize) < $total_pages
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_memory_reg_body {
    ($detected:expr, $fallback_base:expr, $fallback_size:expr) => {{
        let (base, size) = $detected.unwrap_or(($fallback_base, $fallback_size));
        if size != 0 && base.checked_add(size).is_some() {
            Some((base, size))
        } else {
            None
        }
    }};
}

pub(crate) fn memory_reg(
    detected: Option<(usize, usize)>,
    fallback_base: usize,
    fallback_size: usize,
) -> Option<(usize, usize)> {
    smros_ll_memory_reg_body!(detected, fallback_base, fallback_size)
}

pub(crate) struct PageFrameAllocatorCore<const WORDS: usize> {
    bitmap: [u64; WORDS],
    base_pfn: u64,
    total_pages: usize,
    allocated_pages: usize,
    next_search_page: usize,
}

impl<const WORDS: usize> PageFrameAllocatorCore<WORDS> {
    pub(crate) const fn new(total_pages: usize) -> Self {
        Self {
            bitmap: [0; WORDS],
            base_pfn: 0,
            total_pages,
            allocated_pages: 0,
            next_search_page: 0,
        }
    }

    pub(crate) fn init_range(&mut self, start: usize, end: usize, page_size: usize) -> bool {
        if page_size == 0 || start % page_size != 0 || end % page_size != 0 || start >= end {
            return false;
        }

        let pages = (end - start) / page_size;
        let Some(capacity) = WORDS.checked_mul(64) else {
            return false;
        };
        if pages > capacity {
            return false;
        }

        self.bitmap.fill(0);
        self.base_pfn = (start / page_size) as u64;
        self.total_pages = pages;
        self.allocated_pages = 0;
        self.next_search_page = 0;
        true
    }

    pub(crate) fn alloc(&mut self) -> Option<u64> {
        if self.total_pages == 0 {
            return None;
        }

        let start = self.next_search_page.min(self.total_pages - 1);
        for offset in 0..self.total_pages {
            let page_index = (start + offset) % self.total_pages;
            let word_index = page_index / 64;
            let bit_index = page_index % 64;
            let mask = 1u64 << bit_index;
            if self.bitmap[word_index] & mask == 0 {
                self.bitmap[word_index] |= mask;
                self.allocated_pages += 1;
                self.next_search_page = (page_index + 1) % self.total_pages;
                return self.base_pfn.checked_add(page_index as u64);
            }
        }

        None
    }

    pub(crate) fn free(&mut self, pfn: u64) -> bool {
        let Some(page_index) = pfn
            .checked_sub(self.base_pfn)
            .filter(|index| *index < self.total_pages as u64)
            .map(|index| index as usize)
        else {
            return false;
        };
        let word_index = page_index / 64;
        let mask = 1u64 << (page_index % 64);
        if self.bitmap[word_index] & mask == 0 {
            return false;
        }

        self.bitmap[word_index] &= !mask;
        self.allocated_pages -= 1;
        self.next_search_page = self.next_search_page.min(page_index);
        true
    }

    pub(crate) fn pfn_address(&self, pfn: u64, page_size: usize) -> Option<usize> {
        let index = pfn.checked_sub(self.base_pfn)?;
        if index >= self.total_pages as u64 || pfn > usize::MAX as u64 {
            return None;
        }
        (pfn as usize).checked_mul(page_size)
    }

    pub(crate) fn pfn_index(&self, pfn: u64) -> Option<usize> {
        let index = pfn.checked_sub(self.base_pfn)?;
        (index < self.total_pages as u64).then_some(index as usize)
    }

    pub(crate) const fn total_pages(&self) -> usize {
        self.total_pages
    }

    pub(crate) const fn allocated_pages(&self) -> usize {
        self.allocated_pages
    }

    pub(crate) const fn free_pages(&self) -> usize {
        self.total_pages - self.allocated_pages
    }
}

#[allow(unused_macros)]
macro_rules! smros_ll_pfn_from_index_body {
    ($index:expr, $base_pfn:expr) => {{
        ($base_pfn).checked_add($index as u64)
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_pfn_index_body {
    ($pfn:expr, $base_pfn:expr, $total_pages:expr) => {{
        ($pfn)
            .checked_sub($base_pfn)
            .filter(|index| *index < $total_pages as u64)
            .map(|index| index as usize)
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_pfn_address_body {
    ($pfn:expr, $base_pfn:expr, $total_pages:expr, $page_size:expr) => {{
        smros_ll_pfn_index_body!($pfn, $base_pfn, $total_pages)
            .and_then(|_| ($pfn as usize).checked_mul($page_size))
    }};
}

#[cfg(not(target_os = "none"))]
macro_rules! smros_ll_bitmap_word_index_body {
    ($pfn:expr) => {{
        ($pfn as usize) / 64
    }};
}

#[cfg(not(target_os = "none"))]
macro_rules! smros_ll_bitmap_bit_index_body {
    ($pfn:expr) => {{
        ($pfn as usize) % 64
    }};
}

#[cfg(not(target_os = "none"))]
macro_rules! smros_ll_bitmap_mask_body {
    ($bit:expr) => {{
        1u64 << $bit
    }};
}

macro_rules! smros_ll_process_index_valid_body {
    ($index:expr, $max_processes:expr) => {{
        $index < $max_processes
    }};
}

macro_rules! smros_ll_thread_state_runnable_body {
    ($state:expr, $ready:expr, $running:expr) => {{
        $state == $ready || $state == $running
    }};
}

macro_rules! smros_ll_thread_id_idle_body {
    ($id:expr, $idle:expr) => {{
        $id == $idle
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_pte_set_flag_body {
    ($value:expr, $flag:expr, $enabled:expr) => {{
        if $enabled {
            $value | $flag
        } else {
            $value & !$flag
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_pte_output_address_body {
    ($value:expr) => {{
        $value & 0x0000_FFFF_FFFF_F000u64
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_pte_set_output_address_body {
    ($value:expr, $paddr:expr) => {{
        ($value & !0x0000_FFFF_FFFF_F000u64) | ($paddr & 0x0000_FFFF_FFFF_F000u64)
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_pte_attr_idx_body {
    ($value:expr, $idx:expr) => {{
        ($value & !0x1Cu64) | (($idx << 2) & 0x1Cu64)
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_pte_sh_body {
    ($value:expr, $sharability:expr) => {{
        ($value & !0x300u64) | (($sharability << 8) & 0x300u64)
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_pte_table_body {
    ($value:expr) => {{
        ($value & 1u64) != 0 && ($value & (1u64 << 1)) == 0
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_pt_index_body {
    ($vaddr:expr, $entries:expr) => {{
        ($vaddr >> 21) & ($entries - 1)
    }};
}

macro_rules! smros_ll_vma_size_body {
    ($start:expr, $end:expr) => {{
        if $end >= $start {
            $end - $start
        } else {
            0
        }
    }};
}

macro_rules! smros_ll_mmio_addr_body {
    ($base:expr, $offset:expr) => {{
        smros_ll_checked_end_body!($base, $offset)
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_uart_control_body {
    ($uarten:expr, $txe:expr, $rxe:expr) => {{
        $uarten | $txe | $rxe
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_uart_lcrh_body {
    ($word_len_8:expr, $fifo_enable:expr) => {{
        $word_len_8 | $fifo_enable
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_uart_has_byte_body {
    ($flags:expr, $rx_empty_flag:expr) => {{
        ($flags & $rx_empty_flag) == 0
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_uart_tx_ready_body {
    ($flags:expr, $tx_full_flag:expr) => {{
        ($flags & $tx_full_flag) == 0
    }};
}

macro_rules! smros_ll_ascii_printable_body {
    ($byte:expr) => {{
        $byte >= 0x20 && $byte <= 0x7e
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_hex_digit_body {
    ($nibble:expr) => {{
        if $nibble < 10 {
            48u8 + $nibble as u8
        } else {
            97u8 + ($nibble as u8 - 10)
        }
    }};
}

macro_rules! smros_ll_timer_period_body {
    ($frequency:expr) => {{
        $frequency / 100
    }};
}

macro_rules! smros_ll_timer_compare_body {
    ($current:expr, $period:expr) => {{
        if $period == 0 {
            $current
        } else {
            ($current / $period)
                .checked_add(1)
                .and_then(|tick| tick.checked_mul($period))
                .unwrap_or_else(|| $current.wrapping_add($period))
        }
    }};
}

macro_rules! smros_ll_timer_tick_count_body {
    ($counter:expr, $period:expr) => {{
        if $period == 0 {
            0
        } else {
            $counter / $period
        }
    }};
}

macro_rules! smros_ll_timer_counter_nanoseconds_body {
    ($counter:expr, $frequency:expr) => {{
        if $frequency == 0 {
            0
        } else {
            let nanoseconds = ($counter as u128).saturating_mul(1_000_000_000u128)
                / ($frequency as u128);
            nanoseconds.min(u64::MAX as u128) as u64
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_timer_ctl_body {
    ($enable:expr, $imask:expr) => {{
        $enable & !$imask
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_gic_reg_offset_body {
    ($base_offset:expr, $irq:expr, $field_width:expr) => {{
        $base_offset + (($irq as usize / $field_width) * 4)
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_gic_byte_shift_body {
    ($irq:expr) => {{
        (($irq % 4) as usize) * 8
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_gic_set_byte_field_body {
    ($value:expr, $byte_shift:expr, $field:expr) => {{
        ($value & !(0xFFu32 << $byte_shift)) | (($field as u32) << $byte_shift)
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_gic_enable_bit_body {
    ($irq:expr) => {{
        1u32 << ($irq % 32)
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_gic_interrupt_id_body {
    ($iar:expr) => {{
        $iar & 0x3FFu32
    }};
}

macro_rules! smros_ll_dt_reg_valid_body {
    ($base:expr, $size:expr) => {{
        if $size == 0 {
            false
        } else {
            match smros_ll_checked_end_body!($base, $size) {
                Some(_) => true,
                None => false,
            }
        }
    }};
}

macro_rules! smros_ll_dt_reg_contains_body {
    ($base:expr, $size:expr, $addr:expr) => {{
        if $size == 0 {
            false
        } else {
            match smros_ll_checked_end_body!($base, $size) {
                Some(end) => $addr >= $base && $addr < end,
                None => false,
            }
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_dt_irq_valid_body {
    ($irq:expr, $max_irqs:expr) => {{
        $max_irqs != 0 && $irq < $max_irqs
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_dt_platform_index_body {
    ($candidate:expr, $platform_count:expr, $fallback:expr) => {{
        if $candidate < $platform_count {
            $candidate
        } else if $fallback < $platform_count {
            $fallback
        } else {
            0
        }
    }};
}

macro_rules! smros_ll_fdt_range_valid_body {
    ($offset:expr, $len:expr, $total:expr) => {{
        $offset <= $total && $len <= $total - $offset
    }};
}

macro_rules! smros_ll_fdt_align4_body {
    ($offset:expr) => {{
        smros_ll_align_up_body!($offset, 4usize)
    }};
}

macro_rules! smros_ll_fdt_cells_to_bytes_body {
    ($cells:expr) => {{
        $cells.checked_mul(4usize)
    }};
}

macro_rules! smros_ll_fdt_reg_tuple_bytes_body {
    ($address_cells:expr, $size_cells:expr) => {{
        match $address_cells.checked_add($size_cells) {
            Some(cells) if cells != 0 => smros_ll_fdt_cells_to_bytes_body!(cells),
            _ => None,
        }
    }};
}

macro_rules! smros_ll_fdt_reg_tuple_offset_body {
    ($index:expr, $address_cells:expr, $size_cells:expr) => {{
        match smros_ll_fdt_reg_tuple_bytes_body!($address_cells, $size_cells) {
            Some(tuple_bytes) => $index.checked_mul(tuple_bytes),
            None => None,
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_dt_gic_irq_body {
    ($kind:expr, $hwirq:expr, $max_irqs:expr) => {{
        let translated = if $kind == 0 {
            $hwirq.checked_add(32)
        } else if $kind == 1 {
            $hwirq.checked_add(16)
        } else {
            None
        };
        match translated {
            Some(irq) if irq < $max_irqs => Some(irq),
            _ => None,
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_dt_timer_irq_index_body {
    ($entry_count:expr) => {{
        if $entry_count >= 4 {
            1usize
        } else {
            0usize
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_cpu_id_from_mpidr_body {
    ($mpidr:expr) => {{
        ($mpidr & 0xFFu64) as u32
    }};
}

macro_rules! smros_ll_valid_cpu_id_body {
    ($cpu_id:expr, $max_cpus:expr) => {{
        ($cpu_id as usize) < $max_cpus
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_display_mpidr_body {
    ($cpu_id:expr) => {{
        0x8000_0000u64 | ($cpu_id as u64)
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ll_psci_success_body {
    ($result:expr, $success:expr, $on_pending:expr) => {{
        $result == $success || $result == $on_pending
    }};
}
