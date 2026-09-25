//! Native Python view of the FIX dictionary, its message, and the default.
//!
//! Nothing here resolves, folds, merges, shards or validates: the registry is
//! one [`Arc`] over the core [`FixRegistry`], and every accessor coerces its
//! key once at the boundary and redirects to the most specific native method.
//! The typed `FIX:` vocabulary is not here either - it lives on the protocol
//! view class [`crate::field::PyProtocolField`], which is what `field.fix`
//! already answers.
//!
//! An identifier crosses as the `int` the core derives from a tag and a name,
//! read once here through [`id_from_py`]; a bare `int` anywhere else is a
//! tag, and a `str` is a name or a path, so one integer never has two
//! readings. A dictionary's contribution is membership on the field -
//! `field.fix.branches` - and never a key a lookup takes.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use pyo3::exceptions::{PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyDateTime, PyDict, PyInt};

use yggdryl::graph::{Element, Event, Market, Operation};
use yggdryl::{
    DataType as CoreDataType, Error as CoreError, Field as CoreField, FixCapture as CoreFixCapture,
    FixCode as CoreFixCode, FixCodeSet as CoreFixCodeSet, FixCodec as CoreFixCodec,
    FixEntry as CoreFixEntry, FixHeader as CoreFixHeader, FixId as CoreFixId, FixKey,
    FixMsg as CoreFixMsg, FixRegistry as CoreFixRegistry, IOBase as CoreIOBase,
    MsgType as CoreMsgType, Scalar, StructType, TimeInForce as CoreTimeInForce, TimeUnit, Timezone,
};

use crate::field::{PyField, core_field_from_value};
use crate::graph::market_data::PyMarketData;
use crate::graph::operation::PyLane;
use crate::graph::{code_scalar, decimal_scalar, idmap_dict, securityids_dict, uuid_scalar};
use crate::iceberg::folder_holder_from_value;
use crate::iobase::{PyIOBase, located_holder};
use crate::iomedia::{batch_reader_from_value, batch_reader_to_pyarrow};
use crate::scalar::{PyScalar, from_py};
use crate::text::codec::{PythonWriter, with_python_bytes};
use crate::text::line::{PyTextLine, core_path_from_value};
use crate::uri::core_url_from_value;
use crate::{Failed, Pulled, python_failure, value_error};

/// Read one dictionary file through whatever Python named it with.
///
/// A `CBlock` and a JSON snapshot are both files, so the location is held as
/// whichever role it actually is rather than as a container: a folder handle
/// reads no bytes, and a reader handed one answers an empty vocabulary instead
/// of a refusal. A handle crosses as itself rather than being rebuilt, so
/// bytes held in memory are readable and no second mapping is opened.
fn read_located<T>(
    location: &Bound<'_, PyAny>,
    read: impl FnOnce(&dyn CoreIOBase) -> yggdryl::Result<T>,
) -> PyResult<T> {
    if let Ok(handle) = location.extract::<PyRef<'_, PyIOBase>>() {
        return read(handle.inner()?.as_io()).map_err(value_error);
    }
    let url = core_url_from_value(location)?;
    read(located_holder(&url)?.as_io()).map_err(value_error)
}

/// One entry as the tuple Python reads: `(tag, name, value, entries)`.
///
/// The native record is a tree - a group's occurrences and a component's
/// members nest under the entry that heads them - and a binding is a view,
/// so the tuple nests the same way rather than flattening into a second
/// shape. Order is the row's.
fn entry_tuple<'py>(py: Python<'py>, entry: &CoreFixEntry) -> PyResult<Bound<'py, PyAny>> {
    let nested = entry
        .entries()
        .iter()
        .map(|held| entry_tuple(py, held))
        .collect::<PyResult<Vec<_>>>()?;
    Ok((entry.tag(), entry.name(), entry.value(), nested)
        .into_pyobject(py)?
        .into_any())
}

/// An optional text as a `repr` spells it: `None`, or the quoted text.
fn repr_text(value: Option<&str>) -> String {
    value.map_or_else(|| "None".to_owned(), |held| format!("{held:?}"))
}

/// One code as the record Python reads: `{"value", "name", "description",
/// "aliases", "group"}`.
///
/// The shape `FIX:directions` already crosses in - one record per entry of
/// the document, every key stated - because a code set is that same kind of
/// document and a binding is a view rather than a second vocabulary. A key
/// the specification said nothing about is `None` rather than absent, so one
/// record reads like the next.
fn code_record<'py>(py: Python<'py>, code: &CoreFixCode) -> PyResult<Bound<'py, PyDict>> {
    let record = PyDict::new(py);
    record.set_item("value", code.value())?;
    record.set_item("name", code.name())?;
    record.set_item("description", code.description())?;
    let aliases: Vec<&str> = code.aliases().iter().map(AsRef::as_ref).collect();
    record.set_item("aliases", aliases)?;
    record.set_item("group", code.group())?;
    Ok(record)
}

/// Every member of one stored set, in the order the document states them.
fn code_records<'py>(
    py: Python<'py>,
    set: CoreFixCodeSet<'_>,
) -> PyResult<Vec<Bound<'py, PyDict>>> {
    set.codes()
        .map(|code| {
            let code = CoreFixCode::from(code.map_err(value_error)?);
            code_record(py, &code)
        })
        .collect()
}

/// One optional text of a code record, absent and `None` reading alike.
fn code_text(record: &Bound<'_, PyAny>, key: &str) -> PyResult<Option<String>> {
    match record.get_item(key) {
        Ok(value) => value.extract::<Option<String>>(),
        Err(_) => Ok(None),
    }
}

/// The codes a Python value states, typed.
///
/// An iterable of records shaped the way `codeset` answers them, where
/// `value` and `name` are the two keys a code has to state and the rest are
/// optional - the one shape `directions` already crosses in, so the two
/// document properties a caller states are stated alike.
fn codes_from_py(codes: &Bound<'_, PyAny>) -> PyResult<Vec<CoreFixCode>> {
    let mut held = Vec::new();
    for record in codes.try_iter()? {
        let record = record?;
        let mut code = CoreFixCode::new(
            record.get_item("name")?.extract::<String>()?,
            record.get_item("value")?.extract::<String>()?,
        );
        if let Some(description) = code_text(&record, "description")? {
            code = code.with_description(description);
        }
        if let Some(group) = code_text(&record, "group")? {
            code = code.with_group(group);
        }
        if let Ok(aliases) = record.get_item("aliases")
            && !aliases.is_none()
        {
            for alias in aliases.try_iter()? {
                code.push_alias(alias?.extract::<String>()?);
            }
        }
        held.push(code);
    }
    Ok(held)
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

/// Read one identifier: the `int` a field's `fix.id` answered.
///
/// An identifier is the integer the core derives from a tag and a name, so
/// it crosses as that integer and nothing is parsed; `bool` is refused the
/// way a tag refuses it, and a value outside `i32` is the `OverflowError`
/// the extraction reports. Nothing is checked beyond that: whether a field
/// stands behind the integer is the registry's answer.
fn id_from_py(value: &Bound<'_, PyAny>) -> PyResult<CoreFixId> {
    if value.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err(
            "a FIX identifier must be an integer, not bool",
        ));
    }
    value.extract::<i32>().map(CoreFixId::from_digest)
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

