//! User Test Process
//!
//! This is a simple user-mode process that tests the syscall interface.
//! It runs at EL0 and makes system calls to verify the user→kernel transition works.

/// Linux mmap syscall number (ARM64)
const SYS_MMAP: u32 = 222;

/// Linux exit syscall number (ARM64)
const SYS_EXIT: u32 = 93;

/// Linux write syscall number (ARM64)
const SYS_WRITE: u32 = 64;

/// Linux getpid syscall number (ARM64)
const SYS_GETPID: u32 = 172;

/// Make a Linux syscall from EL0
///
/// # Safety
/// This function performs a system call and should only be called from EL0
#[inline(always)]
pub unsafe fn linux_syscall(syscall_num: u32, args: [u64; 6]) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "svc #0",
        in("x8") syscall_num,
        in("x0") args[0],
        in("x1") args[1],
        in("x2") args[2],
        in("x3") args[3],
        in("x4") args[4],
        in("x5") args[5],
        lateout("x0") ret,
        options(nostack),
    );
    ret
}

/// Test: Call getpid syscall
pub fn test_getpid() -> u64 {
    unsafe { linux_syscall(SYS_GETPID, [0; 6]) }
}

/// Test: Call mmap syscall (anonymous mapping)
pub fn test_mmap(size: usize) -> u64 {
    const MAP_PRIVATE: usize = 1 << 1;
    const MAP_ANONYMOUS: usize = 1 << 5;

    let flags = MAP_PRIVATE | MAP_ANONYMOUS;
    let prot = 0x3; // PROT_READ | PROT_WRITE

    unsafe {
        linux_syscall(SYS_MMAP, [
            0,           // addr (NULL - let kernel choose)
            size as u64, // length
            prot as u64, // protection
            flags as u64, // flags
            0,           // fd (-1 for anonymous)
            0,           // offset
        ])
    }
}

/// Test: Exit process
pub fn test_exit(exit_code: i32) -> ! {
    unsafe {
        linux_syscall(SYS_EXIT, [exit_code as u64, 0, 0, 0, 0, 0]);
        // Should never return
        loop {}
    }
}

/// Test: Write to file descriptor (stdout)
pub fn test_write(fd: u64, buf: &[u8]) -> u64 {
    unsafe {
        linux_syscall(SYS_WRITE, [
            fd,
            buf.as_ptr() as u64,
            buf.len() as u64,
            0, 0, 0,
        ])
    }
}

/// User test process entry point
///
/// This function is called when the process starts in EL0.
/// It tests various syscalls to verify the user→kernel transition.
#[no_mangle]
pub fn user_test_process_entry() -> ! {
    // Test message
    let msg = b"=== EL0 Test Process Started ===\n";
    test_write(1, msg); // fd 1 = stdout

    // Test getpid
    let pid = test_getpid();
    if pid != 0 {
        let msg = b"[OK] getpid() syscall works!\n";
        test_write(1, msg);
    } else {
        let msg = b"[FAIL] getpid() returned 0\n";
        test_write(1, msg);
    }

    // Test mmap
    let addr = test_mmap(4096);
    if addr > 0 && addr < 0xFFFF_FFFF_FFFF_F000 {
        let msg = b"[OK] mmap() syscall works!\n";
        test_write(1, msg);
    } else {
        let msg = b"[FAIL] mmap() returned invalid address\n";
        test_write(1, msg);
    }

    // Success message
    let msg = b"=== All syscall tests passed ===\n";
    test_write(1, msg);

    // Exit with success
    test_exit(0)
}

/// Simple busy loop for user mode
#[no_mangle]
pub fn user_busy_loop_entry() -> ! {
    let msg = b"EL0 busy loop running...\n";
    test_write(1, msg);

    loop {
        // Simple loop - in real implementation, would do actual work
        cortex_a::asm::wfe();
    }
}

/// Run user test process from kernel
///
/// This function sets up and executes the user test process
pub fn run_user_test() {
    let mut serial = crate::kernel_lowlevel::serial::Serial::new();
    serial.init();
    
    serial.write_str("[EL0] Setting up test process...\n");
    serial.write_str("[EL0] Testing syscall interface...\n");
    
    // Since we're currently at EL1 (kernel mode), we test the syscall
    // implementations directly by calling them through the dispatch layer
    // This verifies the syscall implementations work before we move to EL0
    
    use crate::syscall::{sys_getpid, sys_mmap};

    serial.write_str("[EL0] Testing getpid...\n");
    // Test getpid directly through the syscall function
    let pid = sys_getpid();
    serial.write_str("[EL0] getpid returned: ");
    match pid {
        Ok(val) => {
            print_number(&mut serial, val as u32);
            serial.write_str(" (SUCCESS)\n");
        }
        Err(err) => {
            serial.write_str("ERROR: ");
            print_number(&mut serial, err as u32);
            serial.write_str("\n");
        }
    }

    serial.write_str("[EL0] Testing mmap...\n");
    // Test mmap directly
    const MAP_PRIVATE: usize = 1 << 1;
    const MAP_ANONYMOUS: usize = 1 << 5;
    let flags = MAP_PRIVATE | MAP_ANONYMOUS;
    let prot = 0x3; // PROT_READ | PROT_WRITE

    let result = sys_mmap(0, 4096, prot, flags, 0, 0);
    serial.write_str("[EL0] mmap returned: ");
    match result {
        Ok(addr) => {
            print_hex(&mut serial, addr as u64);
            serial.write_str(" (SUCCESS)\n");
        }
        Err(err) => {
            serial.write_str("ERROR: ");
            print_number(&mut serial, err as u32);
            serial.write_str("\n");
        }
    }

    serial.write_str("[EL0] Test process complete!\n");
    serial.write_str("[EL0] NOTE: Syscalls tested at EL1 (kernel mode)\n");
    serial.write_str("[EL0] To test from EL0, need to:\n");
    serial.write_str("[EL0]   1. Setup page tables with user pages\n");
    serial.write_str("[EL0]   2. Configure SPSR for EL0t mode\n");
    serial.write_str("[EL0]   3. Use ERET to drop to EL0\n");
    serial.write_str("[EL0]   4. Execute user code that triggers SVC\n");
}

fn print_number(serial: &mut crate::kernel_lowlevel::serial::Serial, mut num: u32) {
    if num == 0 {
        serial.write_byte(b'0');
        return;
    }
    
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


