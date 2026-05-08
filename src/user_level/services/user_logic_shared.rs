macro_rules! smros_user_checked_end_body {
    ($addr:expr, $len:expr) => {{
        if $addr <= usize::MAX - $len {
            Some($addr + $len)
        } else {
            None
        }
    }};
}

macro_rules! smros_user_page_offset_body {
    ($base:expr, $page_index:expr, $page_size:expr) => {{
        match $page_index.checked_mul($page_size) {
            Some(offset) => smros_user_checked_end_body!($base, offset),
            None => None,
        }
    }};
}

macro_rules! smros_user_page_down_body {
    ($value:expr, $page_size:expr) => {{
        if $page_size == 0 {
            None
        } else {
            $value.checked_sub($value % $page_size)
        }
    }};
}

macro_rules! smros_user_page_up_body {
    ($value:expr, $page_size:expr) => {{
        if $page_size == 0 {
            None
        } else {
            match $value.checked_add($page_size - 1) {
                Some(adjusted) => adjusted.checked_sub(adjusted % $page_size),
                None => None,
            }
        }
    }};
}

macro_rules! smros_user_pfn_to_paddr_body {
    ($pfn:expr, $page_size:expr) => {{
        $pfn.checked_mul($page_size)
    }};
}

macro_rules! smros_user_stack_top_u64_body {
    ($stack_base:expr, $stack_size:expr) => {{
        $stack_base.checked_add($stack_size as u64)
    }};
}

macro_rules! smros_user_el0_thread_pstate_body {
    () => {
        0x3C0u64
    };
}

macro_rules! smros_user_el0_spsr_body {
    () => {
        0u64
    };
}

macro_rules! smros_user_el1h_spsr_masked_body {
    () => {
        0x3C5u64
    };
}

macro_rules! smros_user_syscall_should_advance_elr_body {
    () => {
        0u64
    };
}

macro_rules! smros_user_ascii_shell_input_body {
    ($byte:expr) => {{
        $byte >= 0x20 && $byte <= 0x7e
    }};
}

macro_rules! smros_user_decimal_digit_value_body {
    ($byte:expr) => {{
        if $byte >= 48u8 && $byte <= 57u8 {
            Some(($byte - 48u8) as usize)
        } else {
            None
        }
    }};
}

macro_rules! smros_user_parse_digit_step_body {
    ($result:expr, $digit:expr) => {{
        match $result.checked_mul(10) {
            Some(scaled) => scaled.checked_add($digit),
            None => None,
        }
    }};
}

macro_rules! smros_user_ipv4_octet_step_body {
    ($value:expr, $digit:expr) => {{
        match $value.checked_mul(10) {
            Some(scaled) => match scaled.checked_add($digit) {
                Some(next) if next <= 255 => Some(next),
                _ => None,
            },
            None => None,
        }
    }};
}

macro_rules! smros_user_saturating_sub_body {
    ($lhs:expr, $rhs:expr) => {{
        if $lhs >= $rhs {
            $lhs - $rhs
        } else {
            0
        }
    }};
}

macro_rules! smros_user_pages_to_kb_body {
    ($pages:expr, $page_size:expr) => {{
        match $pages.checked_mul($page_size) {
            Some(bytes) => bytes / 1024,
            None => usize::MAX,
        }
    }};
}

macro_rules! smros_user_usage_percent_body {
    ($used_pages:expr, $total_pages:expr) => {{
        if $total_pages == 0 {
            0
        } else {
            match $used_pages.checked_mul(100) {
                Some(scaled) => scaled / $total_pages,
                None => usize::MAX,
            }
        }
    }};
}

macro_rules! smros_user_uptime_parts_body {
    ($ticks:expr) => {{
        let seconds = $ticks / 100;
        let minutes = seconds / 60;
        let hours = minutes / 60;
        let days = hours / 24;
        (seconds, minutes, hours, days)
    }};
}

macro_rules! smros_user_mmap_result_ok_body {
    ($addr:expr, $page_size:expr, $base:expr, $limit:expr) => {{
        $page_size != 0 && $addr >= $base && $addr < $limit && $addr % $page_size == 0
    }};
}

macro_rules! smros_user_dns_host_len_valid_body {
    ($len:expr, $max_len:expr) => {{
        $len > 0 && $len <= $max_len
    }};
}

macro_rules! smros_user_dns_label_len_valid_body {
    ($len:expr, $max_len:expr) => {{
        $len > 0 && $len <= $max_len
    }};
}

macro_rules! smros_user_dns_label_byte_valid_body {
    ($byte:expr) => {{
        ($byte >= 0x61u8 && $byte <= 0x7au8)
            || ($byte >= 0x41u8 && $byte <= 0x5au8)
            || ($byte >= 0x30u8 && $byte <= 0x39u8)
            || $byte == 0x2du8
    }};
}

macro_rules! smros_user_kernel_success_body {
    (
        $kernel_entered:expr,
        $kernel_finished:expr,
        $exit_code:expr,
        $kernel_write:expr,
        $kernel_pid:expr,
        $kernel_mmap:expr,
        $banner_len:expr
    ) => {{
        $kernel_entered
            && $kernel_finished
            && $exit_code == 0
            && $kernel_write == $banner_len as u64
            && $kernel_pid == 1
            && $kernel_mmap > 0
            && $kernel_mmap < 0xFFFF_FFFF_FFFF_F000u64
    }};
}

macro_rules! smros_user_component_start_allowed_body {
    ($binary_exists:expr, $destroyed:expr, $already_started:expr) => {{
        $already_started || ($binary_exists && !$destroyed)
    }};
}

