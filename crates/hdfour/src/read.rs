//! File opening and dataset reads.
#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::dd::{self, Descriptor, Index};
use crate::special::{self, Special};
use crate::window::Window;
use crate::{
    Array, Attribute, ByteOrder, Codec, DType, Dataset, Eos, Error, Limits, Memory, Result,
    Skipped, Source, Storage, Value, chunk, codec, element, nt, sd,
};

/// An open read-only file. Array payloads are read only when requested.
pub struct File {
    source: Box<dyn Source>,
    index: Index,
    metadata: sd::Metadata,
}

impl File {
    /// Retain the byte container without copying it and read its metadata.
    pub fn from_bytes(bytes: impl AsRef<[u8]> + Send + Sync + 'static) -> Result<Self> {
        Self::from_source(Memory::new(bytes), Limits::default())
    }

    /// Read metadata through a positional byte source with per-file resource limits.
    pub fn from_source(source: impl Source + 'static, limits: Limits) -> Result<Self> {
        let index = dd::parse(&source, &limits)?;
        let metadata = sd::parse(&Reader {
            source: &source,
            index: &index,
        })?;
        Ok(Self {
            source: Box::new(source),
            index,
            metadata,
        })
    }

    pub fn datasets(&self) -> &[Dataset] {
        &self.metadata.datasets
    }

    /// Select a unique name; duplicate names must be selected through `datasets()`.
    pub fn dataset(&self, name: &str) -> Result<&Dataset> {
        let mut matches = self
            .datasets()
            .iter()
            .filter(|dataset| dataset.name == name);
        let first = matches.next().ok_or_else(|| Error::NotFound(name.into()))?;
        let count = matches
            .count()
            .checked_add(1)
            .ok_or_else(|| malformed("dataset count overflow"))?;
        if count != 1 {
            return Err(Error::Ambiguous {
                name: name.into(),
                count,
            });
        }
        Ok(first)
    }

    pub fn attributes(&self) -> &[Attribute] {
        &self.metadata.attributes
    }
    pub fn skipped(&self) -> &[Skipped] {
        &self.metadata.skipped
    }
    pub fn eos(&self) -> Option<&Eos> {
        self.metadata.eos.as_ref()
    }

    /// Read a whole C-order array, without applying calibration or masking fill values.
    /// The dataset must be borrowed from this file; cloned or foreign metadata is rejected.
    pub fn read(&self, dataset: &Dataset) -> Result<Array> {
        let (reader, plan) = self.prepare_read(dataset)?;
        read_values(dataset.dtype, &dataset.shape, plan.bytes, |out| {
            reader.read_native(dataset, &plan.storage, out)
        })
    }

    /// Read a rectangular region as raw, native-endian values in C order.
    /// `start` is zero-based and `count` is the number of values along each axis.
    /// Both must match the dataset rank. A zero count returns an empty array.
    /// Plain arrays use positional reads; chunked arrays decode only overlapping chunks.
    /// Linked storage and standalone compression may require reading the full array.
    pub fn read_window(&self, dataset: &Dataset, start: &[u64], count: &[u64]) -> Result<Array> {
        let reader = self.reader_for(dataset)?;
        let window = Window::new(&dataset.shape, start, count)?;
        if count == dataset.shape {
            return self.read(dataset);
        }
        let plan = reader
            .read_plan(dataset, Some(&window))
            .map_err(|error| named(error, dataset))?;
        let bytes = array_bytes(count, dataset.dtype.size(), reader.limits())?;
        read_values(dataset.dtype, count, bytes, |output| {
            if !output.is_empty() {
                reader
                    .read_region(dataset, &window, &plan, output)
                    .map_err(|error| named(error, dataset))?;
                to_native(dataset, output);
            }
            Ok(())
        })
    }

