#![no_std]
#![no_main]

extern crate alloc;

use core::alloc::Layout;
use core::cell::UnsafeCell;
use core::mem::{align_of, size_of};
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};
use tock_registers::interfaces::Readable;

mod kernel_lowlevel;
mod kernel_objects;
mod main_logic;
mod syscall;
mod user_level;

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

#[repr(C)]
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

#[repr(C)]
struct AllocationHeader {
    block_start: usize,
    block_size: usize,
}

struct KernelAllocatorState {
    initialized: bool,
    free_head: *mut FreeBlock,
}

struct AllocIrqGuard {
    daif: usize,
}

// 64 MiB heap for kernel dynamic allocations.
static HEAP: SyncUnsafeCell<[u8; main_logic::KERNEL_HEAP_SIZE]> =
    SyncUnsafeCell::new([0; main_logic::KERNEL_HEAP_SIZE]);
static ALLOC_STATE: SyncUnsafeCell<KernelAllocatorState> =
    SyncUnsafeCell::new(KernelAllocatorState {
        initialized: false,
        free_head: core::ptr::null_mut(),
    });
static ALLOC_LOCK: AtomicBool = AtomicBool::new(false);

fn allocator_align_up(value: usize, align: usize) -> Option<usize> {
    if align == 0 || !align.is_power_of_two() {
        return None;
    }
    let mask = align - 1;
    value.checked_add(mask).map(|next| next & !mask)
}

fn allocator_lock() -> AllocIrqGuard {
    let daif: usize;
    unsafe {
        core::arch::asm!("mrs {}, daif", out(reg) daif, options(nomem, nostack, preserves_flags));
        let masked = daif | 0x3c0;
        core::arch::asm!("msr daif, {}", in(reg) masked, options(nomem, nostack, preserves_flags));
    }
    while ALLOC_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    AllocIrqGuard { daif }
}

impl Drop for AllocIrqGuard {
    fn drop(&mut self) {
        ALLOC_LOCK.store(false, Ordering::Release);
        unsafe {
            core::arch::asm!("msr daif, {}", in(reg) self.daif, options(nomem, nostack, preserves_flags));
        }
    }
}

unsafe fn init_kernel_allocator(state: &mut KernelAllocatorState) {
    let heap_start = (*HEAP.get()).as_mut_ptr() as usize;
    let heap_end = heap_start + main_logic::KERNEL_HEAP_SIZE;
    let block_start = match allocator_align_up(heap_start, align_of::<FreeBlock>()) {
        Some(value) => value,
        None => {
            state.initialized = true;
            state.free_head = core::ptr::null_mut();
            return;
        }
    };
    if block_start + size_of::<FreeBlock>() > heap_end {
        state.initialized = true;
        state.free_head = core::ptr::null_mut();
        return;
    }
    let block = block_start as *mut FreeBlock;
    (*block).size = heap_end - block_start;
    (*block).next = core::ptr::null_mut();
    state.initialized = true;
    state.free_head = block;
}

unsafe fn replace_free_block(
    state: &mut KernelAllocatorState,
    prev: *mut FreeBlock,
    old: *mut FreeBlock,
    replacement: *mut FreeBlock,
) {
    if prev.is_null() {
        state.free_head = replacement;
    } else {
        (*prev).next = replacement;
    }
    let _ = old;
}

