//! Native Python view of the FIX dictionary, its message, and the default.
//!
//! Nothing here resolves, folds, merges, shards or validates: the registry is
//! one [`Arc`] over the core [`FixRegistry`], and every accessor coerces its
//! key once at the boundary and redirects to the most specific native method.
//! The typed `fix:` vocabulary is not here either - it lives on the protocol
//! view class [`crate::types::field::PyProtocolField`], which is what `field.fix`
//! already answers.
//!
//! A branch and an identifier cross as `str` and are parsed once here through
//! [`branch_from_py`] and [`id_from_py`], so neither gets a class of its own in
//! Python and the grammar, the folding and the standard-tag rule all stay the
//! core's. A bare tag or name uses the core's deterministic best match, and a
//! colon-bearing string is a name, never an identifier.

use std::sync::Arc;

use pyo3::exceptions::{PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyInt};

use yggdryl::{
    DataType as CoreDataType, Error as CoreError, Field as CoreField, FixBranch as CoreFixBranch,
    FixId as CoreFixId, FixKey, FixMsg as CoreFixMsg, FixRegistry as CoreFixRegistry, Scalar,
    from_json_scalar_with_field, into_json_scalar,
};

use crate::enums::PyMimeType;
use crate::media::iceberg::folder_holder_from_value;
use crate::text::codec::with_python_bytes;
use crate::types::field::{PyField, core_field_from_value};
use crate::types::scalar::{PyScalar, from_py};
use crate::value_error;

/// A FIX tag as Python hands one over: an `int` that fits `i32`.
///
/// `bool` is an `int` in Python and never a tag, so it is refused by name
/// rather than silently read as 0 or 1; a value outside `i32` raises the
/// `OverflowError` the extraction itself reports, never a narrowed tag.
#[derive(Clone, Copy)]
pub(crate) struct FixTag(pub(crate) i32);

impl FromPyObject<'_, '_> for FixTag {
    type Error = PyErr;

    fn extract(value: Borrowed<'_, '_, PyAny>) -> PyResult<Self> {
        if value.is_instance_of::<PyBool>() {
            return Err(PyTypeError::new_err(
                "a FIX tag must be an integer, not bool",
            ));
        }
        value.extract::<i32>().map(Self)
    }
}

/// Read one branch, or report the native parse failure as a `ValueError`.
///
/// A branch crosses as text and becomes a `FixBranch` here, once, so no second
/// class exists in Python and the grammar - a leading ASCII letter, no `:` or
/// `,`, at most 23 bytes, ASCII case folded - stays the core's.
pub(crate) fn branch_from_py(text: &str) -> PyResult<CoreFixBranch> {
    CoreFixBranch::from_str(text).map_err(value_error)
}

/// Read one identifier, or report the native parse failure as a `ValueError`.
///
/// The text is `tag:branch`, and `FixId::from_str` is what parses it - the
/// standard-tag rule included, so `35:cme` is refused here exactly as it is in
/// Rust.
pub(crate) fn id_from_py(text: &str) -> PyResult<CoreFixId> {
    CoreFixId::from_str(text).map_err(value_error)
}

/// Retain the branch spelling beside the packed identifier for a field write.
pub(crate) fn id_parts_from_py(text: &str) -> PyResult<(CoreFixBranch, CoreFixId)> {
    let id = id_from_py(text)?;
    let branch = text
        .split_once(':')
        .map(|(_, branch)| branch)
        .ok_or_else(|| PyValueError::new_err("a FIX identifier requires tag:branch"))?;
    Ok((branch_from_py(branch)?, id))
}

/// One lookup key, read once at the boundary.
///
/// An `int` is a tag and a `str` is a name or dotted path, exactly as the
/// core's `FixKey` splits them; a
/// colon-bearing string is a name, never an identifier. The owned name is what
/// lets the borrowed key be rebuilt for each call without the caller's object
/// staying alive.
enum FixKeyArg {
    Tag(i32),
    Name(String),
}

impl FixKeyArg {
    /// Read a key, or report what a FIX lookup accepts.
    fn from_py(key: &Bound<'_, PyAny>) -> PyResult<Self> {
        if key.is_instance_of::<PyBool>() {
            return Err(PyTypeError::new_err(
                "a FIX tag must be an integer, not bool",
            ));
        }
        if key.is_instance_of::<PyInt>() {
            return key.extract::<i32>().map(Self::Tag);
        }
        if let Ok(name) = key.extract::<String>() {
            return Ok(Self::Name(name));
        }
        Err(PyTypeError::new_err(format!(
            "a FIX key must be an int tag or a str name, got {}",
            key.get_type().name()?
        )))
    }

