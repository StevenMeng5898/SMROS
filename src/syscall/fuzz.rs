use super::{
    dispatch_linux_syscall, dispatch_zircon_syscall, memory_root_vmar_handle, sys_bti_create,
    sys_bti_pin, sys_channel_create, sys_clock_create, sys_close, sys_debuglog_create,
    sys_event_create, sys_eventfd2, sys_fifo_create, sys_handle_close, sys_interrupt_create,
    sys_iommu_create, sys_job_create, sys_mmap, sys_msgget, sys_munmap, sys_openat,
    sys_pager_create, sys_pager_create_vmo, sys_pci_get_nth_device, sys_pipe2, sys_port_create,
    sys_process_create, sys_profile_create, sys_resource_create, sys_semget, sys_shmget,
    sys_socket, sys_socket_create, sys_socketpair, sys_stream_create, sys_thread_create,
    sys_timer_create, sys_timerfd_create, sys_vcpu_create, sys_vmo_create, LinuxCapUserData,
    LinuxCapUserHeader, LinuxIovec, LinuxItimerval, LinuxPollFd, LinuxTimespec, LinuxTimeval,
    MmapFlags, MmapProt, SysError, ZxError, ZxWaitItem, LINUX_AF_UNIX, LINUX_CAPABILITY_VERSION_3,
    LINUX_CONTAINER_NAMESPACE_FLAGS, LINUX_O_CLOEXEC, LINUX_O_CREAT, LINUX_O_DIRECTORY,
    LINUX_O_RDWR, LINUX_SECCOMP_FILTER_ALLOWED_FLAGS, LINUX_SECCOMP_GET_NOTIF_SIZES,
    LINUX_SECCOMP_SET_MODE_FILTER, LINUX_SIGSET_SIZE, LINUX_SOCK_DGRAM, LINUX_SOCK_STREAM,
};
use crate::kernel_lowlevel::memory::PAGE_SIZE;
use crate::kernel_lowlevel::timer;
use crate::kernel_objects::port::{PortPacket, PORT_PACKET_TYPE_USER};
use crate::kernel_objects::{ObjectType, VmOptions, VmarFlags, INVALID_HANDLE, RIGHT_SAME_RIGHTS};
use crate::syscall::syscall_logic;
use crate::user_level::fxfs;

const FUZZ_SCRATCH_BYTES: usize = 4096;
const FUZZ_DEFAULT_ITERATIONS: usize = 2;
const FUZZ_IO_BYTES: usize = 64;
const FUZZ_LINUX_NEW_FD_BASE: usize = 10_000;
const FUZZ_LINUX_NEW_FD_SPAN: usize = 1_000;
const FUZZ_LINUX_INTERFACE_MAX: u32 = 600;
const FUZZ_ZIRCON_INTERFACE_MAX: u32 = 211;
const FUZZ_ERROR_BUCKETS: usize = 16;
const FUZZ_SCRATCH_FDS: usize = 16;
const FUZZ_ZIRCON_VMAR_ALLOC_OFFSET: usize = 0x0200_0000;
const FUZZ_ZIRCON_VMAR_MAP_OFFSET: usize = 0x0300_0000;
const ZX_USER_SIGNAL_0: usize = 1 << 24;
const ZX_VCPU_STATE_SIZE: usize = 256;
const ZX_VCPU_IO_SIZE: usize = 24;

const FUZZ_PROCESS_HANDLE_INDEX: usize = 18;
const FUZZ_PROCESS_VMAR_HANDLE_INDEX: usize = 19;
const FUZZ_THREAD_HANDLE_INDEX: usize = 20;
const FUZZ_INTERRUPT_HANDLE_INDEX: usize = 21;
const FUZZ_IOMMU_HANDLE_INDEX: usize = 22;
const FUZZ_BTI_HANDLE_INDEX: usize = 23;
const FUZZ_RESOURCE_HANDLE_INDEX: usize = 25;
const FUZZ_PCI_HANDLE_INDEX: usize = 26;
const FUZZ_GUEST_HANDLE_INDEX: usize = 27;
const FUZZ_GUEST_VMAR_HANDLE_INDEX: usize = 28;
const FUZZ_VCPU_HANDLE_INDEX: usize = 29;
const FUZZ_PAGER_HANDLE_INDEX: usize = 30;
const FUZZ_PAGER_VMO_HANDLE_INDEX: usize = 31;
const FUZZ_STREAM_HANDLE_INDEX: usize = 32;
const FUZZ_PROFILE_HANDLE_INDEX: usize = 33;
const FUZZ_EXCEPTION_HANDLE_INDEX: usize = 34;

const ZIRCON_SUCCESS_SYSCALLS: &[u32] = &[
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49,
    50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73,
    74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97,
    98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116,
    117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135,
    136, 137, 138, 139, 140, 141, 142, 143, 144, 145, 146, 147, 148, 149, 150, 151, 152, 153, 154,
    183, 184, 185, 186, 187, 188, 189, 190, 191, 192, 193, 194, 195, 196, 197, 198, 199, 200, 201,
    202, 203, 204, 205, 206, 207, 208, 209, 210, 211,
];

#[derive(Clone, Copy, Default)]
pub struct SyscallFuzzReport {
    pub seed: u64,
    pub iterations: usize,
    pub completed_iterations: usize,
    pub time_limit_ticks: u64,
    pub elapsed_ticks: u64,
    pub timed_out: bool,
    pub linux_interface_syscalls: usize,
    pub linux_success_syscalls: usize,
    pub linux_success_call_cases: usize,
    pub linux_calls: usize,
    pub linux_ok: usize,
    pub linux_err: usize,
    pub linux_enosys: usize,
    pub linux_first_err_syscall: u32,
    pub linux_last_err_syscall: u32,
    pub linux_err_syscalls: [u32; FUZZ_ERROR_BUCKETS],
    pub linux_err_syscall_counts: [usize; FUZZ_ERROR_BUCKETS],
    pub linux_err_syscall_count: usize,
    pub linux_first_enosys_syscall: u32,
    pub linux_last_enosys_syscall: u32,
    pub zircon_interface_syscalls: usize,
    pub zircon_success_syscalls: usize,
    pub zircon_success_call_cases: usize,
    pub zircon_calls: usize,
    pub zircon_ok: usize,
    pub zircon_err: usize,
    pub zircon_unsupported: usize,
    pub zircon_first_err_syscall: u32,
    pub zircon_last_err_syscall: u32,
    pub zircon_err_syscalls: [u32; FUZZ_ERROR_BUCKETS],
    pub zircon_err_syscall_counts: [usize; FUZZ_ERROR_BUCKETS],
    pub zircon_err_syscall_count: usize,
    pub zircon_first_unsupported_syscall: u32,
    pub zircon_last_unsupported_syscall: u32,
    pub skipped: usize,
    pub created_handles: usize,
    pub created_fds: usize,
}

#[derive(Clone, Copy)]
pub struct SyscallFuzzConfig {
    pub seed: u64,
    pub iterations: usize,
    pub time_limit_ticks: u64,
}

impl SyscallFuzzConfig {
    pub const fn new(seed: u64, iterations: usize) -> Self {
        Self {
            seed,
            iterations,
            time_limit_ticks: 0,
        }
    }
}

struct FuzzRng {
    state: u64,
}

impl FuzzRng {
    fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9e37_79b9_7f4a_7c15,
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn next_usize(&mut self) -> usize {
        self.next_u64() as usize
    }

    fn below(&mut self, limit: usize) -> usize {
        if limit == 0 {
            0
        } else {
            self.next_usize() % limit
        }
    }

    fn choose<'a>(&mut self, values: &'a [usize]) -> usize {
        values[self.below(values.len())]
    }
}

struct FuzzArena {
    scratch: [u8; FUZZ_SCRATCH_BYTES],
    path: [u8; 32],
    path_alt: [u8; 32],
    name: [u8; 32],
    iov: [LinuxIovec; 2],
    poll: [LinuxPollFd; 2],
    wait_items: [ZxWaitItem; 2],
    packet: PortPacket,
    timespec: LinuxTimespec,
    timeval: LinuxTimeval,
    itimer: LinuxItimerval,
    cap_header: LinuxCapUserHeader,
    cap_data: [LinuxCapUserData; 2],
    handles: [u32; 8],
    words: [usize; 16],
    futex: i32,
    alt_futex: i32,
}

impl FuzzArena {
    fn new(seed: u64) -> Self {
        let mut arena = Self {
            scratch: [0; FUZZ_SCRATCH_BYTES],
            path: [0; 32],
            path_alt: [0; 32],
            name: [0; 32],
            iov: [
                LinuxIovec { base: 0, len: 0 },
                LinuxIovec { base: 0, len: 0 },
            ],
            poll: [
                LinuxPollFd {
                    fd: -1,
                    events: 0,
                    revents: 0,
                },
                LinuxPollFd {
                    fd: -1,
                    events: 0,
                    revents: 0,
                },
            ],
            wait_items: [
                ZxWaitItem {
                    handle: 0,
                    waitfor: 0,
                    pending: 0,
                },
                ZxWaitItem {
                    handle: 0,
                    waitfor: 0,
                    pending: 0,
                },
            ],
            packet: PortPacket {
                key: seed,
                packet_type: PORT_PACKET_TYPE_USER,
                status: 0,
                data0: seed,
                data1: seed.rotate_left(7),
                data2: seed.rotate_left(13),
                data3: seed.rotate_left(29),
            },
            timespec: LinuxTimespec {
                tv_sec: 0,
                tv_nsec: 1,
            },
            timeval: LinuxTimeval {
                tv_sec: 0,
                tv_usec: 1,
            },
            itimer: LinuxItimerval {
                it_interval: LinuxTimeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
                it_value: LinuxTimeval {
                    tv_sec: 0,
                    tv_usec: 1,
                },
            },
            cap_header: LinuxCapUserHeader {
                version: LINUX_CAPABILITY_VERSION_3,
                pid: 0,
            },
            cap_data: [
                LinuxCapUserData {
                    effective: 0,
                    permitted: 0,
                    inheritable: 0,
                },
                LinuxCapUserData {
                    effective: 0,
                    permitted: 0,
                    inheritable: 0,
                },
            ],
            handles: [0; 8],
            words: [0; 16],
            futex: 0,
            alt_futex: 0,
        };
        arena.write_cstrs();
        arena.refresh(seed);
        arena
    }

    fn write_cstrs(&mut self) {
        write_cstr(&mut self.path, b"/tmp/smros-fuzz");
        write_cstr(&mut self.path_alt, b"/tmp/smros-fuzz-alt");
        write_cstr(&mut self.name, b"smros-test");
    }

