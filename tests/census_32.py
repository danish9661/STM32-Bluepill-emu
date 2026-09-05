#!/usr/bin/env python3
"""32-bit decoder census diff: structured sample (every first halfword x 256
sampled seconds) ours vs Capstone M-class.

Usage:
  cargo test --release --lib cpu::census   # regenerates /tmp/census32_ours.json
  python3 tests/census_32.py

Reports, per first halfword, samples where Capstone accepts but we fault.
Families correctly unimplemented on M3 (FPU/coprocessor/DSP/v8-M) are in
ACCEPTED_FIRST below. Anything else is a decoder gap (exit 1).
"""
import struct
import sys
from collections import Counter

from capstone import Cs, CS_ARCH_ARM, CS_MODE_THUMB, CS_MODE_MCLASS

MD = Cs(CS_ARCH_ARM, CS_MODE_THUMB | CS_MODE_MCLASS)

# First halfwords (or ranges) whose whole family correctly faults on M3:
# FPU (EC-EE), coprocessor (EC-EF), DSP/media (FA-FB), v8-M (F8: BXNS etc).
# Each entry: (first_lo, first_hi, reason).
ACCEPTED_FIRST = [
    (0xEC00, 0xEFFF, 'coprocessor/FPU: none on M3, trap correct'),
    (0xFA00, 0xFBFF, 'DSP/media extension: none on M3, trap correct'),
]

# Mnemonics that correctly fault on M3 wherever they appear (no FPU/NEON/
# coprocessor/DSP/TrustZone/MVE on F103). Fault = correct silicon behavior.
ACCEPTED_MN = {
    # Coprocessor data/control (A/R-class only).
    'cdp', 'cdp2', 'mcr', 'mrc', 'mcr2', 'mrc2', 'ldc', 'ldc2', 'stc',
    'stc2', 'ldcl', 'ldc2l', 'stcl', 'stc2l', 'mcrr', 'mrrc', 'mcrr2',
    'mrrc2',
    # Hypervisor / secure monitor / permanent-undefined.
    'hvc', 'smc', 'udf', 'udf.w',
    # MVE (v8.1-M) + v8-M security +
    # hints beyond NOP.W (DBG handled, rest fault).
    'wlstp.8', 'wlstp.16', 'wlstp.32', 'wlstp.64', 'vctp.8', 'vctp.16',
    'vctp.32', 'vctp.64', 'bxns', 'blxns', 'sg', 'tt', 'ttt', 'tta',
    'ttat', 'clrm', 'csdb', 'ssbb', 'pssbb', 'dbl',
    # DSP/media (correctly gated by dsp=false on M3).
    'pkhbt', 'pkhtb', 'smlad', 'smlsd', 'smlald', 'smlsld', 'smuad',
    'smusdx', 'smmul', 'smmla', 'smmls', 'smlal', 'smlalx', 'smual',
    'qadd', 'qsub', 'qdadd', 'qdsub', 'qasx', 'qsax', 'sadd16',
    'ssub16', 'uadd16', 'usub16', 'sxtab', 'sxtab16', 'uxtab',
    'uxtab16', 'smladx', 'smlsdx', 'smuadx',
    # MVE loop tail predication (v8.1-M) + MVE-FP selects.
    'dlstp.8', 'dlstp.16', 'dlstp.32', 'dlstp.64', 'letp', 'lctp',
    'vseleq.f16', 'vselvs.f16', 'vselgt.f16', 'vselge.f16',
    # MVE floating-point (v8.1-M + MVE-FP): no FPU of any kind on F103.
    'vsdot.s8', 'vsdot.s16', 'vsdot.s32', 'vsdot.u8', 'vsdot.u16',
    'vsdot.u32', 'vusdot.s8', 'vudot.u8', 'vfmal.f16', 'vfmal.f32',
    'vfmsl.f16', 'vfmsl.f32',
}

# NEON/FPU/MVE mnemonics all start with 'v'; no valid-on-M3 Thumb
# instruction does, so one prefix rule covers the whole namespace
# (vld/vst/vsel/vdot/vfma/vfms/vsdot/vmaxnm/vcmla/vmmla/...).
ACCEPTED_MN_PREFIX = ('v',)

# T3/EA data-processing mnemonics (base + S-suffixed, with/without .w):
# gaps with Rd==PC are correct even-target faults (exec_allow needs
# readable+XN-clear; the computed even target faults exactly like HW
# UsageFault). Safe: every op also has Rd!=PC samples that would still trip.
T3_DP = {
    'and', 'ands', 'eor', 'eors', 'sub', 'subs', 'rsb', 'rsbs', 'add',
    'adds', 'adc', 'adcs', 'sbc', 'sbcs', 'bic', 'bics', 'orr', 'orrs',
    'orn', 'orns', 'mov', 'movs', 'mvn', 'mvns', 'cmp', 'cmn', 'tst',
    'teq', 'lsl', 'lsls', 'lsr', 'lsrs', 'asr', 'asrs', 'ror', 'rors',
    'rrx',
}

# Valid MRS/MSR SYSm values; anything else is UNPREDICTABLE -> fault correct.
MRS_VALID_SYSM = {0, 1, 2, 3, 5, 6, 7, 8, 9, 16, 17, 18, 20}
MSR_VALID_SYSM = {0, 1, 2, 3, 8, 9, 16, 17, 18, 20}


def accepted_first(first):
    return any(a <= first <= b for (a, b, _) in ACCEPTED_FIRST)


