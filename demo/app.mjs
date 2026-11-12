import { greyscale } from './preview.mjs';

const picker = document.querySelector('#file');
const choices = document.querySelector('#datasets');
const status = document.querySelector('#status');
const previewStatus = document.querySelector('#preview-status');
const preview = document.querySelector('#preview');
const canvas = document.querySelector('#canvas');
const metadata = document.querySelector('#metadata');
const fileMetadata = document.querySelector('#file-metadata');
let reader;
let file;
let datasets = [];
let selection = 0;

function format(value) {
  return JSON.stringify(value, (_, entry) => {
    if (ArrayBuffer.isView(entry)) return Array.from(entry);
    if (typeof entry === 'bigint') return entry.toString();
    return entry;
  }, 2);
}

function showDataset() {
  preview.hidden = true;
  const index = choices.selectedIndex;
  const dataset = datasets[index];
  if (!dataset) return;
  metadata.textContent = format(dataset);
  if (dataset.shape.length !== 2) {
    previewStatus.textContent = 'Only two-dimensional datasets have a greyscale preview.';
    return;
  }
  try {
    const values = file.read(index);
    const fill = file.attributes(index).find(attribute => attribute.name === '_FillValue')?.value[0];
    const image = greyscale(values, dataset.shape, fill);
    if (image.count === 0) {
      previewStatus.textContent = 'This dataset has no finite, non-fill values to preview.';
      return;
    }
    const context = canvas.getContext('2d');
    if (!context) throw new Error('This browser cannot draw the preview. Metadata is still available.');
    canvas.width = image.width;
    canvas.height = image.height;
    context.putImageData(new ImageData(image.pixels, image.width, image.height), 0, 0);
    canvas.setAttribute('aria-label', `${dataset.name}: ${dataset.shape[0]} rows by ${dataset.shape[1]} columns, greyscale preview`);
    document.querySelector('#range').textContent = `Black: ${image.min}. White: ${image.max}. ${image.count} finite, non-fill values. Preview sampled to ${image.width} by ${image.height} pixels.`;
    preview.hidden = false;
    previewStatus.textContent = 'Raw values. Excluded pixels are transparent on a grey background.';
  } catch (error) {
    previewStatus.textContent = `Cannot preview this dataset: ${error.message}`;
  }
}

function clearFile() {
  if (file) file.free();
  file = undefined;
  datasets = [];
  choices.replaceChildren();
  choices.disabled = true;
  preview.hidden = true;
  metadata.textContent = 'No dataset selected.';
  fileMetadata.textContent = 'No file open.';
  previewStatus.textContent = 'Open a file, then select a two-dimensional dataset.';
}

async function openFile(source) {
  const current = ++selection;
  clearFile();
  status.textContent = `Reading ${source.name}.`;
  try {
    const bytes = new Uint8Array(await source.arrayBuffer());
    if (current !== selection) return;
    file = reader.File.fromBytes(bytes);
    datasets = file.datasets();
    for (const dataset of datasets) {
      const option = document.createElement('option');
      option.textContent = `${dataset.name} (${dataset.dtype}, ${dataset.shape.join(' × ')})`;
      choices.append(option);
    }
    fileMetadata.textContent = format({ attributes: file.attributes(), skipped: file.skipped(), eos: file.eos() });
    status.textContent = `${source.name}: ${datasets.length} datasets. The file stays in this browser.`;
    choices.disabled = datasets.length === 0;
    if (datasets.length === 0) {
      previewStatus.textContent = 'No readable datasets. Check the omitted objects below.';
      return;
    }
    const first2d = datasets.findIndex(dataset => dataset.shape.length === 2);
    choices.selectedIndex = Math.max(0, first2d);
    showDataset();
  } catch (error) {
    if (current !== selection) return;
    clearFile();
    status.textContent = `Cannot open ${source.name}: ${error.message}`;
  }
}

picker.addEventListener('change', () => {
  const source = picker.files[0];
  if (source) void openFile(source);
  picker.value = '';
});
choices.addEventListener('change', showDataset);
document.addEventListener('dragover', event => event.preventDefault());
document.addEventListener('drop', event => {
  event.preventDefault();
  if (picker.disabled) return;
  const sources = event.dataTransfer.files;
  if (sources.length !== 1) {
    status.textContent = 'Drop one HDF4 file at a time.';
    return;
  }
  void openFile(sources[0]);
});
window.addEventListener('pagehide', () => {
  selection += 1;
  clearFile();
});

try {
  reader = await import('./pkg/hdfour_wasm.js');
  await reader.default();
  picker.disabled = false;
  status.textContent = 'Ready to open a file.';
} catch (error) {
  status.textContent = `The reader could not load: ${error.message}. Build the demo and serve it over HTTP.`;
}
