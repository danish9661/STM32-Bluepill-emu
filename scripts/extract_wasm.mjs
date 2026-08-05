import { readFileSync, writeFileSync } from 'fs';

const src = readFileSync('unicorn_arm.js', 'utf8');

// Search for base64 wasm - typical Emscripten pattern
const m1 = src.match(/wasmBinary\s*=\s*Uint8Array\.from\(atob\('([^']+)'\)/);
if (m1) {
  const buf = Buffer.from(m1[1], 'base64');
  writeFileSync('unicorn_arm.wasm', buf);
  console.log('Extracted unicorn_arm.wasm:', buf.length, 'bytes');
  process.exit(0);
}

// Try different patterns
const m2 = src.match(/wasmBinary\s*=\s*'([A-Za-z0-9+/=]{500,})'/);
if (m2) {
  const buf = Buffer.from(m2[1], 'base64');
  writeFileSync('unicorn_arm.wasm', buf);
  console.log('Extracted unicorn_arm.wasm via string:', buf.length, 'bytes');
  process.exit(0);
}

// Check if it's a Uint8Array literal
const m3 = src.match(/wasmBinary\s*=\s*new\s+Uint8Array\(\[([^\]]{500,})\]/);
if (m3) {
  const arr = JSON.parse('[' + m3[1] + ']');
  const buf = Buffer.from(arr);
  writeFileSync('unicorn_arm.wasm', buf);
  console.log('Extracted unicorn_arm.wasm via Uint8Array:', buf.length, 'bytes');
  process.exit(0);
}

console.log('Could not find embedded wasm. Searching for any wasm reference...');
const lines = src.split('\n');
for (let i = Math.max(0, lines.length - 50); i < lines.length; i++) {
  if (lines[i].length > 200) {
    console.log(`Line ${i+1}: ${lines[i].substring(0, 300)}`);
  }
}