    /// Validate the dataset and return the byte length of its whole array.
    /// The dataset must be borrowed from this file; cloned or foreign metadata is rejected.
    /// Applies the file's [`Limits`] and returns [`Error::LimitExceeded`] when the array
    /// is larger than the limit allows.
    pub fn byte_len(&self, dataset: &Dataset) -> Result<usize> {
        self.prepare_read(dataset).map(|(_, plan)| plan.bytes)
    }

    /// Read a whole array into an exact-length buffer in C order and native byte order.
    /// No calibration or fill-value masking is applied. The dataset must be borrowed
    /// from this file; cloned or foreign metadata is rejected.
    /// On error, the contents of `output` are unspecified.
    pub fn read_into(&self, dataset: &Dataset, output: &mut [u8]) -> Result<()> {
        let (reader, plan) = self.prepare_read(dataset)?;
        if output.len() != plan.bytes {
            return Err(Error::WrongLength {
                expected: plan.bytes,
                actual: output.len(),
            });
        }
        reader.read_native(dataset, &plan.storage, output)
    }

    fn prepare_read(&self, dataset: &Dataset) -> Result<(Reader<'_>, ReadPlan)> {
        let reader = self.reader_for(dataset)?;
        let plan = reader
            .read_plan(dataset, None)
            .map_err(|error| named(error, dataset))?;
        Ok((reader, plan))
    }

    fn reader_for(&self, dataset: &Dataset) -> Result<Reader<'_>> {
        if !self
            .datasets()
            .as_ptr_range()
            .contains(&std::ptr::from_ref(dataset))
        {
            return Err(Error::NotFound(dataset.name.clone()));
        }
        Ok(Reader {
            source: self.source.as_ref(),
            index: &self.index,
        })
    }
}

pub(crate) struct Reader<'a> {
    pub source: &'a dyn Source,
    pub index: &'a Index,
}