    fn refresh(&mut self, seed: u64) {
        for (index, byte) in self.scratch.iter_mut().enumerate() {
            *byte = seed.wrapping_add((index as u64).wrapping_mul(37)) as u8;
        }
        for (index, word) in self.words.iter_mut().enumerate() {
            *word = seed.rotate_left((index as u32) & 31) as usize;
        }
        self.iov[0] = LinuxIovec {
            base: self.scratch_ptr(),
            len: FUZZ_IO_BYTES / 2,
        };
        self.iov[1] = LinuxIovec {
            base: self.scratch_ptr_offset(FUZZ_IO_BYTES),
            len: FUZZ_IO_BYTES / 2,
        };
        self.poll[0] = LinuxPollFd {
            fd: -1,
            events: 0x0001 | 0x0004,
            revents: 0,
        };
        self.poll[1] = LinuxPollFd {
            fd: -1,
            events: 0x0001,
            revents: 0,
        };
        self.wait_items[0] = ZxWaitItem {
            handle: 0,
            waitfor: 0,
            pending: 0,
        };
        self.wait_items[1] = ZxWaitItem {
            handle: 0,
            waitfor: 0,
            pending: 0,
        };
        self.packet.key = seed;
        self.packet.packet_type = PORT_PACKET_TYPE_USER;
        self.packet.status = 0;
        self.packet.data0 = seed;
        self.packet.data1 = seed.rotate_left(7);
        self.packet.data2 = seed.rotate_left(13);
        self.packet.data3 = seed.rotate_left(29);
        self.timespec = LinuxTimespec {
            tv_sec: 0,
            tv_nsec: 1,
        };
        self.timeval = LinuxTimeval {
            tv_sec: 0,
            tv_usec: 1,
        };
        self.itimer = LinuxItimerval {
            it_interval: LinuxTimeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            it_value: LinuxTimeval {
                tv_sec: 0,
                tv_usec: 1,
            },
        };
        self.cap_header.version = LINUX_CAPABILITY_VERSION_3;
        self.cap_header.pid = 0;
        self.futex = 0;
        self.alt_futex = 0;
    }

    fn scratch_ptr(&mut self) -> usize {
        self.scratch.as_mut_ptr() as usize
    }

    fn scratch_ptr_offset(&mut self, offset: usize) -> usize {
        self.scratch.as_mut_ptr().wrapping_add(offset) as usize
    }

    fn path_ptr(&self) -> usize {
        self.path.as_ptr() as usize
    }

    fn path_alt_ptr(&self) -> usize {
        self.path_alt.as_ptr() as usize
    }

    fn name_ptr(&self) -> usize {
        self.name.as_ptr() as usize
    }

    fn iov_ptr(&self) -> usize {
        self.iov.as_ptr() as usize
    }

    fn poll_ptr(&self) -> usize {
        self.poll.as_ptr() as usize
    }

    fn wait_items_ptr(&self) -> usize {
        self.wait_items.as_ptr() as usize
    }

    fn packet_ptr(&mut self) -> usize {
        &mut self.packet as *mut PortPacket as usize
    }

    fn timespec_ptr(&self) -> usize {
        &self.timespec as *const LinuxTimespec as usize
    }

    fn timeval_ptr(&self) -> usize {
        &self.timeval as *const LinuxTimeval as usize
    }

    fn itimer_ptr(&self) -> usize {
        &self.itimer as *const LinuxItimerval as usize
    }

    fn cap_header_ptr(&mut self) -> usize {
        &mut self.cap_header as *mut LinuxCapUserHeader as usize
    }

    fn cap_data_ptr(&mut self) -> usize {
        self.cap_data.as_mut_ptr() as usize
    }

    fn handles_ptr(&mut self) -> usize {
        self.handles.as_mut_ptr() as usize
    }

    fn u32_out_ptr(&mut self, value: u32) -> usize {
        self.handles[0] = value;
        self.handles.as_mut_ptr() as usize
    }

    fn words_ptr(&mut self) -> usize {
        self.words.as_mut_ptr() as usize
    }

    fn futex_ptr(&mut self) -> usize {
        &mut self.futex as *mut i32 as usize
    }

    fn alt_futex_ptr(&mut self) -> usize {
        &mut self.alt_futex as *mut i32 as usize
    }
}

struct FuzzState {
    arena: FuzzArena,
    rng: FuzzRng,
    handles: [u32; 64],
    handle_count: usize,
    seed_handle_count: usize,
    fds: [usize; 64],
    fd_count: usize,
    seed_fd_count: usize,
    scratch_fds: [usize; FUZZ_SCRATCH_FDS],
    scratch_fd_count: usize,
    mappings: [usize; 32],
    mapping_count: usize,
    zircon_mappings: [usize; 32],
    zircon_mapping_count: usize,
    start_tick: u64,
    deadline_tick: u64,
    report: SyscallFuzzReport,
}

impl FuzzState {
    fn new(seed: u64, iterations: usize, time_limit_ticks: u64, start_tick: u64) -> Self {
        let deadline_tick = if time_limit_ticks == 0 {
            0
        } else {
            start_tick.saturating_add(time_limit_ticks)
        };

        Self {
            arena: FuzzArena::new(seed),
            rng: FuzzRng::new(seed),
            handles: [0; 64],
            handle_count: 0,
            seed_handle_count: 0,
            fds: [0; 64],
            fd_count: 0,
            seed_fd_count: 0,
            scratch_fds: [0; FUZZ_SCRATCH_FDS],
            scratch_fd_count: 0,
            mappings: [0; 32],
            mapping_count: 0,
            zircon_mappings: [0; 32],
            zircon_mapping_count: 0,
            start_tick,
            deadline_tick,
            report: SyscallFuzzReport {
                seed,
                iterations,
                time_limit_ticks,
                linux_interface_syscalls: count_linux_interface_syscalls(),
                linux_success_syscalls: count_linux_success_syscalls(),
                linux_success_call_cases: linux_success_call_cases(),
                zircon_interface_syscalls: count_zircon_interface_syscalls(),
                zircon_success_syscalls: ZIRCON_SUCCESS_SYSCALLS.len(),
                zircon_success_call_cases: zircon_success_call_cases(),
                ..SyscallFuzzReport::default()
            },
        }
    }

    fn freeze_seed_objects(&mut self) {
        self.seed_handle_count = self.handle_count;
        self.seed_fd_count = self.fd_count;
    }

    fn track_handle(&mut self, handle: u32) {
        if handle == 0
            || handle == memory_root_vmar_handle()
            || self.handles[..self.handle_count].contains(&handle)
        {
            return;
        }
        if self.handle_count < self.handles.len() {
            self.handles[self.handle_count] = handle;
            self.handle_count += 1;
            self.report.created_handles += 1;
        } else {
            let _ = sys_handle_close(handle);
        }
    }

    fn seed_handle(&mut self, handle: u32) {
        if handle == 0 || self.handles[..self.handle_count].contains(&handle) {
            return;
        }
        if self.handle_count < self.handles.len() {
            self.handles[self.handle_count] = handle;
            self.handle_count += 1;
            self.report.created_handles += 1;
        } else {
            let _ = sys_handle_close(handle);
        }
    }

    fn track_handle_pair(&mut self, pair: usize) {
        self.track_handle((pair >> 32) as u32);
        self.track_handle(pair as u32);
    }

    fn track_fd(&mut self, fd: usize) {
        if fd <= 2 || self.fds[..self.fd_count].contains(&fd) {
            return;
        }
        if self.fd_count < self.fds.len() {
            self.fds[self.fd_count] = fd;
            self.fd_count += 1;
            self.report.created_fds += 1;
        } else {
            let _ = sys_close(fd);
        }
    }

    fn track_scratch_fd(&mut self, fd: usize) -> usize {
        if fd <= 2 {
            return fd;
        }
        if self.scratch_fds[..self.scratch_fd_count].contains(&fd) {
            return fd;
        }
        if self.scratch_fd_count < self.scratch_fds.len() {
            self.scratch_fds[self.scratch_fd_count] = fd;
            self.scratch_fd_count += 1;
            self.report.created_fds += 1;
            fd
        } else {
            let _ = sys_close(fd);
            self.file_fd()
        }
    }

    fn seed_fd(&mut self, fd: usize) {
        if fd <= 2 {
            return;
        }
        if self.fd_count < self.fds.len() {
            self.fds[self.fd_count] = fd;
            self.fd_count += 1;
            self.report.created_fds += 1;
        } else {
            let _ = sys_close(fd);
        }
    }

    fn track_mapping(&mut self, addr: usize) {
        if addr == 0 || self.mappings[..self.mapping_count].contains(&addr) {
            return;
        }
        if self.mapping_count < self.mappings.len() {
            self.mappings[self.mapping_count] = addr;
            self.mapping_count += 1;
        } else {
            let _ = sys_munmap(addr, PAGE_SIZE);
        }
    }

    fn track_zircon_mapping(&mut self, addr: usize) {
        if addr == 0 || self.zircon_mappings[..self.zircon_mapping_count].contains(&addr) {
            return;
        }
        if self.zircon_mapping_count < self.zircon_mappings.len() {
            self.zircon_mappings[self.zircon_mapping_count] = addr;
            self.zircon_mapping_count += 1;
        } else {
            let _ = dispatch_zircon_syscall(
                80,
                [
                    memory_root_vmar_handle() as usize,
                    addr,
                    PAGE_SIZE,
                    0,
                    0,
                    0,
                    0,
                    0,
                ],
            );
        }
    }

    fn fd(&mut self) -> usize {
        self.tracked_fd_or_stdio()
    }

    fn tracked_fd(&mut self) -> usize {
        if self.fd_count == 0 {
            1
        } else {
            self.fds[self.rng.below(self.fd_count)]
        }
    }

    fn tracked_fd_or_stdio(&mut self) -> usize {
        if self.fd_count == 0 {
            1
        } else {
            match self.rng.below(4) {
                0 => 0,
                1 => 1,
                _ => self.tracked_fd(),
            }
        }
    }

    fn fd_at(&self, index: usize, fallback: usize) -> usize {
        if self.fd_count > index {
            self.fds[index]
        } else {
            fallback
        }
    }

    fn file_fd(&self) -> usize {
        self.fd_at(0, 1)
    }

    fn dir_fd(&self) -> usize {
        self.fd_at(1, self.file_fd())
    }

    fn event_fd(&self) -> usize {
        self.fd_at(2, self.file_fd())
    }

    fn timer_fd(&self) -> usize {
        self.fd_at(3, self.file_fd())
    }

    fn linux_timer_handle(&mut self) -> usize {
        self.find_latest_handle_from(0, |handle| {
            dispatch_linux_syscall(109, [handle as usize, 0, 0, 0, 0, 0]).is_ok()
        }) as usize
    }

    fn linux_close_fd(&mut self) -> usize {
        match dispatch_linux_syscall(23, [self.file_fd(), 0, 0, 0, 0, 0]) {
            Ok(fd) => fd,
            Err(_) => 0,
        }
    }

    fn transient_file_fd(&mut self) -> usize {
        match dispatch_linux_syscall(279, [self.arena.name_ptr(), 0, 0, 0, 0, 0]) {
            Ok(fd) => self.track_scratch_fd(fd),
            Err(_) => self.file_fd(),
        }
    }

    fn transient_mapping(&mut self) -> usize {
        let flags = MmapFlags::PRIVATE.bits() | MmapFlags::ANONYMOUS.bits();
        sys_mmap(
            0,
            PAGE_SIZE,
            MmapProt::READ.bits() | MmapProt::WRITE.bits(),
            flags,
            0,
            0,
        )
        .unwrap_or(0x5000_0000)
    }

    fn linux_timer_delete_handle(&mut self) -> usize {
        let out = self.arena.words_ptr();
        if dispatch_linux_syscall(107, [1, 0, out, 0, 0, 0]).is_err() {
            return 0;
        }
        unsafe { core::ptr::read(out as *const usize) }
    }

    fn zircon_handle_for_replace(&mut self) -> u32 {
        let handle = self.latest_vmo_handle();
        match dispatch_zircon_syscall(
            8,
            [
                handle as usize,
                RIGHT_SAME_RIGHTS as usize,
                0,
                0,
                0,
                0,
                0,
                0,
            ],
        ) {
            Ok(duplicate) => duplicate as u32,
            Err(_) => handle,
        }
    }

