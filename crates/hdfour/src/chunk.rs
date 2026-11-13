//! Chunk tables and array assembly.
#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::collections::BTreeMap;

use crate::read::{Reader, allocation, array_bytes, buffer, malformed};
use crate::special::Chunked;
use crate::{Dataset, Result, Value, dd, vset};

type Entries = BTreeMap<Vec<u64>, (u16, u16)>;

pub(crate) fn validate(reader: &Reader<'_>, header: &Chunked) -> Result<u64> {
    let shape: Vec<_> = header
        .dimensions
        .iter()
        .map(|dimension| dimension.length)
        .collect();
    let chunks: Vec<_> = header
        .dimensions
        .iter()
        .map(|dimension| dimension.chunk_length)
        .collect();
    let width = usize::try_from(header.number_type_size)
        .map_err(|_| malformed("chunk number type width"))?;
    let bytes = array_bytes(&shape, width, reader.limits())?;
    array_bytes(&chunks, width, reader.limits())?;
    let elements = product(&shape)?;
    if elements != header.length || product(&chunks)? != header.chunk_size {
        return Err(malformed("chunk header length disagrees with dimensions"));
    }
    if header.fill_value.len() != width {
        return Err(malformed("chunk fill length differs from number type"));
    }
    let count = header
        .dimensions
        .iter()
        .try_fold(1_u64, |count, dimension| {
            let chunks = dimension
                .length
                .checked_div(dimension.chunk_length)
                .and_then(|whole| {
                    whole.checked_add(u64::from(
                        dimension.length.checked_rem(dimension.chunk_length)? != 0,
                    ))
                })
                .ok_or_else(|| malformed("chunk grid overflow"))?;
            count
                .checked_mul(chunks)
                .ok_or_else(|| malformed("chunk count overflow"))
        })?;
    dd::check_limit("chunks", count, reader.limits().max_chunks)?;
    Ok(bytes as u64)
}

fn product(values: &[u64]) -> Result<u64> {
    values.iter().try_fold(1_u64, |product, value| {
        product
            .checked_mul(*value)
            .ok_or_else(|| malformed("chunk geometry overflow"))
    })
}

fn table(reader: &Reader<'_>, header: &Chunked) -> Result<Entries> {
    if header.table_tag != 1962 {
        return Err(malformed("chunk table must be Vdata"));
    }
    let bytes = reader.bytes(reader.descriptor((1962, header.table_ref))?)?;
    let table = vset::parse_vdata_header(&bytes)?;
    if !table.name.starts_with("_HDF_CHK_TBL_") || table.class != "_HDF_CHK_TBL_0" {
        return Err(malformed("invalid chunk table identity"));
    }
    dd::check_limit(
        "chunks",
        u64::from(table.records),
        reader.limits().max_chunks,
    )?;
    let length = u64::from(table.records)
        .checked_mul(u64::from(table.record_size))
        .ok_or_else(|| malformed("chunk table size overflow"))?;
    allocation(length, reader.limits())?;
    let bytes = reader.records(&table, header.table_ref)?;
    let field = |name| {
        table
            .fields
            .iter()
            .position(|field| field.name == name)
            .ok_or_else(|| malformed("missing chunk table field"))
    };
    // hchunks.c:266-271: origins are chunk indices; each record also has a tag and reference.
    let Value::I32(origins) = vset::field_values(&table, &bytes, field("origin")?)? else {
        return Err(malformed("chunk origin must be i32"));
    };
    let Value::U16(tags) = vset::field_values(&table, &bytes, field("chk_tag")?)? else {
        return Err(malformed("chunk tag must be u16"));
    };
    let Value::U16(references) = vset::field_values(&table, &bytes, field("chk_ref")?)? else {
        return Err(malformed("chunk reference must be u16"));
    };
    let rank = header.dimensions.len();
    let expected = usize::try_from(table.records)
        .ok()
        .and_then(|records| records.checked_mul(rank))
        .ok_or_else(|| malformed("chunk origin count overflow"))?;
    if rank == 0
        || origins.len() != expected
        || tags.len() as u64 != u64::from(table.records)
        || references.len() != tags.len()
    {
        return Err(malformed("chunk table field order mismatch"));
    }
    let mut entries = BTreeMap::new();
    for ((origin, tag), reference) in origins.chunks_exact(rank).zip(tags).zip(references) {
        // hchunks.c:2376-2409 accepts DFTAG_NULL (1), not wildcard tag 0.
        if tag == 0 {
            return Err(malformed("invalid chunk tag"));
        }
        let origin = origin
            .iter()
            .zip(&header.dimensions)
            .map(|(index, dimension)| {
                let index =
                    u64::try_from(*index).map_err(|_| malformed("negative chunk origin"))?;
                let start = index
                    .checked_mul(dimension.chunk_length)
                    .ok_or_else(|| malformed("chunk origin overflow"))?;
                if start >= dimension.length {
                    return Err(malformed("chunk origin outside array"));
                }
                Ok(index)
            })
            .collect::<Result<Vec<_>>>()?;
        if entries.insert(origin, (tag, reference)).is_some() {
            return Err(malformed("duplicate chunk origin"));
        }
    }
    Ok(entries)
}