/// The dictionary a caller named, or the process default where none was.
///
/// Every entry point that resolves against a registry - a message, a codec,
/// a lifecycle, the fixed row - takes the same optional argument and falls
/// back the same way, so the fallback is spelled here once.
fn registry_or_global(
    registry: Option<PyRef<'_, PyFixRegistry>>,
) -> PyResult<Arc<CoreFixRegistry>> {
    match registry {
        Some(held) => Ok(Arc::clone(&held.inner)),
        None => CoreFixRegistry::global()
            .map(Arc::clone)
            .map_err(value_error),
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
/// One namespace of scalar fields, components and repeating groups, each
/// reached through the field doors alone: a Struct is a component, a Serie
/// of Structs or a Map a group, and a message a component carrying
/// `FIX:msgtype`. The registry is mutable, so it is unhashable and compares
/// by the fields it holds. It is held as an `Arc` because a
/// [`FixMsg`][PyFixMsg] links the very registry it was resolved against, a
/// [`MsgType`][PyMsgType] view keeps it, and the process default is one too:
/// a mutation therefore refuses while anything else shares it, rather than
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

    /// A registry holding this crate's own definitions and the standard clocks.
    ///
    /// `fix_crate_fields` lists what the crate adds beside the specification:
    /// its own columns in tag order from 65003 - the clocks, the identities,
    /// the derived facts and the capture's own - and the Map group
    /// `metadata`. Beside them sit two seeded standard
    /// clocks, `SendingTime` (52) and `TransactTime` (60), each a nanosecond
    /// UTC `datetime64`, ordinary definitions a loaded dictionary supplies its
    /// own metadata for; the crate's fields are held by every dictionary
    /// alike. `len` counts the scalar fields, the components and the groups.
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

    /// Load scalar shards and native message, component, and group definitions.
    ///
    /// `location` is an `IOBase` handle or anything that names a folder: a
    /// string, a path-like, a `Url`. A folder that is not there loads as a new
    /// registry - the crate's own fields and the two seeded standard clocks -
    /// and is not created; a stored copy of a crate field is read past,
    /// because the crate's own definition is the one that types a row, while
    /// a stored `SendingTime` or `TransactTime` is the definition the seed
    /// then leaves in place. A shard that does
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
    /// its `grammar-binding`s describe. `dialect` names the dictionary, and
    /// every field, group, component and message root the file produces is
    /// stamped with it in `FIX:branches` - standard tags included, because
    /// membership means the dictionary speaks the field; with none supplied
    /// nothing is stamped.
    ///
    /// A file this cannot be read from is a `ValueError` carrying the native
    /// sentence whole: the byte the reader stopped at, what was expected, what
    /// arrived, and the element the file spells it in.
    #[staticmethod]
    #[pyo3(signature = (location, dialect=None))]
    fn from_cfb_file(
        location: &Bound<'_, PyAny>,
        dialect: Option<&str>,
    ) -> PyResult<(Self, Vec<PyField>)> {
        let (registry, roots) = read_located(location, |handle| {
            CoreFixRegistry::from_cfb_file(handle, dialect)
        })?;
        Ok((
            Self::from_arc(Arc::new(registry)),
            roots.into_iter().map(PyField::from_inner).collect(),
        ))
    }

    /// Fold one field in, adding it when absent and merging it when stored.
    ///
    /// The lenient counterpart of `insert`, which replaces, and of `update`,
    /// which refuses everything new. Answers `True` when the field arrived
    /// and `False` when it folded into a stored one: the same tag under the
    /// same folded name merges, a name folding to a stored canonical name or
    /// alias under another tag merges into that field - aliases, alternate
    /// tags and membership become the union, and the incoming tag joins the
    /// alternates unless another field answers it - the same tag under
    /// another name is added beside the holder, which gains the name as an
    /// alias, a nested field is redirected to `add_definition` under the
    /// category its shape names, and one of this crate's own tags is skipped
    /// as already held.
    ///
    /// One mutation: a refusal - no `FIX:tag`, a datatype disagreeing with
    /// the stored field - leaves the dictionary exactly as it was.
    fn add_field(&mut self, field: &Bound<'_, PyAny>) -> PyResult<bool> {
        let field = core_field_from_value(field)?;
        self.inner_mut()?.add_field(field).map_err(value_error)
    }

    /// Fold `fields` in, adding what is absent and merging what is stored.
    ///
    /// Each entry is anything `Field` accepts. A field whose canonical
    /// identity the dictionary does not hold is inserted, one it holds is
    /// merged, and the answer is the count added and the count merged, in
    /// that order.
    ///
    /// One mutation: the whole fold is staged and only then adopted, so a
    /// refusal - a field with no `FIX:tag`, a datatype disagreeing with the
    /// stored definition - leaves the dictionary exactly as it was.
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
    /// `add_fields` folds one, its membership unioned onto the field it
    /// merges into, and every definition folds beside them.
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
    /// `add_fields` folds any source, every field it produces stamped with
    /// the dialect in `FIX:branches` and that membership unioned onto
    /// whatever it merges into.
    ///
    /// `dialect` names the dictionary, and the location's own stem stands in
    /// when the caller does not; a name that is empty or carries a comma is
    /// a `ValueError`.
    ///
    /// Answers the count added and the count merged. One mutation: a refusal
    /// leaves the dictionary exactly as it was.
    #[pyo3(signature = (location, dialect=None))]
    fn add_cfb_file(
        &mut self,
        location: &Bound<'_, PyAny>,
        dialect: Option<&str>,
    ) -> PyResult<(usize, usize)> {
        let registry = self.inner_mut()?;
        read_located(location, |handle| registry.add_cfb_file(handle, dialect))
    }

    /// Read every Ullink `CBlock` a pattern selects into this dictionary.
    ///
    /// The plural of `add_cfb_file`, over the core's own glob walk: `pattern`
    /// is anchored at `location` the way `IOBase.glob` anchors it - a fixed
    /// prefix is descended rather than listed, `**` spans any number of
    /// levels - and a pattern selecting nothing folds nothing rather than
    /// raising. Private entries are never matched.
    ///
    /// Files fold in ascending URL order whatever order the listing arrived
    /// in, so where two files disagree about one tag the last-sorting file
    /// wins and every spelling of one pattern answers the same dictionary.
    ///
    /// `dialect` is resolved per file: a name supplied here stamps every
    /// matched file with it, and `None` lets each file's own stem stand in,
    /// which is what globbing a folder of counterparty files is for.
    ///
    /// Answers the count of files folded, the count of fields added and the
    /// count merged. One mutation, and one copy of the dictionary for the
    /// whole call: a file that will not parse leaves it exactly as it was and
    /// the `ValueError` names that file.
    #[pyo3(signature = (location, pattern, dialect=None))]
    fn add_cfb_files(
        &mut self,
        location: &Bound<'_, PyAny>,
        pattern: &str,
        dialect: Option<&str>,
    ) -> PyResult<(usize, usize, usize)> {
        // A glob is walked from a container, where a `CBlock` is a leaf.
        let root = folder_holder_from_value(location)?;
        self.inner_mut()?
            .add_cfb_files(root.as_io(), pattern, dialect)
            .map_err(value_error)
    }

    /// Read one JSON registry snapshot into this dictionary, whole.
    ///
    /// The lenient door beside `from_json`, which builds a dictionary of its
    /// own: the file is read through exactly that parse and then folded in
    /// the way `merge_with` folds any dictionary.
    ///
    /// No dialect is taken, and that is the point of the pair: a `CBlock`
    /// states no membership, so `add_cfb_file` has to be told one or guess it
    /// from the stem, while a snapshot is this package's own format and every
    /// field and definition in it already carries the `FIX:branches` its
    /// writer meant.
    ///
    /// Answers the count added and the count merged. One mutation: a document
    /// that does not parse, a reference naming a definition nothing holds, or
    /// a datatype disagreeing with a stored field leaves it as it was, and
    /// the `ValueError` names the file.
    fn add_json_file(&mut self, location: &Bound<'_, PyAny>) -> PyResult<(usize, usize)> {
        let registry = self.inner_mut()?;
        read_located(location, |handle| registry.add_json_file(handle))
    }

    /// Register a message definition and answer its immutable view.
    ///
    /// `spelling` is the wire code, qualified or not - `AR Inbound` is tag
    /// 35 `AR` used one way - and `name` the definition's name, the
    /// spelling itself when none is given.
    #[pyo3(signature = (spelling, name=None, description=None))]
    fn register_msgtype(
        &mut self,
        spelling: &str,
        name: Option<&str>,
        description: Option<&str>,
    ) -> PyResult<PyMsgType> {
        let held = self
            .inner_mut()?
            .register_msgtype(spelling, name, description)
            .map_err(value_error)?
            .clone();
        Ok(PyMsgType::new(Arc::clone(&self.inner), held))
    }

    /// The message definition a spelling reaches, or `None`.
    ///
    /// An exact wire code, a folded canonical name, or an alias of tag 35's
    /// code set, in that order. A code two messages declare under different
    /// names answers the one tag 35's code set names, else the first in name
    /// order; the other is reached by its own name.
    fn get_msgtype(&self, spelling: &str) -> Option<PyMsgType> {
        self.inner
            .get_msgtype(spelling)
            .map(|held| PyMsgType::new(Arc::clone(&self.inner), held.clone()))
    }

    /// The message definition a spelling reaches; absence is a `KeyError`.
    fn msgtype(&self, spelling: &str) -> PyResult<PyMsgType> {
        self.inner
            .msgtype(spelling)
            .map(|held| PyMsgType::new(Arc::clone(&self.inner), held.clone()))
            .map_err(|error| absent(&error))
    }

    /// The members of the code set `name`, or `None`.
    ///
    /// The lenient door beside `codeset`, which raises absence: a caller
    /// asking whether a vocabulary is held asks this. The name folds the way
    /// every name folds, so `SideCodeSet` and `sidecodeset` are one set.
    fn get_codeset<'py>(
        &self,
        py: Python<'py>,
        name: &str,
    ) -> PyResult<Option<Vec<Bound<'py, PyDict>>>> {
        self.inner
            .get_codeset(name)
            .map(|set| code_records(py, set))
            .transpose()
    }

    /// The members of the code set `name`; absence is a `KeyError`.
    ///
    /// A field states the name of the set it reads by and the dictionary
    /// holds the members, once, under that name: this is the door between
    /// them. Each member is a record - `value`, `name`, `description`,
    /// `aliases`, `group` - in the order the set states them, which is the
    /// presentation rank the specification gives each code.
    fn codeset<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let set = self.inner.codeset(name).map_err(|error| absent(&error))?;
        code_records(py, set)
    }

    /// The members of the set `field` reads by, or `None`.
    ///
    /// `field` is anything `Field` accepts. A field naming no set answers
    /// `None`; a held field never names a set this dictionary lacks, because
    /// every door a field arrives through refuses one that does.
    fn codeset_of<'py>(
        &self,
        py: Python<'py>,
        field: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Vec<Bound<'py, PyDict>>>> {
        let field = core_field_from_value(field)?;
        self.inner
            .codeset_of(&field)
            .map(|set| code_records(py, set))
            .transpose()
    }

    /// The name of every code set held, in name order.
    ///
    /// The names alone, and named so: the shipped dictionary holds hundreds
    /// of sets and one of them is read by a hundred and three fields, so
    /// the members are asked for one set at a time through `codeset`. The
    /// JavaScript view spells it `codesetNames` for the same reason.
    fn codeset_names(&self) -> Vec<String> {
        self.inner
            .codesets()
            .map(|set| set.name().to_owned())
            .collect()
    }

    /// Every field that names a message by an identifier, one record per
    /// key its `FIX:idmap` states: `{"tag", "map", "key", "follow",
    /// "role"}`, in tag order. A message rebuilds its `accountids`,
    /// `userids` and `altids` from these, and an operation that follows
    /// another carries the `altids` keys whose record follows.
    fn idmap_sources<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let mut records = Vec::new();
        for (tag, source) in self.inner.idmap_sources() {
            let record = PyDict::new(py);
            record.set_item("tag", tag)?;
            record.set_item("map", source.map().as_str())?;
            record.set_item("key", source.key())?;
            record.set_item("follow", source.follows())?;
            record.set_item("role", source.role())?;
            records.push(record);
        }
        Ok(records)
    }

    /// State the members of the code set `name`, replacing what it held.
    ///
    /// `codes` is an iterable of records - `value` and `name` are required,
    /// `description`, `aliases` and `group` optional - or the canonical JSON
    /// document a store writes, as one `str`. The set is filed under the
    /// folded name, and two names may share a value - that is an alias - but
    /// two codes may not share a name.
    ///
    /// An empty list removes the set, exactly as an empty tag or alias list
    /// removes its own property; removing one a held field still reads by is
    /// a `ValueError`, because a field may not be left naming a vocabulary
    /// nothing states. `msgcatcodeset` is intrinsic: its stable integer market
    /// operation IDs cannot be replaced or removed. One mutation: a refusal
    /// leaves the dictionary exactly as it was.
    fn set_codeset(&mut self, name: &str, codes: &Bound<'_, PyAny>) -> PyResult<()> {
        let codes = codes_from_py(codes)?;
        self.inner_mut()?
            .set_codeset(name, &codes)
            .map_err(value_error)
    }

    /// Fold `codes` into the code set `name`, keeping what it already held.
    ///
    /// The fold is by wire value: a placeholder name yields to a real one,
    /// every surviving spelling is kept as an alias, and a description or a
    /// group either side stated stays. So a venue's statement of a set
    /// enriches the one the dictionary holds rather than replacing it, and a
    /// set no dictionary held yet arrives whole. `msgcatcodeset` is intrinsic
    /// and refuses any merge that would change its stable integer IDs.
    fn merge_codeset(&mut self, name: &str, codes: &Bound<'_, PyAny>) -> PyResult<()> {
        let codes = codes_from_py(codes)?;
        self.inner_mut()?
            .merge_codeset(name, &codes)
            .map_err(value_error)
    }

    /// Remove the code set `name`, answering the members it held.
    ///
    /// A set nothing holds answers `None`. A set a held field still reads by
    /// is a `ValueError` naming that field: the field is moved to another set
    /// first, or removed with it. `msgcatcodeset` is intrinsic and cannot be
    /// removed.
    fn remove_codeset<'py>(
        &mut self,
        py: Python<'py>,
        name: &str,
    ) -> PyResult<Option<Vec<Bound<'py, PyDict>>>> {
        self.inner_mut()?
            .remove_codeset(name)
            .map_err(value_error)?
            .map(|codes| {
                codes
                    .iter()
                    .map(|code| code_record(py, code))
                    .collect::<PyResult<Vec<_>>>()
            })
            .transpose()
    }

    /// The repeating group one counter tag opens, or `None`.
    ///
    /// `tag` is the counter's: `get_field_by_tag` answers the counter itself
    /// off the same key, and the group it heads is a definition of its own,
    /// reached here or by its name. Two groups on one counter name nothing.
    fn get_field_by_counter(&self, tag: FixTag) -> Option<PyField> {
        self.inner
            .get_field_by_counter(tag.0)
            .cloned()
            .map(PyField::from_inner)
    }

    /// The repeating group one counter tag opens; absence is a `KeyError`.
    fn field_by_counter(&self, tag: FixTag) -> PyResult<PyField> {
        self.inner
            .field_by_counter(tag.0)
            .map(|field| PyField::from_inner(field.clone()))
            .map_err(|error| absent(&error))
    }

    #[staticmethod]
    fn from_json(document: &str) -> PyResult<Self> {
        CoreFixRegistry::from_json(document)
            .map(|inner| Self::from_arc(Arc::new(inner)))
            .map_err(value_error)
    }

    #[allow(clippy::wrong_self_convention)]
    fn into_json(&self) -> PyResult<String> {
        self.inner.into_json().map_err(value_error)
    }

    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (String,))> {
        Ok((
            py.get_type::<Self>().getattr("from_json")?.unbind(),
            (self.into_json()?,),
        ))
    }

    fn __copy__(&self) -> Self {
        Self::from_arc(Arc::new(self.inner.as_ref().clone()))
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.__copy__()
    }

    /// Write every populated field shard and named definition under
    /// `location`, removing the files no field or definition populates any
    /// more, and answering what moved.
    ///
    /// Shards are named by their tag's hundred, nine digits wide - tag 55
    /// lands in `fields/000000000.json`, tag 65003 in
    /// `fields/000000650.json` - beside `components/` and `groups/`. The
    /// crate's own definitions are written like every other, the fixed row
    /// among them as `components/fixmsg.json`, so a store states the whole
    /// row rather than the half it declared itself; a reader takes the
    /// definition it holds from construction over the document it finds
    /// there.
    /// Commits the store and answers nothing.
    ///
    /// The same work as `commit` for a caller that does not read what moved.
    fn write_into(&self, location: &Bound<'_, PyAny>) -> PyResult<()> {
        let mut holder = folder_holder_from_value(location)?;
        self.inner.write_into(&mut holder).map_err(value_error)
    }

    /// Each document is digested where it lies and left alone where it
    /// already states this registry, so a commit writes what moved and a
    /// second commit of one registry writes nothing. The report is an
    /// ordinary mapping: `written` and `removed` name the documents, in the
    /// order a store lays them out, and `skipped` counts the ones a run left.
    fn commit<'py>(
        &self,
        python: Python<'py>,
        location: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let mut holder = folder_holder_from_value(location)?;
        let report = self.inner.commit(&mut holder).map_err(value_error)?;
        let answer = PyDict::new(python);
        answer.set_item(
            "written",
            report
                .written
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        )?;
        answer.set_item("skipped", report.skipped)?;
        answer.set_item(
            "removed",
            report
                .removed
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        )?;
        Ok(answer)
    }

    /// The field one identifier names exactly, or `None`.
    ///
    /// `id` is the `int` a field's `fix.id` answers - the identity of its
    /// tag under its name - so this is exact: no alias, alternate tag or
    /// fold is consulted.
    fn get_field_by_id(&self, id: &Bound<'_, PyAny>) -> PyResult<Option<PyField>> {
        let id = id_from_py(id)?;
        Ok(self
            .inner
            .get_field_by_id(id)
            .cloned()
            .map(PyField::from_inner))
    }

    /// The field one identifier names exactly.
    fn field_by_id(&self, id: &Bound<'_, PyAny>) -> PyResult<PyField> {
        let id = id_from_py(id)?;
        self.inner
            .field_by_id(id)
            .map(|field| PyField::from_inner(field.clone()))
            .map_err(|error| absent(&error))
    }

    /// The field a canonical or alternate tag names, or `None`.
    ///
    /// The canonical holder of the tag answers first, then a field holding
    /// it as an alternate, then a component or a group by the tag that is
    /// its identity in the catalog.
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

    /// The field a canonical name or alias names, ASCII case folded, or
    /// `None`.
    ///
    /// A scalar field answers first - the canonical name before an alias,
    /// under the one fold - then a component, then a group.
    fn get_field_by_name(&self, name: &str) -> Option<PyField> {
        self.inner
            .get_field_by_name(name)
            .cloned()
            .map(PyField::from_inner)
    }

    /// The field a canonical name or alias names, raising absence.
    fn field_by_name(&self, name: &str) -> PyResult<PyField> {
        self.inner
            .field_by_name(name)
            .map(|field| PyField::from_inner(field.clone()))
            .map_err(|error| absent(&error))
    }

    /// The field a path reaches through a component or a group.
    fn get_field_by_path(&self, path: &Bound<'_, PyAny>) -> PyResult<Option<PyField>> {
        let path = core_path_from_value(path)?;
        Ok(self
            .inner
            .get_field_by_path(&path)
            .cloned()
            .map(PyField::from_inner))
    }

    /// The field a path reaches through a component or a group.
    ///
    /// A position is spelled the way the one grammar spells it -
    /// ``Parties[0].PartyID`` - and a schema answers the item every
    /// occurrence of a group holds, so that spelling reaches the member here
    /// as well as in a message.
    fn field_by_path(&self, path: &Bound<'_, PyAny>) -> PyResult<PyField> {
        let path = core_path_from_value(path)?;
        self.inner
            .field_by_path(&path)
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
    ///
    /// Filed by its shape: a Struct is a component, a Serie of Structs or a
    /// Map a group, anything else a scalar field. A scalar arriving on a tag
    /// another field holds under another name is a field of its own, added
    /// beside the holder, which gains the arrival's name as an alias; a
    /// component or a group replaces the definition its folded name reaches.
    fn insert(&mut self, field: &Bound<'_, PyAny>) -> PyResult<Option<PyField>> {
        let field = core_field_from_value(field)?;
        Ok(self
            .inner_mut()?
            .insert(field)
            .map_err(value_error)?
            .map(PyField::from_inner))
    }

    /// Merge a definition into the stored one with the same identity: the
    /// same tag under the same folded name for a scalar, the folded name for
    /// a component or a group. A name folding to the stored one keeps the
    /// stored canonical spelling; a definition nothing holds is refused.
    fn update(&mut self, field: &Bound<'_, PyAny>) -> PyResult<()> {
        let field = core_field_from_value(field)?;
        self.inner_mut()?.update(field).map_err(value_error)
    }

    /// Remove what a tag or a name reaches, answering it.
    ///
    /// A scalar field a tag, a name or an alias reaches, else the component
    /// or the group a name spells. A definition another one references
    /// stays, as its members stay, and answers `None`.
    fn remove(&mut self, key: &Bound<'_, PyAny>) -> PyResult<Option<PyField>> {
        let key = FixKeyArg::from_py(key)?;
        Ok(self
            .inner_mut()?
            .remove(key.as_key())
            .map(PyField::from_inner))
    }

    /// Every dictionary name any field or definition carries in
    /// `FIX:branches`, distinct and sorted.
    ///
    /// Membership is provenance a caller filters on; no lookup consults it.
    fn dialects(&self) -> Vec<String> {
        self.inner.dialects()
    }

    /// Remove the field one identifier names exactly, answering it.
    ///
    /// The generic `remove` reads an `int` as a tag; this is how a field
    /// sharing its tag with another leaves the dictionary on its own.
    fn remove_by_id(&mut self, id: &Bound<'_, PyAny>) -> PyResult<Option<PyField>> {
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

    /// The scalar fields in ascending canonical-identifier order, lazily.
    ///
    /// The order is the core's: tag-major, then by identifier. The iterator holds
    /// the registry's `Arc` and the identifier it stopped at, so nothing is
    /// collected crossing the boundary and the dictionary is never cloned to
    /// walk it. Holding it is therefore sharing it: a mutation refuses while a
    /// walk is unfinished, which is what stops the vector moving under a
    /// cursor into it. The components and the groups `len` counts are not
    /// walked here: each is reached by its name or its counter.
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
/// dictionary of thousands costs one lookup, and a walk crosses every field
/// in the one order the core iterates.
#[pyclass(name = "FixFieldIterator", module = "yggdryl._native")]
pub(crate) struct PyFixFieldIterator {
    registry: Arc<CoreFixRegistry>,
    after: Option<CoreFixId>,
    taken: usize,
    done: bool,
}

/// One message definition, as the registry holds it.
///
/// The core borrows one out of its registry; a Python value cannot, so this
/// is a copy of the definition beside the registry it came from, which it
/// keeps shared - a mutation of that registry refuses while the view lives,
/// as it refuses while a message shares it. Immutable, so it hashes, orders,
/// copies and pickles by the definition's own field.
#[pyclass(
    name = "MsgType",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyMsgType {
    registry: Arc<CoreFixRegistry>,
    inner: CoreMsgType,
}

impl PyMsgType {
    const fn new(registry: Arc<CoreFixRegistry>, inner: CoreMsgType) -> Self {
        Self { registry, inner }
    }

    const fn inner(&self) -> &CoreMsgType {
        &self.inner
    }
}

#[pymethods]
impl PyMsgType {
    /// The definition's canonical name.
    #[getter]
    fn name(&self) -> &str {
        self.inner().name()
    }

    /// The exact wire code, case and every non-control character kept.
    #[getter]
    fn value(&self) -> &str {
        self.inner().as_str()
    }

    /// The definition's symbolic business-category name, or `None`.
    #[getter]
    fn msgcat(&self) -> Option<&str> {
        self.inner().msgcat()
    }

    /// The definition's own Struct field, read-only.
    #[getter]
    fn field(&self) -> PyField {
        PyField::from_inner_with_read_only(self.inner().as_field().clone(), true)
    }

    /// The group this message declares under one counter tag, or `None`.
    fn get_group_by_tag(&self, tag: FixTag) -> Option<PyField> {
        self.inner()
            .get_group_by_tag(tag.0)
            .cloned()
            .map(PyField::from_inner)
    }

    /// This component's non-null identifiers at the message's own level.
    ///
    /// The result follows component order, without descending into groups.
    /// Declaration fields are read-only; values retain their native types.
    fn identifier_values(&self, message: &PyFixMsg) -> Vec<(PyField, PyScalar)> {
        self.inner()
            .identifier_values(message.as_inner())
            .map(|(field, value)| {
                (
                    PyField::from_inner_with_read_only(field.clone(), true),
                    PyScalar::from_inner(value.clone()),
                )
            })
            .collect()
    }

    fn stable_hash(&self) -> u64 {
        self.inner().stable_hash()
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.stable_hash())
    }

    fn __richcmp__(
        &self,
        py: Python<'_>,
        other: &Bound<'_, PyAny>,
        operation: pyo3::class::basic::CompareOp,
    ) -> Py<PyAny> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return py.NotImplemented();
        };
        PyBool::new(
            py,
            crate::compare(self.inner().cmp(other.inner()), operation),
        )
        .to_owned()
        .into_any()
        .unbind()
    }

    fn __str__(&self) -> &str {
        self.inner().as_str()
    }

    fn __repr__(&self) -> String {
        format!("MsgType({:?}, {:?})", self.name(), self.value())
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Rebuild a view from the dictionary's snapshot and the definition's
    /// name, which is the one spelling that reaches it alone.
    #[staticmethod]
    fn _from_pickle(registry: &str, name: &str) -> PyResult<Self> {
        let registry = Arc::new(CoreFixRegistry::from_json(registry).map_err(value_error)?);
        let inner = registry
            .get_msgtype(name)
            .filter(|held| held.name() == name)
            .cloned()
            .ok_or_else(|| {
                PyValueError::new_err(format!(
                    "no message definition named {name:?} in the registry"
                ))
            })?;
        Ok(Self { registry, inner })
    }

    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (String, String))> {
        Ok((
            py.get_type::<Self>().getattr("_from_pickle")?.unbind(),
            (
                self.registry.into_json().map_err(value_error)?,
                self.inner.name().to_owned(),
            ),
        ))
    }
}

