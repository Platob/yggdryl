//! The **nested typed layer** of the `yggdryl.typed` submodule — the recursive, heterogeneous
//! carriers grown on top of the flat [`Serie`] / [`ByteSerie`] columns.
//!
//! Mirrors `yggdryl_core::typed::nested`: [`StructSerie`] (the "table" — an ordered set of
//! heterogeneous, equal-length child columns), [`ListSerie`] (a variable-length list over a
//! flattened child column), and [`MapSerie`] (an offsets buffer over key / value entries), each
//! reported by its value-typed schema descriptor ([`StructField`] / [`ListField`] / [`MapField`]).
//!
//! ## Erasing across the FFI — [`column_to_py`] / [`column_from_py`]
//!
//! The core keystone is the erased `Column` (a tagged union over every concrete data column). The
//! binding already erases the flat columns into the [`Serie`]'s [`Inner`] and the [`ByteSerie`]'s
//! [`ByteInner`] enums, so a `Column` crosses the boundary through two matched conversions:
//!
//! - [`column_to_py`] moves a core `Column` **into** the matching Python class — a leaf into a
//!   [`Serie`] / [`ByteSerie`], a nested child into a [`StructSerie`] / [`ListSerie`] / [`MapSerie`].
//! - [`column_from_py`] rebuilds a core `Column` **from** a Python column — extracting the wrapper's
//!   inner series and cloning it (a fixed-width [`Serie`] is a cheap `clone`; the byte and nested
//!   carriers, which are not `Clone`, are reconstructed from their public parts).
//!
//! Because the boundary **copies** (the binding cannot hand a live `&mut` into a core sub-series
//! across the FFI), [`StructSerie::column`] returns a *copy* as a fresh column and mutation is
//! [`StructSerie::set_column`] (replace + rebuild) — the binding-appropriate form of the core's deep,
//! in-place `column_by_name_mut` accessor.

// `useless_conversion`: pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type `From`.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::{PyIndexError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict};

use crate::typed::{i256_to_py, ioerr, ByteInner, ByteSerie, Field, Inner, Serie};
use yggdryl_core::io::memory::{Heap, IOBase};
use yggdryl_core::typed::fixedbyte::Int64;
use yggdryl_core::typed::{
    Column, ColumnField, FixedSerie, FixedSizeSerie, ListField as CoreListField,
    ListSerie as CoreListSerie, MapField as CoreMapField, MapSerie as CoreMapSerie, Scalar,
    StructField as CoreStructField, StructSerie as CoreStructSerie, Value, VarLenType, VarSerie,
    VarType,
};

// =====================================================================================
// Column <-> Python column conversions (the crux)
// =====================================================================================

/// Reconstructs a variable-length byte carrier ([`VarSerie`]) that **shares the same encoded bytes**
/// as `s` (cloning only its small offsets / data / validity heaps, and re-applying its column
/// `name`) — the `Clone` stand-in for a carrier that does not derive `Clone`.
fn rebuild_var<T: VarLenType>(s: &VarSerie<T>) -> VarSerie<T> {
    let mut out = VarSerie::from_parts(
        s.offsets().clone(),
        s.data().clone(),
        s.validity().cloned(),
        s.len(),
    );
    if let Some(name) = s.name() {
        out = out.with_name(name);
    }
    out
}

/// Reconstructs a fixed-size byte carrier ([`FixedSizeSerie`]) that **shares the same encoded bytes**
/// as `s` (cloning only its data / validity heaps, carrying its fixed `width`, and re-applying its
/// column `name`).
fn rebuild_fixed_size<T: VarType>(s: &FixedSizeSerie<T>) -> FixedSizeSerie<T> {
    let mut out =
        FixedSizeSerie::from_parts(s.data().clone(), s.validity().cloned(), s.len(), s.width());
    if let Some(name) = s.name() {
        out = out.with_name(name);
    }
    out
}

/// **Deep-clones** a core [`Column`] — a fixed-width leaf clones directly ([`FixedSerie`] is `Clone`);
/// the byte carriers reconstruct from their parts; a nested child recurses through
/// [`clone_struct`] / [`clone_list`] / [`clone_map`]. The recursive workhorse behind
/// [`column_from_py`] on a nested wrapper and behind [`StructSerie::set_column`]'s rebuild.
fn clone_column(col: &Column) -> Column {
    match col {
        Column::Null(n) => Column::null(*n),
        Column::Int8(s) => Column::from(s.clone()),
        Column::UInt8(s) => Column::from(s.clone()),
        Column::Int16(s) => Column::from(s.clone()),
        Column::UInt16(s) => Column::from(s.clone()),
        Column::Int32(s) => Column::from(s.clone()),
        Column::UInt32(s) => Column::from(s.clone()),
        Column::Int64(s) => Column::from(s.clone()),
        Column::UInt64(s) => Column::from(s.clone()),
        Column::Int128(s) => Column::from(s.clone()),
        Column::UInt128(s) => Column::from(s.clone()),
        Column::Float32(s) => Column::from(s.clone()),
        Column::Float64(s) => Column::from(s.clone()),
        Column::Bool(s) => Column::from(s.clone()),
        Column::Decimal32(s) => Column::from(s.clone()),
        Column::Decimal64(s) => Column::from(s.clone()),
        Column::Decimal128(s) => Column::from(s.clone()),
        Column::Decimal256(s) => Column::from(s.clone()),
        Column::Binary(s) => Column::from(rebuild_var(s)),
        Column::LargeBinary(s) => Column::from(rebuild_var(s)),
        Column::Utf8(s) => Column::from(rebuild_var(s)),
        Column::LargeUtf8(s) => Column::from(rebuild_var(s)),
        Column::FixedBinary(s) => Column::from(rebuild_fixed_size(s)),
        Column::FixedUtf8(s) => Column::from(rebuild_fixed_size(s)),
        Column::Struct(s) => Column::from(clone_struct(s)),
        Column::List(s) => Column::from(clone_list(s)),
        Column::Map(s) => Column::from(clone_map(s)),
        // `Column` is `#[non_exhaustive]`; degrade a future variant to its null run.
        _ => Column::null(col.len()),
    }
}

