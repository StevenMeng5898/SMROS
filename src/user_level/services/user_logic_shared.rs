macro_rules! smros_user_checked_end_body {
    ($addr:expr, $len:expr) => {{
        if $addr <= usize::MAX - $len {
            Some($addr + $len)
        } else {
            None
        }
    }};
}

macro_rules! smros_user_page_offset_body {
    ($base:expr, $page_index:expr, $page_size:expr) => {{
        match $page_index.checked_mul($page_size) {
            Some(offset) => smros_user_checked_end_body!($base, offset),
            None => None,
        }
    }};
}

macro_rules! smros_user_page_down_body {
    ($value:expr, $page_size:expr) => {{
        if $page_size == 0 {
            None
        } else {
            $value.checked_sub($value % $page_size)
        }
    }};
}

macro_rules! smros_user_page_up_body {
    ($value:expr, $page_size:expr) => {{
        if $page_size == 0 {
            None
        } else {
            match $value.checked_add($page_size - 1) {
                Some(adjusted) => adjusted.checked_sub(adjusted % $page_size),
                None => None,
            }
        }
    }};
}

macro_rules! smros_user_pfn_to_paddr_body {
    ($pfn:expr, $page_size:expr) => {{
        $pfn.checked_mul($page_size)
    }};
}

macro_rules! smros_user_stack_top_u64_body {
    ($stack_base:expr, $stack_size:expr) => {{
        $stack_base.checked_add($stack_size as u64)
    }};
}

macro_rules! smros_user_el0_thread_state_body {
    () => {
        0x3C0u64
    };
}

macro_rules! smros_user_el0_spsr_body {
    () => {
        0u64
    };
}

macro_rules! smros_user_el1h_spsr_masked_body {
    () => {
        0x3C5u64
    };
}

macro_rules! smros_user_syscall_should_advance_elr_body {
    () => {
        0u64
    };
}

macro_rules! smros_user_ascii_shell_input_body {
    ($byte:expr) => {{
        $byte >= 0x20 && $byte <= 0x7e
    }};
}

macro_rules! smros_user_decimal_digit_value_body {
    ($byte:expr) => {{
        if $byte >= 48u8 && $byte <= 57u8 {
            Some(($byte - 48u8) as usize)
        } else {
            None
        }
    }};
}

macro_rules! smros_user_parse_digit_step_body {
    ($result:expr, $digit:expr) => {{
        match $result.checked_mul(10) {
            Some(scaled) => scaled.checked_add($digit),
            None => None,
        }
    }};
}

macro_rules! smros_user_ipv4_octet_step_body {
    ($value:expr, $digit:expr) => {{
        match $value.checked_mul(10) {
            Some(scaled) => match scaled.checked_add($digit) {
                Some(next) if next <= 255 => Some(next),
                _ => None,
            },
            None => None,
        }
    }};
}

macro_rules! smros_user_saturating_sub_body {
    ($lhs:expr, $rhs:expr) => {{
        if $lhs >= $rhs {
            $lhs - $rhs
        } else {
            0
        }
    }};
}

macro_rules! smros_user_pages_to_kb_body {
    ($pages:expr, $page_size:expr) => {{
        match $pages.checked_mul($page_size) {
            Some(bytes) => bytes / 1024,
            None => usize::MAX,
        }
    }};
}

macro_rules! smros_user_usage_percent_body {
    ($used_pages:expr, $total_pages:expr) => {{
        if $total_pages == 0 {
            0
        } else {
            match $used_pages.checked_mul(100) {
                Some(scaled) => scaled / $total_pages,
                None => usize::MAX,
            }
        }
    }};
}

macro_rules! smros_user_uptime_parts_body {
    ($ticks:expr) => {{
        let seconds = $ticks / 100;
        let minutes = seconds / 60;
        let hours = minutes / 60;
        let days = hours / 24;
        (seconds, minutes, hours, days)
    }};
}

macro_rules! smros_user_mmap_result_ok_body {
    ($addr:expr, $page_size:expr, $base:expr, $limit:expr) => {{
        $page_size != 0 && $addr >= $base && $addr < $limit && $addr % $page_size == 0
    }};
}

macro_rules! smros_user_dns_host_len_valid_body {
    ($len:expr, $max_len:expr) => {{
        $len > 0 && $len <= $max_len
    }};
}

macro_rules! smros_user_dns_label_len_valid_body {
    ($len:expr, $max_len:expr) => {{
        $len > 0 && $len <= $max_len
    }};
}

