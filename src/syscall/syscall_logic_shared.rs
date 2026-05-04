macro_rules! smros_zircon_syscall_from_raw_body {
    ($syscall_num:expr, $threshold:expr) => {{
        if smros_is_zircon_syscall_number_body!($syscall_num, $threshold) {
            ($syscall_num - $threshold) as u32
        } else {
            u32::MAX
        }
    }};
}

macro_rules! smros_is_zircon_syscall_number_body {
    ($syscall_num:expr, $threshold:expr) => {{
        $syscall_num >= $threshold && $syscall_num - $threshold <= u32::MAX as u64
    }};
}

macro_rules! smros_syscall_handle_invalid_body {
    ($handle:expr, $invalid:expr) => {{
        $handle == 0 || $handle == $invalid
    }};
}

macro_rules! smros_syscall_user_buffer_valid_body {
    ($ptr:expr, $len:expr) => {{
        $len == 0 || $ptr != 0
    }};
}

macro_rules! smros_syscall_channel_buffers_valid_body {
    ($bytes_ptr:expr, $bytes_len:expr, $handles_ptr:expr, $handles_len:expr) => {{
        smros_syscall_user_buffer_valid_body!($bytes_ptr, $bytes_len)
            && smros_syscall_user_buffer_valid_body!($handles_ptr, $handles_len)
    }};
}

macro_rules! smros_syscall_signal_update_body {
    ($current:expr, $clear_mask:expr, $set_mask:expr) => {{
        ($current & !$clear_mask) | $set_mask
    }};
}

macro_rules! smros_syscall_signal_mask_allowed_body {
    ($clear_mask:expr, $set_mask:expr, $allowed_mask:expr) => {{
        (($clear_mask | $set_mask) & !$allowed_mask) == 0
    }};
}

macro_rules! smros_syscall_user_signal_mask_body {
    () => {{
        0xffu32 << 24
    }};
}

macro_rules! smros_syscall_event_signal_mask_body {
    () => {{
        smros_syscall_user_signal_mask_body!() | (1u32 << 4)
    }};
}

macro_rules! smros_syscall_eventpair_signal_mask_body {
    () => {{
        smros_syscall_user_signal_mask_body!() | (1u32 << 4)
    }};
}

macro_rules! smros_syscall_wait_satisfied_body {
    ($observed:expr, $requested:expr) => {{
        $requested == 0 || ($observed & $requested) != 0
    }};
}

macro_rules! smros_linux_clock_id_supported_body {
    ($clock_id:expr) => {{
        $clock_id <= 1
    }};
}

macro_rules! smros_linux_signal_valid_body {
    ($signum:expr, $max_signal:expr) => {{
        $signum <= $max_signal
    }};
}

macro_rules! smros_linux_signal_action_valid_body {
    ($signum:expr, $max_signal:expr) => {{
        $signum != 0 && $signum <= $max_signal
    }};
}

macro_rules! smros_linux_sigset_size_valid_body {
    ($size:expr, $expected:expr) => {{
        $size == $expected
    }};
}

macro_rules! smros_linux_ipc_count_valid_body {
    ($count:expr, $max_count:expr) => {{
        $count != 0 && $count <= $max_count
    }};
}

macro_rules! smros_linux_ipc_size_valid_body {
    ($size:expr, $max_size:expr) => {{
        $size != 0 && $size <= $max_size
    }};
}

macro_rules! smros_linux_msg_size_valid_body {
    ($size:expr, $max_size:expr) => {{
        $size <= $max_size
    }};
}

macro_rules! smros_linux_socket_domain_supported_body {
    ($domain:expr, $unix:expr, $local:expr, $inet:expr, $netlink:expr, $packet:expr) => {{
        $domain == $unix
            || $domain == $local
            || $domain == $inet
            || $domain == $netlink
            || $domain == $packet
    }};
}

