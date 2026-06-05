#[allow(unused_macros)]
macro_rules! smros_hypervisor_name_len_valid_body {
    ($len:expr, $max_len:expr) => {{
        $len != 0 && $len <= $max_len
    }};
}

#[allow(unused_macros)]
macro_rules! smros_hypervisor_name_byte_valid_body {
    ($byte:expr) => {{
        (97u8 <= $byte && $byte <= 122u8)
            || (65u8 <= $byte && $byte <= 90u8)
            || (48u8 <= $byte && $byte <= 57u8)
            || $byte == 95u8
            || $byte == 45u8
            || $byte == 46u8
    }};
}

#[allow(unused_macros)]
macro_rules! smros_hypervisor_uptime_ticks_body {
    ($state:expr, $running_state:expr, $start_tick:expr, $last_event_tick:expr, $now_tick:expr) => {{
        if $state == $running_state {
            $now_tick.saturating_sub($start_tick)
        } else {
            $last_event_tick.saturating_sub($start_tick)
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_hypervisor_cpu_usage_percent_body {
    ($uptime:expr, $cpu_time_slice_us:expr, $realtime_priority:expr) => {{
        if $uptime == 0 {
            0
        } else {
            let quota = if $cpu_time_slice_us < 10_000 {
                $cpu_time_slice_us
            } else {
                10_000
            };
            let priority_boost = if $realtime_priority < 99 {
                $realtime_priority
            } else {
                99
            };
            let factor = match 100u64.checked_add(priority_boost as u64) {
                Some(value) => value,
                None => u64::MAX,
            };
            let scaled = match (quota as u64).checked_mul(factor) {
                Some(value) => value / 10_000u64,
                None => u64::MAX,
            };
            if scaled < 100 {
                scaled as u32
            } else {
                100
            }
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_hypervisor_state_count_delta_body {
    ($state:expr, $running_state:expr, $stopped_state:expr, $crashed_state:expr) => {{
        if $state == $running_state {
            (1usize, 0usize, 0usize)
        } else if $state == $stopped_state {
            (0usize, 1usize, 0usize)
        } else if $state == $crashed_state {
            (0usize, 0usize, 1usize)
        } else {
            (0usize, 0usize, 0usize)
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_hypervisor_crash_transition_body {
    (
        $restart_on_crash:expr,
        $restart_count:expr,
        $restart_limit:expr,
        $start_tick:expr,
        $tick:expr,
        $running_state:expr,
        $crashed_state:expr
    ) => {{
        let mut state = $crashed_state;
        let mut restart_count = $restart_count;
        let mut start_tick = $start_tick;
        let mut restarted = false;
        if $restart_on_crash && restart_count < $restart_limit {
            restart_count = restart_count.saturating_add(1);
            state = $running_state;
            start_tick = $tick;
            restarted = true;
        }
        (state, restart_count, start_tick, restarted)
    }};
}

#[allow(unused_macros)]
macro_rules! smros_hypervisor_kill_transition_body {
    ($tick:expr, $stopped_state:expr) => {{
        ($stopped_state, $tick, true)
    }};
}

#[allow(unused_macros)]
macro_rules! smros_hypervisor_saturating_inc_u32_body {
    ($value:expr) => {{
        $value.saturating_add(1)
    }};
}
