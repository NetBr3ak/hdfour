//! Vgroup and Vdata metadata records.
#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::metadata::Budget;
use crate::{Error, Result, Value, nt};

/// A Vgroup's ordered children and attribute Vdata references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vgroup {
    pub version: u16,
    pub name: String,
    pub class: String,
    /// Child `(tag, reference)` pairs in file order.
    pub members: Vec<(u16, u16)>,
    /// Attribute `(tag, reference)` pairs in file order.
    pub attributes: Vec<(u16, u16)>,
}

/// One field in a fully interlaced Vdata record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VdataField {
    pub name: String,
    /// Modern type code, with byte-order flags retained.
    pub type_code: u16,
    /// Stored bytes per record, including all values of this field.
    pub size: u16,
    /// Byte offset within each record.
    pub offset: u16,
    /// Number of values per record.
    pub order: u16,
}

/// Metadata needed to select a field from a Vdata storage element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VdataHeader {
    pub version: u16,
    pub name: String,
    pub class: String,
    /// Only full interlace (zero) is accepted.
    pub interlace: u16,
    pub records: u32,
    pub record_size: u16,
    pub fields: Vec<VdataField>,
    /// `(field index, tag, reference)`; index -1 denotes the whole Vdata.
    pub attributes: Vec<(i32, u16, u16)>,
}

/// Parse a version 3 or 4 Vgroup, preserving child and attribute order.
/// Names and classes use a byte-preserving Latin-1 mapping.
pub fn parse_vgroup(bytes: &[u8]) -> Result<Vgroup> {
    parse_vgroup_with_budget(bytes, None)
}

pub(crate) fn parse_vgroup_with_budget(bytes: &[u8], budget: Option<&Budget>) -> Result<Vgroup> {
    let (mut input, version, _) = record(bytes, budget)?;
    if !matches!(version, 3 | 4) {
        return Err(input.error("Vgroup version must be 3 or 4"));
    }
    // vgp.c:952-1008 writes separate tag/ref arrays, then v4 attribute pairs.
    let count = usize::from(input.word()?);
    let tags = input.words(count)?;
    let references = input.words(count)?;
    let mut members = input.reserve_items(count)?;
    members.extend(tags.into_iter().zip(references));
    let name = input.name(u16::MAX)?;
    let class = input.name(u16::MAX)?;
    input.take(4)?;
    let mut attributes = Vec::new();
    if version == 4 && input.dword()? & 1 != 0 {
        let count = input.count()?;
        input.require_entries(count, 4)?;
        attributes = input.reserve_items(count)?;
        for _ in 0..count {
            attributes.push((input.word()?, input.word()?));
        }
    }
    input.finish()?;
    Ok(Vgroup {
        version,
        name,
        class,
        members,
        attributes,
    })
}

/// Parse a version 2, 3 or 4 Vdata header without decoding its field types.
/// Names and classes use a byte-preserving Latin-1 mapping.
pub fn parse_vdata_header(bytes: &[u8]) -> Result<VdataHeader> {
    let header = parse_vdata_header_for_sd(bytes, None)?;
    if header.interlace != 0 {
        return Err(malformed("Vdata must use full interlace"));
    }
    Ok(header)
}

