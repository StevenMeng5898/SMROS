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

    // If QEMU entered us at EL2, drop to EL1h before using EL1 state.
    mrs     x1, CurrentEL
    cmp     x1, #(2 << 2)
    b.ne    7f
    mov     x1, #(1 << 31)
    msr     hcr_el2, x1
    mov     x1, #3
    msr     cnthctl_el2, x1
    msr     cntvoff_el2, xzr
    msr     cptr_el2, xzr
    mrs     x1, ICC_SRE_EL2
    orr     x1, x1, #0xf
    msr     ICC_SRE_EL2, x1
    isb
    msr     sp_el1, x3
    ldr     x1, =7f
    msr     elr_el2, x1
    mov     x1, #0x3c5
    msr     spsr_el2, x1
    eret
7:
    
    // Clear BSS for secondary CPU (shared with CPU0)
    // BSS is already cleared by CPU0, so skip this
    
    // Set exception vector base address
    ldr     x1, =exception_vectors
    msr     vbar_el1, x1

    // Enable the GICv3/v4 system-register CPU interface at EL1.
    mrs     x1, ICC_SRE_EL1
    orr     x1, x1, #0x7
    msr     ICC_SRE_EL1, x1
    isb
    
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

    // QEMU `virt,virtualization=on` can enter the image at EL2. Configure the
    // minimal EL2 state needed by EL1 and return into the normal EL1 boot path.
    mrs     x2, CurrentEL
    cmp     x2, #(2 << 2)
    b.ne    8f
    mov     x2, #(1 << 31)
    msr     hcr_el2, x2
    mov     x2, #3
    msr     cnthctl_el2, x2
    msr     cntvoff_el2, xzr
    msr     cptr_el2, xzr
    mrs     x2, ICC_SRE_EL2
    orr     x2, x2, #0xf
    msr     ICC_SRE_EL2, x2
    isb
    msr     sp_el1, x1
    ldr     x2, =8f
    msr     elr_el2, x2
    mov     x2, #0x3c5
    msr     spsr_el2, x2
    eret
