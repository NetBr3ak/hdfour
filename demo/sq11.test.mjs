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
  return runInNewContext(`(async () => { ${code}\nreturn { openFile, format, elements, handlers }; })()`, {
    document: { querySelector: element, addEventListener(name, handler) { handlers.set(name, handler); } },
    window: { addEventListener() {} },
    testReader: { default() {}, File: { fromBytes: () => ({
      datasets: () => [], attributes: () => [], skipped: () => [], eos: () => null, free() {},
    }) } },
    elements, handlers, Uint8Array, ArrayBuffer,
  });
}

test('a pending read disables the picker and refuses a second selection and drop', async () => {
  const page = await app();
  let finish;
  const pending = page.openFile({ name: 'first.hdf', size: 4,
    arrayBuffer: () => new Promise(resolve => { finish = resolve; }) });
  assert.equal(page.elements.get('#file').disabled, true);
  let secondReads = 0;
  const second = { name: 'second.hdf', size: 4, arrayBuffer() { secondReads++; return new ArrayBuffer(4); } };
  page.elements.get('#file').files = [second];
  page.handlers.get('#file:change')();
  page.handlers.get('drop')({ preventDefault() {}, dataTransfer: { files: [second] } });
  assert.equal(secondReads, 0);
  assert.match(page.elements.get('#status').textContent, /already|progress|reading.*finish/i);
  finish(new ArrayBuffer(4));
  await pending;
  assert.equal(page.elements.get('#file').disabled, false);
  await page.openFile(second);
  assert.equal(secondReads, 1);
});

test('a failed read enables the picker for the next file', async () => {
  const page = await app();
  await page.openFile({ name: 'broken.hdf', size: 4, arrayBuffer() { throw Error('read failed'); } });
  assert.equal(page.elements.get('#file').disabled, false);
  assert.match(page.elements.get('#status').textContent, /read failed/);
  await page.openFile({ name: 'next.hdf', size: 4, arrayBuffer: () => new ArrayBuffer(4) });
  assert.match(page.elements.get('#status').textContent, /next.hdf: 0 datasets/);
});

test('the string cutoff excludes a high surrogate at code unit 1023', async () => {
  const page = await app();
  const value = JSON.parse(page.format({ value: `${'a'.repeat(1023)}\u{10400}tail` })).value;
  assert.equal(value, `${'a'.repeat(1023)} (truncated)`);
});

test('the output cutoff excludes a high surrogate at code unit 65535', async () => {
  const page = await app();
  const value = { items: Array(64).fill('a'.repeat(1000)), tail: '' };
  const start = JSON.stringify(value, null, 2).lastIndexOf('"');
  value.tail = `${'b'.repeat(65535 - start)}\u{10400}tail`;
  assert.ok(value.tail.length < 1024);
  const full = JSON.stringify(value, null, 2);
  assert.equal(full.charCodeAt(65535), 0xd801);
  assert.equal(page.format(value), `${full.slice(0, 65535)}\ntruncated`);
});
