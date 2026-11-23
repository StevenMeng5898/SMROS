/// Ready-queue bit updates, tick charging, and retirement decisions.
///
/// The live `Scheduler` still owns the bitmap, the priority counts, and the
/// atomic retirement slots. These macros are the decisions those paths apply.

#[allow(unused_macros)]
macro_rules! smros_sched_ready_word_index_body {
    ($index:expr) => {{
        $index / 64
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_ready_bit_mask_body {
    ($index:expr) => {{
        1u64 << ($index % 64)
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_ready_bit_is_set_body {
    ($word:expr, $mask:expr) => {{
        $word & $mask != 0
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_ready_word_after_body {
    ($word:expr, $mask:expr, $ready:expr) => {{
        if $ready {
            $word | $mask
        } else {
            $word & !$mask
        }
    }};
}

/// Ready-count step. A no-op when the bit does not change. Otherwise the
/// count moves by one and saturates at the ends of `u16`.
#[allow(unused_macros)]
macro_rules! smros_sched_ready_count_after_body {
    ($count:expr, $was_ready:expr, $ready:expr) => {{
        if $was_ready == $ready {
            $count
        } else if $ready {
            $count.saturating_add(1)
        } else {
            $count.saturating_sub(1)
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_priority_count_ready_body {
    ($count:expr) => {{
        $count > 0
    }};
}

/// A candidate may be selected only when it is ready, allowed on the CPU,
/// not the thread already running, not the idle slot, and at the best priority.
#[allow(unused_macros)]
macro_rules! smros_sched_best_priority_candidate_body {
    (
        $ready:expr,
        $priority:expr,
        $best_priority:expr,
        $allowed:expr,
        $is_current:expr,
        $idx_nonzero:expr
    ) => {{
        $idx_nonzero && !$is_current && $ready && $allowed && $priority == $best_priority
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_retire_record_accepted_body {
    ($cpu_id:expr, $cpu_count:expr, $thread:expr, $reclaimable_bit:expr) => {{
        $cpu_id < $cpu_count && $thread != 0 && ($thread & $reclaimable_bit) == 0
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_retire_confirm_accepted_body {
    ($pending:expr, $empty:expr, $reclaimable_bit:expr, $current:expr) => {{
        $pending != $empty && ($pending & $reclaimable_bit) == 0 && $pending != $current
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_retire_take_thread_body {
    ($reclaimable:expr, $empty:expr, $reclaimable_bit:expr) => {{
        if $reclaimable == $empty || ($reclaimable & $reclaimable_bit) == 0 {
            None
        } else {
            Some($reclaimable & !$reclaimable_bit)
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_retired_slot_reuse_body {
    (
        $is_terminated:expr,
        $slot:expr,
        $current:expr,
        $retired_is_slot:expr,
        $has_stack_pointer:expr,
        $has_stack_size:expr,
        $unavailable:expr,
        $reset_only:expr,
        $deallocate:expr
    ) => {{
        if !$is_terminated
            || $slot == 0
            || $slot == $current
            || !$retired_is_slot
            || $has_stack_pointer != $has_stack_size
        {
            $unavailable
        } else if $has_stack_pointer {
            $deallocate
        } else {
            $reset_only
        }
    }};
}
