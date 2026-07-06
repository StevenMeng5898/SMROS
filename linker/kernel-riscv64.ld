/* RISC-V64 Kernel Linker Script */

/* OpenSBI/QEMU RISC-V machines conventionally jump to payloads at
 * DRAM_BASE + 2 MiB. MMIO and topology still come from the FDT at runtime.
 */
KERNEL_BASE = 0x80200000;

ENTRY(_start)

SECTIONS
{
    . = KERNEL_BASE;

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
