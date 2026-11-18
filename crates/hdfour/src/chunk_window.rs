//! Assembly of only the selected rows within intersecting chunks.

use super::{Prepared, fill};
use crate::Result;
use crate::read::{ChunkStorage, Reader, buffer, malformed};
use crate::window::Window as Selection;

pub(crate) fn read_prepared(
    reader: &Reader<'_>,
    prepared: &Prepared,
    selection: &Selection<'_>,
    output: &mut [u8],
) -> Result<()> {
    fill(output, &prepared.header.fill_value);
    if output.is_empty() {
        return Ok(());
    }
    let window = Window {
        start: selection.start,
        count: selection.count,
        chunks: &prepared.chunks,
        width: usize::try_from(prepared.header.number_type_size)
            .map_err(|_| malformed("chunk number type width"))?,
    };
    let mut chunk = Vec::new();
    for (origin, storage) in &prepared.entries {
        if matches!(storage, ChunkStorage::Fill) {
            continue;
        }
        let Some(overlap) = window.overlap(origin)? else {
            continue;
        };
        if chunk.is_empty() {
            chunk = buffer(prepared.bytes)?;
        }
        reader.read_prepared_chunk(storage, &mut chunk)?;
        window.copy(&overlap, &chunk, output)?;
    }
    Ok(())
}

#[cfg(test)]
use super::table;

#[cfg(test)]
fn read(
    reader: &Reader<'_>,
    header: &crate::special::Chunked,
    dataset: &crate::Dataset,
    start: &[u64],
    count: &[u64],
    output: &mut [u8],
) -> Result<()> {
    super::validate_dataset(reader, header, dataset)?;
    let selection = Selection::new(&dataset.shape, start, count)?;
    let expected = crate::read::array_bytes(count, dataset.dtype.size(), reader.limits())?;
    if output.len() != expected {
        return Err(crate::Error::WrongLength {
            expected,
            actual: output.len(),
        });
    }
    let prepared = super::prepare(reader, header.clone(), Some(&selection))?;
    read_prepared(reader, &prepared, &selection, output)
}

struct Window<'a> {
    start: &'a [u64],
    count: &'a [u64],
    chunks: &'a [u64],
    width: usize,
}

struct Axis {
    source: u64,
    target: u64,
    length: u64,
}

impl Window<'_> {
    fn overlap(&self, origin: &[u64]) -> Result<Option<Vec<Axis>>> {
        if origin.len() != self.start.len() || self.chunks.len() != origin.len() {
            return Err(malformed("chunk rank mismatch"));
        }
        let mut overlap = Vec::with_capacity(origin.len());
        for (((&index, &extent), &start), &count) in origin
            .iter()
            .zip(self.chunks)
            .zip(self.start)
            .zip(self.count)
        {
            let chunk_start = index
                .checked_mul(extent)
                .ok_or_else(|| malformed("chunk origin overflow"))?;
            let chunk_end = chunk_start
                .checked_add(extent)
                .ok_or_else(|| malformed("chunk end overflow"))?;
            let end = start
                .checked_add(count)
                .ok_or_else(|| malformed("window end overflow"))?;
            let first = chunk_start.max(start);
            let last = chunk_end.min(end);
            if first >= last {
                return Ok(None);
            }
            overlap.push(Axis {
                source: first
                    .checked_sub(chunk_start)
                    .ok_or_else(|| malformed("chunk intersection offset"))?,
                target: first
                    .checked_sub(start)
                    .ok_or_else(|| malformed("window intersection offset"))?,
                length: last
                    .checked_sub(first)
                    .ok_or_else(|| malformed("chunk intersection length"))?,
            });
        }
        Ok(Some(overlap))
    }

    fn copy(&self, overlap: &[Axis], chunk: &[u8], output: &mut [u8]) -> Result<()> {
        let (last, prefix) = overlap
            .split_last()
            .ok_or_else(|| malformed("empty chunk intersection"))?;
        let rows = prefix.iter().try_fold(1_u64, |rows, axis| {
            rows.checked_mul(axis.length)
                .ok_or_else(|| malformed("window row count overflow"))
        })?;
        let bytes = last
            .length
            .checked_mul(self.width as u64)
            .ok_or_else(|| malformed("window row byte size overflow"))?;
        for row in 0..rows {
            let (source, target) = self.row_offsets(overlap, row)?;
            let source = byte_range(source, bytes)?;
            let target = byte_range(target, bytes)?;
            output
                .get_mut(target)
                .ok_or_else(|| malformed("chunk exceeds window output"))?
                .copy_from_slice(
                    chunk
                        .get(source)
                        .ok_or_else(|| malformed("short chunk row"))?,
                );
        }
        Ok(())
    }

    fn row_offsets(&self, overlap: &[Axis], mut row: u64) -> Result<(u64, u64)> {
        let mut source = 0_u64;
        let mut target = 0_u64;
        let mut source_stride = self.width as u64;
        let mut target_stride = self.width as u64;
        for (index, ((axis, &extent), &count)) in overlap
            .iter()
            .zip(self.chunks)
            .zip(self.count)
            .rev()
            .enumerate()
        {
            let relative = if index == 0 {
                0
            } else {
                let relative = row
                    .checked_rem(axis.length)
                    .ok_or_else(|| malformed("zero intersection extent"))?;
                row = row
                    .checked_div(axis.length)
                    .ok_or_else(|| malformed("zero intersection extent"))?;
                relative
            };
            source = axis
                .source
                .checked_add(relative)
                .and_then(|coordinate| coordinate.checked_mul(source_stride))
                .and_then(|offset| source.checked_add(offset))
                .ok_or_else(|| malformed("chunk row offset overflow"))?;
            target = axis
                .target
                .checked_add(relative)
                .and_then(|coordinate| coordinate.checked_mul(target_stride))
                .and_then(|offset| target.checked_add(offset))
                .ok_or_else(|| malformed("window row offset overflow"))?;
            source_stride = source_stride
                .checked_mul(extent)
                .ok_or_else(|| malformed("chunk stride overflow"))?;
            target_stride = target_stride
                .checked_mul(count)
                .ok_or_else(|| malformed("window stride overflow"))?;
        }
        Ok((source, target))
    }
}