    /// Borrow the key the core matches on.
    fn as_key(&self) -> FixKey<'_> {
        match self {
            Self::Tag(tag) => FixKey::Tag(*tag),
            Self::Name(name) => FixKey::Name(name.as_str()),
        }
    }
}

/// Map a core failure onto the exception its kind means in Python.
///
/// Absence is the mapping protocol's `KeyError` carrying the native message
/// unchanged; everything else keeps the boundary's `ValueError`.
fn absent(error: &CoreError) -> PyErr {
    if error.is_absent() {
        PyKeyError::new_err(error.to_string())
    } else {
        value_error(error)
    }
}

/// FIX field definitions resolved by tag, by name, or by dotted path.
///
/// The registry is mutable, so it is unhashable and compares by the fields it
/// holds. It is held as an `Arc` because a [`FixMsg`][PyFixMsg] links the very
/// registry it was resolved against and the process default is one too: a
/// mutation therefore refuses while anything else shares it, rather than
/// changing a dictionary underneath a message that already used it.
#[pyclass(name = "FixRegistry", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PyFixRegistry {
    pub(crate) inner: Arc<CoreFixRegistry>,
}

impl PyFixRegistry {
    /// Wrap a shared registry, sharing rather than copying it.
    pub(crate) const fn from_arc(inner: Arc<CoreFixRegistry>) -> Self {
        Self { inner }
    }

    /// Borrow the registry for a mutation, refusing a shared one.
    fn inner_mut(&mut self) -> PyResult<&mut CoreFixRegistry> {
        Arc::get_mut(&mut self.inner).ok_or_else(|| {
            PyValueError::new_err(
                "this registry is shared with a message or installed as the process default; build a new one",
            )
        })
    }
}

#[pymethods]
impl PyFixRegistry {
    // A registry is mutable, so it cannot promise a stable hash.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// The empty registry.
    #[new]
    fn new() -> Self {
        Self::from_arc(Arc::new(CoreFixRegistry::new()))
    }

    /// Build a registry by inserting `fields` in order.
    ///
    /// Each entry is anything `Field` accepts - a native field, a field
    /// string, a dataclass, a `PyArrow` field - and the first refusal fails the
    /// whole build.
    #[staticmethod]
    fn from_fields(fields: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut registry = CoreFixRegistry::new();
        for value in fields.try_iter()? {
            registry
                .insert(core_field_from_value(&value?)?)
                .map_err(value_error)?;
        }
        Ok(Self::from_arc(Arc::new(registry)))
    }

    /// Load every shard under `<location>/primitive` and `<location>/nested`.
    ///
    /// `location` is an `IOBase` handle or anything that names a folder: a
    /// string, a path-like, a `Url`. A folder that is not there loads as the
    /// empty registry and is not created; a shard that does not parse, and a
    /// root still holding the retired `records/` layout, are a `ValueError`
    /// naming the URL.
    #[staticmethod]
    fn from_handle(location: &Bound<'_, PyAny>) -> PyResult<Self> {
        let holder = folder_holder_from_value(location)?;
        CoreFixRegistry::from_handle(&holder)
            .map(|registry| Self::from_arc(Arc::new(registry)))
            .map_err(value_error)
    }

    /// Write every populated shard under `<location>/<tree>/<branch>`,
    /// removing the shards, branch folders and trees no field populates any
    /// more.
    fn write_into(&self, location: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut holder = folder_holder_from_value(location)?;
        self.inner.write_into(&mut holder).map_err(value_error)
    }

    /// The field a canonical or alternate identifier names, or `None`.
    ///
    /// `id` is the `tag:branch` text; a malformed one is a `ValueError`
    /// carrying the native parse failure, never a miss.
    fn get_field_by_id(&self, id: &str) -> PyResult<Option<PyField>> {
        let id = id_from_py(id)?;
        Ok(self
            .inner
            .get_field_by_id(id)
            .cloned()
            .map(PyField::from_inner))
    }

    /// The field a canonical or alternate identifier names.
    fn field_by_id(&self, id: &str) -> PyResult<PyField> {
        let id = id_from_py(id)?;
        self.inner
            .field_by_id(id)
            .map(|field| PyField::from_inner(field.clone()))
            .map_err(|error| absent(&error))
    }

