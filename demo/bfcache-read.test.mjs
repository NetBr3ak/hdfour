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
      replaceChildren() {}, append() {}, addEventListener(name, handler) { handlers.set(`${id}:${name}`, handler); },
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

function delayedFile(name) {
  let finish;
  const source = { name, size: 4, arrayBuffer: () => new Promise(resolve => { finish = resolve; }) };
  return { source, finish: () => finish(new ArrayBuffer(4)) };
}

test('leaving during a read allows a new selection without waiting for the old read', async () => {
  const page = await app();
  const first = delayedFile('first.hdf');
  const pending = page.openFile(first.source);
  page.handlers.get('pagehide')();
  assert.equal(page.elements.get('#file').disabled, false);
  first.finish();
  await pending;
});

test('an older read cannot unlock a newer read after the page is restored', async () => {
  const page = await app();
  const first = delayedFile('first.hdf');
  const oldRead = page.openFile(first.source);
  page.handlers.get('pagehide')();
  const second = delayedFile('second.hdf');
  const newRead = page.openFile(second.source);
  assert.equal(page.elements.get('#status').textContent, 'Reading second.hdf.');
  first.finish();
  await oldRead;
  assert.equal(page.elements.get('#file').disabled, true);
  second.finish();
  await newRead;
  assert.equal(page.elements.get('#file').disabled, false);
  assert.equal(page.elements.get('#status').textContent, 'second.hdf: 0 datasets. The file stays in this browser.');
});
