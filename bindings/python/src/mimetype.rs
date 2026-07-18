//! The `yggdryl.mimetype` submodule — one media type (`type/subtype`) with its known
//! extensions and magic-byte signatures, plus the [`MimeCatalog`] registry that resolves a
//! [`MimeType`] from a mime string, a file name, an extension, or a file's magic bytes.
//!
//! Mirrors `yggdryl_core::mimetype`'s root-level [`MimeType`] and the concrete
//! [`MimeCatalog`] (the `MimeRegistry` trait itself is not mirrored — the binding exposes the
//! catalog). A [`MimeType`] is a value type: equal, hashable, byte-serializable (its **essence
//! bytes** — two `MimeType`s with the same essence are the same type), and picklable (through
//! its `essence` / `extensions` / `magic`, so a catalog entry round-trips faithfully). A bad
//! mime string handed to `parse` / `deserialize_bytes` raises a guided `ValueError`.

// `useless_conversion`: pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type
// `From`. `wrong_self_convention`: the `MimeCatalog::from_*` lookups resolve *from* a key
// *against this registry* (a map-style accessor taking `&self`), mirroring the core's
// `MimeRegistry` trait, which carries the same allow.
#![allow(clippy::useless_conversion, clippy::wrong_self_convention)]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use yggdryl_core::io::IoError;
use yggdryl_core::mimetype::{self, MimeRegistry};

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn ioerr(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// The constructor arguments a [`MimeType`] pickles through:
/// `(essence, extensions, magic, names)`.
type MimeTypeArgs<'py> = (String, Vec<String>, Vec<Bound<'py, PyBytes>>, Vec<String>);

/// One media type: a lowercased `type/subtype` **essence** (the mime string without
/// parameters), the file **extensions** it is known by, and the **magic-byte** signatures a
/// file of this type begins with. A value type — equal, hashable, and byte-serializable.
#[pyclass(module = "yggdryl.mimetype")]
#[derive(Clone)]
pub struct MimeType {
    pub(crate) inner: mimetype::MimeType,
}

#[pymethods]
impl MimeType {
    /// Builds a media type from its `essence` (`type/subtype`), known `extensions` (no dot),
    /// `magic` signatures (list of `bytes`), and short `names` / aliases (`["gzip"]`). The
    /// essence and names are lowercased; extensions are lowercased and stripped of a leading
    /// dot.
    #[new]
    #[pyo3(signature = (essence, extensions = None, magic = None, names = None))]
    fn new(
        essence: &str,
        extensions: Option<Vec<String>>,
        magic: Option<Vec<Vec<u8>>>,
        names: Option<Vec<String>>,
    ) -> Self {
        Self {
            inner: mimetype::MimeType::named(
                essence,
                names.unwrap_or_default(),
                extensions.unwrap_or_default(),
                magic.unwrap_or_default(),
            ),
        }
    }

    /// Parses a mime string (`type/subtype` with optional `;`-separated parameters, which are
    /// dropped), returning its **essence** with no extensions or magic. Case-insensitive.
    /// Raises a guided `ValueError` when the string is not a `type/subtype` essence.
    #[staticmethod]
    fn parse(s: &str) -> PyResult<Self> {
        mimetype::MimeType::parse_str(s)
            .map(|inner| Self { inner })
            .map_err(ioerr)
    }

    /// The `application/octet-stream` fallback — an opaque byte stream of unknown type.
    #[staticmethod]
    fn octet_stream() -> Self {
        Self {
            inner: mimetype::MimeType::octet_stream(),
        }
    }

    /// Resolves a media type from a file **extension** (no dot) via the default catalog, or
    /// `None` if unknown.
    #[staticmethod]
    fn from_extension(ext: &str) -> Option<Self> {
        mimetype::MimeType::from_extension(ext).map(|inner| Self { inner })
    }

    /// Resolves a media type from a **file name** (its last extension) via the default
    /// catalog, or `None`.
    #[staticmethod]
    fn from_name(name: &str) -> Option<Self> {
        mimetype::MimeType::from_name(name).map(|inner| Self { inner })
    }

