//! HDF-EOS2 grid and swath metadata mapping.
#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use crate::metadata::Budget;
use std::collections::BTreeSet;

use crate::odl::{self, Entry, Odl, Value};
use crate::{DimensionMap, Eos, EosField, EosObject, Error, Limits, Result, Skipped};

/// Map joined StructMetadata text to EOS objects and explicit feature refusals.
/// Field dataset indices stay unset until the SD reader resolves Vgroup links.
pub fn from_struct_metadata(text: &str, limits: &Limits) -> Result<(Eos, Vec<Skipped>)> {
    let budget = Budget::new(limits.max_metadata_bytes);
    let document = odl::parse(text, limits)?;
    map_document(&document, limits, &budget, parameters)
}

pub(crate) fn from_document(
    document: &Odl,
    limits: &Limits,
    budget: &Budget,
) -> Result<(Eos, Vec<Skipped>)> {
    map_document(document, limits, budget, all_parameters)
}

pub(crate) fn charge_retained(eos: &Eos, skipped: &[Skipped], budget: &Budget) -> Result<()> {
    budget.items::<EosObject>(eos.grids.capacity())?;
    budget.items::<EosObject>(eos.swaths.capacity())?;
    for object in eos.grids.iter().chain(&eos.swaths) {
        budget.charge(object.name.capacity() as u64)?;
        budget.items::<(String, u64)>(object.dims.capacity())?;
        for (name, _) in &object.dims {
            budget.charge(name.capacity() as u64)?;
        }
        budget.items::<EosField>(object.fields.capacity())?;
        for field in &object.fields {
            budget.charge(field.name.capacity() as u64)?;
            budget.items::<String>(field.dims.capacity())?;
            for name in &field.dims {
                budget.charge(name.capacity() as u64)?;
            }
        }
        budget.items::<DimensionMap>(object.dimension_maps.capacity())?;
        for map in &object.dimension_maps {
            budget.charge(map.geo_dim.capacity() as u64)?;
            budget.charge(map.data_dim.capacity() as u64)?;
        }
        budget.items::<(String, String)>(object.params.capacity())?;
        for (name, raw) in &object.params {
            budget.charge(name.capacity() as u64)?;
            budget.charge(raw.capacity() as u64)?;
        }
    }
    budget.items::<Skipped>(skipped.len())?;
    for entry in skipped {
        budget.charge(entry.object.capacity() as u64)?;
    }
    Ok(())
}

fn map_document(
    document: &Odl,
    limits: &Limits,
    budget: &Budget,
    parameters: ParameterReader,
) -> Result<(Eos, Vec<Skipped>)> {
    let mut eos = Eos {
        grids: Vec::new(),
        swaths: Vec::new(),
    };
    let mut skipped = Refusals {
        entries: Vec::new(),
        parameters,
        budget,
        limits,
    };
    for grid in objects(document, "GridStructure") {
        eos.grids.push(grid_object(grid, &mut skipped)?);
    }
    for swath in objects(document, "SwathStructure") {
        eos.swaths.push(swath_object(swath, &mut skipped)?);
    }
    for point in objects(document, "PointStructure") {
        skipped.push(&[scalar(point, "PointName")?], "eos-point")?;
    }
    Ok((eos, skipped.entries))
}

struct Refusals<'a> {
    entries: Vec<Skipped>,
    parameters: ParameterReader,
    budget: &'a Budget,
    limits: &'a Limits,
}

impl Refusals<'_> {
    fn push(&mut self, parts: &[&str], feature: &'static str) -> Result<()> {
        let length = parts.iter().try_fold(0_u64, |total, part| {
            total
                .checked_add(part.len() as u64)
                .ok_or_else(|| malformed("EOS refusal path size overflow"))
        })?;
        self.budget.charge(length)?;
        self.budget.items::<Skipped>(1)?;
        self.entries.push(Skipped {
            object: parts.concat(),
            feature,
        });
        Ok(())
    }
}

fn malformed(what: &'static str) -> Error {
    Error::Malformed { offset: 0, what }
}

fn group<'a>(body: &'a Odl, name: &str) -> Option<&'a Odl> {
    body.entries.iter().find_map(|entry| match entry {
        Entry::Group { name: key, body } if key == name => Some(body),
        _ => None,
    })
}

