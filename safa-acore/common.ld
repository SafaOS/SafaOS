SECTIONS
{
    /* We want to be placed in the topmost 2GiB of the address space, for optimisations */
    /* and because that is what the Limine spec mandates. */
    /* Any address in this region will do, but often 0xffffffff80000000 is chosen as */
    /* that is the beginning of the region. */
    . = 0xffffffff80000000;

    section_text_begin = .;
    .text : {
        *(.text .text.*)
    } :text
    section_text_end = .;

    /* Move to the next memory page for .rodata */
    . = ALIGN(CONSTANT(MAXPAGESIZE));

    section_rodata_begin = .;
    .rodata : {
        *(.rodata .rodata.*)
        . = ALIGN(16);
        section_per_cpu_init_begin = .;
        *(.percpu.initializers.base)
        *(.percpu.initializers)
        section_per_cpu_init_end = .;

        *(.got .got.*)
    } :rodata
    section_rodata_end = .;

    /* Move to the next memory page for .data */
    . = ALIGN(CONSTANT(MAXPAGESIZE));

    section_data_begin = .;
    .data : {
        *(.data .data.*)
        KEEP(*(.oninit))

        /* Place the sections that contain the Limine requests as part of the .data */
        /* output section. */
        KEEP(*(.requests_start_marker))
        KEEP(*(.requests))
        KEEP(*(.requests_end_marker))


        . = ALIGN(16);
        section_oninit_f_begin = .;
        *(.oninit.initializers.base)
        *(.oninit.initializers)
        section_oninit_f_end = .;
    } :data

    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    } :data

    . = ALIGN(CONSTANT(MAXPAGESIZE));
    /* Must come last, because it'd be copied over depending on the cpu counts at runtime */
    .percpu : {
        section_per_cpu_begin = .;
        *(.percpu.base)
        *(.percpu)
        . = ALIGN(0x1000);
        section_per_cpu_end = .;
        section_data_end = .;
    } :percpu


    /* Discard .note.* and .eh_frame* since they may cause issues on some hosts. */
    /DISCARD/ : {
        *(.eh_frame*)
        *(.note .note.*)
    }
}