pub(crate) fn parse_vdata_header_for_sd(
    bytes: &[u8],
    budget: Option<&Budget>,
) -> Result<VdataHeader> {
    let (mut input, version, more) = record(bytes, budget)?;
    if !(2..=4).contains(&version) {
        return Err(input.error("Vdata version must be 2, 3 or 4"));
    }
    // vio.c:475-572 stores type, size, offset and order as separate arrays.
    let interlace = input.word()?;
    let records = input.dword()?;
    if records > i32::MAX as u32 {
        return Err(input.error("negative Vdata record count"));
    }
    let record_size = input.word()?;
    let count = input.word()?;
    if count > i16::MAX as u16 {
        return Err(input.error("negative Vdata field count"));
    }
    let count = usize::from(count);
    input.require_entries(count, 10)?;
    let types = input.words(count)?;
    let sizes = input.words(count)?;
    let offsets = input.words(count)?;
    let orders = input.words(count)?;
    let mut fields = input.reserve_items(count)?;
    for (((type_code, size), offset), order) in
        types.into_iter().zip(sizes).zip(offsets).zip(orders)
    {
        let name = input.name(i16::MAX as u16)?;
        fields.push(VdataField {
            name,
            type_code: if version <= 2 {
                modern_type(type_code)
            } else {
                type_code
            },
            size,
            offset,
            order,
        });
    }
    let name = input.name(i16::MAX as u16)?;
    let class = input.name(i16::MAX as u16)?;
    input.take(4)?;
    // vio.c:585-615 checks the middle pair; old headers can share the final pair.
    if !input.remaining()?.is_empty() && (input.word()? != version || input.word()? != more) {
        return Err(input.error("Vdata version or more copies disagree"));
    }
    let mut attributes = Vec::new();
    if version == 4 && input.dword()? & 1 != 0 {
        let count = input.count()?;
        input.require_entries(count, 8)?;
        attributes = input.reserve_items(count)?;
        for _ in 0..count {
            let field = i32::from_be_bytes(input.array()?);
            if field < -1
                || field
                    >= i32::try_from(fields.len())
                        .map_err(|_| input.error("field count overflow"))?
            {
                return Err(input.error("Vdata attribute field index is outside its field list"));
            }
            attributes.push((field, input.word()?, input.word()?));
        }
    }
    input.finish()?;
    for field in &fields {
        field_range(field, record_size)?;
    }
    Ok(VdataHeader {
        version,
        name,
        class,
        interlace,
        records,
        record_size,
        fields,
        attributes,
    })
}

/// Decode one field from an exact, fully interlaced Vdata record block.
/// Unsupported types remain `number-type` errors from the number-type module.
pub fn field_values(header: &VdataHeader, bytes: &[u8], field: usize) -> Result<Value> {
    if header.interlace != 0 {
        return Err(malformed("Vdata must use full interlace"));
    }
    let field = header
        .fields
        .get(field)
        .ok_or_else(|| malformed("Vdata field index is outside its field list"))?;
    let range = field_range(field, header.record_size)?;
    let (dtype, order) = nt::from_vdata_type(field.type_code)?;
    let width = usize::from(field.order)
        .checked_mul(dtype.size())
        .ok_or_else(|| malformed("Vdata field size overflow"))?;
    if width != usize::from(field.size) {
        return Err(malformed(
            "Vdata field size does not match its type and order",
        ));
    }
    let length = u64::from(header.records)
        .checked_mul(u64::from(header.record_size))
        .ok_or_else(|| malformed("Vdata block size overflow"))?;
    if length > 2_147_483_647 || length != bytes.len() as u64 {
        return Err(malformed(
            "Vdata block length does not match its record count and size",
        ));
    }
    let capacity = usize::try_from(header.records)
        .ok()
        .and_then(|records| records.checked_mul(width))
        .ok_or_else(|| malformed("Vdata field length overflow"))?;
    crate::read::allocation_size(capacity as u64)?;
    let mut selected = Vec::new();
    selected
        .try_reserve_exact(capacity)
        .map_err(|_| crate::read::allocation_error(capacity as u64))?;
    // DS 7.6: full interlace repeats the complete record, not each field's column.
    for record in bytes.chunks_exact(usize::from(header.record_size)) {
        selected.extend_from_slice(
            record
                .get(range.clone())
                .ok_or_else(|| malformed("Vdata field extends past its record"))?,
        );
    }
    nt::decode(dtype, order, &selected)
}