pub(crate) fn prepare(reader: &Reader<'_>, header: &Chunked) -> Result<()> {
    validate(reader, header)?;
    let bytes = chunk_bytes(reader, header)?;
    for &(tag, reference) in table(reader, header)?.values() {
        // hchunks.c:2624-2635 fills explicit DFTAG_NULL (1) records.
        if tag != 1 {
            reader.prepare_chunk((tag, reference), bytes)?;
        }
    }
    Ok(())
}

fn chunk_bytes(reader: &Reader<'_>, header: &Chunked) -> Result<usize> {
    let length = header
        .chunk_size
        .checked_mul(u64::from(header.number_type_size))
        .ok_or_else(|| malformed("chunk byte size overflow"))?;
    allocation(length, reader.limits())
}

/// Writes `value` into `output` once per complete element; trailing bytes stay untouched.
///
/// One call per element costs more than the copy itself (92 million two-byte copies took 43 percent of a
/// 184 MB chunked read). A value whose bytes are all equal becomes one fill; any other value is written once
/// and the filled part is then doubled, so the number of copies grows with the logarithm of the length.
fn fill(output: &mut [u8], value: &[u8]) {
    let Some((&first, rest)) = value.split_first() else {
        return;
    };
    let trailing = output.len().checked_rem(value.len()).unwrap_or_default();
    let whole = output.len().saturating_sub(trailing);
    let Some(output) = output.get_mut(..whole) else {
        return;
    };
    if rest.iter().all(|&byte| byte == first) {
        output.fill(first);
        return;
    }
    // No room for even one element: nothing to write.
    let Some(seed) = output.get_mut(..value.len()) else {
        return;
    };
    seed.copy_from_slice(value);
    let mut filled = value.len();
    while filled < whole {
        let step = filled.min(whole.saturating_sub(filled));
        output.copy_within(..step, filled);
        filled = filled.saturating_add(step);
    }
}

pub(crate) fn read(
    reader: &Reader<'_>,
    header: &Chunked,
    dataset: &Dataset,
    output: &mut [u8],
) -> Result<()> {
    validate(reader, header)?;
    if header
        .dimensions
        .iter()
        .map(|dimension| dimension.length)
        .ne(dataset.shape.iter().copied())
        || u64::from(header.number_type_size) != dataset.dtype.size() as u64
    {
        return Err(malformed("chunk geometry differs from dataset"));
    }
    fill(output, &header.fill_value);
    if output.is_empty() {
        return Ok(());
    }
    let entries = table(reader, header)?;
    let chunks: Vec<_> = header
        .dimensions
        .iter()
        .map(|dimension| dimension.chunk_length)
        .collect();
    let assembly = Assembly {
        reader,
        header,
        entries: &entries,
        geometry: Geometry {
            shape: &dataset.shape,
            chunks: &chunks,
            width: dataset.dtype.size(),
        },
    };
    assembly.read_bands(output)
}

struct Geometry<'a> {
    shape: &'a [u64],
    chunks: &'a [u64],
    width: usize,
}
struct Band<'a> {
    output: &'a mut [u8],
    start: u64,
}
struct Assembly<'a, 'b> {
    reader: &'a Reader<'b>,
    header: &'a Chunked,
    entries: &'a Entries,
    geometry: Geometry<'a>,
}

