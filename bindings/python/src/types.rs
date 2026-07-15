//! The `yggdryl.types` submodule — the **typed-data schema layer**: [`DataType`] (a runtime type
//! descriptor with the category drill-down) and [`Field`] (a named, nullable column descriptor).
//! A field's metadata is the centralized [`Headers`](crate::headers::Headers) map (in
//! `yggdryl.io`) — there is no separate `Metadata` type.
//!
//! Mirrors `yggdryl_core::io`'s `DataType` / `DataTypeId` drill-down and the erased
//! [`Field`](yggdryl_core::io::fixed::Field). Each method is one or two lines over the core; the
//! category predicates reduce to cheap integer range checks on the core's `DataTypeId`.

// pyo3's `#[pymethods]` expansion wraps fallible returns in a same-type `From`.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use yggdryl_core::io::fixed::Field as CoreField;
use yggdryl_core::io::{DataTypeCategory, DataTypeId, FieldType};

use crate::headers::Headers;

/// The lower-case name of a coarse category, mirroring `DataTypeCategory`.
fn category_name(category: DataTypeCategory) -> &'static str {
    match category {
        DataTypeCategory::Null => "null",
        DataTypeCategory::UnsignedInteger => "unsigned_integer",
        DataTypeCategory::SignedInteger => "signed_integer",
        DataTypeCategory::Float => "float",
        DataTypeCategory::Decimal => "decimal",
        DataTypeCategory::Temporal => "temporal",
        DataTypeCategory::Utf8 => "utf8",
        DataTypeCategory::Binary => "binary",
        _ => "unknown", // `DataTypeCategory` is `#[non_exhaustive]`
    }
}

/// A runtime **data-type descriptor** — the type's `name`, `byte_width`, coarse `category`, and
/// the category drill-down predicates (`is_integer()` … answer with a couple of integer
/// comparisons, no matching). Construct one with a factory (`DataType.i32()`, `DataType.utf8()`,
/// `DataType.fixed_binary(16)`, or `DataType.by_name("u96")`).
#[pyclass(module = "yggdryl.types")]
#[derive(Clone)]
pub struct DataType {
    id: DataTypeId,
    byte_width: usize,
}

impl DataType {
    fn of(id: DataTypeId) -> Self {
        Self {
            id,
            byte_width: id.fixed_byte_width().unwrap_or(0),
        }
    }
}

#[pymethods]
impl DataType {
    // Named factories for every intrinsic-width type (`by_name` covers them generically too).
    #[staticmethod]
    fn u8() -> Self {
        Self::of(DataTypeId::U8)
    }
    #[staticmethod]
    fn u16() -> Self {
        Self::of(DataTypeId::U16)
    }
    #[staticmethod]
    fn u32() -> Self {
        Self::of(DataTypeId::U32)
    }
    #[staticmethod]
    fn u64() -> Self {
        Self::of(DataTypeId::U64)
    }
    #[staticmethod]
    fn u96() -> Self {
        Self::of(DataTypeId::U96)
    }
    #[staticmethod]
    fn u128() -> Self {
        Self::of(DataTypeId::U128)
    }
    #[staticmethod]
    fn u256() -> Self {
        Self::of(DataTypeId::U256)
    }
    #[staticmethod]
    fn i8() -> Self {
        Self::of(DataTypeId::I8)
    }
    #[staticmethod]
    fn i16() -> Self {
        Self::of(DataTypeId::I16)
    }
    #[staticmethod]
    fn i32() -> Self {
        Self::of(DataTypeId::I32)
    }
    #[staticmethod]
    fn i64() -> Self {
        Self::of(DataTypeId::I64)
    }
    #[staticmethod]
    fn i96() -> Self {
        Self::of(DataTypeId::I96)
    }
    #[staticmethod]
    fn i128() -> Self {
        Self::of(DataTypeId::I128)
    }
    #[staticmethod]
    fn i256() -> Self {
        Self::of(DataTypeId::I256)
    }
    #[staticmethod]
    fn f16() -> Self {
        Self::of(DataTypeId::F16)
    }
    #[staticmethod]
    fn f32() -> Self {
        Self::of(DataTypeId::F32)
    }
    #[staticmethod]
    fn f64() -> Self {
        Self::of(DataTypeId::F64)
    }
    #[staticmethod]
    fn d32() -> Self {
        Self::of(DataTypeId::D32)
    }
    #[staticmethod]
    fn d64() -> Self {
        Self::of(DataTypeId::D64)
    }
    #[staticmethod]
    fn d128() -> Self {
        Self::of(DataTypeId::D128)
    }
    #[staticmethod]
    fn d256() -> Self {
        Self::of(DataTypeId::D256)
    }
    #[staticmethod]
    fn date32() -> Self {
        Self::of(DataTypeId::Date32)
    }
    #[staticmethod]
    fn date64() -> Self {
        Self::of(DataTypeId::Date64)
    }
    #[staticmethod]
    fn time32() -> Self {
        Self::of(DataTypeId::Time32)
    }
    #[staticmethod]
    fn time64() -> Self {
        Self::of(DataTypeId::Time64)
    }
    #[staticmethod]
    fn ts32() -> Self {
        Self::of(DataTypeId::Ts32)
    }
    #[staticmethod]
    fn ts64() -> Self {
        Self::of(DataTypeId::Ts64)
    }
    #[staticmethod]
    fn ts96() -> Self {
        Self::of(DataTypeId::Ts96)
    }
    #[staticmethod]
    fn duration32() -> Self {
        Self::of(DataTypeId::Duration32)
    }
    #[staticmethod]
    fn duration64() -> Self {
        Self::of(DataTypeId::Duration64)
    }
    #[staticmethod]
    fn utf8() -> Self {
        Self::of(DataTypeId::Utf8)
    }
    #[staticmethod]
    fn large_utf8() -> Self {
        Self::of(DataTypeId::LargeUtf8)
    }
    #[staticmethod]
    fn binary() -> Self {
        Self::of(DataTypeId::Binary)
    }
    #[staticmethod]
    fn large_binary() -> Self {
        Self::of(DataTypeId::LargeBinary)
    }
    #[staticmethod]
    fn null() -> Self {
        Self::of(DataTypeId::Null)
    }

