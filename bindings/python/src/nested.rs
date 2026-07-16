//! The `yggdryl.types` submodule's **nested (composite) layer** — `StructField` (the centralized
//! struct schema) and `StructSerie` (a nullable struct column of heterogeneous child columns),
//! mirroring `yggdryl_core::io::nested`.
//!
//! A `StructField` is a value type (hashable, pickles through its byte codec) describing an ordered,
//! named set of child fields (each a `Field` or a nested `StructField`). A `StructSerie` is a
//! struct column: its children are the crate's existing `Serie` columns (`U8Serie` … `Utf8Serie`,
//! `D32Serie` …, `NullSerie`, or a nested `StructSerie`), erased through the core's `AnySerie`. It
//! serializes to the **same canonical bytes** in every language, so a struct built here round-trips
//! byte-for-byte with the Rust core and the Node extension.

// pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type `From`.
#![allow(clippy::useless_conversion)]

use std::collections::hash_map::DefaultHasher;
use std::ffi::CString;
use std::hash::{Hash, Hasher};

use arrow_array::ffi::{from_ffi, to_ffi, FFI_ArrowArray};
use arrow_array::Array;
use arrow_schema::ffi::FFI_ArrowSchema;
use pyo3::exceptions::{PyIndexError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyCapsule, PyDict, PyFloat, PyInt, PySlice, PyString, PyTuple};

use yggdryl_core::io::fixed::{
    f16, Dec128, Dec256, Dec32, Dec64, Field as CoreField, NativeType, NullSerie as CoreNullSerie,
    I256, I96, U256, U96,
};
use yggdryl_core::io::nested::{
    ListField as CoreListField, ListSerie as CoreListSerie, MapField as CoreMapField,
    MapSerie as CoreMapSerie, StructField as CoreStructField, StructSerie as CoreStructSerie,
};
use yggdryl_core::io::var::{Binary, Utf8};
use yggdryl_core::io::{
    boxed, AnyField, AnyScalar, AnySerie, DataTypeId, IoError, NodePath, PathError, PathSegment,
};

use crate::deccolumn::{D128Serie, D256Serie, D32Serie, D64Serie};
use crate::nullvalues::NullSerie;
use crate::types::{DataType, Field};
use crate::values::{
    F16Serie, F32Serie, F64Serie, I128Serie, I16Serie, I256Serie, I32Serie, I64Serie, I8Serie,
    I96Serie, PyNative, U128Serie, U16Serie, U256Serie, U32Serie, U64Serie, U8Serie, U96Serie,
};
use crate::varvalues::{BinarySerie, PyVarKind, Utf8Serie};

