//! User Test Process
//!
//! This module now performs a real EL0 -> EL1 -> EL0 syscall smoke test
//! during boot. The test drops into EL0, issues Linux-style syscalls using
//! `svc #0`, and returns to EL1 through `exit`.

use alloc::alloc::{alloc, Layout};
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

use crate::user_level::user_logic;

/// Linux write syscall number (ARM64)
const SYS_WRITE: u32 = 64;
/// Linux exit syscall number (ARM64)
const SYS_EXIT: u32 = 93;
/// Linux getpid syscall number (ARM64)
const SYS_GETPID: u32 = 172;
/// Linux mmap syscall number (ARM64)
const SYS_MMAP: u32 = 222;

/// Dedicated EL0 smoke-test stack size.
const EL0_TEST_STACK_SIZE: usize = 0x2000;
const EL0_TEST_BANNER: &[u8] = b"=== EL0 Test Process Started ===\n";
const EL0_TEST_COMPLETE: &[u8] = b"=== EL0 syscall tests complete ===\n";
const EL0_TEST_INFO_GETPID: &[u8] = b"[INFO] EL0 issued getpid()\n";
const EL0_TEST_INFO_MMAP: &[u8] = b"[INFO] EL0 issued mmap()\n";
const EL0_TEST_EXIT_OK: i32 = 0;
const EL0_TEST_EXIT_WRITE_RESULT_MISMATCH: i32 = 10;
const EL0_TEST_EXIT_GETPID_RESULT_MISMATCH: i32 = 11;
const EL0_TEST_EXIT_GETPID_LOG_WRITE_MISMATCH: i32 = 12;
const EL0_TEST_EXIT_MMAP_RESULT_MISMATCH: i32 = 13;
const EL0_TEST_EXIT_MMAP_LOG_WRITE_MISMATCH: i32 = 14;
const EL0_TEST_EXIT_COMPLETE_LOG_WRITE_MISMATCH: i32 = 15;
static EL0_TEST_ACTIVE: AtomicBool = AtomicBool::new(false);
static EL0_TEST_EXIT_CODE: AtomicI32 = AtomicI32::new(-1);
static EL0_TEST_RESUME_ADDR: AtomicU64 = AtomicU64::new(0);
static EL0_TEST_KERNEL_ENTERED: AtomicBool = AtomicBool::new(false);
static EL0_TEST_KERNEL_FINISHED: AtomicBool = AtomicBool::new(false);
static EL0_TEST_KERNEL_WRITE_RESULT: AtomicU64 = AtomicU64::new(0);
static EL0_TEST_KERNEL_PID: AtomicU64 = AtomicU64::new(0);
static EL0_TEST_KERNEL_MMAP_ADDR: AtomicU64 = AtomicU64::new(0);

/// Make a Linux syscall from EL0.
#[inline(always)]
pub unsafe fn linux_syscall(syscall_num: u32, args: [u64; 6]) -> u64 {
    let mut ret = args[0];
    core::arch::asm!(
        "svc #0",
        in("x8") syscall_num,
        inlateout("x0") ret,
        in("x1") args[1],
        in("x2") args[2],
        in("x3") args[3],
        in("x4") args[4],
        in("x5") args[5],
        options(nostack),
    );
    ret
}

pub fn test_getpid() -> u64 {
    unsafe { linux_syscall(SYS_GETPID, [0; 6]) }
}

pub fn test_mmap(size: usize) -> u64 {
    const MAP_PRIVATE: usize = 1 << 1;
    const MAP_ANONYMOUS: usize = 1 << 5;

    let flags = MAP_PRIVATE | MAP_ANONYMOUS;
    let prot = 0x3; // PROT_READ | PROT_WRITE

    unsafe { linux_syscall(SYS_MMAP, [0, size as u64, prot as u64, flags as u64, 0, 0]) }
}

pub fn test_exit(exit_code: i32) -> ! {
    unsafe {
        linux_syscall(SYS_EXIT, [exit_code as u64, 0, 0, 0, 0, 0]);
    }

    loop {
        cortex_a::asm::wfe();
    }
}

pub fn test_write(fd: u64, buf: &[u8]) -> u64 {
    unsafe {
        linux_syscall(
            SYS_WRITE,
            [fd, buf.as_ptr() as u64, buf.len() as u64, 0, 0, 0],
        )
    }
}