/// Rebuilds a [`CoreStructSerie`] from freshly built `children`, restoring the `template`'s name,
/// row-level validity (rebuilt from its per-row `is_valid`), and free-form metadata. A child whose
/// length does not match the others surfaces the core's guided length-mismatch `ValueError`.
fn struct_from_children(
    children: Vec<Column>,
    template: &CoreStructSerie,
) -> PyResult<CoreStructSerie> {
    let mut out = CoreStructSerie::from_columns(children).map_err(ioerr)?;
    if let Some(name) = template.name() {
        out = out.with_name(name);
    }
    if template.field().nullable() {
        let mut validity = Heap::new();
        for index in 0..template.len() as u64 {
            validity
                .pwrite_bit(index, template.is_valid(index as usize))
                .map_err(ioerr)?;
        }
        out = out.with_validity(Some(validity));
    }
    *out.metadata_mut() = template.metadata().clone();
    Ok(out)
}

/// Deep-clones a [`CoreStructSerie`] (its children cloned recursively, its schema preserved).
fn clone_struct(s: &CoreStructSerie) -> CoreStructSerie {
    let children: Vec<Column> = s.columns().iter().map(clone_column).collect();
    struct_from_children(children, s)
        .expect("cloning a struct preserves its length and validity invariants")
}

/// Deep-clones a [`CoreListSerie`] — the flattened child is cloned and each list's span is replayed
/// through `push` / `push_null`. (A null list built through the binding always has an empty span, so
/// the replay is faithful for every binding-created list; a `from_offsets`-built null list carrying a
/// non-empty span is not reachable from the binding.)
fn clone_list(s: &CoreListSerie) -> CoreListSerie {
    let mut out = CoreListSerie::new(s.name().unwrap_or(""), clone_column(s.values()));
    for index in 0..s.len() {
        if s.is_valid(index) {
            let (start, end) = s.list_at(index).unwrap_or((0, 0));
            out.push(end - start);
        } else {
            out.push_null();
        }
    }
    *out.metadata_mut() = s.metadata().clone();
    out
}

/// Deep-clones a [`CoreMapSerie`] — the flattened key / value columns are cloned and each map's entry
/// span is replayed through `push` / `push_null` (same faithfulness note as [`clone_list`]).
fn clone_map(s: &CoreMapSerie) -> CoreMapSerie {
    let mut out = CoreMapSerie::new(
        s.name().unwrap_or(""),
        clone_column(s.keys()),
        clone_column(s.values()),
    )
    .expect(
        "cloned key/value columns preserve the source's non-null-key and equal-length invariants",
    )
    .with_keys_sorted(s.keys_sorted());
    for index in 0..s.len() {
        if s.is_valid(index) {
            let (start, end) = s.map_at(index).unwrap_or((0, 0));
            out.push(end - start);
        } else {
            out.push_null();
        }
    }
    out
}

/// Renames a core [`Column`] — the child name rides on the concrete carrier's field, so renaming a
/// column means renaming its underlying series. A [`Column::Null`] run carries no field to name.
fn rename_column(col: Column, name: &str) -> Column {
    match col {
        Column::Null(n) => Column::null(n),
        Column::Int8(s) => Column::from(s.with_name(name)),
        Column::UInt8(s) => Column::from(s.with_name(name)),
        Column::Int16(s) => Column::from(s.with_name(name)),
        Column::UInt16(s) => Column::from(s.with_name(name)),
        Column::Int32(s) => Column::from(s.with_name(name)),
        Column::UInt32(s) => Column::from(s.with_name(name)),
        Column::Int64(s) => Column::from(s.with_name(name)),
        Column::UInt64(s) => Column::from(s.with_name(name)),
        Column::Int128(s) => Column::from(s.with_name(name)),
        Column::UInt128(s) => Column::from(s.with_name(name)),
        Column::Float32(s) => Column::from(s.with_name(name)),
        Column::Float64(s) => Column::from(s.with_name(name)),
        Column::Bool(s) => Column::from(s.with_name(name)),
        Column::Decimal32(s) => Column::from(s.with_name(name)),
        Column::Decimal64(s) => Column::from(s.with_name(name)),
        Column::Decimal128(s) => Column::from(s.with_name(name)),
        Column::Decimal256(s) => Column::from(s.with_name(name)),
        Column::Binary(s) => Column::from(s.with_name(name)),
        Column::LargeBinary(s) => Column::from(s.with_name(name)),
        Column::Utf8(s) => Column::from(s.with_name(name)),
        Column::LargeUtf8(s) => Column::from(s.with_name(name)),
        Column::FixedBinary(s) => Column::from(s.with_name(name)),
        Column::FixedUtf8(s) => Column::from(s.with_name(name)),
        Column::Struct(s) => Column::from(s.with_name(name)),
        Column::List(s) => Column::from(s.with_name(name)),
        Column::Map(s) => Column::from(s.with_name(name)),
        other => other,
    }
}

