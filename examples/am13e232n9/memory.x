MEMORY
{
  /* AM13E232N9: two 256K flash banks, contiguous from 0. */
  FLASH : ORIGIN = 0x00000000, LENGTH = 512K
  /* SRAM on the system bus. SRAM4 and SRAM5 (64K) are on the code bus at 0x00C20000. */
  RAM   : ORIGIN = 0x20000000, LENGTH = 128K
}

/*
 * The AM13E2x flash is programmed in 128-bit words protected by ECC, and a word can only be
 * written once between erases. The flash loader programs each loadable segment of the ELF on its
 * own, so two segments must never share a 16 byte word: the loader would program it twice and
 * fail.
 *
 * Start each flash section of cortex-m-rt's link.x on a 16 byte boundary. The empty sections below
 * only move the location counter; the gap they leave belongs to no segment. (Padding inside a
 * section would not do: the linker adds the padding to the next segment.)
 */
SECTIONS
{
  .am13_align_text ALIGN(16) (NOLOAD) : { __am13_align_text = .; } > FLASH
}
INSERT AFTER .vector_table;

SECTIONS
{
  .am13_align_rodata ALIGN(16) (NOLOAD) : { __am13_align_rodata = .; } > FLASH
}
INSERT AFTER .text;

SECTIONS
{
  .am13_align_data ALIGN(16) (NOLOAD) : { __am13_align_data = .; } > FLASH
}
INSERT AFTER .rodata;
