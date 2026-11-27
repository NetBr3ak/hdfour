import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { runInNewContext } from 'node:vm';

async function app() {
  const elements = new Map();
  const handlers = new Map();
  const element = id => {
    if (!elements.has(id)) elements.set(id, {
      disabled: false, textContent: '', files: [],
      replaceChildren() {}, addEventListener(name, handler) { handlers.set(`${id}:${name}`, handler); },
    });
    return elements.get(id);
  };
  const source = await readFile(new URL('./app.mjs', import.meta.url), 'utf8');
  const code = source.replace("import { greyscale } from './preview.mjs';", '')
    .replace("await import('./pkg/hdfour_wasm.js')", 'testReader');
  const page = await runInNewContext(`(async () => { ${code}\nreturn { openFile }; })()`, {
    document: { querySelector: element, addEventListener(name, handler) { handlers.set(name, handler); } },
    window: { addEventListener(name, handler) { handlers.set(name, handler); } },
    testReader: { default() {}, File: { fromBytes: () => ({
      datasets: () => [], attributes: () => [], skipped: () => [], eos: () => null, free() {},
    }) } }, Uint8Array, ArrayBuffer,
  });
  return { ...page, elements, handlers };
}

test('a restored page does not report an open file after it was freed', async () => {
  const page = await app();
  await page.openFile({ name: 'example.hdf', size: 4, arrayBuffer: () => new ArrayBuffer(4) });
  page.handlers.get('pagehide')();
  assert.equal(page.elements.get('#status').textContent, 'No file open.');
  assert.equal(page.elements.get('#file-metadata').textContent, 'No file open.');
});

test('a file read finishing after pagehide does not restore the old status', async () => {
  const page = await app();
  let finish;
  const pending = page.openFile({ name: 'example.hdf', size: 4,
    arrayBuffer: () => new Promise(resolve => { finish = resolve; }) });
  page.handlers.get('pagehide')();
  finish(new ArrayBuffer(4));
  await pending;
  assert.equal(page.elements.get('#status').textContent, 'No file open.');
});