/// Maps an [`IoError`] to a Python `ValueError` carrying its guided text.
fn io_err(error: IoError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Names a (self-describing) erased column in place — the one-line replacement for the removed
/// `NamedSerie` carrier (the name goes straight into the column's own header).
fn named_column(mut column: Box<dyn AnySerie>, name: &str) -> Box<dyn AnySerie> {
    column.set_name(name);
    column
}

/// Maps an Arrow error to a Python `ValueError`.
fn arrow_err(error: arrow_schema::ArrowError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// A capsule name (`"arrow_schema"` / `"arrow_array"`), as the Arrow PyCapsule protocol requires.
fn capsule_name(name: &str) -> CString {
    CString::new(name).expect("a static ASCII capsule name has no interior NUL")
}

/// Boxes any yggdryl column wrapper (fixed / decimal / var / null / nested) into an erased
/// [`AnySerie`], by cloning its `inner` core column. Every wrapper shares the `inner` field and its
/// core type implements `AnySerie`, so one list of types covers them all.
fn extract_column(obj: &Bound<'_, PyAny>) -> PyResult<Box<dyn AnySerie>> {
    macro_rules! try_wrappers {
        ($($W:ty),+ $(,)?) => {
            $( if let Ok(w) = obj.extract::<PyRef<$W>>() { return Ok(boxed(w.inner.clone())); } )+
        };
    }
    try_wrappers!(
        U8Serie,
        U16Serie,
        U32Serie,
        U64Serie,
        U96Serie,
        U128Serie,
        U256Serie,
        I8Serie,
        I16Serie,
        I32Serie,
        I64Serie,
        I96Serie,
        I128Serie,
        I256Serie,
        F16Serie,
        F32Serie,
        F64Serie,
        D32Serie,
        D64Serie,
        D128Serie,
        D256Serie,
        Utf8Serie,
        BinarySerie,
        NullSerie,
        StructSerie,
        ListSerie,
        MapSerie,
    );
    Err(PyValueError::new_err(
        "expected a yggdryl column (a Serie: U8Serie … Utf8Serie, D32Serie …, NullSerie, \
         StructSerie, ListSerie, or MapSerie)",
    ))
}

/// Re-wraps an erased child column back into its concrete Python `Serie` class, keyed on its type.
fn rewrap_column(any: &(dyn AnySerie + 'static), py: Python<'_>) -> PyResult<PyObject> {
    macro_rules! fixed {
        ($t:ty, $W:ident) => {
            $W {
                inner: any.as_serie::<$t>().expect("type_id matched").clone(),
            }
            .into_py(py)
        };
    }
    macro_rules! decimal {
        ($B:ty, $W:ident) => {
            $W {
                inner: any.as_decimal::<$B>().expect("type_id matched").clone(),
            }
            .into_py(py)
        };
    }
    Ok(match any.type_id() {
        DataTypeId::U8 => fixed!(u8, U8Serie),
        DataTypeId::U16 => fixed!(u16, U16Serie),
        DataTypeId::U32 => fixed!(u32, U32Serie),
        DataTypeId::U64 => fixed!(u64, U64Serie),
        DataTypeId::U96 => fixed!(yggdryl_core::io::fixed::U96, U96Serie),
        DataTypeId::U128 => fixed!(u128, U128Serie),
        DataTypeId::U256 => fixed!(yggdryl_core::io::fixed::U256, U256Serie),
        DataTypeId::I8 => fixed!(i8, I8Serie),
        DataTypeId::I16 => fixed!(i16, I16Serie),
        DataTypeId::I32 => fixed!(i32, I32Serie),
        DataTypeId::I64 => fixed!(i64, I64Serie),
        DataTypeId::I96 => fixed!(yggdryl_core::io::fixed::I96, I96Serie),
        DataTypeId::I128 => fixed!(i128, I128Serie),
        DataTypeId::I256 => fixed!(yggdryl_core::io::fixed::I256, I256Serie),
        DataTypeId::F16 => fixed!(yggdryl_core::io::fixed::f16, F16Serie),
        DataTypeId::F32 => fixed!(f32, F32Serie),
        DataTypeId::F64 => fixed!(f64, F64Serie),
        DataTypeId::D32 => decimal!(Dec32, D32Serie),
        DataTypeId::D64 => decimal!(Dec64, D64Serie),
        DataTypeId::D128 => decimal!(Dec128, D128Serie),
        DataTypeId::D256 => decimal!(Dec256, D256Serie),
        DataTypeId::Utf8 => Utf8Serie {
            inner: any
                .as_bytes_serie::<Utf8>()
                .expect("type_id matched")
                .clone(),
        }
        .into_py(py),
        DataTypeId::Binary => BinarySerie {
            inner: any
                .as_bytes_serie::<Binary>()
                .expect("type_id matched")
                .clone(),
        }
        .into_py(py),
        DataTypeId::Null => NullSerie {
            inner: any
                .downcast_ref::<CoreNullSerie>()
                .expect("type_id matched")
                .clone(),
        }
        .into_py(py),
        DataTypeId::Struct => StructSerie {
            inner: any
                .downcast_ref::<CoreStructSerie>()
                .expect("type_id matched")
                .clone(),
        }
        .into_py(py),
        DataTypeId::List => ListSerie {
            inner: any
                .downcast_ref::<CoreListSerie>()
                .expect("type_id matched")
                .clone(),
        }
        .into_py(py),
        DataTypeId::Map => MapSerie {
            inner: any
                .downcast_ref::<CoreMapSerie>()
                .expect("type_id matched")
                .clone(),
        }
        .into_py(py),
        other => {
            return Err(PyValueError::new_err(format!(
                "a nested child of type {} has no Python column wrapper",
                other.name()
            )))
        }
    })
}

/// A `Field` (leaf) or nested `StructField` / `ListField` / `MapField` Python object → an erased
/// [`AnyField`].
fn extract_any_field(obj: &Bound<'_, PyAny>) -> PyResult<AnyField> {
    if let Ok(field) = obj.extract::<PyRef<Field>>() {
        return Ok(AnyField::leaf(field.inner.clone()));
    }
    if let Ok(field) = obj.extract::<PyRef<StructField>>() {
        return Ok(field.inner.as_any_field().clone());
    }
    if let Ok(field) = obj.extract::<PyRef<ListField>>() {
        return Ok(field.inner.as_any_field().clone());
    }
    if let Ok(field) = obj.extract::<PyRef<MapField>>() {
        return Ok(field.inner.as_any_field().clone());
    }
    Err(PyValueError::new_err(
        "expected a Field, StructField, ListField, or MapField as a child field",
    ))
}

/// An erased [`AnyField`] → its concrete Python `Field` / `StructField` / `ListField` / `MapField`
/// object (the recursion mirror of [`rewrap_column`]).
fn rewrap_field(any: &AnyField, py: Python<'_>) -> PyResult<PyObject> {
    if any.is_struct() {
        let inner = CoreStructField::from_any_field(any.clone())
            .expect("a struct AnyField rebuilds a StructField");
        Ok(StructField { inner }.into_py(py))
    } else if any.is_list() {
        let inner = CoreListField::from_any_field(any.clone())
            .expect("a list AnyField rebuilds a ListField");
        Ok(ListField { inner }.into_py(py))
    } else if any.is_map() {
        let inner =
            CoreMapField::from_any_field(any.clone()).expect("a map AnyField rebuilds a MapField");
        Ok(MapField { inner }.into_py(py))
    } else {
        let inner = any
            .as_leaf()
            .expect("a non-nested AnyField is a leaf")
            .clone();
        Ok(Field { inner }.into_py(py))
    }
}

// =====================================================================================
// Deep navigation (get / set a cell or sub-column by coords or path) — every method is a
// 1–3 line delegate to the core's `dyn AnySerie` surface (`get_at` / `get_scalar_by_path` /
// `get_by_path` / `set_at` / `set_by_path`). The only binding-side logic is idiom dispatch (an
// int vs a coords tuple vs a str path vs a slice) and marshaling an erased `AnyScalar` cell to /
// from a native Python value, which reuses the leaf `Serie` wrappers' own conversions
// (`PyNative` / `PyVarKind`) so a deep read/write is identical to a leaf-column read/write.
// =====================================================================================

/// The help text for a bad `__getitem__` key.
const GET_KEY_HELP: &str = "a nested index must be an int (a row), a tuple of ints (deep-cell \
    coordinates), a str path (\"a[1]\" reaches a cell, \"a.b\" a sub-column), or a slice";

/// The help text for a bad `get_cell` / `set_cell` key (a cell only — no int row, no name-terminal
/// column).
const CELL_KEY_HELP: &str =
    "a cell key must be a tuple of ints (coordinates) or a str path ending \
    in an index (e.g. \"a[1]\")";

/// Maps a [`PathError`] (a bad path string, or a missing child) to a Python `ValueError` carrying
/// its guided text.
fn path_err(error: PathError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// Maps a deep-navigation [`IoError`] to Python: an out-of-range cell is an `IndexError` (native
/// sequence semantics), every other guided failure a `ValueError`. The core's text passes through
/// unchanged in both.
fn deep_err(error: IoError) -> PyErr {
    match error {
        IoError::IndexOutOfBounds { .. } => PyIndexError::new_err(error.to_string()),
        other => PyValueError::new_err(other.to_string()),
    }
}

/// Marshals an erased [`AnyScalar`] cell to a native Python value — the read half of the keystone
/// bridge, used where **only the scalar is in scope** (a deep-cell read: `get_at` /
/// `get_scalar_by_path`). A null is `None`; a **leaf** is decoded from its canonical little-endian
/// bytes by [`DataTypeId`], reusing the leaf `Serie` wrappers' own `PyNative` / `PyVarKind`
/// conversion (no second marshaling); a **list** / **map** cell returns its inner sub-column as a
/// live `Serie` wrapper (`rewrap_column`).
///
/// A whole **struct** cell is a guided error here: the erased struct scalar is *positional /
/// name-free* (its per-field leaves carry an empty name — see
/// [`AnyScalar::child_scalar_by`](yggdryl_core::io::AnyScalar::child_scalar_by)), so it has no
/// field names to key a dict, and a deep cell carries no schema in scope to recover them from. The
/// **row read** (`s[row]`) never reaches this arm — it goes through [`cell_to_py`], which has the
/// struct column's schema and renders a struct as a `{field_name: cell}` dict (recursively).
// DESIGN: struct-cell dict-ification is anchored on the row path (`cell_to_py`), which holds the
// column schema; a bare deep-cell struct read keeps the guided error rather than emit a name-less
// or index-keyed dict that would diverge from the named dict `s[row]` returns.
fn any_scalar_to_py(scalar: &AnyScalar, py: Python<'_>) -> PyResult<PyObject> {
    if scalar.is_null() {
        return Ok(py.None());
    }
    if let Some(items) = scalar.as_list() {
        return rewrap_column(items, py);
    }
    if let Some((entries, _)) = scalar.as_map() {
        return rewrap_column(entries, py);
    }
    if scalar.as_struct().is_some() {
        return Err(PyValueError::new_err(
            "reading a whole struct cell as a native value is not supported through deep indexing; \
             read the row as a dict with s[row], or index a leaf cell (e.g. s[field_index, row])",
        ));
    }
    // A present leaf: decode its canonical little-endian bytes by type.
    let type_id = scalar.type_id().expect("a non-null scalar reports a type");
    let bytes = scalar.bytes().expect("a leaf carries canonical bytes");
    leaf_bytes_to_py(type_id, bytes, py)
}

/// Renders the `row`-th cell of an erased `column` as a native Python value, **recursing** into a
/// nested struct cell as a `{field_name: cell}` dict — the row-read counterpart of
/// [`any_scalar_to_py`], which (unlike this) has the column's schema in scope, so it can name the
/// struct's fields. A **null** cell (or a null struct row) is `None`; a **struct** cell is a dict
/// keyed by the child columns' own header names (the erased struct scalar is name-free), each value
/// recursing so a struct-in-struct nests as a dict; every **leaf** / **list** / **map** cell defers
/// to [`any_scalar_to_py`] (a leaf → native value, a list/map → its element/entries sub-`Serie`).
fn cell_to_py(column: &(dyn AnySerie + 'static), row: usize, py: Python<'_>) -> PyResult<PyObject> {
    let scalar = column.value(row);
    if scalar.is_null() {
        return Ok(py.None());
    }
    if column.type_id() == DataTypeId::Struct {
        let dict = PyDict::new_bound(py);
        for child_index in 0..column.num_children() {
            let child = column
                .child_serie_at(child_index)
                .expect("child index in range");
            dict.set_item(child.name(), cell_to_py(child, row, py)?)?;
        }
        return Ok(dict.into_any().unbind());
    }
    any_scalar_to_py(&scalar, py)
}

/// Decodes a leaf cell's canonical little-endian `bytes` of type `type_id` to its native Python
/// value, reusing the per-type marshaling the leaf `Serie` wrappers already expose. A type with no
/// native cross-language form here (decimal / temporal / fixed-size) is a guided error naming the
/// column-access fallback.
fn leaf_bytes_to_py(type_id: DataTypeId, bytes: &[u8], py: Python<'_>) -> PyResult<PyObject> {
    macro_rules! fixed {
        ($t:ty) => {
            <$t as NativeType>::read_le(bytes).to_py(py)
        };
    }
    match type_id {
        DataTypeId::U8 => fixed!(u8),
        DataTypeId::U16 => fixed!(u16),
        DataTypeId::U32 => fixed!(u32),
        DataTypeId::U64 => fixed!(u64),
        DataTypeId::U96 => fixed!(U96),
        DataTypeId::U128 => fixed!(u128),
        DataTypeId::U256 => fixed!(U256),
        DataTypeId::I8 => fixed!(i8),
        DataTypeId::I16 => fixed!(i16),
        DataTypeId::I32 => fixed!(i32),
        DataTypeId::I64 => fixed!(i64),
        DataTypeId::I96 => fixed!(I96),
        DataTypeId::I128 => fixed!(i128),
        DataTypeId::I256 => fixed!(I256),
        DataTypeId::F16 => fixed!(f16),
        DataTypeId::F32 => fixed!(f32),
        DataTypeId::F64 => fixed!(f64),
        DataTypeId::Utf8 => Ok(<Utf8 as PyVarKind>::bytes_to_py(bytes, py)),
        DataTypeId::Binary => Ok(<Binary as PyVarKind>::bytes_to_py(bytes, py)),
        other => Err(PyValueError::new_err(format!(
            "reading a {} cell as a native Python value is not supported through deep indexing; read \
             the column with get_column(path) and index its concrete Serie",
            other.name()
        ))),
    }
}

/// Marshals a native Python value into an erased leaf [`AnyScalar`] of type `target` (width `width`)
/// — the write half of the keystone bridge, reusing the leaf `Serie` wrappers' own `PyNative` /
/// `PyVarKind` extraction. A Python `None` (or absent value) is a null cell. A type with no native
/// input form here is a guided error. The core `set_*` then re-validates and gives its own guided
/// error on any residual mismatch.
fn py_to_any_scalar(
    value: Option<&Bound<'_, PyAny>>,
    target: DataTypeId,
    width: usize,
) -> PyResult<AnyScalar> {
    let value = match value {
        Some(value) if !value.is_none() => value,
        _ => return Ok(AnyScalar::null()),
    };
    macro_rules! fixed {
        ($t:ty) => {{
            let parsed = <$t as PyNative>::from_py(value)?;
            let mut scratch = [0u8; 32];
            parsed.write_le(&mut scratch);
            let bytes = scratch[..<$t as NativeType>::WIDTH].to_vec();
            AnyScalar::leaf(CoreField::of("", target, width, false), bytes)
        }};
    }
    // A variable-length leaf field carries the offset width (4), matching `ByteSerie::value`.
    let offset_width = std::mem::size_of::<i32>();
    Ok(match target {
        DataTypeId::U8 => fixed!(u8),
        DataTypeId::U16 => fixed!(u16),
        DataTypeId::U32 => fixed!(u32),
        DataTypeId::U64 => fixed!(u64),
        DataTypeId::U96 => fixed!(U96),
        DataTypeId::U128 => fixed!(u128),
        DataTypeId::U256 => fixed!(U256),
        DataTypeId::I8 => fixed!(i8),
        DataTypeId::I16 => fixed!(i16),
        DataTypeId::I32 => fixed!(i32),
        DataTypeId::I64 => fixed!(i64),
        DataTypeId::I96 => fixed!(I96),
        DataTypeId::I128 => fixed!(i128),
        DataTypeId::I256 => fixed!(I256),
        DataTypeId::F16 => fixed!(f16),
        DataTypeId::F32 => fixed!(f32),
        DataTypeId::F64 => fixed!(f64),
        DataTypeId::Utf8 => AnyScalar::leaf(
            CoreField::of("", target, offset_width, false),
            <Utf8 as PyVarKind>::py_to_bytes(value)?,
        ),
        DataTypeId::Binary => AnyScalar::leaf(
            CoreField::of("", target, offset_width, false),
            <Binary as PyVarKind>::py_to_bytes(value)?,
        ),
        other => {
            return Err(PyValueError::new_err(format!(
            "setting a {} cell through deep indexing is not supported; set the column's concrete \
                 Serie cell directly",
            other.name()
        )))
        }
    })
}

/// The `(type_id, byte_width)` of the leaf **column** addressed by the cell path `cell_path` (its
/// parent), so a value set into a cell — even a currently-null one — casts to the leaf's actual
/// type. Reuses the core's `get_by_path` navigation on the parent path.
fn cell_target_type(
    root: &(dyn AnySerie + 'static),
    cell_path: &NodePath,
) -> PyResult<(DataTypeId, usize)> {
    let column = match cell_path.parent() {
        Some(parent) => root.get_by_path(&parent.to_string()).map_err(path_err)?,
        None => root,
    };
    let id = column.type_id();
    Ok((id, id.fixed_byte_width().unwrap_or(0)))
}

/// Builds the erased cell value to write at `cell_path`: `None` → a null; otherwise the value cast
/// to the addressed leaf column's type.
fn build_cell_scalar(
    root: &(dyn AnySerie + 'static),
    cell_path: &NodePath,
    value: &Bound<'_, PyAny>,
) -> PyResult<AnyScalar> {
    if value.is_none() {
        return Ok(AnyScalar::null());
    }
    let (target, width) = cell_target_type(root, cell_path)?;
    py_to_any_scalar(Some(value), target, width)
}

/// Extracts a coordinate tuple (`s[i, j, …]`) into positional child/cell indices.
fn extract_coords(tuple: &Bound<'_, PyTuple>) -> PyResult<Vec<usize>> {
    let mut coords = Vec::with_capacity(tuple.len());
    for item in tuple.iter() {
        coords.push(item.extract::<usize>().map_err(|_| {
            PyValueError::new_err("nested coordinates must be non-negative integers")
        })?);
    }
    Ok(coords)
}

/// The deep cell at `coords` as a native Python value (`get_at` → the bridge).
fn deep_cell_by_coords(
    root: &(dyn AnySerie + 'static),
    coords: &[usize],
    py: Python<'_>,
) -> PyResult<PyObject> {
    let scalar = root.get_at(coords).map_err(deep_err)?;
    any_scalar_to_py(&scalar, py)
}

/// The deep cell at an index-terminal `path` as a native Python value (`get_scalar_by_path` → the
/// bridge). A name-terminal path (a column, not a cell) surfaces the core's guided error.
fn cell_by_path(root: &(dyn AnySerie + 'static), path: &str, py: Python<'_>) -> PyResult<PyObject> {
    let scalar = root.get_scalar_by_path(path).map_err(deep_err)?;
    any_scalar_to_py(&scalar, py)
}

/// A str key: an index-terminal path reads a **cell** (native), a name-terminal path reads a
/// **sub-column** (a live `Serie` wrapper).
fn cell_or_column_by_path(
    root: &(dyn AnySerie + 'static),
    path: &str,
    py: Python<'_>,
) -> PyResult<PyObject> {
    let parsed = NodePath::parse(path).map_err(path_err)?;
    match parsed.segments().last() {
        Some(PathSegment::Index(_)) => cell_by_path(root, path, py),
        _ => rewrap_column(root.get_by_path(path).map_err(path_err)?, py),
    }
}

/// A `start:stop` slice → a fresh sub-column wrapper (`slice` → `rewrap_column`). Only a step of 1
/// is supported.
fn slice_to_wrapper(
    root: &(dyn AnySerie + 'static),
    slice: &Bound<'_, PySlice>,
    py: Python<'_>,
) -> PyResult<PyObject> {
    let indices = slice.indices(root.len() as isize)?;
    if indices.step != 1 {
        return Err(PyValueError::new_err("a nested slice step must be 1"));
    }
    let sub = root.slice(indices.start as usize, indices.slicelength);
    rewrap_column(sub.as_ref(), py)
}

/// `get_cell(key)` — a cell key (coords tuple or index-terminal path) to a native value only.
fn get_cell_by_key(
    root: &(dyn AnySerie + 'static),
    key: &Bound<'_, PyAny>,
    py: Python<'_>,
) -> PyResult<PyObject> {
    if let Ok(tuple) = key.downcast::<PyTuple>() {
        return deep_cell_by_coords(root, &extract_coords(tuple)?, py);
    }
    if let Ok(path) = key.extract::<String>() {
        return cell_by_path(root, &path, py);
    }
    Err(PyTypeError::new_err(CELL_KEY_HELP))
}

/// `set_cell(key, value)` / `__setitem__` — a cell key (coords tuple or index-terminal path) set to
/// a value (a Python `None` writes a null).
fn set_cell_by_key(
    root: &mut (dyn AnySerie + 'static),
    key: &Bound<'_, PyAny>,
    value: &Bound<'_, PyAny>,
) -> PyResult<()> {
    if let Ok(tuple) = key.downcast::<PyTuple>() {
        let coords = extract_coords(tuple)?;
        let cell_path =
            NodePath::from_segments(coords.iter().map(|&i| PathSegment::Index(i)).collect());
        let scalar = build_cell_scalar(&*root, &cell_path, value)?;
        return root.set_at(&coords, &scalar).map_err(deep_err);
    }
    if let Ok(path) = key.extract::<String>() {
        let cell_path = NodePath::parse(&path).map_err(path_err)?;
        if !matches!(cell_path.segments().last(), Some(PathSegment::Index(_))) {
            return Err(PyValueError::new_err(
                "a str cell assignment must address a leaf cell (an index-terminal path like \
                 \"a[1]\"); a name-terminal path addresses a whole column, which has no in-place \
                 assignment",
            ));
        }
        let scalar = build_cell_scalar(&*root, &cell_path, value)?;
        return root.set_by_path(&path, &scalar).map_err(deep_err);
    }
    Err(PyTypeError::new_err(CELL_KEY_HELP))
}

/// Emits the shared deep-navigation `#[pymethods]` for a nested column wrapper — `__getitem__` /
/// `__setitem__` (dunders) plus the JS-parity named methods (`get_cell` / `set_cell` /
/// `get_column` / `child_at` / `child_named` / `num_children`). Each is a 1–3 line delegate to the
/// core `dyn AnySerie` surface; the per-family int-row read is the class's own `row` method.
macro_rules! nested_navigation {
    ($Serie:ident) => {
        #[pymethods]
        impl $Serie {
            /// `s[key]` — an int reads a row, a tuple of ints a deep cell (native), a str path a
            /// cell (index-terminal) or a sub-column (name-terminal), a slice a sub-column.
            fn __getitem__(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<PyObject> {
                let root = &self.inner as &dyn AnySerie;
                if let Ok(tuple) = key.downcast::<PyTuple>() {
                    return deep_cell_by_coords(root, &extract_coords(tuple)?, py);
                }
                if let Ok(path) = key.extract::<String>() {
                    return cell_or_column_by_path(root, &path, py);
                }
                if let Ok(slice) = key.downcast::<PySlice>() {
                    return slice_to_wrapper(root, slice, py);
                }
                if let Ok(index) = key.extract::<isize>() {
                    return self.row(py, index);
                }
                Err(PyTypeError::new_err(GET_KEY_HELP))
            }

            /// `s[key] = value` — a tuple of ints or an index-terminal str path sets a leaf cell
            /// (a Python `None` writes a null).
            fn __setitem__(
                &mut self,
                key: &Bound<'_, PyAny>,
                value: &Bound<'_, PyAny>,
            ) -> PyResult<()> {
                set_cell_by_key(&mut self.inner as &mut dyn AnySerie, key, value)
            }

            /// The deep cell at `key` (a coords tuple or an index-terminal str path) as a native
            /// value — like `s[key]` but never a sub-column.
            fn get_cell(&self, py: Python<'_>, key: &Bound<'_, PyAny>) -> PyResult<PyObject> {
                get_cell_by_key(&self.inner as &dyn AnySerie, key, py)
            }

            /// Sets the deep cell at `key` (a coords tuple or an index-terminal str path) to
            /// `value` (a Python `None` writes a null) — the named twin of `s[key] = value`.
            fn set_cell(
                &mut self,
                key: &Bound<'_, PyAny>,
                value: &Bound<'_, PyAny>,
            ) -> PyResult<()> {
                set_cell_by_key(&mut self.inner as &mut dyn AnySerie, key, value)
            }

            /// The sub-**column** addressed by `path` (a name-terminal path), as its concrete
            /// `Serie` wrapper.
            fn get_column(&self, py: Python<'_>, path: &str) -> PyResult<PyObject> {
                let column = (&self.inner as &dyn AnySerie)
                    .get_by_path(path)
                    .map_err(path_err)?;
                rewrap_column(column, py)
            }

            /// The child column at `index` (a struct field, a list item, a map key/value), as its
            /// concrete `Serie`; raises `IndexError` out of range.
            fn child_at(&self, py: Python<'_>, index: usize) -> PyResult<PyObject> {
                match (&self.inner as &dyn AnySerie).child_serie_at(index) {
                    Some(child) => rewrap_column(child, py),
                    None => Err(PyIndexError::new_err("child column index out of range")),
                }
            }

            /// The child column named `name`, as its concrete `Serie`, or `None`.
            fn child_named(&self, py: Python<'_>, name: &str) -> PyResult<Option<PyObject>> {
                match (&self.inner as &dyn AnySerie).child_serie_by(name) {
                    Some(child) => rewrap_column(child, py).map(Some),
                    None => Ok(None),
                }
            }

            /// The number of child columns (schema fan-out).
            #[getter]
            fn num_children(&self) -> usize {
                (&self.inner as &dyn AnySerie).num_children()
            }
        }
    };
}

// ---- Arrow C Data Interface (PyCapsule) helpers, shared by every nested column ---------------

/// Exports an Arrow array as the Arrow C Data Interface **(schema, array)** capsule pair, zero-copy.
fn export_c_array(py: Python<'_>, array: arrow_array::ArrayRef) -> PyResult<(PyObject, PyObject)> {
    let data = array.to_data();
    let (ffi_array, ffi_schema) = to_ffi(&data).map_err(arrow_err)?;
    let schema_capsule = PyCapsule::new_bound(py, ffi_schema, Some(capsule_name("arrow_schema")))?;
    let array_capsule = PyCapsule::new_bound(py, ffi_array, Some(capsule_name("arrow_array")))?;
    Ok((
        schema_capsule.into_any().unbind(),
        array_capsule.into_any().unbind(),
    ))
}

/// Exports an Arrow data type as the Arrow C Data Interface **schema** capsule (`"arrow_schema"`).
fn export_c_schema(py: Python<'_>, data_type: &arrow_schema::DataType) -> PyResult<PyObject> {
    let schema = FFI_ArrowSchema::try_from(data_type).map_err(arrow_err)?;
    let capsule = PyCapsule::new_bound(py, schema, Some(capsule_name("arrow_schema")))?;
    Ok(capsule.into_any().unbind())
}

/// Imports any object exposing the Arrow C Data Interface into an owned Arrow array, zero-copy —
/// the shared inverse of [`export_c_array`] every nested `from_arrow` routes through.
fn import_c_array(obj: &Bound<'_, PyAny>) -> PyResult<arrow_array::ArrayRef> {
    let pair = obj.call_method0("__arrow_c_array__")?;
    let (schema_cap, array_cap): (Bound<'_, PyAny>, Bound<'_, PyAny>) = pair.extract()?;
    let schema_cap = schema_cap.downcast::<PyCapsule>()?;
    let array_cap = array_cap.downcast::<PyCapsule>()?;
    // Move the FFI structs out of the producer's capsules, then blank the sources so the producer's
    // capsule destructors see a released struct and do not double-free.
    let (ffi_array, ffi_schema) = unsafe {
        let array_ptr = array_cap.pointer() as *mut FFI_ArrowArray;
        let schema_ptr = schema_cap.pointer() as *mut FFI_ArrowSchema;
        let array = std::ptr::read(array_ptr);
        let schema = std::ptr::read(schema_ptr);
        std::ptr::write(array_ptr, FFI_ArrowArray::empty());
        std::ptr::write(schema_ptr, FFI_ArrowSchema::empty());
        (array, schema)
    };
    // SAFETY: the FFI structs were produced by a conforming Arrow C Data Interface exporter
    // (pyarrow), and we take ownership of them above (blanking the sources).
    let data = unsafe { from_ffi(ffi_array, &ffi_schema) }.map_err(arrow_err)?;
    Ok(arrow_array::make_array(data))
}

/// The **centralized struct schema** — a name, nullability, [`Headers`](crate::headers) metadata, and
/// an ordered list of child fields (each a `Field` or nested `StructField`).
#[pyclass(module = "yggdryl.types")]
#[derive(Clone)]
pub struct StructField {
    pub(crate) inner: CoreStructField,
}

#[pymethods]
impl StructField {
    /// A struct schema from a name, its ordered child fields, and its nullability (default `True`).
    #[new]
    #[pyo3(signature = (name, fields, nullable = true))]
    fn new(name: &str, fields: &Bound<'_, PyAny>, nullable: bool) -> PyResult<Self> {
        let mut children = Vec::new();
        for field in fields.iter()? {
            children.push(extract_any_field(&field?)?);
        }
        Ok(Self {
            inner: CoreStructField::new(name, children, nullable),
        })
    }

    /// The struct's name.
    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    /// Whether the struct column admits nulls.
    #[getter]
    fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// The element type's name (`"struct"`).
    #[getter]
    fn type_name(&self) -> &'static str {
        "struct"
    }

    /// This schema's [`DataType`].
    #[getter]
    fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Struct)
    }

    /// The number of child fields.
    #[getter]
    fn num_fields(&self) -> usize {
        self.inner.num_fields()
    }

    /// The child field at `index` as a `Field` / `StructField`; raises `IndexError` out of range.
    fn field(&self, py: Python<'_>, index: usize) -> PyResult<PyObject> {
        match self.inner.field(index) {
            Some(field) => rewrap_field(field, py),
            None => Err(PyIndexError::new_err("StructField index out of range")),
        }
    }

    /// The child field named `name`, or `None`.
    fn field_named(&self, py: Python<'_>, name: &str) -> PyResult<Option<PyObject>> {
        match self.inner.field_named(name) {
            Some(field) => rewrap_field(field, py).map(Some),
            None => Ok(None),
        }
    }

    /// The 0-based index of the child field named `name`, or `None`.
    fn index_of(&self, name: &str) -> Option<usize> {
        self.inner.index_of(name)
    }

    /// The child fields, in order, as a list of `Field` / `StructField`.
    fn fields(&self, py: Python<'_>) -> PyResult<Vec<PyObject>> {
        self.inner
            .fields()
            .iter()
            .map(|field| rewrap_field(field, py))
            .collect()
    }

    /// A copy of the struct's metadata [`Headers`](crate::headers).
    #[getter]
    fn metadata(&self) -> crate::headers::Headers {
        crate::headers::Headers {
            inner: self.inner.metadata().clone(),
        }
    }

    // ---- ergonomic immutable updates ---------------------------------------------------

    /// A fresh schema renamed to `name`.
    fn with_name(&self, name: &str) -> Self {
        Self {
            inner: self.inner.with_name(name),
        }
    }

    /// A fresh schema with `nullable` set.
    fn with_nullable(&self, nullable: bool) -> Self {
        Self {
            inner: self.inner.with_nullable(nullable),
        }
    }

    /// A fresh schema with one more child field appended.
    fn with_field(&self, field: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.with_field(extract_any_field(field)?),
        })
    }

    /// A fresh schema with the given metadata (`Headers` or `dict[str, str]`) attached.
    fn with_metadata(&self, metadata: &Bound<'_, PyAny>) -> PyResult<Self> {
        let meta = crate::headers::Headers::from_py(Some(metadata))?;
        Ok(Self {
            inner: self.inner.with_metadata(meta),
        })
    }

    /// A fresh schema with one extra `key = value` metadata entry.
    fn with_metadata_entry(&self, key: &str, value: &str) -> Self {
        Self {
            inner: self.inner.with_metadata_entry(key, value),
        }
    }

    /// This schema's canonical bytes (schema tree codec, Arrow-independent).
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.as_any_field().serialize_bytes())
    }

    /// Reconstructs a schema from [`serialize_bytes`](Self::serialize_bytes).
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        let field = AnyField::deserialize_bytes(bytes).map_err(io_err)?;
        CoreStructField::from_any_field(field)
            .map(|inner| Self { inner })
            .ok_or_else(|| PyValueError::new_err("the bytes did not decode to a struct field"))
    }

    fn __len__(&self) -> usize {
        self.inner.num_fields()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    /// An explicit copy.
    fn copy(&self) -> Self {
        self.clone()
    }
    fn __copy__(&self) -> Self {
        self.clone()
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Pickles through `deserialize_bytes`.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<StructField>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.as_any_field().serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!(
            "StructField(name={:?}, num_fields={}, nullable={})",
            self.inner.name(),
            self.inner.num_fields(),
            self.inner.nullable()
        )
    }
}

/// A **nullable struct column** — one child column per field (all the same length), an ordered
/// schema, and an optional top-level validity mask (a null struct row).
#[pyclass(module = "yggdryl.types")]
#[derive(Clone)]
pub struct StructSerie {
    pub(crate) inner: CoreStructSerie,
}

#[pymethods]
impl StructSerie {
    /// A struct column from `(name, column)` pairs — each `column` is a yggdryl `Serie`. The schema
    /// is inferred from each column's type. Raises `ValueError` if the columns differ in length.
    #[new]
    fn new(columns: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut names = Vec::new();
        let mut cols = Vec::new();
        for pair in columns.iter()? {
            let pair = pair?;
            let (name, column): (String, Bound<'_, PyAny>) = pair.extract()?;
            names.push(name);
            cols.push(extract_column(&column)?);
        }
        let named: Vec<(&str, Box<dyn AnySerie>)> =
            names.iter().map(String::as_str).zip(cols).collect();
        CoreStructSerie::from_named(named)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }

    /// A struct column from `(name, column)` pairs — the self-describing builder mirroring the core's
    /// [`StructSerie::from_series`](yggdryl_core::io::nested::StructSerie::from_series): each `column`
    /// is named in its own header before storage. It is functionally identical to the constructor; it
    /// names the core factory so the three languages read alike. Raises `ValueError` if the columns
    /// differ in length.
    #[staticmethod]
    fn from_series(columns: &Bound<'_, PyAny>) -> PyResult<Self> {
        let mut named = Vec::new();
        for pair in columns.iter()? {
            let pair = pair?;
            let (name, column): (String, Bound<'_, PyAny>) = pair.extract()?;
            named.push(named_column(extract_column(&column)?, &name));
        }
        CoreStructSerie::from_series(named)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }

    /// The number of child columns (fields).
    #[getter]
    fn num_columns(&self) -> usize {
        self.inner.num_columns()
    }

    /// The number of null struct rows.
    #[getter]
    fn null_count(&self) -> usize {
        CoreStructSerie::null_count(&self.inner)
    }

    /// Whether any struct row is null.
    #[getter]
    fn has_nulls(&self) -> bool {
        self.inner.has_nulls()
    }

    /// Whether the column has no rows.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// This column's [`DataType`].
    #[getter]
    fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Struct)
    }

    /// A [`StructField`] naming this struct column (nullability inferred from its null rows).
    fn to_field(&self, name: &str) -> StructField {
        StructField {
            inner: self.inner.to_field(name),
        }
    }

    /// The child field at `index` as a `Field` / `StructField`; raises `IndexError` out of range.
    fn field(&self, py: Python<'_>, index: usize) -> PyResult<PyObject> {
        match self.inner.field(index) {
            Some(field) => rewrap_field(&field, py),
            None => Err(PyIndexError::new_err(
                "StructSerie field index out of range",
            )),
        }
    }

    /// The child column at `index` as its concrete `Serie`; raises `IndexError` out of range.
    fn column(&self, py: Python<'_>, index: usize) -> PyResult<PyObject> {
        match self.inner.column(index) {
            Some(column) => rewrap_column(column, py),
            None => Err(PyIndexError::new_err(
                "StructSerie column index out of range",
            )),
        }
    }

    /// The child column named `name` as its concrete `Serie`, or `None`.
    fn column_named(&self, py: Python<'_>, name: &str) -> PyResult<Option<PyObject>> {
        match self.inner.column_named(name) {
            Some(column) => rewrap_column(column, py).map(Some),
            None => Ok(None),
        }
    }

    /// The column's canonical bytes — a self-contained `[schema][len][validity?][children]` frame,
    /// identical across Rust / Python / Node.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a struct column from [`serialize_bytes`](Self::serialize_bytes).
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        CoreStructSerie::deserialize_bytes(bytes)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    /// The child columns, in order, as a list of concrete `Serie`.
    fn columns(&self, py: Python<'_>) -> PyResult<Vec<PyObject>> {
        (0..self.inner.num_columns())
            .map(|index| rewrap_column(self.inner.column(index).expect("in range"), py))
            .collect()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    /// An explicit copy.
    fn copy(&self) -> Self {
        self.clone()
    }
    fn __copy__(&self) -> Self {
        self.clone()
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Pickles through `deserialize_bytes`.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<StructSerie>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!(
            "StructSerie(len={}, num_columns={}, null_count={})",
            self.inner.len(),
            self.inner.num_columns(),
            CoreStructSerie::null_count(&self.inner)
        )
    }
}

/// The **zero-copy Arrow C Data Interface** (PyCapsule protocol) for a struct column, so pyarrow
/// imports it with no payload copy — `pyarrow.array(table)` / `pyarrow.record_batch(table)` and
/// back via [`from_arrow`](StructSerie::from_arrow).
#[pymethods]
impl StructSerie {
    /// The Arrow C Data Interface **schema** capsule (`"arrow_schema"`).
    fn __arrow_c_schema__(&self, py: Python<'_>) -> PyResult<PyObject> {
        let array = self.inner.to_arrow_array().map_err(io_err)?;
        export_c_schema(py, array.data_type())
    }

    /// The Arrow C Data Interface **(schema, array)** capsule pair, exported zero-copy. The
    /// `requested_schema` hint is accepted for protocol compatibility and ignored (this column
    /// always exports its native schema).
    #[pyo3(signature = (requested_schema = None))]
    fn __arrow_c_array__(
        &self,
        py: Python<'_>,
        requested_schema: Option<PyObject>,
    ) -> PyResult<(PyObject, PyObject)> {
        let _ = requested_schema;
        export_c_array(
            py,
            std::sync::Arc::new(self.inner.to_arrow_array().map_err(io_err)?),
        )
    }

    /// Imports any object exposing the Arrow C Data Interface (a pyarrow `StructArray` /
    /// `RecordBatch`) into a struct column, zero-copy — the inverse of `__arrow_c_array__`.
    #[staticmethod]
    fn from_arrow(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        let array = import_c_array(obj)?;
        let struct_array = array
            .as_any()
            .downcast_ref::<arrow_array::StructArray>()
            .ok_or_else(|| PyValueError::new_err("the imported Arrow array is not a struct"))?;
        let nullable = struct_array.null_count() > 0;
        let field = arrow_schema::Field::new("", struct_array.data_type().clone(), nullable);
        CoreStructSerie::from_arrow_array(struct_array, &field)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }
}

// =====================================================================================
// List family: ListField (the centralized list schema) + ListSerie (a nullable list column).
// =====================================================================================

/// The **centralized list schema** — a name, nullability, [`Headers`](crate::headers) metadata, and a
/// single element (item) field (itself a `Field` / `StructField` / `ListField` / `MapField`).
#[pyclass(module = "yggdryl.types")]
#[derive(Clone)]
pub struct ListField {
    pub(crate) inner: CoreListField,
}

#[pymethods]
impl ListField {
    /// A list schema from a name, its element (item) field, and its nullability (default `True`).
    #[new]
    #[pyo3(signature = (name, item, nullable = true))]
    fn new(name: &str, item: &Bound<'_, PyAny>, nullable: bool) -> PyResult<Self> {
        Ok(Self {
            inner: CoreListField::new(name, extract_any_field(item)?, nullable),
        })
    }

    /// The list's name.
    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    /// Whether the list column admits nulls.
    #[getter]
    fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// The element type's name (`"list"`).
    #[getter]
    fn type_name(&self) -> &'static str {
        "list"
    }

    /// This schema's [`DataType`].
    #[getter]
    fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::List)
    }

    /// The element (item) field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[getter]
    fn item(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_field(self.inner.item(), py)
    }

    /// A copy of the list's metadata [`Headers`](crate::headers).
    #[getter]
    fn metadata(&self) -> crate::headers::Headers {
        crate::headers::Headers {
            inner: self.inner.metadata().clone(),
        }
    }

    // ---- ergonomic immutable updates ---------------------------------------------------

    /// A fresh schema renamed to `name`.
    fn with_name(&self, name: &str) -> Self {
        Self {
            inner: self.inner.with_name(name),
        }
    }

    /// A fresh schema with `nullable` set.
    fn with_nullable(&self, nullable: bool) -> Self {
        Self {
            inner: self.inner.with_nullable(nullable),
        }
    }

    /// A fresh schema with a new element (item) field.
    fn with_item(&self, item: &Bound<'_, PyAny>) -> PyResult<Self> {
        Ok(Self {
            inner: self.inner.with_item(extract_any_field(item)?),
        })
    }

    /// A fresh schema with the given metadata (`Headers` or `dict[str, str]`) attached.
    fn with_metadata(&self, metadata: &Bound<'_, PyAny>) -> PyResult<Self> {
        let meta = crate::headers::Headers::from_py(Some(metadata))?;
        Ok(Self {
            inner: self.inner.with_metadata(meta),
        })
    }

    /// A fresh schema with one extra `key = value` metadata entry.
    fn with_metadata_entry(&self, key: &str, value: &str) -> Self {
        Self {
            inner: self.inner.with_metadata_entry(key, value),
        }
    }

    /// This schema's canonical bytes (schema tree codec, Arrow-independent).
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.as_any_field().serialize_bytes())
    }

    /// Reconstructs a schema from [`serialize_bytes`](Self::serialize_bytes).
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        let field = AnyField::deserialize_bytes(bytes).map_err(io_err)?;
        CoreListField::from_any_field(field)
            .map(|inner| Self { inner })
            .ok_or_else(|| PyValueError::new_err("the bytes did not decode to a list field"))
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    /// An explicit copy.
    fn copy(&self) -> Self {
        self.clone()
    }
    fn __copy__(&self) -> Self {
        self.clone()
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Pickles through `deserialize_bytes`.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<ListField>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.as_any_field().serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!(
            "ListField(name={:?}, nullable={})",
            self.inner.name(),
            self.inner.nullable()
        )
    }
}

/// A **nullable list column** — `i32` offsets over one flattened child column (itself any yggdryl
/// `Serie`), plus an optional top-level validity mask. Row `i` is the child sub-range
/// `child[offsets[i] .. offsets[i + 1]]`.
#[pyclass(module = "yggdryl.types")]
#[derive(Clone)]
pub struct ListSerie {
    pub(crate) inner: CoreListSerie,
}

#[pymethods]
impl ListSerie {
    /// A list column from a flattened child `item` column, `offsets` (`len + 1` entries into the
    /// child), an optional per-row `present` mask (`present[i] == False` marks row `i` a null list),
    /// and the item field name (default `"item"`). Raises `ValueError` on invalid offsets.
    #[new]
    #[pyo3(signature = (item, offsets, present = None, item_name = "item"))]
    fn new(
        item: &Bound<'_, PyAny>,
        offsets: Vec<i32>,
        present: Option<Vec<bool>>,
        item_name: &str,
    ) -> PyResult<Self> {
        let items = named_column(extract_column(item)?, item_name);
        CoreListSerie::from_values(items, &offsets, present.as_deref())
            .map(|inner| Self { inner })
            .map_err(io_err)
    }

    /// The number of null list rows.
    #[getter]
    fn null_count(&self) -> usize {
        self.inner.null_count()
    }

    /// Whether any list row is null.
    #[getter]
    fn has_nulls(&self) -> bool {
        self.inner.has_nulls()
    }

    /// Whether the column has no rows.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// This column's [`DataType`].
    #[getter]
    fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::List)
    }

    /// The flattened child column, as its concrete `Serie`.
    #[getter]
    fn values(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_column(self.inner.values(), py)
    }

    /// The row offsets (`len + 1` entries into the flattened child).
    #[getter]
    fn offsets(&self) -> Vec<i32> {
        self.inner.offsets().to_vec()
    }

    /// The element (item) field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[getter]
    fn item_field(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_field(&self.inner.item_field(), py)
    }

    /// The element at `index` as its element sub-`Serie`, or `None` if the row is null; raises
    /// `IndexError` out of range. The single-element logical getter, matching the leaf `Serie.get`.
    fn get(&self, py: Python<'_>, index: usize) -> PyResult<Option<PyObject>> {
        if index >= self.inner.len() {
            return Err(PyIndexError::new_err("ListSerie index out of range"));
        }
        let scalar = self.inner.get_scalar(index);
        if scalar.is_null() {
            return Ok(None);
        }
        rewrap_column(scalar.items(), py).map(Some)
    }

    /// A [`ListField`] naming this list column (nullability inferred from its null rows).
    fn to_field(&self, name: &str) -> ListField {
        ListField {
            inner: self.inner.to_field(name),
        }
    }

    /// The column's canonical bytes — a self-contained `[schema][len][validity?][offsets][child]`
    /// frame, identical across Rust / Python / Node.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a list column from [`serialize_bytes`](Self::serialize_bytes).
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        CoreListSerie::deserialize_bytes(bytes)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    /// An explicit copy.
    fn copy(&self) -> Self {
        self.clone()
    }
    fn __copy__(&self) -> Self {
        self.clone()
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Pickles through `deserialize_bytes`.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<ListSerie>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!(
            "ListSerie(len={}, null_count={})",
            self.inner.len(),
            self.inner.null_count()
        )
    }
}

/// The **zero-copy Arrow C Data Interface** (PyCapsule protocol) for a list column, so pyarrow imports
/// it with no payload copy — `pyarrow.array(list_serie)` → a `ListArray` and back via
/// [`from_arrow`](ListSerie::from_arrow).
#[pymethods]
impl ListSerie {
    /// The Arrow C Data Interface **schema** capsule (`"arrow_schema"`).
    fn __arrow_c_schema__(&self, py: Python<'_>) -> PyResult<PyObject> {
        let array = self.inner.to_arrow_array().map_err(io_err)?;
        export_c_schema(py, array.data_type())
    }

    /// The Arrow C Data Interface **(schema, array)** capsule pair, exported zero-copy. The
    /// `requested_schema` hint is accepted for protocol compatibility and ignored.
    #[pyo3(signature = (requested_schema = None))]
    fn __arrow_c_array__(
        &self,
        py: Python<'_>,
        requested_schema: Option<PyObject>,
    ) -> PyResult<(PyObject, PyObject)> {
        let _ = requested_schema;
        export_c_array(
            py,
            std::sync::Arc::new(self.inner.to_arrow_array().map_err(io_err)?),
        )
    }

    /// Imports any object exposing the Arrow C Data Interface (a pyarrow `ListArray`) into a list
    /// column, zero-copy — the inverse of `__arrow_c_array__`.
    #[staticmethod]
    fn from_arrow(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        let array = import_c_array(obj)?;
        let nullable = array.null_count() > 0;
        let field = arrow_schema::Field::new("", array.data_type().clone(), nullable);
        CoreListSerie::from_arrow_array(array.as_ref(), &field)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }
}

// =====================================================================================
// Map family: MapField (the centralized map schema) + MapSerie (a nullable map column).
// =====================================================================================

/// The **centralized map schema** — a name, nullability, [`Headers`](crate::headers) metadata, a
/// `keys_sorted` flag, and the `key` / `value` fields (each a `Field` / `StructField` / `ListField` /
/// `MapField`).
#[pyclass(module = "yggdryl.types")]
#[derive(Clone)]
pub struct MapField {
    pub(crate) inner: CoreMapField,
}

#[pymethods]
impl MapField {
    /// A map schema from a name, its `key` and `value` fields, its nullability (default `True`), and
    /// whether the entries are sorted by key (default `False`).
    #[new]
    #[pyo3(signature = (name, key, value, nullable = true, keys_sorted = false))]
    fn new(
        name: &str,
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
        nullable: bool,
        keys_sorted: bool,
    ) -> PyResult<Self> {
        Ok(Self {
            inner: CoreMapField::new(
                name,
                extract_any_field(key)?,
                extract_any_field(value)?,
                nullable,
                keys_sorted,
            ),
        })
    }

    /// The map's name.
    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    /// Whether the map column admits nulls.
    #[getter]
    fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// The element type's name (`"map"`).
    #[getter]
    fn type_name(&self) -> &'static str {
        "map"
    }

    /// This schema's [`DataType`].
    #[getter]
    fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Map)
    }

    /// The key field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[getter]
    fn key(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_field(self.inner.key(), py)
    }

    /// The value field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[getter]
    fn value(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_field(self.inner.value(), py)
    }

    /// Whether the entries are sorted by key.
    #[getter]
    fn keys_sorted(&self) -> bool {
        self.inner.keys_sorted()
    }

    /// A copy of the map's metadata [`Headers`](crate::headers).
    #[getter]
    fn metadata(&self) -> crate::headers::Headers {
        crate::headers::Headers {
            inner: self.inner.metadata().clone(),
        }
    }

    // ---- ergonomic immutable updates ---------------------------------------------------

    /// A fresh schema renamed to `name`.
    fn with_name(&self, name: &str) -> Self {
        Self {
            inner: self.inner.with_name(name),
        }
    }

    /// A fresh schema with `nullable` set.
    fn with_nullable(&self, nullable: bool) -> Self {
        Self {
            inner: self.inner.with_nullable(nullable),
        }
    }

    /// A fresh schema with the `keys_sorted` flag set.
    fn with_keys_sorted(&self, keys_sorted: bool) -> Self {
        Self {
            inner: self.inner.with_keys_sorted(keys_sorted),
        }
    }

    /// A fresh schema with the given metadata (`Headers` or `dict[str, str]`) attached.
    fn with_metadata(&self, metadata: &Bound<'_, PyAny>) -> PyResult<Self> {
        let meta = crate::headers::Headers::from_py(Some(metadata))?;
        Ok(Self {
            inner: self.inner.with_metadata(meta),
        })
    }

    /// A fresh schema with one extra `key = value` metadata entry.
    fn with_metadata_entry(&self, key: &str, value: &str) -> Self {
        Self {
            inner: self.inner.with_metadata_entry(key, value),
        }
    }

    /// This schema's canonical bytes (schema tree codec, Arrow-independent).
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.as_any_field().serialize_bytes())
    }

    /// Reconstructs a schema from [`serialize_bytes`](Self::serialize_bytes).
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        let field = AnyField::deserialize_bytes(bytes).map_err(io_err)?;
        CoreMapField::from_any_field(field)
            .map(|inner| Self { inner })
            .ok_or_else(|| PyValueError::new_err("the bytes did not decode to a map field"))
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    /// An explicit copy.
    fn copy(&self) -> Self {
        self.clone()
    }
    fn __copy__(&self) -> Self {
        self.clone()
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Pickles through `deserialize_bytes`.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<MapField>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.as_any_field().serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!(
            "MapField(name={:?}, nullable={}, keys_sorted={})",
            self.inner.name(),
            self.inner.nullable(),
            self.inner.keys_sorted()
        )
    }
}

