#[allow(unused_macros)]
macro_rules! smros_sched_policy_from_match_flags_body {
    (
        $rr_match:expr,
        $round_robin_match:expr,
        $edf_match:expr,
        $credit_match:expr,
        $fair_match:expr,
        $rr_policy:expr,
        $edf_policy:expr,
        $credit_policy:expr,
        $fair_policy:expr
    ) => {{
        if $rr_match || $round_robin_match {
            Some($rr_policy)
        } else if $edf_match {
            Some($edf_policy)
        } else if $credit_match {
            Some($credit_policy)
        } else if $fair_match {
            Some($fair_policy)
        } else {
            None
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_task_allowed_on_cpu_body {
    ($has_affinity:expr, $affinity:expr, $has_cpu_filter:expr, $cpu_id:expr) => {{
        !$has_cpu_filter || !$has_affinity || $affinity == $cpu_id
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_edf_better_body {
    ($candidate_deadline:expr, $best_present:expr, $best_deadline:expr) => {{
        !$best_present || $candidate_deadline < $best_deadline
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_credit_better_body {
    ($candidate_credit:expr, $best_present:expr, $best_credit:expr) => {{
        !$best_present || $candidate_credit > $best_credit
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_fair_better_body {
    (
        $candidate_ticks:expr,
        $candidate_weight:expr,
        $best_present:expr,
        $best_ticks:expr,
        $best_weight:expr
    ) => {{
        let candidate_weight = if $candidate_weight == 0 {
            1u128
        } else {
            $candidate_weight as u128
        };
        let best_weight = if $best_weight == 0 {
            1u128
        } else {
            $best_weight as u128
        };
        let candidate_score = match ($candidate_ticks as u128).checked_mul(best_weight) {
            Some(score) => score,
            None => u128::MAX,
        };
        let best_score = match ($best_ticks as u128).checked_mul(candidate_weight) {
            Some(score) => score,
            None => u128::MAX,
        };
        !$best_present || candidate_score < best_score
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_time_slice_after_tick_body {
    ($time_slice:expr) => {{
        if $time_slice > 0 {
            $time_slice - 1
        } else {
            0
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_credit_after_tick_body {
    ($credit:expr) => {{
        if $credit > 0 {
            $credit - 1
        } else {
            $credit
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_deadline_due_body {
    ($time_slice:expr, $tick_count:expr, $deadline_tick:expr) => {{
        $time_slice == 0 || $tick_count >= $deadline_tick
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_advance_deadline_body {
    ($deadline_tick:expr, $tick_count:expr, $period_ticks:expr) => {{
        let period = if $period_ticks == 0 { 1 } else { $period_ticks };
        let base = if $deadline_tick > $tick_count {
            $deadline_tick
        } else {
            $tick_count
        };
        base.saturating_add(period as u64)
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_refill_credit_body {
    ($credit_cap:expr, $weight:expr, $default_credit:expr, $max_credit_weight:expr) => {{
        let refill = if $weight > $max_credit_weight {
            i32::MAX
        } else {
            let default_credit = if $default_credit > 0 {
                $default_credit as u128
            } else {
                1u128
            };
            let refill = ($weight as u128) * default_credit;
            if refill > i32::MAX as u128 {
                i32::MAX
            } else {
                refill as i32
            }
        };
        if $credit_cap >= refill && $credit_cap >= 1 {
            $credit_cap
        } else if refill >= 1 {
            refill
        } else {
            1
        }
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_should_preempt_body {
    (
        $policy:expr,
        $rr_policy:expr,
        $edf_policy:expr,
        $credit_policy:expr,
        $fair_policy:expr,
        $time_slice:expr,
        $active_threads:expr,
        $deadline_tick:expr,
        $tick_count:expr,
        $credit:expr
    ) => {{
        if $active_threads <= 1 {
            false
        } else if $policy == $rr_policy {
            $time_slice == 0
        } else if $policy == $edf_policy {
            $time_slice == 0 || $deadline_tick <= $tick_count
        } else if $policy == $credit_policy {
            $time_slice == 0 || $credit <= 0
        } else if $policy == $fair_policy {
            $time_slice == 0
        } else {
            false
        }
    }};
}