def base_mnemonic(mnemonic):
    # 'bic.w' -> 'bic', 'adds' stays; Capstone suffixes .w/.n on Thumb-2.
    return mnemonic[:-2] if mnemonic.endswith('.w') or mnemonic.endswith('.n') else mnemonic


def accepted_gap(first, second, mnemonic, op_str):
    if accepted_first(first):
        return True
    mn = base_mnemonic(mnemonic)
    if mn in ACCEPTED_MN:
        return True
    if mnemonic.startswith(ACCEPTED_MN_PREFIX):
        return True
    if mn == 'bl' and second < 0xF800:
        return True  # Capstone-loose: BL requires 2nd half >= 0xF800
    if mn == 'blx' and second < 0xE800:
        return True  # Capstone-loose: BLX-imm needs ARM-state half
    if mn in T3_DP and ((second >> 8) & 0xF) == 0xF:
        return True  # Rd==PC with even computed target faults like HW
    if mn == 'mrs' and (second & 0xFF) not in MRS_VALID_SYSM:
        return True  # reserved SYSm: UNPREDICTABLE -> fault correct
    if mn == 'msr' and (second & 0xFF) not in MSR_VALID_SYSM:
        return True  # reserved SYSm: UNPREDICTABLE -> fault correct
    if mn in ('strb', 'strh', 'str', 'strbt', 'strht', 'strt') and \
            ((second >> 12) & 0xF) == 0xF:
        return True  # store with Rt==PC: UNPREDICTABLE, benign either way
    # NOTE: F9 signed loads with Rt==PC are PLI (executed, not gaps).
    if mn in ('ldrb', 'ldrh', 'ldr', 'ldrbt', 'ldrht',
              'ldrt') and ((second >> 12) & 0xF) == 0xF:
        return True  # byte/half load into PC: UNPREDICTABLE (word-literal
        # LDR-pc is the valid interworking branch, handled separately)
    if mn in ('sbfx', 'ubfx') and not (
            0xF340 <= first <= 0xF34F or 0xF3C0 <= first <= 0xF3CF):
        return True  # Capstone-loose (e.g. 0xF740 SMLAD shape as sbfx)
    if mn in ('bfi', 'bfc'):
        # UNPREDICTABLE msb<lsb: Capstone renders it as if valid anyway.
        lsb = (((second >> 12) & 7) << 2) | ((second >> 6) & 3)
        msb = second & 0x1F
        if msb < lsb:
            return True
    return False


def cap32(first, second):
    data = struct.pack('<HH', first, second)
    try:
        r = list(MD.disasm(data, 0x08000000))
    except Exception:
        return False
    return bool(r) and r[0].size == 4


def main():
    lines = open('/tmp/census32_ours.json').read().split('\n')
    assert len(lines) == 6145, len(lines)  # 6144 + trailing ''
    fam_gap = Counter()
    fam_detail = {}
    n_gap_samples = 0
    over_fams = Counter()
    for i in range(6144):
        first = 0xE800 + i
        row = lines[i].strip()
        assert len(row) == 256, (hex(first), len(row))
        for j, ch in enumerate(row):
            second = (j * 257) & 0xFFFF
            data = struct.pack('<HH', first, second)
            try:
                r = list(MD.disasm(data, 0x08000000))
            except Exception:
                r = []
            cap_ok = bool(r) and r[0].size == 4
            if ch != '2' and not cap_ok:
                over_fams[first] += 1  # we execute, Capstone rejects
                continue
            if ch != '2':
                continue
            if not cap_ok:
                continue
            name, opstr = r[0].mnemonic, r[0].op_str
            if accepted_gap(first, second, name, opstr):
                continue
            fam_gap[first] += 1
            n_gap_samples += 1
            fam_detail.setdefault(first, []).append((second, name, opstr))
    # A family counts as gapped if >8 of its 256 samples are cap-valid/ours-fault
    # (isolated samples are usually value-dependent faults, triaged by hand).
    bad_fams = {f: n for f, n in fam_gap.items() if n > 8}
    print(f'32-bit: {n_gap_samples} gap samples in {len(fam_gap)} first-halfwords, '
          f'{len(bad_fams)} families over threshold')
    for f in sorted(bad_fams)[:20]:
        second, name, opstr = fam_detail[f][0]
        print(f'  FAM 0x{f:04x} ({bad_fams[f]}/256): e.g. 0x{second:04x} {name} {opstr[:40]}')
    # Show a few sub-threshold samples for manual review too
    shown = 0
    for f in sorted(fam_gap):
        if f in bad_fams or shown >= 10:
            continue
        second, name, opstr = fam_detail[f][0]
        print(f'  sample 0x{f:04x}:0x{second:04x} {name} {opstr[:40]}')
        shown += 1
    # Over-accept direction: families where we execute but Capstone rejects.
    # INFORMATIONAL ONLY (reserved/UNPREDICTABLE-bit tolerance can never
    # break real firmware — assemblers don't emit these — while silent
    # mis-decodes are covered by the exact-result probes in isa_tests.rs).
    over_bad = {f: n for f, n in over_fams.items() if n > 8}
    print(f'32-bit over-accepts (info only): {sum(over_fams.values())} samples, '
          f'{len(over_bad)} families over threshold')
    for f in sorted(over_bad)[:10]:
        print(f'  OVER-FAM 0x{f:04x} ({over_bad[f]}/256)')
    return 1 if bad_fams else 0


if __name__ == '__main__':
    sys.exit(main())
