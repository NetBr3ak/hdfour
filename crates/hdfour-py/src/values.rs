//! Ownership-moving NumPy conversion for raw values.

use hdfour::{Array, Attribute, Value};
use numpy::{Element, IntoPyArray, PyArrayMethods, ndarray::IxDyn};
use pyo3::exceptions::PyOverflowError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

pub(crate) fn array(py: Python<'_>, array: Array) -> PyResult<Bound<'_, PyAny>> {
    let shape = array
        .shape
        .into_iter()
        .map(|size| {
            usize::try_from(size)
                .map_err(|_| PyOverflowError::new_err("array dimension exceeds usize"))
        })
        .collect::<PyResult<Vec<_>>>()?;
    values(py, array.data, &shape)
}

pub(crate) fn attributes<'py>(
    py: Python<'py>,
    attributes: &[Attribute],
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    for attribute in attributes {
        if let Value::Char8(bytes) = &attribute.value {
            let text: String = bytes.iter().copied().map(char::from).collect();
            result.set_item(&attribute.name, text)?;
            continue;
        }
        let value = attribute.value.clone();
        let length = match &value {
            Value::Char8(data) | Value::UChar8(data) | Value::U8(data) => data.len(),
            Value::I8(data) => data.len(),
            Value::I16(data) => data.len(),
            Value::U16(data) => data.len(),
            Value::I32(data) => data.len(),
            Value::U32(data) => data.len(),
            Value::F32(data) => data.len(),
            Value::F64(data) => data.len(),
        };
        result.set_item(&attribute.name, values(py, value, &[length])?)?;
    }
    Ok(result)
}

fn values<'py>(py: Python<'py>, value: Value, shape: &[usize]) -> PyResult<Bound<'py, PyAny>> {
    match value {
        // Specification section 6: signed character arrays retain the original byte buffer.
        Value::Char8(data) => numeric(py, data, shape)?.call_method1("view", ("int8",)),
        Value::UChar8(data) | Value::U8(data) => numeric(py, data, shape),
        Value::I8(data) => numeric(py, data, shape),
        Value::I16(data) => numeric(py, data, shape),
        Value::U16(data) => numeric(py, data, shape),
        Value::I32(data) => numeric(py, data, shape),
        Value::U32(data) => numeric(py, data, shape),
        Value::F32(data) => numeric(py, data, shape),
        Value::F64(data) => numeric(py, data, shape),
    }
}

fn numeric<'py, T: Element>(
    py: Python<'py>,
    data: Vec<T>,
    shape: &[usize],
) -> PyResult<Bound<'py, PyAny>> {
    Ok(data.into_pyarray(py).reshape(IxDyn(shape))?.into_any())
}
