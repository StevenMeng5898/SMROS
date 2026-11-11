/* Fixed-VA EL0 probe. Data page is 0x12000000. Code page is 0x12001000.
 *
 * Data layout (little-endian u64):
 *   0  op: 1 read, 2 write, 3 execute, 4 syscall, 5 undef,
 *      6 mprotect-after-store, 7 cow-fork, 8 unaligned ldxr,
 *      9 exit-group while another thread is in futex_wait,
 *      10 wait twice, 11 fork again after the first child exits,
 *      12 SIGSEGV handler that is not an executable user page
 *   8  target
 *  16  value
 *  24  syscall number
 *  32  arg0
 *  40  arg1
 *  48  arg2
 *  56  result
 *  64  arg3
 *  72  arg4
 *  80  arg5
 *  88  wait status (cow-fork)
 * 256  futex word
 * 512  wait4 status
 * 768  rt_sigaction
 */
.section .text
.globl el0_probe_entry
el0_probe_entry:
    movz x20, #0
    movk x20, #0x1200, lsl #16
    ldr x21, [x20]
    ldr x22, [x20, #8]
    cmp x21, #1
    b.eq do_read
    cmp x21, #2
    b.eq do_write
    cmp x21, #3
    b.eq do_exec
    cmp x21, #4
    b.eq do_svc
    cmp x21, #5
    b.eq do_undef
    cmp x21, #6
    b.eq do_mprotect_store
    cmp x21, #7
    b.eq do_cow
    cmp x21, #8
    b.eq do_align
    cmp x21, #9
    b.eq do_exit_futex
    cmp x21, #10
    b.eq do_wait_once
    cmp x21, #11
    b.eq do_exit_refork
    cmp x21, #12
    b.eq do_bad_handler
    mov x0, #99
    b exit_now
do_read:
    ldr x0, [x22]
    str x0, [x20, #56]
    b exit_now
do_write:
    ldr x0, [x20, #16]
    str x0, [x22]
    mov x0, #0
    b exit_now
do_exec:
    br x22
do_svc:
    ldr x8, [x20, #24]
    ldr x0, [x20, #32]
    ldr x1, [x20, #40]
    ldr x2, [x20, #48]
    ldr x3, [x20, #64]
    ldr x4, [x20, #72]
    ldr x5, [x20, #80]
    svc #0
    movz x20, #0
    movk x20, #0x1200, lsl #16
    str x0, [x20, #56]
    mov x0, #0
    b exit_now
do_undef:
    .inst 0x00000000
do_mprotect_store:
    ldr x0, [x20, #16]
    str x0, [x22]
    mov x0, x22
    movz x1, #0x1000
    mov x2, #1
    movz x8, #226
    svc #0
    movz x20, #0
    movk x20, #0x1200, lsl #16
    ldr x22, [x20, #8]
    ldr x0, [x20, #64]
    str x0, [x22]
    mov x0, #0
    b exit_now
do_cow:
    mov x0, #17
    mov x1, #0
    mov x2, #0
    mov x3, #0
    mov x4, #0
    movz x8, #220
    svc #0
    movz x20, #0
    movk x20, #0x1200, lsl #16
    ldr x22, [x20, #8]
    tbnz x0, #63, cow_fail
    cbz x0, cow_child
    movn x0, #0
    movz x1, #88
    add x1, x20, x1
    mov x2, #0
    mov x3, #0
    movz x8, #260
    svc #0
    movz x20, #0
    movk x20, #0x1200, lsl #16
    ldr x22, [x20, #8]
    tbnz x0, #63, cow_fail
    cbz x0, cow_fail
    ldr w0, [x20, #88]
    movz w1, #0x2a00
    cmp w0, w1
    b.ne cow_fail
    ldr x0, [x22]
    ldr x1, [x20, #32]
    cmp x0, x1
    b.ne cow_leaked
    mov x0, #1
    b exit_now
cow_child:
    ldr x0, [x20, #16]
    str x0, [x22]
    mov x0, #42
    b exit_now
cow_leaked:
    mov x0, #2
    b exit_now
cow_fail:
    mov x0, #3
    b exit_now
do_align:
    /* SCTLR_EL1.A is clear, so ordinary loads may be unaligned. A branch to
       an odd PC is an alignment fault (EC 0x22) on every implementation. */
    br x22
do_exit_futex:
    /* CLONE_VM|FS|FILES|SIGHAND|THREAD|SYSVSEM = 0x50f00. Child stack is 0x12003000. */
    movz x0, #0x0f00
    movk x0, #0x5, lsl #16
    movz x1, #0x3000
    movk x1, #0x1200, lsl #16
    mov x2, #0
    mov x3, #0
    mov x4, #0
    movz x8, #220
    svc #0
    tbnz x0, #63, futex_fail
    cbz x0, futex_child
futex_parent_wait:
    movz x20, #0
    movk x20, #0x1200, lsl #16
    ldr x1, [x20, #0x108]
    cbz x1, futex_parent_wait
    mov x0, #0
    movz x8, #94
    svc #0
    b exit_now
futex_child:
    movz x20, #0
    movk x20, #0x1200, lsl #16
    mov x1, #1
    str x1, [x20, #0x108]
    add x0, x20, #0x100
    mov x1, #0
    mov x2, #0
    mov x3, #0
    movz x8, #98
    svc #0
    mov x0, #0
    b exit_now
futex_fail:
    mov x0, #4
    b exit_now
do_wait_once:
    mov x0, #17
    mov x1, #0
    mov x2, #0
    mov x3, #0
    mov x4, #0
    movz x8, #220
    svc #0
    tbnz x0, #63, wait_fail
    cbz x0, wait_child
    movn x0, #0
    movz x1, #0x200
    movk x1, #0x1200, lsl #16
    mov x2, #0
    mov x3, #0
    movz x8, #260
    svc #0
    tbnz x0, #63, wait_fail
    cbz x0, wait_fail
    movn x0, #0
    movz x1, #0x200
    movk x1, #0x1200, lsl #16
    mov x2, #0
    mov x3, #0
    movz x8, #260
    svc #0
    cmp x0, #10
    b.eq wait_not_echild
    cmn x0, #10
    b.ne wait_not_echild
    mov x0, #1
    b exit_now
wait_child:
    mov x0, #0
    b exit_now
wait_not_echild:
    mov x0, #2
    b exit_now
wait_fail:
    mov x0, #3
    b exit_now
do_exit_refork:
    mov x0, #17
    mov x1, #0
    mov x2, #0
    mov x3, #0
    mov x4, #0
    movz x8, #220
    svc #0
    tbnz x0, #63, refork_fail
    cbz x0, refork_first_child
    movn x0, #0
    movz x1, #0x200
    movk x1, #0x1200, lsl #16
    mov x2, #0
    mov x3, #0
    movz x8, #260
    svc #0
    tbnz x0, #63, refork_fail
    cbz x0, refork_fail
    movz x22, #0
    movk x22, #0x1600, lsl #16
    ldr x0, [x22]
    movz x1, #0x1111
    cmp x0, x1
    b.ne refork_saw_child
    mov x0, #17
    mov x1, #0
    mov x2, #0
    mov x3, #0
    mov x4, #0
    movz x8, #220
    svc #0
    tbnz x0, #63, refork_fail
    cbz x0, refork_second_child
    movn x0, #0
    movz x1, #0x200
    movk x1, #0x1200, lsl #16
    mov x2, #0
    mov x3, #0
    movz x8, #260
    svc #0
    tbnz x0, #63, refork_fail
    cbz x0, refork_fail
    movz x1, #0x200
    movk x1, #0x1200, lsl #16
    ldr w0, [x1]
    cbnz w0, refork_saw_child
    mov x0, #1
    b exit_now
refork_first_child:
    movz x22, #0
    movk x22, #0x1600, lsl #16
    movz x0, #0x2222
    str x0, [x22]
    mov x0, #0
    b exit_now
refork_second_child:
    movz x22, #0
    movk x22, #0x1600, lsl #16
    ldr x0, [x22]
    movz x1, #0x2222
    cmp x0, x1
    b.eq refork_saw_child
    movz x1, #0x1111
    cmp x0, x1
    b.ne refork_fail
    mov x0, #0
    b exit_now
refork_saw_child:
    mov x0, #2
    b exit_now
refork_fail:
    mov x0, #3
    b exit_now
do_bad_handler:
    movz x20, #0
    movk x20, #0x1200, lsl #16
    add x1, x20, #0x300
    movz x0, #0
    movk x0, #0x1610, lsl #16
    str x0, [x1]
    str xzr, [x1, #8]
    str xzr, [x1, #16]
    str xzr, [x1, #24]
    mov x0, #11
    mov x2, #0
    mov x3, #8
    movz x8, #134
    svc #0
    tbnz x0, #63, bad_handler_fail
    movz x22, #0
    movk x22, #0x1700, lsl #16
    ldr x0, [x22]
    mov x0, #0
    b exit_now
bad_handler_fail:
    mov x0, #5
    b exit_now
exit_now:
    movz x8, #93
    svc #0
    b exit_now