    /// A **fixed-size binary** type — every value is exactly `width` bytes.
    #[staticmethod]
    fn fixed_binary(width: usize) -> Self {
        Self {
            id: DataTypeId::FixedBinary,
            byte_width: width,
        }
    }

    /// A **fixed-size UTF-8** type — every value is exactly `width` bytes and valid UTF-8.
    #[staticmethod]
    fn fixed_utf8(width: usize) -> Self {
        Self {
            id: DataTypeId::FixedUtf8,
            byte_width: width,
        }
    }

    /// The type for a canonical name (`"u8"`, `"i256"`, `"fixed_utf8"`, …). The fixed-size byte
    /// types take their width from `width` (default `0`); others ignore it. Raises `ValueError`
    /// for an unknown name.
    #[staticmethod]
    #[pyo3(signature = (name, width = 0))]
    fn by_name(name: &str, width: usize) -> PyResult<Self> {
        let id = DataTypeId::from_name(name)
            .ok_or_else(|| PyValueError::new_err(format!("unknown data type name: {name:?}")))?;
        Ok(Self {
            id,
            byte_width: id.fixed_byte_width().unwrap_or(width),
        })
    }

    /// The stable, lower-case type name (e.g. `"i32"`, `"u96"`, `"fixed_utf8"`).
    #[getter]
    fn name(&self) -> &'static str {
        self.id.name()
    }

    /// The fixed byte width — one value for the numeric types, the offset width for the
    /// variable-length types, or `N` for the fixed-size byte types.
    #[getter]
    fn byte_width(&self) -> usize {
        self.byte_width
    }

    /// The coarse category as a string (`"unsigned_integer"`, `"float"`, `"utf8"`, …).
    #[getter]
    fn category(&self) -> &'static str {
        category_name(self.id.category())
    }

    /// A [`Field`] of this type with the given name and nullability.
    #[pyo3(signature = (name, nullable = true))]
    fn field(&self, name: &str, nullable: bool) -> Field {
        Field {
            inner: CoreField::of(name, self.id, self.byte_width, nullable),
        }
    }

    // ---- category drill-down (each a couple of integer comparisons) --------------------
    fn is_integer(&self) -> bool {
        self.id.is_integer()
    }
    fn is_unsigned_integer(&self) -> bool {
        self.id.is_unsigned_integer()
    }
    fn is_signed_integer(&self) -> bool {
        self.id.is_signed_integer()
    }
    fn is_signed(&self) -> bool {
        self.id.is_signed()
    }
    fn is_floating(&self) -> bool {
        self.id.is_floating()
    }
    fn is_decimal(&self) -> bool {
        self.id.is_decimal()
    }
    fn is_temporal(&self) -> bool {
        self.id.is_temporal()
    }
    fn is_numeric(&self) -> bool {
        self.id.is_numeric()
    }
    fn is_utf8(&self) -> bool {
        self.id.is_utf8()
    }
    fn is_binary(&self) -> bool {
        self.id.is_binary()
    }
    fn is_fixed_width(&self) -> bool {
        self.id.is_fixed_width()
    }
    fn is_variable_length(&self) -> bool {
        self.id.is_variable_length()
    }
    fn is_null(&self) -> bool {
        self.id.is_null()
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.id == other.id && self.byte_width == other.byte_width
    }

    fn __hash__(&self) -> u64 {
        (self.id.as_u16() as u64) << 32 | self.byte_width as u64
    }

    /// Pickles through `DataType.by_name(name, byte_width)`.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<(Py<PyAny>, (String, usize))> {
        let ctor = py.get_type_bound::<DataType>().getattr("by_name")?.unbind();
        Ok((ctor, (self.id.name().to_string(), self.byte_width)))
    }

    fn __repr__(&self) -> String {
        match self.id {
            DataTypeId::FixedBinary | DataTypeId::FixedUtf8 => {
                format!("DataType({}[{}])", self.id.name(), self.byte_width)
            }
            _ => format!("DataType({})", self.id.name()),
        }
    }
}

