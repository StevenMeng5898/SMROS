//! EL0 negative cases for the kernel-security baseline.
//!
//! `sectest` is not part of the default smoke command list. It builds a real
//! AArch64 EL0 process with its own TTBR0 root, copies a fixed machine-code
//! probe into a user page, and records fault or syscall results.
//!
//! SEC-0 through SEC-4 cases are hard PASS/FAIL. `sectest all` executes the
//! undefined instruction, the unaligned access, and the exit-state cases.
//! A fault must leave the shell able to print `SEC-HEARTBEAT`. Fault cases
//! also run a second thread that completes only after the fault is observed.

use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicU8, AtomicUsize, Ordering};

use alloc::vec;
use alloc::vec::Vec;

use crate::kernel_lowlevel::serial::Serial;
use crate::kernel_objects::scheduler::{self, yield_now};
use crate::syscall::{linux_process, linux_process_memory, linux_task, SysError};

const DATA_PAGE: usize = 0x1200_0000;
const CODE_PAGE: usize = 0x1200_1000;
const STACK_PAGE: usize = 0x1200_2000;
const RO_PAGE: usize = 0x1200_4000;
const STACK_PAGES: usize = 2;
const PAGE_SIZE: usize = 0x1000;
const OP_READ: u64 = 1;
const OP_WRITE: u64 = 2;
const OP_EXEC: u64 = 3;
const OP_SYSCALL: u64 = 4;
const OP_UNDEF: u64 = 5;
const OP_MPROTECT: u64 = 6;
const OP_COW: u64 = 7;
const OP_ALIGN: u64 = 8;
const OP_EXIT_FUTEX: u64 = 9;
const OP_WAIT_ONCE: u64 = 10;
const OP_EXIT_REFORK: u64 = 11;
const OP_BAD_HANDLER: u64 = 12;
const OP_CHMOD: u64 = 13;
const OP_NOTIFY_GATE: u64 = 14;
const REFORK_PAGE: usize = 0x1600_0000;
const REFORK_PARENT: u64 = 0x1111;
const OTHER_PAGE: usize = 0x1300_0000;
const OTHER_MAGIC: u64 = 0x4F54;
const COW_PAGE: usize = 0x1400_0000;
const COW_PARENT: u64 = 0x1111;
const COW_CHILD: u64 = 0x2222;
const MPROTECT_PAGE: usize = 0x1500_0000;
const MPROTECT_FIRST: u64 = 0x2222;
const MPROTECT_SECOND: u64 = 0x3333;
const MAGIC: u64 = 0x5345_434d_4147_4943;
const CASE_TIMEOUT_NS: u64 = 2_000_000_000;

include!("security_case_logic_shared.rs");

