//! The **Arrow C Data Interface** bridge — the standard, zero-copy pyarrow interop.
//!
//! This module adds the Arrow [PyCapsule protocol] to the typed carriers so a pyarrow build imports
//! them with **no** pyarrow dependency in Rust: a [`StructSerie`] exposes `__arrow_c_schema__` /
//! `__arrow_c_array__` (so `pyarrow.record_batch(serie)` / `pyarrow.table(serie)` /
//! `pyarrow.schema(serie)` work) and a `StructSerie.from_arrow(obj)` constructor (importing any
//! object that exposes `__arrow_c_array__` — a pyarrow `RecordBatch` / `StructArray`), while the leaf
//! [`Serie`] / [`ByteSerie`] expose `__arrow_c_array__` (so `pyarrow.array(serie)` works).
//!
//! Every method is thin over the core `yggdryl_core::arrow` bridge (feature `arrow`): the conversion
//! to / from the arrow-rs [`RecordBatch`] / [`StructArray`] / `ArrayRef` lives there, and this module
//! only handles the **FFI export/import** — moving the arrow-rs [`FFI_ArrowArray`] / [`FFI_ArrowSchema`]
//! structs across the C Data Interface via `to_ffi` / `from_ffi` and boxing each into a
//! [`PyCapsule`] with the protocol-mandated name (`"arrow_schema"` / `"arrow_array"`).
//!
//! ## Capsule lifetime
//!
//! On **export**, pyo3 boxes each `FFI_Arrow*` struct into a capsule whose destructor drops it — and
//! arrow-rs's `Drop` for `FFI_ArrowArray` / `FFI_ArrowSchema` calls the C release callback only when
//! it is still set. A consumer (pyarrow) that moves the array out nulls that callback, so the
//! destructor becomes a no-op: no double free. On **import** ([`StructSerie::from_arrow`]) we do the
//! same in reverse — [`std::ptr::replace`] moves the real struct out of the producer's capsule and
//! leaves an empty (released) struct behind, so the producer's own destructor is a no-op and the
//! imported buffers are owned by the [`ArrayData`](arrow_data::ArrayData) `from_ffi` builds.
//!
//! [PyCapsule protocol]: https://arrow.apache.org/docs/format/CDataInterface/PyCapsuleInterface.html

// `useless_conversion`: pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type `From`.
#![allow(clippy::useless_conversion)]

use std::ffi::CString;

use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyCapsule, PyTuple};

use arrow_array::ffi::{from_ffi, to_ffi, FFI_ArrowArray, FFI_ArrowSchema};
use arrow_array::{Array, RecordBatch, StructArray};
use arrow_schema::{ArrowError, DataType};

use yggdryl_core::arrow::{
    column_to_arrow, struct_field_to_arrow_schema, struct_serie_from_record_batch,
    struct_serie_to_record_batch,
};

use crate::nested::{column_from_byte_inner, column_from_inner, StructSerie};
use crate::typed::{ioerr, ByteSerie, Serie};

/// Maps an arrow-rs [`ArrowError`] to a Python `ValueError` carrying its text — the Arrow-side twin
/// of [`ioerr`] (which maps a core `IoError`).
fn arrow_err(error: ArrowError) -> PyErr {
    PyValueError::new_err(error.to_string())
}

/// The C-string capsule name the Arrow protocol mandates (`"arrow_schema"` / `"arrow_array"`) — a
/// static ASCII literal never carries an interior nul, so the build is infallible.
fn capsule_name(name: &str) -> CString {
    CString::new(name).expect("a static Arrow capsule name has no interior nul byte")
}

/// Boxes the exported `FFI_ArrowSchema` + `FFI_ArrowArray` into the `("arrow_schema", "arrow_array")`
/// [`PyCapsule`] pair the Arrow C array protocol returns from `__arrow_c_array__`. pyo3 gives each
/// capsule a destructor that drops the boxed struct (releasing it if a consumer has not already moved
/// it out).
fn array_capsules(
    py: Python<'_>,
    ffi_schema: FFI_ArrowSchema,
    ffi_array: FFI_ArrowArray,
) -> PyResult<(PyObject, PyObject)> {
    let schema_capsule = PyCapsule::new_bound(py, ffi_schema, Some(capsule_name("arrow_schema")))?;
    let array_capsule = PyCapsule::new_bound(py, ffi_array, Some(capsule_name("arrow_array")))?;
    Ok((
        schema_capsule.into_any().unbind(),
        array_capsule.into_any().unbind(),
    ))
}

