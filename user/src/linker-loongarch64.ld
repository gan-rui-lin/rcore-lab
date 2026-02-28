/* LoongArch64 User Program Linker Script
 * Based on RISC-V linker script from rcore-lab
 */
OUTPUT_ARCH(loongarch)
ENTRY(_start)

BASE_ADDRESS = 0x0;

SECTIONS
{
    . = BASE_ADDRESS;
    .text : {
        *(.text.entry)
        *(.text .text.*)
    }
    . = ALIGN(4K);
    .rodata : {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }
    . = ALIGN(4K);
    .data : {
        *(.data .data.*)
        *(.sdata .sdata.*)
    }
    .bss : {
        PROVIDE(start_bss = .);
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        PROVIDE(end_bss = .);
    }
    /DISCARD/ : {
        *(.eh_frame)
        *(.debug*)
    }
}
