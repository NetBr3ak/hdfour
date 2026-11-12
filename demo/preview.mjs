export function greyscale(values, shape, fill) {
  if (shape.length !== 2 || !shape.every(Number.isSafeInteger)
    || shape.some(size => size < 0) || shape[0] * shape[1] !== values.length) {
    throw new Error('The preview needs a matching two-dimensional shape.');
  }
  let min = Infinity;
  let max = -Infinity;
  let count = 0;
  for (const value of values) {
    if (!Number.isFinite(value) || value === fill) continue;
    min = Math.min(min, value);
    max = Math.max(max, value);
    count += 1;
  }
  const [rows, columns] = shape;
  const scale = Math.min(1, 512 / columns, 512 / rows);
  const width = columns === 0 ? 0 : Math.max(1, Math.round(columns * scale));
  const height = rows === 0 ? 0 : Math.max(1, Math.round(rows * scale));
  const pixels = new Uint8ClampedArray(width * height * 4);
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const value = values[Math.floor(y * rows / height) * columns + Math.floor(x * columns / width)];
      if (!Number.isFinite(value) || value === fill) continue;
      const range = max - min;
      const unit = Number.isFinite(range) ? (value - min) / range
        : (value / 2 - min / 2) / (max / 2 - min / 2);
      const grey = min === max ? 128 : Math.round(unit * 255);
      const offset = (y * width + x) * 4;
      pixels.set([grey, grey, grey, 255], offset);
    }
  }
  return { width, height, pixels, min, max, count };
}
