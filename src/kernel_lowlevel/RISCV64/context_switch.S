// Context switch assembly for RISC-V64.

.section .text

// Arguments:
//   a0 = current TCB pointer
//   a1 = next TCB pointer
//
// ThreadControlBlock layout:
//   0x10 = CpuContext base
// CpuContext layout:
//   0x00 = s0
//   0x08 = s1
//   0x10 = s2
//   0x18 = s3
//   0x20 = s4
//   0x28 = s5
//   0x30 = s6
//   0x38 = s7
//   0x40 = s8
//   0x48 = s9
//   0x50 = s10
//   0x58 = s11
//   0x60 = ra
//   0x68 = sp
//   0x70 = pc
//   0x78 = sstatus

.globl context_switch
.type context_switch, @function
context_switch:
    csrci   sstatus, 0x2

    addi    t0, a0, 0x10
    sd      s0,  0x00(t0)
    sd      s1,  0x08(t0)
    sd      s2,  0x10(t0)
    sd      s3,  0x18(t0)
    sd      s4,  0x20(t0)
    sd      s5,  0x28(t0)
    sd      s6,  0x30(t0)
    sd      s7,  0x38(t0)
    sd      s8,  0x40(t0)
    sd      s9,  0x48(t0)
    sd      s10, 0x50(t0)
    sd      s11, 0x58(t0)
    sd      ra,  0x60(t0)
    sd      sp,  0x68(t0)
    sd      ra,  0x70(t0)
    csrr    t1, sstatus
    sd      t1,  0x78(t0)

    addi    t0, a1, 0x10
    ld      sp,  0x68(t0)
    ld      s0,  0x00(t0)
    ld      s1,  0x08(t0)
    ld      s2,  0x10(t0)
    ld      s3,  0x18(t0)
    ld      s4,  0x20(t0)
    ld      s5,  0x28(t0)
    ld      s6,  0x30(t0)
    ld      s7,  0x38(t0)
    ld      s8,  0x40(t0)
    ld      s9,  0x48(t0)
    ld      s10, 0x50(t0)
    ld      s11, 0x58(t0)
    ld      ra,  0x60(t0)
    ld      t1,  0x70(t0)

    // Keep interrupts masked until schedule() restores the caller's state.
    // Enabling SIE here permits a nested trap while the caller is still
    // executing on a saved syscall trap frame.
    csrci   sstatus, 0x2
    jr      t1
.size context_switch, . - context_switch

.globl context_switch_start
.type context_switch_start, @function
context_switch_start:
    csrci   sstatus, 0x2

    addi    t0, a0, 0x10
    ld      sp,  0x68(t0)
    ld      s0,  0x00(t0)
    ld      s1,  0x08(t0)
    ld      s2,  0x10(t0)
    ld      s3,  0x18(t0)
    ld      s4,  0x20(t0)
    ld      s5,  0x28(t0)
    ld      s6,  0x30(t0)
    ld      s7,  0x38(t0)
    ld      s8,  0x40(t0)
    ld      s9,  0x48(t0)
    ld      s10, 0x50(t0)
    ld      s11, 0x58(t0)
    ld      ra,  0x60(t0)
    ld      t1,  0x70(t0)

    csrsi   sstatus, 0x2
    jr      t1
.size context_switch_start, . - context_switch_start

// Clone and fork startup images use the same layout. The clone path has
// already placed the requested TLS value in frame[2] (`tp`) before entering
// this shared restore sequence.
//
// Restore a fork-owned process startup image and return to the copied user
// context. The image layout is:
//   0x000..0x10f = 34 x u64 trap frame (a0 at 0x40)
//   0x110        = user return PC
//   0x118        = saved sstatus
//   0x120        = user stack pointer
//   0x128        = child root page-table physical address
.globl start_linux_clone_child
.type start_linux_clone_child, @function
start_linux_clone_child:
    j       start_linux_process_child
.size start_linux_clone_child, . - start_linux_clone_child

.globl start_linux_process_child
.type start_linux_process_child, @function
start_linux_process_child:
    // The child entry is invoked on a kernel scheduler stack. Keep interrupts
    // masked until the copied user frame is ready for sret.
    csrci   sstatus, 0x2
    mv      t6, a0

    // Activate the child's Sv39 root while the kernel identity mapping remains
    // available for this trampoline and its scheduler stack.
    ld      t0, 0x128(t6)
    srli    t0, t0, 12
    li      t1, 8
    slli    t1, t1, 60
    or      t0, t0, t1
    csrw    satp, t0
    sfence.vma

    // Return after the parent's ecall, in user mode with supervisor access to
    // user buffers enabled for future traps and syscall copies.
    ld      t0, 0x118(t6)
    li      t1, (1 << 8)
    not     t1, t1
    and     t0, t0, t1
    li      t1, (1 << 5) | (1 << 18) | (3 << 13)
    or      t0, t0, t1
    csrw    sstatus, t0
    ld      t0, 0x110(t6)
    csrw    sepc, t0
    ld      sp, 0x120(t6)

    // Restore every register saved by trap_vector. The final loads may use
    // t6 as their base because the address is evaluated before t6 is written.
    ld      ra,  0(t6)
    ld      gp,  8(t6)
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
.size start_linux_process_child, . - start_linux_process_child
