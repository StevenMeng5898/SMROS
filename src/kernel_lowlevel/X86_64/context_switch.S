.section .text

.globl context_switch
.type context_switch, @function
context_switch:
    cli

    lea rax, [rdi + 0x10]
    mov [rax + 0x00], r15
    mov [rax + 0x08], r14
    mov [rax + 0x10], r13
    mov [rax + 0x18], r12
    mov [rax + 0x20], rbp
    mov [rax + 0x28], rbx
    mov [rax + 0x30], rsp
    lea rcx, [rip + .Lresume]
    mov [rax + 0x38], rcx
    pushfq
    pop rcx
    mov [rax + 0x40], rcx

    lea rax, [rsi + 0x10]
    mov r15, [rax + 0x00]
    mov r14, [rax + 0x08]
    mov r13, [rax + 0x10]
    mov r12, [rax + 0x18]
    mov rbp, [rax + 0x20]
    mov rbx, [rax + 0x28]
    mov rsp, [rax + 0x30]
    mov rcx, [rax + 0x38]
    sti
    jmp rcx
.Lresume:
    sti
    ret
.size context_switch, . - context_switch

.globl context_switch_start
.type context_switch_start, @function
context_switch_start:
    cli
    lea rax, [rdi + 0x10]
    mov r15, [rax + 0x00]
    mov r14, [rax + 0x08]
    mov r13, [rax + 0x10]
    mov r12, [rax + 0x18]
    mov rbp, [rax + 0x20]
    mov rbx, [rax + 0x28]
    mov rsp, [rax + 0x30]
    mov rcx, [rax + 0x38]
    sti
    jmp rcx
.size context_switch_start, . - context_switch_start
