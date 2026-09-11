#!/usr/bin/env python3
"""Differential fuzz: our Rust CPU vs Unicorn (test-only oracle, never shipped).

Usage:
  cargo test --release --lib cpu::census   # refresh /tmp/census*_ours.json
  python3 tests/fuzz_diff.py [--cases N] [--seed S]

Protocol v2: generate N cases (single insns from the census dumps with
constrained regs/flags, plus track-2 multi-step shapes: IT+payload pairs,
LDREX/STREX pairs, 2-ALU chains) -> /tmp/fuzz_cases.txt (`ncode h* regs
sp lr xpsr it steps`) -> `cargo test --release --lib cpu::diffuzz` with
FUZZ_CASES/FUZZ_OUT (executes on our core) -> execute the same cases on
Unicorn (Cortex-M-ish Thumb, MPU off) -> compare regs/PC/xPSR-NZCVQ-T/
memory-hash/fault-bit.

Deliberately excluded (divergent by design, covered elsewhere):
  SVC/BKPT/UDF (trap mechanisms differ; census buckets them), WFI/WFE/SEV
  (sleep/event semantics), CPS/MSR/MRS-PRIMASK (invisible side effects are
  trivially equal; APSR forms INCLUDED), SETEND/CPS underground, DSP/FPU/
  copro (we fault, oracle executes; census proves the fault), hints except
  NOP/YIELD/DMB/DSB/ISB/CLREX/DBG/PLD.
  Exceptions structurally (stacking, nesting order, EXC_RETURN misuse,
  SysTick debt) are NOT fuzzable here: the oracle runs generic ARM
  (UC_ARCH_ARM, no M-profile stacking — a cortex-m3 bring-up was probed
  and dropped as binding-fragile). They are covered natively instead:
  src/cpu/core_tests.rs (WFI, nesting, SVC balance, PSP switch, same-prio
  non-nesting, priority order, bad EXC_RETURN, debt re-pend) plus the
  periph39 SVC/PendSV and mini_rtos PendSV firmware proofs.
Exit 0 on full agreement; exit 1 listing divergences for triage.
"""
import os
import random
import struct
import subprocess
import sys

CASES_PATH = '/tmp/fuzz_cases.txt'
OUT_PATH = '/tmp/fuzz_ours.txt'

# xPSR compare mask: NZCVQ only. T lives at bit 24 in our xPSR but bit 5
# in ARM CPSR (a copy-paste of the value once ran the whole fuzz in ARM
# state!); both sides hold T=1 by construction. ITSTATE via model below.
XPSR_MASK = 0xF8000000


def load_census():
    ops16, ops32 = [], []
    d16 = open('/tmp/census16_ours.json').read().strip()
    assert len(d16) == 65536
    for op in range(65536):
        if d16[op] == '0':
            ops16.append(op)
    lines = open('/tmp/census32_ours.json').read().split('\n')
    for i in range(6144):
        first = 0xE800 + i
        row = lines[i].strip()
        for j, ch in enumerate(row):
            if ch == '0':
                ops32.append((first, (j * 257) & 0xFFFF))
    return ops16, ops32


def skip_op(first, second):
    """Opcodes excluded from fuzzing (divergent by design)."""
    if second is None:
        o = first
        if 0xBE00 <= o <= 0xDFFF:  # BKPT/SVC/UDF
            return True
        if o in (0xBF20, 0xBF30, 0xBF40):  # WFI/WFE/SEV
            return True
        if 0xB660 <= o <= 0xB67F:  # CPS
            return True
        if 0xB650 <= o <= 0xB657:  # SETEND
            return True
        return False
    # 32-bit: MSR/MRS special-reg forms (banking shadows differ invisibly;
    # APSR forms are fine but indistinguishable here -> skip the family).
    if 0xF380 <= first <= 0xF3EF:
        return True
    return False


def rand_reg(rng):
    # All even (word-aligned): Unicorn faults unaligned word accesses while
    # our model (like silicon with UNALIGN_TRP=0) assembles bytes. LR stays
    # odd (return addresses); pop-pc/ldr-pc targets come from odd pattern
    # words, so taken branches are still covered on both sides.
    r = rng.random()
    if r < 0.5:
        return rng.randint(0, 0xFF) & ~1  # small int, even
    if r < 0.8:
        return (0x20000000 + rng.randrange(0, 0x10000)) & ~1  # RAM, even
    return (0x08000000 + rng.randrange(0, 0x10000)) & ~1  # flash, even


T3_EA_DP = {
    'and', 'ands', 'eor', 'eors', 'sub', 'subs', 'rsb', 'rsbs', 'add',
    'adds', 'adc', 'adcs', 'sbc', 'sbcs', 'bic', 'bics', 'orr', 'orrs',
    'orn', 'orns', 'mov', 'movs', 'mvn', 'mvns', 'cmp', 'cmn', 'tst',
    'teq', 'lsl', 'lsls', 'lsr', 'lsrs', 'asr', 'asrs', 'ror', 'rors',
    'rrx',
}


def is_rdpc_dataproc(first, second):
    """T3/EA data-proc and MOVW/MOVT with Rd==PC: computed even targets
    fault-correctly on both sides, but odd targets diverge (we set PC,
    Unicorn faults on unmapped fetch). Undecidable without ALU sim:
    resample at generation."""
    import struct
    from capstone import Cs, CS_ARCH_ARM, CS_MODE_THUMB
    md = Cs(CS_ARCH_ARM, CS_MODE_THUMB)
    data = struct.pack('<H', first)
    if second is not None:
        data += struct.pack('<H', second)
    try:
        r = list(md.disasm(data, 0x20002000))
    except Exception:
        return False
    if not r:
        return False
    mn = r[0].mnemonic
    mn = mn[:-2] if mn.endswith('.w') or mn.endswith('.n') else mn
    if mn not in T3_EA_DP and mn not in ('movw', 'movt'):
        return False
    op2 = second if second is not None else 0
    return ((op2 >> 8) & 0xF) == 0xF


def skip_oracle_limits(first, second):
    """Encodings this Unicorn build faults that silicon (and our core)
    execute: 32-bit LDM/STM with writeback to Rn!=SP, and any single-
    register list. (PUSH.W/POP.W via SP work fine.) Resample at generation;
    the logic itself is probe-verified (ldmdb_forms) and firmware-exercised.
    Also: STREX (any form) — this Unicorn build faults INSN_INVALID on a
    lone STREX (no reservation possible in a 1-insn case); silicon fails
    the store with status 1 instead, which our LDREX/STREX probes verify.
    And single-transfer writeback-to-Rt (Rn==Rt with `!`/post-index):
    UNPREDICTABLE, oracle keeps the loaded value, we write back.
    """
    import struct
    from capstone import Cs, CS_ARCH_ARM, CS_MODE_THUMB
    md = Cs(CS_ARCH_ARM, CS_MODE_THUMB)
    data = struct.pack('<H', first)
    if second is not None:
        data += struct.pack('<H', second)
    try:
        r = list(md.disasm(data, 0x20002000))
    except Exception:
        return False
    if not r:
        return False
    mn = r[0].mnemonic
    mn = mn[:-2] if mn.endswith('.w') or mn.endswith('.n') else mn
    if mn.startswith('strex'):
        return True
    # Bitfield unit (SBFX/UBFX/BFI/BFC): this Unicorn faults INSN_INVALID
    # on all of them (verified incl. plain sbfx/ubfx/bfi/bfc); our
    # implementations are probe-verified (bitfield_forms etc.).
    if mn in ('sbfx', 'ubfx', 'bfi', 'bfc'):
        return True
    if mn in ('ldm', 'stm', 'ldmia', 'stmia', 'ldmdb', 'stmdb',
              'ldmib', 'stmib', 'ldmda', 'stmda', 'push', 'pop'):
        return False
    try:
        if mn in ('push', 'pop'):
            lst = r[0].op_str.split('{')[1].split('}')[0]
            return lst.count(',') == 0  # single-reg list faults on oracle
        parts = r[0].op_str.split()
        rn = parts[0].rstrip('!,')
        wb = '!' in parts[0]
        if rn == 'sp':
            lst = r[0].op_str.split('{')[1].split('}')[0]
            return lst.count(',') == 0
        if wb:
            return True
        lst = r[0].op_str.split('{')[1].split('}')[0]
        return lst.count(',') == 0
    except (IndexError, ValueError, KeyError):
        return False