/// A nullable, all-null `i64` [`Serie`] of `n` elements — the Python representation chosen for a
/// bufferless [`Column::Null`] (which carries no element type of its own).
// DESIGN: `Column::Null` has no dtype (`Unknown`), but the binding surfaces every column as a
// materializable `Serie` / `ByteSerie`; a nullable `i64` column of `n` nulls is the least-surprising
// concrete stand-in. A binding-built struct never produces a `Null` child (`push_null` grows the
// typed children), so this path is only reached for a core-built column.
fn null_serie_py(py: Python<'_>, n: usize) -> PyResult<PyObject> {
    let opts: Vec<Option<i64>> = vec![None; n];
    let serie = Serie {
        inner: Inner::I64(FixedSerie::<Int64>::from_options(&opts)),
    };
    Ok(Py::new(py, serie)?.into_any())
}

/// Moves a core [`Column`] **into** the matching Python class — a fixed-width leaf into a [`Serie`],
/// a byte column into a [`ByteSerie`], a nested child into a [`StructSerie`] / [`ListSerie`] /
/// [`MapSerie`], and a bufferless null run into [`null_serie_py`]. The consuming counterpart of
/// [`column_from_py`].
pub(crate) fn column_to_py(py: Python<'_>, col: Column) -> PyResult<PyObject> {
    let obj =
        match col {
            Column::Null(n) => return null_serie_py(py, n),
            Column::Int8(s) => Py::new(
                py,
                Serie {
                    inner: Inner::I8(s),
                },
            )?
            .into_any(),
            Column::UInt8(s) => Py::new(
                py,
                Serie {
                    inner: Inner::U8(s),
                },
            )?
            .into_any(),
            Column::Int16(s) => Py::new(
                py,
                Serie {
                    inner: Inner::I16(s),
                },
            )?
            .into_any(),
            Column::UInt16(s) => Py::new(
                py,
                Serie {
                    inner: Inner::U16(s),
                },
            )?
            .into_any(),
            Column::Int32(s) => Py::new(
                py,
                Serie {
                    inner: Inner::I32(s),
                },
            )?
            .into_any(),
            Column::UInt32(s) => Py::new(
                py,
                Serie {
                    inner: Inner::U32(s),
                },
            )?
            .into_any(),
            Column::Int64(s) => Py::new(
                py,
                Serie {
                    inner: Inner::I64(s),
                },
            )?
            .into_any(),
            Column::UInt64(s) => Py::new(
                py,
                Serie {
                    inner: Inner::U64(s),
                },
            )?
            .into_any(),
            Column::Int128(s) => Py::new(
                py,
                Serie {
                    inner: Inner::I128(s),
                },
            )?
            .into_any(),
            Column::UInt128(s) => Py::new(
                py,
                Serie {
                    inner: Inner::U128(s),
                },
            )?
            .into_any(),
            Column::Float32(s) => Py::new(
                py,
                Serie {
                    inner: Inner::F32(s),
                },
            )?
            .into_any(),
            Column::Float64(s) => Py::new(
                py,
                Serie {
                    inner: Inner::F64(s),
                },
            )?
            .into_any(),
            Column::Bool(s) => Py::new(
                py,
                Serie {
                    inner: Inner::Bool(s),
                },
            )?
            .into_any(),
            Column::Decimal32(s) => Py::new(
                py,
                Serie {
                    inner: Inner::Decimal32(s),
                },
            )?
            .into_any(),
            Column::Decimal64(s) => Py::new(
                py,
                Serie {
                    inner: Inner::Decimal64(s),
                },
            )?
            .into_any(),
            Column::Decimal128(s) => Py::new(
                py,
                Serie {
                    inner: Inner::Decimal128(s),
                },
            )?
            .into_any(),
            Column::Decimal256(s) => Py::new(
                py,
                Serie {
                    inner: Inner::Decimal256(s),
                },
            )?
            .into_any(),
            Column::Binary(s) => Py::new(
                py,
                ByteSerie {
                    inner: ByteInner::Binary(s),
                },
            )?
            .into_any(),
            Column::LargeBinary(s) => Py::new(
                py,
                ByteSerie {
                    inner: ByteInner::LargeBinary(s),
                },
            )?
            .into_any(),
            Column::Utf8(s) => Py::new(
                py,
                ByteSerie {
                    inner: ByteInner::Utf8(s),
                },
            )?
            .into_any(),
            Column::LargeUtf8(s) => Py::new(
                py,
                ByteSerie {
                    inner: ByteInner::LargeUtf8(s),
                },
            )?
            .into_any(),
            Column::FixedBinary(s) => Py::new(
                py,
                ByteSerie {
                    inner: ByteInner::FixedBinary(s),
                },
            )?
            .into_any(),
            Column::FixedUtf8(s) => Py::new(
                py,
                ByteSerie {
                    inner: ByteInner::FixedUtf8(s),
                },
            )?
            .into_any(),
            Column::Struct(s) => Py::new(py, StructSerie { inner: s })?.into_any(),
            Column::List(s) => Py::new(py, ListSerie { inner: s })?.into_any(),
            Column::Map(s) => Py::new(py, MapSerie { inner: s })?.into_any(),
            // `Column` is `#[non_exhaustive]`; a future variant this build does not yet wrap.
            _ => return Err(PyValueError::new_err(
                "unsupported column variant: this build of yggdryl-core added a column kind the \
                 Python binding does not yet wrap — upgrade the binding",
            )),
        };
    Ok(obj)
}