/// One captured line, as the bytes it is.
fn line_bytes(item: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    with_python_bytes(
        item,
        "a captured line must be bytes, bytearray, or memoryview",
        |bytes| Ok(bytes.to_vec()),
    )
}

/// One message, refusing anything else where it is met.
fn message_of(item: &Bound<'_, PyAny>) -> PyResult<CoreFixMsg> {
    Ok(item.extract::<PyRef<'_, PyFixMsg>>()?.inner.clone())
}

/// A stream of messages, one at a time.
///
/// Every stage of the codec answers one of these - one line's messages, a
/// stream of lines parsed, records parsed, messages chained by the
/// lifecycle, a batch read back - so a message stream has one shape at this
/// boundary whatever made it. Nothing is collected: the core iterator is the
/// stream, and a Python iterable behind it is pulled one item at a time. A
/// line the reader refuses, or a message a stage refuses, raises `ValueError`
/// where it is met and the stream goes on past it; a Python failure behind
/// the stream raises as itself and ends it.
#[pyclass(name = "FixMessages", module = "yggdryl._native")]
pub(crate) struct PyFixMessages {
    /// The stream, behind the lock a class shared between threads needs; a
    /// batch reader behind it is `Send` and nothing more, and the lock is
    /// never contended because a cursor is advanced by one caller.
    inner: Mutex<Box<dyn Iterator<Item = yggdryl::Result<CoreFixMsg>> + Send>>,
    /// Where the Python source behind `inner` failed, when there is one.
    failed: Option<Failed>,
}

impl PyFixMessages {
    /// A stream over a core iterator that pulls nothing from Python.
    fn over<I>(inner: I) -> Self
    where
        I: Iterator<Item = yggdryl::Result<CoreFixMsg>> + Send + 'static,
    {
        Self {
            inner: Mutex::new(Box::new(inner)),
            failed: None,
        }
    }

    /// A stream over a core stage fed by a Python iterable.
    fn pulling<I>(inner: I, failed: Failed) -> Self
    where
        I: Iterator<Item = yggdryl::Result<CoreFixMsg>> + Send + 'static,
    {
        Self {
            inner: Mutex::new(Box::new(inner)),
            failed: Some(failed),
        }
    }
}

#[pymethods]
impl PyFixMessages {
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }
    fn __next__(&mut self) -> PyResult<Option<PyFixMsg>> {
        let next = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .next();
        match next {
            Some(held) => held
                .map(PyFixMsg::from_inner)
                .map(Some)
                .map_err(value_error),
            None => match self.failed.as_ref().and_then(Failed::take) {
                Some(error) => Err(error),
                None => Ok(None),
            },
        }
    }
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
/// and only through a Serie's item; a `Map` field keeps its mapping and every
/// other value crosses untouched. Nothing is typed, ordered or validated
/// here - that is `Field::canonicalize_value`'s work, on what this hands it.
fn named_rows(field: &CoreField, value: Scalar) -> Scalar {
    match field.dtype() {
        CoreDataType::Struct(_) => {
            let children = field.fields();
            if let Some(items) = value.sequence_rows() {
                if items.len() == children.len() {
                    let row: Vec<Scalar> = children
                        .iter()
                        .zip(items.iter())
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
                .and_then(|named| Scalar::from_struct(named).ok())
                .unwrap_or(value)
        }
        sequence_dtype @ (CoreDataType::Serie(_)
        | CoreDataType::SerieView(_)
        | CoreDataType::FixedSizeSerie(..)
        | CoreDataType::LargeSerie(_)
        | CoreDataType::LargeSerieView(_)) => {
            let sequence = &sequence_dtype
                .as_serie_type()
                .expect("the variant was just matched");
            let item = sequence.item();
            let Some(entries) = value.sequence_rows() else {
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
/// parts it needs - the schema as JSON, the row as the variant encoding,
/// and the dictionary's fields as JSON.
type MsgPickle = (Py<PyAny>, (String, Py<PyAny>, String));

/// A FIX message: a typed market event with a content row, against the
/// registry that types it.
///
/// The typed facts live in three holders and two extras - the `event()`
/// the graph vocabulary answers, the standard `header()`, what the
/// `capture()` said about the line, the free `text` and a bridge's own
/// `metadata` - and the row holds everything else the message states: the
/// dictionary's fields, groups as series beside their counter, components
/// as structs. The schema is one non-null Struct `Field` - the only row
/// schema - and the value the row it declares, so a mapping input is
/// canonicalized into that order by the core exactly as every other row is,
/// and a child stating a typed fact fills the holder that owns it and leaves
/// the row. A lookup by a typed tag answers the holder; every other key
/// reaches the row. The row and the holders are written through `set` and
/// `remove`, and every write settles the identity again. The message
/// hashes, pickles, copies and compares by its facts and its row, against
/// the registry it was resolved against - and a hashed message is frozen,
/// which is Python's contract for a hash, so a write after `hash()` refuses
/// and a copy is what takes it.
#[pyclass(name = "FixMsg", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PyFixMsg {
    inner: CoreFixMsg,
    /// Whether `hash()` was answered, after which the row may not move.
    hash_locked: bool,
}

impl PyFixMsg {
    /// Wrap a message the core built.
    pub(crate) const fn from_inner(inner: CoreFixMsg) -> Self {
        Self {
            inner,
            hash_locked: false,
        }
    }

    /// Refuse a write to a message something hashed.
    fn require_mutable(&self) -> PyResult<()> {
        if self.hash_locked {
            return Err(PyTypeError::new_err(
                "a hashed FixMsg is frozen; copy it before mutation",
            ));
        }
        Ok(())
    }

    /// Borrow the message the core holds.
    pub(crate) const fn as_inner(&self) -> &CoreFixMsg {
        &self.inner
    }

    /// Wrap an answered value.
    fn answered(value: Option<Scalar>) -> Option<PyScalar> {
        value.map(PyScalar::from_inner)
    }

    /// The typed facts as the columns that state them, each under the
    /// dictionary's field for its tag - or, for a tag the dictionary does
    /// not hold, a field inferred from the value and carrying the tag - so
    /// a rebuild from the row lifts them back onto their holders.
    fn typed_columns(&self) -> PyResult<(Vec<CoreField>, Vec<Scalar>)> {
        let registry = self.inner.registry();
        let mut fields = Vec::new();
        let mut values = Vec::new();
        // The core publishes what a message holds typed, so this walk never
        // keeps a list of its own: a tag lifted or retired there would
        // otherwise drop out of a pickle without a word.
        let tags = yggdryl::FIX_TYPED_TAGS
            .into_iter()
            .chain(yggdryl::CRATE_TAG_MIN..yggdryl::CRATE_TAG_MAX);
        for tag in tags {
            let Some(value) = self.inner.get_by_tag(tag) else {
                continue;
            };
            let known = registry
                .get_field_by_tag(tag)
                .or_else(|| registry.get_field_by_counter(tag));
            let field = if let Some(known) = known {
                known.clone()
            } else {
                let mut field = value
                    .dtype()
                    .map_err(value_error)?
                    .nullable_field(format!("{tag}"));
                field.as_fix_mut().set_tag(tag).map_err(value_error)?;
                field
            };
            fields.push(field);
            values.push(value);
        }
        Ok((fields, values))
    }
}

#[pymethods]
impl PyFixMsg {
    /// Build a message, linking the process default when none is named.
    ///
    /// `value` is anything the `Scalar` boundary reads - a native `Scalar`, a
    /// mapping of names, a sequence in the root's own order - and is
    /// validated and canonicalized against `field` by the core. A child
    /// stating a typed fact - a header or trailer tag, a crate column, one
    /// of the FIX fields a message lifts, `Text(58)` - fills the holder that
    /// owns it and leaves the row. The clocks settle: `SendingTime` is the stated one,
    /// else UTC now, so a message meant to compare equal to another states
    /// one; the instant `currunix` is the stated one, else the official
    /// transaction clock standing within the crate's default one-second
    /// delay of `SendingTime` - a `TransactTime`, else a ranked
    /// `TrdRegTimestamp` - else `SendingTime` itself, and the creation the
    /// stated one, else the instant. What `OrigSendingTime` says is the
    /// lifecycle's to read.
    /// The identity is then derived:
    /// the cross code from the first stated of `OrderID`,
    /// `ClOrdID`, `OrigClOrdID`, `QuoteID`, `QuoteReqID` and `MDReqID`; the
    /// hash code over the facts, the lifted fields and the row, the
    /// standard header and trailer left out; and the identities from both.
    #[new]
    #[pyo3(signature = (field, value, registry=None))]
    fn new(
        field: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
        registry: Option<PyRef<'_, PyFixRegistry>>,
    ) -> PyResult<Self> {
        let field = core_field_from_value(field)?;
        let value = named_rows(&field, from_py(value)?);
        CoreFixMsg::with_registry(registry_or_global(registry)?, field, value)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// The message a fixed row holds: the inverse of `into_row`.
    ///
    /// `schema` is the row's root - the one `fix_schema` or
    /// `fix_schema_carrying` built, or the one read off a batch - and `row`
    /// anything the `Scalar` boundary reads as it: a native `Scalar`, a
    /// mapping of names, a sequence in the schema's order. The typed
    /// columns fill the holders, while `fixentries` carries only content the
    /// columns did not represent. `from_row` combines both, so a row without
    /// that residual column still rebuilds its projected content; a wire is
    /// rendered in canonical schema order rather than its arrival order. A
    /// capture's own column is carried - the one the crate tags,
    /// `sourceurl`, and every column no tag and no counter names - each
    /// under its name, as `carried` answers, and held as no fact; a column
    /// whose name holds a `.` is a bridge's own statement and lands in the
    /// metadata. `into_row` states each carried cell again at its column.
    /// Nothing is parsed again and no clock is read: a row carries the
    /// instant, the creation and the identities its message settled, and a
    /// row that does not fit the schema is a located `ValueError`.
    /// `registry` defaults to the process one.
    #[staticmethod]
    #[pyo3(signature = (schema, row, registry=None))]
    fn from_row(
        schema: &Bound<'_, PyAny>,
        row: &Bound<'_, PyAny>,
        registry: Option<PyRef<'_, PyFixRegistry>>,
    ) -> PyResult<Self> {
        let schema = core_field_from_value(schema)?;
        let row = named_rows(&schema, from_py(row)?);
        CoreFixMsg::from_row(registry_or_global(registry)?, &schema, &row)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// Rebuild a message from the three parts pickle carried: the schema
    /// as JSON, the row as the variant encoding, the registry as JSON.
    #[staticmethod]
    fn _from_pickle(field: &str, value: &[u8], registry: &str) -> PyResult<Self> {
        let field = CoreField::from_json(field).map_err(value_error)?;
        let value = Scalar::decode_value_bytes(value).map_err(value_error)?;
        let registry = CoreFixRegistry::from_json(registry).map_err(value_error)?;
        CoreFixMsg::with_registry(Arc::new(registry), field, value)
            .map(Self::from_inner)
            .map_err(value_error)
    }

    /// The registry this message resolves against, sharing it.
    #[getter]
    fn registry(&self) -> PyFixRegistry {
        PyFixRegistry::from_arc(Arc::clone(self.inner.registry()))
    }

    /// The root Struct field: the content row's schema, holding every child
    /// the message states beyond its typed facts.
    #[getter]
    fn field(&self) -> PyField {
        PyField::from_inner(self.inner.as_field().clone())
    }

    /// The ordered content row.
    #[getter]
    fn value(&self) -> PyScalar {
        PyScalar::from_inner(self.inner.as_value().clone())
    }

    /// The value an identifier names, or `None`.
    ///
    /// `id` is the `int` a field's `fix.id` answers; the lookup is exact, so
    /// a field this message's dictionary does not hold simply misses. A
    /// typed tag answers its holder, any other the row.
    fn get_by_id(&self, id: &Bound<'_, PyAny>) -> PyResult<Option<PyScalar>> {
        let id = id_from_py(id)?;
        Ok(Self::answered(self.inner.get_by_id(id)))
    }

    /// The value an identifier names; absence is a `KeyError`.
    fn by_id(&self, id: &Bound<'_, PyAny>) -> PyResult<PyScalar> {
        let id = id_from_py(id)?;
        self.inner
            .by_id(id)
            .map(PyScalar::from_inner)
            .map_err(|error| absent(&error))
    }

    /// The value a tag names, or `None`.
    ///
    /// A tag the typed holders own - a header or trailer tag, a crate
    /// column, one of the FIX fields a message lifts, `Text(58)` - answers
    /// the fact the holder states, typed as its column is: `by_tag(35)` is
    /// the type as text, `by_tag(52)` the sending clock as a nanosecond UTC
    /// `datetime64`, `by_tag(44)` an exact price, a crate identity a
    /// `uuid`. Any other tag reaches the row: the canonical holder of the
    /// tag answers first, then the child named as the dictionary names the
    /// tag, then the child named by the tag's decimal spelling.
    fn get_by_tag(&self, tag: FixTag) -> Option<PyScalar> {
        Self::answered(self.inner.get_by_tag(tag.0))
    }

    /// The value a tag names; absence is a `KeyError`.
    fn by_tag(&self, tag: FixTag) -> PyResult<PyScalar> {
        self.inner
            .by_tag(tag.0)
            .map(PyScalar::from_inner)
            .map_err(|error| absent(&error))
    }

    /// The value a name reaches, or `None`.
    ///
    /// The name folds through the registry to its canonical spelling - a
    /// typed fact answers from its holder - and an exact root-child match
    /// is the fallback when the registry does not know it.
    fn get_by_name(&self, name: &str) -> Option<PyScalar> {
        Self::answered(self.inner.get_by_name(name))
    }

    /// The value a name reaches; absence is a `KeyError`.
    fn by_name(&self, name: &str) -> PyResult<PyScalar> {
        self.inner
            .by_name(name)
            .map(PyScalar::from_inner)
            .map_err(|error| absent(&error))
    }

    /// The value a path reaches, or `None`.
    fn get_by_path(&self, path: &Bound<'_, PyAny>) -> PyResult<Option<PyScalar>> {
        let path = core_path_from_value(path)?;
        Ok(Self::answered(self.inner.get_by_path(&path)))
    }

    /// The value a path reaches; absence is a `KeyError`.
    ///
    /// A position is spelled the way the one grammar spells it:
    /// ``Parties[0].PartyID``.
    fn by_path(&self, path: &Bound<'_, PyAny>) -> PyResult<PyScalar> {
        let path = core_path_from_value(path)?;
        self.inner
            .by_path(&path)
            .map(PyScalar::from_inner)
            .map_err(|error| absent(&error))
    }

    /// The value a tag or a name reaches, or `default`.
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
            .map(PyScalar::from_inner)
            .map_err(|error| absent(&error))
    }

    /// Writes one value into the message, typed by the field the key
    /// resolves to.
    ///
    /// `key` is a tag or a name, resolved as a lookup resolves one through
    /// the dictionary. A key reaching a typed fact - a header or trailer
    /// tag, a crate column, one of the FIX fields a message lifts - records
    /// it on the holder that owns it, and `None` clears it. A key reaching the capture's
    /// own column - `sourceurl` (65026), by tag or by name - is a located
    /// `ValueError`: a message holds no fact for it, and a row child would
    /// put it on the wire. Any other key lands in the row: a
    /// known field types the value through the core's value contract, `None`
    /// is stored as a stated null, an existing child is replaced where it
    /// stands and an absent one appended, a name the dictionary does not
    /// know still reaches a child spelled that way, and a bare tag no
    /// dictionary explains appends a text child named by its decimal. Every
    /// write settles the identity again - the hash code, the identities and
    /// the cross code where it named none - and the entries and the wire
    /// follow the row, so `into_bytes` re-emits the message as it now stands.
    ///
    /// A key reaching no field and no child is a `KeyError` naming it, a value
    /// the field refuses a `ValueError`, and either leaves the message as it
    /// was. A hashed message is frozen and refuses with `TypeError`.
    fn set(&mut self, key: &Bound<'_, PyAny>, value: &Bound<'_, PyAny>) -> PyResult<()> {
        self.require_mutable()?;
        let key = FixKeyArg::from_py(key)?;
        let value = from_py(value)?;
        self.inner
            .set(key.as_key(), value)
            .map_err(|error| absent(&error))
    }

    /// Removes what a key reaches, answering the value it held, or `None`.
    ///
    /// The key resolves as `set` resolves one: a typed fact is cleared on
    /// its holder, a row child is removed and the children after it move
    /// up. A key reaching nothing answers `None` and changes nothing. The
    /// identity is settled again. A hashed message is frozen and refuses
    /// with `TypeError`.
    fn remove(&mut self, key: &Bound<'_, PyAny>) -> PyResult<Option<PyScalar>> {
        self.require_mutable()?;
        let key = FixKeyArg::from_py(key)?;
        Ok(self
            .inner
            .remove(key.as_key())
            .map_err(|error| absent(&error))?
            .map(PyScalar::from_inner))
    }

    /// The `(name, value)` pairs of the content row, in the order its root
    /// declares; the typed facts are the holders' to answer.
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
        self.inner.stable_hash()
    }

    /// Freezes the row, then hashes the schema and the value.
    fn __hash__(&mut self) -> isize {
        self.hash_locked = true;
        crate::python_hash(self.stable_hash())
    }

    /// Two messages are equal when they state the same facts and the same
    /// row against the same dictionary.
    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> Py<PyAny> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return py.NotImplemented();
        };
        pyo3::types::PyBool::new(py, self.inner == other.inner)
            .to_owned()
            .into_any()
            .unbind()
    }

    /// Carry the row, the typed facts and the dictionary's fields as
    /// documents.
    ///
    /// The typed facts travel as columns appended behind the content row,
    /// each under the field that types it, so the rebuild lifts them back
    /// onto their holders exactly as a parse does, and all three documents
    /// travel through the JSON paths a field and a value already have. The
    /// message a pickle rebuilds is equal to the one it came from - registry
    /// included, which equality compares - provided it stated its
    /// `SendingTime`: a settled one is stated again by the rebuild.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<MsgPickle> {
        let callable = py.get_type::<Self>().getattr("_from_pickle")?.unbind();
        let (typed_fields, typed_values) = self.typed_columns()?;
        let root = self.inner.as_field();
        let mut members = root.fields().to_vec();
        members.extend(typed_fields);
        let mut values = self
            .inner
            .as_value()
            .sequence_rows()
            .map(Cow::into_owned)
            .unwrap_or_default();
        values.extend(typed_values);
        let mut field = root.clone();
        field
            .set_dtype(
                StructType::from_fields(members)
                    .map(CoreDataType::from)
                    .map_err(value_error)?,
            )
            .map_err(value_error)?;
        let field = field.into_json().map_err(value_error)?;
        let value =
            pyo3::types::PyBytes::new(py, &Scalar::from_sequence(values).into_value_bytes())
                .into_any()
                .unbind();
        let registry = self.inner.registry().into_json().map_err(value_error)?;
        Ok((callable, (field, value, registry)))
    }

    /// This message's wire digest, as sixteen big-endian bytes.
    ///
    /// Over every entry `into_bytes` emits, pre-order, so two messages that
    /// re-emit alike digest alike whatever separator either was read with.
    /// Computed on every call and stored nowhere.
    fn digest<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.digest().to_be_bytes())
    }

    /// The graph market operations this message expands to: an order, a
    /// quote, an execution or an initial trade report is one; a book `W` or
    /// `X` one per `NoMDEntries(268)` occurrence, or one scoped snapshot
    /// control for an empty `W` - each a `MarketData`.
    fn market_operations(&self) -> PyResult<Vec<PyMarketData>> {
        self.inner
            .market_operations()
            .map(|operations| {
                operations
                    .into_iter()
                    .map(PyMarketData::from_core)
                    .collect()
            })
            .map_err(value_error)
    }

    /// The standard header, typed and held still.
    fn header(&self) -> PyFixHeader {
        PyFixHeader {
            inner: self.inner.header().clone(),
        }
    }

    /// The stable integer business-category code lifted from the message type.
    #[getter]
    fn msgcat(&self) -> Option<i32> {
        self.inner.get_marketoperationid()
    }

    /// What the line said about the capture it was written for, typed and
    /// held still: the plugin a bridge logged it under, the message context
    /// and the session instance.
    ///
    /// Not where the line was read from and not when it was recorded: those
    /// are the reader's statements, held nowhere on a message.
    fn capture(&self) -> PyFixCapture {
        PyFixCapture {
            inner: self.inner.capture().clone(),
        }
    }

    /// `Text(58)`: the free text the message carries, or `None`.
    #[getter]
    fn text(&self) -> Option<&str> {
        self.inner.text()
    }

    /// What a bridge stated under its own namespaces - a `TECH.CLIENTID`,
    /// an `AMON.` key - each under the key as the bridge spelled it, folded,
    /// in sorted order; empty where it stated none.
    #[getter]
    fn metadata(&self) -> BTreeMap<String, String> {
        self.inner
            .metadata()
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    /// The message's `UUIDv7` identity: its millisecond and sequence lead an
    /// XXH3 payload over `currhashcode` and the whole sequence, seeded by
    /// `crosshashcode`.
    #[getter]
    fn curruuid(&self) -> PyScalar {
        uuid_scalar(self.inner.get_curruuid())
    }

    /// The identity every message of one lifecycle shares: derived from
    /// the cross code, and the message's own where it names none.
    #[getter]
    fn crossuuid(&self) -> PyScalar {
        uuid_scalar(self.inner.get_crossuuid())
    }

    /// The cross code: the identifier every message of one lifecycle
    /// shares, as the message spells it - `OrderID`, `ClOrdID`,
    /// `OrigClOrdID`, `QuoteID`, `QuoteReqID` or `MDReqID`, the first
    /// stated - and empty where it names none.
    #[getter]
    fn crosscode(&self) -> &str {
        self.inner.get_crosscode()
    }

    /// The code the message's content digests to: the XXH3-64 of what the
    /// event states, the text, the metadata, `MsgType`, the FIX fields the
    /// message lifted and its named content - everything but the standard
    /// header and trailer, and never the chain it is in.
    #[getter]
    fn currhashcode(&self) -> u64 {
        self.inner.get_currhashcode()
    }

    /// The XXH3-64 of the cross code, zero where the message names none.
    #[getter]
    fn crosshashcode(&self) -> u64 {
        self.inner.get_crosshashcode()
    }

    /// When the message happened: nanoseconds since the Unix epoch, UTC -
    /// the stated instant, else the official transaction clock standing
    /// within the codec's `official_time_delay_ms` of `SendingTime`, else
    /// that `SendingTime`.
    #[getter]
    fn currunix(&self) -> i64 {
        self.inner.get_currunix()
    }

    /// The state the order is in, as the `state` code it is.
    #[getter]
    fn state(&self) -> PyScalar {
        code_scalar(self.inner.get_state())
    }

    /// The message's place in its chain: how many came before it.
    #[getter]
    fn seqnum(&self) -> u64 {
        self.inner.get_seqnum()
    }

    /// The identity of the message this one follows, or `None`.
    #[getter]
    fn prevuuid(&self) -> Option<PyScalar> {
        self.inner.get_prevuuid().map(uuid_scalar)
    }

    /// When the order this message belongs to was created, where known.
    #[getter]
    fn creaunix(&self) -> Option<i64> {
        self.inner.get_creaunix()
    }

    /// The latest execution instant the lifecycle reached, where known.
    #[getter]
    fn execunix(&self) -> Option<i64> {
        self.inner.get_execunix()
    }

    /// When the message was recorded, where stated.
    #[getter]
    fn recdunix(&self) -> Option<i64> {
        self.inner.get_recdunix()
    }

    /// When the order expires, where it has an expiry.
    #[getter]
    fn exprtime(&self) -> Option<i64> {
        self.inner.get_exprtime()
    }

    /// When the message this one follows happened, where it follows one.
    #[getter]
    fn prevunix(&self) -> Option<i64> {
        self.inner.get_prevunix()
    }

    /// The grid step a walk read this message as the snapshot of, where one
    /// did.
    #[getter]
    fn snapunix(&self) -> Option<i64> {
        self.inner.get_snapunix()
    }

    /// The identities of the elements this one was read from: the text line
    /// it was parsed out of, and none for one parsed from bytes. Provenance,
    /// never its chain: no walk moves it.
    #[getter]
    fn srcuuids(&self) -> Vec<PyScalar> {
        self.inner
            .get_srcuuids()
            .iter()
            .copied()
            .map(uuid_scalar)
            .collect()
    }

    /// The capture's own cells the message carries, each under the column
    /// it was read from: where the line was read from, its place in the
    /// object, the body it was cut from, when it was recorded. Provenance
    /// and never content - none is an entry, none reaches the wire or the
    /// hash code - and `into_row` states each again at its column. Empty
    /// for a message parsed from bytes.
    #[getter]
    fn carried(&self) -> Vec<(String, PyScalar)> {
        self.inner
            .carried()
            .iter()
            .map(|(name, value)| (name.to_string(), PyScalar::from_inner(value.clone())))
            .collect()
    }

    /// The price the message states, as a decimal; `None` where it states
    /// none. Never a last executed price, which `lastpx` answers.
    #[getter]
    fn price(&self) -> Option<PyScalar> {
        self.inner.get_price().map(decimal_scalar)
    }

    /// The currency, as the `currency` code it is; `XXX` where none is
    /// stated.
    #[getter]
    fn currency(&self) -> PyScalar {
        code_scalar(self.inner.get_currency())
    }

    /// The quantity the message states, as a decimal; `None` where it
    /// states none. Never a last executed quantity, which `lastqty` answers.
    #[getter]
    fn quantity(&self) -> Option<PyScalar> {
        self.inner.get_quantity().map(decimal_scalar)
    }

    /// The unit the quantity is counted in, as spelled; empty where the
    /// message states none.
    #[getter]
    fn unit(&self) -> &str {
        self.inner.get_unit().as_str()
    }

    /// The side, as the `side` code it is: the one stated, else the lane a
    /// single-sided quote states - `BUY` on the bid, `SELL` on the offer -
    /// else `UNKNOWN`.
    #[getter]
    fn side(&self) -> PyScalar {
        PyScalar::from_inner(Scalar::from(self.inner.get_side()))
    }

    /// The identifiers the instrument is stated under, one code under each
    /// source - `ISIN`, `CUSIP`, `FIGI` - read off `SecurityID(48)` under
    /// `SecurityIDSource(22)` and the `SecurityAltID` group, in source
    /// order; empty where the message states none.
    #[getter]
    fn securityids(&self) -> BTreeMap<String, String> {
        securityids_dict(self.inner.get_securityids())
    }

    /// The instrument's classification, read off `CFICode(461)` and what
    /// the message says about the security; `None` where nothing does.
    #[getter]
    fn cficode(&self) -> Option<PyScalar> {
        self.inner.get_cficode().map(code_scalar)
    }

    /// The market, read off `SecurityExchange(207)`, `ExDestination(100)`
    /// or `LastMkt(30)`, the first that names an ISO 10383 MIC.
    #[getter]
    fn miccode(&self) -> Option<PyScalar> {
        self.inner.get_miccode().map(code_scalar)
    }

    /// The price the message last traded at, as a decimal; `None` where it
    /// states none. `FIX`'s own `LastPx(31)`.
    #[getter]
    fn lastpx(&self) -> Option<PyScalar> {
        self.inner.get_lastpx().map(decimal_scalar)
    }

    /// What the message states that its reading could not take as it
    /// stands, each as `(field, reason)` in arrival order: a value that
    /// would not type, a counter disagreeing with its group, what the last
    /// settle dropped. Never a column.
    #[getter]
    fn anomalies(&self) -> Vec<(String, String)> {
        self.inner
            .anomalies()
            .iter()
            .map(|held| (held.field().to_owned(), held.reason().to_owned()))
            .collect()
    }

    /// The quantity it last traded, `LastQty(32)`; `None` where none.
    #[getter]
    fn lastqty(&self) -> Option<PyScalar> {
        self.inner.get_lastqty().map(decimal_scalar)
    }

    /// FIX's own `LastSpotRate(194)`, the spot rate of the last price, as a decimal; `None` where the message
    /// states none.
    #[getter]
    fn lastspotrate(&self) -> Option<PyScalar> {
        self.inner.lifted().lastspotrate().map(decimal_scalar)
    }

    /// FIX's own `LastForwardPoints(195)`, the forward points of the last price, as a decimal; `None` where the message
    /// states none.
    #[getter]
    fn lastforwardpoints(&self) -> Option<PyScalar> {
        self.inner.lifted().lastforwardpoints().map(decimal_scalar)
    }

    /// FIX's own `BidSpotRate(188)`, the bid lane's spot rate, as a decimal; `None` where the message
    /// states none.
    #[getter]
    fn bidspotrate(&self) -> Option<PyScalar> {
        self.inner.lifted().bidspotrate().map(decimal_scalar)
    }

    /// FIX's own `BidForwardPoints(189)`, the bid lane's forward points, as a decimal; `None` where the message
    /// states none.
    #[getter]
    fn bidforwardpoints(&self) -> Option<PyScalar> {
        self.inner.lifted().bidforwardpoints().map(decimal_scalar)
    }

    /// FIX's own `OfferSpotRate(190)`, the ask lane's spot rate, as a decimal; `None` where the message
    /// states none.
    #[getter]
    fn offerspotrate(&self) -> Option<PyScalar> {
        self.inner.lifted().offerspotrate().map(decimal_scalar)
    }

    /// FIX's own `OfferForwardPoints(191)`, the ask lane's forward points, as a decimal; `None` where the message
    /// states none.
    #[getter]
    fn offerforwardpoints(&self) -> Option<PyScalar> {
        self.inner.lifted().offerforwardpoints().map(decimal_scalar)
    }

    /// The price it averaged, `AvgPx(6)`; `None` where none.
    #[getter]
    fn avgpx(&self) -> Option<PyScalar> {
        self.inner.get_avgpx().map(decimal_scalar)
    }

    /// How much of its quantity is done, `CumQty(14)`; `None` where none.
    #[getter]
    fn cumqty(&self) -> Option<PyScalar> {
        self.inner.get_cumqty().map(decimal_scalar)
    }

    /// How much of it is still open, `LeavesQty(151)`; `None` where none.
    #[getter]
    fn leavesqty(&self) -> Option<PyScalar> {
        self.inner.get_leavesqty().map(decimal_scalar)
    }

    /// The price stated before this message - its own closing price, else
    /// what the statement it follows settled on, which a walk fills.
    #[getter]
    fn prevpx(&self) -> Option<PyScalar> {
        self.inner.get_prevpx().map(decimal_scalar)
    }

    /// The quantity that statement settled on; `None` where none.
    #[getter]
    fn prevqty(&self) -> Option<PyScalar> {
        self.inner.get_prevqty().map(decimal_scalar)
    }

    /// The spot part of an FX forward price; `None` where the message
    /// states none.
    #[getter]
    fn spotrate(&self) -> Option<PyScalar> {
        self.inner.get_spotrate().map(decimal_scalar)
    }

    /// The forward points of an FX forward price; `None` where the message
    /// states none.
    #[getter]
    fn forwardpoints(&self) -> Option<PyScalar> {
        self.inner.get_forwardpoints().map(decimal_scalar)
    }

    /// The ticker the instrument is known by; `None` where it has none and
    /// the security identifiers are what name it.
    #[getter]
    fn ticker(&self) -> Option<&str> {
        self.inner.get_ticker()
    }

    /// The stable integer category of the market operation, or `None`.
    #[getter]
    fn marketoperationid(&self) -> Option<i32> {
        self.inner.get_marketoperationid()
    }

    /// How long the message stands, `TimeInForce(59)`, as the code it
    /// states; `None` where it says nothing. What the code `1` names is
    /// the dictionary's to say.
    #[getter]
    fn tif(&self) -> Option<&str> {
        self.inner.get_tif().map(CoreTimeInForce::as_str)
    }

    /// Whether the instrument could be traded when the message was sent, or
    /// `None` where the market said nothing either way - which is not the
    /// same as `False`.
    #[getter]
    fn tradable(&self) -> Option<bool> {
        self.inner.get_tradable()
    }

    /// The accounts the message names - `ACCOUNT` for `Account(1)`, the
    /// customer-account party - each under the field that stated it, in
    /// key order; empty where it names none.
    #[getter]
    fn accountids(&self) -> BTreeMap<String, String> {
        idmap_dict(self.inner.get_accountids())
    }

    /// The users the message names - `SENDERSUBID`, `ONBEHALFOFSUBID`, the
    /// trader parties - each under the field that stated it, in key order;
    /// empty where it names none.
    #[getter]
    fn userids(&self) -> BTreeMap<String, String> {
        idmap_dict(self.inner.get_userids())
    }

    /// The names the message goes by - `ORDERID`, `CLORDID`, `EXECID`,
    /// `QUOTEID` - each under the field that stated it, in key order;
    /// empty where it names none.
    #[getter]
    fn altids(&self) -> BTreeMap<String, String> {
        idmap_dict(self.inner.get_altids())
    }

    /// The bid lane a quote states, typed; `None` where the message states
    /// no bid.
    #[getter]
    fn bid(&self) -> Option<PyLane> {
        self.inner.get_bid().cloned().map(PyLane::from_core)
    }

    /// The ask lane, shaped as the bid; `None` where the message states no
    /// offer.
    #[getter]
    fn ask(&self) -> Option<PyLane> {
        self.inner.get_ask().cloned().map(PyLane::from_core)
    }

    /// What the message states, as a tree: `(tag, name, value, entries)`.
    ///
    /// One tuple per row child that states a value, in the row's order,
    /// carrying the tag the dictionary resolved - `0` for a key no
    /// dictionary explains - the canonical name, and the value as the wire
    /// spells it. A repeating group is one tuple under its counter with the
    /// count as its value, and each occurrence a tuple under it with no
    /// value and the occurrence's members nested; a component is a tuple
    /// with no value and its members nested. The typed facts are not
    /// entries: the header, the event and the capture are the holders' to
    /// answer, and `into_bytes` puts the wire ones in front of these.
    fn entries<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyAny>>> {
        self.inner
            .entries()
            .iter()
            .map(|entry| entry_tuple(py, entry))
            .collect()
    }

    /// This message as the fixed row a table holds.
    ///
    /// `schema` is the fixed root :func:`fix_schema` builds. Every column is
    /// filled by the tag its field carries - never by its spelling - a typed
    /// fact from its holder and the rest from the row, so a message that
    /// carried nothing at a column answers null there rather than shifting
    /// its neighbours, which is what makes two rows of one capture
    /// comparable at all. The capture's own columns answer null - the one
    /// the crate tags, `sourceurl`, and every column no tag and no counter
    /// names - because a message holds no fact for any of them; the capture
    /// readers state them on the row instead. The `fixentries` serie holds
    /// only residual content, counted by `nofixentries`; projected values
    /// remain in their columns. A value a column will not hold is that
    /// column's null; a column that cannot be null keeps the refusal as a
    /// `ValueError`.
    #[allow(clippy::wrong_self_convention)]
    fn into_row(&self, schema: &Bound<'_, PyAny>) -> PyResult<PyScalar> {
        self.inner
            .into_row(&core_field_from_value(schema)?)
            .map(PyScalar::from_inner)
            .map_err(value_error)
    }

    /// Re-emit this message on the wire, separated by `separator`.
    ///
    /// The standard header from the typed header - `SendingTime` only when
    /// the message stated it - the event's own FIX tags, then the row in
    /// its order, each entry stating a value as one pair and an occurrence
    /// or a component as the pairs under it; a coded fact spells as its
    /// wire code. What is emitted is the message as it now stands, derived
    /// values included. Named for what it answers rather than for consuming
    /// the message.
    #[pyo3(signature = (separator=1))]
    #[allow(clippy::wrong_self_convention)]
    fn into_bytes<'py>(&self, py: Python<'py>, separator: u8) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.into_bytes(separator))
    }

    /// The same as `into_bytes`, as text; a value holding a control
    /// character is a `ValueError`.
    #[pyo3(signature = (separator='\x01'))]
    #[allow(clippy::wrong_self_convention)]
    fn into_text(&self, separator: char) -> PyResult<String> {
        self.inner.into_text(separator).map_err(value_error)
    }

    /// A copy that takes writes again, whatever hashed the original.
    fn __copy__(&self) -> Self {
        Self::from_inner(self.inner.clone())
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

/// One dictionary, reading lines into messages, with the Arrow twins.
///
/// The codec is the whole parse surface: a captured line with a verb in front
/// of it, a bare frame, a numeric frame with a stated separator, a bridge's
/// name/value text, pairs a caller already split,
/// a record a text reader answered. Each redirects to the core method of the
/// same name, so nothing here decides a dialect, a version or a separator -
/// it only carries what Python said across. A stage is a call: the stream
/// methods take any iterable and answer a lazy [`FixMessages`](PyFixMessages),
/// and the Arrow methods take and answer a `pyarrow.RecordBatchReader` over
/// the C stream interface, one batch at a time.
#[pyclass(name = "FixCodec", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PyFixCodec {
    inner: CoreFixCodec,
    registry: Arc<CoreFixRegistry>,
}