macro_rules! smros_linux_socket_type_supported_body {
    ($socket_type:expr, $mask:expr, $stream:expr, $dgram:expr, $raw:expr) => {{
        {
            let kind = $socket_type & $mask;
            kind == $stream || kind == $dgram || kind == $raw
        }
    }};
}

macro_rules! smros_linux_socket_domain_type_supported_body {
    ($domain:expr, $kind:expr, $unix:expr, $local:expr, $inet:expr, $netlink:expr, $packet:expr, $stream:expr, $dgram:expr, $raw:expr) => {{
        if $domain == $unix || $domain == $local {
            $kind == $stream || $kind == $dgram
        } else if $domain == $inet {
            $kind == $stream || $kind == $dgram || $kind == $raw
        } else if $domain == $netlink || $domain == $packet {
            $kind == $dgram || $kind == $raw
        } else {
            false
        }
    }};
}

macro_rules! smros_linux_socket_addr_valid_body {
    ($ptr:expr, $len:expr) => {{
        smros_syscall_user_buffer_valid_body!($ptr, $len)
    }};
}

macro_rules! smros_linux_fd_range_valid_body {
    ($first:expr, $last:expr) => {{
        $first <= $last
    }};
}

macro_rules! smros_linux_memfd_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_getrandom_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_open_access_mode_valid_body {
    ($flags:expr, $access_mask:expr, $read_only:expr, $write_only:expr, $read_write:expr) => {{
        {
            let access = $flags & $access_mask;
            access == $read_only || access == $write_only || access == $read_write
        }
    }};
}

macro_rules! smros_linux_open_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_open_is_directory_body {
    ($flags:expr, $directory_flag:expr) => {{
        ($flags & $directory_flag) != 0
    }};
}

macro_rules! smros_linux_fd_target_valid_body {
    ($fd:expr, $stdio_max:expr) => {{
        $fd <= $stdio_max
    }};
}

macro_rules! smros_linux_pipe_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_dup3_args_valid_body {
    ($old_fd:expr, $new_fd:expr) => {{
        $old_fd != $new_fd
    }};
}

macro_rules! smros_linux_fcntl_cmd_supported_body {
    ($cmd:expr, $dupfd:expr, $getfd:expr, $setfd:expr, $getfl:expr, $setfl:expr, $dupfd_cloexec:expr) => {{
        $cmd == $dupfd
            || $cmd == $getfd
            || $cmd == $setfd
            || $cmd == $getfl
            || $cmd == $setfl
            || $cmd == $dupfd_cloexec
    }};
}