/// Rebuilds a core [`Column`] **from** a Python column — a [`Serie`] / [`ByteSerie`] leaf (cloning
/// its erased inner series) or a nested [`StructSerie`] / [`ListSerie`] / [`MapSerie`] (deep-cloned).
/// The counterpart of [`column_to_py`]; anything else is a guided `TypeError`.
pub(crate) fn column_from_py(obj: &Bound<'_, PyAny>) -> PyResult<Column> {
    if let Ok(serie) = obj.extract::<PyRef<'_, Serie>>() {
        return Ok(column_from_inner(&serie.inner));
    }
    if let Ok(serie) = obj.extract::<PyRef<'_, ByteSerie>>() {
        return Ok(column_from_byte_inner(&serie.inner));
    }
    if let Ok(serie) = obj.extract::<PyRef<'_, StructSerie>>() {
        return Ok(Column::from(clone_struct(&serie.inner)));
    }
    if let Ok(serie) = obj.extract::<PyRef<'_, ListSerie>>() {
        return Ok(Column::from(clone_list(&serie.inner)));
    }
    if let Ok(serie) = obj.extract::<PyRef<'_, MapSerie>>() {
        return Ok(Column::from(clone_map(&serie.inner)));
    }
    Err(PyTypeError::new_err(format!(
        "expected a yggdryl.typed column (Serie / ByteSerie / StructSerie / ListSerie / MapSerie), \
         got {}",
        obj.repr()?
    )))
}

/// Clones a [`Serie`]'s erased [`Inner`] into a core [`Column`] — every fixed-width variant is a
/// cheap `FixedSerie` clone (it shares its encoded bytes).
fn column_from_inner(inner: &Inner) -> Column {
    match inner {
        Inner::I8(s) => Column::from(s.clone()),
        Inner::U8(s) => Column::from(s.clone()),
        Inner::I16(s) => Column::from(s.clone()),
        Inner::U16(s) => Column::from(s.clone()),
        Inner::I32(s) => Column::from(s.clone()),
        Inner::U32(s) => Column::from(s.clone()),
        Inner::I64(s) => Column::from(s.clone()),
        Inner::U64(s) => Column::from(s.clone()),
        Inner::I128(s) => Column::from(s.clone()),
        Inner::U128(s) => Column::from(s.clone()),
        Inner::F32(s) => Column::from(s.clone()),
        Inner::F64(s) => Column::from(s.clone()),
        Inner::Bool(s) => Column::from(s.clone()),
        Inner::Decimal32(s) => Column::from(s.clone()),
        Inner::Decimal64(s) => Column::from(s.clone()),
        Inner::Decimal128(s) => Column::from(s.clone()),
        Inner::Decimal256(s) => Column::from(s.clone()),
    }
}

/// Clones a [`ByteSerie`]'s erased [`ByteInner`] into a core [`Column`] — reconstructing the
/// variable-length / fixed-size carrier from its parts (they do not derive `Clone`).
fn column_from_byte_inner(inner: &ByteInner) -> Column {
    match inner {
        ByteInner::Binary(s) => Column::from(rebuild_var(s)),
        ByteInner::LargeBinary(s) => Column::from(rebuild_var(s)),
        ByteInner::Utf8(s) => Column::from(rebuild_var(s)),
        ByteInner::LargeUtf8(s) => Column::from(rebuild_var(s)),
        ByteInner::FixedBinary(s) => Column::from(rebuild_fixed_size(s)),
        ByteInner::FixedUtf8(s) => Column::from(rebuild_fixed_size(s)),
    }
}

/// Marshals an erased core [`Value`] into a Python object — a native scalar as `int` / `float` /
/// `bool`, a decimal as its raw unscaled `int` (a `Decimal256` beyond `i128` as an arbitrary-precision
/// `int`), a binary as `bytes`, a string as `str`, a struct row / list as a `list`, and a map as a
/// `dict`. Nulls (and out-of-range) become `None`.
fn value_to_py(py: Python<'_>, value: &Value) -> PyResult<PyObject> {
    let obj = match value {
        Value::Null => py.None(),
        Value::Int8(v) => (*v).into_py(py),
        Value::UInt8(v) => (*v).into_py(py),
        Value::Int16(v) => (*v).into_py(py),
        Value::UInt16(v) => (*v).into_py(py),
        Value::Int32(v) => (*v).into_py(py),
        Value::UInt32(v) => (*v).into_py(py),
        Value::Int64(v) => (*v).into_py(py),
        Value::UInt64(v) => (*v).into_py(py),
        Value::Int128(v) => (*v).into_py(py),
        Value::UInt128(v) => (*v).into_py(py),
        Value::Float32(v) => (*v).into_py(py),
        Value::Float64(v) => (*v).into_py(py),
        Value::Bool(v) => (*v).into_py(py),
        Value::Decimal32(v) => (*v).into_py(py),
        Value::Decimal64(v) => (*v).into_py(py),
        Value::Decimal128(v) => (*v).into_py(py),
        Value::Decimal256(v) => i256_to_py(py, *v),
        Value::Binary(v) => PyBytes::new_bound(py, v).into_any().unbind(),
        Value::Utf8(v) => v.to_object(py),
        Value::Row(scalar) => values_to_py(py, scalar.values())?.into_py(py),
        Value::List(scalar) => values_to_py(py, scalar.values())?.into_py(py),
        Value::Map(scalar) => {
            let dict = PyDict::new_bound(py);
            for index in 0..scalar.len() {
                let key = value_to_py(py, &scalar.keys()[index])?;
                let val = value_to_py(py, &scalar.values()[index])?;
                dict.set_item(key, val)?;
            }
            dict.into_any().unbind()
        }
    };
    Ok(obj)
}

