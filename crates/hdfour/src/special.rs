//! Special-element storage headers.
#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::{Codec, Error, Limits, Result, dd};

const MAX_CHUNK_HEADER_BYTES: u64 = 256;

/// The storage kind encoded by a special element's u16 discriminator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Special {
    Linked(Linked),
    Compressed(Compressed),
    Chunked(Chunked),
    External,
}

/// Logical byte length and block-table geometry for linked storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Linked {
    pub length: u64,
    pub block_length: u64,
    pub number_blocks: u32,
    pub link_ref: u16,
}

/// A compressed element's decoded byte length and DFTAG_COMPRESSED (40) reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compressed {
    pub length: u64,
    pub reference: u16,
    pub compression: Compression,
}

/// Compression model, codec and uninterpreted codec-specific parameter bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compression {
    pub model: u16,
    pub codec: Codec,
    pub parameters: Vec<u8>,
}

/// Version-zero chunk geometry, table identity and raw file-order fill bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunked {
    pub flags: u32,
    /// Logical element count, not bytes; multiply by `number_type_size` with checks.
    pub length: u64,
    /// Number of values in one full chunk, not bytes.
    pub chunk_size: u64,
    pub number_type_size: u32,
    pub table_tag: u16,
    pub table_ref: u16,
    pub special_tag: u16,
    pub special_ref: u16,
    pub dimensions: Vec<ChunkDimension>,
    pub fill_value: Vec<u8>,
    pub compression: Option<Compression>,
}

/// A chunked dimension's raw flags, current size and full chunk extent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkDimension {
    pub flags: u32,
    pub length: u64,
    pub chunk_length: u64,
}

/// Parse a special header, keeping unsupported codec identities for read-time errors.
pub fn parse(bytes: &[u8], limits: &Limits) -> Result<Special> {
    let mut reader = Reader { bytes, offset: 0 };
    // hblocks.c:656-676 and hcomp.c:608-615 use a u16 special discriminator.
    match reader.u16()? {
        1 => parse_linked(&mut reader).map(Special::Linked),
        2 => Ok(Special::External),
        3 => parse_compressed(&mut reader).map(Special::Compressed),
        5 => parse_chunked(&mut reader, limits).map(Special::Chunked),
        _ => Err(reader.malformed("unknown special-element code")),
    }
}

fn parse_linked(reader: &mut Reader<'_>) -> Result<Linked> {
    // hblocks.c:673-676: length, block_length, number_blocks, then link_ref.
    let header = Linked {
        length: u64::from(reader.nonnegative()?),
        block_length: u64::from(reader.nonnegative()?),
        number_blocks: reader.nonnegative()?,
        link_ref: reader.u16()?,
    };
    if header.number_blocks == 0 {
        return Err(reader.malformed("zero linked-block geometry"));
    }
    if header.length != 0 && header.link_ref == 0 {
        return Err(reader.malformed("missing linked-block table reference"));
    }
    Ok(header)
}

fn parse_compressed(reader: &mut Reader<'_>) -> Result<Compressed> {
    // hcomp.c:608-615: version, decoded length, compressed ref, model and codec.
    if reader.u16()? != 0 {
        return Err(reader.malformed("unknown compressed-header version"));
    }
    let length = u64::from(reader.nonnegative()?);
    let reference = reader.u16()?;
    let compression = parse_compression(reader)?;
    Ok(Compressed {
        length,
        reference,
        compression,
    })
}

fn parse_compression(reader: &mut Reader<'_>) -> Result<Compression> {
    // hcomp.c:446-519: model/codec u16s precede codec-specific parameters.
    let model = reader.u16()?;
    let code = reader.u16()?;
    let parameter_length = match code {
        2 => 16,
        3 => 8,
        4 => 2,
        5 => 14,
        _ => 0,
    };
    if reader.bytes.len() < parameter_length {
        return Err(reader.malformed("truncated compression parameters"));
    }
    let codec = match code {
        0 => Codec::None,
        1 => Codec::Rle,
        4 => Codec::Deflate,
        other => Codec::Other(other),
    };
    Ok(Compression {
        model,
        codec,
        parameters: reader.take(parameter_length)?.to_vec(),
    })
}

