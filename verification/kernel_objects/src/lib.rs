use vstd::prelude::*;

verus! {

include!("../../../src/kernel_objects/object_logic_shared.rs");

pub const PAGE_SIZE: usize = 4096;
pub const MAX_HANDLES_PER_PROCESS: usize = 1024;
pub const INVALID_HANDLE: u32 = 0xffff_ffff;
pub const MAX_CHANNEL_MSG_SIZE: usize = 65536;
pub const MAX_CHANNEL_MSG_HANDLES: usize = 64;
pub const CHANNEL_SIGNAL_READABLE: u32 = 1;
pub const CHANNEL_SIGNAL_PEER_CLOSED: u32 = 4;
pub const MAX_THREADS: usize = 16;
pub const THREAD_EMPTY: u8 = 0;
pub const THREAD_READY: u8 = 1;
pub const THREAD_RUNNING: u8 = 2;
pub const THREAD_BLOCKED: u8 = 3;
pub const THREAD_TERMINATED: u8 = 4;
pub const THREAD_ID_IDLE: usize = 0;

#[derive(Copy, Clone)]
struct HandleEntryModel {
    handle: u32,
    obj_type: u8,
    rights: u32,
    valid: bool,
}

#[derive(Copy, Clone)]
struct VmarMappingModel {
    vaddr: usize,
    size: usize,
    valid: bool,
}

#[derive(Copy, Clone)]
struct ThreadModel {
    state: u8,
    has_affinity: bool,
    affinity: usize,
}

spec fn checked_end_spec(addr: int, len: int) -> Option<int> {
    if 0 <= addr && 0 <= len && addr <= usize::MAX as int - len {
        Some(addr + len)
    } else {
        Option::<int>::None
    }
}

spec fn pages_spec(size: int, page_size: int) -> int {
    if page_size <= 0 {
        0
    } else {
        let whole_pages = size / page_size;
        if size % page_size == 0 {
            whole_pages
        } else if whole_pages < usize::MAX as int {
            whole_pages + 1
        } else {
            usize::MAX as int
        }
    }
}

spec fn roundup_pages_spec(size: int, page_size: int) -> int {
    if page_size <= 0 {
        0
    } else {
        let pages = pages_spec(size, page_size);
        if pages <= usize::MAX as int / page_size {
            pages * page_size
        } else {
            usize::MAX as int
        }
    }
}

spec fn page_aligned_spec(addr: int, page_size: int) -> bool {
    page_size > 0 && addr % page_size == 0
}

spec fn range_within_spec(addr: int, len: int, base: int, size: int) -> bool {
    match (checked_end_spec(addr, len), checked_end_spec(base, size)) {
        (Some(end), Some(limit)) => addr >= base && end <= limit,
        _ => false,
    }
}

spec fn ranges_overlap_spec(start_a: int, len_a: int, start_b: int, len_b: int) -> bool {
    match (checked_end_spec(start_a, len_a), checked_end_spec(start_b, len_b)) {
        (Some(end_a), Some(end_b)) => start_a < end_b && start_b < end_a,
        _ => false,
    }
}

spec fn no_overlap_with_vmar_mappings_spec(
    vaddr: int,
    len: int,
    mappings: Seq<VmarMappingModel>,
) -> bool {
    forall|i: int|
        0 <= i < mappings.len() ==> !mappings[i].valid
            || !ranges_overlap_spec(
                vaddr,
                len,
                mappings[i].vaddr as int,
                mappings[i].size as int,
            )
}

spec fn vmar_range_available_spec(
    base: int,
    size: int,
    vaddr: int,
    len: int,
    mappings: Seq<VmarMappingModel>,
) -> bool {
    range_within_spec(vaddr, len, base, size)
        && no_overlap_with_vmar_mappings_spec(vaddr, len, mappings)
}

spec fn channel_message_fits_spec(
    data_len: int,
    handles_len: int,
    max_data_len: int,
    max_handles_len: int,
) -> bool {
    data_len <= max_data_len && handles_len <= max_handles_len
}

spec fn channel_signal_state_spec(queue_not_empty: bool, peer_closed: bool) -> int {
    if queue_not_empty && peer_closed {
        (CHANNEL_SIGNAL_READABLE | CHANNEL_SIGNAL_PEER_CLOSED) as int
    } else if queue_not_empty {
        CHANNEL_SIGNAL_READABLE as int
    } else if peer_closed {
        CHANNEL_SIGNAL_PEER_CLOSED as int
    } else {
        0
    }
}

spec fn thread_is_runnable_spec(state: int) -> bool {
    state == THREAD_READY as int || state == THREAD_RUNNING as int
}

spec fn scheduler_should_preempt_spec(time_slice: int, active_threads: int) -> bool {
    time_slice == 0 && active_threads > 1
}

spec fn scheduler_can_run_spec(idx: int, current: int, ready: bool) -> bool {
    idx != current && idx != 0 && ready
}

spec fn scheduler_cpu_allowed_spec(has_affinity: bool, affinity: int, cpu_id: int) -> bool {
    !has_affinity || affinity == cpu_id
}

spec fn ready_state(state: u8) -> bool {
    state as int == THREAD_READY as int
}

fn ko_pages(size: usize, page_size: usize) -> (out: usize)
    ensures
        out as int == pages_spec(size as int, page_size as int),
{
    smros_ko_pages_body!(size, page_size)
}

fn ko_roundup_pages(size: usize, page_size: usize) -> (out: usize)
    ensures
        page_size == 0 ==> out == 0,
{
    smros_ko_roundup_pages_body!(size, page_size)
}

fn ko_roundup_pages_bounded(size: usize) -> (out: usize)
    requires
        size <= usize::MAX - (PAGE_SIZE - 1),
    ensures
        out as int == roundup_pages_spec(size as int, PAGE_SIZE as int),
{
    smros_ko_roundup_pages_body!(size, PAGE_SIZE)
}

fn ko_checked_end(addr: usize, len: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(end) => checked_end_spec(addr as int, len as int) == Some(end as int),
            None => checked_end_spec(addr as int, len as int) == Option::<int>::None,
        },
{
    smros_ko_checked_end_body!(addr, len)
}

fn ko_page_aligned(addr: usize, page_size: usize) -> (out: bool)
    ensures
        out == page_aligned_spec(addr as int, page_size as int),
{
    smros_ko_page_aligned_body!(addr, page_size)
}

fn ko_range_within(addr: usize, len: usize, base: usize, size: usize) -> (out: bool)
    ensures
        out == range_within_spec(addr as int, len as int, base as int, size as int),
{
    smros_ko_range_within_body!(addr, len, base, size)
}

fn ko_ranges_overlap(start_a: usize, len_a: usize, start_b: usize, len_b: usize) -> (out: bool)
    ensures
        out == ranges_overlap_spec(
            start_a as int,
            len_a as int,
            start_b as int,
            len_b as int,
        ),
{
    smros_ko_ranges_overlap_body!(start_a, len_a, start_b, len_b)
}

fn ko_intersect_rights(requested: u32, existing: u32) -> (out: u32)
    ensures
        out == requested & existing,
{
    smros_ko_intersect_rights_body!(requested, existing)
}

fn handle_get_rights_model(entries: &Vec<HandleEntryModel>, handle: u32) -> (out: Option<u32>)
    ensures
        match out {
            Some(rights) => exists|i: int|
                0 <= i < entries@.len()
                    && entries@[i].valid
                    && entries@[i].handle == handle
                    && entries@[i].rights == rights,
            None => forall|i: int|
                0 <= i < entries@.len()
                    ==> !(entries@[i].valid && entries@[i].handle == handle),
        },
{
    let mut i = 0usize;
    while i < entries.len()
        invariant
            i <= entries.len(),
            forall|j: int|
                0 <= j < i as int
                    ==> !(entries@[j].valid && entries@[j].handle == handle),
        decreases entries.len() - i,
    {
        let entry = &entries[i];
        if entry.valid && entry.handle == handle {
            return Some(entry.rights);
        }
        i += 1;
    }

    assert forall|j: int|
        0 <= j < entries@.len() implies !(entries@[j].valid && entries@[j].handle == handle) by {
        assert(j < i as int);
    };
    None
}

fn vmo_checked_end_model(offset: usize, len: usize, size: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(end) => checked_end_spec(offset as int, len as int) == Some(end as int)
                && end <= size,
            None => checked_end_spec(offset as int, len as int) == Option::<int>::None
                || match checked_end_spec(offset as int, len as int) {
                    Some(end) => end > size as int,
                    None => true,
                },
        },
{
    match ko_checked_end(offset, len) {
        Some(end) => {
            if end <= size {
                Some(end)
            } else {
                None
            }
        }
        None => None,
    }
}

fn vmo_end_page_model(offset: usize, len: usize, size: usize) -> (out: Option<usize>)
    requires
        pages_spec(size as int, PAGE_SIZE as int) <= usize::MAX as int,
    ensures
        match out {
            Some(end_page) => exists|end: int|
                checked_end_spec(offset as int, len as int) == Some(end)
                    && end <= size as int
                    && end_page as int == pages_spec(end, PAGE_SIZE as int),
            None => checked_end_spec(offset as int, len as int) == Option::<int>::None
                || match checked_end_spec(offset as int, len as int) {
                    Some(end) => end > size as int,
                    None => true,
                },
        },
{
    match vmo_checked_end_model(offset, len, size) {
        Some(end) => Some(ko_pages(end, PAGE_SIZE)),
        None => None,
    }
}

fn no_overlap_with_vmar_mappings(
    vaddr: usize,
    len: usize,
    mappings: &Vec<VmarMappingModel>,
) -> (out: bool)
    ensures
        out == no_overlap_with_vmar_mappings_spec(vaddr as int, len as int, mappings@),
{
    let mut i = 0usize;
    while i < mappings.len()
        invariant
            i <= mappings.len(),
            forall|j: int|
                0 <= j < i as int ==> !mappings@[j].valid
                    || !ranges_overlap_spec(
                        vaddr as int,
                        len as int,
                        mappings@[j].vaddr as int,
                        mappings@[j].size as int,
                    ),
        decreases mappings.len() - i,
    {
        let mapping = &mappings[i];
        if mapping.valid && ko_ranges_overlap(vaddr, len, mapping.vaddr, mapping.size) {
            return false;
        }
        i += 1;
    }

    assert forall|j: int|
        0 <= j < mappings@.len() implies !mappings@[j].valid
            || !ranges_overlap_spec(
                vaddr as int,
                len as int,
                mappings@[j].vaddr as int,
                mappings@[j].size as int,
            ) by {
        assert(j < i as int);
    };
    true
}

fn vmar_range_available_model(
    base: usize,
    size: usize,
    vaddr: usize,
    len: usize,
    mappings: &Vec<VmarMappingModel>,
) -> (out: bool)
    ensures
        out == vmar_range_available_spec(
            base as int,
            size as int,
            vaddr as int,
            len as int,
            mappings@,
        ),
{
    ko_range_within(vaddr, len, base, size) && no_overlap_with_vmar_mappings(vaddr, len, mappings)
}

fn ko_channel_message_fits(
    data_len: usize,
    handles_len: usize,
    max_data_len: usize,
    max_handles_len: usize,
) -> (out: bool)
    ensures
        out == channel_message_fits_spec(
            data_len as int,
            handles_len as int,
            max_data_len as int,
            max_handles_len as int,
        ),
{
    smros_ko_channel_message_fits_body!(data_len, handles_len, max_data_len, max_handles_len)
}

fn ko_channel_signal_state(queue_not_empty: bool, peer_closed: bool) -> (out: u32)
    ensures
        out as int == channel_signal_state_spec(queue_not_empty, peer_closed),
{
    let signals = smros_ko_channel_signal_state_body!(
        queue_not_empty,
        peer_closed,
        CHANNEL_SIGNAL_READABLE,
        CHANNEL_SIGNAL_PEER_CLOSED
    );

    if queue_not_empty && peer_closed {
        assert(signals == (CHANNEL_SIGNAL_READABLE | CHANNEL_SIGNAL_PEER_CLOSED));
    } else if queue_not_empty {
        assert(signals == CHANNEL_SIGNAL_READABLE);
    } else if peer_closed {
        assert(signals == CHANNEL_SIGNAL_PEER_CLOSED);
    } else {
        assert(signals == 0);
    }

    signals
}

fn ko_thread_is_runnable(state: u8) -> (out: bool)
    ensures
        out == thread_is_runnable_spec(state as int),
{
    smros_ko_thread_is_runnable_body!(state, THREAD_READY, THREAD_RUNNING)
}

fn ko_thread_is_idle(id: usize) -> (out: bool)
    ensures
        out == (id == THREAD_ID_IDLE),
{
    smros_ko_thread_is_idle_body!(id)
}

fn ko_scheduler_should_preempt(time_slice: u32, active_threads: usize) -> (out: bool)
    ensures
        out == scheduler_should_preempt_spec(time_slice as int, active_threads as int),
{
    smros_ko_scheduler_should_preempt_body!(time_slice, active_threads)
}

fn ko_scheduler_candidate_index(start: usize, attempts: usize, max_threads: usize) -> (out: usize)
    requires
        max_threads > 0,
    ensures
        out < max_threads,
{
    assert(start % max_threads < max_threads);
    assert(attempts % max_threads < max_threads);
    smros_ko_scheduler_candidate_index_body!(start, attempts, max_threads)
}

fn ko_scheduler_can_run(idx: usize, current: usize, ready: bool) -> (out: bool)
    ensures
        out == scheduler_can_run_spec(idx as int, current as int, ready),
{
    smros_ko_scheduler_can_run_body!(idx, current, ready)
}

fn ko_scheduler_cpu_allowed(has_affinity: bool, affinity: usize, cpu_id: usize) -> (out: bool)
    ensures
        out == scheduler_cpu_allowed_spec(has_affinity, affinity as int, cpu_id as int),
{
    smros_ko_scheduler_cpu_allowed_body!(has_affinity, affinity, cpu_id)
}

fn scheduler_pick_next_model(
    threads: &Vec<ThreadModel>,
    current: usize,
    next_thread: usize,
    active_threads: usize,
) -> (out: usize)
    requires
        threads.len() == MAX_THREADS,
        current < MAX_THREADS,
        next_thread < MAX_THREADS,
    ensures
        out < MAX_THREADS,
        active_threads <= 1 ==> out == THREAD_ID_IDLE,
        active_threads > 1 && out != THREAD_ID_IDLE ==> threads@[out as int].state == THREAD_READY,
{
    if active_threads <= 1 {
        return THREAD_ID_IDLE;
    }
    assert(active_threads > 1);
    assert(!(active_threads <= 1));

    let mut attempts = 0usize;
    while attempts < MAX_THREADS
        invariant
            attempts <= MAX_THREADS,
            threads.len() == MAX_THREADS,
            current < MAX_THREADS,
            next_thread < MAX_THREADS,
            active_threads > 1,
        decreases MAX_THREADS - attempts,
    {
        let idx = ko_scheduler_candidate_index(next_thread, attempts, MAX_THREADS);
        if ko_scheduler_can_run(idx, current, threads[idx].state == THREAD_READY) {
            return idx;
        }
        attempts += 1;
    }

    THREAD_ID_IDLE
}

fn scheduler_pick_next_for_cpu_model(
    threads: &Vec<ThreadModel>,
    current: usize,
    next_thread: usize,
    active_threads: usize,
    cpu_id: usize,
) -> (out: usize)
    requires
        threads.len() == MAX_THREADS,
        current < MAX_THREADS,
        next_thread < MAX_THREADS,
    ensures
        out < MAX_THREADS,
        active_threads <= 1 ==> out == THREAD_ID_IDLE,
        active_threads > 1 && out != THREAD_ID_IDLE ==> threads@[out as int].state == THREAD_READY,
        active_threads > 1 && out != THREAD_ID_IDLE ==> scheduler_cpu_allowed_spec(
            threads@[out as int].has_affinity,
            threads@[out as int].affinity as int,
            cpu_id as int,
        ),
{
    if active_threads <= 1 {
        return THREAD_ID_IDLE;
    }
    assert(active_threads > 1);
    assert(!(active_threads <= 1));

    let mut attempts = 0usize;
    while attempts < MAX_THREADS
        invariant
            attempts <= MAX_THREADS,
            threads.len() == MAX_THREADS,
            current < MAX_THREADS,
            next_thread < MAX_THREADS,
            active_threads > 1,
        decreases MAX_THREADS - attempts,
    {
        let idx = ko_scheduler_candidate_index(next_thread, attempts, MAX_THREADS);
        if ko_scheduler_can_run(idx, current, threads[idx].state == THREAD_READY)
            && ko_scheduler_cpu_allowed(
                threads[idx].has_affinity,
                threads[idx].affinity,
                cpu_id,
            )
        {
            return idx;
        }
        attempts += 1;
    }

    THREAD_ID_IDLE
}

proof fn types_constants_smoke()
    ensures
        MAX_HANDLES_PER_PROCESS == 1024,
        INVALID_HANDLE == 0xffff_ffff,
        PAGE_SIZE == 4096,
{
}

proof fn vmo_checked_end_rejects_overflow(offset: int, len: int, size: int)
    requires
        0 <= offset,
        0 <= len,
        0 <= size,
        offset > usize::MAX as int - len,
    ensures
        checked_end_spec(offset, len) == Option::<int>::None,
{
}

proof fn vmar_overlap_is_symmetric(start_a: int, len_a: int, start_b: int, len_b: int)
    requires
        0 <= start_a,
        0 <= len_a,
        0 <= start_b,
        0 <= len_b,
    ensures
        ranges_overlap_spec(start_a, len_a, start_b, len_b)
            == ranges_overlap_spec(start_b, len_b, start_a, len_a),
{
}

proof fn channel_limits_smoke() {
    assert(channel_message_fits_spec(
        MAX_CHANNEL_MSG_SIZE as int,
        MAX_CHANNEL_MSG_HANDLES as int,
        MAX_CHANNEL_MSG_SIZE as int,
        MAX_CHANNEL_MSG_HANDLES as int,
    ));
    assert(!channel_message_fits_spec(
        MAX_CHANNEL_MSG_SIZE as int + 1,
        0,
        MAX_CHANNEL_MSG_SIZE as int,
        MAX_CHANNEL_MSG_HANDLES as int,
    ));
}

proof fn thread_state_smoke() {
    assert(thread_is_runnable_spec(THREAD_READY as int));
    assert(thread_is_runnable_spec(THREAD_RUNNING as int));
    assert(!thread_is_runnable_spec(THREAD_EMPTY as int));
    assert(!thread_is_runnable_spec(THREAD_BLOCKED as int));
    assert(!thread_is_runnable_spec(THREAD_TERMINATED as int));
}

proof fn scheduler_smoke() {
    assert(scheduler_should_preempt_spec(0, 2));
    assert(!scheduler_should_preempt_spec(1, 2));
    assert(!scheduler_should_preempt_spec(0, 1));
    assert(scheduler_can_run_spec(1, 2, true));
    assert(!scheduler_can_run_spec(0, 2, true));
}

proof fn kernel_object_mod_has_no_pure_runtime_obligation() {
}

} // verus!
