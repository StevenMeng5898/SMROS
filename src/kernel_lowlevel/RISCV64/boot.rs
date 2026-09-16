core::arch::global_asm!(
    r#"
.section .text.boot, "ax"
.globl _start

_start:
    // OpenSBI enters S-mode with a0=hartid and a1=FDT pointer.
    la      t0, __stack_top
    andi    t1, a0, 0xff
    slli    t1, t1, 15
    sub     sp, t0, t1
    mv      s1, a0
    mv      s0, a1

    la      t0, __bss_start
    la      t1, __bss_end
1:
    bgeu    t0, t1, 2f
    sd      zero, 0(t0)
    addi    t0, t0, 8
    j       1b
2:
    la      t0, trap_vector
    csrw    stvec, t0
    la      t0, RISCV_TRAP_STACKS
    li      t1, 0x10000
    add     t0, t0, t1
    csrw    sscratch, t0

    li      t0, (1 << 1) | (1 << 5)
    csrc    sstatus, t0
    li      t0, (1 << 5)
    csrs    sie, t0

    mv      a0, s1
    call    riscv64_record_boot_hart
    mv      a0, s0
    call    kernel_main
3:
    wfi
    j       3b

.section .text.boot, "ax"

.globl secondary_entry
.type secondary_entry, @function
secondary_entry:
    mv      sp, a1
    la      t0, trap_vector
    csrw    stvec, t0
    la      t0, RISCV_TRAP_STACKS
    li      t1, 0x10000
    add     t0, t0, t1
    csrw    sscratch, t0
    tail    secondary_cpu_entry

.align 4
.globl trap_vector
trap_vector:
    // Swap the interrupted stack with the dedicated kernel trap stack. The
    // old stack pointer is retained in sscratch until trap_restore.
    // RISCV_TRAP_FRAME_SEPC_SLOT = 30, RISCV_TRAP_FRAME_USER_SP_SLOT = 31,
    // RISCV_TRAP_FRAME_SSTATUS_SLOT = 32.
    csrrw   sp, sscratch, sp
    // 32 registers plus saved sepc, user sp, and sstatus.
    addi    sp, sp, -272
    sd      ra, 0(sp)
    sd      gp, 8(sp)
    sd      tp, 16(sp)
    sd      t0, 24(sp)
    sd      t1, 32(sp)
    sd      t2, 40(sp)
    sd      s0, 48(sp)
    sd      s1, 56(sp)
    sd      a0, 64(sp)
    sd      a1, 72(sp)
    sd      a2, 80(sp)
    sd      a3, 88(sp)
    sd      a4, 96(sp)
    sd      a5, 104(sp)
    sd      a6, 112(sp)
    sd      a7, 120(sp)
    sd      s2, 128(sp)
    sd      s3, 136(sp)
    sd      s4, 144(sp)
    sd      s5, 152(sp)
    sd      s6, 160(sp)
    sd      s7, 168(sp)
    sd      s8, 176(sp)
    sd      s9, 184(sp)
    sd      s10, 192(sp)
    sd      s11, 200(sp)
    sd      t3, 208(sp)
    sd      t4, 216(sp)
    sd      t5, 224(sp)
    sd      t6, 232(sp)
    csrr    t0, sepc
    sd      t0, 240(sp)
    csrr    t0, sscratch
    sd      t0, 248(sp)
    csrr    t0, sstatus
    sd      t0, 256(sp)
    addi    t0, sp, 272
    csrw    sscratch, t0

    csrr    t0, scause
    bltz    t0, trap_interrupt

    li      t1, 8
    beq     t0, t1, trap_user_ecall
    li      t1, 9
    beq     t0, t1, trap_supervisor_ecall
    li      t1, 12
    beq     t0, t1, trap_page_fault
    li      t1, 13
    beq     t0, t1, trap_page_fault
    li      t1, 15
    beq     t0, t1, trap_page_fault
    j       trap_unknown

trap_interrupt:
    slli    t0, t0, 1
    srli    t0, t0, 1
    li      t1, 5
    beq     t0, t1, trap_timer
    j       trap_restore

trap_timer:
    mv      a0, sp
    call    riscv64_timer_interrupt_handler
    j       trap_restore

trap_user_ecall:
trap_supervisor_ecall:
    mv      a0, sp
    call    reconcile_riscv_trap_owner_entry
    mv      a0, sp
    ld      a1, 120(sp)
    ld      a2, 64(sp)
    ld      a3, 72(sp)
    ld      a4, 80(sp)
    ld      a5, 88(sp)
    ld      a6, 96(sp)
    ld      a7, 104(sp)
    call    handle_syscall_simple
    sd      a0, 64(sp)
    // linux_riscv_syscall_context::install advances the frame's saved sepc
    // before dispatch and preserves explicit restart/return PCs. Publish that
    // return PC to the CSR before signal handling so sigreturn can restore it.
    ld      t0, 240(sp)
    csrw    sepc, t0
    mv      a0, sp
    call    complete_linux_signal_syscall_return
    call    clear_linux_riscv_syscall_context
    j       trap_restore

trap_page_fault:
    mv      a0, sp
    csrr    a1, scause
    csrr    a2, stval
    call    handle_riscv_page_fault
    bnez    a0, trap_restore
    j       trap_unknown

trap_unknown:
    csrr    a0, scause
    csrr    a1, sepc
    csrr    a2, stval
    call    riscv64_record_unhandled_trap
    li      t0, -38
    sd      t0, 64(sp)
    ld      t0, 240(sp)
    addi    t0, t0, 4
    sd      t0, 240(sp)

trap_restore:
    // Keep the frame pointer in t6 while restoring the interrupted stack
    // directly from slot 31. sscratch remains the hart's trap-stack top, so a
    // context switch while a trap is blocked cannot change the return stack.
    mv      t6, sp
    ld      sp, 248(t6)
    addi    t0, t6, 272
    csrw    sscratch, t0
    ld      t0, 240(t6)
    csrw    sepc, t0
    ld      t0, 256(t6)
    // Linux user-space and its dynamic loader use the RISC-V floating-point
    // register file. Mark it dirty before sret so valid fld/fsd instructions
    // do not re-enter the illegal-instruction path when returning from a
    // syscall, signal, or timer trap.
    li      t1, 3 << 13
    or      t0, t0, t1
    csrw    sstatus, t0
    ld      ra, 0(t6)
    ld      gp, 8(t6)
    ld      tp, 16(t6)
    ld      t0, 24(t6)
    ld      t1, 32(t6)
    ld      t2, 40(t6)
    ld      s0, 48(t6)
    ld      s1, 56(t6)
    ld      a0, 64(t6)
    ld      a1, 72(t6)
    ld      a2, 80(t6)
    ld      a3, 88(t6)
    ld      a4, 96(t6)
    ld      a5, 104(t6)
    ld      a6, 112(t6)
    ld      a7, 120(t6)
    ld      s2, 128(t6)
    ld      s3, 136(t6)
    ld      s4, 144(t6)
    ld      s5, 152(t6)
    ld      s6, 160(t6)
    ld      s7, 168(t6)
    ld      s8, 176(t6)
    ld      s9, 184(t6)
    ld      s10, 192(t6)
    ld      s11, 200(t6)
    ld      t3, 208(t6)
    ld      t4, 216(t6)
    ld      t5, 224(t6)
    ld      t6, 232(t6)
    sret
"#,
);

core::arch::global_asm!(include_str!("context_switch.S"));
