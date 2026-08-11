pub const MEMORY_PERMANENT_HANDLE_COUNT: usize = 1;

pub fn logical_memory_handle_count(observed: Option<usize>) -> usize {
    observed.unwrap_or(MEMORY_PERMANENT_HANDLE_COUNT)
}

pub fn linux_exit_status(exit_code: i32) -> i32 {
    (exit_code as u32 & 0xff) as i32
}

pub fn linux_fxfs_stat_identity(object_id: u64) -> Option<(u64, u64)> {
    if object_id == 0 {
        None
    } else {
        Some((1, object_id))
    }
}

macro_rules! smros_zircon_syscall_from_raw_body {
    ($syscall_num:expr, $threshold:expr) => {{
        if smros_is_zircon_syscall_number_body!($syscall_num, $threshold) {
            ($syscall_num - $threshold) as u32
        } else {
            u32::MAX
        }
    }};
}

macro_rules! smros_is_zircon_syscall_number_body {
    ($syscall_num:expr, $threshold:expr) => {{
        $syscall_num >= $threshold && $syscall_num - $threshold <= u32::MAX as u64
    }};
}

macro_rules! smros_syscall_handle_invalid_body {
    ($handle:expr, $invalid:expr) => {{
        $handle == 0 || $handle == $invalid
    }};
}

macro_rules! smros_syscall_user_buffer_valid_body {
    ($ptr:expr, $len:expr) => {{
        $len == 0 || $ptr != 0
    }};
}

macro_rules! smros_syscall_channel_buffers_valid_body {
    ($bytes_ptr:expr, $bytes_len:expr, $handles_ptr:expr, $handles_len:expr) => {{
        smros_syscall_user_buffer_valid_body!($bytes_ptr, $bytes_len)
            && smros_syscall_user_buffer_valid_body!($handles_ptr, $handles_len)
    }};
}

macro_rules! smros_syscall_signal_update_body {
    ($current:expr, $clear_mask:expr, $set_mask:expr) => {{
        ($current & !$clear_mask) | $set_mask
    }};
}

macro_rules! smros_syscall_signal_mask_allowed_body {
    ($clear_mask:expr, $set_mask:expr, $allowed_mask:expr) => {{
        (($clear_mask | $set_mask) & !$allowed_mask) == 0
    }};
}

macro_rules! smros_syscall_user_signal_mask_body {
    () => {{
        0xffu32 << 24
    }};
}

macro_rules! smros_syscall_event_signal_mask_body {
    () => {{
        smros_syscall_user_signal_mask_body!() | (1u32 << 4)
    }};
}

macro_rules! smros_syscall_eventpair_signal_mask_body {
    () => {{
        smros_syscall_user_signal_mask_body!() | (1u32 << 4)
    }};
}

macro_rules! smros_syscall_wait_satisfied_body {
    ($observed:expr, $requested:expr) => {{
        $requested == 0 || ($observed & $requested) != 0
    }};
}

macro_rules! smros_linux_clock_id_supported_body {
    ($clock_id:expr) => {{
        $clock_id <= 1
    }};
}

macro_rules! smros_linux_clock_nanosleep_flags_valid_body {
    ($flags:expr, $timer_abstime:expr) => {{
        ($flags & !$timer_abstime) == 0
    }};
}

const LINUX_POSIX_NANOS_PER_SECOND: u64 = 1_000_000_000;

pub(crate) fn linux_posix_clock_settable(clock_id: usize) -> bool {
    clock_id == 0
}

pub(crate) fn linux_posix_timespec_nanoseconds(
    seconds: i64,
    nanoseconds: i64,
) -> Option<u64> {
    if seconds < 0 || !(0..LINUX_POSIX_NANOS_PER_SECOND as i64).contains(&nanoseconds) {
        return None;
    }
    (seconds as u64)
        .checked_mul(LINUX_POSIX_NANOS_PER_SECOND)?
        .checked_add(nanoseconds as u64)
}

pub(crate) fn linux_realtime_offset_for_set(
    monotonic_nanoseconds: u64,
    seconds: i64,
    nanoseconds: i64,
) -> Option<i64> {
    let requested = linux_posix_timespec_nanoseconds(seconds, nanoseconds)?;
    let offset = i128::from(requested) - i128::from(monotonic_nanoseconds);
    i64::try_from(offset).ok()
}

