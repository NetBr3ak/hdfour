//! Bounded decoding of stored compression streams.
#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::{Codec, Error, Result};
use miniz_oxide::inflate::{
    TINFLStatus,
    core::{DecompressorOxide, decompress, inflate_flags},
};

/// Decode one complete element into its declared output buffer without allocating storage.
///
/// The caller must check its file's array limit and compression ratio before allocating the buffer.
/// This function also checks the ratio before writing any bytes.
/// An error may leave partially decoded bytes in the buffer; callers must discard them.
pub fn decode(codec: &Codec, input: &[u8], output: &mut [u8]) -> Result<()> {
    check_ratio(codec, input.len(), output.len())?;
    match codec {
        Codec::None => {
            if input.len() != output.len() {
                return Err(malformed("uncompressed element size mismatch"));
            }
            output.copy_from_slice(input);
            Ok(())
        }
        Codec::Rle => decode_rle(input, output),
        Codec::Deflate => decode_deflate(input, output),
        Codec::Other(code) => Err(unsupported(*code)),
    }
}

/// Check the codec's maximum expansion ratio before allocating the output buffer.
///
/// The caller must separately check its file's array limit.
pub fn check_ratio(codec: &Codec, input_len: usize, output_len: usize) -> Result<()> {
    let ratio = match codec {
        Codec::None => 1,
        // Specification T4; crle_priv.h:29-41 permits 130 output bytes per two-byte run.
        Codec::Rle => 65,
        Codec::Deflate => 1032,
        Codec::Other(code) => return Err(unsupported(*code)),
    };
    // If multiplication overflows, every representable output length is below the ratio ceiling.
    if input_len
        .checked_mul(ratio)
        .is_some_and(|max_output| output_len > max_output)
    {
        return Err(malformed("decoded size exceeds compression ratio"));
    }
    Ok(())
}

fn decode_rle(mut input: &[u8], mut output: &mut [u8]) -> Result<()> {
    // crle.c:106,139-141 stops when the requested output is full, leaving any old tail unread.
    // SQ4 R1 additionally requires that the output ends at a record boundary.
    while !output.is_empty() {
        let (&control, rest) = input
            .split_first()
            .ok_or_else(|| malformed("RLE stream is shorter than declared output"))?;
        // crle.c:110-119; crle_priv.h:35-41 adds three to runs and one to literal mixes.
        let base = if control & 0x80 != 0 { 3 } else { 1 };
        let length = usize::from(control & 0x7f)
            .checked_add(base)
            .ok_or_else(|| malformed("RLE record size overflow"))?;
        let (record, remaining) = output
            .split_at_mut_checked(length)
            .ok_or_else(|| malformed("RLE record exceeds declared output"))?;
        input = if control & 0x80 != 0 {
            let (&byte, rest) = rest
                .split_first()
                .ok_or_else(|| malformed("truncated RLE run"))?;
            record.fill(byte);
            rest
        } else {
            let (bytes, rest) = rest
                .split_at_checked(length)
                .ok_or_else(|| malformed("truncated RLE mix"))?;
            record.copy_from_slice(bytes);
            rest
        };
        output = remaining;
    }
    Ok(())
}

fn decode_deflate(input: &[u8], output: &mut [u8]) -> Result<()> {
    // cdeflate.c:360 uses inflateInit, so the zlib wrapper and its checksum are required.
    let flags = inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER
        | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
    let (status, _, written) =
        decompress(&mut DecompressorOxide::default(), input, output, 0, flags);
    if status != TINFLStatus::Done || written != output.len() {
        return Err(malformed("invalid zlib stream or decoded size mismatch"));
    }
    Ok(())
}

fn unsupported(code: u16) -> Error {
    // hcomp.h:36-44 assigns these compression codes; specification section 1 names refusals.
    let feature = match code {
        2 => "nbit",
        3 => "skipping-huffman",
        5 => "szip",
        7 => "jpeg",
        12 => "imcomp",
        _ => "codec",
    };
    Error::Unsupported {
        feature,
        object: format!("compression code {code}"),
    }
}

