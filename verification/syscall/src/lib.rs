use vstd::prelude::*;

verus! {

include!("../../../src/syscall/address_logic_shared.rs");
include!("../../../src/syscall/syscall_bridge_shared.rs");
include!("../../../src/syscall/syscall_logic_shared.rs");

pub const PAGE_SIZE: usize = 4096;
pub const LINUX_MAPPING_BASE: usize = 0x5000_0000;
pub const LINUX_MAPPING_LIMIT: usize = 0x6000_0000;
pub const SMROS_SYSCALL_LINUX_THRESHOLD_U64: u64 =
    smros_syscall_linux_threshold_u64!();
pub const SMROS_ZIRCON_SYSCALL_BASE_U64: u64 = SMROS_SYSCALL_LINUX_THRESHOLD_U64;
pub const SMROS_INVALID_HANDLE_U32: u32 = 0xFFFF_FFFF;
pub const SMROS_SAVED_REG_FRAME_BYTES: usize = smros_saved_reg_frame_bytes!();
pub const SMROS_SAVED_REG_WORDS: usize = smros_saved_reg_words!();
pub const SMROS_SAVED_REG_COUNT: usize = smros_saved_reg_count!();
pub const SMROS_SYSCALL_NUMBER_REG_INDEX: usize = smros_syscall_number_reg_index!();
pub const SMROS_SYSCALL_ARG_COUNT_LINUX: usize = smros_syscall_arg_count_linux!();
pub const SMROS_SYSCALL_ARG_COUNT_ZIRCON: usize = smros_syscall_arg_count_zircon!();
pub const ZX_USER_SIGNAL_MASK: u32 = 0xffu32 << 24;
pub const ZX_EVENT_SIGNALED: u32 = 1u32 << 4;
pub const ZX_EVENTPAIR_SIGNALED: u32 = 1u32 << 4;
pub const ZX_EVENT_SIGNAL_MASK: u32 = ZX_USER_SIGNAL_MASK | ZX_EVENT_SIGNALED;
pub const ZX_EVENTPAIR_SIGNAL_MASK: u32 = ZX_USER_SIGNAL_MASK | ZX_EVENTPAIR_SIGNALED;
pub const ZX_CLOCK_OPT_AUTO_START: u32 = 1u32 << 0;
pub const ZX_CLOCK_UPDATE_OPTION_SYNTHETIC_VALUE_VALID: u64 = 1u64 << 0;
pub const ZX_CLOCK_UPDATE_OPTION_REFERENCE_VALUE_VALID: u64 = 1u64 << 1;
pub const ZX_CLOCK_UPDATE_OPTIONS_MASK: u64 =
    ZX_CLOCK_UPDATE_OPTION_SYNTHETIC_VALUE_VALID | ZX_CLOCK_UPDATE_OPTION_REFERENCE_VALUE_VALID;
pub const ZX_TIMER_OPTIONS_MASK: u32 = 0;
pub const ZX_DEBUGLOG_CREATE_OPTIONS_MASK: u32 = 0;
pub const ZX_DEBUGLOG_OPTIONS_MASK: u32 = 0;
pub const ZX_SYSTEM_EVENT_KIND_MAX: u32 = 3;
pub const ZX_EXCEPTION_CHANNEL_DEBUGGER: u32 = 1u32 << 0;
pub const ZX_EXCEPTION_CHANNEL_OPTIONS_MASK: u32 = ZX_EXCEPTION_CHANNEL_DEBUGGER;
pub const ZX_HYPERVISOR_OPTIONS_MASK: u32 = 0;
pub const ZX_GUEST_TRAP_BELL: u32 = 0;
pub const ZX_GUEST_TRAP_MEM: u32 = 1;
pub const ZX_GUEST_TRAP_IO: u32 = 2;
pub const ZX_GUEST_TRAP_KIND_MAX: u32 = ZX_GUEST_TRAP_IO;
pub const ZX_GUEST_PHYS_LIMIT: u64 = 0x1_0000_0000;
pub const ZX_VCPU_ENTRY_ALIGNMENT: u64 = 4;
pub const ZX_VCPU_INTERRUPT_VECTOR_MAX: u32 = 1023;
pub const ZX_VCPU_STATE: u32 = 0;
pub const ZX_VCPU_IO: u32 = 1;
pub const ZX_VCPU_STATE_SIZE: usize = 256;
pub const ZX_VCPU_IO_SIZE: usize = 24;
pub const LINUX_AF_UNIX: usize = 1;
pub const LINUX_AF_LOCAL: usize = LINUX_AF_UNIX;
pub const LINUX_AF_INET: usize = 2;
pub const LINUX_AF_NETLINK: usize = 16;
pub const LINUX_AF_PACKET: usize = 17;
pub const LINUX_SOCK_TYPE_MASK: usize = 0xff;
pub const LINUX_SOCK_STREAM: usize = 1;
pub const LINUX_SOCK_DGRAM: usize = 2;
pub const LINUX_SOCK_RAW: usize = 3;
pub const LINUX_SOCK_NONBLOCK: usize = 0x800;
pub const LINUX_SOCK_CLOEXEC: usize = 0x80000;
pub const LINUX_SOCK_ALLOWED_FLAGS: usize =
    LINUX_SOCK_TYPE_MASK | LINUX_SOCK_NONBLOCK | LINUX_SOCK_CLOEXEC;
pub const LINUX_MAX_SIGNAL: usize = 64;
pub const LINUX_SIGSET_SIZE: usize = 8;
pub const LINUX_MAX_SEMAPHORES: usize = 256;
pub const LINUX_MAX_IPC_BYTES: usize = 65536;
pub const LINUX_MAX_MSG_BYTES: usize = 8192;
pub const LINUX_MEMFD_ALLOWED_FLAGS: usize = 0x0001 | 0x0002 | 0x0004;
pub const LINUX_GETRANDOM_ALLOWED_FLAGS: u32 = 0x0001 | 0x0002;
pub const LINUX_STDIO_FD_MAX: usize = 2;
pub const LINUX_O_ACCMODE: usize = 0o3;
pub const LINUX_O_RDONLY: usize = 0;
pub const LINUX_O_WRONLY: usize = 1;
pub const LINUX_O_RDWR: usize = 2;
pub const LINUX_O_CREAT: usize = 0o100;
pub const LINUX_O_EXCL: usize = 0o200;
pub const LINUX_O_TRUNC: usize = 0o1000;
pub const LINUX_O_APPEND: usize = 0o2000;
pub const LINUX_O_NONBLOCK: usize = 0o4000;
pub const LINUX_O_DIRECTORY: usize = 0o200000;
pub const LINUX_O_CLOEXEC: usize = 0o2000000;
pub const LINUX_OPEN_ALLOWED_FLAGS: usize = LINUX_O_ACCMODE
    | LINUX_O_CREAT
    | LINUX_O_EXCL
    | LINUX_O_TRUNC
    | LINUX_O_APPEND
    | LINUX_O_NONBLOCK
    | LINUX_O_DIRECTORY
    | LINUX_O_CLOEXEC;
pub const LINUX_PIPE_ALLOWED_FLAGS: usize = LINUX_O_CLOEXEC | LINUX_O_NONBLOCK;
pub const LINUX_FCNTL_STATUS_ALLOWED_FLAGS: usize = LINUX_O_APPEND | LINUX_O_NONBLOCK;
pub const LINUX_ACCESS_MODE_MASK: usize = 0o7;
pub const LINUX_AT_REMOVEDIR: usize = 0x200;
pub const LINUX_UNLINK_ALLOWED_FLAGS: usize = LINUX_AT_REMOVEDIR;
pub const LINUX_RENAME_NOREPLACE: usize = 1;
pub const LINUX_RENAME_EXCHANGE: usize = 2;
pub const LINUX_RENAME_WHITEOUT: usize = 4;
pub const LINUX_RENAME_ALLOWED_FLAGS: usize =
    LINUX_RENAME_NOREPLACE | LINUX_RENAME_EXCHANGE | LINUX_RENAME_WHITEOUT;
pub const LINUX_AT_SYMLINK_NOFOLLOW: usize = 0x100;
pub const LINUX_AT_EMPTY_PATH: usize = 0x1000;
pub const LINUX_STAT_ALLOWED_FLAGS: usize = LINUX_AT_SYMLINK_NOFOLLOW | LINUX_AT_EMPTY_PATH;
pub const LINUX_STATX_BASIC_STATS: usize = 0x7ff;
pub const LINUX_SEEK_MAX_WHENCE: usize = 5;
pub const LINUX_MAX_IOV: usize = 1024;
pub const LINUX_MAX_POLL_FDS: usize = 1024;
pub const LINUX_POLL_ALLOWED_EVENTS: i16 = 0x0001 | 0x0004 | 0x0008 | 0x0010 | 0x0020 | 0x0040;

#[derive(Copy, Clone)]
struct LinuxRange {
    addr: usize,
    len: usize,
}

spec fn checked_end_spec(addr: int, len: int) -> Option<int> {
    if 0 <= addr && 0 <= len && addr <= usize::MAX as int - len {
        Some(addr + len)
    } else {
        Option::<int>::None
    }
}

spec fn range_overlaps_spec(start_a: int, len_a: int, start_b: int, len_b: int) -> bool {
    match (checked_end_spec(start_a, len_a), checked_end_spec(start_b, len_b)) {
        (Some(end_a), Some(end_b)) => start_a < end_b && start_b < end_a,
        _ => false,
    }
}

spec fn range_within_window_spec(addr: int, len: int, base: int, limit: int) -> bool {
    match checked_end_spec(addr, len) {
        Some(end) => addr >= base && end <= limit,
        None => false,
    }
}

spec fn page_aligned_spec(addr: int) -> bool {
    addr % PAGE_SIZE as int == 0
}

spec fn fixed_linux_mmap_request_ok_spec(addr: int, len: int) -> bool {
    page_aligned_spec(addr)
        && range_within_window_spec(
            addr,
            len,
            LINUX_MAPPING_BASE as int,
            LINUX_MAPPING_LIMIT as int,
        )
}

spec fn range_ends_before_spec(addr: int, len: int, bound: int) -> bool {
    match checked_end_spec(addr, len) {
        Some(end) => end <= bound,
        None => false,
    }
}

spec fn no_overlap_with_mappings_spec(addr: int, len: int, mappings: Seq<LinuxRange>) -> bool {
    forall|i: int|
        0 <= i < mappings.len() ==> !range_overlaps_spec(
            addr,
            len,
            mappings[i].addr as int,
            mappings[i].len as int,
        )
}

spec fn linux_range_available_spec(addr: int, len: int, mappings: Seq<LinuxRange>) -> bool {
    range_within_window_spec(
        addr,
        len,
        LINUX_MAPPING_BASE as int,
        LINUX_MAPPING_LIMIT as int,
    ) && no_overlap_with_mappings_spec(addr, len, mappings)
}

spec fn linux_syscall_number_spec(syscall_num: int) -> bool {
    0 <= syscall_num && syscall_num < SMROS_SYSCALL_LINUX_THRESHOLD_U64 as int
}

spec fn linux_errno_return_spec(errno: int) -> int {
    if errno == 0 {
        0
    } else {
        u64::MAX as int + 1 - errno
    }
}

spec fn linux_sys_result_return_spec(is_ok: bool, value: int, errno: int) -> int {
    if is_ok {
        value
    } else {
        linux_errno_return_spec(errno)
    }
}

spec fn saved_reg_arg_spec(regs: Seq<u64>, idx: int) -> int {
    regs[idx] as int
}

spec fn zircon_syscall_number_spec(syscall_num: int) -> bool {
    SMROS_ZIRCON_SYSCALL_BASE_U64 as int <= syscall_num
        && syscall_num <= SMROS_ZIRCON_SYSCALL_BASE_U64 as int + u32::MAX as int
}

spec fn zircon_syscall_from_raw_spec(syscall_num: int) -> int {
    if zircon_syscall_number_spec(syscall_num) {
        syscall_num - SMROS_ZIRCON_SYSCALL_BASE_U64 as int
    } else {
        u32::MAX as int
    }
}

spec fn handle_invalid_spec(handle: int, invalid_handle: int) -> bool {
    handle == 0 || handle == invalid_handle
}

spec fn user_buffer_valid_spec(ptr: int, len: int) -> bool {
    len == 0 || ptr != 0
}

spec fn channel_buffers_valid_spec(
    bytes_ptr: int,
    bytes_len: int,
    handles_ptr: int,
    handles_len: int,
) -> bool {
    user_buffer_valid_spec(bytes_ptr, bytes_len)
        && user_buffer_valid_spec(handles_ptr, handles_len)
}

spec fn signal_update_spec(current: u32, clear_mask: u32, set_mask: u32) -> u32 {
    (current & !clear_mask) | set_mask
}

spec fn signal_mask_allowed_spec(clear_mask: u32, set_mask: u32, allowed_mask: u32) -> bool {
    ((clear_mask | set_mask) & !allowed_mask) == 0
}

spec fn wait_satisfied_spec(observed: u32, requested: u32) -> bool {
    requested == 0 || (observed & requested) != 0
}

spec fn linux_clock_id_supported_spec(clock_id: int) -> bool {
    0 <= clock_id && clock_id <= 1
}

spec fn linux_signal_valid_spec(signum: int, max_signal: int) -> bool {
    0 <= signum && signum <= max_signal
}

spec fn linux_signal_action_valid_spec(signum: int, max_signal: int) -> bool {
    0 < signum && signum <= max_signal
}

spec fn linux_sigset_size_valid_spec(size: int, expected: int) -> bool {
    size == expected
}

spec fn linux_ipc_count_valid_spec(count: int, max_count: int) -> bool {
    0 < count && count <= max_count
}

spec fn linux_ipc_size_valid_spec(size: int, max_size: int) -> bool {
    0 < size && size <= max_size
}

spec fn linux_msg_size_valid_spec(size: int, max_size: int) -> bool {
    0 <= size && size <= max_size
}

spec fn linux_socket_domain_supported_spec(
    domain: usize,
    unix: usize,
    local: usize,
    inet: usize,
    netlink: usize,
    packet: usize,
) -> bool {
    domain == unix || domain == local || domain == inet || domain == netlink || domain == packet
}

spec fn linux_socket_type_supported_spec(
    socket_type: usize,
    mask: usize,
    stream: usize,
    dgram: usize,
    raw: usize,
) -> bool {
    let kind = socket_type & mask;
    kind == stream || kind == dgram || kind == raw
}

spec fn linux_socket_domain_type_supported_spec(
    domain: usize,
    kind: usize,
    unix: usize,
    local: usize,
    inet: usize,
    netlink: usize,
    packet: usize,
    stream: usize,
    dgram: usize,
    raw: usize,
) -> bool {
    if domain == unix || domain == local {
        kind == stream || kind == dgram
    } else if domain == inet {
        kind == stream || kind == dgram || kind == raw
    } else if domain == netlink || domain == packet {
        kind == dgram || kind == raw
    } else {
        false
    }
}

spec fn linux_socket_addr_valid_spec(ptr: int, len: int) -> bool {
    user_buffer_valid_spec(ptr, len)
}

spec fn linux_fd_range_valid_spec(first: int, last: int) -> bool {
    first <= last
}

spec fn linux_usize_options_within_mask_spec(options: usize, allowed_mask: usize) -> bool {
    (options & !allowed_mask) == 0
}

spec fn linux_u32_options_within_mask_spec(options: u32, allowed_mask: u32) -> bool {
    (options & !allowed_mask) == 0
}

spec fn linux_open_access_mode_valid_spec(
    flags: usize,
    access_mask: usize,
    read_only: usize,
    write_only: usize,
    read_write: usize,
) -> bool {
    let access = flags & access_mask;
    access == read_only || access == write_only || access == read_write
}

spec fn linux_open_is_directory_spec(flags: usize, directory_flag: usize) -> bool {
    (flags & directory_flag) != 0
}

spec fn linux_fd_target_valid_spec(fd: int, stdio_max: int) -> bool {
    0 <= fd && fd <= stdio_max
}

spec fn linux_dup3_args_valid_spec(old_fd: int, new_fd: int) -> bool {
    old_fd != new_fd
}

spec fn linux_fcntl_cmd_supported_spec(
    cmd: int,
    dupfd: int,
    getfd: int,
    setfd: int,
    getfl: int,
    setfl: int,
    dupfd_cloexec: int,
) -> bool {
    cmd == dupfd
        || cmd == getfd
        || cmd == setfd
        || cmd == getfl
        || cmd == setfl
        || cmd == dupfd_cloexec
}

spec fn linux_lseek_whence_valid_spec(whence: int, max_whence: int) -> bool {
    0 <= whence && whence <= max_whence
}

spec fn linux_iov_count_valid_spec(count: int, max_count: int) -> bool {
    0 <= count && count <= max_count
}

spec fn linux_iov_bytes_valid_spec(count: int, elem_size: int, max_count: int) -> bool {
    0 <= count && 0 < elem_size && count <= max_count && count <= usize::MAX as int / elem_size
}

spec fn linux_poll_count_valid_spec(count: int, max_count: int) -> bool {
    0 <= count && count <= max_count
}

spec fn linux_i16_options_within_mask_spec(options: i16, allowed_mask: i16) -> bool {
    (options & !allowed_mask) == 0
}

spec fn zircon_clock_id_supported_spec(clock_id: int) -> bool {
    0 <= clock_id && clock_id <= 1
}

spec fn u32_options_within_mask_spec(options: u32, allowed_mask: u32) -> bool {
    (options & !allowed_mask) == 0
}

spec fn u64_options_within_mask_spec(options: u64, allowed_mask: u64) -> bool {
    (options & !allowed_mask) == 0
}

spec fn zircon_timer_deadline_expired_spec(deadline: u64, now: u64) -> bool {
    deadline <= now
}

spec fn zircon_system_event_kind_valid_spec(kind: int, max_kind: int) -> bool {
    0 <= kind && kind <= max_kind
}

spec fn zircon_guest_trap_range_valid_spec(addr: int, size: int, limit: int) -> bool {
    0 <= addr && 0 < size && addr <= limit && size <= limit - addr
}

spec fn zircon_guest_trap_alignment_valid_spec(
    kind: int,
    addr: int,
    size: int,
    bell: int,
    mem: int,
    page_size: int,
) -> bool {
    if kind == bell || kind == mem {
        page_size > 0 && addr % page_size == 0 && size % page_size == 0
    } else {
        true
    }
}

spec fn zircon_vcpu_entry_valid_spec(entry: int, alignment: int) -> bool {
    alignment > 0 && entry % alignment == 0
}

spec fn zircon_vcpu_interrupt_vector_valid_spec(vector: int, max_vector: int) -> bool {
    0 <= vector && vector <= max_vector
}

spec fn zircon_vcpu_read_state_args_valid_spec(
    kind: int,
    buffer_size: int,
    state_kind: int,
    state_size: int,
) -> bool {
    kind == state_kind && buffer_size == state_size
}

spec fn zircon_vcpu_write_state_args_valid_spec(
    kind: int,
    buffer_size: int,
    state_kind: int,
    state_size: int,
    io_kind: int,
    io_size: int,
) -> bool {
    (kind == state_kind && buffer_size == state_size)
        || (kind == io_kind && buffer_size == io_size)
}

spec fn linux_syscall_interface_known_spec(syscall_num: int) -> bool {
    0 <= syscall_num && (syscall_num <= 446 || syscall_num == 600)
}

spec fn zircon_syscall_interface_known_spec(syscall_num: int) -> bool {
    0 <= syscall_num && (syscall_num <= 154 || (183 <= syscall_num && syscall_num <= 211))
}

fn checked_end(addr: usize, len: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(end) => checked_end_spec(addr as int, len as int) == Some(end as int),
            None => checked_end_spec(addr as int, len as int) == Option::<int>::None,
        },
{
    smros_checked_end_body!(addr, len)
}

fn range_overlaps(start_a: usize, len_a: usize, start_b: usize, len_b: usize) -> (out: bool)
    ensures
        out == range_overlaps_spec(start_a as int, len_a as int, start_b as int, len_b as int),
{
    smros_range_overlaps_body!(start_a, len_a, start_b, len_b)
}

fn range_within_window(addr: usize, len: usize, base: usize, limit: usize) -> (out: bool)
    ensures
        out == range_within_window_spec(addr as int, len as int, base as int, limit as int),
{
    smros_range_within_window_body!(addr, len, base, limit)
}

fn page_aligned(addr: usize) -> (out: bool)
    ensures
        out == page_aligned_spec(addr as int),
{
    smros_page_aligned_body!(addr, PAGE_SIZE)
}

fn fixed_linux_mmap_request_ok(addr: usize, len: usize) -> (out: bool)
    ensures
        out == fixed_linux_mmap_request_ok_spec(addr as int, len as int),
{
    smros_fixed_linux_mmap_request_ok_body!(
        addr,
        len,
        PAGE_SIZE,
        LINUX_MAPPING_BASE,
        LINUX_MAPPING_LIMIT
    )
}

fn no_overlap_with_mappings(addr: usize, len: usize, mappings: &Vec<LinuxRange>) -> (out: bool)
    ensures
        out == no_overlap_with_mappings_spec(addr as int, len as int, mappings@),
{
    let mut i = 0usize;
    while i < mappings.len()
        invariant
            i <= mappings.len(),
            forall|j: int|
                0 <= j < i as int ==> !range_overlaps_spec(
                    addr as int,
                    len as int,
                    mappings@[j].addr as int,
                    mappings@[j].len as int,
                ),
        decreases mappings.len() - i,
    {
        let mapping = &mappings[i];
        if range_overlaps(addr, len, mapping.addr, mapping.len) {
            assert(range_overlaps_spec(
                addr as int,
                len as int,
                mappings@[i as int].addr as int,
                mappings@[i as int].len as int,
            ));
            assert(!no_overlap_with_mappings_spec(addr as int, len as int, mappings@)) by {
                assert(range_overlaps_spec(
                    addr as int,
                    len as int,
                    mappings@[i as int].addr as int,
                    mappings@[i as int].len as int,
                ));
            }
            return false;
        }
        i += 1;
    }

    assert(i == mappings.len());
    assert(no_overlap_with_mappings_spec(addr as int, len as int, mappings@)) by {
        assert forall|j: int|
            0 <= j < mappings@.len() implies !range_overlaps_spec(
                addr as int,
                len as int,
                mappings@[j].addr as int,
                mappings@[j].len as int,
            ) by {
            assert(j < i as int);
        }
    };
    true
}

fn linux_range_available(addr: usize, len: usize, mappings: &Vec<LinuxRange>) -> (out: bool)
    ensures
        out == linux_range_available_spec(addr as int, len as int, mappings@),
{
    range_within_window(addr, len, LINUX_MAPPING_BASE, LINUX_MAPPING_LIMIT)
        && no_overlap_with_mappings(addr, len, mappings)
}

fn is_linux_syscall_number(syscall_num: u64) -> (out: bool)
    ensures
        out == linux_syscall_number_spec(syscall_num as int),
{
    smros_is_linux_syscall_number_u64_body!(syscall_num)
}

fn is_zircon_syscall_number(syscall_num: u64) -> (out: bool)
    ensures
        out == zircon_syscall_number_spec(syscall_num as int),
{
    smros_is_zircon_syscall_number_body!(syscall_num, SMROS_ZIRCON_SYSCALL_BASE_U64)
}

fn zircon_syscall_from_raw(syscall_num: u64) -> (out: u32)
    ensures
        out as int == zircon_syscall_from_raw_spec(syscall_num as int),
{
    smros_zircon_syscall_from_raw_body!(syscall_num, SMROS_ZIRCON_SYSCALL_BASE_U64)
}

fn handle_invalid(handle: u32, invalid_handle: u32) -> (out: bool)
    ensures
        out == handle_invalid_spec(handle as int, invalid_handle as int),
{
    smros_syscall_handle_invalid_body!(handle, invalid_handle)
}

fn user_buffer_valid(ptr: usize, len: usize) -> (out: bool)
    ensures
        out == user_buffer_valid_spec(ptr as int, len as int),
{
    smros_syscall_user_buffer_valid_body!(ptr, len)
}

fn channel_buffers_valid(
    bytes_ptr: usize,
    bytes_len: usize,
    handles_ptr: usize,
    handles_len: usize,
) -> (out: bool)
    ensures
        out == channel_buffers_valid_spec(
            bytes_ptr as int,
            bytes_len as int,
            handles_ptr as int,
            handles_len as int,
        ),
{
    smros_syscall_channel_buffers_valid_body!(bytes_ptr, bytes_len, handles_ptr, handles_len)
}

fn signal_update(current: u32, clear_mask: u32, set_mask: u32) -> (out: u32)
    ensures
        out == signal_update_spec(current, clear_mask, set_mask),
{
    smros_syscall_signal_update_body!(current, clear_mask, set_mask)
}

fn signal_mask_allowed(clear_mask: u32, set_mask: u32, allowed_mask: u32) -> (out: bool)
    ensures
        out == signal_mask_allowed_spec(clear_mask, set_mask, allowed_mask),
{
    smros_syscall_signal_mask_allowed_body!(clear_mask, set_mask, allowed_mask)
}

fn user_signal_mask() -> (out: u32)
    ensures
        out == ZX_USER_SIGNAL_MASK,
{
    smros_syscall_user_signal_mask_body!()
}

fn event_signal_mask() -> (out: u32)
    ensures
        out == ZX_EVENT_SIGNAL_MASK,
{
    smros_syscall_event_signal_mask_body!()
}

fn eventpair_signal_mask() -> (out: u32)
    ensures
        out == ZX_EVENTPAIR_SIGNAL_MASK,
{
    smros_syscall_eventpair_signal_mask_body!()
}

fn wait_satisfied(observed: u32, requested: u32) -> (out: bool)
    ensures
        out == wait_satisfied_spec(observed, requested),
{
    smros_syscall_wait_satisfied_body!(observed, requested)
}

fn linux_clock_id_supported(clock_id: usize) -> (out: bool)
    ensures
        out == linux_clock_id_supported_spec(clock_id as int),
{
    smros_linux_clock_id_supported_body!(clock_id)
}

fn linux_signal_valid(signum: usize, max_signal: usize) -> (out: bool)
    ensures
        out == linux_signal_valid_spec(signum as int, max_signal as int),
{
    smros_linux_signal_valid_body!(signum, max_signal)
}

fn linux_signal_action_valid(signum: usize, max_signal: usize) -> (out: bool)
    ensures
        out == linux_signal_action_valid_spec(signum as int, max_signal as int),
{
    smros_linux_signal_action_valid_body!(signum, max_signal)
}

fn linux_sigset_size_valid(size: usize, expected: usize) -> (out: bool)
    ensures
        out == linux_sigset_size_valid_spec(size as int, expected as int),
{
    smros_linux_sigset_size_valid_body!(size, expected)
}

fn linux_ipc_count_valid(count: usize, max_count: usize) -> (out: bool)
    ensures
        out == linux_ipc_count_valid_spec(count as int, max_count as int),
{
    smros_linux_ipc_count_valid_body!(count, max_count)
}

fn linux_ipc_size_valid(size: usize, max_size: usize) -> (out: bool)
    ensures
        out == linux_ipc_size_valid_spec(size as int, max_size as int),
{
    smros_linux_ipc_size_valid_body!(size, max_size)
}

fn linux_msg_size_valid(size: usize, max_size: usize) -> (out: bool)
    ensures
        out == linux_msg_size_valid_spec(size as int, max_size as int),
{
    smros_linux_msg_size_valid_body!(size, max_size)
}

fn linux_socket_domain_supported(
    domain: usize,
    unix: usize,
    local: usize,
    inet: usize,
    netlink: usize,
    packet: usize,
) -> (out: bool)
    ensures
        out == linux_socket_domain_supported_spec(domain, unix, local, inet, netlink, packet),
{
    smros_linux_socket_domain_supported_body!(domain, unix, local, inet, netlink, packet)
}

fn linux_socket_type_supported(
    socket_type: usize,
    mask: usize,
    stream: usize,
    dgram: usize,
    raw: usize,
) -> (out: bool)
    ensures
        out == linux_socket_type_supported_spec(socket_type, mask, stream, dgram, raw),
{
    smros_linux_socket_type_supported_body!(socket_type, mask, stream, dgram, raw)
}

fn linux_socket_domain_type_supported(
    domain: usize,
    kind: usize,
    unix: usize,
    local: usize,
    inet: usize,
    netlink: usize,
    packet: usize,
    stream: usize,
    dgram: usize,
    raw: usize,
) -> (out: bool)
    ensures
        out == linux_socket_domain_type_supported_spec(
            domain, kind, unix, local, inet, netlink, packet, stream, dgram, raw,
        ),
{
    smros_linux_socket_domain_type_supported_body!(
        domain, kind, unix, local, inet, netlink, packet, stream, dgram, raw
    )
}

fn linux_socket_addr_valid(ptr: usize, len: usize) -> (out: bool)
    ensures
        out == linux_socket_addr_valid_spec(ptr as int, len as int),
{
    smros_linux_socket_addr_valid_body!(ptr, len)
}

fn linux_fd_range_valid(first: usize, last: usize) -> (out: bool)
    ensures
        out == linux_fd_range_valid_spec(first as int, last as int),
{
    smros_linux_fd_range_valid_body!(first, last)
}

fn linux_memfd_flags_valid(flags: usize, allowed_mask: usize) -> (out: bool)
    ensures
        out == linux_usize_options_within_mask_spec(flags, allowed_mask),
{
    smros_linux_memfd_flags_valid_body!(flags, allowed_mask)
}

fn linux_getrandom_flags_valid(flags: u32, allowed_mask: u32) -> (out: bool)
    ensures
        out == linux_u32_options_within_mask_spec(flags, allowed_mask),
{
    smros_linux_getrandom_flags_valid_body!(flags, allowed_mask)
}

fn linux_open_access_mode_valid(
    flags: usize,
    access_mask: usize,
    read_only: usize,
    write_only: usize,
    read_write: usize,
) -> (out: bool)
    ensures
        out == linux_open_access_mode_valid_spec(
            flags, access_mask, read_only, write_only, read_write,
        ),
{
    smros_linux_open_access_mode_valid_body!(
        flags,
        access_mask,
        read_only,
        write_only,
        read_write
    )
}

fn linux_open_flags_valid(flags: usize, allowed_mask: usize) -> (out: bool)
    ensures
        out == linux_usize_options_within_mask_spec(flags, allowed_mask),
{
    smros_linux_open_flags_valid_body!(flags, allowed_mask)
}

fn linux_open_is_directory(flags: usize, directory_flag: usize) -> (out: bool)
    ensures
        out == linux_open_is_directory_spec(flags, directory_flag),
{
    smros_linux_open_is_directory_body!(flags, directory_flag)
}

fn linux_fd_target_valid(fd: usize, stdio_max: usize) -> (out: bool)
    ensures
        out == linux_fd_target_valid_spec(fd as int, stdio_max as int),
{
    smros_linux_fd_target_valid_body!(fd, stdio_max)
}

fn linux_pipe_flags_valid(flags: usize, allowed_mask: usize) -> (out: bool)
    ensures
        out == linux_usize_options_within_mask_spec(flags, allowed_mask),
{
    smros_linux_pipe_flags_valid_body!(flags, allowed_mask)
}

fn linux_dup3_args_valid(old_fd: usize, new_fd: usize) -> (out: bool)
    ensures
        out == linux_dup3_args_valid_spec(old_fd as int, new_fd as int),
{
    smros_linux_dup3_args_valid_body!(old_fd, new_fd)
}

fn linux_fcntl_cmd_supported(
    cmd: usize,
    dupfd: usize,
    getfd: usize,
    setfd: usize,
    getfl: usize,
    setfl: usize,
    dupfd_cloexec: usize,
) -> (out: bool)
    ensures
        out == linux_fcntl_cmd_supported_spec(
            cmd as int,
            dupfd as int,
            getfd as int,
            setfd as int,
            getfl as int,
            setfl as int,
            dupfd_cloexec as int,
        ),
{
    smros_linux_fcntl_cmd_supported_body!(cmd, dupfd, getfd, setfd, getfl, setfl, dupfd_cloexec)
}

fn linux_fcntl_flags_valid(flags: usize, allowed_mask: usize) -> (out: bool)
    ensures
        out == linux_usize_options_within_mask_spec(flags, allowed_mask),
{
    smros_linux_fcntl_flags_valid_body!(flags, allowed_mask)
}

fn linux_path_mode_valid(mode: usize, allowed_mask: usize) -> (out: bool)
    ensures
        out == linux_usize_options_within_mask_spec(mode, allowed_mask),
{
    smros_linux_path_mode_valid_body!(mode, allowed_mask)
}

fn linux_unlink_flags_valid(flags: usize, allowed_mask: usize) -> (out: bool)
    ensures
        out == linux_usize_options_within_mask_spec(flags, allowed_mask),
{
    smros_linux_unlink_flags_valid_body!(flags, allowed_mask)
}

fn linux_rename_flags_valid(flags: usize, allowed_mask: usize) -> (out: bool)
    ensures
        out == linux_usize_options_within_mask_spec(flags, allowed_mask),
{
    smros_linux_rename_flags_valid_body!(flags, allowed_mask)
}

fn linux_stat_flags_valid(flags: usize, allowed_mask: usize) -> (out: bool)
    ensures
        out == linux_usize_options_within_mask_spec(flags, allowed_mask),
{
    smros_linux_stat_flags_valid_body!(flags, allowed_mask)
}

fn linux_stat_mask_valid(mask: usize, allowed_mask: usize) -> (out: bool)
    ensures
        out == linux_usize_options_within_mask_spec(mask, allowed_mask),
{
    smros_linux_stat_mask_valid_body!(mask, allowed_mask)
}

fn linux_lseek_whence_valid(whence: usize, max_whence: usize) -> (out: bool)
    ensures
        out == linux_lseek_whence_valid_spec(whence as int, max_whence as int),
{
    smros_linux_lseek_whence_valid_body!(whence, max_whence)
}

fn linux_iov_count_valid(count: usize, max_count: usize) -> (out: bool)
    ensures
        out == linux_iov_count_valid_spec(count as int, max_count as int),
{
    smros_linux_iov_count_valid_body!(count, max_count)
}

fn linux_iov_bytes_valid(count: usize, elem_size: usize, max_count: usize) -> (out: bool)
    ensures
        out == linux_iov_bytes_valid_spec(count as int, elem_size as int, max_count as int),
{
    smros_linux_iov_bytes_valid_body!(count, elem_size, max_count)
}

fn linux_poll_count_valid(count: usize, max_count: usize) -> (out: bool)
    ensures
        out == linux_poll_count_valid_spec(count as int, max_count as int),
{
    smros_linux_poll_count_valid_body!(count, max_count)
}

fn linux_poll_events_valid(events: i16, allowed_mask: i16) -> (out: bool)
    ensures
        out == linux_i16_options_within_mask_spec(events, allowed_mask),
{
    smros_linux_poll_events_valid_body!(events, allowed_mask)
}

fn linux_copy_flags_valid(flags: usize, allowed_mask: usize) -> (out: bool)
    ensures
        out == linux_usize_options_within_mask_spec(flags, allowed_mask),
{
    smros_linux_copy_flags_valid_body!(flags, allowed_mask)
}

fn zircon_clock_id_supported(clock_id: u32) -> (out: bool)
    ensures
        out == zircon_clock_id_supported_spec(clock_id as int),
{
    smros_zircon_clock_id_supported_body!(clock_id)
}

fn zircon_clock_create_options_valid(options: u32, allowed_mask: u32) -> (out: bool)
    ensures
        out == u32_options_within_mask_spec(options, allowed_mask),
{
    smros_zircon_clock_create_options_valid_body!(options, allowed_mask)
}

fn zircon_clock_update_options_valid(options: u64, allowed_mask: u64) -> (out: bool)
    ensures
        out == u64_options_within_mask_spec(options, allowed_mask),
{
    smros_zircon_clock_update_options_valid_body!(options, allowed_mask)
}

fn zircon_timer_options_valid(options: u32, allowed_mask: u32) -> (out: bool)
    ensures
        out == u32_options_within_mask_spec(options, allowed_mask),
{
    smros_zircon_timer_options_valid_body!(options, allowed_mask)
}

fn zircon_timer_deadline_expired(deadline: u64, now: u64) -> (out: bool)
    ensures
        out == zircon_timer_deadline_expired_spec(deadline, now),
{
    smros_zircon_timer_deadline_expired_body!(deadline, now)
}

fn zircon_debuglog_create_options_valid(options: u32, allowed_mask: u32) -> (out: bool)
    ensures
        out == u32_options_within_mask_spec(options, allowed_mask),
{
    smros_zircon_debuglog_create_options_valid_body!(options, allowed_mask)
}

fn zircon_debuglog_io_options_valid(options: u32, allowed_mask: u32) -> (out: bool)
    ensures
        out == u32_options_within_mask_spec(options, allowed_mask),
{
    smros_zircon_debuglog_io_options_valid_body!(options, allowed_mask)
}

fn zircon_system_event_kind_valid(kind: u32, max_kind: u32) -> (out: bool)
    ensures
        out == zircon_system_event_kind_valid_spec(kind as int, max_kind as int),
{
    smros_zircon_system_event_kind_valid_body!(kind, max_kind)
}

fn zircon_exception_channel_options_valid(options: u32, allowed_mask: u32) -> (out: bool)
    ensures
        out == u32_options_within_mask_spec(options, allowed_mask),
{
    smros_zircon_exception_channel_options_valid_body!(options, allowed_mask)
}

fn zircon_hypervisor_options_valid(options: u32, allowed_mask: u32) -> (out: bool)
    ensures
        out == u32_options_within_mask_spec(options, allowed_mask),
{
    smros_zircon_hypervisor_options_valid_body!(options, allowed_mask)
}

fn zircon_guest_trap_kind_valid(kind: u32, max_kind: u32) -> (out: bool)
    ensures
        out == zircon_system_event_kind_valid_spec(kind as int, max_kind as int),
{
    smros_zircon_guest_trap_kind_valid_body!(kind, max_kind)
}

fn zircon_guest_trap_is_bell(kind: u32, bell: u32) -> (out: bool)
    ensures
        out == (kind == bell),
{
    smros_zircon_guest_trap_is_bell_body!(kind, bell)
}

fn zircon_guest_trap_is_mem(kind: u32, mem: u32) -> (out: bool)
    ensures
        out == (kind == mem),
{
    smros_zircon_guest_trap_is_mem_body!(kind, mem)
}

fn zircon_guest_trap_range_valid(addr: u64, size: u64, limit: u64) -> (out: bool)
    ensures
        out == zircon_guest_trap_range_valid_spec(addr as int, size as int, limit as int),
{
    smros_zircon_guest_trap_range_valid_body!(addr, size, limit)
}

fn zircon_guest_trap_alignment_valid(
    kind: u32,
    addr: u64,
    size: u64,
    bell: u32,
    mem: u32,
    page_size: u64,
) -> (out: bool)
    ensures
        out == zircon_guest_trap_alignment_valid_spec(
            kind as int,
            addr as int,
            size as int,
            bell as int,
            mem as int,
            page_size as int,
        ),
{
    smros_zircon_guest_trap_alignment_valid_body!(kind, addr, size, bell, mem, page_size)
}

fn zircon_vcpu_entry_valid(entry: u64, alignment: u64) -> (out: bool)
    ensures
        out == zircon_vcpu_entry_valid_spec(entry as int, alignment as int),
{
    smros_zircon_vcpu_entry_valid_body!(entry, alignment)
}

fn zircon_vcpu_interrupt_vector_valid(vector: u32, max_vector: u32) -> (out: bool)
    ensures
        out == zircon_vcpu_interrupt_vector_valid_spec(vector as int, max_vector as int),
{
    smros_zircon_vcpu_interrupt_vector_valid_body!(vector, max_vector)
}

fn zircon_vcpu_read_state_args_valid(
    kind: u32,
    buffer_size: usize,
    state_kind: u32,
    state_size: usize,
) -> (out: bool)
    ensures
        out == zircon_vcpu_read_state_args_valid_spec(
            kind as int,
            buffer_size as int,
            state_kind as int,
            state_size as int,
        ),
{
    smros_zircon_vcpu_read_state_args_valid_body!(kind, buffer_size, state_kind, state_size)
}

fn zircon_vcpu_write_state_args_valid(
    kind: u32,
    buffer_size: usize,
    state_kind: u32,
    state_size: usize,
    io_kind: u32,
    io_size: usize,
) -> (out: bool)
    ensures
        out == zircon_vcpu_write_state_args_valid_spec(
            kind as int,
            buffer_size as int,
            state_kind as int,
            state_size as int,
            io_kind as int,
            io_size as int,
        ),
{
    smros_zircon_vcpu_write_state_args_valid_body!(
        kind,
        buffer_size,
        state_kind,
        state_size,
        io_kind,
        io_size
    )
}

fn linux_syscall_interface_known(syscall_num: u32) -> (out: bool)
    ensures
        out == linux_syscall_interface_known_spec(syscall_num as int),
{
    smros_linux_syscall_interface_known_body!(syscall_num)
}

fn zircon_syscall_interface_known(syscall_num: u32) -> (out: bool)
    ensures
        out == zircon_syscall_interface_known_spec(syscall_num as int),
{
    smros_zircon_syscall_interface_known_body!(syscall_num)
}

fn linux_errno_code_to_u64(errno: u32) -> (out: u64)
    ensures
        out as int == linux_errno_return_spec(errno as int),
{
    smros_linux_errno_to_u64_body!(errno)
}

fn linux_sys_result_to_u64_model(is_ok: bool, value: usize, errno: u32) -> (out: u64)
    ensures
        out as int == linux_sys_result_return_spec(is_ok, value as int, errno as int),
{
    if is_ok {
        value as u64
    } else {
        linux_errno_code_to_u64(errno)
    }
}

fn syscall_num_from_regs(regs: &Vec<u64>) -> (out: u64)
    requires
        regs.len() == SMROS_SAVED_REG_COUNT,
    ensures
        out == regs@[SMROS_SYSCALL_NUMBER_REG_INDEX as int],
{
    assert(SMROS_SYSCALL_NUMBER_REG_INDEX < regs.len());
    smros_syscall_num_from_regs_body!(regs)
}

fn syscall_arg_from_reg(regs: &Vec<u64>, idx: usize) -> (out: usize)
    requires
        regs.len() == SMROS_SAVED_REG_COUNT,
        idx < SMROS_SYSCALL_ARG_COUNT_ZIRCON,
        regs@[idx as int] as int <= usize::MAX as int,
    ensures
        out as int == saved_reg_arg_spec(regs@, idx as int),
{
    assert(SMROS_SYSCALL_ARG_COUNT_ZIRCON <= SMROS_SAVED_REG_COUNT);
    assert(idx < regs.len());
    smros_syscall_arg_from_reg_body!(regs, idx)
}

fn syscall_arg_from_u64(arg: u64) -> (out: usize)
    requires
        arg as int <= usize::MAX as int,
    ensures
        out as int == arg as int,
{
    smros_syscall_arg_from_u64_body!(arg)
}

proof fn checked_end_spec_returns_sum_when_it_fits(addr: int, len: int)
    requires
        0 <= addr,
        0 <= len,
        addr <= usize::MAX as int - len,
    ensures
        checked_end_spec(addr, len) == Some(addr + len),
{
}

proof fn checked_end_spec_rejects_overflow(addr: int, len: int)
    requires
        0 <= addr,
        0 <= len,
        addr > usize::MAX as int - len,
    ensures
        checked_end_spec(addr, len) == Option::<int>::None,
{
}

proof fn range_overlaps_is_symmetric(start_a: int, len_a: int, start_b: int, len_b: int)
    requires
        0 <= start_a,
        0 <= len_a,
        0 <= start_b,
        0 <= len_b,
    ensures
        range_overlaps_spec(start_a, len_a, start_b, len_b) == range_overlaps_spec(start_b, len_b, start_a, len_a),
{
}

proof fn overflowing_ranges_are_not_treated_as_window_members(addr: int, len: int, base: int, limit: int)
    requires
        0 <= addr,
        0 <= len,
        addr > usize::MAX as int - len,
    ensures
        !range_within_window_spec(addr, len, base, limit),
{
}

proof fn fixed_linux_mmap_request_stays_inside_linux_window(addr: int, len: int)
    requires
        fixed_linux_mmap_request_ok_spec(addr, len),
    ensures
        addr >= LINUX_MAPPING_BASE as int,
{
}

proof fn range_end_before_start_means_no_overlap(
    start_a: int,
    len_a: int,
    start_b: int,
    len_b: int,
)
    requires
        0 <= start_a,
        0 <= len_a,
        0 <= start_b,
        0 <= len_b,
        range_ends_before_spec(start_a, len_a, start_b),
    ensures
        !range_overlaps_spec(start_a, len_a, start_b, len_b),
{
}

proof fn range_start_after_end_means_no_overlap(
    start_a: int,
    len_a: int,
    start_b: int,
    len_b: int,
)
    requires
        0 <= start_a,
        0 <= len_a,
        0 <= start_b,
        0 <= len_b,
        range_ends_before_spec(start_b, len_b, start_a),
    ensures
        !range_overlaps_spec(start_a, len_a, start_b, len_b),
{
}

proof fn syscall_bridge_layout_matches_exception_frame()
    ensures
        SMROS_SAVED_REG_FRAME_BYTES == SMROS_SAVED_REG_COUNT * 8,
        SMROS_SAVED_REG_WORDS == SMROS_SAVED_REG_COUNT,
        SMROS_SYSCALL_ARG_COUNT_LINUX == 6,
        SMROS_SYSCALL_ARG_COUNT_ZIRCON == 8,
        SMROS_SYSCALL_ARG_COUNT_LINUX <= SMROS_SYSCALL_ARG_COUNT_ZIRCON,
        SMROS_SYSCALL_ARG_COUNT_ZIRCON <= SMROS_SYSCALL_NUMBER_REG_INDEX,
        SMROS_SYSCALL_NUMBER_REG_INDEX < SMROS_SAVED_REG_COUNT,
{
}

proof fn syscall_bridge_route_smoke() {
    assert(linux_syscall_number_spec(0));
    assert(linux_syscall_number_spec(999));
    assert(!linux_syscall_number_spec(1000));
    assert(!linux_syscall_number_spec(u64::MAX as int));

    assert(linux_errno_return_spec(0) == 0);
    assert(linux_errno_return_spec(38) == u64::MAX as int + 1 - 38);
}

proof fn syscall_zircon_logic_smoke() {
    assert(!zircon_syscall_number_spec(999));
    assert(zircon_syscall_number_spec(1000));
    assert(zircon_syscall_number_spec(1000 + u32::MAX as int));
    assert(!zircon_syscall_number_spec(1001 + u32::MAX as int));

    assert(zircon_syscall_from_raw_spec(1000) == 0);
    assert(zircon_syscall_from_raw_spec(1001) == 1);
    assert(zircon_syscall_from_raw_spec(999) == u32::MAX as int);

    assert(handle_invalid_spec(0, SMROS_INVALID_HANDLE_U32 as int));
    assert(handle_invalid_spec(
        SMROS_INVALID_HANDLE_U32 as int,
        SMROS_INVALID_HANDLE_U32 as int,
    ));
    assert(!handle_invalid_spec(1, SMROS_INVALID_HANDLE_U32 as int));

    assert(user_buffer_valid_spec(0, 0));
    assert(!user_buffer_valid_spec(0, 1));
    assert(user_buffer_valid_spec(0x1000, 1));
    assert(channel_buffers_valid_spec(0x1000, 8, 0, 0));
    assert(!channel_buffers_valid_spec(0x1000, 8, 0, 1));

    assert(wait_satisfied_spec(0, 0));

    assert(signal_mask_allowed_spec(1u32 << 24, 1u32 << 25, ZX_USER_SIGNAL_MASK)) by(bit_vector);
    assert(!signal_mask_allowed_spec(1u32, 0, ZX_USER_SIGNAL_MASK)) by(bit_vector);
    assert(signal_mask_allowed_spec(
        1u32 << 4,
        1u32 << 24,
        ZX_EVENTPAIR_SIGNAL_MASK,
    )) by(bit_vector);

    assert(linux_clock_id_supported_spec(0));
    assert(linux_clock_id_supported_spec(1));
    assert(!linux_clock_id_supported_spec(2));

    assert(linux_signal_valid_spec(0, LINUX_MAX_SIGNAL as int));
    assert(linux_signal_action_valid_spec(1, LINUX_MAX_SIGNAL as int));
    assert(!linux_signal_action_valid_spec(0, LINUX_MAX_SIGNAL as int));
    assert(!linux_signal_valid_spec(65, LINUX_MAX_SIGNAL as int));
    assert(linux_sigset_size_valid_spec(LINUX_SIGSET_SIZE as int, LINUX_SIGSET_SIZE as int));
    assert(!linux_sigset_size_valid_spec(16, LINUX_SIGSET_SIZE as int));
    assert(linux_ipc_count_valid_spec(1, LINUX_MAX_SEMAPHORES as int));
    assert(!linux_ipc_count_valid_spec(0, LINUX_MAX_SEMAPHORES as int));
    assert(linux_ipc_size_valid_spec(PAGE_SIZE as int, LINUX_MAX_IPC_BYTES as int));
    assert(!linux_ipc_size_valid_spec(0, LINUX_MAX_IPC_BYTES as int));
    assert(linux_msg_size_valid_spec(0, LINUX_MAX_MSG_BYTES as int));
    assert(!linux_msg_size_valid_spec(8193, LINUX_MAX_MSG_BYTES as int));
    assert(linux_socket_domain_supported_spec(
        LINUX_AF_INET,
        LINUX_AF_UNIX,
        LINUX_AF_LOCAL,
        LINUX_AF_INET,
        LINUX_AF_NETLINK,
        LINUX_AF_PACKET,
    ));
    assert(!linux_socket_domain_supported_spec(
        99,
        LINUX_AF_UNIX,
        LINUX_AF_LOCAL,
        LINUX_AF_INET,
        LINUX_AF_NETLINK,
        LINUX_AF_PACKET,
    ));
    assert(linux_socket_type_supported_spec(
        LINUX_SOCK_STREAM,
        LINUX_SOCK_TYPE_MASK,
        LINUX_SOCK_STREAM,
        LINUX_SOCK_DGRAM,
        LINUX_SOCK_RAW,
    )) by(bit_vector);
    assert(linux_socket_domain_type_supported_spec(
        LINUX_AF_UNIX,
        LINUX_SOCK_DGRAM,
        LINUX_AF_UNIX,
        LINUX_AF_LOCAL,
        LINUX_AF_INET,
        LINUX_AF_NETLINK,
        LINUX_AF_PACKET,
        LINUX_SOCK_STREAM,
        LINUX_SOCK_DGRAM,
        LINUX_SOCK_RAW,
    ));
    assert(!linux_socket_domain_type_supported_spec(
        LINUX_AF_UNIX,
        LINUX_SOCK_RAW,
        LINUX_AF_UNIX,
        LINUX_AF_LOCAL,
        LINUX_AF_INET,
        LINUX_AF_NETLINK,
        LINUX_AF_PACKET,
        LINUX_SOCK_STREAM,
        LINUX_SOCK_DGRAM,
        LINUX_SOCK_RAW,
    ));
    assert(linux_socket_addr_valid_spec(0, 0));
    assert(!linux_socket_addr_valid_spec(0, 1));
    assert(linux_fd_range_valid_spec(3, 3));
    assert(!linux_fd_range_valid_spec(4, 3));
    assert(linux_usize_options_within_mask_spec(1, LINUX_MEMFD_ALLOWED_FLAGS)) by(bit_vector);
    assert(!linux_usize_options_within_mask_spec(8, LINUX_MEMFD_ALLOWED_FLAGS)) by(bit_vector);
    assert(linux_u32_options_within_mask_spec(1, LINUX_GETRANDOM_ALLOWED_FLAGS)) by(bit_vector);
    assert(!linux_u32_options_within_mask_spec(4, LINUX_GETRANDOM_ALLOWED_FLAGS)) by(bit_vector);

    assert(zircon_clock_id_supported_spec(0));
    assert(zircon_clock_id_supported_spec(1));
    assert(!zircon_clock_id_supported_spec(2));

    assert(u32_options_within_mask_spec(0, ZX_CLOCK_OPT_AUTO_START)) by(bit_vector);
    assert(u32_options_within_mask_spec(
        ZX_CLOCK_OPT_AUTO_START,
        ZX_CLOCK_OPT_AUTO_START,
    )) by(bit_vector);
    assert(!u32_options_within_mask_spec(2, ZX_CLOCK_OPT_AUTO_START)) by(bit_vector);
    assert(u64_options_within_mask_spec(
        ZX_CLOCK_UPDATE_OPTIONS_MASK,
        ZX_CLOCK_UPDATE_OPTIONS_MASK,
    )) by(bit_vector);
    assert(!u64_options_within_mask_spec(4, ZX_CLOCK_UPDATE_OPTIONS_MASK)) by(bit_vector);

    assert(u32_options_within_mask_spec(0, ZX_TIMER_OPTIONS_MASK)) by(bit_vector);
    assert(!u32_options_within_mask_spec(1, ZX_TIMER_OPTIONS_MASK)) by(bit_vector);
    assert(zircon_timer_deadline_expired_spec(5, 5));
    assert(zircon_timer_deadline_expired_spec(4, 5));
    assert(!zircon_timer_deadline_expired_spec(6, 5));

    assert(u32_options_within_mask_spec(0, ZX_DEBUGLOG_CREATE_OPTIONS_MASK)) by(bit_vector);
    assert(!u32_options_within_mask_spec(1, ZX_DEBUGLOG_CREATE_OPTIONS_MASK)) by(bit_vector);
    assert(u32_options_within_mask_spec(0, ZX_DEBUGLOG_OPTIONS_MASK)) by(bit_vector);
    assert(!u32_options_within_mask_spec(1, ZX_DEBUGLOG_OPTIONS_MASK)) by(bit_vector);

    assert(zircon_system_event_kind_valid_spec(0, ZX_SYSTEM_EVENT_KIND_MAX as int));
    assert(zircon_system_event_kind_valid_spec(3, ZX_SYSTEM_EVENT_KIND_MAX as int));
    assert(!zircon_system_event_kind_valid_spec(4, ZX_SYSTEM_EVENT_KIND_MAX as int));
    assert(u32_options_within_mask_spec(
        ZX_EXCEPTION_CHANNEL_DEBUGGER,
        ZX_EXCEPTION_CHANNEL_OPTIONS_MASK,
    )) by(bit_vector);
    assert(!u32_options_within_mask_spec(2, ZX_EXCEPTION_CHANNEL_OPTIONS_MASK)) by(bit_vector);

    assert(u32_options_within_mask_spec(0, ZX_HYPERVISOR_OPTIONS_MASK)) by(bit_vector);
    assert(!u32_options_within_mask_spec(1, ZX_HYPERVISOR_OPTIONS_MASK)) by(bit_vector);
    assert(zircon_system_event_kind_valid_spec(
        ZX_GUEST_TRAP_BELL as int,
        ZX_GUEST_TRAP_KIND_MAX as int,
    ));
    assert(zircon_system_event_kind_valid_spec(
        ZX_GUEST_TRAP_IO as int,
        ZX_GUEST_TRAP_KIND_MAX as int,
    ));
    assert(!zircon_system_event_kind_valid_spec(
        3,
        ZX_GUEST_TRAP_KIND_MAX as int,
    ));
    assert(zircon_guest_trap_range_valid_spec(
        0x1000,
        PAGE_SIZE as int,
        ZX_GUEST_PHYS_LIMIT as int,
    ));
    assert(!zircon_guest_trap_range_valid_spec(
        0x1000,
        0,
        ZX_GUEST_PHYS_LIMIT as int,
    ));
    assert(!zircon_guest_trap_range_valid_spec(
        ZX_GUEST_PHYS_LIMIT as int,
        PAGE_SIZE as int,
        ZX_GUEST_PHYS_LIMIT as int,
    ));
    assert(zircon_guest_trap_alignment_valid_spec(
        ZX_GUEST_TRAP_MEM as int,
        0x2000,
        PAGE_SIZE as int,
        ZX_GUEST_TRAP_BELL as int,
        ZX_GUEST_TRAP_MEM as int,
        PAGE_SIZE as int,
    ));
    assert(!zircon_guest_trap_alignment_valid_spec(
        ZX_GUEST_TRAP_MEM as int,
        0x2001,
        PAGE_SIZE as int,
        ZX_GUEST_TRAP_BELL as int,
        ZX_GUEST_TRAP_MEM as int,
        PAGE_SIZE as int,
    ));
    assert(zircon_guest_trap_alignment_valid_spec(
        ZX_GUEST_TRAP_IO as int,
        3,
        7,
        ZX_GUEST_TRAP_BELL as int,
        ZX_GUEST_TRAP_MEM as int,
        PAGE_SIZE as int,
    ));
    assert(zircon_vcpu_entry_valid_spec(0x4000, ZX_VCPU_ENTRY_ALIGNMENT as int));
    assert(!zircon_vcpu_entry_valid_spec(0x4001, ZX_VCPU_ENTRY_ALIGNMENT as int));
    assert(zircon_vcpu_interrupt_vector_valid_spec(
        1023,
        ZX_VCPU_INTERRUPT_VECTOR_MAX as int,
    ));
    assert(!zircon_vcpu_interrupt_vector_valid_spec(
        1024,
        ZX_VCPU_INTERRUPT_VECTOR_MAX as int,
    ));
    assert(zircon_vcpu_read_state_args_valid_spec(
        ZX_VCPU_STATE as int,
        ZX_VCPU_STATE_SIZE as int,
        ZX_VCPU_STATE as int,
        ZX_VCPU_STATE_SIZE as int,
    ));
    assert(!zircon_vcpu_read_state_args_valid_spec(
        ZX_VCPU_IO as int,
        ZX_VCPU_IO_SIZE as int,
        ZX_VCPU_STATE as int,
        ZX_VCPU_STATE_SIZE as int,
    ));
    assert(zircon_vcpu_write_state_args_valid_spec(
        ZX_VCPU_IO as int,
        ZX_VCPU_IO_SIZE as int,
        ZX_VCPU_STATE as int,
        ZX_VCPU_STATE_SIZE as int,
        ZX_VCPU_IO as int,
        ZX_VCPU_IO_SIZE as int,
    ));

    assert(linux_syscall_interface_known_spec(0));
    assert(linux_syscall_interface_known_spec(446));
    assert(linux_syscall_interface_known_spec(600));
    assert(!linux_syscall_interface_known_spec(447));
    assert(!linux_syscall_interface_known_spec(601));

    assert(zircon_syscall_interface_known_spec(0));
    assert(zircon_syscall_interface_known_spec(154));
    assert(zircon_syscall_interface_known_spec(183));
    assert(zircon_syscall_interface_known_spec(211));
    assert(!zircon_syscall_interface_known_spec(155));
    assert(!zircon_syscall_interface_known_spec(212));
}

fn syscall_signal_mask_exec_smoke() {
    let user_mask = user_signal_mask();
    let event_mask = event_signal_mask();
    let eventpair_mask = eventpair_signal_mask();
    let user_allowed = signal_mask_allowed(1u32 << 24, 1u32 << 25, user_mask);
    let kernel_rejected = signal_mask_allowed(1u32, 0, user_mask);
    let eventpair_allowed = signal_mask_allowed(1u32 << 4, 1u32 << 24, eventpair_mask);

    assert(user_mask == ZX_USER_SIGNAL_MASK);
    assert(event_mask == ZX_EVENT_SIGNAL_MASK);
    assert(eventpair_mask == ZX_EVENTPAIR_SIGNAL_MASK);
    assert(user_allowed == signal_mask_allowed_spec(1u32 << 24, 1u32 << 25, user_mask));
    assert(kernel_rejected == signal_mask_allowed_spec(1u32, 0, user_mask));
    assert(eventpair_allowed == signal_mask_allowed_spec(1u32 << 4, 1u32 << 24, eventpair_mask));
}

fn syscall_time_debug_system_exception_exec_smoke() {
    let clock_auto_start =
        zircon_clock_create_options_valid(ZX_CLOCK_OPT_AUTO_START, ZX_CLOCK_OPT_AUTO_START);
    let clock_bad = zircon_clock_create_options_valid(2, ZX_CLOCK_OPT_AUTO_START);
    let clock_update = zircon_clock_update_options_valid(
        ZX_CLOCK_UPDATE_OPTIONS_MASK,
        ZX_CLOCK_UPDATE_OPTIONS_MASK,
    );
    let timer_zero = zircon_timer_options_valid(0, ZX_TIMER_OPTIONS_MASK);
    let timer_bad = zircon_timer_options_valid(1, ZX_TIMER_OPTIONS_MASK);
    let timer_expired = zircon_timer_deadline_expired(10, 10);
    let debuglog_zero = zircon_debuglog_create_options_valid(0, ZX_DEBUGLOG_CREATE_OPTIONS_MASK);
    let debuglog_bad = zircon_debuglog_io_options_valid(1, ZX_DEBUGLOG_OPTIONS_MASK);
    let event_ok = zircon_system_event_kind_valid(3, ZX_SYSTEM_EVENT_KIND_MAX);
    let event_bad = zircon_system_event_kind_valid(4, ZX_SYSTEM_EVENT_KIND_MAX);
    let exception_debugger = zircon_exception_channel_options_valid(
        ZX_EXCEPTION_CHANNEL_DEBUGGER,
        ZX_EXCEPTION_CHANNEL_OPTIONS_MASK,
    );

    assert(clock_auto_start == u32_options_within_mask_spec(
        ZX_CLOCK_OPT_AUTO_START,
        ZX_CLOCK_OPT_AUTO_START,
    ));
    assert(clock_bad == u32_options_within_mask_spec(2, ZX_CLOCK_OPT_AUTO_START));
    assert(clock_update == u64_options_within_mask_spec(
        ZX_CLOCK_UPDATE_OPTIONS_MASK,
        ZX_CLOCK_UPDATE_OPTIONS_MASK,
    ));
    assert(timer_zero == u32_options_within_mask_spec(0, ZX_TIMER_OPTIONS_MASK));
    assert(timer_bad == u32_options_within_mask_spec(1, ZX_TIMER_OPTIONS_MASK));
    assert(timer_expired == zircon_timer_deadline_expired_spec(10, 10));
    assert(debuglog_zero == u32_options_within_mask_spec(0, ZX_DEBUGLOG_CREATE_OPTIONS_MASK));
    assert(debuglog_bad == u32_options_within_mask_spec(1, ZX_DEBUGLOG_OPTIONS_MASK));
    assert(event_ok == zircon_system_event_kind_valid_spec(
        3,
        ZX_SYSTEM_EVENT_KIND_MAX as int,
    ));
    assert(event_bad == zircon_system_event_kind_valid_spec(
        4,
        ZX_SYSTEM_EVENT_KIND_MAX as int,
    ));
    assert(exception_debugger == u32_options_within_mask_spec(
        ZX_EXCEPTION_CHANNEL_DEBUGGER,
        ZX_EXCEPTION_CHANNEL_OPTIONS_MASK,
    ));
}

fn syscall_linux_signal_ipc_misc_net_exec_smoke() {
    let signal_zero_valid = linux_signal_valid(0, LINUX_MAX_SIGNAL);
    let signal_zero_action = linux_signal_action_valid(0, LINUX_MAX_SIGNAL);
    let signal_term_action = linux_signal_action_valid(15, LINUX_MAX_SIGNAL);
    let sigset_ok = linux_sigset_size_valid(LINUX_SIGSET_SIZE, LINUX_SIGSET_SIZE);
    let sigset_bad = linux_sigset_size_valid(LINUX_SIGSET_SIZE + 1, LINUX_SIGSET_SIZE);
    let sem_count_ok = linux_ipc_count_valid(2, LINUX_MAX_SEMAPHORES);
    let sem_count_bad = linux_ipc_count_valid(0, LINUX_MAX_SEMAPHORES);
    let shm_size_ok = linux_ipc_size_valid(PAGE_SIZE, LINUX_MAX_IPC_BYTES);
    let shm_size_bad = linux_ipc_size_valid(0, LINUX_MAX_IPC_BYTES);
    let msg_size_ok = linux_msg_size_valid(LINUX_MAX_MSG_BYTES, LINUX_MAX_MSG_BYTES);
    let msg_size_bad = linux_msg_size_valid(LINUX_MAX_MSG_BYTES + 1, LINUX_MAX_MSG_BYTES);
    let inet_domain = linux_socket_domain_supported(
        LINUX_AF_INET,
        LINUX_AF_UNIX,
        LINUX_AF_LOCAL,
        LINUX_AF_INET,
        LINUX_AF_NETLINK,
        LINUX_AF_PACKET,
    );
    let stream_type = linux_socket_type_supported(
        LINUX_SOCK_STREAM,
        LINUX_SOCK_TYPE_MASK,
        LINUX_SOCK_STREAM,
        LINUX_SOCK_DGRAM,
        LINUX_SOCK_RAW,
    );
    let unix_stream = linux_socket_domain_type_supported(
        LINUX_AF_UNIX,
        LINUX_SOCK_STREAM,
        LINUX_AF_UNIX,
        LINUX_AF_LOCAL,
        LINUX_AF_INET,
        LINUX_AF_NETLINK,
        LINUX_AF_PACKET,
        LINUX_SOCK_STREAM,
        LINUX_SOCK_DGRAM,
        LINUX_SOCK_RAW,
    );
    let unix_raw = linux_socket_domain_type_supported(
        LINUX_AF_UNIX,
        LINUX_SOCK_RAW,
        LINUX_AF_UNIX,
        LINUX_AF_LOCAL,
        LINUX_AF_INET,
        LINUX_AF_NETLINK,
        LINUX_AF_PACKET,
        LINUX_SOCK_STREAM,
        LINUX_SOCK_DGRAM,
        LINUX_SOCK_RAW,
    );
    let socket_flags_ok = linux_memfd_flags_valid(
        LINUX_SOCK_STREAM | LINUX_SOCK_CLOEXEC,
        LINUX_SOCK_ALLOWED_FLAGS,
    );
    let socket_flags_bad = linux_memfd_flags_valid(
        LINUX_SOCK_STREAM | 0x4000,
        LINUX_SOCK_ALLOWED_FLAGS,
    );
    let socket_addr_zero = linux_socket_addr_valid(0, 0);
    let socket_addr_bad = linux_socket_addr_valid(0, 16);
    let close_range_ok = linux_fd_range_valid(3, 4);
    let close_range_bad = linux_fd_range_valid(5, 4);
    let memfd_flags_ok = linux_memfd_flags_valid(1, LINUX_MEMFD_ALLOWED_FLAGS);
    let memfd_flags_bad = linux_memfd_flags_valid(8, LINUX_MEMFD_ALLOWED_FLAGS);
    let getrandom_flags_ok = linux_getrandom_flags_valid(1, LINUX_GETRANDOM_ALLOWED_FLAGS);
    let getrandom_flags_bad = linux_getrandom_flags_valid(4, LINUX_GETRANDOM_ALLOWED_FLAGS);

    assert(signal_zero_valid == linux_signal_valid_spec(0, LINUX_MAX_SIGNAL as int));
    assert(signal_zero_action == linux_signal_action_valid_spec(0, LINUX_MAX_SIGNAL as int));
    assert(signal_term_action == linux_signal_action_valid_spec(15, LINUX_MAX_SIGNAL as int));
    assert(sigset_ok == linux_sigset_size_valid_spec(
        LINUX_SIGSET_SIZE as int,
        LINUX_SIGSET_SIZE as int,
    ));
    assert(sigset_bad == linux_sigset_size_valid_spec(
        (LINUX_SIGSET_SIZE + 1) as int,
        LINUX_SIGSET_SIZE as int,
    ));
    assert(sem_count_ok == linux_ipc_count_valid_spec(2, LINUX_MAX_SEMAPHORES as int));
    assert(sem_count_bad == linux_ipc_count_valid_spec(0, LINUX_MAX_SEMAPHORES as int));
    assert(shm_size_ok == linux_ipc_size_valid_spec(PAGE_SIZE as int, LINUX_MAX_IPC_BYTES as int));
    assert(shm_size_bad == linux_ipc_size_valid_spec(0, LINUX_MAX_IPC_BYTES as int));
    assert(msg_size_ok == linux_msg_size_valid_spec(
        LINUX_MAX_MSG_BYTES as int,
        LINUX_MAX_MSG_BYTES as int,
    ));
    assert(msg_size_bad == linux_msg_size_valid_spec(
        (LINUX_MAX_MSG_BYTES + 1) as int,
        LINUX_MAX_MSG_BYTES as int,
    ));
    assert(inet_domain == linux_socket_domain_supported_spec(
        LINUX_AF_INET,
        LINUX_AF_UNIX,
        LINUX_AF_LOCAL,
        LINUX_AF_INET,
        LINUX_AF_NETLINK,
        LINUX_AF_PACKET,
    ));
    assert(stream_type == linux_socket_type_supported_spec(
        LINUX_SOCK_STREAM,
        LINUX_SOCK_TYPE_MASK,
        LINUX_SOCK_STREAM,
        LINUX_SOCK_DGRAM,
        LINUX_SOCK_RAW,
    ));
    assert(unix_stream == linux_socket_domain_type_supported_spec(
        LINUX_AF_UNIX,
        LINUX_SOCK_STREAM,
        LINUX_AF_UNIX,
        LINUX_AF_LOCAL,
        LINUX_AF_INET,
        LINUX_AF_NETLINK,
        LINUX_AF_PACKET,
        LINUX_SOCK_STREAM,
        LINUX_SOCK_DGRAM,
        LINUX_SOCK_RAW,
    ));
    assert(unix_raw == linux_socket_domain_type_supported_spec(
        LINUX_AF_UNIX,
        LINUX_SOCK_RAW,
        LINUX_AF_UNIX,
        LINUX_AF_LOCAL,
        LINUX_AF_INET,
        LINUX_AF_NETLINK,
        LINUX_AF_PACKET,
        LINUX_SOCK_STREAM,
        LINUX_SOCK_DGRAM,
        LINUX_SOCK_RAW,
    ));
    assert(linux_usize_options_within_mask_spec(
        LINUX_SOCK_STREAM | LINUX_SOCK_CLOEXEC,
        LINUX_SOCK_ALLOWED_FLAGS,
    )) by(bit_vector);
    assert(socket_flags_ok);
    assert(!linux_usize_options_within_mask_spec(
        LINUX_SOCK_STREAM | 0x4000,
        LINUX_SOCK_ALLOWED_FLAGS,
    )) by(bit_vector);
    assert(!socket_flags_bad);
    assert(socket_addr_zero == linux_socket_addr_valid_spec(0, 0));
    assert(socket_addr_bad == linux_socket_addr_valid_spec(0, 16));
    assert(close_range_ok == linux_fd_range_valid_spec(3, 4));
    assert(close_range_bad == linux_fd_range_valid_spec(5, 4));
    assert(linux_usize_options_within_mask_spec(1, LINUX_MEMFD_ALLOWED_FLAGS)) by(bit_vector);
    assert(memfd_flags_ok);
    assert(!linux_usize_options_within_mask_spec(8, LINUX_MEMFD_ALLOWED_FLAGS)) by(bit_vector);
    assert(!memfd_flags_bad);
    assert(linux_u32_options_within_mask_spec(1, LINUX_GETRANDOM_ALLOWED_FLAGS)) by(bit_vector);
    assert(getrandom_flags_ok);
    assert(!linux_u32_options_within_mask_spec(4, LINUX_GETRANDOM_ALLOWED_FLAGS)) by(bit_vector);
    assert(!getrandom_flags_bad);
}

fn syscall_linux_file_dir_fd_poll_stat_exec_smoke() {
    let open_rdwr = linux_open_access_mode_valid(
        LINUX_O_RDWR,
        LINUX_O_ACCMODE,
        LINUX_O_RDONLY,
        LINUX_O_WRONLY,
        LINUX_O_RDWR,
    );
    let open_bad_access = linux_open_access_mode_valid(
        3,
        LINUX_O_ACCMODE,
        LINUX_O_RDONLY,
        LINUX_O_WRONLY,
        LINUX_O_RDWR,
    );
    let open_flags_ok = linux_open_flags_valid(
        LINUX_O_RDWR | LINUX_O_CREAT | LINUX_O_CLOEXEC,
        LINUX_OPEN_ALLOWED_FLAGS,
    );
    let open_flags_bad = linux_open_flags_valid(0x8000_0000, LINUX_OPEN_ALLOWED_FLAGS);
    let open_dir = linux_open_is_directory(LINUX_O_DIRECTORY, LINUX_O_DIRECTORY);
    let stdio_ok = linux_fd_target_valid(2, LINUX_STDIO_FD_MAX);
    let stdio_bad = linux_fd_target_valid(3, LINUX_STDIO_FD_MAX);
    let pipe_flags_ok = linux_pipe_flags_valid(LINUX_O_CLOEXEC, LINUX_PIPE_ALLOWED_FLAGS);
    let pipe_flags_bad = linux_pipe_flags_valid(LINUX_O_APPEND, LINUX_PIPE_ALLOWED_FLAGS);
    let dup_args_ok = linux_dup3_args_valid(3, 4);
    let dup_args_bad = linux_dup3_args_valid(3, 3);
    let fcntl_cmd_ok = linux_fcntl_cmd_supported(4, 0, 1, 2, 3, 4, 1030);
    let fcntl_cmd_bad = linux_fcntl_cmd_supported(99, 0, 1, 2, 3, 4, 1030);
    let fcntl_flags_ok =
        linux_fcntl_flags_valid(LINUX_O_NONBLOCK, LINUX_FCNTL_STATUS_ALLOWED_FLAGS);
    let fcntl_flags_bad =
        linux_fcntl_flags_valid(LINUX_O_CREAT, LINUX_FCNTL_STATUS_ALLOWED_FLAGS);
    let access_mode_ok = linux_path_mode_valid(0o7, LINUX_ACCESS_MODE_MASK);
    let access_mode_bad = linux_path_mode_valid(0o10, LINUX_ACCESS_MODE_MASK);
    let unlink_flags_ok = linux_unlink_flags_valid(LINUX_AT_REMOVEDIR, LINUX_UNLINK_ALLOWED_FLAGS);
    let unlink_flags_bad = linux_unlink_flags_valid(0x4000, LINUX_UNLINK_ALLOWED_FLAGS);
    let rename_flags_ok =
        linux_rename_flags_valid(LINUX_RENAME_NOREPLACE, LINUX_RENAME_ALLOWED_FLAGS);
    let rename_flags_bad = linux_rename_flags_valid(8, LINUX_RENAME_ALLOWED_FLAGS);
    let stat_flags_ok =
        linux_stat_flags_valid(LINUX_AT_SYMLINK_NOFOLLOW, LINUX_STAT_ALLOWED_FLAGS);
    let stat_flags_bad = linux_stat_flags_valid(0x8000, LINUX_STAT_ALLOWED_FLAGS);
    let stat_mask_ok = linux_stat_mask_valid(LINUX_STATX_BASIC_STATS, LINUX_STATX_BASIC_STATS);
    let stat_mask_bad = linux_stat_mask_valid(0x8000, LINUX_STATX_BASIC_STATS);
    let seek_ok = linux_lseek_whence_valid(5, LINUX_SEEK_MAX_WHENCE);
    let seek_bad = linux_lseek_whence_valid(6, LINUX_SEEK_MAX_WHENCE);
    let iov_count_ok = linux_iov_count_valid(1024, LINUX_MAX_IOV);
    let iov_count_bad = linux_iov_count_valid(1025, LINUX_MAX_IOV);
    let iov_bytes_ok = linux_iov_bytes_valid(2, 16, LINUX_MAX_IOV);
    let iov_bytes_bad = linux_iov_bytes_valid(1025, 16, LINUX_MAX_IOV);
    let poll_count_ok = linux_poll_count_valid(1024, LINUX_MAX_POLL_FDS);
    let poll_count_bad = linux_poll_count_valid(1025, LINUX_MAX_POLL_FDS);
    let poll_events_ok = linux_poll_events_valid(0x0005i16, LINUX_POLL_ALLOWED_EVENTS);
    let poll_events_bad = linux_poll_events_valid(0x4000i16, LINUX_POLL_ALLOWED_EVENTS);
    let copy_flags_ok = linux_copy_flags_valid(0, 0);
    let copy_flags_bad = linux_copy_flags_valid(1, 0);

    assert(open_rdwr == linux_open_access_mode_valid_spec(
        LINUX_O_RDWR,
        LINUX_O_ACCMODE,
        LINUX_O_RDONLY,
        LINUX_O_WRONLY,
        LINUX_O_RDWR,
    ));
    assert(open_bad_access == linux_open_access_mode_valid_spec(
        3,
        LINUX_O_ACCMODE,
        LINUX_O_RDONLY,
        LINUX_O_WRONLY,
        LINUX_O_RDWR,
    ));
    assert(linux_usize_options_within_mask_spec(
        LINUX_O_RDWR | LINUX_O_CREAT | LINUX_O_CLOEXEC,
        LINUX_OPEN_ALLOWED_FLAGS,
    )) by(bit_vector);
    assert(open_flags_ok);
    assert(!linux_usize_options_within_mask_spec(0x8000_0000, LINUX_OPEN_ALLOWED_FLAGS)) by(bit_vector);
    assert(!open_flags_bad);
    assert(open_dir == linux_open_is_directory_spec(LINUX_O_DIRECTORY, LINUX_O_DIRECTORY));
    assert(stdio_ok == linux_fd_target_valid_spec(2, LINUX_STDIO_FD_MAX as int));
    assert(stdio_bad == linux_fd_target_valid_spec(3, LINUX_STDIO_FD_MAX as int));
    assert(linux_usize_options_within_mask_spec(LINUX_O_CLOEXEC, LINUX_PIPE_ALLOWED_FLAGS))
        by(bit_vector);
    assert(pipe_flags_ok);
    assert(!linux_usize_options_within_mask_spec(LINUX_O_APPEND, LINUX_PIPE_ALLOWED_FLAGS))
        by(bit_vector);
    assert(!pipe_flags_bad);
    assert(dup_args_ok == linux_dup3_args_valid_spec(3, 4));
    assert(dup_args_bad == linux_dup3_args_valid_spec(3, 3));
    assert(fcntl_cmd_ok == linux_fcntl_cmd_supported_spec(4, 0, 1, 2, 3, 4, 1030));
    assert(fcntl_cmd_bad == linux_fcntl_cmd_supported_spec(99, 0, 1, 2, 3, 4, 1030));
    assert(linux_usize_options_within_mask_spec(
        LINUX_O_NONBLOCK,
        LINUX_FCNTL_STATUS_ALLOWED_FLAGS,
    )) by(bit_vector);
    assert(fcntl_flags_ok);
    assert(!linux_usize_options_within_mask_spec(
        LINUX_O_CREAT,
        LINUX_FCNTL_STATUS_ALLOWED_FLAGS,
    )) by(bit_vector);
    assert(!fcntl_flags_bad);
    assert(linux_usize_options_within_mask_spec(0o7, LINUX_ACCESS_MODE_MASK)) by(bit_vector);
    assert(access_mode_ok);
    assert(!linux_usize_options_within_mask_spec(0o10, LINUX_ACCESS_MODE_MASK)) by(bit_vector);
    assert(!access_mode_bad);
    assert(linux_usize_options_within_mask_spec(LINUX_AT_REMOVEDIR, LINUX_UNLINK_ALLOWED_FLAGS))
        by(bit_vector);
    assert(unlink_flags_ok);
    assert(!linux_usize_options_within_mask_spec(0x4000, LINUX_UNLINK_ALLOWED_FLAGS))
        by(bit_vector);
    assert(!unlink_flags_bad);
    assert(linux_usize_options_within_mask_spec(
        LINUX_RENAME_NOREPLACE,
        LINUX_RENAME_ALLOWED_FLAGS,
    )) by(bit_vector);
    assert(rename_flags_ok);
    assert(!linux_usize_options_within_mask_spec(8, LINUX_RENAME_ALLOWED_FLAGS)) by(bit_vector);
    assert(!rename_flags_bad);
    assert(linux_usize_options_within_mask_spec(
        LINUX_AT_SYMLINK_NOFOLLOW,
        LINUX_STAT_ALLOWED_FLAGS,
    )) by(bit_vector);
    assert(stat_flags_ok);
    assert(!linux_usize_options_within_mask_spec(0x8000, LINUX_STAT_ALLOWED_FLAGS)) by(bit_vector);
    assert(!stat_flags_bad);
    assert(linux_usize_options_within_mask_spec(
        LINUX_STATX_BASIC_STATS,
        LINUX_STATX_BASIC_STATS,
    )) by(bit_vector);
    assert(stat_mask_ok);
    assert(!linux_usize_options_within_mask_spec(0x8000, LINUX_STATX_BASIC_STATS)) by(bit_vector);
    assert(!stat_mask_bad);
    assert(seek_ok == linux_lseek_whence_valid_spec(5, LINUX_SEEK_MAX_WHENCE as int));
    assert(seek_bad == linux_lseek_whence_valid_spec(6, LINUX_SEEK_MAX_WHENCE as int));
    assert(iov_count_ok == linux_iov_count_valid_spec(1024, LINUX_MAX_IOV as int));
    assert(iov_count_bad == linux_iov_count_valid_spec(1025, LINUX_MAX_IOV as int));
    assert(iov_bytes_ok == linux_iov_bytes_valid_spec(2, 16, LINUX_MAX_IOV as int));
    assert(iov_bytes_bad == linux_iov_bytes_valid_spec(1025, 16, LINUX_MAX_IOV as int));
    assert(poll_count_ok == linux_poll_count_valid_spec(1024, LINUX_MAX_POLL_FDS as int));
    assert(poll_count_bad == linux_poll_count_valid_spec(1025, LINUX_MAX_POLL_FDS as int));
    assert(linux_i16_options_within_mask_spec(0x0005i16, LINUX_POLL_ALLOWED_EVENTS)) by(bit_vector);
    assert(poll_events_ok);
    assert(!linux_i16_options_within_mask_spec(0x4000i16, LINUX_POLL_ALLOWED_EVENTS))
        by(bit_vector);
    assert(!poll_events_bad);
    assert(linux_usize_options_within_mask_spec(0, 0)) by(bit_vector);
    assert(copy_flags_ok);
    assert(!linux_usize_options_within_mask_spec(1, 0)) by(bit_vector);
    assert(!copy_flags_bad);
}

fn syscall_hypervisor_exec_smoke() {
    let guest_options = zircon_hypervisor_options_valid(0, ZX_HYPERVISOR_OPTIONS_MASK);
    let bad_guest_options = zircon_hypervisor_options_valid(1, ZX_HYPERVISOR_OPTIONS_MASK);
    let trap_kind_ok = zircon_guest_trap_kind_valid(ZX_GUEST_TRAP_MEM, ZX_GUEST_TRAP_KIND_MAX);
    let trap_kind_bad = zircon_guest_trap_kind_valid(3, ZX_GUEST_TRAP_KIND_MAX);
    let trap_bell = zircon_guest_trap_is_bell(ZX_GUEST_TRAP_BELL, ZX_GUEST_TRAP_BELL);
    let trap_mem = zircon_guest_trap_is_mem(ZX_GUEST_TRAP_MEM, ZX_GUEST_TRAP_MEM);
    let trap_range_ok =
        zircon_guest_trap_range_valid(0x4000, PAGE_SIZE as u64, ZX_GUEST_PHYS_LIMIT);
    let trap_range_bad = zircon_guest_trap_range_valid(0x4000, 0, ZX_GUEST_PHYS_LIMIT);
    let trap_align_ok = zircon_guest_trap_alignment_valid(
        ZX_GUEST_TRAP_MEM,
        0x4000,
        PAGE_SIZE as u64,
        ZX_GUEST_TRAP_BELL,
        ZX_GUEST_TRAP_MEM,
        PAGE_SIZE as u64,
    );
    let trap_align_bad = zircon_guest_trap_alignment_valid(
        ZX_GUEST_TRAP_MEM,
        0x4001,
        PAGE_SIZE as u64,
        ZX_GUEST_TRAP_BELL,
        ZX_GUEST_TRAP_MEM,
        PAGE_SIZE as u64,
    );
    let entry_ok = zircon_vcpu_entry_valid(0x8000, ZX_VCPU_ENTRY_ALIGNMENT);
    let entry_bad = zircon_vcpu_entry_valid(0x8001, ZX_VCPU_ENTRY_ALIGNMENT);
    let vector_ok =
        zircon_vcpu_interrupt_vector_valid(128, ZX_VCPU_INTERRUPT_VECTOR_MAX);
    let vector_bad =
        zircon_vcpu_interrupt_vector_valid(1024, ZX_VCPU_INTERRUPT_VECTOR_MAX);
    let read_state_ok = zircon_vcpu_read_state_args_valid(
        ZX_VCPU_STATE,
        ZX_VCPU_STATE_SIZE,
        ZX_VCPU_STATE,
        ZX_VCPU_STATE_SIZE,
    );
    let read_state_bad = zircon_vcpu_read_state_args_valid(
        ZX_VCPU_IO,
        ZX_VCPU_IO_SIZE,
        ZX_VCPU_STATE,
        ZX_VCPU_STATE_SIZE,
    );
    let write_io_ok = zircon_vcpu_write_state_args_valid(
        ZX_VCPU_IO,
        ZX_VCPU_IO_SIZE,
        ZX_VCPU_STATE,
        ZX_VCPU_STATE_SIZE,
        ZX_VCPU_IO,
        ZX_VCPU_IO_SIZE,
    );

    assert(guest_options == u32_options_within_mask_spec(0, ZX_HYPERVISOR_OPTIONS_MASK));
    assert(bad_guest_options == u32_options_within_mask_spec(1, ZX_HYPERVISOR_OPTIONS_MASK));
    assert(trap_kind_ok == zircon_system_event_kind_valid_spec(
        ZX_GUEST_TRAP_MEM as int,
        ZX_GUEST_TRAP_KIND_MAX as int,
    ));
    assert(trap_kind_bad == zircon_system_event_kind_valid_spec(
        3,
        ZX_GUEST_TRAP_KIND_MAX as int,
    ));
    assert(trap_bell);
    assert(trap_mem);
    assert(trap_range_ok == zircon_guest_trap_range_valid_spec(
        0x4000,
        PAGE_SIZE as int,
        ZX_GUEST_PHYS_LIMIT as int,
    ));
    assert(trap_range_bad == zircon_guest_trap_range_valid_spec(
        0x4000,
        0,
        ZX_GUEST_PHYS_LIMIT as int,
    ));
    assert(trap_align_ok == zircon_guest_trap_alignment_valid_spec(
        ZX_GUEST_TRAP_MEM as int,
        0x4000,
        PAGE_SIZE as int,
        ZX_GUEST_TRAP_BELL as int,
        ZX_GUEST_TRAP_MEM as int,
        PAGE_SIZE as int,
    ));
    assert(trap_align_bad == zircon_guest_trap_alignment_valid_spec(
        ZX_GUEST_TRAP_MEM as int,
        0x4001,
        PAGE_SIZE as int,
        ZX_GUEST_TRAP_BELL as int,
        ZX_GUEST_TRAP_MEM as int,
        PAGE_SIZE as int,
    ));
    assert(entry_ok == zircon_vcpu_entry_valid_spec(0x8000, ZX_VCPU_ENTRY_ALIGNMENT as int));
    assert(entry_bad == zircon_vcpu_entry_valid_spec(0x8001, ZX_VCPU_ENTRY_ALIGNMENT as int));
    assert(vector_ok == zircon_vcpu_interrupt_vector_valid_spec(
        128,
        ZX_VCPU_INTERRUPT_VECTOR_MAX as int,
    ));
    assert(vector_bad == zircon_vcpu_interrupt_vector_valid_spec(
        1024,
        ZX_VCPU_INTERRUPT_VECTOR_MAX as int,
    ));
    assert(read_state_ok == zircon_vcpu_read_state_args_valid_spec(
        ZX_VCPU_STATE as int,
        ZX_VCPU_STATE_SIZE as int,
        ZX_VCPU_STATE as int,
        ZX_VCPU_STATE_SIZE as int,
    ));
    assert(read_state_bad == zircon_vcpu_read_state_args_valid_spec(
        ZX_VCPU_IO as int,
        ZX_VCPU_IO_SIZE as int,
        ZX_VCPU_STATE as int,
        ZX_VCPU_STATE_SIZE as int,
    ));
    assert(write_io_ok == zircon_vcpu_write_state_args_valid_spec(
        ZX_VCPU_IO as int,
        ZX_VCPU_IO_SIZE as int,
        ZX_VCPU_STATE as int,
        ZX_VCPU_STATE_SIZE as int,
        ZX_VCPU_IO as int,
        ZX_VCPU_IO_SIZE as int,
    ));
}

proof fn syscall_address_logic_smoke() {
    assert(page_aligned_spec(0));
    assert(page_aligned_spec(PAGE_SIZE as int));
    assert(!page_aligned_spec(4095));

    assert(range_within_window_spec(
        LINUX_MAPPING_BASE as int,
        PAGE_SIZE as int,
        LINUX_MAPPING_BASE as int,
        LINUX_MAPPING_LIMIT as int,
    ));
    assert(!range_within_window_spec(
        usize::MAX as int,
        PAGE_SIZE as int,
        LINUX_MAPPING_BASE as int,
        LINUX_MAPPING_LIMIT as int,
    ));

    assert(range_overlaps_spec(
        LINUX_MAPPING_BASE as int,
        PAGE_SIZE as int,
        LINUX_MAPPING_BASE as int,
        PAGE_SIZE as int,
    ));
    assert(!range_overlaps_spec(
        LINUX_MAPPING_BASE as int,
        PAGE_SIZE as int,
        0x5000_1000,
        PAGE_SIZE as int,
    ));
}

proof fn linux_range_availability_smoke() {
    let first = LinuxRange {
        addr: 0x5000_1000,
        len: PAGE_SIZE,
    };
    let second = LinuxRange {
        addr: 0x5000_3000,
        len: PAGE_SIZE,
    };
    let mappings = seq![first, second];

    range_end_before_start_means_no_overlap(
        LINUX_MAPPING_BASE as int,
        PAGE_SIZE as int,
        first.addr as int,
        first.len as int,
    );
    range_end_before_start_means_no_overlap(
        LINUX_MAPPING_BASE as int,
        PAGE_SIZE as int,
        second.addr as int,
        second.len as int,
    );
    assert(linux_range_available_spec(
        LINUX_MAPPING_BASE as int,
        PAGE_SIZE as int,
        mappings,
    ));

    assert(!linux_range_available_spec(
        first.addr as int,
        PAGE_SIZE as int,
        mappings,
    ));

    range_start_after_end_means_no_overlap(
        0x5000_2000 as int,
        PAGE_SIZE as int,
        first.addr as int,
        first.len as int,
    );
    range_end_before_start_means_no_overlap(
        0x5000_2000 as int,
        PAGE_SIZE as int,
        second.addr as int,
        second.len as int,
    );
    assert(linux_range_available_spec(
        0x5000_2000 as int,
        PAGE_SIZE as int,
        mappings,
    ));

    assert(!linux_range_available_spec(
        0x4fff_f000 as int,
        PAGE_SIZE as int,
        mappings,
    ));
}

} // verus!
