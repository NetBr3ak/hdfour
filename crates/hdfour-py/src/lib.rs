//! Read-only Python bindings over the core file API.

mod eos;
mod values;

use std::path::PathBuf;
use std::sync::Arc;

use hdfour::{Codec, DType, Error, Storage};
use pyo3::exceptions::{PyKeyError, PyNotImplementedError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

pyo3::create_exception!(hdfour, FormatError, PyValueError);
pyo3::create_exception!(hdfour, UnsupportedError, PyNotImplementedError);
pyo3::create_exception!(hdfour, LimitError, PyValueError);

fn python_error(error: Error) -> PyErr {
    let message = error.to_string();
    match error {
        Error::Io(error) => error.into(),
        Error::NotHdf4 | Error::Malformed { .. } => FormatError::new_err(message),
        Error::Unsupported { .. } => UnsupportedError::new_err(message),
        Error::LimitExceeded { .. } => LimitError::new_err(message),
        Error::NotFound(_) | Error::Ambiguous { .. } => PyKeyError::new_err(message),
        _ => PyRuntimeError::new_err(message),
    }
}

/// File metadata and a shared source retained by every dataset handle.
#[pyclass(name = "File", module = "hdfour", frozen)]
struct PyFile {
    file: Arc<hdfour::File>,
}

#[pymethods]
impl PyFile {
    #[getter]
    fn datasets(&self) -> Vec<PyDataset> {
        (0..self.file.datasets().len())
            .map(|index| PyDataset {
                file: Arc::clone(&self.file),
                index,
            })
            .collect()
    }

    #[getter]
    fn attrs<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        values::attributes(py, self.file.attributes())
    }

    #[getter]
    fn skipped(&self) -> Vec<(&str, &str)> {
        self.file
            .skipped()
            .iter()
            .map(|item| (item.object.as_str(), item.feature))
            .collect()
    }

    #[getter]
    fn eos<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        self.file
            .eos()
            .map(|value| eos::metadata(py, value))
            .transpose()
    }

    fn __getitem__(&self, name: &str) -> PyResult<PyDataset> {
        let dataset = self.file.dataset(name).map_err(python_error)?;
        let index = self
            .file
            .datasets()
            .iter()
            .position(|item| std::ptr::eq(item, dataset))
            .ok_or_else(|| PyRuntimeError::new_err("dataset does not belong to its file"))?;
        Ok(PyDataset {
            file: Arc::clone(&self.file),
            index,
        })
    }
}

/// A dataset index keeps the core metadata's identity intact for reads.
#[pyclass(name = "Dataset", module = "hdfour", frozen)]
struct PyDataset {
    file: Arc<hdfour::File>,
    index: usize,
}

impl PyDataset {
    fn metadata(&self) -> PyResult<&hdfour::Dataset> {
        self.file
            .datasets()
            .get(self.index)
            .ok_or_else(|| PyRuntimeError::new_err("dataset index is outside its file"))
    }
}

#[pymethods]
impl PyDataset {
    #[getter]
    fn name(&self) -> PyResult<&str> {
        Ok(&self.metadata()?.name)
    }

    #[getter]
    fn dtype(&self) -> PyResult<&'static str> {
        Ok(match self.metadata()?.dtype {
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
        })
    }

    #[getter]
    fn shape(&self) -> PyResult<Vec<u64>> {
        Ok(self.metadata()?.shape.clone())
    }

    #[getter]
    fn dims(&self) -> PyResult<Vec<PyDimension>> {
        Ok(self
            .metadata()?
            .dims
            .iter()
            .cloned()
            .map(|dimension| PyDimension { dimension })
            .collect())
    }

    #[getter]
    fn attrs<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        values::attributes(py, &self.metadata()?.attributes)
    }

    #[getter]
    fn is_coordinate(&self) -> PyResult<bool> {
        Ok(self.metadata()?.is_coordinate)
    }

    #[getter]
    fn storage<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let result = PyDict::new(py);
        let kind = match &self.metadata()?.storage {
            Storage::Empty => "empty",
            Storage::Contiguous => "contiguous",
            Storage::Linked => "linked",
            Storage::External => "external",
            Storage::Chunked { chunk_shape } => {
                result.set_item("chunk_shape", chunk_shape)?;
                "chunked"
            }
        };
        result.set_item("kind", kind)?;
        Ok(result)
    }

    #[getter]
    fn codec<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let name = match self.metadata()?.codec {
            Codec::None => "none",
            Codec::Rle => "rle",
            Codec::Deflate => "deflate",
            Codec::Other(code) => return Ok(code.into_pyobject(py)?.into_any()),
        };
        Ok(name.into_pyobject(py)?.into_any())
    }

    /// Decode raw values into a NumPy allocation without holding the GIL.
    fn read<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let dataset = self.metadata()?;
        // Limits and format are checked here, before NumPy allocates anything for a hostile shape.
        py.detach(|| self.file.byte_len(dataset))
            .map_err(python_error)?;
        values::read(py, &self.file, dataset)
    }
}

/// A named axis with raw attributes, including unlimited-axis metadata.
#[pyclass(name = "Dimension", module = "hdfour", frozen)]
struct PyDimension {
    dimension: hdfour::Dimension,
}

#[pymethods]
impl PyDimension {
    #[getter]
    fn name(&self) -> &str {
        &self.dimension.name
    }

    #[getter]
    fn size(&self) -> u64 {
        self.dimension.size
    }

    #[getter]
    fn unlimited(&self) -> bool {
        self.dimension.unlimited
    }

    #[getter]
    fn attrs<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        values::attributes(py, &self.dimension.attributes)
    }
}

/// Open a path and read metadata; array payloads remain lazy.
#[pyfunction]
fn open(py: Python<'_>, path: PathBuf) -> PyResult<PyFile> {
    let file = py.detach(|| hdfour::open(path)).map_err(python_error)?;
    Ok(PyFile {
        file: Arc::new(file),
    })
}

/// Own the supplied bytes and read metadata without retaining Python objects.
#[pyfunction]
fn from_bytes(py: Python<'_>, data: Vec<u8>) -> PyResult<PyFile> {
    let file = py
        .detach(|| hdfour::File::from_bytes(data))
        .map_err(python_error)?;
    Ok(PyFile {
        file: Arc::new(file),
    })
}

#[pymodule]
fn _hdfour(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyFile>()?;
    module.add_class::<PyDataset>()?;
    module.add_class::<PyDimension>()?;
    module.add("FormatError", module.py().get_type::<FormatError>())?;
    module.add(
        "UnsupportedError",
        module.py().get_type::<UnsupportedError>(),
    )?;
    module.add("LimitError", module.py().get_type::<LimitError>())?;
    module.add_function(wrap_pyfunction!(open, module)?)?;
    module.add_function(wrap_pyfunction!(from_bytes, module)?)?;
    Ok(())
}
