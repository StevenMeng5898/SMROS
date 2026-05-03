macro_rules! smros_port_options_valid_body {
    ($options:expr, $mask:expr) => {{
        ($options & !$mask) == 0
    }};
}

macro_rules! smros_port_packet_ptr_valid_body {
    ($ptr:expr, $size:expr) => {{
        $ptr != 0 && $size != 0
    }};
}

macro_rules! smros_port_queue_has_space_body {
    ($len:expr, $capacity:expr) => {{
        $len < $capacity
    }};
}

macro_rules! smros_port_signal_packet_allowed_body {
    ($observed:expr, $signals:expr) => {{
        $signals == 0 || ($observed & $signals) != 0
    }};
}

macro_rules! smros_port_wait_async_options_valid_body {
    ($options:expr, $mask:expr, $timestamp:expr, $boot_timestamp:expr) => {{
        ($options & !$mask) == 0
            && !(($options & $timestamp) != 0 && ($options & $boot_timestamp) != 0)
    }};
}

macro_rules! smros_port_observer_should_queue_body {
    ($previous:expr, $observed:expr, $signals:expr, $edge:expr) => {{
        let previously_allowed =
            smros_port_signal_packet_allowed_body!($previous, $signals);
        let currently_allowed =
            smros_port_signal_packet_allowed_body!($observed, $signals);
        if $edge {
            !previously_allowed && currently_allowed
        } else {
            currently_allowed
        }
    }};
}
