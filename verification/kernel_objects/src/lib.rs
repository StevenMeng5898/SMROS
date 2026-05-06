use vstd::prelude::*;

verus! {

include!("../../../src/kernel_objects/object_logic_shared.rs");
include!("../../../src/kernel_objects/fifo_logic_shared.rs");
include!("../../../src/kernel_objects/futex_logic_shared.rs");
include!("../../../src/kernel_objects/port_logic_shared.rs");
include!("../../../src/kernel_objects/socket_logic_shared.rs");

pub const PAGE_SIZE: usize = 4096;
pub const MAX_HANDLES_PER_PROCESS: usize = 1024;
pub const INVALID_HANDLE: u32 = 0xffff_ffff;
pub const MAX_CHANNEL_MSG_SIZE: usize = 65536;
pub const MAX_CHANNEL_MSG_HANDLES: usize = 64;
pub const CHANNEL_SIGNAL_READABLE: u32 = 1;
pub const CHANNEL_SIGNAL_WRITABLE: u32 = 2;
pub const CHANNEL_SIGNAL_PEER_CLOSED: u32 = 4;
pub const FIFO_MAX_ELEMS: usize = 64;
pub const FIFO_MAX_ELEM_SIZE: usize = 64;
pub const FIFO_BUFFER_SIZE: usize = FIFO_MAX_ELEMS * FIFO_MAX_ELEM_SIZE;
pub const FIFO_CREATE_OPTIONS_MASK: u32 = 0;
pub const FUTEX_ALIGN: usize = 4;
pub const SOCKET_SIZE: usize = 128 * 2048;
pub const SOCKET_DATAGRAM: u32 = 1;
pub const SOCKET_CREATE_MASK: u32 = SOCKET_DATAGRAM;
pub const SOCKET_PEEK: u32 = 1 << 3;
pub const SOCKET_READ_OPTIONS_MASK: u32 = SOCKET_PEEK;
pub const SOCKET_SHUTDOWN_WRITE: u32 = 1;
pub const SOCKET_SHUTDOWN_READ: u32 = 1 << 1;
pub const SOCKET_SHUTDOWN_MASK: u32 = SOCKET_SHUTDOWN_WRITE | SOCKET_SHUTDOWN_READ;
pub const SOCKET_SIGNAL_READ_THRESHOLD: u32 = 1 << 10;
pub const SOCKET_SIGNAL_WRITE_THRESHOLD: u32 = 1 << 11;
pub const PORT_PACKET_SIZE: usize = 48;
pub const PORT_CREATE_OPTIONS_MASK: u32 = 0;
pub const PORT_QUEUE_CAPACITY: usize = 32;
pub const WAIT_ASYNC_TIMESTAMP: u32 = 1 << 0;
pub const WAIT_ASYNC_BOOT_TIMESTAMP: u32 = 1 << 1;
pub const WAIT_ASYNC_EDGE: u32 = 1 << 2;
pub const WAIT_ASYNC_OPTIONS_MASK: u32 =
    WAIT_ASYNC_TIMESTAMP | WAIT_ASYNC_BOOT_TIMESTAMP | WAIT_ASYNC_EDGE;
pub const MAX_THREADS: usize = 16;
pub const THREAD_EMPTY: u8 = 0;
pub const THREAD_READY: u8 = 1;
pub const THREAD_RUNNING: u8 = 2;
pub const THREAD_BLOCKED: u8 = 3;
pub const THREAD_TERMINATED: u8 = 4;
pub const THREAD_ID_IDLE: usize = 0;
pub const RIGHT_DUPLICATE: u32 = 1 << 0;
pub const RIGHT_TRANSFER: u32 = 1 << 1;
pub const RIGHT_READ: u32 = 1 << 2;
pub const RIGHT_WRITE: u32 = 1 << 3;
pub const RIGHT_EXECUTE: u32 = 1 << 4;
pub const RIGHT_MAP: u32 = 1 << 5;
pub const RIGHT_GET_PROPERTY: u32 = 1 << 6;
pub const RIGHT_SET_PROPERTY: u32 = 1 << 7;
pub const RIGHT_ENUMERATE: u32 = 1 << 8;
pub const RIGHT_DESTROY: u32 = 1 << 9;
pub const RIGHT_SET_POLICY: u32 = 1 << 10;
pub const RIGHT_GET_POLICY: u32 = 1 << 11;
pub const RIGHT_SIGNAL: u32 = 1 << 12;
pub const RIGHT_SIGNAL_PEER: u32 = 1 << 13;
pub const RIGHT_WAIT: u32 = 1 << 14;
pub const RIGHT_INSPECT: u32 = 1 << 15;
pub const RIGHT_MANAGE_JOB: u32 = 1 << 16;
pub const RIGHT_MANAGE_PROCESS: u32 = 1 << 17;
pub const RIGHT_MANAGE_THREAD: u32 = 1 << 18;
pub const RIGHT_APPLY_PROFILE: u32 = 1 << 19;
pub const RIGHT_MANAGE_SOCKET: u32 = 1 << 20;
pub const RIGHT_OP_CHILDREN: u32 = 1 << 21;
pub const RIGHT_RESIZE: u32 = 1 << 22;
pub const RIGHT_ATTACH_VMO: u32 = 1 << 23;
pub const RIGHT_MANAGE_VMO: u32 = 1 << 24;
pub const RIGHT_SAME_RIGHTS: u32 = 0x8000_0000;
pub const RIGHTS_ALL: u32 = RIGHT_DUPLICATE
    | RIGHT_TRANSFER
    | RIGHT_READ
    | RIGHT_WRITE
    | RIGHT_EXECUTE
    | RIGHT_MAP
    | RIGHT_GET_PROPERTY
    | RIGHT_SET_PROPERTY
    | RIGHT_ENUMERATE
    | RIGHT_DESTROY
    | RIGHT_SET_POLICY
    | RIGHT_GET_POLICY
    | RIGHT_SIGNAL
    | RIGHT_SIGNAL_PEER
    | RIGHT_WAIT
    | RIGHT_INSPECT
    | RIGHT_MANAGE_JOB
    | RIGHT_MANAGE_PROCESS
    | RIGHT_MANAGE_THREAD
    | RIGHT_APPLY_PROFILE
    | RIGHT_MANAGE_SOCKET
    | RIGHT_OP_CHILDREN
    | RIGHT_RESIZE
    | RIGHT_ATTACH_VMO
    | RIGHT_MANAGE_VMO;
pub const DEFAULT_JOB_RIGHTS: u32 = RIGHT_DUPLICATE
    | RIGHT_TRANSFER
    | RIGHT_READ
    | RIGHT_WRITE
    | RIGHT_GET_PROPERTY
    | RIGHT_SET_PROPERTY
    | RIGHT_ENUMERATE
    | RIGHT_DESTROY
    | RIGHT_SET_POLICY
    | RIGHT_GET_POLICY
    | RIGHT_SIGNAL
    | RIGHT_WAIT
    | RIGHT_INSPECT
    | RIGHT_MANAGE_JOB
    | RIGHT_MANAGE_PROCESS
    | RIGHT_MANAGE_THREAD;
pub const DEFAULT_PROCESS_RIGHTS: u32 = RIGHT_DUPLICATE
    | RIGHT_TRANSFER
    | RIGHT_READ
    | RIGHT_WRITE
    | RIGHT_GET_PROPERTY
    | RIGHT_SET_PROPERTY
    | RIGHT_ENUMERATE
    | RIGHT_DESTROY
    | RIGHT_SIGNAL
    | RIGHT_WAIT
    | RIGHT_INSPECT
    | RIGHT_MANAGE_PROCESS
    | RIGHT_MANAGE_THREAD;
pub const DEFAULT_THREAD_RIGHTS: u32 = RIGHT_DUPLICATE
    | RIGHT_TRANSFER
    | RIGHT_READ
    | RIGHT_WRITE
    | RIGHT_GET_PROPERTY
    | RIGHT_SET_PROPERTY
    | RIGHT_DESTROY
    | RIGHT_SIGNAL
    | RIGHT_WAIT
    | RIGHT_INSPECT
    | RIGHT_MANAGE_THREAD;
pub const DEFAULT_VMAR_RIGHTS: u32 = RIGHT_DUPLICATE
    | RIGHT_TRANSFER
    | RIGHT_READ
    | RIGHT_WRITE
    | RIGHT_EXECUTE
    | RIGHT_MAP
    | RIGHT_GET_PROPERTY
    | RIGHT_SET_PROPERTY
    | RIGHT_DESTROY
    | RIGHT_INSPECT
    | RIGHT_OP_CHILDREN;
pub const SANDBOX_PROCESS_DENIED_RIGHTS: u32 = RIGHT_MANAGE_PROCESS | RIGHT_SET_PROPERTY;
pub const SANDBOX_ROOT_VMAR_DENIED_RIGHTS: u32 = RIGHT_SET_PROPERTY;
pub const SANDBOX_JOB_DENIED_RIGHTS: u32 = RIGHT_MANAGE_JOB
    | RIGHT_MANAGE_PROCESS
    | RIGHT_MANAGE_THREAD
    | RIGHT_SET_POLICY
    | RIGHT_SET_PROPERTY;
pub const SANDBOX_THREAD_DENIED_RIGHTS: u32 = RIGHT_MANAGE_THREAD | RIGHT_SET_PROPERTY;
pub const SANDBOX_PROCESS_RIGHTS: u32 =
    DEFAULT_PROCESS_RIGHTS & !SANDBOX_PROCESS_DENIED_RIGHTS;
pub const SANDBOX_ROOT_VMAR_RIGHTS: u32 =
    DEFAULT_VMAR_RIGHTS & !SANDBOX_ROOT_VMAR_DENIED_RIGHTS;
pub const SANDBOX_JOB_RIGHTS: u32 = DEFAULT_JOB_RIGHTS & !SANDBOX_JOB_DENIED_RIGHTS;
pub const SANDBOX_THREAD_RIGHTS: u32 = DEFAULT_THREAD_RIGHTS & !SANDBOX_THREAD_DENIED_RIGHTS;
pub const MAX_PROCESS_RIGHT_CONFIG_ENTRIES: usize = 16;
pub const PROCESS_RIGHT_CONFIG_JSON_ENTRY_COUNT: usize = 16;

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

spec fn handle_is_valid_spec(handle: int, invalid: int) -> bool {
    handle != 0 && handle != invalid
}

spec fn rights_valid_spec(rights: u32, known_mask: u32) -> bool {
    (rights & !known_mask) == 0
}

spec fn rights_subset_spec(requested: u32, existing: u32) -> bool {
    (requested & !existing) == 0
}

spec fn rights_has_spec(rights: u32, required: u32) -> bool {
    (rights & required) == required
}

spec fn duplicate_rights_allowed_spec(
    existing: u32,
    requested: u32,
    duplicate_right: u32,
    same_rights: u32,
    known_mask: u32,
) -> bool {
    rights_has_spec(existing, duplicate_right)
        && (requested == same_rights
            || (rights_valid_spec(requested, known_mask) && rights_subset_spec(requested, existing)))
}

spec fn replace_rights_allowed_spec(
    existing: u32,
    requested: u32,
    same_rights: u32,
    known_mask: u32,
) -> bool {
    requested == same_rights
        || (rights_valid_spec(requested, known_mask) && rights_subset_spec(requested, existing))
}

spec fn process_right_profile_valid_spec(
    process_rights: u32,
    root_vmar_rights: u32,
    job_rights: u32,
    thread_rights: u32,
    known_mask: u32,
) -> bool {
    rights_valid_spec(process_rights, known_mask)
        && rights_valid_spec(root_vmar_rights, known_mask)
        && rights_valid_spec(job_rights, known_mask)
        && rights_valid_spec(thread_rights, known_mask)
}

spec fn process_right_profile_is_restricted_spec(
    process_rights: u32,
    root_vmar_rights: u32,
    job_rights: u32,
    thread_rights: u32,
) -> bool {
    !rights_has_spec(process_rights, RIGHT_MANAGE_PROCESS)
        && !rights_has_spec(process_rights, RIGHT_SET_PROPERTY)
        && !rights_has_spec(root_vmar_rights, RIGHT_SET_PROPERTY)
        && !rights_has_spec(job_rights, RIGHT_MANAGE_JOB)
        && !rights_has_spec(job_rights, RIGHT_MANAGE_PROCESS)
        && !rights_has_spec(job_rights, RIGHT_MANAGE_THREAD)
        && !rights_has_spec(job_rights, RIGHT_SET_POLICY)
        && !rights_has_spec(job_rights, RIGHT_SET_PROPERTY)
        && !rights_has_spec(thread_rights, RIGHT_MANAGE_THREAD)
        && !rights_has_spec(thread_rights, RIGHT_SET_PROPERTY)
}

spec fn boot_process_right_config_shape_spec(config_len: int, capacity: int) -> bool {
    config_len > 0 && config_len <= capacity
}

spec fn json_process_right_config_install_allowed_spec(
    parsed_entries: int,
    capacity: int,
    trusted_profile_valid: bool,
    sandbox_profile_valid: bool,
    sandbox_profile_restricted: bool,
) -> bool {
    boot_process_right_config_shape_spec(parsed_entries, capacity)
        && trusted_profile_valid
        && sandbox_profile_valid
        && sandbox_profile_restricted
}

spec fn object_signal_update_spec(current: u32, clear_mask: u32, set_mask: u32) -> u32 {
    (current & !clear_mask) | set_mask
}

spec fn futex_ptr_valid_spec(ptr: int, align: int) -> bool {
    ptr != 0 && align != 0 && ptr % align == 0
}

spec fn futex_value_matches_spec(observed: int, expected: int) -> bool {
    observed == expected
}

spec fn futex_min_count_spec(left: int, right: int) -> int {
    if left <= right {
        left
    } else {
        right
    }
}

spec fn futex_saturating_add_spec(left: int, right: int) -> int {
    if left + right > u32::MAX as int {
        u32::MAX as int
    } else {
        left + right
    }
}

spec fn socket_options_valid_spec(options: u32, mask: u32) -> bool {
    (options & !mask) == 0
}

spec fn port_options_valid_spec(options: u32, mask: u32) -> bool {
    (options & !mask) == 0
}

spec fn port_packet_ptr_valid_spec(ptr: int, size: int) -> bool {
    ptr != 0 && size != 0
}

spec fn port_queue_has_space_spec(len: int, capacity: int) -> bool {
    len < capacity
}

spec fn port_signal_packet_allowed_spec(observed: u32, signals: u32) -> bool {
    signals == 0 || (observed & signals) != 0
}

spec fn port_wait_async_options_valid_spec(
    options: u32,
    mask: u32,
    timestamp: u32,
    boot_timestamp: u32,
) -> bool {
    (options & !mask) == 0
        && !((options & timestamp) != 0 && (options & boot_timestamp) != 0)
}

spec fn port_observer_should_queue_spec(
    previous: u32,
    observed: u32,
    signals: u32,
    edge: bool,
) -> bool {
    let previously_allowed = port_signal_packet_allowed_spec(previous, signals);
    let currently_allowed = port_signal_packet_allowed_spec(observed, signals);
    if edge {
        !previously_allowed && currently_allowed
    } else {
        currently_allowed
    }
}

spec fn fifo_options_valid_spec(options: u32, mask: u32) -> bool {
    (options & !mask) == 0
}

spec fn fifo_transfer_bytes_spec(elem_size: int, count: int) -> Option<int> {
    if elem_size < 0 || count < 0 {
        Option::<int>::None
    } else if count == 0 {
        Some(0)
    } else if 0 <= elem_size && 0 <= count && elem_size <= usize::MAX as int / count {
        Some(elem_size * count)
    } else {
        Option::<int>::None
    }
}

spec fn fifo_ring_index_spec(read_pos: int, offset: int, capacity: int) -> int {
    if capacity <= 0 {
        0
    } else {
        (read_pos % capacity + offset % capacity) % capacity
    }
}

spec fn fifo_remaining_capacity_spec(len: int, capacity: int) -> int {
    if len >= capacity {
        0
    } else {
        capacity - len
    }
}

spec fn fifo_min_count_spec(left: int, right: int) -> int {
    if left <= right {
        left
    } else {
        right
    }
}

spec fn fifo_capacity_valid_spec(
    elem_count: int,
    elem_size: int,
    max_count: int,
    max_size: int,
    max_bytes: int,
) -> bool {
    elem_count != 0
        && elem_size != 0
        && elem_count <= max_count
        && elem_size <= max_size
        && match fifo_transfer_bytes_spec(elem_count, elem_size) {
            Some(bytes) => bytes <= max_bytes,
            None => false,
        }
}

spec fn fifo_refresh_read_signals_spec(signals: u32, len: int, readable_signal: u32) -> u32 {
    if len == 0 {
        signals & !readable_signal
    } else {
        signals | readable_signal
    }
}

spec fn fifo_refresh_write_signals_spec(signals: u32, remaining: int, writable_signal: u32) -> u32 {
    if remaining == 0 {
        signals & !writable_signal
    } else {
        signals | writable_signal
    }
}

spec fn socket_mask_options_spec(options: u32, mask: u32) -> u32 {
    options & mask
}

spec fn socket_ring_index_spec(read_pos: int, offset: int, capacity: int) -> int {
    if capacity <= 0 {
        0
    } else {
        (read_pos % capacity + offset % capacity) % capacity
    }
}

spec fn socket_remaining_capacity_spec(len: int, capacity: int) -> int {
    if len >= capacity {
        0
    } else {
        capacity - len
    }
}

spec fn socket_min_count_spec(left: int, right: int) -> int {
    if left <= right {
        left
    } else {
        right
    }
}

spec fn socket_threshold_met_spec(threshold: int, observed: int) -> bool {
    threshold != 0 && observed >= threshold
}

spec fn socket_refresh_read_signals_spec(
    signals: u32,
    len: int,
    threshold: int,
    readable_signal: u32,
    threshold_signal: u32,
) -> u32 {
    let readable_updated = if len == 0 {
        signals & !readable_signal
    } else {
        signals | readable_signal
    };
    if socket_threshold_met_spec(threshold, len) {
        readable_updated | threshold_signal
    } else {
        readable_updated & !threshold_signal
    }
}

spec fn socket_refresh_write_signals_spec(
    signals: u32,
    write_disabled: bool,
    remaining: int,
    threshold: int,
    writable_signal: u32,
    threshold_signal: u32,
) -> u32 {
    let writable_updated = if write_disabled || remaining == 0 {
        signals & !writable_signal
    } else {
        signals | writable_signal
    };
    if socket_threshold_met_spec(threshold, remaining) {
        writable_updated | threshold_signal
    } else {
        writable_updated & !threshold_signal
    }
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

fn ko_rights_subset(requested: u32, existing: u32) -> (out: bool)
    ensures
        out == rights_subset_spec(requested, existing),
{
    smros_ko_rights_subset_body!(requested, existing)
}

fn ko_rights_has(rights: u32, required: u32) -> (out: bool)
    ensures
        out == rights_has_spec(rights, required),
{
    smros_ko_rights_has_body!(rights, required)
}

fn ko_rights_valid(rights: u32, known_mask: u32) -> (out: bool)
    ensures
        out == rights_valid_spec(rights, known_mask),
{
    smros_ko_rights_valid_body!(rights, known_mask)
}

fn ko_process_right_profile_valid(
    process_rights: u32,
    root_vmar_rights: u32,
    job_rights: u32,
    thread_rights: u32,
    known_mask: u32,
) -> (out: bool)
    ensures
        out == process_right_profile_valid_spec(
            process_rights,
            root_vmar_rights,
            job_rights,
            thread_rights,
            known_mask,
        ),
{
    ko_rights_valid(process_rights, known_mask)
        && ko_rights_valid(root_vmar_rights, known_mask)
        && ko_rights_valid(job_rights, known_mask)
        && ko_rights_valid(thread_rights, known_mask)
}

fn ko_process_right_profile_is_restricted(
    process_rights: u32,
    root_vmar_rights: u32,
    job_rights: u32,
    thread_rights: u32,
) -> (out: bool)
    ensures
        out == process_right_profile_is_restricted_spec(
            process_rights,
            root_vmar_rights,
            job_rights,
            thread_rights,
        ),
{
    !ko_rights_has(process_rights, RIGHT_MANAGE_PROCESS)
        && !ko_rights_has(process_rights, RIGHT_SET_PROPERTY)
        && !ko_rights_has(root_vmar_rights, RIGHT_SET_PROPERTY)
        && !ko_rights_has(job_rights, RIGHT_MANAGE_JOB)
        && !ko_rights_has(job_rights, RIGHT_MANAGE_PROCESS)
        && !ko_rights_has(job_rights, RIGHT_MANAGE_THREAD)
        && !ko_rights_has(job_rights, RIGHT_SET_POLICY)
        && !ko_rights_has(job_rights, RIGHT_SET_PROPERTY)
        && !ko_rights_has(thread_rights, RIGHT_MANAGE_THREAD)
        && !ko_rights_has(thread_rights, RIGHT_SET_PROPERTY)
}

fn ko_boot_process_right_config_shape(config_len: usize, capacity: usize) -> (out: bool)
    ensures
        out == boot_process_right_config_shape_spec(config_len as int, capacity as int),
{
    config_len > 0 && config_len <= capacity
}

fn ko_json_process_right_config_install_allowed(
    parsed_entries: usize,
    capacity: usize,
    trusted_profile_valid: bool,
    sandbox_profile_valid: bool,
    sandbox_profile_restricted: bool,
) -> (out: bool)
    ensures
        out == json_process_right_config_install_allowed_spec(
            parsed_entries as int,
            capacity as int,
            trusted_profile_valid,
            sandbox_profile_valid,
            sandbox_profile_restricted,
        ),
{
    ko_boot_process_right_config_shape(parsed_entries, capacity)
        && trusted_profile_valid
        && sandbox_profile_valid
        && sandbox_profile_restricted
}

fn ko_duplicate_rights_allowed(
    existing: u32,
    requested: u32,
    duplicate_right: u32,
    same_rights: u32,
    known_mask: u32,
) -> (out: bool)
    ensures
        out == duplicate_rights_allowed_spec(
            existing,
            requested,
            duplicate_right,
            same_rights,
            known_mask,
        ),
{
    smros_ko_duplicate_rights_allowed_body!(
        existing,
        requested,
        duplicate_right,
        same_rights,
        known_mask
    )
}

fn ko_replace_rights_allowed(
    existing: u32,
    requested: u32,
    same_rights: u32,
    known_mask: u32,
) -> (out: bool)
    ensures
        out == replace_rights_allowed_spec(existing, requested, same_rights, known_mask),
{
    smros_ko_replace_rights_allowed_body!(existing, requested, same_rights, known_mask)
}

fn ko_handle_is_valid(handle: u32, invalid: u32) -> (out: bool)
    ensures
        out == handle_is_valid_spec(handle as int, invalid as int),
{
    smros_ko_handle_is_valid_body!(handle, invalid)
}

fn ko_signal_update(current: u32, clear_mask: u32, set_mask: u32) -> (out: u32)
    ensures
        out == object_signal_update_spec(current, clear_mask, set_mask),
{
    smros_ko_signal_update_body!(current, clear_mask, set_mask)
}

fn futex_ptr_valid(ptr: usize, align: usize) -> (out: bool)
    ensures
        out == futex_ptr_valid_spec(ptr as int, align as int),
{
    smros_futex_ptr_valid_body!(ptr, align)
}

fn futex_value_matches(observed: i32, expected: i32) -> (out: bool)
    ensures
        out == futex_value_matches_spec(observed as int, expected as int),
{
    smros_futex_value_matches_body!(observed, expected)
}

fn futex_min_count(left: u32, right: u32) -> (out: u32)
    ensures
        out as int == futex_min_count_spec(left as int, right as int),
        out <= left,
        out <= right,
{
    smros_futex_min_count_body!(left, right)
}

fn futex_saturating_add(left: u32, right: u32) -> (out: u32)
    ensures
        out as int == futex_saturating_add_spec(left as int, right as int),
        out >= left,
        out >= right,
{
    smros_futex_saturating_add_body!(left, right)
}

fn fifo_options_valid(options: u32, mask: u32) -> (out: bool)
    ensures
        out == fifo_options_valid_spec(options, mask),
{
    smros_fifo_options_valid_body!(options, mask)
}

fn fifo_transfer_bytes(elem_size: usize, count: usize) -> (out: Option<usize>)
    ensures
        count == 0 ==> out == Option::<usize>::Some(0),
{
    smros_fifo_transfer_bytes_body!(elem_size, count)
}

fn fifo_ring_index(read_pos: usize, offset: usize, capacity: usize) -> (out: usize)
    ensures
        capacity == 0 ==> out == 0,
        capacity > 0 ==> out < capacity,
{
    assert(capacity > 0 ==> read_pos % capacity < capacity);
    assert(capacity > 0 ==> offset % capacity < capacity);
    smros_fifo_ring_index_body!(read_pos, offset, capacity)
}

fn fifo_remaining_capacity(len: usize, capacity: usize) -> (out: usize)
    ensures
        out as int == fifo_remaining_capacity_spec(len as int, capacity as int),
        out <= capacity,
{
    smros_fifo_remaining_capacity_body!(len, capacity)
}

fn fifo_min_count(left: usize, right: usize) -> (out: usize)
    ensures
        out as int == fifo_min_count_spec(left as int, right as int),
        out <= left,
        out <= right,
{
    smros_fifo_min_count_body!(left, right)
}

fn fifo_capacity_valid(
    elem_count: usize,
    elem_size: usize,
    max_count: usize,
    max_size: usize,
    max_bytes: usize,
) -> (out: bool)
    ensures
        out ==> elem_count != 0,
        out ==> elem_size != 0,
        out ==> elem_count <= max_count,
        out ==> elem_size <= max_size,
{
    smros_fifo_capacity_valid_body!(elem_count, elem_size, max_count, max_size, max_bytes)
}

fn fifo_refresh_read_signals(signals: u32, len: usize, readable_signal: u32) -> (out: u32)
    ensures
        out == fifo_refresh_read_signals_spec(signals, len as int, readable_signal),
{
    smros_fifo_refresh_read_signals_body!(signals, len, readable_signal)
}

fn fifo_refresh_write_signals(signals: u32, remaining: usize, writable_signal: u32) -> (out: u32)
    ensures
        out == fifo_refresh_write_signals_spec(signals, remaining as int, writable_signal),
{
    smros_fifo_refresh_write_signals_body!(signals, remaining, writable_signal)
}

fn port_options_valid(options: u32, mask: u32) -> (out: bool)
    ensures
        out == port_options_valid_spec(options, mask),
{
    smros_port_options_valid_body!(options, mask)
}

fn port_packet_ptr_valid(ptr: usize, size: usize) -> (out: bool)
    ensures
        out == port_packet_ptr_valid_spec(ptr as int, size as int),
{
    smros_port_packet_ptr_valid_body!(ptr, size)
}

fn port_queue_has_space(len: usize, capacity: usize) -> (out: bool)
    ensures
        out == port_queue_has_space_spec(len as int, capacity as int),
{
    smros_port_queue_has_space_body!(len, capacity)
}

fn port_signal_packet_allowed(observed: u32, signals: u32) -> (out: bool)
    ensures
        out == port_signal_packet_allowed_spec(observed, signals),
{
    smros_port_signal_packet_allowed_body!(observed, signals)
}

fn port_wait_async_options_valid(
    options: u32,
    mask: u32,
    timestamp: u32,
    boot_timestamp: u32,
) -> (out: bool)
    ensures
        out == port_wait_async_options_valid_spec(options, mask, timestamp, boot_timestamp),
{
    smros_port_wait_async_options_valid_body!(options, mask, timestamp, boot_timestamp)
}

fn port_observer_should_queue(
    previous: u32,
    observed: u32,
    signals: u32,
    edge: bool,
) -> (out: bool)
    ensures
        out == port_observer_should_queue_spec(previous, observed, signals, edge),
{
    smros_port_observer_should_queue_body!(previous, observed, signals, edge)
}

fn socket_options_valid(options: u32, mask: u32) -> (out: bool)
    ensures
        out == socket_options_valid_spec(options, mask),
{
    smros_socket_options_valid_body!(options, mask)
}

fn socket_mask_options(options: u32, mask: u32) -> (out: u32)
    ensures
        out == socket_mask_options_spec(options, mask),
{
    smros_socket_mask_options_body!(options, mask)
}

fn socket_ring_index(read_pos: usize, offset: usize, capacity: usize) -> (out: usize)
    ensures
        capacity == 0 ==> out == 0,
        capacity > 0 ==> out < capacity,
{
    assert(capacity > 0 ==> read_pos % capacity < capacity);
    assert(capacity > 0 ==> offset % capacity < capacity);
    smros_socket_ring_index_body!(read_pos, offset, capacity)
}

fn socket_remaining_capacity(len: usize, capacity: usize) -> (out: usize)
    ensures
        out as int == socket_remaining_capacity_spec(len as int, capacity as int),
        out <= capacity,
{
    smros_socket_remaining_capacity_body!(len, capacity)
}

fn socket_min_count(left: usize, right: usize) -> (out: usize)
    ensures
        out as int == socket_min_count_spec(left as int, right as int),
        out <= left,
        out <= right,
{
    smros_socket_min_count_body!(left, right)
}

fn socket_threshold_met(threshold: usize, observed: usize) -> (out: bool)
    ensures
        out == socket_threshold_met_spec(threshold as int, observed as int),
{
    smros_socket_threshold_met_body!(threshold, observed)
}

fn socket_refresh_read_signals(
    signals: u32,
    len: usize,
    threshold: usize,
    readable_signal: u32,
    threshold_signal: u32,
) -> (out: u32)
    ensures
        out == socket_refresh_read_signals_spec(
            signals,
            len as int,
            threshold as int,
            readable_signal,
            threshold_signal,
        ),
{
    smros_socket_refresh_read_signals_body!(
        signals,
        len,
        threshold,
        readable_signal,
        threshold_signal
    )
}

fn socket_refresh_write_signals(
    signals: u32,
    write_disabled: bool,
    remaining: usize,
    threshold: usize,
    writable_signal: u32,
    threshold_signal: u32,
) -> (out: u32)
    ensures
        out == socket_refresh_write_signals_spec(
            signals,
            write_disabled,
            remaining as int,
            threshold as int,
            writable_signal,
            threshold_signal,
        ),
{
    smros_socket_refresh_write_signals_body!(
        signals,
        write_disabled,
        remaining,
        threshold,
        writable_signal,
        threshold_signal
    )
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

fn handle_rights_allow_duplicate_found_model(
    entries: &Vec<HandleEntryModel>,
    handle: u32,
    requested: u32,
) -> (out: bool)
    requires
        exists|i: int| 0 <= i < entries@.len() && entries@[i].valid && entries@[i].handle == handle,
    ensures
        out ==> exists|i: int|
            0 <= i < entries@.len()
                && entries@[i].valid
                && entries@[i].handle == handle
                && duplicate_rights_allowed_spec(
                    entries@[i].rights,
                    requested,
                    RIGHT_DUPLICATE,
                    RIGHT_SAME_RIGHTS,
                    RIGHTS_ALL,
                ),
{
    match handle_get_rights_model(entries, handle) {
        Some(existing) => ko_duplicate_rights_allowed(
            existing,
            requested,
            RIGHT_DUPLICATE,
            RIGHT_SAME_RIGHTS,
            RIGHTS_ALL,
        ),
        None => false,
    }
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

fn capability_rights_smoke() {
    assert(rights_valid_spec(DEFAULT_JOB_RIGHTS, RIGHTS_ALL)) by(bit_vector);
    assert(rights_valid_spec(DEFAULT_PROCESS_RIGHTS, RIGHTS_ALL)) by(bit_vector);
    assert(rights_valid_spec(DEFAULT_THREAD_RIGHTS, RIGHTS_ALL)) by(bit_vector);
    assert(!rights_valid_spec(RIGHT_SAME_RIGHTS, RIGHTS_ALL)) by(bit_vector);
    assert(rights_has_spec(DEFAULT_JOB_RIGHTS, RIGHT_MANAGE_PROCESS)) by(bit_vector);
    assert(rights_has_spec(DEFAULT_JOB_RIGHTS, RIGHT_SET_POLICY)) by(bit_vector);
    assert(rights_has_spec(DEFAULT_THREAD_RIGHTS, RIGHT_DESTROY)) by(bit_vector);
    assert(rights_has_spec(DEFAULT_PROCESS_RIGHTS, RIGHT_MANAGE_THREAD)) by(bit_vector);
    assert(!rights_has_spec(DEFAULT_THREAD_RIGHTS, RIGHT_MANAGE_JOB)) by(bit_vector);
    assert(rights_subset_spec(RIGHT_READ | RIGHT_WRITE, DEFAULT_PROCESS_RIGHTS)) by(bit_vector);
    assert(!rights_subset_spec(RIGHT_MANAGE_JOB, DEFAULT_PROCESS_RIGHTS)) by(bit_vector);

    let same = ko_duplicate_rights_allowed(
        DEFAULT_PROCESS_RIGHTS,
        RIGHT_SAME_RIGHTS,
        RIGHT_DUPLICATE,
        RIGHT_SAME_RIGHTS,
        RIGHTS_ALL,
    );
    let subset = ko_duplicate_rights_allowed(
        DEFAULT_PROCESS_RIGHTS,
        RIGHT_READ | RIGHT_WRITE,
        RIGHT_DUPLICATE,
        RIGHT_SAME_RIGHTS,
        RIGHTS_ALL,
    );
    let escalation = ko_duplicate_rights_allowed(
        DEFAULT_THREAD_RIGHTS,
        RIGHT_MANAGE_JOB,
        RIGHT_DUPLICATE,
        RIGHT_SAME_RIGHTS,
        RIGHTS_ALL,
    );
    let no_duplicate_cap = ko_duplicate_rights_allowed(
        DEFAULT_THREAD_RIGHTS & !RIGHT_DUPLICATE,
        RIGHT_READ,
        RIGHT_DUPLICATE,
        RIGHT_SAME_RIGHTS,
        RIGHTS_ALL,
    );
    let replace_without_duplicate = ko_replace_rights_allowed(
        DEFAULT_THREAD_RIGHTS & !RIGHT_DUPLICATE,
        RIGHT_READ,
        RIGHT_SAME_RIGHTS,
        RIGHTS_ALL,
    );
    let replace_escalation = ko_replace_rights_allowed(
        DEFAULT_THREAD_RIGHTS,
        RIGHT_MANAGE_JOB,
        RIGHT_SAME_RIGHTS,
        RIGHTS_ALL,
    );

    assert(duplicate_rights_allowed_spec(
        DEFAULT_PROCESS_RIGHTS,
        RIGHT_SAME_RIGHTS,
        RIGHT_DUPLICATE,
        RIGHT_SAME_RIGHTS,
        RIGHTS_ALL,
    )) by(bit_vector);
    assert(duplicate_rights_allowed_spec(
        DEFAULT_PROCESS_RIGHTS,
        RIGHT_READ | RIGHT_WRITE,
        RIGHT_DUPLICATE,
        RIGHT_SAME_RIGHTS,
        RIGHTS_ALL,
    )) by(bit_vector);
    assert(!duplicate_rights_allowed_spec(
        DEFAULT_THREAD_RIGHTS,
        RIGHT_MANAGE_JOB,
        RIGHT_DUPLICATE,
        RIGHT_SAME_RIGHTS,
        RIGHTS_ALL,
    )) by(bit_vector);
    assert(!duplicate_rights_allowed_spec(
        DEFAULT_THREAD_RIGHTS & !RIGHT_DUPLICATE,
        RIGHT_READ,
        RIGHT_DUPLICATE,
        RIGHT_SAME_RIGHTS,
        RIGHTS_ALL,
    )) by(bit_vector);
    assert(replace_rights_allowed_spec(
        DEFAULT_THREAD_RIGHTS & !RIGHT_DUPLICATE,
        RIGHT_READ,
        RIGHT_SAME_RIGHTS,
        RIGHTS_ALL,
    )) by(bit_vector);
    assert(!replace_rights_allowed_spec(
        DEFAULT_THREAD_RIGHTS,
        RIGHT_MANAGE_JOB,
        RIGHT_SAME_RIGHTS,
        RIGHTS_ALL,
    )) by(bit_vector);

    assert(same);
    assert(subset);
    assert(!escalation);
    assert(!no_duplicate_cap);
    assert(replace_without_duplicate);
    assert(!replace_escalation);
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

fn fifo_option_masks_smoke() {
    assert(fifo_options_valid_spec(0, FIFO_CREATE_OPTIONS_MASK)) by(bit_vector);
    assert(!fifo_options_valid_spec(1, FIFO_CREATE_OPTIONS_MASK)) by(bit_vector);

    let empty = fifo_options_valid(0, FIFO_CREATE_OPTIONS_MASK);
    let non_empty = fifo_options_valid(1, FIFO_CREATE_OPTIONS_MASK);

    assert(empty);
    assert(!non_empty);
}

fn fifo_transfer_smoke() {
    let zero_count = fifo_transfer_bytes(usize::MAX, 0);

    assert(zero_count == Option::<usize>::Some(0));
}

fn fifo_capacity_smoke() {
    let invalid_zero_count = fifo_capacity_valid(
        0,
        FIFO_MAX_ELEM_SIZE,
        FIFO_MAX_ELEMS,
        FIFO_MAX_ELEM_SIZE,
        FIFO_BUFFER_SIZE,
    );
    let invalid_large_count = fifo_capacity_valid(
        FIFO_MAX_ELEMS + 1,
        FIFO_MAX_ELEM_SIZE,
        FIFO_MAX_ELEMS,
        FIFO_MAX_ELEM_SIZE,
        FIFO_BUFFER_SIZE,
    );
    let wrapped_index = fifo_ring_index(FIFO_MAX_ELEMS - 1, 1, FIFO_MAX_ELEMS);
    let empty_remaining = fifo_remaining_capacity(0, FIFO_MAX_ELEMS);
    let full_remaining = fifo_remaining_capacity(FIFO_MAX_ELEMS, FIFO_MAX_ELEMS);
    let partial_min = fifo_min_count(8, FIFO_MAX_ELEMS);

    assert(!invalid_zero_count);
    assert(!invalid_large_count);
    assert(wrapped_index < FIFO_MAX_ELEMS);
    assert(empty_remaining == FIFO_MAX_ELEMS);
    assert(full_remaining == 0);
    assert(partial_min == 8);
}

fn fifo_signal_smoke() {
    let read_empty = fifo_refresh_read_signals(CHANNEL_SIGNAL_READABLE, 0, CHANNEL_SIGNAL_READABLE);
    let read_ready = fifo_refresh_read_signals(0, 1, CHANNEL_SIGNAL_READABLE);
    let write_full = fifo_refresh_write_signals(CHANNEL_SIGNAL_WRITABLE, 0, CHANNEL_SIGNAL_WRITABLE);
    let write_ready = fifo_refresh_write_signals(0, 1, CHANNEL_SIGNAL_WRITABLE);

    assert(read_empty == fifo_refresh_read_signals_spec(
        CHANNEL_SIGNAL_READABLE,
        0,
        CHANNEL_SIGNAL_READABLE,
    ));
    assert(read_ready == fifo_refresh_read_signals_spec(
        0,
        1,
        CHANNEL_SIGNAL_READABLE,
    ));
    assert(write_full == fifo_refresh_write_signals_spec(
        CHANNEL_SIGNAL_WRITABLE,
        0,
        CHANNEL_SIGNAL_WRITABLE,
    ));
    assert(write_ready == fifo_refresh_write_signals_spec(
        0,
        1,
        CHANNEL_SIGNAL_WRITABLE,
    ));
}

fn futex_helper_smoke() {
    let valid_ptr = futex_ptr_valid(0x1000, FUTEX_ALIGN);
    let null_ptr = futex_ptr_valid(0, FUTEX_ALIGN);
    let unaligned_ptr = futex_ptr_valid(0x1001, FUTEX_ALIGN);
    let value_match = futex_value_matches(7, 7);
    let value_mismatch = futex_value_matches(7, 8);
    let small_min = futex_min_count(2, 5);
    let capped_min = futex_min_count(8, 3);
    let add_small = futex_saturating_add(2, 3);
    let add_overflow = futex_saturating_add(u32::MAX, 1);

    assert(valid_ptr);
    assert(!null_ptr);
    assert(!unaligned_ptr);
    assert(value_match);
    assert(!value_mismatch);
    assert(small_min == 2);
    assert(capped_min == 3);
    assert(add_small == 5);
    assert(add_overflow == u32::MAX);
}

fn port_option_masks_smoke() {
    assert(port_options_valid_spec(0, PORT_CREATE_OPTIONS_MASK)) by(bit_vector);
    assert(!port_options_valid_spec(1, PORT_CREATE_OPTIONS_MASK)) by(bit_vector);
    assert(port_wait_async_options_valid_spec(
        0,
        WAIT_ASYNC_OPTIONS_MASK,
        WAIT_ASYNC_TIMESTAMP,
        WAIT_ASYNC_BOOT_TIMESTAMP,
    )) by(bit_vector);
    assert(port_wait_async_options_valid_spec(
        WAIT_ASYNC_EDGE,
        WAIT_ASYNC_OPTIONS_MASK,
        WAIT_ASYNC_TIMESTAMP,
        WAIT_ASYNC_BOOT_TIMESTAMP,
    )) by(bit_vector);
    assert(!port_wait_async_options_valid_spec(
        WAIT_ASYNC_TIMESTAMP | WAIT_ASYNC_BOOT_TIMESTAMP,
        WAIT_ASYNC_OPTIONS_MASK,
        WAIT_ASYNC_TIMESTAMP,
        WAIT_ASYNC_BOOT_TIMESTAMP,
    )) by(bit_vector);

    let create_empty = port_options_valid(0, PORT_CREATE_OPTIONS_MASK);
    let create_bad = port_options_valid(1, PORT_CREATE_OPTIONS_MASK);
    let async_edge = port_wait_async_options_valid(
        WAIT_ASYNC_EDGE,
        WAIT_ASYNC_OPTIONS_MASK,
        WAIT_ASYNC_TIMESTAMP,
        WAIT_ASYNC_BOOT_TIMESTAMP,
    );
    let async_bad_timestamp_pair = port_wait_async_options_valid(
        WAIT_ASYNC_TIMESTAMP | WAIT_ASYNC_BOOT_TIMESTAMP,
        WAIT_ASYNC_OPTIONS_MASK,
        WAIT_ASYNC_TIMESTAMP,
        WAIT_ASYNC_BOOT_TIMESTAMP,
    );

    assert(create_empty);
    assert(!create_bad);
    assert(async_edge);
    assert(!async_bad_timestamp_pair);
}

fn port_packet_and_queue_smoke() {
    let valid_packet = port_packet_ptr_valid(0x1000, PORT_PACKET_SIZE);
    let null_packet = port_packet_ptr_valid(0, PORT_PACKET_SIZE);
    let zero_sized_packet = port_packet_ptr_valid(0x1000, 0);
    let queue_empty = port_queue_has_space(0, PORT_QUEUE_CAPACITY);
    let queue_last_slot = port_queue_has_space(PORT_QUEUE_CAPACITY - 1, PORT_QUEUE_CAPACITY);
    let queue_full = port_queue_has_space(PORT_QUEUE_CAPACITY, PORT_QUEUE_CAPACITY);

    assert(valid_packet);
    assert(!null_packet);
    assert(!zero_sized_packet);
    assert(queue_empty);
    assert(queue_last_slot);
    assert(!queue_full);
}

fn port_signal_smoke() {
    let any_signal = port_signal_packet_allowed(0, 0);
    let matched_signal = port_signal_packet_allowed(CHANNEL_SIGNAL_READABLE, CHANNEL_SIGNAL_READABLE);
    let missed_signal = port_signal_packet_allowed(0, CHANNEL_SIGNAL_READABLE);
    let level_ready = port_observer_should_queue(
        CHANNEL_SIGNAL_READABLE,
        CHANNEL_SIGNAL_READABLE,
        CHANNEL_SIGNAL_READABLE,
        false,
    );
    let edge_ready = port_observer_should_queue(
        0,
        CHANNEL_SIGNAL_READABLE,
        CHANNEL_SIGNAL_READABLE,
        true,
    );
    let edge_already_ready = port_observer_should_queue(
        CHANNEL_SIGNAL_READABLE,
        CHANNEL_SIGNAL_READABLE,
        CHANNEL_SIGNAL_READABLE,
        true,
    );

    assert(any_signal);
    assert(port_signal_packet_allowed_spec(
        CHANNEL_SIGNAL_READABLE,
        CHANNEL_SIGNAL_READABLE,
    )) by(bit_vector);
    assert(!port_signal_packet_allowed_spec(0, CHANNEL_SIGNAL_READABLE)) by(bit_vector);
    assert(matched_signal
        == port_signal_packet_allowed_spec(CHANNEL_SIGNAL_READABLE, CHANNEL_SIGNAL_READABLE));
    assert(missed_signal == port_signal_packet_allowed_spec(0, CHANNEL_SIGNAL_READABLE));
    assert(matched_signal);
    assert(!missed_signal);
    assert(level_ready);
    assert(edge_ready);
    assert(!edge_already_ready);
}

fn socket_option_masks_smoke() {
    assert(socket_options_valid_spec(0, SOCKET_CREATE_MASK)) by(bit_vector);
    assert(socket_options_valid_spec(SOCKET_DATAGRAM, SOCKET_CREATE_MASK)) by(bit_vector);
    assert(!socket_options_valid_spec(SOCKET_PEEK, SOCKET_CREATE_MASK)) by(bit_vector);
    assert(socket_options_valid_spec(0, SOCKET_READ_OPTIONS_MASK)) by(bit_vector);
    assert(socket_options_valid_spec(SOCKET_PEEK, SOCKET_READ_OPTIONS_MASK)) by(bit_vector);
    assert(!socket_options_valid_spec(SOCKET_DATAGRAM, SOCKET_READ_OPTIONS_MASK)) by(bit_vector);
    assert(socket_mask_options_spec(
        SOCKET_SHUTDOWN_READ | SOCKET_SHUTDOWN_WRITE | SOCKET_PEEK,
        SOCKET_SHUTDOWN_MASK,
    ) == SOCKET_SHUTDOWN_MASK) by(bit_vector);

    let create_empty = socket_options_valid(0, SOCKET_CREATE_MASK);
    let create_datagram = socket_options_valid(SOCKET_DATAGRAM, SOCKET_CREATE_MASK);
    let create_peek = socket_options_valid(SOCKET_PEEK, SOCKET_CREATE_MASK);
    let read_empty = socket_options_valid(0, SOCKET_READ_OPTIONS_MASK);
    let read_peek = socket_options_valid(SOCKET_PEEK, SOCKET_READ_OPTIONS_MASK);
    let read_datagram = socket_options_valid(SOCKET_DATAGRAM, SOCKET_READ_OPTIONS_MASK);
    let shutdown_masked = socket_mask_options(
        SOCKET_SHUTDOWN_READ | SOCKET_SHUTDOWN_WRITE | SOCKET_PEEK,
        SOCKET_SHUTDOWN_MASK,
    );

    assert(create_empty);
    assert(create_datagram);
    assert(!create_peek);
    assert(read_empty);
    assert(read_peek);
    assert(!read_datagram);
    assert(shutdown_masked == SOCKET_SHUTDOWN_MASK);
}

fn socket_threshold_smoke() {
    let socket_size = SOCKET_SIZE;
    let before_full = SOCKET_SIZE - 1;
    let zero_threshold = socket_threshold_met(0, SOCKET_SIZE);
    let exact_threshold = socket_threshold_met(1, 1);
    let threshold_with_space = socket_threshold_met(1, SOCKET_SIZE);
    let threshold_before_full = socket_threshold_met(socket_size, before_full);

    assert(!zero_threshold);
    assert(exact_threshold);
    assert(threshold_with_space);
    assert(!threshold_before_full);
}

fn socket_capacity_smoke() {
    let socket_size = SOCKET_SIZE;
    let over_socket_size = SOCKET_SIZE + 1;
    let last_socket_index = SOCKET_SIZE - 1;
    let empty_remaining = socket_remaining_capacity(0, SOCKET_SIZE);
    let full_remaining = socket_remaining_capacity(SOCKET_SIZE, SOCKET_SIZE);
    let overfull_remaining = socket_remaining_capacity(over_socket_size, socket_size);
    let small_min = socket_min_count(9, SOCKET_SIZE);
    let capped_min = socket_min_count(over_socket_size, socket_size);
    let wrapped_index = socket_ring_index(last_socket_index, 1, socket_size);

    assert(empty_remaining == SOCKET_SIZE);
    assert(full_remaining == 0);
    assert(overfull_remaining == 0);
    assert(small_min == 9);
    assert(capped_min == socket_size);
    assert(wrapped_index < socket_size);
}

fn socket_signal_smoke() {
    assert(CHANNEL_SIGNAL_READABLE & SOCKET_SIGNAL_READ_THRESHOLD == 0) by(bit_vector);
    assert(CHANNEL_SIGNAL_WRITABLE & SOCKET_SIGNAL_WRITE_THRESHOLD == 0) by(bit_vector);

    let read_empty = socket_refresh_read_signals(
        CHANNEL_SIGNAL_READABLE | SOCKET_SIGNAL_READ_THRESHOLD,
        0,
        1,
        CHANNEL_SIGNAL_READABLE,
        SOCKET_SIGNAL_READ_THRESHOLD,
    );
    assert(read_empty
        == socket_refresh_read_signals_spec(
            CHANNEL_SIGNAL_READABLE | SOCKET_SIGNAL_READ_THRESHOLD,
            0,
            1,
            CHANNEL_SIGNAL_READABLE,
            SOCKET_SIGNAL_READ_THRESHOLD,
        ));
    assert(read_empty
        == socket_refresh_read_signals_spec(
            CHANNEL_SIGNAL_READABLE | SOCKET_SIGNAL_READ_THRESHOLD,
            0,
            1,
            CHANNEL_SIGNAL_READABLE,
            SOCKET_SIGNAL_READ_THRESHOLD,
        ));

    let read_ready = socket_refresh_read_signals(
        0,
        2,
        1,
        CHANNEL_SIGNAL_READABLE,
        SOCKET_SIGNAL_READ_THRESHOLD,
    );
    assert(read_ready
        == socket_refresh_read_signals_spec(
            0,
            2,
            1,
            CHANNEL_SIGNAL_READABLE,
            SOCKET_SIGNAL_READ_THRESHOLD,
        ));
    assert(read_ready
        == socket_refresh_read_signals_spec(
            0,
            2,
            1,
            CHANNEL_SIGNAL_READABLE,
            SOCKET_SIGNAL_READ_THRESHOLD,
        ));

    let write_blocked = socket_refresh_write_signals(
        CHANNEL_SIGNAL_WRITABLE | SOCKET_SIGNAL_WRITE_THRESHOLD,
        true,
        SOCKET_SIZE,
        1,
        CHANNEL_SIGNAL_WRITABLE,
        SOCKET_SIGNAL_WRITE_THRESHOLD,
    );
    assert(write_blocked
        == socket_refresh_write_signals_spec(
            CHANNEL_SIGNAL_WRITABLE | SOCKET_SIGNAL_WRITE_THRESHOLD,
            true,
            SOCKET_SIZE as int,
            1,
            CHANNEL_SIGNAL_WRITABLE,
            SOCKET_SIGNAL_WRITE_THRESHOLD,
        ));
    assert(write_blocked
        == socket_refresh_write_signals_spec(
            CHANNEL_SIGNAL_WRITABLE | SOCKET_SIGNAL_WRITE_THRESHOLD,
            true,
            SOCKET_SIZE as int,
            1,
            CHANNEL_SIGNAL_WRITABLE,
            SOCKET_SIGNAL_WRITE_THRESHOLD,
        ));

    let write_ready = socket_refresh_write_signals(
        0,
        false,
        SOCKET_SIZE,
        1,
        CHANNEL_SIGNAL_WRITABLE,
        SOCKET_SIGNAL_WRITE_THRESHOLD,
    );
    assert(write_ready
        == socket_refresh_write_signals_spec(
            0,
            false,
            SOCKET_SIZE as int,
            1,
            CHANNEL_SIGNAL_WRITABLE,
            SOCKET_SIGNAL_WRITE_THRESHOLD,
        ));
    assert(write_ready
        == socket_refresh_write_signals_spec(
            0,
            false,
            SOCKET_SIZE as int,
            1,
            CHANNEL_SIGNAL_WRITABLE,
            SOCKET_SIGNAL_WRITE_THRESHOLD,
        ));
}

fn process_right_profile_smoke() {
    let trusted_valid = ko_process_right_profile_valid(
        DEFAULT_PROCESS_RIGHTS,
        DEFAULT_VMAR_RIGHTS,
        DEFAULT_JOB_RIGHTS,
        DEFAULT_THREAD_RIGHTS,
        RIGHTS_ALL,
    );
    let sandbox_valid = ko_process_right_profile_valid(
        SANDBOX_PROCESS_RIGHTS,
        SANDBOX_ROOT_VMAR_RIGHTS,
        SANDBOX_JOB_RIGHTS,
        SANDBOX_THREAD_RIGHTS,
        RIGHTS_ALL,
    );
    let sandbox_restricted = ko_process_right_profile_is_restricted(
        SANDBOX_PROCESS_RIGHTS,
        SANDBOX_ROOT_VMAR_RIGHTS,
        SANDBOX_JOB_RIGHTS,
        SANDBOX_THREAD_RIGHTS,
    );

    assert(rights_valid_spec(DEFAULT_PROCESS_RIGHTS, RIGHTS_ALL)) by(bit_vector);
    assert(rights_valid_spec(DEFAULT_VMAR_RIGHTS, RIGHTS_ALL)) by(bit_vector);
    assert(rights_valid_spec(DEFAULT_JOB_RIGHTS, RIGHTS_ALL)) by(bit_vector);
    assert(rights_valid_spec(DEFAULT_THREAD_RIGHTS, RIGHTS_ALL)) by(bit_vector);
    assert(process_right_profile_valid_spec(
        DEFAULT_PROCESS_RIGHTS,
        DEFAULT_VMAR_RIGHTS,
        DEFAULT_JOB_RIGHTS,
        DEFAULT_THREAD_RIGHTS,
        RIGHTS_ALL,
    )) by(bit_vector);
    assert(process_right_profile_valid_spec(
        SANDBOX_PROCESS_RIGHTS,
        SANDBOX_ROOT_VMAR_RIGHTS,
        SANDBOX_JOB_RIGHTS,
        SANDBOX_THREAD_RIGHTS,
        RIGHTS_ALL,
    )) by(bit_vector);
    assert(process_right_profile_is_restricted_spec(
        SANDBOX_PROCESS_RIGHTS,
        SANDBOX_ROOT_VMAR_RIGHTS,
        SANDBOX_JOB_RIGHTS,
        SANDBOX_THREAD_RIGHTS,
    )) by(bit_vector);
    assert(trusted_valid);
    assert(sandbox_valid);
    assert(sandbox_restricted);
}

fn boot_process_right_config_smoke() {
    let shape_ok = ko_boot_process_right_config_shape(
        PROCESS_RIGHT_CONFIG_JSON_ENTRY_COUNT,
        MAX_PROCESS_RIGHT_CONFIG_ENTRIES,
    );

    assert(boot_process_right_config_shape_spec(
        PROCESS_RIGHT_CONFIG_JSON_ENTRY_COUNT as int,
        MAX_PROCESS_RIGHT_CONFIG_ENTRIES as int,
    ));
    assert(process_right_profile_valid_spec(
        DEFAULT_PROCESS_RIGHTS,
        DEFAULT_VMAR_RIGHTS,
        DEFAULT_JOB_RIGHTS,
        DEFAULT_THREAD_RIGHTS,
        RIGHTS_ALL,
    )) by(bit_vector);
    assert(process_right_profile_valid_spec(
        SANDBOX_PROCESS_RIGHTS,
        SANDBOX_ROOT_VMAR_RIGHTS,
        SANDBOX_JOB_RIGHTS,
        SANDBOX_THREAD_RIGHTS,
        RIGHTS_ALL,
    )) by(bit_vector);
    assert(process_right_profile_is_restricted_spec(
        SANDBOX_PROCESS_RIGHTS,
        SANDBOX_ROOT_VMAR_RIGHTS,
        SANDBOX_JOB_RIGHTS,
        SANDBOX_THREAD_RIGHTS,
    )) by(bit_vector);

    let trusted_valid = ko_process_right_profile_valid(
        DEFAULT_PROCESS_RIGHTS,
        DEFAULT_VMAR_RIGHTS,
        DEFAULT_JOB_RIGHTS,
        DEFAULT_THREAD_RIGHTS,
        RIGHTS_ALL,
    );
    let sandbox_valid = ko_process_right_profile_valid(
        SANDBOX_PROCESS_RIGHTS,
        SANDBOX_ROOT_VMAR_RIGHTS,
        SANDBOX_JOB_RIGHTS,
        SANDBOX_THREAD_RIGHTS,
        RIGHTS_ALL,
    );
    let sandbox_restricted = ko_process_right_profile_is_restricted(
        SANDBOX_PROCESS_RIGHTS,
        SANDBOX_ROOT_VMAR_RIGHTS,
        SANDBOX_JOB_RIGHTS,
        SANDBOX_THREAD_RIGHTS,
    );
    let install_allowed = ko_json_process_right_config_install_allowed(
        PROCESS_RIGHT_CONFIG_JSON_ENTRY_COUNT,
        MAX_PROCESS_RIGHT_CONFIG_ENTRIES,
        trusted_valid,
        sandbox_valid,
        sandbox_restricted,
    );

    assert(shape_ok);
    assert(trusted_valid);
    assert(sandbox_valid);
    assert(sandbox_restricted);
    assert(install_allowed);
}

proof fn kernel_object_mod_has_no_pure_runtime_obligation() {
}

} // verus!
