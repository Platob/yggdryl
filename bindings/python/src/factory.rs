//! The `yggdryl.factory` submodule: a convenient **type-inference** factory.
//!
//! `factory.scalar(value)`, `factory.dtype(value)` and
//! `factory.field(name, value)` infer the data type from a native Python value and
//! build the matching `yggdryl.scalar` / `yggdryl.dtype` / `yggdryl.field` object,
//! so a value crosses without naming its type. The inference mirrors the model's
//! available types: `int` → `int64`, `bytes` / `bytearray` → `binary`, `None` →
//! `null`, a homogeneous `list` of ints → an `int64` serie, and a `dict` of named
//! values → a `struct` (`factory.scalar` builds the `RecordScalar` row,
//! `factory.dtype` the `StructType`, `factory.field` the `StructField`). Every
//! object of the model is an inference input too: `factory.scalar` hands a scalar
//! object back as a new handle of the same class and a data type object its
//! default scalar; `factory.dtype` maps a scalar, data type or field object to
//! its data type; `factory.field` pairs `name` with a scalar or data type
//! object's field. A value the model has no type for (a `float`, `str`, `bool`,
//! or a list of anything but ints) raises a `ValueError` — build it through the
//! explicit per-type factories.

// pyo3's `#[pyfunction]` expansion re-wraps the already-`PyErr` result into `PyErr`;
// clippy flags that generated conversion (on the return-type span) as useless.
#![allow(clippy::useless_conversion)]

use pyo3::prelude::*;
use pyo3::types::{PyBool, PyByteArray, PyBytes, PyDict, PyInt, PyList};
use yggdryl_dtype::{arrow_schema, DataType};
use yggdryl_field::{Field, FieldFactory};
use yggdryl_scalar::{Scalar, ScalarFactory};

use crate::DataErr;

/// The inferred type, carrying the extracted native value (built once so `scalar`
/// keeps it while `dtype` / `field` ignore it).
pub(crate) enum Inferred {
    Null,
    Int64(i64),
    Binary(Vec<u8>),
    Serie(Vec<i64>),
    /// A dict row: each field name paired with its own inference, in dict order.
    Record(Vec<(String, Inferred)>),
}

impl Inferred {
    /// The Arrow data type of the inference — exactly the type the matching
    /// scalar's Arrow form carries, so a struct field and its column always agree.
    fn arrow_dtype(&self) -> arrow_schema::DataType {
        match self {
            Inferred::Null => yggdryl_dtype::NullType.to_arrow(),
            Inferred::Int64(_) => yggdryl_dtype::Int64Type.to_arrow(),
            Inferred::Binary(_) => yggdryl_dtype::BinaryType.to_arrow(),
            Inferred::Serie(_) => {
                yggdryl_dtype::TypedSerieType::<yggdryl_dtype::Int64Type>::default().to_arrow()
            }
            Inferred::Record(entries) => struct_type_of(entries).to_arrow(),
        }
    }

    /// The inferred value as a one-element column — the matching scalar's Arrow
    /// scalar form, decomposed into the core's own holder (a record child).
    fn to_column(&self) -> Result<yggdryl_scalar::AnySerie, DataErr> {
        Ok(match self {
            Inferred::Null => yggdryl_scalar::NullScalar::default()
                .to_arrow_scalar()
                .into(),
            Inferred::Int64(integer) => yggdryl_scalar::Int64Scalar::new(*integer)
                .to_arrow_scalar()
                .into(),
            Inferred::Binary(bytes) => yggdryl_scalar::BinaryScalar::new(bytes.clone())
                .to_arrow_scalar()
                .into(),
            Inferred::Serie(values) => yggdryl_scalar::Int64Serie::from(values.clone())
                .to_arrow_scalar()
                .into(),
            Inferred::Record(entries) => record_of(entries)?.to_arrow_scalar().into(),
        })
    }
}

/// The struct data type of inferred dict `entries` — one nullable child field per
/// entry, in dict order.
pub(crate) fn struct_type_of(entries: &[(String, Inferred)]) -> yggdryl_dtype::StructType {
    yggdryl_dtype::StructType::new(
        entries
            .iter()
            .map(|(name, value)| arrow_schema::Field::new(name, value.arrow_dtype(), true))
            .collect::<Vec<_>>()
            .into(),
    )
}

