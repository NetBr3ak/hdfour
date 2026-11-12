//! Python binding entry point; dataset bindings follow in batch B7.

use pyo3::prelude::*;

#[pymodule]
fn _hdfour(_module: &Bound<'_, PyModule>) -> PyResult<()> {
    Ok(())
}
