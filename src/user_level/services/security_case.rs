//! EL0 negative cases for the kernel-security baseline.
//!
//! `sectest` is not part of the default smoke command list. It builds a real
//! AArch64 EL0 process with its own TTBR0 root, copies a fixed machine-code
//! probe into a user page, and records fault or syscall results. Unknown EL0
//! exceptions still halt the kernel unless a later phase changes that; the
//! undefined-instruction case is therefore a separate boot.

use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

use alloc::vec;

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
const MAGIC: u64 = 0x5345_434d_4147_4943;
const CASE_TIMEOUT_NS: u64 = 2_000_000_000;

/// Bytes of `tests/security/aarch64/el0_probe.S` (`.text` only).
const PROBE_CODE: &[u8] = &[
    0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x95, 0x02, 0x40, 0xf9, 0x96, 0x06, 0x40, 0xf9,
    0xbf, 0x06, 0x00, 0xf1, 0x60, 0x01, 0x00, 0x54, 0xbf, 0x0a, 0x00, 0xf1, 0xa0, 0x01, 0x00, 0x54,
    0xbf, 0x0e, 0x00, 0xf1, 0xe0, 0x01, 0x00, 0x54, 0xbf, 0x12, 0x00, 0xf1, 0xc0, 0x01, 0x00, 0x54,
    0xbf, 0x16, 0x00, 0xf1, 0x20, 0x03, 0x00, 0x54, 0x60, 0x0c, 0x80, 0xd2, 0x18, 0x00, 0x00, 0x14,
    0xc0, 0x02, 0x40, 0xf9, 0x80, 0x1e, 0x00, 0xf9, 0x00, 0x00, 0x80, 0xd2, 0x14, 0x00, 0x00, 0x14,
    0x80, 0x0a, 0x40, 0xf9, 0xc0, 0x02, 0x00, 0xf9, 0x00, 0x00, 0x80, 0xd2, 0x10, 0x00, 0x00, 0x14,
    0xc0, 0x02, 0x1f, 0xd6, 0x88, 0x0e, 0x40, 0xf9, 0x80, 0x12, 0x40, 0xf9, 0x81, 0x16, 0x40, 0xf9,
    0x82, 0x1a, 0x40, 0xf9, 0x83, 0x22, 0x40, 0xf9, 0x84, 0x26, 0x40, 0xf9, 0x85, 0x2a, 0x40, 0xf9,
    0x01, 0x00, 0x00, 0xd4, 0x14, 0x00, 0x80, 0xd2, 0x14, 0x40, 0xa2, 0xf2, 0x80, 0x1e, 0x00, 0xf9,
    0x00, 0x00, 0x80, 0xd2, 0x02, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, 0x00, 0xa8, 0x0b, 0x80, 0xd2,
    0x01, 0x00, 0x00, 0xd4, 0xfe, 0xff, 0xff, 0x17,
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

static CFG_OP: AtomicU64 = AtomicU64::new(0);
static CFG_TARGET: AtomicU64 = AtomicU64::new(0);
static CFG_VALUE: AtomicU64 = AtomicU64::new(0);
static CFG_SYSNO: AtomicU64 = AtomicU64::new(0);
static CFG_A0: AtomicU64 = AtomicU64::new(0);
static CFG_A1: AtomicU64 = AtomicU64::new(0);
static CFG_A2: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
enum Verdict {
    Pass,
    Fail,
    Skip,
}

struct FinishedCase {
    verdict: Verdict,
    detail: &'static str,
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
    LAST_SYSCALL.store(number as u64, Ordering::Relaxed);
    LAST_RESULT.store(result, Ordering::Relaxed);
}

pub(crate) fn observe_fault(address: u64) {
    if !ACTIVE.load(Ordering::Acquire) {
        return;
    }
    FAULTED.store(true, Ordering::Relaxed);
    let _ = address;
    DONE.store(true, Ordering::Release);
}

pub(crate) fn observe_exit(code: i32) {
    if !ACTIVE.load(Ordering::Acquire) {
        return;
    }
    EXITED.store(true, Ordering::Relaxed);
    EXIT_CODE.store(code, Ordering::Relaxed);
    DONE.store(true, Ordering::Release);
}

pub fn run(serial: &mut Serial, which: &str) {
    if !cfg!(target_arch = "aarch64") {
        serial.write_str("CASE sectest RESULT SKIP not-aarch64\n");
        serial.write_str("SEC0-SUITE PASS\n");
        return;
    }
    match which {
        "all" => run_listed(serial, false),
        "undef" => run_one(serial, undef_case()),
        "list" => {
            serial.write_str("sectest cases: all | undef | <id>\n");
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
                serial.write_str("SEC0-SUITE FAIL\n");
            }
        }
    }
}

