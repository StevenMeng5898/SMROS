include!("socket_logic_shared.rs");

pub(crate) fn options_valid(options: u32, mask: u32) -> bool {
    smros_socket_options_valid_body!(options, mask)
}

pub(crate) fn mask_options(options: u32, mask: u32) -> u32 {
    smros_socket_mask_options_body!(options, mask)
}

pub(crate) fn ring_index(read_pos: usize, offset: usize, capacity: usize) -> usize {
    smros_socket_ring_index_body!(read_pos, offset, capacity)
}

pub(crate) fn remaining_capacity(len: usize, capacity: usize) -> usize {
    smros_socket_remaining_capacity_body!(len, capacity)
}

pub(crate) fn min_count(left: usize, right: usize) -> usize {
    smros_socket_min_count_body!(left, right)
}

pub(crate) fn threshold_met(threshold: usize, observed: usize) -> bool {
    smros_socket_threshold_met_body!(threshold, observed)
}

pub(crate) fn refresh_read_signals(
    signals: u32,
    len: usize,
    threshold: usize,
    readable_signal: u32,
    threshold_signal: u32,
) -> u32 {
    smros_socket_refresh_read_signals_body!(
        signals,
        len,
        threshold,
        readable_signal,
        threshold_signal
    )
}

pub(crate) fn refresh_write_signals(
    signals: u32,
    write_disabled: bool,
    remaining: usize,
    threshold: usize,
    writable_signal: u32,
    threshold_signal: u32,
) -> u32 {
    smros_socket_refresh_write_signals_body!(
        signals,
        write_disabled,
        remaining,
        threshold,
        writable_signal,
        threshold_signal
    )
}