def skip_excl_rt_pc(first, second):
    """LDREX/STREX with Rt==PC (o2[15:12]==0xF), or STREX status to PC
    (o2[11:8]==0xF): UNPREDICTABLE. The oracle faults; we execute
    (STREX-fail status / LDREX load). Structural check (not capstone's
    mnemonic — capstone mis-decodes some of these shapes, e.g. E842/F2F2
    as `ttat`, which sailed past the strex resample and diverged on
    seed 3). Resample; valid forms are probe-verified (ldrex_strex_forms).
    """
    if second is None:
        return False
    if (first & 0xFFF0) not in (0xE840, 0xE850, 0xE8D0, 0xE8C0):
        return False
    if ((second >> 12) & 0xF) == 0xF:
        return True
    if (first & 0xFFF0) in (0xE840, 0xE8C0) and ((second >> 8) & 0xF) == 0xF:
        return True
    return False


def skip_ldm_stm_rn_list(first, second):
    """32-bit LDM/STM with writeback whose Rn is in its own register list
    (W=o1[5], Rn=o1[3:0], list=o2): UNPREDICTABLE. The oracle faults
    INSN_INVALID; we execute (stores keep the original value, loads fault
    per the LDM-Rn-list-WB rule — both silicon-plausible and probe-noted
    in the decoder). Resample; valid forms are probe-verified
    (ldmdb_forms) and firmware-exercised. Structural, not mnemonic: seed 6
    hit `stm.w r0!,{r0,r3,r4,r8,r11,r12}` clean through the mnemonic-based
    oracle-limit check, which returns False for every ldm/stm spelling by
    construction. Rn==SP (PUSH.W/POP.W) is oracle-verified working and
    keeps flowing; 16-bit STMIA/LDMIA never diverged in ~4K cases and
    stays sampled.
    """
    if second is None:
        return False
    if (first & 0xF000) != 0xE000:
        return False
    if (first & 0x0E40) != 0x0800:  # o1[11:9]==100, bit6==0 (not STRD/LDRD)
        return False
    if not (first & 0x0020):  # W
        return False
    rn = first & 0x000F
    if rn == 0xD:
        return False
    return bool((second >> rn) & 1)


def skip_wb_to_rt(first, second):
    """Single-transfer writeback-to-Rt (Rn==Rt with pre-index `!` or
    post-index): UNPREDICTABLE — the oracle keeps the loaded value while
    we write back the address (differential fuzz: ldrsh r5,[r5,#-x]!).
    Resample at generation; offset-mode Rn==Rt (no writeback) is fine."""
    import struct
    from capstone import Cs, CS_ARCH_ARM, CS_MODE_THUMB
    md = Cs(CS_ARCH_ARM, CS_MODE_THUMB)
    data = struct.pack('<H', first)
    if second is not None:
        data += struct.pack('<H', second)
    try:
        r = list(md.disasm(data, 0x20002000))
    except Exception:
        return False
    if not r or '[' not in r[0].op_str:
        return False
    mn = r[0].mnemonic
    mn = mn[:-2] if mn.endswith('.w') or mn.endswith('.n') else mn
    if mn not in ('ldr', 'ldrb', 'ldrh', 'ldrsb', 'ldrsh', 'str', 'strb',
                  'strh', 'ldrd', 'strd'):
        return False
    ops = r[0].op_str
    wb = '!' in ops or '],' in ops
    if not wb:
        return False
    try:
        dest = ops.split(',')[0].strip()
        inside = ops.split('[')[1].split(']')[0]
        base = inside.split(',')[0].strip().rstrip('!')
        return dest == base
    except (IndexError, ValueError, KeyError):
        return False


def constrain_memory_case(first, second, regs, sp):
    """Force memory-op base/index regs into mapped RAM so effective addresses
    stay mapped on both sides (mutates regs in place). Returns False if the
    op isn't recognized (keep case as-is, triage decides)."""
    import struct
    from capstone import Cs, CS_ARCH_ARM, CS_MODE_THUMB
    md = Cs(CS_ARCH_ARM, CS_MODE_THUMB)
    data = struct.pack('<H', first)
    if second is not None:
        data += struct.pack('<H', second)
    try:
        r = list(md.disasm(data, 0x20002000))
    except Exception:
        return False
    if not r:
        return False
    m, ops = r[0].mnemonic, r[0].op_str
    m = m[:-2] if m.endswith('.w') or m.endswith('.n') else m
    # LDM/STM/POP/PUSH (no brackets in disassembly): force Rn into RAM so
    # the multi-word span stays mapped on both sides.
    if m in ('ldm', 'stm', 'ldmia', 'stmia', 'ldmdb', 'stmdb', 'ldmib',
             'stmib', 'ldmda', 'stmda'):
        try:
            rn = ops.split()[0].rstrip('!,')
            ri = {'r0': 0, 'r1': 1, 'r2': 2, 'r3': 3, 'r4': 4, 'r5': 5,
                  'r6': 6, 'r7': 7, 'r8': 8}.get(rn)
            if ri is not None:
                regs[ri] = 0x20003000
                return True
        except (IndexError, ValueError, KeyError):
            pass
        return False
    if '[' not in ops:
        return False
    try:
        inside = ops.split('[')[1].split(']')[0]
        parts = [x.strip() for x in inside.split(',')]
        ridx = {'r0': 0, 'r1': 1, 'r2': 2, 'r3': 3, 'r4': 4, 'r5': 5,
                'r6': 6, 'r7': 7, 'r8': 8}
        if parts[0] == 'pc' or parts[0] == 'sp':
            base_ok = True
        elif parts[0].rstrip('!') in ridx:
            # Low RAM band: +/-4K immediates and DB reads stay mapped.
            regs[ridx[parts[0].rstrip('!')]] = 0x20001000 + (regs[ridx[parts[0].rstrip('!')]] % 0xD000) & ~3
            base_ok = True
        else:
            return False
        # Register offset/index: force small so base+index stays mapped.
        for p in parts[1:]:
            p = p.strip()
            if p in ridx:
                regs[ridx[p]] &= 0xFF
            elif 'lsl' in p or 'lsr' in p or 'asr' in p:
                rn = p.split()[0]
                if rn in ridx:
                    regs[ridx[rn]] &= 0xFF
        return base_ok
    except (IndexError, ValueError, KeyError):
        return False