fn run_listed(serial: &mut Serial, include_undef: bool) {
    let mut failed = false;
    for case in cases() {
        if case.op == OP_UNDEF && !include_undef {
            serial.write_str("CASE ");
            serial.write_str(case.id);
            serial.write_str(" RESULT SKIP separate-boot-sectest-undef\n");
            continue;
        }
        let finished = execute(case);
        print_case(serial, case.id, finished.verdict, finished.detail);
        if !matches!(finished.verdict, Verdict::Pass | Verdict::Skip) {
            failed = true;
        }
    }
    serial.write_str("CASE undef-insn RESULT SKIP separate-boot-sectest-undef\n");
    serial.write_str("SEC-HEARTBEAT\n");
    serial.write_str(if failed {
        "SEC0-SUITE FAIL\n"
    } else {
        "SEC0-SUITE PASS\n"
    });
}

fn run_one(serial: &mut Serial, case: Case) {
    let finished = execute(case);
    print_case(serial, case.id, finished.verdict, finished.detail);
    let failed = !matches!(finished.verdict, Verdict::Pass | Verdict::Skip);
    serial.write_str(if failed {
        "SEC0-SUITE FAIL\n"
    } else {
        "SEC0-SUITE PASS\n"
    });
}

fn print_case(serial: &mut Serial, id: &str, verdict: Verdict, detail: &str) {
    serial.write_str("CASE ");
    serial.write_str(id);
    serial.write_str(" RESULT ");
    serial.write_str(match verdict {
        Verdict::Pass => "PASS",
        Verdict::Fail => "FAIL",
        Verdict::Skip => "SKIP",
    });
    serial.write_str(" ");
    serial.write_str(detail);
    serial.write_str("\n");
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
    /// Property is outside SEC-0/SEC-1. Recorded as FAIL, not skipped.
    NotProven(&'static str),
}

fn cases() -> [Case; 16] {
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
            op: 0,
            target: 0,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::NotProven("second-address-space"),
        },
        Case {
            id: "cow-fork",
            op: 0,
            target: 0,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::NotProven("fork-cow"),
        },
        Case {
            id: "foreign-handle",
            op: 0,
            target: 0,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::NotProven("per-process-handle"),
        },
        Case {
            id: "closed-fd",
            op: 0,
            target: 0,
            value: 0,
            sysno: 0,
            a0: 0,
            a1: 0,
            a2: 0,
            judge: Judge::NotProven("fd-generation"),
        },
    ]
}

fn undef_case() -> Case {
    Case {
        id: "undef-insn",
        op: OP_UNDEF,
        target: 0,
        value: 0,
        sysno: 0,
        a0: 0,
        a1: 0,
        a2: 0,
        judge: Judge::Trap,
    }
}

fn errno_u64(errno: u64) -> u64 {
    (u64::MAX - errno) + 1
}

fn zircon_invalid_args() -> u64 {
    -10i64 as u64
}

