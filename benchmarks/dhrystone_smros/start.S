.section .text.start,"ax",%progbits
.global _start
.type _start,%function
_start:
    mov x29, #0
    mov x30, #0
    mov x0, sp
    bl smros_dhrystone_main
1:
    wfe
    b 1b