/// Bytes of `tests/security/aarch64/el0_probe.S` (`.text` only).
const PROBE_CODE: &[u8] = &[
    0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x95, 0x02, 0x40, 0xf9, 0x96, 0x06, 0x40, 0xf9,
    0xbf, 0x06, 0x00, 0xf1, 0xa0, 0x03, 0x00, 0x54, 0xbf, 0x0a, 0x00, 0xf1, 0xc0, 0x03, 0x00, 0x54,
    0xbf, 0x0e, 0x00, 0xf1, 0x00, 0x04, 0x00, 0x54, 0xbf, 0x12, 0x00, 0xf1, 0xe0, 0x03, 0x00, 0x54,
    0xbf, 0x16, 0x00, 0xf1, 0x40, 0x05, 0x00, 0x54, 0xbf, 0x1a, 0x00, 0xf1, 0x20, 0x05, 0x00, 0x54,
    0xbf, 0x1e, 0x00, 0xf1, 0xa0, 0x06, 0x00, 0x54, 0xbf, 0x22, 0x00, 0xf1, 0xa0, 0x0b, 0x00, 0x54,
    0xbf, 0x26, 0x00, 0xf1, 0x80, 0x0b, 0x00, 0x54, 0xbf, 0x2a, 0x00, 0xf1, 0x60, 0x0f, 0x00, 0x54,
    0xbf, 0x2e, 0x00, 0xf1, 0xc0, 0x13, 0x00, 0x54, 0xbf, 0x32, 0x00, 0xf1, 0x20, 0x1c, 0x00, 0x54,
    0xbf, 0x36, 0x00, 0xf1, 0xa0, 0x1e, 0x00, 0x54, 0xbf, 0x3a, 0x00, 0xf1, 0x20, 0x29, 0x00, 0x54,
    0x60, 0x0c, 0x80, 0xd2, 0x4e, 0x02, 0x00, 0x14, 0xc0, 0x02, 0x40, 0xf9, 0x80, 0x1e, 0x00, 0xf9,
    0x4b, 0x02, 0x00, 0x14, 0x80, 0x0a, 0x40, 0xf9, 0xc0, 0x02, 0x00, 0xf9, 0x00, 0x00, 0x80, 0xd2,
    0x47, 0x02, 0x00, 0x14, 0xc0, 0x02, 0x1f, 0xd6, 0x88, 0x0e, 0x40, 0xf9, 0x80, 0x12, 0x40, 0xf9,
    0x81, 0x16, 0x40, 0xf9, 0x82, 0x1a, 0x40, 0xf9, 0x83, 0x22, 0x40, 0xf9, 0x84, 0x26, 0x40, 0xf9,
    0x85, 0x2a, 0x40, 0xf9, 0x01, 0x00, 0x00, 0xd4, 0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2,
    0x80, 0x1e, 0x00, 0xf9, 0x00, 0x00, 0x80, 0xd2, 0x39, 0x02, 0x00, 0x14, 0x00, 0x00, 0x00, 0x00,
    0x80, 0x0a, 0x40, 0xf9, 0xc0, 0x02, 0x00, 0xf9, 0xe0, 0x03, 0x16, 0xaa, 0x01, 0x00, 0x82, 0xd2,
    0x22, 0x00, 0x80, 0xd2, 0x48, 0x1c, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0x14, 0x00, 0x80, 0xd2,
    0x14, 0x40, 0xa2, 0xf2, 0x96, 0x06, 0x40, 0xf9, 0x80, 0x22, 0x40, 0xf9, 0xc0, 0x02, 0x00, 0xf9,
    0x00, 0x00, 0x80, 0xd2, 0x2a, 0x02, 0x00, 0x14, 0x20, 0x02, 0x80, 0xd2, 0x01, 0x00, 0x80, 0xd2,
    0x02, 0x00, 0x80, 0xd2, 0x03, 0x00, 0x80, 0xd2, 0x04, 0x00, 0x80, 0xd2, 0x88, 0x1b, 0x80, 0xd2,
    0x01, 0x00, 0x00, 0xd4, 0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x96, 0x06, 0x40, 0xf9,
    0xc0, 0x03, 0xf8, 0xb7, 0xe0, 0x02, 0x00, 0xb4, 0x00, 0x00, 0x80, 0x92, 0x01, 0x0b, 0x80, 0xd2,
    0x81, 0x02, 0x01, 0x8b, 0x02, 0x00, 0x80, 0xd2, 0x03, 0x00, 0x80, 0xd2, 0x88, 0x20, 0x80, 0xd2,
    0x01, 0x00, 0x00, 0xd4, 0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x96, 0x06, 0x40, 0xf9,
    0x40, 0x02, 0xf8, 0xb7, 0x20, 0x02, 0x00, 0xb4, 0x80, 0x5a, 0x40, 0xb9, 0x01, 0x40, 0x85, 0x52,
    0x1f, 0x00, 0x01, 0x6b, 0xa1, 0x01, 0x00, 0x54, 0xc0, 0x02, 0x40, 0xf9, 0x81, 0x12, 0x40, 0xf9,
    0x1f, 0x00, 0x01, 0xeb, 0xe1, 0x00, 0x00, 0x54, 0x20, 0x00, 0x80, 0xd2, 0x08, 0x02, 0x00, 0x14,
    0x80, 0x0a, 0x40, 0xf9, 0xc0, 0x02, 0x00, 0xf9, 0x40, 0x05, 0x80, 0xd2, 0x04, 0x02, 0x00, 0x14,
    0x40, 0x00, 0x80, 0xd2, 0x02, 0x02, 0x00, 0x14, 0x60, 0x00, 0x80, 0xd2, 0x00, 0x02, 0x00, 0x14,
    0xc0, 0x02, 0x1f, 0xd6, 0x00, 0xe0, 0x81, 0xd2, 0xa0, 0x00, 0xa0, 0xf2, 0x01, 0x00, 0x86, 0xd2,
    0x01, 0x40, 0xa2, 0xf2, 0x02, 0x00, 0x80, 0xd2, 0x03, 0x00, 0x80, 0xd2, 0x04, 0x00, 0x80, 0xd2,
    0x88, 0x1b, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0xc0, 0x02, 0xf8, 0xb7, 0x20, 0x01, 0x00, 0xb4,
    0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x81, 0x86, 0x40, 0xf9, 0xa1, 0xff, 0xff, 0xb4,
    0x00, 0x00, 0x80, 0xd2, 0xc8, 0x0b, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0xec, 0x01, 0x00, 0x14,
    0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x21, 0x00, 0x80, 0xd2, 0x81, 0x86, 0x00, 0xf9,
    0x80, 0x02, 0x04, 0x91, 0x01, 0x00, 0x80, 0xd2, 0x02, 0x00, 0x80, 0xd2, 0x03, 0x00, 0x80, 0xd2,
    0x48, 0x0c, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0x00, 0x00, 0x80, 0xd2, 0xe0, 0x01, 0x00, 0x14,
    0x80, 0x00, 0x80, 0xd2, 0xde, 0x01, 0x00, 0x14, 0x20, 0x02, 0x80, 0xd2, 0x01, 0x00, 0x80, 0xd2,
    0x02, 0x00, 0x80, 0xd2, 0x03, 0x00, 0x80, 0xd2, 0x04, 0x00, 0x80, 0xd2, 0x88, 0x1b, 0x80, 0xd2,
    0x01, 0x00, 0x00, 0xd4, 0x80, 0x03, 0xf8, 0xb7, 0xe0, 0x02, 0x00, 0xb4, 0x00, 0x00, 0x80, 0x92,
    0x01, 0x40, 0x80, 0xd2, 0x01, 0x40, 0xa2, 0xf2, 0x02, 0x00, 0x80, 0xd2, 0x03, 0x00, 0x80, 0xd2,
    0x88, 0x20, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0x60, 0x02, 0xf8, 0xb7, 0x40, 0x02, 0x00, 0xb4,
    0x00, 0x00, 0x80, 0x92, 0x01, 0x40, 0x80, 0xd2, 0x01, 0x40, 0xa2, 0xf2, 0x02, 0x00, 0x80, 0xd2,
    0x03, 0x00, 0x80, 0xd2, 0x88, 0x20, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0x1f, 0x28, 0x00, 0xf1,
    0xe0, 0x00, 0x00, 0x54, 0x1f, 0x28, 0x00, 0xb1, 0xa1, 0x00, 0x00, 0x54, 0x20, 0x00, 0x80, 0xd2,
    0xbf, 0x01, 0x00, 0x14, 0x00, 0x00, 0x80, 0xd2, 0xbd, 0x01, 0x00, 0x14, 0x40, 0x00, 0x80, 0xd2,
    0xbb, 0x01, 0x00, 0x14, 0x60, 0x00, 0x80, 0xd2, 0xb9, 0x01, 0x00, 0x14, 0x20, 0x02, 0x80, 0xd2,
    0x01, 0x00, 0x80, 0xd2, 0x02, 0x00, 0x80, 0xd2, 0x03, 0x00, 0x80, 0xd2, 0x04, 0x00, 0x80, 0xd2,
    0x88, 0x1b, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0x80, 0x07, 0xf8, 0xb7, 0x00, 0x05, 0x00, 0xb4,
    0x00, 0x00, 0x80, 0x92, 0x01, 0x40, 0x80, 0xd2, 0x01, 0x40, 0xa2, 0xf2, 0x02, 0x00, 0x80, 0xd2,
    0x03, 0x00, 0x80, 0xd2, 0x88, 0x20, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0x60, 0x06, 0xf8, 0xb7,
    0x40, 0x06, 0x00, 0xb4, 0x16, 0x00, 0x80, 0xd2, 0x16, 0xc0, 0xa2, 0xf2, 0xc0, 0x02, 0x40, 0xf9,
    0x21, 0x22, 0x82, 0xd2, 0x1f, 0x00, 0x01, 0xeb, 0x41, 0x05, 0x00, 0x54, 0x20, 0x02, 0x80, 0xd2,
    0x01, 0x00, 0x80, 0xd2, 0x02, 0x00, 0x80, 0xd2, 0x03, 0x00, 0x80, 0xd2, 0x04, 0x00, 0x80, 0xd2,
    0x88, 0x1b, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0x80, 0x04, 0xf8, 0xb7, 0xc0, 0x02, 0x00, 0xb4,
    0x00, 0x00, 0x80, 0x92, 0x01, 0x40, 0x80, 0xd2, 0x01, 0x40, 0xa2, 0xf2, 0x02, 0x00, 0x80, 0xd2,
    0x03, 0x00, 0x80, 0xd2, 0x88, 0x20, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0x60, 0x03, 0xf8, 0xb7,
    0x40, 0x03, 0x00, 0xb4, 0x01, 0x40, 0x80, 0xd2, 0x01, 0x40, 0xa2, 0xf2, 0x20, 0x00, 0x40, 0xb9,
    0x80, 0x02, 0x00, 0x35, 0x20, 0x00, 0x80, 0xd2, 0x89, 0x01, 0x00, 0x14, 0x16, 0x00, 0x80, 0xd2,
    0x16, 0xc0, 0xa2, 0xf2, 0x40, 0x44, 0x84, 0xd2, 0xc0, 0x02, 0x00, 0xf9, 0x00, 0x00, 0x80, 0xd2,
    0x83, 0x01, 0x00, 0x14, 0x16, 0x00, 0x80, 0xd2, 0x16, 0xc0, 0xa2, 0xf2, 0xc0, 0x02, 0x40, 0xf9,
    0x41, 0x44, 0x84, 0xd2, 0x1f, 0x00, 0x01, 0xeb, 0xc0, 0x00, 0x00, 0x54, 0x21, 0x22, 0x82, 0xd2,
    0x1f, 0x00, 0x01, 0xeb, 0xa1, 0x00, 0x00, 0x54, 0x00, 0x00, 0x80, 0xd2, 0x78, 0x01, 0x00, 0x14,
    0x40, 0x00, 0x80, 0xd2, 0x76, 0x01, 0x00, 0x14, 0x60, 0x00, 0x80, 0xd2, 0x74, 0x01, 0x00, 0x14,
    0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x81, 0x02, 0x0c, 0x91, 0x00, 0x00, 0x80, 0xd2,
    0x00, 0xc2, 0xa2, 0xf2, 0x20, 0x00, 0x00, 0xf9, 0x3f, 0x04, 0x00, 0xf9, 0x3f, 0x08, 0x00, 0xf9,
    0x3f, 0x0c, 0x00, 0xf9, 0x60, 0x01, 0x80, 0xd2, 0x02, 0x00, 0x80, 0xd2, 0x03, 0x01, 0x80, 0xd2,
    0xc8, 0x10, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0xc0, 0x00, 0xf8, 0xb7, 0x16, 0x00, 0x80, 0xd2,
    0x16, 0xe0, 0xa2, 0xf2, 0xc0, 0x02, 0x40, 0xf9, 0x00, 0x00, 0x80, 0xd2, 0x60, 0x01, 0x00, 0x14,
    0xa0, 0x00, 0x80, 0xd2, 0x5e, 0x01, 0x00, 0x14, 0x20, 0x00, 0x80, 0xd2, 0x21, 0x00, 0x80, 0xd2,
    0x22, 0x00, 0x80, 0xd2, 0x68, 0x12, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0xe0, 0x07, 0xf8, 0xb7,
    0x60, 0x0c, 0x80, 0x92, 0x01, 0x80, 0x80, 0xd2, 0x01, 0x40, 0xa2, 0xf2, 0x42, 0x48, 0x80, 0xd2,
    0x83, 0x34, 0x80, 0xd2, 0x08, 0x07, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0x20, 0x07, 0xf8, 0xb7,
    0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x80, 0x42, 0x02, 0xf9, 0x80, 0x42, 0x42, 0xf9,
    0x01, 0x30, 0x80, 0xd2, 0x88, 0x06, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0x60, 0x06, 0xf8, 0xb7,
    0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x80, 0x42, 0x42, 0xf9, 0x01, 0xa0, 0x80, 0xd2,
    0x01, 0x40, 0xa2, 0xf2, 0x08, 0x0a, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0xa0, 0x05, 0xf8, 0xb7,
    0x01, 0xa0, 0x80, 0xd2, 0x01, 0x40, 0xa2, 0xf2, 0x20, 0x10, 0x40, 0xb9, 0x00, 0x2c, 0x00, 0x12,
    0x01, 0x30, 0x80, 0x52, 0x1f, 0x00, 0x01, 0x6b, 0x01, 0x05, 0x00, 0x54, 0x40, 0x00, 0x80, 0xd2,
    0x41, 0x00, 0x80, 0xd2, 0x42, 0x00, 0x80, 0xd2, 0x68, 0x12, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4,
    0x80, 0x04, 0xf8, 0xb7, 0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x80, 0x42, 0x42, 0xf9,
    0x01, 0x34, 0x80, 0xd2, 0x88, 0x06, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0x1f, 0x04, 0x00, 0xb1,
    0xc1, 0x03, 0x00, 0x54, 0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x80, 0x42, 0x42, 0xf9,
    0x01, 0xa0, 0x80, 0xd2, 0x01, 0x40, 0xa2, 0xf2, 0x08, 0x0a, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4,
    0x00, 0x03, 0xf8, 0xb7, 0x01, 0xa0, 0x80, 0xd2, 0x01, 0x40, 0xa2, 0xf2, 0x20, 0x10, 0x40, 0xb9,
    0x00, 0x2c, 0x00, 0x12, 0x01, 0x30, 0x80, 0x52, 0x1f, 0x00, 0x01, 0x6b, 0x61, 0x02, 0x00, 0x54,
    0x00, 0x00, 0x80, 0xd2, 0x1a, 0x01, 0x00, 0x14, 0x60, 0x01, 0x80, 0xd2, 0x18, 0x01, 0x00, 0x14,
    0x80, 0x01, 0x80, 0xd2, 0x16, 0x01, 0x00, 0x14, 0xa0, 0x01, 0x80, 0xd2, 0x14, 0x01, 0x00, 0x14,
    0xc0, 0x01, 0x80, 0xd2, 0x12, 0x01, 0x00, 0x14, 0xe0, 0x01, 0x80, 0xd2, 0x10, 0x01, 0x00, 0x14,
    0x00, 0x02, 0x80, 0xd2, 0x0e, 0x01, 0x00, 0x14, 0x20, 0x02, 0x80, 0xd2, 0x0c, 0x01, 0x00, 0x14,
    0x40, 0x02, 0x80, 0xd2, 0x0a, 0x01, 0x00, 0x14, 0x60, 0x02, 0x80, 0xd2, 0x08, 0x01, 0x00, 0x14,
    0x00, 0x00, 0x80, 0xd2, 0x40, 0x00, 0xc0, 0xf2, 0x80, 0x4a, 0x01, 0xf9, 0x9f, 0x02, 0x01, 0xf9,
    0x40, 0x04, 0x80, 0x52, 0x80, 0x0a, 0x02, 0xb9, 0x40, 0x00, 0x80, 0x52, 0x80, 0x0e, 0x02, 0xb9,
    0x9f, 0x56, 0x01, 0xf9, 0x00, 0x80, 0x90, 0xd2, 0xe0, 0xfa, 0xa2, 0xf2, 0x80, 0x5a, 0x01, 0xf9,
    0x00, 0x00, 0x80, 0xd2, 0x81, 0x42, 0x0a, 0x91, 0x02, 0x00, 0x80, 0xd2, 0x03, 0x01, 0x80, 0xd2,
    0xe8, 0x10, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0xe0, 0x1a, 0xf8, 0xb7, 0x14, 0x00, 0x80, 0xd2,
    0x14, 0x40, 0xa2, 0xf2, 0x20, 0x00, 0x80, 0xd2, 0x81, 0x02, 0x08, 0x91, 0x82, 0x62, 0x0a, 0x91,
    0x68, 0x0d, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0x20, 0x1a, 0xf8, 0xb7, 0x14, 0x00, 0x80, 0xd2,
    0x14, 0x40, 0xa2, 0xf2, 0x60, 0x00, 0x80, 0xd2, 0x80, 0x62, 0x01, 0xf9, 0x14, 0x00, 0x80, 0xd2,
    0x14, 0x40, 0xa2, 0xf2, 0x9f, 0x22, 0x01, 0xf9, 0x9f, 0x26, 0x01, 0xf9, 0x9f, 0x2a, 0x01, 0xf9,
    0x00, 0x20, 0x9c, 0xd2, 0xa0, 0xbe, 0xa0, 0xf2, 0x80, 0x2e, 0x01, 0xf9, 0x80, 0x9a, 0x42, 0xb9,
    0x01, 0x00, 0x80, 0xd2, 0x82, 0x02, 0x09, 0x91, 0x03, 0x00, 0x80, 0xd2, 0xc8, 0x0d, 0x80, 0xd2,
    0x01, 0x00, 0x00, 0xd4, 0x00, 0x18, 0xf8, 0xb7, 0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2,
    0x80, 0x42, 0x0a, 0x91, 0x01, 0x00, 0x80, 0xd2, 0x82, 0xa2, 0x0a, 0x91, 0x03, 0x01, 0x80, 0xd2,
    0x28, 0x11, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0x20, 0x17, 0xf8, 0xb7, 0x14, 0x00, 0x80, 0xd2,
    0x14, 0x40, 0xa2, 0xf2, 0x95, 0x62, 0x41, 0xf9, 0xb5, 0x06, 0x00, 0xd1, 0x95, 0x62, 0x01, 0xf9,
    0x75, 0xfc, 0xff, 0xb5, 0x20, 0x02, 0x80, 0xd2, 0x01, 0x00, 0x80, 0xd2, 0x02, 0x00, 0x80, 0xd2,
    0x03, 0x00, 0x80, 0xd2, 0x04, 0x00, 0x80, 0xd2, 0x88, 0x1b, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4,
    0xa0, 0x15, 0xf8, 0xb7, 0x40, 0x08, 0x00, 0xb4, 0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2,
    0x20, 0x00, 0x80, 0xd2, 0x81, 0xc2, 0x09, 0x91, 0x28, 0x0e, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4,
    0xe0, 0x14, 0xf8, 0xb7, 0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x9f, 0x22, 0x01, 0xf9,
    0x9f, 0x26, 0x01, 0xf9, 0x9f, 0x2a, 0x01, 0xf9, 0x00, 0x48, 0x88, 0xd2, 0xe0, 0x01, 0xa0, 0xf2,
    0x80, 0x2e, 0x01, 0xf9, 0x80, 0x9a, 0x42, 0xb9, 0x01, 0x00, 0x80, 0xd2, 0x82, 0x02, 0x09, 0x91,
    0x03, 0x00, 0x80, 0xd2, 0xc8, 0x0d, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0x40, 0x12, 0xf8, 0xb7,
    0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x20, 0x00, 0x80, 0xd2, 0x81, 0x02, 0x0a, 0x91,
    0x28, 0x0e, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0x20, 0x12, 0xf8, 0xb7, 0x14, 0x00, 0x80, 0xd2,
    0x14, 0x40, 0xa2, 0xf2, 0x81, 0x3a, 0x41, 0xf9, 0x82, 0x42, 0x41, 0xf9, 0x83, 0x3e, 0x41, 0xf9,
    0x84, 0x46, 0x41, 0xf9, 0x42, 0x00, 0x01, 0xcb, 0x84, 0x00, 0x03, 0xeb, 0xaa, 0x00, 0x00, 0x54,
    0x42, 0x04, 0x00, 0xd1, 0x05, 0x40, 0x99, 0xd2, 0x45, 0x73, 0xa7, 0xf2, 0x84, 0x00, 0x05, 0x8b,
    0xa2, 0x00, 0x00, 0xb5, 0x05, 0x60, 0x94, 0xd2, 0x25, 0x3c, 0xa2, 0xf2, 0x9f, 0x00, 0x05, 0xeb,
    0x23, 0xfb, 0xff, 0x54, 0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x00, 0x00, 0x80, 0x92,
    0x81, 0x82, 0x0a, 0x91, 0x02, 0x00, 0x80, 0xd2, 0x88, 0x20, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4,
    0x20, 0x0f, 0xf8, 0xb7, 0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x80, 0xa2, 0x42, 0xb9,
    0xa0, 0x00, 0x00, 0x34, 0x01, 0x1c, 0x00, 0x12, 0xa1, 0x0e, 0x00, 0x35, 0x00, 0x7c, 0x08, 0x53,
    0x83, 0x00, 0x00, 0x14, 0x00, 0x00, 0x80, 0xd2, 0x81, 0x00, 0x00, 0x14, 0x14, 0x00, 0x80, 0xd2,
    0x14, 0x40, 0xa2, 0xf2, 0x9f, 0x32, 0x01, 0xf9, 0x00, 0x10, 0x9e, 0xd2, 0x40, 0x5f, 0xa0, 0xf2,
    0x80, 0x36, 0x01, 0xf9, 0x20, 0x00, 0x80, 0xd2, 0x81, 0xc2, 0x09, 0x91, 0x28, 0x0e, 0x80, 0xd2,
    0x01, 0x00, 0x00, 0xd4, 0x00, 0x0d, 0xf8, 0xb7, 0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2,
    0x80, 0x82, 0x09, 0x91, 0x01, 0x00, 0x80, 0xd2, 0xa8, 0x0c, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4,
    0x60, 0x0c, 0xf8, 0xb7, 0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x20, 0x00, 0x80, 0xd2,
    0x81, 0x02, 0x0a, 0x91, 0x28, 0x0e, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4, 0x40, 0x0b, 0xf8, 0xb7,
    0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x81, 0x3a, 0x41, 0xf9, 0x82, 0x42, 0x41, 0xf9,
    0x83, 0x3e, 0x41, 0xf9, 0x84, 0x46, 0x41, 0xf9, 0x42, 0x00, 0x01, 0xcb, 0x84, 0x00, 0x03, 0xeb,
    0xaa, 0x00, 0x00, 0x54, 0x42, 0x04, 0x00, 0xd1, 0x05, 0x40, 0x99, 0xd2, 0x45, 0x73, 0xa7, 0xf2,
    0x84, 0x00, 0x05, 0x8b, 0x02, 0x0a, 0x00, 0xb5, 0x05, 0x10, 0x9e, 0xd2, 0x45, 0x5f, 0xa0, 0xf2,
    0x9f, 0x00, 0x05, 0xeb, 0x43, 0x0a, 0x00, 0x54, 0x05, 0x40, 0x98, 0xd2, 0x65, 0x7d, 0xa1, 0xf2,
    0x9f, 0x00, 0x05, 0xeb, 0x02, 0x09, 0x00, 0x54, 0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2,
    0x9f, 0x32, 0x01, 0xf9, 0x00, 0x10, 0x9e, 0xd2, 0x40, 0x5f, 0xa0, 0xf2, 0x80, 0x36, 0x01, 0xf9,
    0x20, 0x00, 0x80, 0xd2, 0x81, 0xc2, 0x09, 0x91, 0x28, 0x0e, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4,
    0x20, 0x07, 0xf8, 0xb7, 0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x20, 0x00, 0x80, 0xd2,
    0x01, 0x00, 0x80, 0xd2, 0x82, 0x82, 0x09, 0x91, 0x03, 0x00, 0x80, 0xd2, 0x68, 0x0e, 0x80, 0xd2,
    0x01, 0x00, 0x00, 0xd4, 0xc0, 0x06, 0xf8, 0xb7, 0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2,
    0x20, 0x00, 0x80, 0xd2, 0x81, 0x02, 0x0a, 0x91, 0x28, 0x0e, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4,
    0x20, 0x05, 0xf8, 0xb7, 0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x81, 0x3a, 0x41, 0xf9,
    0x82, 0x42, 0x41, 0xf9, 0x83, 0x3e, 0x41, 0xf9, 0x84, 0x46, 0x41, 0xf9, 0x42, 0x00, 0x01, 0xcb,
    0x84, 0x00, 0x03, 0xeb, 0xaa, 0x00, 0x00, 0x54, 0x42, 0x04, 0x00, 0xd1, 0x05, 0x40, 0x99, 0xd2,
    0x45, 0x73, 0xa7, 0xf2, 0x84, 0x00, 0x05, 0x8b, 0x62, 0x04, 0x00, 0xb5, 0x05, 0x10, 0x9e, 0xd2,
    0x45, 0x5f, 0xa0, 0xf2, 0x9f, 0x00, 0x05, 0xeb, 0x63, 0x04, 0x00, 0x54, 0x05, 0x40, 0x98, 0xd2,
    0x65, 0x7d, 0xa1, 0xf2, 0x9f, 0x00, 0x05, 0xeb, 0x62, 0x03, 0x00, 0x54, 0x00, 0x00, 0x80, 0xd2,
    0x1f, 0x00, 0x00, 0x14, 0xa0, 0x02, 0x80, 0xd2, 0x1d, 0x00, 0x00, 0x14, 0xc0, 0x02, 0x80, 0xd2,
    0x1b, 0x00, 0x00, 0x14, 0xe0, 0x02, 0x80, 0xd2, 0x19, 0x00, 0x00, 0x14, 0x00, 0x03, 0x80, 0xd2,
    0x17, 0x00, 0x00, 0x14, 0x20, 0x03, 0x80, 0xd2, 0x15, 0x00, 0x00, 0x14, 0x40, 0x03, 0x80, 0xd2,
    0x13, 0x00, 0x00, 0x14, 0x60, 0x03, 0x80, 0xd2, 0x11, 0x00, 0x00, 0x14, 0xa0, 0x03, 0x80, 0xd2,
    0x0f, 0x00, 0x00, 0x14, 0xe0, 0x03, 0x80, 0xd2, 0x0d, 0x00, 0x00, 0x14, 0x00, 0x04, 0x80, 0xd2,
    0x0b, 0x00, 0x00, 0x14, 0x20, 0x04, 0x80, 0xd2, 0x09, 0x00, 0x00, 0x14, 0x40, 0x04, 0x80, 0xd2,
    0x07, 0x00, 0x00, 0x14, 0x60, 0x04, 0x80, 0xd2, 0x05, 0x00, 0x00, 0x14, 0x80, 0x04, 0x80, 0xd2,
    0x03, 0x00, 0x00, 0x14, 0xa0, 0x04, 0x80, 0xd2, 0x01, 0x00, 0x00, 0x14, 0xa8, 0x0b, 0x80, 0xd2,
    0x01, 0x00, 0x00, 0xd4, 0xfe, 0xff, 0xff, 0x17
];

