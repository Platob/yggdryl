//! Native Python view of the FIX dictionary, its message, and the default.
//!
//! Nothing here resolves, folds, merges, shards or validates: the registry is
//! one [`Arc`] over the core [`FixRegistry`], and every accessor coerces its
//! key once at the boundary and redirects to the most specific native method.
//! The typed `fix:` vocabulary is not here either - it lives on the protocol
//! view class [`crate::types::field::PyProtocolField`], which is what `field.fix`
//! already answers.
//!
//! A branch *key* and an identifier cross as `str` and are parsed once here
//! through [`branch_from_py`] and [`id_from_py`], so the grammar, the folding
//! and the standard-tag rule all stay the core's. A bare tag or name uses the
//! core's deterministic best match, and a colon-bearing string is a name, never
//! an identifier. [`PyFixBranch`] is what a branch *declaration* is: a key
//! names a dictionary, a declaration also carries its dialect and its session.

use std::sync::Arc;

use pyo3::exceptions::{PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyInt};

use yggdryl::types::MsgDirection;
use yggdryl::{
    DataType as CoreDataType, Error as CoreError, Field as CoreField,
    FixBatchReader as CoreFixBatchReader, FixBranch as CoreFixBranch, FixCodec as CoreFixCodec,
    FixField as CoreFixField, FixId as CoreFixId, FixKey, FixMsg as CoreFixMsg,
    FixOptions as CoreFixOptions, FixRegistry as CoreFixRegistry, IOBase as CoreIOBase, Scalar,
    UlPlugin as CoreUlPlugin, Version as CoreVersion, from_json_scalar_with_field,
    into_json_scalar,
};

use crate::iobase::{PyIOBase, located_holder};
use crate::iomedia::{batch_reader_from_value, batch_reader_to_pyarrow};
use crate::media::iceberg::folder_holder_from_value;
use crate::types::datatype::{arrow_array_from_pyarrow, arrow_array_to_pyarrow};
use crate::types::field::{PyField, core_field_from_value};
use crate::types::scalar::{PyScalar, from_py};
use crate::uri::core_url_from_value;
use crate::value_error;

/// Read one Ullink `CBlock` through whatever Python named it with.
///
/// A `CBlock` is a file, so the location is held as whichever role it actually
/// is rather than as a container: a folder handle reads no bytes, and a reader
/// handed one answers an empty vocabulary instead of a refusal. A handle
/// crosses as itself rather than being rebuilt, so bytes held in memory are
/// readable and no second mapping is opened.
fn read_cfb<T>(
    location: &Bound<'_, PyAny>,
    read: impl FnOnce(&dyn CoreIOBase) -> yggdryl::Result<T>,
) -> PyResult<T> {
    if let Ok(handle) = location.extract::<PyRef<'_, PyIOBase>>() {
        return read(handle.inner()?.as_io()).map_err(value_error);
    }
    let url = core_url_from_value(location)?;
    read(located_holder(&url)?.as_io()).map_err(value_error)
}

/// Every arrival entry, pre-order, as the tuple Python reads.
///
/// The native record nests a group's members under the counter that heads
/// them; a binding is a view, so it flattens rather than inventing a second
/// shape. Order is the wire's.
fn flatten_entries(entries: &[yggdryl::FixEntry], out: &mut Vec<(i32, i32, String, String)>) {
    for entry in entries {
        out.push((
            entry.tag(),
            entry.branch(),
            entry.key().to_owned(),
            entry.value().to_owned(),
        ));
        flatten_entries(entry.children(), out);
    }
}

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

