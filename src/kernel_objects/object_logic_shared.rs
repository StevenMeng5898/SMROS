macro_rules! smros_ko_pages_body {
    ($size:expr, $page_size:expr) => {{
        if $page_size == 0 {
            0
        } else {
            let whole_pages = $size / $page_size;
            if $size % $page_size == 0 {
                whole_pages
            } else {
                match whole_pages.checked_add(1) {
                    Some(pages) => pages,
                    None => usize::MAX,
                }
            }
        }
    }};
}

macro_rules! smros_ko_roundup_pages_body {
    ($size:expr, $page_size:expr) => {{
        if $page_size == 0 {
            0
        } else {
            let pages = smros_ko_pages_body!($size, $page_size);
            match pages.checked_mul($page_size) {
                Some(rounded) => rounded,
                None => usize::MAX,
            }
        }
    }};
}

macro_rules! smros_ko_checked_end_body {
    ($addr:expr, $len:expr) => {{
        if $addr <= usize::MAX - $len {
            Some($addr + $len)
        } else {
            None
        }
    }};
}

macro_rules! smros_ko_page_aligned_body {
    ($addr:expr, $page_size:expr) => {{
        $page_size != 0 && $addr % $page_size == 0
    }};
}

macro_rules! smros_ko_range_within_body {
    ($addr:expr, $len:expr, $base:expr, $size:expr) => {{
        match (
            smros_ko_checked_end_body!($addr, $len),
            smros_ko_checked_end_body!($base, $size),
        ) {
            (Some(end), Some(limit)) => $addr >= $base && end <= limit,
            _ => false,
        }
    }};
}

macro_rules! smros_ko_ranges_overlap_body {
    ($start_a:expr, $len_a:expr, $start_b:expr, $len_b:expr) => {{
        match (
            smros_ko_checked_end_body!($start_a, $len_a),
            smros_ko_checked_end_body!($start_b, $len_b),
        ) {
            (Some(end_a), Some(end_b)) => $start_a < end_b && $start_b < end_a,
            _ => false,
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_ko_align_up_checked_body {
    ($addr:expr, $align:expr) => {{
        if $align == 0 || !$align.is_power_of_two() {
            None
        } else {
            match $addr.checked_add($align - 1) {
                Some(biased) => Some(biased & !($align - 1)),
                None => None,
            }
        }
    }};
}

macro_rules! smros_ko_intersect_rights_body {
    ($requested:expr, $existing:expr) => {{
        $requested & $existing
    }};
}

macro_rules! smros_ko_handle_is_valid_body {
    ($handle:expr, $invalid:expr) => {{
        $handle != 0 && $handle != $invalid
    }};
}

macro_rules! smros_ko_signal_update_body {
    ($current:expr, $clear_mask:expr, $set_mask:expr) => {{
        ($current & !$clear_mask) | $set_mask
    }};
}

macro_rules! smros_ko_channel_message_fits_body {
    ($data_len:expr, $handles_len:expr, $max_data_len:expr, $max_handles_len:expr) => {{
        $data_len <= $max_data_len && $handles_len <= $max_handles_len
    }};
}

macro_rules! smros_ko_channel_signal_state_body {
    ($queue_not_empty:expr, $peer_closed:expr, $readable_signal:expr, $peer_closed_signal:expr) => {{
        if $queue_not_empty && $peer_closed {
            $readable_signal | $peer_closed_signal
        } else if $queue_not_empty {
            $readable_signal
        } else if $peer_closed {
            $peer_closed_signal
        } else {
            0
        }
    }};
}

macro_rules! smros_ko_thread_is_runnable_body {
    ($state:expr, $ready:expr, $running:expr) => {{
        $state == $ready || $state == $running
    }};
}

macro_rules! smros_ko_thread_is_idle_body {
    ($id:expr) => {{
        $id == 0
    }};
}

macro_rules! smros_ko_scheduler_should_preempt_body {
    ($time_slice:expr, $active_threads:expr) => {{
        $time_slice == 0 && $active_threads > 1
    }};
}

macro_rules! smros_ko_scheduler_candidate_index_body {
    ($start:expr, $attempts:expr, $max_threads:expr) => {{
        if $max_threads == 0 {
            0
        } else {
            let start_mod = $start % $max_threads;
            let attempts_mod = $attempts % $max_threads;
            if start_mod >= $max_threads - attempts_mod {
                start_mod - ($max_threads - attempts_mod)
            } else {
                start_mod + attempts_mod
            }
        }
    }};
}

macro_rules! smros_ko_scheduler_can_run_body {
    ($idx:expr, $current:expr, $ready:expr) => {{
        $idx != $current && $idx != 0 && $ready
    }};
}

macro_rules! smros_ko_scheduler_cpu_allowed_body {
    ($has_affinity:expr, $affinity:expr, $cpu_id:expr) => {{
        !$has_affinity || $affinity == $cpu_id
    }};
}