/// Reads the `("arrow_schema", "arrow_array")` capsule pair from any object exposing
/// `__arrow_c_array__` (a pyarrow `RecordBatch` / `StructArray` / …) and **moves** the two arrow-rs
/// FFI structs out of the producer's capsules — leaving empty (released) structs behind so the
/// producer's capsule destructors become no-ops (the imported buffers are then owned by the
/// [`ArrayData`](arrow_data::ArrayData) built from them).
fn import_c_array(obj: &Bound<'_, PyAny>) -> PyResult<(FFI_ArrowArray, FFI_ArrowSchema)> {
    if !obj.hasattr("__arrow_c_array__")? {
        return Err(PyTypeError::new_err(
            "from_arrow expected an object exposing the Arrow C array interface (__arrow_c_array__) \
             — pass a pyarrow RecordBatch or StructArray (or any object with __arrow_c_array__)",
        ));
    }
    let result = obj.call_method0("__arrow_c_array__")?;
    let tuple = result.downcast::<PyTuple>().map_err(|_| {
        PyValueError::new_err(
            "__arrow_c_array__ must return a (schema_capsule, array_capsule) tuple",
        )
    })?;
    if tuple.len() != 2 {
        return Err(PyValueError::new_err(
            "__arrow_c_array__ must return a 2-tuple of (schema_capsule, array_capsule)",
        ));
    }
    let schema_capsule = tuple
        .get_item(0)?
        .downcast_into::<PyCapsule>()
        .map_err(|_| {
            PyValueError::new_err("__arrow_c_array__[0] was not an 'arrow_schema' capsule")
        })?;
    let array_capsule = tuple
        .get_item(1)?
        .downcast_into::<PyCapsule>()
        .map_err(|_| {
            PyValueError::new_err("__arrow_c_array__[1] was not an 'arrow_array' capsule")
        })?;

    let schema_ptr = schema_capsule.pointer() as *mut FFI_ArrowSchema;
    let array_ptr = array_capsule.pointer() as *mut FFI_ArrowArray;
    if schema_ptr.is_null() || array_ptr.is_null() {
        return Err(PyValueError::new_err(
            "the Arrow C array capsules held a null pointer (was the array already consumed?)",
        ));
    }
    // SAFETY: the capsules own live, correctly-typed `FFI_Arrow*` structs (their names identify the
    // C Data Interface). `ptr::replace` moves each out and writes an empty (release == null) struct
    // back, so the producer's capsule destructors drop a no-op struct — no double free.
    let ffi_array = unsafe { std::ptr::replace(array_ptr, FFI_ArrowArray::empty()) };
    let ffi_schema = unsafe { std::ptr::replace(schema_ptr, FFI_ArrowSchema::empty()) };
    Ok((ffi_array, ffi_schema))
}

// =====================================================================================
// StructSerie — the RecordBatch / Schema bridge
// =====================================================================================

#[pymethods]
impl StructSerie {
    /// The Arrow C **schema** capsule (`"arrow_schema"`) for the struct's schema — a `Struct` Arrow
    /// type over the child fields — so `pyarrow.schema(struct_serie)` imports it zero-copy. Delegates
    /// to `yggdryl_core::arrow::struct_field_to_arrow_schema` and exports it through the FFI schema.
    fn __arrow_c_schema__(&self, py: Python<'_>) -> PyResult<PyObject> {
        let schema = struct_field_to_arrow_schema(&self.inner.field());
        let ffi_schema = FFI_ArrowSchema::try_from(&schema).map_err(arrow_err)?;
        let capsule = PyCapsule::new_bound(py, ffi_schema, Some(capsule_name("arrow_schema")))?;
        Ok(capsule.into_any().unbind())
    }

    /// The Arrow C **array** capsule pair `("arrow_schema", "arrow_array")` — the struct as an Arrow
    /// [`StructArray`] (built from a `RecordBatch` via `struct_serie_to_record_batch`) exported over
    /// the C Data Interface, so `pyarrow.record_batch(struct_serie)` / `pyarrow.table(struct_serie)`
    /// import it zero-copy. `requested_schema` is advisory (a hint to cast on export) and is ignored:
    /// we always export the struct's native schema. Raises `ValueError` if the struct holds null rows
    /// (a `RecordBatch` carries no row-level validity — the core bridge refuses it).
    #[pyo3(signature = (requested_schema=None))]
    fn __arrow_c_array__(
        &self,
        py: Python<'_>,
        requested_schema: Option<PyObject>,
    ) -> PyResult<(PyObject, PyObject)> {
        let _ = requested_schema;
        let batch = struct_serie_to_record_batch(&self.inner).map_err(ioerr)?;
        let array = StructArray::from(batch);
        let (ffi_array, ffi_schema) = to_ffi(&array.to_data()).map_err(arrow_err)?;
        array_capsules(py, ffi_schema, ffi_array)
    }