    /// The field a canonical or alternate tag names, or `None`.
    ///
    /// The standard dictionary wins, then named dictionaries in canonical
    /// name order.
    fn get_field_by_tag(&self, tag: FixTag) -> Option<PyField> {
        self.inner
            .get_field_by_tag(tag.0)
            .cloned()
            .map(PyField::from_inner)
    }

    /// The field a canonical or alternate tag names.
    fn field_by_tag(&self, tag: FixTag) -> PyResult<PyField> {
        self.inner
            .field_by_tag(tag.0)
            .map(|field| PyField::from_inner(field.clone()))
            .map_err(|error| absent(&error))
    }

    /// The field a canonical name or alias names, ASCII case folded.
    ///
    /// Supplying `branch` restricts the lookup. Otherwise the core infers the
    /// best match: canonical before alias, standard before named branches.
    #[pyo3(signature = (name, branch=None))]
    fn get_field_by_name(&self, name: &str, branch: Option<&str>) -> PyResult<Option<PyField>> {
        let branch = branch.map(branch_from_py).transpose()?;
        Ok(self
            .inner
            .get_field_by_name(name, branch.as_ref())
            .cloned()
            .map(PyField::from_inner))
    }

    /// The field a canonical name or alias names, raising absence.
    #[pyo3(signature = (name, branch=None))]
    fn field_by_name(&self, name: &str, branch: Option<&str>) -> PyResult<PyField> {
        let branch = branch.map(branch_from_py).transpose()?;
        self.inner
            .field_by_name(name, branch.as_ref())
            .map(|field| PyField::from_inner(field.clone()))
            .map_err(|error| absent(&error))
    }

    /// The field a dotted path reaches through a component or a group.
    #[pyo3(signature = (path, branch=None))]
    fn get_field_by_path(&self, path: &str, branch: Option<&str>) -> PyResult<Option<PyField>> {
        let branch = branch.map(branch_from_py).transpose()?;
        Ok(self
            .inner
            .get_field_by_path(path, branch.as_ref())
            .cloned()
            .map(PyField::from_inner))
    }

    /// The field a dotted path reaches through a component or a group.
    #[pyo3(signature = (path, branch=None))]
    fn field_by_path(&self, path: &str, branch: Option<&str>) -> PyResult<PyField> {
        let branch = branch.map(branch_from_py).transpose()?;
        self.inner
            .field_by_path(path, branch.as_ref())
            .map(|field| PyField::from_inner(field.clone()))
            .map_err(|error| absent(&error))
    }

    /// Infer the native MIME classifier for a byte log line.
    fn infer_bytes_protocol(&self, line: &Bound<'_, PyAny>) -> PyResult<PyMimeType> {
        with_python_bytes(
            line,
            "a FIX line must be bytes, bytearray, or memoryview",
            |line| Ok(PyMimeType::from_core(self.inner.infer_bytes_protocol(line))),
        )
    }

    /// Infer the native MIME classifier for a text log line.
    fn infer_text_protocol(&self, line: &str) -> PyMimeType {
        PyMimeType::from_core(self.inner.infer_text_protocol(line))
    }