/// Retain the branch spelling beside the identifier for a field write.
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

    /// A registry holding nothing but this crate's own fields.
    ///
    /// Every registry starts here: the eleven fields `fix_crate_fields`
    /// lists are what a row is typed by, so a dictionary loaded from a store,
    /// built from fields or left alone holds them alike.
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
    /// string, a path-like, a `Url`. A folder that is not there loads as a new
    /// registry - the crate's own fields and nothing else - and is not
    /// created; a stored copy of the crate's branch is read past, because the
    /// crate's own definition is the one that types a row. A shard that does
    /// not parse, and a root still holding the retired `records/` layout, are
    /// a `ValueError` naming the URL.
    #[staticmethod]
    fn from_handle(location: &Bound<'_, PyAny>) -> PyResult<Self> {
        let holder = folder_holder_from_value(location)?;
        CoreFixRegistry::from_handle(&holder)
            .map(|registry| Self::from_arc(Arc::new(registry)))
            .map_err(value_error)
    }

    /// Read an Ullink `CBlock` into the vocabulary and roots it declares.
    ///
    /// Answers the dictionary its `vocabulary` states and the message roots
    /// its `grammar-binding`s describe. `branch` names the dialect its
    /// user-range tags belong to; the standard tags always land in the
    /// standard branch, because a dialect redefines its own tags and never
    /// FIX's.
    ///
    /// A file this cannot be read from is a `ValueError` carrying the native
    /// sentence whole: the byte the reader stopped at, what was expected, what
    /// arrived, and the element the file spells it in.
    #[staticmethod]
    #[pyo3(signature = (location, branch=None))]
    fn from_cfb_file(
        location: &Bound<'_, PyAny>,
        branch: Option<&str>,
    ) -> PyResult<(Self, Vec<PyField>)> {
        let dialect = branch.map(branch_from_py).transpose()?;
        let (registry, roots) = read_cfb(location, |handle| {
            CoreFixRegistry::from_cfb_file(handle, dialect.as_ref())
        })?;
        Ok((
            Self::from_arc(Arc::new(registry)),
            roots.into_iter().map(PyField::from_inner).collect(),
        ))
    }

    /// Fold `fields` in, adding what is absent and merging what is stored.
    ///
    /// Each entry is anything `Field` accepts. A field whose canonical
    /// identity the dictionary does not hold is inserted, one it holds is
    /// merged, and the answer is the count added and the count merged, in
    /// that order.
    ///
    /// One mutation: the whole fold is staged and only then adopted, so a
    /// refusal - a field with no `fix:tag`, a key another field holds in the
    /// same branch, a name or datatype disagreeing with the stored
    /// definition - leaves the dictionary exactly as it was.
    fn add_fields(&mut self, fields: &Bound<'_, PyAny>) -> PyResult<(usize, usize)> {
        // Coerced whole before anything is written, so a value Python cannot
        // read as a field refuses the fold rather than half of it.
        let mut held = Vec::new();
        for value in fields.try_iter()? {
            held.push(core_field_from_value(&value?)?);
        }
        self.inner_mut()?.add_fields(held).map_err(value_error)
    }

    /// Fold another dictionary into this one.
    ///
    /// The one place two dictionaries combine: every field folds the way
    /// `add_fields` folds one, and every dialect folds beside them - one this
    /// dictionary does not hold arrives whole, one it holds takes the incoming
    /// record while keeping every spelling it already answered to.
    ///
    /// Answers the count added and the count merged, over the fields. One
    /// mutation: a refusal anywhere leaves the dictionary exactly as it was.
    fn merge_with(&mut self, other: &Self) -> PyResult<(usize, usize)> {
        let incoming = Arc::clone(&other.inner);
        self.inner_mut()?.merge_with(&incoming).map_err(value_error)
    }

    /// Read one Ullink `CBlock` into this dictionary, whole.
    ///
    /// The one call an ingest takes: the file's vocabulary folds in the way
    /// `add_fields` folds any source, and the dialect the root element
    /// declares - its FIX version and its session `CompID` pair - is recorded
    /// beside it, which reading the fields alone would lose.
    ///
    /// `branch` names the dialect, and the file names it when the caller does
    /// not. The location's own stem also becomes an alias whenever it is not
    /// already the name, and `aliases` names any others; a branch this
    /// dictionary already holds keeps the spellings it already answered to.
    ///
    /// Answers the count added and the count merged. One mutation: the branch
    /// record and the fields are adopted together, so a refusal leaves the
    /// dictionary exactly as it was.
    #[pyo3(signature = (location, branch=None, aliases=None))]
    fn add_cfb_file(
        &mut self,
        location: &Bound<'_, PyAny>,
        branch: Option<&str>,
        aliases: Option<Vec<String>>,
    ) -> PyResult<(usize, usize)> {
        // Naming none and naming an empty list are the same statement, so
        // both arrive as the empty slice rather than as two shapes.
        let owned = aliases.unwrap_or_default();
        let held: Vec<&str> = owned.iter().map(String::as_str).collect();
        let registry = self.inner_mut()?;
        read_cfb(location, |handle| {
            registry.add_cfb_file(handle, branch, Some(&held))
        })
    }

    /// Add `ULBridge`'s own fields, so a bridge configuration document types.
    ///
    /// A dictionary that has them reads a document's attributes as the ports,
    /// sequence numbers and flags they are; one that does not reads them as
    /// the text they arrived as, because a key no dictionary explains is kept
    /// rather than dropped.
    fn with_ulbridge_fields(&mut self) -> PyResult<()> {
        let held = std::mem::take(self.inner_mut()?);
        *self.inner_mut()? = held.with_ulbridge_fields().map_err(value_error)?;
        Ok(())
    }

    /// Register one message type, answering the value it takes.
    ///
    /// A type the code set does not have is added rather than refused, and a
    /// spelling too long for the datatype takes a stable synthesized value.
    /// `name` is the symbolic name the set files it under, with the spelling
    /// kept as an alias when the two differ, and `description` is the source's
    /// own wording. Idempotent and enriching: a type already spelled answers
    /// its value, gains a spelling the set did not answer to and a description
    /// it did not have, and keeps everything it already held.
    #[pyo3(signature = (spelling, name=None, description=None))]
    fn register_msgtype(
        &mut self,
        spelling: &str,
        name: Option<&str>,
        description: Option<&str>,
    ) -> PyResult<String> {
        self.inner_mut()?
            .register_msgtype(spelling, name, description)
            .map(|held| held.as_str().to_owned())
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

    /// The branch an identifier belongs to, or `None`.
    ///
    /// The identifier carries the branch's identity, so this is what a
    /// `tag:branch` string resolves to without a second lookup.
    fn branch_of(&self, id: &str) -> PyResult<Option<PyFixBranch>> {
        let id = id_from_py(id)?;
        Ok(self
            .inner
            .branch_of(id)
            .cloned()
            .map(PyFixBranch::from_core))
    }

    /// The branch one digest resolves to, or `None`.
    ///
    /// An arrival entry carries its dialect as the digest `branch`, so this is
    /// the table that turns a capture's column back into the branch it was
    /// read under. The digest is one way, which is why the registry publishes
    /// the resolution rather than leaving a reader to reproduce the hash.
    fn get_branch_by_digest(&self, digest: i32) -> Option<PyFixBranch> {
        self.inner
            .get_branch_by_digest(digest)
            .cloned()
            .map(PyFixBranch::from_core)
    }

    /// The branch one digest resolves to.
    ///
    /// # Errors
    ///
    /// Raises `ValueError` naming the digest when no branch carries it.
    fn branch_by_digest(&self, digest: i32) -> PyResult<PyFixBranch> {
        self.inner
            .branch_by_digest(digest)
            .cloned()
            .map(PyFixBranch::from_core)
            .map_err(value_error)
    }

    /// The branch declared under one canonical name, or `None`.
    fn branch_named(&self, name: &str) -> Option<PyFixBranch> {
        self.inner
            .branch_named(name)
            .cloned()
            .map(PyFixBranch::from_core)
    }

    /// Every branch this registry declares.
    fn branches(&self) -> Vec<PyFixBranch> {
        self.inner
            .branches()
            .cloned()
            .map(PyFixBranch::from_core)
            .collect()
    }

    /// Install or replace one complete branch declaration.
    ///
    /// The standard branch declares no dialect and no session, and two
    /// different names may not claim one identity - both are refused here
    /// rather than stored and discovered later.
    fn set_branch(&mut self, branch: &Bound<'_, PyAny>) -> PyResult<()> {
        let branch = branch_value_from_py(branch)?;
        self.inner_mut()?
            .set_branch(branch.as_inner().clone())
            .map_err(value_error)
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
    /// Wrap a message the core built.
    pub(crate) const fn from_inner(inner: CoreFixMsg) -> Self {
        Self { inner }
    }

    /// Borrow the message the core holds.
    pub(crate) const fn as_inner(&self) -> &CoreFixMsg {
        &self.inner
    }

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

    /// This message's value digest, as sixteen big-endian bytes.
    ///
    /// Over what the message says: the whole session envelope is excluded,
    /// so the same order relayed through two sessions or replayed on a
    /// resend is one message. Computed on every call and stored nowhere.
    fn digest<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.digest().to_be_bytes())
    }

    /// One instrument symbol that is the same across venues.
    fn symbol_ticker(&self) -> Option<PyScalar> {
        Self::answered(Some(&self.inner.symbol_ticker()))
    }

    /// The timestamp a capture is ordered and partitioned by.
    ///
    /// The crate's `timestamp` child every built message closes with: the
    /// row's own clock where the capture stated one, else the first clock the
    /// message carries, else the epoch - so a message the codec built always
    /// answers, and only a message built by hand without that child does not.
    fn market_timestamp(&self) -> Option<PyScalar> {
        Self::answered(Some(&self.inner.market_timestamp()))
    }

    /// The partition that timestamp falls in, in whole seconds.
    #[pyo3(signature = (seconds=3600))]
    fn unix_partition(&self, seconds: i64) -> Option<PyScalar> {
        Self::answered(Some(&self.inner.unix_partition(seconds)))
    }

    /// The one value a facet names, or `None` where it is not unambiguous.
    fn lifted(&self, facet: &str) -> Option<PyScalar> {
        Self::answered(self.inner.lifted(facet))
    }

    /// Which tag answered a facet, so a fallback is visible.
    fn lift_source(&self, facet: &str) -> Option<String> {
        self.inner.lift_source(facet).map(|id| id.to_string())
    }

    /// Every facet this message answers, in the table's own order.
    fn lift(&self) -> Vec<(String, PyScalar)> {
        self.inner
            .lift()
            .map(|(facet, value)| (facet.to_owned(), PyScalar::from_inner(value.clone())))
            .collect()
    }

    /// The party bearing one role, matched through the code translation.
    fn party(&self, role: &str) -> Option<Vec<Option<PyScalar>>> {
        let held = self.inner.party(role)?;
        Some(vec![
            Self::answered(held.id()),
            Self::answered(held.source()),
            Self::answered(held.role()),
            Self::answered(held.qualifier()),
        ])
    }

    /// One regulatory timestamp by its type.
    fn trd_reg_timestamp(&self, kind: &str) -> Option<PyScalar> {
        Self::answered(self.inner.trd_reg_timestamp(kind))
    }

    /// What this message says about itself that does not add up.
    ///
    /// Derived by comparing the row against the arrival record, so a caller
    /// who never asks pays nothing.
    fn anomalies(&self) -> Vec<String> {
        self.inner
            .anomalies()
            .map(|held| held.to_string())
            .collect()
    }

    /// What arrived, in arrival order, untranslated.
    ///
    /// Flattened pre-order: a group's members follow the counter pair that
    /// heads them, so a caller reading the sequence reads the wire. The
    /// dialect crosses as its digest, which `FixRegistry.branch_by_digest`
    /// resolves.
    fn entries(&self) -> Vec<(i32, i32, String, String)> {
        let mut held = Vec::new();
        flatten_entries(self.inner.entries(), &mut held);
        held
    }

    /// This message as the fixed row a table holds.
    ///
    /// `schema` is the fixed root :func:`fix_schema` builds. Every column is
    /// filled by the tag its field carries - never by its spelling - so a
    /// message that carried nothing at a column answers null there rather than
    /// shifting its neighbours, which is what makes two rows of one capture
    /// comparable at all. A column no tag names answers null: it is the
    /// capture's, and nothing in the message says what it held.
    fn to_row(&self, schema: &Bound<'_, PyAny>) -> PyResult<PyScalar> {
        self.inner
            .to_row(&core_field_from_value(schema)?)
            .map(PyScalar::from_inner)
            .map_err(value_error)
    }

    /// Re-emit this message on the wire, separated by `separator`.
    ///
    /// Named for what it answers rather than for consuming the message: the
    /// bytes come from the arrival record, which the message keeps.
    #[pyo3(signature = (separator=1))]
    fn to_bytes<'py>(&self, py: Python<'py>, separator: u8) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.into_bytes(separator))
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

