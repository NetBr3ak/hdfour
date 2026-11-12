import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { runInNewContext } from 'node:vm';
import { greyscale } from './preview.mjs';

async function app(file) {
  const elements = new Map();
  function element(id) {
    if (!elements.has(id)) elements.set(id, {
      textContent: '', selectedIndex: 0, append() {}, replaceChildren() {},
      addEventListener() {}, setAttribute() {}, getContext: () => null,
    });
    return elements.get(id);
  }
  const source = await readFile(new URL('./app.mjs', import.meta.url), 'utf8');
  const code = source.replace("import { greyscale } from './preview.mjs';", '')
    .replace("await import('./pkg/hdfour_wasm.js')", 'testReader');
  return runInNewContext(`(async () => { ${code}\nreturn { openFile, format, elements }; })()`, {
    document: { querySelector: element, createElement: () => element('option'), addEventListener() {} },
    window: { addEventListener() {} },
    testReader: { default() {}, File: { fromBytes: () => file } },
    elements, greyscale, Uint8Array, ArrayBuffer,
  });
}

test('oversized files are refused before reading their bytes', async () => {
  const page = await app();
  let reads = 0;
  await page.openFile({ name: 'large.hdf', size: 268435457, arrayBuffer() { reads++; throw Error('bytes read'); } });
  assert.equal(reads, 0);
  assert.match(page.elements.get('#status').textContent, /268435456/);
});

test('attribute formatting keeps 64 elements and reports full length and truncation', async () => {
  const page = await app();
  const values = new Uint16Array(100).fill(9);
  values[64] = 999;
  const text = page.format({ value: values });
  assert.doesNotMatch(text, /999/);
  assert.match(text, /100/);
  assert.match(text, /truncated/);
  assert.equal((text.match(/\b9\b/g) ?? []).length, 64);
});

test('preview takes fill from dataset metadata without another attribute copy', async () => {
  let reads = 0;
  const file = {
    datasets: () => [{ name: 'data', shape: [1, 1], attributes: [{ name: '_FillValue', value: new Int16Array([-1]) }] }],
    attributes(index) { assert.equal(index, undefined); return []; },
    skipped: () => [], eos: () => null, read() { reads++; return new Int16Array([-1]); }, free() {},
  };
  const page = await app(file);
  await page.openFile({ name: 'small.hdf', size: 4, arrayBuffer: () => new ArrayBuffer(4) });
  assert.equal(reads, 1);
  assert.match(page.elements.get('#preview-status').textContent, /no finite, non-fill/);
});