def gen_cases(n, seed):
    rng = random.Random(seed)
    ops16, ops32 = load_census()
    track2_selfcheck()
    cases = []
    guard = 0
    while len(cases) < n and guard < n * 20:
        guard += 1
        roll = rng.random()
        if roll < 0.45:
            case = gen_single(rng, ops16, ops32)
        elif roll < 0.60:
            case = gen_itpair(rng)
        elif roll < 0.70:
            case = gen_ldstpair(rng)
        elif roll < 0.85:
            case = gen_aluchain(rng)
        else:
            case = gen_branchpair(rng)
        if case is None:
            continue
        cases.append(case)
    return cases


def gen_single(rng, ops16, ops32):
    """One instruction, one step (track 1, unchanged semantics)."""
    if rng.random() < 0.6:
        op = rng.choice(ops16)
        if skip_op(op, None):
            return None
        first, second = op, None
    else:
        first, second = rng.choice(ops32)
        if skip_op(first, second):
            return None
    # T3/EA data-proc with Rd==PC: target needs ALU sim to triage;
    # resample instead (control-flow unmapped targets skip cleanly).
    if is_rdpc_dataproc(first, second):
        return None
    # Oracle limits (faults valid encodings): resample.
    if skip_oracle_limits(first, second):
        return None
    # UNPREDICTABLE exclusive-into-PC: resample (structural, not mnemonic).
    if skip_excl_rt_pc(first, second):
        return None
    # UNPREDICTABLE LDM/STM writeback with Rn in its own list: resample
    # (structural — the mnemonic oracle-limit check can't see it).
    if skip_ldm_stm_rn_list(first, second):
        return None
    # UNPREDICTABLE writeback-to-Rt: resample.
    if skip_wb_to_rt(first, second):
        return None
    regs = [rand_reg(rng) for _ in range(13)]
    # Unaligned bases are fine on both sides (both byte-assemble;
    # oracle-verified for word/half/byte): relax the even-only rule for
    # single-transfer bases (never SP).
    relax_alignment(rng, first, second, regs)
    sp = 0x20000000 + rng.randrange(0x1000, 0xF000) & ~7
    lr = rng.choice([0x08000001, 0x20000081, rng.randint(0, 0xFF)])
    if lr & 0x0FFFFFF0 == 0x0FFFFFF0:
        lr ^= 0x100  # keep clear of EXC_RETURN patterns
    nzcvq = rng.randrange(0, 32) << 27
    xpsr = 0x01000000 | nzcvq
    # ITSTATE always 0 at setup (Unicorn mishandles stop-count with a
    # preset IT block; predicated continuation is covered by hand
    # probes + track-2 IT pairs). Bare `it` instructions still occur
    # and their state-setting is model-checked exactly below.
    it = [0, 0, 0, 0]
    # Pin memory-op bases into mapped RAM (and reg-indices small) so
    # effective addresses stay mapped on both sides.
    constrain_memory_case(first, second, regs, sp)
    code = [first] if second is None else [first, second]
    return (code, regs, sp, lr, xpsr, it, 1)


ALU_KINDS = ['lsl_imm5', 'adds_reg', 'subs_reg', 'adds_imm3', 'subs_imm3',
             'movs_imm8', 'cmp_imm8', 'ands', 'eors', 'adcs', 'sbcs', 'orrs']


def alu16(kind, rd, rn, rm, imm):
    """Encode a 16-bit ALU op (r0-r7 only). Verified against Capstone by
    track2_selfcheck; the fuzzer never emits anything else here."""
    if kind == 'lsl_imm5':
        return 0x0000 | (imm & 31) << 6 | (rn & 7) << 3 | (rd & 7)
    if kind == 'adds_reg':
        return 0x1800 | (rm & 7) << 6 | (rn & 7) << 3 | (rd & 7)
    if kind == 'subs_reg':
        return 0x1A00 | (rm & 7) << 6 | (rn & 7) << 3 | (rd & 7)
    if kind == 'adds_imm3':
        return 0x1C00 | (imm & 7) << 6 | (rn & 7) << 3 | (rd & 7)
    if kind == 'subs_imm3':
        return 0x1E00 | (imm & 7) << 6 | (rn & 7) << 3 | (rd & 7)
    if kind == 'movs_imm8':
        return 0x2000 | (rd & 7) << 8 | (imm & 0xFF)
    if kind == 'cmp_imm8':
        return 0x2800 | (rn & 7) << 8 | (imm & 0xFF)
    base = {'ands': 0x4000, 'eors': 0x4040, 'adcs': 0x4140,
            'sbcs': 0x4180, 'orrs': 0x4300}[kind]
    return base | (rm & 7) << 3 | (rd & 7)


def alu_dest(kind):
    """Does this kind write a register (vs flags-only CMP)?"""
    return kind != 'cmp_imm8'


def track2_selfcheck():
    """One-time Capstone check that the ALU encoder + LDREX/STREX shapes
    decode as intended (a bad encoder would fail every track-2 case)."""
    import struct
    from capstone import Cs, CS_ARCH_ARM, CS_MODE_THUMB
    md = Cs(CS_ARCH_ARM, CS_MODE_THUMB)
    for kind in ALU_KINDS:
        op = alu16(kind, 1, 2, 3, 0x55)
        r = list(md.disasm(struct.pack('<H', op), 0x20002000))
        assert r and r[0].size == 2, (kind, hex(op))
    for h1, h2 in [(0xE852, 0x0F00), (0xE842, 0x0100)]:
        r = list(md.disasm(struct.pack('<HH', h1, h2), 0x20002000))
        assert r and r[0].size == 4, (hex(h1), hex(h2))
    assert list(md.disasm(struct.pack('<H', 0xBF08), 0x20002000))[0].mnemonic == 'it'


def any_regs(rng):
    regs = [rng.randint(0, 0xFFFFFFFF) for _ in range(13)]
    sp = 0x20000000 + rng.randrange(0x1000, 0xF000) & ~7
    lr = rng.choice([0x08000001, 0x20000081, rng.randint(0, 0xFF)])
    if lr & 0x0FFFFFF0 == 0x0FFFFFF0:
        lr ^= 0x100
    xpsr = 0x01000000 | (rng.randrange(0, 32) << 27)
    return regs, sp, lr, xpsr