/// A **nullable map column** — the optimized alias of `List<Struct<{key, value}>>`: `i32` offsets over
/// a flattened `key` column and `value` column, plus an optional top-level validity mask and a
/// `keys_sorted` flag. Row `i` is the entries `key[j] -> value[j]` for `j` in `[offsets[i],
/// offsets[i + 1])`.
#[pyclass(module = "yggdryl.types")]
#[derive(Clone)]
pub struct MapSerie {
    pub(crate) inner: CoreMapSerie,
}

#[pymethods]
impl MapSerie {
    /// A map column from a flattened `keys` column, a `values` column, `offsets` (`len + 1` entries
    /// into the entries), an optional per-row `present` mask, whether the entries are sorted by key,
    /// and the key/value field names (default `"key"` / `"value"`). A map key is never null, so `keys`
    /// must not contain nulls. Raises `ValueError` on a null key, invalid offsets, or length mismatch.
    #[new]
    #[pyo3(signature = (keys, values, offsets, present = None, keys_sorted = false, key_name = "key", value_name = "value"))]
    fn new(
        keys: &Bound<'_, PyAny>,
        values: &Bound<'_, PyAny>,
        offsets: Vec<i32>,
        present: Option<Vec<bool>>,
        keys_sorted: bool,
        key_name: &str,
        value_name: &str,
    ) -> PyResult<Self> {
        let keys = named_column(extract_column(keys)?, key_name);
        let values = named_column(extract_column(values)?, value_name);
        CoreMapSerie::from_entries(keys, values, &offsets, present.as_deref(), keys_sorted)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }

