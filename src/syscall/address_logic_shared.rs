macro_rules! smros_checked_end_body {
    ($addr:expr, $len:expr) => {{
        if $addr <= usize::MAX - $len {
            Some($addr + $len)
        } else {
            None
        }
    }};
}

#[cfg(not(target_os = "none"))]
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

macro_rules! smros_regular_file_mmap_span_ok_body {
    ($offset:expr, $len:expr, $offset_max:expr) => {{
        let offset: u64 = $offset;
        let len = $len as u64;
        let offset_max: u64 = $offset_max;
        match offset.checked_add(len) {
            Some(end) => end <= offset_max,
            None => false,
        }
    }};
}

#[cfg(not(target_os = "none"))]
macro_rules! smros_linux_user_range_writable_body {
    ($addr:expr, $len:expr, $ranges:expr) => {{
        let address = $addr;
        let length = $len;
        if length == 0 {
            false
        } else {
            match smros_checked_end_body!(address, length) {
                Some(end) => ($ranges)
                    .into_iter()
                    .any(|(range_start, range_len, writable)| {
                        writable
                            && match smros_checked_end_body!(range_start, range_len) {
                                Some(range_end) => address >= range_start && end <= range_end,
                                None => false,
                            }
                    }),
                None => false,
            }
        }
    }};
}

#[cfg(not(target_os = "none"))]
macro_rules! smros_linux_user_range_readable_body {
    ($addr:expr, $len:expr, $ranges:expr) => {{
        let address = $addr;
        let length = $len;
        if length == 0 {
            false
        } else {
            match smros_checked_end_body!(address, length) {
                Some(end) => {
                    ($ranges)
                        .into_iter()
                        .any(|(range_start, range_len, readable, writable)| {
                            (readable || writable)
                                && match smros_checked_end_body!(range_start, range_len) {
                                    Some(range_end) => address >= range_start && end <= range_end,
                                    None => false,
                                }
                        })
                }
                None => false,
            }
        }
    }};
}