fn byte_range(start: u64, length: u64) -> Result<std::ops::Range<usize>> {
    let end = start
        .checked_add(length)
        .ok_or_else(|| malformed("window byte range overflow"))?;
    Ok(
        usize::try_from(start).map_err(|_| malformed("window byte offset size"))?
            ..usize::try_from(end).map_err(|_| malformed("window byte end size"))?,
    )
}

#[cfg(test)]
#[allow(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::expect_used,
    clippy::panic
)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn selected_chunk_rows_match_coordinate_values(
            dimensions in prop::collection::vec((1_u64..8, 1_u64..6, any::<u8>(), any::<u8>()), 1..5),
            width in 1_usize..5,
        ) {
            let shape: Vec<_> = dimensions.iter().map(|dimension| dimension.0).collect();
            let chunks: Vec<_> = dimensions.iter().map(|dimension| dimension.1).collect();
            let start: Vec<_> = dimensions.iter().map(|dimension| u64::from(dimension.2) % dimension.0).collect();
            let count: Vec<_> = dimensions.iter().zip(&start).map(|(dimension, start)| 1 + u64::from(dimension.3) % (dimension.0 - start)).collect();
            let window = Window { start: &start, count: &count, chunks: &chunks, width };
            let mut output = vec![0; usize::try_from(count.iter().product::<u64>()).expect("small output") * width];
            let grid: Vec<_> = shape.iter().zip(&chunks).map(|(size, chunk)| size.div_ceil(*chunk)).collect();
            let mut origin = vec![0; shape.len()];
            loop {
                let elements = chunks.iter().product::<u64>();
                let mut chunk = Vec::new();
                for flat in 0..elements {
                    let mut remainder = flat;
                    let mut coordinates = vec![0; shape.len()];
                    for axis in (0..shape.len()).rev() {
                        coordinates[axis] = origin[axis] * chunks[axis] + remainder % chunks[axis];
                        remainder /= chunks[axis];
                    }
                    let linear = coordinates.iter().zip(&shape).fold(0, |flat, (coordinate, size)| flat * size + coordinate);
                    for byte in 0..width {
                        chunk.push(u8::try_from((linear + byte as u64) % 251).expect("byte"));
                    }
                }
                if let Some(overlap) = window.overlap(&origin).expect("valid intersection") {
                    window.copy(&overlap, &chunk, &mut output).expect("valid copy");
                }
                let mut axis = shape.len();
                while axis != 0 {
                    axis -= 1;
                    origin[axis] += 1;
                    if origin[axis] != grid[axis] { break; }
                    origin[axis] = 0;
                }
                if origin.iter().all(|coordinate| *coordinate == 0) { break; }
            }
            let mut expected = Vec::new();
            for flat in 0..count.iter().product::<u64>() {
                let mut remainder = flat;
                let mut coordinates = vec![0; shape.len()];
                for axis in (0..shape.len()).rev() {
                    coordinates[axis] = start[axis] + remainder % count[axis];
                    remainder /= count[axis];
                }
                let linear = coordinates.iter().zip(&shape).fold(0, |flat, (coordinate, size)| flat * size + coordinate);
                for byte in 0..width {
                    expected.push(u8::try_from((linear + byte as u64) % 251).expect("byte"));
                }
            }
            prop_assert_eq!(output, expected);
        }
    }

    struct RejectReads {
        memory: crate::Memory<Vec<u8>>,
        forbidden: Vec<std::ops::Range<u64>>,
    }

    impl crate::Source for RejectReads {
        fn len(&self) -> u64 {
            self.memory.len()
        }

        fn read_at(&self, offset: u64, output: &mut [u8]) -> Result<()> {
            let end = offset + output.len() as u64;
            if self
                .forbidden
                .iter()
                .any(|range| offset < range.end && end > range.start)
            {
                return Err(malformed("unselected chunk was read"));
            }
            self.memory.read_at(offset, output)
        }
    }

    #[test]
    fn fixture_windows_skip_unselected_raw_and_compressed_chunk_storage() {
        use crate::{Limits, Memory, Source, dd};

        for bytes in [
            include_bytes!("../../../fixtures/F07.hdf").as_slice(),
            include_bytes!("../../../fixtures/F08.hdf").as_slice(),
        ] {
            let file = crate::File::from_bytes(bytes.to_vec()).expect("fixture");
            let dataset = &file.datasets()[0];
            let source = Memory::new(bytes.to_vec());
            let index = dd::parse(&source, &Limits::default()).expect("index");
            let reader = Reader {
                source: &source,
                index: &index,
            };
            let header = match reader
                .special(
                    reader
                        .descriptor(dataset.data.expect("storage"))
                        .expect("descriptor"),
                )
                .expect("special header")
            {
                crate::special::Special::Chunked(header) => header,
                _ => panic!("chunked fixture"),
            };
            let table = table(&reader, &header).expect("chunk table");
            let expected = file.read(dataset).expect("whole array");
            let crate::Value::I16(expected) = expected.data else {
                panic!("i16");
            };
            for (start, count) in [
                ([0, 3], [1, 1]),
                ([1, 1], [3, 5]),
                ([4, 6], [1, 1]),
                ([5, 0], [0, 7]),
            ] {
                let chunks: Vec<_> = header
                    .dimensions
                    .iter()
                    .map(|dimension| dimension.chunk_length)
                    .collect();
                let window = Window {
                    start: &start,
                    count: &count,
                    chunks: &chunks,
                    width: 2,
                };
                let mut forbidden = Vec::new();
                for (origin, &(tag, reference)) in &table {
                    if tag == 1 || window.overlap(origin).expect("overlap").is_some() {
                        continue;
                    }
                    let descriptor = reader
                        .descriptor((tag, reference))
                        .expect("chunk descriptor");
                    let offset = descriptor.offset.expect("written chunk");
                    forbidden.push(offset..offset + descriptor.length);
                    if descriptor.is_special() {
                        if let crate::special::Special::Compressed(compressed) =
                            reader.special(descriptor).expect("compressed header")
                        {
                            let payload = reader
                                .descriptor((40, compressed.reference))
                                .expect("compressed payload");
                            let offset = payload.offset.expect("written payload");
                            forbidden.push(offset..offset + payload.length);
                        }
                    }
                }
                assert!(!forbidden.is_empty(), "test must exclude stored chunks");
                let watched = RejectReads {
                    memory: source.clone(),
                    forbidden,
                };
                assert_eq!(watched.len(), bytes.len() as u64);
                let watched_reader = Reader {
                    source: &watched,
                    index: &index,
                };
                let mut output =
                    vec![0; usize::try_from(count[0] * count[1] * 2).expect("small window")];
                read(
                    &watched_reader,
                    &header,
                    dataset,
                    &start,
                    &count,
                    &mut output,
                )
                .expect("window without excluded storage");
                let mut selected = Vec::new();
                for row in start[0]..start[0] + count[0] {
                    for column in start[1]..start[1] + count[1] {
                        let index = usize::try_from(row * 7 + column).expect("small index");
                        selected.extend(match dataset.order {
                            crate::ByteOrder::Big => expected[index].to_be_bytes(),
                            crate::ByteOrder::Little => expected[index].to_le_bytes(),
                        });
                    }
                }
                assert_eq!(output, selected);
            }
        }
    }
}
