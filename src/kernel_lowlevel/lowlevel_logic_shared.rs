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

macro_rules! smros_ll_stack_alloc_body {
    ($current:expr, $size:expr, $page_size:expr) => {{
        match smros_ll_align_up_body!($size, $page_size) {
            Some(aligned_size) if $current >= aligned_size => Some($current - aligned_size),
            _ => None,
        }
    }};
}

macro_rules! smros_ll_page_to_vaddr_body {
    ($page_idx:expr, $valid_page_count:expr, $page_size:expr) => {{
        if $page_idx >= $valid_page_count {
            None
        } else {
            $page_idx.checked_mul($page_size)
        }
    }};
}

macro_rules! smros_ll_pfn_valid_body {
    ($pfn:expr, $total_pages:expr) => {{
        ($pfn as usize) < $total_pages
    }};
}

macro_rules! smros_ll_bitmap_word_index_body {
    ($pfn:expr) => {{
        ($pfn as usize) / 64
    }};
}

macro_rules! smros_ll_bitmap_bit_index_body {
    ($pfn:expr) => {{
        ($pfn as usize) % 64
    }};
}

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

macro_rules! smros_ll_pte_set_flag_body {
    ($value:expr, $flag:expr, $enabled:expr) => {{
        if $enabled {
            $value | $flag
        } else {
            $value & !$flag
        }
    }};
}

macro_rules! smros_ll_pte_output_address_body {
    ($paddr:expr) => {{
        $paddr & 0x0000_FFFF_FFFF_F000u64
    }};
}

macro_rules! smros_ll_pte_set_output_address_body {
    ($value:expr, $paddr:expr) => {{
        ($value & 0xFFFu64) | smros_ll_pte_output_address_body!($paddr)
    }};
}

macro_rules! smros_ll_pte_attr_idx_body {
    ($value:expr, $idx:expr) => {{
        ($value & !0x1Cu64) | (($idx << 2) & 0x1Cu64)
    }};
}

macro_rules! smros_ll_pte_sh_body {
    ($value:expr, $sharability:expr) => {{
        ($value & !0x300u64) | (($sharability << 8) & 0x300u64)
    }};
}

macro_rules! smros_ll_pte_table_body {
    ($value:expr) => {{
        ($value & 1u64) != 0 && ($value & (1u64 << 1)) == 0
    }};
}

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

macro_rules! smros_ll_uart_control_body {
    ($uarten:expr, $txe:expr, $rxe:expr) => {{
        $uarten | $txe | $rxe
    }};
}

macro_rules! smros_ll_uart_lcrh_body {
    ($word_len_8:expr, $fifo_enable:expr) => {{
        $word_len_8 | $fifo_enable
    }};
}

macro_rules! smros_ll_uart_has_byte_body {
    ($flags:expr, $rx_empty_flag:expr) => {{
        ($flags & $rx_empty_flag) == 0
    }};
}

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
        $current.wrapping_add($period)
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

macro_rules! smros_ll_timer_ctl_body {
    ($enable:expr, $imask:expr) => {{
        $enable | $imask
    }};
}

macro_rules! smros_ll_gic_reg_offset_body {
    ($base_offset:expr, $irq:expr, $field_width:expr) => {{
        $base_offset + (($irq as usize / $field_width) * 4)
    }};
}

macro_rules! smros_ll_gic_byte_shift_body {
    ($irq:expr) => {{
        (($irq % 4) as usize) * 8
    }};
}

macro_rules! smros_ll_gic_set_byte_field_body {
    ($value:expr, $byte_shift:expr, $field:expr) => {{
        ($value & !(0xFFu32 << $byte_shift)) | (($field as u32) << $byte_shift)
    }};
}

macro_rules! smros_ll_gic_enable_bit_body {
    ($irq:expr) => {{
        1u32 << ($irq % 32)
    }};
}

macro_rules! smros_ll_gic_interrupt_id_body {
    ($iar:expr) => {{
        $iar & 0x3FFu32
    }};
}

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

macro_rules! smros_ll_display_mpidr_body {
    ($cpu_id:expr) => {{
        0x8000_0000u64 | ($cpu_id as u64)
    }};
}

macro_rules! smros_ll_psci_success_body {
    ($result:expr, $success:expr, $on_pending:expr) => {{
        $result == $success || $result == $on_pending
    }};
}