fn parse_chunked(reader: &mut Reader<'_>, limits: &Limits) -> Result<Chunked> {
    let header_length = reader.nonnegative()?;
    // hchunks.c:955-961 caps the body, excluding the u16 code and i32 length.
    dd::check_limit(
        "chunk_header_bytes",
        u64::from(header_length),
        limits.max_chunk_header_bytes.min(MAX_CHUNK_HEADER_BYTES),
    )?;
    let body = reader.take(
        usize::try_from(header_length)
            .map_err(|_| reader.malformed("chunk header length is too large"))?,
    )?;
    let mut body = Reader {
        bytes: body,
        offset: 6,
    };
    if body.take(1)? != [0] {
        return Err(body.malformed("unknown chunk-header version"));
    }
    // hchunks.c:981-1038: fixed fields, 12 bytes per dimension, then fill length/data.
    let flags = body.u32()?;
    let length = u64::from(body.nonnegative()?);
    let chunk_size = u64::from(body.nonnegative()?);
    let number_type_size = body.nonnegative()?;
    let table_tag = body.u16()?;
    let table_ref = body.u16()?;
    let special_tag = body.u16()?;
    let special_ref = body.u16()?;
    let rank = body.nonnegative()?;
    dd::check_limit("rank", u64::from(rank), limits.max_rank)?;
    if rank == 0 || chunk_size == 0 || number_type_size == 0 {
        return Err(body.malformed("zero chunk geometry"));
    }
    let mut dimensions = Vec::new();
    for _ in 0..rank {
        let dimension = ChunkDimension {
            flags: body.u32()?,
            length: u64::from(body.nonnegative()?),
            chunk_length: u64::from(body.nonnegative()?),
        };
        if dimension.chunk_length == 0 {
            return Err(body.malformed("zero chunk dimension length"));
        }
        dimensions.push(dimension);
    }
    let fill_length = body.nonnegative()?;
    let fill_value = body
        .take(
            usize::try_from(fill_length)
                .map_err(|_| body.malformed("chunk fill length is too large"))?,
        )?
        .to_vec();
    let compression = match flags & 0xff {
        0 => None,
        3 => Some(parse_chunk_compression(reader)?),
        _ => return Err(body.malformed("unknown additional chunk specialness")),
    };
    Ok(Chunked {
        flags,
        length,
        chunk_size,
        number_type_size,
        table_tag,
        table_ref,
        special_tag,
        special_ref,
        dimensions,
        fill_value,
        compression,
    })
}

