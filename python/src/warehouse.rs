//! The warehouse: catalogs, namespaces and tables as `IOBase` handles, the
//! collection views one level of the hierarchy answers with, and the
//! registries a dotted path resolves against.
//!
//! An object is a handle. `Catalog`, `Namespace` and `Table` extend `IOBase`
//! over the core's `Holder::Catalog`, `Holder::Namespace` and
//! `Holder::Table`, and the implementation is the subclass below the kind -
//! `MemoryCatalog`, `FolderCatalog`, `MemoryNamespace`, `FolderNamespace`,
//! `MediaTable` - so `type(object)` says which one answers, exactly as it
//! does for every storage role. A view holds its parent object and asks the
//! store when it is asked; nothing here caches an answer.

use std::hash::{Hash, Hasher};

use pyo3::exceptions::{PyKeyError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyString, PyTuple, PyType};

use yggdryl::holder::Holder;
use yggdryl::media::RecordOptions;
use yggdryl::{
    Catalog, CatalogValue, FolderCatalog, FolderLayout, FolderNamespace, IOBase as _, IOKind,
    IOMedia as _, IOMode, IntoObjectPath, MediaTable, MemoryCatalog, MemoryNamespace, Names,
    Namespace, NamespaceValue, Object, ObjectValue, Objects, Properties, SystemWarehouse, Table,
    TableValue, Warehouse,
};

use crate::datatype::core_dtype_from_value;
use crate::field::{PyField, core_field_from_value};
use crate::iobase::{PyIOBase, describe};
use crate::iomedia::{
    batch_reader_from_any, core_record_options_from_value, fold_record_properties,
};
use crate::uri::{core_uri_from_value, core_url_from_value};
use crate::{python_hash, value_error};

/// Which implementation a warehouse object answers through: the class a
/// described handle takes below the class of its kind.
#[derive(Clone, Copy)]
pub(crate) enum Implementation {
    MemoryCatalog,
    FolderCatalog,
    IcebergCatalog,
    /// A catalog this build has no subclass for.
    Catalog,
    MemoryNamespace,
    FolderNamespace,
    IcebergNamespace,
    /// A namespace this build has no subclass for.
    Namespace,
    MediaTable,
    IcebergTable,
    /// A table this build has no subclass for.
    Table,
}

impl Implementation {
    pub(crate) fn of_catalog(catalog: &Catalog) -> Self {
        match catalog {
            Catalog::Memory(_) => Self::MemoryCatalog,
            Catalog::Folder(_) => Self::FolderCatalog,
            Catalog::Iceberg(_) => Self::IcebergCatalog,
            _ => Self::Catalog,
        }
    }

    pub(crate) fn of_namespace(namespace: &Namespace) -> Self {
        match namespace {
            Namespace::Memory(_) => Self::MemoryNamespace,
            Namespace::Folder(_) => Self::FolderNamespace,
            Namespace::Iceberg(_) => Self::IcebergNamespace,
            _ => Self::Namespace,
        }
    }

    pub(crate) fn of_table(table: &Table) -> Self {
        match table {
            Table::Media(_) => Self::MediaTable,
            Table::Iceberg(_) => Self::IcebergTable,
            _ => Self::Table,
        }
    }

    /// The implementation a holder is, when it holds a warehouse object.
    fn of_holder(holder: &Holder) -> Option<Self> {
        match holder {
            Holder::Catalog(catalog) => Some(Self::of_catalog(catalog)),
            Holder::Namespace(namespace) => Some(Self::of_namespace(namespace)),
            Holder::Table(table) => Some(Self::of_table(table)),
            _ => None,
        }
    }

    /// The class name this implementation reports.
    pub(crate) const fn class_name(self) -> &'static str {
        match self {
            Self::MemoryCatalog => "MemoryCatalog",
            Self::FolderCatalog => "FolderCatalog",
            Self::IcebergCatalog => "IcebergCatalog",
            Self::Catalog => "Catalog",
            Self::MemoryNamespace => "MemoryNamespace",
            Self::FolderNamespace => "FolderNamespace",
            Self::IcebergNamespace => "IcebergNamespace",
            Self::Namespace => "Namespace",
            Self::MediaTable => "MediaTable",
            Self::IcebergTable => "IcebergTable",
            Self::Table => "Table",
        }
    }
}

/// Build the class a warehouse object names: the kind, then the
/// implementation below it.
pub(crate) fn describe_object(
    py: Python<'_>,
    base: PyClassInitializer<PyIOBase>,
    implementation: Implementation,
) -> PyResult<Py<PyAny>> {
    Ok(match implementation {
        Implementation::MemoryCatalog => Py::new(
            py,
            base.add_subclass(PyCatalog).add_subclass(PyMemoryCatalog),
        )?
        .into_any(),
        Implementation::FolderCatalog => Py::new(
            py,
            base.add_subclass(PyCatalog).add_subclass(PyFolderCatalog),
        )?
        .into_any(),
        Implementation::IcebergCatalog => Py::new(
            py,
            base.add_subclass(PyCatalog)
                .add_subclass(crate::iceberg::PyIcebergCatalog),
        )?
        .into_any(),
        Implementation::Catalog => Py::new(py, base.add_subclass(PyCatalog))?.into_any(),
        Implementation::MemoryNamespace => Py::new(
            py,
            base.add_subclass(PyNamespace)
                .add_subclass(PyMemoryNamespace),
        )?
        .into_any(),
        Implementation::FolderNamespace => Py::new(
            py,
            base.add_subclass(PyNamespace)
                .add_subclass(PyFolderNamespace),
        )?
        .into_any(),
        Implementation::IcebergNamespace => Py::new(
            py,
            base.add_subclass(PyNamespace)
                .add_subclass(crate::iceberg::PyIcebergNamespace),
        )?
        .into_any(),
        Implementation::Namespace => Py::new(py, base.add_subclass(PyNamespace))?.into_any(),
        Implementation::MediaTable => {
            Py::new(py, base.add_subclass(PyTable).add_subclass(PyMediaTable))?.into_any()
        }
        Implementation::IcebergTable => Py::new(
            py,
            base.add_subclass(PyTable)
                .add_subclass(crate::iceberg::PyIcebergTable),
        )?
        .into_any(),
        Implementation::Table => Py::new(py, base.add_subclass(PyTable))?.into_any(),
    })
}

/// Report a handle that no longer holds a warehouse object.
fn no_object() -> PyErr {
    PyValueError::new_err("this handle no longer holds a warehouse object")
}

/// The object a handle holds, through the contract every one answers.
fn object_of(base: &PyIOBase) -> PyResult<&dyn ObjectValue> {
    match base.inner()? {
        Holder::Catalog(catalog) => Ok(catalog.as_ref()),
        Holder::Namespace(namespace) => Ok(namespace.as_ref()),
        Holder::Table(table) => Ok(table.as_ref()),
        _ => Err(no_object()),
    }
}