impl Assembly<'_, '_> {
    fn read_bands(&self, output: &mut [u8]) -> Result<()> {
        #[cfg(feature = "parallel")]
        {
            let first = self
                .header
                .dimensions
                .first()
                .ok_or_else(|| malformed("empty chunk dimensions"))?;
            let rows = first.length.div_ceil(first.chunk_length);
            if rows >= 2 {
                let threads = (std::thread::available_parallelism()?.get() as u64).min(rows);
                let rows_per_thread = rows
                    .div_ceil(threads)
                    .checked_mul(first.chunk_length)
                    .ok_or_else(|| malformed("parallel chunk band overflow"))?;
                let row_bytes = self.row_bytes()?;
                let band_bytes = allocation(
                    rows_per_thread
                        .checked_mul(row_bytes)
                        .ok_or_else(|| malformed("parallel band byte overflow"))?
                        .min(output.len() as u64),
                    self.reader.limits(),
                )?;
                return std::thread::scope(|scope| {
                    let mut handles = Vec::new();
                    for (index, output) in output.chunks_mut(band_bytes).enumerate() {
                        let start = (index as u64)
                            .checked_mul(rows_per_thread)
                            .ok_or_else(|| malformed("parallel band offset overflow"))?;
                        handles.push(
                            std::thread::Builder::new().spawn_scoped(scope, move || {
                                self.read_band(Band { output, start })
                            })?,
                        );
                    }
                    for handle in handles {
                        handle
                            .join()
                            .map_err(|_| malformed("chunk worker panicked"))??;
                    }
                    Ok(())
                });
            }
        }
        self.read_band(Band { output, start: 0 })
    }

    fn row_bytes(&self) -> Result<u64> {
        let tail = self
            .geometry
            .shape
            .get(1..)
            .ok_or_else(|| malformed("empty array shape"))?;
        product(tail)?
            .checked_mul(self.geometry.width as u64)
            .ok_or_else(|| malformed("array row size overflow"))
    }

    fn read_band(&self, mut band: Band<'_>) -> Result<()> {
        let first = self
            .header
            .dimensions
            .first()
            .ok_or_else(|| malformed("empty chunk dimensions"))?;
        let rows = (band.output.len() as u64)
            .checked_div(self.row_bytes()?)
            .ok_or_else(|| malformed("zero array row width"))?;
        let end = band
            .start
            .checked_add(rows)
            .ok_or_else(|| malformed("chunk band end overflow"))?;
        let start_chunk = band
            .start
            .checked_div(first.chunk_length)
            .ok_or_else(|| malformed("zero first chunk extent"))?;
        let end_chunk = end.div_ceil(first.chunk_length);
        for (origin, &(tag, reference)) in self.entries.range(vec![start_chunk]..vec![end_chunk]) {
            if tag == 1 {
                continue;
            }
            let bytes = chunk_bytes(self.reader, self.header)?;
            self.reader.prepare_chunk((tag, reference), bytes)?;
            let mut chunk = buffer(bytes)?;
            self.reader.read_chunk((tag, reference), &mut chunk)?;
            copy_chunk(&self.geometry, origin, &chunk, &mut band)?;
        }
        Ok(())
    }
}

