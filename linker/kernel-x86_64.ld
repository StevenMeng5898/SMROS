/* x86_64 PVH-compatible kernel linker script */

KERNEL_BASE = 0x00100000;

ENTRY(_start)

SECTIONS
{
    . = KERNEL_BASE;

    .note.Xen : ALIGN(4)
    {
        KEEP(*(.note.Xen))
    }

    .text : ALIGN(4K)
    {
        *(.text.boot)
        *(.text .text.*)
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

    .boot_pagetables (NOLOAD) : ALIGN(4K)
    {
        KEEP(*(.boot_pagetables))
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
        . += 0x80000;
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
