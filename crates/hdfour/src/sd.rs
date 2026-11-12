//! Scientific dataset discovery and metadata.
#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::collections::{BTreeMap, BTreeSet};

use crate::odl::{Entry, Odl};
use crate::read::{Reader, allocation, array_bytes, malformed, value_type};
use crate::vset::{self, VdataHeader, Vgroup};
use crate::{
    Attribute, Dataset, Dimension, Eos, EosObject, Error, Result, Skipped, Value, dd, eos, nt, odl,
};

pub(crate) struct Metadata {
    pub datasets: Vec<Dataset>,
    pub attributes: Vec<Attribute>,
    pub skipped: Vec<Skipped>,
    pub eos: Option<Eos>,
}

struct Catalog<'a, 'b> {
    reader: &'a Reader<'b>,
    groups: BTreeMap<u16, Vgroup>,
    headers: BTreeMap<u16, VdataHeader>,
    attribute_bytes: u64,
    skipped: Vec<Skipped>,
}

pub(crate) fn parse(reader: &Reader<'_>) -> Result<Metadata> {
    let mut catalog = Catalog {
        reader,
        groups: BTreeMap::new(),
        headers: BTreeMap::new(),
        attribute_bytes: 0,
        skipped: Vec::new(),
    };
    for descriptor in reader.index.iter() {
        match descriptor.tag {
            1965 => {
                catalog.groups.insert(
                    descriptor.reference,
                    vset::parse_vgroup(&reader.bytes(descriptor)?)?,
                );
            }
            1962 => {
                catalog.headers.insert(
                    descriptor.reference,
                    vset::parse_vdata_header(&reader.bytes(descriptor)?)?,
                );
            }
            _ => {}
        }
    }
    catalog.check_graph()?;
    // cdf.c:1869-1892 selects the first CDF0.0 Vgroup; its child order is SDselect order.
    let root = catalog
        .groups
        .iter()
        .find(|(_, group)| group.class == "CDF0.0")
        .map(|(reference, _)| *reference);
    let Some(root) = root else {
        for descriptor in reader
            .index
            .iter()
            .filter(|descriptor| matches!(descriptor.tag, 700 | 720))
        {
            catalog.skipped.push(Skipped {
                object: format!(
                    "{} {}",
                    if descriptor.tag == 720 { "NDG" } else { "SDG" },
                    descriptor.reference
                ),
                feature: "legacy-sd-layout",
            });
        }
        return Ok(Metadata {
            datasets: Vec::new(),
            attributes: Vec::new(),
            skipped: catalog.skipped,
            eos: None,
        });
    };
    let members = catalog.group(root)?.members.clone();
    let mut datasets = Vec::new();
    for &(tag, reference) in &members {
        if tag != 1965 || catalog.group(reference)?.class != "Var0.0" {
            continue;
        }
        match catalog.dataset(reference) {
            Ok(dataset) => datasets.push(dataset),
            Err(Error::Unsupported {
                feature: "number-type" | "ragged-array",
                ..
            }) => {}
            Err(error) => return Err(error),
        }
    }
    let attributes = catalog.attributes(root)?;
    catalog.dimension_attributes(&mut datasets)?;
    let eos = catalog.eos(&attributes, &datasets)?;
    Ok(Metadata {
        datasets,
        attributes,
        skipped: catalog.skipped,
        eos,
    })
}

impl Catalog<'_, '_> {
    fn group(&self, reference: u16) -> Result<&Vgroup> {
        self.groups
            .get(&reference)
            .ok_or_else(|| malformed("missing Vgroup"))
    }

    fn header(&self, reference: u16) -> Result<&VdataHeader> {
        self.headers
            .get(&reference)
            .ok_or_else(|| malformed("missing Vdata header"))
    }

    fn check_graph(&self) -> Result<()> {
        let mut heights = BTreeMap::<u16, u64>::new();
        for &root in self.groups.keys() {
            self.check_subgraph(root, &mut heights)?;
        }
        Ok(())
    }