    fn transient_event_handle(&mut self) -> u32 {
        let mut handle = 0u32;
        if sys_event_create(0, &mut handle).is_ok() {
            handle
        } else {
            0
        }
    }

    fn transient_process_pair(&mut self) -> (u32, u32) {
        let mut proc_handle = 0u32;
        let mut vmar_handle = 0u32;
        if sys_process_create(
            self.job_handle(),
            self.arena.name_ptr(),
            10,
            0,
            &mut proc_handle,
            &mut vmar_handle,
        )
        .is_ok()
        {
            (proc_handle, vmar_handle)
        } else {
            (self.process_handle(), self.process_vmar_handle())
        }
    }

    fn transient_process_handle(&mut self) -> u32 {
        let (proc_handle, vmar_handle) = self.transient_process_pair();
        if vmar_handle != 0 && proc_handle != self.process_handle() {
            let _ = sys_handle_close(vmar_handle);
        }
        proc_handle
    }

    fn transient_thread_handle(&mut self) -> u32 {
        let mut thread_handle = 0u32;
        if sys_thread_create(
            self.process_handle(),
            self.arena.name_ptr(),
            10,
            0,
            0,
            &mut thread_handle,
        )
        .is_ok()
        {
            thread_handle
        } else {
            self.thread_handle()
        }
    }

    fn transient_started_thread_handle(&mut self) -> u32 {
        let thread_handle = self.transient_thread_handle();
        let _ = dispatch_zircon_syscall(35, [thread_handle as usize, 1, PAGE_SIZE, 0, 0, 0, 0, 0]);
        thread_handle
    }

    fn transient_socket_handle(&mut self) -> u32 {
        let mut h0 = 0u32;
        let mut h1 = 0u32;
        if sys_socket_create(0, &mut h0, &mut h1).is_ok() {
            let _ = sys_handle_close(h1);
            h0
        } else {
            self.socket_handle()
        }
    }

    fn transient_interrupt_handle(&mut self) -> u32 {
        let mut handle = 0u32;
        if sys_interrupt_create(0, 0, 0, &mut handle).is_ok() {
            handle
        } else {
            self.interrupt_handle()
        }
    }

    fn transient_pmt_handle(&mut self) -> u32 {
        let mut handle = 0u32;
        if sys_bti_pin(
            self.bti_handle(),
            0,
            self.vmo_handle(),
            0,
            PAGE_SIZE,
            self.arena.words_ptr(),
            1,
            &mut handle,
        )
        .is_ok()
        {
            handle
        } else {
            0
        }
    }

    fn transient_child_vmar_handle(&mut self) -> u32 {
        let offset = FUZZ_ZIRCON_VMAR_ALLOC_OFFSET + 0x0040_0000 + self.rng.below(64) * PAGE_SIZE;
        dispatch_zircon_syscall(
            77,
            [
                memory_root_vmar_handle() as usize,
                VmarFlags::SPECIFIC.bits() as usize,
                offset,
                PAGE_SIZE,
                0,
                0,
                0,
                0,
            ],
        )
        .map(|handle| handle as u32)
        .unwrap_or(memory_root_vmar_handle())
    }

    fn transient_vmar_mapping(&mut self) -> (u32, usize) {
        let offset = FUZZ_ZIRCON_VMAR_MAP_OFFSET + 0x0020_0000 + self.rng.below(64) * PAGE_SIZE;
        let addr = dispatch_zircon_syscall(
            79,
            [
                memory_root_vmar_handle() as usize,
                (VmOptions::PERM_RW | VmOptions::SPECIFIC_OVERWRITE).bits() as usize,
                offset,
                self.latest_vmo_handle() as usize,
                0,
                PAGE_SIZE,
                0,
                0,
            ],
        )
        .unwrap_or(0);
        (memory_root_vmar_handle(), addr)
    }

    fn socket_fd(&self) -> usize {
        self.fd_at(4, self.file_fd())
    }

    fn pipe_read_fd(&self) -> usize {
        self.fd_at(5, self.file_fd())
    }

    fn pipe_write_fd(&self) -> usize {
        self.fd_at(6, self.file_fd())
    }

    fn socket_peer_fd(&self) -> usize {
        self.fd_at(8, self.socket_fd())
    }

    fn socketpair_fd(&self) -> usize {
        self.fd_at(7, self.socket_fd())
    }

    fn new_fd(&mut self) -> usize {
        FUZZ_LINUX_NEW_FD_BASE + self.rng.below(FUZZ_LINUX_NEW_FD_SPAN)
    }

    fn handle(&mut self) -> u32 {
        if self.handle_count == 0 {
            0
        } else {
            self.handles[self.rng.below(self.handle_count)]
        }
    }

    fn handle_at(&self, index: usize) -> u32 {
        if self.handle_count > index {
            self.handles[index]
        } else {
            0
        }
    }

    fn msg_handle(&self) -> u32 {
        self.handle_at(0)
    }

    fn sem_handle(&self) -> u32 {
        self.handle_at(1)
    }

    fn shm_handle(&self) -> u32 {
        self.handle_at(2)
    }

    fn vmo_handle(&self) -> u32 {
        self.handle_at(3)
    }

    fn event_handle(&self) -> u32 {
        self.handle_at(4)
    }

    fn port_handle(&self) -> u32 {
        self.handle_at(5)
    }

    fn job_handle(&self) -> u32 {
        self.handle_at(6)
    }

    fn latest_vmo_handle(&self) -> u32 {
        self.find_latest_handle_from(3, |handle| {
            dispatch_zircon_syscall(71, [handle as usize, 0, 0, 0, 0, 0, 0, 0]).is_ok()
        })
    }

    fn channel_handle(&self) -> u32 {
        self.handle_at(7)
    }

    fn channel_peer_handle(&self) -> u32 {
        self.handle_at(8)
    }

    fn socket_handle(&self) -> u32 {
        self.handle_at(9)
    }

    fn socket_peer_handle(&self) -> u32 {
        self.handle_at(10)
    }

    fn timer_handle(&self) -> u32 {
        self.handle_at(11)
    }

    fn debuglog_handle(&self) -> u32 {
        self.handle_at(12)
    }

    fn fifo_handle(&self) -> u32 {
        self.handle_at(13)
    }

    fn fifo_peer_handle(&self) -> u32 {
        self.handle_at(14)
    }

    fn clock_handle(&self) -> u32 {
        self.handle_at(15)
    }

    fn eventpair_handle(&self) -> u32 {
        self.handle_at(16)
    }

    fn ipc_handle(&self) -> u32 {
        self.handle_at(17)
    }

    fn process_handle(&self) -> u32 {
        self.handle_at(FUZZ_PROCESS_HANDLE_INDEX)
    }

    fn process_vmar_handle(&self) -> u32 {
        self.handle_at(FUZZ_PROCESS_VMAR_HANDLE_INDEX)
    }

    fn thread_handle(&self) -> u32 {
        self.handle_at(FUZZ_THREAD_HANDLE_INDEX)
    }

    fn interrupt_handle(&self) -> u32 {
        self.handle_at(FUZZ_INTERRUPT_HANDLE_INDEX)
    }

    fn iommu_handle(&self) -> u32 {
        self.handle_at(FUZZ_IOMMU_HANDLE_INDEX)
    }

    fn bti_handle(&self) -> u32 {
        self.handle_at(FUZZ_BTI_HANDLE_INDEX)
    }

    fn resource_handle(&self) -> u32 {
        self.handle_at(FUZZ_RESOURCE_HANDLE_INDEX)
    }

    fn pci_handle(&self) -> u32 {
        self.handle_at(FUZZ_PCI_HANDLE_INDEX)
    }

    fn guest_handle(&self) -> u32 {
        self.handle_at(FUZZ_GUEST_HANDLE_INDEX)
    }

    fn guest_vmar_handle(&self) -> u32 {
        self.handle_at(FUZZ_GUEST_VMAR_HANDLE_INDEX)
    }

    fn vcpu_handle(&self) -> u32 {
        self.handle_at(FUZZ_VCPU_HANDLE_INDEX)
    }

    fn pager_handle(&self) -> u32 {
        self.handle_at(FUZZ_PAGER_HANDLE_INDEX)
    }

    fn pager_vmo_handle(&self) -> u32 {
        self.handle_at(FUZZ_PAGER_VMO_HANDLE_INDEX)
    }

    fn stream_handle(&self) -> u32 {
        self.handle_at(FUZZ_STREAM_HANDLE_INDEX)
    }

    fn profile_handle(&self) -> u32 {
        self.handle_at(FUZZ_PROFILE_HANDLE_INDEX)
    }

    fn exception_handle(&self) -> u32 {
        self.handle_at(FUZZ_EXCEPTION_HANDLE_INDEX)
    }

    fn find_latest_handle_from(
        &self,
        fallback_index: usize,
        mut valid: impl FnMut(u32) -> bool,
    ) -> u32 {
        let fallback = self.handle_at(fallback_index);
        let mut index = self.handle_count;
        while index > 0 {
            index -= 1;
            let handle = self.handles[index];
            if valid(handle) {
                return handle;
            }
        }
        fallback
    }

    fn mapping(&mut self) -> usize {
        if self.mapping_count == 0 {
            0x5000_0000
        } else {
            self.mappings[self.rng.below(self.mapping_count)]
        }
    }

    fn zircon_mapping(&mut self) -> usize {
        if self.zircon_mapping_count == 0 {
            0x7000_0000
        } else {
            self.zircon_mappings[self.rng.below(self.zircon_mapping_count)]
        }
    }

    fn byte_len(&mut self) -> usize {
        self.rng.choose(&[0, 1, 4, 8, 16, 32, 64, 128])
    }

    fn small_count(&mut self) -> usize {
        self.rng.choose(&[0, 1, 2])
    }

    fn page_len(&mut self) -> usize {
        self.rng.choose(&[0, PAGE_SIZE, PAGE_SIZE * 2])
    }

    fn user_ptr(&mut self, len: usize) -> usize {
        if len == 0 {
            0
        } else {
            self.arena.scratch_ptr_offset(self.rng.below(128))
        }
    }

    fn cstr_ptr(&mut self) -> usize {
        match self.rng.below(4) {
            1 => self.arena.path_alt_ptr(),
            2 => self.arena.name_ptr(),
            _ => self.arena.path_ptr(),
        }
    }

    fn should_stop(&mut self) -> bool {
        if self.deadline_tick != 0 && timer::get_tick_count() >= self.deadline_tick {
            self.report.timed_out = true;
            return true;
        }
        false
    }

    fn record_linux_err_syscall(&mut self, num: u32) {
        let mut index = 0;
        while index < self.report.linux_err_syscall_count {
            if self.report.linux_err_syscalls[index] == num {
                self.report.linux_err_syscall_counts[index] += 1;
                return;
            }
            index += 1;
        }
        if self.report.linux_err_syscall_count < FUZZ_ERROR_BUCKETS {
            let bucket = self.report.linux_err_syscall_count;
            self.report.linux_err_syscalls[bucket] = num;
            self.report.linux_err_syscall_counts[bucket] = 1;
            self.report.linux_err_syscall_count += 1;
        }
    }