impl PyFixCodec {
    /// A `pyarrow` reader over a core reader an Arrow twin answered.
    fn reader_to_pyarrow(
        py: Python<'_>,
        reader: yggdryl::Result<yggdryl::arrow::BatchReader>,
    ) -> PyResult<Bound<'_, PyAny>> {
        batch_reader_to_pyarrow(py, reader.map_err(value_error)?)
    }
}

#[pymethods]
impl PyFixCodec {
    // A codec shares the dictionary it resolves against, which is mutable,
    // so it promises no stable hash of its own.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    #[staticmethod]
    fn infer_msgtype_bytes(py: Python<'_>, body: &Bound<'_, PyAny>) -> PyResult<Option<Py<PyAny>>> {
        with_python_bytes(
            body,
            "a captured line must be bytes, bytearray, or memoryview",
            |body| {
                Ok(CoreFixCodec::infer_msgtype_bytes(body)
                    .map(|value| PyBytes::new(py, value).unbind().into_any()))
            },
        )
    }

    #[staticmethod]
    fn infer_msgtype_text(body: &str) -> Option<String> {
        CoreFixCodec::infer_msgtype_text(body).map(str::to_owned)
    }

    /// Open a codec over one dictionary, or over the process default.
    ///
    /// Every pin is the core's, spelled once here.
    /// `default_sending_time` is the `SendingTime` a genuinely new
    /// message takes when it states no valid one and nothing it was read
    /// with dates it, neither a capture reaching tag 52 nor the `currunix`
    /// of the line it was read out of - a native `Scalar` crosses as itself
    /// and must already be a nanosecond UTC `datetime64`, a `datetime` is
    /// read once into that clock, and any other layout is the core's
    /// `ValueError`; unstated, each undated new
    /// message reads UTC now once, so pinning it is what makes a parse of
    /// undated bytes repeatable. `separator` is the byte a numeric frame
    /// splits on where the line does not say; `payload_column` names the
    /// batch column a line is read from; `capture_names` are what a run's
    /// row-header captures are
    /// called, in the order a line answers them, which is what lets
    /// `parse_text_line` read a capture by position rather than by name;
    /// `null_values` are the spellings that mean nothing was
    /// sent; `direction` is the code of tag 385's set an unmarked line
    /// takes on the batch door, any spelling of one - `"S"`, `"Send"`,
    /// `"R"` - the core's `Send` code when unstated and no pin at all when
    /// empty; `batch_byte_size` and `batch_row_size` are the raw bytes and
    /// the row count one Arrow batch targets, the core's 128 MiB and 32,768
    /// rows when unstated, whichever the batch reaches first;
    /// `threads` is how many threads the line and row doors read on, the
    /// available CPUs when unstated; `threads=1` pulls a stream lazily,
    /// while more read a stream ahead and answer in its order. Arrow parse
    /// doors give whole input batches to at most this many jobs and keep
    /// their batch order; `include_msgtypes` and `exclude_msgtypes`
    /// are the message types a parse keeps and refuses, each read before a
    /// frame is built, spelled
    /// as codes or as names - `"0"`, `"Heartbeat"` - with `"unknown"`
    /// standing for a line stating no type at all. Unstated, the core
    /// refuses `Heartbeat`, `TestRequest` and the untyped line; passing an
    /// empty `exclude_msgtypes` keeps every type. `snapshot_ns` is an
    /// epoch-aligned lifecycle snapshot width in nanoseconds; `None`, zero
    /// and a negative width disable snapshots;
    /// `official_time_delay_ms` is how far from `SendingTime(52)` an
    /// official transaction clock may stand and still date the message, the
    /// core's one second when unstated, and a nonpositive delay admits only
    /// a transaction clock equal to the sending clock.
    #[new]
    #[pyo3(signature = (
        registry=None,
        *,
        default_sending_time=None,
        separator=None,
        payload_column="body",
        capture_names=None,
        null_values=None,
        direction=None,
        batch_byte_size=None,
        batch_row_size=None,
        include_msgtypes=None,
        exclude_msgtypes=None,
        threads=None,
        snapshot_ns=None,
        official_time_delay_ms=None,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        registry: Option<PyRef<'_, PyFixRegistry>>,
        default_sending_time: Option<&Bound<'_, PyAny>>,
        separator: Option<u8>,
        payload_column: &str,
        capture_names: Option<Vec<String>>,
        null_values: Option<Vec<String>>,
        direction: Option<&str>,
        batch_byte_size: Option<u64>,
        batch_row_size: Option<usize>,
        include_msgtypes: Option<Vec<String>>,
        exclude_msgtypes: Option<Vec<String>>,
        threads: Option<usize>,
        snapshot_ns: Option<i64>,
        official_time_delay_ms: Option<i64>,
    ) -> PyResult<Self> {
        let registry = registry_or_global(registry)?;
        let mut inner =
            CoreFixCodec::new(Arc::clone(&registry)).with_payload_column(payload_column);
        if let Some(held) = direction {
            inner = inner.try_with_direction(Some(held)).map_err(value_error)?;
        }
        if let Some(held) = default_sending_time {
            inner = inner
                .try_with_default_sending_time(Some(sending_time_from_py(held)?))
                .map_err(value_error)?;
        }
        if let Some(held) = separator {
            inner = inner.with_separator(held);
        }
        if let Some(held) = capture_names {
            inner = inner.with_capture_names(held);
        }
        if let Some(held) = null_values {
            inner = inner.with_null_values(held);
        }
        if let Some(held) = batch_byte_size {
            inner = inner.with_batch_byte_size(held);
        }
        if let Some(held) = batch_row_size {
            inner = inner.with_batch_row_size(held);
        }
        if let Some(held) = include_msgtypes {
            inner = inner.with_include_msgtypes(held);
        }
        if let Some(held) = exclude_msgtypes {
            inner = inner.with_exclude_msgtypes(held);
        }
        if let Some(held) = threads {
            inner = inner.with_threads(held);
        }
        if let Some(held) = snapshot_ns {
            inner = inner.with_snapshot_ns(held);
        }
        if let Some(held) = official_time_delay_ms {
            inner = inner.with_official_time_delay_ms(held);
        }
        Ok(Self { inner, registry })
    }

    /// The dictionary this codec resolves against, sharing it.
    #[getter]
    fn registry(&self) -> PyFixRegistry {
        PyFixRegistry::from_arc(Arc::clone(&self.registry))
    }

    /// The nanosecond UTC `SendingTime` an undated new message takes - one
    /// neither its row nor its line dates - or `None` where each one reads
    /// UTC now once.
    #[getter]
    fn default_sending_time(&self) -> Option<PyScalar> {
        self.inner
            .default_sending_time()
            .cloned()
            .map(PyScalar::from_inner)
    }

    /// The byte a numeric frame splits on, or `None` where the line decides.
    #[getter]
    fn separator(&self) -> Option<u8> {
        self.inner.separator()
    }

    /// The record column a line is read from.
    #[getter]
    fn payload_column(&self) -> &str {
        self.inner.payload_column()
    }

    /// The spellings that mean nothing was sent.
    #[getter]
    fn null_values(&self) -> Vec<String> {
        self.inner.null_values().to_vec()
    }

    /// The code of tag 385's set an unmarked line takes on the batch door,
    /// or `None` where no pin fills silence.
    #[getter]
    fn direction(&self) -> Option<&str> {
        self.inner.direction()
    }

    /// The raw bytes one Arrow batch targets.
    #[getter]
    fn batch_byte_size(&self) -> u64 {
        self.inner.batch_byte_size()
    }

    /// The rows one Arrow batch targets.
    #[getter]
    fn batch_row_size(&self) -> usize {
        self.inner.batch_row_size()
    }

    /// The threads the line and row doors read on: the available CPUs by
    /// default, and one where a stream is pulled lazily.
    #[getter]
    fn threads(&self) -> usize {
        self.inner.threads()
    }

    /// The epoch-aligned lifecycle snapshot width in nanoseconds, or `None`
    /// where snapshots are disabled.
    #[getter]
    fn snapshot_ns(&self) -> Option<i64> {
        self.inner.snapshot_ns()
    }

    /// How far from `SendingTime(52)` an official transaction clock may
    /// stand and still date the message, in milliseconds.
    #[getter]
    fn official_time_delay_ms(&self) -> i64 {
        self.inner.official_time_delay_ms()
    }

    /// The message types a parse keeps, empty where it keeps every type
    /// the refusals leave.
    #[getter]
    fn include_msgtypes(&self) -> Vec<String> {
        self.inner
            .include_msgtypes()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// The message types a parse refuses before it builds a frame.
    #[getter]
    fn exclude_msgtypes(&self) -> Vec<String> {
        self.inner
            .exclude_msgtypes()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// One captured line, whatever it is wrapped in: its messages.
    fn parse_line(&self, row: &[u8]) -> PyResult<PyFixMessages> {
        self.inner
            .parse_line(row)
            .map(PyFixMessages::over)
            .map_err(value_error)
    }

    /// A stream of captured lines, lazily: each as `parse_line` reads it.
    ///
    /// `lines` is any iterable of bytes-like lines, pulled one line at a time
    /// as the stream is read, so a capture of ten million lines costs one at
    /// a time. A line that is not a row at all raises `ValueError` where it
    /// is met and the stream continues past it; an item that is not bytes
    /// raises `TypeError` and ends it.
    fn parse_lines(&self, lines: &Bound<'_, PyAny>) -> PyResult<PyFixMessages> {
        let pulled = Pulled::new(lines, line_bytes)?;
        let failed = pulled.failed.clone();
        Ok(PyFixMessages::pulling(
            self.inner.parse_lines(pulled),
            failed,
        ))
    }

    /// One numeric frame, read by the pairs it states.
    fn parse_fix_line(&self, body: &[u8]) -> PyResult<PyFixMsg> {
        self.inner
            .parse_fix_line(body)
            .map(PyFixMsg::from_inner)
            .map_err(value_error)
    }

    /// One bridge frame, whose keys are names rather than tags.
    fn parse_ullink_line(&self, body: &[u8]) -> PyResult<PyFixMsg> {
        self.inner
            .parse_ullink_line(body)
            .map(PyFixMsg::from_inner)
            .map_err(value_error)
    }

    /// One FIXML row, whose fields are XML attributes.
    fn parse_fixml_line(&self, body: &[u8]) -> PyResult<PyFixMsg> {
        self.inner
            .parse_fixml_line(body)
            .map(PyFixMsg::from_inner)
            .map_err(value_error)
    }

    /// Pairs a caller already holds, in the order they arrived.
    ///
    /// Taken by value because the borrowed pairs the core reads point into
    /// these strings, so they have to outlive the call rather than the caller.
    #[allow(clippy::needless_pass_by_value)]
    fn parse_pairs(&self, pairs: Vec<(String, String)>) -> PyResult<PyFixMsg> {
        let borrowed: Vec<(&[u8], &[u8])> = pairs
            .iter()
            .map(|(key, value)| (key.as_bytes(), value.as_bytes()))
            .collect();
        self.inner
            .parse_pairs(borrowed)
            .map(PyFixMsg::from_inner)
            .map_err(value_error)
    }

    /// One line a text reader answered: its messages.
    ///
    /// The line's body is the bytes read, and its row-header captures state
    /// the rest - the plugin that logged it, the version, and every field a
    /// capture's name reaches. `capture_names` is what decides which capture
    /// is which, once for the whole run, because a line answers its captures
    /// by position. A `timestamp` capture is context and stamps nothing; the
    /// line's own clock does. Its `currunix` - an `mtime` capture, else its
    /// handle's modification time - is the message's `recdunix`, and the
    /// sending clock of a message stating none: `SendingTime` is the
    /// message's own, else a `SendingTime` capture, else the line's
    /// `currunix`, else the codec's `default_sending_time`, else UTC now,
    /// and the instant `currunix` is read against it - the stated one, else
    /// the official clock standing within `official_time_delay_ms` of it,
    /// else it. A clock the parse supplied is never the message's own:
    /// `header().stated_sendingtime` is false, and neither the wire nor the
    /// row's `sendingtime` column states it.
    ///
    /// A `msgpluginid` capture fills the crate's `msgpluginid` field and selects
    /// nothing: the dictionary is one namespace.
    ///
    /// Nothing else the line holds is communicated: not the object it names,
    /// not its media type, not its place in that object, not the body as a
    /// value - and a capture named for one of the capture's own columns
    /// (`sourceurl`, or a carried one like `mtime`) fills
    /// nothing either. The answer is the message the line's bytes parsed to
    /// and no more; where a line came from is the reader's to state, on the
    /// row, which is what `parse_text_arrow_reader` does.
    ///
    /// A `direction` capture is named so it cannot silently fill a field of
    /// that name, and is not otherwise read: only `parse_text_arrow_reader`
    /// has a column to put a direction in.
    fn parse_text_line(&self, line: &PyTextLine) -> PyResult<PyFixMessages> {
        self.inner
            .parse_text_line(line.as_core())
            .map(PyFixMessages::over)
            .map_err(value_error)
    }

    /// A stream of lines, lazily: each as `parse_text_line` reads it.
    ///
    /// `lines` is any iterable of `TextLine`, pulled one line at a time. A
    /// payload nobody could read is an unknown message rather than the end of
    /// the run; a mandatory clock, layout or content failure raises
    /// `ValueError` where it is met and the stream continues; an item that is
    /// not a `TextLine`, or a failure of the iterable itself, raises as
    /// itself and ends it.
    fn parse_text_lines(&self, lines: &Bound<'_, PyAny>) -> PyResult<PyFixMessages> {
        let pulled = Pulled::new(lines, |held| {
            let held = held.cast::<PyTextLine>().map_err(PyErr::from)?;
            Ok(held.borrow().as_core().clone())
        })?;
        let failed = pulled.failed.clone();
        Ok(PyFixMessages::pulling(
            self.inner.parse_text_lines(pulled),
            failed,
        ))
    }

    /// A stream of Arrow batches of capture rows as batches of FIX rows.
    ///
    /// `source` is a `pyarrow.RecordBatchReader`, a table, a batch, or any
    /// value exporting the Arrow C stream; the answer is a
    /// `pyarrow.RecordBatchReader` pulling one batch at a time. The schema is
    /// decided before the first row: the capture's own columns lead and the
    /// fixed FIX columns follow. Every row is parsed as the line door
    /// parses one - a row's `currunix` cell is its line's clock, so it is
    /// the messages' `recdunix` and the sending clock of one stating none -
    /// and batches close on the bytes each row lands as
    /// against `batch_byte_size`. With more than one `threads`, at most that
    /// many whole input batches are jobs at once and their answers stay in
    /// input-batch order.
    ///
    /// The capture's own columns fill nothing: the carried ones, and the one
    /// the crate tags - a `sourceurl` column - are read off the source row
    /// and written straight into the row this
    /// answers, because where a line was read from is this reader's
    /// statement and never the message's. This is the one door that can
    /// state them, and it is why they survive a parse without a message
    /// holding one.
    fn parse_text_arrow_reader<'py>(
        &self,
        py: Python<'py>,
        source: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let source = batch_reader_from_value(source)?;
        Self::reader_to_pyarrow(py, self.inner.parse_text_arrow_reader(source))
    }

    /// Walks a stream of batches of FIX rows as one lifecycle.
    ///
    /// `lifecycle` over batches: each row is a message through
    /// `FixMsg.from_row`, the messages are walked as `lifecycle` walks
    /// them, and each is written back under the **same** schema, so a
    /// carried column returns to its place and the arrival record is
    /// untouched. Nothing is parsed again, and batches close on the raw
    /// bytes of each message's arrival record against `batch_byte_size`.
    ///
    /// A carried column returns to its place because the message carries
    /// it: each row's own cells are read into the message it made, under
    /// their names, and stated again where that message lands. The pairing
    /// is by message and never by position - a walk answers messages in their
    /// own order, which a capture's lines are routinely not in.
    fn lifecycle_arrow_reader<'py>(
        &self,
        py: Python<'py>,
        source: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let source = batch_reader_from_value(source)?;
        Self::reader_to_pyarrow(py, self.inner.lifecycle_arrow_reader(source))
    }

    /// A stream of batches of FIX rows as the stream of messages it holds.
    ///
    /// Each row is one message through `FixMsg.from_row` under the source's
    /// schema, lazily, one batch held at a time: a batch
    /// `parse_text_arrow_reader` wrote comes back as the messages that made
    /// it without a parse. One half of what the Arrow twins compose;
    /// `arrow_reader` is the other.
    ///
    /// The capture's own columns are carried: each row's own cells are read
    /// into the message it makes, so `arrow_reader(schema, messages(reader))`
    /// states them again exactly as `lifecycle_arrow_reader(reader)` does.
    fn messages(&self, source: &Bound<'_, PyAny>) -> PyResult<PyFixMessages> {
        let source = batch_reader_from_value(source)?;
        Ok(PyFixMessages::over(self.inner.messages(source)))
    }

    /// A stream of messages as a stream of batches of FIX rows under `schema`.
    ///
    /// `schema` is anything `Field` accepts - the fixed root `fix_schema`
    /// builds, or the one read off a batch - and `messages` any iterable of
    /// `FixMsg`, pulled one message at a time as `pyarrow` pulls batches.
    /// Each message fills one row through `FixMsg.into_row`, and batches
    /// close on the bytes each row lands as against `batch_byte_size`. An item that is not a message is the reader's
    /// error, raised by the batch it would have landed in.
    fn arrow_reader<'py>(
        &self,
        py: Python<'py>,
        schema: &Bound<'py, PyAny>,
        messages: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let schema = core_field_from_value(schema)?;
        let pulled = Pulled::new(messages, message_of)?;
        let failed = pulled.failed.clone();
        let messages = pulled.map(Ok).chain(std::iter::from_fn(move || {
            failed.take().map(|error| Err(python_failure(error)))
        }));
        Self::reader_to_pyarrow(py, self.inner.arrow_reader(schema, messages))
    }

    /// Streams sorted FIX messages through native market operations and the
    /// stateful book iterator into a `pyarrow.RecordBatchReader` of lifted
    /// `marketdata` rows, one `book_event` row per book.
    ///
    /// Admits ORDR/QUOT, actual EXEC, BOOK W/X and TRAD AE; other records
    /// are ignored. Source errors and invalid admitted messages still fail,
    /// including unsupported AE corrections, cancellations and status reports.
    ///
    /// `snapshot_millis` enables epoch-aligned book snapshots; `global_`
    /// consolidates symbols into the `GLOBAL` book. Lifecycle enrichment is
    /// explicit: pass `codec.lifecycle(messages)` when it is wanted.
    #[pyo3(signature = (messages, snapshot_millis=0, global_=false))]
    fn book_arrow_reader<'py>(
        &self,
        py: Python<'py>,
        messages: &Bound<'py, PyAny>,
        snapshot_millis: u64,
        global_: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let pulled = Pulled::new(messages, message_of)?;
        let failed = pulled.failed.clone();
        let messages = pulled.map(Ok).chain(std::iter::from_fn(move || {
            failed.take().map(|error| Err(python_failure(error)))
        }));
        Self::reader_to_pyarrow(
            py,
            self.inner
                .book_arrow_reader(messages, snapshot_millis, global_),
        )
    }

    /// A stream of messages as the rows one message field holds them.
    ///
    /// The third verb, and the one a consumer reads by: `parse_*` turns a
    /// capture into messages, filled with what each implies, `lifecycle`
    /// chains them, and this answers them under whatever field a consumer
    /// reads by - a venue's own
    /// message type, `fix_schema` itself, which keeps every column a capture
    /// lands in, or any Struct root a caller built for the table it is
    /// writing.
    ///
    /// `messages` is any iterable of `FixMsg`, pulled one at a time; `field`
    /// is anything `Field` accepts, read once here rather than per message.
    /// Each row is `FixMsg.into_row` under it, so a column the message did
    /// not state is derived where the crate derives it and a value the column
    /// will not hold is that column's null. A column the message does not
    /// carry at all is read off its arrival record first, which is what lets
    /// a narrow row be formatted into a wider field.
    ///
    /// An item that is not a message raises `TypeError` where it is met.
    fn format_messages(
        &self,
        messages: &Bound<'_, PyAny>,
        field: &Bound<'_, PyAny>,
    ) -> PyResult<Vec<PyScalar>> {
        let field = core_field_from_value(field)?;
        let mut rows = Vec::new();
        for message in Pulled::new(messages, message_of)? {
            rows.push(PyScalar::from_inner(
                self.inner
                    .format_messages([Ok(message)], &field)
                    .next()
                    .expect("one message answers one row")
                    .map_err(value_error)?,
            ));
        }
        Ok(rows)
    }

    /// A stream of batches of FIX rows as batches under one message field.
    ///
    /// The Arrow twin of `format_messages`, and the last stage of the
    /// pipeline a capture runs. The schema is answered before a row is read,
    /// from the source's carried columns and `field`, and the capture's own
    /// columns still lead the row - stated from the cells each message
    /// carries. A source carrying no arrival record is a
    /// projection already and is cast batch by batch instead of read back as
    /// messages.
    fn format_arrow_reader<'py>(
        &self,
        py: Python<'py>,
        source: &Bound<'py, PyAny>,
        field: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let field = core_field_from_value(field)?;
        let source = batch_reader_from_value(source)?;
        Self::reader_to_pyarrow(py, self.inner.format_arrow_reader(source, &field))
    }

    /// Chains a stream of messages, lazily: the lifecycle.
    ///
    /// `messages` is any iterable of `FixMsg`, pulled once, in its own
    /// order, and nothing is collected. The one walk states each message
    /// as the one after the live message it follows - the last message of
    /// its chain, under the cross identity its cross code derives, still
    /// alive - so a chained message carries its predecessor's identity and
    /// instant as `prevuuid` and `prevunix`, its place in the chain as
    /// `seqnum`, the lifecycle's creation carried forward as `creaunix`, and
    /// is settled
    /// again around
    /// them; a message that arrives before the live one it would follow is
    /// yielded as it came. A message the walk refuses raises `ValueError`
    /// where it is met and the stream continues; an item that is not a
    /// `FixMsg`, or a failure of the iterable itself, raises as itself and
    /// ends it. The walk reads the structured message first: one whose
    /// sending clock the parse supplied rather than read is dated by the
    /// `TransactTime(60)` it states, so a capture whose frames state no
    /// `SendingTime(52)` still orders, expires and folds by when its
    /// transactions happened.
    fn lifecycle(&self, messages: &Bound<'_, PyAny>) -> PyResult<PyFixMessages> {
        let pulled = Pulled::new(messages, message_of)?;
        let failed = pulled.failed.clone();
        Ok(PyFixMessages::pulling(self.inner.lifecycle(pulled), failed))
    }

    /// Writes a stream of batches of FIX rows back to the wire, answering the
    /// count of lines.
    ///
    /// The encode direction of the same exchange: each row is the message
    /// `messages` reads out of it, written as `into_bytes` with the codec's
    /// `separator` - `SOH` when none is pinned - then a newline, into `sink`,
    /// a binary file-like object with `write`. The wire is rebuilt from the
    /// arrival record, never from the columns, so a batch without the
    /// `fixentries` column is refused before a row is read. One batch is
    /// held at a time.
    fn write_arrow_reader(
        &self,
        source: &Bound<'_, PyAny>,
        sink: &Bound<'_, PyAny>,
    ) -> PyResult<u64> {
        let source = batch_reader_from_value(source)?;
        let mut writer = PythonWriter::new(sink);
        let written = self.inner.write_arrow_reader(source, &mut writer);
        // The sink's own failure is the one to raise: the core reports it
        // as the write it wrapped.
        writer.finish()?;
        written.map_err(value_error)
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

/// Read a default `SendingTime` the way Python states one.
///
/// A `datetime` holds microseconds, so it can never already be the
/// nanosecond UTC clock the codec takes: it crosses the `Scalar` boundary and
/// is cast once through that clock's own datatype, which is where a naive or
/// differently zoned value is refused. Everything else - a native `Scalar`
/// above all - crosses as itself, and the codec's exact-layout check is the
/// one refusal it meets.
fn sending_time_from_py(value: &Bound<'_, PyAny>) -> PyResult<Scalar> {
    let held = from_py(value)?;
    if value.is_instance_of::<PyDateTime>() {
        return CoreDataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)
            .and_then(|clock| clock.scalar(held))
            .map_err(value_error);
    }
    Ok(held)
}