fn objects<'a>(body: &'a Odl, name: &str) -> impl Iterator<Item = &'a Odl> {
    group(body, name)
        .into_iter()
        .flat_map(|group| &group.entries)
        .filter_map(|entry| match entry {
            Entry::Group { body, .. } | Entry::Object { body, .. } => Some(body),
            Entry::Assignment { .. } => None,
        })
}

fn value<'a>(body: &'a Odl, name: &str) -> Result<&'a Value> {
    body.entries
        .iter()
        .find_map(|entry| match entry {
            Entry::Assignment {
                name: key, value, ..
            } if key == name => Some(value),
            _ => None,
        })
        .ok_or_else(|| malformed("missing EOS metadata entry"))
}

fn scalar<'a>(body: &'a Odl, name: &str) -> Result<&'a str> {
    match value(body, name)? {
        Value::Scalar(text) => Ok(text),
        Value::List(_) => Err(malformed("expected EOS scalar")),
    }
}

fn list<'a>(body: &'a Odl, name: &str) -> Result<&'a [Value]> {
    let Value::List(items) = value(body, name)? else {
        return Err(malformed("expected EOS list"));
    };
    Ok(items)
}

fn list_scalar(item: &Value) -> Result<&str> {
    match item {
        Value::Scalar(text) => Ok(text),
        Value::List(_) => Err(malformed("expected EOS scalar list item")),
    }
}

fn size(body: &Odl, name: &str) -> Result<u64> {
    scalar(body, name)?
        .parse()
        .map_err(|_| malformed("invalid EOS dimension size"))
}

fn integer(body: &Odl, name: &str) -> Result<i64> {
    scalar(body, name)?
        .parse()
        .map_err(|_| malformed("invalid EOS dimension mapping integer"))
}

fn dimensions(body: &Odl, budget: &Budget) -> Result<Vec<(String, u64)>> {
    // EHapi.c (GDAL 1.11.5):1902-1906 writes DimensionName and Size in each object.
    objects(body, "Dimension")
        .map(|dimension| {
            budget.items::<(String, u64)>(1)?;
            Ok((
                budget.text(scalar(dimension, "DimensionName")?)?,
                size(dimension, "Size")?,
            ))
        })
        .collect()
}

type ParameterReader = fn(&Odl, &Budget) -> Result<Vec<(String, String)>>;

fn all_parameters(body: &Odl, budget: &Budget) -> Result<Vec<(String, String)>> {
    body.entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Assignment { name, raw, .. } => Some((name, raw)),
            _ => None,
        })
        .map(|(name, raw)| {
            budget.items::<(String, String)>(1)?;
            Ok((budget.text(name)?, budget.text(raw)?))
        })
        .collect()
}

fn parameters(body: &Odl, budget: &Budget) -> Result<Vec<(String, String)>> {
    body.entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Assignment { name, raw, .. }
                if !matches!(name.as_str(), "GridName" | "SwathName" | "XDim" | "YDim") =>
            {
                Some((name, raw))
            }
            _ => None,
        })
        .map(|(name, raw)| {
            budget.items::<(String, String)>(1)?;
            Ok((budget.text(name)?, budget.text(raw)?))
        })
        .collect()
}

fn merged_fields<'a>(
    body: &'a Odl,
    object: &str,
    skipped: &mut Refusals<'_>,
) -> Result<BTreeSet<&'a str>> {
    let mut merged = BTreeSet::new();
    // EHapi.c (GDAL 1.11.5):2141-2145 stores the constituent names in FieldList.
    for fields in objects(body, "MergedFields") {
        scalar(fields, "MergedFieldName")?;
        for item in list(fields, "FieldList")? {
            let name = list_scalar(item)?;
            if merged.contains(name) {
                continue;
            }
            skipped.push(&[object, "/", name], "merged-field")?;
            merged.insert(name);
        }
    }
    Ok(merged)
}

fn fields(
    body: &Odl,
    section: &str,
    merged: &BTreeSet<&str>,
    skipped: &Refusals<'_>,
) -> Result<Vec<EosField>> {
    let name_key = match section {
        "GeoField" => "GeoFieldName",
        "DataField" => "DataFieldName",
        _ => return Err(malformed("invalid EOS field section")),
    };
    let mut fields = Vec::new();
    // EHapi.c (GDAL 1.11.5):2024-2028,2086-2090 writes field names and DimList.
    for field in objects(body, section) {
        let name = scalar(field, name_key)?;
        if merged.contains(name) {
            continue;
        }
        let items = list(field, "DimList")?;
        crate::dd::check_limit("rank", items.len() as u64, skipped.limits.max_rank)?;
        let dims = items
            .iter()
            .map(|item| {
                skipped.budget.items::<String>(1)?;
                skipped.budget.text(list_scalar(item)?)
            })
            .collect::<Result<_>>()?;
        skipped.budget.items::<EosField>(1)?;
        fields.push(EosField {
            name: skipped.budget.text(name)?,
            dims,
            geolocation: section == "GeoField",
            dataset: None,
        });
    }
    Ok(fields)
}

