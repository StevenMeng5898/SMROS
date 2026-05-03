macro_rules! smros_socket_options_valid_body {
    ($options:expr, $mask:expr) => {{
        ($options & !$mask) == 0
    }};
}

macro_rules! smros_socket_mask_options_body {
    ($options:expr, $mask:expr) => {{
        $options & $mask
    }};
}

macro_rules! smros_socket_ring_index_body {
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

macro_rules! smros_socket_remaining_capacity_body {
    ($len:expr, $capacity:expr) => {{
        if $len >= $capacity {
            0
        } else {
            $capacity - $len
        }
    }};
}

macro_rules! smros_socket_min_count_body {
    ($left:expr, $right:expr) => {{
        if $left <= $right {
            $left
        } else {
            $right
        }
    }};
}

macro_rules! smros_socket_threshold_met_body {
    ($threshold:expr, $observed:expr) => {{
        $threshold != 0 && $observed >= $threshold
    }};
}

macro_rules! smros_socket_refresh_read_signals_body {
    ($signals:expr, $len:expr, $threshold:expr, $readable_signal:expr, $threshold_signal:expr) => {{
        let mut signals = $signals;
        if $len == 0 {
            signals &= !$readable_signal;
        } else {
            signals |= $readable_signal;
        }

        if smros_socket_threshold_met_body!($threshold, $len) {
            signals |= $threshold_signal;
        } else {
            signals &= !$threshold_signal;
        }
        signals
    }};
}

macro_rules! smros_socket_refresh_write_signals_body {
    (
        $signals:expr,
        $write_disabled:expr,
        $remaining:expr,
        $threshold:expr,
        $writable_signal:expr,
        $threshold_signal:expr
    ) => {{
        let mut signals = $signals;
        if $write_disabled || $remaining == 0 {
            signals &= !$writable_signal;
        } else {
            signals |= $writable_signal;
        }

        if smros_socket_threshold_met_body!($threshold, $remaining) {
            signals |= $threshold_signal;
        } else {
            signals &= !$threshold_signal;
        }
        signals
    }};
}
