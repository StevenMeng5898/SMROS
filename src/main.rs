#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

extern crate alloc;

use core::panic::PanicInfo;
use tock_registers::interfaces::Readable;
use core::alloc::Layout;

mod serial;
mod timer;
mod interrupt;
mod thread;
mod scheduler;
mod drivers;
mod smp;

use serial::Serial;
use scheduler::{schedule, yield_now, start_first_thread, schedule_on_cpu, yield_now_on_cpu, start_first_thread_for_cpu};
use smp::{boot_all_cpus, print_status as smp_print_status, current_cpu_id};

// Global allocator for no_std environment
struct KernelAllocator;

#[global_allocator]
static ALLOCATOR: KernelAllocator = KernelAllocator;

unsafe impl alloc::alloc::GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Simple bump allocator using a static buffer
        // For a real kernel, you'd use proper page allocation
        static mut HEAP: [u8; 0x100000] = [0; 0x100000]; // 1MB heap
        static mut HEAP_POS: usize = 0;
        
        let align = layout.align();
        let size = layout.size();
        
        // Align the current position
        let mut pos = HEAP_POS;
        let offset = pos % align;
        if offset != 0 {
            pos += align - offset;
        }
        
        if pos + size > HEAP.len() {
            return ptr::null_mut();
        }
        
        let ptr = HEAP.as_mut_ptr().add(pos);
        HEAP_POS = pos + size;
        ptr
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Simple implementation - no deallocation (memory leak)
        // For a real kernel, you'd implement proper deallocation
    }
}

#[alloc_error_handler]
fn alloc_error(_layout: Layout) -> ! {
    panic!("Allocation error");
}

use core::ptr;

