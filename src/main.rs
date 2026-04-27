#![no_std]
#![no_main]

extern crate alloc;

use core::alloc::Layout;
use core::cell::UnsafeCell;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicUsize, Ordering};
use tock_registers::interfaces::Readable;

mod kernel_lowlevel;
mod kernel_objects;
mod syscall;
mod user_level;

use kernel_lowlevel::memory::process_manager;
use kernel_lowlevel::serial::Serial;
use kernel_lowlevel::smp::{boot_all_cpus, current_cpu_id, print_status as smp_print_status};
use kernel_objects::scheduler::schedule_on_cpu;

/// A Sync wrapper around UnsafeCell that is safe to use as a static.
struct SyncUnsafeCell<T>(UnsafeCell<T>);
unsafe impl<T> Sync for SyncUnsafeCell<T> {}
impl<T> SyncUnsafeCell<T> {
    const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }
    fn get(&self) -> *mut T {
        self.0.get()
    }
}

// Global allocator for no_std environment
struct KernelAllocator;

#[global_allocator]
static ALLOCATOR: KernelAllocator = KernelAllocator;

// 1MB heap for the kernel bump allocator
static HEAP: SyncUnsafeCell<[u8; 0x100000]> = SyncUnsafeCell::new([0; 0x100000]);
static HEAP_POS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: This is a simple bump allocator for a kernel. The heap buffer is
// exclusively owned by the allocator. In a real kernel, you'd add proper
// synchronization or use a lock-free allocator design.
unsafe impl alloc::alloc::GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        let size = layout.size();

        // Atomically fetch and update the heap position
        let mut pos = HEAP_POS.load(Ordering::Relaxed);
        loop {
            let offset = pos % align;
            let aligned_pos = if offset != 0 {
                pos + align - offset
            } else {
                pos
            };

            if aligned_pos + size > 0x100000 {
                return core::ptr::null_mut();
            }

            match HEAP_POS.compare_exchange_weak(
                pos,
                aligned_pos + size,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    let ptr = (*HEAP.get()).as_mut_ptr().add(aligned_pos);
                    return ptr;
                }
                Err(new_pos) => pos = new_pos,
            }
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Simple implementation - no deallocation (memory leak)
        // For a real kernel, you'd implement proper deallocation
    }
}

