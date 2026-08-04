// Context switch assembly for ARM64.
// These symbols are linked into the kernel via global_asm!(include_str!(...)).

.section .text

// Arguments:
//   x0 = current TCB pointer
//   x1 = next TCB pointer
//
// ThreadControlBlock layout:
//   0x10 = CpuContext base
// CpuContext layout:
//   0x98 = x19-x20
//   0xA8 = x21-x22
//   0xB8 = x23-x24
//   0xC8 = x25-x26
//   0xD8 = x27-x28
//   0xE8 = x29
//   0xF0 = x30
//   0xF8 = sp
//   0x100 = pc
//   0x110 = sp_el0
//   0x118 = elr_el1
//   0x120 = spsr_el1
//   0x128 = tpidr_el0
//   0x130 = fpcr
//   0x138 = fpsr
//   0x140 = q0-q31

.globl context_switch
.type context_switch, %function
context_switch:
    // Disable interrupts during context switch.
    mrs     x16, daif
    orr     x16, x16, #0x80
    msr     daif, x16

    // Save current thread context.
    add     x16, x0, #0x10
    stp     x19, x20, [x16, #0x98]
    stp     x21, x22, [x16, #0xA8]
    stp     x23, x24, [x16, #0xB8]
    stp     x25, x26, [x16, #0xC8]
    stp     x27, x28, [x16, #0xD8]
    stp     x29, x30, [x16, #0xE8]
    mov     x17, sp
    str     x17, [x16, #0xF8]
    str     x30, [x16, #0x100]
    mrs     x17, sp_el0
    str     x17, [x16, #0x110]
    mrs     x17, elr_el1
    str     x17, [x16, #0x118]
    mrs     x17, spsr_el1
    str     x17, [x16, #0x120]
    mrs     x17, tpidr_el0
    str     x17, [x16, #0x128]
    mrs     x17, fpcr
    str     x17, [x16, #0x130]
    mrs     x17, fpsr
    str     x17, [x16, #0x138]
    stp     q0, q1, [x16, #0x140]
    stp     q2, q3, [x16, #0x160]
    stp     q4, q5, [x16, #0x180]
    stp     q6, q7, [x16, #0x1A0]
    stp     q8, q9, [x16, #0x1C0]
    stp     q10, q11, [x16, #0x1E0]
    stp     q12, q13, [x16, #0x200]
    stp     q14, q15, [x16, #0x220]
    stp     q16, q17, [x16, #0x240]
    stp     q18, q19, [x16, #0x260]
    stp     q20, q21, [x16, #0x280]
    stp     q22, q23, [x16, #0x2A0]
    stp     q24, q25, [x16, #0x2C0]
    stp     q26, q27, [x16, #0x2E0]
    stp     q28, q29, [x16, #0x300]
    stp     q30, q31, [x16, #0x320]

    // Restore next thread context.
    add     x16, x1, #0x10
    ldr     x17, [x16, #0x110]
    msr     sp_el0, x17
    ldr     x17, [x16, #0x118]
    msr     elr_el1, x17
    ldr     x17, [x16, #0x120]
    msr     spsr_el1, x17
    ldr     x17, [x16, #0x128]
    msr     tpidr_el0, x17
    ldr     x17, [x16, #0x130]
    msr     fpcr, x17
    ldr     x17, [x16, #0x138]
    msr     fpsr, x17
    ldp     q0, q1, [x16, #0x140]
    ldp     q2, q3, [x16, #0x160]
    ldp     q4, q5, [x16, #0x180]
    ldp     q6, q7, [x16, #0x1A0]
    ldp     q8, q9, [x16, #0x1C0]
    ldp     q10, q11, [x16, #0x1E0]
    ldp     q12, q13, [x16, #0x200]
    ldp     q14, q15, [x16, #0x220]
    ldp     q16, q17, [x16, #0x240]
    ldp     q18, q19, [x16, #0x260]
    ldp     q20, q21, [x16, #0x280]
    ldp     q22, q23, [x16, #0x2A0]
    ldp     q24, q25, [x16, #0x2C0]
    ldp     q26, q27, [x16, #0x2E0]
    ldp     q28, q29, [x16, #0x300]
    ldp     q30, q31, [x16, #0x320]
    ldr     x17, [x16, #0xF8]
    mov     sp, x17
    ldp     x19, x20, [x16, #0x98]
    ldp     x21, x22, [x16, #0xA8]
    ldp     x23, x24, [x16, #0xB8]
    ldp     x25, x26, [x16, #0xC8]
    ldp     x27, x28, [x16, #0xD8]
    ldp     x29, x30, [x16, #0xE8]
    ldr     x16, [x16, #0x100]

    // The resumed scheduler frame restores the DAIF state it captured before
    // switching. Keeping IRQs masked here also protects exception-return state.
    br      x16
.size context_switch, . - context_switch

.globl context_switch_start
.type context_switch_start, %function
context_switch_start:
    // Disable interrupts until the new context is live.
    mrs     x16, daif
    orr     x16, x16, #0x80
    msr     daif, x16

    // Restore the first thread context without saving a current one.
    add     x16, x0, #0x10
    ldr     x17, [x16, #0x110]
    msr     sp_el0, x17
    ldr     x17, [x16, #0x118]
    msr     elr_el1, x17
    ldr     x17, [x16, #0x120]
    msr     spsr_el1, x17
    ldr     x17, [x16, #0x128]
    msr     tpidr_el0, x17
    ldr     x17, [x16, #0x130]
    msr     fpcr, x17
    ldr     x17, [x16, #0x138]
    msr     fpsr, x17
    ldp     q0, q1, [x16, #0x140]
    ldp     q2, q3, [x16, #0x160]
    ldp     q4, q5, [x16, #0x180]
    ldp     q6, q7, [x16, #0x1A0]
    ldp     q8, q9, [x16, #0x1C0]
    ldp     q10, q11, [x16, #0x1E0]
    ldp     q12, q13, [x16, #0x200]
    ldp     q14, q15, [x16, #0x220]
    ldp     q16, q17, [x16, #0x240]
    ldp     q18, q19, [x16, #0x260]
    ldp     q20, q21, [x16, #0x280]
    ldp     q22, q23, [x16, #0x2A0]
    ldp     q24, q25, [x16, #0x2C0]
    ldp     q26, q27, [x16, #0x2E0]
    ldp     q28, q29, [x16, #0x300]
    ldp     q30, q31, [x16, #0x320]
    ldr     x17, [x16, #0xF8]
    mov     sp, x17
    ldp     x19, x20, [x16, #0x98]
    ldp     x21, x22, [x16, #0xA8]
    ldp     x23, x24, [x16, #0xB8]
    ldp     x25, x26, [x16, #0xC8]
    ldp     x27, x28, [x16, #0xD8]
    ldp     x29, x30, [x16, #0xE8]
    ldr     x16, [x16, #0x100]

    mrs     x17, daif
    bic     x17, x17, #0x80
    msr     daif, x17

    br      x16
.size context_switch_start, . - context_switch_start

// Ordinary newly created threads reach this path through context_switch, whose
// resumed-context path intentionally leaves IRQs masked. x19 holds the entry.
.globl thread_start_trampoline
.type thread_start_trampoline, %function
thread_start_trampoline:
    msr     daifclr, #2
    br      x19
.size thread_start_trampoline, . - thread_start_trampoline

.globl start_linux_clone_child
.type start_linux_clone_child, %function
start_linux_clone_child:
    msr     daifset, #2
    mov     x16, x0

    ldr     x17, [x16, #0x310]
    msr     sp_el0, x17
    ldr     x17, [x16, #0x318]
    msr     elr_el1, x17
    ldr     x17, [x16, #0x320]
    msr     spsr_el1, x17
    ldr     x17, [x16, #0x328]
    msr     tpidr_el0, x17
    ldr     x17, [x16, #0x300]
    msr     fpcr, x17
    ldr     x17, [x16, #0x308]
    msr     fpsr, x17

    ldp     q0, q1, [x16, #0x100]
    ldp     q2, q3, [x16, #0x120]
    ldp     q4, q5, [x16, #0x140]
    ldp     q6, q7, [x16, #0x160]
    ldp     q8, q9, [x16, #0x180]
    ldp     q10, q11, [x16, #0x1A0]
    ldp     q12, q13, [x16, #0x1C0]
    ldp     q14, q15, [x16, #0x1E0]
    ldp     q16, q17, [x16, #0x200]
    ldp     q18, q19, [x16, #0x220]
    ldp     q20, q21, [x16, #0x240]
    ldp     q22, q23, [x16, #0x260]
    ldp     q24, q25, [x16, #0x280]
    ldp     q26, q27, [x16, #0x2A0]
    ldp     q28, q29, [x16, #0x2C0]
    ldp     q30, q31, [x16, #0x2E0]
    isb

    ldp     x0, x1, [x16, #0x00]
    ldp     x2, x3, [x16, #0x10]
    ldp     x4, x5, [x16, #0x20]
    ldp     x6, x7, [x16, #0x30]
    ldp     x8, x9, [x16, #0x40]
    ldp     x10, x11, [x16, #0x50]
    ldp     x12, x13, [x16, #0x60]
    ldp     x14, x15, [x16, #0x70]
    ldr     x17, [x16, #0x88]
    ldr     x18, [x16, #0x90]
    ldp     x19, x20, [x16, #0x98]
    ldp     x21, x22, [x16, #0xA8]
    ldp     x23, x24, [x16, #0xB8]
    ldp     x25, x26, [x16, #0xC8]
    ldp     x27, x28, [x16, #0xD8]
    ldr     x29, [x16, #0xE8]
    ldr     x30, [x16, #0xF0]
    ldr     x16, [x16, #0x80]
    eret
.size start_linux_clone_child, . - start_linux_clone_child