unsafe fn alloc_from_free_list(state: &mut KernelAllocatorState, layout: Layout) -> *mut u8 {
    if !state.initialized {
        init_kernel_allocator(state);
    }

    let request_size = layout.size().max(1);
    let request_align = layout.align().max(align_of::<FreeBlock>());
    let min_free = size_of::<FreeBlock>();
    let header_size = size_of::<AllocationHeader>();

    let mut prev = core::ptr::null_mut();
    let mut current = state.free_head;
    while !current.is_null() {
        let block_start = current as usize;
        let block_size = (*current).size;
        let block_end = match block_start.checked_add(block_size) {
            Some(value) => value,
            None => return core::ptr::null_mut(),
        };
        let payload_addr = match allocator_align_up(block_start + header_size, request_align) {
            Some(value) => value,
            None => return core::ptr::null_mut(),
        };
        let header_addr = payload_addr - header_size;
        let alloc_end = match payload_addr.checked_add(request_size) {
            Some(value) => value,
            None => return core::ptr::null_mut(),
        };
        if alloc_end <= block_end {
            let next = (*current).next;
            let prefix_size = header_addr - block_start;
            let has_prefix = prefix_size >= min_free;
            let alloc_start = if has_prefix { header_addr } else { block_start };
            let suffix_start = match allocator_align_up(alloc_end, align_of::<FreeBlock>()) {
                Some(value) => value,
                None => return core::ptr::null_mut(),
            };
            let suffix_size = block_end - suffix_start;
            let has_suffix = suffix_size >= min_free;
            let alloc_size = if has_suffix {
                suffix_start - alloc_start
            } else {
                block_end - alloc_start
            };

            if has_prefix {
                (*current).size = prefix_size;
                if has_suffix {
                    let suffix = suffix_start as *mut FreeBlock;
                    (*suffix).size = suffix_size;
                    (*suffix).next = next;
                    (*current).next = suffix;
                }
            } else if has_suffix {
                let suffix = suffix_start as *mut FreeBlock;
                (*suffix).size = suffix_size;
                (*suffix).next = next;
                replace_free_block(state, prev, current, suffix);
            } else {
                replace_free_block(state, prev, current, next);
            }

            let header = header_addr as *mut AllocationHeader;
            (*header).block_start = alloc_start;
            (*header).block_size = alloc_size;
            return payload_addr as *mut u8;
        }

        prev = current;
        current = (*current).next;
    }
    core::ptr::null_mut()
}

unsafe fn insert_free_block(
    state: &mut KernelAllocatorState,
    block_start: usize,
    block_size: usize,
) {
    if block_size < size_of::<FreeBlock>() {
        return;
    }

    let heap_start = (*HEAP.get()).as_mut_ptr() as usize;
    let heap_end = heap_start + main_logic::KERNEL_HEAP_SIZE;
    let block_end = match block_start.checked_add(block_size) {
        Some(value) => value,
        None => return,
    };
    if block_start < heap_start || block_end > heap_end {
        return;
    }

    let mut prev = core::ptr::null_mut();
    let mut current = state.free_head;
    while !current.is_null() && (current as usize) < block_start {
        prev = current;
        current = (*current).next;
    }

    let block = block_start as *mut FreeBlock;
    (*block).size = block_size;
    (*block).next = current;

    if prev.is_null() {
        state.free_head = block;
    } else {
        (*prev).next = block;
    }

    if !current.is_null() && block_start + (*block).size == current as usize {
        (*block).size += (*current).size;
        (*block).next = (*current).next;
    }

    if !prev.is_null() {
        let prev_end = prev as usize + (*prev).size;
        if prev_end == block_start {
            (*prev).size += (*block).size;
            (*prev).next = (*block).next;
        }
    }
}

// SAFETY: The heap buffer is exclusively managed behind a global spin lock.
// Freed allocations are returned to a coalescing free list stored inside the
// heap itself, so large temporary Vec buffers do not permanently consume heap.
unsafe impl alloc::alloc::GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _guard = allocator_lock();
        alloc_from_free_list(&mut *ALLOC_STATE.get(), layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        if ptr.is_null() {
            return;
        }
        let _guard = allocator_lock();
        let header = (ptr as usize - size_of::<AllocationHeader>()) as *const AllocationHeader;
        insert_free_block(
            &mut *ALLOC_STATE.get(),
            (*header).block_start,
            (*header).block_size,
        );
    }
}

