#!/usr/bin/env python3
"""Decoder census diff: ours (src/cpu/census.rs dumps) vs Capstone M-class.

Usage:
  cargo test --release --lib cpu::census   # regenerates /tmp/census*_ours.json
  node --version >/dev/null 2>&1; python3 tests/census_16.py [--32]

Coding on both sides: 0 = executes/valid, 1 = known trap (ours only:
SVC/BKPT/UDF fault by design; Capstone calls them valid), 2 = fault/invalid.
'3' (ours dump) = 32-bit prefix halfword, skipped in the 16-bit diff.

Exit 0 when the only diffs are the accepted-triage list below; exit 1 with
the gap list otherwise (CI gate).
"""
import struct
import sys

from capstone import Cs, CS_ARCH_ARM, CS_MODE_THUMB, CS_MODE_MCLASS

MD = Cs(CS_ARCH_ARM, CS_MODE_THUMB | CS_MODE_MCLASS)

# Opcodes where we deliberately differ from Capstone and both are "right".
# Each entry: (first, last, reason). The gate fails on anything outside
# these sets, so decoder regressions/fixes surface loudly.
ACCEPTED_GAPS = [
    # ADD(h)/MOV(h) with Rd=PC: our arms route to branch(), which faults on
    # the even target exactly like HW UsageFault-on-ARM-entry.
    (0x44EF, 0x44EF, 'add pc,sp,pc: even target faults like HW'),
    (0x44FF, 0x44FF, 'add pc,pc: even target faults like HW'),
    (0x46EF, 0x46EF, 'mov pc,sp: even target faults like HW'),
    (0x46FF, 0x46FF, 'mov pc,pc: even target faults like HW'),
    # BX/BLX with SP/PC target: even target -> branch() faults like HW.
    (0x4768, 0x476B, 'bx sp: even target faults like HW'),
    (0x4778, 0x477B, 'bx pc: even target faults like HW'),
    (0x47E8, 0x47E8, 'blx sp: even target faults like HW'),
    (0x47F8, 0x47F8, 'blx pc: even target faults like HW'),
    # ARMv8-M Security extensions: UNDEFINED on v7-M, our fault is correct.
    (0x476C, 0x476F, 'bxns: v8-M only'),
    (0x477C, 0x477F, 'bxns: v8-M only'),
    (0x47EC, 0x47EF, 'blxns: v8-M only'),
    (0x47FC, 0x47FF, 'blxns: v8-M only'),
    # 0xBF6x-0xBFFx low-nibble-0: IT with zero mask = UNPREDICTABLE on v7-M
    # (v8-M redefined the space as HINTs); loud fault is the safe choice.
    (0xBF60, 0xBF60, 'UNPREDICTABLE IT-zero-mask on v7-M'),
    (0xBF70, 0xBF70, 'UNPREDICTABLE IT-zero-mask on v7-M'),
    (0xBF80, 0xBF80, 'UNPREDICTABLE IT-zero-mask on v7-M'),
    (0xBF90, 0xBF90, 'UNPREDICTABLE IT-zero-mask on v7-M'),
    (0xBFA0, 0xBFA0, 'UNPREDICTABLE IT-zero-mask on v7-M'),
    (0xBFB0, 0xBFB0, 'UNPREDICTABLE IT-zero-mask on v7-M'),
    (0xBFC0, 0xBFC0, 'UNPREDICTABLE IT-zero-mask on v7-M'),
    (0xBFD0, 0xBFD0, 'UNPREDICTABLE IT-zero-mask on v7-M'),
    (0xBFE0, 0xBFE0, 'UNPREDICTABLE IT-zero-mask on v7-M'),
    (0xBFF0, 0xBFF0, 'UNPREDICTABLE IT-zero-mask on v7-M'),
]

# Over-accepts: reserved/UNPREDICTABLE-bit combinations we execute (ignoring
# the reserved bits) while Capstone rejects. No assembler emits these; real
# HW behavior is UNPREDICTABLE so executing the obvious reading is safe.
# Encoded as predicates over the opcode for compactness.
def accepted_over(op):
    if 0x4780 <= op <= 0x47FF and op & 0x7 != 0:
        return True  # BX/BLX with reserved low bits set
    if op == 0xB400 or op == 0xBC00:
        return True  # PUSH/POP with empty reglist: benign NOP
    if 0xB620 <= op <= 0xB63F:
        return True  # CPS reserved imod/AIF combos
    if 0xB668 <= op <= 0xB67F:
        return True  # CPS reserved imod/AIF combos
    if 0xB6A0 <= op <= 0xB6BF:
        return True  # CPS reserved imod/AIF combos
    if 0xB6E0 <= op <= 0xB6FF:
        return True  # CPS reserved imod/AIF combos
    if 0xC000 <= op <= 0xCFFF and op & 0xFF == 0x00:
        return True  # STMIA Rn!,{} empty list: benign NOP
    return False


def accepted_gap(op):
    return any(a <= op <= b for (a, b, _) in ACCEPTED_GAPS)


def cap16(op):
    data = struct.pack('<H', op)
    try:
        insns = list(MD.disasm(data, 0x08000000))
    except Exception:
        return 2
    if not insns:
        return 2
    return 0


def main():
    ours = open('/tmp/census16_ours.json').read().strip()
    assert len(ours) == 65536, len(ours)
    gaps, over = [], []
    for op in range(65536):
        o = ours[op]
        if o == '3':  # 32-bit prefix territory
            continue
        c = cap16(op)
        mine = 0 if o in '01' else 2
        if mine == 2 and c == 0:
            if not accepted_gap(op):
                gaps.append(op)
        elif mine == 0 and c == 2:
            if not accepted_over(op):
                over.append(op)
    print(f'16-bit: {len(gaps)} gaps, {len(over)} over-accepts')
    for op in gaps[:60]:
        data = struct.pack('<H', op)
        name = '; '.join(f'{i.mnemonic} {i.op_str}' for i in MD.disasm(data, 0x08000000))
        print(f'  GAP 0x{op:04x}  {name}')
    if len(gaps) > 60:
        print(f'  ... and {len(gaps) - 60} more')
    for op in over[:20]:
        print(f'  OVER 0x{op:04x}')
    if len(over) > 20:
        print(f'  ... and {len(over) - 20} more')
    return 1 if (gaps or over) else 0


if __name__ == '__main__':
    sys.exit(main())
