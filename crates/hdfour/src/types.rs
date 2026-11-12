//! Dataset metadata and raw, unscaled values.

/// A scientific dataset and its storage metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct Dataset {
    pub name: String,
    pub dtype: DType,
    pub shape: Vec<u64>,
    pub dims: Vec<Dimension>,
    pub attributes: Vec<Attribute>,
    pub is_coordinate: bool,
    pub storage: Storage,
    pub codec: Codec,
    pub(crate) data: Option<(u16, u16)>,
    pub(crate) ndg_ref: u16,
    pub(crate) order: ByteOrder,
}

/// A named dataset axis, with its raw attributes.
#[derive(Debug, Clone, PartialEq)]
pub struct Dimension {
    pub name: String,
    pub size: u64,
    pub unlimited: bool,
    pub attributes: Vec<Attribute>,
}

/// A named raw attribute.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub name: String,
    pub value: Value,
}

/// An object omitted because its feature is outside the reader's scope.
#[derive(Debug, Clone, PartialEq)]
pub struct Skipped {
    pub object: String,
    pub feature: &'static str,
}

/// The dataset's on-disk storage layout.
#[derive(Debug, Clone, PartialEq)]
pub enum Storage {
    Empty,
    Contiguous,
    Linked,
    Chunked { chunk_shape: Vec<u64> },
    External,
}

/// The stored compression code, including codes not decoded by this reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    None,
    Rle,
    Deflate,
    Other(u16),
}

/// The byte order of stored values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    Big,
    Little,
}

/// A scoped HDF4 number type, keeping character types distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DType {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    F32,
    F64,
    Char8,
    UChar8,
}

impl DType {
    /// The number of bytes occupied by one value.
    pub fn size(&self) -> usize {
        match self {
            Self::I8 | Self::U8 | Self::Char8 | Self::UChar8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::F64 => 8,
        }
    }
}

/// Owned raw values in native byte order; character values retain their bytes.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Char8(Vec<u8>),
    UChar8(Vec<u8>),
    I8(Vec<i8>),
    U8(Vec<u8>),
    I16(Vec<i16>),
    U16(Vec<u16>),
    I32(Vec<i32>),
    U32(Vec<u32>),
    F32(Vec<f32>),
    F64(Vec<f64>),
}

/// A C-order array of raw values.
#[derive(Debug, Clone, PartialEq)]
pub struct Array {
    pub shape: Vec<u64>,
    pub data: Value,
}

/// HDF-EOS2 grid and swath metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct Eos {
    pub grids: Vec<EosObject>,
    pub swaths: Vec<EosObject>,
}

/// A grid or swath; projection parameters remain raw text.
#[derive(Debug, Clone, PartialEq)]
pub struct EosObject {
    pub name: String,
    pub dims: Vec<(String, u64)>,
    pub fields: Vec<EosField>,
    pub dimension_maps: Vec<DimensionMap>,
    pub params: Vec<(String, String)>,
}

/// An EOS field, optionally linked to an index in the file's dataset list.
#[derive(Debug, Clone, PartialEq)]
pub struct EosField {
    pub name: String,
    pub dims: Vec<String>,
    pub geolocation: bool,
    pub dataset: Option<usize>,
}

/// A linear mapping from a geolocation dimension to a data dimension.
#[derive(Debug, Clone, PartialEq)]
pub struct DimensionMap {
    pub geo_dim: String,
    pub data_dim: String,
    pub offset: i64,
    pub increment: i64,
}