static ACTIVE: AtomicBool = AtomicBool::new(false);
static DONE: AtomicBool = AtomicBool::new(false);
static FAULTED: AtomicBool = AtomicBool::new(false);
static EXITED: AtomicBool = AtomicBool::new(false);
static EXIT_CODE: AtomicI32 = AtomicI32::new(0);
static LAST_SYSCALL: AtomicU64 = AtomicU64::new(0);
static LAST_RESULT: AtomicU64 = AtomicU64::new(u64::MAX);
static SETUP_ERROR: AtomicU64 = AtomicU64::new(0);
static MAGIC_WORD: AtomicU64 = AtomicU64::new(MAGIC);
static EXEC_HITS: AtomicU64 = AtomicU64::new(0);
static FAULT_COUNT: AtomicU64 = AtomicU64::new(0);
static FAULT_SEEN: AtomicBool = AtomicBool::new(false);
static DEFER_DONE: AtomicBool = AtomicBool::new(false);
static PEER_DONE: AtomicBool = AtomicBool::new(false);
static PEER_STOP: AtomicBool = AtomicBool::new(false);
static PEER_VALUE: AtomicU64 = AtomicU64::new(0);

static CFG_OP: AtomicU64 = AtomicU64::new(0);
static CFG_TARGET: AtomicU64 = AtomicU64::new(0);
static CFG_VALUE: AtomicU64 = AtomicU64::new(0);
static CFG_SYSNO: AtomicU64 = AtomicU64::new(0);
static CFG_A0: AtomicU64 = AtomicU64::new(0);
static CFG_A1: AtomicU64 = AtomicU64::new(0);
static CFG_A2: AtomicU64 = AtomicU64::new(0);
static PROBE_THREAD: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Monotonic elapsed time of high-resolution sleeps while `notify-gate-sleep` runs.
/// Slot 0 is the child's nanosleep. Slot 1 is its clock_nanosleep.
const NOTIFY_GATE_SLEEP_SLOTS: usize = 4;
static NOTIFY_GATE_SLEEP_NS: [AtomicU64; NOTIFY_GATE_SLEEP_SLOTS] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static NOTIFY_GATE_SLEEP_COUNT: AtomicUsize = AtomicUsize::new(0);
static CLOCK_SNAPSHOT_VALID: AtomicBool = AtomicBool::new(false);
static CLOCK_SNAPSHOT_DENIED: AtomicBool = AtomicBool::new(false);
static CLOCK_SNAPSHOT_APPLIED: AtomicBool = AtomicBool::new(false);
static WATCH_PAGE: AtomicUsize = AtomicUsize::new(0);
static WATCH_WORD: AtomicU64 = AtomicU64::new(0);
static WATCH_VALID: AtomicBool = AtomicBool::new(false);
static SETUP_OTHER: AtomicBool = AtomicBool::new(false);
static PREPARE: AtomicU8 = AtomicU8::new(0);
const PREPARE_NONE: u8 = 0;
const PREPARE_LIVE_VMO: u8 = 1;
const PREPARE_STALE: u8 = 2;
const PREPARE_RIGHTS: u8 = 3;
const PREPARE_CLOSED_FD: u8 = 4;
static FOREIGN_PHYS: AtomicUsize = AtomicUsize::new(0);
static UACCESS_PEER_DONE: AtomicBool = AtomicBool::new(false);
static UACCESS_CANARY: AtomicU64 = AtomicU64::new(0x00C0_FFEE);
#[cfg(target_arch = "aarch64")]
static mut FOREIGN_SPACE: Option<crate::kernel_lowlevel::Aarch64AddressSpace> = None;
#[cfg(target_arch = "aarch64")]
static mut ORPHAN_SPACE: Option<crate::kernel_lowlevel::Aarch64AddressSpace> = None;

#[derive(Clone, Copy)]
enum Verdict {
    Pass,
    Fail,
    XFail,
    XPass,
}

struct Tally {
    pass: usize,
    fail: usize,
    xfail: usize,
    xpass: usize,
}

/// Probes in `cases()`. `sectest all` also runs heap-fork and elf-memsz.
pub const LISTED_CASE_COUNT: usize = 89;
pub const SECTEST_ALL_CASE_COUNT: usize = LISTED_CASE_COUNT + 2;

static COVERED_LO: AtomicU64 = AtomicU64::new(0);
static COVERED_HI: AtomicU64 = AtomicU64::new(0);

pub fn reset_case_coverage() {
    COVERED_LO.store(0, Ordering::Relaxed);
    COVERED_HI.store(0, Ordering::Relaxed);
}

pub fn note_case_coverage(index: usize) {
    if index >= 128 {
        return;
    }
    let bit = 1u64 << (index % 64);
    if index < 64 {
        COVERED_LO.fetch_or(bit, Ordering::Relaxed);
    } else {
        COVERED_HI.fetch_or(bit, Ordering::Relaxed);
    }
}

pub fn case_coverage_ran() -> usize {
    COVERED_LO.load(Ordering::Relaxed).count_ones() as usize
        + COVERED_HI.load(Ordering::Relaxed).count_ones() as usize
}

/// Counts from the latest `run` summary. Hermes reads this after `sectest all`
/// without parsing serial text. Case bodies do not touch it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SectestSummary {
    pub pass: usize,
    pub fail: usize,
    pub xfail: usize,
    pub xpass: usize,
    pub skipped: bool,
}

static SUMMARY_PASS: AtomicUsize = AtomicUsize::new(0);
static SUMMARY_FAIL: AtomicUsize = AtomicUsize::new(0);
static SUMMARY_XFAIL: AtomicUsize = AtomicUsize::new(0);
static SUMMARY_XPASS: AtomicUsize = AtomicUsize::new(0);
static SUMMARY_SKIPPED: AtomicBool = AtomicBool::new(false);

fn publish_summary(tally: &Tally, skipped: bool) {
    SUMMARY_PASS.store(tally.pass, Ordering::Relaxed);
    SUMMARY_FAIL.store(tally.fail, Ordering::Relaxed);
    SUMMARY_XFAIL.store(tally.xfail, Ordering::Relaxed);
    SUMMARY_XPASS.store(tally.xpass, Ordering::Relaxed);
    SUMMARY_SKIPPED.store(skipped, Ordering::Relaxed);
}

pub fn last_summary() -> SectestSummary {
    SectestSummary {
        pass: SUMMARY_PASS.load(Ordering::Relaxed),
        fail: SUMMARY_FAIL.load(Ordering::Relaxed),
        xfail: SUMMARY_XFAIL.load(Ordering::Relaxed),
        xpass: SUMMARY_XPASS.load(Ordering::Relaxed),
        skipped: SUMMARY_SKIPPED.load(Ordering::Relaxed),
    }
}

impl Tally {
    const fn new() -> Self {
        Self {
            pass: 0,
            fail: 0,
            xfail: 0,
            xpass: 0,
        }
    }

    fn add(&mut self, verdict: Verdict) {
        match verdict {
            Verdict::Pass => self.pass += 1,
            Verdict::Fail => self.fail += 1,
            Verdict::XFail => self.xfail += 1,
            Verdict::XPass => self.xpass += 1,
        }
    }
}

struct FinishedCase {
    verdict: Verdict,
    detail: &'static str,
    /// Set for cases owned by SEC-2 or later. `None` is a hard SEC-0/SEC-1 result.
    stage: Option<&'static str>,
}

fn finished(verdict: Verdict, detail: &'static str) -> FinishedCase {
    FinishedCase {
        verdict,
        detail,
        stage: None,
    }
}

fn expected(stage: &'static str, detail: &'static str, passed: bool) -> FinishedCase {
    FinishedCase {
        verdict: if passed {
            Verdict::XPass
        } else {
            Verdict::XFail
        },
        detail,
        stage: Some(stage),
    }
}

pub fn magic_address() -> usize {
    &MAGIC_WORD as *const AtomicU64 as usize
}