fn catalog_of(base: &PyIOBase) -> PyResult<&Catalog> {
    match base.inner()? {
        Holder::Catalog(catalog) => Ok(catalog),
        _ => Err(no_object()),
    }
}

fn namespace_of(base: &PyIOBase) -> PyResult<&Namespace> {
    match base.inner()? {
        Holder::Namespace(namespace) => Ok(namespace),
        _ => Err(no_object()),
    }
}

fn table_of(base: &PyIOBase) -> PyResult<&Table> {
    match base.inner()? {
        Holder::Table(table) => Ok(table),
        _ => Err(no_object()),
    }
}

/// The dotted path an object displays as, quoted as the plan grammar quotes.
fn path_text(base: &PyIOBase) -> PyResult<String> {
    Ok(match base.inner()? {
        Holder::Catalog(catalog) => catalog.to_string(),
        Holder::Namespace(namespace) => namespace.to_string(),
        Holder::Table(table) => table.to_string(),
        _ => return Err(no_object()),
    })
}

/// Answer one object as the class its implementation names.
fn described(py: Python<'_>, object: Object) -> PyResult<Py<PyAny>> {
    describe(py, object.into_holder())
}

/// A property bag as Python states one: a mapping or an iterable of
/// `(name, value)` pairs, then the keywords beside it, a later entry
/// replacing an earlier one by name.
///
/// A value is its text, a `bool` spelled `true` or `false`; `...` is an
/// argument not given, and `None` clears the name - a bag holds a property or
/// it does not, so there is no property with no value.
pub(crate) fn properties_from_args(
    mapping: Option<&Bound<'_, PyAny>>,
    keywords: Option<&Bound<'_, PyDict>>,
) -> PyResult<Properties> {
    let mut properties = Properties::new();
    if let Some(mapping) = mapping
        && !mapping.is_none()
    {
        let items = if mapping.hasattr("items")? {
            mapping.call_method0("items")?
        } else {
            mapping.clone()
        };
        for item in items.try_iter()? {
            let (name, value) = item?.extract::<(String, Bound<'_, PyAny>)>()?;
            set_property(&mut properties, name, &value)?;
        }
    }
    if let Some(keywords) = keywords {
        let ellipsis = keywords.py().Ellipsis();
        for (name, value) in keywords.iter() {
            if value.is(&ellipsis) {
                continue;
            }
            set_property(&mut properties, name.extract::<String>()?, &value)?;
        }
    }
    Ok(properties)
}

fn set_property(
    properties: &mut Properties,
    name: String,
    value: &Bound<'_, PyAny>,
) -> PyResult<()> {
    if value.is_none() {
        properties.remove(&name);
        return Ok(());
    }
    let text = if let Ok(flag) = value.cast::<PyBool>() {
        (if flag.is_true() { "true" } else { "false" }).to_owned()
    } else {
        value.str()?.to_str()?.to_owned()
    };
    properties.set(name, text);
    Ok(())
}

/// A property bag as Python reads one: an ordered `dict`.
fn properties_into_py<'py>(
    py: Python<'py>,
    properties: &Properties,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    for (name, value) in properties {
        dict.set_item(name, value)?;
    }
    Ok(dict)
}

/// A path as Python spells one: dotted text read through the plan's location
/// grammar, or the parts themselves.
pub(crate) fn object_path_from_value(
    value: &Bound<'_, PyAny>,
) -> PyResult<impl IntoObjectPath + use<>> {
    if let Ok(text) = value.cast::<PyString>() {
        return text.to_str()?.into_object_path().map_err(value_error);
    }
    let parts = crate::enums::strings_from_iterable(value, "path")?;
    parts
        .iter()
        .map(String::as_str)
        .collect::<Vec<&str>>()
        .into_object_path()
        .map_err(value_error)
}

/// The object a Python value is: a `Catalog`, a `Namespace` or a `Table`
/// handle, whose description is taken as the core's own value.
fn object_from_value(value: &Bound<'_, PyAny>) -> PyResult<Object> {
    let refused = || {
        value.get_type().name().map(|name| {
            PyTypeError::new_err(format!(
                "expected a Catalog, a Namespace or a Table, got {name}"
            ))
        })
    };
    let Ok(handle) = value.extract::<PyRef<'_, PyIOBase>>() else {
        return Err(refused()?);
    };
    match handle.inner()? {
        Holder::Catalog(catalog) => Ok(Object::Catalog(catalog.as_ref().clone())),
        Holder::Namespace(namespace) => Ok(Object::Namespace(namespace.as_ref().clone())),
        Holder::Table(table) => Ok(Object::Table(table.as_ref().clone())),
        _ => Err(refused()?),
    }
}

/// The objects an iterable names, none for `None`.
fn objects_from_value(value: Option<&Bound<'_, PyAny>>) -> PyResult<Vec<Object>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_none() {
        return Ok(Vec::new());
    }
    value
        .try_iter()?
        .map(|item| object_from_value(&item?))
        .collect()
}

/// Where an object's storage is, as a constructor takes it: a handle binds,
/// a location names.
pub(crate) enum Located {
    Handle(Box<Holder>),
    Url(yggdryl::Url),
}

pub(crate) fn located_from_value(value: &Bound<'_, PyAny>) -> PyResult<Located> {
    if let Ok(handle) = value.extract::<PyRef<'_, PyIOBase>>() {
        let inner = handle.inner()?;
        if inner.kind() != IOKind::Memory {
            return handle.rebuilt().map(Box::new).map(Located::Handle);
        }
        // No location to rebuild from, so the content is what is bound: a
        // copy of the bytes under the same media type, as `IOBase(handle)`
        // takes an in-memory handle.
        let bytes = inner.read_all_bytes().map_err(value_error)?;
        let mut buffer = Holder::Buffer(yggdryl::holder::Buffer::from_bytes(bytes));
        buffer.set_media_type(inner.media_type().clone());
        return Ok(Located::Handle(Box::new(buffer)));
    }
    core_url_from_value(value).map(Located::Url)
}

/// The initializer every catalog class builds on.
pub(crate) fn catalog_base(catalog: Catalog) -> PyClassInitializer<PyCatalog> {
    PyClassInitializer::from(PyIOBase::from_core(Holder::from(catalog))).add_subclass(PyCatalog)
}

pub(crate) fn namespace_base(namespace: Namespace) -> PyClassInitializer<PyNamespace> {
    PyClassInitializer::from(PyIOBase::from_core(Holder::from(namespace))).add_subclass(PyNamespace)
}

pub(crate) fn table_base(table: Table) -> PyClassInitializer<PyTable> {
    PyClassInitializer::from(PyIOBase::from_core(Holder::from(table))).add_subclass(PyTable)
}

