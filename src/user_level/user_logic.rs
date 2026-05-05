include!("user_logic_shared.rs");

pub(crate) const USER_PROCESS_CAPACITY: usize = 16;
pub(crate) const USER_INIT_PARENT_PID: usize = 1;
pub(crate) const USER_CODE_VADDR: usize = 0x0000_0000;
pub(crate) const USER_DATA_VADDR: usize = 0x0000_1000;
pub(crate) const USER_HEAP_VADDR: usize = 0x0000_2000;
pub(crate) const USER_HEAP_PAGES: usize = 4;
pub(crate) const USER_STACK_VADDR: usize = 0xFFFF_0000;
pub(crate) const USER_STACK_PAGES: usize = 2;
pub(crate) const USER_THREAD_TIME_SLICE: u32 = 10;
pub(crate) const USER_MMAP_BASE: u64 = 0x5000_0000;
pub(crate) const USER_MMAP_LIMIT: u64 = 0x6000_0000;
pub(crate) const USER_NAMESPACE_RIGHTS_MASK: u32 = 0x7;
pub(crate) const USER_FXFS_MAX_NODES: usize = 128;
pub(crate) const USER_FXFS_MAX_DIRENTS: usize = 192;
pub(crate) const USER_FXFS_MAX_FILE_BYTES: usize = 4096;
pub(crate) const USER_ELF_HEADER_SIZE: usize = 64;
pub(crate) const USER_ELF_PHDR_SIZE: usize = 56;
pub(crate) const USER_ELF_MAX_PHDRS: usize = 8;
pub(crate) const USER_ELF_MACHINE_AARCH64: u16 = 183;
pub(crate) const USER_ELF_TYPE_EXEC: u16 = 2;
pub(crate) const USER_ELF_TYPE_DYN: u16 = 3;
pub(crate) const USER_SVC_MAX_NAME_LEN: usize = 64;
pub(crate) const USER_SVC_RIGHTS_MASK: u32 = 0x3;
pub(crate) const USER_SVC_IPC_MAGIC: u32 = 0x534d_4950;
pub(crate) const USER_SVC_IPC_VERSION: u16 = 1;
pub(crate) const USER_SVC_IPC_MESSAGE_SIZE: usize = 32;
pub(crate) const USER_SVC_COMPONENT_MANAGER: u16 = 0;
pub(crate) const USER_SVC_RUNNER: u16 = 1;
pub(crate) const USER_SVC_FILESYSTEM: u16 = 2;
pub(crate) const USER_SVC_COMPONENT_START: u16 = 1;
pub(crate) const USER_SVC_RUNNER_LOAD_ELF: u16 = 2;
pub(crate) const USER_SVC_FILESYSTEM_DESCRIBE: u16 = 3;

pub(crate) fn page_offset_vaddr(base: usize, page_index: usize, page_size: usize) -> Option<usize> {
    smros_user_page_offset_body!(base, page_index, page_size)
}

pub(crate) fn pfn_to_paddr(pfn: u64, page_size: usize) -> Option<u64> {
    smros_user_pfn_to_paddr_body!(pfn, page_size as u64)
}

pub(crate) fn stack_top_u64(stack_base: u64, stack_size: usize) -> Option<u64> {
    smros_user_stack_top_u64_body!(stack_base, stack_size)
}

pub(crate) fn el0_thread_pstate() -> u64 {
    smros_user_el0_thread_pstate_body!()
}

pub(crate) fn el0_spsr() -> u64 {
    smros_user_el0_spsr_body!()
}

pub(crate) fn el1h_spsr_masked() -> u64 {
    smros_user_el1h_spsr_masked_body!()
}

pub(crate) fn syscall_should_advance_elr() -> u64 {
    smros_user_syscall_should_advance_elr_body!()
}

pub(crate) fn ascii_shell_input(byte: u8) -> bool {
    smros_user_ascii_shell_input_body!(byte)
}

pub(crate) fn decimal_digit_value(byte: u8) -> Option<usize> {
    smros_user_decimal_digit_value_body!(byte)
}

pub(crate) fn parse_digit_step(result: usize, digit: usize) -> Option<usize> {
    smros_user_parse_digit_step_body!(result, digit)
}

pub(crate) fn saturating_sub(lhs: usize, rhs: usize) -> usize {
    smros_user_saturating_sub_body!(lhs, rhs)
}

pub(crate) fn pages_to_kb(pages: usize, page_size: usize) -> usize {
    smros_user_pages_to_kb_body!(pages, page_size)
}

pub(crate) fn usage_percent(used_pages: usize, total_pages: usize) -> usize {
    smros_user_usage_percent_body!(used_pages, total_pages)
}

pub(crate) fn uptime_parts(ticks: u64) -> (u64, u64, u64, u64) {
    smros_user_uptime_parts_body!(ticks)
}

pub(crate) fn mmap_result_ok(addr: u64) -> bool {
    smros_user_mmap_result_ok_body!(addr, 4096u64, USER_MMAP_BASE, USER_MMAP_LIMIT)
}

