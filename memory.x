/* This is the memory layout for the Feather M4. It _also_ works for
the PyPortal, because we can get by with less flash and RAM for these
examples. */
MEMORY
{
  /* Leave 16k for the default bootloader on the Feather M4 */
  FLASH (rx) : ORIGIN = 0x00000000 + 16K, LENGTH = 512K - 16K
  RAM (xrw)  : ORIGIN = 0x20000000, LENGTH = 192K
}
_stack_start = ORIGIN(RAM) + LENGTH(RAM);