/// The absence the mapping protocol spells as a `KeyError`, carrying the
/// core's own message; every other failure is a `ValueError`.
fn lookup_error(error: yggdryl::Error) -> PyErr {
    if error.is_absent() {
        PyKeyError::new_err(error.to_string())
    } else {
        value_error(error)
    }
}

/// The members every warehouse object answers, in one `#[pymethods]` block
/// beside the members of its kind: `$access` borrows the core value the
/// equality and the hash read.
macro_rules! object_members {
    ($type:ident, $access:ident, { $($extra:tt)* }) => {
        #[pymethods]
        impl $type {
            /// The last part of the path.
            #[getter]
            fn name(slf: &Bound<'_, Self>) -> PyResult<String> {
                let base = slf.borrow();
                Ok(object_of(base.as_super())?.name().to_owned())
            }

            /// The parts, from the catalog's name down to this object's own.
            #[getter]
            fn path<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyTuple>> {
                let base = slf.borrow();
                let py = slf.py();
                let object = object_of(base.as_super())?;
                PyTuple::new(py, object.path().iter().map(|part| part.as_str()))
            }

            /// What the store says this object is, when it says anything.
            #[getter]
            fn description(slf: &Bound<'_, Self>) -> PyResult<Option<String>> {
                let base = slf.borrow();
                Ok(object_of(base.as_super())?.description().map(str::to_owned))
            }

            /// When this object last changed, in UTC nanoseconds since the
            /// epoch, when the store keeps that fact.
            #[getter]
            fn modified(slf: &Bound<'_, Self>) -> PyResult<Option<i64>> {
                let base = slf.borrow();
                Ok(object_of(base.as_super())?.modified())
            }

            /// The effective properties: the parent's, then what the store
            /// keeps, then what was stated, a later entry replacing an
            /// earlier one by name.
            #[getter]
            fn properties<'py>(slf: &Bound<'py, Self>) -> PyResult<Bound<'py, PyDict>> {
                let base = slf.borrow();
                let properties = object_of(base.as_super())?
                    .properties()
                    .map_err(value_error)?;
                properties_into_py(slf.py(), &properties)
            }

            /// Persist updates and removals of what the store keeps for this
            /// object.
            ///
            /// `updates` is a mapping or a sequence of `(name, value)` pairs
            /// and `removes` an iterable of names. A call given neither
            /// writes nothing; an object that keeps nothing refuses by name.
            #[pyo3(signature = (updates = None, removes = None))]
            fn update_properties(
                slf: &Bound<'_, Self>,
                updates: Option<&Bound<'_, PyAny>>,
                removes: Option<&Bound<'_, PyAny>>,
            ) -> PyResult<()> {
                let base = slf.borrow();
                let updates = properties_from_args(updates, None)?;
                let removes = match removes {
                    Some(value) => crate::enums::strings_from_iterable(value, "removes")?,
                    None => Vec::new(),
                };
                if updates.is_empty() && removes.is_empty() {
                    return Ok(());
                }
                // The names as the core spells them, which is what the parts
                // intake answers for parts given as they are.
                let removes = removes
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<&str>>()
                    .into_object_path()
                    .map_err(value_error)?;
                object_of(base.as_super())?
                    .update_properties(&updates, &removes)
                    .map_err(value_error)
            }

            fn __str__(slf: &Bound<'_, Self>) -> PyResult<String> {
                let base = slf.borrow();
                path_text(base.as_super())
            }

            fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
                let handle = slf.borrow();
                let base = handle.as_super();
                let implementation = Implementation::of_holder(base.inner()?).ok_or_else(no_object)?;
                let path = PyString::new(slf.py(), &path_text(base)?).repr()?;
                Ok(format!("{}({path})", implementation.class_name()))
            }

            /// Two objects are equal when their descriptions are: the path,
            /// the location, what was stated.
            fn __eq__(slf: &Bound<'_, Self>, other: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
                let base = slf.borrow();
                let py = slf.py();
                let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
                    return Ok(py.NotImplemented());
                };
                let equal = $access(base.as_super())? == $access(other.as_super())?;
                Ok(PyBool::new(py, equal).to_owned().into_any().unbind())
            }

            fn __hash__(slf: &Bound<'_, Self>) -> PyResult<isize> {
                let base = slf.borrow();
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                $access(base.as_super())?.hash(&mut hasher);
                Ok(python_hash(hasher.finish()))
            }

            $($extra)*
        }
    };
}

/// A catalog: the first namespace layer, what a warehouse registers by name.
///
/// Never built as this class directly: a described handle is the subclass
/// its implementation names - `MemoryCatalog`, `FolderCatalog` - and
/// [`from_url`](Self::from_url) answers one of those too.
#[pyclass(
    name = "Catalog",
    module = "yggdryl._native",
    extends = PyIOBase,
    subclass,
    skip_from_py_object
)]
pub(crate) struct PyCatalog;