macro_rules! smros_user_dns_label_byte_valid_body {
    ($byte:expr) => {{
        ($byte >= 0x61u8 && $byte <= 0x7au8)
            || ($byte >= 0x41u8 && $byte <= 0x5au8)
            || ($byte >= 0x30u8 && $byte <= 0x39u8)
            || $byte == 0x2du8
    }};
}

macro_rules! smros_user_kernel_success_body {
    (
        $kernel_entered:expr,
        $kernel_finished:expr,
        $exit_code:expr,
        $kernel_write:expr,
        $kernel_pid:expr,
        $kernel_mmap:expr,
        $banner_len:expr
    ) => {{
        $kernel_entered
            && $kernel_finished
            && $exit_code == 0
            && $kernel_write == $banner_len as u64
            && $kernel_pid == 1
            && $kernel_mmap > 0
            && $kernel_mmap < 0xFFFF_FFFF_FFFF_F000u64
    }};
}

macro_rules! smros_user_component_start_allowed_body {
    ($binary_exists:expr, $destroyed:expr, $already_started:expr) => {{
        $already_started || ($binary_exists && !$destroyed)
    }};
}

macro_rules! smros_user_namespace_rights_valid_body {
    ($rights:expr, $allowed_mask:expr) => {{
        $rights & !$allowed_mask == 0
    }};
}

macro_rules! smros_user_fxfs_file_size_valid_body {
    ($size:expr, $max_size:expr) => {{
        $size <= $max_size
    }};
}

macro_rules! smros_user_fxfs_node_capacity_valid_body {
    ($nodes:expr, $max_nodes:expr) => {{
        $nodes < $max_nodes
    }};
}

macro_rules! smros_user_fxfs_dirent_capacity_valid_body {
    ($entries:expr, $max_entries:expr) => {{
        $entries < $max_entries
    }};
}

macro_rules! smros_user_fxfs_append_size_body {
    ($old_size:expr, $append_len:expr) => {{
        $old_size.checked_add($append_len)
    }};
}

macro_rules! smros_user_fxfs_write_end_body {
    ($offset:expr, $len:expr) => {{
        $offset.checked_add($len)
    }};
}

macro_rules! smros_user_fxfs_seek_valid_body {
    ($offset:expr, $size:expr) => {{
        $offset <= $size
    }};
}

macro_rules! smros_user_fxfs_replay_count_valid_body {
    ($replayed:expr, $journal_records:expr) => {{
        $replayed <= $journal_records
    }};
}

macro_rules! smros_user_svc_name_valid_body {
    ($len:expr, $max_len:expr) => {{
        $len > 0 && $len <= $max_len
    }};
}

macro_rules! smros_user_svc_rights_valid_body {
    ($rights:expr, $allowed_mask:expr) => {{
        $rights != 0 && ($rights & !$allowed_mask) == 0
    }};
}

macro_rules! smros_user_svc_ipc_message_size_valid_body {
    ($size:expr, $expected:expr) => {{
        $size == $expected
    }};
}

macro_rules! smros_user_svc_ipc_header_valid_body {
    ($magic:expr, $version:expr, $expected_magic:expr, $expected_version:expr) => {{
        $magic == $expected_magic && $version == $expected_version
    }};
}

macro_rules! smros_user_svc_protocol_allowed_body {
    ($service:expr, $ordinal:expr, $component_manager:expr, $runner:expr, $filesystem:expr, $component_start:expr, $runner_load:expr, $filesystem_describe:expr) => {{
        ($service == $component_manager && $ordinal == $component_start)
            || ($service == $runner && $ordinal == $runner_load)
            || ($service == $filesystem && $ordinal == $filesystem_describe)
    }};
}

macro_rules! smros_user_component_thread_launch_valid_body {
    ($process_created:expr, $queued:expr, $thread_created:expr) => {{
        $process_created && $queued && $thread_created
    }};
}

macro_rules! smros_user_component_return_active_body {
    ($pid:expr) => {{
        $pid != 0
    }};
}

macro_rules! smros_user_elf_header_bounds_valid_body {
    ($image_len:expr, $header_size:expr) => {{
        $image_len >= $header_size
    }};
}

macro_rules! smros_user_elf_magic_valid_body {
    ($b0:expr, $b1:expr, $b2:expr, $b3:expr) => {{
        $b0 == 0x7fu8 && $b1 == 0x45u8 && $b2 == 0x4cu8 && $b3 == 0x46u8
    }};
}

macro_rules! smros_user_elf_class_data_valid_body {
    ($class:expr, $data:expr, $version:expr) => {{
        $class == 2u8 && $data == 1u8 && $version == 1u8
    }};
}

macro_rules! smros_user_elf_type_valid_body {
    ($elf_type:expr, $exec_type:expr, $dyn_type:expr) => {{
        $elf_type == $exec_type || $elf_type == $dyn_type
    }};
}

