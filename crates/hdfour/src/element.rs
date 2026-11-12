//! Reads of plain and linked elements.
#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::collections::BTreeSet;

use crate::dd::{self, Descriptor, Index};
use crate::special::{self, Linked, Special};
use crate::{Error, Result, Source};

/// Get a plain or linked element's logical byte length; unwritten elements are empty.
/// Other special kinds must be interpreted by their higher-level readers.
pub fn len(source: &dyn Source, index: &Index, descriptor: &Descriptor) -> Result<u64> {
    let Some(offset) = descriptor.offset else {
        return Ok(0);
    };
    dd::check_range(source, offset, descriptor.length)?;
    if !descriptor.is_special() {
        return Ok(descriptor.length);
    }
    Ok(linked_header(source, index, descriptor)?.length)
}

/// Fill a prefix of a plain or linked element, rejecting a request past its logical end.
/// This never decodes compressed or chunked elements or follows external storage.
pub fn read(
    source: &dyn Source,
    index: &Index,
    descriptor: &Descriptor,
    output: &mut [u8],
) -> Result<()> {
    dd::check_limit(
        "array_bytes",
        output.len() as u64,
        index.limits().max_array_bytes,
    )?;
    let length = len(source, index, descriptor)?;
    if output.len() as u64 > length {
        return Err(malformed(
            descriptor,
            "element shorter than the requested data",
        ));
    }
    if !descriptor.is_special() || descriptor.offset.is_none() {
        return read_plain(source, descriptor, output);
    }
    let header = linked_header(source, index, descriptor)?;
    read_linked(source, index, &header, output)
}

fn linked_header(source: &dyn Source, index: &Index, descriptor: &Descriptor) -> Result<Linked> {
    if descriptor.length < 16 {
        return Err(malformed(
            descriptor,
            "not a complete linked-element header",
        ));
    }
    let offset = descriptor
        .offset
        .ok_or_else(|| malformed(descriptor, "linked header was never written"))?;
    let mut bytes = [0; 16];
    dd::read_at(source, offset, &mut bytes)?;
    if !bytes.starts_with(&[0, 1]) {
        return Err(malformed(
            descriptor,
            "element reader requires plain or linked storage",
        ));
    }
    let parsed = special::parse(&bytes, index.limits()).map_err(|error| match error {
        Error::Malformed { what, .. } => malformed(descriptor, what),
        other => other,
    })?;
    let Special::Linked(header) = parsed else {
        return Err(malformed(
            descriptor,
            "element reader requires plain or linked storage",
        ));
    };
    dd::check_limit(
        "descriptors",
        u64::from(header.number_blocks),
        index.limits().max_descriptors,
    )?;
    Ok(header)
}

fn read_plain(source: &dyn Source, descriptor: &Descriptor, output: &mut [u8]) -> Result<()> {
    if descriptor.is_special() {
        return Err(malformed(
            descriptor,
            "linked table or block is itself special",
        ));
    }
    let Some(offset) = descriptor.offset else {
        if output.is_empty() {
            return Ok(());
        }
        return Err(malformed(descriptor, "element was never written"));
    };
    if output.len() as u64 > descriptor.length {
        return Err(malformed(
            descriptor,
            "element shorter than the requested data",
        ));
    }
    dd::check_range(source, offset, descriptor.length)?;
    dd::read_at(source, offset, output)
}