/// One dictionary, reading lines into messages.
///
/// The reader is the whole parse surface: a captured line with a verb in front
/// of it, a bare frame, a numeric frame with a stated separator, a bridge's
/// name/value text, or pairs a caller already has. Each redirects to the core
/// method of the same name, so nothing here decides a dialect, a version or a
/// separator - it only carries what Python said across.
#[pyclass(name = "FixCodec", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PyFixCodec {
    inner: CoreFixCodec,
    registry: Arc<CoreFixRegistry>,
}

impl PyFixCodec {
    /// Borrow the reader the core holds.
    pub(crate) const fn as_inner(&self) -> &CoreFixCodec {
        &self.inner
    }
}

#[pymethods]
impl PyFixCodec {
    /// Open a reader over one dictionary, or over the process default.
    #[new]
    #[pyo3(signature = (registry=None, *, branch=None, version=None, null_values=None))]
    fn new(
        registry: Option<PyRef<'_, PyFixRegistry>>,
        branch: Option<&str>,
        version: Option<&str>,
        null_values: Option<Vec<String>>,
    ) -> PyResult<Self> {
        let registry = match registry {
            Some(held) => Arc::clone(&held.inner),
            None => Arc::clone(CoreFixRegistry::global().map_err(value_error)?),
        };
        let mut inner = CoreFixCodec::new(Arc::clone(&registry));
        if let Some(held) = branch {
            inner = inner.with_branch(&branch_from_py(held)?);
        }
        if let Some(held) = version {
            inner = inner.with_version(version_from_py(held)?);
        }
        if let Some(held) = null_values {
            inner = inner.with_null_values(held);
        }
        Ok(Self { inner, registry })
    }

