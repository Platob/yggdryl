//! The content coding a handle presents the decoded bytes of, one class each.
//!
//! A coded handle reads decoded and writes encoded, so everything downstream -
//! a record encoding, a codec, another handle - sees plain bytes. The class
//! names which framing is being removed; `IOBase.codec` still answers it as
//! text, and `isinstance(handle, Coded)` answers it without naming one.

use pyo3::prelude::*;

use crate::iobase::PyIOBase;

/// The handles that present decoded bytes, whatever the framing.
#[pyclass(
    name = "Coded",
    module = "yggdryl._native",
    extends = PyIOBase,
    subclass,
    skip_from_py_object
)]
pub(crate) struct PyCoded;

/// Declare one content coding below [`PyCoded`].
macro_rules! coding {
    ($ident:ident, $name:literal, $doc:expr) => {
        #[doc = $doc]
        #[pyclass(name = $name, module = "yggdryl._native", extends = PyCoded, skip_from_py_object)]
        pub(crate) struct $ident;
    };
}

coding!(
    PyIdentity,
    "Identity",
    "Bytes that pass through unchanged: the coding a name declaring none takes."
);
coding!(PyGzip, "Gzip", "RFC 1952 gzip framing over DEFLATE.");
coding!(
    PyZlib,
    "Zlib",
    "RFC 1950 zlib framing over DEFLATE, which is also where raw DEFLATE lands."
);
coding!(PyZstd, "Zstd", "RFC 8878 Zstandard.");

#[pymethods]
impl PyCoded {
    /// The handle holding the *encoded* bytes, as its own role.
    ///
    /// Any pending write is published on the way out, so the value the coding
    /// stands on is complete. This handle is spent: a coding owns the handle
    /// it codes, so descending moves the value to the one this answers with.
    fn into_handle(mut slf: PyRefMut<'_, Self>, py: Python<'_>) -> PyResult<Py<PyAny>> {
        crate::iobase::unwrapped(py, slf.as_super())
    }
}

/// Build the class one coding names, over an already-coded holder.
pub(crate) fn describe(
    py: Python<'_>,
    base: PyClassInitializer<PyIOBase>,
    codec: yggdryl::Codec,
) -> PyResult<Py<PyAny>> {
    let role = base.add_subclass(PyCoded);
    Ok(match codec {
        yggdryl::Codec::Gzip => Py::new(py, role.add_subclass(PyGzip))?.into_any(),
        yggdryl::Codec::Zlib | yggdryl::Codec::Deflate => {
            Py::new(py, role.add_subclass(PyZlib))?.into_any()
        }
        yggdryl::Codec::Zstd => Py::new(py, role.add_subclass(PyZstd))?.into_any(),
        yggdryl::Codec::Identity => Py::new(py, role.add_subclass(PyIdentity))?.into_any(),
        // A coding this build has no class for is still a coded handle, and
        // reporting it as one of the others would be a lie.
        _ => Py::new(py, role)?.into_any(),
    })
}

/// Register every content coding.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyCoded>()?;
    module.add_class::<PyIdentity>()?;
    module.add_class::<PyGzip>()?;
    module.add_class::<PyZlib>()?;
    module.add_class::<PyZstd>()?;
    Ok(())
}
