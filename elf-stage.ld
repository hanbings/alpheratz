ENTRY(efi_main)

SECTIONS
{
    /* Reserve space for PE headers; sections start at RVA 0x1000 */
    . = 0x1000;

    .text ALIGN(0x1000) : {
        *(.text .text.*)
    }

    .rodata ALIGN(0x1000) : {
        *(.rodata .rodata.*)
        *(.srodata .srodata.*)
    }

    .data ALIGN(0x1000) : {
        *(.data .data.*)
        *(.sdata .sdata.*)
        *(.got .got.plt)
    }

    .bss ALIGN(0x1000) : {
        *(.bss .bss.*)
        *(.sbss .sbss.*)
        *(COMMON)
    }

    /DISCARD/ : {
        *(.comment)
        *(.riscv.attributes)
        *(.loongarch.attributes)
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.debug_*)
        *(.note*)
        *(.plt)
    }
}
