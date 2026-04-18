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

    // Restore next thread context.
    add     x16, x1, #0x10
    ldr     x17, [x16, #0xF8]
    mov     sp, x17
    ldp     x19, x20, [x16, #0x98]
    ldp     x21, x22, [x16, #0xA8]
    ldp     x23, x24, [x16, #0xB8]
    ldp     x25, x26, [x16, #0xC8]
    ldp     x27, x28, [x16, #0xD8]
    ldp     x29, x30, [x16, #0xE8]
    ldr     x16, [x16, #0x100]

    // Re-enable IRQs before transferring control.
    mrs     x17, daif
    bic     x17, x17, #0x80
    msr     daif, x17

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