    fn record_zircon_err_syscall(&mut self, num: u32) {
        let mut index = 0;
        while index < self.report.zircon_err_syscall_count {
            if self.report.zircon_err_syscalls[index] == num {
                self.report.zircon_err_syscall_counts[index] += 1;
                return;
            }
            index += 1;
        }
        if self.report.zircon_err_syscall_count < FUZZ_ERROR_BUCKETS {
            let bucket = self.report.zircon_err_syscall_count;
            self.report.zircon_err_syscalls[bucket] = num;
            self.report.zircon_err_syscall_counts[bucket] = 1;
            self.report.zircon_err_syscall_count += 1;
        }
    }

    fn call_linux(&mut self, num: u32, args: [usize; 6]) {
        self.report.linux_calls += 1;
        match dispatch_linux_syscall(num, args) {
            Ok(value) => {
                self.report.linux_ok += 1;
                self.capture_linux_result(num, value, &args);
            }
            Err(SysError::ENOSYS) => {
                if self.report.linux_enosys == 0 {
                    self.report.linux_first_enosys_syscall = num;
                }
                self.report.linux_last_enosys_syscall = num;
                self.report.linux_enosys += 1;
            }
            Err(_) => {
                self.record_linux_err_syscall(num);
                if self.report.linux_err == 0 {
                    self.report.linux_first_err_syscall = num;
                }
                self.report.linux_last_err_syscall = num;
                self.report.linux_err += 1;
            }
        }
    }

    fn call_zircon(&mut self, num: u32, args: [usize; 8]) {
        self.report.zircon_calls += 1;
        match dispatch_zircon_syscall(num, args) {
            Ok(value) => {
                self.report.zircon_ok += 1;
                self.capture_zircon_result(num, value, &args);
                self.finish_zircon_call(num, &args);
            }
            Err(ZxError::ErrNotSupported) => {
                if self.report.zircon_unsupported == 0 {
                    self.report.zircon_first_unsupported_syscall = num;
                }
                self.report.zircon_last_unsupported_syscall = num;
                self.report.zircon_unsupported += 1;
            }
            Err(_) => {
                self.drain_after_zircon_error(num, &args);
                self.record_zircon_err_syscall(num);
                if self.report.zircon_err == 0 {
                    self.report.zircon_first_err_syscall = num;
                }
                self.report.zircon_last_err_syscall = num;
                self.report.zircon_err += 1;
            }
        }
    }

    fn drain_after_zircon_error(&mut self, num: u32, args: &[usize; 8]) {
        if num == 62 {
            let _ =
                dispatch_zircon_syscall(63, [args[0], 0, self.arena.packet_ptr(), 0, 0, 0, 0, 0]);
        }
    }

    fn capture_linux_result(&mut self, num: u32, value: usize, args: &[usize; 6]) {
        match num {
            19 | 20 | 26 | 56 | 74 | 85 | 198 | 279 | 437 => self.track_fd(value),
            23 | 24 | 25 | 202 | 242 => self.track_fd(value),
            59 => {
                if args[0] != 0 {
                    let ptr = args[0] as *const i32;
                    unsafe {
                        self.track_fd(core::ptr::read(ptr) as usize);
                        self.track_fd(core::ptr::read(ptr.add(1)) as usize);
                    }
                }
            }
            199 => {
                if args[3] != 0 {
                    let ptr = args[3] as *const i32;
                    unsafe {
                        self.track_fd(core::ptr::read(ptr) as usize);
                        self.track_fd(core::ptr::read(ptr.add(1)) as usize);
                    }
                }
            }
            186 | 190 | 194 => self.track_handle(value as u32),
            107 if args[2] != 0 => unsafe {
                self.track_handle(core::ptr::read(args[2] as *const usize) as u32);
            },
            107 | 108 | 109 | 110 => {
                if args[0] != 0 {
                    self.track_handle(args[0] as u32);
                }
            }
            196 | 222 => self.track_mapping(value),
            _ => {}
        }
    }

    fn capture_zircon_result(&mut self, num: u32, value: usize, _args: &[usize; 8]) {
        match num {
            5 | 8 | 9 | 18 | 31 | 34 | 39 | 43 | 46 | 47 | 49 | 51 | 52 | 53 | 61 | 65 | 68
            | 72 | 74 | 76 | 77 | 87 | 88 | 98 | 106 | 107 | 108 | 109 | 110 | 115 | 121 | 122
            | 129 | 132 | 140 | 141 | 187 | 197 | 202 | 203 | 209 | 210 => {
                self.track_handle(value as u32)
            }
            20 | 54 | 84 | 130 => self.track_handle_pair(value),
            27 => {
                let _ = sys_handle_close((value >> 32) as u32);
                let _ = sys_handle_close(value as u32);
            }
            79 => self.track_zircon_mapping(value),
            _ => {}
        }
    }

    fn finish_zircon_call(&mut self, num: u32, args: &[usize; 8]) {
        match num {
            32 => {
                let handle = args[0] as u32;
                if handle != self.socket_handle() && handle != self.socket_peer_handle() {
                    let _ = sys_handle_close(handle);
                }
            }
            38 => {
                let handle = args[0] as u32;
                if handle != self.process_handle() {
                    let _ = sys_handle_close(handle);
                }
            }
            40 => {
                let process_handle = args[0] as u32;
                let thread_handle = args[1] as u32;
                if thread_handle != self.thread_handle() {
                    let _ = sys_handle_close(thread_handle);
                }
                if process_handle != self.process_handle() {
                    let _ = sys_handle_close(process_handle);
                }
            }
            50 => {
                let handle = args[0] as u32;
                if handle != self.thread_handle() && handle != self.process_handle() {
                    let _ = sys_handle_close(handle);
                }
            }
            55 => {
                let _ = dispatch_zircon_syscall(56, [args[0], u32::MAX as usize, 0, 0, 0, 0, 0, 0]);
            }
            23 | 24 => {
                self.drain_channel(self.channel_peer_handle());
            }
            25 => {
                self.drain_channel(self.channel_peer_handle());
            }
            30 => {
                let _ = dispatch_zircon_syscall(
                    31,
                    [self.socket_peer_handle() as usize, 0, 0, 0, 0, 0, 0, 0],
                );
            }
            188 | 189 => {
                self.drain_stream();
            }
            12 => {
                let _ = dispatch_zircon_syscall(
                    63,
                    [args[1], 0, self.arena.packet_ptr(), 0, 0, 0, 0, 0],
                );
            }
            62 => {
                let _ = dispatch_zircon_syscall(
                    63,
                    [args[0], 0, self.arena.packet_ptr(), 0, 0, 0, 0, 0],
                );
            }
            _ => {}
        }
    }

    fn drain_channel(&mut self, handle: u32) {
        loop {
            if dispatch_zircon_syscall(
                21,
                [
                    handle as usize,
                    0,
                    self.arena.scratch_ptr(),
                    FUZZ_SCRATCH_BYTES,
                    self.arena.handles_ptr(),
                    0,
                    0,
                    0,
                ],
            )
            .is_err()
            {
                break;
            }
        }
    }

    fn drain_stream(&mut self) {
        loop {
            if dispatch_zircon_syscall(
                190,
                [
                    self.stream_handle() as usize,
                    0,
                    self.arena.iov_ptr(),
                    2,
                    self.arena.words_ptr(),
                    0,
                    0,
                    0,
                ],
            )
            .is_err()
            {
                break;
            }
        }
    }

    fn cleanup_linux_mappings(&mut self) {
        for index in 0..self.mapping_count {
            let _ = sys_munmap(self.mappings[index], PAGE_SIZE * 2);
        }
        self.mapping_count = 0;
    }

    fn cleanup_zircon_mappings(&mut self) {
        for index in 0..self.zircon_mapping_count {
            let _ = dispatch_zircon_syscall(
                80,
                [
                    memory_root_vmar_handle() as usize,
                    self.zircon_mappings[index],
                    PAGE_SIZE,
                    0,
                    0,
                    0,
                    0,
                    0,
                ],
            );
        }
        self.zircon_mapping_count = 0;
    }

    fn cleanup_transient_fds(&mut self) {
        for index in 0..self.scratch_fd_count {
            let _ = sys_close(self.scratch_fds[index]);
        }
        self.scratch_fd_count = 0;
        for index in self.seed_fd_count..self.fd_count {
            let _ = sys_close(self.fds[index]);
        }
        self.fd_count = self.seed_fd_count;
    }

    fn cleanup_transient_handles(&mut self) {
        for index in self.seed_handle_count..self.handle_count {
            let _ = sys_handle_close(self.handles[index]);
        }
        self.handle_count = self.seed_handle_count;
    }

    fn cleanup(&mut self) {
        self.cleanup_linux_mappings();
        self.cleanup_zircon_mappings();

        for index in 0..self.scratch_fd_count {
            let _ = sys_close(self.scratch_fds[index]);
        }
        self.scratch_fd_count = 0;

        for index in 0..self.fd_count {
            let _ = sys_close(self.fds[index]);
        }
        self.fd_count = 0;

        for index in 0..self.handle_count {
            let handle = self.handles[index];
            if handle != memory_root_vmar_handle() {
                let _ = sys_handle_close(handle);
            }
        }
        self.handle_count = 0;
    }
}

pub fn fuzz_syscalls(seed: u64, requested_iterations: usize) -> SyscallFuzzReport {
    fuzz_syscalls_with_config(SyscallFuzzConfig::new(seed, requested_iterations))
}

pub fn fuzz_syscalls_with_config(config: SyscallFuzzConfig) -> SyscallFuzzReport {
    let iterations = if config.iterations == 0 {
        FUZZ_DEFAULT_ITERATIONS
    } else {
        config.iterations
    };
    let start_tick = timer::get_tick_count();
    let mut state = FuzzState::new(config.seed, iterations, config.time_limit_ticks, start_tick);

    seed_fuzz_state(&mut state);
    state.freeze_seed_objects();

    for round in 0..iterations {
        if state.should_stop() {
            break;
        }

        state.arena.refresh(config.seed.wrapping_add(round as u64));
        if !fuzz_linux_round(&mut state) {
            break;
        }
        state.cleanup_linux_mappings();
        state.cleanup_transient_fds();
        state.cleanup_transient_handles();
        if !fuzz_zircon_round(&mut state) {
            break;
        }
        state.cleanup_zircon_mappings();
        state.cleanup_transient_handles();
        state.report.completed_iterations += 1;
    }

    state.cleanup();
    state.report.elapsed_ticks = timer::get_tick_count().saturating_sub(state.start_tick);
    state.report
}

fn count_linux_interface_syscalls() -> usize {
    let mut count = 0;
    let mut syscall_num = 0;
    while syscall_num <= FUZZ_LINUX_INTERFACE_MAX {
        if syscall_logic::linux_syscall_interface_known(syscall_num) {
            count += 1;
        }
        syscall_num += 1;
    }
    count
}

fn count_zircon_interface_syscalls() -> usize {
    let mut count = 0;
    let mut syscall_num = 0;
    while syscall_num <= FUZZ_ZIRCON_INTERFACE_MAX {
        if syscall_logic::zircon_syscall_interface_known(syscall_num) {
            count += 1;
        }
        syscall_num += 1;
    }
    count
}

fn linux_success_call_cases() -> usize {
    let mut count = 0;
    let mut syscall_num = 0;
    while syscall_num <= FUZZ_LINUX_INTERFACE_MAX {
        if syscall_logic::linux_syscall_interface_known(syscall_num) {
            count += linux_variants(syscall_num);
        }
        syscall_num += 1;
    }
    count
}

fn count_linux_success_syscalls() -> usize {
    count_linux_interface_syscalls()
}