macro_rules! smros_user_elf_machine_valid_body {
    ($machine:expr, $expected:expr) => {{
        $machine == $expected
    }};
}

macro_rules! smros_user_elf_entry_valid_body {
    ($entry:expr) => {{
        $entry != 0
    }};
}

macro_rules! smros_user_elf_phdr_table_valid_body {
    ($phoff:expr, $phentsize:expr, $phnum:expr, $image_len:expr, $expected_phentsize:expr, $max_phnum:expr) => {{
        if $phentsize != $expected_phentsize || $phnum == 0 || $phnum > $max_phnum {
            false
        } else {
            match $phentsize.checked_mul($phnum) {
                Some(table_size) => match $phoff.checked_add(table_size) {
                    Some(end) => end <= $image_len,
                    None => false,
                },
                None => false,
            }
        }
    }};
}

macro_rules! smros_user_elf_segment_bounds_valid_body {
    ($offset:expr, $file_size:expr, $mem_size:expr, $image_len:expr) => {{
        if $mem_size < $file_size {
            false
        } else {
            match $offset.checked_add($file_size) {
                Some(end) => end <= $image_len,
                None => false,
            }
        }
    }};
}

macro_rules! smros_user_elf_vaddr_range_valid_body {
    ($vaddr:expr, $mem_size:expr) => {{
        $vaddr.checked_add($mem_size).is_some()
    }};
}

macro_rules! smros_user_elf_segment_mapping_range_body {
    ($vaddr:expr, $mem_size:expr, $page_size:expr) => {{
        if $mem_size == 0 {
            None
        } else {
            match smros_user_checked_end_body!($vaddr, $mem_size) {
                Some(end) => match smros_user_page_down_body!($vaddr, $page_size) {
                    Some(start) => match smros_user_page_up_body!(end, $page_size) {
                        Some(aligned_end) => Some((start, aligned_end)),
                        None => None,
                    },
                    None => None,
                },
                None => None,
            }
        }
    }};
}

