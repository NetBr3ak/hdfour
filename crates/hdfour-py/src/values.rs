//! NumPy allocations for datasets and ownership-moving conversion for attributes.

use hdfour::{Attribute, DType, Dataset, File, Value};
use numpy::{Element, IntoPyArray, PyArray, PyArrayMethods, ndarray::IxDyn};
use pyo3::exceptions::PyOverflowError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyModule};

pub(crate) fn read<'py>(
    py: Python<'py>,
    file: &File,
    dataset: &Dataset,
) -> PyResult<Bound<'py, PyAny>> {
    let shape = dataset
        .shape
        .iter()
        .map(|size| {
            usize::try_from(*size)
                .map_err(|_| PyOverflowError::new_err("array dimension exceeds usize"))
        })
        .collect::<PyResult<Vec<_>>>()?;
    match dataset.dtype {
        DType::Char8 | DType::I8 => read_numeric::<i8>(py, file, dataset, &shape),
        DType::UChar8 | DType::U8 => read_numeric::<u8>(py, file, dataset, &shape),
        DType::I16 => read_numeric::<i16>(py, file, dataset, &shape),
        DType::U16 => read_numeric::<u16>(py, file, dataset, &shape),
        DType::I32 => read_numeric::<i32>(py, file, dataset, &shape),
        DType::U32 => read_numeric::<u32>(py, file, dataset, &shape),
        DType::F32 => read_numeric::<f32>(py, file, dataset, &shape),
        DType::F64 => read_numeric::<f64>(py, file, dataset, &shape),
    }
}

fn read_numeric<'py, T: Element + bytemuck::Pod>(
    py: Python<'py>,
    file: &File,
    dataset: &Dataset,
    shape: &[usize],
) -> PyResult<Bound<'py, PyAny>> {
    let array = PyModule::import(py, "numpy")?
        .call_method1("zeros", (shape, T::get_dtype(py)))?
        .cast_into::<PyArray<T, IxDyn>>()?;
    {
        let mut borrowed = array.try_readwrite()?;
        let output = bytemuck::must_cast_slice_mut::<T, u8>(borrowed.as_slice_mut()?);
        py.detach(|| file.read_into(dataset, output))
            .map_err(super::python_error)?;
    }
    Ok(array.into_any())
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

pub(crate) fn array<'py>(py: Python<'py>, array: hdfour::Array) -> PyResult<Bound<'py, PyAny>> {
    let shape = array
        .shape
        .iter()
        .map(|size| {
            usize::try_from(*size)
                .map_err(|_| PyOverflowError::new_err("array dimension exceeds usize"))
        })
        .collect::<PyResult<Vec<_>>>()?;
    values(py, array.data, &shape)
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