fn zircon_success_call_cases() -> usize {
    let mut count = 0;
    let mut index = 0;
    while index < ZIRCON_SUCCESS_SYSCALLS.len() {
        count += zircon_variants(ZIRCON_SUCCESS_SYSCALLS[index]);
        index += 1;
    }
    count
}

fn seed_fuzz_state(state: &mut FuzzState) {
    let flags = MmapFlags::PRIVATE.bits() | MmapFlags::ANONYMOUS.bits();
    if let Ok(addr) = sys_mmap(
        0,
        PAGE_SIZE * 2,
        MmapProt::READ.bits() | MmapProt::WRITE.bits(),
        flags,
        0,
        0,
    ) {
        state.track_mapping(addr);
    }

    let _ = fxfs::write_file("/tmp/smros-fuzz", b"smros syscall fuzz seed\n");
    let _ = fxfs::write_file("/tmp/smros-fuzz-alt", b"smros syscall fuzz alt\n");

    if let Ok(fd) = sys_openat(
        usize::MAX - 99,
        state.arena.path_ptr(),
        LINUX_O_CREAT | LINUX_O_RDWR,
        0,
    ) {
        state.seed_fd(fd);
    }
    if let Ok(fd) = sys_openat(
        usize::MAX - 99,
        state.arena.path_ptr(),
        LINUX_O_DIRECTORY,
        0,
    ) {
        state.seed_fd(fd);
    }
    if let Ok(fd) = sys_eventfd2(1, 0) {
        state.seed_fd(fd);
    }
    if let Ok(fd) = sys_timerfd_create(1, 0) {
        state.seed_fd(fd);
    }
    if let Ok(fd) = sys_socket(LINUX_AF_UNIX, LINUX_SOCK_STREAM, 0) {
        state.seed_fd(fd);
    }
    let mut pair = [0i32; 2];
    if sys_pipe2(pair.as_mut_ptr() as usize, 0).is_ok() {
        state.seed_fd(pair[0] as usize);
        state.seed_fd(pair[1] as usize);
    }
    if sys_socketpair(
        LINUX_AF_UNIX,
        LINUX_SOCK_DGRAM,
        0,
        pair.as_mut_ptr() as usize,
    )
    .is_ok()
    {
        state.seed_fd(pair[0] as usize);
        state.seed_fd(pair[1] as usize);
    }

    if let Ok(handle) = sys_msgget(1, 0) {
        state.seed_handle(handle as u32);
    }
    if let Ok(handle) = sys_semget(1, 1, 0) {
        state.seed_handle(handle as u32);
    }
    if let Ok(handle) = sys_shmget(1, PAGE_SIZE, 0) {
        state.seed_handle(handle as u32);
    }

    let mut handle = 0u32;
    if sys_vmo_create((PAGE_SIZE * 2) as u64, 1, &mut handle).is_ok() {
        state.seed_handle(handle);
    }
    if sys_event_create(0, &mut handle).is_ok() {
        state.seed_handle(handle);
    }
    if sys_port_create(0, &mut handle).is_ok() {
        state.seed_handle(handle);
    }
    if sys_job_create(0, 0, &mut handle).is_ok() {
        state.seed_handle(handle);
    }

    let mut h0 = 0u32;
    let mut h1 = 0u32;
    if sys_channel_create(0, &mut h0, &mut h1).is_ok() {
        state.seed_handle(h0);
        state.seed_handle(h1);
    }
    if sys_socket_create(0, &mut h0, &mut h1).is_ok() {
        state.seed_handle(h0);
        state.seed_handle(h1);
    }
    if sys_timer_create(0, 1, &mut handle).is_ok() {
        state.seed_handle(handle);
    }
    if sys_debuglog_create(0, 0, &mut handle).is_ok() {
        state.seed_handle(handle);
    }
    if sys_fifo_create(4, 4, 0, &mut h0, &mut h1).is_ok() {
        state.seed_handle(h0);
        state.seed_handle(h1);
    }
    if sys_clock_create(0, 0, &mut handle).is_ok() {
        state.seed_handle(handle);
    }
    if let Ok((h0, h1)) = crate::kernel_objects::compat::create_pair(ObjectType::EventPair) {
        state.seed_handle(h0.0);
        state.seed_handle(h1.0);
    }

    let mut proc_handle = 0u32;
    let mut proc_vmar_handle = 0u32;
    if sys_process_create(
        state.job_handle(),
        state.arena.name_ptr(),
        10,
        0,
        &mut proc_handle,
        &mut proc_vmar_handle,
    )
    .is_ok()
    {
        state.seed_handle(proc_handle);
        state.seed_handle(proc_vmar_handle);
    }

    let mut thread_handle = 0u32;
    if sys_thread_create(
        state.process_handle(),
        state.arena.name_ptr(),
        10,
        0,
        0,
        &mut thread_handle,
    )
    .is_ok()
    {
        state.seed_handle(thread_handle);
    }

    if sys_interrupt_create(0, 0, 0, &mut handle).is_ok() {
        state.seed_handle(handle);
    }
    if sys_iommu_create(0, 0, 0, 0, &mut handle).is_ok() {
        state.seed_handle(handle);
    }
    if sys_bti_create(state.iommu_handle(), 0, 0, &mut handle).is_ok() {
        state.seed_handle(handle);
    }
    if sys_bti_pin(
        state.bti_handle(),
        0,
        state.vmo_handle(),
        0,
        PAGE_SIZE,
        state.arena.words_ptr(),
        1,
        &mut handle,
    )
    .is_ok()
    {
        state.seed_handle(handle);
    }
    if sys_resource_create(0, 0, 0, 0, state.arena.name_ptr(), 10, &mut handle).is_ok() {
        state.seed_handle(handle);
    }
    if sys_pci_get_nth_device(0, 0, state.arena.scratch_ptr(), &mut handle).is_ok() {
        state.seed_handle(handle);
    }

    if let Ok(handle) = crate::kernel_objects::compat::create_object(ObjectType::Guest) {
        state.seed_handle(handle.0);
    }
    if let Ok(handle) = crate::kernel_objects::compat::create_object(ObjectType::Vmar) {
        state.seed_handle(handle.0);
    }
    if sys_vcpu_create(state.guest_handle(), 0, 0, &mut handle).is_ok() {
        state.seed_handle(handle);
    }
    if sys_pager_create(0, &mut handle).is_ok() {
        state.seed_handle(handle);
    }
    if sys_pager_create_vmo(
        state.pager_handle(),
        0,
        state.port_handle(),
        0,
        PAGE_SIZE,
        &mut handle,
    )
    .is_ok()
    {
        state.seed_handle(handle);
    }
    if sys_stream_create(0, state.vmo_handle(), 0, &mut handle).is_ok() {
        state.seed_handle(handle);
    }
    if sys_profile_create(0, 0, 0, &mut handle).is_ok() {
        state.seed_handle(handle);
    }
    if let Ok(handle) = crate::kernel_objects::compat::create_object(ObjectType::Exception) {
        state.seed_handle(handle.0);
    }
}

fn fuzz_linux_round(state: &mut FuzzState) -> bool {
    let mut num = 0;
    while num <= FUZZ_LINUX_INTERFACE_MAX {
        if syscall_logic::linux_syscall_interface_known(num) {
            let variants = linux_variants(num);
            for variant in 0..variants {
                if state.should_stop() {
                    return false;
                }
                let args = linux_args(state, num, variant);
                state.call_linux(num, args);
            }
        }
        num += 1;
    }
    true
}

fn fuzz_zircon_round(state: &mut FuzzState) -> bool {
    for &num in ZIRCON_SUCCESS_SYSCALLS {
        let variants = zircon_variants(num);
        for variant in 0..variants {
            if state.should_stop() {
                return false;
            }
            let args = zircon_args(state, num, variant);
            state.call_zircon(num, args);
        }
    }

    true
}

fn linux_variants(num: u32) -> usize {
    match num {
        23 | 24 | 25 | 56 | 57 | 59 | 63 | 64 | 65 | 66 | 72 | 73 | 78 | 79 | 80 | 85 | 86 | 87
        | 88 | 95 | 98 | 99 | 100 | 101 | 102 | 103 | 107 | 108 | 110 | 112 | 113 | 114 | 115
        | 132 | 134 | 135 | 136 | 137 | 138 | 140 | 141 | 161 | 162 | 163 | 164 | 165 | 168
        | 169 | 179 | 187 | 188 | 189 | 191 | 193 | 195 | 196 | 197 | 198 | 199 | 202 | 204
        | 205 | 206 | 207 | 208 | 209 | 211 | 212 | 214 | 221 | 222 | 226 | 232 | 240 | 242
        | 243 | 260 | 261 | 276 | 277 | 278 | 279 | 283 | 285 | 286 | 287 | 291 | 436 | 437
        | 439 | 441 => 2,
        _ => 1,
    }
}

fn zircon_variants(num: u32) -> usize {
    match num {
        0 | 1 | 5 | 8 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 20 | 23 | 24 | 27 | 28 | 43
        | 53 | 54 | 56 | 57 | 58 | 59 | 60 | 61 | 62 | 63 | 65 | 66 | 67 | 68 | 69 | 70 | 71
        | 72 | 73 | 74 | 75 | 77 | 79 | 82 | 83 | 84 | 86 | 87 | 88 | 89 | 91 | 95 | 96 | 97
        | 106 | 113 | 129 | 140 | 197 | 198 | 199 | 202 => 2,
        _ => 1,
    }
}

