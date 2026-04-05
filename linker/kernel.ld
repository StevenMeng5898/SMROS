/* ARM64 Kernel Linker Script */

/* Kernel load address - QEMU virt machine loads 64-bit kernels at 0x40000000 */
KERNEL_BASE = 0x40000000;

ENTRY(_start)

SECTIONS
{
    . = KERNEL_BASE;

    .text : ALIGN(4K)
    {
        *(.text.boot)
        *(.text .text.*)
        /* Ensure secondary_entry is in text section */
        KEEP(*(.text.secondary_entry))
        . = ALIGN(4K);
    }
    
    .rodata : ALIGN(4K)
    {
        *(.rodata .rodata.*)
        . = ALIGN(4K);
    }
    
    .data : ALIGN(4K)
    {
        *(.data .data.*)
        . = ALIGN(4K);
    }
    
    .bss : ALIGN(4K)
    {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        __bss_end = .;
        . = ALIGN(4K);
    }
    
    .stack (NOLOAD) : ALIGN(16)
    {
        __stack_bottom = .;
        . += 0x80000; /* 512KB stack */
        __stack_top = .;
    }
    
    /DISCARD/ :
    {
        *(.comment)
        *(.gnu*)
        *(.note*)
        *(.eh_frame*)
    }
    
    __kernel_end = .;
}