pub(crate) fn linux_realtime_from_offset(
    monotonic_nanoseconds: u64,
    offset_nanoseconds: i64,
) -> Option<u64> {
    if offset_nanoseconds >= 0 {
        monotonic_nanoseconds.checked_add(offset_nanoseconds as u64)
    } else {
        monotonic_nanoseconds.checked_sub(offset_nanoseconds.unsigned_abs())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxPosixClock {
    Realtime,
    Monotonic,
}

impl LinuxPosixClock {
    pub(crate) fn from_id(clock_id: usize) -> Option<Self> {
        match clock_id {
            0 => Some(Self::Realtime),
            1 => Some(Self::Monotonic),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxPosixTimerSpec {
    pub interval: u64,
    pub value: u64,
}

impl LinuxPosixTimerSpec {
    pub(crate) const DISARMED: Self = Self {
        interval: 0,
        value: 0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxPosixTimerCore {
    pub handle: u32,
    pub clock: LinuxPosixClock,
    pub signal: usize,
    deadline_clock: LinuxPosixClock,
    deadline: Option<u64>,
    interval: u64,
}

impl LinuxPosixTimerCore {
    pub(crate) const fn new(handle: u32, clock: LinuxPosixClock, signal: usize) -> Self {
        Self {
            handle,
            clock,
            signal,
            deadline_clock: LinuxPosixClock::Monotonic,
            deadline: None,
            interval: 0,
        }
    }

    pub(crate) fn arm(
        &mut self,
        absolute: bool,
        now_monotonic: u64,
        spec: LinuxPosixTimerSpec,
    ) -> Option<()> {
        if spec.value == 0 {
            self.deadline = None;
            self.interval = 0;
            return Some(());
        }
        let (deadline_clock, deadline) = if absolute {
            (self.clock, spec.value)
        } else {
            (
                LinuxPosixClock::Monotonic,
                now_monotonic.checked_add(spec.value)?,
            )
        };
        self.deadline_clock = deadline_clock;
        self.deadline = Some(deadline);
        self.interval = spec.interval;
        Some(())
    }

    pub(crate) fn snapshot(
        &self,
        now_monotonic: u64,
        now_realtime: u64,
    ) -> LinuxPosixTimerSpec {
        let now = match self.deadline_clock {
            LinuxPosixClock::Monotonic => now_monotonic,
            LinuxPosixClock::Realtime => now_realtime,
        };
        LinuxPosixTimerSpec {
            interval: self.interval,
            value: self
                .deadline
                .map(|deadline| deadline.saturating_sub(now))
                .unwrap_or(0),
        }
    }

    pub(crate) fn expire(&mut self, now_monotonic: u64, now_realtime: u64) -> bool {
        let Some(deadline) = self.deadline else {
            return false;
        };
        let now = match self.deadline_clock {
            LinuxPosixClock::Monotonic => now_monotonic,
            LinuxPosixClock::Realtime => now_realtime,
        };
        if now < deadline {
            return false;
        }
        if self.interval == 0 {
            self.deadline = None;
            return true;
        }

        self.deadline = now
            .saturating_sub(deadline)
            .checked_div(self.interval)
            .and_then(|periods| periods.checked_add(1))
            .and_then(|periods| periods.checked_mul(self.interval))
            .and_then(|advance| deadline.checked_add(advance));
        if self.deadline.is_none() {
            self.interval = 0;
        }
        true
    }
}

macro_rules! smros_linux_signal_valid_body {
    ($signum:expr, $max_signal:expr) => {{
        $signum <= $max_signal
    }};
}

macro_rules! smros_linux_signal_action_valid_body {
    ($signum:expr, $max_signal:expr) => {{
        $signum != 0 && $signum <= $max_signal && $signum != 9 && $signum != 19
    }};
}

macro_rules! smros_linux_sigset_size_valid_body {
    ($size:expr, $expected:expr) => {{
        $size == $expected
    }};
}

macro_rules! smros_linux_ipc_count_valid_body {
    ($count:expr, $max_count:expr) => {{
        $count != 0 && $count <= $max_count
    }};
}

macro_rules! smros_linux_ipc_size_valid_body {
    ($size:expr, $max_size:expr) => {{
        $size != 0 && $size <= $max_size
    }};
}

macro_rules! smros_linux_msg_size_valid_body {
    ($size:expr, $max_size:expr) => {{
        $size <= $max_size
    }};
}

macro_rules! smros_linux_socket_domain_supported_body {
    ($domain:expr, $unix:expr, $local:expr, $inet:expr, $netlink:expr, $packet:expr) => {{
        $domain == $unix
            || $domain == $local
            || $domain == $inet
            || $domain == $netlink
            || $domain == $packet
    }};
}

macro_rules! smros_linux_socket_type_supported_body {
    ($socket_type:expr, $mask:expr, $stream:expr, $dgram:expr, $raw:expr) => {{
        {
            let kind = $socket_type & $mask;
            kind == $stream || kind == $dgram || kind == $raw
        }
    }};
}

macro_rules! smros_linux_socket_domain_type_supported_body {
    ($domain:expr, $kind:expr, $unix:expr, $local:expr, $inet:expr, $netlink:expr, $packet:expr, $stream:expr, $dgram:expr, $raw:expr) => {{
        if $domain == $unix || $domain == $local {
            $kind == $stream || $kind == $dgram
        } else if $domain == $inet {
            $kind == $stream || $kind == $dgram || $kind == $raw
        } else if $domain == $netlink || $domain == $packet {
            $kind == $dgram || $kind == $raw
        } else {
            false
        }
    }};
}

macro_rules! smros_linux_socket_addr_valid_body {
    ($ptr:expr, $len:expr) => {{
        smros_syscall_user_buffer_valid_body!($ptr, $len)
    }};
}

macro_rules! smros_linux_fd_range_valid_body {
    ($first:expr, $last:expr) => {{
        $first <= $last
    }};
}

macro_rules! smros_linux_memfd_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_getrandom_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_open_access_mode_valid_body {
    ($flags:expr, $access_mask:expr, $read_only:expr, $write_only:expr, $read_write:expr) => {{
        {
            let access = $flags & $access_mask;
            access == $read_only || access == $write_only || access == $read_write
        }
    }};
}

macro_rules! smros_linux_open_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_open_is_directory_body {
    ($flags:expr, $directory_flag:expr) => {{
        ($flags & $directory_flag) != 0
    }};
}

macro_rules! smros_linux_fd_target_valid_body {
    ($fd:expr, $stdio_max:expr) => {{
        $fd <= $stdio_max
    }};
}

macro_rules! smros_linux_pipe_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_namespace_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_dup3_args_valid_body {
    ($old_fd:expr, $new_fd:expr) => {{
        $old_fd != $new_fd
    }};
}

macro_rules! smros_linux_fcntl_cmd_supported_body {
    ($cmd:expr, $dupfd:expr, $getfd:expr, $setfd:expr, $getfl:expr, $setfl:expr, $dupfd_cloexec:expr) => {{
        $cmd == $dupfd
            || $cmd == $getfd
            || $cmd == $setfd
            || $cmd == $getfl
            || $cmd == $setfl
            || $cmd == $dupfd_cloexec
    }};
}

macro_rules! smros_linux_fcntl_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_path_mode_valid_body {
    ($mode:expr, $allowed_mask:expr) => {{
        ($mode & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_unlink_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_rename_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_stat_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_stat_mask_valid_body {
    ($mask:expr, $allowed_mask:expr) => {{
        ($mask & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_lseek_whence_valid_body {
    ($whence:expr, $max_whence:expr) => {{
        $whence <= $max_whence
    }};
}

macro_rules! smros_linux_iov_count_valid_body {
    ($count:expr, $max_count:expr) => {{
        $count <= $max_count
    }};
}

macro_rules! smros_linux_iov_bytes_valid_body {
    ($count:expr, $elem_size:expr, $max_count:expr) => {{
        $elem_size != 0 && $count <= $max_count && $count <= usize::MAX / $elem_size
    }};
}

macro_rules! smros_linux_poll_count_valid_body {
    ($count:expr, $max_count:expr) => {{
        $count <= $max_count
    }};
}

macro_rules! smros_linux_poll_events_valid_body {
    ($events:expr, $allowed_mask:expr) => {{
        ($events & !$allowed_mask) == 0
    }};
}

macro_rules! smros_linux_copy_flags_valid_body {
    ($flags:expr, $allowed_mask:expr) => {{
        ($flags & !$allowed_mask) == 0
    }};
}

macro_rules! smros_zircon_clock_id_supported_body {
    ($clock_id:expr) => {{
        $clock_id <= 1
    }};
}

macro_rules! smros_zircon_clock_create_options_valid_body {
    ($options:expr, $allowed_mask:expr) => {{
        ($options & !$allowed_mask) == 0
    }};
}

macro_rules! smros_zircon_clock_update_options_valid_body {
    ($options:expr, $allowed_mask:expr) => {{
        ($options & !$allowed_mask) == 0
    }};
}

macro_rules! smros_zircon_timer_options_valid_body {
    ($options:expr, $allowed_mask:expr) => {{
        ($options & !$allowed_mask) == 0
    }};
}

macro_rules! smros_zircon_timer_deadline_expired_body {
    ($deadline:expr, $now:expr) => {{
        $deadline <= $now
    }};
}

macro_rules! smros_zircon_debuglog_create_options_valid_body {
    ($options:expr, $allowed_mask:expr) => {{
        ($options & !$allowed_mask) == 0
    }};
}

macro_rules! smros_zircon_debuglog_io_options_valid_body {
    ($options:expr, $allowed_mask:expr) => {{
        ($options & !$allowed_mask) == 0
    }};
}

macro_rules! smros_zircon_system_event_kind_valid_body {
    ($kind:expr, $max_kind:expr) => {{
        $kind <= $max_kind
    }};
}

macro_rules! smros_zircon_exception_channel_options_valid_body {
    ($options:expr, $allowed_mask:expr) => {{
        ($options & !$allowed_mask) == 0
    }};
}

macro_rules! smros_zircon_hypervisor_options_valid_body {
    ($options:expr, $allowed_mask:expr) => {{
        ($options & !$allowed_mask) == 0
    }};
}

macro_rules! smros_zircon_guest_trap_kind_valid_body {
    ($kind:expr, $max_kind:expr) => {{
        $kind <= $max_kind
    }};
}

macro_rules! smros_zircon_guest_trap_is_bell_body {
    ($kind:expr, $bell:expr) => {{
        $kind == $bell
    }};
}

macro_rules! smros_zircon_guest_trap_is_mem_body {
    ($kind:expr, $mem:expr) => {{
        $kind == $mem
    }};
}

macro_rules! smros_zircon_guest_trap_range_valid_body {
    ($addr:expr, $size:expr, $limit:expr) => {{
        $size != 0 && $addr <= $limit && $size <= $limit - $addr
    }};
}

macro_rules! smros_zircon_guest_trap_alignment_valid_body {
    ($kind:expr, $addr:expr, $size:expr, $bell:expr, $mem:expr, $page_size:expr) => {{
        if $kind == $bell || $kind == $mem {
            $page_size != 0 && $addr % $page_size == 0 && $size % $page_size == 0
        } else {
            true
        }
    }};
}

macro_rules! smros_zircon_vcpu_entry_valid_body {
    ($entry:expr, $alignment:expr) => {{
        $alignment != 0 && $entry % $alignment == 0
    }};
}

macro_rules! smros_zircon_vcpu_interrupt_vector_valid_body {
    ($vector:expr, $max_vector:expr) => {{
        $vector <= $max_vector
    }};
}

macro_rules! smros_zircon_vcpu_read_state_args_valid_body {
    ($kind:expr, $buffer_size:expr, $state_kind:expr, $state_size:expr) => {{
        $kind == $state_kind && $buffer_size == $state_size
    }};
}

macro_rules! smros_zircon_vcpu_write_state_args_valid_body {
    ($kind:expr, $buffer_size:expr, $state_kind:expr, $state_size:expr, $io_kind:expr, $io_size:expr) => {{
        ($kind == $state_kind && $buffer_size == $state_size)
            || ($kind == $io_kind && $buffer_size == $io_size)
    }};
}

macro_rules! smros_linux_syscall_interface_known_body {
    ($syscall_num:expr) => {{
        $syscall_num <= 446 || $syscall_num == 600
    }};
}

macro_rules! smros_zircon_syscall_interface_known_body {
    ($syscall_num:expr) => {{
        $syscall_num <= 154 || (183 <= $syscall_num && $syscall_num <= 211)
    }};
}
