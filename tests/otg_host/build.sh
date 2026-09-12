#!/bin/sh
# Build the bare-metal OTG_FS host (HCD) demo with the Arduino toolchain's gcc.
# No libc/Arduino core (F105 has no Arduino board locally): freestanding
# Cortex-M3, custom linker script. The ELF ships at site/otg_host.elf
# (force-added; *.elf is gitignored) for CI, which never compiles.
set -e
GCC=/home/danish1075/.arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-gcc
D=$(dirname "$0")
mkdir -p "$D/build"
"$GCC" -mcpu=cortex-m3 -mthumb -Os -ffreestanding -nostdlib \
  -Wl,--gc-sections -T "$D/link.ld" "$D/main.c" -o "$D/build/otg_host.elf"
cp "$D/build/otg_host.elf" "$D/../../site/otg_host.elf"
ls -la "$D/../../site/otg_host.elf"