    /// The dictionary this reader resolves against, sharing it.
    #[getter]
    fn registry(&self) -> PyFixRegistry {
        PyFixRegistry::from_arc(Arc::clone(&self.registry))
    }

    /// One captured line, whatever it is wrapped in.
    #[pyo3(signature = (row, enrich=false))]
    fn transform_line(&self, row: &[u8], enrich: bool) -> PyResult<PyFixMsg> {
        self.inner
            .transform_line(row, enrich)
            .map(PyFixMsg::from_inner)
            .map_err(value_error)
    }

    /// One numeric frame, split on the separator stated or inferred.
    #[pyo3(signature = (body, separator=None, enrich=false))]
    fn transform_fix_line(
        &self,
        body: &[u8],
        separator: Option<u8>,
        enrich: bool,
    ) -> PyResult<PyFixMsg> {
        let codec = match separator {
            Some(held) => self.inner.clone().with_separator(held),
            None => self.inner.clone(),
        };
        codec
            .transform_fix_line(body, enrich)
            .map(PyFixMsg::from_inner)
            .map_err(value_error)
    }

    /// One bridge frame, whose keys are names rather than tags.
    #[pyo3(signature = (body, enrich=false))]
    fn transform_ullink_line(&self, body: &[u8], enrich: bool) -> PyResult<PyFixMsg> {
        self.inner
            .transform_ullink_line(body, enrich)
            .map(PyFixMsg::from_inner)
            .map_err(value_error)
    }

    /// One FIXML row, whose fields are XML attributes.
    #[pyo3(signature = (body, enrich=false))]
    fn transform_fixml_line(&self, body: &[u8], enrich: bool) -> PyResult<PyFixMsg> {
        self.inner
            .transform_fixml_line(body, enrich)
            .map(PyFixMsg::from_inner)
            .map_err(value_error)
    }

    /// One bridge configuration document, as a Jolokia answer states it.
    ///
    /// The document is read out of the line it arrived on: a transport writes
    /// a timestamp in front of one and sometimes a duration behind it, and
    /// both are prose.
    #[pyo3(signature = (body, enrich=false))]
    fn transform_ulconfig_line(&self, body: &[u8], enrich: bool) -> PyResult<PyFixMsg> {
        self.inner
            .transform_ulconfig_line(body, enrich)
            .map(PyFixMsg::from_inner)
            .map_err(value_error)
    }

    /// Pairs a caller already holds, in the order they arrived.
    ///
    /// Taken by value because the borrowed pairs the core reads point into
    /// these strings, so they have to outlive the call rather than the caller.
    #[allow(clippy::needless_pass_by_value)]
    #[pyo3(signature = (pairs, enrich=false))]
    fn transform_pairs(&self, pairs: Vec<(String, String)>, enrich: bool) -> PyResult<PyFixMsg> {
        let borrowed: Vec<(&[u8], &[u8])> = pairs
            .iter()
            .map(|(key, value)| (key.as_bytes(), value.as_bytes()))
            .collect();
        self.inner
            .transform_pairs(borrowed, enrich)
            .map(PyFixMsg::from_inner)
            .map_err(value_error)
    }

    /// Fills what one message implies but did not carry.
    ///
    /// An order stating `OrderQty` and `CumQty` has said what `LeavesQty` is.
    /// Only the row is filled: the arrival record is what the wire carried
    /// and is left alone, so `to_bytes` re-emits the received line either
    /// way, and a stated value is never replaced.
    fn enrich_fixmsg(&self, message: &PyFixMsg) -> PyResult<PyFixMsg> {
        self.inner
            .enrich_fixmsg(message.inner.clone())
            .map(PyFixMsg::from_inner)
            .map_err(value_error)
    }

    /// Fills a sequence of messages.
    #[allow(clippy::needless_pass_by_value)]
    fn enrich_fixmsgs(&self, messages: Vec<PyRef<'_, PyFixMsg>>) -> PyResult<Vec<PyFixMsg>> {
        messages
            .into_iter()
            .map(|held| {
                self.inner
                    .enrich_fixmsg(held.inner.clone())
                    .map(PyFixMsg::from_inner)
                    .map_err(value_error)
            })
            .collect()
    }

    fn __copy__(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            registry: Arc::clone(&self.registry),
        }
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.__copy__()
    }

    fn __repr__(&self) -> String {
        format!("FixCodec({} fields)", self.registry.len())
    }
}

/// Read one FIX version, or report the native parse failure as a `ValueError`.
///
/// A version crosses as text and becomes a `Version` here, once, so the
/// grammar - `4.4`, `FIX.4.4`, `5.0SP2` - stays the core's and no second class
/// exists in Python.
fn version_from_py(text: &str) -> PyResult<CoreVersion> {
    let spelling = text.strip_prefix("FIX.").unwrap_or(text);
    spelling.parse::<CoreVersion>().map_err(value_error)
}

/// The fixed root every message answers as, built from one dictionary.
///
/// Header, the fields a consumer reads, the groups worth persisting whole, the
/// trailer, this crate's own derived facts, and the two lists that close every
/// row. Columns are spelled by the dictionary's folded canonical names -
/// `msgtype`, never `35` - so a row reads the way a message reads; the tag
/// stays each column's identity, on its `fix:tag`, and is what fills it.
#[pyfunction]
#[pyo3(name = "fix_schema", signature = (registry=None, name="fix"))]
pub(crate) fn fix_schema(
    registry: Option<PyRef<'_, PyFixRegistry>>,
    name: &str,
) -> PyResult<PyField> {
    let registry = match registry {
        Some(held) => Arc::clone(&held.inner),
        None => Arc::clone(CoreFixRegistry::global().map_err(value_error)?),
    };
    yggdryl::fix_schema(&registry, name.to_owned())
        .map(PyField::from_inner)
        .map_err(value_error)
}