fn grid_object(body: &Odl, skipped: &mut Refusals<'_>) -> Result<EosObject> {
    let name = skipped.budget.text(scalar(body, "GridName")?)?;
    skipped.budget.items::<EosObject>(1)?;
    skipped.budget.items::<(String, u64)>(2)?;
    // GDapi.c (GDAL 1.11.5):467-471 declares XDim and YDim outside Dimension.
    let mut dims = vec![
        (skipped.budget.text("XDim")?, size(body, "XDim")?),
        (skipped.budget.text("YDim")?, size(body, "YDim")?),
    ];
    dims.extend(dimensions(body, skipped.budget)?);
    let merged = merged_fields(body, &name, skipped)?;
    Ok(EosObject {
        name,
        dims,
        fields: fields(body, "DataField", &merged, skipped)?,
        dimension_maps: Vec::new(),
        params: (skipped.parameters)(body, skipped.budget)?,
    })
}

fn swath_object(body: &Odl, skipped: &mut Refusals<'_>) -> Result<EosObject> {
    let name = skipped.budget.text(scalar(body, "SwathName")?)?;
    skipped.budget.items::<EosObject>(1)?;
    let merged = merged_fields(body, &name, skipped)?;
    let mut swath_fields = fields(body, "GeoField", &merged, skipped)?;
    swath_fields.extend(fields(body, "DataField", &merged, skipped)?);
    // EHapi.c (GDAL 1.11.5):1938-1944 writes the four linear map entries.
    let dimension_maps = objects(body, "DimensionMap")
        .map(|map| {
            skipped.budget.items::<DimensionMap>(1)?;
            Ok(DimensionMap {
                geo_dim: skipped.budget.text(scalar(map, "GeoDimension")?)?,
                data_dim: skipped.budget.text(scalar(map, "DataDimension")?)?,
                offset: integer(map, "Offset")?,
                increment: integer(map, "Increment")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    // SWapi.c:2152-2160 distinguishes index maps from linear DimensionMap entries.
    for map in objects(body, "IndexDimensionMap") {
        skipped.push(
            &[
                &name,
                "/",
                scalar(map, "GeoDimension")?,
                "/",
                scalar(map, "DataDimension")?,
            ],
            "index-dimension-map",
        )?;
    }
    Ok(EosObject {
        name,
        dims: dimensions(body, skipped.budget)?,
        fields: swath_fields,
        dimension_maps,
        params: (skipped.parameters)(body, skipped.budget)?,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use super::*;
    const GRID: &str = include_str!("../../../fixtures/odl/grid.odl");
    const SWATH: &str = include_str!("../../../fixtures/odl/swath.odl");

    #[test]
    #[allow(clippy::arithmetic_side_effects)]
    fn sq6_refusal_paths_obey_cumulative_byte_budget() {
        let name = "é".repeat(16);
        for feature in ["index-dimension-map", "merged-field", "eos-point"] {
            let parts = match feature {
                "index-dimension-map" => vec![name.as_str(), "/", "x", "/", "y"],
                "merged-field" => vec![name.as_str(), "/", "x"],
                _ => vec![name.as_str()],
            };
            let path_bytes = parts.iter().map(|part| part.len() as u64).sum::<u64>();
            let entry_bytes = path_bytes + std::mem::size_of::<Skipped>() as u64;
            let max = entry_bytes * 2;
            let limits = Limits {
                max_metadata_bytes: max,
                ..Limits::default()
            };
            let budget = Budget::new(max);
            let mut skipped = Refusals {
                entries: Vec::new(),
                parameters,
                budget: &budget,
                limits: &limits,
            };
            skipped.push(&parts, feature).expect("first path");
            skipped.push(&parts, feature).expect("exact budget");
            assert_eq!(skipped.entries.len(), 2);
            assert!(skipped.entries.iter().all(|entry| entry.feature == feature));
            assert!(matches!(
                skipped.push(&parts, feature),
                Err(Error::LimitExceeded { limit: "metadata_bytes", value, max: ceiling })
                    if value == max + path_bytes && ceiling == max
            ));
        }
    }

    #[test]
    #[allow(clippy::arithmetic_side_effects)]
    fn sq6_refusal_budget_is_shared_across_eos_objects() {
        let text = SWATH.replace(
            "GROUP=PointStructure\nEND_GROUP=PointStructure",
            "GROUP=PointStructure OBJECT=P PointName=P END_OBJECT=P END_GROUP=PointStructure",
        );
        let limits = Limits::default();
        let document = odl::parse(&text, &limits).expect("ODL");
        let budget = Budget::new(limits.max_metadata_bytes);
        let (_, skipped) = from_document(&document, &limits, &budget).expect("refusals");
        assert!(skipped.iter().any(|entry| entry.feature == "eos-point"));
        let text_bytes = [
            "Radiance",
            "GeoTrack",
            "DataTrack",
            "CrossTrack",
            "Latitude",
            "Radiance",
            "GeoTrack",
            "CrossTrack",
            "DataTrack",
            "CrossTrack",
            "GeoTrack",
            "DataTrack",
            "SwathName",
            "\"Radiance\"",
            "Radiance/Pressure",
            "Radiance/Humidity",
            "Radiance/CrossTrack/DataTrack",
            "P",
        ]
        .iter()
        .map(|text| text.len())
        .sum::<usize>();
        let total = (text_bytes
            + std::mem::size_of::<EosObject>()
            + 3 * std::mem::size_of::<(String, u64)>()
            + 2 * std::mem::size_of::<EosField>()
            + 4 * std::mem::size_of::<String>()
            + std::mem::size_of::<DimensionMap>()
            + std::mem::size_of::<(String, String)>()
            + 4 * std::mem::size_of::<Skipped>()) as u64;
        assert_eq!(budget.used(), total);
        let limits = Limits {
            max_metadata_bytes: total,
            ..limits
        };
        assert!(from_document(&document, &limits, &Budget::new(total)).is_ok());
        let limits = Limits {
            max_metadata_bytes: total - 1,
            ..limits
        };
        assert!(matches!(
            from_document(&document, &limits, &Budget::new(total - 1)),
            Err(Error::LimitExceeded { limit: "metadata_bytes", value, max })
                if value == total && max == total - 1
        ));
    }

    #[test]
    #[allow(clippy::arithmetic_side_effects)]
    fn sq13_standalone_refusals_keep_the_cumulative_boundary() {
        let name = "s".repeat(1024);
        let count = 16;
        let text = format!(
            "GROUP=SwathStructure GROUP=S SwathName=\"{name}\" GROUP=IndexDimensionMap {} END_GROUP=IndexDimensionMap END_GROUP=S END_GROUP=SwathStructure",
            "OBJECT=M GeoDimension=x DataDimension=y END_OBJECT=M ".repeat(count)
        );
        let total = (name.len()
            + std::mem::size_of::<EosObject>()
            + count * (name.len() + "/x/y".len() + std::mem::size_of::<Skipped>()))
            as u64;
        let limits = Limits {
            max_metadata_bytes: total,
            ..Limits::default()
        };
        let (_, skipped) = from_struct_metadata(&text, &limits).expect("exact metadata budget");
        assert_eq!(skipped.len(), count);
        assert!(matches!(
            from_struct_metadata(&text, &Limits { max_metadata_bytes: total - 1, ..limits }),
            Err(Error::LimitExceeded { limit: "metadata_bytes", value, max })
                if value == total && max == total - 1
        ));
    }

    #[test]
    #[allow(clippy::arithmetic_side_effects)]
    fn sq10_standalone_refusal_lists_ignore_the_attribute_ceiling() {
        let name = "é".repeat(16);
        for feature in ["index-dimension-map", "merged-field", "eos-point"] {
            let limits = Limits {
                max_attribute_bytes: 0,
                ..Limits::default()
            };
            for count in [2, 3] {
                let text = match feature {
                    "index-dimension-map" => format!(
                        "GROUP=SwathStructure GROUP=S SwathName=\"{name}\" GROUP=IndexDimensionMap {} END_GROUP=IndexDimensionMap END_GROUP=S END_GROUP=SwathStructure",
                        "OBJECT=M GeoDimension=x DataDimension=y END_OBJECT=M ".repeat(count)
                    ),
                    "merged-field" => format!(
                        "GROUP=GridStructure GROUP=G GridName=\"{name}\" XDim=1 YDim=1 GROUP=MergedFields OBJECT=M MergedFieldName=m FieldList=({}) END_OBJECT=M END_GROUP=MergedFields END_GROUP=G END_GROUP=GridStructure",
                        ["x", "y", "z"]
                            .iter()
                            .take(count)
                            .copied()
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                    _ => format!(
                        "GROUP=PointStructure {} END_GROUP=PointStructure",
                        format!("OBJECT=P PointName=\"{name}\" END_OBJECT=P ").repeat(count)
                    ),
                };
                let (_, skipped) = from_struct_metadata(&text, &limits).expect("metadata budget");
                assert_eq!(skipped.len(), count);
                assert!(skipped.iter().all(|entry| entry.feature == feature));
            }
        }
    }

    #[test]
    fn sq10_standalone_refusals_ignore_the_attribute_ceiling() {
        let text =
            "GROUP=PointStructure OBJECT=P PointName=P END_OBJECT=P END_GROUP=PointStructure";
        let limits = Limits {
            max_attribute_bytes: 0,
            ..Limits::default()
        };
        let (_, skipped) =
            from_struct_metadata(text, &limits).expect("metadata budget is sufficient");
        assert_eq!(
            skipped,
            vec![Skipped {
                object: "P".into(),
                feature: "eos-point"
            }]
        );
    }

    #[test]
    fn sq6_eos_parser_obeys_custom_rank_and_depth_limits() {
        let limits = Limits {
            max_rank: 1,
            ..Limits::default()
        };
        assert!(matches!(
            from_struct_metadata(GRID, &limits),
            Err(Error::LimitExceeded {
                limit: "rank",
                value: 2,
                max: 1
            })
        ));
        let limits = Limits {
            max_group_depth: 1,
            ..Limits::default()
        };
        assert!(matches!(
            from_struct_metadata(GRID, &limits),
            Err(Error::LimitExceeded {
                limit: "group depth",
                value: 2,
                max: 1
            })
        ));
        let dimensions = std::iter::repeat_n("\"XDim\"", 33)
            .collect::<Vec<_>>()
            .join(",");
        let grid = GRID.replace(
            "DimList=(\"YDim\",\"XDim\")",
            &format!("DimList=({dimensions})"),
        );
        let limits = Limits {
            max_rank: 33,
            ..Limits::default()
        };
        let (eos, _) = from_struct_metadata(&grid, &limits).expect("raised rank");
        assert_eq!(eos.grids[0].fields[0].dims.len(), 33);
    }

    #[test]
    fn maps_grid_dimensions_fields_and_raw_parameters() {
        let (eos, skipped) = from_struct_metadata(GRID, &Limits::default()).expect("grid");
        assert!(skipped.is_empty());
        assert!(eos.swaths.is_empty());
        assert_eq!(
            eos.grids,
            vec![EosObject {
                name: "Vegetation".into(),
                dims: vec![("XDim".into(), 4), ("YDim".into(), 3), ("Band".into(), 2)],
                fields: vec![
                    EosField {
                        name: "Vegetation Index".into(),
                        dims: vec!["YDim".into(), "XDim".into()],
                        geolocation: false,
                        dataset: None
                    },
                    EosField {
                        name: "Quality".into(),
                        dims: vec!["Band".into(), "YDim".into(), "XDim".into()],
                        geolocation: false,
                        dataset: None
                    },
                ],
                dimension_maps: vec![],
                params: vec![
                    (
                        "UpperLeftPointMtrs".into(),
                        "(-1111950.519667, 5559752.598333)".into()
                    ),
                    ("LowerRightMtrs".into(), "(0.000000,4447802.078667)".into()),
                    ("Projection".into(), "GCTP_SNSOID".into()),
                    (
                        "ProjParams".into(),
                        "(6371007.181,0,0,0,0,0,0,0,0,0,0,0,0)".into()
                    ),
                    ("SphereCode".into(), "-1".into()),
                    ("GridOrigin".into(), "HDFE_GD_UL".into()),
                ],
            }]
        );
    }

    #[test]
    fn maps_swath_geolocation_and_dimension_map() {
        let (eos, _) = from_struct_metadata(SWATH, &Limits::default()).expect("swath");
        assert!(eos.grids.is_empty());
        assert_eq!(eos.swaths.len(), 1);
        let swath = &eos.swaths[0];
        assert_eq!(swath.name, "Radiance");
        assert_eq!(
            swath.dims,
            vec![
                ("GeoTrack".into(), 3),
                ("DataTrack".into(), 6),
                ("CrossTrack".into(), 4)
            ]
        );
        assert_eq!(
            swath.fields,
            vec![
                EosField {
                    name: "Latitude".into(),
                    dims: vec!["GeoTrack".into(), "CrossTrack".into()],
                    geolocation: true,
                    dataset: None
                },
                EosField {
                    name: "Radiance".into(),
                    dims: vec!["DataTrack".into(), "CrossTrack".into()],
                    geolocation: false,
                    dataset: None
                },
            ]
        );
        assert_eq!(
            swath.dimension_maps,
            vec![DimensionMap {
                geo_dim: "GeoTrack".into(),
                data_dim: "DataTrack".into(),
                offset: -1,
                increment: 2
            }]
        );
        assert!(swath.params.is_empty());
    }

    #[test]
    fn reports_merged_fields_and_index_dimension_maps() {
        let (_, skipped) = from_struct_metadata(SWATH, &Limits::default()).expect("swath");
        assert_eq!(
            skipped,
            vec![
                Skipped {
                    object: "Radiance/Pressure".into(),
                    feature: "merged-field"
                },
                Skipped {
                    object: "Radiance/Humidity".into(),
                    feature: "merged-field"
                },
                Skipped {
                    object: "Radiance/CrossTrack/DataTrack".into(),
                    feature: "index-dimension-map"
                },
            ]
        );
    }

    #[test]
    fn reports_point_structure() {
        let (eos, skipped) = from_struct_metadata(
            include_str!("../../../fixtures/odl/point.odl"),
            &Limits::default(),
        )
        .expect("point");
        assert!(eos.grids.is_empty() && eos.swaths.is_empty());
        assert_eq!(
            skipped,
            vec![Skipped {
                object: "Stations".into(),
                feature: "eos-point"
            }]
        );
    }

    #[test]
    fn preserves_object_order_and_keeps_fields_unlinked() {
        let grid = GRID
            .replace("GROUP=GRID_1", "GROUP=GRID_2")
            .replace("Vegetation", "Second");
        let second = grid
            .split_once("  GROUP=GRID_2")
            .expect("start")
            .1
            .split_once("END_GROUP=GridStructure")
            .expect("end")
            .0;
        let two = GRID.replace(
            "END_GROUP=GridStructure",
            &format!("  GROUP=GRID_2{second}END_GROUP=GridStructure"),
        );
        let (eos, _) = from_struct_metadata(&two, &Limits::default()).expect("two grids");
        assert_eq!(
            eos.grids
                .iter()
                .map(|g| g.name.as_str())
                .collect::<Vec<_>>(),
            ["Vegetation", "Second"]
        );
        assert!(
            eos.grids
                .iter()
                .flat_map(|g| &g.fields)
                .all(|f| f.dataset.is_none())
        );
    }

    #[test]
    fn rejects_missing_or_invalid_eos_values() {
        for text in [
            GRID.replace("GridName=\"Vegetation\"", "OtherName=\"Vegetation\""),
            GRID.replace("XDim=4", "XDim=-1"),
            GRID.replace("Size=2", "Size=18446744073709551616"),
            GRID.replace("DimList=(\"YDim\",\"XDim\")", "DimList=\"YDim\""),
            GRID.replace("DimensionName=\"Band\"", "DimensionName=(\"Band\")"),
            SWATH.replace("Offset=-1", "Offset=9223372036854775808"),
            SWATH.replace("Increment=2", "Wrong=2"),
            SWATH.replace(
                "FieldList=(\"Pressure\",\"Humidity\")",
                "FieldList=(\"Pressure\",(\"Humidity\"))",
            ),
        ] {
            assert!(
                matches!(
                    from_struct_metadata(&text, &Limits::default()),
                    Err(crate::Error::Malformed { .. })
                ),
                "{text}"
            );
        }
    }

    #[test]
    fn propagates_depth_limit() {
        assert!(matches!(
            from_struct_metadata(
                include_str!("../../../fixtures/odl/too-deep.odl"),
                &Limits::default()
            ),
            Err(crate::Error::LimitExceeded { .. })
        ));
    }

    #[test]
    fn accepts_empty_structure_sections() {
        let (eos, skipped) = from_struct_metadata(
            "GROUP=GridStructure END_GROUP=GridStructure END",
            &Limits::default(),
        )
        .expect("empty");
        assert!(eos.grids.is_empty() && eos.swaths.is_empty() && skipped.is_empty());
    }
}
