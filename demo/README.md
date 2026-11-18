# Browser inspector

Build from the repository root with `sh demo/build.sh`. This requires the
`wasm32-unknown-unknown` Rust target and `wasm-bindgen-cli` 0.2.130.
The build prints the generated module's raw and gzip byte counts, in that order.
Run `node demo/smoke.mjs` to compare the binding with the committed C golden files.
With Playwright and axe-core available in Node's module path, run
`node demo/browser.mjs` for browser checks at 375 and 1280 pixels.
The browser check closes its pages, browser and local server, including on failure.

Serve the repository through a static HTTP server and open `/demo/`.
Open an HDF4 file with the file picker or drop it on the page.
Use the dataset list to inspect metadata and preview a two-dimensional dataset.
The page reads file bytes locally. It has no upload path, tracker, remote script
or downloaded font. It fetches only its own JavaScript, CSS and WebAssembly.
Hosting is not configured by this batch.

## JavaScript API

```js
import init, { File } from './pkg/hdfour_wasm.js';

await init();
const file = File.fromBytes(bytes); // Uint8Array
try {
  const datasets = file.datasets();
  const attributes = file.attributes();
  const datasetAttributes = file.attributes(0);
  const omitted = file.skipped();
  const eos = file.eos();
  const rawValues = file.read(0);
} finally {
  file.free();
}
```

Metadata uses plain objects and arrays with the Rust field names. `dtype` is
`i8`, `u8`, `i16`, `u16`, `i32`, `u32`, `f32`, `f64`, `char8` or `uchar8`.
Storage and codec objects have a lower-case `kind`; chunked storage also has
`chunk_shape`, and an unknown codec has its numeric `code`.
Attributes are ordered `{ name, value }` objects. Each `value` is a typed array.
Character bytes use `Int8Array` for `char8` and `Uint8Array` for `uchar8`.
Integers outside JavaScript's exact number range use `bigint`.
Absent EOS metadata and absent field links are `null`.

`fromBytes` copies the input once into WebAssembly memory. `read` returns a
separate typed array with one copy out. Results remain valid after `free()`;
the file itself must not be used again. Core errors throw JavaScript errors
with the core message, including the unsupported feature name.

The preview excludes the first explicit `_FillValue` and non-finite values.
It uses the minimum and maximum of the remaining raw values, with a middle
grey for a constant array. Excluded pixels are transparent. The preview is
sampled to at most 512 pixels on either axis; dataset reads remain whole-array.