    /// Resolves a media type from a short **name** / alias (`"gzip"`) via the default catalog,
    /// or `None`.
    #[staticmethod]
    fn from_alias(name: &str) -> Option<Self> {
        mimetype::MimeType::from_alias(name).map(|inner| Self { inner })
    }

    /// Resolves a media type from the **magic bytes** at the start of a file via the default
    /// catalog, or `None`.
    #[staticmethod]
    fn from_magic(head: &[u8]) -> Option<Self> {
        mimetype::MimeType::from_magic(head).map(|inner| Self { inner })
    }

    /// The **best guess** for a file `name` (with optional `head` bytes): magic bytes win when
    /// they match, then the name's extension, else `octet_stream` — always an answer.
    #[staticmethod]
    fn guess(name: &str, head: &[u8]) -> Self {
        Self {
            inner: mimetype::MimeType::guess(name, head),
        }
    }

    /// The `type/subtype` essence, e.g. `"application/json"`.
    #[getter]
    fn essence(&self) -> String {
        self.inner.essence().to_string()
    }

    /// The top-level type, e.g. `"application"` of `"application/json"`.
    #[getter]
    #[pyo3(name = "type")]
    fn type_(&self) -> String {
        self.inner.type_().to_string()
    }

    /// The subtype, e.g. `"json"` of `"application/json"`.
    #[getter]
    fn subtype(&self) -> String {
        self.inner.subtype().to_string()
    }

    /// The known file extensions (lowercase, no dot).
    #[getter]
    fn extensions(&self) -> Vec<String> {
        self.inner.extensions().to_vec()
    }

    /// The **primary** extension (the first), or `None` when the type has none.
    #[getter]
    fn extension(&self) -> Option<String> {
        self.inner.extension().map(str::to_string)
    }

    /// The short **names** / aliases this type is known by (`["gzip"]` for
    /// `application/gzip`).
    #[getter]
    fn names(&self) -> Vec<String> {
        self.inner.names().to_vec()
    }

    /// The magic-byte signatures a file of this type starts with, as a list of `bytes`.
    #[getter]
    fn magic<'py>(&self, py: Python<'py>) -> Vec<Bound<'py, PyBytes>> {
        self.inner
            .magic()
            .iter()
            .map(|sig| PyBytes::new_bound(py, sig))
            .collect()
    }

    /// Whether this type is registered under `ext` (case-insensitive, leading dot ignored).
    fn has_extension(&self, ext: &str) -> bool {
        self.inner.has_extension(ext)
    }

    /// Whether `head` (the start of a file) begins with one of this type's magic signatures.
    fn matches_magic(&self, head: &[u8]) -> bool {
        self.inner.matches_magic(head)
    }

    /// Whether this type is a **compression** format (gzip / zstd / xz-lzma / zlib) — whether a
    /// source of this type can be run through a `yggdryl.compression` codec.
    fn is_compression(&self) -> bool {
        self.inner.is_compression()
    }

    /// Whether this is the `application/octet-stream` fallback.
    fn is_octet_stream(&self) -> bool {
        self.inner.is_octet_stream()
    }

    /// An explicit copy of this media type (equivalent to `copy.copy(mime)`).
    fn copy(&self) -> Self {
        self.clone()
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// The value form — the **essence bytes** (the mime string). Extensions and magic are
    /// catalog metadata, not part of the byte identity.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(
            py,
            &yggdryl_core::io::Serializable::serialize_bytes(&self.inner),
        )
    }

    /// Reconstructs a media type from the essence bytes produced by `serialize_bytes` (an
    /// essence-only value, no extensions or magic), raising a guided `ValueError` on non-UTF-8
    /// bytes or a bad essence.
    #[staticmethod]
    fn deserialize_bytes(data: &[u8]) -> PyResult<Self> {
        <mimetype::MimeType as yggdryl_core::io::Serializable>::deserialize_bytes(data)
            .map(|inner| Self { inner })
            .map_err(ioerr)
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    /// Pickles through the constructor components (`essence`, `extensions`, `magic`, `names`),
    /// so a catalog entry with its extensions, magic, and names round-trips faithfully — unlike
    /// the essence-only byte codec.
    fn __reduce__<'py>(&self, py: Python<'py>) -> PyResult<(Py<PyAny>, MimeTypeArgs<'py>)> {
        let ctor = py.get_type_bound::<MimeType>().into_any().unbind();
        let magic = self
            .inner
            .magic()
            .iter()
            .map(|sig| PyBytes::new_bound(py, sig))
            .collect();
        Ok((
            ctor,
            (
                self.inner.essence().to_string(),
                self.inner.extensions().to_vec(),
                magic,
                self.inner.names().to_vec(),
            ),
        ))
    }

    /// The essence string, e.g. `"application/json"`.
    fn __str__(&self) -> String {
        self.inner.essence().to_string()
    }

    fn __repr__(&self) -> String {
        format!("MimeType({:?})", self.inner.essence())
    }
}

