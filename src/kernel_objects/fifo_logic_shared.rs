macro_rules! smros_fifo_options_valid_body {
    ($options:expr, $mask:expr) => {{
        ($options & !$mask) == 0
    }};
}

macro_rules! smros_fifo_transfer_bytes_body {
    ($elem_size:expr, $count:expr) => {{
        if $count == 0 {
            Some(0)
        } else {
            $elem_size.checked_mul($count)
        }
    }};
}

macro_rules! smros_fifo_ring_index_body {
    ($read_pos:expr, $offset:expr, $capacity:expr) => {{
        if $capacity == 0 {
            0
        } else {
            let base = $read_pos % $capacity;
            let offset = $offset % $capacity;
            if base >= $capacity - offset {
                base - ($capacity - offset)
            } else {
                base + offset
            }
        }
    }};
}

macro_rules! smros_fifo_remaining_capacity_body {
    ($len:expr, $capacity:expr) => {{
        if $len >= $capacity {
            0
        } else {
            $capacity - $len
        }
    }};
}

macro_rules! smros_fifo_min_count_body {
    ($left:expr, $right:expr) => {{
        if $left <= $right {
            $left
        } else {
            $right
        }
    }};
}

macro_rules! smros_fifo_capacity_valid_body {
    ($elem_count:expr, $elem_size:expr, $max_count:expr, $max_size:expr, $max_bytes:expr) => {{
        $elem_count != 0
            && $elem_size != 0
            && $elem_count <= $max_count
            && $elem_size <= $max_size
            && match $elem_count.checked_mul($elem_size) {
                Some(bytes) => bytes <= $max_bytes,
                None => false,
            }
    }};
}

macro_rules! smros_fifo_refresh_read_signals_body {
    ($signals:expr, $len:expr, $readable_signal:expr) => {{
        let mut signals = $signals;
        if $len == 0 {
            signals &= !$readable_signal;
        } else {
            signals |= $readable_signal;
        }
        signals
    }};
}

macro_rules! smros_fifo_refresh_write_signals_body {
    ($signals:expr, $remaining:expr, $writable_signal:expr) => {{
        let mut signals = $signals;
        if $remaining == 0 {
            signals &= !$writable_signal;
        } else {
            signals |= $writable_signal;
        }
        signals
    }};
}
