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

pub(crate) fn signal_mask_allowed(clear_mask: u32, set_mask: u32, allowed_mask: u32) -> bool {
    smros_syscall_signal_mask_allowed_body!(clear_mask, set_mask, allowed_mask)
}

pub(crate) fn user_signal_mask() -> u32 {
    smros_syscall_user_signal_mask_body!()
}

pub(crate) fn event_signal_mask() -> u32 {
    smros_syscall_event_signal_mask_body!()
}

pub(crate) fn eventpair_signal_mask() -> u32 {
    smros_syscall_eventpair_signal_mask_body!()
}

pub(crate) fn wait_satisfied(observed: u32, requested: u32) -> bool {
    smros_syscall_wait_satisfied_body!(observed, requested)
}

pub(crate) fn linux_clock_id_supported(clock_id: usize) -> bool {
    smros_linux_clock_id_supported_body!(clock_id)
}

pub(crate) fn linux_signal_valid(signum: usize, max_signal: usize) -> bool {
    smros_linux_signal_valid_body!(signum, max_signal)
}

pub(crate) fn linux_signal_action_valid(signum: usize, max_signal: usize) -> bool {
    smros_linux_signal_action_valid_body!(signum, max_signal)
}

pub(crate) fn linux_sigset_size_valid(size: usize, expected: usize) -> bool {
    smros_linux_sigset_size_valid_body!(size, expected)
}

pub(crate) fn linux_ipc_count_valid(count: usize, max_count: usize) -> bool {
    smros_linux_ipc_count_valid_body!(count, max_count)
}

pub(crate) fn linux_ipc_size_valid(size: usize, max_size: usize) -> bool {
    smros_linux_ipc_size_valid_body!(size, max_size)
}

pub(crate) fn linux_msg_size_valid(size: usize, max_size: usize) -> bool {
    smros_linux_msg_size_valid_body!(size, max_size)
}

pub(crate) fn linux_socket_domain_supported(
    domain: usize,
    unix: usize,
    local: usize,
    inet: usize,
    netlink: usize,
    packet: usize,
) -> bool {
    smros_linux_socket_domain_supported_body!(domain, unix, local, inet, netlink, packet)
}

pub(crate) fn linux_socket_type_supported(
    socket_type: usize,
    mask: usize,
    stream: usize,
    dgram: usize,
    raw: usize,
) -> bool {
    smros_linux_socket_type_supported_body!(socket_type, mask, stream, dgram, raw)
}

pub(crate) fn linux_socket_domain_type_supported(
    domain: usize,
    kind: usize,
    unix: usize,
    local: usize,
    inet: usize,
    netlink: usize,
    packet: usize,
    stream: usize,
    dgram: usize,
    raw: usize,
) -> bool {
    smros_linux_socket_domain_type_supported_body!(
        domain, kind, unix, local, inet, netlink, packet, stream, dgram, raw
    )
}

pub(crate) fn linux_socket_addr_valid(ptr: usize, len: usize) -> bool {
    smros_linux_socket_addr_valid_body!(ptr, len)
}

pub(crate) fn linux_fd_range_valid(first: usize, last: usize) -> bool {
    smros_linux_fd_range_valid_body!(first, last)
}

pub(crate) fn linux_memfd_flags_valid(flags: usize, allowed_mask: usize) -> bool {
    smros_linux_memfd_flags_valid_body!(flags, allowed_mask)
}

pub(crate) fn linux_getrandom_flags_valid(flags: u32, allowed_mask: u32) -> bool {
    smros_linux_getrandom_flags_valid_body!(flags, allowed_mask)
}

pub(crate) fn zircon_clock_id_supported(clock_id: u32) -> bool {
    smros_zircon_clock_id_supported_body!(clock_id)
}

pub(crate) fn zircon_clock_create_options_valid(options: u32, allowed_mask: u32) -> bool {
    smros_zircon_clock_create_options_valid_body!(options, allowed_mask)
}

