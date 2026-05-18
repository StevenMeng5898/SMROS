use super::{
    dispatch_linux_syscall, dispatch_zircon_syscall, memory_root_vmar_handle, sys_channel_create,
    sys_close, sys_event_create, sys_eventfd2, sys_handle_close, sys_job_create, sys_mmap,
    sys_msgget, sys_munmap, sys_openat, sys_pipe2, sys_port_create, sys_semget, sys_shmget,
    sys_socket, sys_socket_create, sys_socketpair, sys_timerfd_create, sys_vmo_create,
    LinuxCapUserData, LinuxCapUserHeader, LinuxIovec, LinuxItimerval, LinuxPollFd, LinuxTimespec,
    LinuxTimeval, MmapFlags, MmapProt, SysError, ZxError, ZxWaitItem, LINUX_AF_UNIX,
    LINUX_CAPABILITY_VERSION_3, LINUX_CONTAINER_NAMESPACE_FLAGS, LINUX_O_CLOEXEC, LINUX_O_CREAT,
    LINUX_O_DIRECTORY, LINUX_O_RDWR, LINUX_SECCOMP_FILTER_ALLOWED_FLAGS,
    LINUX_SECCOMP_GET_NOTIF_SIZES, LINUX_SECCOMP_SET_MODE_FILTER, LINUX_SIGSET_SIZE,
    LINUX_SOCK_DGRAM, LINUX_SOCK_STREAM,
};
use crate::kernel_lowlevel::memory::PAGE_SIZE;
use crate::kernel_lowlevel::timer;
use crate::kernel_objects::port::{PortPacket, PORT_PACKET_TYPE_USER};
use crate::kernel_objects::{Rights, RIGHT_SAME_RIGHTS};

const FUZZ_SCRATCH_BYTES: usize = 4096;
const FUZZ_LINUX_MAX: u32 = 446;
const FUZZ_ZIRCON_MAX: u32 = 211;
const FUZZ_DEFAULT_ITERATIONS: usize = 2;
const FUZZ_IO_BYTES: usize = 64;

