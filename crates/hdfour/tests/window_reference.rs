use std::collections::BTreeMap;
use std::path::Path;

use hdfour::Value;
use serde_json::Value as Json;

fn coordinates(value: &Json) -> Vec<u64> {
    value
        .as_array()
        .expect("coordinate list")
        .iter()
        .map(|coordinate| coordinate.as_u64().expect("nonnegative coordinate"))
        .collect()
}

fn little_endian_bytes(value: &Value) -> Vec<u8> {
    macro_rules! numeric {
        ($values:expr) => {
            $values
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect()
        };
    }
    match value {
        Value::Char8(values) | Value::UChar8(values) | Value::U8(values) => values.clone(),
        Value::I8(values) => numeric!(values),
        Value::I16(values) => numeric!(values),
        Value::U16(values) => numeric!(values),
        Value::I32(values) => numeric!(values),
        Value::U32(values) => numeric!(values),
        Value::F32(values) => numeric!(values),
        Value::F64(values) => numeric!(values),
    }
}

#[test]
fn read_windows_match_independent_c_selections_and_reference_slices() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let reference: Json = serde_json::from_slice(
        &std::fs::read(fixtures.join("window-reference.json"))
            .expect("immutable C window reference"),
    )
    .expect("reference JSON");
    let files: BTreeMap<_, _> = reference["sources"]
        .as_object()
        .expect("source files")
        .keys()
        .map(|name| {
            (
                name.as_str(),
                hdfour::open(fixtures.join(name)).expect("reference fixture"),
            )
        })
        .collect();
    let windows = reference["windows"].as_array().expect("reference windows");
    assert!(!windows.is_empty());
    for window in windows {
        let name = window["file"].as_str().expect("fixture name");
        let index = usize::try_from(window["dataset"].as_u64().expect("dataset index"))
            .expect("dataset index fits usize");
        let file = files.get(name).expect("open fixture");
        let dataset = file.datasets().get(index).expect("reference dataset");
        let start = coordinates(&window["start"]);
        let count = coordinates(&window["count"]);
        let hex = window["hex"].as_str().expect("reference byte string");
        assert_eq!(hex.len() % 2, 0);
        let expected: Vec<_> = hex
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII byte pair"), 16)
                    .expect("hex byte")
            })
            .collect();
        let array = file
            .read_window(dataset, &start, &count)
            .unwrap_or_else(|error| panic!("{name}/{index} {start:?} {count:?}: {error}"));
        assert_eq!(array.shape, count, "{name}/{index} {start:?}");
        assert_eq!(
            little_endian_bytes(&array.data),
            expected,
            "{name}/{index} {start:?} {count:?}: {}",
            window["reference"],
        );
    }
}