// Boot assembly code and context switch
core::arch::global_asm!(
    r#"
.section .text.boot, "ax"
.globl _start

_start:
    // Check if this is the boot CPU (CPU0) or a secondary CPU
    // Read MPIDR to determine which CPU we are
    mrs     x19, mpidr_el1
    and     x19, x19, #0xFF       // Extract affinity level 0 (CPU ID)
    
    // If CPU0, continue with normal boot
    cbz     x19, 1f
    
    // Secondary CPU: jump to secondary entry point
    // Set up stack from x3 (passed from PSCI CPU_ON call)
    mov     sp, x3
    
    // Clear BSS for secondary CPU (shared with CPU0)
    // BSS is already cleared by CPU0, so skip this
    
    // Set exception vector base address
    ldr     x1, =exception_vectors
    msr     vbar_el1, x1
    
    // Branch to secondary CPU entry point
    bl      secondary_cpu_entry
    
    // Halt if returns (should never happen)
2:
    wfi
    b       2b

1:
    // Boot CPU (CPU0) continues with normal initialization
    
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
3:
    cmp     x1, x2
    b.eq    4f
    str     x3, [x1], #8
    b       3b
4:

    // Set exception vector base address
    ldr     x1, =exception_vectors
    msr     vbar_el1, x1

    // Branch to Rust kernel entry point
    bl      kernel_main

    // Halt if kernel returns (should never happen)
5:
    wfi
    b       5b

// Secondary CPU entry point - must be visible for PSCI CPU_ON
// This is called when a secondary CPU boots via PSCI
.globl secondary_entry
.type secondary_entry, %function
secondary_entry:
    // PSCI CPU_ON passes context_id in x2 (we passed stack_ptr here)
    
    // Set stack pointer from x2
    mov     sp, x2
    
    // Align stack to 16 bytes
    mov     x1, x2
    and     x1, x1, #~0xF
    mov     sp, x1
    
    // Enable FP/SIMD
    mrs     x1, cpacr_el1
    orr     x1, x1, #(0x3 << 20)
    msr     cpacr_el1, x1
    isb
    
    // Jump to Rust entry point
    b       secondary_cpu_entry
    
    // Should never reach here
6:
    wfi
    b       6b

// Exception vectors - must be 2KB aligned and each vector is 0x80 bytes
.align 11
.globl exception_vectors
exception_vectors:
    // Synchronous Exception (Current EL with SP0) - offset 0x000
    b       exception_handler
    .balign 0x80
    // IRQ (Current EL with SP0) - offset 0x080
    b       irq_handler
    .balign 0x80
    // FIQ (Current EL with SP0) - offset 0x100
    b       .
    .balign 0x80
    // SError (Current EL with SP0) - offset 0x180
    b       .
    .balign 0x80

    // Synchronous Exception (Current EL with SPx) - offset 0x200
    b       exception_handler
    .balign 0x80
    // IRQ (Current EL with SPx) - offset 0x280
    b       irq_handler_sp
    .balign 0x80
    // FIQ (Current EL with SPx) - offset 0x300
    b       .
    .balign 0x80
    // SError (Current EL with SPx) - offset 0x380
    b       .
    .balign 0x80

    // Synchronous Exception (Lower EL using AArch64) - offset 0x400
    b       exception_handler
    .balign 0x80
    // IRQ (Lower EL using AArch64) - offset 0x480
    b       irq_handler_lower
    .balign 0x80
    // FIQ (Lower EL using AArch64) - offset 0x500
    b       .
    .balign 0x80
    // SError (Lower EL using AArch64) - offset 0x580
    b       .
    .balign 0x80

    // Synchronous Exception (Lower EL using AArch32) - offset 0x600
    b       exception_handler
    .balign 0x80
    // IRQ (Lower EL using AArch32) - offset 0x680
    b       irq_handler
    .balign 0x80
    // FIQ (Lower EL using AArch32) - offset 0x700
    b       .
    .balign 0x80
    // SError (Lower EL using AArch32) - offset 0x780
    b       .
    .balign 0x80

// IRQ Handler (Current EL with SPx)
irq_handler_sp:
    // Save caller-saved registers
    sub     sp, sp, #128
    stp     x0, x1, [sp, #0]
    stp     x2, x3, [sp, #16]
    stp     x4, x5, [sp, #32]
    stp     x6, x7, [sp, #48]
    stp     x8, x9, [sp, #64]
    stp     x10, x11, [sp, #80]
    stp     x12, x13, [sp, #96]
    stp     x14, x15, [sp, #112]

    // Call timer interrupt handler
    bl      timer_interrupt_handler

    // Check if preemption is needed
    bl      check_preemption

    // Restore caller-saved registers
    ldp     x0, x1, [sp, #0]
    ldp     x2, x3, [sp, #16]
    ldp     x4, x5, [sp, #32]
    ldp     x6, x7, [sp, #48]
    ldp     x8, x9, [sp, #64]
    ldp     x10, x11, [sp, #80]
    ldp     x12, x13, [sp, #96]
    ldp     x14, x15, [sp, #112]
    add     sp, sp, #128

    eret

// IRQ Handler (Current EL with SP0)
irq_handler:
    // Save caller-saved registers
    sub     sp, sp, #128
    stp     x0, x1, [sp, #0]
    stp     x2, x3, [sp, #16]
    stp     x4, x5, [sp, #32]
    stp     x6, x7, [sp, #48]
    stp     x8, x9, [sp, #64]
    stp     x10, x11, [sp, #80]
    stp     x12, x13, [sp, #96]
    stp     x14, x15, [sp, #112]

    // Call timer interrupt handler
    bl      timer_interrupt_handler

    // Check if preemption is needed
    bl      check_preemption

    // Restore caller-saved registers
    ldp     x0, x1, [sp, #0]
    ldp     x2, x3, [sp, #16]
    ldp     x4, x5, [sp, #32]
    ldp     x6, x7, [sp, #48]
    ldp     x8, x9, [sp, #64]
    ldp     x10, x11, [sp, #80]
    ldp     x12, x13, [sp, #96]
    ldp     x14, x15, [sp, #112]
    add     sp, sp, #128

    eret

// IRQ Handler (Lower EL using AArch64)
irq_handler_lower:
    // Save caller-saved registers
    sub     sp, sp, #128
    stp     x0, x1, [sp, #0]
    stp     x2, x3, [sp, #16]
    stp     x4, x5, [sp, #32]
    stp     x6, x7, [sp, #48]
    stp     x8, x9, [sp, #64]
    stp     x10, x11, [sp, #80]
    stp     x12, x13, [sp, #96]
    stp     x14, x15, [sp, #112]

    // Call timer interrupt handler
    bl      timer_interrupt_handler

    // Check if preemption is needed
    bl      check_preemption

    // Restore caller-saved registers
    ldp     x0, x1, [sp, #0]
    ldp     x2, x3, [sp, #16]
    ldp     x4, x5, [sp, #32]
    ldp     x6, x7, [sp, #48]
    ldp     x8, x9, [sp, #64]
    ldp     x10, x11, [sp, #80]
    ldp     x12, x13, [sp, #96]
    ldp     x14, x15, [sp, #112]
    add     sp, sp, #128

    eret

// Exception Handler (placeholder)
exception_handler:
    b       .

// Context Switch Function
// Arguments: x0 = current TCB pointer, x1 = next TCB pointer
// This function saves the current thread context and restores the next thread context
.globl context_switch
.type context_switch, %function
context_switch:
    // Disable interrupts during context switch
    mrs     x16, daif
    orr     x16, x16, #0x80
    msr     daif, x16

    // Calculate context base for current thread (TCB + 0x10 for context)
    add     x16, x0, #0x10

    // Save all callee-saved registers
    // x19-x28 at offsets 0x98-0xE0 from context base
    stp     x19, x20, [x16, #0x98]
    stp     x21, x22, [x16, #0xA8]
    stp     x23, x24, [x16, #0xB8]
    stp     x25, x26, [x16, #0xC8]
    stp     x27, x28, [x16, #0xD8]

    // Save FP (x29) at offset 0xE8 and LR (x30) at offset 0xF0
    stp     x29, x30, [x16, #0xE8]

    // Save current SP at offset 0xF8
    mov     x17, sp
    str     x17, [x16, #0xF8]

    // Save the return address (PC) at offset 0x100
    str     x30, [x16, #0x100]

    // Load next thread context base
    add     x16, x1, #0x10

    // Restore SP first (at offset 0xF8)
    ldr     x17, [x16, #0xF8]
    mov     sp, x17

    // Restore callee-saved registers
    ldp     x19, x20, [x16, #0x98]
    ldp     x21, x22, [x16, #0xA8]
    ldp     x23, x24, [x16, #0xB8]
    ldp     x25, x26, [x16, #0xC8]
    ldp     x27, x28, [x16, #0xD8]

    // Restore FP and LR (at offsets 0xE8 and 0xF0)
    ldp     x29, x30, [x16, #0xE8]

    // Load the saved PC (return address) at offset 0x100
    ldr     x16, [x16, #0x100]

    // Enable interrupts
    mrs     x17, daif
    bic     x17, x17, #0x80
    msr     daif, x17

    // Branch to the saved PC
    br      x16

// Context Switch Start Function (for first thread switch only)
// Arguments: x0 = next TCB pointer
// This function jumps to the next thread without saving current context
.globl context_switch_start
.type context_switch_start, %function
context_switch_start:
    // Disable interrupts
    mrs     x16, daif
    orr     x16, x16, #0x80
    msr     daif, x16

    // Load next thread's context base (TCB + 0x10 for context)
    add     x16, x0, #0x10

    // Restore SP first (at offset 0xF8)
    ldr     x17, [x16, #0xF8]
    mov     sp, x17

    // Restore callee-saved registers (x19-x28)
    ldp     x19, x20, [x16, #0x98]
    ldp     x21, x22, [x16, #0xA8]
    ldp     x23, x24, [x16, #0xB8]
    ldp     x25, x26, [x16, #0xC8]
    ldp     x27, x28, [x16, #0xD8]

    // Restore FP (x29) at offset 0xE8 and LR (x30) at offset 0xF0
    ldp     x29, x30, [x16, #0xE8]

    // Load thread entry point from context.pc (offset 0x100)
    ldr     x16, [x16, #0x100]

    // Enable interrupts
    mrs     x17, daif
    bic     x17, x17, #0x80
    msr     daif, x17

    // Jump to thread entry
    br      x16
"#,
);

/// Kernel version
const KERNEL_VERSION: &str = "0.2.0";

/// Kernel banner
const KERNEL_BANNER: &str = r#"
*********************************************

  SMROS ARM64 Kernel with Preemptive RR Scheduler

*********************************************
  v"#;

/// Main kernel entry point
#[no_mangle]
pub extern "C" fn kernel_main() -> ! {
    // Initialize serial console
    let mut serial = Serial::new();
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

    // Initialize interrupt controller
    serial.write_str("[OK] Initializing GIC interrupt controller... ");
    interrupt::init();
    serial.write_str("done\n");

    // Initialize timer
    serial.write_str("[OK] Initializing ARM Generic Timer... ");
    timer::init();
    serial.write_str("done\n");

    serial.write_str("[INFO] Timer frequency: ");
    let freq = timer::get_frequency();
    print_number(&mut serial, (freq / 1000000) as u32);
    serial.write_str(" MHz\n");

    // Initialize SMP support
    smp::init();

    // Initialize scheduler
    serial.write_str("[OK] Initializing preemptive RR scheduler... ");
    scheduler::scheduler().init();
    serial.write_str("done\n");

    // Enable timer interrupts
    serial.write_str("[OK] Enabling timer interrupts (100Hz tick)... ");
    interrupt::enable_timer_interrupt();
    serial.write_str("done\n");

    // Unmask interrupts
    serial.write_str("[OK] Unmasking CPU interrupts... ");
    // SAFETY: Accessing DAIF register is safe in kernel mode
    unsafe {
        let daif: u64;
        core::arch::asm!(
            "mrs {daif}, daif",
            daif = out(reg) daif,
            options(nomem, nostack, preserves_flags),
        );
        // Clear I (IRQ mask) bit
        let daif = daif & !0x80;
        core::arch::asm!(
            "msr daif, {daif}",
            daif = in(reg) daif,
            options(nomem, nostack, preserves_flags),
        );
    }
    serial.write_str("done\n");

    // Boot all secondary CPUs
    serial.write_str("\n--- SMP Multi-Core Initialization ---\n");
    boot_all_cpus();
    smp_print_status();

    serial.write_str("\n--- Multi-thread SMP Sample Test ---\n");
    serial.write_str("Creating 4 worker threads with CPU affinity...\n\n");

    // Create worker threads bound to specific CPUs
    let s = scheduler::scheduler();
    s.create_thread_on_cpu(thread_worker_1, "worker-1", Some(0));
    s.create_thread_on_cpu(thread_worker_2, "worker-2", Some(1));
    s.create_thread_on_cpu(thread_worker_3, "worker-3", Some(2));
    s.create_thread_on_cpu(thread_worker_4, "worker-4", Some(3));

    serial.write_str("\n[INFO] All threads created with CPU affinity:\n");
    serial.write_str("  - worker-1 -> CPU 0\n");
    serial.write_str("  - worker-2 -> CPU 1\n");
    serial.write_str("  - worker-3 -> CPU 2\n");
    serial.write_str("  - worker-4 -> CPU 3\n\n");
    serial.write_str("[INFO] Scheduler will dispatch threads to their assigned CPUs\n\n");

    // Print initial scheduler status  
    scheduler::scheduler().print_status(&mut serial);

    // Give secondary CPUs some time to boot (they boot asynchronously)
    serial.write_str("[INFO] Starting worker threads (secondary CPUs booting in background)...\n\n");

    serial.write_str("\n[INFO] Starting first worker thread...\n");

    // Start the first worker thread (this jumps to the thread, doesn't return)
    start_first_thread();

    // If we return here, we're back in the main thread (all workers completed)
    serial.write_str("\n[INFO] All worker threads completed!\n");

    // Print final scheduler status
    scheduler::scheduler().print_status(&mut serial);

    serial.write_str("\n[INFO] Kernel is now idle (press Ctrl+A+X to exit QEMU)\n");

    // Enter idle loop
    loop {
        cortex_a::asm::wfi();
    }
}

/// Worker thread 1
extern "C" fn thread_worker_1() -> ! {
    let mut serial = Serial::new();
    // Get our assigned CPU from scheduler
    let my_cpu = scheduler::scheduler().get_thread(scheduler::scheduler().current()).map(|t| t.cpu_affinity).flatten().unwrap_or(0);

    serial.write_str("[Thread-1/CPU");
    smp::print_number(&mut serial, my_cpu as u32);
    serial.write_str("] Started!\n");

    for i in 0..3 {
        serial.write_str("[Thread-1/CPU");
        smp::print_number(&mut serial, my_cpu as u32);
        serial.write_str("] Running iteration ");
        print_number(&mut serial, i + 1);
        serial.write_str("/3\n");

        for _ in 0..100000 {
            core::hint::spin_loop();
        }

        serial.write_str("[Thread-1/CPU");
        smp::print_number(&mut serial, my_cpu as u32);
        serial.write_str("] About to yield...\n");
        yield_now();
        serial.write_str("[Thread-1/CPU");
        smp::print_number(&mut serial, my_cpu as u32);
        serial.write_str("] Returned from yield!\n");
    }

    serial.write_str("[Thread-1/CPU");
    smp::print_number(&mut serial, my_cpu as u32);
    serial.write_str("] Completed all iterations, terminating...\n");
    scheduler::scheduler().terminate_current();
    schedule();

    loop {
        cortex_a::asm::wfi();
    }
}

/// Worker thread 2
extern "C" fn thread_worker_2() -> ! {
    let mut serial = Serial::new();
    let my_cpu = scheduler::scheduler().get_thread(scheduler::scheduler().current()).map(|t| t.cpu_affinity).flatten().unwrap_or(1);

    serial.write_str("[Thread-2/CPU");
    smp::print_number(&mut serial, my_cpu as u32);
    serial.write_str("] Started!\n");

    for i in 0..3 {
        serial.write_str("[Thread-2/CPU");
        smp::print_number(&mut serial, my_cpu as u32);
        serial.write_str("] Running iteration ");
        print_number(&mut serial, i + 1);
        serial.write_str("/3\n");

        for _ in 0..100000 {
            core::hint::spin_loop();
        }

        serial.write_str("[Thread-2/CPU");
        smp::print_number(&mut serial, my_cpu as u32);
        serial.write_str("] About to yield...\n");
        yield_now();
        serial.write_str("[Thread-2/CPU");
        smp::print_number(&mut serial, my_cpu as u32);
        serial.write_str("] Returned from yield!\n");
    }

    serial.write_str("[Thread-2/CPU");
    smp::print_number(&mut serial, my_cpu as u32);
    serial.write_str("] Completed all iterations, terminating...\n");
    scheduler::scheduler().terminate_current();
    schedule();

    loop {
        cortex_a::asm::wfi();
    }
}

/// Worker thread 3
extern "C" fn thread_worker_3() -> ! {
    let mut serial = Serial::new();
    let my_cpu = scheduler::scheduler().get_thread(scheduler::scheduler().current()).map(|t| t.cpu_affinity).flatten().unwrap_or(2);

    serial.write_str("[Thread-3/CPU");
    smp::print_number(&mut serial, my_cpu as u32);
    serial.write_str("] Started!\n");

    for i in 0..3 {
        serial.write_str("[Thread-3/CPU");
        smp::print_number(&mut serial, my_cpu as u32);
        serial.write_str("] Running iteration ");
        print_number(&mut serial, i + 1);
        serial.write_str("/3\n");

        for _ in 0..100000 {
            core::hint::spin_loop();
        }

        serial.write_str("[Thread-3/CPU");
        smp::print_number(&mut serial, my_cpu as u32);
        serial.write_str("] About to yield...\n");
        yield_now();
        serial.write_str("[Thread-3/CPU");
        smp::print_number(&mut serial, my_cpu as u32);
        serial.write_str("] Returned from yield!\n");
    }

    serial.write_str("[Thread-3/CPU");
    smp::print_number(&mut serial, my_cpu as u32);
    serial.write_str("] Completed all iterations, terminating...\n");
    scheduler::scheduler().terminate_current();
    schedule();

    loop {
        cortex_a::asm::wfi();
    }
}

/// Worker thread 4
extern "C" fn thread_worker_4() -> ! {
    let mut serial = Serial::new();
    let my_cpu = scheduler::scheduler().get_thread(scheduler::scheduler().current()).map(|t| t.cpu_affinity).flatten().unwrap_or(3);

    serial.write_str("[Thread-4/CPU");
    smp::print_number(&mut serial, my_cpu as u32);
    serial.write_str("] Started!\n");

    for i in 0..3 {
        serial.write_str("[Thread-4/CPU");
        smp::print_number(&mut serial, my_cpu as u32);
        serial.write_str("] Running iteration ");
        print_number(&mut serial, i + 1);
        serial.write_str("/3\n");

        for _ in 0..100000 {
            core::hint::spin_loop();
        }

        serial.write_str("[Thread-4/CPU");
        smp::print_number(&mut serial, my_cpu as u32);
        serial.write_str("] About to yield...\n");
        yield_now();
        serial.write_str("[Thread-4/CPU");
        smp::print_number(&mut serial, my_cpu as u32);
        serial.write_str("] Returned from yield!\n");
    }

    serial.write_str("[Thread-4/CPU");
    smp::print_number(&mut serial, my_cpu as u32);
    serial.write_str("] Completed all iterations, terminating...\n");
    scheduler::scheduler().terminate_current();
    schedule();

    loop {
        cortex_a::asm::wfi();
    }
}

/// Timer interrupt handler
#[no_mangle]
extern "C" fn timer_interrupt_handler() {
    // Clear the timer interrupt
    timer::clear_interrupt();

    // Acknowledge the interrupt at GIC
    let interrupt_id = interrupt::acknowledge_interrupt();

    // Update scheduler tick count
    scheduler::scheduler().on_timer_tick();

    // End of interrupt
    interrupt::end_of_interrupt(interrupt_id);
}

/// Check if preemption is needed
#[no_mangle]
extern "C" fn check_preemption() {
    let cpu_id = current_cpu_id();
    let s = scheduler::scheduler();

    if s.should_preempt() {
        // Reset time slice for the next thread
        if let Some(next_id) = s.schedule_next_for_cpu(cpu_id as usize) {
            s.reset_time_slice(next_id);
        }

        // Perform context switch on this CPU
        schedule_on_cpu(cpu_id as usize);
    }
}

/// Print a number to serial
fn print_number(serial: &mut Serial, mut num: u32) {
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
    
    // Print in reverse order
    for j in 0..i {
        serial.write_byte(buf[i - 1 - j]);
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
    let mut serial = Serial::new();
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