    /// Infer `MsgType` from a byte log line without parsing its FIX frame.
    fn infer_bytes_msgtype<'py>(
        &self,
        py: Python<'py>,
        line: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Bound<'py, PyBytes>>> {
        with_python_bytes(
            line,
            "a FIX line must be bytes, bytearray, or memoryview",
            |line| {
                Ok(self
                    .inner
                    .infer_bytes_msgtype(line)
                    .map(|value| PyBytes::new(py, value)))
            },
        )
    }

    /// Infer `MsgType` from a text log line without parsing its FIX frame.
    fn infer_text_msgtype(&self, line: &str) -> Option<String> {
        self.inner.infer_text_msgtype(line).map(str::to_owned)
    }

    /// The field a tag or name reaches by deterministic best match, or `None`.
    fn get_field(&self, key: &Bound<'_, PyAny>) -> PyResult<Option<PyField>> {
        let key = FixKeyArg::from_py(key)?;
        Ok(self
            .inner
            .get_field(key.as_key())
            .cloned()
            .map(PyField::from_inner))
    }

    /// The field a tag or name reaches by deterministic best match.
    fn field(&self, key: &Bound<'_, PyAny>) -> PyResult<PyField> {
        let key = FixKeyArg::from_py(key)?;
        self.inner
            .field(key.as_key())
            .map(|field| PyField::from_inner(field.clone()))
            .map_err(|error| absent(&error))
    }

    /// Add a field, answering the one it replaced.
    fn insert(&mut self, field: &Bound<'_, PyAny>) -> PyResult<Option<PyField>> {
        let field = core_field_from_value(field)?;
        Ok(self
            .inner_mut()?
            .insert(field)
            .map_err(value_error)?
            .map(PyField::from_inner))
    }

    /// Merge a definition into the stored field with the same canonical
    /// identifier.
    fn update(&mut self, field: &Bound<'_, PyAny>) -> PyResult<()> {
        let field = core_field_from_value(field)?;
        self.inner_mut()?.update(field).map_err(value_error)
    }

    /// Remove the field a tag or a name reaches in the standard branch,
    /// answering it.
    fn remove(&mut self, key: &Bound<'_, PyAny>) -> PyResult<Option<PyField>> {
        let key = FixKeyArg::from_py(key)?;
        Ok(self
            .inner_mut()?
            .remove(key.as_key())
            .map(PyField::from_inner))
    }

    /// Remove the field a canonical or alternate identifier names, answering
    /// it.
    ///
    /// The generic `remove` reaches the standard branch only, because a
    /// colon-bearing string there is a name; this is how a vendor field
    /// leaves the dictionary.
    fn remove_by_id(&mut self, id: &str) -> PyResult<Option<PyField>> {
        let id = id_from_py(id)?;
        Ok(self.inner_mut()?.remove(id).map(PyField::from_inner))
    }

    /// The field a tag or a name reaches; absence is a `KeyError`.
    fn __getitem__(&self, key: &Bound<'_, PyAny>) -> PyResult<PyField> {
        self.field(key)
    }

    /// The field a tag or name reaches by deterministic best match, or `default`.
    #[pyo3(signature = (key, default=None, /))]
    fn get(
        &self,
        py: Python<'_>,
        key: &Bound<'_, PyAny>,
        default: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        match self.get_field(key)? {
            Some(field) => Ok(field.into_pyobject(py)?.into_any().unbind()),
            None => Ok(default.unwrap_or_else(|| py.None())),
        }
    }

    fn __contains__(&self, key: &Bound<'_, PyAny>) -> PyResult<bool> {
        let key = FixKeyArg::from_py(key)?;
        Ok(self.inner.contains(key.as_key()))
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    /// The fields in ascending canonical-identifier order, lazily.
    ///
    /// The order is the core's: tag-major, then by branch digest. The iterator holds
    /// the registry's `Arc` and the identifier it stopped at, so nothing is
    /// collected crossing the boundary and the dictionary is never cloned to
    /// walk it. Holding it is therefore sharing it: a mutation refuses while a
    /// walk is unfinished, which is what stops the vector moving under a
    /// cursor into it.
    fn __iter__(&self) -> PyFixFieldIterator {
        PyFixFieldIterator {
            registry: Arc::clone(&self.inner),
            after: None,
            taken: 0,
            done: false,
        }
    }

    /// Compares the fields, in canonical-identifier order, never the
    /// identity.
    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> Py<PyAny> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return py.NotImplemented();
        };
        let equal = Arc::ptr_eq(&self.inner, &other.inner) || self.inner == other.inner;
        pyo3::types::PyBool::new(py, equal)
            .to_owned()
            .into_any()
            .unbind()
    }

    fn __repr__(&self) -> String {
        format!("FixRegistry({} fields)", self.inner.len())
    }
}

/// The fields of a registry, in ascending canonical-identifier order.
///
/// Answered by `iter(registry)`. It advances with the core's own cursor - the
/// registry plus the last `FixId` it answered - so taking one field from a
/// dictionary of thousands costs one lookup, and a walk crosses every branch
/// in the one order the core iterates.
#[pyclass(name = "FixFieldIterator", module = "yggdryl._native")]
pub(crate) struct PyFixFieldIterator {
    registry: Arc<CoreFixRegistry>,
    after: Option<CoreFixId>,
    taken: usize,
    done: bool,
}