impl Reader<'_> {
    pub fn limits(&self) -> &Limits {
        self.index.limits()
    }

    pub fn descriptor(&self, key: (u16, u16)) -> Result<&Descriptor> {
        self.index
            .get(key.0, key.1)
            .ok_or_else(|| malformed("missing referenced element"))
    }

    pub fn bytes(&self, descriptor: &Descriptor) -> Result<Vec<u8>> {
        let length = element::len(self.source, self.index, descriptor)?;
        let mut bytes = buffer(allocation(length, self.limits())?)?;
        element::read(self.source, self.index, descriptor, &mut bytes)?;
        Ok(bytes)
    }

    pub fn records(&self, header: &crate::vset::VdataHeader, reference: u16) -> Result<Vec<u8>> {
        if header.interlace != 0 {
            return Err(Error::Unsupported {
                feature: "vdata-interlace",
                object: header.name.clone(),
            });
        }
        let length = u64::from(header.records)
            .checked_mul(u64::from(header.record_size))
            .ok_or_else(|| malformed("Vdata record size overflow"))?;
        allocation(length, self.limits())?;
        let descriptor = self.descriptor((1963, reference))?;
        if element::len(self.source, self.index, descriptor)? != length {
            return Err(malformed("Vdata storage length differs from its records"));
        }
        self.bytes(descriptor)
    }

    pub fn special(&self, descriptor: &Descriptor) -> Result<Special> {
        let offset = descriptor
            .offset
            .ok_or_else(|| malformed("unwritten special header"))?;
        let mut prefix = [0; 2];
        if descriptor.length < 2 {
            return Err(malformed("truncated special header"));
        }
        dd::read_at(self.source, offset, &mut prefix)?;
        let mut length = descriptor.length;
        if prefix == [0, 5] {
            let mut fixed = [0; 35];
            if descriptor.length < fixed.len() as u64 {
                return Err(malformed("truncated chunk header"));
            }
            dd::read_at(self.source, offset, &mut fixed)?;
            let body = word32(
                fixed
                    .get(2..6)
                    .ok_or_else(|| malformed("chunk header length"))?,
            )?;
            if body > i32::MAX as u32 {
                return Err(malformed("negative chunk header length"));
            }
            dd::check_limit(
                "chunk_header_bytes",
                u64::from(body),
                self.limits().max_chunk_header_bytes.min(256),
            )?;
            let rank = word32(
                fixed
                    .get(31..35)
                    .ok_or_else(|| malformed("chunk header rank"))?,
            )?;
            dd::check_limit("rank", u64::from(rank), self.limits().max_rank)?;
            length = u64::from(body)
                .checked_add(6)
                .ok_or_else(|| malformed("chunk header size overflow"))?;
            let flags = word32(fixed.get(7..11).ok_or_else(|| malformed("chunk flags"))?)?;
            if flags & 0xff == 3 {
                // hchunks.c:1055-1088 appends a u16 code, i32 size and compression header.
                let end = length
                    .checked_add(6)
                    .ok_or_else(|| malformed("chunk compression header overflow"))?;
                if end > descriptor.length {
                    return Err(malformed("truncated chunk compression header"));
                }
                let mut extension = [0; 6];
                dd::read_at(
                    self.source,
                    offset
                        .checked_add(length)
                        .ok_or_else(|| malformed("chunk header offset overflow"))?,
                    &mut extension,
                )?;
                let extension_size = word32(
                    extension
                        .get(2..)
                        .ok_or_else(|| malformed("chunk compression size"))?,
                )?;
                if extension_size > i32::MAX as u32 {
                    return Err(malformed("negative chunk compression size"));
                }
                dd::check_limit(
                    "chunk_header_bytes",
                    u64::from(extension_size),
                    self.limits().max_chunk_header_bytes.min(256),
                )?;
                length = end
                    .checked_add(u64::from(extension_size))
                    .ok_or_else(|| malformed("chunk compression header size overflow"))?;
            }
            if length > descriptor.length {
                return Err(malformed("truncated chunk header"));
            }
        }
        let mut bytes = buffer(allocation(length, self.limits())?)?;
        dd::read_at(self.source, offset, &mut bytes)?;
        special::parse(&bytes, self.limits())
    }

    #[cfg(test)]
    pub fn layout(&self, key: Option<(u16, u16)>) -> Result<(Storage, Codec, u64)> {
        self.layout_with_budget(key, self.index.metadata())
    }

    pub fn layout_with_budget(
        &self,
        key: Option<(u16, u16)>,
        budget: &crate::metadata::Budget,
    ) -> Result<(Storage, Codec, u64)> {
        let Some(key) = key else {
            return Ok((Storage::Empty, Codec::None, 0));
        };
        let descriptor = self.descriptor(key)?;
        if descriptor.offset.is_none() {
            return Ok((Storage::Empty, Codec::None, 0));
        }
        if !descriptor.is_special() {
            allocation(descriptor.length, self.limits())?;
            return Ok((Storage::Contiguous, Codec::None, descriptor.length));
        }
        match self.special(descriptor)? {
            Special::Linked(header) => {
                allocation(header.length, self.limits())?;
                Ok((Storage::Linked, Codec::None, header.length))
            }
            Special::Compressed(header) => {
                allocation(header.length, self.limits())?;
                Ok((Storage::Contiguous, header.compression.codec, header.length))
            }
            Special::External => Ok((Storage::External, Codec::None, 0)),
            Special::Chunked(header) => {
                let length = chunk::validate(self, &header)?;
                budget.items::<u64>(header.dimensions.len())?;
                let shape = header
                    .dimensions
                    .iter()
                    .map(|dim| dim.chunk_length)
                    .collect();
                Ok((
                    Storage::Chunked { chunk_shape: shape },
                    header
                        .compression
                        .map_or(Codec::None, |compression| compression.codec),
                    length,
                ))
            }
        }
    }

    fn check_dataset(&self, dataset: &Dataset) -> Result<usize> {
        let bytes = array_bytes(&dataset.shape, dataset.dtype.size(), self.limits())?;
        if dataset.storage == Storage::External {
            return Err(unsupported("external-element", dataset));
        }
        codec::check_ratio(&dataset.codec, 0, 0).map_err(|error| named(error, dataset))?;
        if let Some(fill) = dataset
            .attributes
            .iter()
            .find(|attr| attr.name == "_FillValue")
        {
            if value_type(&fill.value) != dataset.dtype {
                return Err(unsupported("fill-type-mismatch", dataset));
            }
        }
        Ok(bytes)
    }

    #[cfg(test)]
    fn prepare(&self, dataset: &Dataset) -> Result<()> {
        self.read_plan(dataset, None).map(drop)
    }

    fn read_plan(&self, dataset: &Dataset, selection: Option<&Window<'_>>) -> Result<ReadPlan> {
        let bytes = self.check_dataset(dataset)?;
        let storage = self.plan_storage(dataset, bytes, selection)?;
        Ok(ReadPlan { bytes, storage })
    }

    fn plan_storage(
        &self,
        dataset: &Dataset,
        bytes: usize,
        selection: Option<&Window<'_>>,
    ) -> Result<ReadStorage> {
        let Some(key) = dataset.data else {
            return Ok(ReadStorage::Fill);
        };
        let descriptor = self.descriptor(key)?;
        let Some(offset) = descriptor.offset else {
            return Ok(ReadStorage::Fill);
        };
        if !descriptor.is_special() {
            if descriptor.length < bytes as u64 {
                return Err(malformed("element shorter than array"));
            }
            return Ok(ReadStorage::Plain { key, offset });
        }
        match self.special(descriptor)? {
            Special::Compressed(header) if header.length == 0 => Ok(ReadStorage::Fill),
            Special::Compressed(header) => {
                if header.length != bytes as u64 {
                    return Err(malformed("compressed length differs from array"));
                }
                self.check_compressed(&header)?;
                Ok(ReadStorage::Compressed(header))
            }
            Special::Chunked(header) => {
                chunk::validate_dataset(self, &header, dataset)?;
                chunk::prepare(self, header, selection).map(ReadStorage::Chunked)
            }
            Special::Linked(header) if header.length >= bytes as u64 => {
                Ok(ReadStorage::Linked(key))
            }
            Special::Linked(_) => Err(malformed("linked element shorter than array")),
            Special::External => Err(unsupported("external-element", dataset)),
        }
    }

    fn check_compressed(&self, header: &special::Compressed) -> Result<&Descriptor> {
        if header.compression.model != 0 {
            return Err(malformed("unknown compression model"));
        }
        let descriptor = self.descriptor((40, header.reference))?;
        let length = element::len(self.source, self.index, descriptor)?;
        codec::check_ratio(
            &header.compression.codec,
            allocation(length, self.limits())?,
            allocation(header.length, self.limits())?,
        )?;
        Ok(descriptor)
    }

    fn read_native(
        &self,
        dataset: &Dataset,
        storage: &ReadStorage,
        output: &mut [u8],
    ) -> Result<()> {
        self.read_storage(dataset, storage, output)
            .map_err(|error| named(error, dataset))?;
        to_native(dataset, output);
        Ok(())
    }

    fn read_region(
        &self,
        dataset: &Dataset,
        window: &Window<'_>,
        plan: &ReadPlan,
        output: &mut [u8],
    ) -> Result<()> {
        let width = dataset.dtype.size();
        match &plan.storage {
            ReadStorage::Fill => fill_array(dataset, output),
            ReadStorage::Plain { offset: base, .. } => {
                window.read_runs(width, output, |offset, row| {
                    let physical = base
                        .checked_add(offset)
                        .ok_or_else(|| malformed("window offset overflow"))?;
                    dd::read_at(self.source, physical, row)
                })
            }
            ReadStorage::Chunked(prepared) => chunk::read_window(self, prepared, window, output),
            storage => {
                let mut raw = buffer(plan.bytes)?;
                self.read_storage(dataset, storage, &mut raw)?;
                window.read_runs(width, output, |offset, row| {
                    let start = allocation_size(offset)?;
                    let data = raw
                        .get(start..)
                        .and_then(|tail| tail.get(..row.len()))
                        .ok_or_else(|| malformed("window outside decoded array"))?;
                    row.copy_from_slice(data);
                    Ok(())
                })
            }
        }
    }

    fn read_storage(
        &self,
        dataset: &Dataset,
        storage: &ReadStorage,
        output: &mut [u8],
    ) -> Result<()> {
        match storage {
            ReadStorage::Fill => fill_array(dataset, output),
            ReadStorage::Plain { key, .. } | ReadStorage::Linked(key) => {
                element::read(self.source, self.index, self.descriptor(*key)?, output)
            }
            ReadStorage::Compressed(header) => {
                self.decode_prepared(header.reference, header.compression.codec, output)
            }
            ReadStorage::Chunked(prepared) => chunk::read(self, prepared, dataset, output),
        }
    }

    fn decode_prepared(&self, reference: u16, codec: Codec, output: &mut [u8]) -> Result<()> {
        let bytes = self.bytes(self.descriptor((40, reference))?)?;
        codec::decode(&codec, &bytes, output)
    }

    pub(crate) fn chunk_storage(&self, key: (u16, u16), length: usize) -> Result<ChunkStorage> {
        let descriptor = self.descriptor(key)?;
        if !descriptor.is_special() {
            return Ok(ChunkStorage::Element(key));
        }
        match self.special(descriptor)? {
            Special::Compressed(header) => {
                if header.length != length as u64 {
                    return Err(malformed("compressed chunk size mismatch"));
                }
                self.check_compressed(&header)?;
                Ok(ChunkStorage::Compressed {
                    reference: header.reference,
                    codec: header.compression.codec,
                })
            }
            Special::Linked(_) => Ok(ChunkStorage::Element(key)),
            Special::External => Ok(ChunkStorage::External),
            Special::Chunked(_) => Ok(ChunkStorage::Nested),
        }
    }

    pub(crate) fn read_prepared_chunk(
        &self,
        storage: &ChunkStorage,
        output: &mut [u8],
    ) -> Result<()> {
        match storage {
            ChunkStorage::Fill => Ok(()),
            ChunkStorage::Element(key) => {
                element::read(self.source, self.index, self.descriptor(*key)?, output)
            }
            ChunkStorage::Compressed { reference, codec } => {
                self.decode_prepared(*reference, *codec, output)
            }
            ChunkStorage::External => Err(Error::Unsupported {
                feature: "external-element",
                object: "chunk".into(),
            }),
            ChunkStorage::Nested => Err(malformed("nested chunk storage")),
        }
    }
}

