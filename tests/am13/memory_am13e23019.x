MEMORY
{
  /* AM13E23019: two 256K flash banks. The tests run from RAM. */
  FLASH : ORIGIN = 0x00000000, LENGTH = 512K
  RAM   : ORIGIN = 0x20000000, LENGTH = 96K
}