#[pymethods]
impl PyFixFieldIterator {
    // Consumption changes iterator state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<PyField> {
        if self.done {
            return None;
        }
        let field = self.registry.next_field_after(self.after)?;
        // The cursor is the canonical identifier every registered field
        // carries; a field without one cannot be advanced past, so the walk
        // stops there rather than answering it forever.
        match field.as_fix().id() {
            Ok(Some(id)) => self.after = Some(id),
            _ => self.done = true,
        }
        self.taken += 1;
        Some(PyField::from_inner(field.clone()))
    }

    /// What is left to walk: the registry cannot change while this holds it.
    fn __length_hint__(&self) -> usize {
        if self.done {
            return 0;
        }
        self.registry.len().saturating_sub(self.taken)
    }
}

/// Read a Python `dict` as the row a Struct field declares.
///
/// The scalar boundary reads a `dict` as a nested `Mapping`, because a
/// mapping's keys are values, while a row's keys are names - which is why the
/// core canonicalizes a nested `Record` and not a mapping. The declared field
/// is what says which of the two a `dict` meant, exactly as it does in the
/// other direction, so the rewrite happens only where the field is a Struct
/// and only through a List's item; a `Map` field keeps its mapping and every
/// other value crosses untouched. Nothing is typed, ordered or validated
/// here - that is `Field::canonicalize_value`'s work, on what this hands it.
fn named_rows(field: &CoreField, value: Scalar) -> Scalar {
    match field.dtype() {
        CoreDataType::Struct(_) => {
            let children = field.fields();
            if let Some(items) = value.as_sequence() {
                if items.len() == children.len() {
                    let row: Vec<Scalar> = children
                        .iter()
                        .zip(items)
                        .map(|(child, item)| named_rows(child, item.clone()))
                        .collect();
                    return Scalar::from_sequence(row);
                }
                return value;
            }
            let named: Option<Vec<(String, Scalar)>> = value.as_mapping().and_then(|entries| {
                entries
                    .iter()
                    .map(|(key, item)| {
                        let name = key.as_str()?;
                        let child = children.iter().find(|child| child.name() == name);
                        let item = child
                            .map_or_else(|| item.clone(), |child| named_rows(child, item.clone()));
                        Some((name.to_owned(), item))
                    })
                    .collect()
            });
            named
                .and_then(|named| Scalar::from_record(named).ok())
                .unwrap_or(value)
        }
        CoreDataType::List(item)
        | CoreDataType::LargeList(item)
        | CoreDataType::FixedSizeList(item, _)
        | CoreDataType::ListView(item)
        | CoreDataType::LargeListView(item) => {
            let Some(entries) = value.as_sequence() else {
                return value;
            };
            let entries: Vec<Scalar> = entries
                .iter()
                .map(|entry| named_rows(item, entry.clone()))
                .collect();
            Scalar::from_sequence(entries)
        }
        _ => value,
    }
}

/// What [`PyFixMsg::__reduce__`] hands pickle: the rebuilder and the three
/// documents it needs - the schema, the value, and the dictionary's fields.
type MsgPickle = (Py<PyAny>, (String, String, Vec<String>));

/// A FIX message: a value plus the registry that types it.
///
/// The schema is one non-null Struct `Field` - the only row schema - and the
/// value the row it declares, so a mapping input is canonicalized into that
/// order by the core exactly as every other row is. The message is immutable:
/// it hashes, pickles, copies and compares by the schema and the value it
/// carries, against the registry it was resolved against.
#[pyclass(
    name = "FixMsg",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyFixMsg {
    inner: CoreFixMsg,
}

impl PyFixMsg {
    /// The value both the hash and the equality read.
    fn identity_value(&self) -> Scalar {
        Scalar::from_sequence([
            Scalar::from(self.inner.as_field()),
            self.inner.as_value().clone(),
        ])
    }

    /// Wrap an answered value, or report the absence its key names.
    fn answered(value: Option<&Scalar>) -> Option<PyScalar> {
        value.cloned().map(PyScalar::from_inner)
    }
}

#[pymethods]
impl PyFixMsg {
    /// Build a message, linking the process default when none is named.
    ///
    /// `value` is anything the `Scalar` boundary reads - a native `Scalar`, a
    /// mapping of names, a sequence in the root's own order - and is
    /// validated and canonicalized against `field` by the core.
    #[new]
    #[pyo3(signature = (field, value, registry=None))]
    fn new(
        field: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
        registry: Option<PyRef<'_, PyFixRegistry>>,
    ) -> PyResult<Self> {
        let field = core_field_from_value(field)?;
        let value = named_rows(&field, from_py(value)?);
        let registry = match registry {
            Some(registry) => Arc::clone(&registry.inner),
            None => Arc::clone(CoreFixRegistry::global().map_err(value_error)?),
        };
        CoreFixMsg::with_registry(registry, field, value)
            .map(|inner| Self { inner })
            .map_err(value_error)
    }