/// Marshals a slice of erased [`Value`]s into a `Vec` of Python objects (each via [`value_to_py`]) —
/// the shared body behind a struct row and a list element.
fn values_to_py(py: Python<'_>, values: &[Value]) -> PyResult<Vec<PyObject>> {
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        out.push(value_to_py(py, value)?);
    }
    Ok(out)
}

/// Rebuilds a core [`ColumnField`] from a Python schema descriptor — a leaf [`Field`], a nested
/// [`StructField`], a [`ListField`], or a [`MapField`]. Anything else is a guided `TypeError`.
fn column_field_from_py(obj: &Bound<'_, PyAny>) -> PyResult<ColumnField> {
    if let Ok(field) = obj.extract::<PyRef<'_, Field>>() {
        return Ok(ColumnField::Leaf(field.inner.clone()));
    }
    if let Ok(field) = obj.extract::<PyRef<'_, StructField>>() {
        return Ok(ColumnField::Struct(field.inner.clone()));
    }
    if let Ok(field) = obj.extract::<PyRef<'_, ListField>>() {
        return Ok(ColumnField::List(field.inner.clone()));
    }
    if let Ok(field) = obj.extract::<PyRef<'_, MapField>>() {
        return Ok(ColumnField::Map(field.inner.clone()));
    }
    Err(PyTypeError::new_err(format!(
        "expected a yggdryl.typed field (Field / StructField / ListField / MapField), got {}",
        obj.repr()?
    )))
}

/// Marshals a core [`ColumnField`] into its matching Python schema descriptor — the inverse of
/// [`column_field_from_py`].
fn column_field_to_py(py: Python<'_>, field: ColumnField) -> PyResult<PyObject> {
    let obj =
        match field {
            ColumnField::Leaf(inner) => Py::new(py, Field { inner })?.into_any(),
            ColumnField::Struct(inner) => Py::new(py, StructField { inner })?.into_any(),
            ColumnField::List(inner) => Py::new(py, ListField { inner })?.into_any(),
            ColumnField::Map(inner) => Py::new(py, MapField { inner })?.into_any(),
            _ => return Err(PyValueError::new_err(
                "unsupported column field variant: this build of yggdryl-core added a field kind \
                 the Python binding does not yet wrap — upgrade the binding",
            )),
        };
    Ok(obj)
}

/// Hashes a value-typed core schema descriptor (`StructField` / `ListField` / `MapField`) by its
/// canonical bytes — equal schemas hash equal — for the `__hash__` dunders.
fn hash_value<T: std::hash::Hash>(value: &T) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

// =====================================================================================
// StructField — the struct schema
// =====================================================================================

/// A **struct schema** — an ordered set of child [field descriptors](Field) under a `name`, with a
/// `nullable` flag. Value-typed (`__eq__` / `__hash__`) so it keys a dict and sits in a set. Wraps
/// the core `StructField`.
#[pyclass(module = "yggdryl.typed")]
#[derive(Clone)]
pub struct StructField {
    pub(crate) inner: CoreStructField,
}

#[pymethods]
impl StructField {
    /// A struct schema from its `name` (optional) and `fields` (a list of `Field` / `StructField` /
    /// `ListField` / `MapField` child descriptors, default empty).
    #[new]
    #[pyo3(signature = (name = None, fields = None))]
    fn new(name: Option<String>, fields: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let children = match fields {
            Some(list) => {
                let mut children = Vec::new();
                for item in list.iter()? {
                    children.push(column_field_from_py(&item?)?);
                }
                children
            }
            None => Vec::new(),
        };
        Ok(StructField {
            inner: CoreStructField::new(name.as_deref(), children),
        })
    }

    /// The child field names in order (an unnamed child reports `""`).
    fn names(&self) -> Vec<String> {
        self.inner.names().iter().map(|s| s.to_string()).collect()
    }

    /// The child field descriptor at `index` (a `Field` / `StructField` / `ListField` / `MapField`),
    /// or `None` when `index` is out of range.
    fn field(&self, py: Python<'_>, index: usize) -> PyResult<Option<PyObject>> {
        match self.inner.field(index) {
            Some(field) => Ok(Some(column_field_to_py(py, field.clone())?)),
            None => Ok(None),
        }
    }

    /// The first child field named `name`, or `None` when there is none.
    fn field_by_name(&self, py: Python<'_>, name: &str) -> PyResult<Option<PyObject>> {
        match self.inner.field_by_name(name) {
            Some(field) => Ok(Some(column_field_to_py(py, field.clone())?)),
            None => Ok(None),
        }
    }

    /// The number of child fields.
    fn num_fields(&self) -> usize {
        self.inner.num_fields()
    }

    /// The struct's name, if set.
    fn name(&self) -> Option<String> {
        self.inner.name().map(str::to_string)
    }

    /// Whether the struct admits null rows.
    fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// Sets whether the struct admits null rows **in place**.
    fn set_nullable(&mut self, nullable: bool) {
        self.inner.set_nullable(nullable);
    }

