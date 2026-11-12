use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use hdfour::{Error, File, Limits, Memory, Source, Value};

fn text(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend(
        u16::try_from(value.len())
            .expect("short name")
            .to_be_bytes(),
    );
    bytes.extend(value.as_bytes());
}

fn group(name: &str, class: &str, members: &[(u16, u16)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(u16::try_from(members.len()).expect("members").to_be_bytes());
    for (tag, _) in members {
        bytes.extend(tag.to_be_bytes());
    }
    for (_, reference) in members {
        bytes.extend(reference.to_be_bytes());
    }
    text(&mut bytes, name);
    text(&mut bytes, class);
    bytes.extend([0; 4]);
    bytes.extend([0, 3, 0, 0, 0]);
    bytes
}

fn header(name: &str, class: &str, code: u16, width: u16) -> Vec<u8> {
    let mut bytes = vec![0, 0, 0, 0, 0, 1];
    for word in [width, 1, code, width, 0, 1] {
        bytes.extend(word.to_be_bytes());
    }
    text(&mut bytes, "value");
    text(&mut bytes, name);
    text(&mut bytes, class);
    bytes.extend([0; 4]);
    bytes.extend([0, 3, 0, 0, 0]);
    bytes
}

fn image(elements: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = vec![14, 3, 19, 1];
    bytes.extend(u16::try_from(elements.len()).expect("count").to_be_bytes());
    bytes.extend([0; 4]);
    let mut offset = 10 + 12 * elements.len();
    for (tag, reference, data) in elements {
        bytes.extend(tag.to_be_bytes());
        bytes.extend(reference.to_be_bytes());
        bytes.extend(u32::try_from(offset).expect("offset").to_be_bytes());
        bytes.extend(u32::try_from(data.len()).expect("length").to_be_bytes());
        offset += data.len();
    }
    for (_, _, data) in elements {
        bytes.extend(data);
    }
    bytes
}

fn coordinate_file(marker: Option<&str>) -> Vec<(u16, u16, Vec<u8>)> {
    let mut members = vec![(106, 1), (1965, 2), (1962, 4), (720, 1)];
    if marker.is_some() {
        members.push((1962, 5));
    }
    let mut elements = vec![
        (1965, 1, group("root", "CDF0.0", &[(1965, 3)])),
        (1965, 2, group("x", "Dim0.0", &[(1962, 2)])),
        (1965, 3, group("x", "Var0.0", &members)),
        (106, 1, vec![1, 22, 16, 1]),
        (1962, 2, header("size", "DimVal0.1", 24, 4)),
        (1963, 2, 2_i32.to_be_bytes().to_vec()),
        (1962, 4, header("units", "Attr0.0", 4, 1)),
        (1963, 4, b"m".to_vec()),
    ];
    if let Some(class) = marker {
        elements.push((1962, 5, header("marker", class, 4, 1)));
    }
    elements
}

#[test]
fn sq7_unknown_coordinate_status_uses_rank_one_name_fallback() {
    let file = File::from_bytes(image(&coordinate_file(None))).expect("old layout");
    let dataset = file.dataset("x").expect("coordinate");
    assert!(dataset.is_coordinate);
    assert_eq!(dataset.dims[0].attributes, dataset.attributes);
    for (marker, expected) in [("SDSVar", false), ("CoordVar", true)] {
        let file = File::from_bytes(image(&coordinate_file(Some(marker)))).expect("marker");
        assert_eq!(file.datasets()[0].is_coordinate, expected, "{marker}");
    }
}

#[test]
fn sq7_unrelated_noninterlaced_vdata_is_not_parsed() {
    let mut elements = coordinate_file(Some("SDSVar"));
    let mut unrelated = header("unused", "table", 4, 1);
    unrelated[0..2].copy_from_slice(&1_u16.to_be_bytes());
    elements.push((1962, 99, unrelated));
    let file = File::from_bytes(image(&elements)).expect("unrelated Vdata");
    assert_eq!(file.datasets().len(), 1);
    assert!(file.skipped().is_empty());
}

#[test]
fn sq7_referenced_noninterlaced_attribute_is_skipped_at_decode() {
    let mut elements = coordinate_file(Some("SDSVar"));
    let attribute = elements
        .iter_mut()
        .find(|(tag, reference, _)| (*tag, *reference) == (1962, 4))
        .expect("attribute");
    attribute.2[0..2].copy_from_slice(&1_u16.to_be_bytes());
    let file = File::from_bytes(image(&elements)).expect("unsupported attribute layout");
    assert!(file.datasets()[0].attributes.is_empty());
    assert_eq!(
        file.skipped(),
        &[hdfour::Skipped {
            object: "x/units".into(),
            feature: "vdata-interlace"
        }]
    );
}

struct CountReads {
    bytes: Memory<Vec<u8>>,
    count: Arc<AtomicU64>,
}

#[test]
fn sq7_noninterlaced_dimension_is_skipped_at_decode() {
    let mut elements = coordinate_file(Some("SDSVar"));
    let dimension = elements
        .iter_mut()
        .find(|(tag, reference, _)| (*tag, *reference) == (1962, 2))
        .expect("dimension");
    dimension.2[0..2].copy_from_slice(&1_u16.to_be_bytes());
    let file = File::from_bytes(image(&elements)).expect("unsupported dimension layout");
    assert!(file.datasets().is_empty());
    assert_eq!(
        file.skipped(),
        &[hdfour::Skipped {
            object: "x".into(),
            feature: "vdata-interlace"
        }]
    );
}

impl Source for CountReads {
    fn len(&self) -> u64 {
        self.bytes.len()
    }
    fn read_at(&self, offset: u64, output: &mut [u8]) -> hdfour::Result<()> {
        self.count.fetch_add(output.len() as u64, Ordering::Relaxed);
        self.bytes.read_at(offset, output)
    }
}

#[test]
fn sq7_aliased_headers_bound_retained_names_and_reads() {
    let data = group(&"a".repeat(8192), "unrelated", &[]);
    let count = 32_u16;
    let mut bytes = vec![14, 3, 19, 1];
    bytes.extend(count.to_be_bytes());
    bytes.extend([0; 4]);
    let offset = 10 + 12 * u32::from(count);
    for reference in 1..=count {
        bytes.extend(1965_u16.to_be_bytes());
        bytes.extend(reference.to_be_bytes());
        bytes.extend(offset.to_be_bytes());
        bytes.extend(
            u32::try_from(data.len())
                .expect("header size")
                .to_be_bytes(),
        );
    }
    let header_bytes = data.len() as u64;
    bytes.extend(data);
    let count = Arc::new(AtomicU64::new(0));
    let source = CountReads {
        bytes: Memory::new(bytes),
        count: count.clone(),
    };
    let result = File::from_source(
        source,
        Limits {
            max_metadata_bytes: 32768,
            ..Limits::default()
        },
    );
    let copied = count.load(Ordering::Relaxed);
    assert!(
        copied < 32768 + 2 * header_bytes,
        "aliased header bytes read: {copied}"
    );
    assert!(matches!(
        result,
        Err(Error::LimitExceeded {
            limit: "metadata_bytes",
            ..
        })
    ));
}

#[test]
fn sq7_null_chunk_tags_use_header_fill() {
    let mut bytes = include_bytes!("../../../fixtures/F07.hdf").to_vec();
    let source = Memory::new(&bytes);
    let index = hdfour::dd::parse(&source, &Limits::default()).expect("index");
    let table = index
        .iter()
        .filter(|dd| dd.tag == 1962)
        .find_map(|dd| {
            let mut data = vec![0; dd.length as usize];
            hdfour::element::read(&source, &index, dd, &mut data).expect("header");
            let header = hdfour::vset::parse_vdata_header(&data).expect("Vdata");
            header
                .name
                .starts_with("_HDF_CHK_TBL_")
                .then_some((dd.reference, header))
        })
        .expect("table");
    let storage = index.get(1963, table.0).expect("storage");
    let mut records =
        vec![0; hdfour::element::len(&source, &index, storage).expect("length") as usize];
    hdfour::element::read(&source, &index, storage, &mut records).expect("records");
    let tag = usize::from(
        table
            .1
            .fields
            .iter()
            .find(|field| field.name == "chk_tag")
            .expect("tag")
            .offset,
    );
    records[tag..tag + 2].copy_from_slice(&1_u16.to_be_bytes());
    let slot = descriptor_slot(&bytes, storage.tag, storage.reference);
    let offset = u32::try_from(bytes.len()).expect("offset");
    bytes.extend(&records);
    bytes[slot..slot + 2].copy_from_slice(&1963_u16.to_be_bytes());
    bytes[slot + 4..slot + 8].copy_from_slice(&offset.to_be_bytes());
    bytes[slot + 8..slot + 12]
        .copy_from_slice(&u32::try_from(records.len()).expect("length").to_be_bytes());
    let file = File::from_bytes(bytes).expect("null chunk metadata");
    let Value::I16(values) = file
        .read(&file.datasets()[0])
        .expect("null chunk values")
        .data
    else {
        panic!("i16");
    };
    for row in 0..2 {
        assert_eq!(&values[row * 7..row * 7 + 3], &[-12345; 3]);
    }
}

fn descriptor_slot(bytes: &[u8], tag: u16, reference: u16) -> usize {
    let mut block = 4;
    while block != 0 {
        let count = u16::from_be_bytes(bytes[block..block + 2].try_into().expect("count"));
        for slot in 0..usize::from(count) {
            let offset = block + 6 + 12 * slot;
            if bytes[offset..offset + 2] == tag.to_be_bytes()
                && bytes[offset + 2..offset + 4] == reference.to_be_bytes()
            {
                return offset;
            }
        }
        block = u32::from_be_bytes(bytes[block + 2..block + 6].try_into().expect("next")) as usize;
    }
    panic!("descriptor");
}

#[test]
fn sq7_retained_dataset_names_share_one_metadata_budget() {
    let name = "n".repeat(8192);
    let mut elements = coordinate_file(Some("SDSVar"));
    elements[2].2 = group(&name, "Var0.0", &[(106, 1), (1965, 2), (720, 1)]);
    let limits = Limits {
        max_metadata_bytes: 32768,
        ..Limits::default()
    };
    let file = File::from_source(Memory::new(image(&elements)), limits.clone())
        .expect("one header and one dataset fit the budget");
    assert_eq!(file.datasets()[0].name, name);
    elements[0].2 = group("root", "CDF0.0", &[(1965, 3); 32]);
    assert!(matches!(
        File::from_source(Memory::new(image(&elements)), limits),
        Err(Error::LimitExceeded {
            limit: "metadata_bytes",
            ..
        })
    ));
}

#[test]
fn sq10_four_mib_attribute_is_charged_once() {
    let length = 4 * 1024 * 1024;
    let mut attribute = header("large", "Attr0.0", 21, 1);
    attribute[2..6].copy_from_slice(&u32::try_from(length).expect("length").to_be_bytes());
    let bytes = image(&[
        (1965, 1, group("root", "CDF0.0", &[(1962, 2)])),
        (1962, 2, attribute),
        (1963, 2, vec![7; length]),
    ]);
    let limits = Limits {
        max_metadata_bytes: (length + 4096) as u64,
        ..Limits::default()
    };
    let file = File::from_source(Memory::new(bytes.clone()), limits.clone())
        .expect("4 MiB value plus at most 4 KiB of metadata");
    assert_eq!(file.attributes()[0].value, Value::U8(vec![7; length]));
    assert!(matches!(
        File::from_source(
            Memory::new(bytes),
            Limits {
                max_metadata_bytes: (length - 1) as u64,
                ..limits
            }
        ),
        Err(Error::LimitExceeded {
            limit: "metadata_bytes",
            ..
        })
    ));
}

#[test]
fn sq10_truncated_character_attributes_charge_their_allocation() {
    let length = 1024 * 1024;
    let mut attribute = header("text", "Attr0.0", 4, 1);
    attribute[2..6].copy_from_slice(&u32::try_from(length).expect("length").to_be_bytes());
    let bytes = image(&[
        (1965, 1, group("root", "CDF0.0", &[(1962, 2); 2])),
        (1962, 2, attribute),
        (1963, 2, vec![b'x'; length]),
    ]);
    assert!(matches!(
        File::from_source(
            Memory::new(bytes),
            Limits {
                max_metadata_bytes: (length + 4096) as u64,
                ..Limits::default()
            }
        ),
        Err(Error::LimitExceeded {
            limit: "metadata_bytes",
            ..
        })
    ));
}

#[test]
fn sq10_refused_dataset_names_are_not_charged_twice() {
    let name = "r".repeat(16384);
    let bytes = image(&[
        (1965, 1, group("root", "CDF0.0", &[(1965, 2)])),
        (1965, 2, group(&name, "Var0.0", &[(781, 1)])),
    ]);
    let file = File::from_source(
        Memory::new(bytes),
        Limits {
            max_metadata_bytes: 32768,
            ..Limits::default()
        },
    )
    .expect("only the refused path is retained");
    assert!(file.datasets().is_empty());
    assert_eq!(
        file.skipped(),
        &[hdfour::Skipped {
            object: name,
            feature: "ragged-array"
        }]
    );
}

#[test]
fn sq10_repeated_member_copies_do_not_exhaust_the_cache_budget() {
    let mut elements = coordinate_file(Some("SDSVar"));
    elements[0].2 = group("root", "CDF0.0", &[(1965, 3); 512]);
    let mut members = vec![(106, 1), (1965, 2), (720, 1)];
    members.extend([(1962, 5); 512]);
    elements[2].2 = group("x", "Var0.0", &members);
    let file = File::from_source(
        Memory::new(image(&elements)),
        Limits {
            max_metadata_bytes: 262144,
            ..Limits::default()
        },
    )
    .expect("discarded member copies do not accumulate");
    assert_eq!(file.datasets().len(), 512);
    assert!(file.datasets().iter().all(|dataset| dataset.shape == [2]));
}

#[test]
fn sq10_repeated_dimension_records_are_cached() {
    let mut elements = coordinate_file(Some("SDSVar"));
    elements[0].2 = group("root", "CDF0.0", &[(1965, 3); 64]);
    elements[1].2 = group("x", "Dim0.0", &[(1962, 2); 64]);
    let baseline_count = Arc::new(AtomicU64::new(0));
    File::from_source(
        CountReads {
            bytes: Memory::new(image(&elements)),
            count: baseline_count.clone(),
        },
        Limits::default(),
    )
    .expect("small shared dimension records");
    let records = elements
        .iter_mut()
        .find(|(tag, reference, _)| (*tag, *reference) == (1963, 2))
        .expect("records");
    records.2.resize(16384, 0);
    let header = elements
        .iter_mut()
        .find(|(tag, reference, _)| (*tag, *reference) == (1962, 2))
        .expect("header");
    header.2[2..6].copy_from_slice(&4096_u32.to_be_bytes());
    let count = Arc::new(AtomicU64::new(0));
    let source = CountReads {
        bytes: Memory::new(image(&elements)),
        count: count.clone(),
    };
    let result = File::from_source(
        source,
        Limits {
            max_metadata_bytes: 32768,
            ..Limits::default()
        },
    );
    let copied = count.load(Ordering::Relaxed);
    assert!(copied < 65536, "dimension record bytes read: {copied}");
    assert_eq!(copied, baseline_count.load(Ordering::Relaxed) + 16384 - 4);
    let file = result.expect("shared dimension records are decoded once");
    assert_eq!(file.datasets().len(), 64);
    assert!(file.datasets().iter().all(|dataset| dataset.shape == [2]));
}

#[test]
fn sq13_unsupported_dimension_type_is_refused_before_its_records_are_read() {
    // Native-order int32 (0x1018) is outside the supported number types.
    let read_bytes = |records: u32| {
        let mut elements = coordinate_file(Some("SDSVar"));
        elements[0].2 = group("root", "CDF0.0", &[(1965, 3); 64]);
        let size = elements
            .iter_mut()
            .find(|(tag, reference, _)| (*tag, *reference) == (1962, 2))
            .expect("header");
        size.2 = header("size", "DimVal0.1", 0x1018, 4);
        size.2[2..6].copy_from_slice(&records.to_be_bytes());
        let block = elements
            .iter_mut()
            .find(|(tag, reference, _)| (*tag, *reference) == (1963, 2))
            .expect("records");
        block.2.resize(records as usize * 4, 0);
        let count = Arc::new(AtomicU64::new(0));
        let file = File::from_source(
            CountReads {
                bytes: Memory::new(image(&elements)),
                count: count.clone(),
            },
            Limits::default(),
        )
        .expect("an unsupported dimension type skips its datasets");
        assert!(file.datasets().is_empty());
        count.load(Ordering::Relaxed)
    };
    assert_eq!(read_bytes(4096), read_bytes(1));
}

#[test]
fn sq13_unrelated_vgroup_members_fit_the_cache_budget() {
    let mut elements = coordinate_file(Some("SDSVar"));
    let members: Vec<_> = (1000..1256).map(|reference| (1962, reference)).collect();
    for &(_, reference) in &members {
        elements.push((1962, reference, header("unused", "table", 4, 1)));
    }
    for reference in 100..132 {
        elements.push((1965, reference, group("unused", "unrelated", &members)));
    }
    let file = File::from_source(
        Memory::new(image(&elements)),
        Limits {
            max_metadata_bytes: 45056,
            ..Limits::default()
        },
    )
    .expect("retained members fit without charging discarded tag and reference arrays");
    assert_eq!(file.datasets().len(), 1);
    assert_eq!(file.datasets()[0].shape, [2]);
    assert!(file.skipped().is_empty());
}

#[test]
fn sq13_repeated_eos_members_charge_each_cached_name_once() {
    let mut elements = coordinate_file(Some("SDSVar"));
    let metadata = b"GROUP=GridStructure GROUP=G GridName=g XDim=1 YDim=1 END_GROUP=G END_GROUP=GridStructure END";
    let width = u16::try_from(metadata.len()).expect("metadata length");
    let mut attribute = header("StructMetadata.0", "Attr0.0", 4, width);
    attribute[16..18].copy_from_slice(&width.to_be_bytes());
    elements[0].2 = group("root", "CDF0.0", &[(1965, 3), (1962, 6)]);
    let name = "v".repeat(1024);
    elements.extend([
        (1962, 6, attribute),
        (1963, 6, metadata.to_vec()),
        (1965, 10, group("g", "GRID", &[(1965, 11)])),
        (1965, 11, group("fields", "fields", &[(1962, 12); 64])),
        (1962, 12, header(&name, "Field", 4, 1)),
    ]);
    let file = File::from_source(
        Memory::new(image(&elements)),
        Limits {
            max_metadata_bytes: 8192,
            ..Limits::default()
        },
    )
    .expect("one cached field name despite repeated members");
    assert_eq!(file.skipped().len(), 1);
    assert_eq!(file.skipped()[0].object, format!("g/{name}"));
    assert_eq!(file.skipped()[0].feature, "vdata-field");
}

#[test]
fn sq10_interlaced_dimensions_are_refused_before_work_charging() {
    let mut elements = coordinate_file(Some("SDSVar"));
    let header = elements
        .iter_mut()
        .find(|(tag, reference, _)| (*tag, *reference) == (1962, 2))
        .expect("dimension header");
    header.2[0..2].copy_from_slice(&1_u16.to_be_bytes());
    header.2[2..6].copy_from_slice(&4096_u32.to_be_bytes());
    let file = File::from_source(
        Memory::new(image(&elements)),
        Limits {
            max_metadata_bytes: 4096,
            ..Limits::default()
        },
    )
    .expect("no dimension records are decoded");
    assert!(file.datasets().is_empty());
    assert_eq!(
        file.skipped(),
        &[hdfour::Skipped {
            object: "x".into(),
            feature: "vdata-interlace"
        }]
    );
}
