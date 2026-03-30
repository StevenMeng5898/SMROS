#![no_std]
#![no_main]
#![feature(asm_const)]
#![feature(naked_functions)]

use core::panic::PanicInfo;
use tock_registers::interfaces::Readable;

mod serial;

use serial::Serial;

// Boot assembly code
core::arch::global_asm!(
    r#"
.section .text.boot, "ax"
.globl _start

_start:
    // Mask all interrupts
    mrs     x1, daif
    orr     x1, x1, #0x3C0
    msr     daif, x1
    
    // Set stack pointer to our kernel stack
    ldr     x1, =__stack_top
    mov     sp, x1
    
    // Clear BSS section
    ldr     x1, =__bss_start
    ldr     x2, =__bss_end
    mov     x3, #0
1:
    cmp     x1, x2
    b.eq    2f
    str     x3, [x1], #8
    b       1b
2:
    
    // Branch to Rust kernel entry point
    bl      kernel_main
    
    // Halt if kernel returns (should never happen)
3:
    wfi
    b       3b

// Exception vectors (placeholder)
.align 11
.globl exception_vectors
exception_vectors:
    .rept 16
    b       .
    .endr
"#,
);

/// Kernel version
const KERNEL_VERSION: &str = "0.1.0";

/// Kernel banner
const KERNEL_BANNER: &str = r#"
*********************************************
  
  SMROS ARM64 Kernel by Steven Meng 

*********************************************  
  v"#;

/// Main kernel entry point
#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    // Initialize serial console
    let mut serial = unsafe { Serial::new() };
    serial.init();
    
    // Print kernel banner
    serial.write_str(KERNEL_BANNER);
    serial.write_str(KERNEL_VERSION);
    serial.write_str("\n\n");
    
    serial.write_str("[OK] Kernel initialized successfully!\n");
    serial.write_str("[OK] Serial console initialized\n");
    serial.write_str("[OK] ARM64 architecture detected\n");
    
    // Print system information
    print_system_info(&mut serial);
    
    serial.write_str("\n[INFO] Kernel is now idle (press Ctrl+A+X to exit QEMU)\n");
    
    // Enter idle loop
    loop {
        cortex_a::asm::wfi();
    }
}

/// Print system information
fn print_system_info(serial: &mut Serial) {
    serial.write_str("\n--- System Information ---\n");
    
    // Read and print CPU IDx
    let mpidr = cortex_a::registers::MPIDR_EL1.get();
    serial.write_str("[CPU] MPIDR_EL1: 0x");
    serial.write_hex(mpidr);
    serial.write_str("\n");
    
    // Read and print system control register
    let sctlr = cortex_a::registers::SCTLR_EL1.get();
    serial.write_str("[SYS] SCTLR_EL1: 0x");
    serial.write_hex(sctlr);
    serial.write_str("\n");
    
    serial.write_str("--------------------------\n");
}

/// Panic handler
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    let mut serial = unsafe { Serial::new() };
    serial.init();
    
    serial.write_str("\n!!! KERNEL PANIC !!!\n");
    
    if let Some(location) = info.location() {
        serial.write_str("[PANIC] In file ");
        serial.write_str(location.file());
        serial.write_str(" at line ");
        // Convert line number to string manually
        let mut num = location.line();
        let mut buf = [0u8; 16];
        let mut i = 0;
        if num == 0 {
            buf[i] = b'0';
            i += 1;
        } else {
            let mut temp = [0u8; 16];
            let mut j = 0;
            while num > 0 {
                temp[j] = b'0' + (num % 10) as u8;
                num /= 10;
                j += 1;
            }
            while j > 0 {
                j -= 1;
                buf[i] = temp[j];
                i += 1;
            }
        }
        serial.write_buf(&buf[..i]);
        serial.write_str("\n");
    }
    
    serial.write_str("\n[ERROR] System halted\n");
    
    loop {
        cortex_a::asm::wfi();
    }
}
