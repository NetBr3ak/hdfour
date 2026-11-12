use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use hdfour::{Attribute, DType, EosObject, Error, File, Value};
use serde_json::{Value as Json, json};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn value_bytes(value: &Value) -> (u16, Vec<u8>) {
    macro_rules! numeric {
        ($code:expr, $values:expr) => {
            (
                $code,
                $values
                    .iter()
                    .flat_map(|value| value.to_le_bytes())
                    .collect(),
            )
        };
    }
    match value {
        Value::Char8(values) => (4, values.clone()),
        Value::UChar8(values) => (3, values.clone()),
        Value::I8(values) => numeric!(20, values),
        Value::U8(values) => (21, values.clone()),
        Value::I16(values) => numeric!(22, values),
        Value::U16(values) => numeric!(23, values),
        Value::I32(values) => numeric!(24, values),
        Value::U32(values) => numeric!(25, values),
        Value::F32(values) => numeric!(5, values),
        Value::F64(values) => numeric!(6, values),
    }
}

fn dtype_code(dtype: DType) -> u16 {
    match dtype {
        DType::Char8 => 4,
        DType::UChar8 => 3,
        DType::I8 => 20,
        DType::U8 => 21,
        DType::I16 => 22,
        DType::U16 => 23,
        DType::I32 => 24,
        DType::U32 => 25,
        DType::F32 => 5,
        DType::F64 => 6,
    }
}

fn attributes(attributes: &[Attribute]) -> Json {
    Json::Array(attributes.iter().map(|attribute| {
        let (dtype, bytes) = value_bytes(&attribute.value);
        let width = match dtype { 22 | 23 => 2, 5 | 24 | 25 => 4, 6 => 8, _ => 1 };
        let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        json!({"name": attribute.name, "dtype": dtype, "count": bytes.len() / width, "hex": hex})
    }).collect())
}