object_members!(PyCatalog, catalog_of, {
    /// The catalog a location names, under `properties`, touching no storage.
    ///
    /// `url` is a URL, a path, or an identifier that locates one - a table
    /// bucket's ARN is kept beside the `s3tables://<name>` it locates, so its
    /// region and account are never asked for. The `type` property decides
    /// first - `memory` or `folder` - and otherwise every location a byte
    /// backend holds is a folder catalog over the container it names, called
    /// what the `name` property says, else the location's last segment.
    #[classmethod]
    #[pyo3(signature = (url, properties = None, **keywords))]
    fn from_url(
        _cls: &Bound<'_, PyType>,
        py: Python<'_>,
        url: &Bound<'_, PyAny>,
        properties: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        // The identifier as named, located by the core: an ARN states more
        // than the location it locates.
        let location = core_uri_from_value(url)?;
        let properties = properties_from_args(properties, keywords)?;
        let catalog = Catalog::from_url(&location, &properties).map_err(value_error)?;
        describe(py, Holder::from(catalog))
    }

    /// How many namespace levels may sit under the catalog: `1` for a folder
    /// catalog read as `catalog.schema.table`, `None` where namespaces nest
    /// to any depth.
    #[getter]
    fn namespace_levels(slf: &Bound<'_, Self>) -> PyResult<Option<usize>> {
        let base = slf.borrow();
        Ok(catalog_of(base.as_super())?.namespace_levels())
    }

    /// The namespaces one level down, as a lazy mapping view.
    #[getter]
    fn namespaces(slf: &Bound<'_, Self>) -> PyResult<PyNamespaces> {
        let base = slf.borrow();
        Ok(PyNamespaces {
            parent: Parent::Catalog(catalog_of(base.as_super())?.clone()),
        })
    }

    /// The tables one level down, as a lazy mapping view.
    #[getter]
    fn tables(slf: &Bound<'_, Self>) -> PyResult<PyTables> {
        let base = slf.borrow();
        Ok(PyTables {
            parent: Parent::Catalog(catalog_of(base.as_super())?.clone()),
        })
    }

    /// The children, lazily, in the store's order, each as the class its
    /// implementation names.
    fn children(slf: &Bound<'_, Self>) -> PyResult<PyObjects> {
        let base = slf.borrow();
        Ok(PyObjects::new(
            catalog_of(base.as_super())?.children(),
            None,
            false,
        ))
    }

    /// The child called `name`, one level down.
    fn get(slf: &Bound<'_, Self>, name: &str) -> PyResult<Py<PyAny>> {
        let base = slf.borrow();
        let object = catalog_of(base.as_super())?
            .get(name)
            .map_err(value_error)?;
        described(slf.py(), object)
    }

    /// The object a path below the catalog names - dotted text or parts -
    /// descending one level per part.
    fn resolve(slf: &Bound<'_, Self>, path: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let base = slf.borrow();
        let path = object_path_from_value(path)?;
        let object = catalog_of(base.as_super())?
            .resolve(path)
            .map_err(value_error)?;
        described(slf.py(), object)
    }

    /// The table a path below the catalog names; a namespace there is the
    /// absence of a table.
    fn table(slf: &Bound<'_, Self>, path: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let base = slf.borrow();
        let path = object_path_from_value(path)?;
        let table = catalog_of(base.as_super())?
            .table(path)
            .map_err(value_error)?;
        describe(slf.py(), Holder::from(table))
    }

    /// The namespace a path below the catalog names; a table there is the
    /// absence of a namespace.
    fn namespace(slf: &Bound<'_, Self>, path: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let base = slf.borrow();
        let path = object_path_from_value(path)?;
        let namespace = catalog_of(base.as_super())?
            .namespace(path)
            .map_err(value_error)?;
        describe(slf.py(), Holder::from(namespace))
    }

    /// Create the namespace `name` under the catalog, with `properties`.
    #[pyo3(signature = (name, properties = None, **keywords))]
    fn create_namespace(
        slf: &Bound<'_, Self>,
        name: &str,
        properties: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let base = slf.borrow();
        let properties = properties_from_args(properties, keywords)?;
        let namespace = catalog_of(base.as_super())?
            .create_namespace(name, &properties)
            .map_err(value_error)?;
        describe(slf.py(), Holder::from(namespace))
    }

    /// Create the table `name` under the catalog, `field` its row schema.
    #[pyo3(signature = (name, field, properties = None, **keywords))]
    fn create_table(
        slf: &Bound<'_, Self>,
        name: &str,
        field: &Bound<'_, PyAny>,
        properties: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let base = slf.borrow();
        let field = core_field_from_value(field)?;
        let properties = properties_from_args(properties, keywords)?;
        let table = catalog_of(base.as_super())?
            .create_table(name, &field, &properties)
            .map_err(value_error)?;
        describe(slf.py(), Holder::from(table))
    }
});

/// A catalog of registered objects, in order, with no storage behind it.
#[pyclass(
    name = "MemoryCatalog",
    module = "yggdryl._native",
    extends = PyCatalog,
    skip_from_py_object
)]
pub(crate) struct PyMemoryCatalog;

#[pymethods]
impl PyMemoryCatalog {
    /// An empty catalog called `name`, holding `objects` - each registered
    /// one level below it - and stating `properties`, which every object
    /// under it inherits.
    #[new]
    #[pyo3(signature = (name, *, description = None, objects = None, properties = None, **keywords))]
    fn new(
        name: &str,
        description: Option<&str>,
        objects: Option<&Bound<'_, PyAny>>,
        properties: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyClassInitializer<Self>> {
        let mut catalog =
            MemoryCatalog::new(name).with_properties(properties_from_args(properties, keywords)?);
        if let Some(description) = description {
            catalog = catalog.with_description(description);
        }
        for object in objects_from_value(objects)? {
            catalog = catalog.with_object(object).map_err(value_error)?;
        }
        Ok(catalog_base(Catalog::from(catalog)).add_subclass(Self))
    }
}

/// A container read as a catalog of namespaces and tables.
#[pyclass(
    name = "FolderCatalog",
    module = "yggdryl._native",
    extends = PyCatalog,
    skip_from_py_object
)]
pub(crate) struct PyFolderCatalog;

#[pymethods]
impl PyFolderCatalog {
    /// The catalog `name` over `location`, touching nothing: an `IOBase`
    /// handle binds the container it holds, a `Url`, a string or a path-like
    /// names one. `levels` namespace levels sit under it - one by default, so
    /// a path reads `catalog.schema.table` - and `properties` is what its
    /// container and every object under it open with.
    #[new]
    #[pyo3(signature = (name, location, *, description = None, levels = FolderCatalog::DEFAULT_LEVELS, properties = None, **keywords))]
    fn new(
        name: &str,
        location: &Bound<'_, PyAny>,
        description: Option<&str>,
        levels: usize,
        properties: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyClassInitializer<Self>> {
        let mut catalog = match located_from_value(location)? {
            Located::Handle(holder) => FolderCatalog::bound(name, *holder),
            Located::Url(url) => FolderCatalog::new(name, url).map_err(value_error)?,
        }
        .with_levels(levels)
        .with_properties(properties_from_args(properties, keywords)?);
        if let Some(description) = description {
            catalog = catalog.with_description(description);
        }
        Ok(catalog_base(Catalog::from(catalog)).add_subclass(Self))
    }
}

/// A namespace: a container of namespaces and tables under a catalog.
///
/// Never built as this class directly: a described handle is the subclass
/// its implementation names - `MemoryNamespace`, `FolderNamespace`.
#[pyclass(
    name = "Namespace",
    module = "yggdryl._native",
    extends = PyIOBase,
    subclass,
    skip_from_py_object
)]
pub(crate) struct PyNamespace;

