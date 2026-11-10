/* Fixed-VA EL0 probe. Data page is 0x12000000. Code page is 0x12001000.
 *
 * Data layout (little-endian u64):
 *   0  op: 1 read, 2 write, 3 execute, 4 syscall, 5 undef,
 *      6 mprotect-after-store, 7 cow-fork
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
exit_now:
    mov x8, #93
    svc #0
    b exit_now
