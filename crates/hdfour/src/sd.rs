//! Scientific dataset discovery and metadata.
#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::metadata::Budget;
use std::collections::{BTreeMap, BTreeSet, HashMap};

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
    skipped: Vec<Skipped>,
}

pub(crate) fn parse(reader: &Reader<'_>) -> Result<Metadata> {
    let mut catalog = Catalog {
        reader,
        groups: BTreeMap::new(),
        headers: BTreeMap::new(),
        skipped: Vec::new(),
    };
    for descriptor in reader
        .index
        .iter()
        .filter(|descriptor| descriptor.tag == 1965)
    {
        reader.index.parser_metadata().items::<(u16, Vgroup)>(1)?;
        let group = vset::parse_vgroup_with_budget(
            &catalog.metadata_bytes(descriptor)?,
            Some(reader.index.parser_metadata()),
        )?;
        catalog.groups.insert(descriptor.reference, group);
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
            catalog.skip(
                &[
                    if descriptor.tag == 720 {
                        "NDG "
                    } else {
                        "SDG "
                    },
                    &descriptor.reference.to_string(),
                ],
                "legacy-sd-layout",
            )?;
        }
        return Ok(Metadata {
            datasets: Vec::new(),
            attributes: Vec::new(),
            skipped: catalog.skipped,
            eos: None,
        });
    };
    let members = catalog.members(root)?;
    let mut datasets = Vec::new();
    let mut dimension_sizes = BTreeMap::new();
    for &(tag, reference) in &members {
        if tag != 1965 || catalog.group(reference)?.class != "Var0.0" {
            continue;
        }
        let budget = Budget::new(reader.limits().max_metadata_bytes);
        match catalog.dataset(reference, &budget, &mut dimension_sizes) {
            Ok(dataset) => {
                reader.index.metadata().charge(budget.used())?;
                reader.index.metadata().items::<Dataset>(1)?;
                datasets.push(dataset);
            }
            Err(Error::Unsupported {
                feature: "number-type" | "ragged-array" | "vdata-interlace",
                ..
            }) => {}
            Err(error) => return Err(error),
        }
    }
    let attributes = catalog.attributes(root, reader.index.metadata())?;
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
    fn budget(&self) -> &Budget {
        self.reader.index.metadata()
    }

    fn metadata_bytes(&self, descriptor: &crate::dd::Descriptor) -> Result<Vec<u8>> {
        self.reader.bytes(descriptor)
    }

    fn members(&self, reference: u16) -> Result<Vec<(u16, u16)>> {
        let members = &self.group(reference)?.members;
        let mut copied = crate::read::buffer(members.len())?;
        copied.copy_from_slice(members);
        Ok(copied)
    }

    fn group(&self, reference: u16) -> Result<&Vgroup> {
        self.groups
            .get(&reference)
            .ok_or_else(|| malformed("missing Vgroup"))
    }

    fn header(&mut self, reference: u16) -> Result<&VdataHeader> {
        if !self.headers.contains_key(&reference) {
            let descriptor = self.reader.descriptor((1962, reference))?;
            self.reader
                .index
                .parser_metadata()
                .items::<(u16, VdataHeader)>(1)?;
            let header = vset::parse_vdata_header_for_sd(
                &self.metadata_bytes(descriptor)?,
                Some(self.reader.index.parser_metadata()),
            )?;
            self.headers.insert(reference, header);
        }
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

    fn dataset(
        &mut self,
        reference: u16,
        budget: &Budget,
        dimension_sizes: &mut BTreeMap<u16, u64>,
    ) -> Result<Dataset> {
        let group = self.group(reference)?;
        let name = budget.text(&group.name)?;
        let members = self.members(reference)?;
        if members.iter().any(|(tag, _)| *tag == 781) {
            return Err(self.skip(&[&name], "ragged-array")?);
        }
        let number = members
            .iter()
            .find(|(tag, _)| *tag == 106)
            .copied()
            .ok_or_else(|| malformed("dataset has no number type"))?;
        let (dtype, order) = match nt::parse(&self.metadata_bytes(self.reader.descriptor(number)?)?)
        {
            Err(Error::Unsupported {
                feature: "number-type",
                ..
            }) => return Err(self.skip(&[&name], "number-type")?),
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
        budget.items::<Dimension>(dimension_refs.len())?;
        let mut dims = match dimension_refs
            .into_iter()
            .map(|reference| self.dimension(reference, budget, dimension_sizes))
            .collect::<Result<Vec<_>>>()
        {
            Err(Error::Unsupported { feature, .. }) => return Err(self.skip(&[&name], feature)?),
            result => result?,
        };
        let data = members.iter().find(|(tag, _)| *tag == 702).copied();
        let (storage, codec, length) = self.reader.layout_with_budget(data, budget)?;
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
        budget.items::<u64>(dims.len())?;
        let shape: Vec<_> = dims.iter().map(|dim| dim.size).collect();
        array_bytes(&shape, dtype.size(), self.reader.limits())?;
        let mut coordinate_status = None;
        for &(_, reference) in members.iter().filter(|(tag, _)| *tag == 1962) {
            // cdf.c:1674-1679 distinguishes explicit variable classes from older unmarked variables.
            match self.header(reference)?.class.as_str() {
                "SDSVar" => coordinate_status = Some(false),
                "CoordVar" => coordinate_status = Some(true),
                _ => {}
            }
        }
        // SQ7 B1 limits the mfsd.c:4205-4227 name fallback to rank-one variables.
        let is_coordinate = coordinate_status
            .unwrap_or_else(|| dims.len() == 1 && dims.first().is_some_and(|dim| dim.name == name));
        let ndg_ref = members
            .iter()
            .find(|(tag, _)| *tag == 720)
            .map(|(_, reference)| *reference)
            .ok_or_else(|| malformed("dataset has no NDG reference"))?;
        let attributes = self.attributes(reference, budget)?;
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

    fn dimension(
        &mut self,
        reference: u16,
        budget: &Budget,
        dimension_sizes: &mut BTreeMap<u16, u64>,
    ) -> Result<Dimension> {
        let group = self.group(reference)?;
        if !matches!(group.class.as_str(), "Dim0.0" | "UDim0.0") {
            return Err(malformed("non-dimension Vgroup in dataset"));
        }
        let unlimited = group.class == "UDim0.0";
        let name = budget.text(&group.name)?;
        let members = self.members(reference)?;
        let mut size = None;
        // cdf.c:1236-1275: DimVal0.0 uses the record count; DimVal0.1 stores one i32 size.
        for &(_, reference) in members.iter().filter(|(tag, _)| *tag == 1962) {
            let header = self.header(reference)?;
            match header.class.as_str() {
                "DimVal0.0" if !unlimited => {
                    if size.is_none() {
                        size = Some(u64::from(header.records));
                    }
                }
                "DimVal0.1" | "DimVal0.0" => {
                    size = Some(self.dimension_size(reference, dimension_sizes)?);
                }
                _ => {}
            }
        }
        let size = size.ok_or_else(|| malformed("dimension has no size"))?;
        Ok(Dimension {
            name,
            size: if unlimited { 0 } else { size },
            unlimited,
            attributes: Vec::new(),
        })
    }

    fn dimension_size(&mut self, reference: u16, sizes: &mut BTreeMap<u16, u64>) -> Result<u64> {
        if let Some(&size) = sizes.get(&reference) {
            return Ok(size);
        }
        let reader = self.reader;
        let header = self.header(reference)?;
        // An unsupported size type skips the dataset and the open goes on, so it is refused before
        // the record block is read: otherwise every dataset that shares the dimension reads it again.
        if let Some(field) = header.fields.first() {
            nt::from_vdata_type(field.type_code)?;
        }
        let bytes = reader.records(header, reference)?;
        let Value::I32(values) = vset::field_values(header, &bytes, 0)? else {
            return Err(malformed("dimension size must be i32"));
        };
        let value = *values
            .first()
            .ok_or_else(|| malformed("missing dimension size"))?;
        let size = u64::try_from(value).map_err(|_| malformed("negative dimension size"))?;
        reader.index.parser_metadata().items::<(u16, u64)>(1)?;
        sizes.insert(reference, size);
        Ok(size)
    }

    fn attributes(&mut self, reference: u16, budget: &Budget) -> Result<Vec<Attribute>> {
        let group = self.group(reference)?;
        let parent = if group.class == "CDF0.0" {
            String::new()
        } else {
            copy_text(&group.name)?
        };
        let members = self.members(reference)?;
        let mut attributes = Vec::new();
        let reader = self.reader;
        // cdf.c:1412-1493 reads Attr0.0 children, not the Vgroup API's separate v4 attributes.
        for (_, reference) in members.into_iter().filter(|(tag, _)| *tag == 1962) {
            let header = self.header(reference)?;
            if header.class != "Attr0.0" {
                continue;
            }
            let name = copy_text(&header.name)?;
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
                        self.skip(&[&parent, "/", &name], "number-type")?;
                        continue;
                    }
                    other => return Err(other),
                }
            }
            let length = u64::from(header.records)
                .checked_mul(u64::from(header.record_size))
                .ok_or_else(|| malformed("attribute size overflow"))?;
            let order = usize::from(field.order);
            dd::check_limit(
                "attribute_bytes",
                length,
                reader.limits().max_attribute_bytes,
            )?;
            let retained = u64::from(header.records)
                .checked_mul(u64::from(field.size))
                .ok_or_else(|| malformed("attribute value size overflow"))?;
            if header.interlace != 0 {
                self.skip(&[&parent, "/", &name], "vdata-interlace")?;
                continue;
            }
            // Truncating character values keeps their allocation, so charge the whole field.
            self.charge_attributes(retained, budget)?;
            budget.charge(name.len() as u64)?;
            let header = self.header(reference)?;
            let bytes = match reader.records(header, reference) {
                Err(Error::Unsupported { feature, .. }) => {
                    self.skip(&[&parent, "/", &name], feature)?;
                    continue;
                }
                result => result?,
            };
            budget.items::<Attribute>(1)?;
            let mut value = vset::field_values(header, &bytes, 0)?;
            // cdf.c:1485-1493 maps both character types to NC_CHAR and uses the first field order.
            if let Value::Char8(values) | Value::UChar8(values) = &mut value {
                values.truncate(order);
            }
            attributes.push(Attribute { name, value });
        }
        Ok(attributes)
    }

    fn charge_attributes(&self, length: u64, budget: &Budget) -> Result<()> {
        dd::check_limit(
            "attribute_bytes",
            length,
            self.reader.limits().max_attribute_bytes,
        )?;
        budget.charge(length)
    }

    fn dimension_attributes(&mut self, datasets: &mut [Dataset]) -> Result<()> {
        let coordinates: BTreeMap<_, _> = datasets
            .iter()
            .enumerate()
            .filter(|(_, dataset)| dataset.is_coordinate)
            .map(|(index, dataset)| Ok((copy_text(&dataset.name)?, index)))
            .collect::<Result<_>>()?;
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
                self.budget().items::<Attribute>(attributes.len())?;
                for attribute in attributes {
                    self.budget().charge(attribute.name.len() as u64)?;
                    self.charge_attributes(
                        value_len(&attribute.value)?
                            .checked_mul(value_type(&attribute.value).size() as u64)
                            .ok_or_else(|| malformed("dimension attribute size overflow"))?,
                        self.reader.index.metadata(),
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

    fn skip(&mut self, parts: &[&str], feature: &'static str) -> Result<Error> {
        let length = parts.iter().try_fold(0_u64, |length, part| {
            length
                .checked_add(part.len() as u64)
                .ok_or_else(|| malformed("refusal path size overflow"))
        })?;
        self.budget().charge(length)?;
        self.budget().items::<Skipped>(1)?;
        let object = parts.concat();
        let error_object = copy_text(&object)?;
        self.skipped.push(Skipped { object, feature });
        Ok(Error::Unsupported {
            feature,
            object: error_object,
        })
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
        let text_length = parts.values().try_fold(0_u64, |length, bytes| {
            bytes.iter().try_fold(length, |length, byte| {
                length
                    .checked_add(char::from(*byte).len_utf8() as u64)
                    .ok_or_else(|| malformed("StructMetadata text size overflow"))
            })
        })?;
        let mut text = String::new();
        text.try_reserve_exact(allocation(text_length, self.reader.limits())?)
            .map_err(|_| crate::read::allocation_error(text_length))?;
        for (expected, (&index, bytes)) in parts.iter().enumerate() {
            if expected as u64 != u64::from(index) {
                return Err(malformed("missing StructMetadata fragment"));
            }
            text.extend(bytes.iter().map(|byte| char::from(*byte)));
        }
        check_odl_depth(&text, self.reader.limits().max_group_depth)?;
        let document = odl::parse(&text, self.reader.limits())?;
        check_eos_ranks(&document, self.reader.limits().max_rank)?;
        let staged = Budget::new(self.reader.limits().max_metadata_bytes);
        let (mut eos, skipped) = eos::from_document(&document, self.reader.limits(), &staged)?;
        let scratch = Budget::new(self.reader.limits().max_metadata_bytes);
        let mut links = EosLinks::new(&self.groups, datasets, &scratch)?;
        for (objects, class) in [(&mut eos.grids, "GRID"), (&mut eos.swaths, "SWATH")] {
            for object in objects {
                self.link_indexed_fields(object, &mut links, class)?;
            }
        }
        eos::charge_retained(&eos, &skipped, self.budget())?;
        self.skipped.extend(skipped);
        Ok(Some(eos))
    }

    #[cfg(test)]
    fn link_fields(
        &mut self,
        object: &mut EosObject,
        datasets: &[Dataset],
        class: &str,
    ) -> Result<()> {
        let mut links = EosLinks::new(&self.groups, datasets, self.budget())?;
        self.link_indexed_fields(object, &mut links, class)
    }

    fn link_indexed_fields(
        &mut self,
        object: &mut EosObject,
        links: &mut EosLinks<'_>,
        class: &str,
    ) -> Result<()> {
        let reference = links
            .groups
            .get(class)
            .and_then(|names| names.get(object.name.as_str()))
            .ok_or_else(|| malformed("EOS object has no Vgroup"))?;
        let groups = links
            .field_groups
            .get(reference)
            .ok_or_else(|| malformed("EOS field group index"))?;
        let count = if class == "GRID" { 1 } else { 2 };
        if groups.len() != count {
            return Err(malformed("EOS object is missing field groups"));
        }
        for reference in groups {
            if !links.fields.contains_key(reference) {
                let fields = self.field_links(*reference, &links.datasets)?;
                self.reader
                    .index
                    .parser_metadata()
                    .items::<(u16, FieldLinks<'_>)>(1)?;
                links.fields.insert(*reference, fields);
            }
        }
        let mut vdata = BTreeSet::new();
        for reference in groups {
            let fields = links
                .fields
                .get(reference)
                .ok_or_else(|| malformed("EOS field index"))?;
            for name in &fields.vdata {
                vdata.insert(name.as_str());
            }
        }
        for name in &vdata {
            self.skip(&[&object.name, "/", name], "vdata-field")?;
        }
        object
            .fields
            .retain(|field| !vdata.contains(field.name.as_str()));
        // GDapi.c:379 and SWapi.c:336 select grid data, or swath geolocation then data.
        for (group_index, reference) in groups.iter().enumerate() {
            let fields = links
                .fields
                .get(reference)
                .ok_or_else(|| malformed("EOS field index"))?;
            for field in &mut object.fields {
                if field.geolocation == (count == 2 && group_index == 0) {
                    field.dataset = fields.datasets.get(field.name.as_str()).copied();
                }
            }
        }
        Ok(())
    }

    fn field_links<'a>(
        &mut self,
        reference: u16,
        datasets: &HashMap<u16, Option<(usize, &'a Dataset)>>,
    ) -> Result<FieldLinks<'a>> {
        let mut links = FieldLinks {
            datasets: HashMap::new(),
            vdata: BTreeSet::new(),
        };
        for (tag, reference) in self.members(reference)? {
            if tag == 720 {
                let Some(&(index, dataset)) = datasets
                    .get(&reference)
                    .ok_or_else(|| malformed("EOS NDG has no dataset"))?
                    .as_ref()
                else {
                    continue;
                };
                if !links.datasets.contains_key(dataset.name.as_str()) {
                    self.reader
                        .index
                        .parser_metadata()
                        .items::<(&str, usize)>(1)?;
                }
                links.datasets.insert(dataset.name.as_str(), index);
            }
            if tag == 1962 {
                let budget = self.reader.index.parser_metadata();
                let name = &self.header(reference)?.name;
                if !links.vdata.contains(name) {
                    budget.items::<String>(1)?;
                    links.vdata.insert(budget.text(name)?);
                }
            }
        }
        Ok(links)
    }
}

fn copy_text(value: &str) -> Result<String> {
    crate::read::allocation_size(value.len() as u64)?;
    let mut copied = String::new();
    copied
        .try_reserve_exact(value.len())
        .map_err(|_| crate::read::allocation_error(value.len() as u64))?;
    copied.push_str(value);
    Ok(copied)
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

struct FieldLinks<'a> {
    datasets: HashMap<&'a str, usize>,
    vdata: BTreeSet<String>,
}

struct EosLinks<'a> {
    groups: HashMap<String, HashMap<String, u16>>,
    field_groups: HashMap<u16, Vec<u16>>,
    fields: HashMap<u16, FieldLinks<'a>>,
    datasets: HashMap<u16, Option<(usize, &'a Dataset)>>,
}

impl<'a> EosLinks<'a> {
    fn new(
        groups: &BTreeMap<u16, Vgroup>,
        datasets: &'a [Dataset],
        budget: &Budget,
    ) -> Result<Self> {
        let mut indexed = HashMap::<String, HashMap<String, u16>>::new();
        let mut field_groups = HashMap::new();
        for (&reference, group) in groups {
            #[cfg(test)]
            EOS_PROBES.with(|count| count.set(count.get().saturating_add(1)));
            if !matches!(group.class.as_str(), "GRID" | "SWATH") {
                continue;
            }
            if !indexed.contains_key(&group.class) {
                budget.items::<(String, HashMap<String, u16>)>(1)?;
                indexed.insert(budget.text(&group.class)?, HashMap::new());
            }
            let names = indexed
                .get_mut(&group.class)
                .ok_or_else(|| malformed("EOS class index"))?;
            if !names.contains_key(&group.name) {
                budget.items::<(String, u16)>(1)?;
                names.insert(budget.text(&group.name)?, reference);
                let count = if group.class == "GRID" { 1 } else { 2 };
                budget.items::<u16>(count)?;
                budget.items::<(u16, Vec<u16>)>(1)?;
                let children = group
                    .members
                    .iter()
                    .filter(|(tag, _)| {
                        #[cfg(test)]
                        EOS_MEMBER_VISITS.with(|count| count.set(count.get().saturating_add(1)));
                        *tag == 1965
                    })
                    .take(count)
                    .map(|(_, reference)| *reference)
                    .collect();
                field_groups.insert(reference, children);
            }
        }
        budget.items::<(u16, Option<(usize, &Dataset)>)>(datasets.len())?;
        let mut by_reference = HashMap::new();
        for (index, dataset) in datasets.iter().enumerate() {
            by_reference
                .entry(dataset.ndg_ref)
                .or_insert(Some((index, dataset)));
        }
        // Root variables missing from the dataset list were explicitly skipped during
        // SD parsing. Keep their EOS fields unlinked instead of rejecting the whole file.
        let root = groups.values().find(|group| group.class == "CDF0.0");
        for (_, reference) in root
            .into_iter()
            .flat_map(|group| &group.members)
            .filter(|(tag, _)| *tag == 1965)
        {
            let Some(group) = groups
                .get(reference)
                .filter(|group| group.class == "Var0.0")
            else {
                continue;
            };
            let Some((_, reference)) = group.members.iter().find(|(tag, _)| *tag == 720) else {
                continue;
            };
            if let std::collections::hash_map::Entry::Vacant(entry) = by_reference.entry(*reference)
            {
                budget.items::<(u16, Option<(usize, &Dataset)>)>(1)?;
                entry.insert(None);
            }
        }
        Ok(Self {
            groups: indexed,
            datasets: by_reference,
            field_groups,
            fields: HashMap::new(),
        })
    }
}

#[cfg(test)]
type Parameters<'a> = HashMap<(&'static str, &'a str), &'a Odl>;

#[cfg(test)]
fn parameter_index<'a>(document: &'a Odl, budget: &Budget) -> Result<Parameters<'a>> {
    let mut parameters = HashMap::new();
    let mut stack = vec![document];
    while let Some(body) = stack.pop() {
        #[cfg(test)]
        EOS_PROBES.with(|count| count.set(count.get().saturating_add(1)));
        for entry in &body.entries {
            if let Entry::Assignment {
                name,
                value: odl::Value::Scalar(value),
                ..
            } = entry
            {
                let class = match name.as_str() {
                    "GridName" => "GRID",
                    "SwathName" => "SWATH",
                    _ => continue,
                };
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    parameters.entry((class, value.as_str()))
                {
                    budget.items::<((&str, &str), &Odl)>(1)?;
                    entry.insert(body);
                }
            }
        }
        for entry in body.entries.iter().rev() {
            if let Entry::Group { body, .. } | Entry::Object { body, .. } = entry {
                stack.push(body);
            }
        }
    }
    Ok(parameters)
}

