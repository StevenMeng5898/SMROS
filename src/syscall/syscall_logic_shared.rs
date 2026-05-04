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