/// The fixed root behind a capture's own columns.
///
/// `carrier` is a capture's own root - where a line was read from, which line
/// it was, what stamped it - and its columns lead the row, because that is what
/// a monitor orders and joins on. A carried column whose folded name a FIX
/// column already takes - `sessionId` and `sessionid` are one name - is dropped
/// rather than renamed: the FIX column is the one a reader spelling it means,
/// and the row fills it from what the capture stated.
#[pyfunction]
#[pyo3(name = "fix_schema_carrying", signature = (carrier, read))]
pub(crate) fn fix_schema_carrying(
    carrier: &Bound<'_, PyAny>,
    read: &Bound<'_, PyAny>,
) -> PyResult<PyField> {
    let carrier = core_field_from_value(carrier)?;
    let read = core_field_from_value(read)?;
    yggdryl::fix_schema_carrying(&carrier, &read)
        .map(PyField::from_inner)
        .map_err(value_error)
}

/// Stream one Arrow source's payload column through a dictionary.
///
/// The thin redirect to `FixBatchReader::from_column`: the capture's own
/// columns lead the row, the dictionary's fixed columns follow, and one input
/// row stays one output row unless `dedup` says otherwise. The reader stays
/// lazy across the boundary -- `PyArrow` pulls one batch at a time.
///
/// Every keyword is the per-stream form of an argument `FixCodec` already
/// takes per call, so a stream parses exactly as a line does. A row's own
/// columns speak for that row: `timestamp` stamps its message, `branch`,
/// `beginstring`, `sep` and `direction` are the parameters of the same name,
/// and any other column named after a field fills it where the frame did not
/// state it - never as an entry. A column whose folded name a fixed column
/// takes lands there rather than being carried in front.
#[pyfunction]
#[pyo3(
    name = "fix_parse_arrow_reader",
    signature = (
        source,
        registry = None,
        column = "body",
        *,
        name = "fix",
        branch = None,
        version = None,
        separator = None,
        direction = None,
        null_values = None,
        dedup = false,
        batch_row_size = None,
        batch_byte_size = None,
    )
)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn fix_parse_arrow_reader<'py>(
    py: Python<'py>,
    source: &Bound<'_, PyAny>,
    registry: Option<PyRef<'_, PyFixRegistry>>,
    column: &str,
    name: &str,
    branch: Option<&str>,
    version: Option<&str>,
    separator: Option<u8>,
    direction: Option<&str>,
    null_values: Option<Vec<String>>,
    dedup: bool,
    batch_row_size: Option<usize>,
    batch_byte_size: Option<u64>,
) -> PyResult<Bound<'py, PyAny>> {
    let registry = match registry {
        Some(held) => Arc::clone(&held.inner),
        None => Arc::clone(CoreFixRegistry::global().map_err(value_error)?),
    };
    let mut options = CoreFixOptions::new().with_dedup(dedup);
    options.name = name.into();
    if let Some(branch) = branch {
        options = options.with_branch(branch_from_py(branch)?);
    }
    if let Some(version) = version {
        options = options.with_version(version_from_py(version)?);
    }
    if let Some(separator) = separator {
        options = options.with_separator(separator);
    }
    if let Some(direction) = direction {
        options = options.with_direction(direction_from_py(direction)?);
    }
    if let Some(spellings) = null_values {
        options = options.with_null_values(spellings);
    }
    options.batch_row_size = batch_row_size;
    options.batch_byte_size = batch_byte_size;
    let source = batch_reader_from_value(source)?;
    let parsed =
        CoreFixBatchReader::from_column(registry, source, column, options).map_err(value_error)?;
    batch_reader_to_pyarrow(py, parsed)
}

/// What a whole payload column says about itself, without building a message.
///
/// Three `pyarrow` `utf8` arrays the length of the input, in one shallow pass:
/// the media type each record infers, the raw `MsgType` its frame spells, and
/// the direction it moved. `direction` names what an unmarked line took --
/// `"sent"`, `"recv"`, or `"unknown"`.
#[pyfunction]
#[pyo3(name = "fix_classify_arrow_array", signature = (column, direction = "sent"))]
pub(crate) fn fix_classify_arrow_array<'py>(
    py: Python<'py>,
    column: &Bound<'py, PyAny>,
    direction: &str,
) -> PyResult<(Bound<'py, PyAny>, Bound<'py, PyAny>, Bound<'py, PyAny>)> {
    let values = arrow_array_from_pyarrow(column)?;
    let (protocol, msgtype, moved) =
        yggdryl::classify_arrow_array(&values, direction_from_py(direction)?)
            .map_err(value_error)?;
    Ok((
        arrow_array_to_pyarrow(py, &protocol, None)?,
        arrow_array_to_pyarrow(py, &msgtype, None)?,
        arrow_array_to_pyarrow(py, &moved, None)?,
    ))
}

/// Read the direction an unmarked line takes, as the core spells it.
///
/// `"unknown"` is the third answer: a capture whose silence really means
/// nothing, rather than the side that wrote it.
fn direction_from_py(text: &str) -> PyResult<Option<&'static str>> {
    match text.to_ascii_uppercase().as_str() {
        "SENT" | "S" | "SEND" => Ok(Some(MsgDirection::SENT)),
        "RECV" | "R" | "RECEIVE" => Ok(Some(MsgDirection::RECV)),
        "UNKNOWN" | "" => Ok(None),
        other => Err(PyValueError::new_err(format!(
            "expected one of sent, recv, unknown, got {other:?}"
        ))),
    }
}

/// One row's columns, in order, as tags.
#[pyfunction]
#[pyo3(name = "fix_schema_tags")]
pub(crate) fn fix_schema_tags() -> Vec<i32> {
    yggdryl::fix_schema_tags()
}

