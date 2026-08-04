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
    tail    secondary_cpu_entry

.align 4
.globl trap_vector
trap_vector:
    addi    sp, sp, -256
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

    csrr    t0, scause
    bltz    t0, trap_interrupt

    li      t1, 8
    beq     t0, t1, trap_user_ecall
    li      t1, 9
    beq     t0, t1, trap_supervisor_ecall
    j       trap_unknown

trap_interrupt:
    slli    t0, t0, 1
    srli    t0, t0, 1
    li      t1, 5
    beq     t0, t1, trap_timer
    j       trap_restore

trap_timer:
    call    timer_interrupt_handler
    j       trap_restore

trap_user_ecall:
trap_supervisor_ecall:
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
    csrr    t0, sepc
    addi    t0, t0, 4
    csrw    sepc, t0
    j       trap_restore

trap_unknown:
    li      t0, -38
    sd      t0, 64(sp)
    csrr    t0, sepc
    addi    t0, t0, 4
    csrw    sepc, t0

trap_restore:
    ld      ra, 0(sp)
    ld      gp, 8(sp)
    ld      tp, 16(sp)
    ld      t0, 24(sp)
    ld      t1, 32(sp)
    ld      t2, 40(sp)
    ld      s0, 48(sp)
    ld      s1, 56(sp)
    ld      a0, 64(sp)
    ld      a1, 72(sp)
    ld      a2, 80(sp)
    ld      a3, 88(sp)
    ld      a4, 96(sp)
    ld      a5, 104(sp)
    ld      a6, 112(sp)
    ld      a7, 120(sp)
    ld      s2, 128(sp)
    ld      s3, 136(sp)
    ld      s4, 144(sp)
    ld      s5, 152(sp)
    ld      s6, 160(sp)
    ld      s7, 168(sp)
    ld      s8, 176(sp)
    ld      s9, 184(sp)
    ld      s10, 192(sp)
    ld      s11, 200(sp)
    ld      t3, 208(sp)
    ld      t4, 216(sp)
    ld      t5, 224(sp)
    ld      t6, 232(sp)
    addi    sp, sp, 256
    sret
"#,
);

core::arch::global_asm!(include_str!("context_switch.S"));
