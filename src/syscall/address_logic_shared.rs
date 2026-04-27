macro_rules! smros_checked_end_body {
    ($addr:expr, $len:expr) => {{
        if $addr <= usize::MAX - $len {
            Some($addr + $len)
        } else {
            None
        }
    }};
}

macro_rules! smros_range_overlaps_body {
    ($start_a:expr, $len_a:expr, $start_b:expr, $len_b:expr) => {{
        match (
            smros_checked_end_body!($start_a, $len_a),
            smros_checked_end_body!($start_b, $len_b),
        ) {
            (Some(end_a), Some(end_b)) => $start_a < end_b && $start_b < end_a,
            _ => false,
        }
    }};
}

macro_rules! smros_range_within_window_body {
    ($addr:expr, $len:expr, $base:expr, $limit:expr) => {{
        match smros_checked_end_body!($addr, $len) {
            Some(end) => $addr >= $base && end <= $limit,
            None => false,
        }
    }};
}

macro_rules! smros_page_aligned_body {
    ($addr:expr, $page_size:expr) => {{
        if $page_size == 0 {
            false
        } else {
            $addr % $page_size == 0
        }
    }};
}

macro_rules! smros_fixed_linux_mmap_request_ok_body {
    ($addr:expr, $len:expr, $page_size:expr, $base:expr, $limit:expr) => {{
        smros_page_aligned_body!($addr, $page_size)
            && smros_range_within_window_body!($addr, $len, $base, $limit)
    }};
}