    /// The number of null map rows.
    #[getter]
    fn null_count(&self) -> usize {
        self.inner.null_count()
    }

    /// Whether any map row is null.
    #[getter]
    fn has_nulls(&self) -> bool {
        self.inner.has_nulls()
    }

    /// Whether the column has no rows.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Whether the entries are sorted by key.
    #[getter]
    fn keys_sorted(&self) -> bool {
        self.inner.keys_sorted()
    }

    /// This column's [`DataType`].
    #[getter]
    fn data_type(&self) -> DataType {
        DataType::of(DataTypeId::Map)
    }

    /// The flattened key column, as its concrete `Serie`.
    #[getter]
    fn keys(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_column(self.inner.keys(), py)
    }

    /// The flattened value column, as its concrete `Serie`.
    #[getter]
    fn values(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_column(self.inner.values(), py)
    }

    /// The key field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[getter]
    fn key_field(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_field(&self.inner.key_field(), py)
    }

    /// The value field as a `Field` / `StructField` / `ListField` / `MapField`.
    #[getter]
    fn value_field(&self, py: Python<'_>) -> PyResult<PyObject> {
        rewrap_field(&self.inner.value_field(), py)
    }

    /// The row offsets (`len + 1` entries into the flattened entries).
    #[getter]
    fn offsets(&self) -> Vec<i32> {
        self.inner.offsets().to_vec()
    }

