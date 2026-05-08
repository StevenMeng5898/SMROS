#![allow(unused_macros)]

use vstd::prelude::*;

verus! {

include!("../../../src/main_logic_shared.rs");
include!("../../../src/user_level/drivers/driver_logic_shared.rs");
include!("../../../src/user_level/services/user_logic_shared.rs");

pub const KERNEL_HEAP_SIZE: usize = 0x0400_0000;
pub const PAGE_SIZE: usize = 4096;
pub const USER_PROCESS_CAPACITY: usize = 16;
pub const USER_INIT_PARENT_PID: usize = 1;
pub const USER_CODE_VADDR: usize = 0x0000_0000;
pub const USER_DATA_VADDR: usize = 0x0000_1000;
pub const USER_HEAP_VADDR: usize = 0x0000_2000;
pub const USER_HEAP_PAGES: usize = 4;
pub const USER_STACK_VADDR: usize = 0xFFFF_0000;
pub const USER_STACK_PAGES: usize = 2;
pub const USER_THREAD_TIME_SLICE: u32 = 10;
pub const DEFAULT_STACK_SIZE: usize = 0x4000;
pub const USER_MMAP_BASE: u64 = 0x5000_0000;
pub const USER_MMAP_LIMIT: u64 = 0x6000_0000;
pub const USER_NAMESPACE_RIGHTS_MASK: u32 = 0x7;
pub const USER_FXFS_MAX_NODES: usize = 512;
pub const USER_FXFS_MAX_DIRENTS: usize = 768;
pub const USER_FXFS_MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
pub const USER_ELF_HEADER_SIZE: usize = 64;
pub const USER_ELF_PHDR_SIZE: usize = 56;
pub const USER_ELF_MAX_PHDRS: usize = 16;
pub const USER_ELF_MACHINE_AARCH64: u16 = 183;
pub const USER_ELF_TYPE_EXEC: u16 = 2;
pub const USER_ELF_TYPE_DYN: u16 = 3;
pub const USER_SVC_MAX_NAME_LEN: usize = 64;
pub const USER_SVC_RIGHTS_MASK: u32 = 0x3;
pub const USER_SVC_IPC_MAGIC: u32 = 0x534d_4950;
pub const USER_SVC_IPC_VERSION: u16 = 1;
pub const USER_SVC_IPC_MESSAGE_SIZE: usize = 32;
pub const USER_SVC_COMPONENT_MANAGER: u16 = 0;
pub const USER_SVC_RUNNER: u16 = 1;
pub const USER_SVC_FILESYSTEM: u16 = 2;
pub const USER_SVC_COMPONENT_START: u16 = 1;
pub const USER_SVC_RUNNER_LOAD_ELF: u16 = 2;
pub const USER_SVC_FILESYSTEM_DESCRIBE: u16 = 3;
pub const USER_DRIVER_MMIO_BASE: usize = 0x0a00_0000;
pub const USER_DRIVER_MMIO_STRIDE: usize = 0x200;
pub const USER_DRIVER_MMIO_SLOT_COUNT: usize = 32;
pub const USER_DRIVER_BLOCK_SIZE: usize = 512;
pub const USER_DRIVER_ETHERNET_MTU: usize = 1500;
pub const USER_DRIVER_ETHERNET_HEADER_LEN: usize = 14;
pub const USER_DRIVER_ETHERNET_FRAME_MAX: usize = USER_DRIVER_ETHERNET_HEADER_LEN
    + USER_DRIVER_ETHERNET_MTU;
pub const USER_DRIVER_NET_HDR_LEN: usize = 10;
pub const USER_DRIVER_NET_BUFFER_SIZE: usize = 2048;
pub const USER_DRIVER_VIRTIO_QUEUE_SIZE: u16 = 8;
pub const USER_DRIVER_VIRTIO_MAGIC: u32 = 0x7472_6976;
pub const USER_DRIVER_VIRTIO_VENDOR_QEMU: u32 = 0x554d_4551;
pub const USER_DRIVER_VIRTIO_VERSION_LEGACY: u32 = 1;
pub const USER_DRIVER_VIRTIO_VERSION_MODERN: u32 = 2;
pub const USER_DRIVER_VIRTIO_DEVICE_NET: u32 = 1;
pub const USER_DRIVER_VIRTIO_DEVICE_BLOCK: u32 = 2;
pub const USER_DRIVER_VIRTIO_F_VERSION_1: u64 = 1u64 << 32;
pub const USER_DRIVER_VIRTIO_BLK_F_FLUSH: u64 = 1u64 << 9;
pub const USER_DRIVER_VIRTIO_BLK_F_CONFIG_WCE: u64 = 1u64 << 11;
pub const USER_DRIVER_VIRTIO_NET_F_MAC: u64 = 1u64 << 5;
pub const USER_DRIVER_VIRTIO_NET_F_STATUS: u64 = 1u64 << 16;

pub const SYS_WRITE: u32 = 64;
pub const SYS_EXIT: u32 = 93;
pub const SYS_GETPID: u32 = 172;
pub const SYS_MMAP: u32 = 222;
pub const EL0_TEST_STACK_SIZE: usize = 0x2000;
pub const EL0_TEST_BANNER_LEN: usize = 33;
pub const EL0_TEST_INFO_GETPID_LEN: usize = 27;
pub const EL0_TEST_INFO_MMAP_LEN: usize = 25;
pub const EL0_TEST_COMPLETE_LEN: usize = 35;
pub const EL0_TEST_EXIT_OK: i32 = 0;
pub const EL0_TEST_EXIT_WRITE_RESULT_MISMATCH: i32 = 10;
pub const EL0_TEST_EXIT_GETPID_RESULT_MISMATCH: i32 = 11;
pub const EL0_TEST_EXIT_GETPID_LOG_WRITE_MISMATCH: i32 = 12;
pub const EL0_TEST_EXIT_MMAP_RESULT_MISMATCH: i32 = 13;
pub const EL0_TEST_EXIT_MMAP_LOG_WRITE_MISMATCH: i32 = 14;
pub const EL0_TEST_EXIT_COMPLETE_LOG_WRITE_MISMATCH: i32 = 15;

#[derive(Copy, Clone)]
struct UserProcessSlotModel {
    pid: usize,
    occupied: bool,
}

#[derive(Copy, Clone)]
struct El0KernelObservation {
    entered: bool,
    write_result: u64,
    pid: u64,
    mmap_addr: u64,
}

spec fn checked_end_spec(addr: int, len: int) -> Option<int> {
    if 0 <= addr && 0 <= len && addr <= usize::MAX as int - len {
        Some(addr + len)
    } else {
        Option::<int>::None
    }
}

spec fn checked_mul_spec(lhs: int, rhs: int) -> Option<int> {
    if 0 <= lhs && 0 <= rhs && (rhs == 0 || lhs <= usize::MAX as int / rhs) {
        Some(lhs * rhs)
    } else {
        Option::<int>::None
    }
}

spec fn checked_sub_spec(lhs: int, rhs: int) -> Option<int> {
    if 0 <= lhs && 0 <= rhs && rhs <= lhs {
        Some(lhs - rhs)
    } else {
        Option::<int>::None
    }
}

spec fn align_up_spec(pos: int, align: int) -> Option<int> {
    if pos < 0 || align <= 0 {
        Option::<int>::None
    } else {
        let offset = pos % align;
        if offset == 0 {
            Some(pos)
        } else if pos <= usize::MAX as int - (align - offset) {
            Some(pos + align - offset)
        } else {
            Option::<int>::None
        }
    }
}

spec fn bump_alloc_next_spec(pos: int, size: int, align: int, heap_size: int) -> Option<(int, int)> {
    match align_up_spec(pos, align) {
        Some(aligned_pos) => match checked_end_spec(aligned_pos, size) {
            Some(next_pos) => if next_pos <= heap_size {
                Some((aligned_pos, next_pos))
            } else {
                Option::<(int, int)>::None
            },
            None => Option::<(int, int)>::None,
        },
        None => Option::<(int, int)>::None,
    }
}

spec fn page_offset_spec(base: int, page_index: int, page_size: int) -> Option<int> {
    if 0 <= base
        && 0 <= page_index
        && 0 <= page_size
        && (page_size == 0 || page_index <= usize::MAX as int / page_size)
    {
        checked_end_spec(base, page_index * page_size)
    } else {
        Option::<int>::None
    }
}

spec fn page_down_spec(value: int, page_size: int) -> Option<int> {
    if 0 <= value && 0 < page_size {
        checked_sub_spec(value, value % page_size)
    } else {
        Option::<int>::None
    }
}

spec fn page_up_spec(value: int, page_size: int) -> Option<int> {
    if 0 <= value && 0 < page_size && value <= usize::MAX as int - (page_size - 1) {
        let adjusted = value + page_size - 1;
        checked_sub_spec(adjusted, adjusted % page_size)
    } else {
        Option::<int>::None
    }
}

spec fn pfn_to_paddr_spec(pfn: int, page_size: int) -> Option<int> {
    if 0 <= pfn && 0 <= page_size && (page_size == 0 || pfn <= u64::MAX as int / page_size) {
        Some(pfn * page_size)
    } else {
        Option::<int>::None
    }
}

spec fn stack_top_u64_spec(stack_base: int, stack_size: int) -> Option<int> {
    if 0 <= stack_base && 0 <= stack_size && stack_base <= u64::MAX as int - stack_size {
        Some(stack_base + stack_size)
    } else {
        Option::<int>::None
    }
}

spec fn driver_mmio_slot_base_spec(base: int, slot: int, stride: int) -> Option<int> {
    if 0 <= base
        && 0 <= slot
        && 0 <= stride
        && (stride == 0 || slot <= usize::MAX as int / stride)
    {
        checked_end_spec(base, slot * stride)
    } else {
        Option::<int>::None
    }
}

spec fn driver_virtio_identity_valid_spec(
    magic: int,
    device_id: int,
    expected_device: int,
    vendor: int,
    expected_vendor: int,
) -> bool {
    magic == USER_DRIVER_VIRTIO_MAGIC as int
        && device_id == expected_device
        && vendor == expected_vendor
}

spec fn driver_virtio_version_supported_spec(version: int, legacy: int, modern: int) -> bool {
    version == legacy || version == modern
}

spec fn driver_virtio_version_is_modern_spec(version: int, modern: int) -> bool {
    version == modern
}

spec fn driver_virtio_queue_size_valid_spec(max_queue: int, queue_size: int) -> bool {
    queue_size != 0 && max_queue >= queue_size
}

spec fn driver_virtio_block_accepted_features_spec(
    features: u64,
    flush: u64,
    config_wce: u64,
) -> u64 {
    features & (flush | config_wce)
}

spec fn driver_virtio_driver_features_spec(accepted: u64, version_1: u64, modern: bool) -> u64 {
    if modern {
        accepted | version_1
    } else {
        accepted
    }
}

spec fn driver_virtio_net_accepted_features_spec(
    features: u64,
    mac: u64,
    status: u64,
    version_1: u64,
    modern: bool,
) -> u64 {
    let accepted_mac = if features & mac != 0u64 {
        0u64 | mac
    } else {
        0u64
    };
    let accepted_status = if features & status != 0u64 {
        accepted_mac | status
    } else {
        accepted_mac
    };
    let accepted_version = if modern {
        accepted_status | version_1
    } else {
        accepted_status
    };
    accepted_version
}

spec fn driver_block_capacity_bytes_spec(blocks: int, block_size: int) -> int {
    match checked_mul_spec(blocks, block_size) {
        Some(bytes) => bytes,
        None => usize::MAX as int,
    }
}

spec fn driver_block_range_valid_spec(
    offset: int,
    len: int,
    blocks: int,
    block_size: int,
) -> bool {
    match checked_mul_spec(blocks, block_size) {
        Some(capacity) => match checked_end_spec(offset, len) {
            Some(end) => end <= capacity,
            None => false,
        },
        None => false,
    }
}

spec fn driver_block_len_valid_spec(len: int, block_size: int) -> bool {
    len == block_size
}

spec fn driver_block_id_valid_spec(block: int, capacity_blocks: int) -> bool {
    block < capacity_blocks
}

spec fn driver_net_tx_frame_len_valid_spec(
    frame_len: int,
    max_frame: int,
    header_len: int,
    buffer_size: int,
) -> bool {
    frame_len <= max_frame
        && match checked_end_spec(frame_len, header_len) {
            Some(total) => total <= buffer_size,
            None => false,
        }
}

spec fn driver_net_rx_packet_len_valid_spec(
    packet_len: int,
    header_len: int,
    buffer_size: int,
) -> bool {
    packet_len >= header_len && packet_len <= buffer_size
}

spec fn driver_net_rx_frame_len_spec(packet_len: int, header_len: int) -> Option<int> {
    checked_sub_spec(packet_len, header_len)
}

spec fn driver_net_rx_output_len_valid_spec(frame_len: int, out_len: int) -> bool {
    frame_len <= out_len
}

spec fn ascii_shell_input_spec(byte: int) -> bool {
    0x20 <= byte && byte <= 0x7e
}

spec fn decimal_digit_value_spec(byte: int) -> Option<int> {
    if 48 <= byte && byte <= 57 {
        Some(byte - 48)
    } else {
        Option::<int>::None
    }
}

spec fn parse_digit_step_spec(result: int, digit: int) -> Option<int> {
    if 0 <= result
        && 0 <= digit
        && result <= usize::MAX as int / 10
        && result * 10 <= usize::MAX as int - digit
    {
        Some(result * 10 + digit)
    } else {
        Option::<int>::None
    }
}

spec fn ipv4_octet_step_spec(value: int, digit: int) -> Option<int> {
    if 0 <= value && 0 <= digit && value <= u32::MAX as int / 10 {
        let scaled = value * 10;
        if scaled <= u32::MAX as int - digit && scaled + digit <= 255 {
            Some(scaled + digit)
        } else {
            Option::<int>::None
        }
    } else {
        Option::<int>::None
    }
}

spec fn saturating_sub_spec(lhs: int, rhs: int) -> int {
    if lhs >= rhs {
        lhs - rhs
    } else {
        0
    }
}

spec fn pages_to_kb_spec(pages: int, page_size: int) -> int {
    if 0 <= pages && 0 <= page_size && (page_size == 0 || pages <= usize::MAX as int / page_size) {
        (pages * page_size) / 1024
    } else {
        usize::MAX as int
    }
}

spec fn usage_percent_spec(used_pages: int, total_pages: int) -> int {
    if total_pages == 0 {
        0
    } else if 0 <= used_pages && used_pages <= usize::MAX as int / 100 {
        (used_pages * 100) / total_pages
    } else {
        usize::MAX as int
    }
}

spec fn uptime_parts_spec(ticks: int) -> (int, int, int, int) {
    let seconds = ticks / 100;
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    (seconds, minutes, hours, days)
}

spec fn mmap_result_ok_spec(addr: int, page_size: int, base: int, limit: int) -> bool {
    page_size != 0 && addr >= base && addr < limit && addr % page_size == 0
}

spec fn dns_host_len_valid_spec(len: int, max_len: int) -> bool {
    0 < len && len <= max_len
}

spec fn dns_label_len_valid_spec(len: int, max_len: int) -> bool {
    0 < len && len <= max_len
}

spec fn dns_label_byte_valid_spec(byte: int) -> bool {
    (0x61 <= byte && byte <= 0x7a)
        || (0x41 <= byte && byte <= 0x5a)
        || (0x30 <= byte && byte <= 0x39)
        || byte == 0x2d
}

spec fn kernel_success_spec(
    kernel_entered: bool,
    kernel_finished: bool,
    exit_code: int,
    kernel_write: int,
    kernel_pid: int,
    kernel_mmap: int,
    banner_len: int,
) -> bool {
    kernel_entered
        && kernel_finished
        && exit_code == EL0_TEST_EXIT_OK as int
        && kernel_write == banner_len
        && kernel_pid == 1
        && kernel_mmap > 0
        && kernel_mmap < 0xFFFF_FFFF_FFFF_F000u64 as int
}

spec fn component_start_allowed_spec(
    binary_exists: bool,
    destroyed: bool,
    already_started: bool,
) -> bool {
    already_started || (binary_exists && !destroyed)
}

spec fn namespace_rights_valid_spec(rights: int, allowed_mask: int) -> bool {
    0 <= rights && 0 <= allowed_mask && rights <= allowed_mask
}

spec fn fxfs_file_size_valid_spec(size: int, max_size: int) -> bool {
    0 <= size && size <= max_size
}

spec fn fxfs_node_capacity_valid_spec(nodes: int, max_nodes: int) -> bool {
    0 <= nodes && nodes < max_nodes
}

spec fn fxfs_dirent_capacity_valid_spec(entries: int, max_entries: int) -> bool {
    0 <= entries && entries < max_entries
}

spec fn fxfs_append_size_spec(old_size: int, append_len: int) -> Option<int> {
    checked_end_spec(old_size, append_len)
}

spec fn fxfs_write_end_spec(offset: int, len: int) -> Option<int> {
    checked_end_spec(offset, len)
}

spec fn fxfs_seek_valid_spec(offset: int, size: int) -> bool {
    0 <= offset && offset <= size
}

spec fn fxfs_replay_count_valid_spec(replayed: int, journal_records: int) -> bool {
    0 <= replayed && replayed <= journal_records
}

spec fn svc_name_valid_spec(len: int, max_len: int) -> bool {
    0 < len && len <= max_len
}

spec fn svc_rights_valid_spec(rights: int, allowed_mask: int) -> bool {
    0 < rights && 0 <= allowed_mask && rights <= allowed_mask
}

spec fn svc_ipc_message_size_valid_spec(size: int, expected: int) -> bool {
    size == expected
}

spec fn svc_ipc_header_valid_spec(
    magic: int,
    version: int,
    expected_magic: int,
    expected_version: int,
) -> bool {
    magic == expected_magic && version == expected_version
}

spec fn svc_protocol_allowed_spec(
    service: int,
    ordinal: int,
    component_manager: int,
    runner: int,
    filesystem: int,
    component_start: int,
    runner_load: int,
    filesystem_describe: int,
) -> bool {
    (service == component_manager && ordinal == component_start)
        || (service == runner && ordinal == runner_load)
        || (service == filesystem && ordinal == filesystem_describe)
}

spec fn component_thread_launch_valid_spec(
    process_created: bool,
    queued: bool,
    thread_created: bool,
) -> bool {
    process_created && queued && thread_created
}

spec fn component_return_active_spec(pid: int) -> bool {
    pid != 0
}

spec fn elf_header_bounds_valid_spec(image_len: int, header_size: int) -> bool {
    image_len >= header_size
}

spec fn elf_magic_valid_spec(b0: int, b1: int, b2: int, b3: int) -> bool {
    b0 == 0x7f && b1 == 0x45 && b2 == 0x4c && b3 == 0x46
}

spec fn elf_class_data_valid_spec(class: int, data: int, version: int) -> bool {
    class == 2 && data == 1 && version == 1
}

spec fn elf_type_valid_spec(elf_type: int, exec_type: int, dyn_type: int) -> bool {
    elf_type == exec_type || elf_type == dyn_type
}

spec fn elf_machine_valid_spec(machine: int, expected: int) -> bool {
    machine == expected
}

spec fn elf_entry_valid_spec(entry: int) -> bool {
    entry != 0
}

spec fn elf_phdr_table_valid_spec(
    phoff: int,
    phentsize: int,
    phnum: int,
    image_len: int,
    expected_phentsize: int,
    max_phnum: int,
) -> bool {
    if phentsize != expected_phentsize || phnum == 0 || phnum > max_phnum {
        false
    } else {
        match checked_mul_spec(phentsize, phnum) {
            Some(table_size) => match checked_end_spec(phoff, table_size) {
                Some(end) => end <= image_len,
                None => false,
            },
            None => false,
        }
    }
}

spec fn elf_segment_bounds_valid_spec(
    offset: int,
    file_size: int,
    mem_size: int,
    image_len: int,
) -> bool {
    if file_size > mem_size {
        false
    } else {
        match checked_end_spec(offset, file_size) {
            Some(end) => end <= image_len,
            None => false,
        }
    }
}

spec fn elf_vaddr_range_valid_spec(vaddr: int, mem_size: int) -> bool {
    0 <= vaddr && 0 <= mem_size && vaddr <= u64::MAX as int - mem_size
}

spec fn elf_segment_mapping_range_spec(
    vaddr: int,
    mem_size: int,
    page_size: int,
) -> Option<(int, int)> {
    if mem_size == 0 {
        Option::<(int, int)>::None
    } else {
        match checked_end_spec(vaddr, mem_size) {
            Some(end) => match page_down_spec(vaddr, page_size) {
                Some(start) => match page_up_spec(end, page_size) {
                    Some(aligned_end) => Some((start, aligned_end)),
                    None => Option::<(int, int)>::None,
                },
                None => Option::<(int, int)>::None,
            },
            None => Option::<(int, int)>::None,
        }
    }
}

spec fn user_test_exit_code_spec(
    write_result: int,
    pid: int,
    getpid_log_write: int,
    mmap_addr: int,
    mmap_log_write: int,
    complete_log_write: int,
) -> int {
    if write_result != EL0_TEST_BANNER_LEN as int {
        EL0_TEST_EXIT_WRITE_RESULT_MISMATCH as int
    } else if pid != 1 {
        EL0_TEST_EXIT_GETPID_RESULT_MISMATCH as int
    } else if getpid_log_write != EL0_TEST_INFO_GETPID_LEN as int {
        EL0_TEST_EXIT_GETPID_LOG_WRITE_MISMATCH as int
    } else if !mmap_result_ok_spec(
        mmap_addr,
        PAGE_SIZE as int,
        USER_MMAP_BASE as int,
        USER_MMAP_LIMIT as int,
    ) {
        EL0_TEST_EXIT_MMAP_RESULT_MISMATCH as int
    } else if mmap_log_write != EL0_TEST_INFO_MMAP_LEN as int {
        EL0_TEST_EXIT_MMAP_LOG_WRITE_MISMATCH as int
    } else if complete_log_write != EL0_TEST_COMPLETE_LEN as int {
        EL0_TEST_EXIT_COMPLETE_LOG_WRITE_MISMATCH as int
    } else {
        EL0_TEST_EXIT_OK as int
    }
}

fn main_align_up(pos: usize, align: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(aligned) => align_up_spec(pos as int, align as int) == Some(aligned as int),
            None => align_up_spec(pos as int, align as int) == Option::<int>::None,
        },
{
    smros_main_align_up_body!(pos, align)
}

fn main_bump_alloc_next(
    pos: usize,
    size: usize,
    align: usize,
    heap_size: usize,
) -> (out: Option<(usize, usize)>)
    ensures
        match out {
            Some((aligned_pos, next_pos)) => bump_alloc_next_spec(
                pos as int,
                size as int,
                align as int,
                heap_size as int,
            ) == Some((aligned_pos as int, next_pos as int)),
            None => bump_alloc_next_spec(
                pos as int,
                size as int,
                align as int,
                heap_size as int,
            ) == Option::<(int, int)>::None,
        },
{
    smros_main_bump_alloc_next_body!(pos, size, align, heap_size)
}

fn user_checked_end(addr: usize, len: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(end) => checked_end_spec(addr as int, len as int) == Some(end as int),
            None => checked_end_spec(addr as int, len as int) == Option::<int>::None,
        },
{
    smros_user_checked_end_body!(addr, len)
}

fn user_checked_mul(lhs: usize, rhs: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(product) => checked_mul_spec(lhs as int, rhs as int) == Some(product as int),
            None => checked_mul_spec(lhs as int, rhs as int) == Option::<int>::None,
        },
{
    if rhs == 0 {
        assert(checked_mul_spec(lhs as int, rhs as int) == Some(0int));
        return Some(0);
    }

    if lhs <= usize::MAX / rhs {
        assert((lhs as int) * (rhs as int) <= usize::MAX as int) by(nonlinear_arith)
            requires
                rhs as int > 0,
                lhs as int <= usize::MAX as int / rhs as int,
        ;
        let product = lhs * rhs;
        assert(product == lhs * rhs);
        assert(product as int == (lhs as int) * (rhs as int)) by(nonlinear_arith)
            requires
                rhs as int > 0,
                lhs as int <= usize::MAX as int / rhs as int,
                product as int == (lhs as int) * (rhs as int),
        ;
        assert(checked_mul_spec(lhs as int, rhs as int) == Some(product as int));
        Some(product)
    } else {
        assert(checked_mul_spec(lhs as int, rhs as int) == Option::<int>::None);
        None
    }
}

fn user_page_offset(base: usize, page_index: usize, page_size: usize) -> (out: Option<usize>)
    requires
        page_size > 0,
        page_index <= usize::MAX / page_size,
    ensures
        match out {
            Some(vaddr) => page_offset_spec(base as int, page_index as int, page_size as int)
                == Some(vaddr as int),
            None => page_offset_spec(base as int, page_index as int, page_size as int)
                == Option::<int>::None,
        },
{
    smros_user_page_offset_body!(base, page_index, page_size)
}

fn user_page_down(value: usize, page_size: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(aligned) => page_down_spec(value as int, page_size as int) == Some(aligned as int),
            None => page_down_spec(value as int, page_size as int) == Option::<int>::None,
        },
{
    smros_user_page_down_body!(value, page_size)
}

fn user_page_up(value: usize, page_size: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(aligned) => page_up_spec(value as int, page_size as int) == Some(aligned as int),
            None => page_up_spec(value as int, page_size as int) == Option::<int>::None,
        },
{
    smros_user_page_up_body!(value, page_size)
}

fn user_pfn_to_paddr(pfn: u64, page_size: u64) -> (out: Option<u64>)
    requires
        page_size > 0,
        pfn as int <= u64::MAX as int / page_size as int,
    ensures
        match out {
            Some(paddr) => pfn_to_paddr_spec(pfn as int, page_size as int)
                == Some(paddr as int),
            None => pfn_to_paddr_spec(pfn as int, page_size as int) == Option::<int>::None,
        },
{
    let out = smros_user_pfn_to_paddr_body!(pfn, page_size);
    assert((pfn as int) * (page_size as int) <= u64::MAX as int) by (nonlinear_arith)
        requires
            page_size as int > 0,
            pfn as int <= u64::MAX as int / page_size as int,
    ;
    out
}

fn user_stack_top_u64(stack_base: u64, stack_size: usize) -> (out: Option<u64>)
    ensures
        match out {
            Some(stack_top) => stack_top_u64_spec(stack_base as int, stack_size as int)
                == Some(stack_top as int),
            None => stack_top_u64_spec(stack_base as int, stack_size as int)
                == Option::<int>::None,
        },
{
    smros_user_stack_top_u64_body!(stack_base, stack_size)
}

fn driver_mmio_slot_base(base: usize, slot: usize, stride: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(slot_base) => driver_mmio_slot_base_spec(base as int, slot as int, stride as int)
                == Some(slot_base as int),
            None => driver_mmio_slot_base_spec(base as int, slot as int, stride as int)
                == Option::<int>::None,
        },
{
    match user_checked_mul(slot, stride) {
        Some(offset) => user_checked_end(base, offset),
        None => None,
    }
}

fn driver_virtio_identity_valid(
    magic: u32,
    device_id: u32,
    expected_device: u32,
    vendor: u32,
    expected_vendor: u32,
) -> (out: bool)
    ensures
        out == driver_virtio_identity_valid_spec(
            magic as int,
            device_id as int,
            expected_device as int,
            vendor as int,
            expected_vendor as int,
        ),
{
    smros_driver_virtio_identity_valid_body!(
        magic,
        device_id,
        expected_device,
        vendor,
        expected_vendor
    )
}

fn driver_virtio_version_supported(version: u32, legacy: u32, modern: u32) -> (out: bool)
    ensures
        out == driver_virtio_version_supported_spec(
            version as int,
            legacy as int,
            modern as int,
        ),
{
    smros_driver_virtio_version_supported_body!(version, legacy, modern)
}

fn driver_virtio_version_is_modern(version: u32, modern: u32) -> (out: bool)
    ensures
        out == driver_virtio_version_is_modern_spec(version as int, modern as int),
{
    smros_driver_virtio_version_is_modern_body!(version, modern)
}

fn driver_virtio_queue_size_valid(max_queue: u32, queue_size: u16) -> (out: bool)
    ensures
        out == driver_virtio_queue_size_valid_spec(max_queue as int, queue_size as int),
{
    smros_driver_virtio_queue_size_valid_body!(max_queue, queue_size)
}

fn driver_virtio_feature_present(features: u64, feature: u64) -> (out: bool)
    ensures
        out == (features & feature != 0),
{
    smros_driver_virtio_feature_present_body!(features, feature)
}

fn driver_virtio_block_accepted_features(
    features: u64,
    flush: u64,
    config_wce: u64,
) -> (out: u64)
    ensures
        out == driver_virtio_block_accepted_features_spec(features, flush, config_wce),
{
    smros_driver_virtio_block_accepted_features_body!(features, flush, config_wce)
}

fn driver_virtio_driver_features(accepted: u64, version_1: u64, modern: bool) -> (out: u64)
    ensures
        out == driver_virtio_driver_features_spec(accepted, version_1, modern),
{
    smros_driver_virtio_driver_features_body!(accepted, version_1, modern)
}

fn driver_virtio_net_accepted_features(
    features: u64,
    mac: u64,
    status: u64,
    version_1: u64,
    modern: bool,
) -> (out: u64)
    ensures
        out == driver_virtio_net_accepted_features_spec(features, mac, status, version_1, modern),
{
    smros_driver_virtio_net_accepted_features_body!(features, mac, status, version_1, modern)
}

fn driver_block_capacity_bytes(blocks: usize, block_size: usize) -> (out: usize)
    ensures
        out as int == driver_block_capacity_bytes_spec(blocks as int, block_size as int),
{
    match user_checked_mul(blocks, block_size) {
        Some(bytes) => bytes,
        None => usize::MAX,
    }
}

fn driver_block_range_valid(
    offset: usize,
    len: usize,
    blocks: usize,
    block_size: usize,
) -> (out: bool)
    ensures
        out == driver_block_range_valid_spec(
            offset as int,
            len as int,
            blocks as int,
            block_size as int,
        ),
{
    match user_checked_mul(blocks, block_size) {
        Some(capacity) => match user_checked_end(offset, len) {
            Some(end) => end <= capacity,
            None => false,
        },
        None => false,
    }
}

fn driver_block_len_valid(len: usize, block_size: usize) -> (out: bool)
    ensures
        out == driver_block_len_valid_spec(len as int, block_size as int),
{
    smros_driver_block_len_valid_body!(len, block_size)
}

fn driver_block_id_valid(block: usize, capacity_blocks: usize) -> (out: bool)
    ensures
        out == driver_block_id_valid_spec(block as int, capacity_blocks as int),
{
    smros_driver_block_id_valid_body!(block, capacity_blocks)
}

fn driver_net_tx_frame_len_valid(
    frame_len: usize,
    max_frame: usize,
    header_len: usize,
    buffer_size: usize,
) -> (out: bool)
    ensures
        out == driver_net_tx_frame_len_valid_spec(
            frame_len as int,
            max_frame as int,
            header_len as int,
            buffer_size as int,
        ),
{
    smros_driver_net_tx_frame_len_valid_body!(frame_len, max_frame, header_len, buffer_size)
}

fn driver_net_rx_packet_len_valid(
    packet_len: usize,
    header_len: usize,
    buffer_size: usize,
) -> (out: bool)
    ensures
        out == driver_net_rx_packet_len_valid_spec(
            packet_len as int,
            header_len as int,
            buffer_size as int,
        ),
{
    smros_driver_net_rx_packet_len_valid_body!(packet_len, header_len, buffer_size)
}

fn driver_net_rx_frame_len(packet_len: usize, header_len: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(frame_len) => driver_net_rx_frame_len_spec(packet_len as int, header_len as int)
                == Some(frame_len as int),
            None => driver_net_rx_frame_len_spec(packet_len as int, header_len as int)
                == Option::<int>::None,
        },
{
    smros_driver_net_rx_frame_len_body!(packet_len, header_len)
}

fn driver_net_rx_output_len_valid(frame_len: usize, out_len: usize) -> (out: bool)
    ensures
        out == driver_net_rx_output_len_valid_spec(frame_len as int, out_len as int),
{
    smros_driver_net_rx_output_len_valid_body!(frame_len, out_len)
}

fn user_el0_thread_pstate() -> (out: u64)
    ensures
        out == 0x3C0u64,
{
    smros_user_el0_thread_pstate_body!()
}

fn user_el0_spsr() -> (out: u64)
    ensures
        out == 0u64,
{
    smros_user_el0_spsr_body!()
}

fn user_el1h_spsr_masked() -> (out: u64)
    ensures
        out == 0x3C5u64,
{
    smros_user_el1h_spsr_masked_body!()
}

fn user_syscall_should_advance_elr() -> (out: u64)
    ensures
        out == 0u64,
{
    smros_user_syscall_should_advance_elr_body!()
}

fn user_ascii_shell_input(byte: u8) -> (out: bool)
    ensures
        out == ascii_shell_input_spec(byte as int),
{
    smros_user_ascii_shell_input_body!(byte)
}

fn user_decimal_digit_value(byte: u8) -> (out: Option<usize>)
    ensures
        match out {
            Some(digit) => decimal_digit_value_spec(byte as int) == Some(digit as int),
            None => decimal_digit_value_spec(byte as int) == Option::<int>::None,
        },
{
    smros_user_decimal_digit_value_body!(byte)
}

fn user_parse_digit_step(result: usize, digit: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(next) => parse_digit_step_spec(result as int, digit as int) == Some(next as int),
            None => parse_digit_step_spec(result as int, digit as int) == Option::<int>::None,
        },
{
    smros_user_parse_digit_step_body!(result, digit)
}

fn user_ipv4_octet_step(value: u32, digit: u32) -> (out: Option<u32>)
    ensures
        match out {
            Some(next) => ipv4_octet_step_spec(value as int, digit as int) == Some(next as int),
            None => ipv4_octet_step_spec(value as int, digit as int) == Option::<int>::None,
        },
{
    smros_user_ipv4_octet_step_body!(value, digit)
}

fn user_saturating_sub(lhs: usize, rhs: usize) -> (out: usize)
    ensures
        out as int == saturating_sub_spec(lhs as int, rhs as int),
{
    smros_user_saturating_sub_body!(lhs, rhs)
}

fn user_pages_to_kb(pages: usize, page_size: usize) -> (out: usize)
    requires
        page_size > 0,
        pages as int <= usize::MAX as int / page_size as int,
    ensures
        out as int == pages_to_kb_spec(pages as int, page_size as int),
{
    let out = smros_user_pages_to_kb_body!(pages, page_size);
    assert((pages as int) * (page_size as int) <= usize::MAX as int) by (nonlinear_arith)
        requires
            page_size as int > 0,
            pages as int <= usize::MAX as int / page_size as int,
    ;
    out
}

fn user_usage_percent(used_pages: usize, total_pages: usize) -> (out: usize)
    ensures
        out as int == usage_percent_spec(used_pages as int, total_pages as int),
{
    smros_user_usage_percent_body!(used_pages, total_pages)
}

fn user_uptime_parts(ticks: u64) -> (out: (u64, u64, u64, u64))
    ensures
        out.0 as int == uptime_parts_spec(ticks as int).0,
        out.1 as int == uptime_parts_spec(ticks as int).1,
        out.2 as int == uptime_parts_spec(ticks as int).2,
        out.3 as int == uptime_parts_spec(ticks as int).3,
{
    smros_user_uptime_parts_body!(ticks)
}

fn user_mmap_result_ok(addr: u64) -> (out: bool)
    ensures
        out == mmap_result_ok_spec(
            addr as int,
            PAGE_SIZE as int,
            USER_MMAP_BASE as int,
            USER_MMAP_LIMIT as int,
        ),
{
    smros_user_mmap_result_ok_body!(addr, PAGE_SIZE as u64, USER_MMAP_BASE, USER_MMAP_LIMIT)
}

fn user_dns_host_len_valid(len: usize, max_len: usize) -> (out: bool)
    ensures
        out == dns_host_len_valid_spec(len as int, max_len as int),
{
    smros_user_dns_host_len_valid_body!(len, max_len)
}

fn user_dns_label_len_valid(len: usize, max_len: usize) -> (out: bool)
    ensures
        out == dns_label_len_valid_spec(len as int, max_len as int),
{
    smros_user_dns_label_len_valid_body!(len, max_len)
}

fn user_dns_label_byte_valid(byte: u8) -> (out: bool)
    ensures
        out == dns_label_byte_valid_spec(byte as int),
{
    smros_user_dns_label_byte_valid_body!(byte)
}

fn user_kernel_success(
    kernel_entered: bool,
    kernel_finished: bool,
    exit_code: i32,
    kernel_write: u64,
    kernel_pid: u64,
    kernel_mmap: u64,
    banner_len: usize,
) -> (out: bool)
    ensures
        out == kernel_success_spec(
            kernel_entered,
            kernel_finished,
            exit_code as int,
            kernel_write as int,
            kernel_pid as int,
            kernel_mmap as int,
            banner_len as int,
        ),
{
    smros_user_kernel_success_body!(
        kernel_entered,
        kernel_finished,
        exit_code,
        kernel_write,
        kernel_pid,
        kernel_mmap,
        banner_len
    )
}

fn user_component_start_allowed(
    binary_exists: bool,
    destroyed: bool,
    already_started: bool,
) -> (out: bool)
    ensures
        out == component_start_allowed_spec(binary_exists, destroyed, already_started),
{
    smros_user_component_start_allowed_body!(binary_exists, destroyed, already_started)
}

fn user_namespace_rights_valid(rights: u32, allowed_mask: u32) -> (out: bool)
    ensures
        out == (rights & !allowed_mask == 0),
{
    smros_user_namespace_rights_valid_body!(rights, allowed_mask)
}

fn user_fxfs_file_size_valid(size: usize, max_size: usize) -> (out: bool)
    ensures
        out == fxfs_file_size_valid_spec(size as int, max_size as int),
{
    smros_user_fxfs_file_size_valid_body!(size, max_size)
}

fn user_fxfs_node_capacity_valid(nodes: usize, max_nodes: usize) -> (out: bool)
    ensures
        out == fxfs_node_capacity_valid_spec(nodes as int, max_nodes as int),
{
    smros_user_fxfs_node_capacity_valid_body!(nodes, max_nodes)
}

fn user_fxfs_dirent_capacity_valid(entries: usize, max_entries: usize) -> (out: bool)
    ensures
        out == fxfs_dirent_capacity_valid_spec(entries as int, max_entries as int),
{
    smros_user_fxfs_dirent_capacity_valid_body!(entries, max_entries)
}

fn user_fxfs_append_size(old_size: usize, append_len: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(size) => fxfs_append_size_spec(old_size as int, append_len as int)
                == Some(size as int),
            None => fxfs_append_size_spec(old_size as int, append_len as int)
                == Option::<int>::None,
        },
{
    smros_user_fxfs_append_size_body!(old_size, append_len)
}

fn user_fxfs_write_end(offset: usize, len: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(end) => fxfs_write_end_spec(offset as int, len as int) == Some(end as int),
            None => fxfs_write_end_spec(offset as int, len as int) == Option::<int>::None,
        },
{
    smros_user_fxfs_write_end_body!(offset, len)
}

fn user_fxfs_seek_valid(offset: usize, size: usize) -> (out: bool)
    ensures
        out == fxfs_seek_valid_spec(offset as int, size as int),
{
    smros_user_fxfs_seek_valid_body!(offset, size)
}

fn user_fxfs_replay_count_valid(replayed: usize, journal_records: usize) -> (out: bool)
    ensures
        out == fxfs_replay_count_valid_spec(replayed as int, journal_records as int),
{
    smros_user_fxfs_replay_count_valid_body!(replayed, journal_records)
}

fn user_svc_name_valid(len: usize, max_len: usize) -> (out: bool)
    ensures
        out == svc_name_valid_spec(len as int, max_len as int),
{
    smros_user_svc_name_valid_body!(len, max_len)
}

fn user_svc_rights_valid(rights: u32, allowed_mask: u32) -> (out: bool)
    ensures
        out == (rights != 0 && (rights & !allowed_mask) == 0),
{
    smros_user_svc_rights_valid_body!(rights, allowed_mask)
}

fn user_svc_ipc_message_size_valid(size: usize, expected: usize) -> (out: bool)
    ensures
        out == svc_ipc_message_size_valid_spec(size as int, expected as int),
{
    smros_user_svc_ipc_message_size_valid_body!(size, expected)
}

fn user_svc_ipc_header_valid(
    magic: u32,
    version: u16,
    expected_magic: u32,
    expected_version: u16,
) -> (out: bool)
    ensures
        out == svc_ipc_header_valid_spec(
            magic as int,
            version as int,
            expected_magic as int,
            expected_version as int,
        ),
{
    smros_user_svc_ipc_header_valid_body!(magic, version, expected_magic, expected_version)
}

fn user_svc_protocol_allowed(
    service: u16,
    ordinal: u16,
    component_manager: u16,
    runner: u16,
    filesystem: u16,
    component_start: u16,
    runner_load: u16,
    filesystem_describe: u16,
) -> (out: bool)
    ensures
        out == svc_protocol_allowed_spec(
            service as int,
            ordinal as int,
            component_manager as int,
            runner as int,
            filesystem as int,
            component_start as int,
            runner_load as int,
            filesystem_describe as int,
        ),
{
    smros_user_svc_protocol_allowed_body!(
        service,
        ordinal,
        component_manager,
        runner,
        filesystem,
        component_start,
        runner_load,
        filesystem_describe
    )
}

fn user_component_thread_launch_valid(
    process_created: bool,
    queued: bool,
    thread_created: bool,
) -> (out: bool)
    ensures
        out == component_thread_launch_valid_spec(process_created, queued, thread_created),
{
    smros_user_component_thread_launch_valid_body!(process_created, queued, thread_created)
}

fn user_component_return_active(pid: usize) -> (out: bool)
    ensures
        out == component_return_active_spec(pid as int),
{
    smros_user_component_return_active_body!(pid)
}

fn user_elf_header_bounds_valid(image_len: usize, header_size: usize) -> (out: bool)
    ensures
        out == elf_header_bounds_valid_spec(image_len as int, header_size as int),
{
    smros_user_elf_header_bounds_valid_body!(image_len, header_size)
}

fn user_elf_magic_valid(b0: u8, b1: u8, b2: u8, b3: u8) -> (out: bool)
    ensures
        out == elf_magic_valid_spec(b0 as int, b1 as int, b2 as int, b3 as int),
{
    smros_user_elf_magic_valid_body!(b0, b1, b2, b3)
}

fn user_elf_class_data_valid(class: u8, data: u8, version: u8) -> (out: bool)
    ensures
        out == elf_class_data_valid_spec(class as int, data as int, version as int),
{
    smros_user_elf_class_data_valid_body!(class, data, version)
}

fn user_elf_type_valid(elf_type: u16, exec_type: u16, dyn_type: u16) -> (out: bool)
    ensures
        out == elf_type_valid_spec(elf_type as int, exec_type as int, dyn_type as int),
{
    smros_user_elf_type_valid_body!(elf_type, exec_type, dyn_type)
}

fn user_elf_machine_valid(machine: u16, expected: u16) -> (out: bool)
    ensures
        out == elf_machine_valid_spec(machine as int, expected as int),
{
    smros_user_elf_machine_valid_body!(machine, expected)
}

fn user_elf_entry_valid(entry: u64) -> (out: bool)
    ensures
        out == elf_entry_valid_spec(entry as int),
{
    smros_user_elf_entry_valid_body!(entry)
}

fn user_elf_phdr_table_valid(
    phoff: usize,
    phentsize: usize,
    phnum: usize,
    image_len: usize,
    expected_phentsize: usize,
    max_phnum: usize,
) -> (out: bool)
    ensures
        out == elf_phdr_table_valid_spec(
            phoff as int,
            phentsize as int,
            phnum as int,
            image_len as int,
            expected_phentsize as int,
            max_phnum as int,
        ),
{
    if phentsize != expected_phentsize || phnum == 0 || phnum > max_phnum {
        return false;
    }

    match user_checked_mul(phentsize, phnum) {
        Some(table_size) => match user_checked_end(phoff, table_size) {
            Some(end) => end <= image_len,
            None => false,
        },
        None => false,
    }
}

fn user_elf_segment_bounds_valid(
    offset: usize,
    file_size: usize,
    mem_size: usize,
    image_len: usize,
) -> (out: bool)
    ensures
        out == elf_segment_bounds_valid_spec(
            offset as int,
            file_size as int,
            mem_size as int,
            image_len as int,
        ),
{
    smros_user_elf_segment_bounds_valid_body!(offset, file_size, mem_size, image_len)
}

fn user_elf_vaddr_range_valid(vaddr: u64, mem_size: u64) -> (out: bool)
    ensures
        out == elf_vaddr_range_valid_spec(vaddr as int, mem_size as int),
{
    smros_user_elf_vaddr_range_valid_body!(vaddr, mem_size)
}

fn user_elf_segment_mapping_range(
    vaddr: usize,
    mem_size: usize,
    page_size: usize,
) -> (out: Option<(usize, usize)>)
    ensures
        match out {
            Some((start, end)) => elf_segment_mapping_range_spec(
                vaddr as int,
                mem_size as int,
                page_size as int,
            ) == Some((start as int, end as int)),
            None => elf_segment_mapping_range_spec(
                vaddr as int,
                mem_size as int,
                page_size as int,
            ) == Option::<(int, int)>::None,
        },
{
    smros_user_elf_segment_mapping_range_body!(vaddr, mem_size, page_size)
}

fn find_empty_user_process_slot(slots: &Vec<UserProcessSlotModel>) -> (out: Option<usize>)
    ensures
        match out {
            Some(i) => i < slots.len()
                && !slots@[i as int].occupied
                && forall|j: int| 0 <= j < i as int ==> slots@[j].occupied,
            None => forall|i: int| 0 <= i < slots@.len() ==> slots@[i].occupied,
        },
{
    let mut i = 0usize;
    while i < slots.len()
        invariant
            i <= slots.len(),
            forall|j: int| 0 <= j < i as int ==> slots@[j].occupied,
        decreases slots.len() - i,
    {
        if !slots[i].occupied {
            return Some(i);
        }
        i += 1;
    }

    assert forall|j: int| 0 <= j < slots@.len() implies slots@[j].occupied by {
        assert(j < i as int);
    };
    None
}

fn find_user_process_by_pid(slots: &Vec<UserProcessSlotModel>, pid: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(i) => i < slots.len() && slots@[i as int].occupied && slots@[i as int].pid == pid,
            None => forall|i: int|
                0 <= i < slots@.len() ==> !(slots@[i].occupied && slots@[i].pid == pid),
        },
{
    let mut i = 0usize;
    while i < slots.len()
        invariant
            i <= slots.len(),
            forall|j: int|
                0 <= j < i as int ==> !(slots@[j].occupied && slots@[j].pid == pid),
        decreases slots.len() - i,
    {
        if slots[i].occupied && slots[i].pid == pid {
            return Some(i);
        }
        i += 1;
    }

    assert forall|j: int|
        0 <= j < slots@.len() implies !(slots@[j].occupied && slots@[j].pid == pid) by {
        assert(j < i as int);
    };
    None
}

fn user_test_exit_code_model(
    write_result: u64,
    pid: u64,
    getpid_log_write: u64,
    mmap_addr: u64,
    mmap_log_write: u64,
    complete_log_write: u64,
) -> (out: i32)
    ensures
        out as int == user_test_exit_code_spec(
            write_result as int,
            pid as int,
            getpid_log_write as int,
            mmap_addr as int,
            mmap_log_write as int,
            complete_log_write as int,
        ),
{
    if write_result != EL0_TEST_BANNER_LEN as u64 {
        EL0_TEST_EXIT_WRITE_RESULT_MISMATCH
    } else if pid != 1 {
        EL0_TEST_EXIT_GETPID_RESULT_MISMATCH
    } else if getpid_log_write != EL0_TEST_INFO_GETPID_LEN as u64 {
        EL0_TEST_EXIT_GETPID_LOG_WRITE_MISMATCH
    } else if !user_mmap_result_ok(mmap_addr) {
        EL0_TEST_EXIT_MMAP_RESULT_MISMATCH
    } else if mmap_log_write != EL0_TEST_INFO_MMAP_LEN as u64 {
        EL0_TEST_EXIT_MMAP_LOG_WRITE_MISMATCH
    } else if complete_log_write != EL0_TEST_COMPLETE_LEN as u64 {
        EL0_TEST_EXIT_COMPLETE_LOG_WRITE_MISMATCH
    } else {
        EL0_TEST_EXIT_OK
    }
}

fn record_el0_kernel_syscall_result_model(
    active: bool,
    syscall_num: u32,
    result: u64,
    old: El0KernelObservation,
) -> (out: El0KernelObservation)
    ensures
        !active ==> out.entered == old.entered
            && out.write_result == old.write_result
            && out.pid == old.pid
            && out.mmap_addr == old.mmap_addr,
        active ==> out.entered,
        active && syscall_num == SYS_WRITE && old.write_result == 0 ==> out.write_result == result,
        active && syscall_num == SYS_WRITE && old.write_result != 0 ==> out.write_result == old.write_result,
        active && syscall_num == SYS_GETPID ==> out.pid == result,
        active && syscall_num == SYS_MMAP ==> out.mmap_addr == result,
{
    if !active {
        return old;
    }

    let mut next = old;
    next.entered = true;
    if syscall_num == SYS_WRITE {
        if next.write_result == 0 {
            next.write_result = result;
        }
    } else if syscall_num == SYS_GETPID {
        next.pid = result;
    } else if syscall_num == SYS_MMAP {
        next.mmap_addr = result;
    }
    next
}

proof fn main_rs_allocator_smoke() {
    assert(align_up_spec(0int, 16int) == Some(0int));
    assert(align_up_spec(1int, 16int) == Some(16int));
    assert(bump_alloc_next_spec(1int, 8int, 16int, KERNEL_HEAP_SIZE as int) == Some((16int, 24int)));
    assert(bump_alloc_next_spec(
        KERNEL_HEAP_SIZE as int,
        1int,
        16int,
        KERNEL_HEAP_SIZE as int,
    ) == Option::<(int, int)>::None);
}

proof fn user_level_mod_has_no_extra_pure_runtime_obligation()
    ensures
        USER_PROCESS_CAPACITY == 16,
        USER_INIT_PARENT_PID == 1,
{
}

proof fn user_process_layout_smoke() {
    assert(USER_CODE_VADDR == 0);
    assert(USER_DATA_VADDR == PAGE_SIZE);
    assert(USER_HEAP_VADDR == 2 * PAGE_SIZE);
    assert(USER_HEAP_PAGES == 4);
    assert(USER_STACK_PAGES == 2);
    assert(page_offset_spec(USER_STACK_VADDR as int, USER_STACK_PAGES as int, PAGE_SIZE as int)
        == Some((USER_STACK_VADDR + USER_STACK_PAGES * PAGE_SIZE) as int));
    assert(USER_THREAD_TIME_SLICE == 10);
    assert(0x3C0u64 == smros_user_el0_thread_pstate_body!());
    assert(0u64 == smros_user_el0_spsr_body!());
}

fn user_driver_logic_smoke() {
    let slot0 = driver_mmio_slot_base(
        USER_DRIVER_MMIO_BASE,
        0,
        USER_DRIVER_MMIO_STRIDE,
    );
    let slot31 = driver_mmio_slot_base(
        USER_DRIVER_MMIO_BASE,
        USER_DRIVER_MMIO_SLOT_COUNT - 1,
        USER_DRIVER_MMIO_STRIDE,
    );
    let slot_overflow = driver_mmio_slot_base(usize::MAX, 1, 1);
    let block_identity_ok = driver_virtio_identity_valid(
        USER_DRIVER_VIRTIO_MAGIC,
        USER_DRIVER_VIRTIO_DEVICE_BLOCK,
        USER_DRIVER_VIRTIO_DEVICE_BLOCK,
        USER_DRIVER_VIRTIO_VENDOR_QEMU,
        USER_DRIVER_VIRTIO_VENDOR_QEMU,
    );
    let net_identity_ok = driver_virtio_identity_valid(
        USER_DRIVER_VIRTIO_MAGIC,
        USER_DRIVER_VIRTIO_DEVICE_NET,
        USER_DRIVER_VIRTIO_DEVICE_NET,
        USER_DRIVER_VIRTIO_VENDOR_QEMU,
        USER_DRIVER_VIRTIO_VENDOR_QEMU,
    );
    let identity_bad = driver_virtio_identity_valid(
        0,
        USER_DRIVER_VIRTIO_DEVICE_BLOCK,
        USER_DRIVER_VIRTIO_DEVICE_BLOCK,
        USER_DRIVER_VIRTIO_VENDOR_QEMU,
        USER_DRIVER_VIRTIO_VENDOR_QEMU,
    );
    let legacy_ok = driver_virtio_version_supported(
        USER_DRIVER_VIRTIO_VERSION_LEGACY,
        USER_DRIVER_VIRTIO_VERSION_LEGACY,
        USER_DRIVER_VIRTIO_VERSION_MODERN,
    );
    let modern_ok = driver_virtio_version_supported(
        USER_DRIVER_VIRTIO_VERSION_MODERN,
        USER_DRIVER_VIRTIO_VERSION_LEGACY,
        USER_DRIVER_VIRTIO_VERSION_MODERN,
    );
    let version_bad = driver_virtio_version_supported(
        3,
        USER_DRIVER_VIRTIO_VERSION_LEGACY,
        USER_DRIVER_VIRTIO_VERSION_MODERN,
    );
    let is_modern = driver_virtio_version_is_modern(
        USER_DRIVER_VIRTIO_VERSION_MODERN,
        USER_DRIVER_VIRTIO_VERSION_MODERN,
    );
    let queue_ok = driver_virtio_queue_size_valid(8, USER_DRIVER_VIRTIO_QUEUE_SIZE);
    let queue_bad = driver_virtio_queue_size_valid(7, USER_DRIVER_VIRTIO_QUEUE_SIZE);
    let block_features = driver_virtio_block_accepted_features(
        USER_DRIVER_VIRTIO_BLK_F_FLUSH | USER_DRIVER_VIRTIO_F_VERSION_1,
        USER_DRIVER_VIRTIO_BLK_F_FLUSH,
        USER_DRIVER_VIRTIO_BLK_F_CONFIG_WCE,
    );
    let block_features_modern = driver_virtio_driver_features(
        block_features,
        USER_DRIVER_VIRTIO_F_VERSION_1,
        true,
    );
    let net_features = driver_virtio_net_accepted_features(
        USER_DRIVER_VIRTIO_NET_F_MAC | USER_DRIVER_VIRTIO_NET_F_STATUS,
        USER_DRIVER_VIRTIO_NET_F_MAC,
        USER_DRIVER_VIRTIO_NET_F_STATUS,
        USER_DRIVER_VIRTIO_F_VERSION_1,
        true,
    );
    let status_present = driver_virtio_feature_present(
        USER_DRIVER_VIRTIO_NET_F_STATUS,
        USER_DRIVER_VIRTIO_NET_F_STATUS,
    );
    let capacity = driver_block_capacity_bytes(4, USER_DRIVER_BLOCK_SIZE);
    let capacity_overflow = driver_block_capacity_bytes(usize::MAX, 2);
    let range_ok = driver_block_range_valid(512, 512, 4, USER_DRIVER_BLOCK_SIZE);
    let range_bad_end = driver_block_range_valid(2048, 1, 4, USER_DRIVER_BLOCK_SIZE);
    let range_bad_capacity = driver_block_range_valid(0, 1, usize::MAX, 2);
    let block_len_ok = driver_block_len_valid(USER_DRIVER_BLOCK_SIZE, USER_DRIVER_BLOCK_SIZE);
    let block_len_bad = driver_block_len_valid(1, USER_DRIVER_BLOCK_SIZE);
    let block_id_ok = driver_block_id_valid(3, 4);
    let block_id_bad = driver_block_id_valid(4, 4);
    let tx_ok = driver_net_tx_frame_len_valid(
        USER_DRIVER_ETHERNET_FRAME_MAX,
        USER_DRIVER_ETHERNET_FRAME_MAX,
        USER_DRIVER_NET_HDR_LEN,
        USER_DRIVER_NET_BUFFER_SIZE,
    );
    let tx_bad = driver_net_tx_frame_len_valid(
        USER_DRIVER_ETHERNET_FRAME_MAX + 1,
        USER_DRIVER_ETHERNET_FRAME_MAX,
        USER_DRIVER_NET_HDR_LEN,
        USER_DRIVER_NET_BUFFER_SIZE,
    );
    let rx_packet_ok = driver_net_rx_packet_len_valid(
        USER_DRIVER_NET_HDR_LEN + 64,
        USER_DRIVER_NET_HDR_LEN,
        USER_DRIVER_NET_BUFFER_SIZE,
    );
    let rx_packet_bad = driver_net_rx_packet_len_valid(
        USER_DRIVER_NET_HDR_LEN - 1,
        USER_DRIVER_NET_HDR_LEN,
        USER_DRIVER_NET_BUFFER_SIZE,
    );
    let rx_frame = driver_net_rx_frame_len(USER_DRIVER_NET_HDR_LEN + 64, USER_DRIVER_NET_HDR_LEN);
    let rx_frame_bad = driver_net_rx_frame_len(USER_DRIVER_NET_HDR_LEN - 1, USER_DRIVER_NET_HDR_LEN);
    let rx_out_ok = driver_net_rx_output_len_valid(64, 64);
    let rx_out_bad = driver_net_rx_output_len_valid(65, 64);

    assert(slot0 == Some(USER_DRIVER_MMIO_BASE));
    assert(slot31 == Some((USER_DRIVER_MMIO_BASE + 31 * USER_DRIVER_MMIO_STRIDE) as usize));
    assert(slot_overflow == Option::<usize>::None);
    assert(block_identity_ok);
    assert(net_identity_ok);
    assert(!identity_bad);
    assert(legacy_ok);
    assert(modern_ok);
    assert(!version_bad);
    assert(is_modern);
    assert(queue_ok);
    assert(!queue_bad);
    assert(((USER_DRIVER_VIRTIO_BLK_F_FLUSH | USER_DRIVER_VIRTIO_F_VERSION_1)
        & (USER_DRIVER_VIRTIO_BLK_F_FLUSH | USER_DRIVER_VIRTIO_BLK_F_CONFIG_WCE))
        == USER_DRIVER_VIRTIO_BLK_F_FLUSH) by(bit_vector);
    assert(block_features == USER_DRIVER_VIRTIO_BLK_F_FLUSH);
    assert(block_features_modern == USER_DRIVER_VIRTIO_BLK_F_FLUSH
        | USER_DRIVER_VIRTIO_F_VERSION_1);
    assert(((USER_DRIVER_VIRTIO_NET_F_MAC | USER_DRIVER_VIRTIO_NET_F_STATUS)
        & USER_DRIVER_VIRTIO_NET_F_MAC) != 0u64) by(bit_vector);
    assert(((USER_DRIVER_VIRTIO_NET_F_MAC | USER_DRIVER_VIRTIO_NET_F_STATUS)
        & USER_DRIVER_VIRTIO_NET_F_STATUS) != 0u64) by(bit_vector);
    assert((0u64 | USER_DRIVER_VIRTIO_NET_F_MAC) | USER_DRIVER_VIRTIO_NET_F_STATUS
        | USER_DRIVER_VIRTIO_F_VERSION_1 == USER_DRIVER_VIRTIO_NET_F_MAC
        | USER_DRIVER_VIRTIO_NET_F_STATUS
        | USER_DRIVER_VIRTIO_F_VERSION_1) by(bit_vector);
    assert(net_features == USER_DRIVER_VIRTIO_NET_F_MAC
        | USER_DRIVER_VIRTIO_NET_F_STATUS
        | USER_DRIVER_VIRTIO_F_VERSION_1);
    assert((USER_DRIVER_VIRTIO_NET_F_STATUS & USER_DRIVER_VIRTIO_NET_F_STATUS) != 0u64)
        by(bit_vector);
    assert(status_present
        == ((USER_DRIVER_VIRTIO_NET_F_STATUS & USER_DRIVER_VIRTIO_NET_F_STATUS) != 0u64));
    assert(status_present);
    assert(capacity == 2048);
    assert(capacity_overflow == usize::MAX);
    assert(range_ok);
    assert(!range_bad_end);
    assert(!range_bad_capacity);
    assert(block_len_ok);
    assert(!block_len_bad);
    assert(block_id_ok);
    assert(!block_id_bad);
    assert(tx_ok);
    assert(!tx_bad);
    assert(rx_packet_ok);
    assert(!rx_packet_bad);
    assert(rx_frame == Some(64usize));
    assert(rx_frame_bad == Option::<usize>::None);
    assert(rx_out_ok);
    assert(!rx_out_bad);
}

proof fn user_shell_logic_smoke() {
    assert(ascii_shell_input_spec(0x20));
    assert(ascii_shell_input_spec(0x7e));
    assert(!ascii_shell_input_spec(0x1f));
    assert(decimal_digit_value_spec(48int) == Some(0int));
    assert(decimal_digit_value_spec(57int) == Some(9int));
    assert(decimal_digit_value_spec(65int) == Option::<int>::None);
    assert(parse_digit_step_spec(12int, 3int) == Some(123int));
    assert(saturating_sub_spec(3int, 5int) == 0);
    assert(saturating_sub_spec(8int, 3int) == 5);
    assert(pages_to_kb_spec(2int, PAGE_SIZE as int) == 8);
    assert(usage_percent_spec(25int, 100int) == 25);
    assert(uptime_parts_spec(86400int * 100int).3 == 1);
    assert(ipv4_octet_step_spec(19int, 2int) == Some(192int));
    assert(ipv4_octet_step_spec(25int, 6int) == Option::<int>::None);
    assert(dns_host_len_valid_spec(1int, 253int));
    assert(!dns_host_len_valid_spec(0int, 253int));
    assert(dns_label_len_valid_spec(63int, 63int));
    assert(!dns_label_len_valid_spec(64int, 63int));
    assert(dns_label_byte_valid_spec(0x61int));
    assert(dns_label_byte_valid_spec(0x5aint));
    assert(dns_label_byte_valid_spec(0x39int));
    assert(dns_label_byte_valid_spec(0x2dint));
    assert(!dns_label_byte_valid_spec(0x5fint));
}

fn user_component_fxfs_exec_smoke() {
    let start_ok = user_component_start_allowed(true, false, false);
    let start_existing = user_component_start_allowed(false, true, true);
    let start_missing = user_component_start_allowed(false, false, false);
    let rights_ok = user_namespace_rights_valid(0x7, USER_NAMESPACE_RIGHTS_MASK);
    let rights_bad = user_namespace_rights_valid(0x8, USER_NAMESPACE_RIGHTS_MASK);
    let file_size_ok = user_fxfs_file_size_valid(
        USER_FXFS_MAX_FILE_BYTES,
        USER_FXFS_MAX_FILE_BYTES,
    );
    let file_size_bad = user_fxfs_file_size_valid(
        USER_FXFS_MAX_FILE_BYTES + 1,
        USER_FXFS_MAX_FILE_BYTES,
    );
    let nodes_ok = user_fxfs_node_capacity_valid(
        USER_FXFS_MAX_NODES - 1,
        USER_FXFS_MAX_NODES,
    );
    let nodes_bad = user_fxfs_node_capacity_valid(USER_FXFS_MAX_NODES, USER_FXFS_MAX_NODES);
    let dirents_ok =
        user_fxfs_dirent_capacity_valid(USER_FXFS_MAX_DIRENTS - 1, USER_FXFS_MAX_DIRENTS);
    let dirents_bad =
        user_fxfs_dirent_capacity_valid(USER_FXFS_MAX_DIRENTS, USER_FXFS_MAX_DIRENTS);
    let append_ok = user_fxfs_append_size(10, 5);
    let append_bad = user_fxfs_append_size(usize::MAX, 1);
    let write_end_ok = user_fxfs_write_end(4, 6);
    let write_end_bad = user_fxfs_write_end(usize::MAX, 1);
    let seek_ok = user_fxfs_seek_valid(10, 10);
    let seek_bad = user_fxfs_seek_valid(11, 10);
    let replay_ok = user_fxfs_replay_count_valid(7, 8);
    let replay_bad = user_fxfs_replay_count_valid(9, 8);
    let svc_name_ok = user_svc_name_valid(32, USER_SVC_MAX_NAME_LEN);
    let svc_name_bad = user_svc_name_valid(0, USER_SVC_MAX_NAME_LEN);
    let svc_rights_ok = user_svc_rights_valid(0x3, USER_SVC_RIGHTS_MASK);
    let svc_rights_bad = user_svc_rights_valid(0x4, USER_SVC_RIGHTS_MASK);
    let svc_size_ok =
        user_svc_ipc_message_size_valid(USER_SVC_IPC_MESSAGE_SIZE, USER_SVC_IPC_MESSAGE_SIZE);
    let svc_size_bad =
        user_svc_ipc_message_size_valid(USER_SVC_IPC_MESSAGE_SIZE - 1, USER_SVC_IPC_MESSAGE_SIZE);
    let svc_header_ok = user_svc_ipc_header_valid(
        USER_SVC_IPC_MAGIC,
        USER_SVC_IPC_VERSION,
        USER_SVC_IPC_MAGIC,
        USER_SVC_IPC_VERSION,
    );
    let svc_header_bad = user_svc_ipc_header_valid(
        0,
        USER_SVC_IPC_VERSION,
        USER_SVC_IPC_MAGIC,
        USER_SVC_IPC_VERSION,
    );
    let svc_component_ok = user_svc_protocol_allowed(
        USER_SVC_COMPONENT_MANAGER,
        USER_SVC_COMPONENT_START,
        USER_SVC_COMPONENT_MANAGER,
        USER_SVC_RUNNER,
        USER_SVC_FILESYSTEM,
        USER_SVC_COMPONENT_START,
        USER_SVC_RUNNER_LOAD_ELF,
        USER_SVC_FILESYSTEM_DESCRIBE,
    );
    let svc_runner_ok = user_svc_protocol_allowed(
        USER_SVC_RUNNER,
        USER_SVC_RUNNER_LOAD_ELF,
        USER_SVC_COMPONENT_MANAGER,
        USER_SVC_RUNNER,
        USER_SVC_FILESYSTEM,
        USER_SVC_COMPONENT_START,
        USER_SVC_RUNNER_LOAD_ELF,
        USER_SVC_FILESYSTEM_DESCRIBE,
    );
    let svc_bad = user_svc_protocol_allowed(
        USER_SVC_FILESYSTEM,
        USER_SVC_RUNNER_LOAD_ELF,
        USER_SVC_COMPONENT_MANAGER,
        USER_SVC_RUNNER,
        USER_SVC_FILESYSTEM,
        USER_SVC_COMPONENT_START,
        USER_SVC_RUNNER_LOAD_ELF,
        USER_SVC_FILESYSTEM_DESCRIBE,
    );
    let launch_ok = user_component_thread_launch_valid(true, true, true);
    let launch_bad = user_component_thread_launch_valid(true, true, false);
    let return_ok = user_component_return_active(2);
    let return_bad = user_component_return_active(0);
    let elf_header_ok = user_elf_header_bounds_valid(
        USER_ELF_HEADER_SIZE,
        USER_ELF_HEADER_SIZE,
    );
    let elf_header_bad = user_elf_header_bounds_valid(
        USER_ELF_HEADER_SIZE - 1,
        USER_ELF_HEADER_SIZE,
    );
    let elf_magic_ok = user_elf_magic_valid(0x7fu8, 0x45u8, 0x4cu8, 0x46u8);
    let elf_magic_bad = user_elf_magic_valid(0u8, 0x45u8, 0x4cu8, 0x46u8);
    let elf_class_ok = user_elf_class_data_valid(2, 1, 1);
    let elf_class_bad = user_elf_class_data_valid(1, 1, 1);
    let elf_type_ok = user_elf_type_valid(
        USER_ELF_TYPE_EXEC,
        USER_ELF_TYPE_EXEC,
        USER_ELF_TYPE_DYN,
    );
    let elf_machine_ok = user_elf_machine_valid(
        USER_ELF_MACHINE_AARCH64,
        USER_ELF_MACHINE_AARCH64,
    );
    let elf_entry_ok = user_elf_entry_valid(0x1000);
    let elf_entry_bad = user_elf_entry_valid(0);
    let elf_phdr_ok = user_elf_phdr_table_valid(
        USER_ELF_HEADER_SIZE,
        USER_ELF_PHDR_SIZE,
        1,
        USER_ELF_HEADER_SIZE + USER_ELF_PHDR_SIZE,
        USER_ELF_PHDR_SIZE,
        USER_ELF_MAX_PHDRS,
    );
    let elf_phdr_bad = user_elf_phdr_table_valid(
        USER_ELF_HEADER_SIZE,
        USER_ELF_PHDR_SIZE + 1,
        1,
        USER_ELF_HEADER_SIZE + USER_ELF_PHDR_SIZE,
        USER_ELF_PHDR_SIZE,
        USER_ELF_MAX_PHDRS,
    );
    let elf_segment_ok = user_elf_segment_bounds_valid(0, 120, 4096, 120);
    let elf_segment_bad = user_elf_segment_bounds_valid(100, 32, 16, 120);
    let elf_vaddr_ok = user_elf_vaddr_range_valid(0x1000, 4096);
    let elf_vaddr_bad = user_elf_vaddr_range_valid(u64::MAX, 1);
    let page_down_ok = user_page_down(0x1234, PAGE_SIZE);
    let page_down_bad = user_page_down(0x1234, 0);
    let page_up_ok = user_page_up(0x1234, PAGE_SIZE);
    let page_up_aligned = user_page_up(0x2000, PAGE_SIZE);
    let page_up_bad = user_page_up(usize::MAX, PAGE_SIZE);
    let dns_host_len_ok = user_dns_host_len_valid(253, 253);
    let dns_host_len_bad = user_dns_host_len_valid(254, 253);
    let dns_label_len_ok = user_dns_label_len_valid(63, 63);
    let dns_label_len_bad = user_dns_label_len_valid(0, 63);
    let dns_label_alpha = user_dns_label_byte_valid(0x61u8);
    let dns_label_bad = user_dns_label_byte_valid(0x5fu8);
    let ipv4_step_ok = user_ipv4_octet_step(24, 5);
    let ipv4_step_bad = user_ipv4_octet_step(25, 6);
    let elf_map_ok = user_elf_segment_mapping_range(0x1234, 0x2000, PAGE_SIZE);
    let elf_map_empty = user_elf_segment_mapping_range(0x1234, 0, PAGE_SIZE);
    let elf_map_overflow = user_elf_segment_mapping_range(usize::MAX, 2, PAGE_SIZE);

    assert(start_ok == component_start_allowed_spec(true, false, false));
    assert(start_existing == component_start_allowed_spec(false, true, true));
    assert(start_missing == component_start_allowed_spec(false, false, false));
    assert((0x7u32 & !USER_NAMESPACE_RIGHTS_MASK) == 0) by(bit_vector);
    assert((0x8u32 & !USER_NAMESPACE_RIGHTS_MASK) != 0) by(bit_vector);
    assert(rights_ok == (0x7u32 & !USER_NAMESPACE_RIGHTS_MASK == 0));
    assert(rights_bad == (0x8u32 & !USER_NAMESPACE_RIGHTS_MASK == 0));
    assert(file_size_ok == fxfs_file_size_valid_spec(
        USER_FXFS_MAX_FILE_BYTES as int,
        USER_FXFS_MAX_FILE_BYTES as int,
    ));
    assert(file_size_bad == fxfs_file_size_valid_spec(
        (USER_FXFS_MAX_FILE_BYTES + 1) as int,
        USER_FXFS_MAX_FILE_BYTES as int,
    ));
    assert(nodes_ok == fxfs_node_capacity_valid_spec(
        (USER_FXFS_MAX_NODES - 1) as int,
        USER_FXFS_MAX_NODES as int,
    ));
    assert(nodes_bad == fxfs_node_capacity_valid_spec(
        USER_FXFS_MAX_NODES as int,
        USER_FXFS_MAX_NODES as int,
    ));
    assert(dirents_ok == fxfs_dirent_capacity_valid_spec(
        (USER_FXFS_MAX_DIRENTS - 1) as int,
        USER_FXFS_MAX_DIRENTS as int,
    ));
    assert(dirents_bad == fxfs_dirent_capacity_valid_spec(
        USER_FXFS_MAX_DIRENTS as int,
        USER_FXFS_MAX_DIRENTS as int,
    ));
    assert(append_ok == Some(15usize));
    assert(append_bad == Option::<usize>::None);
    assert(write_end_ok == Some(10usize));
    assert(write_end_bad == Option::<usize>::None);
    assert(seek_ok == fxfs_seek_valid_spec(10, 10));
    assert(seek_bad == fxfs_seek_valid_spec(11, 10));
    assert(replay_ok == fxfs_replay_count_valid_spec(7, 8));
    assert(replay_bad == fxfs_replay_count_valid_spec(9, 8));
    assert(svc_name_ok == svc_name_valid_spec(32, USER_SVC_MAX_NAME_LEN as int));
    assert(svc_name_bad == svc_name_valid_spec(0, USER_SVC_MAX_NAME_LEN as int));
    assert((0x3u32 & !USER_SVC_RIGHTS_MASK) == 0) by(bit_vector);
    assert((0x4u32 & !USER_SVC_RIGHTS_MASK) != 0) by(bit_vector);
    assert(svc_rights_ok == (0x3u32 != 0 && (0x3u32 & !USER_SVC_RIGHTS_MASK) == 0));
    assert(svc_rights_bad == (0x4u32 != 0 && (0x4u32 & !USER_SVC_RIGHTS_MASK) == 0));
    assert(svc_size_ok == svc_ipc_message_size_valid_spec(
        USER_SVC_IPC_MESSAGE_SIZE as int,
        USER_SVC_IPC_MESSAGE_SIZE as int,
    ));
    assert(svc_size_bad == svc_ipc_message_size_valid_spec(
        (USER_SVC_IPC_MESSAGE_SIZE - 1) as int,
        USER_SVC_IPC_MESSAGE_SIZE as int,
    ));
    assert(svc_header_ok == svc_ipc_header_valid_spec(
        USER_SVC_IPC_MAGIC as int,
        USER_SVC_IPC_VERSION as int,
        USER_SVC_IPC_MAGIC as int,
        USER_SVC_IPC_VERSION as int,
    ));
    assert(svc_header_bad == svc_ipc_header_valid_spec(
        0,
        USER_SVC_IPC_VERSION as int,
        USER_SVC_IPC_MAGIC as int,
        USER_SVC_IPC_VERSION as int,
    ));
    assert(svc_component_ok == svc_protocol_allowed_spec(
        USER_SVC_COMPONENT_MANAGER as int,
        USER_SVC_COMPONENT_START as int,
        USER_SVC_COMPONENT_MANAGER as int,
        USER_SVC_RUNNER as int,
        USER_SVC_FILESYSTEM as int,
        USER_SVC_COMPONENT_START as int,
        USER_SVC_RUNNER_LOAD_ELF as int,
        USER_SVC_FILESYSTEM_DESCRIBE as int,
    ));
    assert(svc_runner_ok == svc_protocol_allowed_spec(
        USER_SVC_RUNNER as int,
        USER_SVC_RUNNER_LOAD_ELF as int,
        USER_SVC_COMPONENT_MANAGER as int,
        USER_SVC_RUNNER as int,
        USER_SVC_FILESYSTEM as int,
        USER_SVC_COMPONENT_START as int,
        USER_SVC_RUNNER_LOAD_ELF as int,
        USER_SVC_FILESYSTEM_DESCRIBE as int,
    ));
    assert(svc_bad == svc_protocol_allowed_spec(
        USER_SVC_FILESYSTEM as int,
        USER_SVC_RUNNER_LOAD_ELF as int,
        USER_SVC_COMPONENT_MANAGER as int,
        USER_SVC_RUNNER as int,
        USER_SVC_FILESYSTEM as int,
        USER_SVC_COMPONENT_START as int,
        USER_SVC_RUNNER_LOAD_ELF as int,
        USER_SVC_FILESYSTEM_DESCRIBE as int,
    ));
    assert(launch_ok == component_thread_launch_valid_spec(true, true, true));
    assert(launch_bad == component_thread_launch_valid_spec(true, true, false));
    assert(return_ok == component_return_active_spec(2));
    assert(return_bad == component_return_active_spec(0));
    assert(elf_header_ok == elf_header_bounds_valid_spec(
        USER_ELF_HEADER_SIZE as int,
        USER_ELF_HEADER_SIZE as int,
    ));
    assert(elf_header_bad == elf_header_bounds_valid_spec(
        (USER_ELF_HEADER_SIZE - 1) as int,
        USER_ELF_HEADER_SIZE as int,
    ));
    assert(elf_magic_ok == elf_magic_valid_spec(0x7f, 0x45, 0x4c, 0x46));
    assert(elf_magic_bad == elf_magic_valid_spec(0, 0x45, 0x4c, 0x46));
    assert(elf_class_ok == elf_class_data_valid_spec(2, 1, 1));
    assert(elf_class_bad == elf_class_data_valid_spec(1, 1, 1));
    assert(elf_type_ok == elf_type_valid_spec(
        USER_ELF_TYPE_EXEC as int,
        USER_ELF_TYPE_EXEC as int,
        USER_ELF_TYPE_DYN as int,
    ));
    assert(elf_machine_ok == elf_machine_valid_spec(
        USER_ELF_MACHINE_AARCH64 as int,
        USER_ELF_MACHINE_AARCH64 as int,
    ));
    assert(elf_entry_ok == elf_entry_valid_spec(0x1000));
    assert(elf_entry_bad == elf_entry_valid_spec(0));
    assert(elf_phdr_ok == elf_phdr_table_valid_spec(
        USER_ELF_HEADER_SIZE as int,
        USER_ELF_PHDR_SIZE as int,
        1,
        (USER_ELF_HEADER_SIZE + USER_ELF_PHDR_SIZE) as int,
        USER_ELF_PHDR_SIZE as int,
        USER_ELF_MAX_PHDRS as int,
    ));
    assert(elf_phdr_bad == elf_phdr_table_valid_spec(
        USER_ELF_HEADER_SIZE as int,
        (USER_ELF_PHDR_SIZE + 1) as int,
        1,
        (USER_ELF_HEADER_SIZE + USER_ELF_PHDR_SIZE) as int,
        USER_ELF_PHDR_SIZE as int,
        USER_ELF_MAX_PHDRS as int,
    ));
    assert(elf_segment_ok == elf_segment_bounds_valid_spec(0, 120, 4096, 120));
    assert(elf_segment_bad == elf_segment_bounds_valid_spec(100, 32, 16, 120));
    assert(elf_vaddr_ok == elf_vaddr_range_valid_spec(0x1000, 4096));
    assert(elf_vaddr_bad == elf_vaddr_range_valid_spec(u64::MAX as int, 1));
    assert(page_down_ok == Some(0x1000usize));
    assert(page_down_bad == Option::<usize>::None);
    assert(page_up_ok == Some(0x2000usize));
    assert(page_up_aligned == Some(0x2000usize));
    assert(page_up_bad == Option::<usize>::None);
    assert(dns_host_len_ok == dns_host_len_valid_spec(253, 253));
    assert(dns_host_len_bad == dns_host_len_valid_spec(254, 253));
    assert(dns_label_len_ok == dns_label_len_valid_spec(63, 63));
    assert(dns_label_len_bad == dns_label_len_valid_spec(0, 63));
    assert(dns_label_alpha == dns_label_byte_valid_spec(0x61));
    assert(dns_label_bad == dns_label_byte_valid_spec(0x5f));
    assert(ipv4_step_ok == Some(245u32));
    assert(ipv4_step_bad == Option::<u32>::None);
    assert(elf_map_ok == Some((0x1000usize, 0x4000usize)));
    assert(elf_map_empty == Option::<(usize, usize)>::None);
    assert(elf_map_overflow == Option::<(usize, usize)>::None);
}

proof fn user_test_syscall_smoke() {
    assert(SYS_WRITE == 64);
    assert(SYS_EXIT == 93);
    assert(SYS_GETPID == 172);
    assert(SYS_MMAP == 222);
    assert(EL0_TEST_STACK_SIZE == 2 * PAGE_SIZE);
    assert(mmap_result_ok_spec(
        USER_MMAP_BASE as int,
        PAGE_SIZE as int,
        USER_MMAP_BASE as int,
        USER_MMAP_LIMIT as int,
    ));
    assert(!mmap_result_ok_spec(
        USER_MMAP_LIMIT as int,
        PAGE_SIZE as int,
        USER_MMAP_BASE as int,
        USER_MMAP_LIMIT as int,
    ));
    assert(user_test_exit_code_spec(
        EL0_TEST_BANNER_LEN as int,
        1,
        EL0_TEST_INFO_GETPID_LEN as int,
        USER_MMAP_BASE as int,
        EL0_TEST_INFO_MMAP_LEN as int,
        EL0_TEST_COMPLETE_LEN as int,
    ) == EL0_TEST_EXIT_OK as int);
    assert(kernel_success_spec(
        true,
        true,
        EL0_TEST_EXIT_OK as int,
        EL0_TEST_BANNER_LEN as int,
        1,
        USER_MMAP_BASE as int,
        EL0_TEST_BANNER_LEN as int,
    ));
    assert(!kernel_success_spec(
        false,
        true,
        EL0_TEST_EXIT_OK as int,
        EL0_TEST_BANNER_LEN as int,
        1,
        USER_MMAP_BASE as int,
        EL0_TEST_BANNER_LEN as int,
    ));
    assert(0x3C5u64 == smros_user_el1h_spsr_masked_body!());
    assert(0u64 == smros_user_syscall_should_advance_elr_body!());
}

} // verus!