/// The fixed root every message answers as, built from one dictionary.
///
/// The crate's own columns lead - its clocks, then its identities, then the
/// rest - because a table is read by time and joined by identity; then the
/// standard header, the fields a consumer reads, the four groups worth
/// persisting whole, the trailer, `MsgDirection` (385), and the one
/// `fixentries` serie that closes every row with residual content under the
/// `nofixentries` that counts it. Projected content stays in its columns.
/// Columns are spelled by the dictionary's
/// folded canonical names - `msgtype`, never `35` - so a row reads the way a
/// message reads; the tag stays each column's identity, on its `FIX:tag`,
/// and is what fills it. `beginstring`, `currunix`, `creaunix`, `currhashcode`,
/// `crosshashcode`, `curruuid` and `crossuuid` are the non-null columns,
/// because every message settles them; a tag the dictionary does not hold
/// is skipped rather than invented.
#[pyfunction]
#[pyo3(name = "fix_schema", signature = (registry=None, name="fix"))]
pub(crate) fn fix_schema(
    registry: Option<PyRef<'_, PyFixRegistry>>,
    name: &str,
) -> PyResult<PyField> {
    let registry = registry_or_global(registry)?;
    yggdryl::fix_schema(&registry, name.to_owned())
        .map(PyField::from_inner)
        .map_err(value_error)
}

