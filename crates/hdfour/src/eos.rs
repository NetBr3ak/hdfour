//! HDF-EOS2 grid and swath metadata mapping.
#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use std::collections::BTreeSet;

use crate::odl::{self, Entry, Odl, Value};
use crate::{DimensionMap, Eos, EosField, EosObject, Error, Limits, Result, Skipped};

/// Map joined StructMetadata text to EOS objects and explicit feature refusals.
/// Field dataset indices stay unset until the SD reader resolves Vgroup links.
pub fn from_struct_metadata(text: &str, limits: &Limits) -> Result<(Eos, Vec<Skipped>)> {
    let document = odl::parse(text, limits)?;
    let mut eos = Eos {
        grids: Vec::new(),
        swaths: Vec::new(),
    };
    let mut skipped = Refusals {
        entries: Vec::new(),
        bytes: 0,
        max_bytes: limits.max_attribute_bytes,
    };
    for grid in objects(&document, "GridStructure") {
        eos.grids.push(grid_object(grid, &mut skipped, limits)?);
    }
    for swath in objects(&document, "SwathStructure") {
        eos.swaths.push(swath_object(swath, &mut skipped, limits)?);
    }
    for point in objects(&document, "PointStructure") {
        skipped.push(&[scalar(point, "PointName")?], "eos-point")?;
    }
    Ok((eos, skipped.entries))
}

struct Refusals {
    entries: Vec<Skipped>,
    bytes: u64,
    max_bytes: u64,
}

impl Refusals {
    fn push(&mut self, parts: &[&str], feature: &'static str) -> Result<()> {
        let bytes = parts.iter().try_fold(self.bytes, |total, part| {
            total
                .checked_add(part.len() as u64)
                .ok_or_else(|| malformed("EOS refusal path size overflow"))
        })?;
        crate::dd::check_limit("attribute_bytes", bytes, self.max_bytes)?;
        self.entries.push(Skipped {
            object: parts.concat(),
            feature,
        });
        self.bytes = bytes;
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

fn dimensions(body: &Odl) -> Result<Vec<(String, u64)>> {
    // EHapi.c (GDAL 1.11.5):1902-1906 writes DimensionName and Size in each object.
    objects(body, "Dimension")
        .map(|dimension| {
            Ok((
                scalar(dimension, "DimensionName")?.to_owned(),
                size(dimension, "Size")?,
            ))
        })
        .collect()
}

fn parameters(body: &Odl) -> Vec<(String, String)> {
    body.entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Assignment { name, raw, .. }
                if !matches!(name.as_str(), "GridName" | "SwathName" | "XDim" | "YDim") =>
            {
                Some((name.clone(), raw.clone()))
            }
            _ => None,
        })
        .collect()
}

fn merged_fields(body: &Odl, object: &str, skipped: &mut Refusals) -> Result<BTreeSet<String>> {
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
            merged.insert(name.to_owned());
        }
    }
    Ok(merged)
}

fn fields(
    body: &Odl,
    section: &str,
    merged: &BTreeSet<String>,
    limits: &Limits,
) -> Result<Vec<EosField>> {
    let name_key = match section {
        "GeoField" => "GeoFieldName",
        "DataField" => "DataFieldName",
        _ => return Err(malformed("invalid EOS field section")),
    };
    let mut fields = Vec::new();
    // EHapi.c (GDAL 1.11.5):2024-2028,2086-2090 writes field names and DimList.
    for field in objects(body, section) {
        let name = scalar(field, name_key)?.to_owned();
        if merged.contains(&name) {
            continue;
        }
        let items = list(field, "DimList")?;
        crate::dd::check_limit("rank", items.len() as u64, limits.max_rank)?;
        let dims = items
            .iter()
            .map(|item| list_scalar(item).map(str::to_owned))
            .collect::<Result<_>>()?;
        fields.push(EosField {
            name,
            dims,
            geolocation: section == "GeoField",
            dataset: None,
        });
    }
    Ok(fields)
}

fn grid_object(body: &Odl, skipped: &mut Refusals, limits: &Limits) -> Result<EosObject> {
    let name = scalar(body, "GridName")?.to_owned();
    // GDapi.c (GDAL 1.11.5):467-471 declares XDim and YDim outside Dimension.
    let mut dims = vec![
        ("XDim".into(), size(body, "XDim")?),
        ("YDim".into(), size(body, "YDim")?),
    ];
    dims.extend(dimensions(body)?);
    let merged = merged_fields(body, &name, skipped)?;
    Ok(EosObject {
        name,
        dims,
        fields: fields(body, "DataField", &merged, limits)?,
        dimension_maps: Vec::new(),
        params: parameters(body),
    })
}

fn swath_object(body: &Odl, skipped: &mut Refusals, limits: &Limits) -> Result<EosObject> {
    let name = scalar(body, "SwathName")?.to_owned();
    let merged = merged_fields(body, &name, skipped)?;
    let mut swath_fields = fields(body, "GeoField", &merged, limits)?;
    swath_fields.extend(fields(body, "DataField", &merged, limits)?);
    // EHapi.c (GDAL 1.11.5):1938-1944 writes the four linear map entries.
    let dimension_maps = objects(body, "DimensionMap")
        .map(|map| {
            Ok(DimensionMap {
                geo_dim: scalar(map, "GeoDimension")?.to_owned(),
                data_dim: scalar(map, "DataDimension")?.to_owned(),
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
        dims: dimensions(body)?,
        fields: swath_fields,
        dimension_maps,
        params: parameters(body),
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
            let path_bytes = name.len()
                + match feature {
                    "index-dimension-map" => 4,
                    "merged-field" => 2,
                    _ => 0,
                };
            let max = u64::try_from(path_bytes * 2).expect("budget");
            let limits = Limits {
                max_attribute_bytes: max,
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
                let result = from_struct_metadata(&text, &limits);
                if count == 2 {
                    let (_, skipped) = result.expect("exact budget");
                    assert_eq!(skipped.len(), count);
                    assert!(skipped.iter().all(|entry| entry.feature == feature));
                } else {
                    assert!(
                        matches!(result, Err(Error::LimitExceeded { limit: "attribute_bytes", value, max: ceiling }) if value == max + u64::try_from(path_bytes).expect("path bytes") && ceiling == max),
                        "{feature}: {result:?}"
                    );
                }
            }
        }
    }

    #[test]
    #[allow(clippy::arithmetic_side_effects)]
    fn sq6_refusal_budget_is_shared_across_eos_objects() {
        let text = SWATH.replace(
            "GROUP=PointStructure\nEND_GROUP=PointStructure",
            "GROUP=PointStructure OBJECT=P PointName=P END_OBJECT=P END_GROUP=PointStructure",
        );
        let (_, skipped) = from_struct_metadata(&text, &Limits::default()).expect("refusals");
        let total = skipped
            .iter()
            .map(|entry| u64::try_from(entry.object.len()).expect("path length"))
            .sum::<u64>();
        let limits = Limits {
            max_attribute_bytes: total,
            ..Limits::default()
        };
        assert!(from_struct_metadata(&text, &limits).is_ok());
        let limits = Limits {
            max_attribute_bytes: total - 1,
            ..limits
        };
        assert!(
            matches!(from_struct_metadata(&text, &limits), Err(Error::LimitExceeded { limit: "attribute_bytes", value, max }) if value == total && max == total - 1)
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
