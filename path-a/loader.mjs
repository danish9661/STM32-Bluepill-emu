// Node wrapper: reads merged4.wasm from disk. Browser code should import
// loader-core.mjs directly and call loadMergedFromBytes(fetchedBytes).
import { readFileSync } from 'node:fs';
import { loadMergedFromBytes } from './loader-core.mjs';

export async function loadMerged() {
    return loadMergedFromBytes(readFileSync(new URL('./merged4.wasm', import.meta.url)));
}