#[cfg(test)]
fn restore_parameters(object: &mut EosObject, document: &Odl, class: &str) -> Result<()> {
    let budget = Budget::new(u64::MAX);
    restore_indexed_parameters(object, &parameter_index(document, &budget)?, class, &budget)
}

#[cfg(test)]
fn restore_indexed_parameters(
    object: &mut EosObject,
    parameters: &Parameters<'_>,
    class: &str,
    budget: &Budget,
) -> Result<()> {
    let body = parameters
        .get(&(class, object.name.as_str()))
        .ok_or_else(|| malformed("EOS parameters have no object"))?;
    object.params = body
        .entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Assignment { name, raw, .. } => Some((name, raw)),
            _ => None,
        })
        .map(|(name, raw)| {
            budget.items::<(String, String)>(1)?;
            Ok((budget.text(name)?, budget.text(raw)?))
        })
        .collect::<Result<_>>()?;
    Ok(())
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

#[cfg(test)]
std::thread_local! { static EOS_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[cfg(test)]
#[allow(clippy::expect_used, clippy::arithmetic_side_effects)]
mod sq7_tests {
    use super::*;
    use crate::{Limits, Memory};

    #[test]
    fn sq10_eos_parameters_are_charged_once() -> Result<()> {
        let source = Memory::new(vec![14, 3, 19, 1, 0, 0, 0, 0, 0, 0]);
        let index = dd::parse(&source, &Limits::default())?;
        let reader = Reader {
            source: &source,
            index: &index,
        };
        let groups = BTreeMap::from([
            (
                1,
                Vgroup {
                    version: 3,
                    name: "G".into(),
                    class: "GRID".into(),
                    members: vec![(1965, 2)],
                    attributes: vec![],
                },
            ),
            (
                2,
                Vgroup {
                    version: 3,
                    name: String::new(),
                    class: String::new(),
                    members: vec![(1962, 3)],
                    attributes: vec![],
                },
            ),
        ]);
        let mut catalog = Catalog {
            reader: &reader,
            groups,
            headers: BTreeMap::from([(
                3,
                VdataHeader {
                    version: 3,
                    name: "F".into(),
                    class: "Field".into(),
                    interlace: 0,
                    records: 0,
                    record_size: 0,
                    fields: vec![],
                    attributes: vec![],
                },
            )]),
            skipped: vec![],
        };
        let attributes = [Attribute { name: "StructMetadata.0".into(), value: Value::Char8(
            b"GROUP=GridStructure GROUP=G GridName=G XDim=1 YDim=1 Projection=GEO GROUP=DataField OBJECT=F DataFieldName=F DimList=(XDim,YDim) END_OBJECT=F END_GROUP=DataField END_GROUP=G END_GROUP=GridStructure END".to_vec()
        ) }];
        let before = index.metadata().used();
        let eos = catalog.eos(&attributes, &[])?.expect("EOS");
        let object = eos.grids.first().expect("grid");
        assert_eq!(
            object.params,
            vec![
                ("GridName".into(), "G".into()),
                ("XDim".into(), "1".into()),
                ("YDim".into(), "1".into()),
                ("Projection".into(), "GEO".into()),
            ]
        );
        assert!(object.fields.is_empty());
        assert_eq!(
            catalog.skipped,
            vec![Skipped {
                object: "G/F".into(),
                feature: "vdata-field"
            }]
        );
        let retained = eos.grids.capacity() * std::mem::size_of::<EosObject>()
            + object.name.len()
            + object.dims.capacity() * std::mem::size_of::<(String, u64)>()
            + object
                .dims
                .iter()
                .map(|(name, _)| name.len())
                .sum::<usize>()
            + object.params.capacity() * std::mem::size_of::<(String, String)>()
            + object
                .params
                .iter()
                .map(|(name, raw)| name.len() + raw.len())
                .sum::<usize>()
            + object.fields.capacity() * std::mem::size_of::<crate::EosField>()
            + std::mem::size_of::<Skipped>()
            + 3;
        assert_eq!(index.metadata().used() - before, retained as u64);
        Ok(())
    }

    fn eos_work() -> Result<(usize, usize)> {
        let source = Memory::new(vec![14, 3, 19, 1, 0, 0, 0, 0, 0, 0]);
        let index = dd::parse(&source, &Limits::default())?;
        let reader = Reader {
            source: &source,
            index: &index,
        };
        let mut catalog = Catalog {
            reader: &reader,
            groups: BTreeMap::new(),
            headers: BTreeMap::new(),
            skipped: Vec::new(),
        };
        let mut text = String::from("GROUP=GridStructure ");
        for i in 0..32_u16 {
            let name = format!("G{i}");
            text.push_str(&format!(
                "GROUP={name} GridName={name} XDim=1 YDim=1 END_GROUP={name} "
            ));
            catalog.groups.insert(
                i * 2,
                Vgroup {
                    version: 3,
                    name,
                    class: "GRID".into(),
                    members: vec![(1965, i * 2 + 1)],
                    attributes: vec![],
                },
            );
            catalog.groups.insert(
                i * 2 + 1,
                Vgroup {
                    version: 3,
                    name: String::new(),
                    class: String::new(),
                    members: vec![],
                    attributes: vec![],
                },
            );
        }
        text.push_str("END_GROUP=GridStructure END");
        let attributes = [Attribute {
            name: "StructMetadata.0".into(),
            value: Value::Char8(text.into_bytes()),
        }];
        odl::PARSE_CALLS.with(|count| count.set(0));
        EOS_PROBES.with(|count| count.set(0));
        let eos = catalog.eos(&attributes, &[])?.expect("EOS");
        assert_eq!(eos.grids.len(), 32);
        Ok((
            odl::PARSE_CALLS.with(std::cell::Cell::get),
            EOS_PROBES.with(std::cell::Cell::get),
        ))
    }

    #[test]
    fn sq7_repeated_eos_objects_scan_each_parent_once() -> Result<()> {
        let source = Memory::new(vec![14, 3, 19, 1, 0, 0, 0, 0, 0, 0]);
        let index = dd::parse(&source, &Limits::default())?;
        let reader = Reader {
            source: &source,
            index: &index,
        };
        let mut members = vec![(900, 0); 128];
        members.push((1965, 2));
        let groups = BTreeMap::from([
            (
                1,
                Vgroup {
                    version: 3,
                    name: "G".into(),
                    class: "GRID".into(),
                    members,
                    attributes: vec![],
                },
            ),
            (
                2,
                Vgroup {
                    version: 3,
                    name: String::new(),
                    class: String::new(),
                    members: vec![],
                    attributes: vec![],
                },
            ),
        ]);
        let mut catalog = Catalog {
            reader: &reader,
            groups,
            headers: BTreeMap::new(),
            skipped: vec![],
        };
        let text = format!(
            "GROUP=GridStructure {} END_GROUP=GridStructure END",
            "GROUP=G GridName=G XDim=1 YDim=1 END_GROUP=G ".repeat(32)
        );
        let attributes = [Attribute {
            name: "StructMetadata.0".into(),
            value: Value::Char8(text.into_bytes()),
        }];
        EOS_MEMBER_VISITS.with(|count| count.set(0));
        let eos = catalog.eos(&attributes, &[])?.expect("EOS");
        assert_eq!(eos.grids.len(), 32);
        let visits = EOS_MEMBER_VISITS.with(std::cell::Cell::get);
        assert_eq!(visits, 129, "parent member comparisons");
        Ok(())
    }

    #[test]
    fn sq7_eos_parses_odl_once() -> Result<()> {
        let (parses, _) = eos_work()?;
        assert_eq!(parses, 1, "ODL parse calls");
        Ok(())
    }

    #[test]
    fn sq7_eos_parameter_lookup_has_linear_visit_count() -> Result<()> {
        let (_, visits) = eos_work()?;
        assert!(visits <= 32 * 4, "ODL object visits: {visits}");
        Ok(())
    }
}

#[cfg(test)]
std::thread_local! { static EOS_MEMBER_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