struct ReadPlan {
    bytes: usize,
    storage: ReadStorage,
}

enum ReadStorage {
    Fill,
    Plain { key: (u16, u16), offset: u64 },
    Linked((u16, u16)),
    Compressed(special::Compressed),
    Chunked(chunk::Prepared),
}

pub(crate) enum ChunkStorage {
    Fill,
    Element((u16, u16)),
    Compressed { reference: u16, codec: Codec },
    External,
    Nested,
}

fn to_native(dataset: &Dataset, output: &mut [u8]) {
    let native = if cfg!(target_endian = "little") {
        ByteOrder::Little
    } else {
        ByteOrder::Big
    };
    if dataset.order != native {
        nt::swap_in_place(output, dataset.dtype.size());
    }
}

fn read_values(
    dtype: DType,
    shape: &[u64],
    bytes: usize,
    read: impl FnOnce(&mut [u8]) -> Result<()>,
) -> Result<Array> {
    let count = bytes
        .checked_div(dtype.size())
        .ok_or_else(|| malformed("zero value width"))?;
    macro_rules! values {
        ($variant:ident, $ty:ty) => {{
            let mut values = buffer::<$ty>(count)?;
            read(bytemuck::must_cast_slice_mut::<$ty, u8>(&mut values))?;
            Value::$variant(values)
        }};
    }
    let data = match dtype {
        DType::Char8 => values!(Char8, u8),
        DType::UChar8 => values!(UChar8, u8),
        DType::I8 => values!(I8, i8),
        DType::U8 => values!(U8, u8),
        DType::I16 => values!(I16, i16),
        DType::U16 => values!(U16, u16),
        DType::I32 => values!(I32, i32),
        DType::U32 => values!(U32, u32),
        DType::F32 => values!(F32, f32),
        DType::F64 => values!(F64, f64),
    };
    Ok(Array {
        shape: shape.to_vec(),
        data,
    })
}