/// The fixed root behind a capture's own columns.
///
/// `carrier` is a capture's own root - where a line was read from, which line
/// it was, what stamped it - and its columns lead the row, because that is what
/// a monitor orders and joins on. A carried column whose folded name a FIX
/// column already takes - `MsgCtxId` and `msgctxid` are one name - is dropped
/// rather than renamed: the FIX column is the one a reader spelling it means,
/// and the row fills it from what the capture stated. No FIX column takes
/// `sourceurl`: where a line was read from is the reader's word about the
/// line, so a capture stating it leads the row with it, beside the body and
/// the row number.
///
/// A carried column is nullable whatever the capture declared it: a capture's
/// own column is the *reading's* statement and no message holds one, so a
/// pass that has no source row in hand writes null there rather than
/// refusing per row. The one-pass readers state every one of them.
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

/// One row's columns, in order, as tags.
#[pyfunction]
#[pyo3(name = "fix_schema_tags")]
pub(crate) fn fix_schema_tags() -> Vec<i32> {
    yggdryl::fix_schema_tags()
}

/// The definitions this crate lists, in tag order from 65003.
///
/// The event's clocks - `currunix`, `creaunix`, `execunix`, `recdunix`,
/// `prevunix`, `snapunix`, `exprtime` - its identities - `currhashcode`,
/// `crosshashcode`, `curruuid`, `crossuuid`, `prevuuid`, the `crosscode` they
/// derive from, its `seqnum` - the `state` it reached - the `srcuuids` of
/// the lines it was read from - what a bridge's own log states about a line
/// - the `msgpluginid`, the `msgctxid`, the `msgsessionid` and the
/// `msgsesseventid` they join to with the message type and sequence - the
/// `sourceurl` a line was read from, the `nofixentries` that counts its
/// content, and the Map group `metadata`, plus the generic
/// `marketoperationid` shared with market operations. Twenty-eight in all,
/// each a fact no FIX dictionary publishes, at the datatype its graph column
/// names.
#[pyfunction]
#[pyo3(name = "fix_crate_fields")]
pub(crate) fn fix_crate_fields() -> PyResult<Vec<PyField>> {
    yggdryl::fix_crate_fields()
        .map(|held| held.iter().cloned().map(PyField::from_inner).collect())
        .map_err(value_error)
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
        let value = self.value.get(self.index)?.into_owned();
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
/// crate's own fields and the two seeded standard clocks. Only the third step
/// treats absence as that
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

/// The standard header of one message, typed and held still.
///
/// What FIX puts in front of every message: the version it says it speaks,
/// the type it is, who sent it to whom, its place in the session and when it
/// was sent - each read off the message's own holder, never looked up in the
/// row, which holds none of them. A copy at the moment it was asked for;
/// immutable, so it compares and hashes by its facts.
#[pyclass(
    name = "FixHeader",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyFixHeader {
    inner: CoreFixHeader,
}

#[pymethods]
impl PyFixHeader {
    /// `BeginString(8)`: the version the message says it speaks, empty
    /// where it stated none.
    #[getter]
    fn beginstring(&self) -> &str {
        self.inner.beginstring()
    }

    /// `MsgType(35)`, empty for a message stating no type.
    #[getter]
    fn msgtype(&self) -> &str {
        self.inner.msgtype()
    }

    /// `SenderCompID(49)`, or `None`.
    #[getter]
    fn sendercompid(&self) -> Option<&str> {
        self.inner.sendercompid()
    }

    /// `TargetCompID(56)`, or `None`.
    #[getter]
    fn targetcompid(&self) -> Option<&str> {
        self.inner.targetcompid()
    }

    /// `MsgSeqNum(34)`, or `None`.
    #[getter]
    fn msgseqnum(&self) -> Option<u64> {
        self.inner.msgseqnum()
    }

    /// `SendingTime(52)` as the intake settled it: nanoseconds since the
    /// Unix epoch, UTC.
    #[getter]
    fn sendingtime(&self) -> i64 {
        self.inner.sendingtime()
    }

    /// Whether the message stated its sending time itself; only a stated
    /// one goes back on the wire.
    #[getter]
    fn stated_sendingtime(&self) -> bool {
        self.inner.stated_sendingtime()
    }

    /// `PossDupFlag(43)`, or `None`.
    #[getter]
    fn possdupflag(&self) -> Option<bool> {
        self.inner.possdupflag()
    }

    /// `MsgDirection(385)`: the code of tag 385's set the line moved in, or
    /// `None`.
    #[getter]
    fn msgdirection(&self) -> Option<&str> {
        self.inner.msgdirection()
    }

    /// `SignatureLength(93)`: how many bytes the signature runs to, or
    /// `None` where the frame carried none.
    #[getter]
    fn signaturelength(&self) -> Option<i32> {
        self.inner.signaturelength()
    }

    /// `Signature(89)`: the bytes the frame was signed with, or `None`.
    #[getter]
    fn signature(&self) -> Option<&[u8]> {
        self.inner.signature()
    }

    /// `CheckSum(10)`: the frame's own checksum as the wire spelled it, or
    /// `None`. Always the last pair of a frame, and the last the wire emits.
    #[getter]
    fn checksum(&self) -> Option<&str> {
        self.inner.checksum()
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

    fn __hash__(&self) -> isize {
        let mut state = std::hash::DefaultHasher::new();
        (
            self.inner.beginstring(),
            self.inner.msgtype(),
            self.inner.sendercompid(),
            self.inner.targetcompid(),
            self.inner.msgseqnum(),
            self.inner.sendingtime(),
            self.inner.stated_sendingtime(),
            self.inner.possdupflag(),
            self.inner.msgdirection(),
        )
            .hash(&mut state);
        crate::python_hash(state.finish())
    }

    fn __repr__(&self) -> String {
        format!(
            "FixHeader({:?}, {:?}, {} -> {})",
            self.inner.beginstring(),
            self.inner.msgtype(),
            repr_text(self.inner.sendercompid()),
            repr_text(self.inner.targetcompid())
        )
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// What the line itself said about the capture it was written for, typed
/// and held still.
///
/// What a bridge's own row header states about the line it wrote - the
/// plugin, the message context and the session instance - read off the
/// line's own bytes like every other fact a message holds. None of it is
/// FIX and none is content, so none of it is an entry or on the wire, and
/// none of it reaches the code the content digests to. Where the message
/// type, the session instance, the message context and `MsgSeqNum` are all
/// stated, their values joined by `:` are `msgsesseventid`, the session
/// event the message was delivered as: derived whenever the message
/// settles, delivery provenance rather than the message's content identity
/// or its chain code, and the key two observations of one delivery merge
/// on.
///
/// What the *reader* said about the line is not here: the object it was
/// read from, and whatever else the reader carried, are the capture's own
/// columns, which the message carries under their names - `carried` - and
/// `into_row` states again.
///
/// A copy at the moment it was asked for; immutable, so it compares and
/// hashes by its facts.
#[pyclass(
    name = "FixCapture",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyFixCapture {
    inner: CoreFixCapture,
}

#[pymethods]
impl PyFixCapture {
    /// The plugin that logged the line, as a bridge names it, or `None`.
    #[getter]
    fn msgpluginid(&self) -> Option<&str> {
        self.inner.msgpluginid()
    }

    /// The message context a bridge handled the line in, or `None`.
    #[getter]
    fn msgctxid(&self) -> Option<&str> {
        self.inner.msgctxid()
    }

    /// The session instance a bridge handled the line on, or `None`.
    #[getter]
    fn msgsessionid(&self) -> Option<&str> {
        self.inner.msgsessionid()
    }

    /// The session event the message was delivered as - `MsgType`,
    /// `msgsessionid`, `msgctxid` and `MsgSeqNum` joined by `:`, as
    /// `8:e7256476:9effef3e6a:1094` - or `None` where one is missing.
    #[getter]
    fn msgsesseventid(&self) -> Option<&str> {
        self.inner.msgsesseventid()
    }

    /// The plugin the message came into a bridge through, as the bridge's
    /// log line names it - `OMS_X1_OrderOut` in `Message received: ... from
    /// (OMS_X1_OrderOut as OD9EOEDJ400)` - or `None`.
    #[getter]
    fn msgoriginator(&self) -> Option<&str> {
        self.inner.msgoriginator()
    }

    /// The conversation a bridge filed the message under - a
    /// `CONVERSATIONID` the message stated, else the `{conversationId: ..}`
    /// of its log line - or `None`.
    #[getter]
    fn conversationid(&self) -> Option<&str> {
        self.inner.conversationid()
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

    fn __hash__(&self) -> isize {
        let mut state = std::hash::DefaultHasher::new();
        (
            self.inner.msgpluginid(),
            self.inner.msgctxid(),
            self.inner.msgsessionid(),
            self.inner.msgsesseventid(),
            self.inner.msgoriginator(),
            self.inner.conversationid(),
        )
            .hash(&mut state);
        crate::python_hash(state.finish())
    }

    fn __repr__(&self) -> String {
        format!(
            "FixCapture({}, {})",
            repr_text(self.inner.msgpluginid()),
            repr_text(self.inner.msgsessionid())
        )
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}
