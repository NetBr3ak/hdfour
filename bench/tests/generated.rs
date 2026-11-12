use std::error::Error;
use std::path::Path;

use hdfour::{DType, Value, nt};
use serde_json::Value as Json;

#[test]
fn generated_arrays_match_c_metadata_and_bytes() -> Result<(), Box<dyn Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(".build");
    for kind in ["chunked", "contiguous"] {
        let golden = root.join(kind);
        let expected: Vec<Json> =
            serde_json::from_slice(&std::fs::read(golden.join("manifest.json"))?)?;
        let file = hdfour::open(root.join(format!("{kind}.hdf")))?;
        assert_eq!(
            file.datasets().len(),
            expected.len(),
            "{kind}: dataset count"
        );
        assert!(!expected.is_empty());
        for (index, (dataset, metadata)) in file.datasets().iter().zip(&expected).enumerate() {
            assert_eq!(dataset.name, metadata["name"].as_str().unwrap());
            let dtype = match metadata["dtype"].as_str().unwrap() {
                "<i2" => DType::I16,
                "<f4" => DType::F32,
                other => return Err(format!("unexpected generator dtype: {other}").into()),
            };
            assert_eq!(dataset.dtype, dtype);
            let shape: Vec<_> = metadata["shape"]
                .as_array()
                .unwrap()
                .iter()
                .map(|size| size.as_u64().unwrap())
                .collect();
            assert_eq!(dataset.shape, shape);
            let array = file.read(dataset)?;
            assert_eq!(array.shape, shape);
            let mut actual = bytes(&array.data)?;
            if cfg!(target_endian = "big") {
                nt::swap_in_place(&mut actual, dataset.dtype.size());
            }
            let expected = std::fs::read(golden.join(format!("{index}.bin")))?;
            assert_eq!(actual, expected, "{kind}: dataset {index} bytes");
        }
        println!(
            "PASS core {kind}: {} datasets match SDreaddata",
            expected.len()
        );
    }
    Ok(())
}

fn bytes(value: &Value) -> Result<Vec<u8>, Box<dyn Error>> {
    match value {
        Value::I16(values) => Ok(bytemuck::must_cast_slice::<i16, u8>(values).to_vec()),
        Value::F32(values) => Ok(bytemuck::must_cast_slice::<f32, u8>(values).to_vec()),
        _ => Err("the generator only writes int16 and float32".into()),
    }
}