    /// The value mapped to `key` in row `row`, or `None` if the row is null / out of range or the key
    /// is absent. The `key` is a single-element yggdryl `Serie` of the key type (its first element is
    /// the probe); the result crosses as a one-element `Serie` of the value type. Delegates the lookup
    /// to the core [`MapSerie::get_value`](yggdryl_core::io::nested::MapSerie::get_value).
    fn get_value(
        &self,
        py: Python<'_>,
        row: usize,
        key: &Bound<'_, PyAny>,
    ) -> PyResult<Option<PyObject>> {
        let probe_col = extract_column(key)?;
        if probe_col.is_empty() {
            return Err(PyValueError::new_err(
                "get_value needs a non-empty single-element Serie for the key (its first element is \
                 the probe)",
            ));
        }
        let probe = probe_col.value(0);
        match self.inner.get_value(row, &probe) {
            None => Ok(None),
            // The core returns the matched value; locate it in the row's value range so the value
            // column can be sliced and rewrapped uniformly (any value type) into a one-element Serie.
            Some(found) => {
                let (start, end) = self.inner.value_range(row).unwrap_or((0, 0));
                let values = self.inner.values();
                match (start..end).find(|&index| values.value(index) == found) {
                    Some(index) => rewrap_column(values.slice(index, 1).as_ref(), py).map(Some),
                    None => Ok(None),
                }
            }
        }
    }