pub(crate) fn array_bytes(shape: &[u64], width: usize, limits: &Limits) -> Result<usize> {
    dd::check_limit("rank", shape.len() as u64, limits.max_rank)?;
    let count = shape.iter().try_fold(1_u64, |count, size| {
        count
            .checked_mul(*size)
            .ok_or_else(|| malformed("array shape overflow"))
    })?;
    let bytes = count
        .checked_mul(width as u64)
        .ok_or_else(|| malformed("array byte length overflow"))?;
    allocation(bytes, limits)
}

pub(crate) fn allocation(length: u64, limits: &Limits) -> Result<usize> {
    dd::check_limit("array_bytes", length, limits.max_array_bytes)?;
    allocation_size(length)
}

pub(crate) fn allocation_size(length: u64) -> Result<usize> {
    dd::check_limit("allocation_bytes", length, isize::MAX as u64)?;
    usize::try_from(length).map_err(|_| allocation_error(length))
}

pub(crate) fn allocation_error(length: u64) -> Error {
    Error::LimitExceeded {
        limit: "allocation_bytes",
        value: length,
        max: isize::MAX as u64,
    }
}

pub(crate) fn buffer<T: Default + Clone>(count: usize) -> Result<Vec<T>> {
    let bytes = count
        .checked_mul(std::mem::size_of::<T>())
        .ok_or_else(|| allocation_error(u64::MAX))?;
    allocation_size(bytes as u64)?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| allocation_error(bytes as u64))?;
    values.resize(count, T::default());
    Ok(values)
}

