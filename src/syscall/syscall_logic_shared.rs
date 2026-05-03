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
