import assert from 'node:assert/strict';
import { test } from 'node:test';
import { greyscale } from './preview.mjs';

test('greyscale excludes fill and non-finite values from the range and pixels', () => {
  const result = greyscale(new Float64Array([-999, 10, 20, NaN, Infinity, 15]), [2, 3], -999);
  assert.equal(result.min, 10);
  assert.equal(result.max, 20);
  assert.equal(result.count, 3);
  assert.deepEqual([...result.pixels], [0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 255, 255,
    0, 0, 0, 0, 0, 0, 0, 0, 128, 128, 128, 255]);
});

test('greyscale handles constant, empty and all-fill arrays', () => {
  assert.deepEqual([...greyscale(new Int16Array([7]), [1, 1]).pixels], [128, 128, 128, 255]);
  assert.equal(greyscale(new Uint8Array(), [0, 5]).count, 0);
  assert.equal(greyscale(new Int16Array([-1, -1]), [1, 2], -1).count, 0);
});

test('greyscale preserves extreme ranges and bounds the canvas dimensions', () => {
  const extreme = greyscale(new Float64Array([-Number.MAX_VALUE, 0, Number.MAX_VALUE]), [1, 3]);
  assert.deepEqual([...extreme.pixels.filter((_, index) => index % 4 === 0)], [0, 128, 255]);
  const large = greyscale(new Uint8Array(2048), [1, 2048]);
  assert.equal(large.width, 512);
  assert.equal(large.height, 1);
  assert.equal(large.pixels.length, 2048);
});

test('greyscale rejects a shape that does not describe a 2D array', () => {
  assert.throws(() => greyscale(new Uint8Array(2), [1, 1]), /shape/);
  assert.throws(() => greyscale(new Uint8Array(2), [2]), /shape/);
});