    fn check_subgraph(&self, root: u16, heights: &mut BTreeMap<u16, u64>) -> Result<()> {
        let mut visiting = BTreeSet::new();
        let mut stack = vec![(root, false)];
        while let Some((reference, exiting)) = stack.pop() {
            if heights.contains_key(&reference) {
                continue;
            }
            let group = self.group(reference)?;
            if !exiting {
                if !visiting.insert(reference) {
                    return Err(malformed("cycle in Vgroups"));
                }
                dd::check_limit(
                    "group_depth",
                    visiting.len() as u64,
                    self.reader.limits().max_group_depth,
                )?;
                stack.push((reference, true));
                for &(_, child) in group.members.iter().rev().filter(|(tag, _)| *tag == 1965) {
                    stack.push((child, false));
                }
                continue;
            }
            let mut height = 1_u64;
            for &(_, child) in group.members.iter().filter(|(tag, _)| *tag == 1965) {
                let child_height = heights
                    .get(&child)
                    .ok_or_else(|| malformed("unresolved Vgroup depth"))?;
                height = height.max(
                    child_height
                        .checked_add(1)
                        .ok_or_else(|| malformed("Vgroup depth overflow"))?,
                );
            }
            dd::check_limit("group_depth", height, self.reader.limits().max_group_depth)?;
            heights.insert(reference, height);
            visiting.remove(&reference);
        }
        Ok(())
    }

    fn dataset(&mut self, reference: u16) -> Result<Dataset> {
        let group = self.group(reference)?;
        let name = group.name.clone();
        let members = group.members.clone();
        if members.iter().any(|(tag, _)| *tag == 781) {
            return Err(self.skip(name, "ragged-array"));
        }
        let number = members
            .iter()
            .find(|(tag, _)| *tag == 106)
            .copied()
            .ok_or_else(|| malformed("dataset has no number type"))?;
        let (dtype, order) = match nt::parse(&self.reader.bytes(self.reader.descriptor(number)?)?) {
            Err(Error::Unsupported {
                feature: "number-type",
                ..
            }) => return Err(self.skip(name, "number-type")),
            result => result?,
        };
        let dimension_refs: Vec<_> = members
            .iter()
            .filter(|(tag, _)| *tag == 1965)
            .map(|(_, reference)| *reference)
            .collect();
        dd::check_limit(
            "rank",
            dimension_refs.len() as u64,
            self.reader.limits().max_rank,
        )?;
        let mut dims = dimension_refs
            .into_iter()
            .map(|reference| self.dimension(reference))
            .collect::<Result<Vec<_>>>()?;
        let data = members.iter().find(|(tag, _)| *tag == 702).copied();
        let (storage, codec, length) = self.reader.layout(data)?;
        if dims.first().is_some_and(|dimension| dimension.unlimited) {
            let mut record_bytes = dtype.size() as u64;
            for dimension in dims.iter().skip(1) {
                if dimension.unlimited {
                    return Err(malformed("unlimited dimension is not first"));
                }
                record_bytes = record_bytes
                    .checked_mul(dimension.size)
                    .ok_or_else(|| malformed("record size overflow"))?;
            }
            let first = dims
                .first_mut()
                .ok_or_else(|| malformed("missing first dimension"))?;
            // cdf.c:1787-1817 derives each variable's record count from its logical storage length.
            first.size = length
                .checked_div(record_bytes)
                .ok_or_else(|| malformed("zero record byte length"))?;
        }
        if dims.iter().skip(1).any(|dim| dim.unlimited) {
            return Err(malformed("unlimited dimension is not first"));
        }
        let shape: Vec<_> = dims.iter().map(|dim| dim.size).collect();
        array_bytes(&shape, dtype.size(), self.reader.limits())?;
        let is_coordinate = members.iter().filter(|(tag, _)| *tag == 1962).try_fold(
            false,
            |coordinate, (_, reference)| {
                Ok::<_, Error>(coordinate || self.header(*reference)?.class == "CoordVar")
            },
        )?;
        let ndg_ref = members
            .iter()
            .find(|(tag, _)| *tag == 720)
            .map(|(_, reference)| *reference)
            .ok_or_else(|| malformed("dataset has no NDG reference"))?;
        let attributes = self.attributes(reference)?;
        Ok(Dataset {
            name,
            dtype,
            shape,
            dims,
            attributes,
            is_coordinate,
            storage,
            codec,
            data,
            ndg_ref,
            order,
        })
    }