fn parse_chunk_compression(reader: &mut Reader<'_>) -> Result<Compression> {
    // hchunks.c:1055-1088 appends a special-code/length prefix and model/codec body.
    if reader.u16()? != 3 {
        return Err(reader.malformed("chunk compression header has the wrong special code"));
    }
    let length = reader.nonnegative()?;
    let offset = reader.offset;
    let bytes = reader.take(
        usize::try_from(length)
            .map_err(|_| reader.malformed("chunk compression header is too large"))?,
    )?;
    parse_compression(&mut Reader { bytes, offset })
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: u64,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let bytes = self
            .bytes
            .get(..count)
            .ok_or_else(|| self.malformed("truncated special-element header"))?;
        self.bytes = self
            .bytes
            .get(count..)
            .ok_or_else(|| self.malformed("truncated special-element header"))?;
        self.offset = self
            .offset
            .checked_add(count as u64)
            .ok_or_else(|| self.malformed("special-header offset overflow"))?;
        Ok(bytes)
    }

    fn u16(&mut self) -> Result<u16> {
        let bytes = self
            .take(2)?
            .try_into()
            .map_err(|_| self.malformed("truncated u16 in special header"))?;
        Ok(u16::from_be_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32> {
        let bytes = self
            .take(4)?
            .try_into()
            .map_err(|_| self.malformed("truncated u32 in special header"))?;
        Ok(u32::from_be_bytes(bytes))
    }

    fn nonnegative(&mut self) -> Result<u32> {
        let value = self.u32()?;
        if value > 2_147_483_647 {
            return Err(self.malformed("negative length or count in special header"));
        }
        Ok(value)
    }

    fn malformed(&self, what: &'static str) -> Error {
        Error::Malformed {
            offset: self.offset,
            what,
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn chunk_header(rank: u32, fill: &[u8]) -> Vec<u8> {
        let mut body = vec![0];
        for value in [0_u32, 24, 6, 2] {
            body.extend(value.to_be_bytes());
        }
        for value in [1962_u16, 77, 0, 0] {
            body.extend(value.to_be_bytes());
        }
        body.extend(rank.to_be_bytes());
        for _ in 0..rank {
            for value in [1_u32, 4, 2] {
                body.extend(value.to_be_bytes());
            }
        }
        body.extend(
            u32::try_from(fill.len())
                .expect("fill length")
                .to_be_bytes(),
        );
        body.extend(fill);
        let mut bytes = 5_u16.to_be_bytes().to_vec();
        bytes.extend(
            u32::try_from(body.len())
                .expect("header length")
                .to_be_bytes(),
        );
        bytes.extend(body);
        bytes
    }

    #[test]
    fn sq6_special_parser_obeys_custom_limits_and_format_ceiling() {
        let bytes = chunk_header(2, &[0, 0]);
        let limits = Limits {
            max_rank: 1,
            ..Limits::default()
        };
        assert!(matches!(
            super::parse(&bytes, &limits),
            Err(Error::LimitExceeded {
                limit: "rank",
                value: 2,
                max: 1
            })
        ));
        let body_length = u64::try_from(bytes.len() - 6).expect("body length");
        let limits = Limits {
            max_chunk_header_bytes: body_length - 1,
            ..Limits::default()
        };
        assert!(
            matches!(super::parse(&bytes, &limits), Err(Error::LimitExceeded { limit: "chunk_header_bytes", value, max }) if value == body_length && max == body_length - 1)
        );
        let limits = Limits {
            max_chunk_header_bytes: body_length,
            ..Limits::default()
        };
        assert!(super::parse(&bytes, &limits).is_ok());
        let limits = Limits {
            max_rank: 33,
            ..Limits::default()
        };
        assert!(matches!(
            super::parse(&chunk_header(33, &[]), &limits),
            Err(Error::LimitExceeded {
                limit: "chunk_header_bytes",
                max: 256,
                ..
            })
        ));
        let limits = Limits {
            max_chunk_header_bytes: 512,
            ..Limits::default()
        };
        assert!(matches!(
            super::parse(&chunk_header(1, &[0; 212]), &limits),
            Err(Error::LimitExceeded {
                limit: "chunk_header_bytes",
                value: 257,
                max: 256
            })
        ));
    }

    #[test]
    fn sq6_compression_parameters_exclude_ignored_tails() {
        for (code, width) in [
            (0_u16, 0),
            (1, 0),
            (2, 16),
            (3, 8),
            (4, 2),
            (5, 14),
            (7, 0),
            (12, 0),
            (99, 0),
        ] {
            let mut bytes = vec![0, 3, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0];
            bytes.extend(code.to_be_bytes());
            bytes.extend(vec![6; width]);
            bytes.extend(vec![0x55; 4096]);
            let Special::Compressed(header) =
                parse(&bytes, &Limits::default()).expect("compressed header")
            else {
                panic!("compressed variant");
            };
            assert_eq!(
                header.compression.parameters,
                vec![6; width],
                "codec {code}"
            );
        }
    }

    #[test]
    fn sq6_chunk_compression_parameters_exclude_ignored_tails() {
        let mut bytes = chunk_header(1, &[0, 0]);
        bytes[7..11].copy_from_slice(&3_u32.to_be_bytes());
        bytes.extend(3_u16.to_be_bytes());
        bytes.extend(4100_u32.to_be_bytes());
        bytes.extend([0, 0, 0, 1]);
        bytes.extend([0x55; 4096]);
        let Special::Chunked(header) = parse(&bytes, &Limits::default()).expect("chunk header")
        else {
            panic!("chunk variant");
        };
        assert!(
            header
                .compression
                .expect("compression")
                .parameters
                .is_empty()
        );
    }

    #[test]
    fn t1_linked_header_uses_u16_and_three_signed_i32_fields() {
        let bytes = [0, 1, 0, 0, 0, 9, 0, 0, 0, 4, 0, 0, 0, 2, 0, 7];
        let Special::Linked(header) = parse(&bytes, &Limits::default()).expect("linked header")
        else {
            panic!("linked variant");
        };
        assert_eq!(header.length, 9);
        assert_eq!(header.block_length, 4);
        assert_eq!(header.number_blocks, 2);
        assert_eq!(header.link_ref, 7);
        for end in 0..bytes.len() {
            assert!(parse(&bytes[..end], &Limits::default()).is_err());
        }
        for start in [2, 6, 10] {
            let mut bad = bytes;
            bad[start] = 0x80;
            assert!(matches!(
                parse(&bad, &Limits::default()),
                Err(Error::Malformed { .. })
            ));
        }
    }

    #[test]
    fn compressed_header_exposes_model_codec_and_parameters() {
        let bytes = [0, 3, 0, 0, 0, 0, 0, 12, 0, 5, 0, 0, 0, 4, 0, 6];
        let Special::Compressed(header) =
            parse(&bytes, &Limits::default()).expect("compressed header")
        else {
            panic!("compressed variant");
        };
        assert_eq!(header.length, 12);
        assert_eq!(header.reference, 5);
        assert_eq!(header.compression.model, 0);
        assert_eq!(header.compression.codec, Codec::Deflate);
        assert_eq!(header.compression.parameters, [0, 6]);
        for end in 0..bytes.len() {
            assert!(parse(&bytes[..end], &Limits::default()).is_err());
        }
        let mut bad = bytes;
        bad[3] = 1;
        assert!(parse(&bad, &Limits::default()).is_err());
    }

    #[test]
    fn unsupported_codecs_are_preserved_for_read_time_refusal() {
        for (code, parameters) in [
            (0_u16, 0),
            (1, 0),
            (2, 16),
            (3, 8),
            (5, 14),
            (7, 0),
            (12, 0),
            (99, 0),
        ] {
            let mut bytes = vec![0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            bytes.extend(code.to_be_bytes());
            bytes.resize(bytes.len() + parameters, 0);
            let Special::Compressed(header) =
                parse(&bytes, &Limits::default()).expect("codec header")
            else {
                panic!("compressed variant");
            };
            let expected = match code {
                0 => Codec::None,
                1 => Codec::Rle,
                _ => Codec::Other(code),
            };
            assert_eq!(header.compression.codec, expected);
        }
        assert!(matches!(
            parse(&[0, 2], &Limits::default()),
            Ok(Special::External)
        ));
        assert!(matches!(
            parse(&[0, 99], &Limits::default()),
            Err(Error::Malformed { .. })
        ));
    }

    #[test]
    fn t6_chunk_header_exposes_dimensions_table_and_file_order_fill() {
        let bytes = chunk_header(2, &[0xff, 0x81]);
        let Special::Chunked(header) = parse(&bytes, &Limits::default()).expect("chunk header")
        else {
            panic!("chunk variant");
        };
        assert_eq!(header.length, 24);
        assert_eq!(header.chunk_size, 6);
        assert_eq!(header.number_type_size, 2);
        assert_eq!((header.table_tag, header.table_ref), (1962, 77));
        assert_eq!((header.special_tag, header.special_ref), (0, 0));
        assert_eq!(header.dimensions.len(), 2);
        assert_eq!(header.dimensions[0].flags, 1);
        assert_eq!(header.dimensions[0].length, 4);
        assert_eq!(header.dimensions[0].chunk_length, 2);
        assert_eq!(header.fill_value, [0xff, 0x81]);
        assert!(header.compression.is_none());
        for end in 0..bytes.len() {
            assert!(parse(&bytes[..end], &Limits::default()).is_err());
        }
    }

    #[test]
    fn t6_chunk_header_checks_version_size_rank_and_signed_values() {
        let mut bytes = chunk_header(1, &[0, 0]);
        bytes[6] = 1;
        assert!(parse(&bytes, &Limits::default()).is_err());
        for start in [2, 11, 15, 19, 31, 39, 43, 47] {
            let mut bytes = chunk_header(1, &[0, 0]);
            bytes[start] = 0x80;
            assert!(parse(&bytes, &Limits::default()).is_err());
        }
        for (rank, fill_len) in [(0, 2), (33, 2), (1, 212)] {
            assert!(parse(&chunk_header(rank, &vec![0; fill_len]), &Limits::default()).is_err());
        }
        let bytes = chunk_header(1, &[0; 211]);
        assert_eq!(bytes.len(), 262);
        assert!(parse(&bytes, &Limits::default()).is_ok());
    }

    #[test]
    fn declared_chunk_body_cannot_borrow_bytes_from_the_next_header() {
        let mut bytes = chunk_header(1, &[0, 0]);
        bytes[2..6].copy_from_slice(&32_u32.to_be_bytes());
        assert!(matches!(
            parse(&bytes, &Limits::default()),
            Err(Error::Malformed { .. })
        ));
        let mut bytes = chunk_header(1, &[0, 0]);
        bytes[43..47].copy_from_slice(&0_u32.to_be_bytes());
        assert!(matches!(
            parse(&bytes, &Limits::default()),
            Err(Error::Malformed { .. })
        ));
    }

    #[test]
    fn compressed_chunks_parse_the_appended_compression_header() {
        let mut bytes = chunk_header(1, &[0, 0]);
        bytes[7..11].copy_from_slice(&3_u32.to_be_bytes());
        bytes.extend([0, 3, 0, 0, 0, 6, 0, 0, 0, 4, 0, 6]);
        let Special::Chunked(header) =
            parse(&bytes, &Limits::default()).expect("compressed chunks")
        else {
            panic!("chunk variant");
        };
        assert_eq!(
            header.compression.expect("compression").codec,
            Codec::Deflate
        );
        bytes.pop();
        assert!(parse(&bytes, &Limits::default()).is_err());
    }

    proptest! {
        #[test]
        fn random_special_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
            let _result = parse(&bytes, &Limits::default());
        }
    }
}