#[no_mangle]
pub extern "C" fn security_case_exec_marker() {
    EXEC_HITS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn case_active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

pub(crate) fn observe_syscall(number: u32, result: u64) {
    if !ACTIVE.load(Ordering::Acquire) || number == 93 || number == 94 {
        return;
    }
    if !probe_thread_is_current() {
        return;
    }
    LAST_SYSCALL.store(number as u64, Ordering::Relaxed);
    LAST_RESULT.store(result, Ordering::Relaxed);
}

pub(crate) fn observe_fault(address: u64) {
    if !ACTIVE.load(Ordering::Acquire) || !probe_thread_is_current() {
        return;
    }
    snapshot_watch();
    FAULT_COUNT.fetch_add(1, Ordering::Relaxed);
    FAULTED.store(true, Ordering::Relaxed);
    FAULT_SEEN.store(true, Ordering::Release);
    let _ = address;
    if !DEFER_DONE.load(Ordering::Relaxed) {
        DONE.store(true, Ordering::Release);
    }
}

/// Record process death when the fault itself must not complete the case.
/// Used so a rejected signal handler is not confused with a handler that ran
/// and faulted again.
pub(crate) fn observe_terminated() {
    if !ACTIVE.load(Ordering::Acquire) || !probe_thread_is_current() {
        return;
    }
    DONE.store(true, Ordering::Release);
}

pub(crate) fn observe_fault_if_new(address: u64) {
    if FAULTED.load(Ordering::Relaxed) {
        if !DEFER_DONE.load(Ordering::Relaxed) {
            DONE.store(true, Ordering::Release);
        }
        return;
    }
    observe_fault(address);
}

pub(crate) fn observe_exit(code: i32) {
    if !ACTIVE.load(Ordering::Acquire) || !probe_thread_is_current() {
        return;
    }
    snapshot_watch();
    // Process exit resets tasks, futexes, and mqueues. Capture the clock
    // probe before that teardown, while foreign waits are still in place.
    let (denied, applied) = crate::syscall::security_snapshot_clock_probe();
    CLOCK_SNAPSHOT_DENIED.store(denied, Ordering::Relaxed);
    CLOCK_SNAPSHOT_APPLIED.store(applied, Ordering::Relaxed);
    CLOCK_SNAPSHOT_VALID.store(true, Ordering::Release);
    EXITED.store(true, Ordering::Relaxed);
    EXIT_CODE.store(code, Ordering::Relaxed);
    DONE.store(true, Ordering::Release);
}

fn probe_thread_is_current() -> bool {
    let probe = PROBE_THREAD.load(Ordering::Relaxed);
    probe != usize::MAX && scheduler::scheduler().current().0 == probe
}

fn snapshot_watch() {
    let page = WATCH_PAGE.load(Ordering::Relaxed);
    if page == 0 {
        return;
    }
    let Ok(pid) = linux_process::current_pid() else {
        return;
    };
    let mut buf = [0u8; 8];
    if linux_process_memory::copy_from_process(pid, page, &mut buf).is_ok() {
        WATCH_WORD.store(u64::from_le_bytes(buf), Ordering::Relaxed);
        WATCH_VALID.store(true, Ordering::Relaxed);
    }
}

pub fn run(serial: &mut Serial, which: &str) {
    publish_summary(&Tally::new(), false);
    if !cfg!(target_arch = "aarch64") {
        serial.write_str("CASE sectest RESULT SKIP not-aarch64\n");
        print_summary(serial, Tally::new(), true);
        return;
    }
    match which {
        "all" => run_listed(serial, false),
        "undef" => run_undef(serial),
        "heap-fork" => run_heap_fork(serial),
        "elf-memsz" => run_elf_memsz(serial),
        "list" => {
            serial.write_str("sectest cases: all | undef | heap-fork | elf-memsz | <id>\n");
            for case in cases() {
                serial.write_str(case.id);
                serial.write_str("\n");
            }
        }
        other => {
            if let Some(case) = cases().into_iter().find(|case| case.id == other) {
                run_one(serial, case);
                serial.write_str("SEC-HEARTBEAT\n");
            } else {
                serial.write_str("sectest: unknown case\n");
                serial.write_str("CASE ");
                serial.write_str(other);
                serial.write_str(" RESULT FAIL unknown-id\n");
                let mut tally = Tally::new();
                tally.add(Verdict::Fail);
                print_summary(serial, tally, false);
            }
        }
    }
}

fn run_listed(serial: &mut Serial, _include_undef: bool) {
    let mut tally = Tally::new();
    for (index, case) in cases().into_iter().enumerate() {
        note_case_coverage(index);
        let finished = run_and_print(serial, case);
        tally.add(finished.verdict);
    }
    note_case_coverage(LISTED_CASE_COUNT);
    record_kernel_case(serial, &mut tally, "heap-fork", run_heap_fork_case());
    note_case_coverage(LISTED_CASE_COUNT + 1);
    record_kernel_case(serial, &mut tally, "elf-memsz", run_elf_memsz_case());
    serial.write_str("SEC-HEARTBEAT\n");
    print_summary(serial, tally, false);
}

fn record_kernel_case(serial: &mut Serial, tally: &mut Tally, id: &str, pass: bool) {
    let finished = FinishedCase {
        verdict: if pass { Verdict::Pass } else { Verdict::Fail },
        stage: None,
        detail: if pass { "ok" } else { "budget" },
    };
    print_case(serial, id, &finished);
    tally.add(finished.verdict);
}

fn run_heap_fork(serial: &mut Serial) {
    let pass = run_heap_fork_case();
    let mut tally = Tally::new();
    record_kernel_case(serial, &mut tally, "heap-fork", pass);
    serial.write_str("SEC-HEARTBEAT\n");
    print_summary(serial, tally, false);
}

fn run_elf_memsz(serial: &mut Serial) {
    let pass = run_elf_memsz_case();
    let mut tally = Tally::new();
    record_kernel_case(serial, &mut tally, "elf-memsz", pass);
    serial.write_str("SEC-HEARTBEAT\n");
    print_summary(serial, tally, false);
}

extern "C" fn heap_budget_thread() -> ! {
    loop {
        crate::kernel_lowlevel::cpu::wait_for_interrupt();
    }
}

fn run_heap_fork_case() -> bool {
    let before = crate::kernel_lowlevel::kernel_budget::heap_allocated();
    let mut ids = Vec::new();
    if ids.try_reserve_exact(2048).is_err() {
        return false;
    }
    loop {
        if ids.len() >= 2048 {
            break;
        }
        match scheduler::scheduler().create_suspended_thread(heap_budget_thread, "sectest-heap") {
            Some(id) => ids.push(id),
            None => break,
        }
    }
    let created = ids.len();
    let mut probe: Vec<u8> = Vec::new();
    let reserved = probe.try_reserve_exact(40960).is_ok();
    drop(probe);
    for id in ids.drain(..) {
        let _ = scheduler::scheduler().terminate_thread(id);
    }
    drop(ids);
    let after = crate::kernel_lowlevel::kernel_budget::heap_allocated();
    created > 32 && created < 2000 && reserved && after <= before.saturating_add(64 * 1024)
}

fn run_elf_memsz_case() -> bool {
    use crate::user_level::services::elf_load_logic_shared::{
        plan_elf_load, ElfLoadSegment, ELF_LOAD_ENOMEM,
    };
    let window_start = 0x1000_0000u64;
    let window_end = 0x2000_0000u64;
    let page = 0x1000u64;
    let huge = ElfLoadSegment {
        vaddr: 0,
        mem_size: 1 << 44,
        file_size: 0,
        prot: 1,
    };
    if plan_elf_load(&[huge], window_start, window_start, window_end, page) != Err(ELF_LOAD_ENOMEM)
    {
        return false;
    }
    let bss = ElfLoadSegment {
        vaddr: 0x2000,
        mem_size: page * 8,
        file_size: 0,
        prot: 1,
    };
    let Ok(runs) = plan_elf_load(&[bss], window_start, window_start, window_end, page) else {
        return false;
    };
    let bss_in_window = smros_sectest_address_in_window_body!(
        window_start + bss.vaddr,
        bss.mem_size,
        window_start,
        window_end
    );
    bss_in_window && runs.len() == 1 && runs[0].len == (page as usize) * 8
}

fn run_one(serial: &mut Serial, case: Case) {
    let finished = run_and_print(serial, case);
    let mut tally = Tally::new();
    tally.add(finished.verdict);
    serial.write_str("SEC-HEARTBEAT\n");
    print_summary(serial, tally, false);
}

/// One-shot cases print a single verdict. `notify-gate-sleep` prints one
/// summary line from this function: attempt count, late count, and every
/// measured sample. A second late or any sample over 300ms is not retried.
fn run_and_print(serial: &mut Serial, case: Case) -> FinishedCase {
    if !matches!(case.judge, Judge::NotifyGateSleep) {
        let finished = execute(case);
        print_case(serial, case.id, &finished);
        return finished;
    }
    reset_notify_gate_sleep_samples();
    let mut attempts = 0usize;
    let mut late = 0usize;
    let finished = loop {
        let before = NOTIFY_GATE_SLEEP_COUNT.load(Ordering::Relaxed);
        let finished = run_probe(case);
        attempts = attempts.saturating_add(1);
        let max_ns = samples_max_since(before);
        let is_late = notify_gate_late(&finished);
        if is_late {
            late = late.saturating_add(1);
        }
        let allow = smros_notify_gate_allow_retry_body!(attempts, late, max_ns, is_late);
        if !allow {
            break finished;
        }
    };
    print_notify_gate_summary(serial, attempts, late);
    print_case(serial, case.id, &finished);
    finished
}

fn samples_max_since(before: usize) -> u64 {
    let count = NOTIFY_GATE_SLEEP_COUNT
        .load(Ordering::Relaxed)
        .min(NOTIFY_GATE_SLEEP_NS.len());
    let mut max_ns = 0u64;
    let start = before.min(count);
    for index in start..count {
        let sample = NOTIFY_GATE_SLEEP_NS[index].load(Ordering::Relaxed);
        if sample > max_ns {
            max_ns = sample;
        }
    }
    max_ns
}

fn notify_gate_late(finished: &FinishedCase) -> bool {
    matches!(finished.verdict, Verdict::Fail)
        && (finished.detail == "nanosleep-late" || finished.detail == "clock-nanosleep-late")
}

fn print_case(serial: &mut Serial, id: &str, finished: &FinishedCase) {
    serial.write_str("CASE ");
    serial.write_str(id);
    serial.write_str(" RESULT ");
    match finished.verdict {
        Verdict::Pass => serial.write_str("PASS"),
        Verdict::Fail => serial.write_str("FAIL"),
        Verdict::XFail => {
            serial.write_str("XFAIL (");
            serial.write_str(finished.stage.unwrap_or("SEC-?"));
            serial.write_str(")");
        }
        Verdict::XPass => {
            serial.write_str("XPASS (");
            serial.write_str(finished.stage.unwrap_or("SEC-?"));
            serial.write_str(")");
        }
    }
    serial.write_str(" ");
    serial.write_str(finished.detail);
    serial.write_str("\n");
}

fn print_summary(serial: &mut Serial, tally: Tally, skipped: bool) {
    publish_summary(&tally, skipped);
    serial.write_str("SECTEST SUMMARY pass=");
    write_usize(serial, tally.pass);
    serial.write_str(" fail=");
    write_usize(serial, tally.fail);
    serial.write_str(" xfail=");
    write_usize(serial, tally.xfail);
    serial.write_str(" xpass=");
    write_usize(serial, tally.xpass);
    serial.write_str("\n");
}

/// Record one high-resolution sleep that finished during an active notify-gate probe.
///
/// The sample is monotonic time from arming the precision deadline until the
/// sleeper runs again. That is the duration `notify-gate-sleep` compares with
/// its 200ms bound, aside from a few microseconds of syscall entry and exit.
pub(crate) fn note_notify_gate_sleep(elapsed_ns: u64) {
    if !ACTIVE.load(Ordering::Relaxed) || CFG_OP.load(Ordering::Relaxed) != OP_NOTIFY_GATE {
        return;
    }
    let index = NOTIFY_GATE_SLEEP_COUNT.fetch_add(1, Ordering::Relaxed);
    if index < NOTIFY_GATE_SLEEP_NS.len() {
        NOTIFY_GATE_SLEEP_NS[index].store(elapsed_ns, Ordering::Relaxed);
    }
}

fn reset_notify_gate_sleep_samples() {
    NOTIFY_GATE_SLEEP_COUNT.store(0, Ordering::Relaxed);
    for slot in &NOTIFY_GATE_SLEEP_NS {
        slot.store(0, Ordering::Relaxed);
    }
}

fn print_notify_gate_summary(serial: &mut Serial, attempts: usize, late: usize) {
    let count = NOTIFY_GATE_SLEEP_COUNT
        .load(Ordering::Relaxed)
        .min(NOTIFY_GATE_SLEEP_NS.len());
    serial.write_str("NOTIFY-GATE SUMMARY attempts=");
    write_usize(serial, attempts);
    serial.write_str(" late=");
    write_usize(serial, late);
    serial.write_str(" values=");
    if count == 0 {
        serial.write_str("none");
    } else {
        for index in 0..count {
            if index != 0 {
                serial.write_str(",");
            }
            write_u64(serial, NOTIFY_GATE_SLEEP_NS[index].load(Ordering::Relaxed));
        }
    }
    serial.write_str("\n");
}

fn write_u64(serial: &mut Serial, mut value: u64) {
    if value == 0 {
        serial.write_byte(b'0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut index = 0;
    while value > 0 && index < buf.len() {
        buf[index] = b'0' + (value % 10) as u8;
        value /= 10;
        index += 1;
    }
    while index > 0 {
        index -= 1;
        serial.write_byte(buf[index]);
    }
}

fn write_usize(serial: &mut Serial, mut value: usize) {
    if value == 0 {
        serial.write_byte(b'0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut index = 0;
    while value > 0 && index < buf.len() {
        buf[index] = b'0' + (value % 10) as u8;
        value /= 10;
        index += 1;
    }
    while index > 0 {
        index -= 1;
        serial.write_byte(buf[index]);
    }
}

#[derive(Clone, Copy)]
struct Case {
    id: &'static str,
    op: u64,
    target: u64,
    value: u64,
    sysno: u64,
    a0: u64,
    a1: u64,
    a2: u64,
    judge: Judge,
}

#[derive(Clone, Copy)]
enum Judge {
    /// EL0 access must trap. Kernel magic and exec marker stay unchanged.
    Trap,
    /// The syscall returns to EL0 with this x0 value and does not change magic.
    Syscall(u64),
    /// Owned by a later stage. A miss is XFAIL; an unexpected success is XPASS.
    #[allow(dead_code)]
    Later {
        stage: &'static str,
        detail: &'static str,
    },
    /// Read of another process's VA must not return that process's magic.
    OtherProcess,
    /// Child store after fork must leave the parent's page unchanged.
    CowFork,
    /// Store after mprotect(PROT_READ) must fault instead of using a stale TLB.
    MprotectStore,
    /// A handle that belongs to another table, including the kernel root VMAR.
    ForeignHandle,
    /// A handle value from before its slot was closed and reused.
    StaleHandle,
    /// A same-type handle whose rights do not include the operation.
    BadRights,
    /// An FD number that this process has already closed.
    ClosedFd,
    /// EL0 getpid with no Linux process returns ESRCH.
    GetpidAbsent,
    /// EL0 copies follow the saved SPSR. A user pointer is translated. A kernel
    /// address or an illegal range returns EFAULT, including after another
    /// thread has entered and left its own syscall.
    El0Uaccess,
    /// Undefined instruction or unaligned access kills only this process.
    /// A second thread must finish after the fault is observed.
    PeerTrap,
    /// exit_group while another thread is blocked in futex_wait.
    ExitFutex,
    /// The parent receives the child status once. The next wait is ECHILD.
    WaitOnce,
    /// A second fork, immediately after the first child exits, does not see
    /// that child's store.
    ExitRefork,
    /// A SIGSEGV handler that is not an executable user page terminates.
    BadHandler,
    /// EL0 with no Linux task must not halt the kernel.
    NoLinuxTask,
    /// The file owner chmod is visible to a later stat. Another uid gets EPERM.
    ChmodRoundtrip,
    /// An unprivileged `clock_settime` or `settimeofday` returns `EPERM` and
    /// leaves the realtime offset and any foreign absolute waits alone.
    ClockDenied,
    /// A privileged realtime set succeeds and is visible to `clock_gettime`.
    ClockPrivileged,
    /// Process A re-arms a near-deadline SIGEV_THREAD timer. Process B's
    /// sleeps must last at least the requested 49999872 ns and finish inside
    /// 200ms. One late sample at or under 300ms may be retried once. A second
    /// late, a sample over 300ms, or an early wake fails without a retry.
    NotifyGateSleep,
}

macro_rules! syscall_case {
    ($id:literal, $sysno:expr, $a0:expr, $a1:expr, $a2:expr, $expect:expr) => {
        Case {
            id: $id,
            op: OP_SYSCALL,
            target: 0,
            value: 0,
            sysno: $sysno,
            a0: $a0,
            a1: $a1,
            a2: $a2,
            judge: Judge::Syscall($expect),
        }
    };
}

fn cases() -> [Case; LISTED_CASE_COUNT] {
    let enosys = errno_u64(38);
    let eperm = errno_u64(1);
    let einval = errno_u64(22);
    let efault = errno_u64(14);
    let ebadf = errno_u64(9);
    let enodev = errno_u64(19);
    let eopnotsupp = errno_u64(95);
    let timeval = DATA_PAGE as u64;
    let timex = (DATA_PAGE + 64) as u64;
    let bad_fd = 0x1_0000u64;
    let magic = magic_address() as u64;
    let marker = security_case_exec_marker as *const () as u64;
    // QEMU virt identity addresses. The probe runs only on AArch64.
    let uart = 0x0900_0000u64;
    let gic = 0x0800_0000u64;
    let virtio = 0x0a00_0000u64;
    let ram = 0x4000_0000u64;
    [
        Case {
            id: "el0-read-kernel-ram",
            op: OP_READ,
            target: ram,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::Trap,
        },
        Case {
            id: "el0-write-kernel-ram",
            op: OP_WRITE,
            target: magic,
            value: 0xDEAD,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::Trap,
        },
        Case {
            id: "el0-exec-kernel",
            op: OP_EXEC,
            target: marker,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::Trap,
        },
        Case {
            id: "el0-write-readonly",
            op: OP_WRITE,
            target: RO_PAGE as u64,
            value: 0x11,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::Trap,
        },
        Case {
            id: "el0-read-uart",
            op: OP_READ,
            target: uart,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::Trap,
        },
        Case {
            id: "el0-read-gic",
            op: OP_READ,
            target: gic,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::Trap,
        },
        Case {
            id: "el0-read-virtio",
            op: OP_READ,
            target: virtio,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::Trap,
        },
        Case {
            id: "bad-syscall-pointer",
            op: OP_SYSCALL,
            target: 0,
            value: 0,
            sysno: 63,
            a0: 0,
            a1: magic,
            a2: 8,
            judge: Judge::Syscall(errno_u64(14)),
        },
        Case {
            id: "zero-len-null",
            op: OP_SYSCALL,
            target: 0,
            value: 0,
            sysno: 63,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::Syscall(0),
        },
        Case {
            id: "null-nonzero-read",
            op: OP_SYSCALL,
            target: 0,
            value: 0,
            sysno: 63,
            a0: 0,
            a1: 0,
            a2: 1,
            judge: Judge::Syscall(errno_u64(14)),
        },
        Case {
            id: "cross-page-read",
            op: OP_SYSCALL,
            target: 0,
            value: 0,
            sysno: 63,
            a0: 0,
            a1: (DATA_PAGE + PAGE_SIZE - 16) as u64,
            a2: 32,
            judge: Judge::Syscall(errno_u64(14)),
        },
        Case {
            id: "zircon-vmo-read-kernel",
            op: OP_SYSCALL,
            target: 0,
            value: 0,
            sysno: 1069,
            a0: 0,
            a1: magic,
            a2: 8,
            judge: Judge::Syscall(zircon_invalid_args()),
        },
        Case {
            id: "el0-other-process",
            op: OP_READ,
            target: OTHER_PAGE as u64,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::OtherProcess,
        },
        Case {
            id: "cow-fork",
            op: OP_COW,
            target: COW_PAGE as u64,
            value: COW_CHILD,
            sysno: 0,
            a0: COW_PARENT,
            a1: 0,
            a2: 0,
            judge: Judge::CowFork,
        },
        Case {
            id: "mprotect-after-store",
            op: OP_MPROTECT,
            target: MPROTECT_PAGE as u64,
            value: MPROTECT_FIRST,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::MprotectStore,
        },
        Case {
            id: "foreign-handle",
            op: OP_SYSCALL,
            target: 0,
            value: 0,
            sysno: 1069,
            a0: 0x1000,
            a1: DATA_PAGE as u64,
            a2: 8,
            judge: Judge::ForeignHandle,
        },
        Case {
            id: "stale-handle",
            op: OP_SYSCALL,
            target: 0,
            value: 0,
            sysno: 1069,
            a0: 0,
            a1: DATA_PAGE as u64,
            a2: 8,
            judge: Judge::StaleHandle,
        },
        Case {
            id: "bad-type-rights",
            op: OP_SYSCALL,
            target: 0,
            value: 0,
            sysno: 1069,
            a0: 0,
            a1: DATA_PAGE as u64,
            a2: 8,
            judge: Judge::BadRights,
        },
        Case {
            id: "closed-fd",
            op: OP_SYSCALL,
            target: 0,
            value: 0,
            sysno: 63,
            a0: 0,
            a1: DATA_PAGE as u64,
            a2: 1,
            judge: Judge::ClosedFd,
        },
        Case {
            id: "getpid-no-process",
            op: 0,
            target: 0,
            value: 0,
            sysno: 172,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::GetpidAbsent,
        },
        Case {
            id: "el0-uaccess",
            op: 0,
            target: 0,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::El0Uaccess,
        },
        Case {
            id: "undef-insn",
            op: OP_UNDEF,
            target: 0,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::PeerTrap,
        },
        Case {
            id: "align-fault",
            op: OP_ALIGN,
            target: (CODE_PAGE + 1) as u64,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::PeerTrap,
        },
        Case {
            id: "exit-futex",
            op: OP_EXIT_FUTEX,
            target: 0,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::ExitFutex,
        },
        Case {
            id: "wait-once",
            op: OP_WAIT_ONCE,
            target: 0,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::WaitOnce,
        },
        Case {
            id: "exit-refork",
            op: OP_EXIT_REFORK,
            target: REFORK_PAGE as u64,
            value: REFORK_PARENT,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::ExitRefork,
        },
        Case {
            id: "bad-signal-handler",
            op: OP_BAD_HANDLER,
            target: 0,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::BadHandler,
        },
        Case {
            id: "no-linux-task",
            op: 0,
            target: 0,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::NoLinuxTask,
        },
        syscall_case!("seccomp-strict", 277, 0, 0, 0, enosys),
        syscall_case!("seccomp-filter", 277, 1, 0, 1, enosys),
        syscall_case!("seccomp-action-avail", 277, 2, 0, 1, enosys),
        syscall_case!("prctl-set-seccomp", 167, 22, 1, 0, einval),
        syscall_case!("prctl-get-seccomp", 167, 21, 0, 0, 0),
        syscall_case!("prctl-set-no-new-privs", 167, 38, 1, 0, 0),
        syscall_case!("prctl-set-no-new-privs-bad", 167, 38, 2, 0, einval),
        syscall_case!("prctl-get-no-new-privs", 167, 39, 0, 0, 0),
        syscall_case!("prctl-capbset-drop", 167, 24, 0, 0, eperm),
        syscall_case!("prctl-set-mdwe", 167, 65, 1, 0, einval),
        syscall_case!("prctl-get-dumpable", 167, 3, 0, 0, 1),
        syscall_case!("landlock-create", 444, 0, 0, 0, enosys),
        syscall_case!("landlock-add", 445, 0, 0, 0, enosys),
        syscall_case!("landlock-restrict", 446, 0, 0, 0, enosys),
        syscall_case!("pkey-alloc", 289, 0, 0, 0, enosys),
        syscall_case!("pkey-free", 290, 0, 0, 0, enosys),
        syscall_case!("pkey-mprotect", 288, 0, 0, 0, enosys),
        syscall_case!("iouring-setup", 425, 8, 0, 0, enosys),
        syscall_case!("iouring-enter", 426, 0, 0, 0, enosys),
        syscall_case!("iouring-register", 427, 0, 0, 0, enosys),
        syscall_case!("ptrace-traceme", 117, 0, 0, 0, eperm),
        syscall_case!("process-vm-readv", 270, 0, 0, 0, enosys),
        syscall_case!("process-vm-writev", 271, 0, 0, 0, enosys),
        syscall_case!("bpf", 280, 0, 0, 0, enosys),
        syscall_case!("io-setup", 0, 1, 0, 0, enosys),
        syscall_case!("io-destroy", 1, 0, 0, 0, enosys),
        syscall_case!("io-submit", 2, 0, 0, 0, enosys),
        syscall_case!("io-cancel", 3, 0, 0, 0, enosys),
        syscall_case!("io-getevents", 4, 0, 0, 0, enosys),
        syscall_case!("io-pgetevents", 292, 0, 0, 0, enosys),
        syscall_case!("lookup-dcookie", 18, 0, 0, 0, enosys),
        syscall_case!("nfsservctl", 42, 0, 0, 0, enosys),
        syscall_case!("settimeofday-noop", 170, 0, 0, 0, 0),
        Case {
            id: "settimeofday-write",
            op: OP_SYSCALL,
            target: 0,
            value: 1,
            sysno: 170,
            a0: timeval,
            a1: 0,
            a2: 0,
            judge: Judge::ClockDenied,
        },
        syscall_case!("adjtimex-fault", 171, 0, 0, 0, efault),
        syscall_case!("adjtimex-read", 171, timex, 0, 0, 5),
        syscall_case!("adjtimex-write", 171, timeval, 0, 0, eperm),
        syscall_case!("clock-adjtime-fault", 266, 0, 0, 0, efault),
        syscall_case!("clock-adjtime-read", 266, 0, timex, 0, 5),
        syscall_case!("clock-adjtime-monotonic", 266, 1, timex, 0, eopnotsupp),
        syscall_case!("remap-file-pages", 234, 0, 0, 0, enosys),
        syscall_case!("mbind", 235, 0, 0, 0, enosys),
        syscall_case!("get-mempolicy", 236, 0, 0, 0, enosys),
        syscall_case!("set-mempolicy", 237, 0, 0, 0, enosys),
        syscall_case!("syncfs-badfd", 267, bad_fd, 0, 0, ebadf),
        syscall_case!("sendmmsg-badfd", 269, bad_fd, timex, 1, enodev),
        syscall_case!("sched-getattr-inval", 275, 0, 0, 0, einval),
        syscall_case!("rseq", 293, 0, 0, 0, enosys),
        syscall_case!("chmod-null", 53, usize::MAX as u64 - 99, 0, 0o644, efault),
        syscall_case!("chmod-badfd", 52, bad_fd, 0o644, 0, ebadf),
        syscall_case!(
            "chmod-enoent",
            53,
            usize::MAX as u64 - 99,
            (DATA_PAGE + 0x620) as u64,
            0o644,
            errno_u64(2)
        ),
        Case {
            id: "chmod-bad-flags",
            op: OP_SYSCALL,
            target: 1,
            value: 0,
            sysno: 452,
            a0: usize::MAX as u64 - 99,
            a1: (DATA_PAGE + 0x600) as u64,
            a2: 0o755,
            judge: Judge::Syscall(einval),
        },
        syscall_case!(
            "chmod-system-dir",
            53,
            usize::MAX as u64 - 99,
            (DATA_PAGE + 0x600) as u64,
            0o777,
            eperm
        ),
        Case {
            id: "chmod-flags-ignored",
            op: OP_SYSCALL,
            target: 0xbad,
            value: 0,
            sysno: 53,
            a0: usize::MAX as u64 - 99,
            a1: (DATA_PAGE + 0x600) as u64,
            a2: 0o40755,
            judge: Judge::Syscall(0),
        },
        Case {
            id: "chmod-owner-roundtrip",
            op: OP_CHMOD,
            target: 0,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::ChmodRoundtrip,
        },
        Case {
            id: "clock-settime-unpriv",
            op: OP_SYSCALL,
            target: 0,
            value: 1,
            sysno: 112,
            a0: 0,
            a1: (DATA_PAGE + 0x300) as u64,
            a2: 1,
            judge: Judge::ClockDenied,
        },
        Case {
            id: "clock-settime-root-no-cap",
            op: OP_SYSCALL,
            target: 0,
            value: 2,
            sysno: 112,
            a0: 0,
            a1: (DATA_PAGE + 0x300) as u64,
            a2: 1,
            judge: Judge::ClockDenied,
        },
        Case {
            id: "clock-settime-priv",
            op: OP_SYSCALL,
            target: 0,
            value: 0,
            sysno: 112,
            a0: 0,
            a1: (DATA_PAGE + 0x300) as u64,
            a2: 2,
            judge: Judge::ClockPrivileged,
        },
        Case {
            id: "clock-settime-foreign-waits",
            op: OP_SYSCALL,
            target: 1,
            value: 1,
            sysno: 112,
            a0: 0,
            a1: (DATA_PAGE + 0x300) as u64,
            a2: 1,
            judge: Judge::ClockDenied,
        },
        Case {
            id: "settimeofday-privileged",
            op: OP_SYSCALL,
            target: 0,
            value: 0,
            sysno: 170,
            a0: (DATA_PAGE + 0x340) as u64,
            a1: 0,
            a2: 3,
            judge: Judge::ClockPrivileged,
        },
        Case {
            id: "notify-gate-sleep",
            op: OP_NOTIFY_GATE,
            target: 0,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::NotifyGateSleep,
        },
    ]
}

fn errno_u64(errno: u64) -> u64 {
    (u64::MAX - errno) + 1
}

fn zircon_invalid_args() -> u64 {
    zircon_status(-10)
}

fn zircon_status(status: i32) -> u64 {
    status as i64 as u64
}

fn execute(case: Case) -> FinishedCase {
    match case.judge {
        Judge::Later { stage, detail } => expected(stage, detail, false),
        Judge::GetpidAbsent => judge_getpid_absent(),
        Judge::El0Uaccess => judge_el0_uaccess(),
        Judge::NoLinuxTask => run_no_linux_task(),
        Judge::Trap
        | Judge::Syscall(_)
        | Judge::OtherProcess
        | Judge::CowFork
        | Judge::MprotectStore
        | Judge::ForeignHandle
        | Judge::StaleHandle
        | Judge::BadRights
        | Judge::ClosedFd
        | Judge::PeerTrap
        | Judge::ExitFutex
        | Judge::WaitOnce
        | Judge::ExitRefork
        | Judge::BadHandler
        | Judge::ChmodRoundtrip
        | Judge::ClockDenied
        | Judge::ClockPrivileged
        | Judge::NotifyGateSleep => run_probe(case),
    }
}

fn run_undef(serial: &mut Serial) {
    if let Some(case) = cases().into_iter().find(|case| case.id == "undef-insn") {
        run_one(serial, case);
    }
}

fn run_probe(case: Case) -> FinishedCase {
    crate::syscall::security_finish_clock_probe();
    CLOCK_SNAPSHOT_VALID.store(false, Ordering::Relaxed);
    linux_task::reset();
    linux_process_memory::reset_launch();
    PROBE_THREAD.store(usize::MAX, Ordering::Relaxed);
    WATCH_PAGE.store(0, Ordering::Relaxed);
    WATCH_WORD.store(0, Ordering::Relaxed);
    WATCH_VALID.store(false, Ordering::Relaxed);
    SETUP_OTHER.store(false, Ordering::Relaxed);
    drop_foreign_space();
    if matches!(case.judge, Judge::OtherProcess) {
        SETUP_OTHER.store(true, Ordering::Relaxed);
    }
    if matches!(case.judge, Judge::MprotectStore) {
        WATCH_PAGE.store(MPROTECT_PAGE, Ordering::Relaxed);
    }
    MAGIC_WORD.store(MAGIC, Ordering::Relaxed);
    EXEC_HITS.store(0, Ordering::Relaxed);
    FAULT_COUNT.store(0, Ordering::Relaxed);
    FAULT_SEEN.store(false, Ordering::Relaxed);
    DEFER_DONE.store(matches!(case.judge, Judge::BadHandler), Ordering::Relaxed);
    PEER_DONE.store(false, Ordering::Relaxed);
    PEER_STOP.store(false, Ordering::Relaxed);
    PEER_VALUE.store(0, Ordering::Relaxed);
    DONE.store(false, Ordering::Relaxed);
    FAULTED.store(false, Ordering::Relaxed);
    EXITED.store(false, Ordering::Relaxed);
    EXIT_CODE.store(0, Ordering::Relaxed);
    LAST_SYSCALL.store(0, Ordering::Relaxed);
    LAST_RESULT.store(u64::MAX, Ordering::Relaxed);
    SETUP_ERROR.store(0, Ordering::Relaxed);
    CFG_OP.store(case.op, Ordering::Relaxed);
    CFG_TARGET.store(case.target, Ordering::Relaxed);
    CFG_VALUE.store(case.value, Ordering::Relaxed);
    CFG_SYSNO.store(case.sysno, Ordering::Relaxed);
    CFG_A0.store(case.a0, Ordering::Relaxed);
    CFG_A1.store(case.a1, Ordering::Relaxed);
    CFG_A2.store(case.a2, Ordering::Relaxed);
    PREPARE.store(
        match case.judge {
            Judge::StaleHandle => PREPARE_STALE,
            Judge::BadRights => PREPARE_RIGHTS,
            Judge::ClosedFd => PREPARE_CLOSED_FD,
            Judge::Syscall(_) if case.sysno == 1069 => PREPARE_LIVE_VMO,
            _ => PREPARE_NONE,
        },
        Ordering::Relaxed,
    );
    let require_peer = matches!(case.judge, Judge::PeerTrap);
    if require_peer && !start_peer() {
        return finished(Verdict::Fail, "peer");
    }
    ACTIVE.store(true, Ordering::Release);
    let created =
        scheduler::scheduler().create_thread_on_cpu(security_case_thread, "sectest", Some(0));
    let Some(created) = created else {
        ACTIVE.store(false, Ordering::Release);
        return finished(Verdict::Fail, "thread");
    };
    PROBE_THREAD.store(created.0, Ordering::Relaxed);
    let start = crate::kernel_lowlevel::timer::get_nanoseconds();
    while !DONE.load(Ordering::Acquire) || (require_peer && !PEER_DONE.load(Ordering::Acquire)) {
        yield_now();
        if smros_sectest_case_timed_out_body!(
            start,
            crate::kernel_lowlevel::timer::get_nanoseconds(),
            CASE_TIMEOUT_NS
        )
        {
            PEER_STOP.store(true, Ordering::Release);
            ACTIVE.store(false, Ordering::Release);
            crate::syscall::security_finish_clock_probe();
            return finished(Verdict::Fail, "timeout");
        }
    }
    ACTIVE.store(false, Ordering::Release);
    judge(case)
}

fn judge(case: Case) -> FinishedCase {
    if SETUP_ERROR.load(Ordering::Relaxed) != 0 {
        crate::syscall::security_finish_clock_probe();
        return finished(Verdict::Fail, "setup");
    }
    let magic_ok = MAGIC_WORD.load(Ordering::Relaxed) == MAGIC;
    let exec_ok = EXEC_HITS.load(Ordering::Relaxed) == 0;
    let faulted = FAULTED.load(Ordering::Relaxed);
    match case.judge {
        Judge::Trap => match smros_sectest_trap_verdict_body!(faulted, magic_ok, exec_ok) {
            0 => finished(Verdict::Pass, "trapped"),
            1 => finished(Verdict::Fail, "magic-changed"),
            2 => finished(Verdict::Fail, "kernel-executed"),
            _ => finished(Verdict::Fail, "no-fault"),
        },
        Judge::Syscall(expected) => {
            let result = LAST_RESULT.load(Ordering::Relaxed);
            if faulted {
                finished(Verdict::Fail, "faulted")
            } else if !magic_ok {
                finished(Verdict::Fail, "magic-changed")
            } else if result == expected {
                finished(Verdict::Pass, "errno")
            } else {
                finished(Verdict::Fail, "unexpected-return")
            }
        }
        Judge::Later { stage, detail } => expected(stage, detail, false),
        Judge::OtherProcess => judge_other_process(),
        Judge::CowFork => judge_cow_fork(),
        Judge::MprotectStore => judge_mprotect_store(),
        Judge::ForeignHandle => judge_status(zircon_status(-11), "bad-handle"),
        Judge::StaleHandle => judge_status(zircon_status(-11), "stale"),
        Judge::BadRights => judge_status(zircon_status(-12), "rights"),
        Judge::ClosedFd => judge_status(errno_u64(9), "ebadf"),
        Judge::GetpidAbsent => judge_getpid_absent(),
        Judge::El0Uaccess => finished(Verdict::Fail, "wrong-path"),
        Judge::PeerTrap => judge_peer_trap(magic_ok, exec_ok, faulted),
        Judge::ExitFutex => judge_exit_code(0, "exit-group"),
        Judge::WaitOnce => judge_exit_code(1, "wait-once"),
        Judge::ExitRefork => judge_exit_code(1, "fresh-page"),
        Judge::BadHandler => judge_bad_handler(),
        Judge::ChmodRoundtrip => judge_chmod(),
        Judge::NotifyGateSleep => judge_notify_gate(),
        Judge::NoLinuxTask => finished(Verdict::Fail, "wrong-path"),
        Judge::ClockDenied => judge_clock_denied(),
        Judge::ClockPrivileged => judge_clock_privileged(),
    }
}

fn clock_probe_denied() -> bool {
    if CLOCK_SNAPSHOT_VALID.load(Ordering::Acquire) {
        CLOCK_SNAPSHOT_DENIED.load(Ordering::Relaxed)
    } else {
        crate::syscall::security_clock_change_denied()
    }
}

fn clock_probe_applied() -> bool {
    if CLOCK_SNAPSHOT_VALID.load(Ordering::Acquire) {
        CLOCK_SNAPSHOT_APPLIED.load(Ordering::Relaxed)
    } else {
        crate::syscall::security_clock_change_applied()
    }
}

fn judge_clock_denied() -> FinishedCase {
    let denied = !FAULTED.load(Ordering::Relaxed)
        && LAST_RESULT.load(Ordering::Relaxed) == errno_u64(1)
        && clock_probe_denied();
    crate::syscall::security_finish_clock_probe();
    if denied {
        finished(Verdict::Pass, "eperm-clock-held")
    } else if LAST_RESULT.load(Ordering::Relaxed) != errno_u64(1) {
        finished(Verdict::Fail, "unexpected-return")
    } else {
        finished(Verdict::Fail, "clock-moved")
    }
}

fn judge_clock_privileged() -> FinishedCase {
    let applied = !FAULTED.load(Ordering::Relaxed)
        && LAST_RESULT.load(Ordering::Relaxed) == 0
        && clock_probe_applied();
    crate::syscall::security_finish_clock_probe();
    if applied {
        finished(Verdict::Pass, "clock-set")
    } else if LAST_RESULT.load(Ordering::Relaxed) != 0 {
        finished(Verdict::Fail, "unexpected-return")
    } else {
        finished(Verdict::Fail, "clock-unchanged")
    }
}

fn judge_peer_trap(magic_ok: bool, exec_ok: bool, faulted: bool) -> FinishedCase {
    if !faulted {
        return finished(Verdict::Fail, "no-fault");
    }
    if !magic_ok {
        return finished(Verdict::Fail, "magic-changed");
    }
    if !exec_ok {
        return finished(Verdict::Fail, "kernel-executed");
    }
    if !PEER_DONE.load(Ordering::Acquire) || PEER_VALUE.load(Ordering::Relaxed) == 0 {
        return finished(Verdict::Fail, "peer-incomplete");
    }
    finished(Verdict::Pass, "trapped-peer")
}

fn judge_exit_code(expected: i32, pass_detail: &'static str) -> FinishedCase {
    if !EXITED.load(Ordering::Relaxed) {
        return finished(Verdict::Fail, "no-exit");
    }
    match EXIT_CODE.load(Ordering::Relaxed) {
        code if code == expected => finished(Verdict::Pass, pass_detail),
        2 => finished(Verdict::Fail, "saw-old"),
        3 => finished(Verdict::Fail, "fork-failed"),
        4 => finished(Verdict::Fail, "clone-failed"),
        5 => finished(Verdict::Fail, "sigaction-failed"),
        _ => finished(Verdict::Fail, "unexpected-exit"),
    }
}

fn judge_notify_gate() -> FinishedCase {
    if !EXITED.load(Ordering::Relaxed) {
        return finished(Verdict::Fail, "no-exit");
    }
    match EXIT_CODE.load(Ordering::Relaxed) {
        0 => finished(Verdict::Pass, "slept-on-time"),
        21 => finished(Verdict::Fail, "sigmask"),
        22 => finished(Verdict::Fail, "timer-create"),
        23 => finished(Verdict::Fail, "timer-settime"),
        24 => finished(Verdict::Fail, "sigwait"),
        25 => finished(Verdict::Fail, "fork"),
        26 => finished(Verdict::Fail, "clock"),
        27 => finished(Verdict::Fail, "wait"),
        29 => finished(Verdict::Fail, "child-signal"),
        31 => finished(Verdict::Fail, "child-clock"),
        32 => finished(Verdict::Fail, "nanosleep"),
        33 => finished(Verdict::Fail, "nanosleep-late"),
        34 => finished(Verdict::Fail, "clock-nanosleep"),
        35 => finished(Verdict::Fail, "clock-nanosleep-late"),
        36 => finished(Verdict::Fail, "nanosleep-early"),
        37 => finished(Verdict::Fail, "clock-nanosleep-early"),
        _ => finished(Verdict::Fail, "unexpected-exit"),
    }
}

fn judge_chmod() -> FinishedCase {
    if !EXITED.load(Ordering::Relaxed) {
        return finished(Verdict::Fail, "no-exit");
    }
    match EXIT_CODE.load(Ordering::Relaxed) {
        0 => finished(Verdict::Pass, "owner-mode"),
        11 => finished(Verdict::Fail, "setuid"),
        12 => finished(Verdict::Fail, "open"),
        13 => finished(Verdict::Fail, "owner-chmod"),
        14 => finished(Verdict::Fail, "stat"),
        15 => finished(Verdict::Fail, "mode"),
        16 => finished(Verdict::Fail, "setuid-other"),
        17 => finished(Verdict::Fail, "not-eperm"),
        18 => finished(Verdict::Fail, "restat"),
        19 => finished(Verdict::Fail, "mode-changed"),
        _ => finished(Verdict::Fail, "unexpected-exit"),
    }
}

fn judge_bad_handler() -> FinishedCase {
    let faults = FAULT_COUNT.load(Ordering::Relaxed);
    if EXITED.load(Ordering::Relaxed) && EXIT_CODE.load(Ordering::Relaxed) == 5 {
        return finished(Verdict::Fail, "sigaction-failed");
    }
    if faults == 1 && FAULTED.load(Ordering::Relaxed) {
        finished(Verdict::Pass, "rejected-handler")
    } else if faults > 1 {
        finished(Verdict::Fail, "handler-ran")
    } else {
        finished(Verdict::Fail, "no-fault")
    }
}

fn judge_status(expected_result: u64, detail: &'static str) -> FinishedCase {
    if SETUP_ERROR.load(Ordering::Relaxed) != 0 {
        return finished(Verdict::Fail, "setup");
    }
    if FAULTED.load(Ordering::Relaxed) {
        return finished(Verdict::Fail, "faulted");
    }
    if MAGIC_WORD.load(Ordering::Relaxed) != MAGIC {
        return finished(Verdict::Fail, "magic-changed");
    }
    if LAST_RESULT.load(Ordering::Relaxed) == expected_result {
        finished(Verdict::Pass, detail)
    } else {
        finished(Verdict::Fail, "unexpected-return")
    }
}

fn judge_getpid_absent() -> FinishedCase {
    linux_task::reset();
    linux_process::reset_launch();
    // SPSR 0 is EL0t. This thread is not inside the exception vector.
    let _guard = crate::syscall::uaccess::El0SyscallGuard::enter_saved_spsr(0);
    match crate::syscall::sys_getpid() {
        Err(SysError::ESRCH) => finished(Verdict::Pass, "esrch"),
        _ => finished(Verdict::Fail, "not-esrch"),
    }
}

fn judge_el0_uaccess() -> FinishedCase {
    let finished = judge_el0_uaccess_body();
    linux_task::reset();
    linux_process_memory::reset_launch();
    finished
}

fn judge_el0_uaccess_body() -> FinishedCase {
    linux_task::reset();
    linux_process_memory::reset_launch();
    UACCESS_CANARY.store(0x00C0_FFEE, Ordering::Relaxed);
    UACCESS_PEER_DONE.store(false, Ordering::Relaxed);

    {
        // EL1h is not an EL0 entry, even though a per-thread slot is occupied.
        let _el1 = crate::syscall::uaccess::El0SyscallGuard::enter_saved_spsr(0x3C5);
        if crate::syscall::uaccess::el0_syscall_active() {
            return finished(Verdict::Fail, "el1-spsr");
        }
    }

    let thread = scheduler::scheduler().current();
    let Ok(pid) = linux_task::register_root(thread) else {
        return finished(Verdict::Fail, "register");
    };
    if linux_process_memory::register_root(pid).is_err() {
        return finished(Verdict::Fail, "address-space");
    }
    let mut page = [0u8; PAGE_SIZE];
    let pattern = [0xE1u8, 0x0A, 0x5C, 0xE5, 0xA5, 0x5A, 0x11, 0x22];
    page[..pattern.len()].copy_from_slice(&pattern);
    let prot = linux_process_memory::LINUX_PROT_READ | linux_process_memory::LINUX_PROT_WRITE;
    let mapped = linux_process_memory::map_current_with_contents(
        Some(DATA_PAGE),
        PAGE_SIZE,
        prot,
        1usize << 1,
        linux_process_memory::LinuxMappingSource::Anonymous,
        false,
        &page,
    );
    let Ok(mapped) = mapped else {
        return finished(Verdict::Fail, "map");
    };
    if mapped != DATA_PAGE {
        return finished(Verdict::Fail, "map-addr");
    }

    let el0 = crate::syscall::uaccess::El0SyscallGuard::enter_saved_spsr(0);
    if !crate::syscall::uaccess::el0_syscall_active() {
        return finished(Verdict::Fail, "el0-spsr");
    }
    if scheduler::scheduler()
        .create_thread_on_cpu(uaccess_peer_thread, "uaccess-peer", Some(0))
        .is_none()
    {
        return finished(Verdict::Fail, "peer");
    }
    let start = crate::kernel_lowlevel::timer::get_nanoseconds();
    while !UACCESS_PEER_DONE.load(Ordering::Acquire) {
        yield_now();
        if smros_sectest_case_timed_out_body!(
            start,
            crate::kernel_lowlevel::timer::get_nanoseconds(),
            CASE_TIMEOUT_NS
        )
        {
            return finished(Verdict::Fail, "peer-timeout");
        }
    }
    if !crate::syscall::uaccess::el0_syscall_active() {
        return finished(Verdict::Fail, "mark-cleared");
    }

    let mut got = [0u8; 8];
    if linux_process_memory::copy_from_current(DATA_PAGE, &mut got).is_err() {
        return finished(Verdict::Fail, "user-fault");
    }
    if got != pattern {
        return finished(Verdict::Fail, "user-mismatch");
    }

    let canary_addr = &UACCESS_CANARY as *const AtomicU64 as usize;
    let poison = [0xFFu8; 8];
    if linux_process_memory::copy_from_current(magic_address(), &mut got).is_ok() {
        return finished(Verdict::Fail, "kernel-read");
    }
    if linux_process_memory::copy_to_current(canary_addr, &poison).is_ok()
        || UACCESS_CANARY.load(Ordering::Relaxed) != 0x00C0_FFEE
    {
        UACCESS_CANARY.store(0x00C0_FFEE, Ordering::Relaxed);
        return finished(Verdict::Fail, "kernel-write");
    }
    if linux_process_memory::copy_from_current(0, &mut got[..1]).is_ok() {
        return finished(Verdict::Fail, "null");
    }
    if linux_process_memory::copy_from_current(0x1FFF_FFFC, &mut got).is_ok() {
        return finished(Verdict::Fail, "overflow");
    }
    if linux_process_memory::copy_from_current(0x1800_0000, &mut got).is_ok() {
        return finished(Verdict::Fail, "unmapped");
    }

    // No Linux pid. The peer already dropped its own entry. A single global
    // slot would now raw-copy the kernel address.
    linux_task::reset();
    linux_process_memory::reset_launch();
    if !crate::syscall::uaccess::el0_syscall_active() {
        return finished(Verdict::Fail, "mark-cleared");
    }
    if linux_process_memory::copy_from_current(magic_address(), &mut got).is_ok() {
        return finished(Verdict::Fail, "bypassed");
    }
    if linux_process_memory::copy_to_current(canary_addr, &poison).is_ok()
        || UACCESS_CANARY.load(Ordering::Relaxed) != 0x00C0_FFEE
    {
        UACCESS_CANARY.store(0x00C0_FFEE, Ordering::Relaxed);
        return finished(Verdict::Fail, "bypassed-write");
    }
    drop(el0);
    if crate::syscall::uaccess::el0_syscall_active() {
        return finished(Verdict::Fail, "still-el0");
    }
    if linux_process_memory::copy_from_current(canary_addr, &mut got).is_err()
        || got != 0x00C0_FFEEu64.to_ne_bytes()
    {
        return finished(Verdict::Fail, "kernel-path");
    }
    finished(Verdict::Pass, "spsr-translated")
}

extern "C" fn uaccess_peer_thread() -> ! {
    {
        let _guard = crate::syscall::uaccess::El0SyscallGuard::enter_saved_spsr(0);
    }
    UACCESS_PEER_DONE.store(true, Ordering::Release);
    linux_task::finish_current_without_el0_return();
}

fn judge_other_process() -> FinishedCase {
    let isolated = foreign_page_intact();
    drop_foreign_space();
    if SETUP_ERROR.load(Ordering::Relaxed) != 0 {
        return finished(Verdict::Fail, "setup");
    }
    if FAULTED.load(Ordering::Relaxed) {
        return finished(Verdict::Fail, "faulted");
    }
    let code = EXIT_CODE.load(Ordering::Relaxed) as u32 as u64;
    if code == OTHER_MAGIC {
        return finished(Verdict::Fail, "saw-foreign");
    }
    if code != 0 {
        return finished(Verdict::Fail, "unexpected-read");
    }
    if isolated {
        finished(Verdict::Pass, "second-address-space")
    } else {
        finished(Verdict::Fail, "foreign-mutated")
    }
}

fn judge_cow_fork() -> FinishedCase {
    if SETUP_ERROR.load(Ordering::Relaxed) != 0 {
        return finished(Verdict::Fail, "setup");
    }
    if FAULTED.load(Ordering::Relaxed) {
        return finished(Verdict::Fail, "faulted");
    }
    match EXIT_CODE.load(Ordering::Relaxed) {
        1 => finished(Verdict::Pass, "fork-cow"),
        2 => finished(Verdict::Fail, "parent-changed"),
        3 => finished(Verdict::Fail, "fork-failed"),
        _ => finished(Verdict::Fail, "unexpected-exit"),
    }
}

fn judge_mprotect_store() -> FinishedCase {
    if SETUP_ERROR.load(Ordering::Relaxed) != 0 {
        return finished(Verdict::Fail, "setup");
    }
    let word = WATCH_WORD.load(Ordering::Relaxed);
    let valid = WATCH_VALID.load(Ordering::Relaxed);
    if !FAULTED.load(Ordering::Relaxed) {
        if valid && word == MPROTECT_SECOND {
            return finished(Verdict::Fail, "stale-store");
        }
        return finished(Verdict::Fail, "no-fault");
    }
    if !valid {
        return finished(Verdict::Fail, "no-snapshot");
    }
    if word == MPROTECT_SECOND {
        finished(Verdict::Fail, "stale-store")
    } else if word != MPROTECT_FIRST {
        finished(Verdict::Fail, "missing-first-store")
    } else {
        finished(Verdict::Pass, "faulted")
    }
}

fn foreign_page_intact() -> bool {
    let phys = FOREIGN_PHYS.load(Ordering::Relaxed);
    if phys == 0 {
        return false;
    }
    let magic = unsafe { core::ptr::read_volatile(phys as *const u64) };
    if magic != OTHER_MAGIC {
        return false;
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let space = &*core::ptr::addr_of!(FOREIGN_SPACE);
        return space.as_ref().is_some_and(|space| {
            space.translate_user(OTHER_PAGE, false) == Some(phys & !(PAGE_SIZE - 1))
        });
    }
    #[cfg(not(target_arch = "aarch64"))]
    false
}

fn drop_foreign_space() {
    FOREIGN_PHYS.store(0, Ordering::Relaxed);
    #[cfg(target_arch = "aarch64")]
    unsafe {
        (*core::ptr::addr_of_mut!(FOREIGN_SPACE)).take();
    }
}

#[cfg(target_arch = "aarch64")]
fn install_foreign_page() -> Result<(), u64> {
    use crate::kernel_lowlevel::memory::PageFrameAllocator;
    let mut space =
        crate::kernel_lowlevel::Aarch64AddressSpace::new_with_kernel_map().map_err(|_| 4u64)?;
    let pfn = PageFrameAllocator::alloc().ok_or(5u64)?;
    let Some(phys) = PageFrameAllocator::pfn_address(pfn) else {
        PageFrameAllocator::free(pfn);
        return Err(6);
    };
    unsafe {
        core::ptr::write_bytes(phys as *mut u8, 0, PAGE_SIZE);
        core::ptr::write(phys as *mut u64, OTHER_MAGIC);
    }
    if space
        .map_user_page(OTHER_PAGE, pfn, true, false, false)
        .is_err()
    {
        PageFrameAllocator::free(pfn);
        return Err(7);
    }
    FOREIGN_PHYS.store(phys, Ordering::Relaxed);
    unsafe {
        core::ptr::addr_of_mut!(FOREIGN_SPACE).write(Some(space));
    }
    Ok(())
}

#[cfg(not(target_arch = "aarch64"))]
fn install_foreign_page() -> Result<(), u64> {
    Err(4)
}

fn prepare_handle_case(pid: usize) -> Result<(), u64> {
    let prepare = PREPARE.load(Ordering::Relaxed);
    if prepare == PREPARE_NONE {
        return Ok(());
    }
    let vmo_rights =
        crate::kernel_objects::default_rights_for_object(crate::kernel_objects::ObjectType::Vmo);
    let handle = match prepare {
        PREPARE_LIVE_VMO => {
            crate::syscall::security_create_process_vmo(pid, vmo_rights).map_err(|_| 8u64)?
        }
        PREPARE_STALE => {
            let stale =
                crate::syscall::security_create_process_vmo(pid, vmo_rights).map_err(|_| 8u64)?;
            crate::syscall::security_close_process_handle(pid, stale).map_err(|_| 9u64)?;
            let _reused =
                crate::syscall::security_create_process_vmo(pid, vmo_rights).map_err(|_| 8u64)?;
            stale
        }
        PREPARE_RIGHTS => crate::syscall::security_create_process_vmo(
            pid,
            crate::kernel_objects::Rights::Duplicate as u32,
        )
        .map_err(|_| 8u64)?,
        PREPARE_CLOSED_FD => {
            let fd = crate::syscall::sys_eventfd2(0, 0).map_err(|_| 8u64)?;
            crate::syscall::sys_close(fd).map_err(|_| 9u64)?;
            fd as u32
        }
        _ => return Ok(()),
    };
    CFG_A0.store(handle as u64, Ordering::Relaxed);
    Ok(())
}

extern "C" fn security_case_thread() -> ! {
    if let Err(error) = launch_probe() {
        SETUP_ERROR.store(error as u64, Ordering::Relaxed);
        DONE.store(true, Ordering::Release);
    }
    linux_task::finish_current_without_el0_return();
}

fn launch_probe() -> Result<(), u64> {
    let thread = scheduler::scheduler().current();
    let pid = linux_task::register_root(thread).map_err(error_code)?;
    if CFG_OP.load(Ordering::Relaxed) == OP_NOTIFY_GATE {
        // timer_create adopts a compatibility object. The probe is not a
        // forked ELF, so it has no handle namespace until one is installed.
        crate::syscall::ensure_linux_root_resources(pid).map_err(|_| 8u64)?;
    }
    prepare_handle_case(pid)?;
    let root = match linux_process_memory::register_root(pid) {
        Ok(root) => root,
        Err(error) => {
            linux_process::reset_launch();
            linux_process_memory::reset_launch();
            return Err(error_code(error));
        }
    };
    let prot_rx = linux_process_memory::LINUX_PROT_READ | linux_process_memory::LINUX_PROT_EXEC;
    let prot_rw = linux_process_memory::LINUX_PROT_READ | linux_process_memory::LINUX_PROT_WRITE;
    let flags = 1usize << 1;
    let mut code = [0u8; PAGE_SIZE];
    let len = core::cmp::min(PROBE_CODE.len(), code.len());
    code[..len].copy_from_slice(&PROBE_CODE[..len]);
    linux_process_memory::map_current_with_contents(
        Some(CODE_PAGE),
        PAGE_SIZE,
        prot_rx,
        flags,
        linux_process_memory::LinuxMappingSource::Anonymous,
        false,
        &code,
    )
    .map_err(abort_setup)?;
    let mut data = [0u8; PAGE_SIZE];
    put_u64(&mut data, 0, CFG_OP.load(Ordering::Relaxed));
    put_u64(&mut data, 8, CFG_TARGET.load(Ordering::Relaxed));
    put_u64(&mut data, 16, CFG_VALUE.load(Ordering::Relaxed));
    put_u64(&mut data, 24, CFG_SYSNO.load(Ordering::Relaxed));
    put_u64(&mut data, 32, CFG_A0.load(Ordering::Relaxed));
    put_u64(&mut data, 40, CFG_A1.load(Ordering::Relaxed));
    put_u64(&mut data, 48, CFG_A2.load(Ordering::Relaxed));
    if CFG_OP.load(Ordering::Relaxed) == OP_MPROTECT {
        put_u64(&mut data, 64, MPROTECT_SECOND);
    }
    if CFG_OP.load(Ordering::Relaxed) == OP_SYSCALL {
        put_u64(&mut data, 64, CFG_TARGET.load(Ordering::Relaxed));
    }
    let chmod_path = b"/dev/shm/smros-sec-chmod\0";
    data[0x400..0x400 + chmod_path.len()].copy_from_slice(chmod_path);
    let shm_path = b"/dev/shm\0";
    data[0x600..0x600 + shm_path.len()].copy_from_slice(shm_path);
    let missing_path = b"/no/such/smros-chmod\0";
    data[0x620..0x620 + missing_path.len()].copy_from_slice(missing_path);
    prepare_clock_request(&mut data).map_err(abort_setup)?;
    linux_process_memory::map_current_with_contents(
        Some(DATA_PAGE),
        PAGE_SIZE,
        prot_rw,
        flags,
        linux_process_memory::LinuxMappingSource::Anonymous,
        false,
        &data,
    )
    .map_err(abort_setup)?;
    let ro = [0xA5u8; PAGE_SIZE];
    linux_process_memory::map_current_with_contents(
        Some(RO_PAGE),
        PAGE_SIZE,
        linux_process_memory::LINUX_PROT_READ,
        flags,
        linux_process_memory::LinuxMappingSource::Anonymous,
        false,
        &ro,
    )
    .map_err(abort_setup)?;
    if SETUP_OTHER.load(Ordering::Relaxed) {
        install_foreign_page()?;
        let zeros = [0u8; PAGE_SIZE];
        linux_process_memory::map_current_with_contents(
            Some(OTHER_PAGE),
            PAGE_SIZE,
            prot_rw,
            flags,
            linux_process_memory::LinuxMappingSource::Anonymous,
            false,
            &zeros,
        )
        .map_err(abort_setup)?;
    }
    if CFG_OP.load(Ordering::Relaxed) == OP_COW {
        let mut cow = [0u8; PAGE_SIZE];
        put_u64(&mut cow, 0, COW_PARENT);
        linux_process_memory::map_current_with_contents(
            Some(COW_PAGE),
            PAGE_SIZE,
            prot_rw,
            flags,
            linux_process_memory::LinuxMappingSource::Anonymous,
            false,
            &cow,
        )
        .map_err(abort_setup)?;
    }
    if CFG_OP.load(Ordering::Relaxed) == OP_MPROTECT {
        let page = [0u8; PAGE_SIZE];
        linux_process_memory::map_current_with_contents(
            Some(MPROTECT_PAGE),
            PAGE_SIZE,
            prot_rw,
            flags,
            linux_process_memory::LinuxMappingSource::Anonymous,
            false,
            &page,
        )
        .map_err(abort_setup)?;
    }
    if CFG_OP.load(Ordering::Relaxed) == OP_EXIT_REFORK {
        let mut page = [0u8; PAGE_SIZE];
        put_u64(&mut page, 0, REFORK_PARENT);
        linux_process_memory::map_current_with_contents(
            Some(REFORK_PAGE),
            PAGE_SIZE,
            prot_rw,
            flags,
            linux_process_memory::LinuxMappingSource::Anonymous,
            false,
            &page,
        )
        .map_err(abort_setup)?;
    }
    let stack_len = STACK_PAGES * PAGE_SIZE;
    let stack = vec![0u8; stack_len];
    linux_process_memory::map_current_with_contents(
        Some(STACK_PAGE),
        stack_len,
        prot_rw,
        flags,
        linux_process_memory::LinuxMappingSource::Anonymous,
        false,
        &stack,
    )
    .map_err(abort_setup)?;
    let stack_top = (STACK_PAGE + stack_len) as u64;
    if let Err(error) = prepare_clock_identity() {
        crate::syscall::security_finish_clock_probe();
        return Err(abort_setup(error));
    }
    unsafe {
        crate::user_level::user_process::switch_to_el0(CODE_PAGE as u64, stack_top, root);
    }
}

fn prepare_clock_request(data: &mut [u8]) -> Result<(), SysError> {
    let sysno = CFG_SYSNO.load(Ordering::Relaxed);
    if sysno != 112 && sysno != 170 {
        return Ok(());
    }
    crate::syscall::security_note_clock_probe();
    let kind = CFG_A2.load(Ordering::Relaxed);
    if kind != 1 && kind != 2 && kind != 3 {
        return Ok(());
    }
    let address = if kind == 3 {
        CFG_A0.load(Ordering::Relaxed)
    } else {
        CFG_A1.load(Ordering::Relaxed)
    };
    let offset = (address as usize)
        .checked_sub(DATA_PAGE)
        .ok_or(SysError::EINVAL)?;
    crate::syscall::security_fill_clock_request(data, offset, kind).map_err(|_| SysError::EINVAL)
}

fn prepare_clock_identity() -> Result<(), SysError> {
    let sysno = CFG_SYSNO.load(Ordering::Relaxed);
    if sysno != 112 && sysno != 170 {
        return Ok(());
    }
    if CFG_TARGET.load(Ordering::Relaxed) == 1
        && !crate::syscall::security_plant_foreign_realtime_waits()
    {
        return Err(SysError::EAGAIN);
    }
    let mode = CFG_VALUE.load(Ordering::Relaxed);
    if (mode == 1 || mode == 2) && !crate::syscall::security_set_clock_credentials(mode) {
        return Err(SysError::EPERM);
    }
    Ok(())
}

fn put_u64(buf: &mut [u8], offset: usize, value: u64) {
    buf[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn abort_setup(error: SysError) -> u64 {
    linux_task::reset();
    linux_process_memory::reset_launch();
    error as u64
}

fn error_code(error: SysError) -> u64 {
    error as u64
}

fn start_peer() -> bool {
    PEER_STOP.store(false, Ordering::Release);
    PEER_DONE.store(false, Ordering::Release);
    scheduler::scheduler()
        .create_thread_on_cpu(security_peer_thread, "sectest-peer", Some(0))
        .is_some()
}

extern "C" fn security_peer_thread() -> ! {
    let start = crate::kernel_lowlevel::timer::get_nanoseconds();
    while !FAULT_SEEN.load(Ordering::Acquire) && !PEER_STOP.load(Ordering::Acquire) {
        yield_now();
        if smros_sectest_case_timed_out_body!(
            start,
            crate::kernel_lowlevel::timer::get_nanoseconds(),
            CASE_TIMEOUT_NS
        )
        {
            break;
        }
    }
    if FAULT_SEEN.load(Ordering::Acquire) {
        let mut acc = 1u64;
        for value in 1..64u64 {
            acc = acc.wrapping_mul(value).wrapping_add(0x51);
        }
        PEER_VALUE.store(acc, Ordering::Relaxed);
        PEER_DONE.store(true, Ordering::Release);
    }
    linux_task::finish_current_without_el0_return();
}

fn run_no_linux_task() -> FinishedCase {
    linux_task::reset();
    linux_process_memory::reset_launch();
    PROBE_THREAD.store(usize::MAX, Ordering::Relaxed);
    drop_foreign_space();
    drop_orphan_space();
    MAGIC_WORD.store(MAGIC, Ordering::Relaxed);
    EXEC_HITS.store(0, Ordering::Relaxed);
    FAULT_COUNT.store(0, Ordering::Relaxed);
    FAULT_SEEN.store(false, Ordering::Relaxed);
    DEFER_DONE.store(false, Ordering::Relaxed);
    DONE.store(false, Ordering::Relaxed);
    FAULTED.store(false, Ordering::Relaxed);
    EXITED.store(false, Ordering::Relaxed);
    SETUP_ERROR.store(0, Ordering::Relaxed);
    if !start_peer() {
        return finished(Verdict::Fail, "peer");
    }
    ACTIVE.store(true, Ordering::Release);
    let created =
        scheduler::scheduler().create_thread_on_cpu(orphan_el0_thread, "sectest-orphan", Some(0));
    let Some(created) = created else {
        PEER_STOP.store(true, Ordering::Release);
        ACTIVE.store(false, Ordering::Release);
        return finished(Verdict::Fail, "thread");
    };
    PROBE_THREAD.store(created.0, Ordering::Relaxed);
    let start = crate::kernel_lowlevel::timer::get_nanoseconds();
    while !DONE.load(Ordering::Acquire) || !PEER_DONE.load(Ordering::Acquire) {
        yield_now();
        if smros_sectest_case_timed_out_body!(
            start,
            crate::kernel_lowlevel::timer::get_nanoseconds(),
            CASE_TIMEOUT_NS
        )
        {
            PEER_STOP.store(true, Ordering::Release);
            ACTIVE.store(false, Ordering::Release);
            drop_orphan_space();
            return finished(Verdict::Fail, "timeout");
        }
    }
    ACTIVE.store(false, Ordering::Release);
    drop_orphan_space();
    if SETUP_ERROR.load(Ordering::Relaxed) != 0 {
        return finished(Verdict::Fail, "setup");
    }
    if !FAULTED.load(Ordering::Relaxed) {
        return finished(Verdict::Fail, "no-fault");
    }
    if !PEER_DONE.load(Ordering::Acquire) || PEER_VALUE.load(Ordering::Relaxed) == 0 {
        return finished(Verdict::Fail, "peer-incomplete");
    }
    if MAGIC_WORD.load(Ordering::Relaxed) != MAGIC {
        return finished(Verdict::Fail, "magic-changed");
    }
    finished(Verdict::Pass, "abandoned-el0")
}

extern "C" fn orphan_el0_thread() -> ! {
    if let Err(error) = launch_orphan_el0() {
        SETUP_ERROR.store(error, Ordering::Relaxed);
        DONE.store(true, Ordering::Release);
    }
    linux_task::finish_current_without_el0_return();
}

fn launch_orphan_el0() -> Result<(), u64> {
    #[cfg(target_arch = "aarch64")]
    {
        use crate::kernel_lowlevel::memory::PageFrameAllocator;
        let mut space =
            crate::kernel_lowlevel::Aarch64AddressSpace::new_with_kernel_map().map_err(|_| 4u64)?;
        let code_pfn = PageFrameAllocator::alloc().ok_or(5u64)?;
        let Some(code_phys) = PageFrameAllocator::pfn_address(code_pfn) else {
            PageFrameAllocator::free(code_pfn);
            return Err(6);
        };
        unsafe {
            core::ptr::write_bytes(code_phys as *mut u8, 0, PAGE_SIZE);
        }
        if space
            .map_user_page(CODE_PAGE, code_pfn, true, false, true)
            .is_err()
        {
            PageFrameAllocator::free(code_pfn);
            return Err(7);
        }
        let stack_pfn = PageFrameAllocator::alloc().ok_or(5u64)?;
        let Some(stack_phys) = PageFrameAllocator::pfn_address(stack_pfn) else {
            PageFrameAllocator::free(stack_pfn);
            return Err(6);
        };
        unsafe {
            core::ptr::write_bytes(stack_phys as *mut u8, 0, PAGE_SIZE);
        }
        if space
            .map_user_page(STACK_PAGE, stack_pfn, true, true, false)
            .is_err()
        {
            PageFrameAllocator::free(stack_pfn);
            return Err(7);
        }
        let root = space.root_paddr();
        unsafe {
            core::ptr::addr_of_mut!(ORPHAN_SPACE).write(Some(space));
            crate::user_level::user_process::switch_to_el0(
                CODE_PAGE as u64,
                (STACK_PAGE + PAGE_SIZE) as u64,
                root,
            );
        }
    }
    #[cfg(not(target_arch = "aarch64"))]
    Err(4)
}

fn drop_orphan_space() {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        (*core::ptr::addr_of_mut!(ORPHAN_SPACE)).take();
    }
}
