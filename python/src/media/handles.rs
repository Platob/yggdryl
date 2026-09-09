//! The record implementation a handle retains, one class per encoding.
//!
//! A handle named by a record encoding arrives already holding that encoding's
//! implementation, so the schema, footer, and dimension caches an `open` fills
//! have somewhere to live. The class names the encoding; `record_options()`
//! still answers the settings, and `isinstance(handle, Media)` answers "this
//! reads rows" without naming one.

use pyo3::prelude::*;

use crate::iobase::PyIOBase;

/// The handles that retain a record implementation.
#[pyclass(
    name = "Media",
    module = "yggdryl._native",
    extends = PyIOBase,
    subclass,
    skip_from_py_object
)]
pub(crate) struct PyMedia;

/// Declare one record encoding below [`PyMedia`].
macro_rules! encoding {
    ($ident:ident, $name:literal, $doc:expr) => {
        #[doc = $doc]
        #[pyclass(name = $name, module = "yggdryl._native", extends = PyMedia, skip_from_py_object)]
        pub(crate) struct $ident;
    };
}

encoding!(PyIpc, "Ipc", "An Arrow IPC stream or file.");
encoding!(PyParquet, "Parquet", "An Apache Parquet file.");
encoding!(PyAvro, "Avro", "An Apache Avro object container.");
encoding!(PyXml, "Xml", "An XML document read and written as rows.");

/// Plain-text rows under one retained flat configuration.
///
/// This is what a `text/plain` name composes to, and it is what keeps a
/// `TextOptions` alive across calls: the options describe the *decoded* rows,
/// so a coding always sits underneath this handle rather than over it.
#[pyclass(
    name = "Text",
    module = "yggdryl._native",
    extends = PyIOBase,
    skip_from_py_object
)]
pub(crate) struct PyText;

#[pymethods]
impl PyMedia {
    /// The byte handle this encoding reads and writes through.
    ///
    /// This handle is spent: a record implementation owns the handle it reads,
    /// so descending moves the value to the one this answers with.
    fn into_handle(mut slf: PyRefMut<'_, Self>, py: Python<'_>) -> PyResult<Py<PyAny>> {
        crate::iobase::unwrapped(py, slf.as_super())
    }
}

#[pymethods]
impl PyText {
    /// The byte handle these rows are read out of, as its own role.
    ///
    /// For `trades.txt.gz` that is the `Gzip` presenting the decoded bytes,
    /// and its own `into_handle` is the location underneath. This handle is
    /// spent, as it is for every wrapper.
    fn into_handle(mut slf: PyRefMut<'_, Self>, py: Python<'_>) -> PyResult<Py<PyAny>> {
        crate::iobase::unwrapped(py, slf.as_super())
    }
}

/// Which record implementation a media value holds.
#[derive(Clone, Copy)]
pub(crate) enum Encoding {
    Ipc,
    Parquet,
    Avro,
    Text,
    Xml,
}

impl Encoding {
    /// Read the encoding off a media value before it is moved.
    pub(crate) const fn of(media: &yggdryl::media::Media) -> Self {
        match media {
            yggdryl::media::Media::Ipc(_) => Self::Ipc,
            yggdryl::media::Media::Parquet(_) => Self::Parquet,
            yggdryl::media::Media::Avro(_) => Self::Avro,
            yggdryl::media::Media::Text(_) => Self::Text,
            yggdryl::media::Media::Xml(_) => Self::Xml,
        }
    }
}

/// Build the class one record media names.
pub(crate) fn describe(
    py: Python<'_>,
    base: PyClassInitializer<PyIOBase>,
    encoding: Encoding,
) -> PyResult<Py<PyAny>> {
    // Text is the one encoding with a holder variant of its own, so it is the
    // one class that does not sit below `Media`.
    if matches!(encoding, Encoding::Text) {
        return describe_text(py, base);
    }
    let media = base.add_subclass(PyMedia);
    Ok(match encoding {
        Encoding::Ipc => Py::new(py, media.add_subclass(PyIpc))?.into_any(),
        Encoding::Parquet => Py::new(py, media.add_subclass(PyParquet))?.into_any(),
        Encoding::Xml => Py::new(py, media.add_subclass(PyXml))?.into_any(),
        Encoding::Avro | Encoding::Text => Py::new(py, media.add_subclass(PyAvro))?.into_any(),
    })
}

/// Build the plain-text class over an already-retained text holder.
pub(crate) fn describe_text(
    py: Python<'_>,
    base: PyClassInitializer<PyIOBase>,
) -> PyResult<Py<PyAny>> {
    Ok(Py::new(py, base.add_subclass(PyText))?.into_any())
}

/// Register every record implementation.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyMedia>()?;
    module.add_class::<PyIpc>()?;
    module.add_class::<PyParquet>()?;
    module.add_class::<PyAvro>()?;
    module.add_class::<PyText>()?;
    module.add_class::<PyXml>()?;
    Ok(())
}