    /// A **fresh** schema with its `nullable` flag set (the clone-with-override front door).
    fn with_nullable(&self, nullable: bool) -> StructField {
        StructField {
            inner: self.inner.clone().with_nullable(nullable),
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        hash_value(&self.inner)
    }

    fn __repr__(&self) -> String {
        format!(
            "StructField(name={:?}, num_fields={}, nullable={})",
            self.inner.name(),
            self.inner.num_fields(),
            if self.inner.nullable() {
                "True"
            } else {
                "False"
            },
        )
    }
}

// =====================================================================================
// ListField — the list schema
// =====================================================================================

/// A **list schema** — the single child `item` field describing every element's type, under a `name`
/// with a `nullable` flag. Value-typed (`__eq__` / `__hash__`). Wraps the core `ListField`.
#[pyclass(module = "yggdryl.typed")]
#[derive(Clone)]
pub struct ListField {
    pub(crate) inner: CoreListField,
}

#[pymethods]
impl ListField {
    /// A list schema from its child `item` field (a `Field` / `StructField` / `ListField` /
    /// `MapField`), `name` (optional), and `nullable` flag (default `False`).
    #[new]
    #[pyo3(signature = (item, name = None, nullable = false))]
    fn new(item: &Bound<'_, PyAny>, name: Option<String>, nullable: bool) -> PyResult<Self> {
        let mut inner = CoreListField::new(name.as_deref(), column_field_from_py(item)?);
        inner.set_nullable(nullable);
        Ok(ListField { inner })
    }

    /// The child **item** field describing every element's type.
    fn item(&self, py: Python<'_>) -> PyResult<PyObject> {
        column_field_to_py(py, self.inner.item().clone())
    }

    /// The list's name, if set.
    fn name(&self) -> Option<String> {
        self.inner.name().map(str::to_string)
    }

    /// Whether the list admits null elements.
    fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// Sets whether the list admits null elements **in place**.
    fn set_nullable(&mut self, nullable: bool) {
        self.inner.set_nullable(nullable);
    }

    /// A **fresh** schema with its `nullable` flag set.
    fn with_nullable(&self, nullable: bool) -> ListField {
        ListField {
            inner: self.inner.clone().with_nullable(nullable),
        }
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        hash_value(&self.inner)
    }

    fn __repr__(&self) -> String {
        format!(
            "ListField(name={:?}, item_dtype='{}', nullable={})",
            self.inner.name(),
            self.inner.item().data_type_id().name(),
            if self.inner.nullable() {
                "True"
            } else {
                "False"
            },
        )
    }
}

// =====================================================================================
// MapField — the map schema
// =====================================================================================

/// A **map schema** — the child `key` and `value` field descriptors under a `name`, with a
/// `nullable` flag and a `keys_sorted` hint. Value-typed (`__eq__` / `__hash__`). Wraps the core
/// `MapField`.
#[pyclass(module = "yggdryl.typed")]
#[derive(Clone)]
pub struct MapField {
    pub(crate) inner: CoreMapField,
}

#[pymethods]
impl MapField {
    /// A map schema from its child `key` and `value` fields, `name` (optional), `nullable` flag
    /// (default `False`), and `keys_sorted` hint (default `False`).
    #[new]
    #[pyo3(signature = (key, value, name = None, nullable = false, keys_sorted = false))]
    fn new(
        key: &Bound<'_, PyAny>,
        value: &Bound<'_, PyAny>,
        name: Option<String>,
        nullable: bool,
        keys_sorted: bool,
    ) -> PyResult<Self> {
        let mut inner = CoreMapField::new(
            name.as_deref(),
            column_field_from_py(key)?,
            column_field_from_py(value)?,
        );
        inner.set_nullable(nullable);
        inner.set_keys_sorted(keys_sorted);
        Ok(MapField { inner })
    }

    /// The child **key** field.
    fn key(&self, py: Python<'_>) -> PyResult<PyObject> {
        column_field_to_py(py, self.inner.key().clone())
    }

    /// The child **value** field.
    fn value(&self, py: Python<'_>) -> PyResult<PyObject> {
        column_field_to_py(py, self.inner.value().clone())
    }

    /// Whether the keys are **sorted** within each map (an Arrow schema hint).
    fn keys_sorted(&self) -> bool {
        self.inner.keys_sorted()
    }

    /// The map's name, if set.
    fn name(&self) -> Option<String> {
        self.inner.name().map(str::to_string)
    }

    /// Whether the map admits null entries.
    fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// Sets whether the map admits null entries **in place**.
    fn set_nullable(&mut self, nullable: bool) {
        self.inner.set_nullable(nullable);
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        hash_value(&self.inner)
    }

    fn __repr__(&self) -> String {
        format!(
            "MapField(name={:?}, key_dtype='{}', value_dtype='{}', keys_sorted={})",
            self.inner.name(),
            self.inner.key().data_type_id().name(),
            self.inner.value().data_type_id().name(),
            if self.inner.keys_sorted() {
                "True"
            } else {
                "False"
            },
        )
    }
}

// =====================================================================================
// StructSerie — the struct "table"
// =====================================================================================

/// A **struct column** — the "table": an ordered set of heterogeneous, equal-length child columns
/// under one name, with an optional row-level validity buffer. Built from a list of columns; a child
/// is read back as a **copy** (`column` / `column_by_name`) and replaced with `set_column` — the FFI
/// cannot hand a live `&mut` into an inner series, so mutation is copy-and-replace. Wraps the core
/// `StructSerie`.
#[pyclass(module = "yggdryl.typed")]
pub struct StructSerie {
    pub(crate) inner: CoreStructSerie,
}

#[pymethods]
impl StructSerie {
    /// A struct from `columns` (a list of `Serie` / `ByteSerie` / nested columns). When `names` is
    /// given (one per column) each column is renamed to the matching name; otherwise each keeps its
    /// own. Raises `ValueError` on a length mismatch (columns of differing row counts, or a `names`
    /// list of the wrong length).
    #[staticmethod]
    #[pyo3(signature = (columns, names = None))]
    fn from_columns(
        columns: &Bound<'_, PyAny>,
        names: Option<Vec<String>>,
    ) -> PyResult<StructSerie> {
        let mut cols: Vec<Column> = Vec::new();
        for item in columns.iter()? {
            cols.push(column_from_py(&item?)?);
        }
        if let Some(names) = names {
            if names.len() != cols.len() {
                return Err(PyValueError::new_err(format!(
                    "names has {} entries but there are {} columns: pass one name per column, or \
                     omit names to keep each column's own name",
                    names.len(),
                    cols.len()
                )));
            }
            cols = cols
                .into_iter()
                .zip(names)
                .map(|(col, name)| rename_column(col, name.as_str()))
                .collect();
        }
        Ok(StructSerie {
            inner: CoreStructSerie::from_columns(cols).map_err(ioerr)?,
        })
    }

