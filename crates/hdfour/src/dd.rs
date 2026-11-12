//! Data descriptor blocks and their index.
#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::collections::{BTreeMap, BTreeSet};

use crate::{Error, Limits, Result, Source};

/// An on-disk data descriptor; `None` marks the never-written offset 0xffffffff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Descriptor {
    pub tag: u16,
    pub reference: u16,
    pub offset: Option<u64>,
    pub length: u64,
}

impl Descriptor {
    /// Whether the tag identifies a special-element header rather than plain bytes.
    pub fn is_special(&self) -> bool {
        // hfile_priv.h:386-390 distinguishes special tags from other high-bit tags.
        self.tag & 0xc000 == 0x4000
    }
}

/// Descriptors keyed by base tag and reference, with the file's resource budgets.
#[derive(Debug)]
pub struct Index {
    descriptors: BTreeMap<(u16, u16), Descriptor>,
    limits: Limits,
    metadata: crate::metadata::Budget,
    // Bound live parser caches separately from the returned metadata.
    parser_metadata: crate::metadata::Budget,
}

impl Index {
    /// Find a descriptor using either its base tag or its special tag.
    pub fn get(&self, tag: u16, reference: u16) -> Option<&Descriptor> {
        self.descriptors.get(&(base_tag(tag), reference))
    }

    /// Visit descriptors in base-tag, reference order, excluding null slots.
    pub fn iter(&self) -> impl Iterator<Item = &Descriptor> {
        self.descriptors.values()
    }

    pub(crate) fn metadata(&self) -> &crate::metadata::Budget {
        &self.metadata
    }

    pub(crate) fn parser_metadata(&self) -> &crate::metadata::Budget {
        &self.parser_metadata
    }

    pub(crate) fn limits(&self) -> &Limits {
        &self.limits
    }
}

/// Parse the HDF4 signature and every data-descriptor block without reading payloads.
pub fn parse(source: &dyn Source, limits: &Limits) -> Result<Index> {
    if source.len() < 4 {
        return Err(Error::NotHdf4);
    }
    let mut signature = [0; 4];
    read_at(source, 0, &mut signature)?;
    if signature != [0x0e, 3, 0x13, 1] {
        return Err(Error::NotHdf4);
    }
    let mut index = Index {
        descriptors: BTreeMap::new(),
        limits: limits.clone(),
        metadata: crate::metadata::Budget::new(limits.max_metadata_bytes),
        parser_metadata: crate::metadata::Budget::new(limits.max_metadata_bytes),
    };
    let mut visited = BTreeSet::new();
    let mut offset = 4;
    let mut descriptor_count = 0_u64;
    // DS 2.2-2.3: each DD block has a u16 count, u32 next offset, then 12-byte DDs.
    while offset != 0 {
        if !visited.insert(offset) {
            return Err(malformed(offset, "cycle in data-descriptor blocks"));
        }
        check_limit("dd_blocks", visited.len() as u64, limits.max_dd_blocks)?;
        let mut header = [0; 6];
        read_at(source, offset, &mut header)?;
        let [c0, c1, n0, n1, n2, n3] = header;
        let count = u64::from(u16::from_be_bytes([c0, c1]));
        descriptor_count = descriptor_count
            .checked_add(count)
            .ok_or_else(|| malformed(offset, "descriptor count overflow"))?;
        check_limit("descriptors", descriptor_count, limits.max_descriptors)?;
        let byte_count = count
            .checked_mul(12)
            .ok_or_else(|| malformed(offset, "DD block length overflow"))?;
        let records_offset = offset
            .checked_add(6)
            .ok_or_else(|| malformed(offset, "DD block offset overflow"))?;
        check_range(source, records_offset, byte_count)?;
        let mut records = crate::read::buffer(crate::read::allocation_size(byte_count)?)?;
        read_at(source, records_offset, &mut records)?;
        for bytes in records.chunks_exact(12) {
            let descriptor = parse_descriptor(bytes)?;
            if descriptor.tag == 1 {
                continue;
            }
            if descriptor.length > 2_147_483_647 {
                return Err(malformed(
                    records_offset,
                    "descriptor length exceeds HDF4 range",
                ));
            }
            if let Some(data_offset) = descriptor.offset {
                check_range(source, data_offset, descriptor.length)?;
            }
            let key = (base_tag(descriptor.tag), descriptor.reference);
            index.metadata.items::<((u16, u16), Descriptor)>(1)?;
            if index.descriptors.insert(key, descriptor).is_some() {
                return Err(malformed(
                    records_offset,
                    "duplicate descriptor tag and reference",
                ));
            }
        }
        offset = u64::from(u32::from_be_bytes([n0, n1, n2, n3]));
    }
    Ok(index)
}