object_members!(PyNamespace, namespace_of, {
    /// The namespaces one level down, as a lazy mapping view.
    #[getter]
    fn namespaces(slf: &Bound<'_, Self>) -> PyResult<PyNamespaces> {
        let base = slf.borrow();
        Ok(PyNamespaces {
            parent: Parent::Namespace(namespace_of(base.as_super())?.clone()),
        })
    }

    /// The tables one level down, as a lazy mapping view.
    #[getter]
    fn tables(slf: &Bound<'_, Self>) -> PyResult<PyTables> {
        let base = slf.borrow();
        Ok(PyTables {
            parent: Parent::Namespace(namespace_of(base.as_super())?.clone()),
        })
    }

    /// The children, lazily, in the store's order, each as the class its
    /// implementation names.
    fn children(slf: &Bound<'_, Self>) -> PyResult<PyObjects> {
        let base = slf.borrow();
        Ok(PyObjects::new(
            namespace_of(base.as_super())?.children(),
            None,
            false,
        ))
    }

    /// The child called `name`, one level down.
    fn get(slf: &Bound<'_, Self>, name: &str) -> PyResult<Py<PyAny>> {
        let base = slf.borrow();
        let object = namespace_of(base.as_super())?
            .get(name)
            .map_err(value_error)?;
        described(slf.py(), object)
    }

    /// The object a path below the namespace names - dotted text or parts -
    /// descending one level per part.
    fn resolve(slf: &Bound<'_, Self>, path: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let base = slf.borrow();
        let path = object_path_from_value(path)?;
        let object = namespace_of(base.as_super())?
            .resolve(path)
            .map_err(value_error)?;
        described(slf.py(), object)
    }

    /// Create the namespace `name` under this one, with `properties`.
    #[pyo3(signature = (name, properties = None, **keywords))]
    fn create_namespace(
        slf: &Bound<'_, Self>,
        name: &str,
        properties: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let base = slf.borrow();
        let properties = properties_from_args(properties, keywords)?;
        let namespace = namespace_of(base.as_super())?
            .create_namespace(name, &properties)
            .map_err(value_error)?;
        describe(slf.py(), Holder::from(namespace))
    }

    /// Create the table `name` under this namespace, `field` its row schema.
    #[pyo3(signature = (name, field, properties = None, **keywords))]
    fn create_table(
        slf: &Bound<'_, Self>,
        name: &str,
        field: &Bound<'_, PyAny>,
        properties: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let base = slf.borrow();
        let field = core_field_from_value(field)?;
        let properties = properties_from_args(properties, keywords)?;
        let table = namespace_of(base.as_super())?
            .create_table(name, &field, &properties)
            .map_err(value_error)?;
        describe(slf.py(), Holder::from(table))
    }
});

/// A namespace of registered objects, in order, with no storage behind it.
#[pyclass(
    name = "MemoryNamespace",
    module = "yggdryl._native",
    extends = PyNamespace,
    skip_from_py_object
)]
pub(crate) struct PyMemoryNamespace;

#[pymethods]
impl PyMemoryNamespace {
    /// An empty namespace at `path` - dotted text or parts, a catalog's name
    /// first - holding `objects`, each registered one level below it, and
    /// stating `properties`.
    #[new]
    #[pyo3(signature = (path, *, description = None, objects = None, properties = None, **keywords))]
    fn new(
        path: &Bound<'_, PyAny>,
        description: Option<&str>,
        objects: Option<&Bound<'_, PyAny>>,
        properties: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyClassInitializer<Self>> {
        let path = object_path_from_value(path)?;
        let mut namespace = MemoryNamespace::new(path)
            .map_err(value_error)?
            .with_properties(properties_from_args(properties, keywords)?);
        if let Some(description) = description {
            namespace = namespace.with_description(description);
        }
        for object in objects_from_value(objects)? {
            namespace = namespace.with_object(object).map_err(value_error)?;
        }
        Ok(namespace_base(Namespace::from(namespace)).add_subclass(Self))
    }
}

/// A folder under a folder catalog, read as a namespace of tables and, while
/// levels remain under it, of namespaces.
#[pyclass(
    name = "FolderNamespace",
    module = "yggdryl._native",
    extends = PyNamespace,
    skip_from_py_object
)]
pub(crate) struct PyFolderNamespace;

#[pymethods]
impl PyFolderNamespace {
    /// The namespace at `path` over `location`, touching nothing: an
    /// `IOBase` handle binds the container it holds, a `Url`, a string or a
    /// path-like names one. `levels` namespace levels sit under it, none by
    /// default.
    #[new]
    #[pyo3(signature = (path, location, *, description = None, levels = 0, properties = None, **keywords))]
    fn new(
        path: &Bound<'_, PyAny>,
        location: &Bound<'_, PyAny>,
        description: Option<&str>,
        levels: usize,
        properties: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyClassInitializer<Self>> {
        let path = object_path_from_value(path)?;
        let mut namespace = match located_from_value(location)? {
            Located::Handle(holder) => FolderNamespace::bound(path, *holder),
            Located::Url(url) => FolderNamespace::new(path, url),
        }
        .map_err(value_error)?
        .with_levels(levels)
        .with_properties(properties_from_args(properties, keywords)?);
        if let Some(description) = description {
            namespace = namespace.with_description(description);
        }
        Ok(namespace_base(Namespace::from(namespace)).add_subclass(Self))
    }
}

/// A table: an object whose rows every record read and write of `IOBase`
/// reaches.
///
/// Never built as this class directly: a described handle is the subclass
/// its implementation names - `MediaTable`.
#[pyclass(
    name = "Table",
    module = "yggdryl._native",
    extends = PyIOBase,
    subclass,
    skip_from_py_object
)]
pub(crate) struct PyTable;

object_members!(PyTable, table_of, {
    /// The row schema, with no row read: the declared field, else the
    /// stored one.
    fn field(slf: &Bound<'_, Self>) -> PyResult<PyField> {
        let base = slf.borrow();
        table_of(base.as_super())?
            .field()
            .map(PyField::from_inner)
            .map_err(value_error)
    }

    /// What holds the rows, as a listing describes it: a leaf's media type,
    /// `directory` for a folder read as the rows beneath it, `table` for a
    /// table format.
    #[getter]
    fn storage(slf: &Bound<'_, Self>) -> PyResult<String> {
        let base = slf.borrow();
        Ok(table_of(base.as_super())?.storage())
    }
});

/// A table over any location a record medium reads: a leaf, a folder read
/// as the rows beneath it, or a folder laid out as a table format.
#[pyclass(
    name = "MediaTable",
    module = "yggdryl._native",
    extends = PyTable,
    skip_from_py_object
)]
pub(crate) struct PyMediaTable;

#[pymethods]
impl PyMediaTable {
    /// The table at `path` - dotted text or parts - over `location`, touching
    /// nothing: an `IOBase` handle binds the storage it holds, a `Url`, a
    /// string or a path-like names it. `field` declares the row schema,
    /// renamed after the table; `dtype` its datatype under a required field,
    /// or under the declared field's own nullability and metadata. `layout`
    /// is `leaf`, `folder` or `format`; unstated, a bound container is a
    /// folder and anything else a leaf.
    #[new]
    #[pyo3(signature = (path, location, *, field = None, dtype = None, description = None, layout = None, properties = None, **keywords))]
    #[allow(clippy::too_many_arguments)] // Every fact a table states, each optional.
    fn new(
        path: &Bound<'_, PyAny>,
        location: &Bound<'_, PyAny>,
        field: Option<&Bound<'_, PyAny>>,
        dtype: Option<&Bound<'_, PyAny>>,
        description: Option<&str>,
        layout: Option<&str>,
        properties: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyClassInitializer<Self>> {
        let path = object_path_from_value(path)?;
        let mut table = match located_from_value(location)? {
            Located::Handle(holder) => MediaTable::bound(path, *holder),
            Located::Url(url) => MediaTable::new(path, url),
        }
        .map_err(value_error)?
        .with_properties(properties_from_args(properties, keywords)?);
        if let Some(field) = field {
            table = table.with_field(core_field_from_value(field)?);
        }
        if let Some(dtype) = dtype {
            table = table
                .with_dtype(core_dtype_from_value(dtype)?)
                .map_err(value_error)?;
        }
        if let Some(description) = description {
            table = table.with_description(description);
        }
        if let Some(layout) = layout {
            table = table.with_layout(layout.parse::<FolderLayout>().map_err(value_error)?);
        }
        Ok(table_base(Table::from(table)).add_subclass(Self))
    }
}

/// The object a collection view lists one level of: a catalog or a
/// namespace, held as its own description.
#[derive(Clone)]
enum Parent {
    Catalog(Catalog),
    Namespace(Namespace),
}

impl Parent {
    fn namespaces(&self) -> yggdryl::Namespaces<'_> {
        match self {
            Self::Catalog(catalog) => catalog.namespaces(),
            Self::Namespace(namespace) => namespace.namespaces(),
        }
    }

