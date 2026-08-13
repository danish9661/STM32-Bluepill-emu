// Browser variant of the Path A loader: fetches merged4.wasm over HTTP and
// builds the same { Module, periph } surface. Node tests use loader.mjs.
import { loadMergedFromBytes } from './loader-core.mjs';

let cached;
export async function browserLoadMerged() {
    if (!cached) {
        const res = await fetch(new URL('./merged4.wasm', import.meta.url));
        cached = loadMergedFromBytes(new Uint8Array(await res.arrayBuffer()));
    }
    return cached;
}