/// The pickle payload of a [`Field`] — its `Field(name, data_type, nullable, metadata)`
/// constructor plus the args tuple (factored out to name the shape [`__reduce__`](Field::__reduce__)
/// returns).
type FieldReduce = (Py<PyAny>, (String, DataType, bool, Headers));

/// A named, nullable **column descriptor** — a name, its [`DataType`], whether it admits nulls,
/// and a [`Headers`] metadata map. A value type: it compares and hashes by content (metadata
/// included).
#[pyclass(module = "yggdryl.types")]
#[derive(Clone)]
pub struct Field {
    pub(crate) inner: CoreField,
}

#[pymethods]
impl Field {
    /// Builds a field from a name, a [`DataType`], its nullability (default `True`), and optional
    /// metadata (a `Headers` or a `dict[str, str]`).
    #[new]
    #[pyo3(signature = (name, data_type, nullable = true, metadata = None))]
    fn new(
        name: &str,
        data_type: &DataType,
        nullable: bool,
        metadata: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        let meta = Headers::from_py(metadata)?;
        Ok(Self {
            inner: CoreField::of(name, data_type.id, data_type.byte_width, nullable)
                .with_metadata(meta),
        })
    }

    /// The column name.
    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    /// The column's [`DataType`].
    #[getter]
    fn data_type(&self) -> DataType {
        DataType {
            id: FieldType::type_id(&self.inner),
            byte_width: self.inner.byte_width(),
        }
    }

    /// The element type's name (e.g. `"i64"`).
    #[getter]
    fn type_name(&self) -> &'static str {
        self.inner.type_name()
    }

    /// The element type's byte width.
    #[getter]
    fn byte_width(&self) -> usize {
        self.inner.byte_width()
    }

    /// Whether the column admits nulls.
    #[getter]
    fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// A copy of the field's metadata [`Headers`].
    #[getter]
    fn metadata(&self) -> Headers {
        Headers {
            inner: self.inner.metadata().clone(),
        }
    }

    /// A fresh field with the given metadata (a `Headers` or `dict[str, str]`) attached.
    fn with_metadata(&self, metadata: &Bound<'_, PyAny>) -> PyResult<Self> {
        let meta = Headers::from_py(Some(metadata))?;
        Ok(Self {
            inner: self.inner.clone().with_metadata(meta),
        })
    }

    /// A fresh field with one extra `key = value` metadata entry.
    fn with_metadata_entry(&self, key: &str, value: &str) -> Self {
        Self {
            inner: self.inner.clone().with_metadata_entry(key, value),
        }
    }

    // ---- category drill-down (mirrored from the element type) --------------------------
    fn is_integer(&self) -> bool {
        self.inner.is_integer()
    }
    fn is_unsigned_integer(&self) -> bool {
        self.inner.is_unsigned_integer()
    }
    fn is_signed_integer(&self) -> bool {
        self.inner.is_signed_integer()
    }
    fn is_signed(&self) -> bool {
        self.inner.is_signed()
    }
    fn is_floating(&self) -> bool {
        self.inner.is_floating()
    }
    fn is_decimal(&self) -> bool {
        self.inner.is_decimal()
    }
    fn is_temporal(&self) -> bool {
        self.inner.is_temporal()
    }
    fn is_numeric(&self) -> bool {
        self.inner.is_numeric()
    }
    fn is_utf8(&self) -> bool {
        self.inner.is_utf8()
    }
    fn is_binary(&self) -> bool {
        self.inner.is_binary()
    }
    fn is_fixed_width(&self) -> bool {
        self.inner.is_fixed_width()
    }
    fn is_variable_length(&self) -> bool {
        self.inner.is_variable_length()
    }

    /// An explicit copy.
    fn copy(&self) -> Self {
        self.clone()
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone() // a `Field` owns its data — no shared mutable state to deep-copy
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.inner == other.inner
    }

    fn __hash__(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.inner.hash(&mut hasher);
        hasher.finish()
    }

    /// Pickles through the `Field(name, data_type, nullable, metadata)` constructor.
    fn __reduce__(&self, py: Python<'_>) -> PyResult<FieldReduce> {
        let ctor = py.get_type_bound::<Field>().into_any().unbind();
        Ok((
            ctor,
            (
                self.inner.name().to_string(),
                self.data_type(),
                self.inner.nullable(),
                self.metadata(),
            ),
        ))
    }

    fn __repr__(&self) -> String {
        format!(
            "Field(name={:?}, type={}, nullable={})",
            self.inner.name(),
            self.inner.type_name(),
            self.inner.nullable()
        )
    }
}

/// Populates the `types` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<DataType>()?;
    module.add_class::<Field>()?;
    Ok(())
}