pub(crate) fn run_elf_environment_entry_valid(entry: &str, max_entry_bytes: usize) -> bool {
    if entry.is_empty() || entry.len() > max_entry_bytes || entry.as_bytes().contains(&0) {
        return false;
    }
    let Some(separator) = entry.as_bytes().iter().position(|byte| *byte == b'=') else {
        return false;
    };
    let key = &entry.as_bytes()[..separator];
    let Some(first) = key.first() else {
        return false;
    };
    if !first.is_ascii_alphabetic() && *first != b'_' {
        return false;
    }
    key[1..]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

pub(crate) fn run_elf_environment_totals_valid(
    entry_count: usize,
    total_bytes: usize,
    max_entries: usize,
    max_total_bytes: usize,
) -> bool {
    entry_count <= max_entries && total_bytes <= max_total_bytes
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RunElfEnvironmentTotals {
    pub(crate) entry_count: usize,
    pub(crate) total_bytes: usize,
    pub(crate) append_default: bool,
}

pub(crate) fn run_elf_environment_effective_totals(
    caller_count: usize,
    caller_total_bytes: usize,
    has_caller_library_path: bool,
    default_entry_bytes: usize,
) -> Option<RunElfEnvironmentTotals> {
    let append_default = !has_caller_library_path;
    let entry_count = caller_count.checked_add(usize::from(append_default))?;
    let total_bytes = if append_default {
        caller_total_bytes.checked_add(default_entry_bytes)?
    } else {
        caller_total_bytes
    };
    Some(RunElfEnvironmentTotals {
        entry_count,
        total_bytes,
        append_default,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunElfEnvironmentSource {
    Caller(usize),
    Default,
}

pub(crate) fn run_elf_environment_source_at(
    output_index: usize,
    caller_count: usize,
    has_caller_library_path: bool,
) -> Option<RunElfEnvironmentSource> {
    if output_index < caller_count {
        Some(RunElfEnvironmentSource::Caller(output_index))
    } else if output_index == caller_count && !has_caller_library_path {
        Some(RunElfEnvironmentSource::Default)
    } else {
        None
    }
}

pub(crate) fn run_elf_environment_entry_has_key(entry: &str, key: &str) -> bool {
    entry
        .as_bytes()
        .iter()
        .position(|byte| *byte == b'=')
        .map(|separator| &entry.as_bytes()[..separator] == key.as_bytes())
        .unwrap_or(false)
}

pub(crate) fn run_elf_environment_keys_equal(left: &str, right: &str) -> bool {
    let Some(left_separator) = left.as_bytes().iter().position(|byte| *byte == b'=') else {
        return false;
    };
    let Some(right_separator) = right.as_bytes().iter().position(|byte| *byte == b'=') else {
        return false;
    };
    left.as_bytes()[..left_separator] == right.as_bytes()[..right_separator]
}

pub(crate) fn run_elf_environment_valid<T: AsRef<str>>(
    env: &[T],
    library_path_key: &str,
    default_entry_bytes: usize,
    max_entries: usize,
    max_entry_bytes: usize,
    max_total_bytes: usize,
) -> bool {
    let mut total_bytes = 0usize;
    let mut has_library_path = false;

    for (index, entry) in env.iter().enumerate() {
        let entry = entry.as_ref();
        if !run_elf_environment_entry_valid(entry, max_entry_bytes) {
            return false;
        }
        let Some(entry_bytes) = entry.len().checked_add(1) else {
            return false;
        };
        let Some(next_total) = total_bytes.checked_add(entry_bytes) else {
            return false;
        };
        total_bytes = next_total;
        has_library_path |= run_elf_environment_entry_has_key(entry, library_path_key);
        if env[..index]
            .iter()
            .any(|previous| run_elf_environment_keys_equal(previous.as_ref(), entry))
        {
            return false;
        }
    }

    let Some(effective) = run_elf_environment_effective_totals(
        env.len(),
        total_bytes,
        has_library_path,
        default_entry_bytes,
    ) else {
        return false;
    };
    run_elf_environment_totals_valid(
        effective.entry_count,
        effective.total_bytes,
        max_entries,
        max_total_bytes,
    )
}

pub(crate) struct RunElfStateCell<T> {
    locked: core::sync::atomic::AtomicBool,
    value: core::cell::UnsafeCell<T>,
}

// SAFETY: Access to the contained value is serialized by `lock`.
unsafe impl<T: Send> Sync for RunElfStateCell<T> {}

impl<T> RunElfStateCell<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self {
            locked: core::sync::atomic::AtomicBool::new(false),
            value: core::cell::UnsafeCell::new(value),
        }
    }

    pub(crate) fn lock(&self) -> RunElfStateGuard<'_, T> {
        while self
            .locked
            .compare_exchange_weak(
                false,
                true,
                core::sync::atomic::Ordering::Acquire,
                core::sync::atomic::Ordering::Relaxed,
            )
            .is_err()
        {
            core::hint::spin_loop();
        }
        RunElfStateGuard { cell: self }
    }
}

pub(crate) struct RunElfStateGuard<'a, T> {
    cell: &'a RunElfStateCell<T>,
}

impl<T> core::ops::Deref for RunElfStateGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: The guard owns the state cell's lock for its lifetime.
        unsafe { &*self.cell.value.get() }
    }
}

impl<T> core::ops::DerefMut for RunElfStateGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: The guard owns the state cell's lock for its lifetime.
        unsafe { &mut *self.cell.value.get() }
    }
}

impl<T> Drop for RunElfStateGuard<'_, T> {
    fn drop(&mut self) {
        self.cell
            .locked
            .store(false, core::sync::atomic::Ordering::Release);
    }
}

pub(crate) struct RunElfLifecycleState<T> {
    request: Option<T>,
    return_pending: bool,
    exit_code: i32,
    completion_seen: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RunElfCompletion<T> {
    Requested(T),
    Repeated,
    MissingRequest,
}

pub(crate) struct RunElfTaken<T> {
    pub(crate) completion: RunElfCompletion<T>,
    pub(crate) exit_code: i32,
}

pub(crate) struct RunElfOwnedResource<T> {
    resource: Option<T>,
    release: fn(T),
}

impl<T> RunElfOwnedResource<T> {
    pub(crate) fn new(resource: T, release: fn(T)) -> Self {
        Self {
            resource: Some(resource),
            release,
        }
    }
}

impl<T> Drop for RunElfOwnedResource<T> {
    fn drop(&mut self) {
        if let Some(resource) = self.resource.take() {
            (self.release)(resource);
        }
    }
}

pub(crate) struct RunElfActiveRequest<T, R> {
    launch: T,
    resource: Option<R>,
}

impl<T, R> RunElfActiveRequest<T, R> {
    pub(crate) const fn new(launch: T) -> Self {
        Self {
            launch,
            resource: None,
        }
    }

    pub(crate) fn launch(&self) -> &T {
        &self.launch
    }

    pub(crate) fn attach_resource(&mut self, resource: R) -> Result<(), R> {
        if self.resource.is_some() {
            return Err(resource);
        }
        self.resource = Some(resource);
        Ok(())
    }

