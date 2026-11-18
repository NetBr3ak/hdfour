//! Bounded entry points shared by fuzz targets and deterministic contract tests.
#![forbid(unsafe_code)]

use hdfour::{Codec, Error, File, Limits, Memory, Result, codec, eos, odl};

const ARRAY_BYTES: u64 = 1_048_576;

fn open(bytes: &[u8]) -> Result<File> {
    let limits = Limits {
        max_descriptors: 4096,
        max_dd_blocks: 128,
        max_group_depth: 16,
        max_rank: 8,
        max_array_bytes: ARRAY_BYTES,
        max_attribute_bytes: 262_144,
        max_metadata_bytes: 1_048_576,
        max_chunks: 1024,
        max_chunk_header_bytes: 256,
    };
    let owned = bytes.to_vec();
    File::from_source(Memory::new(owned), limits)
}

/// Attempt every read; mutations may legitimately make individual reads fail.
pub fn open_read_all(bytes: &[u8]) -> Result<usize> {
    let file = open(bytes)?;
    for dataset in file.datasets() {
        let _ = std::hint::black_box(read(&file, dataset));
        // A bounded interior tile also exercises positional reads and partial edge chunks.
        let start: Vec<_> = dataset.shape.iter().map(|size| size / 2).collect();
        let count: Vec<_> = dataset
            .shape
            .iter()
            .zip(&start)
            .map(|(size, begin)| (size - begin).min(3))
            .collect();
        let _ = std::hint::black_box(file.read_window(dataset, &start, &count));
    }
    Ok(file.datasets().len())
}

/// Check a known fixture, propagating every read error to the contract test.
pub fn read_fixture(bytes: &[u8]) -> Result<usize> {
    let file = open(bytes)?;
    for dataset in file.datasets() {
        std::hint::black_box(read(&file, dataset)?);
    }
    Ok(file.datasets().len())
}

fn read(file: &File, dataset: &hdfour::Dataset) -> Result<hdfour::Array> {
    let length = dataset.shape.iter().try_fold(
        u64::try_from(dataset.dtype.size()).map_err(|_| malformed())?,
        |length, size| length.checked_mul(*size).ok_or_else(malformed),
    )?;
    if length > ARRAY_BYTES {
        return Err(Error::LimitExceeded {
            limit: "fuzz array bytes",
            value: length,
            max: ARRAY_BYTES,
        });
    }
    file.read(dataset)
}

/// The first two bytes declare the output length in little-endian order.
pub fn decode(codec: Codec, bytes: &[u8]) -> Result<Vec<u8>> {
    let Some((prefix, input)) = bytes.split_at_checked(2) else {
        return Err(malformed());
    };
    let prefix: [u8; 2] = prefix.try_into().map_err(|_| malformed())?;
    let mut output = vec![0; usize::from(u16::from_le_bytes(prefix))];
    codec::decode(&codec, input, &mut output)?;
    Ok(output)
}

/// Exercise both structural syntax and the EOS mapper without lossy UTF-8 conversion.
pub fn parse_odl(bytes: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(bytes).map_err(|_| malformed())?;
    odl::parse(text, &Limits::default())?;
    eos::from_struct_metadata(text, &Limits::default())?;
    Ok(())
}

fn malformed() -> Error {
    Error::Malformed {
        offset: 0,
        what: "invalid fuzz input framing",
    }
}
