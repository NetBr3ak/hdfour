//! Browser bindings with owned typed arrays and plain metadata objects.

use hdfour::{
    Attribute, Codec, DType, Dataset, Dimension, EosObject, Limits, Memory, Storage, Value,
};
use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;

const MAX_BROWSER_BYTES: u64 = 268_435_456;

/// An HDF4 file retained in WebAssembly memory until `free()`.
#[wasm_bindgen]
pub struct File {
    inner: hdfour::File,
}

#[wasm_bindgen]
impl File {
    /// Copy file bytes into WebAssembly and read the metadata.
    #[wasm_bindgen(js_name = fromBytes)]
    pub fn from_bytes(bytes: Uint8Array) -> Result<File, JsValue> {
        let length = u64::from(bytes.length());
        if length > MAX_BROWSER_BYTES {
            return Err(js_error(hdfour::Error::LimitExceeded {
                limit: "file bytes",
                value: length,
                max: MAX_BROWSER_BYTES,
            }));
        }
        let limits = Limits {
            max_array_bytes: MAX_BROWSER_BYTES,
            max_attribute_bytes: 1_048_576,
            max_metadata_bytes: 16_777_216,
            ..Limits::default()
        };
        Ok(Self {
            inner: hdfour::File::from_source(Memory::new(bytes.to_vec()), limits)
                .map_err(js_error)?,
        })
    }

    /// Dataset metadata in SD selection order.
    pub fn datasets(&self) -> Result<Array, JsValue> {
        self.inner.datasets().iter().map(dataset).collect()
    }

    /// File attributes, or attributes of one dataset selected by index.
    pub fn attributes(&self, index: Option<u32>) -> Result<Array, JsValue> {
        let values = match index {
            Some(index) => &self.dataset(index)?.attributes,
            None => self.inner.attributes(),
        };
        attributes(values)
    }

    /// Objects omitted by the core, with their refusal feature strings.
    pub fn skipped(&self) -> Result<Array, JsValue> {
        self.inner
            .skipped()
            .iter()
            .map(|entry| {
                object(&[
                    ("object", entry.object.as_str().into()),
                    ("feature", entry.feature.into()),
                ])
            })
            .collect()
    }

    /// Raw EOS metadata with dataset links, or null when absent.
    pub fn eos(&self) -> Result<JsValue, JsValue> {
        let Some(eos) = self.inner.eos() else {
            return Ok(JsValue::NULL);
        };
        object(&[
            (
                "grids",
                eos.grids
                    .iter()
                    .map(eos_object)
                    .collect::<Result<Array, _>>()?
                    .into(),
            ),
            (
                "swaths",
                eos.swaths
                    .iter()
                    .map(eos_object)
                    .collect::<Result<Array, _>>()?
                    .into(),
            ),
        ])
    }

    /// Read raw C-order values into an independent JavaScript typed array.
    pub fn read(&self, index: u32) -> Result<JsValue, JsValue> {
        let array = self.inner.read(self.dataset(index)?).map_err(js_error)?;
        Ok(value(&array.data))
    }
}

impl File {
    fn dataset(&self, index: u32) -> Result<&Dataset, JsValue> {
        let index = usize::try_from(index).map_err(js_error)?;
        self.inner
            .datasets()
            .get(index)
            .ok_or_else(|| js_sys::RangeError::new("dataset index is out of range").into())
    }
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    js_sys::Error::new(&error.to_string()).into()
}

fn object(fields: &[(&str, JsValue)]) -> Result<JsValue, JsValue> {
    let object = Object::new();
    for (key, value) in fields {
        if !Reflect::set(&object, &JsValue::from_str(key), value)? {
            return Err(js_sys::TypeError::new("cannot set metadata property").into());
        }
    }
    Ok(object.into())
}

fn numbers(values: &[u64]) -> Array {
    values.iter().map(|value| unsigned(*value)).collect()
}

fn unsigned(value: u64) -> JsValue {
    if value <= 9_007_199_254_740_991 {
        // Every integer in this range has an exact JavaScript number representation.
        JsValue::from_f64(value as f64)
    } else {
        value.into()
    }
}

fn signed(value: i64) -> JsValue {
    if (-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&value) {
        // Every integer in this range has an exact JavaScript number representation.
        JsValue::from_f64(value as f64)
    } else {
        value.into()
    }
}

fn strings(values: &[String]) -> Array {
    values
        .iter()
        .map(|value| JsValue::from_str(value))
        .collect()
}

fn dtype(dtype: DType) -> &'static str {
    match dtype {
        DType::I8 => "i8",
        DType::U8 => "u8",
        DType::I16 => "i16",
        DType::U16 => "u16",
        DType::I32 => "i32",
        DType::U32 => "u32",
        DType::F32 => "f32",
        DType::F64 => "f64",
        DType::Char8 => "char8",
        DType::UChar8 => "uchar8",
    }
}