    /// Builds a [`StructSerie`] from any object exposing the Arrow C array interface — a pyarrow
    /// `RecordBatch` / `StructArray`, or anything with `__arrow_c_array__` over a struct. Reads the
    /// capsules, imports them through `from_ffi` into an Arrow `StructArray`, and rebuilds the struct
    /// through `yggdryl_core::arrow::struct_serie_from_record_batch`. Raises `TypeError` for a
    /// non-Arrow object, and `ValueError` when the imported array is not a struct, or holds null rows
    /// (a `StructSerie` rebuilt from a `RecordBatch` has no row-level validity — fill / drop them first).
    #[staticmethod]
    fn from_arrow(obj: &Bound<'_, PyAny>) -> PyResult<StructSerie> {
        let (ffi_array, ffi_schema) = import_c_array(obj)?;
        // SAFETY: `ffi_array` / `ffi_schema` were just moved out of valid producer capsules; `from_ffi`
        // takes ownership of the array and reads the schema by reference.
        let array_data = unsafe { from_ffi(ffi_array, &ffi_schema) }.map_err(arrow_err)?;
        if !matches!(array_data.data_type(), DataType::Struct(_)) {
            return Err(PyValueError::new_err(format!(
                "from_arrow expected a struct / record batch, got an Arrow array of type {:?}: pass \
                 a pyarrow RecordBatch or StructArray (or a __arrow_c_array__ object over a struct)",
                array_data.data_type()
            )));
        }
        let struct_array = StructArray::from(array_data);
        if struct_array.null_count() > 0 {
            return Err(PyValueError::new_err(
                "from_arrow cannot import a struct with null rows: a StructSerie built from a record \
                 batch has no row-level validity — fill or drop the null rows first",
            ));
        }
        let batch = RecordBatch::from(struct_array);
        Ok(StructSerie {
            inner: struct_serie_from_record_batch(&batch).map_err(ioerr)?,
        })
    }
}

// =====================================================================================
// Serie / ByteSerie — the leaf array bridge (`pyarrow.array(serie)`)
// =====================================================================================

#[pymethods]
impl Serie {
    /// The Arrow C **array** capsule pair `("arrow_schema", "arrow_array")` for this leaf column, so
    /// `pyarrow.array(serie)` imports it zero-copy. `requested_schema` is advisory and ignored (we
    /// always export the column's native dtype). Delegates to `yggdryl_core::arrow::column_to_arrow`.
    #[pyo3(signature = (requested_schema=None))]
    fn __arrow_c_array__(
        &self,
        py: Python<'_>,
        requested_schema: Option<PyObject>,
    ) -> PyResult<(PyObject, PyObject)> {
        let _ = requested_schema;
        let column = column_from_inner(&self.inner);
        let array = column_to_arrow(&column).map_err(ioerr)?;
        let (ffi_array, ffi_schema) = to_ffi(&array.to_data()).map_err(arrow_err)?;
        array_capsules(py, ffi_schema, ffi_array)
    }
}

#[pymethods]
impl ByteSerie {
    /// The Arrow C **array** capsule pair `("arrow_schema", "arrow_array")` for this byte column, so
    /// `pyarrow.array(byte_serie)` imports it zero-copy. `requested_schema` is advisory and ignored.
    /// Delegates to `yggdryl_core::arrow::column_to_arrow`.
    #[pyo3(signature = (requested_schema=None))]
    fn __arrow_c_array__(
        &self,
        py: Python<'_>,
        requested_schema: Option<PyObject>,
    ) -> PyResult<(PyObject, PyObject)> {
        let _ = requested_schema;
        let column = column_from_byte_inner(&self.inner);
        let array = column_to_arrow(&column).map_err(ioerr)?;
        let (ffi_array, ffi_schema) = to_ffi(&array.to_data()).map_err(arrow_err)?;
        array_capsules(py, ffi_schema, ffi_array)
    }
}