fn linux_args(state: &mut FuzzState, num: u32, variant: usize) -> [usize; 6] {
    let len = state.byte_len();
    let ptr = state.user_ptr(len.max(1));
    let fd = state.fd();
    let file_fd = state.file_fd();
    let dir_fd = state.dir_fd();
    let event_fd = state.event_fd();
    let timer_fd = state.timer_fd();
    let socket_fd = state.socket_fd();
    let path = state.cstr_ptr();
    let path2 = state.arena.path_alt_ptr();
    let mut out = [
        state.rng.next_usize(),
        state.rng.next_usize(),
        state.rng.next_usize(),
        state.rng.next_usize(),
        state.rng.next_usize(),
        state.rng.next_usize(),
    ];

    match num {
        5 | 6 | 8 | 9 | 11 | 12 | 14 | 15 => out = [path, state.arena.name_ptr(), ptr, len, 0, 0],
        7 | 10 | 13 | 16 => out = [file_fd, state.arena.name_ptr(), ptr, len, 0, 0],
        17 => out = [ptr, len.max(2), 0, 0, 0, 0],
        19 => out = [state.rng.below(4), variant * LINUX_O_CLOEXEC, 0, 0, 0, 0],
        20 => out = [0, 0, 0, 0, 0, 0],
        21 => out = [fd, variant, state.fd(), ptr, 0, 0],
        22 | 441 => out = [fd, ptr, state.small_count(), 0, 0, 0],
        23 => out = [file_fd, 0, 0, 0, 0, 0],
        24 => out = [file_fd, state.new_fd(), variant * LINUX_O_CLOEXEC, 0, 0, 0],
        25 => out = [file_fd, if variant == 0 { 3 } else { 4 }, 0, 0, 0, 0],
        26 => out = [0, 0, 0, 0, 0, 0],
        27 => out = [fd, path, 0xffff, 0, 0, 0],
        28 => out = [fd, 1, 0, 0, 0, 0],
        29 => out = [fd, state.rng.below(8), ptr, len, 0, 0],
        32 => out = [fd, variant, 0, 0, 0, 0],
        33 | 34 | 35 | 36 | 37 | 38 | 45 | 48 | 49 | 51 | 53 | 54 | 56 | 78 | 79 | 88 | 291
        | 437 | 439 => out = linux_path_args(state, num, variant, dir_fd, path, path2, ptr),
        39 => out = [path, 0, 0, 0, 0, 0],
        40 => out = [path, path2, state.arena.name_ptr(), variant, 0, 0],
        41 => out = [path, path2, 0, 0, 0, 0],
        43 => out = [path, ptr, 0, 0, 0, 0],
        44 => out = [file_fd, ptr, 0, 0, 0, 0],
        46 | 47 | 52 | 55 | 82 | 83 | 84 => out = [file_fd, ptr, len, variant, ptr, len],
        50 => out = [dir_fd, 0, 0, 0, 0, 0],
        57 => out = [state.linux_close_fd(), 0, 0, 0, 0, 0],
        62 => out = [file_fd, 0, 0, 0, 0, 0],
        71 => out = [file_fd, file_fd, 0, FUZZ_IO_BYTES, 0, 0],
        85 => out = [1, 0, 0, 0, 0, 0],
        86 => out = [timer_fd, 0, state.arena.itimer_ptr(), ptr, 0, 0],
        87 => out = [timer_fd, ptr, 0, 0, 0, 0],
        59 => {
            out = [
                state.arena.handles_ptr(),
                variant * LINUX_O_CLOEXEC,
                0,
                0,
                0,
                0,
            ]
        }
        61 => out = [dir_fd, ptr, len, 0, 0, 0],
        63 => out = [file_fd, ptr, len, 0, 0, 0],
        64 => out = [state.transient_file_fd(), ptr, len, 0, 0, 0],
        65 => out = [file_fd, state.arena.iov_ptr(), 2, 0, 0, 0],
        66 => out = [state.transient_file_fd(), state.arena.iov_ptr(), 2, 0, 0, 0],
        69 | 286 => out = [file_fd, state.arena.iov_ptr(), 2, 0, 0, 0],
        70 | 287 => out = [state.transient_file_fd(), state.arena.iov_ptr(), 2, 0, 0, 0],
        67 => out = [file_fd, ptr, len, 0, 0, 0],
        68 => out = [state.transient_file_fd(), ptr, len, 0, 0, 0],
        72 => out = [0, 0, 0, 0, state.arena.timespec_ptr(), 0],
        73 => {
            out = [
                state.arena.poll_ptr(),
                2,
                state.arena.timespec_ptr(),
                0,
                0,
                0,
            ]
        }
        74 => out = [event_fd, ptr, LINUX_SIGSET_SIZE, 0, 0, 0],
        75 => out = [state.transient_file_fd(), state.arena.iov_ptr(), 2, 0, 0, 0],
        76 => {
            out = [
                state.pipe_read_fd(),
                0,
                state.pipe_write_fd(),
                0,
                FUZZ_IO_BYTES,
                0,
            ]
        }
        77 => {
            out = [
                state.pipe_read_fd(),
                state.pipe_write_fd(),
                FUZZ_IO_BYTES,
                0,
                0,
                0,
            ]
        }
        80 => out = [file_fd, ptr, 0, 0, 0, 0],
        81 | 124 | 139 | 155 | 172 | 173 | 174 | 175 | 176 | 177 | 178 => out = [0, 0, 0, 0, 0, 0],
        127 => out = [0, ptr, 0, 0, 0, 0],
        128 | 129 | 130 => out = [1, 0, 0, 0, 0, 0],
        131 => out = [1, 1, 0, 0, 0, 0],
        90 => {
            out = [
                state.arena.cap_header_ptr(),
                state.arena.cap_data_ptr(),
                0,
                0,
                0,
                0,
            ]
        }
        91 => {
            out = [
                state.arena.cap_header_ptr(),
                state.arena.cap_data_ptr(),
                0,
                0,
                0,
                0,
            ]
        }
        95 => out = [0, 0, ptr, 0, 0, 0],
        96 => out = [ptr, 0, 0, 0, 0, 0],
        97 => out = [LINUX_CONTAINER_NAMESPACE_FLAGS, 0, 0, 0, 0, 0],
        98 => {
            out = [
                state.arena.futex_ptr(),
                variant,
                0,
                0,
                state.arena.alt_futex_ptr(),
                0,
            ]
        }
        99 => out = [ptr, len, 0, 0, 0, 0],
        100 => out = [0, state.arena.words_ptr(), state.arena.words_ptr(), 0, 0, 0],
        101 => out = [state.arena.timespec_ptr(), 0, 0, 0, 0, 0],
        115 => out = [1, 0, state.arena.timespec_ptr(), 0, 0, 0],
        102 | 103 => out = [0, state.arena.itimer_ptr(), ptr, 0, 0, 0],
        107 => out = [1, 0, state.arena.words_ptr(), 0, 0, 0],
        108 => out = [state.linux_timer_handle(), ptr, 0, 0, 0, 0],
        110 => {
            out = [
                state.linux_timer_handle(),
                variant,
                state.arena.itimer_ptr(),
                ptr,
                0,
                0,
            ]
        }
        109 => out = [state.linux_timer_handle(), ptr, 0, 0, 0, 0],
        111 => out = [state.linux_timer_delete_handle(), 0, 0, 0, 0, 0],
        112 | 113 | 114 => out = [1, state.arena.timespec_ptr(), 0, 0, 0, 0],
        116 | 117 | 171 | 180 | 181 | 182 | 183 | 184 | 185 | 217 | 218 | 219 | 224 | 225 | 235
        | 236 | 237 | 238 | 239 | 241 | 267 | 269 | 270 | 271 | 272 | 274 | 275 | 280 | 281
        | 282 | 284 | 288 | 289 | 290 | 292 | 293 | 428 | 429 | 430 | 431 | 432 | 434 | 442 => {
            out = [0, ptr, len, 0, 0, 0]
        }
        118 => out = [0, ptr, 0, 0, 0, 0],
        119 => out = [0, 0, ptr, 0, 0, 0],
        120 => out = [0, 0, 0, 0, 0, 0],
        121 => out = [0, ptr, 0, 0, 0, 0],
        122 | 123 => out = [0, FUZZ_IO_BYTES, ptr, 0, 0, 0],
        125 | 126 => out = [0, 0, 0, 0, 0, 0],
        132 => out = [ptr, ptr, 0, 0, 0, 0],
        133 => out = [ptr, LINUX_SIGSET_SIZE, 0, 0, 0, 0],
        134 => out = [variant + 1, ptr, ptr, LINUX_SIGSET_SIZE, 0, 0],
        135 => out = [variant, ptr, ptr, LINUX_SIGSET_SIZE, 0, 0],
        136 => out = [ptr, LINUX_SIGSET_SIZE, 0, 0, 0, 0],
        137 => out = [ptr, ptr, 0, LINUX_SIGSET_SIZE, 0, 0],
        138 => out = [1, variant + 1, ptr, 0, 0, 0],
        240 => out = [1, 1, variant + 1, ptr, 0, 0],
        140 | 141 | 143 | 144 | 145 | 147 | 149 | 151 | 152 | 154 | 156 | 157 | 159 | 164 => {
            out = [variant, path, len, ptr, 0, 0]
        }
        142 | 170 => out = [1, state.arena.timespec_ptr(), 0, 0, 0, 0],
        161 | 162 => out = [ptr, len.min(32), 0, 0, 0, 0],
        146 | 148 => out = [ptr, ptr, ptr, 0, 0, 0],
        150 => out = [ptr, ptr, ptr, 0, 0, 0],
        153 => out = [ptr, 0, 0, 0, 0, 0],
        160 => out = [ptr, 0, 0, 0, 0, 0],
        158 => out = [2, ptr, 0, 0, 0, 0],
        163 | 165 => out = [0, ptr, 0, 0, 0, 0],
        166 => out = [0o022, 0, 0, 0, 0, 0],
        167 => out = [if variant == 0 { 39 } else { 16 }, ptr, 0, 0, 0, 0],
        168 => out = [ptr, ptr, 0, 0, 0, 0],
        169 => out = [ptr, 0, 0, 0, 0, 0],
        179 => out = [ptr, 0, 0, 0, 0, 0],
        186 => out = [1, 0, 0, 0, 0, 0],
        187 => out = [state.msg_handle() as usize, variant, ptr, 0, 0, 0],
        188 => out = [state.msg_handle() as usize, ptr, FUZZ_IO_BYTES, 0, 0, 0],
        189 => out = [state.msg_handle() as usize, ptr, FUZZ_IO_BYTES, 0, 0, 0],
        190 => out = [1, 1, 0, 0, 0, 0],
        191 => out = [state.sem_handle() as usize, 0, variant, ptr, 0, 0],
        192 => out = [state.sem_handle() as usize, ptr, 0, 0, 0, 0],
        193 => out = [state.sem_handle() as usize, ptr, variant, 0, 0, 0],
        194 => out = [1, PAGE_SIZE, 0, 0, 0, 0],
        195 => out = [state.shm_handle() as usize, variant, ptr, 0, 0, 0],
        196 | 197 => out = [state.shm_handle() as usize, state.mapping(), 0, 0, 0, 0],
        198 => out = [LINUX_AF_UNIX, LINUX_SOCK_STREAM, 0, 0, 0, 0],
        199 => {
            out = [
                LINUX_AF_UNIX,
                LINUX_SOCK_DGRAM,
                0,
                state.arena.handles_ptr(),
                0,
                0,
            ]
        }
        200 | 203 => out = [socket_fd, ptr, len, 0, 0, 0],
        201 | 210 => out = [socket_fd, variant, 0, 0, 0, 0],
        204 | 205 => out = [socket_fd, ptr, state.arena.u32_out_ptr(16), 0, 0, 0],
        208 => out = [socket_fd, 0, 0, ptr, len, 0],
        209 => out = [socket_fd, 0, 0, ptr, state.arena.u32_out_ptr(4), 0],
        202 | 242 => out = [socket_fd, ptr, state.arena.words_ptr(), 0, 0, 0],
        206 => out = [state.socketpair_fd(), ptr, len.max(1), 0, 0, 0],
        207 => out = [state.socket_peer_fd(), ptr, len.max(1), 0, 0, 0],
        211 | 212 | 243 => out = [socket_fd, ptr, 1, 0, state.arena.timespec_ptr(), 0],
        213 | 223 => out = [file_fd, 0, FUZZ_IO_BYTES, 0, 0, 0],
        215 => out = [state.transient_mapping(), PAGE_SIZE, 0, 0, 0, 0],
        216 => out = [state.mapping(), PAGE_SIZE, PAGE_SIZE, 0, 0, 0],
        227 | 228 | 229 | 233 | 234 => out = [state.mapping(), PAGE_SIZE, variant, 0, 0, 0],
        232 => out = [state.mapping(), PAGE_SIZE, ptr, 0, 0, 0],
        214 => out = [if variant == 0 { 0 } else { 0x4000_1000 }, 0, 0, 0, 0, 0],
        220 => out = [LINUX_CONTAINER_NAMESPACE_FLAGS, 0, 0, 0, 0, 0],
        221 => out = [path, 0, 0, 0, 0, 0],
        222 => {
            out = [
                0,
                state.page_len().max(PAGE_SIZE),
                MmapProt::READ.bits() | MmapProt::WRITE.bits(),
                MmapFlags::PRIVATE.bits() | MmapFlags::ANONYMOUS.bits(),
                0,
                0,
            ]
        }
        226 => out = [state.mapping(), PAGE_SIZE, MmapProt::READ.bits(), 0, 0, 0],
        230 | 231 | 283 => out = [variant, 0, 0, 0, 0, 0],
        260 => out = [0, ptr, 0, 0, 0, 0],
        261 => out = [0, 0, 0, ptr, 0, 0],
        285 => {
            out = [
                state.pipe_read_fd(),
                0,
                state.pipe_write_fd(),
                0,
                FUZZ_IO_BYTES,
                0,
            ]
        }
        268 => out = [file_fd, LINUX_CONTAINER_NAMESPACE_FLAGS, 0, 0, 0, 0],
        276 => out = [0, path, 0, path2, variant, 0],
        277 => {
            out = [
                if variant == 0 {
                    LINUX_SECCOMP_GET_NOTIF_SIZES
                } else {
                    LINUX_SECCOMP_SET_MODE_FILTER
                },
                if variant == 0 {
                    0
                } else {
                    LINUX_SECCOMP_FILTER_ALLOWED_FLAGS
                },
                ptr,
                0,
                0,
                0,
            ]
        }
        278 => out = [ptr, len, variant & 0x3, 0, 0, 0],
        279 => out = [state.arena.name_ptr(), variant & 0x7, 0, 0, 0, 0],
        435 => out = [state.arena.words_ptr(), 64, 0, 0, 0, 0],
        436 => {
            let fd = FUZZ_LINUX_NEW_FD_BASE + FUZZ_LINUX_NEW_FD_SPAN + variant;
            out = [fd, fd, 0, 0, 0, 0];
        }
        _ => {}
    }

    out
}

