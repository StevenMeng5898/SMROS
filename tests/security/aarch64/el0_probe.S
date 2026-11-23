/* Fixed-VA EL0 probe. Data page is 0x12000000. Code page is 0x12001000.
 *
 * Data layout (little-endian u64):
 *   0  op: 1 read, 2 write, 3 execute, 4 syscall, 5 undef,
 *      6 mprotect-after-store, 7 cow-fork, 8 unaligned ldxr,
 *      9 exit-group while another thread is in futex_wait,
 *      10 wait twice, 11 fork again after the first child exits,
 *      12 SIGSEGV handler that is not an executable user page,
 *      13 owner chmod then non-owner EPERM
 *      14 SIGEV_THREAD notify-gate: process A re-arms a near deadline while
 *         process B's nanosleep and clock_nanosleep must return within 200ms
 *         and not before the requested 49999872 ns. Early wakes use 36 and 37
 *         and are not retried.
 *   Path for op 13 is planted at 0x400. The fd is saved at 0x480.
 *   stat buffer is 0x500.
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
    cmp x21, #13
    b.eq do_chmod
    cmp x21, #14
    b.eq do_notify_gate
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
do_chmod:
    /* Become uid 1, create /dev/shm/smros-sec-chmod, chmod 0600, stat it,
       then become uid 2 and confirm fchmod is EPERM and the mode is unchanged. */
    mov x0, #1
    mov x1, #1
    mov x2, #1
    movz x8, #147
    svc #0
    tbnz x0, #63, chmod_uid
    movn x0, #99
    movz x1, #0x400
    movk x1, #0x1200, lsl #16
    movz x2, #0x242
    movz x3, #0x1a4
    movz x8, #56
    svc #0
    tbnz x0, #63, chmod_open
    movz x20, #0
    movk x20, #0x1200, lsl #16
    str x0, [x20, #0x480]
    ldr x0, [x20, #0x480]
    movz x1, #0x180
    movz x8, #52
    svc #0
    tbnz x0, #63, chmod_own
    movz x20, #0
    movk x20, #0x1200, lsl #16
    ldr x0, [x20, #0x480]
    movz x1, #0x500
    movk x1, #0x1200, lsl #16
    movz x8, #80
    svc #0
    tbnz x0, #63, chmod_stat
    movz x1, #0x500
    movk x1, #0x1200, lsl #16
    ldr w0, [x1, #16]
    and w0, w0, #0xfff
    movz w1, #0x180
    cmp w0, w1
    b.ne chmod_mode
    mov x0, #2
    mov x1, #2
    mov x2, #2
    movz x8, #147
    svc #0
    tbnz x0, #63, chmod_uid2
    movz x20, #0
    movk x20, #0x1200, lsl #16
    ldr x0, [x20, #0x480]
    movz x1, #0x1a0
    movz x8, #52
    svc #0
    cmn x0, #1
    b.ne chmod_eperm
    movz x20, #0
    movk x20, #0x1200, lsl #16
    ldr x0, [x20, #0x480]
    movz x1, #0x500
    movk x1, #0x1200, lsl #16
    movz x8, #80
    svc #0
    tbnz x0, #63, chmod_stat2
    movz x1, #0x500
    movk x1, #0x1200, lsl #16
    ldr w0, [x1, #16]
    and w0, w0, #0xfff
    movz w1, #0x180
    cmp w0, w1
    b.ne chmod_kept
    mov x0, #0
    b exit_now
chmod_uid:
    mov x0, #11
    b exit_now
chmod_open:
    mov x0, #12
    b exit_now
chmod_own:
    mov x0, #13
    b exit_now
chmod_stat:
    mov x0, #14
    b exit_now
chmod_mode:
    mov x0, #15
    b exit_now
chmod_uid2:
    mov x0, #16
    b exit_now
chmod_eperm:
    mov x0, #17
    b exit_now
chmod_stat2:
    mov x0, #18
    b exit_now
chmod_kept:
    mov x0, #19
    b exit_now

/* Process A arms a one-shot SIGEV_THREAD timer, waits for it in sigwait so
 * the kernel gate is armed, then forks. The child measures a 50ms nanosleep
 * and a 50ms clock_nanosleep. The parent keeps re-arming a 1ms deadline and
 * then wait4s without returning to sigwait, so the gate stays armed. Exit 0
 * only when both of the child's sleeps last at least the requested 49999872 ns
 * and finish in under 200ms.
 *
 * Data page:
 *   0x200 sigevent, 0x240 itimerspec, 0x260 sleep request,
 *   0x270/0x280 timespec samples, 0x290 sigset, 0x298 timer id,
 *   0x2A0 wait status, 0x2A8 sigwait timeout, 0x2C0 arm-loop count.
 */
do_notify_gate:
    movz x0, #0
    movk x0, #2, lsl #32
    str x0, [x20, #0x290]
    str xzr, [x20, #0x200]
    movz w0, #34
    str w0, [x20, #0x208]
    movz w0, #2
    str w0, [x20, #0x20c]
    str xzr, [x20, #0x2A8]
    movz x0, #0x8400
    movk x0, #0x17D7, lsl #16
    str x0, [x20, #0x2B0]
    mov x0, #0
    add x1, x20, #0x290
    mov x2, #0
    mov x3, #8
    movz x8, #135
    svc #0
    tbnz x0, #63, ng_mask
    movz x20, #0
    movk x20, #0x1200, lsl #16
    mov x0, #1
    add x1, x20, #0x200
    add x2, x20, #0x298
    movz x8, #107
    svc #0
    tbnz x0, #63, ng_create
    movz x20, #0
    movk x20, #0x1200, lsl #16
    mov x0, #3
    str x0, [x20, #0x2C0]
ng_arm_loop:
    movz x20, #0
    movk x20, #0x1200, lsl #16
    str xzr, [x20, #0x240]
    str xzr, [x20, #0x248]
    str xzr, [x20, #0x250]
    movz x0, #0xE100
    movk x0, #0x05F5, lsl #16
    str x0, [x20, #0x258]
    ldr w0, [x20, #0x298]
    mov x1, #0
    add x2, x20, #0x240
    mov x3, #0
    movz x8, #110
    svc #0
    tbnz x0, #63, ng_settime
    movz x20, #0
    movk x20, #0x1200, lsl #16
    add x0, x20, #0x290
    mov x1, #0
    add x2, x20, #0x2A8
    mov x3, #8
    movz x8, #137
    svc #0
    tbnz x0, #63, ng_sigwait
    movz x20, #0
    movk x20, #0x1200, lsl #16
    ldr x21, [x20, #0x2C0]
    sub x21, x21, #1
    str x21, [x20, #0x2C0]
    cbnz x21, ng_arm_loop
    mov x0, #17
    mov x1, #0
    mov x2, #0
    mov x3, #0
    mov x4, #0
    movz x8, #220
    svc #0
    tbnz x0, #63, ng_fork
    cbz x0, ng_child
    movz x20, #0
    movk x20, #0x1200, lsl #16
    mov x0, #1
    add x1, x20, #0x270
    movz x8, #113
    svc #0
    tbnz x0, #63, ng_clock
ng_hold:
    movz x20, #0
    movk x20, #0x1200, lsl #16
    str xzr, [x20, #0x240]
    str xzr, [x20, #0x248]
    str xzr, [x20, #0x250]
    movz x0, #0x4240
    movk x0, #0x000F, lsl #16
    str x0, [x20, #0x258]
    ldr w0, [x20, #0x298]
    mov x1, #0
    add x2, x20, #0x240
    mov x3, #0
    movz x8, #110
    svc #0
    tbnz x0, #63, ng_settime
    movz x20, #0
    movk x20, #0x1200, lsl #16
    mov x0, #1
    add x1, x20, #0x280
    movz x8, #113
    svc #0
    tbnz x0, #63, ng_clock
    movz x20, #0
    movk x20, #0x1200, lsl #16
    ldr x1, [x20, #0x270]
    ldr x2, [x20, #0x280]
    ldr x3, [x20, #0x278]
    ldr x4, [x20, #0x288]
    sub x2, x2, x1
    subs x4, x4, x3
    b.ge ng_hold_delta
    sub x2, x2, #1
    movz x5, #0xCA00
    movk x5, #0x3B9A, lsl #16
    add x4, x4, x5
ng_hold_delta:
    cbnz x2, ng_wait_child
    movz x5, #0xA300
    movk x5, #0x11E1, lsl #16
    cmp x4, x5
    b.lo ng_hold
ng_wait_child:
    movz x20, #0
    movk x20, #0x1200, lsl #16
    movn x0, #0
    add x1, x20, #0x2A0
    mov x2, #0
    movz x8, #260
    svc #0
    tbnz x0, #63, ng_wait_fail
    movz x20, #0
    movk x20, #0x1200, lsl #16
    ldr w0, [x20, #0x2A0]
    cbz w0, ng_parent_ok
    and w1, w0, #0xff
    cbnz w1, ng_child_signal
    lsr w0, w0, #8
    b exit_now
ng_parent_ok:
    mov x0, #0
    b exit_now
ng_child:
    movz x20, #0
    movk x20, #0x1200, lsl #16
    str xzr, [x20, #0x260]
    movz x0, #0xF080
    movk x0, #0x02FA, lsl #16
    str x0, [x20, #0x268]
    mov x0, #1
    add x1, x20, #0x270
    movz x8, #113
    svc #0
    tbnz x0, #63, ng_child_clock
    movz x20, #0
    movk x20, #0x1200, lsl #16
    add x0, x20, #0x260
    mov x1, #0
    movz x8, #101
    svc #0
    tbnz x0, #63, ng_child_nano
    movz x20, #0
    movk x20, #0x1200, lsl #16
    mov x0, #1
    add x1, x20, #0x280
    movz x8, #113
    svc #0
    tbnz x0, #63, ng_child_clock
    movz x20, #0
    movk x20, #0x1200, lsl #16
    ldr x1, [x20, #0x270]
    ldr x2, [x20, #0x280]
    ldr x3, [x20, #0x278]
    ldr x4, [x20, #0x288]
    sub x2, x2, x1
    subs x4, x4, x3
    b.ge ng_nano_delta
    sub x2, x2, #1
    movz x5, #0xCA00
    movk x5, #0x3B9A, lsl #16
    add x4, x4, x5
ng_nano_delta:
    cbnz x2, ng_child_slow_nano
    movz x5, #0xF080
    movk x5, #0x02FA, lsl #16
    cmp x4, x5
    b.lo ng_child_early_nano
    movz x5, #0xC200
    movk x5, #0x0BEB, lsl #16
    cmp x4, x5
    b.hs ng_child_slow_nano
    movz x20, #0
    movk x20, #0x1200, lsl #16
    str xzr, [x20, #0x260]
    movz x0, #0xF080
    movk x0, #0x02FA, lsl #16
    str x0, [x20, #0x268]
    mov x0, #1
    add x1, x20, #0x270
    movz x8, #113
    svc #0
    tbnz x0, #63, ng_child_clock
    movz x20, #0
    movk x20, #0x1200, lsl #16
    mov x0, #1
    mov x1, #0
    add x2, x20, #0x260
    mov x3, #0
    movz x8, #115
    svc #0
    tbnz x0, #63, ng_child_cns
    movz x20, #0
    movk x20, #0x1200, lsl #16
    mov x0, #1
    add x1, x20, #0x280
    movz x8, #113
    svc #0
    tbnz x0, #63, ng_child_clock
    movz x20, #0
    movk x20, #0x1200, lsl #16
    ldr x1, [x20, #0x270]
    ldr x2, [x20, #0x280]
    ldr x3, [x20, #0x278]
    ldr x4, [x20, #0x288]
    sub x2, x2, x1
    subs x4, x4, x3
    b.ge ng_cns_delta
    sub x2, x2, #1
    movz x5, #0xCA00
    movk x5, #0x3B9A, lsl #16
    add x4, x4, x5
ng_cns_delta:
    cbnz x2, ng_child_slow_clock
    movz x5, #0xF080
    movk x5, #0x02FA, lsl #16
    cmp x4, x5
    b.lo ng_child_early_clock
    movz x5, #0xC200
    movk x5, #0x0BEB, lsl #16
    cmp x4, x5
    b.hs ng_child_slow_clock
    mov x0, #0
    b exit_now
ng_mask:
    mov x0, #21
    b exit_now
ng_create:
    mov x0, #22
    b exit_now
ng_settime:
    mov x0, #23
    b exit_now
ng_sigwait:
    mov x0, #24
    b exit_now
ng_fork:
    mov x0, #25
    b exit_now
ng_clock:
    mov x0, #26
    b exit_now
ng_wait_fail:
    mov x0, #27
    b exit_now
ng_child_signal:
    mov x0, #29
    b exit_now
ng_child_clock:
    mov x0, #31
    b exit_now
ng_child_nano:
    mov x0, #32
    b exit_now
ng_child_slow_nano:
    mov x0, #33
    b exit_now
ng_child_cns:
    mov x0, #34
    b exit_now
ng_child_slow_clock:
    mov x0, #35
    b exit_now
ng_child_early_nano:
    mov x0, #36
    b exit_now
ng_child_early_clock:
    mov x0, #37
    b exit_now
exit_now:
    movz x8, #93
    svc #0
    b exit_now