macro_rules! smros_user_namespace_rights_valid_body {
    ($rights:expr, $allowed_mask:expr) => {{
        $rights & !$allowed_mask == 0
    }};
}

macro_rules! smros_user_fxfs_file_size_valid_body {
    ($size:expr, $max_size:expr) => {{
        $size <= $max_size
    }};
}

macro_rules! smros_user_fxfs_node_capacity_valid_body {
    ($nodes:expr, $max_nodes:expr) => {{
        $nodes < $max_nodes
    }};
}

macro_rules! smros_user_fxfs_dirent_capacity_valid_body {
    ($entries:expr, $max_entries:expr) => {{
        $entries < $max_entries
    }};
}

macro_rules! smros_user_fxfs_append_size_body {
    ($old_size:expr, $append_len:expr) => {{
        $old_size.checked_add($append_len)
    }};
}

macro_rules! smros_user_fxfs_write_end_body {
    ($offset:expr, $len:expr) => {{
        $offset.checked_add($len)
    }};
}

macro_rules! smros_user_fxfs_seek_valid_body {
    ($offset:expr, $size:expr) => {{
        $offset <= $size
    }};
}

macro_rules! smros_user_fxfs_replay_count_valid_body {
    ($replayed:expr, $journal_records:expr) => {{
        $replayed <= $journal_records
    }};
}

macro_rules! smros_user_svc_name_valid_body {
    ($len:expr, $max_len:expr) => {{
        $len > 0 && $len <= $max_len
    }};
}

macro_rules! smros_user_svc_rights_valid_body {
    ($rights:expr, $allowed_mask:expr) => {{
        $rights != 0 && ($rights & !$allowed_mask) == 0
    }};
}

macro_rules! smros_user_svc_ipc_message_size_valid_body {
    ($size:expr, $expected:expr) => {{
        $size == $expected
    }};
}

macro_rules! smros_user_svc_ipc_header_valid_body {
    ($magic:expr, $version:expr, $expected_magic:expr, $expected_version:expr) => {{
        $magic == $expected_magic && $version == $expected_version
    }};
}

macro_rules! smros_user_svc_protocol_allowed_body {
    ($service:expr, $ordinal:expr, $component_manager:expr, $runner:expr, $filesystem:expr, $component_start:expr, $runner_load:expr, $filesystem_describe:expr) => {{
        ($service == $component_manager && $ordinal == $component_start)
            || ($service == $runner && $ordinal == $runner_load)
            || ($service == $filesystem && $ordinal == $filesystem_describe)
    }};
}

macro_rules! smros_user_component_thread_launch_valid_body {
    ($process_created:expr, $queued:expr, $thread_created:expr) => {{
        $process_created && $queued && $thread_created
    }};
}

macro_rules! smros_user_component_return_active_body {
    ($pid:expr) => {{
        $pid != 0
    }};
}

macro_rules! smros_user_elf_header_bounds_valid_body {
    ($image_len:expr, $header_size:expr) => {{
        $image_len >= $header_size
    }};
}

macro_rules! smros_user_elf_magic_valid_body {
    ($b0:expr, $b1:expr, $b2:expr, $b3:expr) => {{
        $b0 == 0x7fu8 && $b1 == 0x45u8 && $b2 == 0x4cu8 && $b3 == 0x46u8
    }};
}

macro_rules! smros_user_elf_class_data_valid_body {
    ($class:expr, $data:expr, $version:expr) => {{
        $class == 2u8 && $data == 1u8 && $version == 1u8
    }};
}

macro_rules! smros_user_elf_type_valid_body {
    ($elf_type:expr, $exec_type:expr, $dyn_type:expr) => {{
        $elf_type == $exec_type || $elf_type == $dyn_type
    }};
}

macro_rules! smros_user_elf_machine_valid_body {
    ($machine:expr, $expected:expr) => {{
        $machine == $expected
    }};
}

macro_rules! smros_user_elf_entry_valid_body {
    ($entry:expr) => {{
        $entry != 0
    }};
}

macro_rules! smros_user_elf_phdr_table_valid_body {
    ($phoff:expr, $phentsize:expr, $phnum:expr, $image_len:expr, $expected_phentsize:expr, $max_phnum:expr) => {{
        if $phentsize != $expected_phentsize || $phnum == 0 || $phnum > $max_phnum {
            false
        } else {
            match $phentsize.checked_mul($phnum) {
                Some(table_size) => match $phoff.checked_add(table_size) {
                    Some(end) => end <= $image_len,
                    None => false,
                },
                None => false,
            }
        }
    }};
}

macro_rules! smros_user_elf_segment_bounds_valid_body {
    ($offset:expr, $file_size:expr, $mem_size:expr, $image_len:expr) => {{
        if $mem_size < $file_size {
            false
        } else {
            match $offset.checked_add($file_size) {
                Some(end) => end <= $image_len,
                None => false,
            }
        }
    }};
}

macro_rules! smros_user_elf_vaddr_range_valid_body {
    ($vaddr:expr, $mem_size:expr) => {{
        $vaddr.checked_add($mem_size).is_some()
    }};
}

macro_rules! smros_user_elf_segment_mapping_range_body {
    ($vaddr:expr, $mem_size:expr, $page_size:expr) => {{
        if $mem_size == 0 {
            None
        } else {
            match smros_user_checked_end_body!($vaddr, $mem_size) {
                Some(end) => match smros_user_page_down_body!($vaddr, $page_size) {
                    Some(start) => match smros_user_page_up_body!(end, $page_size) {
                        Some(aligned_end) => Some((start, aligned_end)),
                        None => None,
                    },
                    None => None,
                },
                None => None,
            }
        }
    }};
}