    fn tables(&self) -> yggdryl::Tables<'_> {
        match self {
            Self::Catalog(catalog) => catalog.tables(),
            Self::Namespace(namespace) => namespace.tables(),
        }
    }

    fn children(&self) -> Objects {
        match self {
            Self::Catalog(catalog) => catalog.children(),
            Self::Namespace(namespace) => namespace.children(),
        }
    }

    fn path_text(&self) -> String {
        match self {
            Self::Catalog(catalog) => catalog.to_string(),
            Self::Namespace(namespace) => namespace.to_string(),
        }
    }
}

/// The namespaces one level below a catalog or a namespace, as a lazy view.
///
/// Constructing the view touches nothing: membership, iteration and length
/// ask the store when they are asked, indexing answers the namespace as the
/// class its implementation names, and a missing name is a `KeyError`
/// carrying the core's own message. Names may be dotted - `view["sales.eu"]`
/// descends.
#[pyclass(name = "Namespaces", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PyNamespaces {
    parent: Parent,
}

#[pymethods]
impl PyNamespaces {
    // Collection answers depend on storage at the moment they are requested.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// `namespaces["sales"]` opens the namespace; a missing one is a
    /// `KeyError` carrying the core's message.
    fn __getitem__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let namespace = self.parent.namespaces().get(name).map_err(lookup_error)?;
        describe(py, Holder::from(namespace))
    }

    /// `"sales" in namespaces` asks the store whether the namespace exists.
    fn __contains__(&self, name: &str) -> PyResult<bool> {
        self.parent.namespaces().contains(name).map_err(value_error)
    }

    /// Iterating the view yields the namespace names, lazily, in the
    /// store's order.
    fn __iter__(&self) -> PyNames {
        PyNames {
            names: self.parent.namespaces().iter(),
        }
    }

    /// How many namespaces are one level down, which drains the listing.
    fn __len__(&self) -> PyResult<usize> {
        self.parent.namespaces().len().map_err(value_error)
    }

    /// The names, as `dict.keys` answers them - the same lazy iterator.
    fn keys(&self) -> PyNames {
        self.__iter__()
    }

    /// The namespaces themselves, lazily, each described as `__next__`
    /// reaches it.
    fn values(&self) -> PyObjects {
        PyObjects::new(self.parent.children(), Some(IOKind::Namespace), false)
    }

    /// `(name, namespace)` pairs, as lazily as `values`.
    fn items(&self) -> PyObjects {
        PyObjects::new(self.parent.children(), Some(IOKind::Namespace), true)
    }

    /// The named namespace, or `default` when nothing - or a table - is
    /// there.
    #[pyo3(signature = (name, default = None))]
    fn get(&self, py: Python<'_>, name: &str, default: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        match self.parent.namespaces().get(name) {
            Ok(namespace) => describe(py, Holder::from(namespace)),
            Err(error) if error.is_absent() => Ok(default.unwrap_or_else(|| py.None())),
            Err(error) => Err(value_error(error)),
        }
    }

    /// Create the named namespace with `properties`; one already there is a
    /// conflict, and a parent that creates nothing refuses by name.
    #[pyo3(signature = (name, properties = None, **keywords))]
    fn create(
        &self,
        py: Python<'_>,
        name: &str,
        properties: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let properties = properties_from_args(properties, keywords)?;
        let namespace = self
            .parent
            .namespaces()
            .create(name, &properties)
            .map_err(value_error)?;
        describe(py, Holder::from(namespace))
    }

    /// Open the named namespace, creating it with `properties` when absent.
    #[pyo3(signature = (name, properties = None, **keywords))]
    fn open_or_create(
        &self,
        py: Python<'_>,
        name: &str,
        properties: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let properties = properties_from_args(properties, keywords)?;
        let namespace = self
            .parent
            .namespaces()
            .open_or_create(name, &properties)
            .map_err(value_error)?;
        describe(py, Holder::from(namespace))
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "Namespaces({})",
            PyString::new(py, &self.parent.path_text()).repr()?
        ))
    }
}

/// The tables one level below a catalog or a namespace, as a lazy view.
///
/// The same shape as [`Namespaces`](PyNamespaces): indexing opens the table
/// as the class its implementation names, a missing name is a `KeyError`,
/// names may be dotted, and the write helpers create the table on first
/// write from the rows' own schema where the parent creates tables.
#[pyclass(name = "Tables", module = "yggdryl._native", skip_from_py_object)]
pub(crate) struct PyTables {
    parent: Parent,
}

impl PyTables {
    /// Write `data` into the named table under `mode`, creating the table
    /// from the rows' own schema when absent, and answer the table.
    ///
    /// The rows are read under the options given - `options`, then
    /// `properties` set on a copy of them - else under the table's own,
    /// else, for a table not there yet, under the schema-free Arrow
    /// defaults; the write runs under the options given, else the table's
    /// own, which carry its declared field and its name.
    fn write(
        &self,
        py: Python<'_>,
        name: &str,
        data: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
        properties: Option<&Bound<'_, PyDict>>,
        mode: IOMode,
    ) -> PyResult<Py<PyAny>> {
        let tables = self.parent.tables();
        let existing = match tables.get(name) {
            Ok(table) => Some(table),
            Err(error) if error.is_absent() => None,
            Err(error) => return Err(value_error(error)),
        };
        let stated = options.is_some() || crate::properties::given(properties);
        let intake = match options {
            Some(options) => core_record_options_from_value(options)?,
            None => match &existing {
                Some(table) => table.record_options().map_err(value_error)?,
                None => RecordOptions::for_mime_type(&yggdryl::MimeType::ARROW_STREAM)
                    .map_err(value_error)?,
            },
        };
        let intake = match properties {
            Some(properties) if !properties.is_empty() => {
                fold_record_properties(intake, properties)?
            }
            _ => intake,
        };
        let batches = batch_reader_from_any(data, &intake)?;
        let mut table = match existing {
            Some(table) => table,
            None => tables
                .open_or_create_from_arrow_reader(name, &batches, &Properties::new())
                .map_err(value_error)?,
        };
        let options = if stated {
            intake
        } else {
            table.record_options().map_err(value_error)?
        };
        py.detach(|| table.write_arrow_reader(batches, mode, &options))
            .map_err(value_error)?;
        describe(py, Holder::from(table))
    }
}

