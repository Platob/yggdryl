//! Python's native view of the shared [`SpillOptions`]: the bound a column
//! stays resident under and the folder it spills to.
//!
//! [`PySpillOptions`] owns only the core value and is immutable, so it
//! compares, hashes and pickles by the bound and the folder's URL, which is
//! what the core's equality reads. A folder crosses through the package's
//! own doors: a `LocalFolder` handle as it is, a `LocalPath` as the
//! directory it names, and any location spelling - a path, a `file:` URL, a
//! `Url` - through the one URL parser, so no path is split here.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::PyBool;
use yggdryl::holder::Holder;
use yggdryl::local::LocalFolder;
use yggdryl::{DEFAULT_SPILL_BYTE_SIZE, SpillOptions};

use crate::iobase::PyIOBase;
use crate::iomedia::type_name;
use crate::{python_hash, value_error};

/// The bound a column stays resident under, and the folder it spills to.
/// Immutable.
#[pyclass(
    name = "SpillOptions",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PySpillOptions {
    pub(crate) inner: SpillOptions,
}

impl PySpillOptions {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: SpillOptions) -> Self {
        Self { inner }
    }
}

/// The local folder `value` names: a `LocalFolder` handle, a `LocalPath`
/// as the directory it is, or a location spelling read by the one URL
/// parser - a path, a `file:` URL, a `Url` - which a URL of another scheme
/// refuses by name.
pub(crate) fn local_folder_of(value: &Bound<'_, PyAny>) -> PyResult<LocalFolder> {
    if let Ok(handle) = value.extract::<PyRef<'_, PyIOBase>>() {
        return match handle.inner()? {
            Holder::LocalFolder(folder) => Ok(folder.clone()),
            Holder::LocalPath(path) => path.as_directory().map_err(value_error),
            _ => Err(PyTypeError::new_err(format!(
                "expected a LocalFolder, a path or a file URL as the spill folder, got {}",
                type_name(value)
            ))),
        };
    }
    let url = crate::uri::core_url_from_value(value)?;
    LocalFolder::from_url(url).map_err(value_error)
}

/// `options` - else the process default, `SpillOptions.from_env()` - with
/// `byte_size` and `folder` each set on a copy where given: `...` is a
/// keyword not given, and a `folder` of `None` is the platform temporary
/// folder.
pub(crate) fn spill_options_of(
    options: Option<&PySpillOptions>,
    byte_size: &Bound<'_, PyAny>,
    folder: &Bound<'_, PyAny>,
) -> PyResult<SpillOptions> {
    let ellipsis = byte_size.py().Ellipsis();
    let mut options = match options {
        Some(options) => options.inner.clone(),
        None => SpillOptions::from_env().map_err(value_error)?.clone(),
    };
    if !byte_size.is(&ellipsis) {
        options = options.with_byte_size(byte_size.extract()?);
    }
    if !folder.is(&ellipsis) {
        options = with_folder(options, folder)?;
    }
    Ok(options)
}

/// `options` over the folder `folder` names, or over the platform
/// temporary folder for `None`.
fn with_folder(options: SpillOptions, folder: &Bound<'_, PyAny>) -> PyResult<SpillOptions> {
    if folder.is_none() {
        // The core states a folder and never clears one, so the bound is
        // carried onto options that state none.
        return Ok(SpillOptions::new().with_byte_size(options.byte_size()));
    }
    Ok(options.with_folder(local_folder_of(folder)?))
}

/// Hash `options` over what its equality reads: the bound and the folder's
/// URL.
pub(crate) fn hash_spill_options(options: &SpillOptions, state: &mut impl Hasher) {
    options.byte_size().hash(state);
    options.folder().map(LocalFolder::url).hash(state);
}

#[pymethods]
impl PySpillOptions {
    /// The bound never spilling anything.
    #[classattr]
    const NEVER: u64 = SpillOptions::NEVER;

    /// A column spills past `byte_size` resident bytes - `NEVER` spills
    /// nothing and `0` everything - into private files under `folder`: a
    /// `LocalFolder`, a path or a `file:` URL, and the platform temporary
    /// folder for `None`.
    #[new]
    #[pyo3(
        signature = (byte_size = DEFAULT_SPILL_BYTE_SIZE, folder = None),
        // The core's default, spelled for `inspect`: an expression default
        // renders as `...`. The binding's tests pin it to the constant.
        text_signature = "(byte_size=67108864, folder=None)"
    )]
    fn new(byte_size: u64, folder: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let options = SpillOptions::new().with_byte_size(byte_size);
        Ok(Self::from_core(match folder {
            Some(folder) => with_folder(options, folder)?,
            None => options,
        }))
    }

    /// The options the process environment states, read once:
    /// `YGGDRYL_SPILL_BYTE_SIZE` (a byte count, or `never`) and
    /// `YGGDRYL_SPILL_FOLDER` (a path), either unset or empty the default.
    /// Every door that lays a column out settles under it. A refused value
    /// raises naming its variable and leaves the default unresolved.
    #[staticmethod]
    fn from_env() -> PyResult<Self> {
        SpillOptions::from_env()
            .map(|options| Self::from_core(options.clone()))
            .map_err(value_error)
    }

    /// State the process default before anything reads it; refused once it
    /// has been read or installed, so a value every caller saw never
    /// changes under them.
    #[staticmethod]
    #[expect(clippy::needless_pass_by_value)] // PyO3 hands a borrowed class over as `PyRef`.
    fn install_env(options: PyRef<'_, Self>) -> PyResult<()> {
        SpillOptions::install_env(options.inner.clone()).map_err(value_error)
    }

    /// The resident bytes a column may hold before it spills.
    #[getter]
    fn byte_size(&self) -> u64 {
        self.inner.byte_size()
    }

    /// The folder spill files are created in, `None` for the platform
    /// temporary folder.
    #[getter]
    fn folder(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        self.inner
            .folder()
            .map(|folder| crate::iobase::describe(py, Holder::LocalFolder(folder.clone())))
            .transpose()
    }

    /// Whether the bound is `NEVER`.
    fn is_never(&self) -> bool {
        self.inner.is_never()
    }

    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> Py<PyAny> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return py.NotImplemented();
        };
        PyBool::new(py, self.inner == other.inner)
            .to_owned()
            .into_any()
            .unbind()
    }

    /// Hashes the bound and the folder's URL, which equal options share.
    fn __hash__(&self) -> isize {
        let mut state = DefaultHasher::new();
        hash_spill_options(&self.inner, &mut state);
        python_hash(state.finish())
    }

    fn __repr__(&self) -> String {
        match self.inner.folder() {
            Some(folder) => format!(
                "SpillOptions(byte_size={}, folder={:?})",
                self.inner.byte_size(),
                folder.url().to_string()
            ),
            None => format!("SpillOptions(byte_size={})", self.inner.byte_size()),
        }
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (u64, Option<String>)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (
                self.inner.byte_size(),
                self.inner.folder().map(|folder| folder.url().to_string()),
            ),
        )
    }
}
