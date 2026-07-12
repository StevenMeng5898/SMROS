core::arch::global_asm!(
    r#"
.section .note.Xen, "a", @note
.align 4
    .long 4
    .long 4
    .long 18
    .asciz "Xen"
.align 4
    .long _start
.align 4

.section .text.boot, "ax"
.code32
.globl _start
_start:
    cli
    mov ebx, ebx
    mov esp, offset __stack_top

    mov edi, offset pml4_table
    xor eax, eax
    mov ecx, 4096 * 6 / 4
    rep stosd

    mov eax, offset pdpt_table
    or eax, 0x003
    mov dword ptr [pml4_table], eax

    mov eax, offset pd_table0
    or eax, 0x003
    mov dword ptr [pdpt_table], eax

    mov eax, offset pd_table1
    or eax, 0x003
    mov dword ptr [pdpt_table + 8], eax

    mov eax, offset pd_table2
    or eax, 0x003
    mov dword ptr [pdpt_table + 16], eax

    mov eax, offset pd_table3
    or eax, 0x003
    mov dword ptr [pdpt_table + 24], eax

    xor ecx, ecx
1:
    mov eax, ecx
    shl eax, 21
    or eax, 0x083
    mov dword ptr [pd_table0 + ecx * 8], eax
    mov dword ptr [pd_table0 + ecx * 8 + 4], 0
    mov edx, eax
    add edx, 0x40000000
    mov dword ptr [pd_table1 + ecx * 8], edx
    mov dword ptr [pd_table1 + ecx * 8 + 4], 0
    mov edx, eax
    add edx, 0x80000000
    mov dword ptr [pd_table2 + ecx * 8], edx
    mov dword ptr [pd_table2 + ecx * 8 + 4], 0
    mov edx, eax
    add edx, 0xc0000000
    mov dword ptr [pd_table3 + ecx * 8], edx
    mov dword ptr [pd_table3 + ecx * 8 + 4], 0
    inc ecx
    cmp ecx, 512
    jne 1b

    lgdt [gdt64_descriptor]

    mov eax, cr4
    or eax, 1 << 5
    mov cr4, eax

    mov eax, offset pml4_table
    mov cr3, eax

    mov ecx, 0xC0000080
    rdmsr
    or eax, 1 << 8
    wrmsr

    mov eax, cr0
    or eax, 0x80000001
    mov cr0, eax

    .byte 0xea
    .long long_mode_start
    .word 0x08

.code64
long_mode_start:
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov fs, ax
    mov gs, ax

    mov rsp, offset __stack_top
    and rsp, -16

    lea rdi, [rip + __bss_start]
    lea rsi, [rip + __bss_end]
    xor rax, rax
2:
    cmp rdi, rsi
    jae 3f
    mov qword ptr [rdi], rax
    add rdi, 8
    jmp 2b

3:
    mov edi, ebx
    call kernel_main

4:
    hlt
    jmp 4b

.align 16
gdt64:
    .quad 0
    .quad 0x00AF9A000000FFFF
    .quad 0x00AF92000000FFFF
gdt64_end:
gdt64_descriptor:
    .word gdt64_end - gdt64 - 1
    .quad gdt64

.section .boot_pagetables, "aw", @nobits
.align 4096
pml4_table:
    .skip 4096
.align 4096
pdpt_table:
    .skip 4096
.align 4096
pd_table0:
    .skip 4096
.align 4096
pd_table1:
    .skip 4096
.align 4096
pd_table2:
    .skip 4096
.align 4096
pd_table3:
    .skip 4096
"#,
);

core::arch::global_asm!(include_str!("context_switch.S"));