    fn dimension(&self, reference: u16) -> Result<Dimension> {
        let group = self.group(reference)?;
        if !matches!(group.class.as_str(), "Dim0.0" | "UDim0.0") {
            return Err(malformed("non-dimension Vgroup in dataset"));
        }
        let unlimited = group.class == "UDim0.0";
        let mut size = None;
        // cdf.c:1236-1275: DimVal0.0 uses the record count; DimVal0.1 stores one i32 size.
        for &(_, reference) in group.members.iter().filter(|(tag, _)| *tag == 1962) {
            let header = self.header(reference)?;
            match header.class.as_str() {
                "DimVal0.0" if !unlimited => {
                    if size.is_none() {
                        size = Some(u64::from(header.records));
                    }
                }
                "DimVal0.1" | "DimVal0.0" => {
                    let bytes = self.reader.records(header, reference)?;
                    let Value::I32(values) = vset::field_values(header, &bytes, 0)? else {
                        return Err(malformed("dimension size must be i32"));
                    };
                    let value = *values
                        .first()
                        .ok_or_else(|| malformed("missing dimension size"))?;
                    size = Some(
                        u64::try_from(value).map_err(|_| malformed("negative dimension size"))?,
                    );
                }
                _ => {}
            }
        }
        let size = size.ok_or_else(|| malformed("dimension has no size"))?;
        Ok(Dimension {
            name: group.name.clone(),
            size: if unlimited { 0 } else { size },
            unlimited,
            attributes: Vec::new(),
        })
    }

    fn attributes(&mut self, reference: u16) -> Result<Vec<Attribute>> {
        let group = self.group(reference)?;
        let parent = if group.class == "CDF0.0" {
            String::new()
        } else {
            group.name.clone()
        };
        let members = group.members.clone();
        let mut attributes = Vec::new();
        // cdf.c:1412-1493 reads Attr0.0 children, not the Vgroup API's separate v4 attributes.
        for (_, reference) in members.into_iter().filter(|(tag, _)| *tag == 1962) {
            let header = self.header(reference)?;
            if header.class != "Attr0.0" {
                continue;
            }
            let name = header.name.clone();
            let field = header
                .fields
                .first()
                .ok_or_else(|| malformed("attribute has no field"))?;
            if let Err(error) = nt::from_vdata_type(field.type_code) {
                match error {
                    Error::Unsupported {
                        feature: "number-type",
                        ..
                    } => {
                        self.skip(format!("{parent}/{name}"), "number-type");
                        continue;
                    }
                    other => return Err(other),
                }
            }
            let length = u64::from(header.records)
                .checked_mul(u64::from(header.record_size))
                .ok_or_else(|| malformed("attribute size overflow"))?;
            let order = usize::from(field.order);
            self.charge_attributes(length)?;
            let header = self.header(reference)?;
            let bytes = self.reader.records(header, reference)?;
            let mut value = vset::field_values(header, &bytes, 0)?;
            // cdf.c:1485-1493 maps both character types to NC_CHAR and uses the first field order.
            if let Value::Char8(values) | Value::UChar8(values) = &mut value {
                values.truncate(order);
            }
            attributes.push(Attribute { name, value });
        }
        Ok(attributes)
    }

    fn charge_attributes(&mut self, length: u64) -> Result<()> {
        self.attribute_bytes = self
            .attribute_bytes
            .checked_add(length)
            .ok_or_else(|| malformed("total attribute size overflow"))?;
        dd::check_limit(
            "attribute_bytes",
            self.attribute_bytes,
            self.reader.limits().max_attribute_bytes,
        )
    }