/// The fields this crate defines on its own branch, in tag order.
///
/// The digest, the version read, the cross-venue symbol, the market clock, the
/// partition it falls in, the two parent order identifiers no standard tag
/// names, and the four facts a bridge's own log states about a line: its
/// session, its message context, and the plugins it moved between. Every
/// registry holds them from construction; this is the listing.
#[pyfunction]
#[pyo3(name = "fix_crate_fields")]
pub(crate) fn fix_crate_fields() -> PyResult<Vec<PyField>> {
    yggdryl::fix_crate_fields()
        .map(|held| held.iter().cloned().map(PyField::from_inner).collect())
        .map_err(value_error)
}

/// One text-keyed mapping as the record a parsed document holds there.
fn folded_record(value: &Scalar) -> Option<Scalar> {
    let entries = value.as_mapping()?;
    let mut named = Vec::with_capacity(entries.len());
    for (name, held) in entries {
        named.push((name.as_str()?.to_owned(), stated_document(held.clone())));
    }
    Scalar::from_record(named).ok()
}

/// The document Python stated, as the records a parsed one is made of.
///
/// A Python mapping crosses as a mapping - its keys are values rather than
/// names - and every reader here resolves an attribute by name, so a `dict`
/// would answer nothing. Folding a text-keyed mapping into a record at every
/// depth is what makes a `dict` the same document the equivalent bytes parse
/// to. Anything else crosses as itself: a value the core already built is
/// already records, and a mapping keyed by something other than text is not a
/// document.
fn stated_document(value: Scalar) -> Scalar {
    if let Some(folded) = folded_record(&value) {
        return folded;
    }
    if let Some(items) = value.as_sequence() {
        return Scalar::from_sequence(items.iter().cloned().map(stated_document));
    }
    value
}

/// The attributes a caller stated, as the record a document would have made.
///
/// The same fold, refusing what it could not make a record of: a caller
/// stating attributes states their names, and a mapping keyed by anything
/// else would build a plugin that answers nothing.
fn stated_attributes(value: Scalar) -> PyResult<Scalar> {
    let held = stated_document(value);
    if let Some(entries) = held.as_mapping() {
        let unnamed = entries
            .iter()
            .find(|(name, _)| name.as_str().is_none())
            .map_or("value", |(name, _)| name.kind());
        return Err(PyTypeError::new_err(format!(
            "expected an attribute name, got {unnamed}"
        )));
    }
    Ok(held)
}

/// What [`PyUlPlugin::__reduce__`] hands pickle: the rebuilder and the two
/// parts it needs - the attributes as a document, and the `ObjectName`.
type PluginPickle = (Py<PyAny>, (String, Option<String>));

/// One plugin a bridge configuration document answers for.
///
/// A Jolokia read answers one `MBean`'s attributes or a map of them keyed by
/// `ObjectName`, and both are the same statement made once or many times. This
/// is one of those statements - the `ObjectName` the bridge holds the plugin
/// under, beside the attributes it stated - which is what a monitor walking a
/// hundred of them holds before it types any of them. `FixMsg` is the same
/// facts typed against a dictionary, and the two cross both ways.
///
/// A mapping crossing the boundary is folded into the record a parsed document
/// holds, so a `dict` states one as well as bytes do.
///
/// Immutable, so it hashes, copies and pickles like every other value here.
#[pyclass(
    name = "UlPlugin",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
pub(crate) struct PyUlPlugin {
    inner: CoreUlPlugin,
}

impl PyUlPlugin {
    /// Wrap a plugin the core answered.
    const fn from_inner(inner: CoreUlPlugin) -> Self {
        Self { inner }
    }

    /// The value both the hash and the pickle read.
    fn identity_value(&self) -> Scalar {
        Scalar::from_sequence([
            self.inner.mbean().map_or(Scalar::Null, Scalar::from),
            self.inner.as_attributes().clone(),
        ])
    }
}

#[pymethods]
impl PyUlPlugin {
    /// Build one plugin from the parts a document states.
    ///
    /// `attributes` is anything the `Scalar` boundary reads - a mapping of
    /// names, a native `Scalar`, a parsed document - and `mbean` is the
    /// `ObjectName` the bridge holds the plugin under, where one is known.
    #[new]
    #[pyo3(signature = (attributes, mbean=None))]
    fn new(attributes: &Bound<'_, PyAny>, mbean: Option<&str>) -> PyResult<Self> {
        Ok(Self::from_inner(CoreUlPlugin::new(
            mbean,
            stated_attributes(from_py(attributes)?)?,
        )))
    }

    /// Every plugin the document a line carries answers for.
    ///
    /// The document is found inside the line the way the classifier finds it:
    /// a transport writes a timestamp in front of one and sometimes a duration
    /// behind it, and both are prose. Bytes that name no `MBean` are read whole.
    #[staticmethod]
    fn from_json_bytes(body: &[u8]) -> PyResult<Vec<Self>> {
        CoreUlPlugin::from_json_bytes(body)
            .map(|held| held.map(Self::from_inner).collect())
            .map_err(value_error)
    }

    /// The same, over a document a caller already parsed.
    ///
    /// The envelope is optional: a `value` under a Jolokia answer, an array of
    /// those answers for a bulk read, or a bare attribute map. A document that
    /// answers nothing answers no plugins rather than raising - a Jolokia
    /// error is a document too.
    #[staticmethod]
    fn from_json_scalar(document: &Bound<'_, PyAny>) -> PyResult<Vec<Self>> {
        let document = stated_document(from_py(document)?);
        Ok(CoreUlPlugin::from_json_scalar(&document)
            .map(Self::from_inner)
            .collect())
    }

    /// Every plugin one typed message carries, one per occurrence.
    #[staticmethod]
    fn from_fixmsg(message: &PyFixMsg) -> Vec<Self> {
        CoreUlPlugin::from_fixmsg(message.as_inner())
            .map(Self::from_inner)
            .collect()
    }

