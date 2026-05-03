include!("fifo_logic_shared.rs");

pub(crate) fn options_valid(options: u32, mask: u32) -> bool {
    smros_fifo_options_valid_body!(options, mask)
}

pub(crate) fn transfer_bytes(elem_size: usize, count: usize) -> Option<usize> {
    smros_fifo_transfer_bytes_body!(elem_size, count)
}

pub(crate) fn ring_index(read_pos: usize, offset: usize, capacity: usize) -> usize {
    smros_fifo_ring_index_body!(read_pos, offset, capacity)
}

pub(crate) fn remaining_capacity(len: usize, capacity: usize) -> usize {
    smros_fifo_remaining_capacity_body!(len, capacity)
}

pub(crate) fn min_count(left: usize, right: usize) -> usize {
    smros_fifo_min_count_body!(left, right)
}

pub(crate) fn capacity_valid(
    elem_count: usize,
    elem_size: usize,
    max_count: usize,
    max_size: usize,
    max_bytes: usize,
) -> bool {
    smros_fifo_capacity_valid_body!(elem_count, elem_size, max_count, max_size, max_bytes)
}

pub(crate) fn refresh_read_signals(signals: u32, len: usize, readable_signal: u32) -> u32 {
    smros_fifo_refresh_read_signals_body!(signals, len, readable_signal)
}

pub(crate) fn refresh_write_signals(signals: u32, remaining: usize, writable_signal: u32) -> u32 {
    smros_fifo_refresh_write_signals_body!(signals, remaining, writable_signal)
}