#[no_mangle]
pub fn user_test_process_entry() -> ! {
    let write_result = test_write(1, EL0_TEST_BANNER);
    if write_result != EL0_TEST_BANNER.len() as u64 {
        test_exit(EL0_TEST_EXIT_WRITE_RESULT_MISMATCH);
    }

    let pid = test_getpid();
    if pid != 1 {
        test_exit(EL0_TEST_EXIT_GETPID_RESULT_MISMATCH);
    }
    if test_write(1, EL0_TEST_INFO_GETPID) != EL0_TEST_INFO_GETPID.len() as u64 {
        test_exit(EL0_TEST_EXIT_GETPID_LOG_WRITE_MISMATCH);
    }

    let addr = test_mmap(4096);
    if !user_logic::mmap_result_ok(addr) {
        test_exit(EL0_TEST_EXIT_MMAP_RESULT_MISMATCH);
    }
    if test_write(1, EL0_TEST_INFO_MMAP) != EL0_TEST_INFO_MMAP.len() as u64 {
        test_exit(EL0_TEST_EXIT_MMAP_LOG_WRITE_MISMATCH);
    }

    if test_write(1, EL0_TEST_COMPLETE) != EL0_TEST_COMPLETE.len() as u64 {
        test_exit(EL0_TEST_EXIT_COMPLETE_LOG_WRITE_MISMATCH);
    }
    test_exit(EL0_TEST_EXIT_OK)
}

pub fn record_el0_kernel_syscall_result(syscall_num: u32, result: u64) {
    if !EL0_TEST_ACTIVE.load(Ordering::SeqCst) {
        return;
    }

    EL0_TEST_KERNEL_ENTERED.store(true, Ordering::SeqCst);

    match syscall_num {
        SYS_WRITE => {
            let _ = EL0_TEST_KERNEL_WRITE_RESULT.compare_exchange(
                0,
                result,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
        }
        SYS_GETPID => {
            EL0_TEST_KERNEL_PID.store(result, Ordering::SeqCst);
        }
        SYS_MMAP => {
            EL0_TEST_KERNEL_MMAP_ADDR.store(result, Ordering::SeqCst);
        }
        _ => {}
    }
}

/// Prepare an EL0 test `exit` to return into EL1 boot code instead of
/// resuming EL0 user mode.
pub fn prepare_el0_test_kernel_return(exit_code: i32) -> bool {
    if !EL0_TEST_ACTIVE.swap(false, Ordering::SeqCst) {
        return false;
    }

    let resume_addr = EL0_TEST_RESUME_ADDR.load(Ordering::SeqCst);
    if resume_addr == 0 {
        return false;
    }

    EL0_TEST_EXIT_CODE.store(exit_code, Ordering::SeqCst);
    EL0_TEST_KERNEL_FINISHED.store(true, Ordering::SeqCst);

    // Return to EL1h with interrupts masked, matching the kernel thread model.
    let spsr_el1: u64 = user_logic::el1h_spsr_masked();
    unsafe {
        core::arch::asm!(
            "msr elr_el1, {resume}",
            "msr spsr_el1, {spsr}",
            resume = in(reg) resume_addr,
            spsr = in(reg) spsr_el1,
            options(nostack),
        );
    }

    true
}

/// Called by the active exception vector to decide whether it should add 4 to
/// `ELR_EL1` before returning from an exception.
#[no_mangle]
pub extern "C" fn syscall_should_advance_elr() -> u64 {
    user_logic::syscall_should_advance_elr()
}

fn finish_boot_after_user_test() -> ! {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();

    serial.write_str("\n[INFO] User test complete! Starting user shell...\n");
    crate::user_level::user_shell::start_user_shell();
    serial.write_str("[KERNEL] Starting scheduler - jumping to shell thread...\n\n");
    crate::kernel_objects::scheduler::start_first_thread();
}

#[no_mangle]
pub extern "C" fn el0_test_resume() -> ! {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();

    let exit_code = EL0_TEST_EXIT_CODE.load(Ordering::SeqCst);
    let kernel_entered = EL0_TEST_KERNEL_ENTERED.load(Ordering::SeqCst);
    let kernel_finished = EL0_TEST_KERNEL_FINISHED.load(Ordering::SeqCst);
    let kernel_write = EL0_TEST_KERNEL_WRITE_RESULT.load(Ordering::SeqCst);
    let kernel_pid = EL0_TEST_KERNEL_PID.load(Ordering::SeqCst);
    let kernel_mmap = EL0_TEST_KERNEL_MMAP_ADDR.load(Ordering::SeqCst);

    serial.write_str("[EL0] Returned to EL1 after real EL0 syscall test\n");
    serial.write_str("[EL0] exit code: ");
    print_number(&mut serial, exit_code as u32);
    serial.write_str("\n");
    serial.write_str("[EL0] exit meaning: ");
    serial.write_str(el0_test_exit_meaning(exit_code));
    serial.write_str("\n");

    serial.write_str("[EL0] Kernel-observed write() returned: ");
    print_number(&mut serial, kernel_write as u32);
    serial.write_str("\n");

    serial.write_str("[EL0] Kernel-observed getpid() returned: ");
    print_number(&mut serial, kernel_pid as u32);
    serial.write_str("\n");

    serial.write_str("[EL0] Kernel-observed mmap() returned: ");
    print_hex(&mut serial, kernel_mmap);
    serial.write_str("\n");

    serial.write_str("[EL0] Kernel observed entry/exit: ");
    serial.write_str(if kernel_entered && kernel_finished {
        "yes"
    } else {
        "no"
    });
    serial.write_str("\n");

    let kernel_success = user_logic::kernel_success(
        kernel_entered,
        kernel_finished,
        exit_code,
        kernel_write,
        kernel_pid,
        kernel_mmap,
        EL0_TEST_BANNER.len(),
    );

    if kernel_success {
        serial.write_str("[EL0] Real EL0 -> SVC -> EL1 validation: SUCCESS\n");
    } else {
        serial.write_str("[EL0] Real EL0 -> SVC -> EL1 validation: FAIL\n");
    }

    finish_boot_after_user_test()
}

/// Run the boot-time user test by actually dropping into EL0.
pub fn run_user_test() -> ! {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();

    serial.write_str("[EL0] Setting up real EL0 test process...\n");
    serial.write_str("[EL0] Dropping to EL0 and validating the active SVC path...\n");

    EL0_TEST_EXIT_CODE.store(-1, Ordering::SeqCst);
    EL0_TEST_RESUME_ADDR.store(el0_test_resume as *const () as u64, Ordering::SeqCst);
    EL0_TEST_KERNEL_ENTERED.store(false, Ordering::SeqCst);
    EL0_TEST_KERNEL_FINISHED.store(false, Ordering::SeqCst);
    EL0_TEST_KERNEL_WRITE_RESULT.store(0, Ordering::SeqCst);
    EL0_TEST_KERNEL_PID.store(0, Ordering::SeqCst);
    EL0_TEST_KERNEL_MMAP_ADDR.store(0, Ordering::SeqCst);

    let layout = match Layout::from_size_align(EL0_TEST_STACK_SIZE, 16) {
        Ok(layout) => layout,
        Err(_) => {
            serial.write_str("[EL0] Failed to build EL0 stack layout\n");
            finish_boot_after_user_test();
        }
    };

    let stack = unsafe { alloc(layout) };
    if stack.is_null() {
        serial.write_str("[EL0] Failed to allocate EL0 stack\n");
        finish_boot_after_user_test();
    }

    let stack_top = match user_logic::stack_top_u64(stack as u64, EL0_TEST_STACK_SIZE) {
        Some(stack_top) => stack_top,
        None => {
            serial.write_str("[EL0] EL0 stack top overflow\n");
            finish_boot_after_user_test();
        }
    };
    EL0_TEST_ACTIVE.store(true, Ordering::SeqCst);

    unsafe {
        crate::user_level::user_process::switch_to_el0(
            user_test_process_entry as *const () as u64,
            stack_top,
            0,
        );
    }
}

fn el0_test_exit_meaning(exit_code: i32) -> &'static str {
    match exit_code {
        EL0_TEST_EXIT_OK => "all EL0-observed syscall return values matched expectations",
        EL0_TEST_EXIT_WRITE_RESULT_MISMATCH => "banner write() returned an unexpected value",
        EL0_TEST_EXIT_GETPID_RESULT_MISMATCH => "getpid() returned an unexpected value",
        EL0_TEST_EXIT_GETPID_LOG_WRITE_MISMATCH => {
            "post-getpid write() returned an unexpected value"
        }
        EL0_TEST_EXIT_MMAP_RESULT_MISMATCH => "mmap() returned an unexpected value",
        EL0_TEST_EXIT_MMAP_LOG_WRITE_MISMATCH => "post-mmap write() returned an unexpected value",
        EL0_TEST_EXIT_COMPLETE_LOG_WRITE_MISMATCH => {
            "final completion write() returned an unexpected value"
        }
        _ => "unexpected exit code",
    }
}

