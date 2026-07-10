import struct, sys, json
from unicorn import *
from unicorn.arm_const import *

PERIPH_START = 0x40000000
PERIPH_END = 0xB0000000
SYS_START = 0xE0000000
SYS_END = 0xE1000000

def run():
    config = json.loads(sys.argv[1])
    firmware = bytes.fromhex(config['firmware_hex'])
    vt = config['vector_table']
    patches = config.get('patches', [])

    mode = UC_MODE_THUMB | UC_MODE_MCLASS | UC_MODE_LITTLE_ENDIAN
    uc = Uc(UC_ARCH_ARM, mode)

    readBuf = b''

    def read_stdin():
        nonlocal readBuf
        while b'\n' not in readBuf:
            readBuf += sys.stdin.buffer.read(4096)
        line, readBuf = readBuf.split(b'\n', 1)
        return json.loads(line.decode())

    def write_stdout(obj):
        sys.stdout.write(json.dumps(obj) + '\n')
        sys.stdout.flush()

    # Setup memory regions
    for r in config.get('regions', []):
        start = r['start']
        size = r['size']
        perms = UC_PROT_READ | UC_PROT_EXEC if r.get('name') == 'ROM' else UC_PROT_ALL
        uc.mem_map(start, size, perms)

    # Load firmware into ROM
    rom_region = next((r for r in config.get('regions', []) if r.get('load')), None)
    if rom_region:
        uc.mem_write(rom_region['start'], firmware)

    # Apply patches
    for p in patches:
        uc.mem_write(p['start'], bytes(p['data']))

    # Map peripheral space
    uc.mem_map(PERIPH_START, PERIPH_END - PERIPH_START, UC_PROT_READ | UC_PROT_WRITE)
    uc.mem_map(SYS_START, SYS_END - SYS_START, UC_PROT_READ | UC_PROT_WRITE)

    def hook_mem(uc2, access, address, size, value, user_data):
        if access == UC_MEM_WRITE:
            write_stdout({"type": "write", "addr": address, "size": size, "value": value})
            resp = read_stdin()
        elif access == UC_MEM_READ:
            write_stdout({"type": "read", "addr": address, "size": size})
            resp = read_stdin()
            if 'value' in resp:
                val = resp['value']
                data = struct.pack(f'<{"B" if size==1 else "H" if size==2 else "I"}', val)
                uc2.mem_write(address, data)

    uc.hook_add(UC_HOOK_MEM_READ, hook_mem, begin=PERIPH_START, end=PERIPH_END - 1)
    uc.hook_add(UC_HOOK_MEM_WRITE, hook_mem, begin=PERIPH_START, end=PERIPH_END - 1)
    uc.hook_add(UC_HOOK_MEM_READ, hook_mem, begin=SYS_START, end=SYS_END - 1)
    uc.hook_add(UC_HOOK_MEM_WRITE, hook_mem, begin=SYS_START, end=SYS_END - 1)

    # Set up SP and PC
    sp_vec = struct.unpack('<I', firmware[0:4])[0]
    pc_vec = struct.unpack('<I', firmware[4:8])[0]
    uc.reg_write(UC_ARM_REG_SP, sp_vec)
    uc.reg_write(UC_ARM_REG_PC, pc_vec | 1)

    write_stdout({"type": "init", "sp": sp_vec, "pc": pc_vec | 1})

    # Main execution loop
    total_inst = 0
    while True:
        cur_pc = uc.reg_read(UC_ARM_REG_PC)

        # Check if we should stop
        msg = read_stdin()
        if msg.get('cmd') == 'stop':
            break

        batch = msg.get('batch', 100000)
        inst_limit = msg.get('inst_limit', 0)

        try:
            uc.emu_start(cur_pc | 1, 0, timeout=0, count=batch)
        except UcError as e:
            write_stdout({"type": "crash", "pc": uc.reg_read(UC_ARM_REG_PC),
                          "sp": uc.reg_read(UC_ARM_REG_SP), "error": str(e)})
            break

        final_pc = uc.reg_read(UC_ARM_REG_PC)
        final_sp = uc.reg_read(UC_ARM_REG_SP)
        write_stdout({"type": "step", "pc": final_pc, "sp": final_sp})

    uc.close()

if __name__ == '__main__':
    run()