fn eos_objects(objects: &[EosObject]) -> Json {
    Json::Array(
        objects
            .iter()
            .map(|object| {
                json!({
                    "name": object.name,
                    "dims": object.dims,
                    "params": object.params,
                    "fields": object.fields.iter().map(|field| json!({
                        "name": field.name, "dims": field.dims,
                        "geolocation": field.geolocation, "dataset": field.dataset,
                    })).collect::<Vec<_>>(),
                    "dimension_maps": object.dimension_maps.iter().map(|map| json!({
                        "geo_dim": map.geo_dim, "data_dim": map.data_dim,
                        "offset": map.offset, "increment": map.increment,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

fn compare(id: &str) {
    let root = root();
    let golden: Json = serde_json::from_slice(
        &std::fs::read(root.join(format!("fixtures/golden/{id}.json"))).expect("golden JSON"),
    )
    .expect("valid golden JSON");
    let input = if id.starts_with('R') {
        "corpus"
    } else {
        "fixtures"
    };
    let file = hdfour::open(
        root.join(input)
            .join(golden["file"].as_str().expect("input file")),
    )
    .unwrap_or_else(|error| panic!("{id}: open: {error}"));
    assert_eq!(
        attributes(file.attributes()),
        golden["attributes"],
        "{id}: file attributes"
    );
    let expected = golden["datasets"].as_array().expect("dataset list");
    let expectations: Json = serde_json::from_slice(
        &std::fs::read(root.join("fixtures/expect.json")).expect("expectations"),
    )
    .expect("expectation JSON");
    assert_eq!(
        golden["skipped"], expectations[id]["skipped"],
        "{id}: skipped contract"
    );
    assert_eq!(golden["eos"], expectations[id]["eos"], "{id}: EOS contract");
    for dataset in expected {
        let index = dataset["index"].as_u64().expect("index").to_string();
        let expectation = expectations[id]["datasets"][&index]
            .as_str()
            .unwrap_or("values");
        assert_eq!(
            dataset["expect"], expectation,
            "{id}/{index}: refusal contract"
        );
    }
    let present: Vec<_> = expected
        .iter()
        .filter(|ds| {
            !ds["expect"]
                .as_str()
                .expect("expectation")
                .starts_with("skipped:")
        })
        .collect();
    assert_eq!(file.datasets().len(), present.len(), "{id}: dataset count");
    for (dataset, expected) in file.datasets().iter().zip(present) {
        let index = expected["index"].as_u64().expect("SDselect index");
        let metadata = json!({
            "name": dataset.name, "dtype": dtype_code(dataset.dtype), "shape": dataset.shape,
            "attributes": attributes(&dataset.attributes), "is_coordinate": dataset.is_coordinate,
            "dimensions": dataset.dims.iter().map(|dim| json!({
                "name": dim.name, "size": dim.size, "unlimited": dim.unlimited,
                "attributes": attributes(&dim.attributes),
            })).collect::<Vec<_>>(),
        });
        for key in [
            "name",
            "dtype",
            "shape",
            "attributes",
            "is_coordinate",
            "dimensions",
        ] {
            assert_eq!(metadata[key], expected[key], "{id}/{index}: {key}");
        }
        let expectation = expected["expect"].as_str().expect("expectation");
        match expectation {
            "values" => {
                let array = file
                    .read(dataset)
                    .unwrap_or_else(|error| panic!("{id}/{index}: {error}"));
                assert_eq!(array.shape, dataset.shape, "{id}/{index}: array shape");
                let bytes = std::fs::read(root.join(format!("fixtures/golden/{id}/{index}.bin")))
                    .expect("reference bytes");
                assert_eq!(
                    value_bytes(&array.data).1,
                    bytes,
                    "{id}/{index}: exact values"
                );
            }
            "malformed" => assert!(
                matches!(file.read(dataset), Err(Error::Malformed { .. })),
                "{id}/{index}"
            ),
            unsupported if unsupported.starts_with("unsupported:") => {
                let feature = unsupported.strip_prefix("unsupported:").expect("prefix");
                assert!(
                    matches!(file.read(dataset), Err(Error::Unsupported { feature: actual, ref object }) if actual == feature && object == &dataset.name),
                    "{id}/{index}: {feature}"
                );
            }
            other => panic!("{id}/{index}: unknown expectation {other}"),
        }
    }
    let skipped: Vec<_> = file
        .skipped()
        .iter()
        .map(|item| json!({"object": item.object, "feature": item.feature}))
        .collect();
    assert_eq!(json!(skipped), golden["skipped"], "{id}: skipped");
    let eos = file
        .eos()
        .map(|eos| json!({"grids": eos_objects(&eos.grids), "swaths": eos_objects(&eos.swaths)}))
        .unwrap_or(Json::Null);
    assert_eq!(eos, golden["eos"], "{id}: EOS");
}

macro_rules! golden_tests {
    ($($test:ident: $id:literal),* $(,)?) => {$ (
        #[test]
        fn $test() { compare($id); }
    )*};
}

golden_tests! {
    golden_f01: "F01", golden_f02: "F02", golden_f03: "F03",
    golden_f04: "F04", golden_f05: "F05", golden_f06: "F06",
    golden_f07: "F07", golden_f08: "F08", golden_f09: "F09",
    golden_f10: "F10", golden_f11: "F11", golden_f12: "F12",
    golden_f13: "F13", golden_f14: "F14", golden_f15: "F15",
    golden_f16: "F16", golden_f17: "F17", golden_f18: "F18",
    golden_f19: "F19", golden_f20: "F20", golden_f21: "F21",
    golden_r02: "R02", golden_r03: "R03",
}

#[test]
fn dataset_lookup_reports_missing_and_duplicate_names() {
    let file = hdfour::open(root().join("fixtures/F06.hdf")).expect("F06");
    let name = &file.datasets()[0].name;
    assert!(matches!(
        file.dataset(name),
        Err(Error::Ambiguous { count: 2, .. })
    ));
    assert!(matches!(file.dataset("absent"), Err(Error::NotFound(_))));
}

#[test]
fn from_bytes_matches_path_metadata_and_values() {
    let path = root().join("fixtures/F01.hdf");
    let file = File::from_bytes(std::fs::read(&path).expect("F01 bytes")).expect("memory file");
    let disk = hdfour::open(path).expect("disk file");
    assert_eq!(file.datasets(), disk.datasets());
    for (memory, disk_dataset) in file.datasets().iter().zip(disk.datasets()) {
        assert_eq!(
            file.read(memory).expect("memory values"),
            disk.read(disk_dataset).expect("disk values")
        );
    }
}

#[test]
fn opening_does_not_read_array_payloads() {
    use hdfour::{Limits, Memory, dd};
    let bytes = std::fs::read(root().join("fixtures/F01.hdf")).expect("fixture");
    let source = Memory::new(bytes);
    let index = dd::parse(&source, &Limits::default()).expect("index");
    let descriptor = index.iter().find(|dd| dd.tag == 702).expect("SD");
    let start = descriptor.offset.expect("written");
    let (source, read_data) = watch_reads(source, start..start + descriptor.length);
    let file = File::from_source(source, Limits::default()).expect("metadata only");
    assert!(!read_data.load(Ordering::SeqCst));
    for dataset in file.datasets() {
        file.read(dataset).expect("values");
    }
    assert!(read_data.load(Ordering::SeqCst));
}

#[test]
fn custom_limits_bound_metadata_arrays_attributes_and_chunks() {
    use hdfour::{Limits, Memory};
    for (id, limits) in [
        (
            "F01",
            Limits {
                max_array_bytes: 4,
                ..Limits::default()
            },
        ),
        (
            "F01",
            Limits {
                max_rank: 1,
                ..Limits::default()
            },
        ),
        (
            "F03",
            Limits {
                max_attribute_bytes: 1,
                ..Limits::default()
            },
        ),
        (
            "F07",
            Limits {
                max_chunk_header_bytes: 58,
                ..Limits::default()
            },
        ),
        (
            "F07",
            Limits {
                max_chunks: 1,
                ..Limits::default()
            },
        ),
        (
            "F01",
            Limits {
                max_group_depth: 1,
                ..Limits::default()
            },
        ),
    ] {
        let bytes = std::fs::read(root().join(format!("fixtures/{id}.hdf"))).expect("fixture");
        match File::from_source(Memory::new(bytes), limits) {
            Err(Error::LimitExceeded { .. }) => {}
            Ok(file) => assert!(
                file.datasets()
                    .iter()
                    .any(|ds| matches!(file.read(ds), Err(Error::LimitExceeded { .. }))),
                "{id}"
            ),
            Err(error) => panic!("{id}: wrong limit error: {error}"),
        }
    }
}

#[test]
fn foreign_or_modified_dataset_handles_are_rejected() {
    let file = hdfour::open(root().join("fixtures/F01.hdf")).expect("F01");
    let foreign = hdfour::open(root().join("fixtures/F02.hdf")).expect("F02");
    assert!(file.read(&foreign.datasets()[0]).is_err());
    let mut modified = file.datasets()[0].clone();
    modified.shape = vec![u64::MAX, u64::MAX];
    assert!(file.read(&modified).is_err());
}

fn descriptor_slot(bytes: &[u8], key: (u16, u16)) -> usize {
    let mut block = 4;
    while block != 0 {
        let count = u16::from_be_bytes(bytes[block..block + 2].try_into().expect("count"));
        for slot in 0..usize::from(count) {
            let offset = block + 6 + 12 * slot;
            let tag = u16::from_be_bytes(bytes[offset..offset + 2].try_into().expect("tag"));
            let reference =
                u16::from_be_bytes(bytes[offset + 2..offset + 4].try_into().expect("ref"));
            if (tag, reference) == key {
                return offset;
            }
        }
        block = usize::try_from(u32::from_be_bytes(
            bytes[block + 2..block + 6].try_into().expect("next"),
        ))
        .expect("block");
    }
    panic!("missing descriptor {key:?}")
}

#[test]
fn oversized_attribute_storage_is_rejected_before_payload_read() {
    use hdfour::{Limits, Memory, dd};
    let mut bytes = std::fs::read(root().join("fixtures/F03.hdf")).expect("F03");
    let index = dd::parse(&Memory::new(&bytes), &Limits::default()).expect("index");
    let offset = index
        .get(1963, 9)
        .expect("i8 attribute")
        .offset
        .expect("written");
    let slot = descriptor_slot(&bytes, (1963, 9));
    bytes[slot + 8..slot + 12].copy_from_slice(&4_u32.to_be_bytes());
    let (source, read) = watch_reads(Memory::new(bytes), offset..offset + 3);
    let limits = Limits {
        max_attribute_bytes: 3,
        ..Limits::default()
    };
    assert!(matches!(
        File::from_source(source, limits),
        Err(Error::Malformed { .. })
    ));
    assert!(
        !read.load(Ordering::SeqCst),
        "mismatched attribute storage must not be read"
    );
}

#[test]
fn eos_field_rank_obeys_the_file_limit() {
    let mut bytes = std::fs::read(root().join("fixtures/F15.hdf")).expect("F15");
    let original = b"(\"YDim\",\"XDim\")";
    let replacement = b"(\"Y\",\"X\",\"Z\")  ";
    assert_eq!(original.len(), replacement.len());
    let (header, _) = vdata_named(&bytes, "StructMetadata.0");
    let index =
        hdfour::dd::parse(&hdfour::Memory::new(&bytes), &hdfour::Limits::default()).expect("index");
    let storage = index.get(1963, header.reference).expect("active metadata");
    let start = usize::try_from(storage.offset.expect("written")).expect("offset");
    let end = start + usize::try_from(storage.length).expect("length");
    let offset = start
        + bytes[start..end]
            .windows(original.len())
            .position(|window| window == original)
            .expect("active DimList");
    bytes[offset..offset + original.len()].copy_from_slice(replacement);
    let limits = hdfour::Limits {
        max_rank: 2,
        ..hdfour::Limits::default()
    };
    match File::from_source(hdfour::Memory::new(bytes), limits) {
        Err(error) => assert!(
            matches!(
                error,
                Error::LimitExceeded {
                    limit: "rank",
                    value: 3,
                    max: 2
                }
            ),
            "unexpected error: {error}"
        ),
        Ok(_) => panic!("rank-3 EOS field must exceed the file limit"),
    }
}

#[test]
fn chunk_header_reads_do_not_include_unused_descriptor_tail() {
    use hdfour::{Limits, Memory, dd};
    let mut bytes = std::fs::read(root().join("fixtures/F07.hdf")).expect("F07");
    let index = dd::parse(&Memory::new(&bytes), &Limits::default()).expect("index");
    let descriptor = index.get(702, 3).expect("chunks");
    let original_offset = usize::try_from(descriptor.offset.expect("offset")).expect("offset");
    let length = usize::try_from(descriptor.length).expect("length");
    let header = bytes[original_offset..original_offset + length].to_vec();
    let start = bytes.len();
    bytes.extend(header);
    bytes.resize(bytes.len() + 4096, 0);
    let slot = descriptor_slot(&bytes, (0x42be, 3));
    bytes[slot + 4..slot + 8].copy_from_slice(&u32::try_from(start).expect("start").to_be_bytes());
    bytes[slot + 8..slot + 12]
        .copy_from_slice(&u32::try_from(length + 4096).expect("length").to_be_bytes());
    let tail = (start + length) as u64..bytes.len() as u64;
    let (source, read) = watch_reads(Memory::new(bytes), tail);
    let file = File::from_source(source, Limits::default()).expect("header with unused tail");
    file.read(&file.datasets()[0]).expect("chunk values");
    assert!(
        !read.load(Ordering::SeqCst),
        "unused descriptor tail must not be read"
    );
}

struct ReadWatch {
    source: hdfour::Memory<Vec<u8>>,
    range: std::ops::Range<u64>,
    read: Arc<AtomicBool>,
}

impl hdfour::Source for ReadWatch {
    fn len(&self) -> u64 {
        self.source.len()
    }
    fn read_at(&self, offset: u64, bytes: &mut [u8]) -> hdfour::Result<()> {
        if offset < self.range.end && offset + bytes.len() as u64 > self.range.start {
            self.read.store(true, Ordering::SeqCst);
        }
        self.source.read_at(offset, bytes)
    }
}

fn watch_reads(
    source: hdfour::Memory<Vec<u8>>,
    range: std::ops::Range<u64>,
) -> (ReadWatch, Arc<AtomicBool>) {
    let read = Arc::new(AtomicBool::new(false));
    (
        ReadWatch {
            source,
            range,
            read: read.clone(),
        },
        read,
    )
}

fn vdata_named(bytes: &[u8], name: &str) -> (hdfour::dd::Descriptor, hdfour::vset::VdataHeader) {
    let source = hdfour::Memory::new(bytes);
    let index = hdfour::dd::parse(&source, &hdfour::Limits::default()).expect("index");
    for descriptor in index.iter().filter(|descriptor| descriptor.tag == 1962) {
        let mut bytes = vec![0; usize::try_from(descriptor.length).expect("header size")];
        hdfour::element::read(&source, &index, descriptor, &mut bytes).expect("header bytes");
        let header = hdfour::vset::parse_vdata_header(&bytes).expect("header");
        if header.name == name {
            return (descriptor.clone(), header);
        }
    }
    panic!("missing Vdata {name}")
}

#[test]
fn unknown_dataset_and_attribute_types_are_skipped_by_object_path() {
    use hdfour::{Limits, Memory, dd};
    for (key, object, change) in [
        ((106, 22), "attributes", 0),
        ((1962, 9), "attributes/i8", 10),
        ((1962, 37), "/i8", 10),
    ] {
        let mut bytes = std::fs::read(root().join("fixtures/F03.hdf")).expect("F03");
        let index = dd::parse(&Memory::new(&bytes), &Limits::default()).expect("index");
        let start = usize::try_from(
            index
                .get(key.0, key.1)
                .expect("descriptor")
                .offset
                .expect("written"),
        )
        .expect("offset");
        if change == 0 {
            bytes[start] = 2;
        } else {
            bytes[start + change] = 0x10;
        }
        let file =
            File::from_bytes(bytes).expect("unsupported number type must not reject the file");
        assert_eq!(
            file.skipped(),
            &[hdfour::Skipped {
                object: object.into(),
                feature: "number-type"
            }]
        );
        if change == 0 {
            assert!(matches!(
                file.dataset("attributes"),
                Err(Error::NotFound(_))
            ));
        }
    }
}

#[test]
fn fill_type_mismatch_is_a_read_time_refusal() {
    let mut bytes = std::fs::read(root().join("fixtures/F03.hdf")).expect("F03");
    let (descriptor, _) = vdata_named(&bytes, "_FillValue");
    let start = usize::try_from(descriptor.offset.expect("written")).expect("offset");
    bytes[start + 10..start + 12].copy_from_slice(&23_u16.to_be_bytes());
    let file = File::from_bytes(bytes).expect("raw mismatched fill metadata");
    assert!(matches!(
        file.read(file.dataset("attributes").expect("dataset")),
        Err(Error::Unsupported {
            feature: "fill-type-mismatch",
            ..
        })
    ));
}

#[test]
fn ragged_arrays_are_skipped_and_vgroup_cycles_are_malformed() {
    use hdfour::{Limits, Memory, dd, vset};
    for tag in [781_u16, 1965] {
        let mut bytes = std::fs::read(root().join("fixtures/F03.hdf")).expect("F03");
        let index = dd::parse(&Memory::new(&bytes), &Limits::default()).expect("index");
        let descriptor = index.get(1965, 23).expect("variable Vgroup");
        let start = usize::try_from(descriptor.offset.expect("written")).expect("offset");
        let length = usize::try_from(descriptor.length).expect("length");
        let group = vset::parse_vgroup(&bytes[start..start + length]).expect("group");
        let member = group
            .members
            .iter()
            .position(|(tag, _)| *tag == 701)
            .expect("SDD");
        let offset = start + 2 + 2 * member;
        bytes[offset..offset + 2].copy_from_slice(&tag.to_be_bytes());
        if tag == 1965 {
            let offset = start + 2 + 2 * group.members.len() + 2 * member;
            bytes[offset..offset + 2].copy_from_slice(&23_u16.to_be_bytes());
            assert!(matches!(
                File::from_bytes(bytes),
                Err(Error::Malformed { .. })
            ));
        } else {
            let file = File::from_bytes(bytes).expect("ragged array is skipped");
            assert_eq!(
                file.skipped(),
                &[hdfour::Skipped {
                    object: "attributes".into(),
                    feature: "ragged-array"
                }]
            );
            assert!(matches!(
                file.dataset("attributes"),
                Err(Error::NotFound(_))
            ));
        }
    }
}

proptest::proptest! {
    #[test]
    fn arbitrary_file_bytes_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..4096)) {
        let limits = hdfour::Limits {
            max_descriptors: 128, max_dd_blocks: 16, max_group_depth: 8, max_rank: 4,
            max_array_bytes: 65536, max_attribute_bytes: 4096, max_chunks: 64,
            max_chunk_header_bytes: 256,
        };
        if let Ok(file) = File::from_source(hdfour::Memory::new(bytes), limits) {
            for dataset in file.datasets() { let _ = file.read(dataset); }
        }
    }
}

#[test]
fn zero_chunk_tags_use_header_fill_not_the_dataset_attribute() {
    use hdfour::{Limits, Memory, dd, element, vset};
    let mut bytes = std::fs::read(root().join("fixtures/F07.hdf")).expect("F07");
    let (descriptor, header) = vdata_named(&bytes, "_HDF_CHK_TBL_702_3_1962_4");
    let index = dd::parse(&Memory::new(&bytes), &Limits::default()).expect("index");
    let storage = index
        .get(1963, descriptor.reference)
        .expect("table storage");
    let length = element::len(&Memory::new(&bytes), &index, storage).expect("logical table size");
    let mut records = vec![0; usize::try_from(length).expect("length")];
    element::read(&Memory::new(&bytes), &index, storage, &mut records).expect("table records");
    let origin_field = header
        .fields
        .iter()
        .position(|field| field.name == "origin")
        .expect("origin field");
    let Value::I32(origins) = vset::field_values(&header, &records, origin_field).expect("origins")
    else {
        panic!("origin type")
    };
    assert_eq!(&origins[..2], &[0, 0]);
    let tag_offset = usize::from(
        header
            .fields
            .iter()
            .find(|field| field.name == "chk_tag")
            .expect("tag field")
            .offset,
    );
    let ref_offset = usize::from(
        header
            .fields
            .iter()
            .find(|field| field.name == "chk_ref")
            .expect("ref field")
            .offset,
    );
    records[tag_offset..tag_offset + 2].copy_from_slice(&0_u16.to_be_bytes());
    records[ref_offset..ref_offset + 2].copy_from_slice(&u16::MAX.to_be_bytes());
    let slot = descriptor_slot(&bytes, (storage.tag, storage.reference));
    let offset = u32::try_from(bytes.len()).expect("offset");
    bytes.extend(&records);
    bytes[slot..slot + 2].copy_from_slice(&1963_u16.to_be_bytes());
    bytes[slot + 4..slot + 8].copy_from_slice(&offset.to_be_bytes());
    bytes[slot + 8..slot + 12]
        .copy_from_slice(&u32::try_from(records.len()).expect("length").to_be_bytes());
    let (fill, _) = vdata_named(&bytes, "_FillValue");
    let index = dd::parse(&Memory::new(&bytes), &Limits::default()).expect("index");
    let fill_offset = usize::try_from(
        index
            .get(1963, fill.reference)
            .expect("fill storage")
            .offset
            .expect("written"),
    )
    .expect("offset");
    bytes[fill_offset..fill_offset + 2].copy_from_slice(&0x1234_i16.to_be_bytes());
    let file = File::from_bytes(bytes).expect("zero chunk tag");
    let Value::I16(actual) = file.read(&file.datasets()[0]).expect("sparse values").data else {
        panic!("i16 values")
    };
    let mut expected: Vec<_> = std::fs::read(root().join("fixtures/golden/F07/0.bin"))
        .expect("golden bytes")
        .chunks_exact(2)
        .map(|bytes| i16::from_le_bytes(bytes.try_into().expect("i16")))
        .collect();
    for row in 0..2 {
        for column in 0..3 {
            expected[row * 7 + column] = -12345;
        }
    }
    assert_eq!(actual, expected);
}

proptest::proptest! {
    #[test]
    fn mutated_fixture_bytes_never_panic(fixture in 0_usize..4, position in proptest::prelude::any::<usize>(), byte in proptest::prelude::any::<u8>()) {
        let fixtures = [
            include_bytes!("../../../fixtures/F03.hdf").as_slice(),
            include_bytes!("../../../fixtures/F07.hdf").as_slice(),
            include_bytes!("../../../fixtures/F08.hdf").as_slice(),
            include_bytes!("../../../fixtures/F16.hdf").as_slice(),
        ];
        let mut bytes = fixtures[fixture].to_vec();
        let position = position % bytes.len();
        bytes[position] = byte;
        let limits = hdfour::Limits {
            max_descriptors: 512, max_dd_blocks: 16, max_group_depth: 8, max_rank: 4,
            max_array_bytes: 65536, max_attribute_bytes: 4096, max_chunks: 64,
            max_chunk_header_bytes: 256,
        };
        if let Ok(file) = File::from_source(hdfour::Memory::new(bytes), limits) {
            for dataset in file.datasets() { let _ = file.read(dataset); }
        }
    }
}