    fn dimension_attributes(&mut self, datasets: &mut [Dataset]) -> Result<()> {
        let coordinates: BTreeMap<_, _> = datasets
            .iter()
            .enumerate()
            .filter(|(_, dataset)| dataset.is_coordinate)
            .map(|(index, dataset)| (dataset.name.clone(), index))
            .collect();
        for dataset_index in 0..datasets.len() {
            let dimensions = datasets
                .get(dataset_index)
                .ok_or_else(|| malformed("dataset index"))?
                .dims
                .len();
            for dimension_index in 0..dimensions {
                let dimension = datasets
                    .get(dataset_index)
                    .and_then(|dataset| dataset.dims.get(dimension_index))
                    .ok_or_else(|| malformed("dimension index"))?;
                let Some(&coordinate_index) = coordinates.get(&dimension.name) else {
                    continue;
                };
                let attributes = &datasets
                    .get(coordinate_index)
                    .ok_or_else(|| malformed("coordinate index"))?
                    .attributes;
                for attribute in attributes {
                    self.charge_attributes(
                        value_len(&attribute.value)?
                            .checked_mul(value_type(&attribute.value).size() as u64)
                            .ok_or_else(|| malformed("dimension attribute size overflow"))?,
                    )?;
                }
                let attributes = attributes.clone();
                datasets
                    .get_mut(dataset_index)
                    .and_then(|dataset| dataset.dims.get_mut(dimension_index))
                    .ok_or_else(|| malformed("dimension index"))?
                    .attributes = attributes;
            }
        }
        Ok(())
    }

    fn skip(&mut self, object: String, feature: &'static str) -> Error {
        self.skipped.push(Skipped {
            object: object.clone(),
            feature,
        });
        Error::Unsupported { feature, object }
    }

    fn eos(&mut self, attributes: &[Attribute], datasets: &[Dataset]) -> Result<Option<Eos>> {
        let mut parts = BTreeMap::new();
        for attribute in attributes {
            let Some(suffix) = attribute.name.strip_prefix("StructMetadata.") else {
                continue;
            };
            let index = suffix
                .parse::<u32>()
                .map_err(|_| malformed("invalid StructMetadata fragment number"))?;
            let Value::Char8(bytes) = &attribute.value else {
                return Err(malformed("StructMetadata must be char8"));
            };
            if parts.insert(index, bytes).is_some() {
                return Err(malformed("duplicate StructMetadata fragment"));
            }
        }
        if parts.is_empty() {
            return Ok(None);
        }
        let length = parts.values().try_fold(0_u64, |length, bytes| {
            length
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| malformed("StructMetadata size overflow"))
        })?;
        dd::check_limit(
            "attribute_bytes",
            length,
            self.reader.limits().max_attribute_bytes,
        )?;
        let mut text = String::with_capacity(allocation(
            length
                .checked_mul(2)
                .ok_or_else(|| malformed("StructMetadata text size overflow"))?,
            self.reader.limits(),
        )?);
        for (expected, (&index, bytes)) in parts.iter().enumerate() {
            if expected as u64 != u64::from(index) {
                return Err(malformed("missing StructMetadata fragment"));
            }
            text.extend(bytes.iter().map(|byte| char::from(*byte)));
        }
        check_odl_depth(&text, self.reader.limits().max_group_depth)?;
        let document = odl::parse(&text, self.reader.limits())?;
        check_eos_ranks(&document, self.reader.limits().max_rank)?;
        let (mut eos, skipped) = eos::from_struct_metadata(&text, self.reader.limits())?;
        for (objects, class) in [(&mut eos.grids, "GRID"), (&mut eos.swaths, "SWATH")] {
            for object in objects {
                restore_parameters(object, &document, class)?;
                self.link_fields(object, datasets, class)?;
            }
        }
        self.skipped.extend(skipped);
        Ok(Some(eos))
    }

    fn link_fields(
        &mut self,
        object: &mut EosObject,
        datasets: &[Dataset],
        class: &str,
    ) -> Result<()> {
        let group = self
            .groups
            .values()
            .find(|group| group.name == object.name && group.class == class);
        let Some(group) = group else {
            return Err(malformed("EOS object has no Vgroup"));
        };
        let count = if group.class == "GRID" { 1 } else { 2 };
        let groups: Vec<_> = group
            .members
            .iter()
            .filter(|(tag, _)| *tag == 1965)
            .take(count)
            .map(|(_, reference)| *reference)
            .collect();
        if groups.len() != count {
            return Err(malformed("EOS object is missing field groups"));
        }
        let mut links = BTreeMap::new();
        let mut vdata = BTreeSet::new();
        // GDapi.c:379 and SWapi.c:336: grid data child, or swath geolocation then data children.
        for (group_index, reference) in groups.into_iter().enumerate() {
            for &(tag, reference) in &self.group(reference)?.members {
                if tag == 720 {
                    let (index, dataset) = datasets
                        .iter()
                        .enumerate()
                        .find(|(_, dataset)| dataset.ndg_ref == reference)
                        .ok_or_else(|| malformed("EOS NDG has no dataset"))?;
                    links.insert(
                        (dataset.name.clone(), count == 2 && group_index == 0),
                        index,
                    );
                }
                if tag == 1962 {
                    vdata.insert(self.header(reference)?.name.clone());
                }
            }
        }
        for name in &vdata {
            self.skipped.push(Skipped {
                object: format!("{}/{name}", object.name),
                feature: "vdata-field",
            });
        }
        object.fields.retain(|field| !vdata.contains(&field.name));
        for field in &mut object.fields {
            field.dataset = links.get(&(field.name.clone(), field.geolocation)).copied();
        }
        Ok(())
    }
}