#[pymethods]
impl PyTables {
    // Collection answers depend on storage at the moment they are requested.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// `tables["orders"]` opens the table; a missing one is a `KeyError`
    /// carrying the core's message.
    fn __getitem__(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let table = self.parent.tables().get(name).map_err(lookup_error)?;
        describe(py, Holder::from(table))
    }

    /// `"orders" in tables` asks the store whether the table exists.
    fn __contains__(&self, name: &str) -> PyResult<bool> {
        self.parent.tables().contains(name).map_err(value_error)
    }

    /// Iterating the view yields the table names, lazily, in the store's
    /// order.
    fn __iter__(&self) -> PyNames {
        PyNames {
            names: self.parent.tables().iter(),
        }
    }

    /// How many tables are one level down, which drains the listing.
    fn __len__(&self) -> PyResult<usize> {
        self.parent.tables().len().map_err(value_error)
    }

    /// The names, as `dict.keys` answers them - the same lazy iterator.
    fn keys(&self) -> PyNames {
        self.__iter__()
    }

    /// The tables themselves, lazily, each described as `__next__` reaches
    /// it.
    fn values(&self) -> PyObjects {
        PyObjects::new(self.parent.children(), Some(IOKind::Table), false)
    }

    /// `(name, table)` pairs, as lazily as `values`.
    fn items(&self) -> PyObjects {
        PyObjects::new(self.parent.children(), Some(IOKind::Table), true)
    }

    /// The named table, or `default` when nothing - or a namespace - is
    /// there.
    #[pyo3(signature = (name, default = None))]
    fn get(&self, py: Python<'_>, name: &str, default: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        match self.parent.tables().get(name) {
            Ok(table) => describe(py, Holder::from(table)),
            Err(error) if error.is_absent() => Ok(default.unwrap_or_else(|| py.None())),
            Err(error) => Err(value_error(error)),
        }
    }

    /// Create the named table with `field` as its row schema and
    /// `properties`; one already there is a conflict, and a parent that
    /// creates nothing refuses by name.
    #[pyo3(signature = (name, field, properties = None, **keywords))]
    fn create(
        &self,
        py: Python<'_>,
        name: &str,
        field: &Bound<'_, PyAny>,
        properties: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let field = core_field_from_value(field)?;
        let properties = properties_from_args(properties, keywords)?;
        let table = self
            .parent
            .tables()
            .create(name, &field, &properties)
            .map_err(value_error)?;
        describe(py, Holder::from(table))
    }

    /// Open the named table, creating it with `field` and `properties` when
    /// absent; an existing table is opened as it is.
    #[pyo3(signature = (name, field, properties = None, **keywords))]
    fn open_or_create(
        &self,
        py: Python<'_>,
        name: &str,
        field: &Bound<'_, PyAny>,
        properties: Option<&Bound<'_, PyAny>>,
        keywords: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let field = core_field_from_value(field)?;
        let properties = properties_from_args(properties, keywords)?;
        let table = self
            .parent
            .tables()
            .open_or_create(name, &field, &properties)
            .map_err(value_error)?;
        describe(py, Holder::from(table))
    }

    /// Append `data` - anything the record surface takes - to the named
    /// table, creating it on first write from the rows' own schema, and
    /// answer the table. `options` and the properties beside it are the
    /// record settings this write runs under.
    #[pyo3(signature = (name, data, *, options = None, **properties))]
    fn append(
        &self,
        py: Python<'_>,
        name: &str,
        data: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        self.write(py, name, data, options, properties, IOMode::Append)
    }

    /// Replace the named table's rows with `data`, creating it on first
    /// write from the rows' own schema, and answer the table.
    #[pyo3(signature = (name, data, *, options = None, **properties))]
    fn overwrite(
        &self,
        py: Python<'_>,
        name: &str,
        data: &Bound<'_, PyAny>,
        options: Option<&Bound<'_, PyAny>>,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        self.write(py, name, data, options, properties, IOMode::Overwrite)
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        Ok(format!(
            "Tables({})",
            PyString::new(py, &self.parent.path_text()).repr()?
        ))
    }
}

/// The lazy names iterator every collection view walks.
///
/// It wraps the core's own, so nothing is collected on the way across the
/// boundary and a failure raises at the entry it happened on, after which the
/// iterator is spent.
#[pyclass(name = "WarehouseNames", module = "yggdryl._native")]
pub(crate) struct PyNames {
    names: Names,
}

#[pymethods]
impl PyNames {
    // Consumption changes iterator state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> PyResult<Option<String>> {
        self.names
            .next()
            .transpose()
            .map(|name| name.map(|name| name.to_string()))
            .map_err(value_error)
    }
}

/// The lazy objects iterator behind `children()` and the views' `values()`
/// and `items()`: one walk of the parent's children, each kept one
/// described only as `__next__` reaches it.
#[pyclass(name = "WarehouseObjects", module = "yggdryl._native")]
pub(crate) struct PyObjects {
    objects: Objects,
    /// The one kind yielded, every kind for `None`.
    kind: Option<IOKind>,
    /// Whether each object is yielded beside its name, as `items` does.
    items: bool,
}

impl PyObjects {
    const fn new(objects: Objects, kind: Option<IOKind>, items: bool) -> Self {
        Self {
            objects,
            kind,
            items,
        }
    }
}

#[pymethods]
impl PyObjects {
    // Consumption changes iterator state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        loop {
            let Some(object) = self.objects.next().transpose().map_err(value_error)? else {
                return Ok(None);
            };
            if self.kind.is_some_and(|kind| object.kind() != kind) {
                continue;
            }
            let name = object.name().to_owned();
            let object = described(py, object)?;
            return Ok(Some(if self.items {
                (name, object).into_pyobject(py)?.into_any().unbind()
            } else {
                object
            }));
        }
    }
}

/// The registry of catalogs a path resolves against.
///
/// Catalogs register by name, in order; a namespace or a table registers at
/// its path, the memory catalogs and namespaces along it created as needed.
/// A folder catalog lists its own store, and registering under one is
/// refused by name.
#[pyclass(name = "Warehouse", module = "yggdryl._native", skip_from_py_object)]
#[derive(Default)]
pub(crate) struct PyWarehouse {
    inner: Warehouse,
}

impl PyWarehouse {
    /// The core registry, what a plan resolves against.
    pub(crate) const fn core(&self) -> &Warehouse {
        &self.inner
    }

    fn catalogs_into_py(py: Python<'_>, catalogs: &[Catalog]) -> PyResult<Vec<Py<PyAny>>> {
        catalogs
            .iter()
            .cloned()
            .map(|catalog| describe(py, Holder::from(catalog)))
            .collect()
    }
}

