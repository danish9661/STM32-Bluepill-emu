from keystone import *
import struct

ks = Ks(KS_ARCH_ARM, KS_MODE_THUMB)

# Blink test for STM32F103C8 (Blue Pill) - LED on PC13
# Code starts at 0x08000040
code = """
    // Enable GPIOC clock: RCC_APB2ENR (0x40021018) bit 4 = IOPCEN
    ldr r3, =0x40021018
    ldr r1, [r3]
    movs r0, #0x10
    orrs r1, r0
    str r1, [r3]

    // Configure PC13 as output push-pull 50MHz
    // GPIOC_CRH at 0x40011004, PC13 is bits 23:20
    ldr r3, =0x40011004
    ldr r1, [r3]
    // Clear bits 23:20
    ldr r2, =0xFF0FFFFF
    ands r1, r2
    // Set bits 21:20 = 0x3 (output push-pull 50MHz)
    ldr r2, =0x00300000
    orrs r1, r2
    str r1, [r3]

    // Blink loop
loop:
    // Set PC13 (LED off - active low) via BSRR BS13
    ldr r3, =0x40011010
    movs r0, #0x20
    lsls r0, r0, #8        // r0 = 0x2000 = bit 13
    str r0, [r3]

    // Delay
    ldr r2, =0x1FFFF
delay1:
    subs r2, r2, #1
    bne delay1

    // Reset PC13 (LED on) via BSRR BR13
    ldr r0, =0x20000000    // bit 29
    str r0, [r3]

    // Delay
    ldr r2, =0x1FFFF
delay2:
    subs r2, r2, #1
    bne delay2

    b loop
"""

encoding, _ = ks.asm(code, 0x08000040)

binary = bytearray()

# Vector table (16 entries = 0x40 bytes)
binary += struct.pack('<I', 0x20005000)  # SP
binary += struct.pack('<I', 0x08000041)  # Reset handler (Thumb)
for i in range(2, 16):
    binary += struct.pack('<I', 0x08000100)  # All handlers -> inf loop at 0x100

# Assembly code goes right after at 0x40
for b in encoding:
    binary += bytes([b])

# Pad to 0x100 for infinite loop
while len(binary) < 0x100:
    binary += b'\x00'

# Infinite loop at 0x08000100: B .
binary += b'\xfe\xe7'

with open('blink_test/firmware.bin', 'wb') as f:
    f.write(binary)

print(f"Firmware size: {len(binary)} bytes")
print(f"Encoding length: {len(encoding)} bytes")
print(f"Binary written to blink_test/firmware.bin")
