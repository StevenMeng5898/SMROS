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

    csrsi   sstatus, 0x2
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
