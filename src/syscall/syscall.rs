#![allow(dead_code)]
#![allow(static_mut_refs)]
//! System Call Interface Layer
//!
//! This module provides comprehensive syscall compatibility with both Linux and Zircon APIs,
//! inspired by the grt-zcore project architecture. It bridges SMROS memory management
//! with standard syscall interfaces for process management, virtual memory, and IPC.
//!
//! # Architecture
//!
//! Based on grt-zcore design patterns:
//! - Linux syscalls: mmap, munmap, mprotect, fork, execve, wait4, etc.
//! - Zircon syscalls: VMO, VMAR, handle management, object operations, channels, etc.
//!
//! # Syscall Categories
//!
//! ## Memory Management (VM)
//! - Linux: sys_mmap, sys_munmap, sys_mprotect, sys_mremap
//! - Zircon: sys_vmo_create, sys_vmo_read, sys_vmo_write, sys_vmar_map, sys_vmar_unmap
//!
//! ## Process/Task Management
//! - Linux: sys_fork, sys_execve, sys_wait4, sys_exit, sys_getpid, sys_kill
//! - Zircon: sys_process_create, sys_thread_create, sys_task_kill, sys_process_exit
//!
//! ## Handle & Object Management (Zircon-style)
//! - Handle operations: create, close, duplicate, replace
//! - Object operations: wait, signal, get_info, get_property
//!
//! ## IPC & Communication
//! - Channels: create, read, write
//! - Sockets: create, read, write, shutdown
//! - FIFOs: create, read, write
//! - Futex: wait, wake, requeue
//!
//! ## Time & Clock
//! - Clock: get, create, read
//! - Timer: create, set, cancel
//! - Sleep: nanosleep, clock_nanosleep

use alloc::string::String;
use alloc::vec::Vec;
use core::convert::TryFrom;

use super::address_logic::{
    checked_end, fixed_linux_mmap_request_ok as shared_fixed_linux_mmap_request_ok,
    page_aligned as shared_page_aligned, range_overlaps, range_within_window,
};
use crate::kernel_lowlevel::memory::{process_manager, PageFrameAllocator, PAGE_SIZE};
use crate::kernel_objects::channel;
use crate::kernel_objects::compat;
use crate::kernel_objects::fifo;
use crate::kernel_objects::fifo_logic;
use crate::kernel_objects::futex;
use crate::kernel_objects::job::JobRecord;
use crate::kernel_objects::port;
use crate::kernel_objects::process::{ProcessRecord, ThreadRecord};
use crate::kernel_objects::right::{self, ProcessRightProfile};
use crate::kernel_objects::scheduler;
use crate::kernel_objects::socket;
use crate::kernel_objects::vmar::Vmar;
use crate::syscall::syscall_logic;
use crate::user_level::fxfs;

// Re-export kernel objects for convenience
pub use crate::kernel_objects::channel::{
    sys_channel_call_noretry, sys_channel_create, sys_channel_read, sys_channel_write,
};
pub use crate::kernel_objects::{
    pages, roundup_pages, CachePolicy, HandleValue, MmuFlags, ObjectType, Rights, VmOptions,
    VmarFlags, Vmo, VmoCloneFlags, VmoOpType, VmoType, ZxError, ZxResult, INVALID_HANDLE,
    RIGHT_SAME_RIGHTS,
};

// Simple logging macros (placeholder for real logging)
macro_rules! info {
    ($($arg:tt)*) => {
        // In a real kernel, would write to debug log
        let _ = format_args!($($arg)*);
    };
}

macro_rules! warn {
    ($($arg:tt)*) => {
        // In a real kernel, would write to warning log
        let _ = format_args!($($arg)*);
    };
}

// ============================================================================
// Constants and Types
// ============================================================================

// These are now in kernel_objects.rs and re-exported above

/// Linux syscall error codes
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysError {
    EPERM = 1,
    ENOENT = 2,
    ESRCH = 3,
    EINTR = 4,
    EIO = 5,
    ENXIO = 6,
    E2BIG = 7,
    ENOMEM = 12,
    EACCES = 13,
    EFAULT = 14,
    EBUSY = 16,
    EEXIST = 17,
    ENODEV = 19,
    EINVAL = 22,
    ENOSYS = 38,
    ENOTSOCK = 88,
}

/// Syscall result type
pub type SysResult = Result<usize, SysError>;

impl From<SysError> for usize {
    fn from(err: SysError) -> Self {
        -(err as i32) as usize
    }
}

const LINUX_MAPPING_BASE: usize = 0x5000_0000;
const LINUX_MAPPING_LIMIT: usize = 0x6000_0000;
const BRK_HEAP_START: usize = 0x4000_0000;
const BRK_HEAP_LIMIT: usize = BRK_HEAP_START + (1024 * 1024);
const ZIRCON_ROOT_VMAR_BASE: usize = 0x7000_0000;
const ZIRCON_ROOT_VMAR_SIZE: usize = 0x1000_0000;
const MEMORY_HANDLE_START: u32 = 0x1000;
const ARM64_SYS_IO_SETUP: u32 = 0;
const ARM64_SYS_IO_DESTROY: u32 = 1;
const ARM64_SYS_IO_SUBMIT: u32 = 2;
const ARM64_SYS_IO_CANCEL: u32 = 3;
const ARM64_SYS_IO_GETEVENTS: u32 = 4;
const ARM64_SYS_SETXATTR: u32 = 5;
const ARM64_SYS_LSETXATTR: u32 = 6;
const ARM64_SYS_FSETXATTR: u32 = 7;
const ARM64_SYS_GETXATTR: u32 = 8;
const ARM64_SYS_LGETXATTR: u32 = 9;
const ARM64_SYS_FGETXATTR: u32 = 10;
const ARM64_SYS_LISTXATTR: u32 = 11;
const ARM64_SYS_LLISTXATTR: u32 = 12;
const ARM64_SYS_FLISTXATTR: u32 = 13;
const ARM64_SYS_REMOVEXATTR: u32 = 14;
const ARM64_SYS_LREMOVEXATTR: u32 = 15;
const ARM64_SYS_FREMOVEXATTR: u32 = 16;
const ARM64_SYS_GETCWD: u32 = 17;
const ARM64_SYS_EVENTFD2: u32 = 19;
const ARM64_SYS_EPOLL_CREATE1: u32 = 20;
const ARM64_SYS_EPOLL_CTL: u32 = 21;
const ARM64_SYS_EPOLL_PWAIT: u32 = 22;
const ARM64_SYS_INOTIFY_INIT1: u32 = 26;
const ARM64_SYS_INOTIFY_ADD_WATCH: u32 = 27;
const ARM64_SYS_INOTIFY_RM_WATCH: u32 = 28;
const ARM64_SYS_OPENAT: u32 = 56;
const ARM64_SYS_PIPE2: u32 = 59;
const ARM64_SYS_GETDENTS64: u32 = 61;
const ARM64_SYS_WRITE: u32 = 64;
const ARM64_SYS_READ: u32 = 63;
const ARM64_SYS_CLOSE: u32 = 57;
const ARM64_SYS_DUP: u32 = 23;
const ARM64_SYS_DUP3: u32 = 24;
const ARM64_SYS_FCNTL: u32 = 25;
const ARM64_SYS_IOCTL: u32 = 29;
const ARM64_SYS_FLOCK: u32 = 32;
const ARM64_SYS_MKNODAT: u32 = 33;
const ARM64_SYS_MKDIRAT: u32 = 34;
const ARM64_SYS_UNLINKAT: u32 = 35;
const ARM64_SYS_SYMLINKAT: u32 = 36;
const ARM64_SYS_LINKAT: u32 = 37;
const ARM64_SYS_RENAMEAT: u32 = 38;
const ARM64_SYS_UMOUNT2: u32 = 39;
const ARM64_SYS_MOUNT: u32 = 40;
const ARM64_SYS_PIVOT_ROOT: u32 = 41;
const ARM64_SYS_NFSSERVCTL: u32 = 42;
const ARM64_SYS_STATFS: u32 = 43;
const ARM64_SYS_FSTATFS: u32 = 44;
const ARM64_SYS_TRUNCATE: u32 = 45;
const ARM64_SYS_FTRUNCATE: u32 = 46;
const ARM64_SYS_FALLOCATE: u32 = 47;
const ARM64_SYS_FACCESSAT: u32 = 48;
const ARM64_SYS_CHDIR: u32 = 49;
const ARM64_SYS_FCHDIR: u32 = 50;
const ARM64_SYS_CHROOT: u32 = 51;
const ARM64_SYS_FCHMOD: u32 = 52;
const ARM64_SYS_FCHMODAT: u32 = 53;
const ARM64_SYS_FCHOWNAT: u32 = 54;
const ARM64_SYS_FCHOWN: u32 = 55;
const ARM64_SYS_LSEEK: u32 = 62;
const ARM64_SYS_READV: u32 = 65;
const ARM64_SYS_WRITEV: u32 = 66;
const ARM64_SYS_PREAD64: u32 = 67;
const ARM64_SYS_PWRITE64: u32 = 68;
const ARM64_SYS_PREADV: u32 = 69;
const ARM64_SYS_PWRITEV: u32 = 70;
const ARM64_SYS_SENDFILE: u32 = 71;
const ARM64_SYS_PSELECT6: u32 = 72;
const ARM64_SYS_PPOLL: u32 = 73;
const ARM64_SYS_SIGNALFD4: u32 = 74;
const ARM64_SYS_VMSPLICE: u32 = 75;
const ARM64_SYS_SPLICE: u32 = 76;
const ARM64_SYS_TEE: u32 = 77;
const ARM64_SYS_READLINKAT: u32 = 78;
const ARM64_SYS_NEWFSTATAT: u32 = 79;
const ARM64_SYS_FSTATAT: u32 = ARM64_SYS_NEWFSTATAT;
const ARM64_SYS_FSTAT: u32 = 80;
const ARM64_SYS_SYNC: u32 = 81;
const ARM64_SYS_FSYNC: u32 = 82;
const ARM64_SYS_FDATASYNC: u32 = 83;
const ARM64_SYS_SYNC_FILE_RANGE: u32 = 84;
const ARM64_SYS_TIMERFD_CREATE: u32 = 85;
const ARM64_SYS_TIMERFD_SETTIME: u32 = 86;
const ARM64_SYS_TIMERFD_GETTIME: u32 = 87;
const ARM64_SYS_UTIMENSAT: u32 = 88;
const ARM64_SYS_EXIT: u32 = 93;
const ARM64_SYS_EXIT_GROUP: u32 = 94;
const ARM64_SYS_WAITID: u32 = 95;
const ARM64_SYS_SET_TID_ADDRESS: u32 = 96;
const ARM64_SYS_UNSHARE: u32 = 97;
const ARM64_SYS_FUTEX: u32 = 98;
const ARM64_SYS_SET_ROBUST_LIST: u32 = 99;
const ARM64_SYS_GET_ROBUST_LIST: u32 = 100;
const ARM64_SYS_NANOSLEEP: u32 = 101;
const ARM64_SYS_GETITIMER: u32 = 102;
const ARM64_SYS_SETITIMER: u32 = 103;
const ARM64_SYS_TIMER_CREATE: u32 = 107;
const ARM64_SYS_TIMER_GETTIME: u32 = 108;
const ARM64_SYS_TIMER_GETOVERRUN: u32 = 109;
const ARM64_SYS_TIMER_SETTIME: u32 = 110;
const ARM64_SYS_TIMER_DELETE: u32 = 111;
const ARM64_SYS_CLOCK_SETTIME: u32 = 112;
const ARM64_SYS_CLOCK_GETTIME: u32 = 113;
const ARM64_SYS_CLOCK_GETRES: u32 = 114;
const ARM64_SYS_CLOCK_NANOSLEEP: u32 = 115;
const ARM64_SYS_SCHED_SETPARAM: u32 = 118;
const ARM64_SYS_SCHED_SETSCHEDULER: u32 = 119;
const ARM64_SYS_SCHED_GETSCHEDULER: u32 = 120;
const ARM64_SYS_SCHED_GETPARAM: u32 = 121;
const ARM64_SYS_SCHED_SETAFFINITY: u32 = 122;
const ARM64_SYS_SCHED_GETAFFINITY: u32 = 123;
const ARM64_SYS_SCHED_YIELD: u32 = 124;
const ARM64_SYS_SCHED_GET_PRIORITY_MAX: u32 = 125;
const ARM64_SYS_SCHED_GET_PRIORITY_MIN: u32 = 126;
const ARM64_SYS_SCHED_RR_GET_INTERVAL: u32 = 127;
const ARM64_SYS_KILL: u32 = 129;
const ARM64_SYS_TKILL: u32 = 130;
const ARM64_SYS_TGKILL: u32 = 131;
const ARM64_SYS_SIGALTSTACK: u32 = 132;
const ARM64_SYS_RT_SIGSUSPEND: u32 = 133;
const ARM64_SYS_RT_SIGACTION: u32 = 134;
const ARM64_SYS_RT_SIGPROCMASK: u32 = 135;
const ARM64_SYS_RT_SIGPENDING: u32 = 136;
const ARM64_SYS_RT_SIGTIMEDWAIT: u32 = 137;
const ARM64_SYS_RT_SIGQUEUEINFO: u32 = 138;
const ARM64_SYS_RT_SIGRETURN: u32 = 139;
const ARM64_SYS_SETPRIORITY: u32 = 140;
const ARM64_SYS_GETPRIORITY: u32 = 141;
const ARM64_SYS_SETREGID: u32 = 143;
const ARM64_SYS_SETGID: u32 = 144;
const ARM64_SYS_SETREUID: u32 = 145;
const ARM64_SYS_SETUID: u32 = 146;
const ARM64_SYS_SETRESUID: u32 = 147;
const ARM64_SYS_GETRESUID: u32 = 148;
const ARM64_SYS_SETRESGID: u32 = 149;
const ARM64_SYS_GETRESGID: u32 = 150;
const ARM64_SYS_SETFSUID: u32 = 151;
const ARM64_SYS_SETFSGID: u32 = 152;
const ARM64_SYS_TIMES: u32 = 153;
const ARM64_SYS_SETPGID: u32 = 154;
const ARM64_SYS_GETPGID: u32 = 155;
const ARM64_SYS_GETSID: u32 = 156;
const ARM64_SYS_SETSID: u32 = 157;
const ARM64_SYS_GETGROUPS: u32 = 158;
const ARM64_SYS_SETGROUPS: u32 = 159;
const ARM64_SYS_UNAME: u32 = 160;
const ARM64_SYS_SETHOSTNAME: u32 = 161;
const ARM64_SYS_SETDOMAINNAME: u32 = 162;
const ARM64_SYS_GETRLIMIT: u32 = 163;
const ARM64_SYS_SETRLIMIT: u32 = 164;
const ARM64_SYS_GETRUSAGE: u32 = 165;
const ARM64_SYS_UMASK: u32 = 166;
const ARM64_SYS_PRCTL: u32 = 167;
const ARM64_SYS_GETCPU: u32 = 168;
const ARM64_SYS_GETTIMEOFDAY: u32 = 169;
const ARM64_SYS_GETPID: u32 = 172;
const ARM64_SYS_GETPPID: u32 = 173;
const ARM64_SYS_GETUID: u32 = 174;
const ARM64_SYS_GETEUID: u32 = 175;
const ARM64_SYS_GETGID: u32 = 176;
const ARM64_SYS_GETEGID: u32 = 177;
const ARM64_SYS_GETTID: u32 = 178;
const ARM64_SYS_SYSINFO: u32 = 179;
const ARM64_SYS_MSGGET: u32 = 186;
const ARM64_SYS_MSGCTL: u32 = 187;
const ARM64_SYS_MSGRCV: u32 = 188;
const ARM64_SYS_MSGSND: u32 = 189;
const ARM64_SYS_SEMGET: u32 = 190;
const ARM64_SYS_SEMCTL: u32 = 191;
const ARM64_SYS_SEMTIMEDOP: u32 = 192;
const ARM64_SYS_SEMOP: u32 = 193;
const ARM64_SYS_SHMGET: u32 = 194;
const ARM64_SYS_SHMCTL: u32 = 195;
const ARM64_SYS_SHMAT: u32 = 196;
const ARM64_SYS_SHMDT: u32 = 197;
const ARM64_SYS_SOCKET: u32 = 198;
const ARM64_SYS_SOCKETPAIR: u32 = 199;
const ARM64_SYS_BIND: u32 = 200;
const ARM64_SYS_LISTEN: u32 = 201;
const ARM64_SYS_ACCEPT: u32 = 202;
const ARM64_SYS_CONNECT: u32 = 203;
const ARM64_SYS_GETSOCKNAME: u32 = 204;
const ARM64_SYS_GETPEERNAME: u32 = 205;
const ARM64_SYS_SENDTO: u32 = 206;
const ARM64_SYS_RECVFROM: u32 = 207;
const ARM64_SYS_SETSOCKOPT: u32 = 208;
const ARM64_SYS_GETSOCKOPT: u32 = 209;
const ARM64_SYS_SHUTDOWN: u32 = 210;
const ARM64_SYS_SENDMSG: u32 = 211;
const ARM64_SYS_RECVMSG: u32 = 212;
const ARM64_SYS_READAHEAD: u32 = 213;
const ARM64_SYS_BRK: u32 = 214;
const ARM64_SYS_MUNMAP: u32 = 215;
const ARM64_SYS_MREMAP: u32 = 216;
const ARM64_SYS_CLONE: u32 = 220;
const ARM64_SYS_EXECVE: u32 = 221;
const ARM64_SYS_MMAP: u32 = 222;
const ARM64_SYS_FADVISE64: u32 = 223;
const ARM64_SYS_MPROTECT: u32 = 226;
const ARM64_SYS_MSYNC: u32 = 227;
const ARM64_SYS_MLOCK: u32 = 228;
const ARM64_SYS_MUNLOCK: u32 = 229;
const ARM64_SYS_MLOCKALL: u32 = 230;
const ARM64_SYS_MUNLOCKALL: u32 = 231;
const ARM64_SYS_MINCORE: u32 = 232;
const ARM64_SYS_MADVISE: u32 = 233;
const ARM64_SYS_REMAP_FILE_PAGES: u32 = 234;
const ARM64_SYS_RT_TGSIGQUEUEINFO: u32 = 240;
const ARM64_SYS_ACCEPT4: u32 = 242;
const ARM64_SYS_RECVMMSG: u32 = 243;
const ARM64_SYS_WAIT4: u32 = 260;
const ARM64_SYS_PRLIMIT64: u32 = 261;
const ARM64_SYS_GETRANDOM: u32 = 278;
const ARM64_SYS_MEMFD_CREATE: u32 = 279;
const ARM64_SYS_MEMBARRIER: u32 = 283;
const ARM64_SYS_COPY_FILE_RANGE: u32 = 285;
const ARM64_SYS_PREADV2: u32 = 286;
const ARM64_SYS_PWRITEV2: u32 = 287;
const ARM64_SYS_STATX: u32 = 291;
const ARM64_SYS_CLOSE_RANGE: u32 = 436;
const ARM64_SYS_OPENAT2: u32 = 437;
const ARM64_SYS_FACCESSAT2: u32 = 439;
const ARM64_SYS_EPOLL_PWAIT2: u32 = 441;
const ARM64_SYS_LOOKUP_DCOOKIE: u32 = 18;
const ARM64_SYS_IOPRIO_SET: u32 = 30;
const ARM64_SYS_IOPRIO_GET: u32 = 31;
const ARM64_SYS_VHANGUP: u32 = 58;
const ARM64_SYS_QUOTACTL: u32 = 60;
const ARM64_SYS_ACCT: u32 = 89;
const ARM64_SYS_CAPGET: u32 = 90;
const ARM64_SYS_CAPSET: u32 = 91;
const ARM64_SYS_PERSONALITY: u32 = 92;
const ARM64_SYS_KEXEC_LOAD: u32 = 104;
const ARM64_SYS_INIT_MODULE: u32 = 105;
const ARM64_SYS_DELETE_MODULE: u32 = 106;
const ARM64_SYS_SYSLOG: u32 = 116;
const ARM64_SYS_PTRACE: u32 = 117;
const ARM64_SYS_RESTART_SYSCALL: u32 = 128;
const ARM64_SYS_REBOOT: u32 = 142;
const ARM64_SYS_SETTIMEOFDAY: u32 = 170;
const ARM64_SYS_ADJTIMEX: u32 = 171;
const ARM64_SYS_MQ_OPEN: u32 = 180;
const ARM64_SYS_MQ_UNLINK: u32 = 181;
const ARM64_SYS_MQ_TIMEDSEND: u32 = 182;
const ARM64_SYS_MQ_TIMEDRECEIVE: u32 = 183;
const ARM64_SYS_MQ_NOTIFY: u32 = 184;
const ARM64_SYS_MQ_GETSETATTR: u32 = 185;
const ARM64_SYS_ADD_KEY: u32 = 217;
const ARM64_SYS_REQUEST_KEY: u32 = 218;
const ARM64_SYS_KEYCTL: u32 = 219;
const ARM64_SYS_SWAPON: u32 = 224;
const ARM64_SYS_SWAPOFF: u32 = 225;
const ARM64_SYS_MBIND: u32 = 235;
const ARM64_SYS_GET_MEMPOLICY: u32 = 236;
const ARM64_SYS_SET_MEMPOLICY: u32 = 237;
const ARM64_SYS_MIGRATE_PAGES: u32 = 238;
const ARM64_SYS_MOVE_PAGES: u32 = 239;
const ARM64_SYS_PERF_EVENT_OPEN: u32 = 241;
const ARM64_SYS_FANOTIFY_INIT: u32 = 262;
const ARM64_SYS_FANOTIFY_MARK: u32 = 263;
const ARM64_SYS_NAME_TO_HANDLE_AT: u32 = 264;
const ARM64_SYS_OPEN_BY_HANDLE_AT: u32 = 265;
const ARM64_SYS_CLOCK_ADJTIME: u32 = 266;
const ARM64_SYS_SYNCFS: u32 = 267;
const ARM64_SYS_SETNS: u32 = 268;
const ARM64_SYS_SENDMMSG: u32 = 269;
const ARM64_SYS_PROCESS_VM_READV: u32 = 270;
const ARM64_SYS_PROCESS_VM_WRITEV: u32 = 271;
const ARM64_SYS_KCMP: u32 = 272;
const ARM64_SYS_FINIT_MODULE: u32 = 273;
const ARM64_SYS_SCHED_SETATTR: u32 = 274;
const ARM64_SYS_SCHED_GETATTR: u32 = 275;
const ARM64_SYS_RENAMEAT2: u32 = 276;
const ARM64_SYS_SECCOMP: u32 = 277;
const ARM64_SYS_BPF: u32 = 280;
const ARM64_SYS_EXECVEAT: u32 = 281;
const ARM64_SYS_USERFAULTFD: u32 = 282;
const ARM64_SYS_MLOCK2: u32 = 284;
const ARM64_SYS_PKEY_MPROTECT: u32 = 288;
const ARM64_SYS_PKEY_ALLOC: u32 = 289;
const ARM64_SYS_PKEY_FREE: u32 = 290;
const ARM64_SYS_IO_PGETEVENTS: u32 = 292;
const ARM64_SYS_RSEQ: u32 = 293;
const ARM64_SYS_KEXEC_FILE_LOAD: u32 = 294;
const ARM64_SYS_PIDFD_SEND_SIGNAL: u32 = 424;
const ARM64_SYS_IO_URING_SETUP: u32 = 425;
const ARM64_SYS_IO_URING_ENTER: u32 = 426;
const ARM64_SYS_IO_URING_REGISTER: u32 = 427;
const ARM64_SYS_OPEN_TREE: u32 = 428;
const ARM64_SYS_MOVE_MOUNT: u32 = 429;
const ARM64_SYS_FSOPEN: u32 = 430;
const ARM64_SYS_FSCONFIG: u32 = 431;
const ARM64_SYS_FSMOUNT: u32 = 432;
const ARM64_SYS_FSPICK: u32 = 433;
const ARM64_SYS_PIDFD_OPEN: u32 = 434;
const ARM64_SYS_CLONE3: u32 = 435;
const ARM64_SYS_PIDFD_GETFD: u32 = 438;
const ARM64_SYS_PROCESS_MADVISE: u32 = 440;
const ARM64_SYS_MOUNT_SETATTR: u32 = 442;
const ARM64_SYS_LANDLOCK_CREATE_RULESET: u32 = 444;
const ARM64_SYS_LANDLOCK_ADD_RULE: u32 = 445;
const ARM64_SYS_LANDLOCK_RESTRICT_SELF: u32 = 446;
const ZX_SIGNAL_TERMINATED: u32 = 1 << 3;
const ZX_USER_SIGNAL_0: u32 = 1 << 24;
const ZX_TIMER_SIGNALED: u32 = 1 << 7;
const CLOCK_REALTIME: usize = 0;
const CLOCK_MONOTONIC: usize = 1;
const ZX_CLOCK_OPT_AUTO_START: u32 = 1 << 0;
const ZX_CLOCK_UPDATE_OPTION_SYNTHETIC_VALUE_VALID: u64 = 1 << 0;
const ZX_CLOCK_UPDATE_OPTION_REFERENCE_VALUE_VALID: u64 = 1 << 1;
const ZX_CLOCK_UPDATE_OPTIONS_MASK: u64 =
    ZX_CLOCK_UPDATE_OPTION_SYNTHETIC_VALUE_VALID | ZX_CLOCK_UPDATE_OPTION_REFERENCE_VALUE_VALID;
const ZX_TIMER_OPTIONS_MASK: u32 = 0;
const ZX_DEBUGLOG_CREATE_OPTIONS_MASK: u32 = 0;
const ZX_DEBUGLOG_OPTIONS_MASK: u32 = 0;
const ZX_SYSTEM_EVENT_KIND_MAX: u32 = 3;
const ZX_EXCEPTION_CHANNEL_DEBUGGER: u32 = 1 << 0;
const ZX_EXCEPTION_CHANNEL_OPTIONS_MASK: u32 = ZX_EXCEPTION_CHANNEL_DEBUGGER;
const ZX_HYPERVISOR_OPTIONS_MASK: u32 = 0;
const ZX_GUEST_TRAP_BELL: u32 = 0;
const ZX_GUEST_TRAP_MEM: u32 = 1;
const ZX_GUEST_TRAP_IO: u32 = 2;
const ZX_GUEST_TRAP_KIND_MAX: u32 = ZX_GUEST_TRAP_IO;
const ZX_GUEST_PHYS_LIMIT: u64 = 0x1_0000_0000;
const ZX_VCPU_ENTRY_ALIGNMENT: u64 = 4;
const ZX_VCPU_INTERRUPT_VECTOR_MAX: u32 = 1023;
const ZX_VCPU_STATE: u32 = 0;
const ZX_VCPU_IO: u32 = 1;
const ZX_VCPU_STATE_SIZE: usize = 256;
const ZX_VCPU_IO_SIZE: usize = 24;
const ZX_VCPU_PACKET_SIZE: usize = 48;
const ZX_SMC_PARAMETERS_SIZE: usize = 64;
const ZX_SMC_RESULT_SIZE: usize = 64;
const COMPAT_FD_START: usize = 3;
const LINUX_STDIO_FD_MAX: usize = 2;
const LINUX_O_ACCMODE: usize = 0o3;
const LINUX_O_RDONLY: usize = 0;
const LINUX_O_WRONLY: usize = 1;
const LINUX_O_RDWR: usize = 2;
const LINUX_O_CREAT: usize = 0o100;
const LINUX_O_EXCL: usize = 0o200;
const LINUX_O_TRUNC: usize = 0o1000;
const LINUX_O_APPEND: usize = 0o2000;
const LINUX_O_NONBLOCK: usize = 0o4000;
const LINUX_O_LARGEFILE: usize = 0o100000;
const LINUX_O_DIRECTORY: usize = 0o200000;
const LINUX_O_NOFOLLOW: usize = 0o400000;
const LINUX_O_CLOEXEC: usize = 0o2000000;
const LINUX_OPEN_ALLOWED_FLAGS: usize = LINUX_O_ACCMODE
    | LINUX_O_CREAT
    | LINUX_O_EXCL
    | LINUX_O_TRUNC
    | LINUX_O_APPEND
    | LINUX_O_NONBLOCK
    | LINUX_O_LARGEFILE
    | LINUX_O_DIRECTORY
    | LINUX_O_NOFOLLOW
    | LINUX_O_CLOEXEC;
const LINUX_PATH_MAX_BYTES: usize = 4096;
const LINUX_PIPE_ALLOWED_FLAGS: usize = LINUX_O_CLOEXEC | LINUX_O_NONBLOCK;
const LINUX_FCNTL_STATUS_ALLOWED_FLAGS: usize = LINUX_O_APPEND | LINUX_O_NONBLOCK;
const LINUX_ACCESS_MODE_MASK: usize = 0o7;
const LINUX_AT_REMOVEDIR: usize = 0x200;
const LINUX_UNLINK_ALLOWED_FLAGS: usize = LINUX_AT_REMOVEDIR;
const LINUX_RENAME_NOREPLACE: usize = 1;
const LINUX_RENAME_EXCHANGE: usize = 2;
const LINUX_RENAME_WHITEOUT: usize = 4;
const LINUX_RENAME_ALLOWED_FLAGS: usize =
    LINUX_RENAME_NOREPLACE | LINUX_RENAME_EXCHANGE | LINUX_RENAME_WHITEOUT;
const LINUX_AT_SYMLINK_NOFOLLOW: usize = 0x100;
const LINUX_AT_EMPTY_PATH: usize = 0x1000;
const LINUX_STAT_ALLOWED_FLAGS: usize = LINUX_AT_SYMLINK_NOFOLLOW | LINUX_AT_EMPTY_PATH;
const LINUX_STATX_BASIC_STATS: usize = 0x7ff;
const LINUX_SEEK_MAX_WHENCE: usize = 5;
const LINUX_MAX_IOV: usize = 1024;
const LINUX_MAX_POLL_FDS: usize = 1024;
const LINUX_POLL_ALLOWED_EVENTS: i16 = 0x0001 | 0x0004 | 0x0008 | 0x0010 | 0x0020 | 0x0040;
const LINUX_AF_UNIX: usize = 1;
const LINUX_AF_LOCAL: usize = LINUX_AF_UNIX;
const LINUX_AF_INET: usize = 2;
const LINUX_AF_NETLINK: usize = 16;
const LINUX_AF_PACKET: usize = 17;
const LINUX_SOCK_TYPE_MASK: usize = 0xff;
const LINUX_SOCK_STREAM: usize = 1;
const LINUX_SOCK_DGRAM: usize = 2;
const LINUX_SOCK_RAW: usize = 3;
const LINUX_SOCK_NONBLOCK: usize = 0x800;
const LINUX_SOCK_CLOEXEC: usize = 0x80000;
const LINUX_SOCK_ALLOWED_FLAGS: usize =
    LINUX_SOCK_TYPE_MASK | LINUX_SOCK_NONBLOCK | LINUX_SOCK_CLOEXEC;
const LINUX_IPPROTO_IP: usize = 0;
const LINUX_IPPROTO_TCP: usize = 6;
const LINUX_IPPROTO_UDP: usize = 17;
const LINUX_MAX_SIGNAL: usize = 64;
const LINUX_SIGSET_SIZE: usize = core::mem::size_of::<u64>();
const LINUX_MAX_SEMAPHORES: usize = 256;
const LINUX_MAX_IPC_BYTES: usize = 65536;
const LINUX_MAX_MSG_BYTES: usize = 8192;
const LINUX_MEMFD_ALLOWED_FLAGS: usize = 0x0001 | 0x0002 | 0x0004;
const LINUX_GETRANDOM_ALLOWED_FLAGS: u32 = 0x0001 | 0x0002;
const LINUX_CLONE_NEWNS: usize = 0x0002_0000;
const LINUX_CLONE_NEWCGROUP: usize = 0x0200_0000;
const LINUX_CLONE_NEWUTS: usize = 0x0400_0000;
const LINUX_CLONE_NEWIPC: usize = 0x0800_0000;
const LINUX_CLONE_NEWUSER: usize = 0x1000_0000;
const LINUX_CLONE_NEWPID: usize = 0x2000_0000;
const LINUX_CLONE_NEWNET: usize = 0x4000_0000;
const LINUX_CONTAINER_NAMESPACE_FLAGS: usize = LINUX_CLONE_NEWNS
    | LINUX_CLONE_NEWCGROUP
    | LINUX_CLONE_NEWUTS
    | LINUX_CLONE_NEWIPC
    | LINUX_CLONE_NEWUSER
    | LINUX_CLONE_NEWPID
    | LINUX_CLONE_NEWNET;
const LINUX_MOUNT_ALLOWED_FLAGS: usize = 0x1
    | 0x2
    | 0x4
    | 0x8
    | 0x10
    | 0x20
    | 0x40
    | 0x80
    | 0x400
    | 0x800
    | 0x1000
    | 0x2000
    | 0x4000
    | 0x8000
    | 0x10000
    | 0x20000
    | 0x40000
    | 0x100000
    | 0x200000
    | 0x400000
    | 0x800000;
const LINUX_UMOUNT_ALLOWED_FLAGS: usize = 0x1 | 0x2 | 0x4 | 0x8;
const LINUX_SECCOMP_MODE_STRICT: usize = 1;
const LINUX_SECCOMP_MODE_FILTER: usize = 2;
const LINUX_SECCOMP_SET_MODE_STRICT: usize = 0;
const LINUX_SECCOMP_SET_MODE_FILTER: usize = 1;
const LINUX_SECCOMP_GET_ACTION_AVAIL: usize = 2;
const LINUX_SECCOMP_GET_NOTIF_SIZES: usize = 3;
const LINUX_SECCOMP_FILTER_FLAG_TSYNC: usize = 1 << 0;
const LINUX_SECCOMP_FILTER_FLAG_LOG: usize = 1 << 1;
const LINUX_SECCOMP_FILTER_FLAG_SPEC_ALLOW: usize = 1 << 2;
const LINUX_SECCOMP_FILTER_FLAG_NEW_LISTENER: usize = 1 << 3;
const LINUX_SECCOMP_FILTER_ALLOWED_FLAGS: usize = LINUX_SECCOMP_FILTER_FLAG_TSYNC
    | LINUX_SECCOMP_FILTER_FLAG_LOG
    | LINUX_SECCOMP_FILTER_FLAG_SPEC_ALLOW
    | LINUX_SECCOMP_FILTER_FLAG_NEW_LISTENER;
const LINUX_CAPABILITY_VERSION_3: u32 = 0x2008_0522;
const LINUX_CAP_LAST_CAP: u32 = 40;
const LINUX_CAP_FULL_SET: u64 = (1u64 << (LINUX_CAP_LAST_CAP + 1)) - 1;
const LINUX_MAX_MOUNTS: usize = 16;
const LINUX_UTS_NAME_MAX: usize = 64;

#[derive(Clone)]
struct LinuxMappingRecord {
    addr: usize,
    len: usize,
    prot: usize,
    flags: usize,
    pfns: Vec<u64>,
}

#[derive(Default)]
struct BrkState {
    start: usize,
    current: usize,
    limit: usize,
    pfns: Vec<u64>,
}

impl BrkState {
    fn new() -> Self {
        Self {
            start: BRK_HEAP_START,
            current: BRK_HEAP_START,
            limit: BRK_HEAP_LIMIT,
            pfns: Vec::new(),
        }
    }

    fn committed_pages(&self) -> usize {
        self.pfns.len()
    }
}

struct VmoRecord {
    handle: u32,
    vmo: Vmo,
}

struct VmarRecord {
    handle: u32,
    vmar: Vmar,
}

#[derive(Clone, Copy)]
struct KernelHandleRecord {
    handle: u32,
    object_handle: u32,
    obj_type: ObjectType,
    rights: u32,
}

struct SignalRecord {
    handle: u32,
    signals: u32,
    property_value: u64,
}

#[derive(Clone)]
struct LinuxFdRecord {
    fd: usize,
    handle: u32,
    readable: bool,
    writable: bool,
}

#[derive(Clone)]
struct LinuxFxfsFileRecord {
    handle: u32,
    cursor: fxfs::FxfsCursor,
    path: String,
}

#[derive(Clone, Copy)]
struct LinuxMountRecord {
    flags: usize,
}

#[repr(C)]
struct ZxWaitItem {
    handle: u32,
    waitfor: u32,
    pending: u32,
}

#[repr(C)]
struct LinuxTimespec {
    tv_sec: i64,
    tv_nsec: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxTimeval {
    tv_sec: i64,
    tv_usec: i64,
}

#[repr(C)]
struct LinuxTms {
    tms_utime: isize,
    tms_stime: isize,
    tms_cutime: isize,
    tms_cstime: isize,
}

#[repr(C)]
struct LinuxRusage {
    ru_utime: LinuxTimeval,
    ru_stime: LinuxTimeval,
    rest: [isize; 14],
}

#[repr(C)]
struct LinuxRlimit64 {
    rlim_cur: u64,
    rlim_max: u64,
}

#[repr(C)]
struct LinuxSysinfo {
    uptime: isize,
    loads: [usize; 3],
    totalram: usize,
    freeram: usize,
    sharedram: usize,
    bufferram: usize,
    totalswap: usize,
    freeswap: usize,
    procs: u16,
    pad: u16,
    totalhigh: usize,
    freehigh: usize,
    mem_unit: u32,
}

#[repr(C)]
struct LinuxIovec {
    base: usize,
    len: usize,
}

#[repr(C)]
struct LinuxPollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

#[repr(C)]
struct LinuxStat {
    data: [u8; 128],
}

#[repr(C)]
struct LinuxStatFs {
    f_type: i64,
    f_bsize: i64,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_fsid: (i32, i32),
    f_namelen: isize,
    f_frsize: isize,
    f_flags: isize,
    f_spare: [isize; 4],
}

#[repr(C)]
struct LinuxUtsname {
    sysname: [u8; 65],
    nodename: [u8; 65],
    release: [u8; 65],
    version: [u8; 65],
    machine: [u8; 65],
    domainname: [u8; 65],
}

#[repr(C)]
struct LinuxCapUserHeader {
    version: u32,
    pid: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxCapUserData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

#[derive(Clone, Copy, Default)]
pub struct MemorySyscallStats {
    pub linux_mapping_count: usize,
    pub linux_mapped_bytes: usize,
    pub linux_committed_pages: usize,
    pub brk_start: usize,
    pub brk_current: usize,
    pub brk_limit: usize,
    pub brk_committed_pages: usize,
    pub zircon_vmo_count: usize,
    pub zircon_vmo_bytes: usize,
    pub zircon_vmo_committed_pages: usize,
    pub zircon_vmar_count: usize,
    pub zircon_mapping_count: usize,
    pub zircon_root_vmar_handle: u32,
}

#[derive(Clone, Copy, Default)]
pub struct LinuxContainerStats {
    pub namespace_flags: usize,
    pub setns_count: usize,
    pub mount_count: usize,
    pub mount_flags: usize,
    pub pivot_rooted: bool,
    pub chrooted: bool,
    pub no_new_privs: bool,
    pub seccomp_mode: usize,
    pub seccomp_filters: usize,
    pub cap_effective: u64,
    pub cap_permitted: u64,
    pub cap_inheritable: u64,
    pub hostname_set: bool,
    pub domainname_set: bool,
}

struct MemorySyscallState {
    linux_mappings: Vec<LinuxMappingRecord>,
    next_linux_addr: usize,
    brk: BrkState,
    vmos: Vec<VmoRecord>,
    vmars: Vec<VmarRecord>,
    jobs: Vec<JobRecord>,
    processes: Vec<ProcessRecord>,
    threads: Vec<ThreadRecord>,
    handles: Vec<KernelHandleRecord>,
    signals: Vec<SignalRecord>,
    linux_fds: Vec<LinuxFdRecord>,
    linux_fxfs_files: Vec<LinuxFxfsFileRecord>,
    linux_mounts: Vec<LinuxMountRecord>,
    linux_namespace_flags: usize,
    linux_setns_count: usize,
    linux_pivot_rooted: bool,
    linux_chrooted: bool,
    linux_no_new_privs: bool,
    linux_seccomp_mode: usize,
    linux_seccomp_filters: usize,
    linux_cap_effective: u64,
    linux_cap_permitted: u64,
    linux_cap_inheritable: u64,
    linux_hostname_set: bool,
    linux_domainname_set: bool,
    next_handle: u32,
    next_fd: usize,
    root_vmar_handle: u32,
}

impl MemorySyscallState {
    fn new() -> Self {
        let root_vmar_handle = MEMORY_HANDLE_START;
        let mut root_vmar = Vmar::new(ZIRCON_ROOT_VMAR_BASE, ZIRCON_ROOT_VMAR_SIZE);
        root_vmar.handle = HandleValue(root_vmar_handle);
        let mut vmars = Vec::new();
        vmars.push(VmarRecord {
            handle: root_vmar_handle,
            vmar: root_vmar,
        });
        let mut handles = Vec::new();
        handles.push(KernelHandleRecord {
            handle: root_vmar_handle,
            object_handle: root_vmar_handle,
            obj_type: ObjectType::Vmar,
            rights: crate::kernel_objects::default_rights_for_object(ObjectType::Vmar),
        });

        Self {
            linux_mappings: Vec::new(),
            next_linux_addr: LINUX_MAPPING_BASE,
            brk: BrkState::new(),
            vmos: Vec::new(),
            vmars,
            jobs: Vec::new(),
            processes: Vec::new(),
            threads: Vec::new(),
            handles,
            signals: Vec::new(),
            linux_fds: Vec::new(),
            linux_fxfs_files: Vec::new(),
            linux_mounts: Vec::new(),
            linux_namespace_flags: 0,
            linux_setns_count: 0,
            linux_pivot_rooted: false,
            linux_chrooted: false,
            linux_no_new_privs: false,
            linux_seccomp_mode: 0,
            linux_seccomp_filters: 0,
            linux_cap_effective: LINUX_CAP_FULL_SET,
            linux_cap_permitted: LINUX_CAP_FULL_SET,
            linux_cap_inheritable: 0,
            linux_hostname_set: false,
            linux_domainname_set: false,
            next_handle: MEMORY_HANDLE_START + 1,
            next_fd: COMPAT_FD_START,
            root_vmar_handle,
        }
    }

    fn alloc_handle(&mut self) -> u32 {
        let handle = self.next_handle;
        self.next_handle = self.next_handle.wrapping_add(1);
        handle
    }

    fn register_handle(
        &mut self,
        handle: u32,
        object_handle: u32,
        obj_type: ObjectType,
        rights: u32,
    ) -> bool {
        if syscall_logic::handle_invalid(handle, INVALID_HANDLE)
            || !crate::kernel_objects::rights_are_valid(rights)
            || self.handles.iter().any(|record| record.handle == handle)
        {
            return false;
        }

        self.handles.push(KernelHandleRecord {
            handle,
            object_handle,
            obj_type,
            rights,
        });
        true
    }

    fn register_object_handle(&mut self, handle: u32, obj_type: ObjectType) -> bool {
        self.register_handle(
            handle,
            handle,
            obj_type,
            crate::kernel_objects::default_rights_for_object(obj_type),
        )
    }

    fn register_object_handle_with_rights(
        &mut self,
        handle: u32,
        obj_type: ObjectType,
        rights: u32,
    ) -> bool {
        self.register_handle(handle, handle, obj_type, rights)
    }

    fn alloc_object_handle(&mut self, obj_type: ObjectType) -> u32 {
        let handle = self.alloc_handle();
        let _ = self.register_object_handle(handle, obj_type);
        handle
    }

    fn alloc_object_handle_with_rights(
        &mut self,
        obj_type: ObjectType,
        rights: u32,
    ) -> ZxResult<u32> {
        let handle = self.alloc_handle();
        if self.register_object_handle_with_rights(handle, obj_type, rights) {
            Ok(handle)
        } else {
            Err(ZxError::ErrInvalidArgs)
        }
    }

    fn handle_record(&self, handle: u32) -> Option<KernelHandleRecord> {
        self.handles
            .iter()
            .find(|record| record.handle == handle)
            .copied()
    }

    fn handle_object_type(&self, handle: u32) -> Option<ObjectType> {
        self.handle_record(handle).map(|record| record.obj_type)
    }

    fn handle_rights(&self, handle: u32) -> Option<u32> {
        self.handle_record(handle).map(|record| record.rights)
    }

    fn handle_has_rights(&self, handle: u32, required: u32) -> bool {
        self.handle_rights(handle)
            .map(|rights| crate::kernel_objects::rights_contain(rights, required))
            .unwrap_or(false)
    }

    fn resolve_handle(&self, handle: u32, obj_type: ObjectType) -> Option<u32> {
        self.handle_record(handle)
            .filter(|record| record.obj_type == obj_type)
            .map(|record| record.object_handle)
    }

    fn duplicate_handle(&mut self, handle: u32, requested_rights: u32) -> ZxResult<u32> {
        let record = self.handle_record(handle).ok_or(ZxError::ErrNotFound)?;
        if !crate::kernel_objects::object_logic::duplicate_rights_allowed(
            record.rights,
            requested_rights,
            Rights::Duplicate as u32,
            RIGHT_SAME_RIGHTS,
            crate::kernel_objects::RIGHTS_ALL,
        ) {
            return Err(ZxError::ErrAccessDenied);
        }

        let rights = if requested_rights == RIGHT_SAME_RIGHTS {
            record.rights
        } else {
            requested_rights
        };
        let new_handle = self.alloc_handle();
        if self.register_handle(new_handle, record.object_handle, record.obj_type, rights) {
            Ok(new_handle)
        } else {
            Err(ZxError::ErrNoMemory)
        }
    }

    fn replace_handle(&mut self, handle: u32, requested_rights: u32) -> ZxResult<u32> {
        let record = self.handle_record(handle).ok_or(ZxError::ErrNotFound)?;
        if !crate::kernel_objects::object_logic::replace_rights_allowed(
            record.rights,
            requested_rights,
            RIGHT_SAME_RIGHTS,
            crate::kernel_objects::RIGHTS_ALL,
        ) {
            return Err(ZxError::ErrAccessDenied);
        }

        let rights = if requested_rights == RIGHT_SAME_RIGHTS {
            record.rights
        } else {
            requested_rights
        };
        let new_handle = self.alloc_handle();
        if !self.register_handle(new_handle, record.object_handle, record.obj_type, rights) {
            return Err(ZxError::ErrNoMemory);
        }
        if self.release_handle(handle) {
            Ok(new_handle)
        } else {
            let _ = self.release_handle(new_handle);
            Err(ZxError::ErrNotFound)
        }
    }

    fn stats(&self) -> MemorySyscallStats {
        MemorySyscallStats {
            linux_mapping_count: self.linux_mappings.len(),
            linux_mapped_bytes: self.linux_mappings.iter().map(|mapping| mapping.len).sum(),
            linux_committed_pages: self
                .linux_mappings
                .iter()
                .map(|mapping| mapping.pfns.len())
                .sum(),
            brk_start: self.brk.start,
            brk_current: self.brk.current,
            brk_limit: self.brk.limit,
            brk_committed_pages: self.brk.committed_pages(),
            zircon_vmo_count: self.vmos.len(),
            zircon_vmo_bytes: self.vmos.iter().map(|record| record.vmo.len()).sum(),
            zircon_vmo_committed_pages: self
                .vmos
                .iter()
                .map(|record| record.vmo.committed_pages())
                .sum(),
            zircon_vmar_count: self.vmars.len(),
            zircon_mapping_count: self
                .vmars
                .iter()
                .map(|record| record.vmar.mappings.len())
                .sum(),
            zircon_root_vmar_handle: self.root_vmar_handle,
        }
    }

    fn free_linux_pages(pfns: &[u64]) {
        for pfn in pfns {
            PageFrameAllocator::free(*pfn);
        }
    }

    fn alloc_linux_pages(page_count: usize) -> Option<Vec<u64>> {
        let mut pfns = Vec::with_capacity(page_count);

        for _ in 0..page_count {
            if let Some(pfn) = PageFrameAllocator::alloc() {
                pfns.push(pfn);
            } else {
                Self::free_linux_pages(&pfns);
                return None;
            }
        }

        Some(pfns)
    }

    fn sort_linux_mappings(&mut self) {
        self.linux_mappings.sort_by_key(|mapping| mapping.addr);
    }

    fn linux_range_available(&self, addr: usize, len: usize) -> bool {
        range_within_window(addr, len, LINUX_MAPPING_BASE, LINUX_MAPPING_LIMIT)
            && !self
                .linux_mappings
                .iter()
                .any(|mapping| range_overlaps(addr, len, mapping.addr, mapping.len))
    }

    fn find_free_linux_region(&mut self, hint: Option<usize>, len: usize) -> Option<usize> {
        if let Some(addr) = hint {
            if self.linux_range_available(addr, len) {
                return Some(addr);
            }
        }

        self.sort_linux_mappings();
        let mut candidate = self.next_linux_addr.max(LINUX_MAPPING_BASE);

        for mapping in &self.linux_mappings {
            let candidate_end = checked_end(candidate, len)?;
            if candidate_end <= mapping.addr {
                self.next_linux_addr = candidate_end;
                return Some(candidate);
            }

            candidate = candidate.max(checked_end(mapping.addr, mapping.len)?);
        }

        let candidate_end = checked_end(candidate, len)?;
        if candidate_end <= LINUX_MAPPING_LIMIT {
            self.next_linux_addr = candidate_end;
            return Some(candidate);
        }

        None
    }

    fn get_vmo(&self, handle: u32) -> Option<&Vmo> {
        let handle = self.resolve_handle(handle, ObjectType::Vmo)?;
        self.vmos
            .iter()
            .find(|record| record.handle == handle)
            .map(|record| &record.vmo)
    }

    fn get_vmo_mut(&mut self, handle: u32) -> Option<&mut Vmo> {
        let handle = self.resolve_handle(handle, ObjectType::Vmo)?;
        self.vmos
            .iter_mut()
            .find(|record| record.handle == handle)
            .map(|record| &mut record.vmo)
    }

    fn get_vmar(&self, handle: u32) -> Option<&Vmar> {
        let handle = self.resolve_handle(handle, ObjectType::Vmar)?;
        self.vmars
            .iter()
            .find(|record| record.handle == handle)
            .map(|record| &record.vmar)
    }

    fn get_vmar_mut(&mut self, handle: u32) -> Option<&mut Vmar> {
        let handle = self.resolve_handle(handle, ObjectType::Vmar)?;
        self.vmars
            .iter_mut()
            .find(|record| record.handle == handle)
            .map(|record| &mut record.vmar)
    }

    fn get_process_mut(&mut self, handle: u32) -> Option<&mut ProcessRecord> {
        let handle = self.resolve_handle(handle, ObjectType::Process)?;
        self.processes
            .iter_mut()
            .find(|record| record.handle == handle)
    }

    fn get_process_by_object(&self, object_handle: u32) -> Option<&ProcessRecord> {
        self.processes
            .iter()
            .find(|record| record.handle == object_handle)
    }

    fn get_job(&self, handle: u32) -> Option<&JobRecord> {
        let handle = self.resolve_handle(handle, ObjectType::Job)?;
        self.jobs.iter().find(|record| record.handle == handle)
    }

    fn get_job_mut(&mut self, handle: u32) -> Option<&mut JobRecord> {
        let handle = self.resolve_handle(handle, ObjectType::Job)?;
        self.jobs.iter_mut().find(|record| record.handle == handle)
    }

    fn get_thread_mut(&mut self, handle: u32) -> Option<&mut ThreadRecord> {
        let handle = self.resolve_handle(handle, ObjectType::Thread)?;
        self.threads
            .iter_mut()
            .find(|record| record.handle == handle)
    }

    fn task_handle_known(&self, handle: u32) -> bool {
        if !self.live_handle_known(handle) {
            return false;
        }
        matches!(
            self.handle_object_type(handle),
            Some(ObjectType::Job | ObjectType::Process | ObjectType::Thread)
        )
    }

    fn handle_known(&self, handle: u32) -> bool {
        self.handle_record(handle).is_some()
    }

    fn live_handle_known(&self, handle: u32) -> bool {
        let Some(record) = self.handle_record(handle) else {
            return false;
        };
        match record.obj_type {
            ObjectType::Vmo => self
                .vmos
                .iter()
                .any(|object| object.handle == record.object_handle),
            ObjectType::Vmar => self
                .vmars
                .iter()
                .any(|object| object.handle == record.object_handle),
            ObjectType::Job => self
                .jobs
                .iter()
                .any(|object| object.handle == record.object_handle),
            ObjectType::Process => self
                .processes
                .iter()
                .any(|object| object.handle == record.object_handle),
            ObjectType::Thread => self
                .threads
                .iter()
                .any(|object| object.handle == record.object_handle),
            _ => true,
        }
    }

    fn process_handle_known(&self, handle: u32) -> bool {
        let Some(object_handle) = self.resolve_handle(handle, ObjectType::Process) else {
            return false;
        };
        self.processes
            .iter()
            .any(|record| record.handle == object_handle)
    }

    fn signal_key(&self, handle: u32) -> u32 {
        self.handle_record(handle)
            .map(|record| record.object_handle)
            .unwrap_or(handle)
    }

    fn get_signal_value(&self, handle: u32) -> u32 {
        let handle = self.signal_key(handle);
        self.signals
            .iter()
            .find(|record| record.handle == handle)
            .map(|record| record.signals)
            .unwrap_or(0)
    }

    fn get_property_value(&self, handle: u32) -> u64 {
        let handle = self.signal_key(handle);
        self.signals
            .iter()
            .find(|record| record.handle == handle)
            .map(|record| record.property_value)
            .unwrap_or(0)
    }

    fn set_property_value(&mut self, handle: u32, value: u64) {
        let handle = self.signal_key(handle);
        if let Some(record) = self
            .signals
            .iter_mut()
            .find(|record| record.handle == handle)
        {
            record.property_value = value;
        } else {
            self.signals.push(SignalRecord {
                handle,
                signals: 0,
                property_value: value,
            });
        }
    }

    fn update_signal_value(&mut self, handle: u32, clear_mask: u32, set_mask: u32) -> u32 {
        let handle = self.signal_key(handle);
        if let Some(record) = self
            .signals
            .iter_mut()
            .find(|record| record.handle == handle)
        {
            record.signals = syscall_logic::signal_update(record.signals, clear_mask, set_mask);
            record.signals
        } else {
            let signals = syscall_logic::signal_update(0, clear_mask, set_mask);
            self.signals.push(SignalRecord {
                handle,
                signals,
                property_value: 0,
            });
            signals
        }
    }

    fn remove_vmo(&mut self, handle: u32) -> bool {
        let Some(record) = self.handle_record(handle) else {
            return false;
        };
        if record.obj_type != ObjectType::Vmo {
            return false;
        }

        let object_handle = record.object_handle;
        let shared = self
            .handles
            .iter()
            .any(|entry| entry.handle != handle && entry.object_handle == object_handle);
        self.handles.retain(|entry| entry.handle != handle);
        self.signals.retain(|signal| signal.handle != handle);
        if shared {
            return true;
        }

        if let Some(index) = self
            .vmos
            .iter()
            .position(|record| record.handle == object_handle)
        {
            let mut record = self.vmos.swap_remove(index);
            record.vmo.release_pages();
            true
        } else {
            false
        }
    }

    fn destroy_vmar_recursive(&mut self, handle: u32) -> bool {
        let Some(index) = self.vmars.iter().position(|record| record.handle == handle) else {
            return false;
        };

        let child_handles = self.vmars[index].vmar.children.clone();
        for child_handle in child_handles {
            self.destroy_vmar_recursive(child_handle as u32);
        }

        if handle == self.root_vmar_handle {
            self.vmars[index].vmar.destroy().ok();
            self.handles.retain(|entry| entry.object_handle != handle);
            self.signals.retain(|signal| signal.handle != handle);
            return true;
        }

        let parent_handle = self.vmars[index]
            .vmar
            .parent_idx
            .map(|parent| parent as u32);
        let mut record = self.vmars.swap_remove(index);
        record.vmar.destroy().ok();
        self.handles.retain(|entry| entry.object_handle != handle);
        self.signals.retain(|signal| signal.handle != handle);

        if let Some(parent_handle) = parent_handle {
            if let Some(parent) = self.get_vmar_mut(parent_handle) {
                parent.children.retain(|child| *child != handle as usize);
            }
        }

        true
    }

    fn remove_vmar(&mut self, handle: u32) -> bool {
        let Some(record) = self.handle_record(handle) else {
            return false;
        };
        if record.obj_type != ObjectType::Vmar {
            return false;
        }

        let object_handle = record.object_handle;
        let shared = self
            .handles
            .iter()
            .any(|entry| entry.handle != handle && entry.object_handle == object_handle);
        self.handles.retain(|entry| entry.handle != handle);
        self.signals.retain(|signal| signal.handle != handle);
        if shared {
            return true;
        }

        self.destroy_vmar_recursive(object_handle)
    }

    fn remove_job(&mut self, handle: u32) -> bool {
        let Some(record) = self.handle_record(handle) else {
            return false;
        };
        if record.obj_type != ObjectType::Job {
            return false;
        }

        let object_handle = record.object_handle;
        let shared = self
            .handles
            .iter()
            .any(|entry| entry.handle != handle && entry.object_handle == object_handle);
        self.handles.retain(|entry| entry.handle != handle);
        self.signals.retain(|signal| signal.handle != handle);
        if shared {
            return true;
        }

        if let Some(index) = self
            .jobs
            .iter()
            .position(|record| record.handle == object_handle)
        {
            let parent = self.jobs[index].parent;
            self.jobs.swap_remove(index);
            if let Some(parent) = parent {
                if let Some(parent_job) =
                    self.jobs.iter_mut().find(|record| record.handle == parent)
                {
                    parent_job.remove_child();
                }
            }
            true
        } else {
            false
        }
    }

    fn remove_process(&mut self, handle: u32) -> bool {
        let Some(record) = self.handle_record(handle) else {
            return false;
        };
        if record.obj_type != ObjectType::Process {
            return false;
        }

        let object_handle = record.object_handle;
        let shared = self
            .handles
            .iter()
            .any(|entry| entry.handle != handle && entry.object_handle == object_handle);
        self.handles.retain(|entry| entry.handle != handle);
        self.signals.retain(|signal| signal.handle != handle);
        if shared {
            return true;
        }

        if let Some(index) = self
            .processes
            .iter()
            .position(|record| record.handle == object_handle)
        {
            let record = self.processes.swap_remove(index);
            if record.pid != 0 {
                let _ = process_manager().terminate_process(record.pid);
            }
            if let Some(job) = self
                .jobs
                .iter_mut()
                .find(|job| job.handle == record.job_handle)
            {
                job.remove_child();
            }
            let root_vmar_handle = self
                .handles
                .iter()
                .find(|entry| {
                    entry.obj_type == ObjectType::Vmar
                        && entry.object_handle == record.root_vmar_handle
                })
                .map(|entry| entry.handle);
            if let Some(root_vmar_handle) = root_vmar_handle {
                let _ = self.remove_vmar(root_vmar_handle);
            }
            true
        } else {
            false
        }
    }

    fn remove_thread(&mut self, handle: u32) -> bool {
        let Some(record) = self.handle_record(handle) else {
            return false;
        };
        if record.obj_type != ObjectType::Thread {
            return false;
        }

        let object_handle = record.object_handle;
        let shared = self
            .handles
            .iter()
            .any(|entry| entry.handle != handle && entry.object_handle == object_handle);
        self.handles.retain(|entry| entry.handle != handle);
        self.signals.retain(|signal| signal.handle != handle);
        if shared {
            return true;
        }

        if let Some(index) = self
            .threads
            .iter()
            .position(|record| record.handle == object_handle)
        {
            self.threads.swap_remove(index);
            true
        } else {
            false
        }
    }

    fn release_handle(&mut self, handle: u32) -> bool {
        self.remove_vmo(handle)
            || self.remove_vmar(handle)
            || self.remove_job(handle)
            || self.remove_process(handle)
            || self.remove_thread(handle)
    }

    fn alloc_fd(&mut self, handle: u32, readable: bool, writable: bool) -> usize {
        let fd = self.next_fd;
        self.next_fd = self.next_fd.saturating_add(1);
        self.linux_fds.push(LinuxFdRecord {
            fd,
            handle,
            readable,
            writable,
        });
        fd
    }

    fn update_fd_access(&mut self, fd: usize, readable: bool, writable: bool) -> bool {
        if let Some(record) = self.linux_fds.iter_mut().find(|record| record.fd == fd) {
            record.readable = readable;
            record.writable = writable;
            true
        } else {
            false
        }
    }

    fn alloc_fd_from(
        &mut self,
        min_fd: usize,
        handle: u32,
        readable: bool,
        writable: bool,
    ) -> usize {
        let mut fd = min_fd.max(COMPAT_FD_START);
        while self.linux_fds.iter().any(|record| record.fd == fd) {
            fd = fd.saturating_add(1);
        }
        self.next_fd = self.next_fd.max(fd.saturating_add(1));
        self.linux_fds.push(LinuxFdRecord {
            fd,
            handle,
            readable,
            writable,
        });
        fd
    }

    fn get_fd(&self, fd: usize) -> Option<&LinuxFdRecord> {
        self.linux_fds.iter().find(|record| record.fd == fd)
    }

    fn close_fd(&mut self, fd: usize) -> Option<u32> {
        let index = self.linux_fds.iter().position(|record| record.fd == fd)?;
        Some(self.linux_fds.swap_remove(index).handle)
    }

    fn close_fd_record(&mut self, fd: usize) -> Option<LinuxFdRecord> {
        let index = self.linux_fds.iter().position(|record| record.fd == fd)?;
        Some(self.linux_fds.swap_remove(index))
    }

    fn handle_has_fd(&self, handle: u32) -> bool {
        self.linux_fds.iter().any(|record| record.handle == handle)
    }

    fn bind_linux_fxfs_file(&mut self, handle: u32, path: String, cursor: fxfs::FxfsCursor) {
        if let Some(record) = self
            .linux_fxfs_files
            .iter_mut()
            .find(|record| record.handle == handle)
        {
            record.cursor = cursor;
            record.path = path;
        } else {
            self.linux_fxfs_files
                .push(LinuxFxfsFileRecord {
                    handle,
                    cursor,
                    path,
                });
        }
    }

    fn linux_fxfs_file(&self, handle: u32) -> Option<&LinuxFxfsFileRecord> {
        self.linux_fxfs_files
            .iter()
            .find(|record| record.handle == handle)
    }

    fn linux_fxfs_file_mut(&mut self, handle: u32) -> Option<&mut LinuxFxfsFileRecord> {
        self.linux_fxfs_files
            .iter_mut()
            .find(|record| record.handle == handle)
    }

    fn remove_linux_fxfs_file(&mut self, handle: u32) {
        if let Some(index) = self
            .linux_fxfs_files
            .iter()
            .position(|record| record.handle == handle)
        {
            self.linux_fxfs_files.swap_remove(index);
        }
    }

    fn apply_linux_namespace_flags(&mut self, flags: usize) {
        self.linux_namespace_flags |= flags & LINUX_CONTAINER_NAMESPACE_FLAGS;
    }

    fn record_linux_setns(&mut self, namespace: usize) {
        self.apply_linux_namespace_flags(namespace);
        self.linux_setns_count = self.linux_setns_count.saturating_add(1);
    }

    fn record_linux_mount(&mut self, flags: usize) -> SysResult {
        if self.linux_mounts.len() >= LINUX_MAX_MOUNTS {
            return Err(SysError::EBUSY);
        }
        self.linux_mounts.push(LinuxMountRecord { flags });
        Ok(0)
    }

    fn record_linux_umount(&mut self) {
        let _ = self.linux_mounts.pop();
    }

    fn reset_linux_container_state(&mut self) {
        self.linux_mounts.clear();
        self.linux_namespace_flags = 0;
        self.linux_setns_count = 0;
        self.linux_pivot_rooted = false;
        self.linux_chrooted = false;
        self.linux_no_new_privs = false;
        self.linux_seccomp_mode = 0;
        self.linux_seccomp_filters = 0;
        self.linux_cap_effective = LINUX_CAP_FULL_SET;
        self.linux_cap_permitted = LINUX_CAP_FULL_SET;
        self.linux_cap_inheritable = 0;
        self.linux_hostname_set = false;
        self.linux_domainname_set = false;
    }

    fn linux_container_stats(&self) -> LinuxContainerStats {
        let mut mount_flags = 0usize;
        for mount in &self.linux_mounts {
            mount_flags |= mount.flags;
        }
        LinuxContainerStats {
            namespace_flags: self.linux_namespace_flags,
            setns_count: self.linux_setns_count,
            mount_count: self.linux_mounts.len(),
            mount_flags,
            pivot_rooted: self.linux_pivot_rooted,
            chrooted: self.linux_chrooted,
            no_new_privs: self.linux_no_new_privs,
            seccomp_mode: self.linux_seccomp_mode,
            seccomp_filters: self.linux_seccomp_filters,
            cap_effective: self.linux_cap_effective,
            cap_permitted: self.linux_cap_permitted,
            cap_inheritable: self.linux_cap_inheritable,
            hostname_set: self.linux_hostname_set,
            domainname_set: self.linux_domainname_set,
        }
    }
}

static mut MEMORY_SYSCALL_STATE: Option<MemorySyscallState> = None;

fn memory_state() -> &'static mut MemorySyscallState {
    unsafe {
        if MEMORY_SYSCALL_STATE.is_none() {
            MEMORY_SYSCALL_STATE = Some(MemorySyscallState::new());
        }

        MEMORY_SYSCALL_STATE.as_mut().unwrap()
    }
}

fn linux_user_cstr(ptr: usize, max_len: usize) -> Result<&'static str, SysError> {
    if ptr == 0 {
        return Err(SysError::EFAULT);
    }

    let mut len = 0usize;
    while len < max_len {
        let addr = ptr.checked_add(len).ok_or(SysError::EFAULT)?;
        let byte = unsafe { core::ptr::read(addr as *const u8) };
        if byte == 0 {
            let bytes = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
            return core::str::from_utf8(bytes).map_err(|_| SysError::EINVAL);
        }
        len = len.saturating_add(1);
    }

    Err(SysError::EINVAL)
}

fn linux_path_is_container_pseudo_file(path: &str) -> bool {
    path.starts_with("/sys/fs/cgroup/") || path.starts_with("/proc/self/attr/")
}

fn linux_path_visible(path: &str) -> bool {
    fxfs::attrs(path).is_ok()
        || linux_path_is_container_pseudo_file(path)
        || crate::user_level::run_elf::active_exec_path()
            .map(|exec_path| exec_path == path)
            .unwrap_or(false)
}

fn linux_prepare_fxfs_cursor(
    path: &str,
    object_type: ObjectType,
    flags: usize,
    access_mode: usize,
) -> Option<fxfs::FxfsCursor> {
    if object_type != ObjectType::LinuxFile {
        return None;
    }

    let readonly_existing = access_mode == LINUX_O_RDONLY
        && flags & (LINUX_O_CREAT | LINUX_O_TRUNC | LINUX_O_APPEND) == 0;
    let container_pseudo = linux_path_is_container_pseudo_file(path);
    if !readonly_existing && !container_pseudo {
        return None;
    }

    if container_pseudo {
        if flags & LINUX_O_CREAT != 0 && !fxfs::exists(path) && fxfs::write_file(path, &[]).is_err()
        {
            return None;
        }
        if flags & LINUX_O_TRUNC != 0 && fxfs::write_file(path, &[]).is_err() {
            return None;
        }
    }

    let mut cursor = fxfs::open_cursor(path).ok()?;
    if flags & LINUX_O_APPEND != 0 {
        let attrs = fxfs::attrs(path).ok()?;
        let _ = fxfs::seek_cursor(&mut cursor, attrs.size).ok()?;
    }
    Some(cursor)
}

pub fn linux_container_stats() -> LinuxContainerStats {
    memory_state().linux_container_stats()
}

pub fn reset_linux_container_state() {
    memory_state().reset_linux_container_state();
}

fn mmu_flags_from_vm_options(options: VmOptions) -> MmuFlags {
    let mut flags = MmuFlags::USER;

    if options.contains(VmOptions::PERM_READ) {
        flags |= MmuFlags::READ;
    }
    if options.contains(VmOptions::PERM_WRITE) {
        flags |= MmuFlags::WRITE;
    }
    if options.contains(VmOptions::PERM_EXECUTE) {
        flags |= MmuFlags::EXECUTE;
    }
    if flags == MmuFlags::USER {
        flags |= MmuFlags::READ;
    }

    flags
}

fn split_linux_mapping(
    mapping: LinuxMappingRecord,
    start: usize,
    len: usize,
) -> Vec<LinuxMappingRecord> {
    let mut pieces = Vec::new();
    let Some(end) = checked_end(start, len) else {
        return pieces;
    };
    let Some(mapping_end) = checked_end(mapping.addr, mapping.len) else {
        return pieces;
    };

    if start > mapping.addr {
        let left_pages = (start - mapping.addr) / PAGE_SIZE;
        let left_len = start - mapping.addr;
        pieces.push(LinuxMappingRecord {
            addr: mapping.addr,
            len: left_len,
            prot: mapping.prot,
            flags: mapping.flags,
            pfns: mapping.pfns[..left_pages].to_vec(),
        });
    }

    if end < mapping_end {
        let right_start_page = (end - mapping.addr) / PAGE_SIZE;
        pieces.push(LinuxMappingRecord {
            addr: end,
            len: mapping_end - end,
            prot: mapping.prot,
            flags: mapping.flags,
            pfns: mapping.pfns[right_start_page..].to_vec(),
        });
    }

    pieces
}

pub fn memory_syscall_stats() -> MemorySyscallStats {
    memory_state().stats()
}

pub fn memory_root_vmar_handle() -> u32 {
    memory_state().root_vmar_handle
}

fn channel_handle_known(handle: u32) -> bool {
    channel::channel_table()
        .get_channel(HandleValue(handle))
        .is_some()
}

fn handle_known_type(handle: u32) -> Option<ObjectType> {
    if syscall_logic::handle_invalid(handle, INVALID_HANDLE) {
        return None;
    }
    let state = memory_state();
    if state.live_handle_known(handle) {
        let obj_type = state.handle_object_type(handle)?;
        return Some(obj_type);
    }
    if channel_handle_known(handle) {
        return Some(ObjectType::Channel);
    }
    if fifo::fifo_table().contains(HandleValue(handle)) {
        return Some(ObjectType::Fifo);
    }
    if port::port_table().contains(HandleValue(handle)) {
        return Some(ObjectType::Port);
    }
    if socket::socket_table().contains(HandleValue(handle)) {
        return Some(ObjectType::Socket);
    }
    compat::table().object_type(HandleValue(handle))
}

fn handle_known_rights(handle: u32) -> Option<u32> {
    let obj_type = handle_known_type(handle)?;
    if memory_state().live_handle_known(handle) {
        let rights = memory_state().handle_rights(handle)?;
        return Some(rights);
    }
    if let Some(rights) = channel::channel_table().rights(HandleValue(handle)) {
        return Some(rights);
    }
    if let Some(rights) = fifo::fifo_table().rights(HandleValue(handle)) {
        return Some(rights);
    }
    if let Some(rights) = port::port_table().rights(HandleValue(handle)) {
        return Some(rights);
    }
    if let Some(rights) = socket::socket_table().rights(HandleValue(handle)) {
        return Some(rights);
    }
    compat::table().rights(HandleValue(handle)).or(Some(
        crate::kernel_objects::default_rights_for_object(obj_type),
    ))
}

fn handle_has_rights(handle: u32, required: u32) -> bool {
    handle_known_rights(handle)
        .map(|rights| crate::kernel_objects::rights_contain(rights, required))
        .unwrap_or(false)
}

fn kernel_object_handle_known(handle: u32) -> bool {
    handle_known_type(handle).is_some()
}

fn object_signal_state(handle: u32) -> ZxResult<u32> {
    let state = memory_state();
    if state.live_handle_known(handle) {
        let record = state.handle_record(handle).ok_or(ZxError::ErrNotFound)?;
        if record.obj_type == ObjectType::Channel {
            let channel_signal = channel::channel_table()
                .get_channel(HandleValue(record.object_handle))
                .map(|channel| channel.get_signal_state(HandleValue(record.object_handle)))
                .ok_or(ZxError::ErrNotFound)?;
            return Ok(channel_signal | state.get_signal_value(handle));
        }
        return Ok(state.get_signal_value(handle));
    }

    let channel_signal = channel::channel_table()
        .get_channel(HandleValue(handle))
        .map(|channel| channel.get_signal_state(HandleValue(handle)));

    if let Some(channel_signal) = channel_signal {
        Ok(channel_signal | memory_state().get_signal_value(handle))
    } else if let Some(signals) = fifo::fifo_table().signals(HandleValue(handle)) {
        Ok(signals | memory_state().get_signal_value(handle))
    } else if let Some(signals) = port::port_table().signals(HandleValue(handle)) {
        Ok(signals | memory_state().get_signal_value(handle))
    } else if let Some(signals) = socket::socket_table().signals(HandleValue(handle)) {
        Ok(signals | memory_state().get_signal_value(handle))
    } else if let Some(signals) = compat::table().signals(HandleValue(handle)) {
        Ok(signals | memory_state().get_signal_value(handle))
    } else {
        Err(ZxError::ErrNotFound)
    }
}

fn set_object_signal_state(handle: u32, clear_mask: u32, set_mask: u32) -> ZxResult<u32> {
    let user_signals_allowed =
        syscall_logic::signal_mask_allowed(clear_mask, set_mask, syscall_logic::user_signal_mask());

    let updated = if memory_state().live_handle_known(handle) {
        if !user_signals_allowed {
            return Err(ZxError::ErrInvalidArgs);
        }
        Ok(memory_state().update_signal_value(handle, clear_mask, set_mask))
    } else if channel_handle_known(handle) {
        if !user_signals_allowed {
            return Err(ZxError::ErrInvalidArgs);
        }
        Ok(memory_state().update_signal_value(handle, clear_mask, set_mask))
    } else if fifo::fifo_table().contains(HandleValue(handle)) {
        if !user_signals_allowed {
            return Err(ZxError::ErrInvalidArgs);
        }
        fifo::fifo_table()
            .update_signals(HandleValue(handle), clear_mask, set_mask)
            .ok_or(ZxError::ErrNotFound)
    } else if port::port_table().contains(HandleValue(handle)) {
        if !user_signals_allowed {
            return Err(ZxError::ErrInvalidArgs);
        }
        port::port_table()
            .update_signals(HandleValue(handle), clear_mask, set_mask)
            .ok_or(ZxError::ErrNotFound)
    } else if socket::socket_table().contains(HandleValue(handle)) {
        if !user_signals_allowed {
            return Err(ZxError::ErrInvalidArgs);
        }
        socket::socket_table()
            .update_signals(HandleValue(handle), clear_mask, set_mask)
            .ok_or(ZxError::ErrNotFound)
    } else if compat::handle_known(HandleValue(handle)) {
        compat::table().update_signals_checked(HandleValue(handle), clear_mask, set_mask)
    } else {
        Err(ZxError::ErrNotFound)
    }?;

    port::port_table().notify_signal(HandleValue(handle), updated);
    Ok(updated)
}

fn monotonic_nanos() -> u64 {
    scheduler::scheduler()
        .get_tick_count()
        .saturating_mul(10_000_000)
}

// ============================================================================
// Linux-compatible Memory Syscalls
// ============================================================================

// Linux mmap protection flags
bitflags::bitflags! {
    pub struct MmapProt: usize {
        const READ = 1 << 0;
        const WRITE = 1 << 1;
        const EXEC = 1 << 2;
    }
}

impl MmapProt {
    pub fn to_flags(&self) -> MmuFlags {
        let mut flags = MmuFlags::USER;
        if self.contains(MmapProt::READ) {
            flags |= MmuFlags::READ;
        }
        if self.contains(MmapProt::WRITE) {
            flags |= MmuFlags::WRITE;
        }
        if self.contains(MmapProt::EXEC) {
            flags |= MmuFlags::EXECUTE;
        }
        if self.is_empty() {
            flags |= MmuFlags::READ | MmuFlags::WRITE;
        }
        flags
    }
}

// Linux mmap flags
bitflags::bitflags! {
    pub struct MmapFlags: usize {
        const SHARED = 1 << 0;
        const PRIVATE = 1 << 1;
        const FIXED = 1 << 4;
        const ANONYMOUS = 1 << 5;
    }
}

/// Helper: check if address is page-aligned
pub fn page_aligned(addr: usize) -> bool {
    shared_page_aligned(addr, PAGE_SIZE)
}

fn fixed_linux_mmap_request_ok(addr: usize, len: usize) -> bool {
    shared_fixed_linux_mmap_request_ok(
        addr,
        len,
        PAGE_SIZE,
        LINUX_MAPPING_BASE,
        LINUX_MAPPING_LIMIT,
    )
}

fn update_linux_protection(
    state: &mut MemorySyscallState,
    addr: usize,
    len: usize,
    prot_bits: usize,
) -> SysResult {
    let end = checked_end(addr, len).ok_or(SysError::EINVAL)?;
    let mut touched = false;
    let mappings = core::mem::take(&mut state.linux_mappings);

    for mapping in mappings {
        if !range_overlaps(addr, len, mapping.addr, mapping.len) {
            state.linux_mappings.push(mapping);
            continue;
        }

        touched = true;
        let overlap_start = core::cmp::max(addr, mapping.addr);
        let mapping_end = checked_end(mapping.addr, mapping.len).ok_or(SysError::EINVAL)?;
        let overlap_end = core::cmp::min(end, mapping_end);
        let start_page = (overlap_start - mapping.addr) / PAGE_SIZE;
        let end_page = (overlap_end - mapping.addr) / PAGE_SIZE;

        for piece in
            split_linux_mapping(mapping.clone(), overlap_start, overlap_end - overlap_start)
        {
            state.linux_mappings.push(piece);
        }

        state.linux_mappings.push(LinuxMappingRecord {
            addr: overlap_start,
            len: overlap_end - overlap_start,
            prot: prot_bits,
            flags: mapping.flags,
            pfns: mapping.pfns[start_page..end_page].to_vec(),
        });
    }

    state.sort_linux_mappings();
    if touched {
        Ok(0)
    } else {
        Err(SysError::EINVAL)
    }
}

fn unmap_linux_range(state: &mut MemorySyscallState, addr: usize, len: usize) -> SysResult {
    let end = checked_end(addr, len).ok_or(SysError::EINVAL)?;
    let mut removed = false;
    let mappings = core::mem::take(&mut state.linux_mappings);

    for mapping in mappings {
        if !range_overlaps(addr, len, mapping.addr, mapping.len) {
            state.linux_mappings.push(mapping);
            continue;
        }

        removed = true;
        let overlap_start = core::cmp::max(addr, mapping.addr);
        let mapping_end = checked_end(mapping.addr, mapping.len).ok_or(SysError::EINVAL)?;
        let overlap_end = core::cmp::min(end, mapping_end);
        let start_page = (overlap_start - mapping.addr) / PAGE_SIZE;
        let end_page = (overlap_end - mapping.addr) / PAGE_SIZE;
        MemorySyscallState::free_linux_pages(&mapping.pfns[start_page..end_page]);

        for piece in split_linux_mapping(mapping, overlap_start, overlap_end - overlap_start) {
            state.linux_mappings.push(piece);
        }
    }

    state.sort_linux_mappings();
    if removed {
        Ok(0)
    } else {
        Err(SysError::EINVAL)
    }
}

/// Linux sys_mmap implementation
pub fn sys_mmap(
    addr: usize,
    len: usize,
    prot: usize,
    flags: usize,
    fd: usize,
    offset: u64,
) -> SysResult {
    let prot = MmapProt::from_bits_truncate(prot);
    let flags = MmapFlags::from_bits_truncate(flags);

    info!(
        "mmap: addr={:#x}, size={:#x}, prot={:?}, flags={:?}",
        addr, len, prot, flags
    );

    if len == 0 {
        return Err(SysError::EINVAL);
    }
    if flags.contains(MmapFlags::SHARED) && flags.contains(MmapFlags::PRIVATE) {
        return Err(SysError::EINVAL);
    }
    if !flags.contains(MmapFlags::SHARED) && !flags.contains(MmapFlags::PRIVATE) {
        return Err(SysError::EINVAL);
    }

    let anonymous = flags.contains(MmapFlags::ANONYMOUS);
    let file_path = if anonymous {
        if offset != 0 {
            return Err(SysError::EINVAL);
        }
        None
    } else {
        if !page_aligned(offset as usize) {
            return Err(SysError::EINVAL);
        }
        Some(linux_fxfs_path_for_fd(fd, true)?)
    };

    if checked_end(addr, len).is_none() {
        return Err(SysError::EINVAL);
    }

    let len = roundup_pages(len);
    let state = memory_state();
    let requested = if flags.contains(MmapFlags::FIXED) {
        if !fixed_linux_mmap_request_ok(addr, len) {
            return Err(SysError::EINVAL);
        }
        let _ = unmap_linux_range(state, addr, len);
        Some(addr)
    } else if addr != 0 && page_aligned(addr) {
        Some(addr)
    } else {
        None
    };

    let vaddr = state
        .find_free_linux_region(requested, len)
        .ok_or(SysError::ENOMEM)?;
    let pfns = MemorySyscallState::alloc_linux_pages(pages(len)).ok_or(SysError::ENOMEM)?;

    state.linux_mappings.push(LinuxMappingRecord {
        addr: vaddr,
        len,
        prot: prot.bits(),
        flags: flags.bits(),
        pfns,
    });
    state.sort_linux_mappings();

    linux_zero_user(vaddr, len)?;
    if let Some(path) = file_path {
        let attrs = fxfs::attrs(path.as_str()).map_err(|_| SysError::EIO)?;
        let offset = offset as usize;
        if offset < attrs.size {
            let read_len = core::cmp::min(len, attrs.size - offset);
            let out = unsafe { core::slice::from_raw_parts_mut(vaddr as *mut u8, read_len) };
            let _ = fxfs::read_file_at(path.as_str(), offset, out).map_err(|_| SysError::EIO)?;
        }
    }

    Ok(vaddr)
}

/// Linux sys_mprotect implementation
pub fn sys_mprotect(addr: usize, len: usize, prot: usize) -> SysResult {
    let prot = MmapProt::from_bits_truncate(prot);

    info!(
        "mprotect: addr={:#x}, size={:#x}, prot={:?}",
        addr, len, prot
    );

    if !page_aligned(addr) || len == 0 {
        return Err(SysError::EINVAL);
    }

    update_linux_protection(memory_state(), addr, roundup_pages(len), prot.bits())
}

/// Linux sys_munmap implementation
pub fn sys_munmap(addr: usize, len: usize) -> SysResult {
    info!("munmap: addr={:#x}, size={:#x}", addr, len);

    if !page_aligned(addr) || len == 0 {
        return Err(SysError::EINVAL);
    }

    unmap_linux_range(memory_state(), addr, roundup_pages(len))
}

/// Linux sys_brk implementation
///
/// The brk syscall is used to change the program break (heap end).
/// It's the traditional way to implement heap allocation in Linux.
///
/// # Arguments
/// * `new_brk` - The new program break address
///
/// # Returns
/// * On success: The current program break address
/// * On error: Negative error code
pub fn sys_brk(new_brk: usize) -> SysResult {
    info!("brk: new_brk={:#x}", new_brk);

    let state = memory_state();
    let old_brk = state.brk.current;

    if new_brk == 0 {
        return Ok(state.brk.current);
    }
    if new_brk < state.brk.start || new_brk > state.brk.limit {
        return Ok(state.brk.current);
    }

    let old_pages = pages(state.brk.current.saturating_sub(state.brk.start));
    let new_pages = pages(new_brk.saturating_sub(state.brk.start));

    if new_pages > old_pages {
        let mut newly_allocated = Vec::with_capacity(new_pages - old_pages);
        for _ in old_pages..new_pages {
            if let Some(pfn) = PageFrameAllocator::alloc() {
                newly_allocated.push(pfn);
            } else {
                MemorySyscallState::free_linux_pages(&newly_allocated);
                return Ok(state.brk.current);
            }
        }
        state.brk.pfns.extend(newly_allocated);
    } else if new_pages < old_pages {
        for _ in new_pages..old_pages {
            if let Some(pfn) = state.brk.pfns.pop() {
                PageFrameAllocator::free(pfn);
            }
        }
    }

    state.brk.current = new_brk;
    if new_brk > old_brk {
        let _ = linux_zero_user(old_brk, new_brk - old_brk);
    }
    Ok(state.brk.current)
}

/// Linux sys_mremap implementation
///
/// The mremap syscall is used to resize existing memory mappings.
///
/// # Arguments
/// * `old_address` - Current mapping address
/// * `old_size` - Current mapping size
/// * `new_size` - New desired size
/// * `flags` - Mremap flags (MREMAP_MAYMOVE, MREMAP_FIXED)
/// * `new_address` - New address if MREMAP_FIXED is set
///
/// # Returns
/// * On success: New mapping address
/// * On error: Negative error code
pub fn sys_mremap(
    old_address: usize,
    old_size: usize,
    new_size: usize,
    flags: usize,
    new_address: usize,
) -> SysResult {
    info!(
        "mremap: old_addr={:#x}, old_size={:#x}, new_size={:#x}, flags={:#x}",
        old_address, old_size, new_size, flags
    );

    const MREMAP_MAYMOVE: usize = 1 << 0;
    const MREMAP_FIXED: usize = 1 << 1;
    const MREMAP_DONTUNMAP: usize = 1 << 2;

    if old_address == 0 || new_size == 0 {
        return Err(SysError::EINVAL);
    }
    if !page_aligned(old_address) || old_size == 0 {
        return Err(SysError::EINVAL);
    }

    if flags & MREMAP_FIXED != 0 && flags & MREMAP_MAYMOVE == 0 {
        return Err(SysError::EINVAL);
    }
    if flags & MREMAP_DONTUNMAP != 0 && flags & MREMAP_MAYMOVE == 0 {
        return Err(SysError::EINVAL);
    }

    let old_len = roundup_pages(old_size);
    let new_len = roundup_pages(new_size);
    let state = memory_state();
    let Some(index) = state
        .linux_mappings
        .iter()
        .position(|mapping| mapping.addr == old_address && mapping.len == old_len)
    else {
        return Err(SysError::EINVAL);
    };

    if new_len == old_len {
        return Ok(old_address);
    }

    if new_len < old_len {
        let mapping = state.linux_mappings.remove(index);
        let keep_pages = new_len / PAGE_SIZE;
        let mut keep_pfns = mapping.pfns;
        let tail_pfns = keep_pfns.split_off(keep_pages);
        MemorySyscallState::free_linux_pages(&tail_pfns);
        state.linux_mappings.push(LinuxMappingRecord {
            addr: old_address,
            len: new_len,
            prot: mapping.prot,
            flags: mapping.flags,
            pfns: keep_pfns,
        });
        state.sort_linux_mappings();
        return Ok(old_address);
    }

    let extra_len = new_len - old_len;
    let grow_start = checked_end(old_address, old_len).ok_or(SysError::EINVAL)?;
    if flags & MREMAP_FIXED == 0 && state.linux_range_available(grow_start, extra_len) {
        let extra_pfns =
            MemorySyscallState::alloc_linux_pages(extra_len / PAGE_SIZE).ok_or(SysError::ENOMEM)?;
        state.linux_mappings[index].len = new_len;
        state.linux_mappings[index].pfns.extend(extra_pfns);
        state.sort_linux_mappings();
        return Ok(old_address);
    }

    if flags & MREMAP_MAYMOVE == 0 {
        return Err(SysError::ENOMEM);
    }

    let requested_addr = if flags & MREMAP_FIXED != 0 {
        if new_address == 0 || !page_aligned(new_address) {
            return Err(SysError::EINVAL);
        }
        let _ = unmap_linux_range(state, new_address, new_len);
        Some(new_address)
    } else {
        None
    };

    let new_addr = state
        .find_free_linux_region(requested_addr, new_len)
        .ok_or(SysError::ENOMEM)?;
    let new_pfns =
        MemorySyscallState::alloc_linux_pages(new_len / PAGE_SIZE).ok_or(SysError::ENOMEM)?;
    let prot = state.linux_mappings[index].prot;
    let flags_bits = state.linux_mappings[index].flags;

    if flags & MREMAP_DONTUNMAP == 0 {
        let old_mapping = state.linux_mappings.swap_remove(index);
        MemorySyscallState::free_linux_pages(&old_mapping.pfns);
    }

    state.linux_mappings.push(LinuxMappingRecord {
        addr: new_addr,
        len: new_len,
        prot,
        flags: flags_bits,
        pfns: new_pfns,
    });
    state.sort_linux_mappings();
    Ok(new_addr)
}

/// Linux sys_write implementation
pub fn sys_write(fd: usize, buf_ptr: usize, len: usize) -> SysResult {
    info!("write: fd={}, buf={:#x}, len={:#x}", fd, buf_ptr, len);

    if len == 0 {
        return Ok(0);
    }
    if buf_ptr == 0 {
        return Err(SysError::EFAULT);
    }

    match fd {
        1 | 2 => {
            let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len) };
            let mut serial = crate::kernel_lowlevel::serial::Serial::new();
            serial.init();
            for byte in buf {
                serial.write_byte(*byte);
            }
            Ok(len)
        }
        _ => {
            let handle = memory_state()
                .get_fd(fd)
                .filter(|record| record.writable)
                .map(|record| record.handle)
                .ok_or(SysError::ENODEV)?;
            let buf = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, len) };
            if socket::socket_table().contains(HandleValue(handle)) {
                return socket::socket_table()
                    .write(HandleValue(handle), buf)
                    .map_err(|_| SysError::EIO);
            }
            if let Some(file) = memory_state().linux_fxfs_file_mut(handle) {
                return fxfs::cursor_write(&mut file.cursor, buf).map_err(|_| SysError::EIO);
            }
            compat::table()
                .write_bytes(HandleValue(handle), buf)
                .map_err(|_| SysError::EIO)
        }
    }
}

/// Linux sys_read implementation.
pub fn sys_read(fd: usize, buf_ptr: usize, len: usize) -> SysResult {
    info!("read: fd={}, buf={:#x}, len={:#x}", fd, buf_ptr, len);

    if len == 0 {
        return Ok(0);
    }
    if buf_ptr == 0 {
        return Err(SysError::EFAULT);
    }

    let handle = memory_state()
        .get_fd(fd)
        .filter(|record| record.readable)
        .map(|record| record.handle)
        .ok_or(SysError::ENODEV)?;
    let out = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len) };

    if socket::socket_table().contains(HandleValue(handle)) {
        return match socket::socket_table().read(HandleValue(handle), 0, out) {
            Ok(read) => Ok(read),
            Err(ZxError::ErrShouldWait) => Ok(0),
            Err(_) => Err(SysError::EIO),
        };
    }

    if let Some(file) = memory_state().linux_fxfs_file_mut(handle) {
        return fxfs::cursor_read(&mut file.cursor, out).map_err(|_| SysError::EIO);
    }

    match compat::table().read_bytes(HandleValue(handle), out) {
        Ok(read) => Ok(read),
        Err(ZxError::ErrShouldWait) => Ok(0),
        Err(_) => Err(SysError::EIO),
    }
}

/// Linux sys_close implementation.
pub fn sys_close(fd: usize) -> SysResult {
    if fd <= 2 {
        return Ok(0);
    }

    let record = memory_state().close_fd_record(fd).ok_or(SysError::EBUSY)?;
    let handle_still_open = memory_state().handle_has_fd(record.handle);
    if !handle_still_open {
        memory_state().remove_linux_fxfs_file(record.handle);
        let _ = sys_handle_close(record.handle);
    }
    Ok(0)
}

/// Linux sys_pipe2 implementation.
pub fn sys_pipe2(fds_ptr: usize, flags: usize) -> SysResult {
    if !syscall_logic::linux_pipe_flags_valid(flags, LINUX_PIPE_ALLOWED_FLAGS) {
        return Err(SysError::EINVAL);
    }
    if fds_ptr == 0 {
        return Err(SysError::EFAULT);
    }

    let (read_handle, write_handle) =
        compat::create_pair(ObjectType::LinuxPipe).map_err(|_| SysError::ENOMEM)?;
    let state = memory_state();
    let read_fd = state.alloc_fd(read_handle.0, true, false);
    let write_fd = state.alloc_fd(write_handle.0, false, true);

    unsafe {
        let out = fds_ptr as *mut i32;
        core::ptr::write(out, read_fd as i32);
        core::ptr::write(out.add(1), write_fd as i32);
    }

    Ok(0)
}

pub fn sys_dup(fd: usize) -> SysResult {
    let record = memory_state().get_fd(fd).cloned().ok_or(SysError::EBUSY)?;
    Ok(memory_state().alloc_fd(record.handle, record.readable, record.writable))
}

pub fn sys_dup3(fd: usize, new_fd: usize, flags: usize) -> SysResult {
    if !syscall_logic::linux_dup3_args_valid(fd, new_fd) {
        return Err(SysError::EINVAL);
    }
    if !syscall_logic::linux_pipe_flags_valid(flags, LINUX_O_CLOEXEC) {
        return Err(SysError::EINVAL);
    }
    let record = memory_state().get_fd(fd).cloned().ok_or(SysError::EBUSY)?;
    let _ = memory_state().close_fd(new_fd);
    memory_state().linux_fds.push(LinuxFdRecord {
        fd: new_fd,
        handle: record.handle,
        readable: record.readable,
        writable: record.writable,
    });
    Ok(new_fd)
}

pub fn sys_getrandom(buf_ptr: usize, len: usize, flags: u32) -> SysResult {
    if !syscall_logic::linux_getrandom_flags_valid(flags, LINUX_GETRANDOM_ALLOWED_FLAGS) {
        return Err(SysError::EINVAL);
    }
    if !syscall_logic::user_buffer_valid(buf_ptr, len) {
        return Err(SysError::EFAULT);
    }
    if len == 0 {
        return Ok(0);
    }

    let seed = monotonic_nanos() as u8;
    let out = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, len) };
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = seed
            .wrapping_add((index as u8).wrapping_mul(37))
            .wrapping_add(0xA5);
    }
    Ok(len)
}

fn linux_fd_known(fd: usize) -> bool {
    fd <= 2 || memory_state().get_fd(fd).is_some()
}

fn linux_fd_handle(fd: usize) -> Result<u32, SysError> {
    memory_state()
        .get_fd(fd)
        .map(|record| record.handle)
        .ok_or(SysError::ENODEV)
}

fn linux_fxfs_path_for_fd(fd: usize, require_readable: bool) -> Result<String, SysError> {
    let state = memory_state();
    let record = state
        .get_fd(fd)
        .filter(|record| !require_readable || record.readable)
        .ok_or(SysError::ENODEV)?;
    let file = state
        .linux_fxfs_file(record.handle)
        .ok_or(SysError::ENODEV)?;
    Ok(file.path.clone())
}

fn linux_fd_object_type(fd: usize) -> Option<ObjectType> {
    let handle = memory_state().get_fd(fd)?.handle;
    compat::table().object_type(HandleValue(handle))
}

fn linux_fd_is_dir(fd: usize) -> bool {
    linux_fd_object_type(fd) == Some(ObjectType::LinuxDir)
}

fn linux_fd_is_file(fd: usize) -> bool {
    matches!(
        linux_fd_object_type(fd),
        Some(ObjectType::LinuxFile | ObjectType::LinuxDir | ObjectType::MemFd)
    )
}

fn linux_fd_is_file_or_pipe(fd: usize) -> bool {
    matches!(
        linux_fd_object_type(fd),
        Some(
            ObjectType::LinuxFile
                | ObjectType::LinuxDir
                | ObjectType::MemFd
                | ObjectType::LinuxPipe
        )
    )
}

fn linux_socket_object_type(domain: usize, socket_type: usize, protocol: usize) -> ObjectType {
    let socket_kind = socket_type & LINUX_SOCK_TYPE_MASK;

    match (domain, socket_kind, protocol) {
        (LINUX_AF_INET, LINUX_SOCK_STREAM, LINUX_IPPROTO_IP | LINUX_IPPROTO_TCP) => {
            ObjectType::LinuxTcpSocket
        }
        (LINUX_AF_INET, LINUX_SOCK_DGRAM, LINUX_IPPROTO_IP | LINUX_IPPROTO_UDP) => {
            ObjectType::LinuxUdpSocket
        }
        (LINUX_AF_INET | LINUX_AF_PACKET, LINUX_SOCK_RAW, _) => ObjectType::LinuxRawSocket,
        (LINUX_AF_NETLINK, _, _) => ObjectType::LinuxNetlinkSocket,
        _ => ObjectType::Socket,
    }
}

fn linux_socket_args_valid(domain: usize, socket_type: usize) -> bool {
    syscall_logic::linux_memfd_flags_valid(socket_type, LINUX_SOCK_ALLOWED_FLAGS)
        && syscall_logic::linux_socket_domain_supported(
            domain,
            LINUX_AF_UNIX,
            LINUX_AF_LOCAL,
            LINUX_AF_INET,
            LINUX_AF_NETLINK,
            LINUX_AF_PACKET,
        )
        && syscall_logic::linux_socket_type_supported(
            socket_type,
            LINUX_SOCK_TYPE_MASK,
            LINUX_SOCK_STREAM,
            LINUX_SOCK_DGRAM,
            LINUX_SOCK_RAW,
        )
        && syscall_logic::linux_socket_domain_type_supported(
            domain,
            socket_type & LINUX_SOCK_TYPE_MASK,
            LINUX_AF_UNIX,
            LINUX_AF_LOCAL,
            LINUX_AF_INET,
            LINUX_AF_NETLINK,
            LINUX_AF_PACKET,
            LINUX_SOCK_STREAM,
            LINUX_SOCK_DGRAM,
            LINUX_SOCK_RAW,
        )
}

fn linux_compat_handle_is_type(handle: u32, object_type: ObjectType) -> bool {
    compat::table().is_type(HandleValue(handle), object_type)
}

fn linux_handle_is_socket(handle: u32) -> bool {
    if socket::socket_table().contains(HandleValue(handle)) {
        return true;
    }
    matches!(
        compat::table().object_type(HandleValue(handle)),
        Some(
            ObjectType::Socket
                | ObjectType::LinuxTcpSocket
                | ObjectType::LinuxUdpSocket
                | ObjectType::LinuxRawSocket
                | ObjectType::LinuxNetlinkSocket
        )
    )
}

fn linux_socket_fd_handle(fd: usize) -> Result<u32, SysError> {
    let handle = linux_fd_handle(fd)?;
    if linux_handle_is_socket(handle) {
        Ok(handle)
    } else {
        Err(SysError::ENOTSOCK)
    }
}

fn linux_zero_user(ptr: usize, len: usize) -> SysResult {
    if !syscall_logic::user_buffer_valid(ptr, len) {
        return Err(SysError::EFAULT);
    }
    if len != 0 {
        unsafe {
            core::ptr::write_bytes(ptr as *mut u8, 0, len);
        }
    }
    Ok(0)
}

fn linux_write_cstr(buf: usize, len: usize, value: &[u8]) -> SysResult {
    if len == 0 || buf == 0 {
        return Err(SysError::EFAULT);
    }
    if value.len().saturating_add(1) > len {
        return Err(SysError::EINVAL);
    }
    unsafe {
        core::ptr::copy_nonoverlapping(value.as_ptr(), buf as *mut u8, value.len());
        core::ptr::write((buf + value.len()) as *mut u8, 0);
    }
    Ok(buf)
}

fn linux_write_stat_from_attrs(stat_ptr: usize, attrs: fxfs::FxfsAttributes) -> SysResult {
    const ST_DEV_OFF: usize = 0;
    const ST_INO_OFF: usize = 8;
    const ST_MODE_OFF: usize = 16;
    const ST_NLINK_OFF: usize = 20;
    const ST_UID_OFF: usize = 24;
    const ST_GID_OFF: usize = 28;
    const ST_SIZE_OFF: usize = 48;
    const ST_BLKSIZE_OFF: usize = 56;
    const ST_BLOCKS_OFF: usize = 64;

    linux_zero_user(stat_ptr, core::mem::size_of::<LinuxStat>())?;

    let size = core::cmp::min(attrs.size, i64::MAX as usize) as i64;
    let blocks = ((attrs.size.saturating_add(511)) / 512) as i64;
    unsafe {
        core::ptr::write_unaligned((stat_ptr + ST_DEV_OFF) as *mut u64, 1);
        core::ptr::write_unaligned((stat_ptr + ST_INO_OFF) as *mut u64, 1);
        core::ptr::write_unaligned((stat_ptr + ST_MODE_OFF) as *mut u32, attrs.mode);
        core::ptr::write_unaligned((stat_ptr + ST_NLINK_OFF) as *mut u32, attrs.link_count);
        core::ptr::write_unaligned((stat_ptr + ST_UID_OFF) as *mut u32, attrs.uid);
        core::ptr::write_unaligned((stat_ptr + ST_GID_OFF) as *mut u32, attrs.gid);
        core::ptr::write_unaligned((stat_ptr + ST_SIZE_OFF) as *mut i64, size);
        core::ptr::write_unaligned((stat_ptr + ST_BLKSIZE_OFF) as *mut i32, PAGE_SIZE as i32);
        core::ptr::write_unaligned((stat_ptr + ST_BLOCKS_OFF) as *mut i64, blocks);
    }
    Ok(0)
}

fn linux_write_stat(stat_ptr: usize) -> SysResult {
    linux_write_stat_from_attrs(
        stat_ptr,
        fxfs::FxfsAttributes {
            mode: 0o100644,
            uid: 0,
            gid: 0,
            size: 0,
            created_at: 0,
            modified_at: 0,
            accessed_at: 0,
            link_count: 1,
        },
    )
}

fn linux_write_statfs(buf: usize) -> SysResult {
    if buf == 0 {
        return Err(SysError::EFAULT);
    }
    unsafe {
        core::ptr::write(
            buf as *mut LinuxStatFs,
            LinuxStatFs {
                f_type: 0x534d_524f,
                f_bsize: PAGE_SIZE as i64,
                f_blocks: 256,
                f_bfree: 128,
                f_bavail: 128,
                f_files: 256,
                f_ffree: 128,
                f_fsid: (0, 0),
                f_namelen: 255,
                f_frsize: PAGE_SIZE as isize,
                f_flags: 0,
                f_spare: [0; 4],
            },
        );
    }
    Ok(0)
}

fn linux_write_uts_field(field: &mut [u8; 65], value: &[u8]) {
    let count = core::cmp::min(value.len(), field.len() - 1);
    field[..count].copy_from_slice(&value[..count]);
    field[count] = 0;
}

/// Linux sys_pipe wrapper.
pub fn sys_pipe(fds_ptr: usize) -> SysResult {
    sys_pipe2(fds_ptr, 0)
}

/// Linux sys_dup2 wrapper.
pub fn sys_dup2(fd: usize, new_fd: usize) -> SysResult {
    if fd == new_fd {
        if linux_fd_known(fd) {
            Ok(new_fd)
        } else {
            Err(SysError::ENODEV)
        }
    } else {
        sys_dup3(fd, new_fd, 0)
    }
}

pub fn sys_eventfd2(_initval: usize, flags: usize) -> SysResult {
    const LINUX_EVENTFD_ALLOWED_FLAGS: usize = 0x1 | 0x800 | 0x80000;
    if !syscall_logic::linux_memfd_flags_valid(flags, LINUX_EVENTFD_ALLOWED_FLAGS) {
        return Err(SysError::EINVAL);
    }
    let handle = compat::create_object(ObjectType::EventFd).map_err(|_| SysError::ENOMEM)?;
    Ok(memory_state().alloc_fd(handle.0, true, true))
}

pub fn sys_epoll_create1(flags: usize) -> SysResult {
    const LINUX_EPOLL_ALLOWED_FLAGS: usize = 0x80000;
    if !syscall_logic::linux_memfd_flags_valid(flags, LINUX_EPOLL_ALLOWED_FLAGS) {
        return Err(SysError::EINVAL);
    }
    let handle = compat::create_object(ObjectType::Port).map_err(|_| SysError::ENOMEM)?;
    Ok(memory_state().alloc_fd(handle.0, true, true))
}

pub fn sys_epoll_ctl(epfd: usize, _op: usize, fd: usize, event: usize) -> SysResult {
    if !linux_fd_known(epfd) || !linux_fd_known(fd) {
        return Err(SysError::ENODEV);
    }
    if event == 0 {
        return Err(SysError::EFAULT);
    }
    Ok(0)
}

pub fn sys_epoll_pwait(
    epfd: usize,
    events: usize,
    maxevents: usize,
    _timeout: isize,
    _sigmask: usize,
) -> SysResult {
    if !linux_fd_known(epfd) {
        return Err(SysError::ENODEV);
    }
    if maxevents != 0 && events == 0 {
        return Err(SysError::EFAULT);
    }
    Ok(0)
}

/// Linux sys_open wrapper.
pub fn sys_open(path: usize, flags: usize, mode: usize) -> SysResult {
    sys_openat(usize::MAX - 99, path, flags, mode)
}

/// Linux sys_openat compatibility implementation.
pub fn sys_openat(dirfd: usize, path: usize, flags: usize, _mode: usize) -> SysResult {
    if path == 0 {
        return Err(SysError::EFAULT);
    }
    if dirfd != usize::MAX - 99
        && !syscall_logic::linux_fd_target_valid(dirfd, LINUX_STDIO_FD_MAX)
        && !linux_fd_is_dir(dirfd)
    {
        return Err(SysError::ENODEV);
    }
    if !syscall_logic::linux_open_access_mode_valid(
        flags,
        LINUX_O_ACCMODE,
        LINUX_O_RDONLY,
        LINUX_O_WRONLY,
        LINUX_O_RDWR,
    ) || !syscall_logic::linux_open_flags_valid(flags, LINUX_OPEN_ALLOWED_FLAGS)
    {
        return Err(SysError::EINVAL);
    }

    let object_type = if syscall_logic::linux_open_is_directory(flags, LINUX_O_DIRECTORY) {
        ObjectType::LinuxDir
    } else {
        ObjectType::LinuxFile
    };
    let path_str = linux_user_cstr(path, LINUX_PATH_MAX_BYTES)?;
    let access_mode = flags & LINUX_O_ACCMODE;
    if object_type == ObjectType::LinuxFile
        && access_mode == LINUX_O_RDONLY
        && flags & LINUX_O_CREAT == 0
        && !linux_path_visible(path_str)
    {
        return Err(SysError::ENOENT);
    }
    let fxfs_cursor = linux_prepare_fxfs_cursor(path_str, object_type, flags, access_mode);
    let handle = compat::create_object(object_type).map_err(|_| SysError::ENOMEM)?;
    let readable = access_mode != LINUX_O_WRONLY;
    let writable = access_mode != LINUX_O_RDONLY;
    let state = memory_state();
    let fd = state.alloc_fd(handle.0, readable, writable);
    if let Some(cursor) = fxfs_cursor {
        state.bind_linux_fxfs_file(handle.0, String::from(path_str), cursor);
    }
    Ok(fd)
}

pub fn sys_access(path: usize, mode: usize) -> SysResult {
    sys_faccessat(usize::MAX - 99, path, mode, 0)
}

pub fn sys_xattr_path(path: usize, _name: usize, value: usize, size: usize) -> SysResult {
    if path == 0 {
        return Err(SysError::EFAULT);
    }
    if size != 0 && value == 0 {
        return Err(SysError::EFAULT);
    }
    Ok(0)
}

pub fn sys_xattr_fd(fd: usize, _name: usize, value: usize, size: usize) -> SysResult {
    if !linux_fd_known(fd) {
        return Err(SysError::ENODEV);
    }
    if size != 0 && value == 0 {
        return Err(SysError::EFAULT);
    }
    Ok(0)
}

pub fn sys_faccessat(_dirfd: usize, path: usize, mode: usize, _flags: usize) -> SysResult {
    if path == 0 {
        return Err(SysError::EFAULT);
    }
    if !syscall_logic::linux_path_mode_valid(mode, LINUX_ACCESS_MODE_MASK) {
        return Err(SysError::EINVAL);
    }
    let path_str = linux_user_cstr(path, LINUX_PATH_MAX_BYTES)?;
    if !linux_path_visible(path_str) {
        return Err(SysError::ENOENT);
    }
    Ok(0)
}

pub fn sys_faccessat2(dirfd: usize, path: usize, mode: usize, flags: usize) -> SysResult {
    if !syscall_logic::linux_stat_flags_valid(flags, LINUX_STAT_ALLOWED_FLAGS) {
        return Err(SysError::EINVAL);
    }
    sys_faccessat(dirfd, path, mode, flags)
}

pub fn sys_path_noop(path: usize) -> SysResult {
    if path == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_getcwd(buf: usize, len: usize) -> SysResult {
    linux_write_cstr(buf, len, b"/")
}

pub fn sys_chdir(path: usize) -> SysResult {
    if path == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_fchdir(fd: usize) -> SysResult {
    if linux_fd_is_dir(fd) {
        Ok(0)
    } else {
        Err(SysError::ENODEV)
    }
}

pub fn sys_chroot(path: usize) -> SysResult {
    sys_chdir(path)?;
    memory_state().linux_chrooted = true;
    Ok(0)
}

pub fn sys_mount(
    source: usize,
    target: usize,
    filesystemtype: usize,
    flags: usize,
    _data: usize,
) -> SysResult {
    if target == 0 {
        return Err(SysError::EFAULT);
    }
    let propagation_only = source == 0 && filesystemtype == 0 && flags != 0;
    if source == 0 && filesystemtype == 0 && !propagation_only {
        return Err(SysError::EFAULT);
    }
    if flags & !LINUX_MOUNT_ALLOWED_FLAGS != 0 {
        return Err(SysError::EINVAL);
    }
    memory_state().record_linux_mount(flags)
}

pub fn sys_umount2(target: usize, flags: usize) -> SysResult {
    if target == 0 {
        return Err(SysError::EFAULT);
    }
    if flags & !LINUX_UMOUNT_ALLOWED_FLAGS != 0 {
        return Err(SysError::EINVAL);
    }
    memory_state().record_linux_umount();
    Ok(0)
}

pub fn sys_pivot_root(new_root: usize, put_old: usize) -> SysResult {
    if new_root == 0 || put_old == 0 {
        return Err(SysError::EFAULT);
    }
    memory_state().linux_pivot_rooted = true;
    Ok(0)
}

pub fn sys_mkdir(path: usize, mode: usize) -> SysResult {
    sys_mkdirat(usize::MAX - 99, path, mode)
}

pub fn sys_mkdirat(_dirfd: usize, path: usize, _mode: usize) -> SysResult {
    if path == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_mknodat(_dirfd: usize, path: usize, _mode: usize, _dev: usize) -> SysResult {
    if path == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_rmdir(path: usize) -> SysResult {
    if path == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_link(oldpath: usize, newpath: usize) -> SysResult {
    sys_linkat(usize::MAX - 99, oldpath, usize::MAX - 99, newpath, 0)
}

pub fn sys_linkat(
    _olddirfd: usize,
    oldpath: usize,
    _newdirfd: usize,
    newpath: usize,
    _flags: usize,
) -> SysResult {
    if oldpath == 0 || newpath == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_symlinkat(oldpath: usize, _newdirfd: usize, newpath: usize) -> SysResult {
    if oldpath == 0 || newpath == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_unlink(path: usize) -> SysResult {
    sys_unlinkat(usize::MAX - 99, path, 0)
}

pub fn sys_unlinkat(_dirfd: usize, path: usize, flags: usize) -> SysResult {
    if path == 0 {
        return Err(SysError::EFAULT);
    }
    if !syscall_logic::linux_unlink_flags_valid(flags, LINUX_UNLINK_ALLOWED_FLAGS) {
        return Err(SysError::EINVAL);
    }
    Ok(0)
}

pub fn sys_rename(oldpath: usize, newpath: usize) -> SysResult {
    sys_renameat(usize::MAX - 99, oldpath, usize::MAX - 99, newpath)
}

pub fn sys_renameat(
    _olddirfd: usize,
    oldpath: usize,
    _newdirfd: usize,
    newpath: usize,
) -> SysResult {
    if oldpath == 0 || newpath == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_renameat2(
    olddirfd: usize,
    oldpath: usize,
    newdirfd: usize,
    newpath: usize,
    flags: usize,
) -> SysResult {
    if !syscall_logic::linux_rename_flags_valid(flags, LINUX_RENAME_ALLOWED_FLAGS) {
        return Err(SysError::EINVAL);
    }
    sys_renameat(olddirfd, oldpath, newdirfd, newpath)
}

pub fn sys_readlink(path: usize, buf: usize, len: usize) -> SysResult {
    sys_readlinkat(usize::MAX - 99, path, buf, len)
}

pub fn sys_readlinkat(_dirfd: usize, path: usize, buf: usize, len: usize) -> SysResult {
    if path == 0 {
        return Err(SysError::EFAULT);
    }
    if len != 0 && buf == 0 {
        return Err(SysError::EFAULT);
    }
    let path_str = linux_user_cstr(path, LINUX_PATH_MAX_BYTES)?;
    if path_str == "/proc/self/exe" {
        let Some(exec_path) = crate::user_level::run_elf::active_exec_path() else {
            return Err(SysError::ENOENT);
        };
        let bytes = exec_path.as_bytes();
        let write_len = core::cmp::min(len, bytes.len());
        if write_len != 0 {
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, write_len);
            }
        }
        return Ok(write_len);
    }
    if !linux_path_visible(path_str) {
        return Err(SysError::ENOENT);
    }
    Ok(0)
}

pub fn sys_stat(path: usize, stat_ptr: usize) -> SysResult {
    sys_fstatat(usize::MAX - 99, path, stat_ptr, 0)
}

pub fn sys_lstat(path: usize, stat_ptr: usize) -> SysResult {
    sys_fstatat(usize::MAX - 99, path, stat_ptr, 0)
}

pub fn sys_fstat(fd: usize, stat_ptr: usize) -> SysResult {
    if !linux_fd_is_file_or_pipe(fd) && !linux_fd_is_dir(fd) {
        return Err(SysError::ENODEV);
    }
    if let Ok(path) = linux_fxfs_path_for_fd(fd, false) {
        if let Ok(attrs) = fxfs::attrs(path.as_str()) {
            return linux_write_stat_from_attrs(stat_ptr, attrs);
        }
    }
    linux_write_stat(stat_ptr)
}

pub fn sys_fstatat(_dirfd: usize, path: usize, stat_ptr: usize, flags: usize) -> SysResult {
    if path == 0 {
        return Err(SysError::EFAULT);
    }
    if !syscall_logic::linux_stat_flags_valid(flags, LINUX_STAT_ALLOWED_FLAGS) {
        return Err(SysError::EINVAL);
    }
    let path_str = linux_user_cstr(path, LINUX_PATH_MAX_BYTES)?;
    if let Ok(attrs) = fxfs::attrs(path_str) {
        return linux_write_stat_from_attrs(stat_ptr, attrs);
    }
    if !linux_path_visible(path_str) {
        return Err(SysError::ENOENT);
    }
    linux_write_stat(stat_ptr)
}

pub fn sys_statfs(path: usize, buf: usize) -> SysResult {
    if path == 0 {
        return Err(SysError::EFAULT);
    }
    linux_write_statfs(buf)
}

pub fn sys_fstatfs(fd: usize, buf: usize) -> SysResult {
    if !linux_fd_is_file_or_pipe(fd) && !linux_fd_is_dir(fd) {
        return Err(SysError::ENODEV);
    }
    linux_write_statfs(buf)
}

pub fn sys_truncate(path: usize, _len: usize) -> SysResult {
    if path == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_ftruncate(fd: usize, _len: usize) -> SysResult {
    if linux_fd_is_file(fd) {
        Ok(0)
    } else {
        Err(SysError::ENODEV)
    }
}

pub fn sys_fallocate(fd: usize, _mode: usize, _offset: usize, _len: usize) -> SysResult {
    if linux_fd_is_file(fd) {
        Ok(0)
    } else {
        Err(SysError::ENODEV)
    }
}

pub fn sys_chmod(path: usize, _mode: usize) -> SysResult {
    if path == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_fchmod(fd: usize, _mode: usize) -> SysResult {
    if linux_fd_is_file(fd) {
        Ok(0)
    } else {
        Err(SysError::ENODEV)
    }
}

pub fn sys_fchmodat(_dirfd: usize, path: usize, mode: usize, _flags: usize) -> SysResult {
    sys_chmod(path, mode)
}

pub fn sys_chown(path: usize, _uid: usize, _gid: usize) -> SysResult {
    if path == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_fchown(fd: usize, _uid: usize, _gid: usize) -> SysResult {
    if linux_fd_is_file(fd) {
        Ok(0)
    } else {
        Err(SysError::ENODEV)
    }
}

pub fn sys_fchownat(
    _dirfd: usize,
    path: usize,
    uid: usize,
    gid: usize,
    _flags: usize,
) -> SysResult {
    sys_chown(path, uid, gid)
}

pub fn sys_sync() -> SysResult {
    Ok(0)
}

pub fn sys_fsync(fd: usize) -> SysResult {
    if linux_fd_is_file_or_pipe(fd) {
        Ok(0)
    } else {
        Err(SysError::ENODEV)
    }
}

pub fn sys_fdatasync(fd: usize) -> SysResult {
    sys_fsync(fd)
}

pub fn sys_sync_file_range(fd: usize, _offset: usize, _nbytes: usize, _flags: usize) -> SysResult {
    sys_fsync(fd)
}

pub fn sys_utimensat(_dirfd: usize, path: usize, times: usize, _flags: usize) -> SysResult {
    if path == 0 {
        return Err(SysError::EFAULT);
    }
    if times != 0
        && !syscall_logic::user_buffer_valid(times, 2 * core::mem::size_of::<LinuxTimespec>())
    {
        return Err(SysError::EFAULT);
    }
    Ok(0)
}

pub fn sys_signalfd4(_fd: usize, mask: usize, sizemask: usize, flags: usize) -> SysResult {
    const LINUX_SIGNALFD_ALLOWED_FLAGS: usize = 0x800 | 0x80000;
    if !syscall_logic::linux_memfd_flags_valid(flags, LINUX_SIGNALFD_ALLOWED_FLAGS) {
        return Err(SysError::EINVAL);
    }
    if !syscall_logic::linux_sigset_size_valid(sizemask, LINUX_SIGSET_SIZE) {
        return Err(SysError::EINVAL);
    }
    if !syscall_logic::user_buffer_valid(mask, sizemask) {
        return Err(SysError::EFAULT);
    }
    let handle = compat::create_object(ObjectType::SignalFd).map_err(|_| SysError::ENOMEM)?;
    Ok(memory_state().alloc_fd(handle.0, true, false))
}

pub fn sys_inotify_init1(_flags: usize) -> SysResult {
    let handle = compat::create_object(ObjectType::Inotify).map_err(|_| SysError::ENOMEM)?;
    Ok(memory_state().alloc_fd(handle.0, true, false))
}

pub fn sys_inotify_add_watch(fd: usize, path: usize, _mask: usize) -> SysResult {
    if !linux_fd_known(fd) {
        return Err(SysError::ENODEV);
    }
    if path == 0 {
        return Err(SysError::EFAULT);
    }
    Ok(1)
}

pub fn sys_inotify_rm_watch(fd: usize, _wd: usize) -> SysResult {
    if linux_fd_known(fd) {
        Ok(0)
    } else {
        Err(SysError::ENODEV)
    }
}

pub fn sys_ioctl(
    fd: usize,
    _request: usize,
    _arg1: usize,
    _arg2: usize,
    _arg3: usize,
) -> SysResult {
    if linux_fd_known(fd) {
        Ok(0)
    } else {
        Err(SysError::ENODEV)
    }
}

pub fn sys_flock(fd: usize, _operation: usize) -> SysResult {
    if linux_fd_known(fd) {
        Ok(0)
    } else {
        Err(SysError::ENODEV)
    }
}

pub fn sys_fcntl(fd: usize, cmd: usize, arg: usize) -> SysResult {
    const F_DUPFD: usize = 0;
    const F_GETFD: usize = 1;
    const F_SETFD: usize = 2;
    const F_GETFL: usize = 3;
    const F_SETFL: usize = 4;
    const F_DUPFD_CLOEXEC: usize = 1030;

    if !syscall_logic::linux_fcntl_cmd_supported(
        cmd,
        F_DUPFD,
        F_GETFD,
        F_SETFD,
        F_GETFL,
        F_SETFL,
        F_DUPFD_CLOEXEC,
    ) {
        return Err(SysError::EINVAL);
    }

    match cmd {
        F_DUPFD | F_DUPFD_CLOEXEC => {
            let record = memory_state().get_fd(fd).cloned().ok_or(SysError::ENODEV)?;
            Ok(memory_state().alloc_fd_from(arg, record.handle, record.readable, record.writable))
        }
        F_GETFD | F_GETFL => {
            if linux_fd_known(fd) {
                Ok(0)
            } else {
                Err(SysError::ENODEV)
            }
        }
        F_SETFD => {
            if !linux_fd_known(fd) {
                return Err(SysError::ENODEV);
            }
            if !syscall_logic::linux_fcntl_flags_valid(arg, LINUX_O_CLOEXEC) {
                return Err(SysError::EINVAL);
            }
            Ok(0)
        }
        F_SETFL => {
            if !linux_fd_known(fd) {
                return Err(SysError::ENODEV);
            }
            if !syscall_logic::linux_fcntl_flags_valid(arg, LINUX_FCNTL_STATUS_ALLOWED_FLAGS) {
                return Err(SysError::EINVAL);
            }
            Ok(0)
        }
        _ => Err(SysError::EINVAL),
    }
}

pub fn sys_getdents64(fd: usize, buf: usize, len: usize) -> SysResult {
    if !linux_fd_is_dir(fd) {
        return Err(SysError::ENODEV);
    }
    if !syscall_logic::user_buffer_valid(buf, len) {
        return Err(SysError::EFAULT);
    }
    if len != 0 {
        unsafe {
            core::ptr::write_bytes(buf as *mut u8, 0, len);
        }
    }
    Ok(0)
}

fn linux_lseek_target(base: usize, offset: i64) -> Option<usize> {
    if offset >= 0 {
        base.checked_add(offset as usize)
    } else {
        base.checked_sub(offset.checked_neg()? as usize)
    }
}

pub fn sys_lseek(fd: usize, offset: i64, whence: usize) -> SysResult {
    if !syscall_logic::linux_lseek_whence_valid(whence, LINUX_SEEK_MAX_WHENCE) {
        return Err(SysError::EINVAL);
    }
    let state = memory_state();
    let record = state.get_fd(fd).cloned().ok_or(SysError::ENODEV)?;
    if let Some(file) = state.linux_fxfs_file_mut(record.handle) {
        const SEEK_SET: usize = 0;
        const SEEK_CUR: usize = 1;
        const SEEK_END: usize = 2;

        let base = match whence {
            SEEK_SET => 0,
            SEEK_CUR => file.cursor.offset(),
            SEEK_END => fxfs::attrs(file.path.as_str())
                .map_err(|_| SysError::EIO)?
                .size,
            _ => return Err(SysError::EINVAL),
        };
        let target = linux_lseek_target(base, offset).ok_or(SysError::EINVAL)?;
        fxfs::seek_cursor(&mut file.cursor, target).map_err(|_| SysError::EINVAL)?;
        return Ok(target);
    }

    Ok(linux_lseek_target(0, offset).unwrap_or(0))
}

pub fn sys_pread(fd: usize, buf: usize, len: usize, offset: u64) -> SysResult {
    if len == 0 {
        return Ok(0);
    }
    if buf == 0 {
        return Err(SysError::EFAULT);
    }

    let file = {
        let state = memory_state();
        let record = state
            .get_fd(fd)
            .filter(|record| record.readable)
            .cloned()
            .ok_or(SysError::ENODEV)?;
        state.linux_fxfs_file(record.handle).cloned()
    };

    if let Some(mut file) = file {
        let out = unsafe { core::slice::from_raw_parts_mut(buf as *mut u8, len) };
        fxfs::seek_cursor(&mut file.cursor, offset as usize).map_err(|_| SysError::EINVAL)?;
        return fxfs::cursor_read(&mut file.cursor, out).map_err(|_| SysError::EIO);
    }

    sys_read(fd, buf, len)
}

pub fn sys_pwrite(fd: usize, buf: usize, len: usize, _offset: u64) -> SysResult {
    sys_write(fd, buf, len)
}

pub fn sys_preadv(fd: usize, iov_ptr: usize, iov_count: usize, _offset: u64) -> SysResult {
    sys_readv(fd, iov_ptr, iov_count)
}

pub fn sys_pwritev(fd: usize, iov_ptr: usize, iov_count: usize, _offset: u64) -> SysResult {
    sys_writev(fd, iov_ptr, iov_count)
}

pub fn sys_readv(fd: usize, iov_ptr: usize, iov_count: usize) -> SysResult {
    if iov_count == 0 {
        return Ok(0);
    }
    if !syscall_logic::linux_iov_count_valid(iov_count, LINUX_MAX_IOV)
        || !syscall_logic::linux_iov_bytes_valid(
            iov_count,
            core::mem::size_of::<LinuxIovec>(),
            LINUX_MAX_IOV,
        )
    {
        return Err(SysError::EINVAL);
    }
    let byte_len = iov_count * core::mem::size_of::<LinuxIovec>();
    if !syscall_logic::user_buffer_valid(iov_ptr, byte_len) {
        return Err(SysError::EFAULT);
    }

    let iovs = unsafe { core::slice::from_raw_parts(iov_ptr as *const LinuxIovec, iov_count) };
    let mut total = 0usize;
    for iov in iovs {
        if iov.len == 0 {
            continue;
        }
        if iov.base == 0 {
            return Err(SysError::EFAULT);
        }
        let read = sys_read(fd, iov.base, iov.len)?;
        total = total.saturating_add(read);
        if read < iov.len {
            break;
        }
    }
    Ok(total)
}

pub fn sys_writev(fd: usize, iov_ptr: usize, iov_count: usize) -> SysResult {
    if iov_count == 0 {
        return Ok(0);
    }
    if !syscall_logic::linux_iov_count_valid(iov_count, LINUX_MAX_IOV)
        || !syscall_logic::linux_iov_bytes_valid(
            iov_count,
            core::mem::size_of::<LinuxIovec>(),
            LINUX_MAX_IOV,
        )
    {
        return Err(SysError::EINVAL);
    }
    let byte_len = iov_count * core::mem::size_of::<LinuxIovec>();
    if !syscall_logic::user_buffer_valid(iov_ptr, byte_len) {
        return Err(SysError::EFAULT);
    }

    let iovs = unsafe { core::slice::from_raw_parts(iov_ptr as *const LinuxIovec, iov_count) };
    let mut total = 0usize;
    for iov in iovs {
        if iov.len == 0 {
            continue;
        }
        if iov.base == 0 {
            return Err(SysError::EFAULT);
        }
        total = total.saturating_add(sys_write(fd, iov.base, iov.len)?);
    }
    Ok(total)
}

pub fn sys_sendfile(out_fd: usize, in_fd: usize, offset_ptr: usize, count: usize) -> SysResult {
    sys_copy_file_range(in_fd, offset_ptr, out_fd, 0, count, 0)
}

pub fn sys_copy_file_range(
    in_fd: usize,
    in_offset: usize,
    out_fd: usize,
    out_offset: usize,
    count: usize,
    flags: usize,
) -> SysResult {
    if !syscall_logic::linux_copy_flags_valid(flags, 0) {
        return Err(SysError::EINVAL);
    }
    if !linux_fd_is_file_or_pipe(in_fd) || !linux_fd_is_file_or_pipe(out_fd) {
        return Err(SysError::ENODEV);
    }
    let mut buffer = [0u8; 256];
    let mut total = 0usize;
    while total < count {
        let chunk = core::cmp::min(buffer.len(), count - total);
        let read = sys_read(in_fd, buffer.as_mut_ptr() as usize, chunk)?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(sys_write(out_fd, buffer.as_ptr() as usize, read)?);
        if read < chunk {
            break;
        }
    }
    if in_offset != 0 {
        unsafe {
            let old = core::ptr::read(in_offset as *const u64);
            core::ptr::write(in_offset as *mut u64, old.saturating_add(total as u64));
        }
    }
    if out_offset != 0 {
        unsafe {
            let old = core::ptr::read(out_offset as *const u64);
            core::ptr::write(out_offset as *mut u64, old.saturating_add(total as u64));
        }
    }
    Ok(total)
}

pub fn sys_splice(
    in_fd: usize,
    in_offset: usize,
    out_fd: usize,
    out_offset: usize,
    len: usize,
    flags: usize,
) -> SysResult {
    if !syscall_logic::linux_copy_flags_valid(flags, 0) {
        return Err(SysError::EINVAL);
    }
    sys_copy_file_range(in_fd, in_offset, out_fd, out_offset, len, 0)
}

pub fn sys_tee(in_fd: usize, out_fd: usize, len: usize, flags: usize) -> SysResult {
    if !syscall_logic::linux_copy_flags_valid(flags, 0) {
        return Err(SysError::EINVAL);
    }
    sys_copy_file_range(in_fd, 0, out_fd, 0, len, 0)
}

pub fn sys_vmsplice(fd: usize, iov_ptr: usize, iov_count: usize, flags: usize) -> SysResult {
    if !syscall_logic::linux_copy_flags_valid(flags, 0) {
        return Err(SysError::EINVAL);
    }
    sys_writev(fd, iov_ptr, iov_count)
}

pub fn sys_poll(fds: usize, nfds: usize, _timeout: isize) -> SysResult {
    if !syscall_logic::linux_poll_count_valid(nfds, LINUX_MAX_POLL_FDS) {
        return Err(SysError::EINVAL);
    }
    if nfds == 0 {
        return Ok(0);
    }
    let byte_len = nfds
        .checked_mul(core::mem::size_of::<LinuxPollFd>())
        .ok_or(SysError::EINVAL)?;
    if !syscall_logic::user_buffer_valid(fds, byte_len) {
        return Err(SysError::EFAULT);
    }
    let poll_fds = unsafe { core::slice::from_raw_parts_mut(fds as *mut LinuxPollFd, nfds) };
    let mut ready = 0usize;
    for poll_fd in poll_fds {
        poll_fd.revents = 0;
        if poll_fd.fd < 0 {
            continue;
        }
        if !syscall_logic::linux_poll_events_valid(poll_fd.events, LINUX_POLL_ALLOWED_EVENTS) {
            return Err(SysError::EINVAL);
        }
        let fd = poll_fd.fd as usize;
        if !linux_fd_known(fd) {
            poll_fd.revents = 0x0020;
            ready = ready.saturating_add(1);
            continue;
        }
        let record = memory_state().get_fd(fd).cloned().ok_or(SysError::ENODEV)?;
        if record.readable && (poll_fd.events & 0x0001) != 0 {
            poll_fd.revents |= 0x0001;
        }
        if record.writable && (poll_fd.events & 0x0004) != 0 {
            poll_fd.revents |= 0x0004;
        }
        if poll_fd.revents != 0 {
            ready = ready.saturating_add(1);
        }
    }
    Ok(ready)
}

pub fn sys_ppoll(fds: usize, nfds: usize, _timeout: usize, _sigmask: usize) -> SysResult {
    sys_poll(fds, nfds, 0)
}

pub fn sys_select(
    nfds: usize,
    readfds: usize,
    writefds: usize,
    exceptfds: usize,
    _timeout: usize,
) -> SysResult {
    let word_bits = usize::BITS as usize;
    let words = nfds.checked_add(word_bits - 1).ok_or(SysError::EINVAL)? / word_bits;
    let bytes = words
        .checked_mul(core::mem::size_of::<usize>())
        .ok_or(SysError::EINVAL)?;
    if !syscall_logic::user_buffer_valid(readfds, bytes)
        || !syscall_logic::user_buffer_valid(writefds, bytes)
        || !syscall_logic::user_buffer_valid(exceptfds, bytes)
    {
        return Err(SysError::EFAULT);
    }
    Ok(0)
}

pub fn sys_pselect6(
    nfds: usize,
    readfds: usize,
    writefds: usize,
    exceptfds: usize,
    timeout: usize,
    _sigmask: usize,
) -> SysResult {
    sys_select(nfds, readfds, writefds, exceptfds, timeout)
}

pub fn sys_socket(domain: usize, socket_type: usize, protocol: usize) -> SysResult {
    if !linux_socket_args_valid(domain, socket_type) {
        return Err(SysError::EINVAL);
    }
    let handle = compat::create_object(linux_socket_object_type(domain, socket_type, protocol))
        .map_err(|_| SysError::ENOMEM)?;
    Ok(memory_state().alloc_fd(handle.0, true, true))
}

pub fn sys_socketpair(
    domain: usize,
    socket_type: usize,
    protocol: usize,
    fds_ptr: usize,
) -> SysResult {
    if fds_ptr == 0 {
        return Err(SysError::EFAULT);
    }
    if !linux_socket_args_valid(domain, socket_type) {
        return Err(SysError::EINVAL);
    }
    let socket_kind = socket_type & LINUX_SOCK_TYPE_MASK;
    let (left, right) = if (domain == LINUX_AF_UNIX || domain == LINUX_AF_LOCAL)
        && socket_kind == LINUX_SOCK_STREAM
    {
        socket::socket_table()
            .create_pair(0)
            .map_err(|_| SysError::ENOMEM)?
    } else if (domain == LINUX_AF_UNIX || domain == LINUX_AF_LOCAL)
        && socket_kind == LINUX_SOCK_DGRAM
    {
        socket::socket_table()
            .create_pair(socket::SOCKET_DATAGRAM)
            .map_err(|_| SysError::ENOMEM)?
    } else {
        compat::create_pair(linux_socket_object_type(domain, socket_type, protocol))
            .map_err(|_| SysError::ENOMEM)?
    };
    let state = memory_state();
    let left_fd = state.alloc_fd(left.0, true, true);
    let right_fd = state.alloc_fd(right.0, true, true);

    unsafe {
        let out = fds_ptr as *mut i32;
        core::ptr::write(out, left_fd as i32);
        core::ptr::write(out.add(1), right_fd as i32);
    }

    Ok(0)
}

pub fn sys_bind(sockfd: usize, addr: usize, addrlen: usize) -> SysResult {
    linux_socket_fd_handle(sockfd)?;
    if !syscall_logic::linux_socket_addr_valid(addr, addrlen) {
        return Err(SysError::EFAULT);
    }
    Ok(0)
}

pub fn sys_connect(sockfd: usize, addr: usize, addrlen: usize) -> SysResult {
    sys_bind(sockfd, addr, addrlen)
}

pub fn sys_listen(sockfd: usize, _backlog: usize) -> SysResult {
    linux_socket_fd_handle(sockfd).map(|_| 0)
}

pub fn sys_accept(sockfd: usize, addr: usize, addrlen: usize) -> SysResult {
    let handle = linux_socket_fd_handle(sockfd)?;
    if addrlen != 0 && addr == 0 {
        return Err(SysError::EFAULT);
    }
    if addrlen != 0 {
        unsafe {
            core::ptr::write(addrlen as *mut u32, 0);
        }
    }
    if addr != 0 {
        unsafe {
            core::ptr::write_bytes(addr as *mut u8, 0, 16);
        }
    }
    let record = memory_state()
        .get_fd(sockfd)
        .cloned()
        .ok_or(SysError::ENODEV)?;
    Ok(memory_state().alloc_fd(handle, record.readable, record.writable))
}

pub fn sys_getsockname(sockfd: usize, addr: usize, addrlen: usize) -> SysResult {
    linux_socket_fd_handle(sockfd)?;
    if addr == 0 || addrlen == 0 {
        return Err(SysError::EFAULT);
    }
    unsafe {
        let len = core::ptr::read(addrlen as *const u32) as usize;
        if len != 0 {
            core::ptr::write_bytes(addr as *mut u8, 0, len);
        }
        core::ptr::write(addrlen as *mut u32, 0);
    }
    Ok(0)
}

pub fn sys_getpeername(sockfd: usize, addr: usize, addrlen: usize) -> SysResult {
    sys_getsockname(sockfd, addr, addrlen)
}

pub fn sys_setsockopt(
    sockfd: usize,
    _level: usize,
    _optname: usize,
    optval: usize,
    optlen: usize,
) -> SysResult {
    linux_socket_fd_handle(sockfd)?;
    if !syscall_logic::linux_socket_addr_valid(optval, optlen) {
        return Err(SysError::EFAULT);
    }
    Ok(0)
}

pub fn sys_getsockopt(
    sockfd: usize,
    _level: usize,
    _optname: usize,
    optval: usize,
    optlen: usize,
) -> SysResult {
    linux_socket_fd_handle(sockfd)?;
    if optlen == 0 {
        return Err(SysError::EFAULT);
    }
    unsafe {
        let len = core::ptr::read(optlen as *const u32) as usize;
        if optval == 0 && len != 0 {
            return Err(SysError::EFAULT);
        }
        if optval != 0 && len >= core::mem::size_of::<u32>() {
            core::ptr::write(optval as *mut u32, 0);
            core::ptr::write(optlen as *mut u32, core::mem::size_of::<u32>() as u32);
        } else {
            core::ptr::write(optlen as *mut u32, 0);
        }
    }
    Ok(0)
}

pub fn sys_sendto(
    sockfd: usize,
    buf: usize,
    len: usize,
    _flags: usize,
    dest_addr: usize,
    addrlen: usize,
) -> SysResult {
    linux_socket_fd_handle(sockfd)?;
    if dest_addr != 0 && !syscall_logic::linux_socket_addr_valid(dest_addr, addrlen) {
        return Err(SysError::EFAULT);
    }
    sys_write(sockfd, buf, len)
}

pub fn sys_recvfrom(
    sockfd: usize,
    buf: usize,
    len: usize,
    _flags: usize,
    src_addr: usize,
    addrlen: usize,
) -> SysResult {
    linux_socket_fd_handle(sockfd)?;
    if src_addr == 0 && addrlen != 0 {
        return Err(SysError::EFAULT);
    }
    let read = sys_read(sockfd, buf, len)?;
    if src_addr != 0 && addrlen != 0 {
        unsafe {
            let len = core::ptr::read(addrlen as *const u32) as usize;
            core::ptr::write_bytes(src_addr as *mut u8, 0, len);
            core::ptr::write(addrlen as *mut u32, 0);
        }
    }
    Ok(read)
}

pub fn sys_recvmsg(sockfd: usize, msg: usize, _flags: usize) -> SysResult {
    linux_socket_fd_handle(sockfd)?;
    if msg == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_sendmsg(sockfd: usize, msg: usize, _flags: usize) -> SysResult {
    linux_socket_fd_handle(sockfd)?;
    if msg == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_recvmmsg(
    sockfd: usize,
    msgvec: usize,
    vlen: usize,
    flags: usize,
    _timeout: usize,
) -> SysResult {
    if vlen == 0 {
        return Ok(0);
    }
    sys_recvmsg(sockfd, msgvec, flags).map(|_| 0)
}

pub fn sys_shutdown(sockfd: usize, _howto: usize) -> SysResult {
    linux_socket_fd_handle(sockfd)?;
    Ok(0)
}

pub fn sys_semget(_key: usize, nsems: usize, _flags: usize) -> SysResult {
    if !syscall_logic::linux_ipc_count_valid(nsems, LINUX_MAX_SEMAPHORES) {
        return Err(SysError::EINVAL);
    }
    Ok(compat::create_object(ObjectType::Semaphore)
        .map_err(|_| SysError::ENOMEM)?
        .0 as usize)
}

pub fn sys_semctl(id: usize, _num: usize, _cmd: usize, _arg: usize) -> SysResult {
    if linux_compat_handle_is_type(id as u32, ObjectType::Semaphore) {
        Ok(0)
    } else {
        Err(SysError::EINVAL)
    }
}

pub fn sys_semop(id: usize, ops: usize, num_ops: usize) -> SysResult {
    if !linux_compat_handle_is_type(id as u32, ObjectType::Semaphore) {
        return Err(SysError::EINVAL);
    }
    if num_ops != 0 && ops == 0 {
        return Err(SysError::EFAULT);
    }
    Ok(0)
}

pub fn sys_msgget(_key: usize, _flags: usize) -> SysResult {
    Ok(compat::create_object(ObjectType::MessageQueue)
        .map_err(|_| SysError::ENOMEM)?
        .0 as usize)
}

pub fn sys_msgctl(id: usize, _cmd: usize, buffer: usize) -> SysResult {
    if !linux_compat_handle_is_type(id as u32, ObjectType::MessageQueue) {
        return Err(SysError::EINVAL);
    }
    if buffer != 0 {
        linux_zero_user(buffer, 128)?;
    }
    Ok(0)
}

pub fn sys_msgsnd(id: usize, msg_ptr: usize, msg_size: usize, _flags: usize) -> SysResult {
    if !linux_compat_handle_is_type(id as u32, ObjectType::MessageQueue) {
        return Err(SysError::EINVAL);
    }
    if !syscall_logic::linux_msg_size_valid(msg_size, LINUX_MAX_MSG_BYTES) {
        return Err(SysError::EINVAL);
    }
    if !syscall_logic::user_buffer_valid(msg_ptr, msg_size) {
        return Err(SysError::EFAULT);
    }
    let bytes = if msg_size == 0 {
        &[][..]
    } else {
        unsafe { core::slice::from_raw_parts(msg_ptr as *const u8, msg_size) }
    };
    compat::table()
        .write_bytes(HandleValue(id as u32), bytes)
        .map(|_| 0)
        .map_err(|_| SysError::EIO)
}

pub fn sys_msgrcv(
    id: usize,
    msg_ptr: usize,
    msg_size: usize,
    _msg_type: isize,
    _flags: usize,
) -> SysResult {
    if !linux_compat_handle_is_type(id as u32, ObjectType::MessageQueue) {
        return Err(SysError::EINVAL);
    }
    if !syscall_logic::linux_msg_size_valid(msg_size, LINUX_MAX_MSG_BYTES) {
        return Err(SysError::EINVAL);
    }
    if !syscall_logic::user_buffer_valid(msg_ptr, msg_size) {
        return Err(SysError::EFAULT);
    }
    let out = if msg_size == 0 {
        &mut [][..]
    } else {
        unsafe { core::slice::from_raw_parts_mut(msg_ptr as *mut u8, msg_size) }
    };
    match compat::table().read_bytes(HandleValue(id as u32), out) {
        Ok(read) => Ok(read),
        Err(ZxError::ErrShouldWait) => Ok(0),
        Err(_) => Err(SysError::EIO),
    }
}

pub fn sys_shmget(_key: usize, size: usize, _shmflg: usize) -> SysResult {
    if !syscall_logic::linux_ipc_size_valid(size, LINUX_MAX_IPC_BYTES) {
        return Err(SysError::EINVAL);
    }
    let handle = compat::create_object(ObjectType::SharedMemory).map_err(|_| SysError::ENOMEM)?;
    let _ = compat::table().set_property(handle, size as u64);
    Ok(handle.0 as usize)
}

pub fn sys_shmat(id: usize, addr: usize, _shmflg: usize) -> SysResult {
    let handle = HandleValue(id as u32);
    if !compat::table().is_type(handle, ObjectType::SharedMemory) {
        return Err(SysError::EINVAL);
    }
    let size = compat::table()
        .property(handle)
        .map(|value| value as usize)
        .unwrap_or(PAGE_SIZE)
        .max(PAGE_SIZE);
    let flags = MmapFlags::PRIVATE.bits() | MmapFlags::ANONYMOUS.bits();
    sys_mmap(
        addr,
        size,
        MmapProt::READ.bits() | MmapProt::WRITE.bits(),
        flags,
        0,
        0,
    )
    .or_else(|_| {
        sys_mmap(
            0,
            size,
            MmapProt::READ.bits() | MmapProt::WRITE.bits(),
            flags,
            0,
            0,
        )
    })
}

pub fn sys_shmdt(_id: usize, addr: usize, _shmflg: usize) -> SysResult {
    if addr == 0 {
        Err(SysError::EINVAL)
    } else {
        Ok(0)
    }
}

pub fn sys_shmctl(id: usize, _cmd: usize, buffer: usize) -> SysResult {
    if !linux_compat_handle_is_type(id as u32, ObjectType::SharedMemory) {
        return Err(SysError::EINVAL);
    }
    if buffer != 0 {
        let _ = linux_zero_user(buffer, 128)?;
    }
    Ok(0)
}

pub fn sys_rt_sigaction(signum: usize, _act: usize, oldact: usize, sigsetsize: usize) -> SysResult {
    if !syscall_logic::linux_signal_action_valid(signum, LINUX_MAX_SIGNAL)
        || !syscall_logic::linux_sigset_size_valid(sigsetsize, LINUX_SIGSET_SIZE)
    {
        return Err(SysError::EINVAL);
    }
    if oldact != 0 {
        linux_zero_user(oldact, sigsetsize)?;
    }
    Ok(0)
}

pub fn sys_rt_sigprocmask(_how: isize, _set: usize, oldset: usize, sigsetsize: usize) -> SysResult {
    if !syscall_logic::linux_sigset_size_valid(sigsetsize, LINUX_SIGSET_SIZE) {
        return Err(SysError::EINVAL);
    }
    if oldset != 0 {
        linux_zero_user(oldset, sigsetsize)?;
    }
    Ok(0)
}

pub fn sys_rt_sigreturn() -> SysResult {
    Ok(0)
}

pub fn sys_rt_sigpending(set: usize, sigsetsize: usize) -> SysResult {
    if !syscall_logic::linux_sigset_size_valid(sigsetsize, LINUX_SIGSET_SIZE) {
        return Err(SysError::EINVAL);
    }
    linux_zero_user(set, sigsetsize)
}

pub fn sys_rt_sigtimedwait(
    set: usize,
    info: usize,
    _timeout: usize,
    sigsetsize: usize,
) -> SysResult {
    if !syscall_logic::linux_sigset_size_valid(sigsetsize, LINUX_SIGSET_SIZE) {
        return Err(SysError::EINVAL);
    }
    if !syscall_logic::user_buffer_valid(set, sigsetsize) {
        return Err(SysError::EFAULT);
    }
    if info != 0 {
        linux_zero_user(info, 128)?;
    }
    Ok(0)
}

pub fn sys_rt_sigsuspend(mask: usize, sigsetsize: usize) -> SysResult {
    if !syscall_logic::linux_sigset_size_valid(sigsetsize, LINUX_SIGSET_SIZE) {
        return Err(SysError::EINVAL);
    }
    if !syscall_logic::user_buffer_valid(mask, sigsetsize) {
        return Err(SysError::EFAULT);
    }
    Err(SysError::EINTR)
}

pub fn sys_rt_sigqueueinfo(_pid: usize, sig: usize, info: usize) -> SysResult {
    if !syscall_logic::linux_signal_action_valid(sig, LINUX_MAX_SIGNAL) {
        return Err(SysError::EINVAL);
    }
    if info == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_rt_tgsigqueueinfo(_tgid: usize, _tid: usize, sig: usize, info: usize) -> SysResult {
    sys_rt_sigqueueinfo(_tid, sig, info)
}

pub fn sys_sigaltstack(_ss: usize, old_ss: usize) -> SysResult {
    if old_ss != 0 {
        linux_zero_user(old_ss, 24)?;
    }
    Ok(0)
}

pub fn sys_tkill(tid: usize, signum: usize) -> SysResult {
    sys_kill(tid as isize, signum)
}

pub fn sys_tgkill(_tgid: usize, tid: usize, signum: usize) -> SysResult {
    sys_tkill(tid, signum)
}

pub fn sys_set_priority(_priority: usize) -> SysResult {
    Ok(0)
}

pub fn sys_get_priority(_which: usize, _who: usize) -> SysResult {
    Ok(0)
}

pub fn sys_sched_getaffinity(_pid: usize, len: usize, mask: usize) -> SysResult {
    if !syscall_logic::user_buffer_valid(mask, len) {
        return Err(SysError::EFAULT);
    }
    if len != 0 {
        unsafe {
            core::ptr::write_bytes(mask as *mut u8, 0xff, len);
        }
    }
    Ok(len)
}

pub fn sys_sched_setaffinity(_pid: usize, len: usize, mask: usize) -> SysResult {
    if !syscall_logic::user_buffer_valid(mask, len) {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_sched_getparam(_pid: usize, param: usize) -> SysResult {
    linux_zero_user(param, core::mem::size_of::<i32>())
}

pub fn sys_sched_setparam(_pid: usize, param: usize) -> SysResult {
    if param == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_sched_getscheduler(_pid: usize) -> SysResult {
    Ok(0)
}

pub fn sys_sched_setscheduler(_pid: usize, _policy: usize, param: usize) -> SysResult {
    if param == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_sched_get_priority_max(_policy: usize) -> SysResult {
    Ok(0)
}

pub fn sys_sched_get_priority_min(_policy: usize) -> SysResult {
    Ok(0)
}

pub fn sys_sched_rr_get_interval(_pid: usize, tp: usize) -> SysResult {
    linux_zero_user(tp, core::mem::size_of::<LinuxTimespec>())
}

pub fn sys_yield() -> SysResult {
    sys_sched_yield()
}

pub fn sys_arch_prctl(code: i32, _addr: usize) -> SysResult {
    const ARCH_SET_FS: i32 = 0x1002;
    const ARCH_GET_FS: i32 = 0x1003;
    match code {
        ARCH_SET_FS | ARCH_GET_FS => Ok(0),
        _ => Err(SysError::EINVAL),
    }
}

pub fn sys_uname(buf: usize) -> SysResult {
    if buf == 0 {
        return Err(SysError::EFAULT);
    }
    let mut uts = LinuxUtsname {
        sysname: [0; 65],
        nodename: [0; 65],
        release: [0; 65],
        version: [0; 65],
        machine: [0; 65],
        domainname: [0; 65],
    };
    linux_write_uts_field(&mut uts.sysname, b"Linux");
    linux_write_uts_field(&mut uts.nodename, b"smros");
    linux_write_uts_field(&mut uts.release, b"0.1-smros");
    linux_write_uts_field(&mut uts.version, b"SMROS");
    linux_write_uts_field(&mut uts.machine, b"aarch64");
    linux_write_uts_field(&mut uts.domainname, b"localdomain");
    unsafe {
        core::ptr::write(buf as *mut LinuxUtsname, uts);
    }
    Ok(0)
}

pub fn sys_time(time_ptr: usize) -> SysResult {
    let seconds = (monotonic_nanos() / 1_000_000_000) as usize;
    if time_ptr != 0 {
        unsafe {
            core::ptr::write(time_ptr as *mut usize, seconds);
        }
    }
    Ok(seconds)
}

pub fn sys_getitimer(_which: usize, curr_value: usize) -> SysResult {
    linux_zero_user(curr_value, 32)
}

pub fn sys_setitimer(_which: usize, new_value: usize, old_value: usize) -> SysResult {
    if new_value == 0 {
        return Err(SysError::EFAULT);
    }
    if old_value != 0 {
        linux_zero_user(old_value, 32)?;
    }
    Ok(0)
}

pub fn sys_timerfd_create(_clockid: usize, _flags: usize) -> SysResult {
    let handle = compat::create_object(ObjectType::TimerFd).map_err(|_| SysError::ENOMEM)?;
    Ok(memory_state().alloc_fd(handle.0, true, true))
}

pub fn sys_timerfd_settime(
    fd: usize,
    _flags: usize,
    new_value: usize,
    old_value: usize,
) -> SysResult {
    if !linux_fd_known(fd) {
        return Err(SysError::ENODEV);
    }
    if new_value == 0 {
        return Err(SysError::EFAULT);
    }
    if old_value != 0 {
        linux_zero_user(old_value, 32)?;
    }
    Ok(0)
}

pub fn sys_timerfd_gettime(fd: usize, curr_value: usize) -> SysResult {
    if !linux_fd_known(fd) {
        return Err(SysError::ENODEV);
    }
    linux_zero_user(curr_value, 32)
}

pub fn sys_linux_timer_create(_clockid: usize, _sevp: usize, timerid: usize) -> SysResult {
    if timerid == 0 {
        return Err(SysError::EFAULT);
    }
    let handle = compat::create_object(ObjectType::Timer).map_err(|_| SysError::ENOMEM)?;
    unsafe {
        core::ptr::write(timerid as *mut usize, handle.0 as usize);
    }
    Ok(0)
}

pub fn sys_linux_timer_settime(
    timerid: usize,
    _flags: usize,
    new_value: usize,
    old_value: usize,
) -> SysResult {
    if !compat::handle_known(HandleValue(timerid as u32)) {
        return Err(SysError::EINVAL);
    }
    if new_value == 0 {
        return Err(SysError::EFAULT);
    }
    if old_value != 0 {
        linux_zero_user(old_value, 32)?;
    }
    Ok(0)
}

pub fn sys_linux_timer_gettime(timerid: usize, curr_value: usize) -> SysResult {
    if !compat::handle_known(HandleValue(timerid as u32)) {
        return Err(SysError::EINVAL);
    }
    linux_zero_user(curr_value, 32)
}

pub fn sys_linux_timer_getoverrun(timerid: usize) -> SysResult {
    if compat::handle_known(HandleValue(timerid as u32)) {
        Ok(0)
    } else {
        Err(SysError::EINVAL)
    }
}

pub fn sys_linux_timer_delete(timerid: usize) -> SysResult {
    if compat::close_handle(HandleValue(timerid as u32)) {
        Ok(0)
    } else {
        Err(SysError::EINVAL)
    }
}

pub fn sys_clock_settime(clockid: usize, tp: usize) -> SysResult {
    if !syscall_logic::linux_clock_id_supported(clockid) {
        return Err(SysError::EINVAL);
    }
    if tp == 0 {
        return Err(SysError::EFAULT);
    }
    Ok(0)
}

pub fn sys_clock_nanosleep(clockid: usize, _flags: usize, req: usize, _rem: usize) -> SysResult {
    if !syscall_logic::linux_clock_id_supported(clockid) {
        return Err(SysError::EINVAL);
    }
    sys_nanosleep_linux(req)
}

pub fn sys_block_in_kernel() -> SysResult {
    Ok(0)
}

pub fn sys_futex(
    uaddr: usize,
    _op: u32,
    _val: u32,
    _val2: usize,
    _uaddr2: usize,
    _val3: u32,
) -> SysResult {
    if uaddr == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

fn linux_iov_write_compat(handle: u32, vector: usize, vector_size: usize) -> ZxResult<usize> {
    if vector_size == 0 {
        return Ok(0);
    }
    let byte_len = vector_size
        .checked_mul(core::mem::size_of::<LinuxIovec>())
        .ok_or(ZxError::ErrInvalidArgs)?;
    if !syscall_logic::user_buffer_valid(vector, byte_len) {
        return Err(ZxError::ErrInvalidArgs);
    }
    let iovs = unsafe { core::slice::from_raw_parts(vector as *const LinuxIovec, vector_size) };
    let mut total = 0usize;
    for iov in iovs {
        if iov.len == 0 {
            continue;
        }
        if iov.base == 0 {
            return Err(ZxError::ErrInvalidArgs);
        }
        let bytes = unsafe { core::slice::from_raw_parts(iov.base as *const u8, iov.len) };
        total = total.saturating_add(compat::table().write_bytes(HandleValue(handle), bytes)?);
    }
    Ok(total)
}

fn linux_iov_read_compat(handle: u32, vector: usize, vector_size: usize) -> ZxResult<usize> {
    if vector_size == 0 {
        return Ok(0);
    }
    let byte_len = vector_size
        .checked_mul(core::mem::size_of::<LinuxIovec>())
        .ok_or(ZxError::ErrInvalidArgs)?;
    if !syscall_logic::user_buffer_valid(vector, byte_len) {
        return Err(ZxError::ErrInvalidArgs);
    }
    let iovs = unsafe { core::slice::from_raw_parts(vector as *const LinuxIovec, vector_size) };
    let mut total = 0usize;
    for iov in iovs {
        if iov.len == 0 {
            continue;
        }
        if iov.base == 0 {
            return Err(ZxError::ErrInvalidArgs);
        }
        let out = unsafe { core::slice::from_raw_parts_mut(iov.base as *mut u8, iov.len) };
        match compat::table().read_bytes(HandleValue(handle), out) {
            Ok(read) => {
                total = total.saturating_add(read);
                if read < iov.len {
                    break;
                }
            }
            Err(ZxError::ErrShouldWait) if total != 0 => break,
            Err(err) => return Err(err),
        }
    }
    Ok(total)
}

// ============================================================================
// Zircon VMO Syscalls
// ============================================================================

/// Zircon sys_vmo_create implementation
pub fn sys_vmo_create(size: u64, options: u32, out_handle: &mut u32) -> ZxResult {
    info!("vmo.create: size={:#x?}, options={:#x?}", size, options);

    // Options flags:
    // bit 0: resizable
    // bit 1: physical (if set, creates physical VMO)
    // bit 2: contiguous (if set, creates contiguous VMO)

    let resizable = options & 1 != 0;
    let is_physical = options & 2 != 0;
    let is_contiguous = options & 4 != 0;

    if is_physical && is_contiguous {
        return Err(ZxError::ErrInvalidArgs);
    }

    if resizable && (is_physical || is_contiguous) {
        return Err(ZxError::ErrInvalidArgs);
    }

    let page_count = pages(size as usize);
    let mut vmo = if is_physical {
        let mut vmo = Vmo::new_contiguous(size as usize).ok_or(ZxError::ErrNoMemory)?;
        vmo.vmo_type = VmoType::Physical;
        vmo
    } else if is_contiguous {
        Vmo::new_contiguous(size as usize).ok_or(ZxError::ErrNoMemory)?
    } else {
        Vmo::new_paged_with_resizable(resizable, page_count).ok_or(ZxError::ErrNoMemory)?
    };

    let state = memory_state();
    let handle = state.alloc_object_handle(ObjectType::Vmo);
    vmo.handle = HandleValue(handle);
    state.vmos.push(VmoRecord { handle, vmo });
    *out_handle = handle;
    Ok(())
}

/// Zircon sys_vmo_read implementation
pub fn sys_vmo_read(handle: u32, buf: &mut [u8], offset: u64) -> ZxResult<usize> {
    info!("vmo.read: handle={:#x?}, offset={:#x?}", handle, offset);

    let state = memory_state();
    if !state.handle_has_rights(handle, Rights::Read as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    let vmo = state.get_vmo(handle).ok_or(ZxError::ErrNotFound)?;
    vmo.read(offset as usize, buf)?;
    Ok(buf.len())
}

/// Zircon sys_vmo_write implementation
pub fn sys_vmo_write(handle: u32, buf: &[u8], offset: u64) -> ZxResult<usize> {
    info!("vmo.write: handle={:#x?}, offset={:#x?}", handle, offset);

    let state = memory_state();
    if !state.handle_has_rights(handle, Rights::Write as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    let vmo = state.get_vmo_mut(handle).ok_or(ZxError::ErrNotFound)?;
    vmo.write(offset as usize, buf)?;
    Ok(buf.len())
}

/// Zircon sys_vmo_get_size implementation
pub fn sys_vmo_get_size(handle: u32, out_size: &mut usize) -> ZxResult {
    info!("vmo.get_size: handle={:?}", handle);

    let state = memory_state();
    if !state.handle_has_rights(handle, Rights::GetProperty as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    let vmo = state.get_vmo(handle).ok_or(ZxError::ErrNotFound)?;
    *out_size = vmo.len();
    Ok(())
}

/// Zircon sys_vmo_set_size implementation
pub fn sys_vmo_set_size(handle: u32, size: usize) -> ZxResult {
    info!("vmo.set_size: handle={:#x}, size={:#x}", handle, size);

    let state = memory_state();
    if !state.handle_has_rights(handle, Rights::Resize as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    let vmo = state.get_vmo_mut(handle).ok_or(ZxError::ErrNotFound)?;
    vmo.set_len(size)
}

/// Zircon sys_vmo_op_range implementation
pub fn sys_vmo_op_range(handle: u32, op: u32, offset: usize, len: usize) -> ZxResult<usize> {
    info!(
        "vmo.op_range: handle={:#x}, op={:#X}, offset={:#x}, len={:#x}",
        handle, op, offset, len
    );

    let op = VmoOpType::try_from(op).or(Err(ZxError::ErrInvalidArgs))?;
    let state = memory_state();
    let required_right = match op {
        VmoOpType::Commit
        | VmoOpType::Decommit
        | VmoOpType::Zero
        | VmoOpType::Lock
        | VmoOpType::Unlock => Rights::Write as u32,
        VmoOpType::CacheSync
        | VmoOpType::CacheInvalidate
        | VmoOpType::CacheClean
        | VmoOpType::CacheCleanInvalidate => Rights::Read as u32,
    };
    if !state.handle_has_rights(handle, required_right) {
        return Err(ZxError::ErrAccessDenied);
    }
    let vmo = state.get_vmo_mut(handle).ok_or(ZxError::ErrNotFound)?;

    if checked_end(offset, len)
        .filter(|end| *end <= vmo.len())
        .is_none()
    {
        return Err(ZxError::ErrOutOfRange);
    }

    match op {
        VmoOpType::Commit => {
            if !page_aligned(offset) || !page_aligned(len) {
                return Err(ZxError::ErrInvalidArgs);
            }
            vmo.commit(offset, len)?;
            Ok(0)
        }
        VmoOpType::Decommit => {
            if !page_aligned(offset) || !page_aligned(len) {
                return Err(ZxError::ErrInvalidArgs);
            }
            vmo.decommit(offset, len)?;
            Ok(0)
        }
        VmoOpType::Zero => {
            vmo.zero(offset, len)?;
            Ok(0)
        }
        VmoOpType::Lock
        | VmoOpType::Unlock
        | VmoOpType::CacheSync
        | VmoOpType::CacheInvalidate
        | VmoOpType::CacheClean
        | VmoOpType::CacheCleanInvalidate => Ok(0),
    }
}

/// Zircon sys_vmo_create_child implementation.
pub fn sys_vmo_create_child(
    handle: u32,
    options: u32,
    offset: usize,
    size: usize,
    out_handle: &mut u32,
) -> ZxResult {
    info!(
        "vmo.create_child: handle={:#x}, options={:#x}, offset={:#x}, size={:#x}",
        handle, options, offset, size
    );

    let flags = VmoCloneFlags::from_bits(options).ok_or(ZxError::ErrInvalidArgs)?;
    let resizable = flags.contains(VmoCloneFlags::RESIZABLE);
    let is_slice = flags.contains(VmoCloneFlags::SLICE);

    let child = {
        let state = memory_state();
        if !state.handle_has_rights(handle, Rights::Duplicate as u32 | Rights::Read as u32) {
            return Err(ZxError::ErrAccessDenied);
        }
        let parent = state.get_vmo(handle).ok_or(ZxError::ErrNotFound)?;
        if checked_end(offset, size)
            .filter(|end| *end <= parent.len())
            .is_none()
        {
            return Err(ZxError::ErrOutOfRange);
        }

        if is_slice {
            parent.create_slice(offset, size)?
        } else {
            parent.create_child(resizable, offset, size)?
        }
    };

    let state = memory_state();
    let child_handle = state.alloc_object_handle(ObjectType::Vmo);
    let mut child = child;
    child.handle = HandleValue(child_handle);
    state.vmos.push(VmoRecord {
        handle: child_handle,
        vmo: child,
    });
    *out_handle = child_handle;
    Ok(())
}

/// Zircon sys_vmo_create_physical implementation.
pub fn sys_vmo_create_physical(
    _resource: u32,
    paddr: u64,
    size: usize,
    out_handle: &mut u32,
) -> ZxResult {
    info!("vmo.create_physical: paddr={:#x}, size={:#x}", paddr, size);

    if size == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }

    let mut vmo = Vmo::new_physical(paddr, size).ok_or(ZxError::ErrNoMemory)?;
    let state = memory_state();
    let handle = state.alloc_object_handle(ObjectType::Vmo);
    vmo.handle = HandleValue(handle);
    state.vmos.push(VmoRecord { handle, vmo });
    *out_handle = handle;
    Ok(())
}

/// Zircon sys_vmo_create_contiguous implementation.
pub fn sys_vmo_create_contiguous(
    _bti: u32,
    size: usize,
    _alignment_log2: u32,
    out_handle: &mut u32,
) -> ZxResult {
    info!("vmo.create_contiguous: size={:#x}", size);

    if size == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }

    let mut vmo = Vmo::new_contiguous(size).ok_or(ZxError::ErrNoMemory)?;
    let state = memory_state();
    let handle = state.alloc_object_handle(ObjectType::Vmo);
    vmo.handle = HandleValue(handle);
    state.vmos.push(VmoRecord { handle, vmo });
    *out_handle = handle;
    Ok(())
}

/// Zircon sys_vmo_replace_as_executable implementation.
pub fn sys_vmo_replace_as_executable(handle: u32, _vmex: u32, out_handle: &mut u32) -> ZxResult {
    info!("vmo.replace_as_executable: handle={:#x}", handle);

    let state = memory_state();
    if state.get_vmo(handle).is_none() {
        return Err(ZxError::ErrNotFound);
    }

    let source_rights = state.handle_rights(handle).ok_or(ZxError::ErrNotFound)?;
    let executable_rights = source_rights | Rights::Execute as u32;
    if !crate::kernel_objects::rights_are_valid(executable_rights) {
        return Err(ZxError::ErrInvalidArgs);
    }
    let object_handle = state
        .resolve_handle(handle, ObjectType::Vmo)
        .ok_or(ZxError::ErrNotFound)?;
    let executable_handle = state.alloc_handle();
    if !state.register_handle(
        executable_handle,
        object_handle,
        ObjectType::Vmo,
        executable_rights,
    ) {
        return Err(ZxError::ErrNoMemory);
    }
    if !state.release_handle(handle) {
        let _ = state.release_handle(executable_handle);
        return Err(ZxError::ErrNotFound);
    }
    *out_handle = executable_handle;
    Ok(())
}

/// Zircon sys_vmo_set_cache_policy implementation.
pub fn sys_vmo_set_cache_policy(handle: u32, policy: u32) -> ZxResult {
    let policy = match policy {
        0 => CachePolicy::Cached,
        1 => CachePolicy::Uncached,
        2 => CachePolicy::UncachedDevice,
        3 => CachePolicy::WriteCombining,
        _ => return Err(ZxError::ErrInvalidArgs),
    };

    let state = memory_state();
    if !state.handle_has_rights(handle, Rights::SetProperty as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    let vmo = state.get_vmo_mut(handle).ok_or(ZxError::ErrNotFound)?;
    vmo.set_cache_policy(policy)
}

/// Zircon compatibility alias used by the sample syscall crate.
pub fn sys_vmo_cache_policy(handle: u32, policy: u32) -> ZxResult {
    sys_vmo_set_cache_policy(handle, policy)
}

// ============================================================================
// Zircon VMAR Syscalls
// ============================================================================

/// Zircon sys_vmar_map implementation
#[allow(clippy::too_many_arguments)]
pub fn sys_vmar_map(
    vmar_handle: u32,
    options: u32,
    vmar_offset: usize,
    vmo_handle: u32,
    vmo_offset: usize,
    len: usize,
    out_addr: &mut usize,
) -> ZxResult {
    info!(
        "vmar.map: vmar={:#x}, offset={:#x}, vmo={:#x}, len={:#x}",
        vmar_handle, vmar_offset, vmo_handle, len
    );

    let options = VmOptions::from_bits(options).ok_or(ZxError::ErrInvalidArgs)?;

    let len = roundup_pages(len);
    if len == 0 || !page_aligned(vmo_offset) {
        return Err(ZxError::ErrInvalidArgs);
    }

    let vmo_len = {
        let state = memory_state();
        let mut vmo_required_rights = Rights::Map as u32;
        if options.contains(VmOptions::PERM_READ) {
            vmo_required_rights |= Rights::Read as u32;
        }
        if options.contains(VmOptions::PERM_WRITE) {
            vmo_required_rights |= Rights::Write as u32;
        }
        if options.contains(VmOptions::PERM_EXECUTE) {
            vmo_required_rights |= Rights::Execute as u32;
        }
        if !state.handle_has_rights(vmar_handle, Rights::Map as u32)
            || !state.handle_has_rights(vmo_handle, vmo_required_rights)
        {
            return Err(ZxError::ErrAccessDenied);
        }
        let vmo = state.get_vmo(vmo_handle).ok_or(ZxError::ErrNotFound)?;
        vmo.len()
    };

    if checked_end(vmo_offset, len)
        .filter(|end| *end <= vmo_len)
        .is_none()
    {
        return Err(ZxError::ErrOutOfRange);
    }

    let overwrite = options.contains(VmOptions::SPECIFIC_OVERWRITE);
    let specific = options.contains(VmOptions::SPECIFIC) || overwrite;
    let requested_offset = if specific { Some(vmar_offset) } else { None };
    let flags = mmu_flags_from_vm_options(options);
    let state = memory_state();
    if !state.handle_has_rights(vmar_handle, Rights::OpChildren as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    let vmar = state
        .get_vmar_mut(vmar_handle)
        .ok_or(ZxError::ErrNotFound)?;
    *out_addr = vmar.map_ext(
        requested_offset,
        HandleValue(vmo_handle),
        vmo_offset,
        len,
        flags,
        flags,
        overwrite,
        options.contains(VmOptions::MAP_RANGE),
    )?;
    Ok(())
}

/// Zircon sys_vmar_unmap implementation
pub fn sys_vmar_unmap(vmar_handle: u32, addr: usize, len: usize) -> ZxResult {
    info!(
        "vmar.unmap: vmar={:#x}, addr={:#x}, len={:#x}",
        vmar_handle, addr, len
    );

    if !page_aligned(addr) || len == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }

    let state = memory_state();
    if !state.handle_has_rights(vmar_handle, Rights::OpChildren as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    let vmar = state
        .get_vmar_mut(vmar_handle)
        .ok_or(ZxError::ErrNotFound)?;
    vmar.unmap(addr, roundup_pages(len))
}

/// Zircon sys_vmar_protect implementation
pub fn sys_vmar_protect(vmar_handle: u32, options: u32, addr: u64, len: u64) -> ZxResult {
    let raw_options = options;
    let options = VmOptions::from_bits(options).ok_or(ZxError::ErrInvalidArgs)?;

    info!(
        "vmar.protect: vmar={:#x}, options={:#x}, addr={:#x}, len={:#x}",
        vmar_handle, raw_options, addr, len
    );

    if !page_aligned(addr as usize) || len == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }

    let state = memory_state();
    if !state.handle_has_rights(vmar_handle, Rights::Map as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    let vmar = state
        .get_vmar_mut(vmar_handle)
        .ok_or(ZxError::ErrNotFound)?;
    vmar.protect(
        addr as usize,
        roundup_pages(len as usize),
        mmu_flags_from_vm_options(options),
    )
}

/// Zircon sys_vmar_allocate implementation
pub fn sys_vmar_allocate(
    parent_vmar: u32,
    options: u32,
    offset: u64,
    size: u64,
    out_child_vmar: &mut u32,
    out_child_addr: &mut usize,
) -> ZxResult {
    let flags = VmarFlags::from_bits(options).ok_or(ZxError::ErrInvalidArgs)?;

    info!(
        "vmar.allocate: parent={:#x?}, options={:#x?}, offset={:#x?}, size={:#x?}",
        parent_vmar, options, offset, size,
    );

    let size = roundup_pages(size as usize);
    if size == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }

    let requested_offset = if flags.contains(VmarFlags::SPECIFIC) || offset != 0 {
        Some(offset as usize)
    } else {
        None
    };

    let (child_handle, child_addr) = {
        let state = memory_state();
        if !state.handle_has_rights(parent_vmar, Rights::OpChildren as u32) {
            return Err(ZxError::ErrAccessDenied);
        }
        let child_handle = state.alloc_object_handle(ObjectType::Vmar);
        let parent = state
            .get_vmar_mut(parent_vmar)
            .ok_or(ZxError::ErrNotFound)?;
        let child_addr = parent.allocate(requested_offset, size, flags, PAGE_SIZE)?;
        parent.children.push(child_handle as usize);
        (child_handle, child_addr)
    };

    let mut child_vmar = Vmar::new(child_addr, size);
    child_vmar.handle = HandleValue(child_handle);
    child_vmar.parent_idx = Some(parent_vmar as usize);

    let state = memory_state();
    state.vmars.push(VmarRecord {
        handle: child_handle,
        vmar: child_vmar,
    });

    *out_child_vmar = child_handle;
    *out_child_addr = child_addr;
    Ok(())
}

/// Zircon sys_vmar_destroy implementation
pub fn sys_vmar_destroy(vmar_handle: u32) -> ZxResult {
    info!("vmar.destroy: handle={:#x?}", vmar_handle);

    let state = memory_state();
    if !state.handle_has_rights(vmar_handle, Rights::OpChildren as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    if state.remove_vmar(vmar_handle) {
        Ok(())
    } else {
        Err(ZxError::ErrNotFound)
    }
}

/// Zircon sys_vmar_unmap_handle_close_thread_exit implementation
///
/// This is a special Zircon syscall that unmaps a region and handles
/// closing threads that are exiting. It's used when a thread is exiting
/// and needs to clean up its stack mapping.
///
/// # Arguments
/// * `vmar_handle` - VMAR handle
/// * `addr` - Address to unmap
/// * `len` - Length of region to unmap
///
/// # Returns
/// * On success: Ok(())
/// * On error: ZxError
pub fn sys_vmar_unmap_handle_close_thread_exit(
    vmar_handle: u32,
    addr: usize,
    len: usize,
) -> ZxResult {
    info!(
        "vmar.unmap_handle_close_thread_exit: vmar={:#x}, addr={:#x}, len={:#x}",
        vmar_handle, addr, len
    );

    if addr == 0 || len == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }

    if !page_aligned(addr) {
        return Err(ZxError::ErrInvalidArgs);
    }

    let state = memory_state();
    if !state.handle_has_rights(vmar_handle, Rights::OpChildren as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    let vmar = state
        .get_vmar_mut(vmar_handle)
        .ok_or(ZxError::ErrNotFound)?;
    vmar.unmap_handle_close_thread_exit(addr, roundup_pages(len))
}

// ============================================================================
// Linux Process/Task Syscalls
// ============================================================================

/// Linux sys_fork implementation
pub fn sys_fork() -> SysResult {
    info!("fork");

    let pm = process_manager();
    if let Some(pid) = pm.create_process("forked") {
        Ok(pid)
    } else {
        Err(SysError::ENOMEM)
    }
}

/// Linux sys_vfork implementation
pub fn sys_vfork() -> SysResult {
    info!("vfork");
    sys_fork()
}

/// Linux sys_clone implementation
pub fn sys_clone(
    flags: usize,
    newsp: usize,
    _parent_tid: usize,
    _newtls: usize,
    _child_tid: usize,
) -> SysResult {
    info!("clone: flags={:#x}, newsp={:#x}", flags, newsp);
    memory_state().apply_linux_namespace_flags(flags);
    sys_fork()
}

pub fn sys_clone3(args: usize, size: usize) -> SysResult {
    if args == 0 || size < core::mem::size_of::<u64>() {
        return Err(SysError::EFAULT);
    }
    let flags = unsafe { core::ptr::read(args as *const u64) as usize };
    sys_clone(flags, 0, 0, 0, 0)
}

/// Linux sys_execve implementation
pub fn sys_execve(path: usize, _argv: usize, _envp: usize) -> SysResult {
    info!("execve: path={:#x}", path);

    if path == 0 {
        return Err(SysError::EFAULT);
    }

    Ok(0)
}

/// Linux sys_wait4 implementation
pub fn sys_wait4(pid: i32, wstatus: usize, options: u32) -> SysResult {
    info!("wait4: pid={}, options={:#x}", pid, options);

    if wstatus != 0 {
        unsafe {
            core::ptr::write(wstatus as *mut i32, 0);
        }
    }

    if pid > 0 {
        Ok(pid as usize)
    } else {
        Ok(0)
    }
}

/// Linux sys_exit implementation
pub fn sys_exit(exit_code: i32) -> SysResult {
    info!("exit: code={}", exit_code);

    if crate::user_level::user_test::prepare_el0_test_kernel_return(exit_code) {
        return Ok(0);
    }

    if crate::user_level::component::prepare_component_return(exit_code) {
        return Ok(0);
    }

    if crate::user_level::run_elf::prepare_run_elf_return(exit_code) {
        return Ok(0);
    }

    // No current-process binding is modeled yet; EL0 exits through the hooks above.
    Ok(0)
}

/// Linux sys_exit_group implementation
pub fn sys_exit_group(exit_code: i32) -> SysResult {
    info!("exit_group: code={}", exit_code);
    sys_exit(exit_code)
}

/// Linux sys_getpid implementation
pub fn sys_getpid() -> SysResult {
    Ok(1)
}

/// Linux sys_getppid implementation
pub fn sys_getppid() -> SysResult {
    Ok(0)
}

/// Linux sys_gettid implementation
pub fn sys_gettid() -> SysResult {
    Ok(1)
}

pub fn sys_getuid() -> SysResult {
    Ok(0)
}

pub fn sys_geteuid() -> SysResult {
    Ok(0)
}

pub fn sys_getgid() -> SysResult {
    Ok(0)
}

pub fn sys_getegid() -> SysResult {
    Ok(0)
}

pub fn sys_setuid(_uid: usize) -> SysResult {
    Ok(0)
}

pub fn sys_setgid(_gid: usize) -> SysResult {
    Ok(0)
}

pub fn sys_setreuid(_ruid: usize, _euid: usize) -> SysResult {
    Ok(0)
}

pub fn sys_setregid(_rgid: usize, _egid: usize) -> SysResult {
    Ok(0)
}

pub fn sys_setresuid(_ruid: usize, _euid: usize, _suid: usize) -> SysResult {
    Ok(0)
}

pub fn sys_getresuid(ruid: usize, euid: usize, suid: usize) -> SysResult {
    if ruid != 0 {
        unsafe {
            core::ptr::write(ruid as *mut u32, 0);
        }
    }
    if euid != 0 {
        unsafe {
            core::ptr::write(euid as *mut u32, 0);
        }
    }
    if suid != 0 {
        unsafe {
            core::ptr::write(suid as *mut u32, 0);
        }
    }
    Ok(0)
}

pub fn sys_setresgid(_rgid: usize, _egid: usize, _sgid: usize) -> SysResult {
    Ok(0)
}

pub fn sys_getresgid(rgid: usize, egid: usize, sgid: usize) -> SysResult {
    sys_getresuid(rgid, egid, sgid)
}

pub fn sys_setfsuid(_uid: usize) -> SysResult {
    Ok(0)
}

pub fn sys_setfsgid(_gid: usize) -> SysResult {
    Ok(0)
}

/// Linux sys_kill implementation
pub fn sys_kill(pid: isize, signum: usize) -> SysResult {
    info!("kill: pid={}, signal={}", pid, signum);

    if !syscall_logic::linux_signal_valid(signum, LINUX_MAX_SIGNAL) {
        return Err(SysError::EINVAL);
    }
    if pid <= 0 {
        return Err(SysError::ESRCH);
    }
    if signum == 0 {
        return Ok(0);
    }

    let pm = process_manager();
    if pm.terminate_process(pid as usize) {
        Ok(0)
    } else {
        Err(SysError::ESRCH)
    }
}

pub fn sys_set_tid_address(_tidptr: usize) -> SysResult {
    sys_gettid()
}

pub fn sys_set_robust_list(head: usize, len: usize) -> SysResult {
    if !syscall_logic::user_buffer_valid(head, len) {
        return Err(SysError::EFAULT);
    }
    Ok(0)
}

pub fn sys_get_robust_list(_pid: isize, head_ptr: usize, len_ptr: usize) -> SysResult {
    if head_ptr == 0 || len_ptr == 0 {
        return Err(SysError::EFAULT);
    }
    unsafe {
        core::ptr::write(head_ptr as *mut usize, 0);
        core::ptr::write(len_ptr as *mut usize, 0);
    }
    Ok(0)
}

pub fn sys_sched_yield() -> SysResult {
    Ok(0)
}

pub fn sys_umask(_mask: usize) -> SysResult {
    Ok(0o022)
}

pub fn sys_setpgid(_pid: usize, _pgid: usize) -> SysResult {
    Ok(0)
}

pub fn sys_getpgid(_pid: usize) -> SysResult {
    Ok(1)
}

pub fn sys_getsid(_pid: usize) -> SysResult {
    Ok(1)
}

pub fn sys_setsid() -> SysResult {
    Ok(1)
}

pub fn sys_getgroups(size: usize, list: usize) -> SysResult {
    if size != 0 && list == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_setgroups(size: usize, list: usize) -> SysResult {
    if size != 0 && list == 0 {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_capget(header: usize, data: usize) -> SysResult {
    if header == 0 || data == 0 {
        return Err(SysError::EFAULT);
    }
    let stats = linux_container_stats();
    unsafe {
        let header_ptr = header as *mut LinuxCapUserHeader;
        let requested = core::ptr::read(header_ptr).version;
        if requested != 0 && requested != LINUX_CAPABILITY_VERSION_3 {
            return Err(SysError::EINVAL);
        }
        core::ptr::write(
            header_ptr,
            LinuxCapUserHeader {
                version: LINUX_CAPABILITY_VERSION_3,
                pid: 0,
            },
        );
        let data_ptr = data as *mut LinuxCapUserData;
        core::ptr::write(
            data_ptr,
            LinuxCapUserData {
                effective: stats.cap_effective as u32,
                permitted: stats.cap_permitted as u32,
                inheritable: stats.cap_inheritable as u32,
            },
        );
        core::ptr::write(
            data_ptr.add(1),
            LinuxCapUserData {
                effective: (stats.cap_effective >> 32) as u32,
                permitted: (stats.cap_permitted >> 32) as u32,
                inheritable: (stats.cap_inheritable >> 32) as u32,
            },
        );
    }
    Ok(0)
}

pub fn sys_capset(header: usize, data: usize) -> SysResult {
    if header == 0 || data == 0 {
        return Err(SysError::EFAULT);
    }
    unsafe {
        let version = core::ptr::read(header as *const LinuxCapUserHeader).version;
        if version != LINUX_CAPABILITY_VERSION_3 {
            return Err(SysError::EINVAL);
        }
        let data_ptr = data as *const LinuxCapUserData;
        let lo = core::ptr::read(data_ptr);
        let hi = core::ptr::read(data_ptr.add(1));
        let state = memory_state();
        state.linux_cap_effective =
            (((hi.effective as u64) << 32) | lo.effective as u64) & LINUX_CAP_FULL_SET;
        state.linux_cap_permitted =
            (((hi.permitted as u64) << 32) | lo.permitted as u64) & LINUX_CAP_FULL_SET;
        state.linux_cap_inheritable =
            (((hi.inheritable as u64) << 32) | lo.inheritable as u64) & LINUX_CAP_FULL_SET;
    }
    Ok(0)
}

pub fn sys_sethostname(name: usize, len: usize) -> SysResult {
    if !syscall_logic::user_buffer_valid(name, len) || len > LINUX_UTS_NAME_MAX {
        return Err(SysError::EFAULT);
    }
    memory_state().linux_hostname_set = true;
    Ok(0)
}

pub fn sys_setdomainname(name: usize, len: usize) -> SysResult {
    if !syscall_logic::user_buffer_valid(name, len) || len > LINUX_UTS_NAME_MAX {
        return Err(SysError::EFAULT);
    }
    memory_state().linux_domainname_set = true;
    Ok(0)
}

pub fn sys_prctl(
    option: usize,
    arg2: usize,
    _arg3: usize,
    _arg4: usize,
    _arg5: usize,
) -> SysResult {
    const PR_SET_NO_NEW_PRIVS: usize = 38;
    const PR_GET_NO_NEW_PRIVS: usize = 39;
    const PR_SET_SECCOMP: usize = 22;
    const PR_GET_SECCOMP: usize = 21;
    const PR_SET_NAME: usize = 15;
    const PR_GET_NAME: usize = 16;
    const PR_CAPBSET_READ: usize = 23;
    const PR_CAPBSET_DROP: usize = 24;
    const PR_SET_DUMPABLE: usize = 4;
    const PR_GET_DUMPABLE: usize = 3;

    match option {
        PR_SET_NO_NEW_PRIVS => {
            if arg2 > 1 {
                return Err(SysError::EINVAL);
            }
            memory_state().linux_no_new_privs = arg2 != 0;
            Ok(0)
        }
        PR_GET_NO_NEW_PRIVS => Ok(memory_state().linux_no_new_privs as usize),
        PR_SET_SECCOMP => sys_seccomp_mode(arg2, 0),
        PR_GET_SECCOMP => Ok(memory_state().linux_seccomp_mode),
        PR_SET_NAME => {
            if arg2 == 0 {
                Err(SysError::EFAULT)
            } else {
                Ok(0)
            }
        }
        PR_GET_NAME => linux_write_cstr(arg2, 16, b"smros-docker").map(|_| 0),
        PR_CAPBSET_READ => {
            if arg2 > LINUX_CAP_LAST_CAP as usize {
                Err(SysError::EINVAL)
            } else {
                Ok(1)
            }
        }
        PR_CAPBSET_DROP | PR_SET_DUMPABLE => Ok(0),
        PR_GET_DUMPABLE => Ok(1),
        _ => Ok(0),
    }
}

pub fn sys_getcpu(cpu: usize, node: usize, _cache: usize) -> SysResult {
    if cpu != 0 {
        unsafe {
            core::ptr::write(cpu as *mut u32, 0);
        }
    }
    if node != 0 {
        unsafe {
            core::ptr::write(node as *mut u32, 0);
        }
    }
    Ok(0)
}

pub fn sys_madvise(_addr: usize, _len: usize, _advice: usize) -> SysResult {
    Ok(0)
}

pub fn sys_waitid(
    _which: usize,
    _pid: usize,
    infop: usize,
    _options: usize,
    _rusage: usize,
) -> SysResult {
    if infop != 0 {
        linux_zero_user(infop, 128)?;
    }
    Ok(0)
}

pub fn sys_close_range(first: usize, last: usize, _flags: usize) -> SysResult {
    if !syscall_logic::linux_fd_range_valid(first, last) {
        return Err(SysError::EINVAL);
    }
    for fd in first..=last {
        if fd > 1024 {
            break;
        }
        let _ = sys_close(fd);
    }
    Ok(0)
}

pub fn sys_memfd_create(name: usize, flags: usize) -> SysResult {
    if name == 0 {
        return Err(SysError::EFAULT);
    }
    if !syscall_logic::linux_memfd_flags_valid(flags, LINUX_MEMFD_ALLOWED_FLAGS) {
        return Err(SysError::EINVAL);
    }
    let handle = compat::create_object(ObjectType::MemFd).map_err(|_| SysError::ENOMEM)?;
    Ok(memory_state().alloc_fd(handle.0, true, true))
}

pub fn sys_membarrier(_cmd: usize, flags: usize, _cpu_id: usize) -> SysResult {
    if flags != 0 {
        return Err(SysError::EINVAL);
    }
    Ok(0)
}

pub fn sys_unshare(flags: usize) -> SysResult {
    if !syscall_logic::linux_namespace_flags_valid(flags, LINUX_CONTAINER_NAMESPACE_FLAGS) {
        return Err(SysError::EINVAL);
    }
    memory_state().apply_linux_namespace_flags(flags);
    Ok(0)
}

pub fn sys_setns(fd: usize, nstype: usize) -> SysResult {
    if !linux_fd_known(fd) {
        return Err(SysError::ENODEV);
    }
    if nstype != 0
        && !syscall_logic::linux_namespace_flags_valid(nstype, LINUX_CONTAINER_NAMESPACE_FLAGS)
    {
        return Err(SysError::EINVAL);
    }
    memory_state().record_linux_setns(nstype);
    Ok(0)
}

fn sys_seccomp_mode(mode: usize, _filter: usize) -> SysResult {
    match mode {
        LINUX_SECCOMP_MODE_STRICT => {
            memory_state().linux_seccomp_mode = LINUX_SECCOMP_MODE_STRICT;
            Ok(0)
        }
        LINUX_SECCOMP_MODE_FILTER => {
            let state = memory_state();
            state.linux_seccomp_mode = LINUX_SECCOMP_MODE_FILTER;
            state.linux_seccomp_filters = state.linux_seccomp_filters.saturating_add(1);
            Ok(0)
        }
        _ => Err(SysError::EINVAL),
    }
}

pub fn sys_seccomp(operation: usize, flags: usize, args: usize) -> SysResult {
    match operation {
        LINUX_SECCOMP_SET_MODE_STRICT => {
            if flags != 0 {
                Err(SysError::EINVAL)
            } else {
                sys_seccomp_mode(LINUX_SECCOMP_MODE_STRICT, args)
            }
        }
        LINUX_SECCOMP_SET_MODE_FILTER => {
            if flags & !LINUX_SECCOMP_FILTER_ALLOWED_FLAGS != 0 {
                Err(SysError::EINVAL)
            } else {
                sys_seccomp_mode(LINUX_SECCOMP_MODE_FILTER, args)
            }
        }
        LINUX_SECCOMP_GET_ACTION_AVAIL => {
            if args == 0 {
                Err(SysError::EFAULT)
            } else {
                Ok(0)
            }
        }
        LINUX_SECCOMP_GET_NOTIF_SIZES => {
            if args == 0 {
                return Err(SysError::EFAULT);
            }
            linux_zero_user(args, 6)
        }
        _ => Err(SysError::EINVAL),
    }
}

pub fn sys_statx(
    _dirfd: usize,
    path: usize,
    flags: usize,
    mask: usize,
    statxbuf: usize,
) -> SysResult {
    if path == 0 || statxbuf == 0 {
        return Err(SysError::EFAULT);
    }
    if !syscall_logic::linux_stat_flags_valid(flags, LINUX_STAT_ALLOWED_FLAGS)
        || !syscall_logic::linux_stat_mask_valid(mask, LINUX_STATX_BASIC_STATS)
    {
        return Err(SysError::EINVAL);
    }
    linux_zero_user(statxbuf, 256)
}

pub fn sys_memory_noop(addr: usize, len: usize) -> SysResult {
    if !syscall_logic::user_buffer_valid(addr, len) {
        Err(SysError::EFAULT)
    } else {
        Ok(0)
    }
}

pub fn sys_mincore(addr: usize, len: usize, vec: usize) -> SysResult {
    if !syscall_logic::user_buffer_valid(addr, len)
        || !syscall_logic::user_buffer_valid(vec, pages(len))
    {
        return Err(SysError::EFAULT);
    }
    if len != 0 {
        unsafe {
            core::ptr::write_bytes(vec as *mut u8, 1, pages(len));
        }
    }
    Ok(0)
}

pub fn sys_readahead(fd: usize, _offset: usize, _count: usize) -> SysResult {
    if linux_fd_known(fd) {
        Ok(0)
    } else {
        Err(SysError::ENODEV)
    }
}

pub fn sys_fadvise64(fd: usize, _offset: usize, _len: usize, _advice: usize) -> SysResult {
    sys_readahead(fd, 0, 0)
}

// ============================================================================
// Zircon Process/Task Syscalls
// ============================================================================

fn process_profile_from_user_name(
    name_ptr: usize,
    name_len: usize,
) -> ZxResult<(ProcessRightProfile, &'static str)> {
    if !right::process_right_config_initialized() {
        return Err(ZxError::ErrBadState);
    }

    if name_ptr != 0 && name_len != 0 {
        let len = name_len.min(right::MAX_PROCESS_NAME_BYTES);
        let bytes = unsafe { core::slice::from_raw_parts(name_ptr as *const u8, len) };
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(bytes.len());
        if let Ok(name) = core::str::from_utf8(&bytes[..end]) {
            if !name.is_empty() {
                let profile = right::process_right_profile_for_name_checked(name)?;
                return Ok((profile, right::canonical_process_name(profile.kind)));
            }
        }
    }

    let profile = right::process_right_profile_for_name_checked("zircon_proc")?;
    Ok((profile, right::canonical_process_name(profile.kind)))
}

/// Zircon sys_process_create implementation
pub fn sys_process_create(
    job_handle: u32,
    name_ptr: usize,
    name_len: usize,
    options: u32,
    out_proc_handle: &mut u32,
    out_vmar_handle: &mut u32,
) -> ZxResult {
    info!(
        "process.create: job={:#x}, name_len={}",
        job_handle, name_len
    );

    if options != 0 || !syscall_logic::user_buffer_valid(name_ptr, name_len) {
        return Err(ZxError::ErrInvalidArgs);
    }
    if job_handle == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }
    {
        let state = memory_state();
        if state.get_job(job_handle).is_none() {
            return Err(ZxError::ErrNotFound);
        }
        if !state.handle_has_rights(job_handle, Rights::ManageProcess as u32) {
            return Err(ZxError::ErrAccessDenied);
        }
    }

    let (right_profile, process_name) = process_profile_from_user_name(name_ptr, name_len)?;
    if !right_profile.rights_valid() {
        return Err(ZxError::ErrInvalidArgs);
    }

    let pid = process_manager().create_process(process_name).unwrap_or(0);

    let state = memory_state();
    let proc_handle =
        state.alloc_object_handle_with_rights(ObjectType::Process, right_profile.process_rights)?;
    let vmar_handle =
        state.alloc_object_handle_with_rights(ObjectType::Vmar, right_profile.root_vmar_rights)?;
    let mut root_vmar = Vmar::new(ZIRCON_ROOT_VMAR_BASE, ZIRCON_ROOT_VMAR_SIZE);
    root_vmar.handle = HandleValue(vmar_handle);

    state.vmars.push(VmarRecord {
        handle: vmar_handle,
        vmar: root_vmar,
    });
    state.processes.push(ProcessRecord::new(
        proc_handle,
        job_handle,
        pid,
        vmar_handle,
        right_profile,
    ));
    if let Some(job) = state.get_job_mut(job_handle) {
        job.add_child();
    }

    *out_proc_handle = proc_handle;
    *out_vmar_handle = vmar_handle;
    Ok(())
}

/// Zircon sys_process_exit implementation
pub fn sys_process_exit(handle: u32, exit_code: i32) -> ZxResult {
    info!("process.exit: handle={:#x}, code={}", handle, exit_code);

    let state = memory_state();
    if !state.handle_has_rights(handle, Rights::Signal as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    let pid_to_terminate = {
        let proc = state.get_process_mut(handle).ok_or(ZxError::ErrNotFound)?;
        proc.mark_exited(exit_code)
    };
    if let Some(pid) = pid_to_terminate {
        let _ = process_manager().terminate_process(pid);
    }
    state.update_signal_value(handle, 0, ZX_SIGNAL_TERMINATED);
    Ok(())
}

/// Zircon sys_thread_create implementation
pub fn sys_thread_create(
    proc_handle: u32,
    name_ptr: usize,
    name_len: usize,
    entry_point: usize,
    _stack_size: usize,
    out_thread_handle: &mut u32,
) -> ZxResult {
    info!(
        "thread.create: proc={:#x}, name_len={}, entry={:#x}",
        proc_handle, name_len, entry_point
    );

    let state = memory_state();
    if !state.process_handle_known(proc_handle)
        || !syscall_logic::user_buffer_valid(name_ptr, name_len)
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !state.handle_has_rights(proc_handle, Rights::ManageThread as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    let process_object = state
        .resolve_handle(proc_handle, ObjectType::Process)
        .ok_or(ZxError::ErrInvalidArgs)?;
    let thread_rights = state
        .get_process_by_object(process_object)
        .map(|process| process.right_profile.thread_rights)
        .unwrap_or(Rights::DefaultThread as u32);
    let handle = state.alloc_object_handle_with_rights(ObjectType::Thread, thread_rights)?;
    state
        .threads
        .push(ThreadRecord::new(handle, process_object, entry_point));
    *out_thread_handle = handle;
    Ok(())
}

/// Zircon sys_thread_start implementation
pub fn sys_thread_start(
    thread_handle: u32,
    entry_point: usize,
    _stack_top: usize,
    _arg1: usize,
    _arg2: usize,
) -> ZxResult {
    info!(
        "thread.start: handle={:#x}, entry={:#x}",
        thread_handle, entry_point
    );

    let state = memory_state();
    if !state.handle_has_rights(thread_handle, Rights::Write as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    let thread = state
        .get_thread_mut(thread_handle)
        .ok_or(ZxError::ErrNotFound)?;
    if !thread.start(entry_point, _stack_top, _arg1, _arg2) {
        return Err(ZxError::ErrBadState);
    }
    Ok(())
}

/// Zircon sys_thread_exit implementation
pub fn sys_thread_exit() -> ZxResult {
    info!("thread.exit");
    Ok(())
}

/// Zircon sys_thread_read_state placeholder.
pub fn sys_thread_read_state(
    thread_handle: u32,
    _kind: u32,
    buffer: usize,
    buffer_size: usize,
) -> ZxResult {
    let state = memory_state();
    if !syscall_logic::user_buffer_valid(buffer, buffer_size) {
        return Err(ZxError::ErrInvalidArgs);
    }
    if state.get_thread_mut(thread_handle).is_none() {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !state.handle_has_rights(thread_handle, Rights::Read as u32) {
        return Err(ZxError::ErrAccessDenied);
    }

    if buffer_size >= core::mem::size_of::<u64>() {
        unsafe {
            core::ptr::write(buffer as *mut u64, thread_handle as u64);
        }
    }
    Ok(())
}

/// Zircon sys_thread_write_state placeholder.
pub fn sys_thread_write_state(
    thread_handle: u32,
    _kind: u32,
    buffer: usize,
    buffer_size: usize,
) -> ZxResult {
    let state = memory_state();
    if !syscall_logic::user_buffer_valid(buffer, buffer_size) {
        return Err(ZxError::ErrInvalidArgs);
    }
    if state.get_thread_mut(thread_handle).is_none() {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !state.handle_has_rights(thread_handle, Rights::Write as u32) {
        return Err(ZxError::ErrAccessDenied);
    }

    Ok(())
}

/// Zircon sys_task_kill implementation
pub fn sys_task_kill(task_handle: u32) -> ZxResult {
    info!("task.kill: handle={:#x}", task_handle);

    {
        let state = memory_state();
        if state.task_handle_known(task_handle)
            && !state.handle_has_rights(task_handle, Rights::Destroy as u32)
        {
            return Err(ZxError::ErrAccessDenied);
        }
        if let Some(proc) = state.get_process_mut(task_handle) {
            let pid_to_terminate = proc.mark_exited(0);
            if let Some(pid) = pid_to_terminate {
                let _ = process_manager().terminate_process(pid);
            }
            state.update_signal_value(task_handle, 0, ZX_SIGNAL_TERMINATED);
            return Ok(());
        }
        if let Some(thread) = state.get_thread_mut(task_handle) {
            thread.mark_exited();
            state.update_signal_value(task_handle, 0, ZX_SIGNAL_TERMINATED);
            return Ok(());
        }
    }

    sys_handle_close(task_handle)
}

/// Zircon sys_process_start placeholder.
pub fn sys_process_start(
    proc_handle: u32,
    thread_handle: u32,
    entry: usize,
    stack: usize,
    arg1: usize,
    arg2: usize,
) -> ZxResult {
    let state = memory_state();
    if !state.process_handle_known(proc_handle) {
        return Err(ZxError::ErrNotFound);
    }
    if !state.handle_has_rights(proc_handle, Rights::ManageThread as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    let Some(thread_record) = state.handle_record(thread_handle) else {
        return Err(ZxError::ErrNotFound);
    };
    let proc_object = state
        .resolve_handle(proc_handle, ObjectType::Process)
        .ok_or(ZxError::ErrNotFound)?;
    if thread_record.obj_type != ObjectType::Thread {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !state.threads.iter().any(|thread| {
        thread.handle == thread_record.object_handle && thread.process_handle == proc_object
    }) {
        return Err(ZxError::ErrInvalidArgs);
    }
    sys_thread_start(thread_handle, entry, stack, arg1, arg2)
}

/// Zircon sys_process_read_memory placeholder.
pub fn sys_process_read_memory(
    proc_handle: u32,
    _vaddr: usize,
    buffer: usize,
    len: usize,
) -> ZxResult<usize> {
    let state = memory_state();
    if !state.process_handle_known(proc_handle) || !syscall_logic::user_buffer_valid(buffer, len) {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !state.handle_has_rights(proc_handle, Rights::Read as u32) {
        return Err(ZxError::ErrAccessDenied);
    }

    if len != 0 {
        let out = unsafe { core::slice::from_raw_parts_mut(buffer as *mut u8, len) };
        out.fill(0);
    }
    Ok(len)
}

/// Zircon sys_process_write_memory placeholder.
pub fn sys_process_write_memory(
    proc_handle: u32,
    _vaddr: usize,
    buffer: usize,
    len: usize,
) -> ZxResult<usize> {
    let state = memory_state();
    if !state.process_handle_known(proc_handle) || !syscall_logic::user_buffer_valid(buffer, len) {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !state.handle_has_rights(proc_handle, Rights::Write as u32) {
        return Err(ZxError::ErrAccessDenied);
    }

    Ok(len)
}

/// Zircon sys_job_create implementation.
pub fn sys_job_create(parent_job: u32, options: u32, out_handle: &mut u32) -> ZxResult {
    if options != 0 {
        return Err(ZxError::ErrInvalidArgs);
    }
    if parent_job != 0 {
        let state = memory_state();
        if state.get_job(parent_job).is_none() {
            return Err(ZxError::ErrNotFound);
        }
        if !state.handle_has_rights(parent_job, Rights::ManageJob as u32) {
            return Err(ZxError::ErrAccessDenied);
        }
    }

    let state = memory_state();
    let parent_object = if parent_job == 0 {
        None
    } else {
        state.resolve_handle(parent_job, ObjectType::Job)
    };
    let handle = state.alloc_object_handle(ObjectType::Job);
    state.jobs.push(JobRecord::new(handle, parent_object));
    if parent_job != 0 {
        if let Some(parent) = state.get_job_mut(parent_job) {
            parent.add_child();
        }
    }
    *out_handle = handle;
    Ok(())
}

pub fn sys_job_set_policy(
    job_handle: u32,
    _options: u32,
    _topic: u32,
    policy_ptr: usize,
    policy_count: usize,
) -> ZxResult {
    if !syscall_logic::user_buffer_valid(policy_ptr, policy_count) {
        return Err(ZxError::ErrInvalidArgs);
    }
    let state = memory_state();
    if !state.handle_has_rights(job_handle, Rights::SetPolicy as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    let Some(job) = state.get_job_mut(job_handle) else {
        return Err(ZxError::ErrNotFound);
    };
    job.set_policy_count(policy_count);
    Ok(())
}

pub fn sys_job_set_critical(job_handle: u32, _options: u32, proc_handle: u32) -> ZxResult {
    let state = memory_state();
    if !state.process_handle_known(proc_handle) {
        return Err(ZxError::ErrInvalidArgs);
    }
    if state.get_job(job_handle).is_none() {
        return Err(ZxError::ErrNotFound);
    }
    if !state.handle_has_rights(job_handle, Rights::SetPolicy as u32)
        || !state.handle_has_rights(proc_handle, Rights::GetProperty as u32)
    {
        return Err(ZxError::ErrAccessDenied);
    }
    let proc_object = state
        .resolve_handle(proc_handle, ObjectType::Process)
        .ok_or(ZxError::ErrInvalidArgs)?;
    if let Some(job) = state.get_job_mut(job_handle) {
        job.set_critical_process(proc_object);
    }
    Ok(())
}

pub fn sys_task_bind_exception_port(
    task_handle: u32,
    port_handle: u32,
    _key: u64,
    options: u32,
) -> ZxResult {
    if !syscall_logic::zircon_exception_channel_options_valid(
        options,
        ZX_EXCEPTION_CHANNEL_OPTIONS_MASK,
    ) {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !kernel_object_handle_known(task_handle)
        || (port_handle != INVALID_HANDLE && !kernel_object_handle_known(port_handle))
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !handle_has_rights(task_handle, Rights::Inspect as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    if port_handle != INVALID_HANDLE && !handle_has_rights(port_handle, Rights::Write as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    Ok(())
}

pub fn sys_task_resume_from_exception(task_handle: u32, exception: u32, options: u32) -> ZxResult {
    if options != 0 {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !kernel_object_handle_known(task_handle)
        || !compat::table().is_type(HandleValue(exception), ObjectType::Exception)
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !handle_has_rights(task_handle, Rights::Inspect as u32)
        || !handle_has_rights(exception, Rights::Inspect as u32)
    {
        return Err(ZxError::ErrAccessDenied);
    }
    let _ = compat::table().update_signals(HandleValue(exception), ZX_USER_SIGNAL_0, 0);
    Ok(())
}

pub fn sys_task_suspend_token(task_handle: u32, out_handle: &mut u32) -> ZxResult {
    if !kernel_object_handle_known(task_handle) {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !handle_has_rights(task_handle, Rights::Inspect as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    *out_handle = compat::create_object(ObjectType::SuspendToken)?.0;
    Ok(())
}

// ============================================================================
// Handle Syscalls (Zircon)
// ============================================================================

/// Zircon sys_handle_close implementation
pub fn sys_handle_close(handle: u32) -> ZxResult {
    info!("handle.close: handle={:#x}", handle);

    if syscall_logic::handle_invalid(handle, INVALID_HANDLE) {
        return Err(ZxError::ErrInvalidArgs);
    }

    if memory_state().release_handle(handle)
        || channel::channel_table().remove_channel(HandleValue(handle))
        || fifo::fifo_table().close(HandleValue(handle))
        || port::port_table().close(HandleValue(handle))
        || socket::socket_table().close(HandleValue(handle))
        || compat::close_handle(HandleValue(handle))
    {
        Ok(())
    } else {
        Err(ZxError::ErrNotFound)
    }
}

/// Zircon sys_handle_close_many implementation
pub fn sys_handle_close_many(handles_ptr: usize, num_handles: usize) -> ZxResult {
    info!(
        "handle.close_many: ptr={:#x}, count={}",
        handles_ptr, num_handles
    );

    if num_handles == 0 {
        return Ok(());
    }
    if handles_ptr == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }

    let handles = unsafe { core::slice::from_raw_parts(handles_ptr as *const u32, num_handles) };
    for handle in handles {
        sys_handle_close(*handle)?;
    }

    Ok(())
}

/// Zircon sys_handle_duplicate implementation
pub fn sys_handle_duplicate(handle: u32, rights: u32, out_handle: &mut u32) -> ZxResult {
    info!(
        "handle.duplicate: handle={:#x}, rights={:#x}",
        handle, rights
    );

    if !kernel_object_handle_known(handle) {
        return Err(ZxError::ErrInvalidArgs);
    }
    if memory_state().live_handle_known(handle) {
        *out_handle = memory_state().duplicate_handle(handle, rights)?;
        return Ok(());
    }
    if channel_handle_known(handle) {
        return Err(ZxError::ErrNotSupported);
    }
    let existing_rights = handle_known_rights(handle).ok_or(ZxError::ErrInvalidArgs)?;
    if !crate::kernel_objects::object_logic::duplicate_rights_allowed(
        existing_rights,
        rights,
        Rights::Duplicate as u32,
        RIGHT_SAME_RIGHTS,
        crate::kernel_objects::RIGHTS_ALL,
    ) {
        return Err(ZxError::ErrAccessDenied);
    }
    Err(ZxError::ErrNotSupported)
}

/// Zircon sys_handle_replace implementation
pub fn sys_handle_replace(handle: u32, rights: u32, out_handle: &mut u32) -> ZxResult {
    info!("handle.replace: handle={:#x}, rights={:#x}", handle, rights);

    if !kernel_object_handle_known(handle) {
        return Err(ZxError::ErrInvalidArgs);
    }
    if memory_state().live_handle_known(handle) {
        *out_handle = memory_state().replace_handle(handle, rights)?;
        return Ok(());
    }
    if channel_handle_known(handle) {
        channel::channel_table().duplicate_endpoint_rights(HandleValue(handle), rights, true)?;
        *out_handle = handle;
        return Ok(());
    }
    let existing_rights = handle_known_rights(handle).ok_or(ZxError::ErrInvalidArgs)?;
    if !crate::kernel_objects::object_logic::replace_rights_allowed(
        existing_rights,
        rights,
        RIGHT_SAME_RIGHTS,
        crate::kernel_objects::RIGHTS_ALL,
    ) {
        return Err(ZxError::ErrAccessDenied);
    }
    Err(ZxError::ErrNotSupported)
}

pub fn sys_channel_write_etc(
    handle: u32,
    options: u32,
    bytes_ptr: usize,
    bytes_count: usize,
    handles_ptr: usize,
    handles_count: usize,
) -> ZxResult {
    sys_channel_write(
        handle,
        options,
        bytes_ptr,
        bytes_count,
        handles_ptr,
        handles_count,
    )
}

pub fn sys_channel_call_finish(
    _deadline: u64,
    _args: usize,
    _actual_bytes: usize,
    _actual_handles: usize,
) -> ZxResult {
    Err(ZxError::ErrBadState)
}

// ============================================================================
// Object Syscalls (Zircon)
// ============================================================================

/// Zircon sys_object_wait_one implementation
pub fn sys_object_wait_one(
    handle: u32,
    signals: u32,
    deadline: u64,
    out_pending: &mut u32,
) -> ZxResult {
    info!(
        "object.wait_one: handle={:#x}, signals={:#x}",
        handle, signals
    );

    let observed = object_signal_state(handle)?;
    if !handle_has_rights(handle, Rights::Wait as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    *out_pending = observed;

    if syscall_logic::wait_satisfied(observed, signals) || deadline != 0 {
        Ok(())
    } else {
        Err(ZxError::ErrTimedOut)
    }
}

/// Zircon sys_object_wait_many implementation
pub fn sys_object_wait_many(items_ptr: usize, count: usize, deadline: u64) -> ZxResult {
    info!(
        "object.wait_many: count={}, deadline={:#x}",
        count, deadline
    );

    if count == 0 {
        return Ok(());
    }
    if !syscall_logic::user_buffer_valid(
        items_ptr,
        count.saturating_mul(core::mem::size_of::<ZxWaitItem>()),
    ) {
        return Err(ZxError::ErrInvalidArgs);
    }

    let items = unsafe { core::slice::from_raw_parts_mut(items_ptr as *mut ZxWaitItem, count) };
    let mut satisfied = false;

    for item in items.iter_mut() {
        if !handle_has_rights(item.handle, Rights::Wait as u32) {
            return Err(ZxError::ErrAccessDenied);
        }
        let observed = object_signal_state(item.handle)?;
        item.pending = observed;
        if syscall_logic::wait_satisfied(observed, item.waitfor) {
            satisfied = true;
        }
    }

    if satisfied || deadline != 0 {
        Ok(())
    } else {
        Err(ZxError::ErrTimedOut)
    }
}

/// Zircon sys_object_signal implementation
pub fn sys_object_signal(handle: u32, clear_mask: u32, set_mask: u32) -> ZxResult {
    info!(
        "object.signal: handle={:#x}, clear={:#x}, set={:#x}",
        handle, clear_mask, set_mask
    );

    if !handle_has_rights(handle, Rights::Signal as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    set_object_signal_state(handle, clear_mask, set_mask).map(|_| ())
}

/// Zircon sys_object_signal_peer implementation.
pub fn sys_object_signal_peer(handle: u32, clear_mask: u32, set_mask: u32) -> ZxResult {
    info!(
        "object.signal_peer: handle={:#x}, clear={:#x}, set={:#x}",
        handle, clear_mask, set_mask
    );

    if !handle_has_rights(handle, Rights::SignalPeer as u32) {
        return Err(ZxError::ErrAccessDenied);
    }

    if socket::socket_table().contains(HandleValue(handle)) {
        if !syscall_logic::signal_mask_allowed(
            clear_mask,
            set_mask,
            syscall_logic::user_signal_mask(),
        ) {
            return Err(ZxError::ErrInvalidArgs);
        }
        return socket::socket_table()
            .signal_peer(HandleValue(handle), clear_mask, set_mask)
            .map(|_| ());
    }
    if fifo::fifo_table().contains(HandleValue(handle)) {
        if !syscall_logic::signal_mask_allowed(
            clear_mask,
            set_mask,
            syscall_logic::user_signal_mask(),
        ) {
            return Err(ZxError::ErrInvalidArgs);
        }
        return fifo::fifo_table()
            .signal_peer(HandleValue(handle), clear_mask, set_mask)
            .map(|_| ());
    }

    compat::table()
        .signal_peer(HandleValue(handle), clear_mask, set_mask)
        .map(|_| ())
}

/// Zircon sys_object_wait_async implementation.
pub fn sys_object_wait_async(
    handle: u32,
    port_handle: u32,
    key: u64,
    signals: u32,
    options: u32,
) -> ZxResult {
    info!(
        "object.wait_async: handle={:#x}, port={:#x}, signals={:#x}",
        handle, port_handle, signals
    );

    if !crate::kernel_objects::port_logic::wait_async_options_valid(
        options,
        port::WAIT_ASYNC_OPTIONS_MASK,
        port::WAIT_ASYNC_TIMESTAMP,
        port::WAIT_ASYNC_BOOT_TIMESTAMP,
    ) || !kernel_object_handle_known(handle)
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !handle_has_rights(handle, Rights::Wait as u32)
        || !handle_has_rights(port_handle, Rights::Write as u32)
    {
        return Err(ZxError::ErrAccessDenied);
    }

    let observed = object_signal_state(handle)?;
    port::port_table()
        .queue_signal(
            HandleValue(port_handle),
            HandleValue(handle),
            key,
            observed,
            signals,
            options,
        )
        .map(|_| ())
        .map_err(|err| {
            if err == ZxError::ErrNotFound {
                ZxError::ErrInvalidArgs
            } else {
                err
            }
        })?;
    Ok(())
}

/// Zircon sys_object_get_child placeholder.
pub fn sys_object_get_child(
    handle: u32,
    koid: u64,
    _rights: u32,
    out_handle: &mut u32,
) -> ZxResult {
    if !kernel_object_handle_known(handle) {
        return Err(ZxError::ErrNotFound);
    }
    if !handle_has_rights(handle, Rights::Inspect as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    if koid == 0 || koid == handle as u64 {
        *out_handle = handle;
        Ok(())
    } else {
        Err(ZxError::ErrNotFound)
    }
}

/// Zircon sys_object_get_info implementation
pub fn sys_object_get_info(
    handle: u32,
    topic: u32,
    buffer: usize,
    buffer_size: usize,
    out_actual_size: &mut usize,
) -> ZxResult {
    info!("object.get_info: handle={:#x}, topic={:#x}", handle, topic);

    if !kernel_object_handle_known(handle) || !syscall_logic::user_buffer_valid(buffer, buffer_size)
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !handle_has_rights(handle, Rights::Inspect as u32) {
        return Err(ZxError::ErrAccessDenied);
    }

    if topic == socket::OBJECT_INFO_TOPIC_SOCKET {
        let info = socket::socket_table()
            .info(HandleValue(handle))
            .ok_or(ZxError::ErrNotFound)?;
        let info_size = core::mem::size_of::<socket::SocketInfo>();
        *out_actual_size = info_size;
        if buffer_size >= info_size {
            unsafe {
                core::ptr::write(buffer as *mut socket::SocketInfo, info);
            }
        }
        return Ok(());
    }

    *out_actual_size = core::mem::size_of::<u64>();
    if buffer_size >= core::mem::size_of::<u64>() {
        unsafe {
            core::ptr::write(buffer as *mut u64, ((topic as u64) << 32) | handle as u64);
        }
    }
    Ok(())
}

/// Zircon sys_object_get_property implementation
pub fn sys_object_get_property(
    handle: u32,
    prop_id: u32,
    buffer: usize,
    buffer_size: usize,
) -> ZxResult {
    info!(
        "object.get_property: handle={:#x}, prop={:#x}",
        handle, prop_id
    );

    if !kernel_object_handle_known(handle)
        || !syscall_logic::user_buffer_valid(buffer, buffer_size)
        || buffer_size < core::mem::size_of::<u64>()
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !handle_has_rights(handle, Rights::GetProperty as u32) {
        return Err(ZxError::ErrAccessDenied);
    }

    let value = socket::socket_table()
        .property(HandleValue(handle), prop_id)
        .or_else(|| compat::table().property(HandleValue(handle)))
        .unwrap_or_else(|| memory_state().get_property_value(handle));
    unsafe {
        core::ptr::write(buffer as *mut u64, value);
    }
    Ok(())
}

/// Zircon sys_object_set_property implementation
pub fn sys_object_set_property(
    handle: u32,
    prop_id: u32,
    buffer: usize,
    buffer_size: usize,
) -> ZxResult {
    info!(
        "object.set_property: handle={:#x}, prop={:#x}",
        handle, prop_id
    );

    if !kernel_object_handle_known(handle)
        || !syscall_logic::user_buffer_valid(buffer, buffer_size)
        || buffer_size < core::mem::size_of::<u64>()
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !handle_has_rights(handle, Rights::SetProperty as u32) {
        return Err(ZxError::ErrAccessDenied);
    }

    let value = unsafe { core::ptr::read(buffer as *const u64) };
    match socket::socket_table().set_property(HandleValue(handle), prop_id, value) {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(e) => return Err(e),
    }
    if !compat::table().set_property(HandleValue(handle), value) {
        memory_state().set_property_value(handle, value);
    }
    Ok(())
}

pub fn sys_object_set_profile(handle: u32, profile: u32, _options: u32) -> ZxResult {
    if !kernel_object_handle_known(handle) || !kernel_object_handle_known(profile) {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !handle_has_rights(handle, Rights::SetProperty as u32) {
        return Err(ZxError::ErrAccessDenied);
    }
    Ok(())
}

// ============================================================================
// Lightweight Zircon Compatibility Objects
// ============================================================================

pub fn sys_event_create(options: u32, out_handle: &mut u32) -> ZxResult {
    if options != 0 {
        return Err(ZxError::ErrInvalidArgs);
    }
    *out_handle = compat::create_object(ObjectType::Event)?.0;
    Ok(())
}

pub fn sys_eventpair_create(
    options: u32,
    out_handle0: &mut u32,
    out_handle1: &mut u32,
) -> ZxResult {
    if options != 0 {
        return Err(ZxError::ErrInvalidArgs);
    }
    let (h0, h1) = compat::create_pair(ObjectType::EventPair)?;
    *out_handle0 = h0.0;
    *out_handle1 = h1.0;
    Ok(())
}

pub fn sys_socket_create(options: u32, out_handle0: &mut u32, out_handle1: &mut u32) -> ZxResult {
    let (h0, h1) = socket::socket_table().create_pair(options)?;
    *out_handle0 = h0.0;
    *out_handle1 = h1.0;
    Ok(())
}

pub fn sys_socket_write(
    handle: u32,
    options: u32,
    buffer: usize,
    len: usize,
    out_actual: &mut usize,
) -> ZxResult {
    if options != 0 || !syscall_logic::user_buffer_valid(buffer, len) {
        return Err(ZxError::ErrInvalidArgs);
    }
    let bytes = if len == 0 {
        &[][..]
    } else {
        unsafe { core::slice::from_raw_parts(buffer as *const u8, len) }
    };
    *out_actual = socket::socket_table().write(HandleValue(handle), bytes)?;
    Ok(())
}

pub fn sys_socket_read(
    handle: u32,
    options: u32,
    buffer: usize,
    len: usize,
    out_actual: &mut usize,
) -> ZxResult {
    if !syscall_logic::user_buffer_valid(buffer, len) {
        return Err(ZxError::ErrInvalidArgs);
    }
    let out = if len == 0 {
        &mut [][..]
    } else {
        unsafe { core::slice::from_raw_parts_mut(buffer as *mut u8, len) }
    };
    *out_actual = socket::socket_table().read(HandleValue(handle), options, out)?;
    Ok(())
}

pub fn sys_socket_share(handle: u32, socket_to_share: u32) -> ZxResult {
    socket::socket_table().share(HandleValue(handle), HandleValue(socket_to_share))
}

pub fn sys_socket_accept(handle: u32, out_handle: &mut u32) -> ZxResult {
    *out_handle = socket::socket_table().accept(HandleValue(handle))?.0;
    Ok(())
}

pub fn sys_socket_shutdown(handle: u32, options: u32) -> ZxResult {
    socket::socket_table().shutdown(HandleValue(handle), options)
}

pub fn sys_fifo_create(
    elem_count: usize,
    elem_size: usize,
    options: u32,
    out_handle0: &mut u32,
    out_handle1: &mut u32,
) -> ZxResult {
    let (h0, h1) = fifo::fifo_table().create_pair(elem_count, elem_size, options)?;
    *out_handle0 = h0.0;
    *out_handle1 = h1.0;
    Ok(())
}

pub fn sys_fifo_write(
    handle: u32,
    elem_size: usize,
    buffer: usize,
    count: usize,
    out_actual: &mut usize,
) -> ZxResult {
    let byte_len = fifo_logic::transfer_bytes(elem_size, count).ok_or(ZxError::ErrOutOfRange)?;
    if elem_size == 0 || !syscall_logic::user_buffer_valid(buffer, byte_len) {
        return Err(ZxError::ErrInvalidArgs);
    }
    let bytes = if byte_len == 0 {
        &[][..]
    } else {
        unsafe { core::slice::from_raw_parts(buffer as *const u8, byte_len) }
    };
    *out_actual = fifo::fifo_table().write(HandleValue(handle), elem_size, bytes)?;
    Ok(())
}

pub fn sys_fifo_read(
    handle: u32,
    elem_size: usize,
    buffer: usize,
    count: usize,
    out_actual: &mut usize,
) -> ZxResult {
    let byte_len = fifo_logic::transfer_bytes(elem_size, count).ok_or(ZxError::ErrOutOfRange)?;
    if elem_size == 0 || !syscall_logic::user_buffer_valid(buffer, byte_len) {
        return Err(ZxError::ErrInvalidArgs);
    }
    let out = if byte_len == 0 {
        &mut [][..]
    } else {
        unsafe { core::slice::from_raw_parts_mut(buffer as *mut u8, byte_len) }
    };
    *out_actual = fifo::fifo_table().read(HandleValue(handle), elem_size, out)?;
    Ok(())
}

pub fn sys_port_create(options: u32, out_handle: &mut u32) -> ZxResult {
    *out_handle = port::port_table().create(options)?.0;
    Ok(())
}

pub fn sys_port_queue(port_handle: u32, packet: usize) -> ZxResult {
    if !port::port_table().contains(HandleValue(port_handle))
        || !crate::kernel_objects::port_logic::packet_ptr_valid(packet, port::PORT_PACKET_SIZE)
        || !syscall_logic::user_buffer_valid(packet, port::PORT_PACKET_SIZE)
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    let queued = unsafe { core::ptr::read(packet as *const port::PortPacket) };
    port::port_table().queue(HandleValue(port_handle), queued)?;
    Ok(())
}

pub fn sys_port_wait(port_handle: u32, deadline: u64, packet: usize) -> ZxResult {
    if !port::port_table().contains(HandleValue(port_handle))
        || !crate::kernel_objects::port_logic::packet_ptr_valid(packet, port::PORT_PACKET_SIZE)
        || !syscall_logic::user_buffer_valid(packet, port::PORT_PACKET_SIZE)
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    let received = port::port_table().wait(HandleValue(port_handle), deadline)?;
    unsafe {
        core::ptr::write(packet as *mut port::PortPacket, received);
    }
    Ok(())
}

pub fn sys_port_cancel(port_handle: u32, source: u32, key: u64) -> ZxResult<u32> {
    if !port::port_table().contains(HandleValue(port_handle)) || !kernel_object_handle_known(source)
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    port::port_table().cancel(HandleValue(port_handle), HandleValue(source), key)
}

pub fn sys_profile_create(
    _root_resource: u32,
    options: u32,
    _profile: usize,
    out_handle: &mut u32,
) -> ZxResult {
    if options != 0 {
        return Err(ZxError::ErrInvalidArgs);
    }
    *out_handle = compat::create_object(ObjectType::Profile)?.0;
    Ok(())
}

pub fn sys_timer_create(options: u32, _clock_id: u32, out_handle: &mut u32) -> ZxResult {
    if !syscall_logic::zircon_timer_options_valid(options, ZX_TIMER_OPTIONS_MASK) {
        return Err(ZxError::ErrInvalidArgs);
    }
    *out_handle = compat::create_object_with_options(ObjectType::Timer, options)?.0;
    Ok(())
}

pub fn sys_timer_set(handle: u32, deadline: u64, slack: i64) -> ZxResult {
    if slack < 0 || !compat::table().is_type(HandleValue(handle), ObjectType::Timer) {
        return Err(ZxError::ErrInvalidArgs);
    }

    let now = monotonic_nanos();
    let set_mask = if syscall_logic::zircon_timer_deadline_expired(deadline, now) {
        ZX_TIMER_SIGNALED
    } else {
        0
    };

    if !compat::table().set_state_value(HandleValue(handle), deadline) {
        return Err(ZxError::ErrNotFound);
    }
    compat::table()
        .update_signals(HandleValue(handle), ZX_TIMER_SIGNALED, set_mask)
        .ok_or(ZxError::ErrNotFound)?;
    Ok(())
}

pub fn sys_timer_cancel(handle: u32) -> ZxResult {
    if !compat::table().is_type(HandleValue(handle), ObjectType::Timer) {
        return Err(ZxError::ErrInvalidArgs);
    }
    let _ = compat::table().set_state_value(HandleValue(handle), 0);
    if compat::table()
        .update_signals(HandleValue(handle), ZX_TIMER_SIGNALED, 0)
        .is_some()
    {
        Ok(())
    } else {
        Err(ZxError::ErrNotFound)
    }
}

pub fn sys_debuglog_create(_resource: u32, options: u32, out_handle: &mut u32) -> ZxResult {
    if !syscall_logic::zircon_debuglog_create_options_valid(
        options,
        ZX_DEBUGLOG_CREATE_OPTIONS_MASK,
    ) {
        return Err(ZxError::ErrInvalidArgs);
    }
    *out_handle = compat::create_object_with_options(ObjectType::DebugLog, options)?.0;
    Ok(())
}

pub fn sys_debuglog_write(handle: u32, options: u32, buffer: usize, len: usize) -> ZxResult {
    if !syscall_logic::zircon_debuglog_io_options_valid(options, ZX_DEBUGLOG_OPTIONS_MASK)
        || !compat::table().is_type(HandleValue(handle), ObjectType::DebugLog)
        || !syscall_logic::user_buffer_valid(buffer, len)
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    let bytes = if len == 0 {
        &[][..]
    } else {
        unsafe { core::slice::from_raw_parts(buffer as *const u8, len) }
    };
    let _ = compat::table().write_bytes(HandleValue(handle), bytes)?;
    let _ = sys_write(1, buffer, len);
    Ok(())
}

pub fn sys_debuglog_read(handle: u32, options: u32, buffer: usize, len: usize) -> ZxResult<usize> {
    if !syscall_logic::zircon_debuglog_io_options_valid(options, ZX_DEBUGLOG_OPTIONS_MASK)
        || !compat::table().is_type(HandleValue(handle), ObjectType::DebugLog)
        || !syscall_logic::user_buffer_valid(buffer, len)
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    let out = if len == 0 {
        &mut [][..]
    } else {
        unsafe { core::slice::from_raw_parts_mut(buffer as *mut u8, len) }
    };
    compat::table().read_bytes(HandleValue(handle), out)
}

pub fn sys_resource_create(
    parent: u32,
    options: u32,
    _base: u64,
    _size: usize,
    _name: usize,
    name_len: usize,
    out_handle: &mut u32,
) -> ZxResult {
    if options != 0 || (parent != 0 && !kernel_object_handle_known(parent)) {
        return Err(ZxError::ErrInvalidArgs);
    }
    if name_len > 32 {
        return Err(ZxError::ErrOutOfRange);
    }
    *out_handle = compat::create_object(ObjectType::Resource)?.0;
    Ok(())
}

pub fn sys_cprng_draw_once(buffer: usize, len: usize) -> ZxResult {
    if !syscall_logic::user_buffer_valid(buffer, len) {
        return Err(ZxError::ErrInvalidArgs);
    }
    if len != 0 {
        let seed = monotonic_nanos() as u8;
        let out = unsafe { core::slice::from_raw_parts_mut(buffer as *mut u8, len) };
        for (index, byte) in out.iter_mut().enumerate() {
            *byte = seed
                .wrapping_add((index as u8).wrapping_mul(17))
                .wrapping_add(0x5A);
        }
    }
    Ok(())
}

pub fn sys_cprng_add_entropy(buffer: usize, len: usize) -> ZxResult {
    if syscall_logic::user_buffer_valid(buffer, len) {
        Ok(())
    } else {
        Err(ZxError::ErrInvalidArgs)
    }
}

pub fn sys_debug_read(buffer: usize, len: usize) -> ZxResult<usize> {
    if !syscall_logic::user_buffer_valid(buffer, len) {
        return Err(ZxError::ErrInvalidArgs);
    }
    if len != 0 {
        unsafe {
            core::ptr::write_bytes(buffer as *mut u8, 0, len);
        }
    }
    Ok(0)
}

pub fn sys_debug_write(buffer: usize, len: usize) -> ZxResult {
    if !syscall_logic::user_buffer_valid(buffer, len) {
        return Err(ZxError::ErrInvalidArgs);
    }
    sys_write(1, buffer, len)
        .map(|_| ())
        .map_err(|_| ZxError::ErrInvalidArgs)
}

pub fn sys_debug_send_command(buffer: usize, len: usize) -> ZxResult {
    if syscall_logic::user_buffer_valid(buffer, len) {
        Ok(())
    } else {
        Err(ZxError::ErrInvalidArgs)
    }
}

pub fn sys_ktrace_read(handle: u32, buffer: usize, len: usize, out_actual: &mut usize) -> ZxResult {
    if handle != 0 && !kernel_object_handle_known(handle) {
        return Err(ZxError::ErrNotFound);
    }
    *out_actual = sys_debug_read(buffer, len)?;
    Ok(())
}

pub fn sys_ktrace_control(_handle: u32, _action: u32, _options: u32, _ptr: usize) -> ZxResult {
    Ok(())
}

pub fn sys_ktrace_write(_handle: u32, _id: u32, _arg0: u64, _arg1: u64) -> ZxResult {
    Ok(())
}

pub fn sys_mtrace_control(
    _handle: u32,
    _kind: u32,
    _action: u32,
    _options: u32,
    _ptr: usize,
) -> ZxResult {
    Ok(())
}

pub fn sys_system_get_event(
    _root_resource: u32,
    event_kind: u32,
    out_handle: &mut u32,
) -> ZxResult {
    if !syscall_logic::zircon_system_event_kind_valid(event_kind, ZX_SYSTEM_EVENT_KIND_MAX) {
        return Err(ZxError::ErrInvalidArgs);
    }
    *out_handle = compat::create_object(ObjectType::Event)?.0;
    Ok(())
}

pub fn sys_create_exception_channel(task: u32, options: u32, out_handle: &mut u32) -> ZxResult {
    if !syscall_logic::zircon_exception_channel_options_valid(
        options,
        ZX_EXCEPTION_CHANNEL_OPTIONS_MASK,
    ) || !kernel_object_handle_known(task)
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    let mut h0 = 0u32;
    let mut h1 = 0u32;
    sys_channel_create(0, &mut h0, &mut h1)?;
    let exception = compat::create_object(ObjectType::Exception)?;
    let handles = [exception.0];
    let packet = [0u8; 8];
    let _ = sys_channel_write(
        h1,
        0,
        packet.as_ptr() as usize,
        packet.len(),
        handles.as_ptr() as usize,
        handles.len(),
    );
    *out_handle = h0;
    Ok(())
}

pub fn sys_exception_get_thread(exception: u32, out_handle: &mut u32) -> ZxResult {
    if !compat::table().is_type(HandleValue(exception), ObjectType::Exception) {
        return Err(ZxError::ErrNotFound);
    }
    *out_handle = compat::create_object(ObjectType::Thread)?.0;
    Ok(())
}

pub fn sys_exception_get_process(exception: u32, out_handle: &mut u32) -> ZxResult {
    if !compat::table().is_type(HandleValue(exception), ObjectType::Exception) {
        return Err(ZxError::ErrNotFound);
    }
    *out_handle = compat::create_object(ObjectType::Process)?.0;
    Ok(())
}

pub fn sys_iommu_create(
    _resource: u32,
    type_: u32,
    desc: usize,
    desc_size: usize,
    out_handle: &mut u32,
) -> ZxResult {
    if desc_size != 0 && desc == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }
    if type_ != 0 {
        return Err(ZxError::ErrNotSupported);
    }
    *out_handle = compat::create_object(ObjectType::Iommu)?.0;
    Ok(())
}

pub fn sys_bti_create(iommu: u32, options: u32, _bti_id: u64, out_handle: &mut u32) -> ZxResult {
    if options != 0 || !kernel_object_handle_known(iommu) {
        return Err(ZxError::ErrInvalidArgs);
    }
    *out_handle = compat::create_object(ObjectType::Bti)?.0;
    Ok(())
}

pub fn sys_bti_pin(
    bti: u32,
    _options: u32,
    vmo: u32,
    offset: usize,
    size: usize,
    addrs: usize,
    addrs_count: usize,
    out_handle: &mut u32,
) -> ZxResult {
    if !kernel_object_handle_known(bti) || memory_state().get_vmo(vmo).is_none() {
        return Err(ZxError::ErrNotFound);
    }
    if !page_aligned(offset) || !page_aligned(size) {
        return Err(ZxError::ErrInvalidArgs);
    }
    if addrs_count != 0 && addrs == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }
    for index in 0..addrs_count {
        unsafe {
            core::ptr::write(
                (addrs as *mut u64).add(index),
                (offset + index * PAGE_SIZE) as u64,
            );
        }
    }
    *out_handle = compat::create_object(ObjectType::Pmt)?.0;
    Ok(())
}

pub fn sys_pmt_unpin(pmt: u32) -> ZxResult {
    sys_handle_close(pmt).or(Ok(()))
}

pub fn sys_bti_release_quarantine(bti: u32) -> ZxResult {
    if kernel_object_handle_known(bti) {
        Ok(())
    } else {
        Err(ZxError::ErrNotFound)
    }
}

pub fn sys_pc_firmware_tables(_resource: u32, acpi_rsdp_ptr: usize, smbios_ptr: usize) -> ZxResult {
    if acpi_rsdp_ptr != 0 {
        unsafe {
            core::ptr::write(acpi_rsdp_ptr as *mut u64, 0);
        }
    }
    if smbios_ptr != 0 {
        unsafe {
            core::ptr::write(smbios_ptr as *mut u64, 0);
        }
    }
    Err(ZxError::ErrNotSupported)
}

pub fn sys_framebuffer_get_info(info: usize) -> ZxResult {
    if info == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }
    unsafe {
        core::ptr::write_bytes(info as *mut u8, 0, 32);
    }
    Ok(())
}

pub fn sys_framebuffer_set_range(_vmo: u32, _len: usize, _format: u32) -> ZxResult {
    Err(ZxError::ErrNotSupported)
}

pub fn sys_interrupt_create(
    _resource: u32,
    _src_num: usize,
    _options: u32,
    out_handle: &mut u32,
) -> ZxResult {
    *out_handle = compat::create_object(ObjectType::Interrupt)?.0;
    Ok(())
}

pub fn sys_interrupt_bind(interrupt: u32, port: u32, _key: u64, _options: u32) -> ZxResult {
    if !kernel_object_handle_known(interrupt) || !kernel_object_handle_known(port) {
        return Err(ZxError::ErrInvalidArgs);
    }
    Ok(())
}

pub fn sys_interrupt_bind_vcpu(interrupt: u32, vcpu: u32, _options: u32) -> ZxResult {
    if !kernel_object_handle_known(interrupt) || !kernel_object_handle_known(vcpu) {
        return Err(ZxError::ErrInvalidArgs);
    }
    Ok(())
}

pub fn sys_interrupt_trigger(interrupt: u32, _options: u32, _timestamp: i64) -> ZxResult {
    if compat::table()
        .update_signals(HandleValue(interrupt), 0, ZX_USER_SIGNAL_0)
        .is_some()
    {
        Ok(())
    } else {
        Err(ZxError::ErrNotFound)
    }
}

pub fn sys_interrupt_ack(interrupt: u32) -> ZxResult {
    if compat::table()
        .update_signals(HandleValue(interrupt), ZX_USER_SIGNAL_0, 0)
        .is_some()
    {
        Ok(())
    } else {
        Err(ZxError::ErrNotFound)
    }
}

pub fn sys_interrupt_destroy(interrupt: u32) -> ZxResult {
    sys_handle_close(interrupt)
}

pub fn sys_interrupt_wait(interrupt: u32, out_timestamp: usize) -> ZxResult {
    if !kernel_object_handle_known(interrupt) {
        return Err(ZxError::ErrNotFound);
    }
    if out_timestamp != 0 {
        unsafe {
            core::ptr::write(out_timestamp as *mut i64, monotonic_nanos() as i64);
        }
    }
    Ok(())
}

pub fn sys_pci_add_subtract_io_range(
    _handle: u32,
    _mmio: bool,
    _base: u64,
    _len: u64,
    _add: bool,
) -> ZxResult {
    Err(ZxError::ErrNotSupported)
}

pub fn sys_pci_cfg_pio_rw(
    _handle: u32,
    _bus: u8,
    _dev: u8,
    _func: u8,
    _offset: u8,
    _value_ptr: usize,
    _width: usize,
    _write: bool,
) -> ZxResult {
    Err(ZxError::ErrNotSupported)
}

pub fn sys_pci_init(_handle: u32, _init_buf: usize, _len: u32) -> ZxResult {
    Err(ZxError::ErrNotSupported)
}

pub fn sys_pci_map_interrupt(_dev: u32, _irq: i32, out_handle: &mut u32) -> ZxResult {
    *out_handle = compat::create_object(ObjectType::Interrupt)?.0;
    Ok(())
}

pub fn sys_pci_get_nth_device(
    _handle: u32,
    _index: u32,
    out_info: usize,
    out_handle: &mut u32,
) -> ZxResult {
    if out_info != 0 {
        unsafe {
            core::ptr::write_bytes(out_info as *mut u8, 0, 64);
        }
    }
    *out_handle = compat::create_object(ObjectType::PciDevice)?.0;
    Ok(())
}

pub fn sys_pci_get_bar(
    handle: u32,
    _bar_num: u32,
    out_bar: usize,
    out_handle: &mut u32,
) -> ZxResult {
    if !kernel_object_handle_known(handle) {
        return Err(ZxError::ErrNotFound);
    }
    if out_bar != 0 {
        unsafe {
            core::ptr::write_bytes(out_bar as *mut u8, 0, 24);
        }
    }
    let mut vmo = 0u32;
    sys_vmo_create(PAGE_SIZE as u64, 0, &mut vmo)?;
    *out_handle = vmo;
    Ok(())
}

pub fn sys_pci_enable_bus_master(handle: u32, _enable: bool) -> ZxResult {
    if kernel_object_handle_known(handle) {
        Ok(())
    } else {
        Err(ZxError::ErrNotFound)
    }
}

pub fn sys_pci_query_irq_mode(handle: u32, _mode: u32, out_max_irqs: usize) -> ZxResult {
    if !kernel_object_handle_known(handle) {
        return Err(ZxError::ErrNotFound);
    }
    if out_max_irqs != 0 {
        unsafe {
            core::ptr::write(out_max_irqs as *mut u32, 1);
        }
    }
    Ok(())
}

pub fn sys_pci_set_irq_mode(handle: u32, _mode: u32, _requested_irq_count: u32) -> ZxResult {
    if kernel_object_handle_known(handle) {
        Ok(())
    } else {
        Err(ZxError::ErrNotFound)
    }
}

pub fn sys_pci_reset_device(handle: u32) -> ZxResult {
    if kernel_object_handle_known(handle) {
        Ok(())
    } else {
        Err(ZxError::ErrNotFound)
    }
}

pub fn sys_pci_config_read(handle: u32, _offset: usize, _width: usize, out_val: usize) -> ZxResult {
    if !kernel_object_handle_known(handle) {
        return Err(ZxError::ErrNotFound);
    }
    if out_val != 0 {
        unsafe {
            core::ptr::write(out_val as *mut u32, 0);
        }
    }
    Ok(())
}

pub fn sys_pci_config_write(handle: u32, _offset: usize, _width: usize, _value: u32) -> ZxResult {
    if kernel_object_handle_known(handle) {
        Ok(())
    } else {
        Err(ZxError::ErrNotFound)
    }
}

pub fn sys_smc_call(handle: u32, parameters: usize, out_smc_result: usize) -> ZxResult {
    if handle != 0 && !compat::table().is_type(HandleValue(handle), ObjectType::Resource) {
        return Err(ZxError::ErrNotFound);
    }
    if !syscall_logic::user_buffer_valid(parameters, ZX_SMC_PARAMETERS_SIZE)
        || !syscall_logic::user_buffer_valid(out_smc_result, ZX_SMC_RESULT_SIZE)
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    unsafe {
        core::ptr::write_bytes(out_smc_result as *mut u8, 0, ZX_SMC_RESULT_SIZE);
    }
    Err(ZxError::ErrNotSupported)
}

pub fn sys_guest_create(
    _resource: u32,
    options: u32,
    guest_handle: &mut u32,
    vmar_handle: &mut u32,
) -> ZxResult {
    if !syscall_logic::zircon_hypervisor_options_valid(options, ZX_HYPERVISOR_OPTIONS_MASK) {
        return Err(ZxError::ErrInvalidArgs);
    }
    *guest_handle = compat::create_object_with_options(ObjectType::Guest, options)?.0;
    let mut child_addr = 0usize;
    sys_vmar_allocate(
        memory_root_vmar_handle(),
        0,
        0,
        ZIRCON_ROOT_VMAR_SIZE as u64,
        vmar_handle,
        &mut child_addr,
    )?;
    Ok(())
}

pub fn sys_guest_set_trap(
    handle: u32,
    kind: u32,
    addr: u64,
    size: u64,
    port_handle: u32,
    key: u64,
) -> ZxResult {
    if !compat::table().is_type(HandleValue(handle), ObjectType::Guest) {
        return Err(ZxError::ErrNotFound);
    }
    if !syscall_logic::zircon_guest_trap_kind_valid(kind, ZX_GUEST_TRAP_KIND_MAX)
        || !syscall_logic::zircon_guest_trap_range_valid(addr, size, ZX_GUEST_PHYS_LIMIT)
        || !syscall_logic::zircon_guest_trap_alignment_valid(
            kind,
            addr,
            size,
            ZX_GUEST_TRAP_BELL,
            ZX_GUEST_TRAP_MEM,
            PAGE_SIZE as u64,
        )
        || (port_handle != INVALID_HANDLE && !port::port_table().contains(HandleValue(port_handle)))
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    let trap_is_page_backed = syscall_logic::zircon_guest_trap_is_bell(kind, ZX_GUEST_TRAP_BELL)
        || syscall_logic::zircon_guest_trap_is_mem(kind, ZX_GUEST_TRAP_MEM);
    let trap_encoding = ((kind as u64) << 56) | (key & 0x00ff_ffff_ffff_ffff);
    let modeled_trap = if trap_is_page_backed {
        trap_encoding | 1
    } else {
        trap_encoding
    };
    let _ = compat::table().set_state_value(HandleValue(handle), modeled_trap);
    Ok(())
}

pub fn sys_vcpu_create(
    guest_handle: u32,
    options: u32,
    entry: u64,
    out_handle: &mut u32,
) -> ZxResult {
    if !syscall_logic::zircon_hypervisor_options_valid(options, ZX_HYPERVISOR_OPTIONS_MASK)
        || !compat::table().is_type(HandleValue(guest_handle), ObjectType::Guest)
        || !syscall_logic::zircon_vcpu_entry_valid(entry, ZX_VCPU_ENTRY_ALIGNMENT)
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    let handle = compat::create_object_with_options(ObjectType::Vcpu, options)?;
    let _ = compat::table().set_state_value(handle, entry);
    *out_handle = handle.0;
    Ok(())
}

pub fn sys_vcpu_resume(handle: u32, user_packet: usize) -> ZxResult {
    if !compat::table().is_type(HandleValue(handle), ObjectType::Vcpu) {
        return Err(ZxError::ErrNotFound);
    }
    if !syscall_logic::user_buffer_valid(user_packet, ZX_VCPU_PACKET_SIZE) {
        return Err(ZxError::ErrInvalidArgs);
    }
    if user_packet != 0 {
        unsafe {
            core::ptr::write_bytes(user_packet as *mut u8, 0, ZX_VCPU_PACKET_SIZE);
        }
    }
    Err(ZxError::ErrNotSupported)
}

pub fn sys_vcpu_interrupt(handle: u32, vector: u32) -> ZxResult {
    if !compat::table().is_type(HandleValue(handle), ObjectType::Vcpu) {
        Err(ZxError::ErrNotFound)
    } else if !syscall_logic::zircon_vcpu_interrupt_vector_valid(
        vector,
        ZX_VCPU_INTERRUPT_VECTOR_MAX,
    ) {
        Err(ZxError::ErrInvalidArgs)
    } else {
        let _ = compat::table().set_state_value(HandleValue(handle), vector as u64);
        Ok(())
    }
}

pub fn sys_vcpu_read_state(
    handle: u32,
    kind: u32,
    user_buffer: usize,
    buffer_size: usize,
) -> ZxResult {
    if !compat::table().is_type(HandleValue(handle), ObjectType::Vcpu)
        || !syscall_logic::zircon_vcpu_read_state_args_valid(
            kind,
            buffer_size,
            ZX_VCPU_STATE,
            ZX_VCPU_STATE_SIZE,
        )
        || !syscall_logic::user_buffer_valid(user_buffer, buffer_size)
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    if buffer_size != 0 {
        unsafe {
            core::ptr::write_bytes(user_buffer as *mut u8, 0, buffer_size);
        }
    }
    Ok(())
}

pub fn sys_vcpu_write_state(
    handle: u32,
    kind: u32,
    user_buffer: usize,
    buffer_size: usize,
) -> ZxResult {
    if !compat::table().is_type(HandleValue(handle), ObjectType::Vcpu)
        || !syscall_logic::zircon_vcpu_write_state_args_valid(
            kind,
            buffer_size,
            ZX_VCPU_STATE,
            ZX_VCPU_STATE_SIZE,
            ZX_VCPU_IO,
            ZX_VCPU_IO_SIZE,
        )
        || !syscall_logic::user_buffer_valid(user_buffer, buffer_size)
    {
        return Err(ZxError::ErrInvalidArgs);
    }
    let _ = compat::table().set_state_value(HandleValue(handle), kind as u64);
    Ok(())
}

pub fn sys_system_mexec(_kernel_vmo: u32, _bootimage_vmo: u32) -> ZxResult {
    Err(ZxError::ErrNotSupported)
}

pub fn sys_system_mexec_payload_get(_buffer: usize, _buffer_size: usize) -> ZxResult {
    Err(ZxError::ErrNotSupported)
}

pub fn sys_system_powerctl(_resource: u32, _cmd: u32, _arg: usize) -> ZxResult {
    Err(ZxError::ErrNotSupported)
}

pub fn sys_pager_create(options: u32, out_handle: &mut u32) -> ZxResult {
    if options != 0 {
        return Err(ZxError::ErrInvalidArgs);
    }
    *out_handle = compat::create_object(ObjectType::Pager)?.0;
    Ok(())
}

pub fn sys_pager_create_vmo(
    pager: u32,
    options: u32,
    _port: u32,
    _key: u64,
    size: usize,
    out_handle: &mut u32,
) -> ZxResult {
    if options != 0 || !kernel_object_handle_known(pager) {
        return Err(ZxError::ErrInvalidArgs);
    }
    sys_vmo_create(size as u64, 0, out_handle)
}

pub fn sys_pager_detach_vmo(pager: u32, vmo: u32) -> ZxResult {
    if !kernel_object_handle_known(pager) || memory_state().get_vmo(vmo).is_none() {
        return Err(ZxError::ErrNotFound);
    }
    Ok(())
}

pub fn sys_pager_supply_pages(
    pager: u32,
    pager_vmo: u32,
    _offset: usize,
    _len: usize,
    aux_vmo: u32,
    _aux_offset: usize,
) -> ZxResult {
    if !kernel_object_handle_known(pager)
        || memory_state().get_vmo(pager_vmo).is_none()
        || memory_state().get_vmo(aux_vmo).is_none()
    {
        return Err(ZxError::ErrNotFound);
    }
    Ok(())
}

pub fn sys_stream_create(
    options: u32,
    vmo_handle: u32,
    _seek: usize,
    out_handle: &mut u32,
) -> ZxResult {
    if options & !0b11 != 0 || memory_state().get_vmo(vmo_handle).is_none() {
        return Err(ZxError::ErrInvalidArgs);
    }
    *out_handle = compat::create_object(ObjectType::Stream)?.0;
    Ok(())
}

pub fn sys_stream_seek(handle: u32, _whence: u32, offset: i64, out_seek: &mut usize) -> ZxResult {
    if !compat::handle_known(HandleValue(handle)) {
        return Err(ZxError::ErrNotFound);
    }
    *out_seek = if offset < 0 { 0 } else { offset as usize };
    Ok(())
}

pub fn sys_stream_writev(
    handle: u32,
    options: u32,
    vector: usize,
    vector_size: usize,
    actual_count: usize,
) -> ZxResult {
    if options & !1 != 0 || !compat::handle_known(HandleValue(handle)) {
        return Err(ZxError::ErrInvalidArgs);
    }
    let written = linux_iov_write_compat(handle, vector, vector_size)?;
    if actual_count != 0 {
        unsafe {
            core::ptr::write(actual_count as *mut usize, written);
        }
    }
    Ok(())
}

pub fn sys_stream_writev_at(
    handle: u32,
    options: u32,
    _offset: usize,
    vector: usize,
    vector_size: usize,
    actual_count: usize,
) -> ZxResult {
    if options != 0 {
        return Err(ZxError::ErrInvalidArgs);
    }
    sys_stream_writev(handle, 0, vector, vector_size, actual_count)
}

pub fn sys_stream_readv(
    handle: u32,
    options: u32,
    vector: usize,
    vector_size: usize,
    actual_count: usize,
) -> ZxResult {
    if options != 0 || !compat::handle_known(HandleValue(handle)) {
        return Err(ZxError::ErrInvalidArgs);
    }
    let read = linux_iov_read_compat(handle, vector, vector_size)?;
    if actual_count != 0 {
        unsafe {
            core::ptr::write(actual_count as *mut usize, read);
        }
    }
    Ok(())
}

pub fn sys_stream_readv_at(
    handle: u32,
    options: u32,
    _offset: usize,
    vector: usize,
    vector_size: usize,
    actual_count: usize,
) -> ZxResult {
    if options != 0 {
        return Err(ZxError::ErrInvalidArgs);
    }
    sys_stream_readv(handle, 0, vector, vector_size, actual_count)
}

pub fn sys_futex_wait(
    value_ptr: usize,
    current_value: i32,
    new_owner: u32,
    deadline: u64,
) -> ZxResult {
    futex::futex_table().wait(value_ptr, current_value, new_owner, deadline)
}

pub fn sys_futex_wake(value_ptr: usize, count: u32) -> ZxResult<u32> {
    futex::futex_table().wake(value_ptr, count)
}

pub fn sys_futex_wake_single_owner(value_ptr: usize) -> ZxResult<u32> {
    sys_futex_wake(value_ptr, 1)
}

pub fn sys_futex_requeue(
    value_ptr: usize,
    wake_count: u32,
    current_value: i32,
    requeue_ptr: usize,
    requeue_count: u32,
    new_requeue_owner: u32,
) -> ZxResult<(u32, u32)> {
    futex::futex_table().requeue(
        value_ptr,
        wake_count,
        current_value,
        requeue_ptr,
        requeue_count,
        new_requeue_owner,
    )
}

pub fn sys_futex_get_owner(value_ptr: usize) -> ZxResult<u32> {
    futex::futex_table().get_owner(value_ptr)
}

// ============================================================================
// Time Syscalls
// ============================================================================

/// Zircon sys_clock_get_monotonic implementation
pub fn sys_clock_get_monotonic() -> ZxResult<u64> {
    Ok(monotonic_nanos())
}

pub fn sys_clock_create(options: u32, _args: usize, out_handle: &mut u32) -> ZxResult {
    if !syscall_logic::zircon_clock_create_options_valid(options, ZX_CLOCK_OPT_AUTO_START) {
        return Err(ZxError::ErrInvalidArgs);
    }
    let handle = compat::create_object_with_options(ObjectType::Clock, options)?;
    let synthetic_time = if options & ZX_CLOCK_OPT_AUTO_START != 0 {
        monotonic_nanos()
    } else {
        0
    };
    let _ = compat::table().set_state_value(handle, synthetic_time);
    *out_handle = handle.0;
    Ok(())
}

pub fn sys_clock_get(clock_id: u32, out_time: usize) -> ZxResult {
    if !syscall_logic::zircon_clock_id_supported(clock_id) || out_time == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }
    unsafe {
        core::ptr::write(out_time as *mut u64, monotonic_nanos());
    }
    Ok(())
}

pub fn sys_clock_read(handle: u32, out_time: usize) -> ZxResult {
    if !compat::table().is_type(HandleValue(handle), ObjectType::Clock) || out_time == 0 {
        return Err(ZxError::ErrInvalidArgs);
    }
    let stored = compat::table()
        .state_value(HandleValue(handle))
        .unwrap_or(0);
    let value = stored.max(monotonic_nanos());
    unsafe {
        core::ptr::write(out_time as *mut u64, value);
    }
    Ok(())
}

pub fn sys_clock_adjust(_resource: u32, clock_id: u32, _offset: u64) -> ZxResult {
    if !syscall_logic::zircon_clock_id_supported(clock_id) {
        Err(ZxError::ErrInvalidArgs)
    } else {
        Ok(())
    }
}

pub fn sys_clock_update(handle: u32, options: u64, _args: usize) -> ZxResult {
    if !syscall_logic::zircon_clock_update_options_valid(options, ZX_CLOCK_UPDATE_OPTIONS_MASK) {
        return Err(ZxError::ErrInvalidArgs);
    }
    if !compat::table().is_type(HandleValue(handle), ObjectType::Clock) {
        return Err(ZxError::ErrNotFound);
    }
    let value = if options & ZX_CLOCK_UPDATE_OPTION_SYNTHETIC_VALUE_VALID != 0 {
        monotonic_nanos()
    } else {
        compat::table()
            .state_value(HandleValue(handle))
            .unwrap_or(0)
    };
    let _ = compat::table().set_state_value(HandleValue(handle), value);
    Ok(())
}

/// Zircon sys_nanosleep implementation
pub fn sys_nanosleep(deadline: u64) -> ZxResult {
    info!("nanosleep: deadline={:#x}", deadline);
    Ok(())
}

/// Linux sys_clock_gettime implementation
pub fn sys_clock_gettime(clock: usize, buf: usize) -> SysResult {
    info!("clock_gettime: clock={}", clock);

    if !syscall_logic::linux_clock_id_supported(clock) {
        return Err(SysError::EINVAL);
    }
    if buf == 0 {
        return Err(SysError::EFAULT);
    }

    let now = monotonic_nanos().max(1);
    let timespec = LinuxTimespec {
        tv_sec: (now / 1_000_000_000) as i64,
        tv_nsec: (now % 1_000_000_000) as i64,
    };
    unsafe {
        core::ptr::write(buf as *mut LinuxTimespec, timespec);
    }
    Ok(0)
}

pub fn sys_clock_getres(clock: usize, buf: usize) -> SysResult {
    if !syscall_logic::linux_clock_id_supported(clock) {
        return Err(SysError::EINVAL);
    }
    if buf != 0 {
        unsafe {
            core::ptr::write(
                buf as *mut LinuxTimespec,
                LinuxTimespec {
                    tv_sec: 0,
                    tv_nsec: 10_000_000,
                },
            );
        }
    }
    Ok(0)
}

pub fn sys_gettimeofday(tv: usize, _tz: usize) -> SysResult {
    if tv != 0 {
        let now = monotonic_nanos().max(1);
        unsafe {
            core::ptr::write(
                tv as *mut LinuxTimeval,
                LinuxTimeval {
                    tv_sec: (now / 1_000_000_000) as i64,
                    tv_usec: ((now % 1_000_000_000) / 1_000) as i64,
                },
            );
        }
    }
    Ok(0)
}

pub fn sys_times(buf: usize) -> SysResult {
    let ticks = scheduler::scheduler().get_tick_count() as isize;
    if buf != 0 {
        unsafe {
            core::ptr::write(
                buf as *mut LinuxTms,
                LinuxTms {
                    tms_utime: ticks,
                    tms_stime: ticks,
                    tms_cutime: 0,
                    tms_cstime: 0,
                },
            );
        }
    }
    Ok(ticks as usize)
}

pub fn sys_getrusage(_who: usize, usage: usize) -> SysResult {
    if usage == 0 {
        return Err(SysError::EFAULT);
    }
    let now = monotonic_nanos().max(1);
    let timeval = LinuxTimeval {
        tv_sec: (now / 1_000_000_000) as i64,
        tv_usec: ((now % 1_000_000_000) / 1_000) as i64,
    };
    unsafe {
        core::ptr::write(
            usage as *mut LinuxRusage,
            LinuxRusage {
                ru_utime: timeval,
                ru_stime: timeval,
                rest: [0; 14],
            },
        );
    }
    Ok(0)
}

pub fn sys_prlimit64(
    _pid: usize,
    _resource: usize,
    new_limit: usize,
    old_limit: usize,
) -> SysResult {
    if new_limit != 0
        && !syscall_logic::user_buffer_valid(new_limit, core::mem::size_of::<LinuxRlimit64>())
    {
        return Err(SysError::EFAULT);
    }
    if old_limit != 0 {
        unsafe {
            core::ptr::write(
                old_limit as *mut LinuxRlimit64,
                LinuxRlimit64 {
                    rlim_cur: u64::MAX,
                    rlim_max: u64::MAX,
                },
            );
        }
    }
    Ok(0)
}

pub fn sys_getrlimit(resource: usize, old_limit: usize) -> SysResult {
    sys_prlimit64(0, resource, 0, old_limit)
}

pub fn sys_setrlimit(resource: usize, new_limit: usize) -> SysResult {
    sys_prlimit64(0, resource, new_limit, 0)
}

pub fn sys_sysinfo(info: usize) -> SysResult {
    if info == 0 {
        return Err(SysError::EFAULT);
    }
    unsafe {
        core::ptr::write(
            info as *mut LinuxSysinfo,
            LinuxSysinfo {
                uptime: (monotonic_nanos() / 1_000_000_000) as isize,
                loads: [0; 3],
                totalram: 1024 * 1024,
                freeram: 512 * 1024,
                sharedram: 0,
                bufferram: 0,
                totalswap: 0,
                freeswap: 0,
                procs: 1,
                pad: 0,
                totalhigh: 0,
                freehigh: 0,
                mem_unit: 1,
            },
        );
    }
    Ok(0)
}

/// Linux sys_nanosleep implementation
pub fn sys_nanosleep_linux(req: usize) -> SysResult {
    info!("nanosleep (linux)");

    if req == 0 {
        return Err(SysError::EFAULT);
    }
    Ok(0)
}

// ============================================================================
// Syscall Number Definitions
// ============================================================================

/// Linux syscall numbers
///
/// The active ARM64 dispatch path uses the explicit `ARM64_SYS_*` constants above for
/// syscalls implemented in SMROS. This enum is kept as a broad name catalog plus the
/// legacy synthetic fork/vfork entries used by direct dispatcher tests.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxSyscall {
    IoSetup = 0,
    IoDestroy = 1,
    IoSubmit = 2,
    IoCancel = 3,
    IoGetevents = 4,
    Setxattr = 5,
    Lsetxattr = 6,
    Fsetxattr = 7,
    Getxattr = 8,
    Lgetxattr = 9,
    Fgetxattr = 10,
    Listxattr = 11,
    Llistxattr = 12,
    Flistxattr = 13,
    Removexattr = 14,
    Lremovexattr = 15,
    Fremovexattr = 16,
    Getcwd = 17,
    LookupDcookie = 18,
    Eventfd2 = 19,
    EpollCreate1 = 20,
    EpollCtl = 21,
    EpollPwait = 22,
    Dup = 23,
    Dup3 = 24,
    Fcntl = 25,
    InotifyInit1 = 26,
    InotifyAddWatch = 27,
    InotifyRmWatch = 28,
    Ioctl = 29,
    IoprioSet = 30,
    IoprioGet = 31,
    Flock = 32,
    Mknodat = 33,
    Mkdirat = 34,
    Unlinkat = 35,
    Symlinkat = 36,
    Linkat = 37,
    Renameat = 38,
    Umount = 39,
    Mount = 40,
    PivotRoot = 41,
    Nfsservctl = 42,
    Statfs = 43,
    Fstatfs = 44,
    Truncate = 45,
    Ftruncate = 46,
    Fallocate = 47,
    Faccessat = 48,
    Chdir = 49,
    Fchdir = 50,
    Chroot = 51,
    Fchmod = 52,
    Fchmodat = 53,
    Fchownat = 54,
    Fchown = 55,
    Openat = 56,
    Close = 57,
    Vhangup = 58,
    Pipe2 = 59,
    Quotactl = 60,
    Getdents64 = 61,
    Lseek = 62,
    Read = 63,
    Write = 64,
    Readv = 65,
    Writev = 66,
    Pread64 = 67,
    Pwrite64 = 68,
    Preadv = 69,
    Pwritev = 70,
    Sendfile = 71,
    Pselect6 = 72,
    Ppoll = 73,
    Signalfd4 = 74,
    Vmsplice = 75,
    Splice = 76,
    Tee = 77,
    Readlinkat = 78,
    Fstatat = 79,
    Fstat = 80,
    Sync = 81,
    Fsync = 82,
    Fdatasync = 83,
    SyncFileRange = 84,
    TimerfdCreate = 85,
    TimerfdSettime = 86,
    TimerfdGettime = 87,
    Utimensat = 88,
    Acct = 89,
    Capget = 90,
    Capset = 91,
    Personality = 92,
    Exit = 93,
    ExitGroup = 94,
    Waitid = 95,
    SetTidAddress = 96,
    Unshare = 97,
    Futex = 98,
    SetRobustList = 99,
    GetRobustList = 100,
    Nanosleep = 101,
    Getitimer = 102,
    Setitimer = 103,
    KexecLoad = 104,
    InitModule = 105,
    DeleteModule = 106,
    TimerCreate = 107,
    TimerGettime = 108,
    TimerGetoverrun = 109,
    TimerDelete = 110,
    ClockSettime = 111,
    ClockGettime = 112,
    ClockGetres = 113,
    ClockNanosleep = 114,
    Syslog = 115,
    Ptrace = 116,
    SchedSetparam = 117,
    SchedSetscheduler = 118,
    SchedGetscheduler = 119,
    SchedGetparam = 120,
    SchedSetaffinity = 121,
    SchedAffinity = 122,
    SchedYield = 123,
    SchedGetPriorityMax = 124,
    SchedGetPriorityMin = 125,
    SchedRrGetInterval = 126,
    RestartSyscall = 127,
    Kill = 128,
    Tkill = 129,
    Tgkill = 130,
    Sigaltstack = 131,
    RtSigaction = 132,
    RtSigprocmask = 133,
    RtSigpending = 134,
    RtSigtimedwait = 135,
    RtSigqueueinfo = 136,
    RtSigreturn = 137,
    Setpriority = 138,
    Getpriority = 139,
    Reboot = 140,
    Setregid = 141,
    Setgid = 142,
    Setreuid = 143,
    Setuid = 144,
    Setresuid = 145,
    Getresuid = 146,
    Setresgid = 147,
    Getresgid = 148,
    Setfsuid = 149,
    Setfsgid = 150,
    Times = 151,
    Setpgid = 152,
    Getpgid = 153,
    Getsid = 154,
    Setsid = 155,
    Getgroups = 156,
    Setgroups = 157,
    Uname = 158,
    Sethostname = 159,
    Setdomainname = 160,
    Getrlimit = 161,
    Setrlimit = 162,
    Getrusage = 163,
    Umask = 164,
    Prctl = 165,
    Getcpu = 166,
    Gettimeofday = 167,
    Settimeofday = 168,
    Adjtimex = 169,
    Getpid = 170,
    Getppid = 171,
    Getuid = 172,
    Geteuid = 173,
    Getgid = 174,
    Getegid = 175,
    Gettid = 176,
    Sysinfo = 177,
    MqOpen = 178,
    MqUnlink = 179,
    MqTimedsend = 180,
    MqTimedreceive = 181,
    MqNotify = 182,
    MqGetsetattr = 183,
    Msgget = 184,
    Msgctl = 185,
    Msgsnd = 186,
    Msgrcv = 187,
    Semget = 188,
    Semctl = 189,
    Semtimedop = 190,
    Semop = 191,
    Shmget = 192,
    Shmctl = 193,
    Shmat = 194,
    Shmdt = 195,
    Socket = 196,
    Socketpair = 197,
    Bind = 198,
    Listen = 199,
    Accept = 200,
    Connect = 201,
    Getsockname = 202,
    Getpeername = 203,
    Sendto = 204,
    Recvfrom = 205,
    Setsockopt = 206,
    Getsockopt = 207,
    Shutdown = 208,
    Sendmsg = 209,
    Recvmsg = 210,
    Readahead = 211,
    Brk = 212,
    Munmap = 213,
    Clone = 214,
    Execve = 215,
    Mmap = 216, // Note: actual number may differ
    Mprotect = 217,
    Mremap = 218,
    Msync = 219,
    Mincore = 220,
    Madvise = 221,
    Accept4 = 222,
    Recvmsg2 = 223,
    // ARM64 specific
    Fork = 1000,
    Vfork = 1001,
}

/// Zircon syscall numbers
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZirconSyscall {
    ClockGet = 0,
    ClockGetNew = 1,
    ClockGetMonotonic = 2,
    Nanosleep = 3,
    ClockAdjust = 4,
    SystemGetEvent = 5,
    HandleClose = 6,
    HandleCloseMany = 7,
    HandleDuplicate = 8,
    HandleReplace = 9,
    ObjectWaitOne = 10,
    ObjectWaitMany = 11,
    ObjectWaitAsync = 12,
    ObjectSignal = 13,
    ObjectSignalPeer = 14,
    ObjectGetProperty = 15,
    ObjectSetProperty = 16,
    ObjectGetInfo = 17,
    ObjectGetChild = 18,
    ObjectSetProfile = 19,
    ChannelCreate = 20,
    ChannelRead = 21,
    ChannelReadEtc = 22,
    ChannelWrite = 23,
    ChannelWriteEtc = 24,
    ChannelCallNoretry = 25,
    ChannelCallFinish = 26,
    SocketCreate = 27,
    SocketWrite = 28,
    SocketRead = 29,
    SocketShare = 30,
    SocketAccept = 31,
    SocketShutdown = 32,
    ThreadExit = 33,
    ThreadCreate = 34,
    ThreadStart = 35,
    ThreadReadState = 36,
    ThreadWriteState = 37,
    ProcessExit = 38,
    ProcessCreate = 39,
    ProcessStart = 40,
    ProcessReadMemory = 41,
    ProcessWriteMemory = 42,
    JobCreate = 43,
    JobSetPolicy = 44,
    TaskBindExceptionPort = 45,
    TaskSuspend = 46,
    TaskSuspendToken = 47,
    TaskResumeFromException = 48,
    TaskCreateExceptionChannel = 49,
    TaskKill = 50,
    ExceptionGetThread = 51,
    ExceptionGetProcess = 52,
    EventCreate = 53,
    EventpairCreate = 54,
    FutexWait = 55,
    FutexWake = 56,
    FutexRequeue = 57,
    FutexWakeSingleOwner = 58,
    FutexRequeueSingleOwner = 59,
    FutexGetOwner = 60,
    PortCreate = 61,
    PortQueue = 62,
    PortWait = 63,
    PortCancel = 64,
    TimerCreate = 65,
    TimerSet = 66,
    TimerCancel = 67,
    VmoCreate = 68,
    VmoRead = 69,
    VmoWrite = 70,
    VmoGetSize = 71,
    VmoSetSize = 72,
    VmoOpRange = 73,
    VmoCreateChild = 74,
    VmoSetCachePolicy = 75,
    VmoReplaceAsExecutable = 76,
    VmarAllocate = 77,
    VmarDestroy = 78,
    VmarMap = 79,
    VmarUnmap = 80,
    VmarProtect = 81,
    CprngDrawOnce = 82,
    CprngAddEntropy = 83,
    FifoCreate = 84,
    FifoRead = 85,
    FifoWrite = 86,
    ProfileCreate = 87,
    DebuglogCreate = 88,
    DebuglogWrite = 89,
    DebuglogRead = 90,
    KtraceRead = 91,
    KtraceControl = 92,
    KtraceWrite = 93,
    MtraceControl = 94,
    DebugRead = 95,
    DebugWrite = 96,
    DebugSendCommand = 97,
    InterruptCreate = 98,
    InterruptBind = 99,
    InterruptWait = 100,
    InterruptDestroy = 101,
    InterruptAck = 102,
    InterruptTrigger = 103,
    InterruptBindVcpu = 104,
    IoportsRequest = 105,
    VmoCreateContiguous = 106,
    VmoCreatePhysical = 107,
    IommuCreate = 108,
    BtiCreate = 109,
    BtiPin = 110,
    BtiReleaseQuarantine = 111,
    PmtUnpin = 112,
    FramebufferGetInfo = 113,
    FramebufferSetRange = 114,
    PciGetNthDevice = 115,
    PciEnableBusMaster = 116,
    PciResetDevice = 117,
    PciConfigRead = 118,
    PciConfigWrite = 119,
    PciCfgPioRw = 120,
    PciGetBar = 121,
    PciMapInterrupt = 122,
    PciQueryIrqMode = 123,
    PciSetIrqMode = 124,
    PciInit = 125,
    PciAddSubtractIoRange = 126,
    PcFirmwareTables = 127,
    SmcCall = 128,
    ResourceCreate = 129,
    GuestCreate = 130,
    GuestSetTrap = 131,
    VcpuCreate = 132,
    VcpuResume = 133,
    VcpuInterrupt = 134,
    VcpuReadState = 135,
    VcpuWriteState = 136,
    SystemMexec = 137,
    SystemMexecPayloadGet = 138,
    SystemPowerctl = 139,
    PagerCreate = 140,
    PagerCreateVmo = 141,
    PagerDetachVmo = 142,
    PagerSupplyPages = 143,
    SyscallTest0 = 144,
    SyscallTest1 = 145,
    SyscallTest2 = 146,
    SyscallTest3 = 147,
    SyscallTest4 = 148,
    SyscallTest5 = 149,
    SyscallTest6 = 150,
    SyscallTest7 = 151,
    SyscallTest8 = 152,
    SyscallTestWrapper = 153,
    Count = 154,
    JobSetCritical = 183,
    StreamCreate = 187,
    StreamWritev = 188,
    StreamWritevAt = 189,
    StreamReadv = 190,
    StreamReadvAt = 191,
    StreamSeek = 192,
    ClockCreate = 197,
    ClockRead = 198,
    ClockUpdate = 199,
    FutexWakeHandleCloseThreadExit = 200,
    VmarUnmapHandleCloseThreadExit = 201,
    SystemGetEventCompat = 202,
    TaskCreateExceptionChannelCompat = 203,
    BtiReleaseQuarantineCompat = 204,
    PcFirmwareTablesCompat = 205,
    InterruptTriggerCompat = 206,
    InterruptDestroyCompat = 207,
    InterruptAckCompat = 208,
    ExceptionGetThreadCompat = 209,
    ExceptionGetProcessCompat = 210,
    IoportsRequestCompat = 211,
}

// ============================================================================
// Syscall Dispatcher
// ============================================================================

/// Dispatch a Linux syscall
pub fn dispatch_linux_syscall(syscall_num: u32, args: [usize; 6]) -> SysResult {
    let result = match syscall_num {
        ARM64_SYS_IO_SETUP
        | ARM64_SYS_IO_DESTROY
        | ARM64_SYS_IO_SUBMIT
        | ARM64_SYS_IO_CANCEL
        | ARM64_SYS_IO_GETEVENTS => Err(SysError::ENOSYS),
        ARM64_SYS_LOOKUP_DCOOKIE
        | ARM64_SYS_IOPRIO_SET
        | ARM64_SYS_IOPRIO_GET
        | ARM64_SYS_VHANGUP
        | ARM64_SYS_ACCT
        | ARM64_SYS_PERSONALITY
        | ARM64_SYS_KEXEC_LOAD
        | ARM64_SYS_INIT_MODULE
        | ARM64_SYS_DELETE_MODULE
        | ARM64_SYS_SYSLOG
        | ARM64_SYS_PTRACE
        | ARM64_SYS_RESTART_SYSCALL
        | ARM64_SYS_REBOOT
        | ARM64_SYS_SETTIMEOFDAY
        | ARM64_SYS_ADJTIMEX
        | ARM64_SYS_MQ_OPEN
        | ARM64_SYS_MQ_UNLINK
        | ARM64_SYS_MQ_TIMEDSEND
        | ARM64_SYS_MQ_TIMEDRECEIVE
        | ARM64_SYS_MQ_NOTIFY
        | ARM64_SYS_MQ_GETSETATTR
        | ARM64_SYS_ADD_KEY
        | ARM64_SYS_REQUEST_KEY
        | ARM64_SYS_KEYCTL
        | ARM64_SYS_SWAPON
        | ARM64_SYS_SWAPOFF
        | ARM64_SYS_MBIND
        | ARM64_SYS_GET_MEMPOLICY
        | ARM64_SYS_SET_MEMPOLICY
        | ARM64_SYS_MIGRATE_PAGES
        | ARM64_SYS_MOVE_PAGES
        | ARM64_SYS_PERF_EVENT_OPEN
        | ARM64_SYS_FANOTIFY_INIT
        | ARM64_SYS_FANOTIFY_MARK
        | ARM64_SYS_NAME_TO_HANDLE_AT
        | ARM64_SYS_OPEN_BY_HANDLE_AT
        | ARM64_SYS_CLOCK_ADJTIME
        | ARM64_SYS_SYNCFS
        | ARM64_SYS_SENDMMSG
        | ARM64_SYS_PROCESS_VM_READV
        | ARM64_SYS_PROCESS_VM_WRITEV
        | ARM64_SYS_KCMP
        | ARM64_SYS_FINIT_MODULE
        | ARM64_SYS_SCHED_SETATTR
        | ARM64_SYS_SCHED_GETATTR
        | ARM64_SYS_BPF
        | ARM64_SYS_EXECVEAT
        | ARM64_SYS_USERFAULTFD
        | ARM64_SYS_MLOCK2
        | ARM64_SYS_PKEY_MPROTECT
        | ARM64_SYS_PKEY_ALLOC
        | ARM64_SYS_PKEY_FREE
        | ARM64_SYS_IO_PGETEVENTS
        | ARM64_SYS_KEXEC_FILE_LOAD
        | ARM64_SYS_PIDFD_SEND_SIGNAL
        | ARM64_SYS_IO_URING_SETUP
        | ARM64_SYS_IO_URING_ENTER
        | ARM64_SYS_IO_URING_REGISTER
        | ARM64_SYS_FSPICK
        | ARM64_SYS_PIDFD_GETFD
        | ARM64_SYS_PROCESS_MADVISE
        | ARM64_SYS_LANDLOCK_CREATE_RULESET
        | ARM64_SYS_LANDLOCK_ADD_RULE
        | ARM64_SYS_LANDLOCK_RESTRICT_SELF => Err(SysError::ENOSYS),
        ARM64_SYS_RSEQ => Err(SysError::EINVAL),
        ARM64_SYS_SETXATTR | ARM64_SYS_LSETXATTR => {
            sys_xattr_path(args[0], args[1], args[2], args[3])
        }
        ARM64_SYS_FSETXATTR => sys_xattr_fd(args[0], args[1], args[2], args[3]),
        ARM64_SYS_GETXATTR | ARM64_SYS_LGETXATTR | ARM64_SYS_LISTXATTR | ARM64_SYS_LLISTXATTR => {
            sys_xattr_path(args[0], args[1], args[2], args[3])
        }
        ARM64_SYS_FGETXATTR | ARM64_SYS_FLISTXATTR => {
            sys_xattr_fd(args[0], args[1], args[2], args[3])
        }
        ARM64_SYS_REMOVEXATTR | ARM64_SYS_LREMOVEXATTR => sys_xattr_path(args[0], args[1], 0, 0),
        ARM64_SYS_FREMOVEXATTR => sys_xattr_fd(args[0], args[1], 0, 0),
        ARM64_SYS_GETCWD => sys_getcwd(args[0], args[1]),
        ARM64_SYS_EVENTFD2 => sys_eventfd2(args[0], args[1]),
        ARM64_SYS_EPOLL_CREATE1 => sys_epoll_create1(args[0]),
        ARM64_SYS_EPOLL_CTL => sys_epoll_ctl(args[0], args[1], args[2], args[3]),
        ARM64_SYS_EPOLL_PWAIT | ARM64_SYS_EPOLL_PWAIT2 => {
            sys_epoll_pwait(args[0], args[1], args[2], args[3] as isize, args[4])
        }
        ARM64_SYS_INOTIFY_INIT1 => sys_inotify_init1(args[0]),
        ARM64_SYS_INOTIFY_ADD_WATCH => sys_inotify_add_watch(args[0], args[1], args[2]),
        ARM64_SYS_INOTIFY_RM_WATCH => sys_inotify_rm_watch(args[0], args[1]),
        ARM64_SYS_READ => sys_read(args[0], args[1], args[2]),
        ARM64_SYS_WRITE => sys_write(args[0], args[1], args[2]),
        ARM64_SYS_CLOSE => sys_close(args[0]),
        ARM64_SYS_OPENAT => sys_openat(args[0], args[1], args[2], args[3]),
        ARM64_SYS_MKNODAT => sys_mknodat(args[0], args[1], args[2], args[3]),
        ARM64_SYS_MKDIRAT => sys_mkdirat(args[0], args[1], args[2]),
        ARM64_SYS_UNLINKAT => sys_unlinkat(args[0], args[1], args[2]),
        ARM64_SYS_SYMLINKAT => sys_symlinkat(args[0], args[1], args[2]),
        ARM64_SYS_LINKAT => sys_linkat(args[0], args[1], args[2], args[3], args[4]),
        ARM64_SYS_RENAMEAT => sys_renameat(args[0], args[1], args[2], args[3]),
        ARM64_SYS_RENAMEAT2 => sys_renameat2(args[0], args[1], args[2], args[3], args[4]),
        ARM64_SYS_UMOUNT2 => sys_umount2(args[0], args[1]),
        ARM64_SYS_MOUNT => sys_mount(args[0], args[1], args[2], args[3], args[4]),
        ARM64_SYS_PIVOT_ROOT => sys_pivot_root(args[0], args[1]),
        ARM64_SYS_NFSSERVCTL => Err(SysError::ENOSYS),
        ARM64_SYS_STATFS => sys_statfs(args[0], args[1]),
        ARM64_SYS_FSTATFS => sys_fstatfs(args[0], args[1]),
        ARM64_SYS_TRUNCATE => sys_truncate(args[0], args[1]),
        ARM64_SYS_FTRUNCATE => sys_ftruncate(args[0], args[1]),
        ARM64_SYS_FALLOCATE => sys_fallocate(args[0], args[1], args[2], args[3]),
        ARM64_SYS_FACCESSAT => sys_faccessat(args[0], args[1], args[2], 0),
        ARM64_SYS_FACCESSAT2 => sys_faccessat2(args[0], args[1], args[2], args[3]),
        ARM64_SYS_CHDIR => sys_chdir(args[0]),
        ARM64_SYS_FCHDIR => sys_fchdir(args[0]),
        ARM64_SYS_CHROOT => sys_chroot(args[0]),
        ARM64_SYS_FCHMOD => sys_fchmod(args[0], args[1]),
        ARM64_SYS_FCHMODAT => sys_fchmodat(args[0], args[1], args[2], args[3]),
        ARM64_SYS_FCHOWN => sys_fchown(args[0], args[1], args[2]),
        ARM64_SYS_FCHOWNAT => sys_fchownat(args[0], args[1], args[2], args[3], args[4]),
        ARM64_SYS_LSEEK => sys_lseek(args[0], args[1] as i64, args[2]),
        ARM64_SYS_READV => sys_readv(args[0], args[1], args[2]),
        ARM64_SYS_WRITEV => sys_writev(args[0], args[1], args[2]),
        ARM64_SYS_PREAD64 => sys_pread(args[0], args[1], args[2], args[3] as u64),
        ARM64_SYS_PWRITE64 => sys_pwrite(args[0], args[1], args[2], args[3] as u64),
        ARM64_SYS_PREADV | ARM64_SYS_PREADV2 => {
            sys_preadv(args[0], args[1], args[2], args[3] as u64)
        }
        ARM64_SYS_PWRITEV | ARM64_SYS_PWRITEV2 => {
            sys_pwritev(args[0], args[1], args[2], args[3] as u64)
        }
        ARM64_SYS_SENDFILE => sys_sendfile(args[0], args[1], args[2], args[3]),
        ARM64_SYS_PSELECT6 => sys_pselect6(args[0], args[1], args[2], args[3], args[4], args[5]),
        ARM64_SYS_PPOLL => sys_ppoll(args[0], args[1], args[2], args[3]),
        ARM64_SYS_SIGNALFD4 => sys_signalfd4(args[0], args[1], args[2], args[3]),
        ARM64_SYS_VMSPLICE => sys_vmsplice(args[0], args[1], args[2], args[3]),
        ARM64_SYS_SPLICE => sys_splice(args[0], args[1], args[2], args[3], args[4], args[5]),
        ARM64_SYS_TEE => sys_tee(args[0], args[1], args[2], args[3]),
        ARM64_SYS_READLINKAT => sys_readlinkat(args[0], args[1], args[2], args[3]),
        ARM64_SYS_NEWFSTATAT => sys_fstatat(args[0], args[1], args[2], args[3]),
        ARM64_SYS_FSTAT => sys_fstat(args[0], args[1]),
        ARM64_SYS_SYNC => sys_sync(),
        ARM64_SYS_FSYNC => sys_fsync(args[0]),
        ARM64_SYS_FDATASYNC => sys_fdatasync(args[0]),
        ARM64_SYS_SYNC_FILE_RANGE => sys_sync_file_range(args[0], args[1], args[2], args[3]),
        ARM64_SYS_TIMERFD_CREATE => sys_timerfd_create(args[0], args[1]),
        ARM64_SYS_TIMERFD_SETTIME => sys_timerfd_settime(args[0], args[1], args[2], args[3]),
        ARM64_SYS_TIMERFD_GETTIME => sys_timerfd_gettime(args[0], args[1]),
        ARM64_SYS_UTIMENSAT => sys_utimensat(args[0], args[1], args[2], args[3]),
        ARM64_SYS_EXIT => sys_exit(args[0] as i32),
        ARM64_SYS_EXIT_GROUP => sys_exit_group(args[0] as i32),
        ARM64_SYS_WAITID => sys_waitid(args[0], args[1], args[2], args[3], args[4]),
        ARM64_SYS_NANOSLEEP => sys_nanosleep_linux(args[0]),
        ARM64_SYS_GETITIMER => sys_getitimer(args[0], args[1]),
        ARM64_SYS_SETITIMER => sys_setitimer(args[0], args[1], args[2]),
        ARM64_SYS_TIMER_CREATE => sys_linux_timer_create(args[0], args[1], args[2]),
        ARM64_SYS_TIMER_GETTIME => sys_linux_timer_gettime(args[0], args[1]),
        ARM64_SYS_TIMER_GETOVERRUN => sys_linux_timer_getoverrun(args[0]),
        ARM64_SYS_TIMER_SETTIME => sys_linux_timer_settime(args[0], args[1], args[2], args[3]),
        ARM64_SYS_TIMER_DELETE => sys_linux_timer_delete(args[0]),
        ARM64_SYS_CLOCK_SETTIME => sys_clock_settime(args[0], args[1]),
        ARM64_SYS_CLOCK_GETTIME => sys_clock_gettime(args[0], args[1]),
        ARM64_SYS_CLOCK_GETRES => sys_clock_getres(args[0], args[1]),
        ARM64_SYS_CLOCK_NANOSLEEP => sys_clock_nanosleep(args[0], args[1], args[2], args[3]),
        ARM64_SYS_SCHED_SETPARAM => sys_sched_setparam(args[0], args[1]),
        ARM64_SYS_SCHED_SETSCHEDULER => sys_sched_setscheduler(args[0], args[1], args[2]),
        ARM64_SYS_SCHED_GETSCHEDULER => sys_sched_getscheduler(args[0]),
        ARM64_SYS_SCHED_GETPARAM => sys_sched_getparam(args[0], args[1]),
        ARM64_SYS_SCHED_SETAFFINITY => sys_sched_setaffinity(args[0], args[1], args[2]),
        ARM64_SYS_SCHED_GETAFFINITY => sys_sched_getaffinity(args[0], args[1], args[2]),
        ARM64_SYS_SCHED_GET_PRIORITY_MAX => sys_sched_get_priority_max(args[0]),
        ARM64_SYS_SCHED_GET_PRIORITY_MIN => sys_sched_get_priority_min(args[0]),
        ARM64_SYS_SCHED_RR_GET_INTERVAL => sys_sched_rr_get_interval(args[0], args[1]),
        ARM64_SYS_KILL => sys_kill(args[0] as isize, args[1]),
        ARM64_SYS_TKILL => sys_tkill(args[0], args[1]),
        ARM64_SYS_TGKILL => sys_tgkill(args[0], args[1], args[2]),
        ARM64_SYS_SIGALTSTACK => sys_sigaltstack(args[0], args[1]),
        ARM64_SYS_RT_SIGSUSPEND => sys_rt_sigsuspend(args[0], args[1]),
        ARM64_SYS_RT_SIGACTION => sys_rt_sigaction(args[0], args[1], args[2], args[3]),
        ARM64_SYS_RT_SIGPROCMASK => sys_rt_sigprocmask(args[0] as isize, args[1], args[2], args[3]),
        ARM64_SYS_RT_SIGPENDING => sys_rt_sigpending(args[0], args[1]),
        ARM64_SYS_RT_SIGTIMEDWAIT => sys_rt_sigtimedwait(args[0], args[1], args[2], args[3]),
        ARM64_SYS_RT_SIGQUEUEINFO => sys_rt_sigqueueinfo(args[0], args[1], args[2]),
        ARM64_SYS_RT_TGSIGQUEUEINFO => sys_rt_tgsigqueueinfo(args[0], args[1], args[2], args[3]),
        ARM64_SYS_RT_SIGRETURN => sys_rt_sigreturn(),
        ARM64_SYS_SETPRIORITY => sys_set_priority(args[1]),
        ARM64_SYS_GETPRIORITY => sys_get_priority(args[0], args[1]),
        ARM64_SYS_SETREGID => sys_setregid(args[0], args[1]),
        ARM64_SYS_SETGID => sys_setgid(args[0]),
        ARM64_SYS_SETREUID => sys_setreuid(args[0], args[1]),
        ARM64_SYS_SETUID => sys_setuid(args[0]),
        ARM64_SYS_SETRESUID => sys_setresuid(args[0], args[1], args[2]),
        ARM64_SYS_GETRESUID => sys_getresuid(args[0], args[1], args[2]),
        ARM64_SYS_SETRESGID => sys_setresgid(args[0], args[1], args[2]),
        ARM64_SYS_GETRESGID => sys_getresgid(args[0], args[1], args[2]),
        ARM64_SYS_SETFSUID => sys_setfsuid(args[0]),
        ARM64_SYS_SETFSGID => sys_setfsgid(args[0]),
        ARM64_SYS_SETPGID => sys_setpgid(args[0], args[1]),
        ARM64_SYS_GETPGID => sys_getpgid(args[0]),
        ARM64_SYS_GETSID => sys_getsid(args[0]),
        ARM64_SYS_SETSID => sys_setsid(),
        ARM64_SYS_GETGROUPS => sys_getgroups(args[0], args[1]),
        ARM64_SYS_SETGROUPS => sys_setgroups(args[0], args[1]),
        ARM64_SYS_SETHOSTNAME => sys_sethostname(args[0], args[1]),
        ARM64_SYS_SETDOMAINNAME => sys_setdomainname(args[0], args[1]),
        ARM64_SYS_GETRLIMIT => sys_getrlimit(args[0], args[1]),
        ARM64_SYS_SETRLIMIT => sys_setrlimit(args[0], args[1]),
        ARM64_SYS_GETPID => sys_getpid(),
        ARM64_SYS_GETPPID => sys_getppid(),
        ARM64_SYS_GETUID => sys_getuid(),
        ARM64_SYS_GETEUID => sys_geteuid(),
        ARM64_SYS_GETGID => sys_getgid(),
        ARM64_SYS_GETEGID => sys_getegid(),
        ARM64_SYS_GETTID => sys_gettid(),
        ARM64_SYS_SYSINFO => sys_sysinfo(args[0]),
        ARM64_SYS_MSGGET => sys_msgget(args[0], args[1]),
        ARM64_SYS_MSGCTL => sys_msgctl(args[0], args[1], args[2]),
        ARM64_SYS_MSGRCV => sys_msgrcv(args[0], args[1], args[2], args[3] as isize, args[4]),
        ARM64_SYS_MSGSND => sys_msgsnd(args[0], args[1], args[2], args[3]),
        ARM64_SYS_SEMGET => sys_semget(args[0], args[1], args[2]),
        ARM64_SYS_SEMCTL => sys_semctl(args[0], args[1], args[2], args[3]),
        ARM64_SYS_SEMOP | ARM64_SYS_SEMTIMEDOP => sys_semop(args[0], args[1], args[2]),
        ARM64_SYS_SHMGET => sys_shmget(args[0], args[1], args[2]),
        ARM64_SYS_SHMCTL => sys_shmctl(args[0], args[1], args[2]),
        ARM64_SYS_SHMAT => sys_shmat(args[0], args[1], args[2]),
        ARM64_SYS_SHMDT => sys_shmdt(args[0], args[1], args[2]),
        ARM64_SYS_SOCKET => sys_socket(args[0], args[1], args[2]),
        ARM64_SYS_SOCKETPAIR => sys_socketpair(args[0], args[1], args[2], args[3]),
        ARM64_SYS_BIND => sys_bind(args[0], args[1], args[2]),
        ARM64_SYS_LISTEN => sys_listen(args[0], args[1]),
        ARM64_SYS_ACCEPT | ARM64_SYS_ACCEPT4 => sys_accept(args[0], args[1], args[2]),
        ARM64_SYS_CONNECT => sys_connect(args[0], args[1], args[2]),
        ARM64_SYS_GETSOCKNAME => sys_getsockname(args[0], args[1], args[2]),
        ARM64_SYS_GETPEERNAME => sys_getpeername(args[0], args[1], args[2]),
        ARM64_SYS_SENDTO => sys_sendto(args[0], args[1], args[2], args[3], args[4], args[5]),
        ARM64_SYS_RECVFROM => sys_recvfrom(args[0], args[1], args[2], args[3], args[4], args[5]),
        ARM64_SYS_SETSOCKOPT => sys_setsockopt(args[0], args[1], args[2], args[3], args[4]),
        ARM64_SYS_GETSOCKOPT => sys_getsockopt(args[0], args[1], args[2], args[3], args[4]),
        ARM64_SYS_SHUTDOWN => sys_shutdown(args[0], args[1]),
        ARM64_SYS_SENDMSG => sys_sendmsg(args[0], args[1], args[2]),
        ARM64_SYS_RECVMSG => sys_recvmsg(args[0], args[1], args[2]),
        ARM64_SYS_RECVMMSG => sys_recvmmsg(args[0], args[1], args[2], args[3], args[4]),
        ARM64_SYS_READAHEAD => sys_readahead(args[0], args[1], args[2]),
        ARM64_SYS_GETTIMEOFDAY => sys_gettimeofday(args[0], args[1]),
        ARM64_SYS_BRK => sys_brk(args[0]),
        ARM64_SYS_MUNMAP => sys_munmap(args[0], args[1]),
        ARM64_SYS_MREMAP => sys_mremap(args[0], args[1], args[2], args[3], args[4]),
        ARM64_SYS_CLONE => sys_clone(args[0], args[1], args[2], args[3], args[4]),
        ARM64_SYS_CLONE3 => sys_clone3(args[0], args[1]),
        ARM64_SYS_EXECVE => sys_execve(args[0], args[1], args[2]),
        ARM64_SYS_MMAP => sys_mmap(args[0], args[1], args[2], args[3], args[4], args[5] as u64),
        ARM64_SYS_FADVISE64 => sys_fadvise64(args[0], args[1], args[2], args[3]),
        ARM64_SYS_MPROTECT => sys_mprotect(args[0], args[1], args[2]),
        ARM64_SYS_MSYNC => sys_memory_noop(args[0], args[1]),
        ARM64_SYS_MLOCK | ARM64_SYS_MUNLOCK => sys_memory_noop(args[0], args[1]),
        ARM64_SYS_MLOCKALL | ARM64_SYS_MUNLOCKALL => Ok(0),
        ARM64_SYS_MINCORE => sys_mincore(args[0], args[1], args[2]),
        ARM64_SYS_MADVISE => sys_madvise(args[0], args[1], args[2]),
        ARM64_SYS_REMAP_FILE_PAGES => Ok(0),
        ARM64_SYS_WAIT4 => sys_wait4(args[0] as i32, args[1], args[2] as u32),
        ARM64_SYS_PRLIMIT64 => sys_prlimit64(args[0], args[1], args[2], args[3]),
        ARM64_SYS_GETRANDOM => sys_getrandom(args[0], args[1], args[2] as u32),
        ARM64_SYS_MEMFD_CREATE => sys_memfd_create(args[0], args[1]),
        ARM64_SYS_MEMBARRIER => sys_membarrier(args[0], args[1], args[2]),
        ARM64_SYS_SECCOMP => sys_seccomp(args[0], args[1], args[2]),
        ARM64_SYS_CAPGET => sys_capget(args[0], args[1]),
        ARM64_SYS_CAPSET => sys_capset(args[0], args[1]),
        ARM64_SYS_SETNS => sys_setns(args[0], args[1]),
        ARM64_SYS_QUOTACTL
        | ARM64_SYS_OPEN_TREE
        | ARM64_SYS_MOVE_MOUNT
        | ARM64_SYS_FSOPEN
        | ARM64_SYS_FSCONFIG
        | ARM64_SYS_FSMOUNT
        | ARM64_SYS_PIDFD_OPEN
        | ARM64_SYS_MOUNT_SETATTR => Ok(0),
        ARM64_SYS_COPY_FILE_RANGE => {
            sys_copy_file_range(args[0], args[1], args[2], args[3], args[4], args[5])
        }
        ARM64_SYS_STATX => sys_statx(args[0], args[1], args[2], args[3], args[4]),
        ARM64_SYS_CLOSE_RANGE => sys_close_range(args[0], args[1], args[2]),
        ARM64_SYS_OPENAT2 => sys_openat(args[0], args[1], 0, 0),
        ARM64_SYS_SCHED_YIELD => sys_sched_yield(),
        ARM64_SYS_UMASK => sys_umask(args[0]),
        ARM64_SYS_PIPE2 => sys_pipe2(args[0], args[1]),
        ARM64_SYS_DUP => sys_dup(args[0]),
        ARM64_SYS_DUP3 => sys_dup3(args[0], args[1], args[2]),
        ARM64_SYS_FCNTL => sys_fcntl(args[0], args[1], args[2]),
        ARM64_SYS_IOCTL => sys_ioctl(args[0], args[1], args[2], args[3], args[4]),
        ARM64_SYS_FLOCK => sys_flock(args[0], args[1]),
        ARM64_SYS_GETDENTS64 => sys_getdents64(args[0], args[1], args[2]),
        ARM64_SYS_UNSHARE => sys_unshare(args[0]),
        ARM64_SYS_FUTEX => sys_futex(
            args[0],
            args[1] as u32,
            args[2] as u32,
            args[3],
            args[4],
            args[5] as u32,
        ),
        ARM64_SYS_SET_TID_ADDRESS => sys_set_tid_address(args[0]),
        ARM64_SYS_SET_ROBUST_LIST => sys_set_robust_list(args[0], args[1]),
        ARM64_SYS_GET_ROBUST_LIST => sys_get_robust_list(args[0] as isize, args[1], args[2]),
        ARM64_SYS_TIMES => sys_times(args[0]),
        ARM64_SYS_GETRUSAGE => sys_getrusage(args[0], args[1]),
        ARM64_SYS_UNAME => sys_uname(args[0]),
        ARM64_SYS_PRCTL => sys_prctl(args[0], args[1], args[2], args[3], args[4]),
        ARM64_SYS_GETCPU => sys_getcpu(args[0], args[1], args[2]),
        num if num == LinuxSyscall::Fork as u32 => sys_fork(),
        num if num == LinuxSyscall::Vfork as u32 => sys_vfork(),
        num if syscall_logic::linux_syscall_interface_known(num) => {
            warn!("Unsupported Linux syscall interface: {}", syscall_num);
            Err(SysError::ENOSYS)
        }
        _ => {
            warn!("Unimplemented Linux syscall: {}", syscall_num);
            Err(SysError::ENOSYS)
        }
    };
    result
}

/// Dispatch a Zircon syscall
pub fn dispatch_zircon_syscall(syscall_num: u32, args: [usize; 8]) -> ZxResult<usize> {
    match syscall_num {
        num if num == ZirconSyscall::ClockGet as u32
            || num == ZirconSyscall::ClockGetNew as u32 =>
        {
            sys_clock_get(args[0] as u32, args[1]).map(|_| 0)
        }
        num if num == ZirconSyscall::ClockGetMonotonic as u32 => {
            sys_clock_get_monotonic().map(|value| value as usize)
        }
        num if num == ZirconSyscall::Nanosleep as u32 => sys_nanosleep(args[0] as u64).map(|_| 0),
        num if num == ZirconSyscall::ClockAdjust as u32 => {
            sys_clock_adjust(args[0] as u32, args[1] as u32, args[2] as u64).map(|_| 0)
        }
        num if num == ZirconSyscall::SystemGetEvent as u32 => {
            let mut out = 0u32;
            sys_system_get_event(args[0] as u32, args[1] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::HandleClose as u32 => {
            sys_handle_close(args[0] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::HandleCloseMany as u32 => {
            sys_handle_close_many(args[0], args[1]).map(|_| 0)
        }
        num if num == ZirconSyscall::HandleDuplicate as u32 => {
            let mut out = 0u32;
            sys_handle_duplicate(args[0] as u32, args[1] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::HandleReplace as u32 => {
            let mut out = 0u32;
            sys_handle_replace(args[0] as u32, args[1] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::ObjectGetInfo as u32 => {
            let mut actual = 0usize;
            sys_object_get_info(
                args[0] as u32,
                args[1] as u32,
                args[2],
                args[3],
                &mut actual,
            )
            .map(|_| actual)
        }
        num if num == ZirconSyscall::ObjectGetProperty as u32 => {
            sys_object_get_property(args[0] as u32, args[1] as u32, args[2], args[3]).map(|_| 0)
        }
        num if num == ZirconSyscall::ObjectSetProperty as u32 => {
            sys_object_set_property(args[0] as u32, args[1] as u32, args[2], args[3]).map(|_| 0)
        }
        num if num == ZirconSyscall::ObjectSignal as u32 => {
            sys_object_signal(args[0] as u32, args[1] as u32, args[2] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::ObjectSignalPeer as u32 => {
            sys_object_signal_peer(args[0] as u32, args[1] as u32, args[2] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::ObjectWaitOne as u32 => {
            let mut pending = 0u32;
            sys_object_wait_one(args[0] as u32, args[1] as u32, args[2] as u64, &mut pending)
                .map(|_| pending as usize)
        }
        num if num == ZirconSyscall::ObjectWaitMany as u32 => {
            sys_object_wait_many(args[0], args[1], args[2] as u64).map(|_| 0)
        }
        num if num == ZirconSyscall::ObjectWaitAsync as u32 => sys_object_wait_async(
            args[0] as u32,
            args[1] as u32,
            args[2] as u64,
            args[3] as u32,
            args[4] as u32,
        )
        .map(|_| 0),
        num if num == ZirconSyscall::ObjectGetChild as u32 => {
            let mut out = 0u32;
            sys_object_get_child(args[0] as u32, args[1] as u64, args[2] as u32, &mut out)
                .map(|_| out as usize)
        }
        num if num == ZirconSyscall::ObjectSetProfile as u32 => {
            sys_object_set_profile(args[0] as u32, args[1] as u32, args[2] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::ThreadCreate as u32 => {
            let mut out = 0u32;
            sys_thread_create(args[0] as u32, args[1], args[2], 0, 0, &mut out)
                .map(|_| out as usize)
        }
        num if num == ZirconSyscall::ThreadStart as u32 => {
            sys_thread_start(args[0] as u32, args[1], args[2], args[3], args[4]).map(|_| 0)
        }
        num if num == ZirconSyscall::ThreadReadState as u32 => {
            sys_thread_read_state(args[0] as u32, args[1] as u32, args[2], args[3]).map(|_| 0)
        }
        num if num == ZirconSyscall::ThreadWriteState as u32 => {
            sys_thread_write_state(args[0] as u32, args[1] as u32, args[2], args[3]).map(|_| 0)
        }
        num if num == ZirconSyscall::TaskKill as u32 => sys_task_kill(args[0] as u32).map(|_| 0),
        num if num == ZirconSyscall::ThreadExit as u32 => sys_thread_exit().map(|_| 0),
        num if num == ZirconSyscall::ProcessStart as u32 => sys_process_start(
            args[0] as u32,
            args[1] as u32,
            args[2],
            args[3],
            args[4],
            args[5],
        )
        .map(|_| 0),
        num if num == ZirconSyscall::ProcessReadMemory as u32 => {
            sys_process_read_memory(args[0] as u32, args[1], args[2], args[3])
        }
        num if num == ZirconSyscall::ProcessWriteMemory as u32 => {
            sys_process_write_memory(args[0] as u32, args[1], args[2], args[3])
        }
        num if num == ZirconSyscall::JobCreate as u32 => {
            let mut out = 0u32;
            sys_job_create(args[0] as u32, args[1] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::JobSetPolicy as u32 => sys_job_set_policy(
            args[0] as u32,
            args[1] as u32,
            args[2] as u32,
            args[3],
            args[4],
        )
        .map(|_| 0),
        num if num == ZirconSyscall::JobSetCritical as u32 => {
            sys_job_set_critical(args[0] as u32, args[1] as u32, args[2] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::TaskBindExceptionPort as u32 => sys_task_bind_exception_port(
            args[0] as u32,
            args[1] as u32,
            args[2] as u64,
            args[3] as u32,
        )
        .map(|_| 0),
        num if num == ZirconSyscall::TaskSuspend as u32
            || num == ZirconSyscall::TaskSuspendToken as u32 =>
        {
            let mut out = 0u32;
            sys_task_suspend_token(args[0] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::TaskResumeFromException as u32 => {
            sys_task_resume_from_exception(args[0] as u32, args[1] as u32, args[2] as u32)
                .map(|_| 0)
        }
        num if num == ZirconSyscall::TaskCreateExceptionChannel as u32 => {
            let mut out = 0u32;
            sys_create_exception_channel(args[0] as u32, args[1] as u32, &mut out)
                .map(|_| out as usize)
        }
        num if num == ZirconSyscall::ExceptionGetThread as u32 => {
            let mut out = 0u32;
            sys_exception_get_thread(args[0] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::ExceptionGetProcess as u32 => {
            let mut out = 0u32;
            sys_exception_get_process(args[0] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::VmoCreate as u32 => {
            let mut out = 0u32;
            sys_vmo_create(args[0] as u64, args[1] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::VmoRead as u32 => {
            if !syscall_logic::user_buffer_valid(args[1], args[2]) {
                return Err(ZxError::ErrInvalidArgs);
            }
            if args[2] == 0 {
                sys_vmo_read(args[0] as u32, &mut [], args[3] as u64)
            } else {
                let buf = unsafe { core::slice::from_raw_parts_mut(args[1] as *mut u8, args[2]) };
                sys_vmo_read(args[0] as u32, buf, args[3] as u64)
            }
        }
        num if num == ZirconSyscall::VmoWrite as u32 => {
            if !syscall_logic::user_buffer_valid(args[1], args[2]) {
                return Err(ZxError::ErrInvalidArgs);
            }
            if args[2] == 0 {
                sys_vmo_write(args[0] as u32, &[], args[3] as u64)
            } else {
                let buf = unsafe { core::slice::from_raw_parts(args[1] as *const u8, args[2]) };
                sys_vmo_write(args[0] as u32, buf, args[3] as u64)
            }
        }
        num if num == ZirconSyscall::VmoGetSize as u32 => {
            let mut size = 0usize;
            sys_vmo_get_size(args[0] as u32, &mut size).map(|_| size)
        }
        num if num == ZirconSyscall::VmoSetSize as u32 => {
            sys_vmo_set_size(args[0] as u32, args[1]).map(|_| 0)
        }
        num if num == ZirconSyscall::VmoOpRange as u32 => {
            sys_vmo_op_range(args[0] as u32, args[1] as u32, args[2], args[3])
        }
        num if num == ZirconSyscall::VmoCreateChild as u32 => {
            let mut out = 0u32;
            sys_vmo_create_child(args[0] as u32, args[1] as u32, args[2], args[3], &mut out)
                .map(|_| out as usize)
        }
        num if num == ZirconSyscall::VmoSetCachePolicy as u32 => {
            sys_vmo_set_cache_policy(args[0] as u32, args[1] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::VmoReplaceAsExecutable as u32 => {
            let mut out = 0u32;
            sys_vmo_replace_as_executable(args[0] as u32, args[1] as u32, &mut out)
                .map(|_| out as usize)
        }
        num if num == ZirconSyscall::VmoCreateContiguous as u32 => {
            let mut out = 0u32;
            sys_vmo_create_contiguous(args[0] as u32, args[1], args[2] as u32, &mut out)
                .map(|_| out as usize)
        }
        num if num == ZirconSyscall::VmoCreatePhysical as u32 => {
            let mut out = 0u32;
            sys_vmo_create_physical(args[0] as u32, args[1] as u64, args[2], &mut out)
                .map(|_| out as usize)
        }
        num if num == ZirconSyscall::VmarMap as u32 => {
            let mut addr = 0usize;
            sys_vmar_map(
                args[0] as u32,
                args[1] as u32,
                args[2],
                args[3] as u32,
                args[4],
                args[5],
                &mut addr,
            )
            .map(|_| addr)
        }
        num if num == ZirconSyscall::VmarUnmap as u32 => {
            sys_vmar_unmap(args[0] as u32, args[1], args[2]).map(|_| 0)
        }
        num if num == ZirconSyscall::VmarAllocate as u32 => {
            let mut child = 0u32;
            let mut addr = 0usize;
            sys_vmar_allocate(
                args[0] as u32,
                args[1] as u32,
                args[2] as u64,
                args[3] as u64,
                &mut child,
                &mut addr,
            )
            .map(|_| child as usize)
        }
        num if num == ZirconSyscall::VmarProtect as u32 => sys_vmar_protect(
            args[0] as u32,
            args[1] as u32,
            args[2] as u64,
            args[3] as u64,
        )
        .map(|_| 0),
        num if num == ZirconSyscall::VmarDestroy as u32 => {
            sys_vmar_destroy(args[0] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::VmarUnmapHandleCloseThreadExit as u32 => {
            sys_vmar_unmap_handle_close_thread_exit(args[0] as u32, args[1], args[2]).map(|_| 0)
        }
        num if num == ZirconSyscall::ProcessCreate as u32 => {
            let mut proc_h = 0u32;
            let mut vmar_h = 0u32;
            sys_process_create(
                args[0] as u32,
                args[1],
                args[2],
                args[3] as u32,
                &mut proc_h,
                &mut vmar_h,
            )
            .map(|_| proc_h as usize)
        }
        num if num == ZirconSyscall::ProcessExit as u32 => {
            sys_process_exit(args[0] as u32, args[1] as i32).map(|_| 0)
        }
        num if num == ZirconSyscall::ChannelCreate as u32 => {
            let mut h0 = 0u32;
            let mut h1 = 0u32;
            sys_channel_create(args[0] as u32, &mut h0, &mut h1)
                .map(|_| ((h0 as usize) << 32) | h1 as usize)
        }
        num if num == ZirconSyscall::ChannelRead as u32 => {
            let mut actual_bytes = 0usize;
            let mut actual_handles = 0usize;
            sys_channel_read(
                args[0] as u32,
                args[1] as u32,
                args[2],
                args[3],
                args[4],
                args[5],
                &mut actual_bytes,
                &mut actual_handles,
            )
            .map(|_| actual_bytes)
        }
        num if num == ZirconSyscall::ChannelReadEtc as u32 => {
            let mut actual_bytes = 0usize;
            let mut actual_handles = 0usize;
            sys_channel_read(
                args[0] as u32,
                args[1] as u32,
                args[2],
                args[3],
                args[4],
                args[5],
                &mut actual_bytes,
                &mut actual_handles,
            )
            .map(|_| actual_bytes)
        }
        num if num == ZirconSyscall::ChannelWrite as u32 => sys_channel_write(
            args[0] as u32,
            args[1] as u32,
            args[2],
            args[3],
            args[4],
            args[5],
        )
        .map(|_| 0),
        num if num == ZirconSyscall::ChannelWriteEtc as u32 => sys_channel_write(
            args[0] as u32,
            args[1] as u32,
            args[2],
            args[3],
            args[4],
            args[5],
        )
        .map(|_| 0),
        num if num == ZirconSyscall::ChannelCallNoretry as u32 => {
            let mut actual_bytes = 0usize;
            let mut actual_handles = 0usize;
            sys_channel_call_noretry(
                args[0] as u32,
                args[1] as u32,
                args[2],
                args[3],
                args[4],
                args[5],
                args[6],
                args[7],
                &mut actual_bytes,
                &mut actual_handles,
            )
            .map(|_| actual_bytes)
        }
        num if num == ZirconSyscall::ChannelCallFinish as u32 => Err(ZxError::ErrNotSupported),
        num if num == ZirconSyscall::SocketCreate as u32 => {
            let mut h0 = 0u32;
            let mut h1 = 0u32;
            sys_socket_create(args[0] as u32, &mut h0, &mut h1)
                .map(|_| ((h0 as usize) << 32) | h1 as usize)
        }
        num if num == ZirconSyscall::SocketWrite as u32 => {
            let mut actual = 0usize;
            sys_socket_write(
                args[0] as u32,
                args[1] as u32,
                args[2],
                args[3],
                &mut actual,
            )
            .map(|_| actual)
        }
        num if num == ZirconSyscall::SocketRead as u32 => {
            let mut actual = 0usize;
            sys_socket_read(
                args[0] as u32,
                args[1] as u32,
                args[2],
                args[3],
                &mut actual,
            )
            .map(|_| actual)
        }
        num if num == ZirconSyscall::SocketShare as u32 => {
            sys_socket_share(args[0] as u32, args[1] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::SocketAccept as u32 => {
            let mut out = 0u32;
            sys_socket_accept(args[0] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::SocketShutdown as u32 => {
            sys_socket_shutdown(args[0] as u32, args[1] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::EventCreate as u32 => {
            let mut out = 0u32;
            sys_event_create(args[0] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::EventpairCreate as u32 => {
            let mut h0 = 0u32;
            let mut h1 = 0u32;
            sys_eventpair_create(args[0] as u32, &mut h0, &mut h1)
                .map(|_| ((h0 as usize) << 32) | h1 as usize)
        }
        num if num == ZirconSyscall::FifoCreate as u32 => {
            let mut h0 = 0u32;
            let mut h1 = 0u32;
            sys_fifo_create(args[0], args[1], args[2] as u32, &mut h0, &mut h1)
                .map(|_| ((h0 as usize) << 32) | h1 as usize)
        }
        num if num == ZirconSyscall::FifoRead as u32 => {
            let mut actual = 0usize;
            sys_fifo_read(args[0] as u32, args[1], args[2], args[3], &mut actual).map(|_| actual)
        }
        num if num == ZirconSyscall::FifoWrite as u32 => {
            let mut actual = 0usize;
            sys_fifo_write(args[0] as u32, args[1], args[2], args[3], &mut actual).map(|_| actual)
        }
        num if num == ZirconSyscall::PortCreate as u32 => {
            let mut out = 0u32;
            sys_port_create(args[0] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::PortQueue as u32 => {
            sys_port_queue(args[0] as u32, args[1]).map(|_| 0)
        }
        num if num == ZirconSyscall::PortWait as u32 => {
            sys_port_wait(args[0] as u32, args[1] as u64, args[2]).map(|_| 0)
        }
        num if num == ZirconSyscall::PortCancel as u32 => {
            sys_port_cancel(args[0] as u32, args[1] as u32, args[2] as u64)
                .map(|removed| removed as usize)
        }
        num if num == ZirconSyscall::TimerCreate as u32 => {
            let mut out = 0u32;
            sys_timer_create(args[0] as u32, args[1] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::TimerSet as u32 => {
            sys_timer_set(args[0] as u32, args[1] as u64, args[2] as i64).map(|_| 0)
        }
        num if num == ZirconSyscall::TimerCancel as u32 => {
            sys_timer_cancel(args[0] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::FutexWait as u32 => {
            sys_futex_wait(args[0], args[1] as i32, args[2] as u32, args[3] as u64).map(|_| 0)
        }
        num if num == ZirconSyscall::FutexWake as u32 => {
            sys_futex_wake(args[0], args[1] as u32).map(|count| count as usize)
        }
        num if num == ZirconSyscall::FutexWakeSingleOwner as u32 => {
            sys_futex_wake_single_owner(args[0]).map(|count| count as usize)
        }
        num if num == ZirconSyscall::FutexRequeue as u32
            || num == ZirconSyscall::FutexRequeueSingleOwner as u32 =>
        {
            sys_futex_requeue(
                args[0],
                args[1] as u32,
                args[2] as i32,
                args[3],
                args[4] as u32,
                args[5] as u32,
            )
            .map(|(woken, requeued)| ((woken as usize) << 32) | requeued as usize)
        }
        num if num == ZirconSyscall::FutexGetOwner as u32 => {
            sys_futex_get_owner(args[0]).map(|owner| owner as usize)
        }
        num if num == ZirconSyscall::CprngDrawOnce as u32 => {
            sys_cprng_draw_once(args[0], args[1]).map(|_| 0)
        }
        num if num == ZirconSyscall::CprngAddEntropy as u32 => {
            sys_cprng_add_entropy(args[0], args[1]).map(|_| 0)
        }
        num if num == ZirconSyscall::ProfileCreate as u32 => {
            let mut out = 0u32;
            sys_profile_create(args[0] as u32, args[1] as u32, args[2], &mut out)
                .map(|_| out as usize)
        }
        num if num == ZirconSyscall::KtraceRead as u32 => {
            let mut actual = 0usize;
            sys_ktrace_read(args[0] as u32, args[1], args[2], &mut actual).map(|_| actual)
        }
        num if num == ZirconSyscall::KtraceControl as u32 => {
            sys_ktrace_control(args[0] as u32, args[1] as u32, args[2] as u32, args[3]).map(|_| 0)
        }
        num if num == ZirconSyscall::KtraceWrite as u32 => sys_ktrace_write(
            args[0] as u32,
            args[1] as u32,
            args[2] as u64,
            args[3] as u64,
        )
        .map(|_| 0),
        num if num == ZirconSyscall::MtraceControl as u32 => sys_mtrace_control(
            args[0] as u32,
            args[1] as u32,
            args[2] as u32,
            args[3] as u32,
            args[4],
        )
        .map(|_| 0),
        num if num == ZirconSyscall::DebugRead as u32 => sys_debug_read(args[0], args[1]),
        num if num == ZirconSyscall::ClockCreate as u32 => {
            let mut out = 0u32;
            sys_clock_create(args[0] as u32, args[1], &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::ClockRead as u32 => {
            sys_clock_read(args[0] as u32, args[1]).map(|_| 0)
        }
        num if num == ZirconSyscall::ClockUpdate as u32 => {
            sys_clock_update(args[0] as u32, args[1] as u64, args[2]).map(|_| 0)
        }
        num if num == ZirconSyscall::DebugWrite as u32 => {
            sys_debug_write(args[0], args[1]).map(|_| 0)
        }
        num if num == ZirconSyscall::DebugSendCommand as u32 => {
            sys_debug_send_command(args[0], args[1]).map(|_| 0)
        }
        num if num == ZirconSyscall::DebuglogCreate as u32 => {
            let mut out = 0u32;
            sys_debuglog_create(args[0] as u32, args[1] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::DebuglogWrite as u32 => {
            sys_debuglog_write(args[0] as u32, args[1] as u32, args[2], args[3]).map(|_| 0)
        }
        num if num == ZirconSyscall::DebuglogRead as u32 => {
            sys_debuglog_read(args[0] as u32, args[1] as u32, args[2], args[3])
        }
        num if num == ZirconSyscall::ResourceCreate as u32 => {
            let mut out = 0u32;
            sys_resource_create(
                args[0] as u32,
                args[1] as u32,
                args[2] as u64,
                args[3],
                args[4],
                args[5],
                &mut out,
            )
            .map(|_| out as usize)
        }
        num if num == ZirconSyscall::StreamCreate as u32 => {
            let mut out = 0u32;
            sys_stream_create(args[0] as u32, args[1] as u32, args[2], &mut out)
                .map(|_| out as usize)
        }
        num if num == ZirconSyscall::StreamSeek as u32 => {
            let mut seek = 0usize;
            sys_stream_seek(args[0] as u32, args[1] as u32, args[2] as i64, &mut seek).map(|_| seek)
        }
        num if num == ZirconSyscall::StreamWritev as u32 => {
            sys_stream_writev(args[0] as u32, args[1] as u32, args[2], args[3], args[4]).map(|_| 0)
        }
        num if num == ZirconSyscall::StreamWritevAt as u32 => sys_stream_writev_at(
            args[0] as u32,
            args[1] as u32,
            args[2],
            args[3],
            args[4],
            args[5],
        )
        .map(|_| 0),
        num if num == ZirconSyscall::StreamReadv as u32 => {
            sys_stream_readv(args[0] as u32, args[1] as u32, args[2], args[3], args[4]).map(|_| 0)
        }
        num if num == ZirconSyscall::StreamReadvAt as u32 => sys_stream_readv_at(
            args[0] as u32,
            args[1] as u32,
            args[2],
            args[3],
            args[4],
            args[5],
        )
        .map(|_| 0),
        num if num == ZirconSyscall::FutexWakeHandleCloseThreadExit as u32 => {
            let _ = sys_futex_wake(args[0], args[1] as u32);
            let _ = sys_handle_close(args[3] as u32);
            sys_thread_exit().map(|_| 0)
        }
        num if num == ZirconSyscall::IommuCreate as u32 => {
            let mut out = 0u32;
            sys_iommu_create(args[0] as u32, args[1] as u32, args[2], args[3], &mut out)
                .map(|_| out as usize)
        }
        num if num == ZirconSyscall::BtiCreate as u32 => {
            let mut out = 0u32;
            sys_bti_create(args[0] as u32, args[1] as u32, args[2] as u64, &mut out)
                .map(|_| out as usize)
        }
        num if num == ZirconSyscall::BtiPin as u32 => {
            let mut out = 0u32;
            sys_bti_pin(
                args[0] as u32,
                args[1] as u32,
                args[2] as u32,
                args[3],
                args[4],
                args[5],
                args[6],
                &mut out,
            )
            .map(|_| out as usize)
        }
        num if num == ZirconSyscall::BtiReleaseQuarantine as u32
            || num == ZirconSyscall::BtiReleaseQuarantineCompat as u32 =>
        {
            sys_bti_release_quarantine(args[0] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::PmtUnpin as u32 => sys_pmt_unpin(args[0] as u32).map(|_| 0),
        num if num == ZirconSyscall::FramebufferGetInfo as u32 => {
            sys_framebuffer_get_info(args[0]).map(|_| 0)
        }
        num if num == ZirconSyscall::FramebufferSetRange as u32 => {
            sys_framebuffer_set_range(args[0] as u32, args[1], args[2] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::InterruptCreate as u32 => {
            let mut out = 0u32;
            sys_interrupt_create(args[0] as u32, args[1], args[2] as u32, &mut out)
                .map(|_| out as usize)
        }
        num if num == ZirconSyscall::InterruptBind as u32 => sys_interrupt_bind(
            args[0] as u32,
            args[1] as u32,
            args[2] as u64,
            args[3] as u32,
        )
        .map(|_| 0),
        num if num == ZirconSyscall::InterruptBindVcpu as u32 => {
            sys_interrupt_bind_vcpu(args[0] as u32, args[1] as u32, args[2] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::InterruptWait as u32 => {
            sys_interrupt_wait(args[0] as u32, args[1]).map(|_| 0)
        }
        num if num == ZirconSyscall::InterruptDestroy as u32
            || num == ZirconSyscall::InterruptDestroyCompat as u32 =>
        {
            sys_interrupt_destroy(args[0] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::InterruptAck as u32
            || num == ZirconSyscall::InterruptAckCompat as u32 =>
        {
            sys_interrupt_ack(args[0] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::InterruptTrigger as u32
            || num == ZirconSyscall::InterruptTriggerCompat as u32 =>
        {
            sys_interrupt_trigger(args[0] as u32, args[1] as u32, args[2] as i64).map(|_| 0)
        }
        num if num == ZirconSyscall::IoportsRequest as u32
            || num == ZirconSyscall::IoportsRequestCompat as u32 =>
        {
            Err(ZxError::ErrNotSupported)
        }
        num if num == ZirconSyscall::PciGetNthDevice as u32 => {
            let mut out = 0u32;
            sys_pci_get_nth_device(args[0] as u32, args[1] as u32, args[2], &mut out)
                .map(|_| out as usize)
        }
        num if num == ZirconSyscall::PciEnableBusMaster as u32 => {
            sys_pci_enable_bus_master(args[0] as u32, args[1] != 0).map(|_| 0)
        }
        num if num == ZirconSyscall::PciResetDevice as u32 => {
            sys_pci_reset_device(args[0] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::PciConfigRead as u32 => {
            sys_pci_config_read(args[0] as u32, args[1], args[2], args[3]).map(|_| 0)
        }
        num if num == ZirconSyscall::PciConfigWrite as u32 => {
            sys_pci_config_write(args[0] as u32, args[1], args[2], args[3] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::PciCfgPioRw as u32 => sys_pci_cfg_pio_rw(
            args[0] as u32,
            args[1] as u8,
            args[2] as u8,
            args[3] as u8,
            args[4] as u8,
            args[5],
            args[6],
            args[7] != 0,
        )
        .map(|_| 0),
        num if num == ZirconSyscall::PciGetBar as u32 => {
            let mut out = 0u32;
            sys_pci_get_bar(args[0] as u32, args[1] as u32, args[2], &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::PciMapInterrupt as u32 => {
            let mut out = 0u32;
            sys_pci_map_interrupt(args[0] as u32, args[1] as i32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::PciQueryIrqMode as u32 => {
            sys_pci_query_irq_mode(args[0] as u32, args[1] as u32, args[2]).map(|_| 0)
        }
        num if num == ZirconSyscall::PciSetIrqMode as u32 => {
            sys_pci_set_irq_mode(args[0] as u32, args[1] as u32, args[2] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::PciInit as u32 => {
            sys_pci_init(args[0] as u32, args[1], args[2] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::PciAddSubtractIoRange as u32 => sys_pci_add_subtract_io_range(
            args[0] as u32,
            args[1] != 0,
            args[2] as u64,
            args[3] as u64,
            args[4] != 0,
        )
        .map(|_| 0),
        num if num == ZirconSyscall::PcFirmwareTables as u32
            || num == ZirconSyscall::PcFirmwareTablesCompat as u32 =>
        {
            sys_pc_firmware_tables(args[0] as u32, args[1], args[2]).map(|_| 0)
        }
        num if num == ZirconSyscall::SmcCall as u32 => {
            sys_smc_call(args[0] as u32, args[1], args[2]).map(|_| 0)
        }
        num if num == ZirconSyscall::GuestCreate as u32 => {
            let mut guest = 0u32;
            let mut vmar = 0u32;
            sys_guest_create(args[0] as u32, args[1] as u32, &mut guest, &mut vmar)
                .map(|_| ((guest as usize) << 32) | vmar as usize)
        }
        num if num == ZirconSyscall::GuestSetTrap as u32 => sys_guest_set_trap(
            args[0] as u32,
            args[1] as u32,
            args[2] as u64,
            args[3] as u64,
            args[4] as u32,
            args[5] as u64,
        )
        .map(|_| 0),
        num if num == ZirconSyscall::VcpuCreate as u32 => {
            let mut out = 0u32;
            sys_vcpu_create(args[0] as u32, args[1] as u32, args[2] as u64, &mut out)
                .map(|_| out as usize)
        }
        num if num == ZirconSyscall::VcpuResume as u32 => {
            sys_vcpu_resume(args[0] as u32, args[1]).map(|_| 0)
        }
        num if num == ZirconSyscall::VcpuInterrupt as u32 => {
            sys_vcpu_interrupt(args[0] as u32, args[1] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::VcpuReadState as u32 => {
            sys_vcpu_read_state(args[0] as u32, args[1] as u32, args[2], args[3]).map(|_| 0)
        }
        num if num == ZirconSyscall::VcpuWriteState as u32 => {
            sys_vcpu_write_state(args[0] as u32, args[1] as u32, args[2], args[3]).map(|_| 0)
        }
        num if num == ZirconSyscall::SystemMexec as u32 => {
            sys_system_mexec(args[0] as u32, args[1] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::SystemMexecPayloadGet as u32 => {
            sys_system_mexec_payload_get(args[0], args[1]).map(|_| 0)
        }
        num if num == ZirconSyscall::SystemPowerctl as u32 => {
            sys_system_powerctl(args[0] as u32, args[1] as u32, args[2]).map(|_| 0)
        }
        num if num == ZirconSyscall::PagerCreate as u32 => {
            let mut out = 0u32;
            sys_pager_create(args[0] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::PagerCreateVmo as u32 => {
            let mut out = 0u32;
            sys_pager_create_vmo(
                args[0] as u32,
                args[1] as u32,
                args[2] as u32,
                args[3] as u64,
                args[4],
                &mut out,
            )
            .map(|_| out as usize)
        }
        num if num == ZirconSyscall::PagerDetachVmo as u32 => {
            sys_pager_detach_vmo(args[0] as u32, args[1] as u32).map(|_| 0)
        }
        num if num == ZirconSyscall::PagerSupplyPages as u32 => sys_pager_supply_pages(
            args[0] as u32,
            args[1] as u32,
            args[2],
            args[3],
            args[4] as u32,
            args[5],
        )
        .map(|_| 0),
        num if num == ZirconSyscall::SyscallTest0 as u32
            || num == ZirconSyscall::SyscallTest1 as u32
            || num == ZirconSyscall::SyscallTest2 as u32
            || num == ZirconSyscall::SyscallTest3 as u32
            || num == ZirconSyscall::SyscallTest4 as u32
            || num == ZirconSyscall::SyscallTest5 as u32
            || num == ZirconSyscall::SyscallTest6 as u32
            || num == ZirconSyscall::SyscallTest7 as u32
            || num == ZirconSyscall::SyscallTest8 as u32
            || num == ZirconSyscall::SyscallTestWrapper as u32 =>
        {
            Ok(0)
        }
        num if num == ZirconSyscall::Count as u32 => Err(ZxError::ErrNotSupported),
        num if num == ZirconSyscall::SystemGetEventCompat as u32 => {
            let mut out = 0u32;
            sys_system_get_event(args[0] as u32, args[1] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::TaskCreateExceptionChannelCompat as u32 => {
            let mut out = 0u32;
            sys_create_exception_channel(args[0] as u32, args[1] as u32, &mut out)
                .map(|_| out as usize)
        }
        num if num == ZirconSyscall::ExceptionGetThreadCompat as u32 => {
            let mut out = 0u32;
            sys_exception_get_thread(args[0] as u32, &mut out).map(|_| out as usize)
        }
        num if num == ZirconSyscall::ExceptionGetProcessCompat as u32 => {
            let mut out = 0u32;
            sys_exception_get_process(args[0] as u32, &mut out).map(|_| out as usize)
        }
        num if syscall_logic::zircon_syscall_interface_known(num) => {
            warn!("Unsupported Zircon syscall interface: {}", syscall_num);
            Err(ZxError::ErrNotSupported)
        }
        _ => {
            warn!("Unimplemented Zircon syscall: {}", syscall_num);
            Err(ZxError::ErrNotSupported)
        }
    }
}

// ============================================================================
// Initialization
// ============================================================================

/// Initialize syscall subsystem
pub fn init() {
    info!("Initializing syscall interface layer...");
    info!("  - Linux syscall interface: ready");
    info!("  - Zircon syscall interface: ready");
    info!("  - Handle management: ready");
    info!("  - VMO/VMAR support: ready");
}