    /// Rebuild a message from the three parts pickle carried.
    #[staticmethod]
    fn _from_pickle(field: &str, value: &str, registry: Vec<String>) -> PyResult<Self> {
        let field = CoreField::from_json(field).map_err(value_error)?;
        let value = from_json_scalar_with_field(value, &field).map_err(value_error)?;
        let mut fields = Vec::with_capacity(registry.len());
        for document in registry {
            fields.push(CoreField::from_json(&document).map_err(value_error)?);
        }
        let registry = CoreFixRegistry::from_fields(fields).map_err(value_error)?;
        CoreFixMsg::with_registry(Arc::new(registry), field, value)
            .map(|inner| Self { inner })
            .map_err(value_error)
    }

    /// The registry this message resolves against, sharing it.
    #[getter]
    fn registry(&self) -> PyFixRegistry {
        PyFixRegistry::from_arc(Arc::clone(self.inner.registry()))
    }

    /// The root Struct field: the message's resolved schema.
    #[getter]
    fn field(&self) -> PyField {
        PyField::from_inner(self.inner.as_field().clone())
    }

    /// The ordered row value.
    #[getter]
    fn value(&self) -> PyScalar {
        PyScalar::from_inner(self.inner.as_value().clone())
    }

    /// The dictionary this message is spelled in.
    ///
    /// Derived from the root field's own `fix:branch` at construction, never
    /// declared, so nothing can disagree with it; empty when the root states
    /// none.
    #[getter]
    fn branch(&self) -> String {
        self.inner.branch().name().to_owned()
    }

    /// The value of the root child an identifier names, or `None`.
    ///
    /// An identifier is exact and does not tier: a dictionary this message
    /// does not speak simply misses.
    fn get_by_id(&self, id: &str) -> PyResult<Option<PyScalar>> {
        let id = id_from_py(id)?;
        Ok(Self::answered(self.inner.get_by_id(id)))
    }

    /// The value of the root child an identifier names.
    fn by_id(&self, id: &str) -> PyResult<PyScalar> {
        let id = id_from_py(id)?;
        self.inner
            .by_id(id)
            .map(|value| PyScalar::from_inner(value.clone()))
            .map_err(|error| absent(&error))
    }

    /// The value of the root child a tag names, or `None`.
    ///
    /// The tag resolves in this message's own branch first, then in the
    /// standard one.
    fn get_by_tag(&self, tag: FixTag) -> Option<PyScalar> {
        Self::answered(self.inner.get_by_tag(tag.0))
    }

    /// The value of the root child a tag names.
    fn by_tag(&self, tag: FixTag) -> PyResult<PyScalar> {
        self.inner
            .by_tag(tag.0)
            .map(|value| PyScalar::from_inner(value.clone()))
            .map_err(|error| absent(&error))
    }

    /// The value of the root child a name reaches, or `None`.
    ///
    /// The name folds through this message's own branch first, then the
    /// standard one.
    fn get_by_name(&self, name: &str) -> Option<PyScalar> {
        Self::answered(self.inner.get_by_name(name))
    }

    /// The value of the root child a name reaches.
    fn by_name(&self, name: &str) -> PyResult<PyScalar> {
        self.inner
            .by_name(name)
            .map(|value| PyScalar::from_inner(value.clone()))
            .map_err(|error| absent(&error))
    }

    /// The value a dotted path reaches, or `None`.
    fn get_by_path(&self, path: &str) -> Option<PyScalar> {
        Self::answered(self.inner.get_by_path(path))
    }

    /// The value a dotted path reaches.
    fn by_path(&self, path: &str) -> PyResult<PyScalar> {
        self.inner
            .by_path(path)
            .map(|value| PyScalar::from_inner(value.clone()))
            .map_err(|error| absent(&error))
    }