def gen_itpair(rng):
    """`it <cond>` + predicable 16-bit ALU payload(s) (+ NOP pads).
    Tests predicated flag updates: payloads must set flags (and C!)
    only when their slot condition holds — the it_suppress path both
    sides. Masks are free (multi-slot ITSTATE executes correctly when
    set by a real IT insn; only *preset* multi-slot ITSTATE is broken
    in the oracle, and we never preset). Oracle counting quirk
    (verified): a *skipped* payload advances PC without consuming a
    stop, so joint counts overrun into the pads past skipped payloads
    (taken stops exactly). The NOP pads absorb the overrun
    deterministically (NOPs never fault, touch no state); the
    comparator skips r15 for pairs (counting semantics, not decoder
    behavior) and checks regs/flags/mem exactly. Our side runs exact
    steps from zero ITSTATE."""
    cond = rng.randrange(0, 14)
    if rng.random() < 0.7:
        mask = 0x8
        npay, npad, steps = 1, 2, 2
    else:
        mask = rng.randrange(1, 16)
        npay, npad, steps = 2, 4, 3
    it = 0xBF00 | (cond << 4) | mask
    code = [it]
    for _ in range(npay):
        kind = rng.choice(ALU_KINDS)
        rd, rn, rm = (rng.randrange(0, 8) for _ in range(3))
        imm = rng.choice([rng.randrange(0, 256), rng.randrange(0, 8),
                          rng.randrange(1, 32)])
        code.append(alu16(kind, rd, rn, rm, imm))
    code += [0xBF00] * npad  # NOP pads absorb the stop-count overrun
    regs, sp, lr, xpsr = any_regs(rng)
    return (code, regs, sp, lr, xpsr, [0, 0, 0, 0], steps)


def gen_ldstpair(rng):
    """LDREX r0,[r2] + STREX r1,r0,[r2] (2 steps, mapped word). Tests the
    exclusive monitor across instructions; both sides must succeed with
    status 0 and land the store. (Byte/half pairs arrive with the
    LDREXB/STREXB backlog fix.)"""
    regs, sp, lr, xpsr = any_regs(rng)
    regs[2] = 0x20004000
    regs[0] = rng.randint(0, 0xFFFFFFFF)
    regs[1] = rng.randint(0, 0xFFFFFFFF)
    return ([0xE852, 0x0F00, 0xE842, 0x0100], regs, sp, lr, xpsr,
            [0, 0, 0, 0], 2)


def gen_aluchain(rng):
    """Two chained 16-bit ALU ops (2 steps): ALU2 reads ALU1's Rd (and
    often its flags via ADC/SBC), testing flag chaining across steps."""
    k1 = rng.choice(ALU_KINDS)
    k2 = rng.choice(ALU_KINDS)
    rd1 = rng.randrange(0, 8)
    a, b = (rng.randrange(0, 8) for _ in range(2))
    i1 = rng.choice([rng.randrange(0, 256), rng.randrange(0, 8),
                     rng.randrange(1, 32)])
    i2 = rng.choice([rng.randrange(0, 256), rng.randrange(0, 8),
                     rng.randrange(1, 32)])
    ins1 = alu16(k1, rd1, a, b, i1)
    # Force the chain half the time (as Rn for reg/imm forms, as Rdn for
    # data-proc forms where Rn==Rd field; CMP chains flags+reg anyway).
    if alu_dest(k1) and rng.random() < 0.5:
        d2 = n2 = rd1
    else:
        d2, n2 = rng.randrange(0, 8), rng.randrange(0, 8)
    ins2 = alu16(k2, d2, n2, b, i2)
    regs, sp, lr, xpsr = any_regs(rng)
    return ([ins1, ins2], regs, sp, lr, xpsr, [0, 0, 0, 0], 2)


NOP = 0xBF00


def gen_branchpair(rng):
    """A conditional/unconditional branch with a small MAPPED target plus
    explicit NOP landing pads (2+ steps: branch + landing). Tests cond
    evaluation (Bcond), CBZ/CBNZ, and wide-branch offset math
    differentially — branches used to resample/skip entirely. Every
    landing zone is padded with NOPs so step 2+ never runs into pattern
    bytes; backward branches land back inside the pads (budgets cap
    execution, no loops). Any fault or value divergence here is REAL
    (no triage)."""
    kind = rng.random()
    if kind < 0.3:
        # B<cond>.N forward +16 (imm8=8: off=imm8*2; target index 10).
        cond = rng.randrange(0, 14)
        code = [0xD000 | (cond << 8) | 0x08] + [NOP] * 10
        steps = 2
    elif kind < 0.45:
        # B.N forward +16, unconditional (landing index 10).
        code = [0xE008] + [NOP] * 10
        steps = 2
    elif kind < 0.6:
        # B.N backward -16 (imm8=-8=0xF8): branch sits at index 6
        # (addr +12), lands on index 0.
        code = [NOP] * 6 + [0xE000 | 0xF8]
        steps = 8
    elif kind < 0.75:
        # CBZ/CBNZ r<n>, +16 (imm5=8: off=imm5*2; target index 10).
        rn = rng.randrange(0, 8)
        cbnz = rng.random() < 0.5
        code = [(0xB900 if cbnz else 0xB100) | (8 << 3) | rn] + [NOP] * 10
        steps = 2
    elif kind < 0.9:
        # Bcc.W forward +0x20 (S=0,J=0,imm6=0,imm11=0x10; bit12=0 valid
        # conditional shape; target index 18).
        cond = rng.randrange(0, 14)
        code = [0xF000 | (cond << 6), 0x8010] + [NOP] * 17
        steps = 2
    else:
        # Bcc.W backward -0x10 (S=1,J=1,imm6=0x3F,imm11=0x7F8; branch at
        # index 6-7, lands on index 0).
        cond = rng.randrange(0, 14)
        code = [NOP] * 6 + [0xF000 | (1 << 10) | (cond << 6) | 0x3F, 0xAFF8]
        steps = 8
    regs, sp, lr, xpsr = any_regs(rng)
    return (code, regs, sp, lr, xpsr, [0, 0, 0, 0], steps)


def relax_alignment(rng, first, second, regs):
    """Allow odd bases for single-transfer memory ops (both sides
    byte-assemble unaligned word/half/byte accesses identically,
    oracle-verified). SP stays word-aligned; literals are fixed."""
    import struct
    from capstone import Cs, CS_ARCH_ARM, CS_MODE_THUMB
    md = Cs(CS_ARCH_ARM, CS_MODE_THUMB)
    data = struct.pack('<H', first)
    if second is not None:
        data += struct.pack('<H', second)
    try:
        r = list(md.disasm(data, 0x20002000))
    except Exception:
        return
    if not r or '[' not in r[0].op_str:
        return
    mn = r[0].mnemonic
    mn = mn[:-2] if mn.endswith('.w') or mn.endswith('.n') else mn
    if mn not in ('ldr', 'ldrb', 'ldrh', 'ldrsb', 'ldrsh', 'str', 'strb',
                  'strh', 'ldrd', 'strd'):
        return
    try:
        inside = r[0].op_str.split('[')[1].split(']')[0]
        breg = inside.split(',')[0].strip().rstrip('!')
        if breg.startswith('r') and breg not in ('sp',):
            idx = int(breg[1:])
            if 0 <= idx <= 12 and rng.random() < 0.5:
                regs[idx] |= 1
    except (ValueError, IndexError):
        pass


def write_cases(cases):
    # NOTE: no header/comment lines — fuzz_divs.txt references cases by
    # raw line number, so every line must be a case.
    with open(CASES_PATH, 'w') as f:
        for (code, regs, sp, lr, xpsr, it, steps) in cases:
            f.write(f'{len(code)} ' + ' '.join(f'{h:04X}' for h in code)
                    + ' ' + ' '.join(f'{r:08X}' for r in regs)
                    + f' {sp:08X} {lr:08X} {xpsr:08X} '
                    + f'{it[0]:02X} {it[1]:02X} {it[2]:02X} {it[3]:02X} {steps}\n')