    /// This plugin as a message typed against `codec`'s dictionary.
    ///
    /// The same build every other reader funnels into, so a dictionary
    /// carrying `ULBridge`'s fields types a port as a number and a flag as a
    /// boolean, and one that does not keeps every attribute as the text it
    /// arrived as.
    #[pyo3(signature = (codec, enrich=false))]
    #[allow(clippy::wrong_self_convention)]
    fn into_fixmsg(&self, codec: &PyFixCodec, enrich: bool) -> PyResult<PyFixMsg> {
        self.inner
            .into_fixmsg(codec.as_inner(), enrich)
            .map(PyFixMsg::from_inner)
            .map_err(value_error)
    }

    /// The `ObjectName` the bridge holds this plugin under.
    #[getter]
    fn mbean(&self) -> Option<&str> {
        self.inner.mbean()
    }

    /// What the `ObjectName` says this `MBean` is: `Plugin`, `ConfigurationPlugin`.
    #[getter]
    fn mbean_type(&self) -> Option<&str> {
        self.inner.mbean_type()
    }

    /// The protocol the `ObjectName` says this plugin speaks.
    #[getter]
    fn plugin_type(&self) -> Option<&str> {
        self.inner.plugin_type()
    }

    /// The name the bridge knows this plugin by.
    #[getter]
    fn name(&self) -> Option<&str> {
        self.inner.name()
    }

    /// The plugin version this session interface runs.
    #[getter]
    fn version(&self) -> Option<&str> {
        self.inner.version()
    }

    /// The category the bridge files this plugin under.
    #[getter]
    fn category(&self) -> Option<&str> {
        self.inner.category()
    }

    /// What the session is doing now, where the document says.
    #[getter]
    fn state(&self) -> Option<&str> {
        self.inner.state()
    }

    /// Every attribute this plugin states, by name.
    #[getter]
    fn attributes(&self) -> std::collections::BTreeMap<String, PyScalar> {
        self.inner
            .attributes()
            .map(|(name, value)| (name.to_owned(), PyScalar::from_inner(value.clone())))
            .collect()
    }

    /// One attribute as the document stated it, or `None`.
    ///
    /// The spelling is folded the way every other name in this crate is, so
    /// `PrimaryHost`, `primaryhost` and `primary_host` are one attribute.
    fn get(&self, attribute: &str) -> Option<PyScalar> {
        self.inner.get(attribute).cloned().map(PyScalar::from_inner)
    }

    /// The stable digest of this plugin, the same in every process.
    fn stable_hash(&self) -> u64 {
        self.identity_value().stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.stable_hash())
    }

    /// Two plugins are equal with the same `ObjectName` and attributes.
    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> Py<PyAny> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return py.NotImplemented();
        };
        pyo3::types::PyBool::new(py, self.inner == other.inner)
            .to_owned()
            .into_any()
            .unbind()
    }

    fn __contains__(&self, attribute: &str) -> bool {
        self.inner.get(attribute).is_some()
    }

    fn __len__(&self) -> usize {
        self.inner.attributes().count()
    }

    /// Rebuild a plugin from the two parts pickle carried.
    #[staticmethod]
    fn _from_pickle(attributes: &str, mbean: Option<&str>) -> PyResult<Self> {
        let attributes = yggdryl::from_json_scalar(attributes.as_bytes()).map_err(value_error)?;
        Ok(Self::from_inner(CoreUlPlugin::new(mbean, attributes)))
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<PluginPickle> {
        let callable = py.get_type::<Self>().getattr("_from_pickle")?.unbind();
        let attributes = into_json_scalar(self.inner.as_attributes()).map_err(value_error)?;
        Ok((
            callable,
            (attributes, self.inner.mbean().map(str::to_owned)),
        ))
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
            "UlPlugin({:?}, {} attributes)",
            self.inner.name().unwrap_or_default(),
            self.inner.attributes().count()
        )
    }
}

/// The fields `ULBridge`'s own dictionary defines, in tag order.
///
/// What a bridge configuration document states about a session interface -
/// the venue it talks to, the host and port, the sequence numbers, the state -
/// on the `ulbridge` branch rather than FIX's, because the specification
/// publishes none of it. Registering them is a caller's choice, which is what
/// `FixRegistry.with_ulbridge_fields` is for.
#[pyfunction]
#[pyo3(name = "fix_ulbridge_fields")]
pub(crate) fn fix_ulbridge_fields() -> PyResult<Vec<PyField>> {
    yggdryl::fix_ulbridge_fields()
        .map(|held| held.iter().cloned().map(PyField::from_inner).collect())
        .map_err(value_error)
}

/// The vocabulary one Ullink `CBlock` declares, in declaration order./// The vocabulary one Ullink `CBlock` declares, in declaration order.
///
/// The dictionary half of `FixRegistry.from_cfb_file`, answered on its own: every
/// field carries the `fix:tag` and `fix:branch` that key it and whatever code
/// set the file's maps decode for it, which is what `FixRegistry.add_fields`
/// needs to fold one counterparty's file into a dictionary that exists. The
/// message roots and the branch record are what the registry form answers
/// instead.
///
/// `branch` names the dialect, and the file names it when the caller does
/// not: a `CBlock` states a version and a session but no name for the pair, so
/// with none supplied the location's own stem stands in. A stem that is not a
/// branch is a `ValueError` rather than a guess.
#[pyfunction]
#[pyo3(name = "fix_cfb_fields", signature = (location, branch=None))]
pub(crate) fn fix_cfb_fields(
    location: &Bound<'_, PyAny>,
    branch: Option<&str>,
) -> PyResult<Vec<PyField>> {
    read_cfb(location, |handle| {
        CoreFixField::from_cfb_file(handle, branch)
    })
    .map(|held| held.into_iter().map(PyField::from_inner).collect())
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
/// then `~/.config/fix` when it exists, then a new registry holding the
/// crate's own fields alone. Only the third step treats absence as that
/// default; every other failure is a `ValueError` carrying the native
/// message, and the default stays unresolved so the next call retries.
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

/// One FIX dictionary declaration: a name, a dialect, and a session.
///
/// A branch crosses as `str` wherever it is a *key* - `field.fix.branch`, a
/// `tag:branch` identifier, a lookup - because a key is a name. This class is
/// what a *declaration* is, because a declaration also carries the dialect's
/// default FIX version and the session `CompID`s that select it.
#[pyclass(
    name = "FixBranch",
    module = "yggdryl._native",
    skip_from_py_object,
    frozen
)]
#[derive(Clone)]
pub(crate) struct PyFixBranch {
    inner: CoreFixBranch,
}