fn read_linked(
    source: &dyn Source,
    index: &Index,
    header: &Linked,
    mut output: &mut [u8],
) -> Result<()> {
    // hblocks.c:977-993: a table has next_ref followed by number_blocks u16 refs.
    let table_length = u64::from(header.number_blocks)
        .checked_mul(2)
        .and_then(|length| length.checked_add(2))
        .ok_or(Error::Malformed {
            offset: 0,
            what: "linked table size overflow",
        })?;
    dd::check_limit("array_bytes", table_length, index.limits().max_array_bytes)?;
    let mut table_bytes = crate::read::buffer(crate::read::allocation_size(table_length)?)?;
    let mut visited = BTreeSet::new();
    let mut reference = header.link_ref;
    let mut slots = 0_u64;
    let mut first = true;
    while reference != 0 {
        if !visited.insert(reference) {
            return Err(Error::Malformed {
                offset: 0,
                what: "cycle in linked-block tables",
            });
        }
        slots = slots
            .checked_add(u64::from(header.number_blocks))
            .ok_or(Error::Malformed {
                offset: 0,
                what: "linked block count overflow",
            })?;
        dd::check_limit("descriptors", slots, index.limits().max_descriptors)?;
        let table = index.get(20, reference).ok_or(Error::Malformed {
            offset: 0,
            what: "missing linked-block table",
        })?;
        read_plain(source, table, &mut table_bytes)?;
        let (next, references) = table_bytes
            .split_first_chunk::<2>()
            .ok_or_else(|| malformed(table, "truncated linked-block table"))?;
        reference = u16::from_be_bytes(*next);
        for bytes in references.chunks_exact(2) {
            if output.is_empty() {
                continue;
            }
            let &[r0, r1] = bytes else {
                return Err(malformed(table, "truncated linked-block reference"));
            };
            let block_ref = u16::from_be_bytes([r0, r1]);
            let block = match block_ref {
                0 => None,
                other => Some(
                    index
                        .get(20, other)
                        .ok_or_else(|| malformed(table, "missing linked data block"))?,
                ),
            };
            // hblocks.c:689-697: only the first block uses its DD's physical length.
            let block_length = match (first, block) {
                (true, Some(descriptor)) => descriptor.length,
                _ => header.block_length,
            };
            first = false;
            output = read_linked_block(source, block, block_length, output)?;
        }
    }
    if !output.is_empty() {
        return Err(Error::Malformed {
            offset: 0,
            what: "linked element ends before the requested data",
        });
    }
    Ok(())
}

fn read_linked_block<'a>(
    source: &dyn Source,
    descriptor: Option<&Descriptor>,
    length: u64,
    output: &'a mut [u8],
) -> Result<&'a mut [u8]> {
    let count = usize::try_from(length.min(output.len() as u64)).map_err(|_| Error::Malformed {
        offset: 0,
        what: "linked block is too large",
    })?;
    let (bytes, remaining) = output.split_at_mut_checked(count).ok_or(Error::Malformed {
        offset: 0,
        what: "linked block exceeds output buffer",
    })?;
    match descriptor {
        Some(block) => read_plain(source, block, bytes)?,
        // hblocks.c:1144-1147 defines an absent block reference as zero-filled storage.
        None => bytes.fill(0),
    }
    Ok(remaining)
}