    /// The element at `index` as its `key -> value` entries `StructSerie` (columns `[keys, values]`),
    /// or `None` if the row is null; raises `IndexError` out of range. The single-element logical
    /// getter, matching the leaf `Serie.get`.
    fn get(&self, py: Python<'_>, index: usize) -> PyResult<Option<PyObject>> {
        if index >= self.inner.len() {
            return Err(PyIndexError::new_err("MapSerie index out of range"));
        }
        let scalar = self.inner.get_scalar(index);
        if scalar.is_null() {
            return Ok(None);
        }
        rewrap_column(scalar.entries(), py).map(Some)
    }

    /// A [`MapField`] naming this map column (nullability inferred from its null rows).
    fn to_field(&self, name: &str) -> MapField {
        MapField {
            inner: self.inner.to_field(name),
        }
    }

    /// The column's canonical bytes — a self-contained `[schema][len][validity?][offsets][entries]`
    /// frame, identical across Rust / Python / Node.
    fn serialize_bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.serialize_bytes())
    }

    /// Reconstructs a map column from [`serialize_bytes`](Self::serialize_bytes).
    #[staticmethod]
    fn deserialize_bytes(bytes: &[u8]) -> PyResult<Self> {
        CoreMapSerie::deserialize_bytes(bytes)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    /// An explicit copy.
    fn copy(&self) -> Self {
        self.clone()
    }
    fn __copy__(&self) -> Self {
        self.clone()
    }
    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    /// Pickles through `deserialize_bytes`.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (Py<PyAny>,))> {
        let ctor = py
            .get_type_bound::<MapSerie>()
            .getattr("deserialize_bytes")?
            .unbind();
        let state = PyBytes::new_bound(py, &self.inner.serialize_bytes())
            .into_any()
            .unbind();
        Ok((ctor, (state,)))
    }

    fn __repr__(&self) -> String {
        format!(
            "MapSerie(len={}, null_count={}, keys_sorted={})",
            self.inner.len(),
            self.inner.null_count(),
            self.inner.keys_sorted()
        )
    }
}

