import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import init, { File } from './pkg/hdfour_wasm.js';

await init({ module_or_path: await readFile(new URL('./pkg/hdfour_wasm_bg.wasm', import.meta.url)) });

test('rectangular reads return independent typed arrays and validate coordinates', async () => {
  const file = File.fromBytes(await readFile(new URL('../fixtures/F01.hdf', import.meta.url)));
  try {
    const full = file.read(2);
    const expected = [...full.slice(6, 9), ...full.slice(11, 14)];
    const window = file.readWindow(2, [1, 1], [2, 3]);
    assert.ok(window instanceof Int16Array);
    assert.deepEqual([...window], expected);
    window[0] = 0;
    assert.deepEqual([...file.readWindow(2, [1n, 1n], [2n, 3n])], expected);
    assert.throws(() => file.readWindow(2, [1], [1]), RangeError);
    assert.throws(() => file.readWindow(2, [2, 4], [2, 2]), RangeError);
    for (const coordinate of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1, NaN, '1']) {
      assert.throws(() => file.readWindow(2, [coordinate, 0], [1, 1]));
    }
  } finally {
    file.free();
  }
});