pub(crate) fn kernel_success(
    kernel_entered: bool,
    kernel_finished: bool,
    exit_code: i32,
    kernel_write: u64,
    kernel_pid: u64,
    kernel_mmap: u64,
    banner_len: usize,
) -> bool {
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

pub(crate) fn component_start_allowed(
    binary_exists: bool,
    destroyed: bool,
    already_started: bool,
) -> bool {
    smros_user_component_start_allowed_body!(binary_exists, destroyed, already_started)
}

pub(crate) fn namespace_rights_valid(rights: u32) -> bool {
    smros_user_namespace_rights_valid_body!(rights, USER_NAMESPACE_RIGHTS_MASK)
}

pub(crate) fn fxfs_file_size_valid(size: usize) -> bool {
    smros_user_fxfs_file_size_valid_body!(size, USER_FXFS_MAX_FILE_BYTES)
}

pub(crate) fn fxfs_node_capacity_valid(nodes: usize) -> bool {
    smros_user_fxfs_node_capacity_valid_body!(nodes, USER_FXFS_MAX_NODES)
}

pub(crate) fn fxfs_dirent_capacity_valid(entries: usize) -> bool {
    smros_user_fxfs_dirent_capacity_valid_body!(entries, USER_FXFS_MAX_DIRENTS)
}

pub(crate) fn fxfs_append_size(old_size: usize, append_len: usize) -> Option<usize> {
    smros_user_fxfs_append_size_body!(old_size, append_len)
}

pub(crate) fn fxfs_write_end(offset: usize, len: usize) -> Option<usize> {
    smros_user_fxfs_write_end_body!(offset, len)
}

pub(crate) fn fxfs_seek_valid(offset: usize, size: usize) -> bool {
    smros_user_fxfs_seek_valid_body!(offset, size)
}

pub(crate) fn fxfs_replay_count_valid(replayed: usize, journal_records: usize) -> bool {
    smros_user_fxfs_replay_count_valid_body!(replayed, journal_records)
}

pub(crate) fn svc_name_valid(len: usize) -> bool {
    smros_user_svc_name_valid_body!(len, USER_SVC_MAX_NAME_LEN)
}

pub(crate) fn svc_rights_valid(rights: u32) -> bool {
    smros_user_svc_rights_valid_body!(rights, USER_SVC_RIGHTS_MASK)
}

pub(crate) fn svc_ipc_message_size_valid(size: usize) -> bool {
    smros_user_svc_ipc_message_size_valid_body!(size, USER_SVC_IPC_MESSAGE_SIZE)
}

pub(crate) fn svc_ipc_header_valid(magic: u32, version: u16) -> bool {
    smros_user_svc_ipc_header_valid_body!(magic, version, USER_SVC_IPC_MAGIC, USER_SVC_IPC_VERSION)
}

pub(crate) fn svc_protocol_allowed(service: u16, ordinal: u16) -> bool {
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

pub(crate) fn component_thread_launch_valid(
    process_created: bool,
    queued: bool,
    thread_created: bool,
) -> bool {
    smros_user_component_thread_launch_valid_body!(process_created, queued, thread_created)
}

pub(crate) fn component_return_active(pid: usize) -> bool {
    smros_user_component_return_active_body!(pid)
}

pub(crate) fn elf_header_bounds_valid(image_len: usize) -> bool {
    smros_user_elf_header_bounds_valid_body!(image_len, USER_ELF_HEADER_SIZE)
}

pub(crate) fn elf_magic_valid(b0: u8, b1: u8, b2: u8, b3: u8) -> bool {
    smros_user_elf_magic_valid_body!(b0, b1, b2, b3)
}

pub(crate) fn elf_class_data_valid(class: u8, data: u8, version: u8) -> bool {
    smros_user_elf_class_data_valid_body!(class, data, version)
}

pub(crate) fn elf_type_valid(elf_type: u16) -> bool {
    smros_user_elf_type_valid_body!(elf_type, USER_ELF_TYPE_EXEC, USER_ELF_TYPE_DYN)
}

pub(crate) fn elf_machine_valid(machine: u16) -> bool {
    smros_user_elf_machine_valid_body!(machine, USER_ELF_MACHINE_AARCH64)
}

pub(crate) fn elf_entry_valid(entry: u64) -> bool {
    smros_user_elf_entry_valid_body!(entry)
}

pub(crate) fn elf_phdr_table_valid(
    phoff: usize,
    phentsize: usize,
    phnum: usize,
    image_len: usize,
) -> bool {
    smros_user_elf_phdr_table_valid_body!(
        phoff,
        phentsize,
        phnum,
        image_len,
        USER_ELF_PHDR_SIZE,
        USER_ELF_MAX_PHDRS
    )
}

pub(crate) fn elf_segment_bounds_valid(
    offset: usize,
    file_size: usize,
    mem_size: usize,
    image_len: usize,
) -> bool {
    smros_user_elf_segment_bounds_valid_body!(offset, file_size, mem_size, image_len)
}

pub(crate) fn elf_vaddr_range_valid(vaddr: u64, mem_size: u64) -> bool {
    smros_user_elf_vaddr_range_valid_body!(vaddr, mem_size)
}
