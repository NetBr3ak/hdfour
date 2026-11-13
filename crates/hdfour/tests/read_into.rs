use std::path::{Path, PathBuf};

use hdfour::{Error, Value};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture_files() -> Vec<PathBuf> {
    (1..=21)
        .map(|index| root().join(format!("fixtures/F{index:02}.hdf")))
        .chain(["R02", "R03"].map(|id| {
            let golden: serde_json::Value = serde_json::from_slice(
                &std::fs::read(root().join(format!("fixtures/golden/{id}.json")))
                    .expect("golden metadata"),
            )
            .expect("valid metadata");
            root()
                .join("corpus")
                .join(golden["file"].as_str().expect("corpus file"))
        }))
        .collect()
}

fn value_bytes(value: &Value) -> &[u8] {
    match value {
        Value::Char8(values) | Value::UChar8(values) | Value::U8(values) => values,
        Value::I8(values) => bytemuck::cast_slice(values),
        Value::I16(values) => bytemuck::cast_slice(values),
        Value::U16(values) => bytemuck::cast_slice(values),
        Value::I32(values) => bytemuck::cast_slice(values),
        Value::U32(values) => bytemuck::cast_slice(values),
        Value::F32(values) => bytemuck::cast_slice(values),
        Value::F64(values) => bytemuck::cast_slice(values),
    }
}

#[test]
fn read_into_matches_read_for_every_fixture_dataset() {
    for path in fixture_files() {
        let file = hdfour::open(&path).expect("fixture");
        for dataset in file.datasets() {
            let context = format!("{}: {}", path.display(), dataset.name);
            match file.read(dataset) {
                Ok(array) => {
                    let expected = value_bytes(&array.data);
                    let length = file.byte_len(dataset).expect(&context);
                    assert_eq!(length, expected.len(), "{context}");
                    let mut output = vec![0xa5; length];
                    file.read_into(dataset, &mut output).expect(&context);
                    assert_eq!(output, expected, "{context}");
                }
                Err(error) => {
                    assert_eq!(
                        file.byte_len(dataset).expect_err(&context).to_string(),
                        error.to_string(),
                        "{context}"
                    );
                    assert_eq!(
                        file.read_into(dataset, &mut [])
                            .expect_err(&context)
                            .to_string(),
                        error.to_string(),
                        "{context}"
                    );
                }
            }
        }
    }
}

#[test]
fn read_into_rejects_short_and_long_buffers() {
    for path in fixture_files() {
        let file = hdfour::open(&path).expect("fixture");
        for dataset in file.datasets() {
            let Ok(expected) = file.byte_len(dataset) else {
                continue;
            };
            for actual in expected.checked_sub(1).into_iter().chain([expected + 1]) {
                let mut output = vec![0xa5; actual];
                let error = file
                    .read_into(dataset, &mut output)
                    .expect_err("wrong length");
                assert!(matches!(
                    error,
                    Error::WrongLength { expected: length, actual: supplied }
                        if length == expected && supplied == actual
                ));
                assert_eq!(
                    error.to_string(),
                    format!("output buffer holds {actual} bytes, the dataset needs {expected}")
                );
                assert!(output.iter().all(|byte| *byte == 0xa5));
            }
        }
    }
}

#[test]
fn output_buffer_apis_reject_foreign_and_cloned_datasets() {
    let path = root().join("fixtures/F01.hdf");
    let file = hdfour::open(&path).expect("fixture");
    let foreign = hdfour::open(&path).expect("foreign fixture");
    let cloned = file.datasets()[0].clone();
    for dataset in [&foreign.datasets()[0], &cloned] {
        let expected = file.read(dataset).expect_err("foreign dataset");
        assert!(matches!(expected, Error::NotFound(_)));
        let length_error = file.byte_len(dataset).expect_err("foreign dataset");
        let read_error = file
            .read_into(dataset, &mut [])
            .expect_err("foreign dataset");
        assert!(matches!(length_error, Error::NotFound(_)));
        assert!(matches!(read_error, Error::NotFound(_)));
        assert_eq!(length_error.to_string(), expected.to_string());
        assert_eq!(read_error.to_string(), expected.to_string());
    }
}