def run_ours():
    env = dict(os.environ, FUZZ_CASES=CASES_PATH, FUZZ_OUT=OUT_PATH)
    subprocess.run(['cargo', 'test', '--release', '--lib', 'cpu::diffuzz'],
                   env=env, check=True, capture_output=True)
    out = []
    for line in open(OUT_PATH):
        out.append(line.split())
    return out


def pattern_byte(addr):
    i = addr & 0xFFFF
    base = 0x08000000 if addr < 0x20000000 else 0x20000000
    return (((base + i) ^ (i >> 8)) & 0xFF) | 1


def pattern_word(addr):
    return (pattern_byte(addr) | (pattern_byte(addr + 1) << 8)
            | (pattern_byte(addr + 2) << 16) | (pattern_byte(addr + 3) << 24))


def mapped(addr):
    return ((0x08000000 <= addr < 0x08010000)
            or (0x20000000 <= addr < 0x20010000))


def branch_target_mapped(first, second, regs, sp):
    """For control-flow divergences: recompute the branch target from inputs
    (Capstone-decoded) and report whether it is mapped. Returns True (mapped,
    so the divergence is REAL), False (unmapped: ours sets PC silently while
    Unicorn prefetch-faults — expected), or None (not control flow)."""
    import struct
    from capstone import Cs, CS_ARCH_ARM, CS_MODE_THUMB
    md = Cs(CS_ARCH_ARM, CS_MODE_THUMB)
    data = struct.pack('<H', first)
    if second is not None:
        data += struct.pack('<H', second)
    try:
        r = list(md.disasm(data, 0x20002000))
    except Exception:
        return None
    if not r:
        return None
    ins = r[0]
    m, ops = ins.mnemonic, ins.op_str
    bm = m[:-2] if m.endswith('.w') or m.endswith('.n') else m
    # Direct branches (b/bl/bcc, 16- and 32-bit): target in op_str.
    if bm == 'b' or bm == 'bl' or (bm.startswith('b') and len(bm) == 3
                                   and bm not in ('bx', 'blx', 'bic',
                                                  'bfi', 'bfc')):
        try:
            # Capstone 5.x prefixes targets with '#' (6.x prints bare).
            return mapped(int(ops.split(',')[0].strip().lstrip('#'), 16))
        except ValueError:
            return None
    if m in ('bx', 'blx'):
        rn = ops.strip()
        idx = {'r0': 0, 'r1': 1, 'r2': 2, 'r3': 3, 'r4': 4, 'r5': 5,
               'r6': 6, 'r7': 7, 'r8': 8, 'r9': 9, 'r10': 10, 'r11': 11,
               'r12': 12, 'sp': 13, 'lr': 14, 'pc': 15}.get(rn)
        if idx is None:
            return None
        v = [regs[i] if i < 13 else (sp if i == 13 else 0) for i in range(16)]
        if idx == 15:
            v[15] = 0x20002004
        # Even target: we fault like silicon (INTSTATE UsageFault — no ARM
        # state on Cortex-M); the oracle jumps. Divergent by design, same
        # class as the census even-target over-accepts (compilers only emit
        # odd targets).
        if v[idx] & 1 == 0:
            return False
        return mapped(v[idx] & ~1)
    if bm in ('mov', 'add'):
        # 16-bit high-reg MOV/ADD with Rd==PC (e.g. mov pc, r4): even
        # source faults on our side like HW (branch() to ARM state);
        # the oracle jumps. Same design divergence as bx/blx above.
        # (32-bit Rd==PC data-proc resamples at generation.)
        try:
            parts = [p.strip() for p in ops.split(',')]
            if len(parts) != 2 or parts[0] != 'pc':
                return None
            src = parts[1]
            idx = {'r0': 0, 'r1': 1, 'r2': 2, 'r3': 3, 'r4': 4, 'r5': 5,
                   'r6': 6, 'r7': 7, 'r8': 8, 'r9': 9, 'r10': 10,
                   'r11': 11, 'r12': 12, 'sp': 13, 'lr': 14,
                   'pc': 15}.get(src)
            if idx is None:
                return None
            v = regs[idx] if idx < 13 else (sp if idx == 13 else 0)
            if idx == 15:
                v = 0x20002004
            if v & 1 == 0:
                return False
            return mapped(v & ~1)
        except (IndexError, ValueError, KeyError):
            return None
    if bm in ('pop', 'ldm', 'ldmia', 'ldmdb', 'ldmib', 'ldmda', 'vldmia'):
        if 'pc' not in ops:
            return None
        # Reglist low->high; SP/Rn advances 4 per entry; pc is last.
        items = [x.strip() for x in ops.split('{')[1].split('}')[0].split(',')]
        base = sp
        if m.startswith('ldm'):
            try:
                rn = ops.split()[0].rstrip('!,')
                if rn == 'sp':
                    base = sp
                else:
                    base = regs[{'r0': 0, 'r1': 1, 'r2': 2, 'r3': 3, 'r4': 4,
                                 'r5': 5, 'r6': 6, 'r7': 7, 'r8': 8}[rn]]
            except (IndexError, ValueError, KeyError):
                pass
        addr = (base + 4 * (len(items) - 1)) & 0xFFFFFFFF
        return mapped(pattern_word(addr) & ~1)
    if bm in ('ldr', 'ldrh', 'ldrb', 'ldrsh', 'ldrsb', 'ldrt', 'ldrbt',
              'ldrht', 'tbh', 'tbb', 'ldrd', 'ldrsht', 'ldrsbt'):
        # Only interesting with Rt==pc (loads) or any tbb/tbh. LDRD can
        # carry pc in either slot (`ldrd r4, pc, [...]`); F9 signed
        # T-forms (`ldrsht pc`, `ldrsbt pc`) are genuine interworking
        # loads, not PLI (the oracle branches immediately where we
        # advance-then-fault-on-fetch for unmapped targets).
        dests = [p.split('[')[0].strip() for p in ops.split(',')[:2]]
        if bm in ('tbh', 'tbb') or 'pc' in ops.split(',')[0] or (bm == 'ldrd' and 'pc' in dests):
            if 'pc' in ops and '[' not in ops:
                return None
            # Approximate: resolve base+index via regs, read pattern word.
            try:
                inside = ops.split('[')[1].split(']')[0]
                parts = [x.strip() for x in inside.split(',')]
                base_reg = parts[0]
                if base_reg == 'pc':
                    baddr = 0x20002004
                elif base_reg == 'sp':
                    baddr = sp
                else:
                    try:
                        baddr = regs[int(base_reg[1:])] if base_reg.startswith('r') else None
                    except (ValueError, IndexError):
                        baddr = None
                    if baddr is None:
                        return None
                if bm in ('tbh', 'tbb'):
                    idx = regs[['r0', 'r1', 'r2', 'r3', 'r4', 'r5', 'r6',
                                'r7', 'r8'].index(parts[1])] if parts[1] in [
                        'r0', 'r1', 'r2', 'r3', 'r4', 'r5', 'r6', 'r7',
                        'r8'] else 0
                    if bm == 'tbh':
                        raw = (pattern_byte(baddr + idx * 2)
                               | (pattern_byte(baddr + idx * 2 + 1) << 8))
                        return mapped((baddr + raw * 2) & 0xFFFFFFFF & ~1)
                    raw = pattern_byte(baddr + idx)
                    return mapped((baddr + raw * 2) & 0xFFFFFFFF & ~1)
                if bm == 'ldrd':
                    # PC value comes from the second word when Rt2==pc.
                    pw = 4 if dests[1] == 'pc' else 0
                    return mapped(pattern_word((baddr + pw) & 0xFFFFFFFF) & ~1)
                # Offset forms ([Rn, #off]): resolve the effective address
                # (literals have no offset part, unchanged).
                off = 0
                for p in parts[1:]:
                    p = p.strip()
                    if p.startswith('#'):
                        try:
                            off += int(p[1:], 0)
                        except ValueError:
                            pass
                    elif p.startswith('-'):
                        try:
                            off -= int(p[2:] if p[1] == '#' else p[1:], 0)
                        except ValueError:
                            pass
                return mapped(pattern_word((baddr + off) & 0xFFFFFFFF) & ~1)
            except (IndexError, ValueError, KeyError):
                return None
        return None
    if m in ('cbz', 'cbnz'):
        return True  # tiny direct offset: always mapped; divergence is real
    return None