#[allow(clippy::too_many_arguments)]
fn linux_path_args(
    state: &mut FuzzState,
    num: u32,
    variant: usize,
    fd: usize,
    path: usize,
    path2: usize,
    ptr: usize,
) -> [usize; 6] {
    let visible_path = state.arena.path_ptr();
    match num {
        35 => [fd, path, variant * 0x200, 0, 0, 0],
        36 => [path, fd, path2, 0, 0, 0],
        37 => [fd, path, fd, path2, 0, 0],
        38 => [fd, path, fd, path2, 0, 0],
        45 => [path, FUZZ_IO_BYTES, 0, 0, 0, 0],
        48 | 439 => [fd, visible_path, 0, 0, 0, 0],
        56 => [
            usize::MAX - 99,
            path,
            if variant == 0 {
                LINUX_O_CREAT | LINUX_O_RDWR
            } else {
                LINUX_O_DIRECTORY
            },
            0,
            0,
            0,
        ],
        78 => [fd, visible_path, ptr, FUZZ_IO_BYTES, 0, 0],
        79 => [fd, visible_path, ptr, 0, 0, 0],
        88 => [fd, path, state.arena.timespec_ptr(), 0, 0, 0],
        291 => [fd, visible_path, 0, 0x7ff, ptr, 0],
        437 => [fd, visible_path, ptr, 0, 0, 0],
        _ => [fd, path, variant, ptr, FUZZ_IO_BYTES, 0],
    }
}

