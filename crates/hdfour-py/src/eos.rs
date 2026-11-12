//! Plain Python dictionaries for EOS metadata, without interpreting parameters.

use hdfour::{DimensionMap, Eos, EosField, EosObject};
use pyo3::prelude::*;
use pyo3::types::PyDict;

pub(crate) fn metadata<'py>(py: Python<'py>, eos: &Eos) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("grids", objects(py, &eos.grids)?)?;
    result.set_item("swaths", objects(py, &eos.swaths)?)?;
    Ok(result)
}

fn objects<'py>(py: Python<'py>, objects: &[EosObject]) -> PyResult<Vec<Bound<'py, PyDict>>> {
    objects
        .iter()
        .map(|object| {
            let result = PyDict::new(py);
            result.set_item("name", &object.name)?;
            result.set_item("dims", &object.dims)?;
            result.set_item("params", &object.params)?;
            let fields = object
                .fields
                .iter()
                .map(|value| field(py, value))
                .collect::<PyResult<Vec<_>>>()?;
            let maps = object
                .dimension_maps
                .iter()
                .map(|value| dimension_map(py, value))
                .collect::<PyResult<Vec<_>>>()?;
            result.set_item("fields", fields)?;
            result.set_item("dimension_maps", maps)?;
            Ok(result)
        })
        .collect()
}

fn field<'py>(py: Python<'py>, field: &EosField) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("name", &field.name)?;
    result.set_item("dims", &field.dims)?;
    result.set_item("geolocation", field.geolocation)?;
    result.set_item("dataset", field.dataset)?;
    Ok(result)
}

fn dimension_map<'py>(py: Python<'py>, map: &DimensionMap) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    result.set_item("geo_dim", &map.geo_dim)?;
    result.set_item("data_dim", &map.data_dim)?;
    result.set_item("offset", map.offset)?;
    result.set_item("increment", map.increment)?;
    Ok(result)
}