    /// The number of child columns.
    fn num_columns(&self) -> usize {
        self.inner.num_columns()
    }

    /// The number of rows.
    fn len(&self) -> usize {
        self.inner.len()
    }

    /// The number of rows (so `len(struct)` works).
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Truthiness — `True` when the struct holds at least one row.
    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    /// Whether the struct has no rows.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// How many rows are null.
    fn null_count(&self) -> usize {
        self.inner.null_count()
    }

    /// A **copy** of the child column at `index` (a `Serie` / `ByteSerie` / nested column); raises
    /// `IndexError` when `index` is out of range.
    fn column(&self, py: Python<'_>, index: usize) -> PyResult<PyObject> {
        match self.inner.column(index) {
            Some(col) => column_to_py(py, clone_column(col)),
            None => Err(PyIndexError::new_err(format!(
                "column index {index} out of range: the struct has {} columns",
                self.inner.num_columns()
            ))),
        }
    }

    /// A **copy** of the first child column named `name`, or `None` when there is none.
    fn column_by_name(&self, py: Python<'_>, name: &str) -> PyResult<Option<PyObject>> {
        match self.inner.column_by_name(name) {
            Some(col) => Ok(Some(column_to_py(py, clone_column(col))?)),
            None => Ok(None),
        }
    }

    /// The child column names in order (an unnamed column reports `""`).
    fn column_names(&self) -> Vec<String> {
        self.inner
            .columns()
            .iter()
            .map(|col| col.name().unwrap_or("").to_string())
            .collect()
    }

    /// **Replaces** the child column named `name` with `column` (renamed to `name`), rebuilding the
    /// struct in place. Raises `ValueError` when no column is named `name`, or when the replacement's
    /// row count does not match the struct's.
    fn set_column(&mut self, name: &str, column: &Bound<'_, PyAny>) -> PyResult<()> {
        let index = self
            .inner
            .columns()
            .iter()
            .position(|col| col.name() == Some(name))
            .ok_or_else(|| {
                PyValueError::new_err(format!(
                    "no column named {name:?}: set_column replaces an existing column — check \
                     column_names(), or add columns at construction with from_columns"
                ))
            })?;
        let replacement = rename_column(column_from_py(column)?, name);
        let mut children: Vec<Column> = self.inner.columns().iter().map(clone_column).collect();
        children[index] = replacement;
        self.inner = struct_from_children(children, &self.inner)?;
        Ok(())
    }

    /// The **row** at `index` — the row's child values as a Python `list` (each marshalled from its
    /// erased value: `int` / `float` / `bool` / `bytes` / `str` / `None`, a nested row / list as a
    /// `list`, a nested map as a `dict`). Raises `IndexError` when `index` is out of range.
    fn row(&self, py: Python<'_>, index: usize) -> PyResult<Vec<PyObject>> {
        match self.inner.row(index) {
            Some(scalar) => values_to_py(py, scalar.values()),
            None => Err(PyIndexError::new_err(format!(
                "row index {index} out of range: the struct has {} rows",
                self.inner.len()
            ))),
        }
    }

    /// The struct's [`StructField`] schema — its name, nullability, and ordered child field
    /// descriptors.
    fn field(&self) -> StructField {
        StructField {
            inner: self.inner.field(),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "StructSerie(name={:?}, num_columns={}, len={}, null_count={})",
            self.inner.name(),
            self.inner.num_columns(),
            self.inner.len(),
            self.inner.null_count(),
        )
    }
}

// =====================================================================================
// ListSerie — the variable-length list column
// =====================================================================================

/// A **list column** — a variable-length list over a flattened child column. Build it over the child
/// column, then demarcate each list with `push(child_len)` (or `push_null`); read a list back with
/// `list(index)`. Wraps the core `ListSerie`.
#[pyclass(module = "yggdryl.typed")]
pub struct ListSerie {
    pub(crate) inner: CoreListSerie,
}

#[pymethods]
impl ListSerie {
    /// An **empty** list column over the flattened child `values` (a `Serie` / `ByteSerie` / nested
    /// column), named `name`. The child's rows become the list elements as `push` demarcates them.
    #[new]
    #[pyo3(signature = (values, name = None))]
    fn new(values: &Bound<'_, PyAny>, name: Option<String>) -> PyResult<ListSerie> {
        Ok(ListSerie {
            inner: CoreListSerie::new(name.as_deref().unwrap_or(""), column_from_py(values)?),
        })
    }

    /// Appends a **non-null** list spanning the next `child_len` rows of the flattened child.
    fn push(&mut self, child_len: usize) {
        self.inner.push(child_len);
    }