fn malformed(descriptor: &Descriptor, what: &'static str) -> Error {
    Error::Malformed {
        offset: descriptor.offset.unwrap_or(0),
        what,
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use crate::{Limits, Memory};
    use proptest::prelude::*;

    fn image(records: &[(u16, u16, Vec<u8>)]) -> Memory<Vec<u8>> {
        let mut bytes = vec![0x0e, 3, 0x13, 1];
        bytes.extend(
            u16::try_from(records.len())
                .expect("test count")
                .to_be_bytes(),
        );
        bytes.extend(0_u32.to_be_bytes());
        let mut offset = 10 + 12 * records.len();
        for (tag, reference, payload) in records {
            bytes.extend(tag.to_be_bytes());
            bytes.extend(reference.to_be_bytes());
            bytes.extend(u32::try_from(offset).expect("test offset").to_be_bytes());
            bytes.extend(
                u32::try_from(payload.len())
                    .expect("test length")
                    .to_be_bytes(),
            );
            offset += payload.len();
        }
        for (_, _, payload) in records {
            bytes.extend(payload);
        }
        Memory::new(bytes)
    }

    fn linked(length: u32, block_length: u32, number_blocks: u32, link_ref: u16) -> Vec<u8> {
        let mut bytes = 1_u16.to_be_bytes().to_vec();
        for value in [length, block_length, number_blocks] {
            bytes.extend(value.to_be_bytes());
        }
        bytes.extend(link_ref.to_be_bytes());
        bytes
    }

    #[test]
    fn plain_reads_and_lengths_are_exact_and_bounded() {
        let source = image(&[(702, 1, b"abc".to_vec())]);
        let index = dd::parse(&source, &Limits::default()).expect("index");
        let descriptor = index.get(702, 1).expect("plain descriptor");
        assert_eq!(len(&source, &index, descriptor).expect("length"), 3);
        let mut bytes = [0; 3];
        read(&source, &index, descriptor, &mut bytes).expect("plain read");
        assert_eq!(&bytes, b"abc");
        assert!(read(&source, &index, descriptor, &mut [0; 4]).is_err());
        read(&source, &index, descriptor, &mut [0; 2]).expect("prefix read");
    }

    #[test]
    fn t1_first_block_uses_its_dd_length_and_tables_chain() {
        let source = image(&[
            (0x42be, 1, linked(9, 4, 2, 100)),
            (20, 100, vec![0, 101, 0, 1, 0, 2]),
            (20, 101, vec![0, 0, 0, 3, 0, 0]),
            (20, 1, b"abc".to_vec()),
            (20, 2, b"defg".to_vec()),
            (20, 3, b"hijk".to_vec()),
        ]);
        let index = dd::parse(&source, &Limits::default()).expect("index");
        let descriptor = index.get(702, 1).expect("linked descriptor");
        assert_eq!(len(&source, &index, descriptor).expect("logical length"), 9);
        let mut bytes = [0; 9];
        read(&source, &index, descriptor, &mut bytes).expect("linked read");
        assert_eq!(&bytes, b"abcdefghi");
    }

    #[test]
    fn missing_linked_blocks_are_zero_filled() {
        let source = image(&[
            (0x42be, 1, linked(6, 3, 2, 100)),
            (20, 100, vec![0, 0, 0, 0, 0, 1]),
            (20, 1, b"xyz".to_vec()),
        ]);
        let index = dd::parse(&source, &Limits::default()).expect("index");
        let mut bytes = [99; 6];
        read(
            &source,
            &index,
            index.get(702, 1).expect("linked DD"),
            &mut bytes,
        )
        .expect("sparse linked read");
        assert_eq!(&bytes, b"\0\0\0xyz");
    }

    #[test]
    fn linked_cycles_missing_refs_short_tables_and_short_blocks_are_malformed() {
        for records in [
            vec![
                (0x42be, 1, linked(3, 3, 1, 100)),
                (20, 100, vec![0, 100, 0, 1]),
                (20, 1, b"abc".to_vec()),
            ],
            vec![(0x42be, 1, linked(3, 3, 1, 100))],
            vec![
                (0x42be, 1, linked(3, 3, 2, 100)),
                (20, 100, vec![0, 0, 0, 1]),
            ],
            vec![
                (0x42be, 1, linked(7, 4, 2, 100)),
                (20, 100, vec![0, 0, 0, 1, 0, 2]),
                (20, 1, b"abc".to_vec()),
                (20, 2, b"def".to_vec()),
            ],
            vec![
                (0x42be, 1, linked(4, 3, 1, 100)),
                (20, 100, vec![0, 0, 0, 1]),
                (20, 1, b"abc".to_vec()),
            ],
        ] {
            let source = image(&records);
            let index = dd::parse(&source, &Limits::default()).expect("index");
            let descriptor = index.get(702, 1).expect("linked DD");
            let mut bytes = vec![
                0;
                usize::try_from(len(&source, &index, descriptor).expect("length"))
                    .expect("test size")
            ];
            assert!(matches!(
                read(&source, &index, descriptor, &mut bytes),
                Err(Error::Malformed { .. })
            ));
        }
    }

    #[test]
    fn sq6_zero_block_length_reads_first_block_and_rejects_short_storage() {
        for length in [3, 4] {
            let source = image(&[
                (0x42be, 1, linked(length, 0, 2, 100)),
                (20, 100, vec![0, 0, 0, 1, 0, 2]),
                (20, 1, b"abc".to_vec()),
                (20, 2, b"ignored".to_vec()),
            ]);
            let index = dd::parse(&source, &Limits::default()).expect("index");
            let descriptor = index.get(702, 1).expect("linked descriptor");
            assert_eq!(
                len(&source, &index, descriptor).expect("logical length"),
                u64::from(length)
            );
            let mut bytes = vec![0; usize::try_from(length).expect("length")];
            let result = read(&source, &index, descriptor, &mut bytes);
            if length == 3 {
                result.expect("first block supplies all data");
                assert_eq!(&bytes, b"abc");
            } else {
                assert!(matches!(
                    result,
                    Err(Error::Malformed {
                        what: "linked element ends before the requested data",
                        ..
                    })
                ));
            }
        }
        let source = image(&[(0x42be, 1, linked(0, 0, 0, 0))]);
        let index = dd::parse(&source, &Limits::default()).expect("index");
        assert!(matches!(
            len(&source, &index, index.get(702, 1).expect("DD")),
            Err(Error::Malformed { .. })
        ));
    }

    #[test]
    fn non_linked_specials_are_malformed_for_element_reads() {
        // hextelt.c:474-496 includes all three i32 fields and the external file name.
        let mut external = 2_u16.to_be_bytes().to_vec();
        external.extend(0_u32.to_be_bytes());
        external.extend(0_u32.to_be_bytes());
        external.extend(8_u32.to_be_bytes());
        external.extend(b"data.bin");
        let compressed = vec![0, 3, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 4, 0, 6];
        let mut body = vec![0];
        for value in [0_u32, 1, 1, 1] {
            body.extend(value.to_be_bytes());
        }
        body.extend(1962_u16.to_be_bytes());
        body.extend(1_u16.to_be_bytes());
        body.extend([0; 4]);
        for value in [1_u32, 0, 1, 1, 1] {
            body.extend(value.to_be_bytes());
        }
        body.push(0);
        let mut chunked = 5_u16.to_be_bytes().to_vec();
        chunked.extend(
            u32::try_from(body.len())
                .expect("body length")
                .to_be_bytes(),
        );
        chunked.extend(body);
        for payload in [external, compressed, chunked] {
            let source = image(&[(0x42be, 1, payload)]);
            let index = dd::parse(&source, &Limits::default()).expect("index");
            let descriptor = index.get(702, 1).expect("special DD");
            assert!(matches!(
                len(&source, &index, descriptor),
                Err(Error::Malformed {
                    what: "element reader requires plain or linked storage",
                    ..
                })
            ));
            assert!(matches!(
                read(&source, &index, descriptor, &mut []),
                Err(Error::Malformed {
                    what: "element reader requires plain or linked storage",
                    ..
                })
            ));
        }
    }

    #[test]
    fn element_reads_obey_the_index_array_limit() {
        let source = image(&[(702, 1, vec![1; 4])]);
        let limits = Limits {
            max_array_bytes: 3,
            ..Limits::default()
        };
        let index = dd::parse(&source, &limits).expect("index");
        let descriptor = index.get(702, 1).expect("DD");
        assert!(matches!(
            read(&source, &index, descriptor, &mut [0; 4]),
            Err(Error::LimitExceeded {
                limit: "array_bytes",
                ..
            })
        ));
    }

    #[test]
    fn unwritten_plain_elements_have_no_bytes() {
        let source = image(&[]);
        let index = dd::parse(&source, &Limits::default()).expect("index");
        let descriptor = Descriptor {
            tag: 702,
            reference: 1,
            offset: None,
            length: 0,
        };
        assert_eq!(
            len(&source, &index, &descriptor).expect("unwritten length"),
            0
        );
        read(&source, &index, &descriptor, &mut []).expect("empty read");
        assert!(read(&source, &index, &descriptor, &mut [0]).is_err());
    }

    #[test]
    fn linked_counts_and_table_buffers_obey_custom_limits() {
        let source = image(&[(0x42be, 1, linked(1, 1, 3, 100)), (20, 100, vec![0; 8])]);
        let limits = Limits {
            max_descriptors: 2,
            ..Limits::default()
        };
        let index = dd::parse(&source, &limits).expect("index");
        assert!(matches!(
            len(&source, &index, index.get(702, 1).expect("DD")),
            Err(Error::LimitExceeded {
                limit: "descriptors",
                ..
            })
        ));
        let limits = Limits {
            max_array_bytes: 4,
            ..Limits::default()
        };
        let index = dd::parse(&source, &limits).expect("index");
        assert!(matches!(
            read(&source, &index, index.get(702, 1).expect("DD"), &mut [0]),
            Err(Error::LimitExceeded {
                limit: "array_bytes",
                ..
            })
        ));
    }

    #[test]
    fn linked_counts_follow_custom_limits_above_default() {
        let count = 4_194_305;
        let source = image(&[(0x42be, 1, linked(0, 1, count, 0))]);
        let default_index = dd::parse(&source, &Limits::default()).expect("default index");
        assert!(matches!(
            len(
                &source,
                &default_index,
                default_index.get(702, 1).expect("DD")
            ),
            Err(Error::LimitExceeded {
                limit: "descriptors",
                ..
            })
        ));
        let limits = Limits {
            max_descriptors: u64::from(count),
            ..Limits::default()
        };
        let index = dd::parse(&source, &limits).expect("index with a raised limit");
        assert_eq!(
            len(&source, &index, index.get(702, 1).expect("DD")).expect("raised linked limit"),
            0
        );
    }

    #[test]
    fn linked_slot_budget_bounds_repeated_table_entries() {
        let source = image(&[
            (0x42be, 1, linked(1, 1, 2, 100)),
            (20, 100, vec![0, 101, 0, 0, 0, 0]),
            (20, 101, vec![0, 0, 0, 0, 0, 0]),
        ]);
        let limits = Limits {
            max_descriptors: 3,
            ..Limits::default()
        };
        let index = dd::parse(&source, &limits).expect("index");
        assert!(matches!(
            read(&source, &index, index.get(702, 1).expect("DD"), &mut [0]),
            Err(Error::LimitExceeded {
                limit: "descriptors",
                ..
            })
        ));
    }

    proptest! {
        #[test]
        fn linked_reads_equal_plain_reads(payload in prop::collection::vec(any::<u8>(), 1..128)) {
            let blocks: Vec<_> = payload.chunks(5).collect();
            let mut table = vec![0, 0];
            let mut records = vec![(0x42be, 1, linked(u32::try_from(payload.len()).expect("length"), 5, u32::try_from(blocks.len()).expect("count"), 100))];
            for (number, block) in blocks.iter().enumerate() {
                let reference = u16::try_from(number + 1).expect("block ref");
                table.extend(reference.to_be_bytes());
                records.push((20, reference, block.to_vec()));
            }
            records.push((20, 100, table));
            records.push((702, 2, payload.clone()));
            let source = image(&records);
            let index = dd::parse(&source, &Limits::default()).expect("index");
            let mut linked_bytes = vec![0; payload.len()];
            let mut plain_bytes = vec![0; payload.len()];
            read(&source, &index, index.get(702, 1).expect("linked DD"), &mut linked_bytes).expect("linked read");
            read(&source, &index, index.get(702, 2).expect("plain DD"), &mut plain_bytes).expect("plain read");
            prop_assert_eq!(linked_bytes, plain_bytes);
        }
    }
}