// Boot assembly code
core::arch::global_asm!(
    r#"
.section .text.boot, "ax"
.globl _start

_start:
    // AArch64 Linux Image header. QEMU's `virt` -kernel path uses this to
    // load us at RAM_BASE + text_offset and pass the FDT pointer in x0.
    b       1f
    .word   0
    .quad   0x00200000
    .quad   __kernel_end - _start
    .quad   0
    .quad   0
    .quad   0
    .quad   0
    .word   0x644d5241
    .word   0

1:
    // Check if this is the boot CPU (CPU0) or a secondary CPU
    // Read MPIDR to determine which CPU we are
    mrs     x19, mpidr_el1
    and     x19, x19, #0xFF       // Extract affinity level 0 (CPU ID)
    
    // If CPU0, continue with normal boot
    cbz     x19, 2f
    
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
3:
    wfi
    b       3b

2:
    // Boot CPU (CPU0) continues with normal initialization
    mov     x20, x0
    
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
4:
    cmp     x1, x2
    b.eq    5f
    str     x3, [x1], #8
    b       4b
5:

    // Set exception vector base address
    ldr     x1, =exception_vectors
    msr     vbar_el1, x1

    // Enable FP/SIMD before Rust code can emit vector instructions.
    mrs     x1, cpacr_el1
    orr     x1, x1, #(0x3 << 20)
    msr     cpacr_el1, x1
    isb

    // Branch to Rust kernel entry point with the FDT pointer from x0.
    mov     x0, x20
    bl      kernel_main

    // Halt if kernel returns (should never happen)
6:
    wfi
    b       6b

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
    // Save all general-purpose registers because IRQs interrupt arbitrary code.
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

    // Call timer interrupt handler
    bl      timer_interrupt_handler

    // Restore registers
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

// IRQ Handler (Current EL with SP0)
irq_handler:
    // Save all general-purpose registers because IRQs interrupt arbitrary code.
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

    // Call timer interrupt handler
    bl      timer_interrupt_handler

    // Restore registers
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

// IRQ Handler (Lower EL using AArch64)
irq_handler_lower:
    // Save a complete EL0 register frame. Timer-based signal delivery may
    // patch x0/x30 and ELR_EL1 before returning to user code.
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

    // Call timer interrupt handler
    bl      timer_interrupt_handler

    // Give the Linux compatibility layer a chance to deliver SIGALRM.
    mov     x0, sp
    bl      deliver_linux_timer_signal_from_irq

    // Restore registers
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
pub extern "C" fn kernel_main(fdt_base: usize) -> ! {
    let _ = kernel_lowlevel::drivers::init_from_fdt(fdt_base);

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

    kernel_lowlevel::drivers::describe(&mut serial);

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

    // Install kernel-owned capability profiles before any user process exists.
    serial.write_str("[OK] Installing kernel object rights config... ");
    crate::kernel_objects::init();
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
    crate::user_level::init();
    serial.write_str("done\n");

    // Initialize scheduler
    serial.write_str("[OK] Initializing preemptive RR scheduler... ");
    crate::kernel_objects::scheduler::scheduler().init();
    serial.write_str("done\n");

    serial.write_str("[OK] Deferring bootstrap component EL0 launchers until requested\n");

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

    serial.write_str(
        "\n[INFO] Fast boot complete. Starting shell; run testsc for syscall validation.\n",
    );
    crate::user_level::user_shell::start_user_shell();
    serial.write_str("[KERNEL] Starting scheduler - jumping to shell thread...\n\n");
    crate::kernel_objects::scheduler::start_first_thread();
}

/// Timer interrupt handler
#[no_mangle]
extern "C" fn timer_interrupt_handler() {
    // Clear the timer interrupt
    kernel_lowlevel::timer::clear_interrupt();

    crate::kernel_objects::scheduler::scheduler().on_timer_tick();

    // Acknowledge the interrupt at GIC
    let interrupt_id = kernel_lowlevel::interrupt::acknowledge_interrupt();

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