/// The **zero-copy Arrow C Data Interface** (PyCapsule protocol) for a map column, so pyarrow imports
/// it with no payload copy — `pyarrow.array(map_serie)` → a `MapArray` and back via
/// [`from_arrow`](MapSerie::from_arrow).
#[pymethods]
impl MapSerie {
    /// The Arrow C Data Interface **schema** capsule (`"arrow_schema"`).
    fn __arrow_c_schema__(&self, py: Python<'_>) -> PyResult<PyObject> {
        let array = self.inner.to_arrow_array().map_err(io_err)?;
        export_c_schema(py, array.data_type())
    }

    /// The Arrow C Data Interface **(schema, array)** capsule pair, exported zero-copy. The
    /// `requested_schema` hint is accepted for protocol compatibility and ignored.
    #[pyo3(signature = (requested_schema = None))]
    fn __arrow_c_array__(
        &self,
        py: Python<'_>,
        requested_schema: Option<PyObject>,
    ) -> PyResult<(PyObject, PyObject)> {
        let _ = requested_schema;
        export_c_array(
            py,
            std::sync::Arc::new(self.inner.to_arrow_array().map_err(io_err)?),
        )
    }

    /// Imports any object exposing the Arrow C Data Interface (a pyarrow `MapArray`) into a map
    /// column, zero-copy — the inverse of `__arrow_c_array__`.
    #[staticmethod]
    fn from_arrow(obj: &Bound<'_, PyAny>) -> PyResult<Self> {
        let array = import_c_array(obj)?;
        let nullable = array.null_count() > 0;
        let field = arrow_schema::Field::new("", array.data_type().clone(), nullable);
        CoreMapSerie::from_arrow_array(array.as_ref(), &field)
            .map(|inner| Self { inner })
            .map_err(io_err)
    }
}

// ---- per-family int-row read (the `row` each `__getitem__` int branch calls) ----------------

impl StructSerie {
    /// The `index`-th struct row as a `{column_name: native-or-None}` dict — or `None` if the row is
    /// a **null struct** (negative-index aware); raises `IndexError` out of range. A cell that is
    /// itself a struct nests as a dict (see [`cell_to_py`]).
    fn row(&self, py: Python<'_>, index: isize) -> PyResult<PyObject> {
        let len = self.inner.len() as isize;
        let resolved = if index < 0 { index + len } else { index };
        if resolved < 0 || resolved >= len {
            return Err(PyIndexError::new_err("StructSerie row index out of range"));
        }
        cell_to_py(&self.inner as &dyn AnySerie, resolved as usize, py)
    }
}

impl ListSerie {
    /// The `index`-th list row as its element sub-`Serie`, or `None` if the row is null
    /// (negative-index aware) — the `s[i]` twin of `get`; raises `IndexError` out of range.
    fn row(&self, py: Python<'_>, index: isize) -> PyResult<PyObject> {
        let len = self.inner.len() as isize;
        let resolved = if index < 0 { index + len } else { index };
        if resolved < 0 || resolved >= len {
            return Err(PyIndexError::new_err("ListSerie row index out of range"));
        }
        let scalar = self.inner.get_scalar(resolved as usize);
        if scalar.is_null() {
            return Ok(py.None());
        }
        rewrap_column(scalar.items(), py)
    }
}

