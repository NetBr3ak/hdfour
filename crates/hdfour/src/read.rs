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
        if !self.datasets().iter().any(|own| std::ptr::eq(own, dataset)) {
            return Err(Error::NotFound(dataset.name.clone()));
        }
        let reader = Reader {
            source: self.source.as_ref(),
            index: &self.index,
        };
        reader
            .prepare(dataset)
            .map_err(|error| named(error, dataset))?;
        let count = array_bytes(&dataset.shape, dataset.dtype.size(), reader.limits())?;
        let data = match dataset.dtype {
            DType::Char8 => Value::Char8(reader.values::<u8>(dataset, count)?),
            DType::UChar8 => Value::UChar8(reader.values::<u8>(dataset, count)?),
            DType::I8 => Value::I8(reader.values::<i8>(dataset, count)?),
            DType::U8 => Value::U8(reader.values::<u8>(dataset, count)?),
            DType::I16 => Value::I16(reader.values::<i16>(dataset, count)?),
            DType::U16 => Value::U16(reader.values::<u16>(dataset, count)?),
            DType::I32 => Value::I32(reader.values::<i32>(dataset, count)?),
            DType::U32 => Value::U32(reader.values::<u32>(dataset, count)?),
            DType::F32 => Value::F32(reader.values::<f32>(dataset, count)?),
            DType::F64 => Value::F64(reader.values::<f64>(dataset, count)?),
        };
        Ok(Array {
            shape: dataset.shape.clone(),
            data,
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
        let mut bytes = vec![0; allocation(length, self.limits())?];
        element::read(self.source, self.index, descriptor, &mut bytes)?;
        Ok(bytes)
    }

    pub fn records(&self, header: &crate::vset::VdataHeader, reference: u16) -> Result<Vec<u8>> {
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
        let mut bytes = vec![0; allocation(length, self.limits())?];
        dd::read_at(self.source, offset, &mut bytes)?;
        special::parse(&bytes, self.limits())
    }

    pub fn layout(&self, key: Option<(u16, u16)>) -> Result<(Storage, Codec, u64)> {
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

    fn prepare(&self, dataset: &Dataset) -> Result<()> {
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
        let Some(key) = dataset.data else {
            return Ok(());
        };
        let descriptor = self.descriptor(key)?;
        if descriptor.offset.is_none() {
            return Ok(());
        }
        if !descriptor.is_special() {
            if descriptor.length < bytes as u64 {
                return Err(malformed("element shorter than array"));
            }
            return Ok(());
        }
        match self.special(descriptor)? {
            Special::Compressed(header) => {
                if header.length == 0 {
                    return Ok(());
                }
                if header.length != bytes as u64 {
                    return Err(malformed("compressed length differs from array"));
                }
                self.check_compressed(&header)?;
            }
            Special::Chunked(header) => {
                if header
                    .dimensions
                    .iter()
                    .map(|dimension| dimension.length)
                    .ne(dataset.shape.iter().copied())
                    || u64::from(header.number_type_size) != dataset.dtype.size() as u64
                {
                    return Err(malformed("chunk geometry differs from dataset"));
                }
                chunk::prepare(self, &header)?;
            }
            Special::Linked(header) => {
                if header.length < bytes as u64 {
                    return Err(malformed("linked element shorter than array"));
                }
            }
            Special::External => {}
        }
        Ok(())
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

    fn compressed_input(&self, header: &special::Compressed) -> Result<Vec<u8>> {
        self.bytes(self.check_compressed(header)?)
    }

    fn values<T: bytemuck::Pod + Default>(
        &self,
        dataset: &Dataset,
        bytes: usize,
    ) -> Result<Vec<T>> {
        let count = bytes
            .checked_div(std::mem::size_of::<T>())
            .ok_or_else(|| malformed("zero value width"))?;
        let mut values = vec![T::default(); count];
        let output = bytemuck::must_cast_slice_mut::<T, u8>(&mut values);
        self.read_array(dataset, output)
            .map_err(|error| named(error, dataset))?;
        let native = if cfg!(target_endian = "little") {
            ByteOrder::Little
        } else {
            ByteOrder::Big
        };
        if dataset.order != native {
            nt::swap_in_place(output, dataset.dtype.size());
        }
        Ok(values)
    }

    fn read_array(&self, dataset: &Dataset, output: &mut [u8]) -> Result<()> {
        let Some(key) = dataset.data else {
            return fill_array(dataset, output);
        };
        let descriptor = self.descriptor(key)?;
        if descriptor.offset.is_none() {
            return fill_array(dataset, output);
        }
        if !descriptor.is_special() {
            return element::read(self.source, self.index, descriptor, output);
        }
        match self.special(descriptor)? {
            Special::Linked(_) => element::read(self.source, self.index, descriptor, output),
            Special::External => Err(unsupported("external-element", dataset)),
            Special::Compressed(header) => {
                if header.length == 0 {
                    return fill_array(dataset, output);
                }
                if header.length != output.len() as u64 {
                    return Err(malformed("compressed length differs from array"));
                }
                codec::decode(
                    &header.compression.codec,
                    &self.compressed_input(&header)?,
                    output,
                )
            }
            Special::Chunked(header) => chunk::read(self, &header, dataset, output),
        }
    }

    pub fn read_chunk(&self, key: (u16, u16), output: &mut [u8]) -> Result<()> {
        let descriptor = self.descriptor(key)?;
        if !descriptor.is_special() {
            return element::read(self.source, self.index, descriptor, output);
        }
        match self.special(descriptor)? {
            Special::Linked(_) => element::read(self.source, self.index, descriptor, output),
            Special::Compressed(header) => {
                if header.length != output.len() as u64 {
                    return Err(malformed("compressed chunk size mismatch"));
                }
                codec::decode(
                    &header.compression.codec,
                    &self.compressed_input(&header)?,
                    output,
                )
            }
            Special::External => Err(Error::Unsupported {
                feature: "external-element",
                object: "chunk".into(),
            }),
            Special::Chunked(_) => Err(malformed("nested chunk storage")),
        }
    }

    pub fn prepare_chunk(&self, key: (u16, u16), length: usize) -> Result<()> {
        let descriptor = self.descriptor(key)?;
        if descriptor.is_special() {
            if let Special::Compressed(header) = self.special(descriptor)? {
                if header.length != length as u64 {
                    return Err(malformed("compressed chunk size mismatch"));
                }
                self.check_compressed(&header)?;
            }
        }
        Ok(())
    }
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
    usize::try_from(length).map_err(|_| malformed("allocation size does not fit address space"))
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
    for value in output.chunks_exact_mut(bytes.len()) {
        value.copy_from_slice(&bytes);
    }
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