    /// The value a tag or a name reaches in the standard branch tier, or
    /// `default`.
    #[pyo3(signature = (key, default=None, /))]
    fn get(
        &self,
        py: Python<'_>,
        key: &Bound<'_, PyAny>,
        default: Option<Py<PyAny>>,
    ) -> PyResult<Py<PyAny>> {
        let key = FixKeyArg::from_py(key)?;
        match Self::answered(self.inner.get(key.as_key())) {
            Some(value) => Ok(value.into_pyobject(py)?.into_any().unbind()),
            None => Ok(default.unwrap_or_else(|| py.None())),
        }
    }

    /// The value a tag or a name reaches; absence is a `KeyError`.
    fn __getitem__(&self, key: &Bound<'_, PyAny>) -> PyResult<PyScalar> {
        let key = FixKeyArg::from_py(key)?;
        self.inner
            .value(key.as_key())
            .map(|value| PyScalar::from_inner(value.clone()))
            .map_err(|error| absent(&error))
    }

    /// The `(name, value)` pairs of the root, in the order it declares.
    fn __iter__(&self) -> PyFixMsgIterator {
        PyFixMsgIterator {
            field: self.inner.as_field().clone(),
            value: self.inner.as_value().clone(),
            index: 0,
        }
    }

    fn __len__(&self) -> usize {
        self.inner.as_field().fields().len()
    }

    /// A deterministic hash of the schema and the value.
    fn stable_hash(&self) -> u64 {
        self.identity_value().stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.stable_hash())
    }

    /// Two messages are equal with the same schema, value and dictionary.
    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> Py<PyAny> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return py.NotImplemented();
        };
        pyo3::types::PyBool::new(py, self.inner == other.inner)
            .to_owned()
            .into_any()
            .unbind()
    }

    /// Carry the schema, the value and the dictionary's fields as documents.
    ///
    /// All three travel through the JSON paths a field and a value already
    /// have, so the message a pickle rebuilds is equal to the one it came
    /// from - registry included, which equality compares.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<MsgPickle> {
        let callable = py.get_type::<Self>().getattr("_from_pickle")?.unbind();
        let field = self
            .inner
            .as_field()
            .clone()
            .into_json()
            .map_err(value_error)?;
        let value = into_json_scalar(self.inner.as_value()).map_err(value_error)?;
        let mut registry = Vec::with_capacity(self.inner.registry().len());
        for stored in self.inner.registry().iter() {
            registry.push(stored.clone().into_json().map_err(value_error)?);
        }
        Ok((callable, (field, value, registry)))
    }

    fn __copy__(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.__copy__()
    }

    fn __repr__(&self) -> String {
        format!(
            "FixMsg({:?}, {} values)",
            self.inner.as_field().name(),
            self.inner.as_field().fields().len()
        )
    }
}

/// The `(name, value)` pairs of one message's root, in declared order.
#[pyclass(name = "FixMsgIterator", module = "yggdryl._native")]
pub(crate) struct PyFixMsgIterator {
    field: CoreField,
    value: Scalar,
    index: usize,
}

#[pymethods]
impl PyFixMsgIterator {
    // Consumption changes iterator state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<(String, PyScalar)> {
        let child = self.field.fields().get(self.index)?;
        let value = self.value.get(self.index)?.clone();
        self.index += 1;
        Some((child.name().to_owned(), PyScalar::from_inner(value)))
    }

    fn __length_hint__(&self) -> usize {
        self.field.fields().len().saturating_sub(self.index)
    }
}

/// The process-wide registry, loading it on the first call.
///
/// The order is the core's: a registry installed by
/// [`install_global_registry`], then the folder `YGGDRYL_FIX_REGISTRY` names,
/// then `~/.config/fix` when it exists, then the empty registry. Only the
/// third step treats absence as empty; every other failure is a `ValueError`
/// carrying the native message, and the default stays unresolved so the next
/// call retries.
#[pyfunction]
#[pyo3(name = "global_registry")]
pub(crate) fn fix_global_registry() -> PyResult<PyFixRegistry> {
    CoreFixRegistry::global()
        .map(|registry| PyFixRegistry::from_arc(Arc::clone(registry)))
        .map_err(value_error)
}

/// Install the process-wide registry before anything resolves it.
///
/// Raises `ValueError` once the default has resolved or been installed, so
/// the value every caller saw cannot change underneath them.
#[pyfunction]
#[pyo3(name = "install_global_registry")]
pub(crate) fn fix_install_global_registry(registry: &PyFixRegistry) -> PyResult<()> {
    CoreFixRegistry::install_global((*registry.inner).clone()).map_err(value_error)
}