def reserved_shift_bit(first, second):
    """Fault-divergence where Capstone itself rejects the encoding: reserved
    / UNPREDICTABLE opcodes that we execute benignly but the oracle faults
    (same philosophy as census over-accepts, which compilers never emit).
    Returns True if Capstone-invalid (expected divergence)."""
    import struct
    from capstone import Cs, CS_ARCH_ARM, CS_MODE_THUMB, CS_MODE_MCLASS
    md = Cs(CS_ARCH_ARM, CS_MODE_THUMB | CS_MODE_MCLASS)
    data = struct.pack('<H', first)
    if second is not None:
        data += struct.pack('<H', second)
    try:
        r = list(md.disasm(data, 0x20002000))
    except Exception:
        return False
    if not r:
        return True
    # 16-bit insn decoded out of a 32-bit sample (or vice versa): the other
    # side may legitimately see something else; treat as expected.
    want_len = 4 if (second is not None and (first & 0xF800) >= 0xE800) else 2
    if not r or r[0].size != want_len:
        return True
    # Capstone-MCLASS rejects a whole zoo of reserved E8/E9/F2/F6 shapes
    # (odd LDM/STMAM modes, F2xx-with-RdPC, PKHBT-as-SSAT, ...) that our
    # decoder over-accepts benignly (differential fuzz singletons, all
    # cap-INVALID, compilers never emit). Any remaining cap-REJECTED
    # sample is expected by the same rule — but only when OUR side is the
    # one executing (fault ours=0): if WE faulted too, the fault bits agree
    # and this function is never consulted.
    mn = r[0].mnemonic
    if mn == 'udf' or 'unallocated' in (r[0].op_str or ''):
        return True
    # Multiply group with reserved op2[7:4] (MUL/MLA/MLS/UMULL/SMULL/
    # UMLAL/SMLAL need 0 there; UDIV/SDIV need 0xF): we execute the
    # obvious reading, the oracle faults. No codegen emits these.
    if second is not None:
        if mn in ('mul', 'mla', 'mls', 'umull', 'smull', 'umlal',
                  'smlal') and (second & 0xF0) != 0:
            return True
        if mn in ('udiv', 'sdiv') and (second & 0xF0F0) != 0xF0F0:
            return True
    return False


def data_addrs_mapped(first, second, regs, sp):
    """For data-access fault divergences: recompute every accessed address
    (Capstone-decoded) and report whether all are mapped. Unmapped access:
    ours returns 0/drops silently, Unicorn faults — expected. Returns True
    (all mapped: divergence is REAL), False (expected), None (not a memory
    op: divergence is REAL)."""
    import struct
    from capstone import Cs, CS_ARCH_ARM, CS_MODE_THUMB
    md = Cs(CS_ARCH_ARM, CS_MODE_THUMB)
    data = struct.pack('<H', first)
    if second is not None:
        data += struct.pack('<H', second)
    try:
        r = list(md.disasm(data, 0x20002000))
    except Exception:
        return None
    if not r:
        return None
    m, ops = r[0].mnemonic, r[0].op_str
    base = m[:-2] if m.endswith('.w') or m.endswith('.n') else m
    if base in ('ldrb', 'ldrh', 'ldr', 'ldrsb', 'ldrsh', 'strb', 'strh',
                'str', 'ldrd', 'strd', 'ldrex', 'strex', 'ldrt', 'ldrbt',
                'ldrht', 'strt', 'strbt', 'strht'):
        # UNPREDICTABLE-but-decodable edges the oracle strict-faults:
        # - store/load with Rt==PC handled by callers; byte/half loads
        #   into PC and stores from PC have no valid encoding;
        # - shifted-register operand with op2[15] set (reserved bit).
        if second is not None:
            rt = (second >> 12) & 0xF
            if rt == 0xF and base not in ('ldr',):
                return False
            if second & 0x8000:
                ops_has_shift = (', lsl #' in ops or ', lsr #' in ops
                                 or ', asr #' in ops or ', ror #' in ops
                                 or ', rrx' in ops)
                if ops_has_shift:
                    return False
        # Single transfer: find [...] operand, resolve base + offset.
        if '[' not in ops:
            return None
        try:
            inside = ops.split('[')[1].split(']')[0]
            parts = [x.strip() for x in inside.split(',')]
            breg = parts[0]
            if breg == 'pc':
                baddr = 0x20002004
            elif breg == 'sp':
                baddr = sp
            else:
                # Full r0-r12: the generator randomizes all 13 (constrain
                # only pins r0-r8, so r9-r12 need exact values too).
                try:
                    baddr = regs[int(breg[1:])] if breg.startswith('r') else 0
                except (ValueError, IndexError):
                    baddr = 0
            off = 0
            for p in parts[1:]:
                p = p.strip()
                if p.startswith('#'):
                    off += int(p[1:], 0)
                elif p.startswith('-'):
                    off -= int(p[2:] if p[1] == '#' else p[1:], 0)
            nwords = 2 if base in ('ldrd', 'strd') else 1
            for k in range(nwords):
                if not mapped((baddr + off + 4 * k) & 0xFFFFFFFF):
                    return False
            # Stores to flash: ours drops (flash protection), Unicorn writes
            # its mapping. Loads from flash agree (same pattern bytes).
            if base.startswith('str'):
                for k in range(nwords):
                    a = (baddr + off + 4 * k) & 0xFFFFFFFF
                    if 0x08000000 <= a < 0x08010000:
                        return False
            return True
        except (IndexError, ValueError, KeyError):
            return None
    if base in ('ldm', 'stm', 'ldmia', 'stmia', 'ldmdb',
                'stmdb', 'ldmib', 'stmib', 'ldmda', 'stmda', 'vldmia'):
        try:
            if base in ('pop', 'push'):
                baddr, n, wb = sp, ops.count(',') + 1, True
                up, pre = True, False
            else:
                rn = ops.split()[0].rstrip('!,')
                if rn == 'pc':
                    return False  # LDM/STM with Rn==PC: UNPREDICTABLE
                if rn == 'sp':
                    baddr = sp
                else:
                    try:
                        baddr = regs[int(rn[1:])] if rn.startswith('r') else 0
                    except (ValueError, IndexError):
                        baddr = 0
                lst = ops.split('{')[1].split('}')[0]
                n = lst.count(',') + 1
                wb = '!' in ops
                suf = m[3:] if m.startswith(('ldm', 'stm')) else ''
                up = not suf.startswith('d')
                pre = (suf == 'db' or suf == 'ib')
            lo = baddr + (-4 * n if not up else (4 if pre else 0))
            hi = baddr + (4 * n if up else 0)
            a = lo
            while a < hi + 4:
                if not mapped(a & 0xFFFFFFFF):
                    return False
                a += 4
            return True
        except (IndexError, ValueError, KeyError):
            return None
    return None