impl MapSerie {
    /// The `index`-th map row as its `key -> value` entries `StructSerie`, or `None` if the row is
    /// null (negative-index aware) — the `s[i]` twin of `get`; raises `IndexError` out of range.
    fn row(&self, py: Python<'_>, index: isize) -> PyResult<PyObject> {
        let len = self.inner.len() as isize;
        let resolved = if index < 0 { index + len } else { index };
        if resolved < 0 || resolved >= len {
            return Err(PyIndexError::new_err("MapSerie row index out of range"));
        }
        let scalar = self.inner.get_scalar(resolved as usize);
        if scalar.is_null() {
            return Ok(py.None());
        }
        rewrap_column(scalar.entries(), py)
    }
}

// The shared deep-navigation dunders + named methods, one block per family.
nested_navigation!(StructSerie);
nested_navigation!(ListSerie);
nested_navigation!(MapSerie);

#[pymethods]
impl StructSerie {
    /// `name in s` — whether the struct has a column named `name` (dict-like membership).
    fn __contains__(&self, name: &Bound<'_, PyAny>) -> bool {
        name.extract::<String>()
            .is_ok_and(|name| self.inner.column_named(&name).is_some())
    }
}

// =====================================================================================
// Generic inference factory — `yggdryl.types.column(values, dtype=None)`. A thin inference over the
// existing typed `Serie` constructors: it picks a leaf column type from the Python values (or an
// explicit `dtype`), then builds the matching `Serie` and returns its wrapper.
// =====================================================================================

/// The leaf families a list of Python values may contain, tallied while scanning.
#[derive(Default)]
struct Inferred {
    saw_int: bool,
    saw_float: bool,
    saw_str: bool,
    saw_bytes: bool,
    int_overflow: bool,
    min: i128,
    max: i128,
}

impl Inferred {
    /// Folds one integer value into the running `[min, max]` (and marks an int was seen).
    fn record_int(&mut self, value: i128) {
        if self.saw_int {
            self.min = self.min.min(value);
            self.max = self.max.max(value);
        } else {
            self.min = value;
            self.max = value;
        }
        self.saw_int = true;
    }
}

/// The smallest **signed** integer type that holds `[min, max]` (widening to `i128` at the top).
fn sized_signed_int(min: i128, max: i128) -> DataTypeId {
    if min >= i8::MIN as i128 && max <= i8::MAX as i128 {
        DataTypeId::I8
    } else if min >= i16::MIN as i128 && max <= i16::MAX as i128 {
        DataTypeId::I16
    } else if min >= i32::MIN as i128 && max <= i32::MAX as i128 {
        DataTypeId::I32
    } else if min >= i64::MIN as i128 && max <= i64::MAX as i128 {
        DataTypeId::I64
    } else {
        DataTypeId::I128
    }
}

/// Scans `values` and infers one leaf column [`DataTypeId`]: all-int → the smallest signed int that
/// holds them (`i64` when the list is empty / all-null); any float → `f64`; str → `utf8`; bytes →
/// `binary`; a `None` is a nullable slot. A mix that shares no leaf type is a guided error naming
/// the offending families.
fn infer_column_id(values: &Bound<'_, PyAny>) -> PyResult<DataTypeId> {
    let mut info = Inferred::default();
    for item in values.iter()? {
        let item = item?;
        if item.is_none() {
            continue;
        }
        // A Python `bool` is an `int` subclass; treat it as the integer 0 / 1.
        if item.is_instance_of::<PyBool>() {
            info.record_int(if item.extract::<bool>()? { 1 } else { 0 });
        } else if item.is_instance_of::<PyInt>() {
            match item.extract::<i128>() {
                Ok(value) => info.record_int(value),
                Err(_) => {
                    info.saw_int = true;
                    info.int_overflow = true;
                }
            }
        } else if item.is_instance_of::<PyFloat>() {
            info.saw_float = true;
        } else if item.is_instance_of::<PyString>() {
            info.saw_str = true;
        } else if item.is_instance_of::<PyBytes>() {
            info.saw_bytes = true;
        } else {
            return Err(PyValueError::new_err(
                "column() cannot infer a type from this value; the supported element types are int, \
                 float, str, bytes, and None — pass an explicit dtype= (a DataType or a name like \
                 \"i64\") for anything else",
            ));
        }
    }
    resolve_inferred(&info)
}

/// Resolves the tallied families to one column type, or a guided ambiguity error.
fn resolve_inferred(info: &Inferred) -> PyResult<DataTypeId> {
    let numeric = info.saw_int || info.saw_float;
    match (numeric, info.saw_str, info.saw_bytes) {
        // All-null or empty: default to i64 (holds any small integer; nullable via the null slots).
        (false, false, false) => Ok(DataTypeId::I64),
        (true, false, false) => {
            if info.saw_float {
                Ok(DataTypeId::F64)
            } else if info.int_overflow {
                Err(PyValueError::new_err(
                    "column() cannot fit these integer values in any built-in integer type (they \
                     exceed i128); pass an explicit dtype= (e.g. \"utf8\") or split the data",
                ))
            } else {
                Ok(sized_signed_int(info.min, info.max))
            }
        }
        (false, true, false) => Ok(DataTypeId::Utf8),
        (false, false, true) => Ok(DataTypeId::Binary),
        _ => {
            let mut families = Vec::new();
            if info.saw_int {
                families.push("int");
            }
            if info.saw_float {
                families.push("float");
            }
            if info.saw_str {
                families.push("str");
            }
            if info.saw_bytes {
                families.push("bytes");
            }
            Err(PyValueError::new_err(format!(
                "column() cannot infer a single column type for a mix of {} values; these do not \
                 share a leaf type — pass an explicit dtype= (a DataType or a name like \"utf8\") to \
                 disambiguate",
                families.join(", ")
            )))
        }
    }
}

/// Resolves an explicit `dtype` (a `DataType` object or a type-name string) to a [`DataTypeId`].
fn resolve_dtype_id(dtype: &Bound<'_, PyAny>) -> PyResult<DataTypeId> {
    let name: String = match dtype.extract::<String>() {
        Ok(name) => name,
        Err(_) => dtype.getattr("name")?.extract()?,
    };
    DataTypeId::from_name(&name)
        .ok_or_else(|| PyValueError::new_err(format!("unknown data type name: {name:?}")))
}

/// Builds the `Serie` wrapper of type `id` from the Python `values` by delegating to that wrapper's
/// own constructor. A type whose column needs extra parameters (decimal / temporal / fixed-size)
/// has no plain-list constructor here, so it is a guided error.
fn build_column(py: Python<'_>, id: DataTypeId, values: &Bound<'_, PyAny>) -> PyResult<PyObject> {
    macro_rules! cls {
        ($W:ty) => {
            py.get_type_bound::<$W>()
        };
    }
    let cls = match id {
        DataTypeId::U8 => cls!(U8Serie),
        DataTypeId::U16 => cls!(U16Serie),
        DataTypeId::U32 => cls!(U32Serie),
        DataTypeId::U64 => cls!(U64Serie),
        DataTypeId::U96 => cls!(U96Serie),
        DataTypeId::U128 => cls!(U128Serie),
        DataTypeId::U256 => cls!(U256Serie),
        DataTypeId::I8 => cls!(I8Serie),
        DataTypeId::I16 => cls!(I16Serie),
        DataTypeId::I32 => cls!(I32Serie),
        DataTypeId::I64 => cls!(I64Serie),
        DataTypeId::I96 => cls!(I96Serie),
        DataTypeId::I128 => cls!(I128Serie),
        DataTypeId::I256 => cls!(I256Serie),
        DataTypeId::F16 => cls!(F16Serie),
        DataTypeId::F32 => cls!(F32Serie),
        DataTypeId::F64 => cls!(F64Serie),
        DataTypeId::Utf8 => cls!(Utf8Serie),
        DataTypeId::Binary => cls!(BinarySerie),
        other => {
            return Err(PyValueError::new_err(format!(
                "column() cannot build a {} column from a plain list; construct its Serie directly \
                 (a decimal / temporal / fixed-size column needs extra parameters like precision, \
                 scale, a unit, or a width)",
                other.name()
            )))
        }
    };
    Ok(cls.call1((values,))?.unbind())
}

/// Builds a column by inferring its leaf type from `values` (or using an explicit `dtype`), then
/// delegating to the matching typed `Serie` constructor — the easy, native-list entry point:
/// `yggdryl.types.column([1, 2, 3])` → an `I8Serie`, `column(["a", "b"])` → a `Utf8Serie`.
#[pyfunction]
#[pyo3(signature = (values, dtype = None))]
fn column(
    py: Python<'_>,
    values: &Bound<'_, PyAny>,
    dtype: Option<&Bound<'_, PyAny>>,
) -> PyResult<PyObject> {
    let id = match dtype {
        Some(dtype) => resolve_dtype_id(dtype)?,
        None => infer_column_id(values)?,
    };
    build_column(py, id, values)
}

/// Adds the nested (`Struct` / `List` / `Map`) field + column classes and the `column` inference
/// factory to the `yggdryl.types` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<StructField>()?;
    module.add_class::<StructSerie>()?;
    module.add_class::<ListField>()?;
    module.add_class::<ListSerie>()?;
    module.add_class::<MapField>()?;
    module.add_class::<MapSerie>()?;
    module.add_function(wrap_pyfunction!(column, module)?)?;
    Ok(())
}