fn copy_chunk(
    geometry: &Geometry<'_>,
    origin: &[u64],
    chunk: &[u8],
    band: &mut Band<'_>,
) -> Result<()> {
    let (&last_size, shape) = geometry
        .shape
        .split_last()
        .ok_or_else(|| malformed("empty array shape"))?;
    let (&last_chunk, chunks) = geometry
        .chunks
        .split_last()
        .ok_or_else(|| malformed("empty chunk shape"))?;
    let (&last_origin, origin) = origin
        .split_last()
        .ok_or_else(|| malformed("empty chunk origin"))?;
    if shape.len() != chunks.len() || shape.len() != origin.len() {
        return Err(malformed("chunk rank mismatch"));
    }
    let row_bytes = last_chunk
        .checked_mul(geometry.width as u64)
        .ok_or_else(|| malformed("chunk row size overflow"))?;
    let row_bytes = usize::try_from(row_bytes).map_err(|_| malformed("chunk row size"))?;
    let start_column = last_origin
        .checked_mul(last_chunk)
        .ok_or_else(|| malformed("chunk column overflow"))?;
    let columns = last_size
        .checked_sub(start_column)
        .ok_or_else(|| malformed("chunk column outside array"))?
        .min(last_chunk);
    let copy_bytes = usize::try_from(
        columns
            .checked_mul(geometry.width as u64)
            .ok_or_else(|| malformed("chunk copy size overflow"))?,
    )
    .map_err(|_| malformed("chunk copy size"))?;
    let band_start = band
        .start
        .checked_mul(product(
            geometry
                .shape
                .get(1..)
                .ok_or_else(|| malformed("empty shape"))?,
        )?)
        .and_then(|offset| offset.checked_mul(geometry.width as u64))
        .ok_or_else(|| malformed("band start overflow"))?;
    for (row, source) in chunk.chunks_exact(row_bytes).enumerate() {
        let mut remainder = row as u64;
        let mut target = start_column;
        let mut stride = last_size;
        let mut outside = false;
        for ((&size, &extent), &origin) in shape.iter().zip(chunks).zip(origin).rev() {
            let coordinate = origin
                .checked_mul(extent)
                .and_then(|start| start.checked_add(remainder.checked_rem(extent)?))
                .ok_or_else(|| malformed("chunk coordinate overflow"))?;
            remainder = remainder
                .checked_div(extent)
                .ok_or_else(|| malformed("zero chunk extent"))?;
            if coordinate >= size {
                outside = true;
                break;
            }
            target = target
                .checked_add(
                    coordinate
                        .checked_mul(stride)
                        .ok_or_else(|| malformed("chunk coordinate overflow"))?,
                )
                .ok_or_else(|| malformed("chunk target overflow"))?;
            stride = stride
                .checked_mul(size)
                .ok_or_else(|| malformed("array stride overflow"))?;
        }
        if outside {
            continue;
        }
        let target = target
            .checked_mul(geometry.width as u64)
            .and_then(|offset| offset.checked_sub(band_start))
            .ok_or_else(|| malformed("chunk outside output band"))?;
        let start = usize::try_from(target).map_err(|_| malformed("chunk target size"))?;
        let end = start
            .checked_add(copy_bytes)
            .ok_or_else(|| malformed("chunk target end overflow"))?;
        let output = band
            .output
            .get_mut(start..end)
            .ok_or_else(|| malformed("chunk exceeds output band"))?;
        output.copy_from_slice(
            source
                .get(..copy_bytes)
                .ok_or_else(|| malformed("short chunk row"))?,
        );
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::expect_used
)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn fill_matches_one_copy_per_element(
            value in prop::collection::vec(any::<u8>(), 0..17),
            length in 0_usize..4096,
        ) {
            let mut expected = vec![0xA5_u8; length];
            if !value.is_empty() {
                for element in expected.chunks_exact_mut(value.len()) {
                    element.copy_from_slice(&value);
                }
            }
            let mut output = vec![0xA5_u8; length];
            fill(&mut output, &value);
            prop_assert_eq!(output, expected);
        }

        #[test]
        fn coordinate_filled_chunks_assemble(shape in prop::collection::vec(1_u64..8, 1..5), chunk_extent in 1_u64..5) {
            let chunks = vec![chunk_extent; shape.len()];
            let count: u64 = shape.iter().product();
            let mut output = vec![0; usize::try_from(count).expect("small count")];
            let grid: Vec<_> = shape.iter().map(|size| size.div_ceil(chunk_extent)).collect();
            let mut origin = vec![0; shape.len()];
            loop {
                let chunk_count: u64 = chunks.iter().product();
                let mut bytes = vec![0; usize::try_from(chunk_count).expect("small chunk")];
                for (flat, value) in bytes.iter_mut().enumerate() {
                    let mut remainder = flat as u64;
                    let mut coordinates = vec![0; shape.len()];
                    for axis in (0..shape.len()).rev() {
                        coordinates[axis] = origin[axis] * chunk_extent + remainder % chunk_extent;
                        remainder /= chunk_extent;
                    }
                    if coordinates.iter().zip(&shape).all(|(coordinate, size)| coordinate < size) {
                        let linear = coordinates.iter().zip(&shape).fold(0, |flat, (coordinate, size)| flat * size + coordinate);
                        *value = u8::try_from(linear % 251).expect("byte");
                    }
                }
                copy_chunk(&Geometry { shape: &shape, chunks: &chunks, width: 1 }, &origin, &bytes, &mut Band { output: &mut output, start: 0 }) .expect("assembly");
                let mut axis = shape.len();
                while axis != 0 {
                    axis -= 1;
                    origin[axis] += 1;
                    if origin[axis] != grid[axis] { break; }
                    origin[axis] = 0;
                }
                if origin.iter().all(|coordinate| *coordinate == 0) { break; }
            }
            let expected: Vec<_> = (0..count).map(|value| u8::try_from(value % 251).expect("byte")).collect();
            prop_assert_eq!(output, expected);
        }
    }
}