fn parse_descriptor(bytes: &[u8]) -> Result<Descriptor> {
    let &[t0, t1, r0, r1, o0, o1, o2, o3, l0, l1, l2, l3] = bytes else {
        return Err(malformed(0, "truncated data descriptor"));
    };
    let offset = u32::from_be_bytes([o0, o1, o2, o3]);
    let length = u32::from_be_bytes([l0, l1, l2, l3]);
    Ok(Descriptor {
        tag: u16::from_be_bytes([t0, t1]),
        reference: u16::from_be_bytes([r0, r1]),
        offset: (offset != u32::MAX).then_some(u64::from(offset)),
        // hfile.c:3376-3378: two -1 sentinels mean no storage, not a 4 GiB element.
        length: if offset == u32::MAX && length == u32::MAX {
            0
        } else {
            u64::from(length)
        },
    })
}

fn base_tag(tag: u16) -> u16 {
    if tag & 0xc000 == 0x4000 {
        tag & 0x3fff
    } else {
        tag
    }
}

pub(crate) fn check_limit(limit: &'static str, value: u64, max: u64) -> Result<()> {
    if value > max {
        return Err(Error::LimitExceeded { limit, value, max });
    }
    Ok(())
}

pub(crate) fn check_range(source: &dyn Source, offset: u64, length: u64) -> Result<()> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| malformed(offset, "element range overflow"))?;
    if end > source.len() || end > 2_147_483_647 {
        return Err(malformed(
            offset,
            "element outside the file or HDF4 address range",
        ));
    }
    Ok(())
}

pub(crate) fn read_at(source: &dyn Source, offset: u64, bytes: &mut [u8]) -> Result<()> {
    check_range(source, offset, bytes.len() as u64)?;
    source.read_at(offset, bytes)
}