fn malformed(what: &'static str) -> Error {
    Error::Malformed { offset: 0, what }
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
    use crate::{Codec, Error, Result};
    use proptest::prelude::*;

    const ZLIB_HELLO: &[u8] = &[
        120, 156, 243, 72, 205, 201, 201, 215, 81, 168, 202, 201, 76, 82, 4, 0, 27, 101, 4, 19,
    ];
    const ZLIB_EMPTY: &[u8] = &[120, 156, 3, 0, 0, 0, 0, 1];
    const ZLIB_STORED: &[u8] = &[
        0x78, 0x01, 0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c', 0x02, 0x4d, 0x01, 0x27,
    ];

    fn assert_malformed(result: Result<()>) {
        assert!(matches!(result, Err(Error::Malformed { .. })), "{result:?}");
    }

    #[test]
    fn none_copies_exactly() -> Result<()> {
        let mut output = [0; 4];
        decode(&Codec::None, &[0, 255, 128, 1], &mut output)?;
        assert_eq!(output, [0, 255, 128, 1]);
        decode(&Codec::None, &[], &mut [])?;
        assert_malformed(decode(&Codec::None, &[1], &mut []));
        assert_malformed(decode(&Codec::None, &[], &mut [0]));
        Ok(())
    }

    #[test]
    fn rle_decodes_runs_and_mixes() -> Result<()> {
        let mut output = [0; 12];
        decode(
            &Codec::Rle,
            &[0, 9, 0x80, 7, 2, 1, 2, 3, 0x82, 255],
            &mut output,
        )?;
        assert_eq!(output, [9, 7, 7, 7, 1, 2, 3, 255, 255, 255, 255, 255]);
        Ok(())
    }

    #[test]
    fn rle_decodes_every_control_byte() -> Result<()> {
        for control in 0..=u8::MAX {
            let count = usize::from(control & 0x7f);
            if control & 0x80 != 0 {
                let mut output = vec![0; count + 3];
                decode(&Codec::Rle, &[control, 0xa5], &mut output)?;
                assert_eq!(output, vec![0xa5; count + 3]);
            } else {
                let expected: Vec<u8> = (0..=control).collect();
                let mut encoded = vec![control];
                encoded.extend_from_slice(&expected);
                let mut output = vec![0; expected.len()];
                decode(&Codec::Rle, &encoded, &mut output)?;
                assert_eq!(output, expected);
            }
        }
        Ok(())
    }

    #[test]
    fn rle_rejects_truncated_records() {
        for encoded in [&[0x80, 7][..], &[2, 1, 2, 3][..]] {
            for length in 0..encoded.len() {
                assert_malformed(decode(&Codec::Rle, &encoded[..length], &mut [0; 3]));
            }
        }
    }

    #[test]
    fn rle_requires_exact_output_and_ignores_stale_tail() -> Result<()> {
        for encoded in [&[0x80, 7][..], &[2, 1, 2, 3][..]] {
            assert_malformed(decode(&Codec::Rle, encoded, &mut [0; 2]));
            assert_malformed(decode(&Codec::Rle, encoded, &mut [0; 4]));
            for suffix in [&[0, 8][..], &[2, 3], &[0x80]] {
                let mut trailing = encoded.to_vec();
                trailing.extend_from_slice(suffix);
                let mut output = [0; 3];
                decode(&Codec::Rle, &trailing, &mut output)?;
                let expected = if encoded[0] == 0x80 {
                    [7; 3]
                } else {
                    [1, 2, 3]
                };
                assert_eq!(output, expected);
            }
        }
        decode(&Codec::Rle, &[], &mut [])?;
        assert_malformed(decode(&Codec::Rle, &[], &mut [0]));
        decode(&Codec::Rle, &[0, 1], &mut [])?;
        Ok(())
    }

    #[test]
    fn rle_ratio_boundary() -> Result<()> {
        decode(&Codec::Rle, &[0xff, 42], &mut [0; 130])?;
        let mut output = [0xa5; 131];
        assert_malformed(decode(&Codec::Rle, &[0xff, 42], &mut output));
        assert_eq!(output, [0xa5; 131]);
        Ok(())
    }

    #[test]
    fn zlib_decodes_fixed_and_stored_vectors() -> Result<()> {
        let mut output = [0; 12];
        decode(&Codec::Deflate, ZLIB_HELLO, &mut output)?;
        assert_eq!(&output, b"Hello, zlib!");
        let mut output = [0; 3];
        decode(&Codec::Deflate, ZLIB_STORED, &mut output)?;
        assert_eq!(&output, b"abc");
        decode(&Codec::Deflate, ZLIB_EMPTY, &mut [])?;
        Ok(())
    }

    #[test]
    fn zlib_decodes_dynamic_huffman_and_long_matches() -> Result<()> {
        let encoded = [
            120, 156, 237, 196, 71, 13, 0, 0, 8, 4, 48, 173, 199, 240, 111, 1, 29, 36, 237, 163,
            169, 158, 141, 36, 73, 146, 36, 73, 146, 36, 73, 146, 36, 73, 146, 36, 73, 191, 59,
            179, 21, 28, 18,
        ];
        let mut output = vec![0; 10_000];
        decode(&Codec::Deflate, &encoded, &mut output)?;
        assert_eq!(output, b"abcde".repeat(2000));
        Ok(())
    }

    #[test]
    fn zlib_refuses_preset_dictionary() {
        let mut encoded = vec![0x78, 0xbb, 0, 0, 0, 1];
        encoded.extend_from_slice(&ZLIB_HELLO[2..]);
        assert_malformed(decode(&Codec::Deflate, &encoded, &mut [0; 12]));
    }

    #[test]
    fn zlib_rejects_truncation_and_corruption() -> Result<()> {
        for (encoded, expected) in [
            (ZLIB_HELLO, &b"Hello, zlib!"[..]),
            (ZLIB_EMPTY, &b""[..]),
            (ZLIB_STORED, &b"abc"[..]),
        ] {
            let mut output = vec![0; expected.len()];
            decode(&Codec::Deflate, encoded, &mut output)?;
            assert_eq!(output, expected);
            for length in 0..encoded.len() {
                assert_malformed(decode(
                    &Codec::Deflate,
                    &encoded[..length],
                    &mut vec![0; expected.len()],
                ));
            }
        }
        for index in [0, 1, 2, ZLIB_HELLO.len() - 1] {
            let mut corrupt = ZLIB_HELLO.to_vec();
            corrupt[index] ^= 0xff;
            assert_malformed(decode(&Codec::Deflate, &corrupt, &mut [0; 12]));
        }
        Ok(())
    }

    #[test]
    fn zlib_requires_exact_output_and_ignores_stale_tail() -> Result<()> {
        assert_malformed(decode(&Codec::Deflate, ZLIB_HELLO, &mut [0; 11]));
        assert_malformed(decode(&Codec::Deflate, ZLIB_HELLO, &mut [0; 13]));
        assert_malformed(decode(&Codec::Deflate, ZLIB_HELLO, &mut []));
        assert_malformed(decode(&Codec::Deflate, ZLIB_EMPTY, &mut [0]));
        for suffix in [&[0][..], ZLIB_EMPTY] {
            let mut trailing = ZLIB_HELLO.to_vec();
            trailing.extend_from_slice(suffix);
            let mut output = [0; 12];
            decode(&Codec::Deflate, &trailing, &mut output)?;
            assert_eq!(&output, b"Hello, zlib!");
        }
        Ok(())
    }

    #[test]
    fn deflate_refuses_raw_and_gzip_streams() {
        assert_malformed(decode(&Codec::Deflate, &ZLIB_HELLO[2..16], &mut [0; 12]));
        let mut gzip = vec![31, 139, 8, 0, 0, 0, 0, 0, 0, 3];
        gzip.extend_from_slice(&ZLIB_HELLO[2..16]);
        gzip.extend_from_slice(&[0x9d, 0x16, 0xf9, 0x85, 12, 0, 0, 0]);
        assert_malformed(decode(&Codec::Deflate, &gzip, &mut [0; 12]));
    }

    #[test]
    fn deflate_ratio_rejected_before_writing() {
        let mut output = vec![0xa5; ZLIB_HELLO.len() * 1032 + 1];
        assert_malformed(decode(&Codec::Deflate, ZLIB_HELLO, &mut output));
        assert!(output.iter().all(|&byte| byte == 0xa5));
    }

    #[test]
    fn ratio_checks_cover_boundaries_and_overflow() -> Result<()> {
        check_ratio(&Codec::Rle, 2, 130)?;
        assert_malformed(check_ratio(&Codec::Rle, 2, 131));
        check_ratio(&Codec::Deflate, 1, 1032)?;
        assert_malformed(check_ratio(&Codec::Deflate, 1, 1033));
        check_ratio(&Codec::Deflate, usize::MAX, 1)?;
        check_ratio(&Codec::Rle, 0, 0)?;
        assert_malformed(check_ratio(&Codec::Rle, 0, 1));
        Ok(())
    }

    #[test]
    fn other_codecs_name_the_unsupported_feature() {
        for code in 0..=u16::MAX {
            let expected = match code {
                2 => "nbit",
                3 => "skipping-huffman",
                5 => "szip",
                7 => "jpeg",
                12 => "imcomp",
                _ => "codec",
            };
            let mut output = [0xa5];
            let error =
                decode(&Codec::Other(code), &[], &mut output).expect_err("unsupported codec");
            assert!(matches!(error, Error::Unsupported { feature, ref object }
                if feature == expected && object.contains(&code.to_string())));
            assert_eq!(output, [0xa5]);
        }
    }

    proptest! {
        #[test]
        fn rle_round_trip(records in prop::collection::vec((any::<bool>(), any::<u8>(), 0_u8..128), 0..64)) {
            let mut encoded = Vec::new();
            let mut expected = Vec::new();
            for (run, byte, count) in records {
                if run {
                    encoded.extend_from_slice(&[0x80 | count, byte]);
                    expected.extend(std::iter::repeat_n(byte, usize::from(count) + 3));
                } else {
                    encoded.push(count);
                    for offset in 0..=count {
                        let value = byte.wrapping_add(offset);
                        encoded.push(value);
                        expected.push(value);
                    }
                }
            }
            let mut output = vec![0; expected.len()];
            decode(&Codec::Rle, &encoded, &mut output).expect("valid RLE stream");
            prop_assert_eq!(output, expected);
        }

        #[test]
        fn zlib_round_trip(bytes in prop::collection::vec(any::<u8>(), 0..4096)) {
            let encoded = miniz_oxide::deflate::compress_to_vec_zlib(&bytes, 6);
            let mut output = vec![0; bytes.len()];
            decode(&Codec::Deflate, &encoded, &mut output).expect("valid zlib stream");
            prop_assert_eq!(output, bytes);
        }

        #[test]
        fn arbitrary_streams_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..512), length in 0_usize..4096) {
            for codec in [Codec::None, Codec::Rle, Codec::Deflate] {
                let _ = decode(&codec, &bytes, &mut vec![0; length]);
            }
        }
    }
}