pub(crate) fn word32(bytes: &[u8]) -> Result<u32> {
    Ok(u32::from_be_bytes(
        bytes
            .try_into()
            .map_err(|_| malformed("truncated 32-bit number"))?,
    ))
}

pub(crate) fn malformed(what: &'static str) -> Error {
    Error::Malformed { offset: 0, what }
}

fn unsupported(feature: &'static str, dataset: &Dataset) -> Error {
    Error::Unsupported {
        feature,
        object: dataset.name.clone(),
    }
}

fn named(error: Error, dataset: &Dataset) -> Error {
    match error {
        Error::Unsupported { feature, .. } => unsupported(feature, dataset),
        other => other,
    }
}

pub(crate) fn value_type(value: &Value) -> DType {
    match value {
        Value::Char8(_) => DType::Char8,
        Value::UChar8(_) => DType::UChar8,
        Value::I8(_) => DType::I8,
        Value::U8(_) => DType::U8,
        Value::I16(_) => DType::I16,
        Value::U16(_) => DType::U16,
        Value::I32(_) => DType::I32,
        Value::U32(_) => DType::U32,
        Value::F32(_) => DType::F32,
        Value::F64(_) => DType::F64,
    }
}

fn fill_array(dataset: &Dataset, output: &mut [u8]) -> Result<()> {
    let fill = dataset
        .attributes
        .iter()
        .find(|attr| attr.name == "_FillValue")
        .map(|attr| first_bytes(&attr.value, dataset.order))
        .transpose()?;
    let bytes = match fill {
        Some(bytes) => bytes,
        None => default_fill(dataset.dtype, dataset.order),
    };
    if bytes.len() != dataset.dtype.size() {
        return Err(malformed("empty or invalid fill value"));
    }
    chunk::fill(output, &bytes);
    Ok(())
}