/// The core record scalar of inferred dict `entries`: the struct type plus one
/// one-element child column per field, each built once in Rust.
pub(crate) fn record_of(
    entries: &[(String, Inferred)],
) -> Result<yggdryl_scalar::RecordScalar, DataErr> {
    let columns = entries
        .iter()
        .map(|(_, value)| value.to_column())
        .collect::<Result<Vec<_>, DataErr>>()?;
    Ok(yggdryl_scalar::RecordScalar::new(
        struct_type_of(entries),
        columns,
    )?)
}

/// Infer every entry of a dict row — each field name paired with its inference —
/// shared by the factory and the `RecordScalar` constructor.
pub(crate) fn infer_entries(row: &Bound<'_, PyDict>) -> PyResult<Vec<(String, Inferred)>> {
    let mut entries = Vec::with_capacity(row.len());
    for (name, value) in row.iter() {
        let name = name.extract::<String>().map_err(|_| {
            PyErr::from(DataErr::Message(
                "cannot infer a record: every dict key must be a str field name".to_string(),
            ))
        })?;
        entries.push((name, infer(&value)?));
    }
    Ok(entries)
}

/// Resolve a struct child declaration — an example native value or a
/// `yggdryl.dtype` class instance — to its Arrow data type; the `StructType`
/// constructor resolves each dict value through this.
pub(crate) fn resolve_arrow_dtype(value: &Bound<'_, PyAny>) -> PyResult<arrow_schema::DataType> {
    macro_rules! arrow_of {
        ($($class:ident),+ $(,)?) => {
            $(if let Ok(dtype) = value.downcast::<crate::dtype::$class>() {
                return Ok(dtype.borrow().inner.to_arrow());
            })+
        };
    }
    arrow_of!(
        NullType,
        BinaryType,
        StructType,
        Int8Type,
        Int16Type,
        Int32Type,
        Int64Type,
        UInt8Type,
        UInt16Type,
        UInt32Type,
        UInt64Type,
        Int8SerieType,
        Int16SerieType,
        Int32SerieType,
        Int64SerieType,
        UInt8SerieType,
        UInt16SerieType,
        UInt32SerieType,
        UInt64SerieType,
    );
    Ok(infer(value)?.arrow_dtype())
}

/// Raises a `ValueError` naming the Python type the model cannot infer.
fn unsupported(py_type: &str) -> PyErr {
    DataErr::Message(format!(
        "cannot infer a yggdryl type from a Python {py_type}; the model has no matching type — \
         use int / bytes / None / a list of int / a dict of named values, one of the model's \
         scalar / dtype / field objects, or an explicit per-type factory"
    ))
    .into()
}

/// Infer the data type from `value`, extracting the native value.
fn infer(value: &Bound<'_, PyAny>) -> PyResult<Inferred> {
    if value.is_none() {
        return Ok(Inferred::Null);
    }
    // A Python `bool` is an `int` subclass; reject it before the int check so it does
    // not silently become an int64.
    if value.is_instance_of::<PyBool>() {
        return Err(unsupported("bool"));
    }
    if value.is_instance_of::<PyInt>() {
        let integer = value.extract::<i64>().map_err(|_| {
            PyErr::from(DataErr::Message(
                "cannot infer a scalar: the integer is outside the int64 range; build it with the \
                 explicit uint64 / int64 factory"
                    .to_string(),
            ))
        })?;
        return Ok(Inferred::Int64(integer));
    }
    if value.is_instance_of::<PyBytes>() || value.is_instance_of::<PyByteArray>() {
        return Ok(Inferred::Binary(value.extract()?));
    }
    if value.is_instance_of::<PyList>() {
        // A homogeneous list of ints → an int64 serie (the model's only bindable
        // serie); an empty list defaults to it too.
        let values = value.extract::<Vec<i64>>().map_err(|_| {
            PyErr::from(DataErr::Message(
                "cannot infer a serie: expected a list of int64 values".to_string(),
            ))
        })?;
        return Ok(Inferred::Serie(values));
    }
    if let Ok(row) = value.downcast::<PyDict>() {
        // A dict of named values → a struct row, each child inferred recursively.
        return Ok(Inferred::Record(infer_entries(row)?));
    }
    Err(unsupported(
        &value
            .get_type()
            .name()
            .map(|name| name.to_string())
            .unwrap_or_else(|_| "value".to_string()),
    ))
}

