import assert from 'node:assert/strict';
import { once } from 'node:events';
import { readFile, mkdir } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const { chromium } = require('playwright');
const axe = require('axe-core');
const root = new URL('../', import.meta.url);
const assets = new Map([
  ['/demo/', ['demo/index.html', 'text/html']],
  ...['app.mjs', 'preview.mjs', 'pkg/hdfour_wasm.js'].map(path => [`/demo/${path}`, [`demo/${path}`, 'text/javascript']]),
  ['/demo/style.css', ['demo/style.css', 'text/css']],
  ['/demo/pkg/hdfour_wasm_bg.wasm', ['demo/pkg/hdfour_wasm_bg.wasm', 'application/wasm']],
]);
await mkdir(new URL('./artifacts/', import.meta.url), { recursive: true });
const server = createServer(async (request, response) => {
  const asset = assets.get(request.url);
  if (request.method !== 'GET' || !asset) {
    response.writeHead(404).end();
    return;
  }
  try {
    const bytes = await readFile(new URL(asset[0], root));
    response.writeHead(200, { 'Content-Type': asset[1] }).end(bytes);
  } catch (error) {
    console.error(error);
    response.writeHead(500).end();
  }
});
server.listen(0, '127.0.0.1');
await once(server, 'listening');
const origin = `http://127.0.0.1:${server.address().port}`;
let browser;

async function checkAxe(page) {
  await page.evaluate(axe.source);
  const violations = await page.evaluate(async () => (await globalThis.axe.run(document, {
    runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'] },
  })).violations);
  assert.deepEqual(violations.map(violation => ({ id: violation.id, nodes: violation.nodes.length })), []);
}

async function focusNext(page, expected) {
  await page.keyboard.press('Tab');
  assert.equal(await page.evaluate(() => document.activeElement.id || document.activeElement.tagName), expected);
  assert.notEqual(await page.evaluate(() => getComputedStyle(document.activeElement).outlineStyle), 'none');
}

try {
  browser = await chromium.launch({ headless: true });
  for (const width of [375, 1280]) {
    const page = await browser.newPage({ viewport: { width, height: 900 }, reducedMotion: 'reduce' });
    const errors = [];
    const requests = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
    page.on('request', request => requests.push({ url: request.url(), method: request.method() }));
    try {
      await page.goto(`${origin}/demo/`);
      await page.getByRole('status').filter({ hasText: 'Ready to open a file.' }).waitFor();
      await checkAxe(page);
      await page.screenshot({ path: fileURLToPath(new URL(`./artifacts/empty-${width}.png`, import.meta.url)), fullPage: true });
      await focusNext(page, 'file');
      const loadedRequests = requests.length;
      await page.locator('#file').setInputFiles(fileURLToPath(new URL('fixtures/F01.hdf', root)));
      await page.locator('#status').filter({ hasText: '10 datasets' }).waitFor();
      assert.equal(await page.locator('#datasets option').count(), 10);
      assert.equal(await page.locator('#preview').isVisible(), true);
      await focusNext(page, 'datasets');
      await page.keyboard.press('ArrowDown');
      assert.equal(await page.locator('#datasets').evaluate(select => select.selectedIndex), 1);
      assert.match(await page.locator('#metadata').textContent(), /"name": "u8"/);
      await focusNext(page, 'metadata');
      await focusNext(page, 'SUMMARY');
      await page.keyboard.press('Enter');
      await focusNext(page, 'file-metadata');
      await checkAxe(page);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      assert.ok(await page.locator('#canvas').evaluate(canvas => canvas.getBoundingClientRect().width) > 200);
      await page.screenshot({ path: fileURLToPath(new URL(`./artifacts/loaded-${width}.png`, import.meta.url)), fullPage: true });

      await page.locator('#file').setInputFiles(fileURLToPath(new URL('fixtures/F19.hdf', root)));
      await page.locator('#range').filter({ hasText: '6 finite, non-fill values' }).waitFor();
      assert.equal(await page.locator('#canvas').evaluate(canvas => {
        const pixels = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data;
        return pixels.filter((_, index) => index % 4 === 3 && pixels[index] === 0).length;
      }), 29);
      await page.locator('#file').setInputFiles(fileURLToPath(new URL('fixtures/F10.hdf', root)));
      await page.locator('#preview-status').filter({ hasText: 'skipping-huffman' }).waitFor();
      assert.equal(await page.locator('#preview').isVisible(), false);
      await page.locator('#file').setInputFiles({ name: 'invalid.hdf', mimeType: 'application/octet-stream', buffer: Buffer.from([1, 2, 3, 4]) });
      await page.locator('#status').filter({ hasText: 'Cannot open invalid.hdf' }).waitFor();
      assert.equal(await page.locator('#datasets').isDisabled(), true);
      const bytes = [...await readFile(new URL('fixtures/F01.hdf', root))];
      await page.evaluate(bytes => {
        const transfer = new DataTransfer();
        transfer.items.add(new File([new Uint8Array(bytes)], 'dropped.hdf'));
        document.querySelector('#drop-zone').dispatchEvent(new DragEvent('drop', { dataTransfer: transfer, bubbles: true, cancelable: true }));
      }, bytes);
      await page.locator('#status').filter({ hasText: 'dropped.hdf: 10 datasets' }).waitFor();
      assert.equal(requests.length, loadedRequests, 'opening files makes no network request');
      assert.ok(requests.every(request => request.url.startsWith(origin) && request.method === 'GET'));
      assert.deepEqual(errors, []);
      console.log(`PASS ${width}px: keyboard, focus, axe, layout, preview, fill, errors, drop and local-only reads`);
    } finally {
      await page.close();
    }
  }
} finally {
  if (browser) await browser.close();
  server.closeAllConnections();
  await new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
}