def fnv(data):
    h = 0xcbf29ce484222325
    for b in data:
        h ^= b
        h = (h * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return h


def run_oracle(cases):
    from unicorn import Uc, UC_ARCH_ARM, UC_MODE_THUMB
    from unicorn.arm_const import (UC_ARM_REG_R0, UC_ARM_REG_R1,
                                   UC_ARM_REG_R2, UC_ARM_REG_R3,
                                   UC_ARM_REG_R4, UC_ARM_REG_R5,
                                   UC_ARM_REG_R6, UC_ARM_REG_R7,
                                   UC_ARM_REG_R8, UC_ARM_REG_R9,
                                   UC_ARM_REG_R10, UC_ARM_REG_R11,
                                   UC_ARM_REG_R12, UC_ARM_REG_SP,
                                   UC_ARM_REG_LR, UC_ARM_REG_PC,
                                   UC_ARM_REG_CPSR)
    regs_consts = [UC_ARM_REG_R0, UC_ARM_REG_R1, UC_ARM_REG_R2,
                   UC_ARM_REG_R3, UC_ARM_REG_R4, UC_ARM_REG_R5,
                   UC_ARM_REG_R6, UC_ARM_REG_R7, UC_ARM_REG_R8,
                   UC_ARM_REG_R9, UC_ARM_REG_R10, UC_ARM_REG_R11,
                   UC_ARM_REG_R12]
    flash = bytes(pattern_byte(0x08000000 + i) for i in range(0x10000))
    ram = bytes(pattern_byte(0x20000000 + i) for i in range(0x10000))
    results = []
    mu = Uc(UC_ARCH_ARM, UC_MODE_THUMB)
    mu.mem_map(0x08000000, 0x10000)
    mu.mem_map(0x20000000, 0x10000)
    # Fixed PC (0x20002001, like the Rust side — PC-relative cases need
    # identical addresses). mem_write does not reliably invalidate Unicorn's
    # TB cache, so flush it every case (re-executing a stale TB once
    # masqueraded as divine state leakage).
    pc = 0x20002000
    for case_idx, (code, regs, sp, lr, xpsr, it, steps) in enumerate(cases):
        mu.mem_write(0x08000000, flash)
        mu.mem_write(0x20000000, ram)
        # install snippet at PC (fixed 0x20002001, like the Rust side)
        mu.mem_write(pc, struct.pack('<%dH' % len(code), *code))
        mu.ctl_flush_tb()
        for c, v in zip(regs_consts, regs):
            mu.reg_write(c, v)
        mu.reg_write(UC_ARM_REG_SP, sp)
        mu.reg_write(UC_ARM_REG_LR, lr)
        # Our xpsr bit 24 is T; ARM CPSR wants T at bit 5 (+ ITSTATE
        # at [26:25,15:10], NZCVQ shared).
        cpsr = (xpsr & 0xF8000000) | 0x20
        if it[0] or it[1]:
            itstate = (it[0] << 4) | it[1]
            cpsr |= (((itstate >> 2) & 0x3F) << 10) | ((itstate & 0x3) << 25)
        start = pc | 1
        count = steps
        mu.reg_write(UC_ARM_REG_CPSR, cpsr)
        fault = 0
        try:
            mu.emu_start(start, 0, timeout=0, count=count)
        except Exception:
            fault = 1
        # Harness self-check: every case must advance PC or fault. A
        # silent no-op means the driver (not the CPU) is broken.
        if not fault and mu.reg_read(UC_ARM_REG_PC) == (start & ~1):
            raise RuntimeError(f'oracle silent skip on case {case_idx}')
        got_regs = [mu.reg_read(c) for c in regs_consts]
        got = got_regs + [mu.reg_read(UC_ARM_REG_SP),
                          mu.reg_read(UC_ARM_REG_LR),
                          mu.reg_read(UC_ARM_REG_PC),
                          mu.reg_read(UC_ARM_REG_CPSR)]
        mh = fnv(mu.mem_read(0x08000000, 0x10000) + mu.mem_read(0x20000000, 0x10000))
        results.append((got, mh, fault))
    return results


def main():
    n = 2000
    seed = 1
    for i, a in enumerate(sys.argv):
        if a == '--cases' and i + 1 < len(sys.argv):
            n = int(sys.argv[i + 1])
        if a == '--seed' and i + 1 < len(sys.argv):
            seed = int(sys.argv[i + 1])
    cases = gen_cases(n, seed)
    write_cases(cases)
    ours = run_ours()
    oracle = run_oracle(cases)
    assert len(ours) == len(oracle) == n
    divs = []
    unmapped_skips = 0
    for idx, ((code, regs, sp, lr, xpsr, it, _steps), oline, (got, mh, fault)) in enumerate(zip(cases, ours, oracle)):
        orr = [int(x, 16) for x in oline[0:16]]
        oxpsr = int(oline[16], 16)
        omh = int(oline[21], 16)
        ofault = int(oline[22])
        first = code[0]
        second = code[1] if len(code) > 1 else None
        multi = len(code) > 2 or (len(code) == 2 and steps_of(cases[idx]) > 1)
        issues = []
        if ofault != fault:
            # Track-2 multi-step cases are generated fault-free by
            # construction (mapped, no branches, no PC writes), so any
            # fault mismatch there is REAL — no first-insn triage (the
            # fault may be in step 2 with clobbered regs).
            if multi:
                issues.append(f'fault ours={ofault} unicorn={fault} (multi-step)')
            else:
                # Ours sets PC silently while Unicorn prefetch-faults when the
                # branch target is unmapped: recompute and expect those. Same
                # for data accesses outside mapped RAM/flash (ours returns
                # 0/drops, Unicorn faults). Reserved shift bit: oracle-strict.
                tm = branch_target_mapped(first, second, regs, sp)
                if tm is None:
                    tm = data_addrs_mapped(first, second, regs, sp)
                if tm is None and reserved_shift_bit(first, second):
                    tm = False
                if tm is None and ofault == 1 and fault == 0 and cap_dsp_mnemonic(first, second):
                    tm = False
                if tm is False:
                    # Expected divergence BY DESIGN (unmapped / reserved /
                    # oracle-strict): the faulting side stops while the other
                    # advances, so PC/regs/mem legitimately differ downstream.
                    # Comparing values here only re-reports the same triaged
                    # fault (this once listed ~35 phantom r15-only "divergences"
                    # per 200 cases). Skip value checks for the case entirely:
                    # mapped-agreement is proven by the remaining cases.
                    unmapped_skips += 1
                    continue
                else:
                    issues.append(f'fault ours={ofault} unicorn={fault}')
        if (oxpsr & XPSR_MASK) != (got[16] & XPSR_MASK):
            issues.append(f'xpsr ours={oxpsr:08x} unicorn={got[16]:08x}')
        # Both sides faulted: r15 is fault-reporting state, not
        # architectural (we record the faulting address; the oracle
        # reports exception-entry state, e.g. subw-pc leaves
        # 0xFFFFF1E8). Regs/mem must still agree (fault = no state
        # change on either side). IT pairs also skip r15 (oracle
        # stop-counting quirk, documented in gen_itpair).
        is_itpair = (len(code) > 2 and (code[0] & 0xFF00) == 0xBF00)
        for ri in range(16):
            if ri == 15 and (is_itpair or (ofault == 1 and fault == 1)):
                continue
            want = (orr[ri] & ~1) if ri == 15 else (orr[ri] & 0xFFFFFFFF)
            have = (got[ri] & ~1) if ri == 15 else (got[ri] & 0xFFFFFFFF)
            if want != have:
                issues.append(f'r{ri} ours={want:08x} unicorn={have:08x}')
                if len(issues) > 6:
                    break
        if omh != mh:
            # Memory differs with agreeing faults/regs: only defensible
            # when the access itself is out-of-scope — unmapped (we return
            # 0/drop, oracle faults... but faults AGREED here, so both
            # executed) or FLASH stores (we drop for flash protection,
            # the oracle's mapping is writable: STRD to flash changes
            # only its hash). data_addrs_mapped answers exactly this.
            if data_addrs_mapped(first, second, regs, sp) is False:
                pass  # out-of-scope access, memory legitimately differs
            else:
                issues.append(f'memhash ours={omh:016x} unicorn={mh:016x}')
        # IT-state model check: a bare `it` instruction must set the
        # identical block state on both sides. (Single-step only:
        # track-2 pairs end mid-block by design; their IT advancement
        # is covered implicitly by the predicated-payload values.)
        oit = [int(x, 16) for x in oline[17:21]]
        if not multi and (oit[0] or oit[1]):
            exp = expected_it(it, oit, first, second, orr, got)
            if exp is not None:
                ucitfield = ((got[16] >> 10) & 0x3F) << 2 | ((got[16] >> 25) & 0x3)
                if ucitfield != exp[0] or (oit[0], oit[1], oit[2], oit[3]) != exp[1]:
                    issues.append(f'it ours={tuple(oit)} unicorn-it={ucitfield:02x} expected={exp}')
        if issues:
            # Capstone-MCLASS rejects the encoding outright (reserved /
            # UNPREDICTABLE): out of scope even when both sides execute
            # with different values (census over-accept philosophy —
            # compilers never emit these; e.g. E839:F4F4 where we load
            # zeros then fault on the PC load while the oracle faults
            # cleanly). Only excuses cap-REJECTED samples, never valid
            # encodings. Same when both sides faulted on an unmapped
            # access (values polluted by the 0-vs-fault design gap before
            # the agreed fault, e.g. LDRD-post-indexed-PC from unmapped).
            # (Single-step only: multi-step cases are constrained
            # fault-free, so anything there is REAL.)
            if not multi and cap_invalid_mclass(first, second):
                unmapped_skips += 1
                continue
            if ofault == 1 and fault == 1 and data_addrs_mapped(first, second, regs, sp) is False:
                unmapped_skips += 1
                continue
            op = ':'.join(f'{h:04X}' for h in code)
            divs.append((idx, op, issues[:8]))
    print(f'fuzz: {n} cases, {len(divs)} divergences, '
          f'{unmapped_skips} expected-unmapped-target skips')
    with open('/tmp/fuzz_divs.txt', 'w') as f:
        for idx, op, issues in divs:
            f.write(f'# case {idx} [{op}]\n')
            f.write(open(CASES_PATH).readlines()[idx])
            for iss in issues:
                f.write(f'#   {iss}\n')
    for idx, op, issues in divs[:30]:
        print(f'  case {idx} [{op}]')
        for iss in issues:
            print(f'    {iss}')
    return 1 if divs else 0


def cap_invalid_mclass(first, second):
    """True when Capstone-MCLASS rejects the sample (no decode, or a
    16/32-bit length mismatch): reserved/UNPREDICTABLE, out of scope."""
    import struct
    from capstone import Cs, CS_ARCH_ARM, CS_MODE_THUMB, CS_MODE_MCLASS
    md = Cs(CS_ARCH_ARM, CS_MODE_THUMB | CS_MODE_MCLASS)
    data = struct.pack('<H', first)
    if second is not None:
        data += struct.pack('<H', second)
    try:
        r = list(md.disasm(data, 0x20002000))
    except Exception:
        return False
    if not r:
        return True
    want_len = 4 if (second is not None and (first & 0xF800) >= 0xE800) else 2
    return r[0].size != want_len


DSP_MNEMONICS = frozenset('''
    smlalbb smlalbt smlaltb smlaltt smlad smladx smlald smlaldx
    smuad smuadx smusd smusdx smlsd smlsdx smmla smmlar smmls smmlsr
    smmul smmulr smlawb smlawt smulwb smulwt usad8 usada8
    umlalbb umlalbt umlaltb umlaltt umaal
    '''.split())


def cap_dsp_mnemonic(first, second):
    """True when Capstone-MCLASS decodes a DSP-extension form we don't
    implement (cpu.dsp=false faults): the oracle advances past them
    without writing, we fault — out of scope (census proves the fault)."""
    import struct
    from capstone import Cs, CS_ARCH_ARM, CS_MODE_THUMB, CS_MODE_MCLASS
    md = Cs(CS_ARCH_ARM, CS_MODE_THUMB | CS_MODE_MCLASS)
    data = struct.pack('<H', first)
    if second is not None:
        data += struct.pack('<H', second)
    try:
        r = list(md.disasm(data, 0x20002000))
    except Exception:
        return False
    if not r:
        return False
    mn = r[0].mnemonic
    mn = mn[:-2] if mn.endswith('.w') or mn.endswith('.n') else mn
    return mn in DSP_MNEMONICS


def steps_of(case):
    return case[6]


def expected_it(it_in, it_out, first, second, regs_out, regs_uc):
    """Model-check IT advancement. Returns (expected_uc_itfield,
    expected_ours_tuple) or None to skip."""
    # Fresh IT instruction: both sides must show cond:mask.
    if it_in == [0, 0, 0, 0]:
        if it_out[1] == 0:
            return None  # not an IT insn, nothing to check
        cond, mask = it_out[0], it_out[1]
        tz = (mask & -mask).bit_length() - 1 if mask else 0
        exp_ours = (cond, mask, 4 - tz, 0)
        exp_uc = (cond << 4) | mask
        if tuple(it_out) != exp_ours:
            return (exp_uc, exp_ours)
        return (exp_uc, exp_ours)
    # No preset blocks in v1 (see generator note).
    return None


if __name__ == '__main__':
    sys.exit(main())
