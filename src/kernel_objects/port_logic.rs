include!("port_logic_shared.rs");

pub(crate) fn options_valid(options: u32, mask: u32) -> bool {
    smros_port_options_valid_body!(options, mask)
}

pub(crate) fn packet_ptr_valid(ptr: usize, size: usize) -> bool {
    smros_port_packet_ptr_valid_body!(ptr, size)
}

pub(crate) fn queue_has_space(len: usize, capacity: usize) -> bool {
    smros_port_queue_has_space_body!(len, capacity)
}

pub(crate) fn signal_packet_allowed(observed: u32, signals: u32) -> bool {
    smros_port_signal_packet_allowed_body!(observed, signals)
}

pub(crate) fn wait_async_options_valid(
    options: u32,
    mask: u32,
    timestamp: u32,
    boot_timestamp: u32,
) -> bool {
    smros_port_wait_async_options_valid_body!(options, mask, timestamp, boot_timestamp)
}

pub(crate) fn observer_should_queue(
    previous: u32,
    observed: u32,
    signals: u32,
    edge: bool,
) -> bool {
    smros_port_observer_should_queue_body!(previous, observed, signals, edge)
}
