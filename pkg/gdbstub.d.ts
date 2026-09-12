// TypeScript declarations for pkg/gdbstub.mjs
// GDB Remote Serial Protocol stub (TCP) for live firmware debugging.

/** Options: createEmulator opts plus server settings. */
export interface GdbOptions {
    firmware?: Uint8Array | ArrayBuffer | string;
    chip?: string | { name: string; svd?: string; flash?: number; ram?: number; idcode?: number };
    flash_size?: number;
    ram_size?: number;
    vector_table?: number;
    ext_devices?: object;
    /** TCP port to listen on (0 = ephemeral, reported back). Default 1234. */
    port?: number;
    /** Instructions per continue chunk. Default 20000. */
    chunk?: number;
}

/**
 * Serve one emulator instance over GDB RSP (`target remote :<port>`).
 * Registers, memory, BKPT breakpoints (Z0), data watchpoints (Z2/Z3/Z4),
 * step/continue, target.xml.
 */
export function serveGdb(opts?: GdbOptions): Promise<{
    port: number;
    emu: any;
    close(): void;
}>;
