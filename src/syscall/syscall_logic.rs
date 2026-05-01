include!("syscall_logic_shared.rs");

pub(crate) const ZIRCON_SYSCALL_BASE: u64 = 1000;

pub(crate) fn is_zircon_syscall_number(syscall_num: u64) -> bool {
    smros_is_zircon_syscall_number_body!(syscall_num, ZIRCON_SYSCALL_BASE)
}

pub(crate) fn zircon_syscall_from_raw(syscall_num: u64) -> u32 {
    smros_zircon_syscall_from_raw_body!(syscall_num, ZIRCON_SYSCALL_BASE)
}

pub(crate) fn handle_invalid(handle: u32, invalid_handle: u32) -> bool {
    smros_syscall_handle_invalid_body!(handle, invalid_handle)
}

pub(crate) fn user_buffer_valid(ptr: usize, len: usize) -> bool {
    smros_syscall_user_buffer_valid_body!(ptr, len)
}

pub(crate) fn channel_buffers_valid(
    bytes_ptr: usize,
    bytes_len: usize,
    handles_ptr: usize,
    handles_len: usize,
) -> bool {
    smros_syscall_channel_buffers_valid_body!(bytes_ptr, bytes_len, handles_ptr, handles_len)
}

pub(crate) fn signal_update(current: u32, clear_mask: u32, set_mask: u32) -> u32 {
    smros_syscall_signal_update_body!(current, clear_mask, set_mask)
}

pub(crate) fn wait_satisfied(observed: u32, requested: u32) -> bool {
    smros_syscall_wait_satisfied_body!(observed, requested)
}

pub(crate) fn linux_clock_id_supported(clock_id: usize) -> bool {
    smros_linux_clock_id_supported_body!(clock_id)
}