fn zircon_args(state: &mut FuzzState, num: u32, variant: usize) -> [usize; 8] {
    let event_handle = state.event_handle();
    let eventpair_handle = state.eventpair_handle();
    let port_handle = state.port_handle();
    let job_handle = state.job_handle();
    let channel_handle = state.channel_handle();
    let channel_peer_handle = state.channel_peer_handle();
    let socket_handle = state.socket_handle();
    let socket_peer_handle = state.socket_peer_handle();
    let timer_handle = state.timer_handle();
    let debuglog_handle = state.debuglog_handle();
    let fifo_handle = state.fifo_handle();
    let fifo_peer_handle = state.fifo_peer_handle();
    let clock_handle = state.clock_handle();
    let ipc_handle = state.ipc_handle();
    let process_handle = state.process_handle();
    let thread_handle = state.thread_handle();
    let interrupt_handle = state.interrupt_handle();
    let iommu_handle = state.iommu_handle();
    let bti_handle = state.bti_handle();
    let resource_handle = state.resource_handle();
    let pci_handle = state.pci_handle();
    let guest_handle = state.guest_handle();
    let vcpu_handle = state.vcpu_handle();
    let pager_handle = state.pager_handle();
    let pager_vmo_handle = state.pager_vmo_handle();
    let stream_handle = state.stream_handle();
    let profile_handle = state.profile_handle();
    let exception_handle = state.exception_handle();
    let len = state.byte_len();
    let ptr = state.user_ptr(len.max(1));
    let latest_vmo_handle = state.latest_vmo_handle();
    let mut out = [
        state.rng.next_usize(),
        state.rng.next_usize(),
        state.rng.next_usize(),
        state.rng.next_usize(),
        state.rng.next_usize(),
        state.rng.next_usize(),
        state.rng.next_usize(),
        state.rng.next_usize(),
    ];

    match num {
        0 | 1 => out = [1, state.arena.words_ptr(), 0, 0, 0, 0, 0, 0],
        2 | 3 => out = [0, 0, 0, 0, 0, 0, 0, 0],
        4 => out = [1, 0, 0, 0, 0, 0, 0, 0],
        5 | 202 => out = [0, (variant & 0x3) as usize, 0, 0, 0, 0, 0, 0],
        8 => {
            out = [
                latest_vmo_handle as usize,
                RIGHT_SAME_RIGHTS as usize,
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        9 => {
            out = [
                state.zircon_handle_for_replace() as usize,
                RIGHT_SAME_RIGHTS as usize,
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        6 => out = [state.transient_event_handle() as usize, 0, 0, 0, 0, 0, 0, 0],
        7 => {
            state.arena.handles[0] = state.transient_event_handle();
            state.arena.handles[1] = state.transient_event_handle();
            out = [state.arena.handles_ptr(), 2, 0, 0, 0, 0, 0, 0];
        }
        10 => out = [event_handle as usize, 0, 1, 0, 0, 0, 0, 0],
        11 => {
            state.arena.wait_items[0].handle = event_handle;
            state.arena.wait_items[0].waitfor = 0;
            out = [state.arena.wait_items_ptr(), 1, 1, 0, 0, 0, 0, 0];
        }
        12 => {
            out = [
                event_handle as usize,
                port_handle as usize,
                1,
                ZX_USER_SIGNAL_0,
                0,
                0,
                0,
                0,
            ]
        }
        13 => out = [event_handle as usize, 0, ZX_USER_SIGNAL_0, 0, 0, 0, 0, 0],
        14 => {
            out = [
                eventpair_handle as usize,
                0,
                ZX_USER_SIGNAL_0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        15 | 16 | 17 => out = [event_handle as usize, variant, ptr, len.max(8), 0, 0, 0, 0],
        18 => {
            out = [
                event_handle as usize,
                0,
                RIGHT_SAME_RIGHTS as usize,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        19 => {
            out = [
                event_handle as usize,
                profile_handle as usize,
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        20 | 53 | 54 | 61 | 65 | 87 | 88 | 140 | 154 | 184..=186 | 193..=197 => {
            out = [0, 0, 0, 0, 0, 0, 0, 0]
        }
        27 => out = [variant & 1, 0, 0, 0, 0, 0, 0, 0],
        68 => out = [PAGE_SIZE as usize, 1, 0, 0, 0, 0, 0, 0],
        21 | 22 | 23 | 24 => out = [channel_handle as usize, 0, ptr, len.max(1), 0, 0, 0, 0],
        25 => {
            out = [
                channel_handle as usize,
                0,
                ptr,
                len.max(1),
                0,
                ptr,
                FUZZ_SCRATCH_BYTES,
                0,
            ]
        }
        26 | 144..=153 => out = [0, 0, 0, 0, 0, 0, 0, 0],
        28 => out = [socket_handle as usize, 0, ptr, len.max(1), 0, 0, 0, 0],
        29 => out = [socket_handle as usize, 0, ptr, len.max(1), 0, 0, 0, 0],
        30 => {
            out = [
                socket_handle as usize,
                socket_peer_handle as usize,
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        31 => out = [socket_peer_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        32 => {
            out = [
                state.transient_socket_handle() as usize,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        33 => out = [0, 0, 0, 0, 0, 0, 0, 0],
        34 => {
            out = [
                process_handle as usize,
                state.arena.name_ptr(),
                10,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        35 => {
            out = [
                state.transient_thread_handle() as usize,
                1,
                PAGE_SIZE,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        36 | 37 => out = [thread_handle as usize, 0, ptr, 8, 0, 0, 0, 0],
        38 => {
            out = [
                state.transient_process_handle() as usize,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        39 => {
            out = [
                job_handle as usize,
                state.arena.name_ptr(),
                10,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        40 => {
            let (proc_handle, _vmar_handle) = state.transient_process_pair();
            let mut start_thread = 0u32;
            if sys_thread_create(
                proc_handle,
                state.arena.name_ptr(),
                10,
                0,
                0,
                &mut start_thread,
            )
            .is_err()
            {
                start_thread = state.transient_thread_handle();
            }
            out = [
                proc_handle as usize,
                start_thread as usize,
                1,
                PAGE_SIZE,
                0,
                0,
                0,
                0,
            ];
        }
        41 | 42 => out = [process_handle as usize, 0, ptr, len, 0, 0, 0, 0],
        44 => out = [job_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        45 => {
            out = [
                process_handle as usize,
                INVALID_HANDLE as usize,
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        46 | 47 => out = [process_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        48 => {
            out = [
                process_handle as usize,
                exception_handle as usize,
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        49 | 203 => out = [process_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        50 => {
            out = [
                state.transient_started_thread_handle() as usize,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        51 | 52 | 209 | 210 => out = [exception_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        82 | 83 | 95 | 97 => out = [ptr, len, 0, 0, 0, 0, 0, 0],
        96 => out = [ptr, 0, 0, 0, 0, 0, 0, 0],
        89 => out = [debuglog_handle as usize, 0, ptr, 0, 0, 0, 0, 0],
        91 => out = [0, ptr, len, 0, 0, 0, 0, 0],
        43 => out = [job_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        55 => out = [state.arena.futex_ptr(), 0, 1, 1, 0, 0, 0, 0],
        56 | 58 => out = [state.arena.futex_ptr(), 1, 0, 0, 0, 0, 0, 0],
        57 | 59 => {
            out = [
                state.arena.futex_ptr(),
                1,
                0,
                state.arena.alt_futex_ptr(),
                1,
                0,
                0,
                0,
            ]
        }
        60 => out = [state.arena.futex_ptr(), 0, 0, 0, 0, 0, 0, 0],
        64 => {
            out = [
                port_handle as usize,
                event_handle as usize,
                1,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        62 => {
            out = [
                port_handle as usize,
                state.arena.packet_ptr(),
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        63 => {
            out = [
                port_handle as usize,
                1,
                state.arena.packet_ptr(),
                0,
                0,
                0,
                0,
                0,
            ]
        }
        66 | 67 => out = [timer_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        69 | 70 => out = [latest_vmo_handle as usize, ptr, len, 0, 0, 0, 0, 0],
        71 => out = [latest_vmo_handle as usize, PAGE_SIZE, 0, 0, 0, 0, 0, 0],
        72 => out = [state.vmo_handle() as usize, PAGE_SIZE, 0, 0, 0, 0, 0, 0],
        73 => out = [latest_vmo_handle as usize, 10, 0, len, 0, 0, 0, 0],
        74 => out = [latest_vmo_handle as usize, 0, 0, PAGE_SIZE, 0, 0, 0, 0],
        75 => out = [latest_vmo_handle as usize, variant, 0, 0, 0, 0, 0, 0],
        76 => {
            out = [
                state.zircon_handle_for_replace() as usize,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        77 => {
            let offset = FUZZ_ZIRCON_VMAR_ALLOC_OFFSET + (variant * PAGE_SIZE);
            out = [
                memory_root_vmar_handle() as usize,
                VmarFlags::SPECIFIC.bits() as usize,
                offset,
                PAGE_SIZE,
                0,
                0,
                0,
                0,
            ]
        }
        78 => {
            out = [
                state.transient_child_vmar_handle() as usize,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        79 => {
            let offset = FUZZ_ZIRCON_VMAR_MAP_OFFSET + (variant * PAGE_SIZE);
            out = [
                memory_root_vmar_handle() as usize,
                (VmOptions::PERM_RW | VmOptions::SPECIFIC_OVERWRITE).bits() as usize,
                offset,
                latest_vmo_handle as usize,
                0,
                PAGE_SIZE,
                0,
                0,
            ]
        }
        80 => {
            let offset = FUZZ_ZIRCON_VMAR_MAP_OFFSET + 0x0010_0000 + (variant * PAGE_SIZE);
            let addr = dispatch_zircon_syscall(
                79,
                [
                    memory_root_vmar_handle() as usize,
                    (VmOptions::PERM_RW | VmOptions::SPECIFIC_OVERWRITE).bits() as usize,
                    offset,
                    latest_vmo_handle as usize,
                    0,
                    PAGE_SIZE,
                    0,
                    0,
                ],
            )
            .unwrap_or(0);
            out = [
                memory_root_vmar_handle() as usize,
                addr,
                PAGE_SIZE,
                0,
                0,
                0,
                0,
                0,
            ];
        }
        81 => {
            out = [
                memory_root_vmar_handle() as usize,
                VmOptions::PERM_READ.bits() as usize,
                state.zircon_mapping(),
                PAGE_SIZE,
                0,
                0,
                0,
                0,
            ]
        }
        84 => out = [4, 4, 0, 0, 0, 0, 0, 0],
        85 => {
            out = [
                fifo_handle as usize,
                4,
                ptr,
                state.small_count().max(1),
                0,
                0,
                0,
                0,
            ]
        }
        86 => {
            out = [
                fifo_handle as usize,
                4,
                ptr,
                state.small_count().max(1),
                0,
                0,
                0,
                0,
            ]
        }
        90 => out = [debuglog_handle as usize, 0, ptr, len.max(1), 0, 0, 0, 0],
        98 => out = [0, 0, 0, 0, 0, 0, 0, 0],
        99 => {
            out = [
                interrupt_handle as usize,
                port_handle as usize,
                1,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        100 => {
            out = [
                interrupt_handle as usize,
                state.arena.words_ptr(),
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        101 | 207 => {
            out = [
                state.transient_interrupt_handle() as usize,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        102 | 208 => out = [interrupt_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        103 | 206 => out = [interrupt_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        104 => {
            out = [
                interrupt_handle as usize,
                vcpu_handle as usize,
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        105 | 211 => out = [0, 0, 0, 0, 0, 0, 0, 0],
        108 => out = [0, 0, 0, 0, 0, 0, 0, 0],
        109 => out = [iommu_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        110 => {
            out = [
                bti_handle as usize,
                0,
                latest_vmo_handle as usize,
                0,
                PAGE_SIZE,
                state.arena.words_ptr(),
                1,
                0,
            ]
        }
        111 | 204 => out = [bti_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        112 => out = [state.transient_pmt_handle() as usize, 0, 0, 0, 0, 0, 0, 0],
        114 => out = [latest_vmo_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        115 => out = [resource_handle as usize, variant, ptr, 0, 0, 0, 0, 0],
        116 => out = [pci_handle as usize, 1, 0, 0, 0, 0, 0, 0],
        117 => out = [pci_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        118 => {
            out = [
                pci_handle as usize,
                0,
                4,
                state.arena.words_ptr(),
                0,
                0,
                0,
                0,
            ]
        }
        119 => out = [pci_handle as usize, 0, 4, 0, 0, 0, 0, 0],
        120 => {
            out = [
                pci_handle as usize,
                0,
                0,
                0,
                0,
                state.arena.words_ptr(),
                4,
                0,
            ]
        }
        121 => out = [pci_handle as usize, 0, ptr, 0, 0, 0, 0, 0],
        122 => out = [pci_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        123 => {
            out = [
                pci_handle as usize,
                0,
                state.arena.words_ptr(),
                0,
                0,
                0,
                0,
                0,
            ]
        }
        124 => out = [pci_handle as usize, 0, 1, 0, 0, 0, 0, 0],
        125 => out = [pci_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        126 => out = [pci_handle as usize, 1, 0, 1, 1, 0, 0, 0],
        127 | 205 => {
            out = [
                resource_handle as usize,
                state.arena.words_ptr(),
                state.arena.scratch_ptr(),
                0,
                0,
                0,
                0,
                0,
            ]
        }
        128 => {
            out = [
                resource_handle as usize,
                state.arena.scratch_ptr(),
                state.arena.scratch_ptr_offset(128),
                0,
                0,
                0,
                0,
                0,
            ]
        }
        188 | 190 => {
            out = [
                stream_handle as usize,
                0,
                state.arena.iov_ptr(),
                2,
                state.arena.words_ptr(),
                0,
                0,
                0,
            ]
        }
        189 | 191 => {
            out = [
                stream_handle as usize,
                0,
                0,
                state.arena.iov_ptr(),
                2,
                state.arena.words_ptr(),
                0,
                0,
            ]
        }
        192 => out = [stream_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        92 | 93 | 94 => out = [0, 0, 0, ptr, 0, 0, 0, 0],
        106 => out = [0, PAGE_SIZE, 1, 0, 0, 0, 0, 0],
        107 => out = [0, 0, PAGE_SIZE, 0, 0, 0, 0, 0],
        113 => out = [ptr, 0, 0, 0, 0, 0, 0, 0],
        129 => out = [0, 0, 0, len, state.arena.name_ptr(), 10, 0, 0],
        130 => out = [resource_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        131 => {
            out = [
                guest_handle as usize,
                2,
                0,
                1,
                INVALID_HANDLE as usize,
                0,
                0,
                0,
            ]
        }
        132 => out = [guest_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        133 => {
            out = [
                vcpu_handle as usize,
                state.arena.scratch_ptr(),
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        134 => out = [vcpu_handle as usize, 1, 0, 0, 0, 0, 0, 0],
        135 => {
            out = [
                vcpu_handle as usize,
                0,
                state.arena.scratch_ptr(),
                ZX_VCPU_STATE_SIZE,
                0,
                0,
                0,
                0,
            ]
        }
        136 => {
            let write_size = if variant == 0 {
                ZX_VCPU_STATE_SIZE
            } else {
                ZX_VCPU_IO_SIZE
            };
            out = [
                vcpu_handle as usize,
                variant,
                state.arena.scratch_ptr(),
                write_size,
                0,
                0,
                0,
                0,
            ]
        }
        137 => {
            out = [
                latest_vmo_handle as usize,
                state.vmo_handle() as usize,
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        138 => out = [ptr, len, 0, 0, 0, 0, 0, 0],
        139 => out = [resource_handle as usize, 0, 0, 0, 0, 0, 0, 0],
        141 => {
            out = [
                pager_handle as usize,
                0,
                port_handle as usize,
                0,
                PAGE_SIZE,
                0,
                0,
                0,
            ]
        }
        142 => {
            out = [
                pager_handle as usize,
                pager_vmo_handle as usize,
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        143 => {
            out = [
                pager_handle as usize,
                pager_vmo_handle as usize,
                0,
                PAGE_SIZE,
                latest_vmo_handle as usize,
                0,
                0,
                0,
            ]
        }
        183 => {
            out = [
                job_handle as usize,
                0,
                process_handle as usize,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        187 => out = [0, latest_vmo_handle as usize, 0, 0, 0, 0, 0, 0],
        200 => {
            out = [
                state.arena.futex_ptr(),
                1,
                0,
                state.transient_event_handle() as usize,
                0,
                0,
                0,
                0,
            ]
        }
        201 => {
            let (vmar_handle, addr) = state.transient_vmar_mapping();
            out = [vmar_handle as usize, addr, PAGE_SIZE, 0, 0, 0, 0, 0];
        }
        198 => {
            out = [
                clock_handle as usize,
                state.arena.words_ptr(),
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        199 => out = [clock_handle as usize, variant, ptr, 0, 0, 0, 0, 0],
        _ => {
            out[0] = ipc_handle as usize;
            out[1] = ptr;
            out[2] = len;
        }
    }

    match num {
        21 | 22 => {
            let _ = dispatch_zircon_syscall(
                23,
                [channel_peer_handle as usize, 0, ptr, len.max(1), 0, 0, 0, 0],
            );
        }
        23 | 24 if len != 0 => {
            let _ = dispatch_zircon_syscall(
                21,
                [channel_peer_handle as usize, 0, ptr, len, 0, 0, 0, 0],
            );
        }
        25 => {
            let _ = dispatch_zircon_syscall(
                23,
                [channel_peer_handle as usize, 0, ptr, len.max(1), 0, 0, 0, 0],
            );
        }
        28 => {
            let _ = dispatch_zircon_syscall(
                29,
                [socket_peer_handle as usize, 0, ptr, len.max(1), 0, 0, 0, 0],
            );
        }
        29 => {
            let _ = dispatch_zircon_syscall(
                28,
                [socket_peer_handle as usize, 0, ptr, len.max(1), 0, 0, 0, 0],
            );
        }
        31 => {
            let _ = dispatch_zircon_syscall(
                30,
                [
                    socket_handle as usize,
                    socket_handle as usize,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                ],
            );
        }
        85 => {
            let _ = dispatch_zircon_syscall(
                86,
                [fifo_peer_handle as usize, 4, ptr, out[3], 0, 0, 0, 0],
            );
        }
        86 => {
            let _ = dispatch_zircon_syscall(
                85,
                [fifo_peer_handle as usize, 4, ptr, out[3], 0, 0, 0, 0],
            );
        }
        90 => {
            let _ = dispatch_zircon_syscall(
                89,
                [debuglog_handle as usize, 0, ptr, len.max(1), 0, 0, 0, 0],
            );
        }
        190 => {
            let _ = dispatch_zircon_syscall(
                188,
                [
                    stream_handle as usize,
                    0,
                    state.arena.iov_ptr(),
                    2,
                    state.arena.words_ptr(),
                    0,
                    0,
                    0,
                ],
            );
        }
        191 => {
            let _ = dispatch_zircon_syscall(
                189,
                [
                    stream_handle as usize,
                    0,
                    0,
                    state.arena.iov_ptr(),
                    2,
                    state.arena.words_ptr(),
                    0,
                    0,
                ],
            );
        }
        _ => {}
    }

    out
}

fn write_cstr(out: &mut [u8], value: &[u8]) {
    let count = value.len().min(out.len().saturating_sub(1));
    out[..count].copy_from_slice(&value[..count]);
    if count < out.len() {
        out[count] = 0;
    }
}
