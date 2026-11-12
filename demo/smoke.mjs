import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import init, { File } from './pkg/hdfour_wasm.js';
import './preview.test.mjs';

const wasm = await init({ module_or_path: await readFile(new URL('./pkg/hdfour_wasm_bg.wasm', import.meta.url)) });
const fixtures = new URL('../fixtures/', import.meta.url);
const constructors = [Int8Array, Uint8Array, Int16Array, Uint16Array, Int32Array,
  Uint32Array, Float32Array, Float64Array, Int8Array, Uint8Array];

test('F01: metadata and every typed array match the C golden bytes', async () => {
  const bytes = await readFile(new URL('F01.hdf', fixtures));
  const golden = JSON.parse(await readFile(new URL('golden/F01.json', fixtures), 'utf8'));
  const file = File.fromBytes(bytes);
  let freed = false;
  bytes.fill(0);
  try {
    const datasets = file.datasets();
    assert.equal(datasets.length, 10);
    assert.deepEqual(file.attributes(), []);
    assert.deepEqual(file.skipped(), []);
    assert.equal(file.eos(), null);
    for (const expected of golden.datasets) {
      const index = expected.index;
      const metadata = datasets[index];
      assert.equal(Object.getPrototypeOf(metadata), Object.prototype);
      assert.equal(metadata.name, expected.name);
      assert.equal(metadata.dtype, expected.name);
      assert.deepEqual(metadata.shape, expected.shape);
      assert.deepEqual(metadata.dims, expected.dimensions);
      assert.equal(metadata.is_coordinate, expected.is_coordinate);
      assert.deepEqual(metadata.storage, { kind: 'contiguous' });
      assert.deepEqual(metadata.codec, { kind: 'none' });
      assert.deepEqual(metadata.attributes, expected.attributes);
      assert.deepEqual(file.attributes(index), expected.attributes);
      const values = file.read(index);
      assert.ok(values instanceof constructors[index], expected.name);
      const expectedBytes = await readFile(new URL(`golden/F01/${index}.bin`, fixtures));
      assert.deepEqual(Buffer.from(values.buffer, values.byteOffset, values.byteLength), expectedBytes);
      const first = values[0];
      values[0] = 0;
      assert.equal(file.read(index)[0], first, 'results do not alias file storage');
    }
    const retained = file.read(0);
    const retainedBytes = Buffer.from(retained.buffer, retained.byteOffset, retained.byteLength);
    const expectedRetained = await readFile(new URL('golden/F01/0.bin', fixtures));
    assert.notEqual(retained.buffer, wasm.memory.buffer, 'results own their buffer');
    file.free();
    freed = true;
    assert.deepEqual(retainedBytes, expectedRetained, 'results survive free()');
    assert.throws(() => file.datasets());
  } finally {
    if (!freed) file.free();
  }
});

test('attributes, EOS links, skips and chunk metadata stay plain objects', async () => {
  for (const name of ['F03', 'F07', 'F15', 'F16']) {
    const file = File.fromBytes(await readFile(new URL(`${name}.hdf`, fixtures)));
    const golden = JSON.parse(await readFile(new URL(`golden/${name}.json`, fixtures), 'utf8'));
    try {
      assert.deepEqual(file.skipped(), golden.skipped, name);
      assert.deepEqual(file.eos(), golden.eos, name);
      for (const [actual, expected] of [[file.attributes(), golden.attributes],
        ...file.datasets().map((dataset, index) => [file.attributes(index), golden.datasets[index].attributes])]) {
        assert.equal(actual.length, expected.length, name);
        for (const [index, attribute] of actual.entries()) {
          assert.equal(Object.getPrototypeOf(attribute), Object.prototype);
          assert.equal(attribute.name, expected[index].name);
          const values = attribute.value;
          assert.ok(ArrayBuffer.isView(values));
          assert.equal(Buffer.from(values.buffer, values.byteOffset, values.byteLength).toString('hex'), expected[index].hex);
        }
      }
      if (name === 'F07') {
        assert.equal(file.datasets()[0].storage.kind, 'chunked');
        assert.deepEqual(file.datasets()[0].storage.chunk_shape, [2, 3]);
      }
    } finally {
      file.free();
    }
  }
});

test('invalid file and out-of-range dataset indices throw errors', async () => {
  assert.throws(() => File.fromBytes(new Uint8Array([1, 2, 3, 4])), /HDF4/);
  const file = File.fromBytes(await readFile(new URL('F01.hdf', fixtures)));
  try {
    assert.throws(() => file.read(10), /dataset index/);
    assert.throws(() => file.attributes(10), /dataset index/);
  } finally {
    file.free();
  }
});