fn execute(case: Case) -> FinishedCase {
    match case.judge {
        Judge::NotProven(reason) => FinishedCase {
            verdict: Verdict::Fail,
            detail: reason,
        },
        Judge::Trap | Judge::Syscall(_) => run_probe(case),
    }
}

fn run_probe(case: Case) -> FinishedCase {
    linux_task::reset();
    linux_process_memory::reset_launch();
    MAGIC_WORD.store(MAGIC, Ordering::Relaxed);
    EXEC_HITS.store(0, Ordering::Relaxed);
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
    if case.sysno == 1069 {
        match create_probe_vmo() {
            Ok(handle) => CFG_A0.store(handle as u64, Ordering::Relaxed),
            Err(()) => {
                return FinishedCase {
                    verdict: Verdict::Fail,
                    detail: "vmo-create",
                };
            }
        }
    }
    ACTIVE.store(true, Ordering::Release);
    let created = scheduler::scheduler().create_thread_on_cpu(security_case_thread, "sectest", Some(0));
    if created.is_none() {
        ACTIVE.store(false, Ordering::Release);
        return FinishedCase {
            verdict: Verdict::Fail,
            detail: "thread",
        };
    }
    let start = crate::kernel_lowlevel::timer::get_nanoseconds();
    while !DONE.load(Ordering::Acquire) {
        yield_now();
        if crate::kernel_lowlevel::timer::get_nanoseconds().saturating_sub(start) > CASE_TIMEOUT_NS {
            ACTIVE.store(false, Ordering::Release);
            return FinishedCase {
                verdict: Verdict::Fail,
                detail: "timeout",
            };
        }
    }
    ACTIVE.store(false, Ordering::Release);
    judge(case)
}

fn judge(case: Case) -> FinishedCase {
    if SETUP_ERROR.load(Ordering::Relaxed) != 0 {
        return FinishedCase {
            verdict: Verdict::Fail,
            detail: "setup",
        };
    }
    let magic_ok = MAGIC_WORD.load(Ordering::Relaxed) == MAGIC;
    let exec_ok = EXEC_HITS.load(Ordering::Relaxed) == 0;
    let faulted = FAULTED.load(Ordering::Relaxed);
    match case.judge {
        Judge::Trap => {
            if faulted && magic_ok && exec_ok {
                FinishedCase {
                    verdict: Verdict::Pass,
                    detail: "trapped",
                }
            } else if !magic_ok {
                FinishedCase {
                    verdict: Verdict::Fail,
                    detail: "magic-changed",
                }
            } else if !exec_ok {
                FinishedCase {
                    verdict: Verdict::Fail,
                    detail: "kernel-executed",
                }
            } else {
                FinishedCase {
                    verdict: Verdict::Fail,
                    detail: "no-fault",
                }
            }
        }
        Judge::Syscall(expected) => {
            let result = LAST_RESULT.load(Ordering::Relaxed);
            if faulted {
                FinishedCase {
                    verdict: Verdict::Fail,
                    detail: "faulted",
                }
            } else if !magic_ok {
                FinishedCase {
                    verdict: Verdict::Fail,
                    detail: "magic-changed",
                }
            } else if result == expected {
                FinishedCase {
                    verdict: Verdict::Pass,
                    detail: "errno",
                }
            } else {
                FinishedCase {
                    verdict: Verdict::Fail,
                    detail: "unexpected-return",
                }
            }
        }
        Judge::NotProven(_) => FinishedCase {
            verdict: Verdict::Fail,
            detail: "not-proven",
        },
    }
}

fn create_probe_vmo() -> Result<u32, ()> {
    let mut handle = 0u32;
    crate::syscall::sys_vmo_create(4096, 0, &mut handle).map_err(|_| ())?;
    if handle == 0 {
        return Err(());
    }
    Ok(handle)
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
    unsafe {
        crate::user_level::user_process::switch_to_el0(CODE_PAGE as u64, stack_top, root);
    }
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
