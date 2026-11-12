import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import init, { File } from './pkg/hdfour_wasm.js';

await init({ module_or_path: await readFile(new URL('./pkg/hdfour_wasm_bg.wasm', import.meta.url)) });

test('binding rejects an oversized input with a readable limit error', () => {
  assert.throws(() => File.fromBytes(new Uint8Array(268435457)), /file bytes.*268435457.*268435456/);
});

test('a small unwritten file cannot declare an array above the browser budget', async () => {
  const bytes = new Uint8Array(await readFile(new URL('../fixtures/F13.hdf', import.meta.url)));
  const view = new DataView(bytes.buffer);
  const descriptors = [];
  for (let block = 4; block !== 0; block = view.getUint32(block + 2)) {
    const count = view.getUint16(block);
    for (let index = 0; index < count; index++) {
      const offset = block + 6 + index * 12;
      descriptors.push({ tag: view.getUint16(offset), ref: view.getUint16(offset + 2),
        offset: view.getUint32(offset + 4), length: view.getUint32(offset + 8) });
    }
  }
  // cdf.c:1236-1275: DimVal0.1 stores one i32 dimension size in its Vdata.
  const dims = descriptors.filter(dd => dd.tag === 1962 &&
    new TextDecoder().decode(bytes.subarray(dd.offset, dd.offset + dd.length)).includes('DimVal0.1'));
  assert.ok(dims.length >= 2);
  for (const dimension of dims.slice(0, 2)) {
    const data = descriptors.find(dd => dd.tag === 1963 && dd.ref === dimension.ref);
    assert.equal(data.length, 4);
    view.setInt32(data.offset, 32768);
  }
  assert.throws(() => File.fromBytes(bytes), /array_bytes.*1073741824.*268435456/);
});
