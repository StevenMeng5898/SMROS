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
macro_rules! smros_sched_priority_better_body {
    ($candidate_priority:expr, $best_present:expr, $best_priority:expr) => {{
        !$best_present || $candidate_priority > $best_priority
    }};
}

#[allow(unused_macros)]
macro_rules! smros_sched_priority_should_preempt_body {
    ($current_priority:expr, $best_ready_present:expr, $best_ready_priority:expr) => {{
        $best_ready_present && $best_ready_priority > $current_priority
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SchedulerSlotReuse {
    Unavailable,
    ResetOnly,
    DeallocateAndReuse,
}

pub(crate) fn scheduler_retired_slot_reuse_action(
    is_terminated: bool,
    slot: usize,
    current_thread: usize,
    retired_thread: Option<usize>,
    has_stack_pointer: bool,
    has_stack_size: bool,
) -> SchedulerSlotReuse {
    if !is_terminated
        || slot == 0
        || slot == current_thread
        || retired_thread != Some(slot)
        || has_stack_pointer != has_stack_size
    {
        SchedulerSlotReuse::Unavailable
    } else if has_stack_pointer {
        SchedulerSlotReuse::DeallocateAndReuse
    } else {
        SchedulerSlotReuse::ResetOnly
    }
}

pub(crate) struct DeferredThreadRetirements<const CPU_COUNT: usize> {
    retired_by_cpu: [core::sync::atomic::AtomicUsize; CPU_COUNT],
}

impl<const CPU_COUNT: usize> DeferredThreadRetirements<CPU_COUNT> {
    const EMPTY: usize = usize::MAX;
    const RECLAIMABLE_BIT: usize = 1usize << (usize::BITS - 1);

    pub(crate) const fn new() -> Self {
        Self {
            retired_by_cpu: [const { core::sync::atomic::AtomicUsize::new(Self::EMPTY) };
                CPU_COUNT],
        }
    }

    pub(crate) fn record_before_switch(&self, cpu_id: usize, thread: usize) -> bool {
        if cpu_id >= CPU_COUNT || thread == 0 || thread & Self::RECLAIMABLE_BIT != 0 {
            return false;
        }
        self.retired_by_cpu[cpu_id]
            .compare_exchange(
                Self::EMPTY,
                thread,
                core::sync::atomic::Ordering::Release,
                core::sync::atomic::Ordering::Relaxed,
            )
            .is_ok()
    }

    pub(crate) fn confirm_after_switch(&self, cpu_id: usize, current_thread: usize) -> bool {
        let slot = match self.retired_by_cpu.get(cpu_id) {
            Some(slot) => slot,
            None => return false,
        };
        let pending = slot.load(core::sync::atomic::Ordering::Acquire);
        if pending == Self::EMPTY
            || pending & Self::RECLAIMABLE_BIT != 0
            || pending == current_thread
        {
            return false;
        }
        slot.compare_exchange(
            pending,
            pending | Self::RECLAIMABLE_BIT,
            core::sync::atomic::Ordering::AcqRel,
            core::sync::atomic::Ordering::Acquire,
        )
        .is_ok()
    }

    pub(crate) fn take_reclaimable(&self, cpu_id: usize) -> Option<usize> {
        let slot = self.retired_by_cpu.get(cpu_id)?;
        let reclaimable = slot.load(core::sync::atomic::Ordering::Acquire);
        if reclaimable == Self::EMPTY || reclaimable & Self::RECLAIMABLE_BIT == 0 {
            return None;
        }
        slot.compare_exchange(
            reclaimable,
            Self::EMPTY,
            core::sync::atomic::Ordering::AcqRel,
            core::sync::atomic::Ordering::Acquire,
        )
        .ok()
        .map(|encoded| encoded & !Self::RECLAIMABLE_BIT)
    }
}