/// Infer the data type from `value` and build the matching `yggdryl.scalar`: a
/// native value becomes its scalar (a dict a `RecordScalar`), a scalar object of
/// the model a new handle of the same class over the same value, and a data type
/// object of the model its default scalar.
#[pyfunction]
fn scalar(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<PyObject> {
    // A model scalar object: a new handle of the same class over the same value.
    macro_rules! same_scalar {
        ($($class:ident),+ $(,)?) => {
            $(if let Ok(handle) = value.downcast::<crate::scalar::$class>() {
                let inner = handle.borrow().inner.clone();
                return Ok(Py::new(py, crate::scalar::$class { inner })?.into_any());
            })+
        };
    }
    same_scalar!(
        NullScalar,
        BinaryScalar,
        RecordScalar,
        Int8Scalar,
        Int16Scalar,
        Int32Scalar,
        Int64Scalar,
        UInt8Scalar,
        UInt16Scalar,
        UInt32Scalar,
        UInt64Scalar,
        Int8Serie,
        Int16Serie,
        Int32Serie,
        Int64Serie,
        UInt8Serie,
        UInt16Serie,
        UInt32Serie,
        UInt64Serie,
    );
    // A model data type object: its default scalar (the null type's is the null
    // scalar, a serie type's the empty serie).
    if value.downcast::<crate::dtype::NullType>().is_ok() {
        return Ok(Py::new(py, crate::scalar::NullScalar::default())?.into_any());
    }
    macro_rules! default_scalar_of {
        ($(($dtype:ident, $scalar:ident)),+ $(,)?) => {
            $(if let Ok(handle) = value.downcast::<crate::dtype::$dtype>() {
                let inner = handle.borrow().inner.default_scalar();
                return Ok(Py::new(py, crate::scalar::$scalar { inner })?.into_any());
            })+
        };
    }
    default_scalar_of!(
        (BinaryType, BinaryScalar),
        (Int8Type, Int8Scalar),
        (Int16Type, Int16Scalar),
        (Int32Type, Int32Scalar),
        (Int64Type, Int64Scalar),
        (UInt8Type, UInt8Scalar),
        (UInt16Type, UInt16Scalar),
        (UInt32Type, UInt32Scalar),
        (UInt64Type, UInt64Scalar),
    );
    macro_rules! default_serie_of {
        ($(($dtype:ident, $serie:ident)),+ $(,)?) => {
            $(if value.downcast::<crate::dtype::$dtype>().is_ok() {
                let inner = yggdryl_scalar::$serie::default();
                return Ok(Py::new(py, crate::scalar::$serie { inner })?.into_any());
            })+
        };
    }
    default_serie_of!(
        (Int8SerieType, Int8Serie),
        (Int16SerieType, Int16Serie),
        (Int32SerieType, Int32Serie),
        (Int64SerieType, Int64Serie),
        (UInt8SerieType, UInt8Serie),
        (UInt16SerieType, UInt16Serie),
        (UInt32SerieType, UInt32Serie),
        (UInt64SerieType, UInt64Serie),
    );
    Ok(match infer(value)? {
        Inferred::Null => Py::new(py, crate::scalar::NullScalar::default())?.into_any(),
        Inferred::Int64(integer) => Py::new(
            py,
            crate::scalar::Int64Scalar {
                inner: yggdryl_scalar::Int64Scalar::new(integer),
            },
        )?
        .into_any(),
        Inferred::Binary(bytes) => Py::new(
            py,
            crate::scalar::BinaryScalar {
                inner: yggdryl_scalar::BinaryScalar::new(bytes),
            },
        )?
        .into_any(),
        Inferred::Serie(values) => Py::new(
            py,
            crate::scalar::Int64Serie {
                inner: yggdryl_scalar::Int64Serie::from(values),
            },
        )?
        .into_any(),
        Inferred::Record(entries) => Py::new(
            py,
            crate::scalar::RecordScalar {
                inner: record_of(&entries)?,
            },
        )?
        .into_any(),
    })
}

/// Infer the data type from `value` and build the matching `yggdryl.dtype`: a
/// native value names its type (a dict a `StructType`), and a data type, scalar
/// or field object of the model maps to its data type.
#[pyfunction]
fn dtype(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<PyObject> {
    // A model data type object: a same-type new instance.
    macro_rules! same_dtype {
        ($($class:ident),+ $(,)?) => {
            $(if value.downcast::<crate::dtype::$class>().is_ok() {
                return Ok(Py::new(py, crate::dtype::$class::default())?.into_any());
            })+
        };
    }
    same_dtype!(
        NullType,
        BinaryType,
        Int8Type,
        Int16Type,
        Int32Type,
        Int64Type,
        UInt8Type,
        UInt16Type,
        UInt32Type,
        UInt64Type,
        Int8SerieType,
        Int16SerieType,
        Int32SerieType,
        Int64SerieType,
        UInt8SerieType,
        UInt16SerieType,
        UInt32SerieType,
        UInt64SerieType,
    );
    if let Ok(handle) = value.downcast::<crate::dtype::StructType>() {
        return Ok(Py::new(py, handle.borrow().clone())?.into_any());
    }
    // A model scalar or field object: its data type.
    macro_rules! dtype_of {
        ($module:ident, $(($class:ident, $dtype:ident)),+ $(,)?) => {
            $(if value.downcast::<crate::$module::$class>().is_ok() {
                return Ok(Py::new(py, crate::dtype::$dtype::default())?.into_any());
            })+
        };
    }
    dtype_of!(
        scalar,
        (NullScalar, NullType),
        (BinaryScalar, BinaryType),
        (Int8Scalar, Int8Type),
        (Int16Scalar, Int16Type),
        (Int32Scalar, Int32Type),
        (Int64Scalar, Int64Type),
        (UInt8Scalar, UInt8Type),
        (UInt16Scalar, UInt16Type),
        (UInt32Scalar, UInt32Type),
        (UInt64Scalar, UInt64Type),
        (Int8Serie, Int8SerieType),
        (Int16Serie, Int16SerieType),
        (Int32Serie, Int32SerieType),
        (Int64Serie, Int64SerieType),
        (UInt8Serie, UInt8SerieType),
        (UInt16Serie, UInt16SerieType),
        (UInt32Serie, UInt32SerieType),
        (UInt64Serie, UInt64SerieType),
    );
    if let Ok(handle) = value.downcast::<crate::scalar::RecordScalar>() {
        let inner = handle.borrow().inner.data_type().clone();
        return Ok(Py::new(py, crate::dtype::StructType { inner })?.into_any());
    }
    dtype_of!(
        field,
        (NullField, NullType),
        (BinaryField, BinaryType),
        (Int8Field, Int8Type),
        (Int16Field, Int16Type),
        (Int32Field, Int32Type),
        (Int64Field, Int64Type),
        (UInt8Field, UInt8Type),
        (UInt16Field, UInt16Type),
        (UInt32Field, UInt32Type),
        (UInt64Field, UInt64Type),
        (Int8SerieField, Int8SerieType),
        (Int16SerieField, Int16SerieType),
        (Int32SerieField, Int32SerieType),
        (Int64SerieField, Int64SerieType),
        (UInt8SerieField, UInt8SerieType),
        (UInt16SerieField, UInt16SerieType),
        (UInt32SerieField, UInt32SerieType),
        (UInt64SerieField, UInt64SerieType),
    );
    if let Ok(handle) = value.downcast::<crate::field::StructField>() {
        let inner = handle.borrow().inner.data_type().clone();
        return Ok(Py::new(py, crate::dtype::StructType { inner })?.into_any());
    }
    Ok(match infer(value)? {
        Inferred::Null => Py::new(py, crate::dtype::NullType::default())?.into_any(),
        Inferred::Int64(_) => Py::new(py, crate::dtype::Int64Type::default())?.into_any(),
        Inferred::Binary(_) => Py::new(py, crate::dtype::BinaryType::default())?.into_any(),
        Inferred::Serie(_) => Py::new(py, crate::dtype::Int64SerieType::default())?.into_any(),
        Inferred::Record(entries) => Py::new(
            py,
            crate::dtype::StructType {
                inner: struct_type_of(&entries),
            },
        )?
        .into_any(),
    })
}

/// Infer the data type from `value` and build the matching `yggdryl.field` named
/// `name`: a native value names its type (a dict a `StructField`), and a data
/// type or scalar object of the model pairs `name` with its field.
#[pyfunction]
#[pyo3(signature = (name, value, nullable = true))]
fn field(
    py: Python<'_>,
    name: String,
    value: &Bound<'_, PyAny>,
    nullable: bool,
) -> PyResult<PyObject> {
    // A model data type object: its field.
    if value.downcast::<crate::dtype::NullType>().is_ok() {
        return Ok(Py::new(
            py,
            crate::field::NullField {
                inner: yggdryl_field::NullField::new(name, nullable),
            },
        )?
        .into_any());
    }
    if let Ok(handle) = value.downcast::<crate::dtype::StructType>() {
        let inner = yggdryl_field::StructField::new(name, handle.borrow().inner.clone(), nullable);
        return Ok(Py::new(py, crate::field::StructField { inner })?.into_any());
    }
    macro_rules! field_of_dtype {
        ($(($dtype:ident, $field:ident)),+ $(,)?) => {
            $(if let Ok(handle) = value.downcast::<crate::dtype::$dtype>() {
                let inner = handle.borrow().inner.field(name, nullable);
                return Ok(Py::new(py, crate::field::$field { inner })?.into_any());
            })+
        };
    }
    field_of_dtype!(
        (BinaryType, BinaryField),
        (Int8Type, Int8Field),
        (Int16Type, Int16Field),
        (Int32Type, Int32Field),
        (Int64Type, Int64Field),
        (UInt8Type, UInt8Field),
        (UInt16Type, UInt16Field),
        (UInt32Type, UInt32Field),
        (UInt64Type, UInt64Field),
        (Int8SerieType, Int8SerieField),
        (Int16SerieType, Int16SerieField),
        (Int32SerieType, Int32SerieField),
        (Int64SerieType, Int64SerieField),
        (UInt8SerieType, UInt8SerieField),
        (UInt16SerieType, UInt16SerieField),
        (UInt32SerieType, UInt32SerieField),
        (UInt64SerieType, UInt64SerieField),
    );
    // A model scalar object: the field of its data type.
    if value.downcast::<crate::scalar::NullScalar>().is_ok() {
        return Ok(Py::new(
            py,
            crate::field::NullField {
                inner: yggdryl_field::NullField::new(name, nullable),
            },
        )?
        .into_any());
    }
    if let Ok(handle) = value.downcast::<crate::scalar::RecordScalar>() {
        let inner = yggdryl_field::StructField::new(
            name,
            handle.borrow().inner.data_type().clone(),
            nullable,
        );
        return Ok(Py::new(py, crate::field::StructField { inner })?.into_any());
    }
    macro_rules! field_of_scalar {
        ($(($scalar:ident, $field:ident)),+ $(,)?) => {
            $(if let Ok(handle) = value.downcast::<crate::scalar::$scalar>() {
                let inner = handle.borrow().inner.data_type().field(name, nullable);
                return Ok(Py::new(py, crate::field::$field { inner })?.into_any());
            })+
        };
    }
    field_of_scalar!(
        (BinaryScalar, BinaryField),
        (Int8Scalar, Int8Field),
        (Int16Scalar, Int16Field),
        (Int32Scalar, Int32Field),
        (Int64Scalar, Int64Field),
        (UInt8Scalar, UInt8Field),
        (UInt16Scalar, UInt16Field),
        (UInt32Scalar, UInt32Field),
        (UInt64Scalar, UInt64Field),
        (Int8Serie, Int8SerieField),
        (Int16Serie, Int16SerieField),
        (Int32Serie, Int32SerieField),
        (Int64Serie, Int64SerieField),
        (UInt8Serie, UInt8SerieField),
        (UInt16Serie, UInt16SerieField),
        (UInt32Serie, UInt32SerieField),
        (UInt64Serie, UInt64SerieField),
    );
    Ok(match infer(value)? {
        Inferred::Null => Py::new(
            py,
            crate::field::NullField {
                inner: yggdryl_field::NullField::new(name, nullable),
            },
        )?
        .into_any(),
        Inferred::Int64(_) => Py::new(
            py,
            crate::field::Int64Field {
                inner: yggdryl_field::Int64Field::new(name, nullable),
            },
        )?
        .into_any(),
        Inferred::Binary(_) => Py::new(
            py,
            crate::field::BinaryField {
                inner: yggdryl_field::BinaryField::new(name, nullable),
            },
        )?
        .into_any(),
        Inferred::Serie(_) => Py::new(
            py,
            crate::field::Int64SerieField {
                inner: yggdryl_field::TypedSerieField::new(name, nullable),
            },
        )?
        .into_any(),
        Inferred::Record(entries) => Py::new(
            py,
            crate::field::StructField {
                inner: yggdryl_field::StructField::new(name, struct_type_of(&entries), nullable),
            },
        )?
        .into_any(),
    })
}

/// Populates the `yggdryl.factory` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(scalar, module)?)?;
    module.add_function(wrap_pyfunction!(dtype, module)?)?;
    module.add_function(wrap_pyfunction!(field, module)?)?;
    Ok(())
}
