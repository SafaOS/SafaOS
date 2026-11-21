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
    percpu  PT_LOAD;
}

INCLUDE common.ld