    /// Appends a **null** list (an empty span with the validity bit cleared).
    fn push_null(&mut self) {
        self.inner.push_null();
    }

    /// A **copy** of the flattened child column holding every list's elements.
    fn values(&self, py: Python<'_>) -> PyResult<PyObject> {
        column_to_py(py, clone_column(self.inner.values()))
    }

    /// The list element at `index` as a Python `list` of its child values, or `None` when the list is
    /// null or `index` is out of range. (A valid empty list is `[]`, distinct from a null list's
    /// `None`.)
    fn list(&self, py: Python<'_>, index: usize) -> PyResult<Option<Vec<PyObject>>> {
        match self.inner.list(index) {
            Some(scalar) if scalar.is_valid() => Ok(Some(values_to_py(py, scalar.values())?)),
            _ => Ok(None),
        }
    }

    /// The number of lists.
    fn len(&self) -> usize {
        self.inner.len()
    }

    /// The number of lists (so `len(list_serie)` works).
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Truthiness — `True` when the column holds at least one list.
    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    /// Whether the column has no lists.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// How many lists are null.
    fn null_count(&self) -> usize {
        self.inner.null_count()
    }

    /// The list's [`ListField`] schema — its name, nullability, and child item field.
    fn field(&self) -> ListField {
        ListField {
            inner: self.inner.field(),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "ListSerie(name={:?}, len={}, null_count={})",
            self.inner.name(),
            self.inner.len(),
            self.inner.null_count(),
        )
    }
}

// =====================================================================================
// MapSerie — the map column
// =====================================================================================

/// A **map column** — an offsets buffer over flattened `key` + `value` entry columns. Build it over
/// the two columns, then demarcate each map with `push(entry_count)` (or `push_null`); read a map
/// back with `get(index)`. The key column must be non-nullable (map keys cannot be null). Wraps the
/// core `MapSerie`.
#[pyclass(module = "yggdryl.typed")]
pub struct MapSerie {
    pub(crate) inner: CoreMapSerie,
}

#[pymethods]
impl MapSerie {
    /// An **empty** map column over the flattened `keys` + `values` entry columns (each a `Serie` /
    /// `ByteSerie` / nested column), named `name`. Their rows become the entries as `push` demarcates
    /// them. Raises `ValueError` when the key column is nullable or the two columns differ in length.
    #[new]
    #[pyo3(signature = (keys, values, name = None))]
    fn new(
        keys: &Bound<'_, PyAny>,
        values: &Bound<'_, PyAny>,
        name: Option<String>,
    ) -> PyResult<MapSerie> {
        Ok(MapSerie {
            inner: CoreMapSerie::new(
                name.as_deref().unwrap_or(""),
                column_from_py(keys)?,
                column_from_py(values)?,
            )
            .map_err(ioerr)?,
        })
    }

    /// Appends a **non-null** map spanning the next `entry_count` rows of the flattened entries.
    fn push(&mut self, entry_count: usize) {
        self.inner.push(entry_count);
    }

    /// Appends a **null** map (an empty span with the validity bit cleared).
    fn push_null(&mut self) {
        self.inner.push_null();
    }

    /// A **copy** of the flattened **key** column.
    fn keys(&self, py: Python<'_>) -> PyResult<PyObject> {
        column_to_py(py, clone_column(self.inner.keys()))
    }

    /// A **copy** of the flattened **value** column.
    fn values(&self, py: Python<'_>) -> PyResult<PyObject> {
        column_to_py(py, clone_column(self.inner.values()))
    }

    /// The map element at `index` as a Python `dict` of its key→value entries, or `None` when the map
    /// is null or `index` is out of range. (A duplicate key keeps the last entry, per `dict`.)
    fn get(&self, py: Python<'_>, index: usize) -> PyResult<Option<PyObject>> {
        match self.inner.map(index) {
            Some(scalar) if scalar.is_valid() => {
                let dict = PyDict::new_bound(py);
                for entry in 0..scalar.len() {
                    let key = value_to_py(py, &scalar.keys()[entry])?;
                    let value = value_to_py(py, &scalar.values()[entry])?;
                    dict.set_item(key, value)?;
                }
                Ok(Some(dict.into_any().unbind()))
            }
            _ => Ok(None),
        }
    }

    /// Whether the keys are **sorted** within each map (an Arrow schema hint).
    fn keys_sorted(&self) -> bool {
        self.inner.keys_sorted()
    }

    /// The number of maps.
    fn len(&self) -> usize {
        self.inner.len()
    }

    /// The number of maps (so `len(map_serie)` works).
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    /// Truthiness — `True` when the column holds at least one map.
    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    /// Whether the column has no maps.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// How many maps are null.
    fn null_count(&self) -> usize {
        self.inner.null_count()
    }

    /// The map's [`MapField`] schema — its name, nullability, child key / value fields, and
    /// `keys_sorted` hint.
    fn field(&self) -> MapField {
        MapField {
            inner: self.inner.field(),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "MapSerie(name={:?}, len={}, null_count={}, keys_sorted={})",
            self.inner.name(),
            self.inner.len(),
            self.inner.null_count(),
            if self.inner.keys_sorted() {
                "True"
            } else {
                "False"
            },
        )
    }
}

/// Registers the nested carriers and their schema descriptors on the `yggdryl.typed` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<StructSerie>()?;
    module.add_class::<StructField>()?;
    module.add_class::<ListSerie>()?;
    module.add_class::<ListField>()?;
    module.add_class::<MapSerie>()?;
    module.add_class::<MapField>()?;
    Ok(())
}
