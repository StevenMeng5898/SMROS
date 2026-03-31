// Context Switch Assembly for ARM64
// This file provides the low-level context switching functionality

.section .text
.globl context_switch
.type context_switch, %function

/**
 * context_switch(current: *mut ThreadControlBlock, next: *mut ThreadControlBlock)
 * 
 * Arguments:
 *   x0 = current thread TCB pointer
 *   x1 = next thread TCB pointer
 * 
 * TCB layout (offsets in bytes):
 *   0x00: id (ThreadId)
 *   0x08: state (ThreadState)
 *   0x09: padding
 *   0x10: context (CpuContext) - starts here
 *   ... context fields ...
 *   0x110: stack pointer
 *   0x118: stack_size
 *   0x120: entry
 *   0x128: time_slice
 *   0x12C: total_ticks
 *   0x130: name
 * 
 * CpuContext layout (64-bit registers, 8 bytes each):
 *   0x00-0xE0: x0-x28 (29 registers * 8 = 232 bytes)
 *   0xE8: x29 (FP)
 *   0xF0: lr (x30)
 *   0xF8: sp
 *   0x100: pc
 *   0x108: pstate
 */

// Context offset within TCB
#define CONTEXT_OFFSET 0x10

// Register offsets within CpuContext
#define X0_OFFSET   0x00
#define X19_OFFSET  0x98  // x19 is at index 19
#define X29_OFFSET  0xE8  // FP
#define LR_OFFSET   0xF0  // x30
#define SP_OFFSET   0xF8
#define PC_OFFSET   0x100
#define PSTATE_OFFSET 0x108

context_switch:
    // Disable interrupts during context switch
    mrs     x16, daif
    orr     x16, x16, #0x80
    msr     daif, x16
    
    // Save current thread's callee-saved registers (x19-x28)
    add     x16, x0, #CONTEXT_OFFSET
    stp     x19, x20, [x16, #0x98]
    stp     x21, x22, [x16, #0xA8]
    stp     x23, x24, [x16, #0xB8]
    stp     x25, x26, [x16, #0xC8]
    stp     x27, x28, [x16, #0xD8]
    
    // Save FP (x29) and LR (x30)
    stp     x29, x30, [x16, #0xE8]
    
    // Save current SP
    mov     x17, sp
    str     x17, [x16, #SP_OFFSET]
    
    // We don't save PC explicitly - it will be restored by the return
    // We don't save PSTATE - it's preserved by the CPU
    
    // Load next thread's context
    add     x16, x1, #CONTEXT_OFFSET
    
    // Restore next thread's callee-saved registers (x19-x28)
    ldp     x19, x20, [x16, #0x98]
    ldp     x21, x22, [x16, #0xA8]
    ldp     x23, x24, [x16, #0xB8]
    ldp     x25, x26, [x16, #0xC8]
    ldp     x27, x28, [x16, #0xD8]
    
    // Restore next thread's FP and LR
    ldp     x29, x30, [x16, #0xE8]
    
    // Restore next thread's SP
    ldr     x17, [x16, #SP_OFFSET]
    mov     sp, x17
    
    // Restore PSTATE (if saved)
    // ldr     x16, [x16, #PSTATE_OFFSET]
    // msr     daif, x16
    
    // Enable interrupts
    mrs     x16, daif
    bic     x16, x16, #0x80
    msr     daif, x16
    
    // Return to the next thread's context
    // The PC will be set to the value in LR (which contains the thread's entry point)
    ret

.size context_switch, . - context_switch
