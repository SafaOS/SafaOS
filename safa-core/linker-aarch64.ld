/* Tell the linker that we want an aarch64 ELF64 output file */
OUTPUT_FORMAT(elf64-littleaarch64)

/*
kboot is defined in src/arch/aarch64/mod.rs in a global_asm block
it eventually calls kstart at src/main.rs
but it has to setup some stuff first unlike x86_64
*/
ENTRY(kboot)

/* Define the program headers we want so the bootloader gives us the right */
/* MMU permissions; this also allows us to exert more control over the linking */
/* process. */
PHDRS
{
    text PT_LOAD;
    rodata PT_LOAD;
    data PT_LOAD;
}

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
    } :rodata
    section_rodata_end = .;

    /* Move to the next memory page for .data */
    . = ALIGN(CONSTANT(MAXPAGESIZE));

    section_data_begin = .;
    .data : {
        *(.data .data.*)
        /* Place the sections that contain the Limine requests as part of the .data */
        /* output section. */
        KEEP(*(.requests_start_marker))
        KEEP(*(.requests))
        KEEP(*(.requests_end_marker))
    } :data

    /* NOTE: .bss needs to be the last thing mapped to :data, otherwise lots of */
    /* unnecessary zeros will be written to the binary. */
    /* If you need, for example, .init_array and .fini_array, those should be placed */
    /* above this. */
    .bss : {
        *(.bss .bss.*)
        *(COMMON)
    } :data
    section_data_end = .;

    /* Discard .note.* and .eh_frame* since they may cause issues on some hosts. */
    /DISCARD/ : {
        *(.eh_frame*)
        *(.note .note.*)
    }
}