fn value_len(value: &Value) -> Result<u64> {
    let length = match value {
        Value::Char8(values) | Value::UChar8(values) | Value::U8(values) => values.len(),
        Value::I8(values) => values.len(),
        Value::I16(values) => values.len(),
        Value::U16(values) => values.len(),
        Value::I32(values) => values.len(),
        Value::U32(values) => values.len(),
        Value::F32(values) => values.len(),
        Value::F64(values) => values.len(),
    };
    u64::try_from(length).map_err(|_| malformed("value count overflow"))
}

fn restore_parameters(object: &mut EosObject, document: &Odl, class: &str) -> Result<()> {
    let name_key = match class {
        "GRID" => "GridName",
        "SWATH" => "SwathName",
        _ => return Err(malformed("invalid EOS object class")),
    };
    let mut stack = vec![document];
    while let Some(body) = stack.pop() {
        let matches = body.entries.iter().any(|entry| matches!(entry, Entry::Assignment { name, value: odl::Value::Scalar(value), .. } if name == name_key && value == &object.name));
        if matches {
            object.params = body
                .entries
                .iter()
                .filter_map(|entry| match entry {
                    Entry::Assignment { name, raw, .. } => Some((name.clone(), raw.clone())),
                    _ => None,
                })
                .collect();
            return Ok(());
        }
        for entry in body.entries.iter().rev() {
            if let Entry::Group { body, .. } | Entry::Object { body, .. } = entry {
                stack.push(body);
            }
        }
    }
    Err(malformed("EOS parameters have no object"))
}

fn check_eos_ranks(document: &Odl, max: u64) -> Result<()> {
    let mut stack = vec![document];
    while let Some(body) = stack.pop() {
        for entry in &body.entries {
            match entry {
                Entry::Assignment {
                    name,
                    value: odl::Value::List(dimensions),
                    ..
                } if name == "DimList" => {
                    dd::check_limit("rank", dimensions.len() as u64, max)?;
                }
                Entry::Group { body, .. } | Entry::Object { body, .. } => stack.push(body),
                _ => {}
            }
        }
    }
    Ok(())
}