#[pymethods]
impl PyWarehouse {
    // A warehouse changes as objects register, so it follows Python's hash
    // contract for a mutable value with equality.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    /// An empty warehouse.
    #[new]
    fn new() -> Self {
        Self::default()
    }

    /// Register an object: a catalog by its name; a namespace or a table at
    /// its path, creating memory catalogs and namespaces along the path. A
    /// name taken at that level is a conflict.
    fn register(&mut self, object: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner
            .register(object_from_value(object)?)
            .map_err(value_error)
    }

    /// Register an object, replacing the one of its name at that level and
    /// answering what was replaced, `None` for nothing.
    fn replace(
        &mut self,
        py: Python<'_>,
        object: &Bound<'_, PyAny>,
    ) -> PyResult<Option<Py<PyAny>>> {
        self.inner
            .replace(object_from_value(object)?)
            .map_err(value_error)?
            .map(|replaced| described(py, replaced))
            .transpose()
    }

    /// Remove the object registered at `path` - dotted text or parts -
    /// answering it.
    fn unregister(&mut self, py: Python<'_>, path: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let path = object_path_from_value(path)?;
        let object = self.inner.unregister(path).map_err(value_error)?;
        described(py, object)
    }

    /// The catalogs, in registration order.
    #[getter]
    fn catalogs(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        Self::catalogs_into_py(py, self.inner.catalogs())
    }

    /// The catalog called `name`.
    fn catalog(&self, py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let catalog = self.inner.catalog(name).map_err(value_error)?.clone();
        describe(py, Holder::from(catalog))
    }

    /// The object a path names: a catalog by its one part, else what the
    /// catalog's descent reaches.
    fn get(&self, py: Python<'_>, path: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let path = object_path_from_value(path)?;
        described(py, self.inner.get(path).map_err(value_error)?)
    }

    /// The table a path names; a namespace or a catalog there is the absence
    /// of a table.
    fn table(&self, py: Python<'_>, path: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let path = object_path_from_value(path)?;
        let table = self.inner.table(path).map_err(value_error)?;
        describe(py, Holder::from(table))
    }

    /// The namespace a path names; a table or a catalog there is the absence
    /// of a namespace.
    fn namespace(&self, py: Python<'_>, path: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let path = object_path_from_value(path)?;
        let namespace = self.inner.namespace(path).map_err(value_error)?;
        describe(py, Holder::from(namespace))
    }

    /// The effective properties of the deepest registered object whose URL
    /// holds `url` on a path boundary; the empty mapping where none does.
    fn properties_for<'py>(
        &self,
        py: Python<'py>,
        url: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let url = core_url_from_value(url)?;
        properties_into_py(py, &self.inner.properties_for(&url))
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

    fn __repr__(&self) -> String {
        let names: Vec<&str> = self
            .inner
            .catalogs()
            .iter()
            .map(ObjectValue::name)
            .collect();
        format!("Warehouse({names:?})")
    }
}

/// The process's one warehouse, which a plan's `from catalog.namespace.table`
/// resolves against when it is given no other.
///
/// It starts with the memory catalog `local`, holding the folder namespaces
/// `temporary`, `home` and `config`. Every method is static and takes the
/// registry's lock for its own call.
#[pyclass(name = "SystemWarehouse", module = "yggdryl._native")]
pub(crate) struct PySystemWarehouse;

#[pymethods]
impl PySystemWarehouse {
    /// [`Warehouse.register`](PyWarehouse::register) on the system
    /// warehouse.
    #[staticmethod]
    fn register(object: &Bound<'_, PyAny>) -> PyResult<()> {
        SystemWarehouse::register(object_from_value(object)?).map_err(value_error)
    }

    /// [`Warehouse.replace`](PyWarehouse::replace) on the system warehouse.
    #[staticmethod]
    fn replace(py: Python<'_>, object: &Bound<'_, PyAny>) -> PyResult<Option<Py<PyAny>>> {
        SystemWarehouse::replace(object_from_value(object)?)
            .map_err(value_error)?
            .map(|replaced| described(py, replaced))
            .transpose()
    }

    /// [`Warehouse.unregister`](PyWarehouse::unregister) on the system
    /// warehouse.
    #[staticmethod]
    fn unregister(py: Python<'_>, path: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let path = object_path_from_value(path)?;
        described(py, SystemWarehouse::unregister(path).map_err(value_error)?)
    }

    /// The registered catalogs, in order.
    #[staticmethod]
    fn catalogs(py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        PyWarehouse::catalogs_into_py(py, &SystemWarehouse::catalogs())
    }

    /// The catalog called `name`.
    #[staticmethod]
    fn catalog(py: Python<'_>, name: &str) -> PyResult<Py<PyAny>> {
        let catalog = SystemWarehouse::catalog(name).map_err(value_error)?;
        describe(py, Holder::from(catalog))
    }

    /// [`Warehouse.get`](PyWarehouse::get) on the system warehouse.
    #[staticmethod]
    fn get(py: Python<'_>, path: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let path = object_path_from_value(path)?;
        described(py, SystemWarehouse::get(path).map_err(value_error)?)
    }

    /// [`Warehouse.table`](PyWarehouse::table) on the system warehouse.
    #[staticmethod]
    fn table(py: Python<'_>, path: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let path = object_path_from_value(path)?;
        let table = SystemWarehouse::table(path).map_err(value_error)?;
        describe(py, Holder::from(table))
    }

    /// [`Warehouse.namespace`](PyWarehouse::namespace) on the system
    /// warehouse.
    #[staticmethod]
    fn namespace(py: Python<'_>, path: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
        let path = object_path_from_value(path)?;
        let namespace = SystemWarehouse::namespace(path).map_err(value_error)?;
        describe(py, Holder::from(namespace))
    }

    /// [`Warehouse.properties_for`](PyWarehouse::properties_for) on the
    /// system warehouse.
    #[staticmethod]
    fn properties_for<'py>(
        py: Python<'py>,
        url: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let url = core_url_from_value(url)?;
        properties_into_py(py, &SystemWarehouse::properties_for(&url))
    }
}

/// Register every warehouse class.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyCatalog>()?;
    module.add_class::<PyMemoryCatalog>()?;
    module.add_class::<PyFolderCatalog>()?;
    module.add_class::<PyNamespace>()?;
    module.add_class::<PyMemoryNamespace>()?;
    module.add_class::<PyFolderNamespace>()?;
    module.add_class::<PyTable>()?;
    module.add_class::<PyMediaTable>()?;
    module.add_class::<PyNamespaces>()?;
    module.add_class::<PyTables>()?;
    module.add_class::<PyNames>()?;
    module.add_class::<PyObjects>()?;
    module.add_class::<PyWarehouse>()?;
    module.add_class::<PySystemWarehouse>()?;
    Ok(())
}
