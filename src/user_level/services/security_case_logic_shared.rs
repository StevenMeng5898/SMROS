// Pure sectest predicates. The EL0 probe, fault observation, and shell output
// stay in `security_case.rs`.

/// `0` pass, `1` magic changed, `2` kernel executed the probe, `3` no fault.
macro_rules! smros_sectest_trap_verdict_body {
    ($faulted:expr, $magic_ok:expr, $exec_ok:expr) => {{
        if $faulted && $magic_ok && $exec_ok {
            0u8
        } else if !$magic_ok {
            1u8
        } else if !$exec_ok {
            2u8
        } else {
            3u8
        }
    }};
}

macro_rules! smros_sectest_case_timed_out_body {
    ($start:expr, $now:expr, $budget:expr) => {{
        if $now >= $start {
            $now - $start > $budget
        } else {
            false
        }
    }};
}

/// One retry is allowed only for a single late sample that did not pass 300ms.
/// `attempt` and `late_count` already include the attempt being judged.
/// A second late, a sample above 300ms, or a non-late failure does not retry.
macro_rules! smros_notify_gate_allow_retry_body {
    ($attempt:expr, $late_count:expr, $max_ns:expr, $late:expr) => {{
        $late && $attempt < 2 && $late_count < 2 && $max_ns <= 300_000_000
    }};
}

/// Half-open window `[start, end)`. A zero length names `addr` itself and is
/// inside only when `start <= addr < end` is not required; zero length is
/// inside when `addr` is in `[start, end]`.
macro_rules! smros_sectest_address_in_window_body {
    ($addr:expr, $len:expr, $start:expr, $end:expr) => {{
        if $end < $start {
            false
        } else if $len == 0 {
            $addr >= $start && $addr <= $end
        } else {
            match $addr.checked_add($len) {
                Some(last) => $addr >= $start && last <= $end,
                None => false,
            }
        }
    }};
}