// Boot assembly code
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
    sub     sp, sp, #144
    stp     x0, x1, [sp, #0]
    stp     x2, x3, [sp, #16]
    stp     x4, x5, [sp, #32]
    stp     x6, x7, [sp, #48]
    stp     x8, x9, [sp, #64]
    stp     x10, x11, [sp, #80]
    stp     x12, x13, [sp, #96]
    stp     x14, x15, [sp, #112]
    stp     x30, xzr, [sp, #128]

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
    ldp     x30, xzr, [sp, #128]
    add     sp, sp, #144

    eret

// IRQ Handler (Current EL with SP0)
irq_handler:
    // Save caller-saved registers
    sub     sp, sp, #144
    stp     x0, x1, [sp, #0]
    stp     x2, x3, [sp, #16]
    stp     x4, x5, [sp, #32]
    stp     x6, x7, [sp, #48]
    stp     x8, x9, [sp, #64]
    stp     x10, x11, [sp, #80]
    stp     x12, x13, [sp, #96]
    stp     x14, x15, [sp, #112]
    stp     x30, xzr, [sp, #128]

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
    ldp     x30, xzr, [sp, #128]
    add     sp, sp, #144

    eret

// IRQ Handler (Lower EL using AArch64)
irq_handler_lower:
    // Save caller-saved registers
    sub     sp, sp, #144
    stp     x0, x1, [sp, #0]
    stp     x2, x3, [sp, #16]
    stp     x4, x5, [sp, #32]
    stp     x6, x7, [sp, #48]
    stp     x8, x9, [sp, #64]
    stp     x10, x11, [sp, #80]
    stp     x12, x13, [sp, #96]
    stp     x14, x15, [sp, #112]
    stp     x30, xzr, [sp, #128]

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
    ldp     x30, xzr, [sp, #128]
    add     sp, sp, #144

    eret

// Exception Handler - handles all synchronous exceptions
exception_handler:
    // Save all general purpose registers to stack
    sub     sp, sp, #256
    stp     x0, x1, [sp, #0]
    stp     x2, x3, [sp, #16]
    stp     x4, x5, [sp, #32]
    stp     x6, x7, [sp, #48]
    stp     x8, x9, [sp, #64]
    stp     x10, x11, [sp, #80]
    stp     x12, x13, [sp, #96]
    stp     x14, x15, [sp, #112]
    stp     x16, x17, [sp, #128]
    stp     x18, x19, [sp, #144]
    stp     x20, x21, [sp, #160]
    stp     x22, x23, [sp, #176]
    stp     x24, x25, [sp, #192]
    stp     x26, x27, [sp, #208]
    stp     x28, x29, [sp, #224]
    stp     x30, xzr, [sp, #240]

    // Read exception class from ESR_EL1
    mrs     x0, esr_el1
    ubfx    x0, x0, #26, #6  // Extract EC field (bits 31:26)
    
    // EC = 0x15 for SVC from AArch64
    cmp     x0, #0x15
    b.ne    99f // Not SVC, jump to error handler
    
    // This is SVC exception - handle syscall
    // Load syscall number from x8 (saved at sp+64)
    ldr     x0, [sp, #64]
    
    // Load syscall arguments from saved registers
    ldp     x1, x2, [sp, #0]    // x0, x1 -> arg0, arg1
    ldp     x3, x4, [sp, #16]   // x2, x3 -> arg2, arg3
    ldp     x5, x6, [sp, #32]   // x4, x5 -> arg4, arg5
    
    // Call Rust syscall handler
    // Arguments: x0=syscall_num, x1-x6=args
    bl      handle_syscall_simple

    // Save result back to x0 position on stack
    str     x0, [sp, #0]
    b       3f
    
99:
    // General exception - return error
    mov     x0, #-38  // ENOSYS
    str     x0, [sp, #0]
    
3:
    // On AArch64 SVC, ELR_EL1 already points at the next instruction. Keep
    // the hook so tests can override the behavior if needed, but do not
    // advance by default.
    bl      syscall_should_advance_elr
    cbz     x0, 5f
    mrs     x0, elr_el1
    add     x0, x0, #4
    msr     elr_el1, x0

5:  // Restore registers and return
    ldp     x0, x1, [sp, #0]
    ldp     x2, x3, [sp, #16]
    ldp     x4, x5, [sp, #32]
    ldp     x6, x7, [sp, #48]
    ldp     x8, x9, [sp, #64]
    ldp     x10, x11, [sp, #80]
    ldp     x12, x13, [sp, #96]
    ldp     x14, x15, [sp, #112]
    ldp     x16, x17, [sp, #128]
    ldp     x18, x19, [sp, #144]
    ldp     x20, x21, [sp, #160]
    ldp     x22, x23, [sp, #176]
    ldp     x24, x25, [sp, #192]
    ldp     x26, x27, [sp, #208]
    ldp     x28, x29, [sp, #224]
    ldp     x30, xzr, [sp, #240]
    add     sp, sp, #256
    eret

"#,
);

core::arch::global_asm!(include_str!("kernel_lowlevel/context_switch.S"));

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
    kernel_lowlevel::interrupt::init();
    serial.write_str("done\n");

    // Initialize timer
    serial.write_str("[OK] Initializing ARM Generic Timer... ");
    kernel_lowlevel::timer::init();
    serial.write_str("done\n");

    serial.write_str("[INFO] Timer frequency: ");
    let freq = kernel_lowlevel::timer::get_frequency();
    print_number(&mut serial, (freq / 1000000) as u32);
    serial.write_str(" MHz\n");

    // Initialize SMP support
    kernel_lowlevel::smp::init();

    // Initialize memory management
    serial.write_str("[OK] Initializing memory management... ");
    kernel_lowlevel::memory::init();
    serial.write_str("done\n");

    // Initialize syscall interface
    serial.write_str("[OK] Initializing syscall interface... ");
    crate::syscall::init();
    serial.write_str("done\n");

    // Initialize MMU
    serial.write_str("[OK] Initializing MMU... ");
    kernel_lowlevel::mmu::init();
    serial.write_str("done\n");

    // Initialize syscall handler
    serial.write_str("[OK] Initializing syscall handler... ");
    crate::syscall::init();
    serial.write_str("done\n");

    // Initialize channel subsystem
    serial.write_str("[OK] Initializing channel subsystem... ");
    crate::kernel_objects::channel::init();
    serial.write_str("done\n");

    // Initialize user-level process management
    serial.write_str("[OK] Initializing user-level process management... ");
    crate::user_level::user_process::init();
    serial.write_str("done\n");

    // Initialize scheduler
    serial.write_str("[OK] Initializing preemptive RR scheduler... ");
    crate::kernel_objects::scheduler::scheduler().init();
    serial.write_str("done\n");

    // Enable timer interrupts
    serial.write_str("[OK] Enabling timer interrupts (100Hz tick)... ");
    kernel_lowlevel::interrupt::enable_timer_interrupt();
    serial.write_str("done\n");

    // Unmask interrupts
    serial.write_str("[OK] Unmasking CPU interrupts... ");
    // SAFETY: Reading and writing DAIF is safe in kernel mode. We only clear
    // the IRQ mask bit, leaving other flags intact.
    let daif: u64;
    unsafe {
        core::arch::asm!(
            "mrs {daif}, daif",
            daif = out(reg) daif,
            options(nomem, nostack, preserves_flags),
        );
    }
    // Clear I (IRQ mask) bit
    let daif = daif & !0x80;
    unsafe {
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

    serial.write_str("\n--- Multi-Process Memory Management ---\n");
    serial.write_str("Creating sample processes for demonstration...\n\n");

    // Create sample processes for shell demo
    let pm = process_manager();
    pm.create_process("shell");
    pm.create_process("editor");
    pm.create_process("compiler");

    serial.write_str("[INFO] Created 3 sample processes:\n");
    serial.write_str("  - shell (PID 2)\n");
    serial.write_str("  - editor (PID 3)\n");
    serial.write_str("  - compiler (PID 4)\n\n");

    // Print process and memory status
    pm.print_status(&mut serial);

    serial.write_str("\n[INFO] Boot complete! Starting user test process...\n");

    // Run the syscall validation by dropping into EL0. This function
    // continues the boot flow itself after the EL0 test exits.
    crate::user_level::user_test::run_user_test();
}

/// Timer interrupt handler
#[no_mangle]
extern "C" fn timer_interrupt_handler() {
    // Clear the timer interrupt
    kernel_lowlevel::timer::clear_interrupt();

    // Acknowledge the interrupt at GIC
    let interrupt_id = kernel_lowlevel::interrupt::acknowledge_interrupt();

    // Update scheduler tick count (decrements time_slice)
    crate::kernel_objects::scheduler::scheduler().on_timer_tick();

    // Check if preemption is needed (time_slice expired)
    check_preemption();

    // End of interrupt
    kernel_lowlevel::interrupt::end_of_interrupt(interrupt_id);
}

/// Check if preemption is needed
#[no_mangle]
extern "C" fn check_preemption() {
    let cpu_id = current_cpu_id();
    let s = crate::kernel_objects::scheduler::scheduler();

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