#[derive(Clone, Copy, Default)]
pub struct SyscallFuzzReport {
    pub seed: u64,
    pub iterations: usize,
    pub completed_iterations: usize,
    pub time_limit_ticks: u64,
    pub elapsed_ticks: u64,
    pub timed_out: bool,
    pub linux_calls: usize,
    pub linux_ok: usize,
    pub linux_err: usize,
    pub linux_enosys: usize,
    pub zircon_calls: usize,
    pub zircon_ok: usize,
    pub zircon_err: usize,
    pub zircon_unsupported: usize,
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
        write_cstr(&mut self.name, b"smros-fuzz");
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
            len: 16,
        };
        self.iov[1] = LinuxIovec {
            base: self.scratch_ptr_offset(64),
            len: 32,
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
    fds: [usize; 64],
    fd_count: usize,
    mappings: [usize; 32],
    mapping_count: usize,
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
            fds: [0; 64],
            fd_count: 0,
            mappings: [0; 32],
            mapping_count: 0,
            start_tick,
            deadline_tick,
            report: SyscallFuzzReport {
                seed,
                iterations,
                time_limit_ticks,
                ..SyscallFuzzReport::default()
            },
        }
    }

    fn track_handle(&mut self, handle: u32) {
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

    fn fd(&mut self) -> usize {
        match self.rng.below(4) {
            0 => 0,
            1 => 1,
            _ if self.fd_count != 0 => self.fds[self.rng.below(self.fd_count)],
            _ => usize::MAX,
        }
    }

    fn tracked_fd(&mut self) -> usize {
        if self.fd_count == 0 {
            usize::MAX
        } else {
            self.fds[self.rng.below(self.fd_count)]
        }
    }

    fn handle(&mut self) -> u32 {
        if self.handle_count == 0 || self.rng.below(5) == 0 {
            0
        } else {
            self.handles[self.rng.below(self.handle_count)]
        }
    }

    fn mapping(&mut self) -> usize {
        if self.mapping_count == 0 || self.rng.below(4) == 0 {
            0x5000_0000
        } else {
            self.mappings[self.rng.below(self.mapping_count)]
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
        } else if self.rng.below(8) == 0 {
            0
        } else {
            self.arena.scratch_ptr_offset(self.rng.below(128))
        }
    }

    fn cstr_ptr(&mut self) -> usize {
        match self.rng.below(5) {
            0 => 0,
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

    fn call_linux(&mut self, num: u32, args: [usize; 6]) {
        self.report.linux_calls += 1;
        match dispatch_linux_syscall(num, args) {
            Ok(value) => {
                self.report.linux_ok += 1;
                self.capture_linux_result(num, value, &args);
            }
            Err(SysError::ENOSYS) => {
                self.report.linux_err += 1;
                self.report.linux_enosys += 1;
            }
            Err(_) => {
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
            }
            Err(ZxError::ErrNotSupported) => {
                self.report.zircon_err += 1;
                self.report.zircon_unsupported += 1;
            }
            Err(_) => {
                self.report.zircon_err += 1;
            }
        }
    }

    fn capture_linux_result(&mut self, num: u32, value: usize, args: &[usize; 6]) {
        match num {
            19 | 20 | 26 | 56 | 85 | 198 | 279 => self.track_fd(value),
            23 | 24 | 25 | 242 => self.track_fd(value),
            59 | 199 => {
                if args[0] != 0 {
                    let ptr = args[0] as *const i32;
                    unsafe {
                        self.track_fd(core::ptr::read(ptr) as usize);
                        self.track_fd(core::ptr::read(ptr.add(1)) as usize);
                    }
                }
            }
            186 | 190 | 194 => self.track_handle(value as u32),
            196 | 222 => self.track_mapping(value),
            _ => {}
        }
    }

    fn capture_zircon_result(&mut self, num: u32, value: usize, _args: &[usize; 8]) {
        match num {
            5 | 8 | 9 | 18 | 34 | 43 | 47 | 49 | 51 | 52 | 53 | 61 | 65 | 68 | 74 | 77 | 87
            | 88 | 98 | 106 | 107 | 108 | 109 | 115 | 121 | 122 | 129 | 132 | 140 | 141 | 187
            | 197 | 202 | 203 | 209 | 210 => self.track_handle(value as u32),
            20 | 27 | 54 | 84 | 130 => self.track_handle_pair(value),
            39 => {
                self.track_handle(value as u32);
                self.track_handle(unsafe { core::ptr::read(self.arena.handles.as_ptr().add(1)) });
            }
            79 => self.track_mapping(value),
            _ => {}
        }
    }

    fn cleanup(&mut self) {
        for index in 0..self.mapping_count {
            let _ = sys_munmap(self.mappings[index], PAGE_SIZE * 2);
        }
        self.mapping_count = 0;

        for index in 0..self.fd_count {
            let _ = sys_close(self.fds[index]);
        }
        self.fd_count = 0;

        for index in 0..self.handle_count {
            let _ = sys_handle_close(self.handles[index]);
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

    for round in 0..iterations {
        if state.should_stop() {
            break;
        }

        state.arena.refresh(config.seed.wrapping_add(round as u64));
        if !fuzz_linux_round(&mut state) {
            break;
        }
        if !fuzz_zircon_round(&mut state) {
            break;
        }
        state.report.completed_iterations += 1;
    }

    state.cleanup();
    state.report.elapsed_ticks = timer::get_tick_count().saturating_sub(state.start_tick);
    state.report
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

    if let Ok(fd) = sys_openat(
        usize::MAX - 99,
        state.arena.path_ptr(),
        LINUX_O_CREAT | LINUX_O_RDWR,
        0,
    ) {
        state.track_fd(fd);
    }
    if let Ok(fd) = sys_openat(
        usize::MAX - 99,
        state.arena.path_ptr(),
        LINUX_O_DIRECTORY,
        0,
    ) {
        state.track_fd(fd);
    }
    if let Ok(fd) = sys_eventfd2(1, 0) {
        state.track_fd(fd);
    }
    if let Ok(fd) = sys_timerfd_create(1, 0) {
        state.track_fd(fd);
    }
    if let Ok(fd) = sys_socket(LINUX_AF_UNIX, LINUX_SOCK_STREAM, 0) {
        state.track_fd(fd);
    }
    let mut pair = [0i32; 2];
    if sys_pipe2(pair.as_mut_ptr() as usize, 0).is_ok() {
        state.track_fd(pair[0] as usize);
        state.track_fd(pair[1] as usize);
    }
    if sys_socketpair(
        LINUX_AF_UNIX,
        LINUX_SOCK_DGRAM,
        0,
        pair.as_mut_ptr() as usize,
    )
    .is_ok()
    {
        state.track_fd(pair[0] as usize);
        state.track_fd(pair[1] as usize);
    }

    if let Ok(handle) = sys_msgget(1, 0) {
        state.track_handle(handle as u32);
    }
    if let Ok(handle) = sys_semget(1, 1, 0) {
        state.track_handle(handle as u32);
    }
    if let Ok(handle) = sys_shmget(1, PAGE_SIZE, 0) {
        state.track_handle(handle as u32);
    }

    let mut handle = 0u32;
    if sys_vmo_create((PAGE_SIZE * 2) as u64, 1, &mut handle).is_ok() {
        state.track_handle(handle);
    }
    if sys_event_create(0, &mut handle).is_ok() {
        state.track_handle(handle);
    }
    if sys_port_create(0, &mut handle).is_ok() {
        state.track_handle(handle);
    }
    if sys_job_create(0, 0, &mut handle).is_ok() {
        state.track_handle(handle);
    }

    let mut h0 = 0u32;
    let mut h1 = 0u32;
    if sys_channel_create(0, &mut h0, &mut h1).is_ok() {
        state.track_handle(h0);
        state.track_handle(h1);
    }
    if sys_socket_create(0, &mut h0, &mut h1).is_ok() {
        state.track_handle(h0);
        state.track_handle(h1);
    }
}

fn fuzz_linux_round(state: &mut FuzzState) -> bool {
    for num in 0..=FUZZ_LINUX_MAX {
        if linux_dispatch_excluded(num) {
            state.report.skipped += 1;
            continue;
        }
        let variants = linux_variants(num);
        for variant in 0..variants {
            if state.should_stop() {
                return false;
            }
            let args = linux_args(state, num, variant);
            state.call_linux(num, args);
        }
    }

    for num in [600u32] {
        if state.should_stop() {
            return false;
        }
        let args = linux_args(state, num, 0);
        state.call_linux(num, args);
    }

    true
}

fn fuzz_zircon_round(state: &mut FuzzState) -> bool {
    for num in 0..=FUZZ_ZIRCON_MAX {
        if zircon_dispatch_excluded(num) {
            state.report.skipped += 1;
            continue;
        }
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

fn linux_dispatch_excluded(num: u32) -> bool {
    matches!(
        num,
        93 | 94 | 129 | 130 | 131 | 170 | 171 | 220 | 224 | 225 | 424 | 435
    )
}

fn zircon_dispatch_excluded(num: u32) -> bool {
    matches!(
        num,
        6 | 7 | 9 | 33 | 38 | 46 | 50 | 78 | 80 | 98 | 101 | 112 | 137 | 200 | 201 | 207 | 208
    )
}

fn linux_variants(num: u32) -> usize {
    match num {
        23 | 24 | 25 | 56 | 57 | 59 | 63 | 64 | 65 | 66 | 72 | 73 | 78 | 79 | 80 | 85 | 86 | 87
        | 88 | 95 | 98 | 99 | 100 | 101 | 102 | 103 | 107 | 108 | 110 | 111 | 112 | 113 | 114
        | 115 | 132 | 134 | 135 | 136 | 137 | 138 | 140 | 141 | 142 | 161 | 162 | 163 | 164
        | 165 | 168 | 169 | 179 | 187 | 188 | 189 | 191 | 192 | 193 | 195 | 196 | 197 | 198
        | 199 | 202 | 204 | 205 | 206 | 207 | 208 | 209 | 211 | 212 | 214 | 215 | 216 | 220
        | 221 | 222 | 226 | 232 | 240 | 242 | 243 | 260 | 261 | 276 | 277 | 278 | 279 | 283
        | 285 | 286 | 287 | 291 | 435 | 436 | 437 | 439 | 441 => 2,
        _ => 1,
    }
}

fn zircon_variants(num: u32) -> usize {
    match num {
        0 | 1 | 5 | 8 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 20 | 21 | 22 | 23 | 24 | 25
        | 27 | 28 | 29 | 34 | 35 | 36 | 37 | 39 | 40 | 41 | 42 | 43 | 44 | 45 | 47 | 49 | 51
        | 52 | 53 | 54 | 55 | 56 | 57 | 58 | 59 | 60 | 61 | 62 | 63 | 64 | 65 | 66 | 67 | 68
        | 69 | 70 | 71 | 72 | 73 | 74 | 75 | 76 | 77 | 79 | 81 | 82 | 83 | 84 | 85 | 86 | 87
        | 88 | 89 | 90 | 91 | 95 | 96 | 97 | 100 | 102 | 103 | 104 | 106 | 107 | 108 | 109
        | 110 | 111 | 113 | 114 | 115 | 118 | 119 | 120 | 121 | 122 | 123 | 124 | 125 | 126
        | 127 | 128 | 129 | 130 | 131 | 132 | 133 | 134 | 135 | 136 | 139 | 140 | 141 | 142
        | 143 | 183 | 187 | 188 | 189 | 190 | 191 | 192 | 197 | 198 | 199 | 202 | 203 | 204
        | 205 | 206 | 209 | 210 | 211 => 2,
        _ => 1,
    }
}

fn linux_args(state: &mut FuzzState, num: u32, variant: usize) -> [usize; 6] {
    let len = state.byte_len();
    let ptr = state.user_ptr(len.max(1));
    let fd = state.fd();
    let path = state.cstr_ptr();
    let path2 = if variant == 0 {
        state.arena.path_alt_ptr()
    } else {
        0
    };
    let mut out = [
        state.rng.next_usize(),
        state.rng.next_usize(),
        state.rng.next_usize(),
        state.rng.next_usize(),
        state.rng.next_usize(),
        state.rng.next_usize(),
    ];

    match num {
        5..=16 => out = [path, state.arena.name_ptr(), ptr, len, 0, 0],
        17 => out = [ptr, len.max(2), 0, 0, 0, 0],
        19 => out = [state.rng.below(4), 0, 0, 0, 0, 0],
        20 => out = [0, 0, 0, 0, 0, 0],
        21 => out = [fd, variant, state.fd(), ptr, 0, 0],
        22 | 441 => out = [fd, ptr, state.small_count(), 0, 0, 0],
        23 => out = [fd, 0, 0, 0, 0, 0],
        24 => {
            out = [
                fd,
                state.rng.below(16) + 3,
                variant * LINUX_O_CLOEXEC,
                0,
                0,
                0,
            ]
        }
        25 => out = [fd, if variant == 0 { 3 } else { 4 }, 0, 0, 0, 0],
        26 => out = [0, 0, 0, 0, 0, 0],
        27 => out = [fd, path, 0xffff, 0, 0, 0],
        28 => out = [fd, 1, 0, 0, 0, 0],
        29 => out = [fd, state.rng.below(8), ptr, len, 0, 0],
        32 => out = [fd, variant, 0, 0, 0, 0],
        33 | 34 | 35 | 36 | 37 | 38 | 45 | 48 | 49 | 51 | 53 | 54 | 56 | 78 | 79 | 88 | 291
        | 437 | 439 => out = linux_path_args(state, num, variant, fd, path, path2, ptr, len),
        39 | 40 | 41 => out = [path, path2, state.arena.name_ptr(), variant, ptr, 0],
        43 | 44 => out = [path, ptr, 0, 0, 0, 0],
        50 | 52 | 55 | 57 | 62 | 71 | 82 | 83 | 84 | 85 | 86 | 87 => {
            out = [fd, ptr, len, variant, ptr, len]
        }
        46 | 47 => out = [fd, 0, 0, FUZZ_IO_BYTES, 0, 0],
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
        61 | 63 => out = [fd, ptr, len, 0, 0, 0],
        64 => out = [state.tracked_fd(), ptr, len, 0, 0, 0],
        65 => out = [fd, state.arena.iov_ptr(), 2, 0, 0, 0],
        66 => out = [state.tracked_fd(), state.arena.iov_ptr(), 2, 0, 0, 0],
        69 | 286 => out = [fd, state.arena.iov_ptr(), 2, 0, 0, 0],
        70 | 287 => out = [state.tracked_fd(), state.arena.iov_ptr(), 2, 0, 0, 0],
        67 => out = [fd, ptr, len, 0, 0, 0],
        68 => out = [state.tracked_fd(), ptr, len, 0, 0, 0],
        72 | 73 => {
            out = [
                fd,
                state.arena.poll_ptr(),
                2,
                state.arena.timespec_ptr(),
                0,
                0,
            ]
        }
        74 => out = [fd, ptr, len, variant, 0, 0],
        75 => out = [state.tracked_fd(), state.arena.iov_ptr(), 2, 0, 0, 0],
        76 => out = [fd, 0, state.tracked_fd(), 0, FUZZ_IO_BYTES, 0],
        77 => out = [fd, state.tracked_fd(), FUZZ_IO_BYTES, 0, 0, 0],
        80 => out = [fd, ptr, 0, 0, 0, 0],
        81 | 124 | 127 | 139 | 155 | 172 | 173 | 174 | 175 | 176 | 177 | 178 => {
            out = [0, 0, 0, 0, 0, 0]
        }
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
        101 | 115 => out = [state.arena.timespec_ptr(), 0, 0, 0, 0, 0],
        102 | 103 => out = [0, state.arena.itimer_ptr(), ptr, 0, 0, 0],
        107 | 108 | 110 => out = [1, 0, state.arena.words_ptr(), 0, state.arena.words_ptr(), 0],
        111 => out = [fd, 0, 0, 0, 0, 0],
        112 | 113 | 114 => out = [1, state.arena.timespec_ptr(), 0, 0, 0, 0],
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
        138 | 240 => out = [1, variant + 1, ptr, 0, 0, 0],
        140 | 141 | 142 | 143 | 144 | 145 | 147 | 149 | 151 | 152 | 154 | 156 | 157 | 159 | 164 => {
            out = [variant, path, len, ptr, 0, 0]
        }
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
        187 | 195 => out = [state.handle() as usize, variant, ptr, 0, 0, 0],
        188 | 189 => out = [state.handle() as usize, ptr, FUZZ_IO_BYTES, 0, 0, 0],
        190 => out = [1, 1, 0, 0, 0, 0],
        191 | 192 | 193 => out = [state.handle() as usize, 0, variant, ptr, 0, 0],
        194 => out = [1, PAGE_SIZE, 0, 0, 0, 0],
        196 | 197 => out = [state.handle() as usize, state.mapping(), 0, 0, 0, 0],
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
        200 | 203 => out = [fd, ptr, len, 0, 0, 0],
        201 | 210 => out = [fd, variant, 0, 0, 0, 0],
        204 | 205 => out = [fd, ptr, state.arena.u32_out_ptr(16), 0, 0, 0],
        208 => out = [fd, 0, 0, ptr, len, 0],
        209 => out = [fd, 0, 0, ptr, state.arena.u32_out_ptr(4), 0],
        202 | 242 => out = [fd, ptr, state.arena.words_ptr(), 0, 0, 0],
        206 => out = [fd, ptr, len, 0, ptr, 16],
        207 => out = [fd, ptr, len, 0, ptr, state.arena.u32_out_ptr(16)],
        211 | 212 | 243 => out = [fd, ptr, 1, 0, state.arena.timespec_ptr(), 0],
        213 | 223 | 227 | 228 | 229 | 233 | 234 => {
            out = [state.mapping(), PAGE_SIZE, variant, 0, 0, 0]
        }
        232 => out = [state.mapping(), PAGE_SIZE, ptr, 0, 0, 0],
        214 => out = [if variant == 0 { 0 } else { 0x4000_1000 }, 0, 0, 0, 0, 0],
        215 => out = [state.mapping(), PAGE_SIZE, 0, 0, 0, 0],
        216 => out = [state.mapping(), PAGE_SIZE, PAGE_SIZE * 2, 1, 0, 0],
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
                state.tracked_fd(),
                0,
                state.tracked_fd(),
                0,
                FUZZ_IO_BYTES,
                0,
            ]
        }
        268 => out = [fd, LINUX_CONTAINER_NAMESPACE_FLAGS, 0, 0, 0, 0],
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
        278 => out = [ptr, len, variant, 0, 0, 0],
        279 => out = [state.arena.name_ptr(), variant & 0x7, 0, 0, 0, 0],
        435 => out = [state.arena.words_ptr(), 64, 0, 0, 0, 0],
        436 => out = [3, 16, 0, 0, 0, 0],
        600 | 1000 | 1001 => out = [0, 0, 0, 0, 0, 0],
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
    len: usize,
) -> [usize; 6] {
    match num {
        35 => [fd, path, variant * 0x200, 0, 0, 0],
        36 => [path, fd, path2, 0, 0, 0],
        37 => [fd, path, fd, path2, 0, 0],
        38 => [fd, path, fd, path2, 0, 0],
        45 => [path, len, 0, 0, 0, 0],
        48 | 439 => [fd, path, 0, 0, 0, 0],
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
        78 => [fd, path, ptr, len, 0, 0],
        79 => [fd, path, ptr, 0, 0, 0],
        88 => [fd, path, state.arena.timespec_ptr(), 0, 0, 0],
        291 => [fd, path, 0, 0x7ff, ptr, 0],
        437 => [fd, path, ptr, len, 0, 0],
        _ => [fd, path, variant, ptr, len, 0],
    }
}

fn zircon_args(state: &mut FuzzState, num: u32, variant: usize) -> [usize; 8] {
    let handle = state.handle();
    let len = state.byte_len();
    let ptr = state.user_ptr(len.max(1));
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
        5 | 202 => out = [0, variant as usize, 0, 0, 0, 0, 0, 0],
        8 | 9 => {
            out = [
                handle as usize,
                RIGHT_SAME_RIGHTS as usize,
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        10 => out = [handle as usize, 0, 0, 0, 0, 0, 0, 0],
        11 => {
            let wait_handle = state.handle();
            state.arena.wait_items[0].handle = wait_handle;
            state.arena.wait_items[0].waitfor = 0;
            out = [state.arena.wait_items_ptr(), 1, 0, 0, 0, 0, 0, 0];
        }
        12 => out = [handle as usize, state.handle() as usize, 1, 0, 0, 0, 0, 0],
        13 | 14 => out = [handle as usize, 0, 1 << 24, 0, 0, 0, 0, 0],
        15 | 16 | 17 => out = [handle as usize, variant, ptr, len.max(8), 0, 0, 0, 0],
        18 => {
            out = [
                handle as usize,
                0,
                RIGHT_SAME_RIGHTS as usize,
                0,
                0,
                0,
                0,
                0,
            ]
        }
        19 => out = [handle as usize, 0, state.handle() as usize, 0, 0, 0, 0, 0],
        20 | 27 | 53 | 54 | 61 | 65 | 68 | 87 | 88 | 98 | 108 | 140 | 197 => {
            out = [variant, 0, 0, 0, 0, 0, 0, 0]
        }
        21 | 22 => {
            out = [
                handle as usize,
                0,
                ptr,
                len,
                state.arena.handles_ptr(),
                0,
                0,
                0,
            ]
        }
        23 | 24 => out = [handle as usize, 0, ptr, len, 0, 0, 0, 0],
        25 => out = [handle as usize, 0, ptr, len, 0, ptr, len, 0],
        26 | 144..=153 | 154 => out = [0, 0, 0, 0, 0, 0, 0, 0],
        28 | 29 | 82 | 83 | 90 | 91 | 95 | 96 | 97 => {
            out = [handle as usize, 0, ptr, len, 0, 0, 0, 0]
        }
        89 => out = [handle as usize, 0, ptr, 0, 0, 0, 0, 0],
        30 | 31 | 32 => out = [handle as usize, state.handle() as usize, 0, 0, 0, 0, 0, 0],
        34 => {
            out = [
                handle as usize,
                state.arena.name_ptr(),
                10,
                0x1000,
                0,
                0,
                0,
                0,
            ]
        }
        35 => out = [handle as usize, 0x1000, 0, 0, 0, 0, 0, 0],
        36 | 37 => out = [handle as usize, 0, ptr, len.max(8), 0, 0, 0, 0],
        39 => {
            state.arena.handles[1] = 0;
            out = [
                handle as usize,
                state.arena.name_ptr(),
                10,
                0,
                state.arena.handles_ptr(),
                unsafe { state.arena.handles.as_mut_ptr().add(1) as usize },
                0,
                0,
            ]
        }
        40 => out = [handle as usize, handle as usize, 0, 0, 0, 0, 0, 0],
        41 | 42 => out = [handle as usize, 0, ptr, len, 0, 0, 0, 0],
        43 => out = [handle as usize, 0, 0, 0, 0, 0, 0, 0],
        44 => out = [handle as usize, 0, 0, ptr, len, 0, 0, 0],
        45 | 49 | 203 => out = [handle as usize, 0, 0, 0, 0, 0, 0, 0],
        47 => out = [handle as usize, 0, 0, 0, 0, 0, 0, 0],
        48 => out = [handle as usize, handle as usize, 0, 0, 0, 0, 0, 0],
        51 | 52 | 209 | 210 => out = [handle as usize, 0, 0, 0, 0, 0, 0, 0],
        55 => {
            out = [
                state.arena.futex_ptr(),
                state.arena.futex as usize,
                0,
                0,
                0,
                0,
                0,
                0,
            ]
        }
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
        62 => out = [handle as usize, state.arena.packet_ptr(), 0, 0, 0, 0, 0, 0],
        63 => out = [handle as usize, 0, state.arena.packet_ptr(), 0, 0, 0, 0, 0],
        64 => out = [handle as usize, state.handle() as usize, 0, 0, 0, 0, 0, 0],
        66 | 67 => out = [handle as usize, 0, 0, 0, 0, 0, 0, 0],
        69 | 70 => out = [handle as usize, ptr, len, 0, 0, 0, 0, 0],
        71 | 72 => out = [handle as usize, PAGE_SIZE, 0, 0, 0, 0, 0, 0],
        73 => out = [handle as usize, variant, 0, PAGE_SIZE, 0, 0, 0, 0],
        74 => out = [handle as usize, 0, 0, PAGE_SIZE, 0, 0, 0, 0],
        75 | 76 => out = [handle as usize, variant, 0, 0, 0, 0, 0, 0],
        77 => {
            out = [
                memory_root_vmar_handle() as usize,
                0,
                0,
                PAGE_SIZE,
                0,
                0,
                0,
                0,
            ]
        }
        79 => {
            out = [
                memory_root_vmar_handle() as usize,
                Rights::Read as usize | Rights::Write as usize,
                0,
                handle as usize,
                0,
                PAGE_SIZE,
                0,
                0,
            ]
        }
        81 => {
            out = [
                memory_root_vmar_handle() as usize,
                1,
                state.mapping(),
                PAGE_SIZE,
                0,
                0,
                0,
                0,
            ]
        }
        84 => out = [4, 4, 0, 0, 0, 0, 0, 0],
        85 | 86 => out = [handle as usize, 4, ptr, state.small_count(), 0, 0, 0, 0],
        92 | 93 | 94 => out = [handle as usize, 0, 0, ptr, 0, 0, 0, 0],
        100 | 206 => out = [handle as usize, state.arena.words_ptr(), 0, 0, 0, 0, 0, 0],
        102 | 208 => out = [handle as usize, 0, 0, 0, 0, 0, 0, 0],
        103 | 207 => out = [handle as usize, 0, 0, 0, 0, 0, 0, 0],
        104 => out = [handle as usize, state.handle() as usize, 0, 0, 0, 0, 0, 0],
        106 => out = [handle as usize, PAGE_SIZE, 1, 0, 0, 0, 0, 0],
        107 => out = [handle as usize, 0, PAGE_SIZE, 0, 0, 0, 0, 0],
        109 => out = [handle as usize, 0, 0, 0, 0, 0, 0, 0],
        110 => out = [handle as usize, 0, handle as usize, 0, PAGE_SIZE, ptr, 1, 0],
        111 | 204 => out = [handle as usize, 0, 0, 0, 0, 0, 0, 0],
        113 => out = [ptr, 0, 0, 0, 0, 0, 0, 0],
        114 => out = [handle as usize, PAGE_SIZE, 0, 0, 0, 0, 0, 0],
        115 | 121 => out = [handle as usize, variant, ptr, 0, 0, 0, 0, 0],
        116 | 117 | 118 | 119 | 123 | 124 | 125 => {
            out = [handle as usize, variant, ptr, 0, 0, 0, 0, 0]
        }
        120 | 126 => out = [handle as usize, 0, 0, 0, 0, 0, 0, 0],
        122 => out = [handle as usize, 0, 0, 0, 0, 0, 0, 0],
        127 => out = [handle as usize, ptr, len, 0, 0, 0, 0, 0],
        128 => out = [0, ptr, len, 0, 0, 0, 0, 0],
        129 => out = [handle as usize, 0, 0, len, state.arena.name_ptr(), 10, 0, 0],
        130 => out = [0, 0, 0, 0, 0, 0, 0, 0],
        131 => {
            out = [
                handle as usize,
                0,
                0,
                PAGE_SIZE as usize,
                handle as usize,
                1,
                0,
                0,
            ]
        }
        132 => out = [handle as usize, 0, 0, 0, 0, 0, 0, 0],
        133 => out = [handle as usize, ptr, 0, 0, 0, 0, 0, 0],
        134 => out = [handle as usize, variant, 0, 0, 0, 0, 0, 0],
        135 | 136 => out = [handle as usize, 0, ptr, len.max(24), 0, 0, 0, 0],
        139 => out = [handle as usize, 0, ptr, 0, 0, 0, 0, 0],
        141 => out = [handle as usize, 0, handle as usize, 0, PAGE_SIZE, 0, 0, 0],
        142 => out = [handle as usize, handle as usize, 0, 0, 0, 0, 0, 0],
        143 => {
            out = [
                handle as usize,
                handle as usize,
                0,
                PAGE_SIZE,
                handle as usize,
                0,
                0,
                0,
            ]
        }
        183 => out = [handle as usize, 0, 0, 0, 0, 0, 0, 0],
        187 => out = [0, handle as usize, 0, 0, 0, 0, 0, 0],
        188 | 190 => out = [handle as usize, 0, state.arena.iov_ptr(), 2, ptr, 0, 0, 0],
        189 | 191 => out = [handle as usize, 0, 0, state.arena.iov_ptr(), 2, ptr, 0, 0],
        192 => out = [handle as usize, 0, 0, 0, 0, 0, 0, 0],
        198 => out = [handle as usize, state.arena.words_ptr(), 0, 0, 0, 0, 0, 0],
        199 => out = [handle as usize, variant, ptr, 0, 0, 0, 0, 0],
        205 => out = [ptr, len, 0, 0, 0, 0, 0, 0],
        211 => out = [0, 0, 0, 0, 0, 0, 0, 0],
        _ => {
            out[0] = handle as usize;
            out[1] = ptr;
            out[2] = len;
        }
    }

    if num == 40 || num == 41 || num == 42 {
        out[0] = handle as usize;
    }
    if num == 55 {
        state.arena.futex = 0;
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
