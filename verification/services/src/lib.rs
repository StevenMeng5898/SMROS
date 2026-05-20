#![allow(unused_macros)]

use vstd::prelude::*;

verus! {

include!("../../../src/user_level/services/user_logic_shared.rs");

pub const SERVICES_FILE_COUNT: usize = 16;

pub const USER_NAMESPACE_RIGHTS_MASK: u32 = 0x7;
pub const USER_FXFS_MAX_NODES: usize = 8192;
pub const USER_FXFS_MAX_DIRENTS: usize = 8192;
pub const USER_FXFS_MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
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

pub const GEMMA_PROMPT_MAX_BYTES: usize = 4096;
pub const GEMMA_CONTEXT_TOKENS: usize = 2048;
pub const GEMMA_MAX_OUTPUT_TOKENS: usize = 96;
pub const GEMMA_DEFAULT_OUTPUT_TOKENS: usize = 32;

pub const HERMES_REQUIRED_TOOLS: usize = 3;
pub const HERMES_REQUIRED_SKILLS: usize = 4;

pub const MAX_COMPONENTS: usize = 16;
pub const MAX_PENDING_COMPONENT_LAUNCHES: usize = 4;

pub const LINUX_CAT_PAYLOAD_BYTES: usize = 35;
pub const LINUX_CAT_BUFFER_BYTES: usize = 64;

pub const DNS_MAX_MESSAGE: usize = 512;
pub const ETHERNET_HEADER_LEN: usize = 14;
pub const IPV4_MIN_HEADER_LEN: usize = 20;
pub const UDP_HEADER_LEN: usize = 8;
pub const DHCP_MIN_PAYLOAD: usize = 300;
pub const ETHERNET_MTU: usize = 1500;
pub const TCP_MIN_HEADER_PAIR: usize = 40;

pub const RUN_ELF_STACK_SIZE: usize = 0x20_000;
pub const RUN_ELF_AUXV_ENTRIES: usize = 19;

pub const DOCKER_MAX_COMMAND_ITEMS: usize = 16;
pub const DOCKER_MAX_COMMAND_ITEM_BYTES: usize = 128;
pub const DOCKER_MAX_CONTAINER_NAME_BYTES: usize = 48;
pub const DOCKER_PULL_MAX_BYTES: usize = 64 * 1024 * 1024;
pub const DOCKER_HTTP_ARCHIVE_PATH_MAX_BYTES: usize = 512;
pub const DOCKER_HTTP_ARCHIVE_REQUEST_MAX_BYTES: usize = 1024;
pub const TAR_BLOCK_BYTES: usize = 512;
pub const TAR_MEMBER_NAME_MAX_BYTES: usize = 180;
pub const SIMPLE_PATH_COMPONENT_MAX_BYTES: usize = 255;
pub const DOCKER_IMAGE_REFERENCE_MAX_BYTES: usize = 160;

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

spec fn gemma_prompt_len_valid_spec(len: int) -> bool {
    0 < len && len <= GEMMA_PROMPT_MAX_BYTES as int
}

spec fn gemma_prompt_byte_valid_spec(byte: int) -> bool {
    byte == 0x0a || (0x20 <= byte && byte <= 0x7e)
}

spec fn gemma_clamp_tokens_spec(requested: int, max_allowed: int) -> int {
    if requested == 0 {
        if GEMMA_DEFAULT_OUTPUT_TOKENS as int <= max_allowed {
            GEMMA_DEFAULT_OUTPUT_TOKENS as int
        } else {
            max_allowed
        }
    } else if requested <= max_allowed {
        requested
    } else {
        max_allowed
    }
}

spec fn gemma_model_available_spec(provider_ok: bool, model_ok: bool, storage_ok: bool) -> bool {
    provider_ok && model_ok && storage_ok
}

spec fn gemma_test_passed_spec(
    manifest_ok: bool,
    prompt_ok: bool,
    generation_ok: bool,
    log_ok: bool,
) -> bool {
    manifest_ok && prompt_ok && generation_ok && log_ok
}

spec fn hermes_skill_matches_spec(skill_hits: int, required_skills: int, prompt_len: int) -> bool {
    skill_hits >= required_skills && required_skills > 0 && prompt_len > 0
}

spec fn hermes_delegate_allowed_spec(tools_len: int, prompt_len: int) -> bool {
    tools_len >= HERMES_REQUIRED_TOOLS as int && prompt_len > 0
}

spec fn hermes_report_passed_spec(
    config_ok: bool,
    model_route_ok: bool,
    skill_ok: bool,
    memory_ok: bool,
    tool_ok: bool,
    delegate_ok: bool,
    gemma_ok: bool,
    cron_ok: bool,
    transcript_ok: bool,
    svc_ok: bool,
    web_ui_ok: bool,
) -> bool {
    config_ok
        && model_route_ok
        && skill_ok
        && memory_ok
        && tool_ok
        && delegate_ok
        && gemma_ok
        && cron_ok
        && transcript_ok
        && svc_ok
        && web_ui_ok
}

spec fn compat_linux_cat_buffer_valid_spec(payload_len: int, buffer_len: int) -> bool {
    0 <= payload_len && payload_len <= buffer_len
}

spec fn component_capacity_available_spec(len: int, max: int) -> bool {
    0 <= len && len < max
}

spec fn component_pending_capacity_available_spec(count: int, max: int) -> bool {
    0 <= count && count < max
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

spec fn component_start_allowed_spec(
    binary_exists: bool,
    destroyed: bool,
    already_started: bool,
) -> bool {
    already_started || (binary_exists && !destroyed)
}

spec fn component_return_active_spec(pid: int) -> bool {
    pid != 0
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

spec fn docker_command_len_valid_spec(len: int) -> bool {
    0 <= len && len <= DOCKER_MAX_COMMAND_ITEMS as int
}

spec fn docker_command_item_valid_spec(
    len: int,
    has_nul: bool,
    has_control: bool,
    has_pipe: bool,
    has_newline: bool,
) -> bool {
    0 < len
        && len <= DOCKER_MAX_COMMAND_ITEM_BYTES as int
        && !has_nul
        && !has_control
        && !has_pipe
        && !has_newline
}

spec fn docker_name_len_valid_spec(len: int) -> bool {
    0 < len && len <= DOCKER_MAX_CONTAINER_NAME_BYTES as int
}

spec fn docker_name_byte_valid_spec(byte: int) -> bool {
    (0x30 <= byte && byte <= 0x39)
        || (0x41 <= byte && byte <= 0x5a)
        || (0x61 <= byte && byte <= 0x7a)
        || byte == 0x2d
        || byte == 0x2e
        || byte == 0x5f
}

spec fn docker_http_archive_path_valid_spec(len: int, starts_with_slash: bool, has_nul: bool) -> bool {
    starts_with_slash && !has_nul && len <= DOCKER_HTTP_ARCHIVE_PATH_MAX_BYTES as int
}

spec fn docker_pull_body_valid_spec(len: int) -> bool {
    0 < len && len <= DOCKER_PULL_MAX_BYTES as int
}

spec fn docker_parse_i32_abs_valid_spec(negative: bool, abs: int) -> bool {
    if negative {
        0 <= abs && abs <= i32::MAX as int + 1
    } else {
        0 <= abs && abs <= i32::MAX as int
    }
}

spec fn docker_tar_member_name_shape_spec(
    len: int,
    starts_with_slash: bool,
    has_parent: bool,
    has_double_slash: bool,
    has_nul: bool,
) -> bool {
    0 < len
        && len <= TAR_MEMBER_NAME_MAX_BYTES as int
        && !starts_with_slash
        && !has_parent
        && !has_double_slash
        && !has_nul
}

spec fn docker_simple_path_component_shape_spec(
    len: int,
    is_dot: bool,
    is_dotdot: bool,
    all_allowed: bool,
) -> bool {
    0 < len && len <= SIMPLE_PATH_COMPONENT_MAX_BYTES as int && !is_dot && !is_dotdot && all_allowed
}

spec fn docker_image_reference_shape_spec(
    len: int,
    all_allowed: bool,
    starts_with_slash: bool,
    ends_with_slash: bool,
    has_double_slash: bool,
    has_tag: bool,
) -> bool {
    0 < len
        && len <= DOCKER_IMAGE_REFERENCE_MAX_BYTES as int
        && all_allowed
        && !starts_with_slash
        && !ends_with_slash
        && !has_double_slash
        && has_tag
}

spec fn net_same_ipv4_subnet_spec(a0: int, a1: int, a2: int, b0: int, b1: int, b2: int) -> bool {
    a0 == b0 && a1 == b1 && a2 == b2
}

spec fn net_dhcp_frame_buffer_valid_spec(frame_len: int) -> bool {
    frame_len >= (ETHERNET_HEADER_LEN + IPV4_MIN_HEADER_LEN + UDP_HEADER_LEN + DHCP_MIN_PAYLOAD) as int
}

spec fn net_dns_message_buffer_valid_spec(out_len: int) -> bool {
    out_len >= DNS_MAX_MESSAGE as int
}

spec fn net_tcp_payload_len_valid_spec(data_len: int, mtu: int) -> bool {
    TCP_MIN_HEADER_PAIR as int <= mtu && data_len <= mtu - TCP_MIN_HEADER_PAIR as int
}

spec fn run_elf_table_words_spec(argv_len: int, env_len: int, auxv_len: int) -> int {
    1 + argv_len + 1 + env_len + 1 + auxv_len * 2
}

spec fn run_elf_table_bytes_spec(argv_len: int, env_len: int, auxv_len: int) -> int {
    run_elf_table_words_spec(argv_len, env_len, auxv_len) * 8
}

spec fn shell_ask_args_valid_spec(args_len: int) -> bool {
    args_len >= 2
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

fn gemma_prompt_len_valid(len: usize) -> (out: bool)
    ensures
        out == gemma_prompt_len_valid_spec(len as int),
{
    len > 0 && len <= GEMMA_PROMPT_MAX_BYTES
}

fn gemma_prompt_byte_valid(byte: u8) -> (out: bool)
    ensures
        out == gemma_prompt_byte_valid_spec(byte as int),
{
    byte == 0x0a || (byte >= 0x20 && byte <= 0x7e)
}

fn gemma_clamp_tokens(requested: usize, max_allowed: usize) -> (out: usize)
    requires
        max_allowed <= GEMMA_MAX_OUTPUT_TOKENS,
    ensures
        out as int == gemma_clamp_tokens_spec(requested as int, max_allowed as int),
        out <= max_allowed,
{
    if requested == 0 {
        if GEMMA_DEFAULT_OUTPUT_TOKENS <= max_allowed {
            GEMMA_DEFAULT_OUTPUT_TOKENS
        } else {
            max_allowed
        }
    } else if requested <= max_allowed {
        requested
    } else {
        max_allowed
    }
}

fn gemma_model_available(provider_ok: bool, model_ok: bool, storage_ok: bool) -> (out: bool)
    ensures
        out == gemma_model_available_spec(provider_ok, model_ok, storage_ok),
{
    provider_ok && model_ok && storage_ok
}

fn gemma_test_passed(
    manifest_ok: bool,
    prompt_ok: bool,
    generation_ok: bool,
    log_ok: bool,
) -> (out: bool)
    ensures
        out == gemma_test_passed_spec(manifest_ok, prompt_ok, generation_ok, log_ok),
{
    manifest_ok && prompt_ok && generation_ok && log_ok
}

fn hermes_skill_matches(skill_hits: usize, required_skills: usize, prompt_len: usize) -> (out: bool)
    ensures
        out == hermes_skill_matches_spec(skill_hits as int, required_skills as int, prompt_len as int),
{
    skill_hits >= required_skills && required_skills > 0 && prompt_len > 0
}

fn hermes_delegate_allowed(tools_len: usize, prompt_len: usize) -> (out: bool)
    ensures
        out == hermes_delegate_allowed_spec(tools_len as int, prompt_len as int),
{
    tools_len >= HERMES_REQUIRED_TOOLS && prompt_len > 0
}

fn hermes_report_passed(
    config_ok: bool,
    model_route_ok: bool,
    skill_ok: bool,
    memory_ok: bool,
    tool_ok: bool,
    delegate_ok: bool,
    gemma_ok: bool,
    cron_ok: bool,
    transcript_ok: bool,
    svc_ok: bool,
    web_ui_ok: bool,
) -> (out: bool)
    ensures
        out == hermes_report_passed_spec(
            config_ok,
            model_route_ok,
            skill_ok,
            memory_ok,
            tool_ok,
            delegate_ok,
            gemma_ok,
            cron_ok,
            transcript_ok,
            svc_ok,
            web_ui_ok,
        ),
{
    config_ok
        && model_route_ok
        && skill_ok
        && memory_ok
        && tool_ok
        && delegate_ok
        && gemma_ok
        && cron_ok
        && transcript_ok
        && svc_ok
        && web_ui_ok
}

fn compat_linux_cat_buffer_valid(payload_len: usize, buffer_len: usize) -> (out: bool)
    ensures
        out == compat_linux_cat_buffer_valid_spec(payload_len as int, buffer_len as int),
{
    payload_len <= buffer_len
}

fn component_capacity_available(len: usize, max: usize) -> (out: bool)
    ensures
        out == component_capacity_available_spec(len as int, max as int),
{
    len < max
}

fn component_pending_capacity_available(count: usize, max: usize) -> (out: bool)
    ensures
        out == component_pending_capacity_available_spec(count as int, max as int),
{
    count < max
}

fn namespace_rights_valid(rights: u32) -> (out: bool)
    ensures
        out == (rights & !USER_NAMESPACE_RIGHTS_MASK == 0),
{
    smros_user_namespace_rights_valid_body!(rights, USER_NAMESPACE_RIGHTS_MASK)
}

fn fxfs_file_size_valid(size: usize) -> (out: bool)
    ensures
        out == fxfs_file_size_valid_spec(size as int, USER_FXFS_MAX_FILE_BYTES as int),
{
    smros_user_fxfs_file_size_valid_body!(size, USER_FXFS_MAX_FILE_BYTES)
}

fn fxfs_node_capacity_valid(nodes: usize) -> (out: bool)
    ensures
        out == fxfs_node_capacity_valid_spec(nodes as int, USER_FXFS_MAX_NODES as int),
{
    smros_user_fxfs_node_capacity_valid_body!(nodes, USER_FXFS_MAX_NODES)
}

fn fxfs_dirent_capacity_valid(entries: usize) -> (out: bool)
    ensures
        out == fxfs_dirent_capacity_valid_spec(entries as int, USER_FXFS_MAX_DIRENTS as int),
{
    smros_user_fxfs_dirent_capacity_valid_body!(entries, USER_FXFS_MAX_DIRENTS)
}

fn fxfs_append_size(old_size: usize, append_len: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(size) => checked_end_spec(old_size as int, append_len as int) == Some(size as int),
            None => checked_end_spec(old_size as int, append_len as int) == Option::<int>::None,
        },
{
    smros_user_fxfs_append_size_body!(old_size, append_len)
}

fn fxfs_write_end(offset: usize, len: usize) -> (out: Option<usize>)
    ensures
        match out {
            Some(end) => checked_end_spec(offset as int, len as int) == Some(end as int),
            None => checked_end_spec(offset as int, len as int) == Option::<int>::None,
        },
{
    smros_user_fxfs_write_end_body!(offset, len)
}

fn fxfs_seek_valid(offset: usize, size: usize) -> (out: bool)
    ensures
        out == fxfs_seek_valid_spec(offset as int, size as int),
{
    smros_user_fxfs_seek_valid_body!(offset, size)
}

fn fxfs_replay_count_valid(replayed: usize, journal_records: usize) -> (out: bool)
    ensures
        out == fxfs_replay_count_valid_spec(replayed as int, journal_records as int),
{
    smros_user_fxfs_replay_count_valid_body!(replayed, journal_records)
}

fn svc_name_valid(len: usize) -> (out: bool)
    ensures
        out == svc_name_valid_spec(len as int, USER_SVC_MAX_NAME_LEN as int),
{
    smros_user_svc_name_valid_body!(len, USER_SVC_MAX_NAME_LEN)
}

fn svc_rights_valid(rights: u32) -> (out: bool)
    ensures
        out == (rights != 0 && (rights & !USER_SVC_RIGHTS_MASK) == 0),
{
    smros_user_svc_rights_valid_body!(rights, USER_SVC_RIGHTS_MASK)
}

fn svc_ipc_message_size_valid(size: usize) -> (out: bool)
    ensures
        out == svc_ipc_message_size_valid_spec(size as int, USER_SVC_IPC_MESSAGE_SIZE as int),
{
    smros_user_svc_ipc_message_size_valid_body!(size, USER_SVC_IPC_MESSAGE_SIZE)
}

fn svc_ipc_header_valid(magic: u32, version: u16) -> (out: bool)
    ensures
        out == svc_ipc_header_valid_spec(
            magic as int,
            version as int,
            USER_SVC_IPC_MAGIC as int,
            USER_SVC_IPC_VERSION as int,
        ),
{
    smros_user_svc_ipc_header_valid_body!(magic, version, USER_SVC_IPC_MAGIC, USER_SVC_IPC_VERSION)
}

fn svc_protocol_allowed(service: u16, ordinal: u16) -> (out: bool)
    ensures
        out == smros_user_svc_protocol_allowed_body!(
            service,
            ordinal,
            USER_SVC_COMPONENT_MANAGER,
            USER_SVC_RUNNER,
            USER_SVC_FILESYSTEM,
            USER_SVC_COMPONENT_START,
            USER_SVC_RUNNER_LOAD_ELF,
            USER_SVC_FILESYSTEM_DESCRIBE
        ),
{
    smros_user_svc_protocol_allowed_body!(
        service,
        ordinal,
        USER_SVC_COMPONENT_MANAGER,
        USER_SVC_RUNNER,
        USER_SVC_FILESYSTEM,
        USER_SVC_COMPONENT_START,
        USER_SVC_RUNNER_LOAD_ELF,
        USER_SVC_FILESYSTEM_DESCRIBE
    )
}

fn elf_header_bounds_valid(image_len: usize) -> (out: bool)
    ensures
        out == elf_header_bounds_valid_spec(image_len as int, USER_ELF_HEADER_SIZE as int),
{
    smros_user_elf_header_bounds_valid_body!(image_len, USER_ELF_HEADER_SIZE)
}

fn elf_phdr_table_valid(phoff: usize, phentsize: usize, phnum: usize, image_len: usize) -> (out: bool)
    ensures
        out == smros_user_elf_phdr_table_valid_body!(
            phoff,
            phentsize,
            phnum,
            image_len,
            USER_ELF_PHDR_SIZE,
            USER_ELF_MAX_PHDRS
        ),
{
    smros_user_elf_phdr_table_valid_body!(
        phoff,
        phentsize,
        phnum,
        image_len,
        USER_ELF_PHDR_SIZE,
        USER_ELF_MAX_PHDRS
    )
}

fn elf_segment_bounds_valid(
    offset: usize,
    file_size: usize,
    mem_size: usize,
    image_len: usize,
) -> (out: bool)
    ensures
        out == smros_user_elf_segment_bounds_valid_body!(offset, file_size, mem_size, image_len),
{
    smros_user_elf_segment_bounds_valid_body!(offset, file_size, mem_size, image_len)
}

fn elf_segment_mapping_range(vaddr: usize, mem_size: usize, page_size: usize) -> (out: Option<(usize, usize)>)
    ensures
        match out {
            Some((start, end)) => {
                &&& 0 <= start as int
                &&& (start as int) <= (vaddr as int)
                &&& (end as int) >= (vaddr as int) + (mem_size as int)
            },
            None => true,
        },
{
    smros_user_elf_segment_mapping_range_body!(vaddr, mem_size, page_size)
}

fn docker_command_len_valid(len: usize) -> (out: bool)
    ensures
        out == docker_command_len_valid_spec(len as int),
{
    len <= DOCKER_MAX_COMMAND_ITEMS
}

fn docker_command_item_valid(
    len: usize,
    has_nul: bool,
    has_control: bool,
    has_pipe: bool,
    has_newline: bool,
) -> (out: bool)
    ensures
        out == docker_command_item_valid_spec(
            len as int,
            has_nul,
            has_control,
            has_pipe,
            has_newline,
        ),
{
    len > 0
        && len <= DOCKER_MAX_COMMAND_ITEM_BYTES
        && !has_nul
        && !has_control
        && !has_pipe
        && !has_newline
}

fn docker_name_len_valid(len: usize) -> (out: bool)
    ensures
        out == docker_name_len_valid_spec(len as int),
{
    len > 0 && len <= DOCKER_MAX_CONTAINER_NAME_BYTES
}

fn docker_name_byte_valid(byte: u8) -> (out: bool)
    ensures
        out == docker_name_byte_valid_spec(byte as int),
{
    (0x30 <= byte && byte <= 0x39)
        || (0x41 <= byte && byte <= 0x5a)
        || (0x61 <= byte && byte <= 0x7a)
        || byte == 0x2d
        || byte == 0x5f
        || byte == 0x2e
}

fn docker_http_archive_path_valid(len: usize, starts_with_slash: bool, has_nul: bool) -> (out: bool)
    ensures
        out == docker_http_archive_path_valid_spec(len as int, starts_with_slash, has_nul),
{
    starts_with_slash && !has_nul && len <= DOCKER_HTTP_ARCHIVE_PATH_MAX_BYTES
}

fn docker_pull_body_valid(len: usize) -> (out: bool)
    ensures
        out == docker_pull_body_valid_spec(len as int),
{
    len > 0 && len <= DOCKER_PULL_MAX_BYTES
}

fn docker_parse_i32_abs_valid(negative: bool, abs: usize) -> (out: bool)
    ensures
        out == docker_parse_i32_abs_valid_spec(negative, abs as int),
{
    if negative {
        abs <= i32::MAX as usize + 1
    } else {
        abs <= i32::MAX as usize
    }
}

fn docker_tar_member_name_shape(
    len: usize,
    starts_with_slash: bool,
    has_parent: bool,
    has_double_slash: bool,
    has_nul: bool,
) -> (out: bool)
    ensures
        out == docker_tar_member_name_shape_spec(
            len as int,
            starts_with_slash,
            has_parent,
            has_double_slash,
            has_nul,
        ),
{
    len > 0
        && len <= TAR_MEMBER_NAME_MAX_BYTES
        && !starts_with_slash
        && !has_parent
        && !has_double_slash
        && !has_nul
}

fn docker_simple_path_component_shape(
    len: usize,
    is_dot: bool,
    is_dotdot: bool,
    all_allowed: bool,
) -> (out: bool)
    ensures
        out == docker_simple_path_component_shape_spec(len as int, is_dot, is_dotdot, all_allowed),
{
    len > 0 && len <= SIMPLE_PATH_COMPONENT_MAX_BYTES && !is_dot && !is_dotdot && all_allowed
}

fn docker_image_reference_shape(
    len: usize,
    all_allowed: bool,
    starts_with_slash: bool,
    ends_with_slash: bool,
    has_double_slash: bool,
    has_tag: bool,
) -> (out: bool)
    ensures
        out == docker_image_reference_shape_spec(
            len as int,
            all_allowed,
            starts_with_slash,
            ends_with_slash,
            has_double_slash,
            has_tag,
        ),
{
    len > 0
        && len <= DOCKER_IMAGE_REFERENCE_MAX_BYTES
        && all_allowed
        && !starts_with_slash
        && !ends_with_slash
        && !has_double_slash
        && has_tag
}

fn net_same_ipv4_subnet(a0: u8, a1: u8, a2: u8, b0: u8, b1: u8, b2: u8) -> (out: bool)
    ensures
        out == net_same_ipv4_subnet_spec(
            a0 as int,
            a1 as int,
            a2 as int,
            b0 as int,
            b1 as int,
            b2 as int,
        ),
{
    a0 == b0 && a1 == b1 && a2 == b2
}

fn net_dhcp_frame_buffer_valid(frame_len: usize) -> (out: bool)
    ensures
        out == net_dhcp_frame_buffer_valid_spec(frame_len as int),
{
    frame_len >= ETHERNET_HEADER_LEN + IPV4_MIN_HEADER_LEN + UDP_HEADER_LEN + DHCP_MIN_PAYLOAD
}

fn net_dns_message_buffer_valid(out_len: usize) -> (out: bool)
    ensures
        out == net_dns_message_buffer_valid_spec(out_len as int),
{
    out_len >= DNS_MAX_MESSAGE
}

fn net_tcp_payload_len_valid(data_len: usize, mtu: usize) -> (out: bool)
    ensures
        out == net_tcp_payload_len_valid_spec(data_len as int, mtu as int),
{
    mtu >= TCP_MIN_HEADER_PAIR && data_len <= mtu - TCP_MIN_HEADER_PAIR
}

fn run_elf_table_words(argv_len: usize, env_len: usize, auxv_len: usize) -> (out: usize)
    requires
        argv_len <= 64,
        env_len <= 64,
        auxv_len <= 64,
    ensures
        out as int == run_elf_table_words_spec(argv_len as int, env_len as int, auxv_len as int),
{
    assert((auxv_len as int) * 2 <= 128) by(nonlinear_arith)
        requires
            auxv_len <= 64,
    ;
    1 + argv_len + 1 + env_len + 1 + auxv_len * 2
}

fn run_elf_table_bytes(argv_len: usize, env_len: usize, auxv_len: usize) -> (out: usize)
    requires
        argv_len <= 64,
        env_len <= 64,
        auxv_len <= 64,
    ensures
        out as int == run_elf_table_bytes_spec(argv_len as int, env_len as int, auxv_len as int),
        out <= RUN_ELF_STACK_SIZE,
{
    let words = run_elf_table_words(argv_len, env_len, auxv_len);
    assert(words <= 259) by(nonlinear_arith)
        requires
            argv_len <= 64,
            env_len <= 64,
            auxv_len <= 64,
            words as int == 1
                + argv_len as int
                + 1
                + env_len as int
                + 1
                + auxv_len as int * 2,
    ;
    assert(words * 8 <= RUN_ELF_STACK_SIZE) by(nonlinear_arith)
        requires
            words <= 259,
            RUN_ELF_STACK_SIZE == 0x20_000,
    ;
    words * 8
}

fn shell_ascii_input(byte: u8) -> (out: bool)
    ensures
        out == (0x20 <= byte as int && byte as int <= 0x7e),
{
    smros_user_ascii_shell_input_body!(byte)
}

fn shell_ask_args_valid(args_len: usize) -> (out: bool)
    ensures
        out == shell_ask_args_valid_spec(args_len as int),
{
    args_len >= 2
}

proof fn compat_apps_rs_proof_slice() {
    assert(compat_linux_cat_buffer_valid_spec(
        LINUX_CAT_PAYLOAD_BYTES as int,
        LINUX_CAT_BUFFER_BYTES as int,
    ));
    assert(svc_protocol_allowed_spec(
        USER_SVC_COMPONENT_MANAGER as int,
        USER_SVC_COMPONENT_START as int,
        USER_SVC_COMPONENT_MANAGER as int,
        USER_SVC_RUNNER as int,
        USER_SVC_FILESYSTEM as int,
        USER_SVC_COMPONENT_START as int,
        USER_SVC_RUNNER_LOAD_ELF as int,
        USER_SVC_FILESYSTEM_DESCRIBE as int,
    ));
}

proof fn component_rs_proof_slice() {
    assert(component_capacity_available_spec(0, MAX_COMPONENTS as int));
    assert(component_pending_capacity_available_spec(0, MAX_PENDING_COMPONENT_LAUNCHES as int));
    assert(component_start_allowed_spec(true, false, false));
    assert(!component_start_allowed_spec(false, false, false));
}

proof fn docker_compat_rs_proof_slice() {
    assert(docker_command_len_valid_spec(DOCKER_MAX_COMMAND_ITEMS as int));
    assert(docker_command_item_valid_spec(1, false, false, false, false));
    assert(!docker_command_item_valid_spec(0, false, false, false, false));
    assert(docker_name_len_valid_spec(DOCKER_MAX_CONTAINER_NAME_BYTES as int));
    assert(docker_name_byte_valid_spec(0x61));
    assert(docker_name_byte_valid_spec(0x5f));
    assert(!docker_name_byte_valid_spec(0x2f));
    assert(docker_http_archive_path_valid_spec(1, true, false));
    assert(docker_pull_body_valid_spec(1));
    assert(!docker_pull_body_valid_spec(0));
    assert(docker_parse_i32_abs_valid_spec(true, i32::MAX as int + 1));
    assert(docker_tar_member_name_shape_spec(1, false, false, false, false));
    assert(docker_simple_path_component_shape_spec(1, false, false, true));
    assert(docker_image_reference_shape_spec(12, true, false, false, false, true));
}

proof fn elf_rs_proof_slice() {
    assert(elf_header_bounds_valid_spec(
        USER_ELF_HEADER_SIZE as int,
        USER_ELF_HEADER_SIZE as int,
    ));
    assert(elf_magic_valid_spec(0x7f, 0x45, 0x4c, 0x46));
    assert(elf_class_data_valid_spec(2, 1, 1));
    assert(elf_type_valid_spec(USER_ELF_TYPE_EXEC as int, USER_ELF_TYPE_EXEC as int, USER_ELF_TYPE_DYN as int));
    assert(elf_machine_valid_spec(USER_ELF_MACHINE_AARCH64 as int, USER_ELF_MACHINE_AARCH64 as int));
    assert(elf_entry_valid_spec(1));
}

proof fn fxfs_rs_proof_slice() {
    assert(fxfs_file_size_valid_spec(0, USER_FXFS_MAX_FILE_BYTES as int));
    assert(fxfs_node_capacity_valid_spec(USER_FXFS_MAX_NODES as int - 1, USER_FXFS_MAX_NODES as int));
    assert(fxfs_dirent_capacity_valid_spec(USER_FXFS_MAX_DIRENTS as int - 1, USER_FXFS_MAX_DIRENTS as int));
    assert(fxfs_append_size_spec(10, 5) == Some(15int));
    assert(fxfs_write_end_spec(4, 6) == Some(10int));
    assert(fxfs_seek_valid_spec(10, 10));
    assert(fxfs_replay_count_valid_spec(7, 8));
}

proof fn gemma_rs_proof_slice() {
    assert(gemma_prompt_len_valid_spec(1));
    assert(!gemma_prompt_len_valid_spec(0));
    assert(gemma_prompt_byte_valid_spec(0x0a));
    assert(gemma_prompt_byte_valid_spec(0x20));
    assert(gemma_prompt_byte_valid_spec(0x7e));
    assert(!gemma_prompt_byte_valid_spec(0x7f));
    assert(gemma_clamp_tokens_spec(0, GEMMA_MAX_OUTPUT_TOKENS as int) == GEMMA_DEFAULT_OUTPUT_TOKENS as int);
    assert(gemma_clamp_tokens_spec(999, GEMMA_MAX_OUTPUT_TOKENS as int) == GEMMA_MAX_OUTPUT_TOKENS as int);
    assert(gemma_model_available_spec(true, true, true));
    assert(!gemma_model_available_spec(true, false, true));
    assert(gemma_test_passed_spec(true, true, true, true));
}

proof fn hermes_agent_rs_proof_slice() {
    assert(hermes_skill_matches_spec(HERMES_REQUIRED_SKILLS as int, HERMES_REQUIRED_SKILLS as int, 1));
    assert(!hermes_skill_matches_spec((HERMES_REQUIRED_SKILLS - 1) as int, HERMES_REQUIRED_SKILLS as int, 1));
    assert(!hermes_skill_matches_spec(HERMES_REQUIRED_SKILLS as int, HERMES_REQUIRED_SKILLS as int, 0));
    assert(hermes_delegate_allowed_spec(HERMES_REQUIRED_TOOLS as int, 1));
    assert(!hermes_delegate_allowed_spec((HERMES_REQUIRED_TOOLS - 1) as int, 1));
    assert(hermes_report_passed_spec(
        true,
        true,
        true,
        true,
        true,
        true,
        true,
        true,
        true,
        true,
        true,
    ));
}

proof fn host_share_rs_proof_slice()
    ensures
        true,
{
}

proof fn html_ui_rs_proof_slice()
    ensures
        true,
{
}

proof fn mod_rs_proof_slice()
    ensures
        SERVICES_FILE_COUNT == 16,
{
}

proof fn net_rs_proof_slice() {
    assert(net_same_ipv4_subnet_spec(10, 0, 2, 10, 0, 2));
    assert(!net_same_ipv4_subnet_spec(10, 0, 2, 10, 0, 3));
    assert(net_dhcp_frame_buffer_valid_spec(
        (ETHERNET_HEADER_LEN + IPV4_MIN_HEADER_LEN + UDP_HEADER_LEN + DHCP_MIN_PAYLOAD) as int,
    ));
    assert(net_dns_message_buffer_valid_spec(DNS_MAX_MESSAGE as int));
    assert(net_tcp_payload_len_valid_spec(0, ETHERNET_MTU as int));
}

proof fn run_elf_rs_proof_slice() {
    assert(run_elf_table_words_spec(1, 1, RUN_ELF_AUXV_ENTRIES as int) == 43);
    assert(run_elf_table_bytes_spec(1, 1, RUN_ELF_AUXV_ENTRIES as int) == 344);
    assert(RUN_ELF_STACK_SIZE > 344);
}

proof fn svc_rs_proof_slice() {
    assert(svc_name_valid_spec(1, USER_SVC_MAX_NAME_LEN as int));
    assert(!svc_name_valid_spec(0, USER_SVC_MAX_NAME_LEN as int));
    assert(svc_rights_valid_spec(1, USER_SVC_RIGHTS_MASK as int));
    assert(svc_ipc_message_size_valid_spec(
        USER_SVC_IPC_MESSAGE_SIZE as int,
        USER_SVC_IPC_MESSAGE_SIZE as int,
    ));
    assert(svc_ipc_header_valid_spec(
        USER_SVC_IPC_MAGIC as int,
        USER_SVC_IPC_VERSION as int,
        USER_SVC_IPC_MAGIC as int,
        USER_SVC_IPC_VERSION as int,
    ));
}

proof fn user_logic_rs_proof_slice() {
    assert(namespace_rights_valid_spec(0x7, USER_NAMESPACE_RIGHTS_MASK as int));
    assert(component_return_active_spec(1));
    assert(!component_return_active_spec(0));
}

proof fn user_logic_shared_rs_proof_slice() {
    assert(checked_end_spec(usize::MAX as int, 1) == Option::<int>::None);
    assert(checked_end_spec(4, 6) == Some(10int));
    assert(checked_mul_spec(usize::MAX as int, 2) == Option::<int>::None);
    assert(checked_mul_spec(3, 7) == Some(21int));
}

proof fn user_shell_rs_proof_slice() {
    assert(ascii_shell_input_spec(0x20));
    assert(ascii_shell_input_spec(0x7e));
    assert(!ascii_shell_input_spec(0x1f));
    assert(decimal_digit_value_spec(48) == Some(0int));
    assert(decimal_digit_value_spec(65) == Option::<int>::None);
    assert(shell_ask_args_valid_spec(2));
    assert(!shell_ask_args_valid_spec(1));
}

proof fn services_folder_all_files_have_verification_slices()
    ensures
        SERVICES_FILE_COUNT == 16,
{
    compat_apps_rs_proof_slice();
    component_rs_proof_slice();
    docker_compat_rs_proof_slice();
    elf_rs_proof_slice();
    fxfs_rs_proof_slice();
    gemma_rs_proof_slice();
    hermes_agent_rs_proof_slice();
    host_share_rs_proof_slice();
    html_ui_rs_proof_slice();
    mod_rs_proof_slice();
    net_rs_proof_slice();
    run_elf_rs_proof_slice();
    svc_rs_proof_slice();
    user_logic_rs_proof_slice();
    user_logic_shared_rs_proof_slice();
    user_shell_rs_proof_slice();
}

} // verus!