fn field_range(field: &VdataField, record_size: u16) -> Result<std::ops::Range<usize>> {
    let start = usize::from(field.offset);
    let end = start
        .checked_add(usize::from(field.size))
        .ok_or_else(|| malformed("Vdata field extent overflow"))?;
    if field.order == 0 || field.size == 0 || end > usize::from(record_size) {
        return Err(malformed(
            "Vdata field has an invalid size, order or offset",
        ));
    }
    Ok(start..end)
}

fn reserve<T>(count: usize) -> Result<Vec<T>> {
    let bytes = count
        .checked_mul(std::mem::size_of::<T>())
        .ok_or_else(|| crate::read::allocation_error(u64::MAX))?;
    crate::read::allocation_size(bytes as u64)?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| crate::read::allocation_error(bytes as u64))?;
    Ok(values)
}

fn modern_type(code: u16) -> u16 {
    // vconv.c:428-457 maps the complete old code; other codes pass through.
    match code {
        1 => 4,
        2 | 6 => 22,
        3 => 5,
        4 => 24,
        5 => 20,
        7 => 6,
        _ => code,
    }
}

fn record<'a>(bytes: &'a [u8], budget: Option<&'a Budget>) -> Result<(Cursor<'a>, u16, u16)> {
    // vgp.c:1045-1053 and vio.c:455-470 retain the historical extra byte.
    let end = bytes
        .len()
        .checked_sub(5)
        .ok_or_else(|| malformed("truncated Vset trailer"))?;
    if bytes.len() > 2_147_483_647 {
        return Err(malformed("Vset record exceeds the HDF4 address range"));
    }
    let body = bytes
        .get(..end)
        .ok_or_else(|| malformed("truncated Vset record"))?;
    let mut trailer = Cursor {
        bytes,
        position: end,
        budget,
    };
    let version = trailer.word()?;
    let more = trailer.word()?;
    Ok((
        Cursor {
            bytes: body,
            position: 0,
            budget,
        },
        version,
        more,
    ))
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
    budget: Option<&'a Budget>,
}

