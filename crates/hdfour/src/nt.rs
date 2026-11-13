//! Number-type records and native-order value decoding.
#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::{ByteOrder, DType, Error, Result, Value};

/// Parse exactly one four-byte number-type record.
pub fn parse(bytes: &[u8]) -> Result<(DType, ByteOrder)> {
    // cdf.c:855-864 stores width in bits and the actual file class.
    let &[version, code, bits, class] = bytes else {
        return Err(malformed("number-type record must contain four bytes"));
    };
    if version != 1 {
        return Err(unsupported());
    }
    let dtype = dtype_from_code(u16::from(code))?;
    let order = match class {
        0 | 1 => ByteOrder::Big,
        4 => ByteOrder::Little,
        _ => return Err(unsupported()),
    };
    if usize::from(bits)
        != dtype
            .size()
            .checked_mul(8)
            .ok_or_else(|| malformed("number-type width overflow"))?
    {
        return Err(malformed("number-type width does not match its type"));
    }
    Ok((dtype, order))
}

/// Parse a modern Vdata field type; old Vdata codes must first be converted by vset.
pub fn from_vdata_type(code: u16) -> Result<(DType, ByteOrder)> {
    // hntdefs.h:21-26; specification T5 excludes native/custom and unknown flags.
    if code & 0xb000 != 0 {
        return Err(unsupported());
    }
    let dtype = dtype_from_code(code & 0x0fff)?;
    let order = if code & 0x4000 == 0 {
        ByteOrder::Big
    } else {
        ByteOrder::Little
    };
    Ok((dtype, order))
}

/// Decode whole values, preserving floating-point bit patterns and character bytes.
pub fn decode(dtype: DType, order: ByteOrder, bytes: &[u8]) -> Result<Value> {
    if !bytes.chunks_exact(dtype.size()).remainder().is_empty() {
        return Err(malformed("partial number-type value"));
    }
    Ok(match dtype {
        DType::Char8 => Value::Char8(copy_bytes(bytes)?),
        DType::UChar8 => Value::UChar8(copy_bytes(bytes)?),
        DType::I8 => Value::I8(decode_values(&order, bytes)?),
        DType::U8 => Value::U8(copy_bytes(bytes)?),
        DType::I16 => Value::I16(decode_values(&order, bytes)?),
        DType::U16 => Value::U16(decode_values(&order, bytes)?),
        DType::I32 => Value::I32(decode_values(&order, bytes)?),
        DType::U32 => Value::U32(decode_values(&order, bytes)?),
        DType::F32 => Value::F32(decode_values(&order, bytes)?),
        DType::F64 => Value::F64(decode_values(&order, bytes)?),
    })
}

fn copy_bytes(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut output = crate::read::buffer(bytes.len())?;
    output.copy_from_slice(bytes);
    Ok(output)
}

fn decode_values<T: bytemuck::Pod + Default>(order: &ByteOrder, bytes: &[u8]) -> Result<Vec<T>> {
    let width = std::mem::size_of::<T>();
    let mut values = crate::read::buffer(bytes.chunks_exact(width).len())?;
    let output = bytemuck::must_cast_slice_mut::<T, u8>(&mut values);
    output.copy_from_slice(bytes);
    let native_order = match order {
        ByteOrder::Big => cfg!(target_endian = "big"),
        ByteOrder::Little => cfg!(target_endian = "little"),
    };
    if !native_order {
        swap_in_place(output, width);
    }
    Ok(values)
}

/// Reverse each complete value; zero width is a no-op and trailing bytes are untouched.
/// Callers decoding a typed array must reject a partial final value first.
pub fn swap_in_place(bytes: &mut [u8], width: usize) {
    // A width known at compile time lets the compiler swap whole values; with a run-time width it moves one
    // byte at a time (26 percent of a 184 MB contiguous read on x86-64).
    match width {
        0 | 1 => {}
        2 => swap_fixed::<2>(bytes),
        4 => swap_fixed::<4>(bytes),
        8 => swap_fixed::<8>(bytes),
        _ => {
            for value in bytes.chunks_exact_mut(width) {
                value.reverse();
            }
        }
    }
}

fn swap_fixed<const WIDTH: usize>(bytes: &mut [u8]) {
    for value in bytes.chunks_exact_mut(WIDTH) {
        value.reverse();
    }
}