    pub(crate) fn into_parts(self) -> (T, Option<R>) {
        (self.launch, self.resource)
    }
}

impl<T> RunElfLifecycleState<T> {
    pub(crate) const fn new() -> Self {
        Self {
            request: None,
            return_pending: false,
            exit_code: 0,
            completion_seen: false,
        }
    }

    pub(crate) fn request(&self) -> Option<&T> {
        self.request.as_ref()
    }

    pub(crate) fn request_mut(&mut self) -> Option<&mut T> {
        self.request.as_mut()
    }

    pub(crate) fn try_start(&mut self, request: T) -> Result<(), T> {
        if self.request.is_some() {
            return Err(request);
        }
        self.request = Some(request);
        self.return_pending = false;
        self.exit_code = 0;
        self.completion_seen = false;
        Ok(())
    }

    pub(crate) fn clear_without_completion(&mut self) -> Option<T> {
        self.return_pending = false;
        self.exit_code = 0;
        self.completion_seen = false;
        self.request.take()
    }

    pub(crate) fn prepare_return(&mut self, exit_code: i32) -> bool {
        if self.request.is_none() || self.return_pending {
            return false;
        }
        self.return_pending = true;
        self.exit_code = exit_code;
        true
    }

    pub(crate) fn exit_code(&self) -> i32 {
        self.exit_code
    }

    pub(crate) fn take_completion(&mut self) -> RunElfCompletion<T> {
        self.return_pending = false;
        self.exit_code = 0;
        if let Some(request) = self.request.take() {
            self.completion_seen = true;
            RunElfCompletion::Requested(request)
        } else if self.completion_seen {
            RunElfCompletion::Repeated
        } else {
            self.completion_seen = true;
            RunElfCompletion::MissingRequest
        }
    }
}

pub(crate) fn run_elf_start_transition<T>(
    state: &mut RunElfLifecycleState<T>,
    request: T,
    reset: impl FnOnce(),
) -> Result<(), T> {
    let result = state.try_start(request);
    if result.is_ok() {
        reset();
    }
    result
}

pub(crate) fn run_elf_prepare_return_transition<T>(
    state: &mut RunElfLifecycleState<T>,
    exit_code: i32,
    reset: impl FnOnce(),
) -> bool {
    let prepared = state.prepare_return(exit_code);
    if prepared {
        reset();
    }
    prepared
}

pub(crate) fn run_elf_take_completion_transition<T>(
    state: &mut RunElfLifecycleState<T>,
    reset: impl FnOnce(),
) -> RunElfTaken<T> {
    let exit_code = state.exit_code();
    let completion = state.take_completion();
    reset();
    RunElfTaken {
        completion,
        exit_code,
    }
}

pub(crate) fn run_elf_clear_transition<T>(
    state: &mut RunElfLifecycleState<T>,
    reset: impl FnOnce(),
) -> Option<T> {
    let request = state.clear_without_completion();
    reset();
    request
}

pub(crate) fn run_elf_attach_resource_transition<T, R>(
    state: &mut RunElfLifecycleState<RunElfActiveRequest<T, R>>,
    resource: R,
) -> Result<(), R> {
    match state.request_mut() {
        Some(request) => request.attach_resource(resource),
        None => Err(resource),
    }
}

pub(crate) fn run_elf_exit_succeeded(exit_code: i32) -> bool {
    exit_code == 0
}

pub(crate) fn run_elf_elapsed_ticks(start_tick: u64, end_tick: u64) -> u64 {
    end_tick.saturating_sub(start_tick)
}

pub(crate) fn run_elf_library_name_valid(name_or_path: &str) -> bool {
    if name_or_path.is_empty() || name_or_path.as_bytes().contains(&0) {
        return false;
    }

    let absolute = name_or_path.starts_with('/');
    let mut components = name_or_path.split('/');
    if absolute && components.next() != Some("") {
        return false;
    }

    let mut saw_component = false;
    for component in components {
        if component.is_empty() || component == "." || component == ".." {
            return false;
        }
        if !component
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-+".contains(&byte))
        {
            return false;
        }
        saw_component = true;
    }
    saw_component
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunElfLibrarySearchStage {
    Posix,
    Shared,
    System,
    Direct,
}

pub(crate) fn run_elf_library_search_stage(index: usize) -> Option<RunElfLibrarySearchStage> {
    match index {
        0 => Some(RunElfLibrarySearchStage::Posix),
        1 => Some(RunElfLibrarySearchStage::Shared),
        2 => Some(RunElfLibrarySearchStage::System),
        3 => Some(RunElfLibrarySearchStage::Direct),
        _ => None,
    }
}