impl<'a> Cursor<'a> {
    fn error(&self, what: &'static str) -> Error {
        Error::Malformed {
            offset: self.position as u64,
            what,
        }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .ok_or_else(|| self.error("Vset length overflow"))?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| self.error("truncated Vset record"))?;
        self.position = end;
        Ok(value)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?
            .try_into()
            .map_err(|_| self.error("truncated Vset number"))
    }

    fn word(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn dword(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    fn count(&mut self) -> Result<usize> {
        let count = self.dword()?;
        if count > i32::MAX as u32 {
            return Err(self.error("negative Vset attribute count"));
        }
        usize::try_from(count).map_err(|_| self.error("Vset count overflow"))
    }

    fn words(&mut self, count: usize) -> Result<Vec<u16>> {
        self.require_entries(count, 2)?;
        let mut values = reserve(count)?;
        for _ in 0..count {
            values.push(self.word()?);
        }
        Ok(values)
    }

    fn reserve_items<T>(&self, count: usize) -> Result<Vec<T>> {
        if let Some(budget) = self.budget {
            budget.items::<T>(count)?;
        }
        reserve(count)
    }

    fn name(&mut self, max: u16) -> Result<String> {
        let length = self.word()?;
        if length > max {
            return Err(self.error("negative Vdata name length"));
        }
        let bytes = self.take(usize::from(length))?;
        if let Some(budget) = self.budget {
            return budget.latin1(bytes);
        }
        let length = bytes.iter().try_fold(0_usize, |length, byte| {
            length
                .checked_add(char::from(*byte).len_utf8())
                .ok_or_else(|| self.error("Vset name length overflow"))
        })?;
        crate::read::allocation_size(length as u64)?;
        let mut name = String::new();
        name.try_reserve_exact(length)
            .map_err(|_| crate::read::allocation_error(length as u64))?;
        name.extend(bytes.iter().map(|byte| char::from(*byte)));
        Ok(name)
    }

    fn remaining(&self) -> Result<&'a [u8]> {
        self.bytes
            .get(self.position..)
            .ok_or_else(|| self.error("Vset cursor is outside its record"))
    }

    fn require_entries(&self, count: usize, width: usize) -> Result<()> {
        let length = count
            .checked_mul(width)
            .ok_or_else(|| self.error("Vset array size overflow"))?;
        if length > self.remaining()?.len() {
            return Err(self.error("truncated Vset array"));
        }
        Ok(())
    }

    fn finish(&self) -> Result<()> {
        if !self.remaining()?.is_empty() {
            return Err(self.error("unexpected bytes before Vset trailer"));
        }
        Ok(())
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
    use proptest::prelude::*;

    fn word(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend(value.to_be_bytes());
    }

    fn text(bytes: &mut Vec<u8>, value: &[u8]) {
        word(bytes, u16::try_from(value.len()).expect("short test name"));
        bytes.extend(value);
    }

    fn trailer(bytes: &mut Vec<u8>, version: u16) {
        word(bytes, version);
        word(bytes, 0);
        bytes.push(0);
    }

    fn group(version: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        for value in [2, 1965, 1962, 7, 9] {
            word(&mut bytes, value);
        }
        text(&mut bytes, b"variable");
        text(&mut bytes, b"Var0.0");
        word(&mut bytes, 0);
        word(&mut bytes, 0);
        if version == 4 {
            bytes.extend(1_u32.to_be_bytes());
            bytes.extend(2_i32.to_be_bytes());
            for value in [1962, 11, 1962, 12] {
                word(&mut bytes, value);
            }
        }
        trailer(&mut bytes, version);
        bytes
    }

    fn header(version: u16, code: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        word(&mut bytes, 0);
        bytes.extend(2_i32.to_be_bytes());
        for value in [8, 2, code, 21, 4, 1, 1, 6, 2, 1] {
            word(&mut bytes, value);
        }
        text(&mut bytes, b"values");
        text(&mut bytes, b"marker");
        text(&mut bytes, b"table");
        text(&mut bytes, b"Attr0.0");
        word(&mut bytes, 0);
        word(&mut bytes, 0);
        word(&mut bytes, version);
        word(&mut bytes, 0);
        if version == 4 {
            bytes.extend(1_u32.to_be_bytes());
            bytes.extend(2_i32.to_be_bytes());
            for (field, reference) in [(-1_i32, 31_u16), (0, 32)] {
                bytes.extend(field.to_be_bytes());
                word(&mut bytes, 1962);
                word(&mut bytes, reference);
            }
        }
        trailer(&mut bytes, version);
        bytes
    }

    fn malformed<T>(result: Result<T>) {
        assert!(matches!(result, Err(Error::Malformed { .. })));
    }

    #[test]
    fn sq13_vset_reservations_reject_capacity_overflow() {
        for result in [
            reserve::<(u16, u16)>(usize::MAX).map(|_| ()),
            reserve::<(i32, u16, u16)>(usize::MAX).map(|_| ()),
            reserve::<(u16, u16)>(isize::MAX as usize / 4 + 1).map(|_| ()),
            reserve::<(i32, u16, u16)>(isize::MAX as usize / 8 + 1).map(|_| ()),
        ] {
            assert!(matches!(
                result,
                Err(Error::LimitExceeded {
                    limit: "allocation_bytes",
                    ..
                })
            ));
        }
    }

    #[test]
    fn sq13_v4_attribute_counts_are_preserved() -> Result<()> {
        let count = 257_u32;
        let mut group = group(4);
        group.truncate(group.len() - 17);
        group.extend(count.to_be_bytes());
        for reference in 0..count {
            word(&mut group, 1962);
            word(&mut group, u16::try_from(reference).expect("reference"));
        }
        trailer(&mut group, 4);
        let group = parse_vgroup(&group)?;
        assert_eq!(group.attributes.len(), count as usize);
        assert_eq!(group.attributes.last(), Some(&(1962, 256)));

        let mut header = header(4, 22);
        header.truncate(header.len() - 25);
        header.extend(count.to_be_bytes());
        for reference in 0..count {
            header.extend((-1_i32).to_be_bytes());
            word(&mut header, 1962);
            word(&mut header, u16::try_from(reference).expect("reference"));
        }
        trailer(&mut header, 4);
        let header = parse_vdata_header(&header)?;
        assert_eq!(header.attributes.len(), count as usize);
        assert_eq!(header.attributes.last(), Some(&(-1, 1962, 256)));
        Ok(())
    }

    #[test]
    fn vgroup_v3_v4_records() -> Result<()> {
        for version in [3, 4] {
            let group = parse_vgroup(&group(version))?;
            assert_eq!(group.version, version);
            assert_eq!(group.name, "variable");
            assert_eq!(group.class, "Var0.0");
            assert_eq!(group.members, vec![(1965, 7), (1962, 9)]);
            assert_eq!(
                group.attributes,
                if version == 4 {
                    vec![(1962, 11), (1962, 12)]
                } else {
                    vec![]
                }
            );
        }
        Ok(())
    }

    #[test]
    fn vgroup_version_is_at_length_minus_five() -> Result<()> {
        let mut bytes = group(3);
        *bytes.last_mut().expect("padding") = 0xff;
        assert_eq!(parse_vgroup(&bytes)?.version, 3);
        let version_offset = bytes.len() - 5;
        bytes[version_offset..version_offset + 2].copy_from_slice(&5_u16.to_be_bytes());
        malformed(parse_vgroup(&bytes));
        malformed(parse_vgroup(&group(2)));
        Ok(())
    }

    #[test]
    fn vdata_v2_v3_v4_records() -> Result<()> {
        for version in [2, 3, 4] {
            let code = if version == 2 { 2 } else { 22 };
            let parsed = parse_vdata_header(&header(version, code))?;
            assert_eq!(parsed.version, version);
            assert_eq!(parsed.name, "table");
            assert_eq!(parsed.class, "Attr0.0");
            assert_eq!(parsed.records, 2);
            assert_eq!(parsed.record_size, 8);
            assert_eq!(parsed.fields[0].name, "values");
            assert_eq!(parsed.fields[0].type_code, 22);
            assert_eq!(parsed.fields[0].size, 4);
            assert_eq!(parsed.fields[0].offset, 1);
            assert_eq!(parsed.fields[0].order, 2);
            assert_eq!(
                parsed.attributes,
                if version == 4 {
                    vec![(-1, 1962, 31), (0, 1962, 32)]
                } else {
                    vec![]
                }
            );
        }
        Ok(())
    }

    #[test]
    fn vdata_old_type_codes() -> Result<()> {
        for (old, modern) in [
            (1, 4),
            (2, 22),
            (3, 5),
            (4, 24),
            (5, 20),
            (6, 22),
            (7, 6),
            (23, 23),
            (0x4016, 0x4016),
        ] {
            let parsed = parse_vdata_header(&header(2, old))?;
            assert_eq!(parsed.fields[0].type_code, modern);
        }
        assert_eq!(parse_vdata_header(&header(3, 2))?.fields[0].type_code, 2);
        Ok(())
    }

    #[test]
    fn vdata_single_trailer_and_empty_header() -> Result<()> {
        for version in [2, 3] {
            let mut bytes = header(version, 22);
            bytes.drain(bytes.len() - 9..bytes.len() - 5);
            assert_eq!(parse_vdata_header(&bytes)?.version, version);
        }
        let mut empty = vec![0; 10];
        empty.extend([0; 8]);
        trailer(&mut empty, 3);
        let parsed = parse_vdata_header(&empty)?;
        assert!(parsed.fields.is_empty());
        assert_eq!(parsed.records, 0);
        Ok(())
    }

    #[test]
    fn truncated_records_and_bad_lengths() {
        for bytes in [group(3), group(4)] {
            for end in 0..bytes.len() {
                malformed(parse_vgroup(&bytes[..end]));
            }
        }
        for (version, mut bytes) in [(2, header(2, 2)), (3, header(3, 22)), (4, header(4, 22))] {
            if version < 4 {
                bytes.drain(bytes.len() - 9..bytes.len() - 5);
            }
            for end in 0..bytes.len() {
                malformed(parse_vdata_header(&bytes[..end]));
            }
        }
        let mut bytes = group(4);
        let count = bytes.len() - 17;
        bytes[count..count + 4].copy_from_slice(&(-1_i32).to_be_bytes());
        malformed(parse_vgroup(&bytes));
        let mut bytes = header(3, 22);
        bytes[8..10].copy_from_slice(&0xffff_u16.to_be_bytes());
        malformed(parse_vdata_header(&bytes));
        let mut bytes = header(3, 22);
        bytes[2..6].copy_from_slice(&(-1_i32).to_be_bytes());
        malformed(parse_vdata_header(&bytes));
        let mut bytes = header(3, 22);
        let middle_version = bytes.len() - 9;
        bytes[middle_version..middle_version + 2].copy_from_slice(&4_u16.to_be_bytes());
        malformed(parse_vdata_header(&bytes));
    }

    #[test]
    fn fully_interlaced_little_endian_field() -> Result<()> {
        let parsed = parse_vdata_header(&header(3, 0x4016))?;
        let bytes = [
            99, 1, 0, 0xfe, 0xff, 98, 7, 97, 96, 3, 0, 0xfc, 0xff, 95, 8, 94,
        ];
        assert_eq!(
            field_values(&parsed, &bytes, 0)?,
            Value::I16(vec![1, -2, 3, -4])
        );
        assert_eq!(field_values(&parsed, &bytes, 1)?, Value::U8(vec![7, 8]));
        let big = parse_vdata_header(&header(3, 22))?;
        assert_eq!(
            field_values(&big, &bytes, 0)?,
            Value::I16(vec![256, -257, 768, -769])
        );
        Ok(())
    }

    #[test]
    fn field_bounds_sizes_and_interlace() -> Result<()> {
        let mut bytes = header(3, 22);
        bytes[0..2].copy_from_slice(&1_u16.to_be_bytes());
        malformed(parse_vdata_header(&bytes));
        let mut parsed = parse_vdata_header(&header(3, 22))?;
        malformed(field_values(&parsed, &[0; 15], 0));
        malformed(field_values(&parsed, &[0; 17], 0));
        malformed(field_values(&parsed, &[0; 16], 2));
        parsed.fields[0].offset = 7;
        malformed(field_values(&parsed, &[0; 16], 0));
        parsed.fields[0].offset = 1;
        parsed.fields[0].order = 3;
        malformed(field_values(&parsed, &[0; 16], 0));
        parsed.fields[0].order = 0;
        malformed(field_values(&parsed, &[0; 16], 0));
        let mut parsed = parse_vdata_header(&header(3, 22))?;
        parsed.interlace = 1;
        malformed(field_values(&parsed, &[0; 16], 0));
        parsed.interlace = 0;
        parsed.records = 0;
        assert_eq!(field_values(&parsed, &[], 0)?, Value::I16(vec![]));
        Ok(())
    }

    #[test]
    fn unsupported_field_types_are_deferred_to_decode() -> Result<()> {
        for code in [2, 0x1016, 0x2016, 0x3016, 0x8016] {
            let parsed = parse_vdata_header(&header(3, code))?;
            assert!(matches!(
                field_values(&parsed, &[0; 16], 0),
                Err(Error::Unsupported {
                    feature: "number-type",
                    ..
                })
            ));
        }
        Ok(())
    }

    proptest! {
        #[test]
        fn arbitrary_records_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = parse_vgroup(&bytes);
            if let Ok(parsed) = parse_vdata_header(&bytes) {
                let _ = field_values(&parsed, &bytes, 0);
            }
        }
    }
}