macro_rules! smros_linux_fcntl_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_path_mode_valid_body {
    ($mode:expr, $allowed_mask:expr) => {{
        ($mode & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_unlink_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_rename_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_stat_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_stat_mask_valid_body {
    ($mask:expr, $allowed_mask:expr) => {{
        ($mask & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_lseek_whence_valid_body {
    ($whence:expr, $max_whence:expr) => {{
        $whence <= $max_whence
    }};
}

macro_rules! smros_linux_iov_count_valid_body {
    ($count:expr, $max_count:expr) => {{
        $count <= $max_count
    }};
}

macro_rules! smros_linux_iov_bytes_valid_body {
    ($count:expr, $elem_size:expr, $max_count:expr) => {{
        $elem_size != 0 && $count <= $max_count && $count <= usize::MAX / $elem_size
    }};
}

macro_rules! smros_linux_poll_count_valid_body {
    ($count:expr, $max_count:expr) => {{
        $count <= $max_count
    }};
}

macro_rules! smros_linux_poll_events_valid_body {
    ($events:expr, $allowed_mask:expr) => {{
        ($events & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_copy_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_zircon_clock_id_supported_body {
    ($clock_id:expr) => {{
        $clock_id <= 1
    }};
}

macro_rules! smros_zircon_clock_create_options_valid_body {
    ($options:expr, $allowed_mask:expr) => {{
        ($options & !$allowed_mask) == 0
    }};
}

macro_rules! smros_zircon_clock_update_options_valid_body {
    ($options:expr, $allowed_mask:expr) => {{
        ($options & !$allowed_mask) == 0
    }};
}

macro_rules! smros_zircon_timer_options_valid_body {
    ($options:expr, $allowed_mask:expr) => {{
        ($options & !$allowed_mask) == 0
    }};
}

macro_rules! smros_zircon_timer_deadline_expired_body {
    ($deadline:expr, $now:expr) => {{
        $deadline <= $now
    }};
}

macro_rules! smros_zircon_debuglog_create_options_valid_body {
    ($options:expr, $allowed_mask:expr) => {{
        ($options & !$allowed_mask) == 0
    }};
}

macro_rules! smros_zircon_debuglog_io_options_valid_body {
    ($options:expr, $allowed_mask:expr) => {{
        ($options & !$allowed_mask) == 0
    }};
}

macro_rules! smros_zircon_system_event_kind_valid_body {
    ($kind:expr, $max_kind:expr) => {{
        $kind <= $max_kind
    }};
}

macro_rules! smros_zircon_exception_channel_options_valid_body {
    ($options:expr, $allowed_mask:expr) => {{
        ($options & !$allowed_mask) == 0
    }};
}

macro_rules! smros_zircon_hypervisor_options_valid_body {
    ($options:expr, $allowed_mask:expr) => {{
        ($options & !$allowed_mask) == 0
    }};
}

macro_rules! smros_zircon_guest_trap_kind_valid_body {
    ($kind:expr, $max_kind:expr) => {{
        $kind <= $max_kind
    }};
}

macro_rules! smros_zircon_guest_trap_is_bell_body {
    ($kind:expr, $bell:expr) => {{
        $kind == $bell
    }};
}

macro_rules! smros_zircon_guest_trap_is_mem_body {
    ($kind:expr, $mem:expr) => {{
        $kind == $mem
    }};
}

macro_rules! smros_zircon_guest_trap_range_valid_body {
    ($addr:expr, $size:expr, $limit:expr) => {{
        $size != 0 && $addr <= $limit && $size <= $limit - $addr
    }};
}

macro_rules! smros_zircon_guest_trap_alignment_valid_body {
    ($kind:expr, $addr:expr, $size:expr, $bell:expr, $mem:expr, $page_size:expr) => {{
        if $kind == $bell || $kind == $mem {
            $page_size != 0 && $addr % $page_size == 0 && $size % $page_size == 0
        } else {
            true
        }
    }};
}

macro_rules! smros_zircon_vcpu_entry_valid_body {
    ($entry:expr, $alignment:expr) => {{
        $alignment != 0 && $entry % $alignment == 0
    }};
}

macro_rules! smros_zircon_vcpu_interrupt_vector_valid_body {
    ($vector:expr, $max_vector:expr) => {{
        $vector <= $max_vector
    }};
}

macro_rules! smros_zircon_vcpu_read_state_args_valid_body {
    ($kind:expr, $buffer_size:expr, $state_kind:expr, $state_size:expr) => {{
        $kind == $state_kind && $buffer_size == $state_size
    }};
}

macro_rules! smros_zircon_vcpu_write_state_args_valid_body {
    ($kind:expr, $buffer_size:expr, $state_kind:expr, $state_size:expr, $io_kind:expr, $io_size:expr) => {{
        ($kind == $state_kind && $buffer_size == $state_size)
            || ($kind == $io_kind && $buffer_size == $io_size)
    }};
}

macro_rules! smros_linux_syscall_interface_known_body {
    ($syscall_num:expr) => {{
        $syscall_num <= 446 || $syscall_num == 600
    }};
}

macro_rules! smros_zircon_syscall_interface_known_body {
    ($syscall_num:expr) => {{
        $syscall_num <= 154
            || (183 <= $syscall_num && $syscall_num <= 211)
    }};
}
