//! Read-only HDF4 and HDF-EOS2 reader foundations.
//!
//! Every module that parses file bytes starts with the same `#![deny(...)]` list (see `nt.rs`):
//! a malformed file must give an error, never a panic.

pub mod chunk;
pub mod codec;
pub mod dd;
pub mod element;
pub mod eos;
pub mod error;
pub mod limits;
pub mod nt;
pub mod odl;
pub mod read;
pub mod sd;
pub mod source;
pub mod special;
pub mod types;
pub mod vset;

pub use error::{Error, Result};
pub use limits::Limits;
#[cfg(any(unix, windows))]
pub use source::FileSource;
pub use source::{Memory, Source};
pub use types::{
    Array, Attribute, ByteOrder, Codec, DType, Dataset, Dimension, DimensionMap, Eos, EosField,
    EosObject, Skipped, Storage, Value,
};

pub use read::File;

/// Open a file and read its metadata without reading array payloads.
#[cfg(any(unix, windows))]
pub fn open(path: impl AsRef<std::path::Path>) -> Result<File> {
    File::from_source(FileSource::open(path)?, Limits::default())
}