fn print_number(serial: &mut crate::kernel_lowlevel::serial::Serial, num: u32) {
    if num == 0 {
        serial.write_byte(b'0');
        return;
    }

    let signed = num as i32;
    if signed < 0 {
        serial.write_byte(b'-');
        let magnitude = (-(signed as i64)) as u32;
        print_unsigned(serial, magnitude);
        return;
    }

    print_unsigned(serial, num);
}

fn print_unsigned(serial: &mut crate::kernel_lowlevel::serial::Serial, mut num: u32) {
    let mut buf = [0u8; 10];
    let mut i = 0;

    while num > 0 && i < 10 {
        buf[i] = b'0' + (num % 10) as u8;
        num /= 10;
        i += 1;
    }

    for j in 0..i {
        serial.write_byte(buf[i - 1 - j]);
    }
}

fn print_hex(serial: &mut crate::kernel_lowlevel::serial::Serial, num: u64) {
    if num == 0 {
        serial.write_str("0x0");
        return;
    }

    serial.write_str("0x");
    let hex_chars = b"0123456789abcdef";
    let mut buf = [0u8; 16];
    let mut i = 0;
    let mut temp = num;

    while temp > 0 && i < 16 {
        buf[i] = hex_chars[(temp & 0xF) as usize];
        temp >>= 4;
        i += 1;
    }

    for j in (0..i).rev() {
        serial.write_byte(buf[j]);
    }
}