fn first_bytes(value: &Value, order: ByteOrder) -> Result<Vec<u8>> {
    macro_rules! first {
        ($values:expr) => {{
            let value = $values
                .first()
                .ok_or_else(|| malformed("empty fill value"))?;
            if order == ByteOrder::Big {
                value.to_be_bytes().to_vec()
            } else {
                value.to_le_bytes().to_vec()
            }
        }};
    }
    Ok(match value {
        Value::Char8(values) | Value::UChar8(values) | Value::U8(values) => vec![
            *values
                .first()
                .ok_or_else(|| malformed("empty fill value"))?,
        ],
        Value::I8(values) => first!(values),
        Value::I16(values) => first!(values),
        Value::U16(values) => first!(values),
        Value::I32(values) => first!(values),
        Value::U32(values) => first!(values),
        Value::F32(values) => first!(values),
        Value::F64(values) => first!(values),
    })
}

fn default_fill(dtype: DType, order: ByteOrder) -> Vec<u8> {
    macro_rules! bytes {
        ($value:expr) => {
            if order == ByteOrder::Big {
                $value.to_be_bytes().to_vec()
            } else {
                $value.to_le_bytes().to_vec()
            }
        };
    }
    // mfhdf.h:38-44 and array.c:121-165 define the unscaled type defaults.
    match dtype {
        DType::Char8 | DType::UChar8 => vec![0],
        DType::I8 => bytes!(-127_i8),
        DType::U8 => vec![129],
        DType::I16 => bytes!(-32767_i16),
        DType::U16 => bytes!(32769_u16),
        DType::I32 => bytes!(-2147483647_i32),
        DType::U32 => bytes!(2147483649_u32),
        DType::F32 => bytes!(9.969_21e36_f32),
        DType::F64 => bytes!(9.969_209_968_386_869e36_f64),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn sq7_chunk_shape_bytes_are_charged_before_collection() -> Result<()> {
        let source = Memory::new(include_bytes!("../../../fixtures/F07.hdf").as_slice());
        let index = dd::parse(&source, &Limits::default())?;
        let reader = Reader {
            source: &source,
            index: &index,
        };
        let before = index.metadata().used();
        let (storage, _, _) = reader.layout(Some((702, 3)))?;
        let Storage::Chunked { chunk_shape } = storage else {
            return Err(malformed("expected chunked storage"));
        };
        let charged = index.metadata().used().checked_sub(before).expect("charge");
        assert_eq!(
            charged,
            u64::try_from(chunk_shape.len().checked_mul(8).expect("shape bytes"))
                .expect("charge size")
        );
        Ok(())
    }

    #[test]
    fn sq7_allocation_length_rejects_more_than_isize_max() {
        let length = (isize::MAX as u64).checked_add(1).expect("boundary");
        let limits = Limits {
            max_array_bytes: u64::MAX,
            ..Limits::default()
        };
        assert!(
            matches!(allocation(length, &limits), Err(Error::LimitExceeded { limit: "allocation_bytes", value, max }) if value == length && max == isize::MAX as u64)
        );
        let limits = Limits {
            max_array_bytes: 8,
            ..limits
        };
        assert!(matches!(
            allocation(length, &limits),
            Err(Error::LimitExceeded {
                limit: "array_bytes",
                max: 8,
                ..
            })
        ));
    }

    #[test]
    fn prepare_rejects_inconsistent_storage_before_array_allocation() -> Result<()> {
        for bytes in [
            include_bytes!("../../../fixtures/F06.hdf").as_slice(),
            include_bytes!("../../../fixtures/F07.hdf").as_slice(),
        ] {
            let mut file = File::from_bytes(bytes)?;
            let dataset = file.metadata.datasets.first_mut().expect("dataset");
            dataset.shape = vec![1000, 1000];
            let reader = Reader {
                source: file.source.as_ref(),
                index: &file.index,
            };
            assert!(matches!(
                reader.prepare(dataset),
                Err(Error::Malformed { .. })
            ));
        }
        Ok(())
    }
}