fn check_odl_depth(text: &str, max: u64) -> Result<()> {
    let mut characters = text.chars().peekable();
    let mut depth = 0_u64;
    while let Some(character) = characters.next() {
        if character == '"' {
            skip_odl_string(&mut characters)?;
            continue;
        }
        if character == '/' && characters.peek() == Some(&'*') {
            characters.next();
            skip_odl_comment(&mut characters)?;
            continue;
        }
        if character.is_ascii_alphabetic() || character == '_' {
            let mut token = String::new();
            token.push(character);
            while characters
                .peek()
                .is_some_and(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
            {
                token.push(
                    characters
                        .next()
                        .ok_or_else(|| malformed("truncated ODL token"))?,
                );
            }
            while characters.peek().is_some_and(|ch| ch.is_whitespace()) {
                characters.next();
            }
            if characters.peek() != Some(&'=') {
                continue;
            }
            match token.as_str() {
                "GROUP" | "OBJECT" => {
                    depth = depth
                        .checked_add(1)
                        .ok_or_else(|| malformed("ODL depth overflow"))?;
                }
                "END_GROUP" | "END_OBJECT" => {
                    depth = depth
                        .checked_sub(1)
                        .ok_or_else(|| malformed("unbalanced ODL depth"))?;
                }
                _ => {}
            }
        } else if character == '(' {
            depth = depth
                .checked_add(1)
                .ok_or_else(|| malformed("ODL depth overflow"))?;
        } else if character == ')' {
            depth = depth
                .checked_sub(1)
                .ok_or_else(|| malformed("unbalanced ODL depth"))?;
        }
        dd::check_limit("group_depth", depth, max)?;
    }
    Ok(())
}

fn skip_odl_string(characters: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Result<()> {
    for character in characters.by_ref() {
        if character == '"' {
            return Ok(());
        }
    }
    Err(malformed("unterminated ODL string"))
}

fn skip_odl_comment(characters: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Result<()> {
    while let Some(character) = characters.next() {
        if character == '*' && characters.peek() == Some(&'/') {
            characters.next();
            return Ok(());
        }
    }
    Err(malformed("unterminated ODL comment"))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{ByteOrder, Codec, DType, Limits, Memory, Storage};

    #[test]
    fn sq6_odl_preflight_preserves_literal_backslashes() -> Result<()> {
        check_odl_depth(r#"GROUP=A Text="trailing\" END_GROUP=A END"#, 1)?;
        Ok(())
    }

    #[test]
    fn odl_depth_preflight_checks_nesting_not_strings_or_comments() -> Result<()> {
        check_odl_depth(
            r#"GROUP=A Text="GROUP=ignored ( \ OBJECT=ignored" /* GROUP=ignored ( */ END_GROUP=A END"#,
            1,
        )?;
        assert!(matches!(
            check_odl_depth("GROUP=A Values=(0) END_GROUP=A END", 1),
            Err(Error::LimitExceeded {
                value: 2,
                max: 1,
                ..
            })
        ));
        for text in [
            "Text=\"unterminated",
            "/* unterminated",
            "END_GROUP=A",
            "Values=)",
        ] {
            assert!(matches!(
                check_odl_depth(text, 64),
                Err(Error::Malformed { .. })
            ));
        }
        Ok(())
    }

    #[test]
    fn eos_grid_and_swath_names_have_separate_namespaces() -> Result<()> {
        let source = Memory::new(vec![14, 3, 19, 1, 0, 0, 0, 0, 0, 0]);
        let index = dd::parse(&source, &Limits::default())?;
        let reader = Reader {
            source: &source,
            index: &index,
        };
        let group = |class: &str, members| Vgroup {
            version: 3,
            name: "Shared".into(),
            class: class.into(),
            members,
            attributes: Vec::new(),
        };
        let mut catalog = Catalog {
            reader: &reader,
            groups: BTreeMap::from([
                (1, group("GRID", vec![(1965, 3)])),
                (2, group("SWATH", vec![(1965, 4), (1965, 5)])),
                (3, group("field group", vec![(720, 11)])),
                (4, group("field group", vec![(720, 12)])),
                (5, group("field group", vec![])),
            ]),
            headers: BTreeMap::new(),
            attribute_bytes: 0,
            skipped: Vec::new(),
        };
        let dataset = |name: &str, ndg_ref| Dataset {
            name: name.into(),
            dtype: DType::I16,
            shape: vec![],
            dims: vec![],
            attributes: vec![],
            is_coordinate: false,
            storage: Storage::Empty,
            codec: Codec::None,
            data: None,
            ndg_ref,
            order: ByteOrder::Big,
        };
        let datasets = [dataset("grid-field", 11), dataset("swath-field", 12)];
        let mut object = EosObject {
            name: "Shared".into(),
            dims: vec![],
            params: vec![],
            dimension_maps: vec![],
            fields: vec![crate::EosField {
                name: "swath-field".into(),
                dims: vec![],
                geolocation: true,
                dataset: None,
            }],
        };
        catalog.link_fields(&mut object, &datasets, "SWATH")?;
        assert_eq!(object.fields.first().expect("field").dataset, Some(1));
        let document = odl::parse(
            "GROUP=Grid GridName=\"Shared\" Kind=grid END_GROUP=Grid GROUP=Swath SwathName=\"Shared\" Kind=swath END_GROUP=Swath END",
            &Limits::default(),
        )?;
        restore_parameters(&mut object, &document, "SWATH")?;
        assert_eq!(
            object.params,
            vec![
                ("SwathName".into(), "\"Shared\"".into()),
                ("Kind".into(), "swath".into())
            ]
        );
        Ok(())
    }
}
