include!("object_logic_shared.rs");

pub(crate) fn pages(size: usize, page_size: usize) -> usize {
    smros_ko_pages_body!(size, page_size)
}

pub(crate) fn roundup_pages(size: usize, page_size: usize) -> usize {
    smros_ko_roundup_pages_body!(size, page_size)
}

pub(crate) fn checked_end(addr: usize, len: usize) -> Option<usize> {
    smros_ko_checked_end_body!(addr, len)
}

pub(crate) fn page_aligned(addr: usize, page_size: usize) -> bool {
    smros_ko_page_aligned_body!(addr, page_size)
}

pub(crate) fn range_within(addr: usize, len: usize, base: usize, size: usize) -> bool {
    smros_ko_range_within_body!(addr, len, base, size)
}

pub(crate) fn ranges_overlap(start_a: usize, len_a: usize, start_b: usize, len_b: usize) -> bool {
    smros_ko_ranges_overlap_body!(start_a, len_a, start_b, len_b)
}

pub(crate) fn align_up_checked(addr: usize, align: usize) -> Option<usize> {
    smros_ko_align_up_checked_body!(addr, align)
}

pub(crate) fn intersect_rights(requested: u32, existing: u32) -> u32 {
    smros_ko_intersect_rights_body!(requested, existing)
}

pub(crate) fn rights_subset(requested: u32, existing: u32) -> bool {
    smros_ko_rights_subset_body!(requested, existing)
}

pub(crate) fn rights_has(rights: u32, required: u32) -> bool {
    smros_ko_rights_has_body!(rights, required)
}

pub(crate) fn rights_valid(rights: u32, known_mask: u32) -> bool {
    smros_ko_rights_valid_body!(rights, known_mask)
}

pub(crate) fn duplicate_rights_allowed(
    existing: u32,
    requested: u32,
    duplicate_right: u32,
    same_rights: u32,
    known_mask: u32,
) -> bool {
    smros_ko_duplicate_rights_allowed_body!(
        existing,
        requested,
        duplicate_right,
        same_rights,
        known_mask
    )
}

pub(crate) fn replace_rights_allowed(
    existing: u32,
    requested: u32,
    same_rights: u32,
    known_mask: u32,
) -> bool {
    smros_ko_replace_rights_allowed_body!(existing, requested, same_rights, known_mask)
}

pub(crate) fn handle_is_valid(handle: u32, invalid: u32) -> bool {
    smros_ko_handle_is_valid_body!(handle, invalid)
}

pub(crate) fn signal_update(current: u32, clear_mask: u32, set_mask: u32) -> u32 {
    smros_ko_signal_update_body!(current, clear_mask, set_mask)
}

pub(crate) fn channel_message_fits(
    data_len: usize,
    handles_len: usize,
    max_data_len: usize,
    max_handles_len: usize,
) -> bool {
    smros_ko_channel_message_fits_body!(data_len, handles_len, max_data_len, max_handles_len)
}

pub(crate) fn channel_signal_state(
    queue_not_empty: bool,
    peer_closed: bool,
    readable_signal: u32,
    peer_closed_signal: u32,
) -> u32 {
    smros_ko_channel_signal_state_body!(
        queue_not_empty,
        peer_closed,
        readable_signal,
        peer_closed_signal
    )
}

pub(crate) fn thread_is_runnable<T: Copy + PartialEq>(state: T, ready: T, running: T) -> bool {
    smros_ko_thread_is_runnable_body!(state, ready, running)
}

pub(crate) fn thread_is_idle(id: usize) -> bool {
    smros_ko_thread_is_idle_body!(id)
}

pub(crate) fn scheduler_should_preempt(time_slice: u32, active_threads: usize) -> bool {
    smros_ko_scheduler_should_preempt_body!(time_slice, active_threads)
}

pub(crate) fn scheduler_candidate_index(
    start: usize,
    attempts: usize,
    max_threads: usize,
) -> usize {
    smros_ko_scheduler_candidate_index_body!(start, attempts, max_threads)
}

pub(crate) fn scheduler_can_run(idx: usize, current: usize, ready: bool) -> bool {
    smros_ko_scheduler_can_run_body!(idx, current, ready)
}

pub(crate) fn scheduler_cpu_allowed(has_affinity: bool, affinity: usize, cpu_id: usize) -> bool {
    smros_ko_scheduler_cpu_allowed_body!(has_affinity, affinity, cpu_id)
}