impl PyFixBranch {
    pub(crate) const fn from_core(inner: CoreFixBranch) -> Self {
        Self { inner }
    }

    pub(crate) const fn as_inner(&self) -> &CoreFixBranch {
        &self.inner
    }
}

#[pymethods]
impl PyFixBranch {
    /// Declare a branch, validating and folding its name once.
    ///
    /// An empty name is the standard branch, which declares no dialect; the
    /// version is the dialect's own default, spelled the way the
    /// specification spells it.
    #[new]
    #[pyo3(signature = (name = "", *, version = None, aliases = None))]
    fn new(name: &str, version: Option<&str>, aliases: Option<Vec<String>>) -> PyResult<Self> {
        let version = match version {
            Some(text) => text.parse::<yggdryl::Version>().map_err(value_error)?,
            None => yggdryl::Version::default(),
        };
        let branch = CoreFixBranch::from_parts(name, version).map_err(value_error)?;
        match aliases {
            Some(aliases) => branch.with_aliases(aliases).map(Self::from_core),
            None => Ok(Self::from_core(branch)),
        }
        .map_err(value_error)
    }

    /// Parse a branch name, with no dialect.
    #[staticmethod]
    fn from_str(value: &str) -> PyResult<Self> {
        branch_from_py(value).map(Self::from_core)
    }

    /// Accept a branch, or the name of one.
    #[staticmethod]
    fn from_value(value: &Bound<'_, PyAny>) -> PyResult<Self> {
        branch_value_from_py(value)
    }

    /// The FIX specification's own dictionary, and what an absent branch means.
    #[classattr]
    #[pyo3(name = "STANDARD")]
    fn standard() -> Self {
        Self::from_core(CoreFixBranch::STANDARD)
    }

    /// The longest a branch name may be, in bytes.
    #[classattr]
    #[pyo3(name = "MAX_LENGTH")]
    const MAX_LENGTH: usize = CoreFixBranch::MAX_LENGTH;

    /// The canonical lowercase name.
    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    /// The dialect's default FIX version.
    #[getter]
    fn version(&self) -> String {
        self.inner.version().to_string()
    }

    /// The other spellings this dictionary answers to, folded, as declared.
    ///
    /// A lookup spelling and nothing more: the canonical name is what a field
    /// stores and what every identifier packs, so an alias moves no field.
    #[getter]
    fn aliases(&self) -> Vec<&str> {
        self.inner.aliases().iter().map(AsRef::as_ref).collect()
    }

    /// Whether `name` is one of this dictionary's aliases, ASCII case folded.
    ///
    /// The canonical name is not an alias of itself, so this answers `False`
    /// for it.
    fn has_alias(&self, name: &str) -> bool {
        self.inner.has_alias(name)
    }

    /// Whether this is the FIX specification's own dictionary.
    fn is_standard(&self) -> bool {
        self.inner.is_standard()
    }

    /// The identity every identifier of this branch carries.
    ///
    /// The signed reading of the XXH32, which is exactly what an arrival
    /// entry's `branch` carries and what `FixRegistry.branch_by_digest`
    /// takes, so a capture's column joins to a declaration without a
    /// conversion in between. A digest above `i32::MAX` therefore reads
    /// negative; it is the same four bytes either way.
    fn digest(&self) -> i32 {
        self.inner.digest_signed()
    }

    /// The deterministic cross-language hash of the whole declaration.
    ///
    /// Equality is the whole declaration, so the hash is too; `digest` is the
    /// narrower answer, the name identity an identifier carries.
    fn stable_hash(&self) -> u64 {
        Scalar::from_sequence([
            Scalar::from(self.inner.name()),
            Scalar::from(self.inner.version().to_string()),
        ])
        .stable_hash()
    }

    fn __str__(&self) -> &str {
        self.inner.name()
    }

    fn __repr__(&self) -> String {
        format!(
            "FixBranch({:?}, version={:?})",
            self.inner.name(),
            self.inner.version().to_string(),
        )
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.stable_hash())
    }

    fn __richcmp__(
        &self,
        other: &Bound<'_, PyAny>,
        operation: pyo3::class::basic::CompareOp,
    ) -> PyResult<Py<PyAny>> {
        let py = other.py();
        let Ok(other) = branch_value_from_py(other) else {
            return Ok(py.NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(py)?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__(&self) -> (Py<PyAny>, (String, String)) {
        Python::attach(|py| {
            (
                py.get_type::<Self>()
                    .getattr("_from_parts")
                    .unwrap()
                    .unbind(),
                (
                    self.inner.name().to_owned(),
                    self.inner.version().to_string(),
                ),
            )
        })
    }

    /// Rebuild the exact declaration pickle and repr carry.
    #[staticmethod]
    fn _from_parts(name: &str, version: &str) -> PyResult<Self> {
        Self::new(name, Some(version), None)
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// Read a branch declaration, or the name of one.
pub(crate) fn branch_value_from_py(value: &Bound<'_, PyAny>) -> PyResult<PyFixBranch> {
    if let Ok(branch) = value.extract::<PyRef<'_, PyFixBranch>>() {
        return Ok(branch.clone());
    }
    if let Ok(text) = value.extract::<&str>() {
        return branch_from_py(text).map(PyFixBranch::from_core);
    }
    Err(PyTypeError::new_err(
        "expected a yggdryl.fix.FixBranch or a branch name",
    ))
}