fn dtype_from_code(code: u16) -> Result<DType> {
    // hntdefs.h:34-55 defines the scoped number-type codes.
    match code {
        3 => Ok(DType::UChar8),
        4 => Ok(DType::Char8),
        5 => Ok(DType::F32),
        6 => Ok(DType::F64),
        20 => Ok(DType::I8),
        21 => Ok(DType::U8),
        22 => Ok(DType::I16),
        23 => Ok(DType::U16),
        24 => Ok(DType::I32),
        25 => Ok(DType::U32),
        _ => Err(unsupported()),
    }
}

fn unsupported() -> Error {
    Error::Unsupported {
        feature: "number-type",
        object: "number-type record".into(),
    }
}

fn malformed(what: &'static str) -> Error {
    Error::Malformed { offset: 0, what }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const TYPES: [(u8, u8, DType); 10] = [
        (3, 8, DType::UChar8),
        (4, 8, DType::Char8),
        (5, 32, DType::F32),
        (6, 64, DType::F64),
        (20, 8, DType::I8),
        (21, 8, DType::U8),
        (22, 16, DType::I16),
        (23, 16, DType::U16),
        (24, 32, DType::I32),
        (25, 32, DType::U32),
    ];

    fn is_number_type_error<T>(result: Result<T>) -> bool {
        matches!(
            result,
            Err(Error::Unsupported {
                feature: "number-type",
                ..
            })
        )
    }

    #[test]
    fn every_nt_type_byte() -> Result<()> {
        for code in 0..=u8::MAX {
            let expected = TYPES.iter().find(|entry| entry.0 == code);
            if let Some((_, bits, dtype)) = expected {
                assert_eq!(parse(&[1, code, *bits, 1])?, (*dtype, ByteOrder::Big));
            } else {
                assert!(is_number_type_error(parse(&[1, code, 8, 1])));
            }
        }
        Ok(())
    }

    #[test]
    fn every_nt_version_width_and_class_byte() -> Result<()> {
        for (code, bits, dtype) in TYPES {
            for byte in 0..=u8::MAX {
                if byte != 1 {
                    assert!(is_number_type_error(parse(&[byte, code, bits, 1])));
                }
                if byte != bits {
                    assert!(matches!(
                        parse(&[1, code, byte, 1]),
                        Err(Error::Malformed { .. })
                    ));
                }
                match byte {
                    0 | 1 => assert_eq!(parse(&[1, code, bits, byte])?, (dtype, ByteOrder::Big)),
                    4 => assert_eq!(parse(&[1, code, bits, byte])?, (dtype, ByteOrder::Little)),
                    _ => assert!(is_number_type_error(parse(&[1, code, bits, byte]))),
                }
            }
        }
        Ok(())
    }

    #[test]
    fn nt_record_length() {
        for length in 0..4 {
            assert!(matches!(
                parse(&vec![0; length]),
                Err(Error::Malformed { .. })
            ));
        }
        assert!(matches!(
            parse(&[1, 22, 16, 1, 0]),
            Err(Error::Malformed { .. })
        ));
    }

    #[test]
    fn every_vdata_type_word() -> Result<()> {
        for word in 0..=u16::MAX {
            let expected = TYPES
                .iter()
                .find(|entry| u16::from(entry.0) == (word & 0x0fff));
            if word & 0xb000 != 0 || expected.is_none() {
                assert!(is_number_type_error(from_vdata_type(word)));
                continue;
            }
            let (_, _, dtype) = expected.expect("checked above");
            let order = if word & 0x4000 == 0 {
                ByteOrder::Big
            } else {
                ByteOrder::Little
            };
            assert_eq!(from_vdata_type(word)?, (*dtype, order));
        }
        Ok(())
    }

    #[test]
    fn decode_all_types_in_both_orders() -> Result<()> {
        for order in [ByteOrder::Big, ByteOrder::Little] {
            let bytes = if order == ByteOrder::Big {
                [0x80, 0x01]
            } else {
                [0x01, 0x80]
            };
            assert_eq!(decode(DType::I16, order, &bytes)?, Value::I16(vec![-32767]));
            assert_eq!(decode(DType::U16, order, &bytes)?, Value::U16(vec![32769]));
            let bytes = if order == ByteOrder::Big {
                [0x80, 0, 0, 1]
            } else {
                [1, 0, 0, 0x80]
            };
            assert_eq!(
                decode(DType::I32, order, &bytes)?,
                Value::I32(vec![-2147483647])
            );
            assert_eq!(
                decode(DType::U32, order, &bytes)?,
                Value::U32(vec![2147483649])
            );
            let float = if order == ByteOrder::Big {
                (-1.5_f32).to_be_bytes()
            } else {
                (-1.5_f32).to_le_bytes()
            };
            assert_eq!(decode(DType::F32, order, &float)?, Value::F32(vec![-1.5]));
            let double = if order == ByteOrder::Big {
                (-1.5_f64).to_be_bytes()
            } else {
                (-1.5_f64).to_le_bytes()
            };
            assert_eq!(decode(DType::F64, order, &double)?, Value::F64(vec![-1.5]));
            assert_eq!(
                decode(DType::I8, order, &[0x80, 0x7f])?,
                Value::I8(vec![-128, 127])
            );
            assert_eq!(
                decode(DType::U8, order, &[0x80, 0x7f])?,
                Value::U8(vec![128, 127])
            );
            assert_eq!(
                decode(DType::Char8, order, &[0, 255])?,
                Value::Char8(vec![0, 255])
            );
            assert_eq!(
                decode(DType::UChar8, order, &[0, 255])?,
                Value::UChar8(vec![0, 255])
            );
        }
        Ok(())
    }

    #[test]
    fn decode_preserves_float_bits() -> Result<()> {
        for bits in [0_u32, 0x80000000, 0x7f800000, 0xff800000, 0x7fc12345] {
            let Value::F32(values) = decode(DType::F32, ByteOrder::Big, &bits.to_be_bytes())?
            else {
                panic!("wrong variant")
            };
            assert_eq!(values.first().expect("one value").to_bits(), bits);
        }
        for bits in [
            0_u64,
            0x8000000000000000,
            0x7ff0000000000000,
            0xfff0000000000000,
            0x7ff8123456789abc,
        ] {
            let Value::F64(values) = decode(DType::F64, ByteOrder::Little, &bits.to_le_bytes())?
            else {
                panic!("wrong variant")
            };
            assert_eq!(values.first().expect("one value").to_bits(), bits);
        }
        Ok(())
    }

    #[test]
    fn decode_rejects_partial_values() -> Result<()> {
        for (_, bits, dtype) in TYPES {
            assert!(decode(dtype, ByteOrder::Big, &[]).is_ok());
            if bits > 8 {
                assert!(matches!(
                    decode(dtype, ByteOrder::Little, &[1]),
                    Err(Error::Malformed { .. })
                ));
            }
        }
        Ok(())
    }

    #[test]
    fn swap_reverses_each_value() {
        let mut bytes = [1, 2, 3, 4, 5, 6, 7, 8];
        swap_in_place(&mut bytes, 2);
        assert_eq!(bytes, [2, 1, 4, 3, 6, 5, 8, 7]);
        swap_in_place(&mut bytes, 4);
        assert_eq!(bytes, [3, 4, 1, 2, 7, 8, 5, 6]);
    }

    #[test]
    fn swap_handles_empty_zero_width_and_partial_values() {
        swap_in_place(&mut [], 8);
        let mut bytes = [1, 2, 3];
        swap_in_place(&mut bytes, 0);
        assert_eq!(bytes, [1, 2, 3]);
        swap_in_place(&mut bytes, 2);
        assert_eq!(bytes, [2, 1, 3]);
    }

    proptest! {
        #[test]
        fn swap_matches_reversing_each_value(bytes in prop::collection::vec(any::<u8>(), 0..2048), width in 0_usize..17) {
            let mut expected = bytes.clone();
            if width != 0 {
                for value in expected.chunks_exact_mut(width) {
                    value.reverse();
                }
            }
            let mut swapped = bytes;
            swap_in_place(&mut swapped, width);
            prop_assert_eq!(swapped, expected);
        }

        #[test]
        fn swap_round_trip(mut bytes in prop::collection::vec(any::<u8>(), 0..2048), width in 0_usize..17) {
            let original = bytes.clone();
            swap_in_place(&mut bytes, width);
            swap_in_place(&mut bytes, width);
            prop_assert_eq!(bytes, original);
        }

        #[test]
        fn arbitrary_nt_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..16)) {
            let _ = parse(&bytes);
        }
    }
}