fn value(value: &Value) -> JsValue {
    match value {
        Value::Char8(values) => {
            let bytes = js_sys::Uint8Array::from(values.as_slice());
            js_sys::Int8Array::new(&bytes.buffer()).into()
        }
        Value::UChar8(values) | Value::U8(values) => {
            js_sys::Uint8Array::from(values.as_slice()).into()
        }
        Value::I8(values) => js_sys::Int8Array::from(values.as_slice()).into(),
        Value::I16(values) => js_sys::Int16Array::from(values.as_slice()).into(),
        Value::U16(values) => js_sys::Uint16Array::from(values.as_slice()).into(),
        Value::I32(values) => js_sys::Int32Array::from(values.as_slice()).into(),
        Value::U32(values) => js_sys::Uint32Array::from(values.as_slice()).into(),
        Value::F32(values) => js_sys::Float32Array::from(values.as_slice()).into(),
        Value::F64(values) => js_sys::Float64Array::from(values.as_slice()).into(),
    }
}

fn attributes(values: &[Attribute]) -> Result<Array, JsValue> {
    values
        .iter()
        .map(|attribute| {
            object(&[
                ("name", attribute.name.as_str().into()),
                ("value", value(&attribute.value)),
            ])
        })
        .collect()
}

fn dimension(dimension: &Dimension) -> Result<JsValue, JsValue> {
    object(&[
        ("name", dimension.name.as_str().into()),
        ("size", unsigned(dimension.size)),
        ("unlimited", dimension.unlimited.into()),
        ("attributes", attributes(&dimension.attributes)?.into()),
    ])
}

fn dataset(dataset: &Dataset) -> Result<JsValue, JsValue> {
    object(&[
        ("name", dataset.name.as_str().into()),
        ("dtype", dtype(dataset.dtype).into()),
        ("shape", numbers(&dataset.shape).into()),
        (
            "dims",
            dataset
                .dims
                .iter()
                .map(dimension)
                .collect::<Result<Array, _>>()?
                .into(),
        ),
        ("attributes", attributes(&dataset.attributes)?.into()),
        ("is_coordinate", dataset.is_coordinate.into()),
        ("storage", storage(&dataset.storage)?),
        ("codec", codec(dataset.codec)?),
    ])
}

fn storage(storage: &Storage) -> Result<JsValue, JsValue> {
    let kind = match storage {
        Storage::Empty => "empty",
        Storage::Contiguous => "contiguous",
        Storage::Linked => "linked",
        Storage::External => "external",
        Storage::Chunked { chunk_shape } => {
            return object(&[
                ("kind", "chunked".into()),
                ("chunk_shape", numbers(chunk_shape).into()),
            ]);
        }
    };
    object(&[("kind", kind.into())])
}

fn codec(codec: Codec) -> Result<JsValue, JsValue> {
    let kind = match codec {
        Codec::None => "none",
        Codec::Rle => "rle",
        Codec::Deflate => "deflate",
        Codec::Other(code) => {
            return object(&[("kind", "other".into()), ("code", code.into())]);
        }
    };
    object(&[("kind", kind.into())])
}

fn eos_object(eos: &EosObject) -> Result<JsValue, JsValue> {
    let dims: Array = eos
        .dims
        .iter()
        .map(|(name, size)| Array::of2(&JsValue::from_str(name), &unsigned(*size)))
        .collect();
    let params: Array = eos
        .params
        .iter()
        .map(|(name, parameter)| {
            Array::of2(&JsValue::from_str(name), &JsValue::from_str(parameter))
        })
        .collect();
    let fields = eos
        .fields
        .iter()
        .map(|field| {
            let dataset = field.dataset.map_or(Ok(JsValue::NULL), |index| {
                u64::try_from(index).map(unsigned).map_err(js_error)
            })?;
            object(&[
                ("name", field.name.as_str().into()),
                ("dims", strings(&field.dims).into()),
                ("geolocation", field.geolocation.into()),
                ("dataset", dataset),
            ])
        })
        .collect::<Result<Array, _>>()?;
    let maps = eos
        .dimension_maps
        .iter()
        .map(|map| {
            object(&[
                ("geo_dim", map.geo_dim.as_str().into()),
                ("data_dim", map.data_dim.as_str().into()),
                ("offset", signed(map.offset)),
                ("increment", signed(map.increment)),
            ])
        })
        .collect::<Result<Array, _>>()?;
    object(&[
        ("name", eos.name.as_str().into()),
        ("dims", dims.into()),
        ("fields", fields.into()),
        ("dimension_maps", maps.into()),
        ("params", params.into()),
    ])
}