pub(crate) fn zircon_clock_update_options_valid(options: u64, allowed_mask: u64) -> bool {
    smros_zircon_clock_update_options_valid_body!(options, allowed_mask)
}

pub(crate) fn zircon_timer_options_valid(options: u32, allowed_mask: u32) -> bool {
    smros_zircon_timer_options_valid_body!(options, allowed_mask)
}

pub(crate) fn zircon_timer_deadline_expired(deadline: u64, now: u64) -> bool {
    smros_zircon_timer_deadline_expired_body!(deadline, now)
}

pub(crate) fn zircon_debuglog_create_options_valid(options: u32, allowed_mask: u32) -> bool {
    smros_zircon_debuglog_create_options_valid_body!(options, allowed_mask)
}

pub(crate) fn zircon_debuglog_io_options_valid(options: u32, allowed_mask: u32) -> bool {
    smros_zircon_debuglog_io_options_valid_body!(options, allowed_mask)
}

pub(crate) fn zircon_system_event_kind_valid(kind: u32, max_kind: u32) -> bool {
    smros_zircon_system_event_kind_valid_body!(kind, max_kind)
}

pub(crate) fn zircon_exception_channel_options_valid(options: u32, allowed_mask: u32) -> bool {
    smros_zircon_exception_channel_options_valid_body!(options, allowed_mask)
}

pub(crate) fn zircon_hypervisor_options_valid(options: u32, allowed_mask: u32) -> bool {
    smros_zircon_hypervisor_options_valid_body!(options, allowed_mask)
}

pub(crate) fn zircon_guest_trap_kind_valid(kind: u32, max_kind: u32) -> bool {
    smros_zircon_guest_trap_kind_valid_body!(kind, max_kind)
}

pub(crate) fn zircon_guest_trap_is_bell(kind: u32, bell: u32) -> bool {
    smros_zircon_guest_trap_is_bell_body!(kind, bell)
}

pub(crate) fn zircon_guest_trap_is_mem(kind: u32, mem: u32) -> bool {
    smros_zircon_guest_trap_is_mem_body!(kind, mem)
}

pub(crate) fn zircon_guest_trap_range_valid(addr: u64, size: u64, limit: u64) -> bool {
    smros_zircon_guest_trap_range_valid_body!(addr, size, limit)
}

pub(crate) fn zircon_guest_trap_alignment_valid(
    kind: u32,
    addr: u64,
    size: u64,
    bell: u32,
    mem: u32,
    page_size: u64,
) -> bool {
    smros_zircon_guest_trap_alignment_valid_body!(kind, addr, size, bell, mem, page_size)
}

pub(crate) fn zircon_vcpu_entry_valid(entry: u64, alignment: u64) -> bool {
    smros_zircon_vcpu_entry_valid_body!(entry, alignment)
}

pub(crate) fn zircon_vcpu_interrupt_vector_valid(vector: u32, max_vector: u32) -> bool {
    smros_zircon_vcpu_interrupt_vector_valid_body!(vector, max_vector)
}

pub(crate) fn zircon_vcpu_read_state_args_valid(
    kind: u32,
    buffer_size: usize,
    state_kind: u32,
    state_size: usize,
) -> bool {
    smros_zircon_vcpu_read_state_args_valid_body!(kind, buffer_size, state_kind, state_size)
}

pub(crate) fn zircon_vcpu_write_state_args_valid(
    kind: u32,
    buffer_size: usize,
    state_kind: u32,
    state_size: usize,
    io_kind: u32,
    io_size: usize,
) -> bool {
    smros_zircon_vcpu_write_state_args_valid_body!(
        kind,
        buffer_size,
        state_kind,
        state_size,
        io_kind,
        io_size
    )
}

pub(crate) fn linux_syscall_interface_known(syscall_num: u32) -> bool {
    smros_linux_syscall_interface_known_body!(syscall_num)
}

pub(crate) fn zircon_syscall_interface_known(syscall_num: u32) -> bool {
    smros_zircon_syscall_interface_known_body!(syscall_num)
}