8:

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

    // Enable the GICv3/v4 system-register CPU interface at EL1.
    mrs     x1, ICC_SRE_EL1
    orr     x1, x1, #0x7
    msr     ICC_SRE_EL1, x1
    isb

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

    // Enable the GICv3/v4 system-register CPU interface at EL1.
    mrs     x1, ICC_SRE_EL1
    orr     x1, x1, #0x7
    msr     ICC_SRE_EL1, x1
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
    sub     sp, sp, #0x310
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
    stp     q0, q1, [sp, #0x100]
    stp     q2, q3, [sp, #0x120]
    stp     q4, q5, [sp, #0x140]
    stp     q6, q7, [sp, #0x160]
    stp     q8, q9, [sp, #0x180]
    stp     q10, q11, [sp, #0x1a0]
    stp     q12, q13, [sp, #0x1c0]
    stp     q14, q15, [sp, #0x1e0]
    stp     q16, q17, [sp, #0x200]
    stp     q18, q19, [sp, #0x220]
    stp     q20, q21, [sp, #0x240]
    stp     q22, q23, [sp, #0x260]
    stp     q24, q25, [sp, #0x280]
    stp     q26, q27, [sp, #0x2a0]
    stp     q28, q29, [sp, #0x2c0]
    stp     q30, q31, [sp, #0x2e0]
    mrs     x16, fpcr
    str     x16, [sp, #0x300]
    mrs     x16, fpsr
    str     x16, [sp, #0x308]

    // Call timer interrupt handler
    bl      timer_interrupt_handler

    // Restore registers
    ldr     x16, [sp, #0x300]
    msr     fpcr, x16
    ldr     x16, [sp, #0x308]
    msr     fpsr, x16
    ldp     q0, q1, [sp, #0x100]
    ldp     q2, q3, [sp, #0x120]
    ldp     q4, q5, [sp, #0x140]
    ldp     q6, q7, [sp, #0x160]
    ldp     q8, q9, [sp, #0x180]
    ldp     q10, q11, [sp, #0x1a0]
    ldp     q12, q13, [sp, #0x1c0]
    ldp     q14, q15, [sp, #0x1e0]
    ldp     q16, q17, [sp, #0x200]
    ldp     q18, q19, [sp, #0x220]
    ldp     q20, q21, [sp, #0x240]
    ldp     q22, q23, [sp, #0x260]
    ldp     q24, q25, [sp, #0x280]
    ldp     q26, q27, [sp, #0x2a0]
    ldp     q28, q29, [sp, #0x2c0]
    ldp     q30, q31, [sp, #0x2e0]
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
    add     sp, sp, #0x310

    eret

// IRQ Handler (Current EL with SP0)
irq_handler:
    // Save all general-purpose registers because IRQs interrupt arbitrary code.
    sub     sp, sp, #0x310
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
    stp     q0, q1, [sp, #0x100]
    stp     q2, q3, [sp, #0x120]
    stp     q4, q5, [sp, #0x140]
    stp     q6, q7, [sp, #0x160]
    stp     q8, q9, [sp, #0x180]
    stp     q10, q11, [sp, #0x1a0]
    stp     q12, q13, [sp, #0x1c0]
    stp     q14, q15, [sp, #0x1e0]
    stp     q16, q17, [sp, #0x200]
    stp     q18, q19, [sp, #0x220]
    stp     q20, q21, [sp, #0x240]
    stp     q22, q23, [sp, #0x260]
    stp     q24, q25, [sp, #0x280]
    stp     q26, q27, [sp, #0x2a0]
    stp     q28, q29, [sp, #0x2c0]
    stp     q30, q31, [sp, #0x2e0]
    mrs     x16, fpcr
    str     x16, [sp, #0x300]
    mrs     x16, fpsr
    str     x16, [sp, #0x308]

    // Call timer interrupt handler
    bl      timer_interrupt_handler

    // Restore registers
    ldr     x16, [sp, #0x300]
    msr     fpcr, x16
    ldr     x16, [sp, #0x308]
    msr     fpsr, x16
    ldp     q0, q1, [sp, #0x100]
    ldp     q2, q3, [sp, #0x120]
    ldp     q4, q5, [sp, #0x140]
    ldp     q6, q7, [sp, #0x160]
    ldp     q8, q9, [sp, #0x180]
    ldp     q10, q11, [sp, #0x1a0]
    ldp     q12, q13, [sp, #0x1c0]
    ldp     q14, q15, [sp, #0x1e0]
    ldp     q16, q17, [sp, #0x200]
    ldp     q18, q19, [sp, #0x220]
    ldp     q20, q21, [sp, #0x240]
    ldp     q22, q23, [sp, #0x260]
    ldp     q24, q25, [sp, #0x280]
    ldp     q26, q27, [sp, #0x2a0]
    ldp     q28, q29, [sp, #0x2c0]
    ldp     q30, q31, [sp, #0x2e0]
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
    add     sp, sp, #0x310

    eret

// IRQ Handler (Lower EL using AArch64)
irq_handler_lower:
    // Save a complete EL0 register frame. Timer-based signal delivery may
    // patch x0/x30 and ELR_EL1 before returning to user code.
    sub     sp, sp, #0x310
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
    stp     q0, q1, [sp, #0x100]
    stp     q2, q3, [sp, #0x120]
    stp     q4, q5, [sp, #0x140]
    stp     q6, q7, [sp, #0x160]
    stp     q8, q9, [sp, #0x180]
    stp     q10, q11, [sp, #0x1a0]
    stp     q12, q13, [sp, #0x1c0]
    stp     q14, q15, [sp, #0x1e0]
    stp     q16, q17, [sp, #0x200]
    stp     q18, q19, [sp, #0x220]
    stp     q20, q21, [sp, #0x240]
    stp     q22, q23, [sp, #0x260]
    stp     q24, q25, [sp, #0x280]
    stp     q26, q27, [sp, #0x2a0]
    stp     q28, q29, [sp, #0x2c0]
    stp     q30, q31, [sp, #0x2e0]
    mrs     x16, fpcr
    str     x16, [sp, #0x300]
    mrs     x16, fpsr
    str     x16, [sp, #0x308]

    // Call timer interrupt handler
    bl      timer_interrupt_handler

    // Give the Linux compatibility layer a chance to deliver SIGALRM.
    mov     x0, sp
    bl      deliver_linux_timer_signal_from_irq
    bl      check_preemption

    // Restore registers
    ldr     x16, [sp, #0x300]
    msr     fpcr, x16
    ldr     x16, [sp, #0x308]
    msr     fpsr, x16
    ldp     q0, q1, [sp, #0x100]
    ldp     q2, q3, [sp, #0x120]
    ldp     q4, q5, [sp, #0x140]
    ldp     q6, q7, [sp, #0x160]
    ldp     q8, q9, [sp, #0x180]
    ldp     q10, q11, [sp, #0x1a0]
    ldp     q12, q13, [sp, #0x1c0]
    ldp     q14, q15, [sp, #0x1e0]
    ldp     q16, q17, [sp, #0x200]
    ldp     q18, q19, [sp, #0x220]
    ldp     q20, q21, [sp, #0x240]
    ldp     q22, q23, [sp, #0x260]
    ldp     q24, q25, [sp, #0x280]
    ldp     q26, q27, [sp, #0x2a0]
    ldp     q28, q29, [sp, #0x2c0]
    ldp     q30, q31, [sp, #0x2e0]
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
    add     sp, sp, #0x310

    eret

// Exception Handler - handles all synchronous exceptions
exception_handler:
    // Save all general purpose registers to stack
    sub     sp, sp, #0x310
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
    stp     q0, q1, [sp, #0x100]
    stp     q2, q3, [sp, #0x120]
    stp     q4, q5, [sp, #0x140]
    stp     q6, q7, [sp, #0x160]
    stp     q8, q9, [sp, #0x180]
    stp     q10, q11, [sp, #0x1a0]
    stp     q12, q13, [sp, #0x1c0]
    stp     q14, q15, [sp, #0x1e0]
    stp     q16, q17, [sp, #0x200]
    stp     q18, q19, [sp, #0x220]
    stp     q20, q21, [sp, #0x240]
    stp     q22, q23, [sp, #0x260]
    stp     q24, q25, [sp, #0x280]
    stp     q26, q27, [sp, #0x2a0]
    stp     q28, q29, [sp, #0x2c0]
    stp     q30, q31, [sp, #0x2e0]
    mrs     x16, fpcr
    str     x16, [sp, #0x300]
    mrs     x16, fpsr
    str     x16, [sp, #0x308]

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

    // Restore a completed Linux signal frame or deliver a pending handler
    // before the saved EL0 register frame is reloaded.
    mov     x0, sp
    bl      complete_linux_signal_syscall_return
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
    ldr     x16, [sp, #0x300]
    msr     fpcr, x16
    ldr     x16, [sp, #0x308]
    msr     fpsr, x16
    ldp     q0, q1, [sp, #0x100]
    ldp     q2, q3, [sp, #0x120]
    ldp     q4, q5, [sp, #0x140]
    ldp     q6, q7, [sp, #0x160]
    ldp     q8, q9, [sp, #0x180]
    ldp     q10, q11, [sp, #0x1a0]
    ldp     q12, q13, [sp, #0x1c0]
    ldp     q14, q15, [sp, #0x1e0]
    ldp     q16, q17, [sp, #0x200]
    ldp     q18, q19, [sp, #0x220]
    ldp     q20, q21, [sp, #0x240]
    ldp     q22, q23, [sp, #0x260]
    ldp     q24, q25, [sp, #0x280]
    ldp     q26, q27, [sp, #0x2a0]
    ldp     q28, q29, [sp, #0x2c0]
    ldp     q30, q31, [sp, #0x2e0]
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
    add     sp, sp, #0x310
    eret

"#,
);

core::arch::global_asm!(include_str!("context_switch.S"));