/// A registry of known [`MimeType`]s — resolves a `MimeType` from a mime string, a file name,
/// an extension, or the magic bytes of a file's head. Small and linearly scanned; seed it with
/// the built-in known types via [`defaults`](MimeCatalog::defaults) or start empty and
/// [`register`](MimeCatalog::register) your own.
#[pyclass(module = "yggdryl.mimetype")]
#[derive(Clone)]
pub struct MimeCatalog {
    pub(crate) inner: mimetype::MimeCatalog,
}

#[pymethods]
impl MimeCatalog {
    /// An empty catalog.
    #[new]
    fn new() -> Self {
        Self {
            inner: mimetype::MimeCatalog::new(),
        }
    }

    /// A catalog seeded with the **built-in known types** — the common web / data / archive /
    /// image formats, with their extensions and (where distinctive) magic signatures.
    #[staticmethod]
    fn defaults() -> Self {
        Self {
            inner: mimetype::MimeCatalog::defaults(),
        }
    }

    /// Registers `mime`, overriding any earlier entry with the same essence (later
    /// registration wins). In-place; [`with_`](MimeCatalog::with_) is the chainable builder.
    fn register(&mut self, mime: &MimeType) {
        self.inner.register(mime.inner.clone());
    }

    /// Returns a copy of this catalog with `mime` registered — the chainable, non-mutating
    /// builder (`catalog.with_(a).with_(b)`). Named `with_` because `with` is a Python keyword.
    fn with_(&self, mime: &MimeType) -> Self {
        Self {
            inner: self.inner.clone().with(mime.inner.clone()),
        }
    }

    /// The registered types, in registration order.
    fn types(&self) -> Vec<MimeType> {
        self.inner
            .types()
            .iter()
            .map(|inner| MimeType {
                inner: inner.clone(),
            })
            .collect()
    }

    /// The number of registered types.
    fn len(&self) -> usize {
        self.inner.len()
    }

    /// The registered type whose essence equals the parsed mime string, or `None`.
    fn from_mime(&self, mime: &str) -> Option<MimeType> {
        self.inner.from_mime(mime).map(|inner| MimeType { inner })
    }

    /// The registered type known by `ext` (no dot, case-insensitive), or `None`.
    fn from_extension(&self, ext: &str) -> Option<MimeType> {
        self.inner
            .from_extension(ext)
            .map(|inner| MimeType { inner })
    }

    /// The registered type for a **file name** — its last extension is looked up. `None` when
    /// the name has no extension or the extension is unknown.
    fn from_name(&self, name: &str) -> Option<MimeType> {
        self.inner.from_name(name).map(|inner| MimeType { inner })
    }

    /// The registered type whose magic signature prefixes `head` (longest wins), or `None`.
    fn from_magic(&self, head: &[u8]) -> Option<MimeType> {
        self.inner.from_magic(head).map(|inner| MimeType { inner })
    }

    /// An explicit copy of this catalog (equivalent to `copy.copy(catalog)`).
    fn copy(&self) -> Self {
        self.clone()
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// The number of registered types (so `len(catalog)` works).
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Truthiness — `True` when the catalog has at least one registered type.
    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    fn __repr__(&self) -> String {
        format!("MimeCatalog(<{} types>)", self.inner.len())
    }
}

/// Populates the `mimetype` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<MimeType>()?;
    module.add_class::<MimeCatalog>()?;
    Ok(())
}
