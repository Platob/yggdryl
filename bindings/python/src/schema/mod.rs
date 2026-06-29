//! The `yggdryl.schema` submodule — thin wrappers over the `yggdryl-schema` crate.

use pyo3::prelude::*;

mod data_type_id;

pub(crate) use data_type_id::DataTypeId;

/// Populates the `schema` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<DataTypeId>()?;
    Ok(())
}
