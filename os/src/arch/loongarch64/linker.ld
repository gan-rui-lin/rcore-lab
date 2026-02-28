OUTPUT_ARCH(loongarch)
ENTRY(_start)

/* LoongArch64 kernel base address using DMW1 mapping
   DMW1: 0x9000_0000_xxxx_xxxx (cached kernel space)
   Physical offset: 0x9000_0000 (2.25GB) - matches OSKernel2025 */
BASE_ADDRESS = 0x9000000090000000;

SECTIONS
{
    . = BASE_ADDRESS;
    skernel = .;

    stext = .;
    .text : {
        *(.text.entry)
        . = ALIGN(4K);
        strampoline = .;
        *(.text.trampoline);
        . = ALIGN(4K);
        KEEP(*(.text.keep))
        *(.text .text.*)
    }

    . = ALIGN(4K);
    etext = .;
    srodata = .;
    .rodata : {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }

    . = ALIGN(4K);
    erodata = .;
    sdata = .;
    .data : {
        *(.data .data.*)
        *(.sdata .sdata.*)
    }

    . = ALIGN(4K);
    edata = .;

    /* Signal trampoline section (LoongArch specific) */
    .sigtrx ALIGN(4K): {
        *(.sigtrx .sigtrx.*)
    }

    sbss_with_stack = .;
    .bss : {
        *(.bss.stack)
        sbss = .;
        *(.bss .bss.*)
        *(.sbss .sbss.*)
    }

    . = ALIGN(4K);
    ebss = .;
    ekernel = .;

    /DISCARD/ : {
        *(.eh_frame)
        *(.comment)
        *(.gnu*)
        *(.note*)
    }
}