fn malformed(offset: u64, what: &'static str) -> Error {
    Error::Malformed { offset, what }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests {
    use super::*;
    use crate::Memory;

    #[test]
    fn c_written_unwritten_elements_have_two_sentinels() -> crate::Result<()> {
        let bytes = include_bytes!("../../../fixtures/F01.hdf");
        assert_eq!(
            bytes.get(862..874),
            Some(&[7, 171, 0, 62, 255, 255, 255, 255, 255, 255, 255, 255][..])
        );
        let index = parse(&Memory::new(bytes), &Limits::default())?;
        let descriptor = index.get(1963, 62).expect("unwritten SDSVar");
        assert_eq!(descriptor.offset, None);
        assert_eq!(descriptor.length, 0);
        Ok(())
    }

    fn image(records: &[(u16, u16, u32, u32)]) -> Vec<u8> {
        let mut bytes = vec![0x0e, 3, 0x13, 1];
        bytes.extend(
            u16::try_from(records.len())
                .expect("test count")
                .to_be_bytes(),
        );
        bytes.extend(0_u32.to_be_bytes());
        for &(tag, reference, offset, length) in records {
            bytes.extend(tag.to_be_bytes());
            bytes.extend(reference.to_be_bytes());
            bytes.extend(offset.to_be_bytes());
            bytes.extend(length.to_be_bytes());
        }
        bytes
    }

    #[test]
    fn t2_special_tags_match_only_their_base_tag() {
        let mut bytes = image(&[(0x42be, 7, 46, 2), (0x82be, 8, 48, 1), (0xc2be, 9, 49, 1)]);
        bytes.extend([0, 1, 2, 3]);
        let index = parse(&Memory::new(bytes), &Limits::default()).expect("DD index");
        let descriptor = index.get(702, 7).expect("base tag lookup");
        assert!(descriptor.is_special());
        assert_eq!(descriptor.tag, 0x42be);
        assert_eq!(descriptor.offset, Some(46));
        assert_eq!(descriptor.length, 2);
        assert_eq!(index.get(0x42be, 7), Some(descriptor));
        assert!(index.get(702, 8).is_none());
        assert!(index.get(0x82be, 8).is_some());
        assert!(index.get(702, 9).is_none());
        assert!(index.get(0xc2be, 9).is_some());
        assert_eq!(index.iter().count(), 3);
    }

    #[test]
    fn t2_null_descriptors_are_skipped_and_unwritten_offsets_are_retained() {
        let bytes = image(&[(1, 0, u32::MAX, u32::MAX), (702, 1, u32::MAX, 0)]);
        let index = parse(&Memory::new(bytes), &Limits::default()).expect("DD index");
        assert!(index.get(1, 0).is_none());
        assert_eq!(index.get(702, 1).expect("unwritten DD").offset, None);
    }

    #[test]
    fn reads_multiple_dd_blocks() {
        let mut bytes = image(&[]);
        bytes[6..10].copy_from_slice(&10_u32.to_be_bytes());
        bytes.extend_from_slice(&image(&[(702, 1, 28, 1)])[4..]);
        bytes.push(17);
        let index = parse(&Memory::new(bytes), &Limits::default()).expect("DD chain");
        assert_eq!(index.get(702, 1).expect("descriptor").length, 1);
    }

    #[test]
    fn dd_loop_is_malformed() {
        let mut bytes = image(&[]);
        bytes[6..10].copy_from_slice(&4_u32.to_be_bytes());
        assert!(matches!(
            parse(&Memory::new(bytes), &Limits::default()),
            Err(Error::Malformed { .. })
        ));
    }

    #[test]
    fn rejects_offsets_lengths_and_dd_blocks_past_end() {
        for bytes in [
            image(&[(702, 1, 23, 0)]),
            image(&[(702, 1, 22, 1)]),
            image(&[(702, 1, 0x8000_0000, 0)]),
            image(&[(702, 1, 0, 0x8000_0000)]),
            image(&[(702, 1, u32::MAX, 0x8000_0000)]),
            vec![0x0e, 3, 0x13, 1, 0, 1, 0, 0, 0, 0],
        ] {
            assert!(matches!(
                parse(&Memory::new(bytes), &Limits::default()),
                Err(Error::Malformed { .. })
            ));
        }
    }

    #[test]
    fn rejects_wrong_magic_truncation_and_duplicate_keys() {
        for bytes in [vec![], vec![0x0e, 3, 0x13], vec![0x89, b'H', b'D', b'F']] {
            assert!(matches!(
                parse(&Memory::new(bytes), &Limits::default()),
                Err(Error::NotHdf4)
            ));
        }
        let bytes = image(&[(702, 1, 34, 0), (0x42be, 1, 34, 0)]);
        assert!(matches!(
            parse(&Memory::new(bytes), &Limits::default()),
            Err(Error::Malformed { .. })
        ));
    }

    #[test]
    fn hdf4_address_ceiling_is_checked_even_for_a_larger_source() {
        struct LargeSource(Memory<Vec<u8>>);
        impl Source for LargeSource {
            fn len(&self) -> u64 {
                4_294_967_296
            }
            fn read_at(&self, offset: u64, bytes: &mut [u8]) -> Result<()> {
                self.0.read_at(offset, bytes)
            }
        }
        let source = LargeSource(Memory::new(image(&[(702, 1, 2_147_483_647, 1)])));
        assert!(matches!(
            parse(&source, &Limits::default()),
            Err(Error::Malformed { .. })
        ));
    }

    #[test]
    fn dd_counts_obey_custom_limits_including_null_slots() {
        let limits = Limits {
            max_descriptors: 0,
            ..Limits::default()
        };
        assert!(matches!(
            parse(&Memory::new(image(&[(1, 0, 0, 0)])), &limits),
            Err(Error::LimitExceeded {
                limit: "descriptors",
                ..
            })
        ));
        let limits = Limits {
            max_dd_blocks: 0,
            ..Limits::default()
        };
        assert!(matches!(
            parse(&Memory::new(image(&[])), &limits),
            Err(Error::LimitExceeded {
                limit: "dd_blocks",
                ..
            })
        ));
    }
}
