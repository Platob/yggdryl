//! The `yggdryl.types` namespace — the **typed-data schema layer**: [`DataType`] (a runtime type
//! descriptor with the category drill-down) and [`Field`] (a named, nullable column descriptor).
//! A field's metadata is the centralized [`Headers`](crate::headers::Headers) map (in
//! `yggdryl.io`) — there is no separate `Metadata` type.
//!
//! Mirrors `yggdryl_core::io`'s `DataType` / `DataTypeId` drill-down and the erased
//! [`Field`](yggdryl_core::io::fixed::Field). Each method is one or two lines over the core; the
//! category predicates reduce to cheap integer range checks on the core's `DataTypeId`.

use napi::bindgen_prelude::Object;
use napi_derive::napi;

use yggdryl_core::io::fixed::Field as CoreField;
use yggdryl_core::io::{DataTypeCategory, DataTypeId, FieldType};

use crate::headers::{core_headers, Headers};

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

/// A runtime **data-type descriptor** — the type's `name`, `byteWidth`, coarse `category`, and
/// the category drill-down predicates (`isInteger()` … answer with a couple of integer
/// comparisons, no matching). Construct one with a factory (`DataType.i32()`, `DataType.utf8()`,
/// `DataType.fixedBinary(16)`, or `DataType.byName("u96")`).
#[napi(namespace = "types")]
#[derive(Clone)]
pub struct DataType {
    id: DataTypeId,
    byte_width: u32,
}

impl DataType {
    pub(crate) fn of(id: DataTypeId) -> Self {
        Self {
            id,
            byte_width: id.fixed_byte_width().unwrap_or(0) as u32,
        }
    }
}

#[napi(namespace = "types")]
impl DataType {
    // Named factories for every intrinsic-width type (`byName` covers them generically too).
    #[napi(factory)]
    pub fn u8() -> Self {
        Self::of(DataTypeId::U8)
    }
    #[napi(factory)]
    pub fn u16() -> Self {
        Self::of(DataTypeId::U16)
    }
    #[napi(factory)]
    pub fn u32() -> Self {
        Self::of(DataTypeId::U32)
    }
    #[napi(factory)]
    pub fn u64() -> Self {
        Self::of(DataTypeId::U64)
    }
    #[napi(factory)]
    pub fn u96() -> Self {
        Self::of(DataTypeId::U96)
    }
    #[napi(factory)]
    pub fn u128() -> Self {
        Self::of(DataTypeId::U128)
    }
    #[napi(factory)]
    pub fn u256() -> Self {
        Self::of(DataTypeId::U256)
    }
    #[napi(factory)]
    pub fn i8() -> Self {
        Self::of(DataTypeId::I8)
    }
    #[napi(factory)]
    pub fn i16() -> Self {
        Self::of(DataTypeId::I16)
    }
    #[napi(factory)]
    pub fn i32() -> Self {
        Self::of(DataTypeId::I32)
    }
    #[napi(factory)]
    pub fn i64() -> Self {
        Self::of(DataTypeId::I64)
    }
    #[napi(factory)]
    pub fn i96() -> Self {
        Self::of(DataTypeId::I96)
    }
    #[napi(factory)]
    pub fn i128() -> Self {
        Self::of(DataTypeId::I128)
    }
    #[napi(factory)]
    pub fn i256() -> Self {
        Self::of(DataTypeId::I256)
    }
    #[napi(factory)]
    pub fn f16() -> Self {
        Self::of(DataTypeId::F16)
    }
    #[napi(factory)]
    pub fn f32() -> Self {
        Self::of(DataTypeId::F32)
    }
    #[napi(factory)]
    pub fn f64() -> Self {
        Self::of(DataTypeId::F64)
    }
    #[napi(factory)]
    pub fn d32() -> Self {
        Self::of(DataTypeId::D32)
    }
    #[napi(factory)]
    pub fn d64() -> Self {
        Self::of(DataTypeId::D64)
    }
    #[napi(factory)]
    pub fn d128() -> Self {
        Self::of(DataTypeId::D128)
    }
    #[napi(factory)]
    pub fn d256() -> Self {
        Self::of(DataTypeId::D256)
    }
    #[napi(factory)]
    pub fn date32() -> Self {
        Self::of(DataTypeId::Date32)
    }
    #[napi(factory)]
    pub fn date64() -> Self {
        Self::of(DataTypeId::Date64)
    }
    #[napi(factory)]
    pub fn time32() -> Self {
        Self::of(DataTypeId::Time32)
    }
    #[napi(factory)]
    pub fn time64() -> Self {
        Self::of(DataTypeId::Time64)
    }
    #[napi(factory)]
    pub fn ts32() -> Self {
        Self::of(DataTypeId::Ts32)
    }
    #[napi(factory)]
    pub fn ts64() -> Self {
        Self::of(DataTypeId::Ts64)
    }
    #[napi(factory)]
    pub fn ts96() -> Self {
        Self::of(DataTypeId::Ts96)
    }
    #[napi(factory)]
    pub fn duration32() -> Self {
        Self::of(DataTypeId::Duration32)
    }
    #[napi(factory)]
    pub fn duration64() -> Self {
        Self::of(DataTypeId::Duration64)
    }
    #[napi(factory)]
    pub fn utf8() -> Self {
        Self::of(DataTypeId::Utf8)
    }
    #[napi(factory)]
    pub fn large_utf8() -> Self {
        Self::of(DataTypeId::LargeUtf8)
    }
    #[napi(factory)]
    pub fn binary() -> Self {
        Self::of(DataTypeId::Binary)
    }
    #[napi(factory)]
    pub fn large_binary() -> Self {
        Self::of(DataTypeId::LargeBinary)
    }
    #[napi(factory)]
    pub fn null() -> Self {
        Self::of(DataTypeId::Null)
    }

    /// A **fixed-size binary** type — every value is exactly `width` bytes.
    #[napi(factory)]
    pub fn fixed_binary(width: u32) -> Self {
        Self {
            id: DataTypeId::FixedBinary,
            byte_width: width,
        }
    }

    /// A **fixed-size UTF-8** type — every value is exactly `width` bytes and valid UTF-8.
    #[napi(factory)]
    pub fn fixed_utf8(width: u32) -> Self {
        Self {
            id: DataTypeId::FixedUtf8,
            byte_width: width,
        }
    }

    /// The type for a canonical name (`"u8"`, `"i256"`, `"fixed_utf8"`, …). The fixed-size byte
    /// types take their width from `width` (default `0`); others ignore it. Throws for an unknown
    /// name.
    #[napi(factory)]
    pub fn by_name(name: String, width: Option<u32>) -> napi::Result<Self> {
        let id = DataTypeId::from_name(&name)
            .ok_or_else(|| napi::Error::from_reason(format!("unknown data type name: {name:?}")))?;
        Ok(Self {
            id,
            byte_width: id
                .fixed_byte_width()
                .map(|w| w as u32)
                .unwrap_or(width.unwrap_or(0)),
        })
    }

    /// The stable, lower-case type name (e.g. `"i32"`, `"u96"`, `"fixed_utf8"`).
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.id.name().to_string()
    }

    /// The fixed byte width — one value for the numeric types, the offset width for the
    /// variable-length types, or `N` for the fixed-size byte types.
    #[napi(getter)]
    pub fn byte_width(&self) -> u32 {
        self.byte_width
    }

    /// The coarse category as a string (`"unsigned_integer"`, `"float"`, `"utf8"`, …).
    #[napi(getter)]
    pub fn category(&self) -> String {
        category_name(self.id.category()).to_string()
    }

    /// A [`Field`] of this type with the given name and nullability (default nullable).
    #[napi]
    pub fn field(&self, name: String, nullable: Option<bool>) -> Field {
        Field {
            inner: CoreField::of(
                &name,
                self.id,
                self.byte_width as usize,
                nullable.unwrap_or(true),
            ),
        }
    }

    // ---- category drill-down (each a couple of integer comparisons) --------------------
    #[napi]
    pub fn is_integer(&self) -> bool {
        self.id.is_integer()
    }
    #[napi]
    pub fn is_unsigned_integer(&self) -> bool {
        self.id.is_unsigned_integer()
    }
    #[napi]
    pub fn is_signed_integer(&self) -> bool {
        self.id.is_signed_integer()
    }
    #[napi]
    pub fn is_signed(&self) -> bool {
        self.id.is_signed()
    }
    #[napi]
    pub fn is_floating(&self) -> bool {
        self.id.is_floating()
    }
    #[napi]
    pub fn is_decimal(&self) -> bool {
        self.id.is_decimal()
    }
    #[napi]
    pub fn is_temporal(&self) -> bool {
        self.id.is_temporal()
    }
    #[napi]
    pub fn is_numeric(&self) -> bool {
        self.id.is_numeric()
    }
    #[napi]
    pub fn is_utf8(&self) -> bool {
        self.id.is_utf8()
    }
    #[napi]
    pub fn is_binary(&self) -> bool {
        self.id.is_binary()
    }
    #[napi]
    pub fn is_fixed_width(&self) -> bool {
        self.id.is_fixed_width()
    }
    #[napi]
    pub fn is_variable_length(&self) -> bool {
        self.id.is_variable_length()
    }
    #[napi]
    pub fn is_null(&self) -> bool {
        self.id.is_null()
    }

    /// Value equality (same logical type and width).
    #[napi]
    pub fn equals(&self, other: &DataType) -> bool {
        self.id == other.id && self.byte_width == other.byte_width
    }

    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        match self.id {
            DataTypeId::FixedBinary | DataTypeId::FixedUtf8 => {
                format!("DataType({}[{}])", self.id.name(), self.byte_width)
            }
            _ => format!("DataType({})", self.id.name()),
        }
    }
}

/// A named, nullable **column descriptor** — a name, its [`DataType`], whether it admits nulls,
/// and a [`Headers`] metadata map. A value type: it compares by content (metadata included).
#[napi(namespace = "types")]
#[derive(Clone)]
pub struct Field {
    pub(crate) inner: CoreField,
}

#[napi(namespace = "types")]
impl Field {
    /// Builds a field from a name, a [`DataType`], its nullability (default `true`), and optional
    /// metadata (a `Record<string, string>`).
    #[napi(constructor)]
    pub fn new(
        name: String,
        data_type: &DataType,
        nullable: Option<bool>,
        metadata: Option<Object>,
    ) -> napi::Result<Self> {
        Ok(Self {
            inner: CoreField::of(
                &name,
                data_type.id,
                data_type.byte_width as usize,
                nullable.unwrap_or(true),
            )
            .with_metadata(core_headers(metadata)?),
        })
    }

    /// The column name.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The column's [`DataType`].
    #[napi(getter)]
    pub fn data_type(&self) -> DataType {
        DataType {
            id: FieldType::type_id(&self.inner),
            byte_width: self.inner.byte_width() as u32,
        }
    }

    /// The element type's name (e.g. `"i64"`).
    #[napi(getter)]
    pub fn type_name(&self) -> String {
        self.inner.type_name().to_string()
    }

    /// The element type's byte width.
    #[napi(getter)]
    pub fn byte_width(&self) -> u32 {
        self.inner.byte_width() as u32
    }

    /// Whether the column admits nulls.
    #[napi(getter)]
    pub fn nullable(&self) -> bool {
        self.inner.nullable()
    }

    /// A copy of the field's metadata [`Headers`].
    #[napi(getter)]
    pub fn metadata(&self) -> Headers {
        Headers {
            inner: self.inner.metadata().clone(),
        }
    }

    /// A fresh field with the given metadata (a `Record<string, string>`) attached.
    #[napi]
    pub fn with_metadata(&self, metadata: Object) -> napi::Result<Self> {
        Ok(Self {
            inner: self
                .inner
                .clone()
                .with_metadata(core_headers(Some(metadata))?),
        })
    }

    /// A fresh field with one extra `key = value` metadata entry.
    #[napi]
    pub fn with_metadata_entry(&self, key: String, value: String) -> Self {
        Self {
            inner: self.inner.clone().with_metadata_entry(&key, &value),
        }
    }

    // ---- category drill-down (mirrored from the element type) --------------------------
    #[napi]
    pub fn is_integer(&self) -> bool {
        self.inner.is_integer()
    }
    #[napi]
    pub fn is_unsigned_integer(&self) -> bool {
        self.inner.is_unsigned_integer()
    }
    #[napi]
    pub fn is_signed_integer(&self) -> bool {
        self.inner.is_signed_integer()
    }
    #[napi]
    pub fn is_signed(&self) -> bool {
        self.inner.is_signed()
    }
    #[napi]
    pub fn is_floating(&self) -> bool {
        self.inner.is_floating()
    }
    #[napi]
    pub fn is_decimal(&self) -> bool {
        self.inner.is_decimal()
    }
    #[napi]
    pub fn is_temporal(&self) -> bool {
        self.inner.is_temporal()
    }
    #[napi]
    pub fn is_numeric(&self) -> bool {
        self.inner.is_numeric()
    }
    #[napi]
    pub fn is_utf8(&self) -> bool {
        self.inner.is_utf8()
    }
    #[napi]
    pub fn is_binary(&self) -> bool {
        self.inner.is_binary()
    }
    #[napi]
    pub fn is_fixed_width(&self) -> bool {
        self.inner.is_fixed_width()
    }
    #[napi]
    pub fn is_variable_length(&self) -> bool {
        self.inner.is_variable_length()
    }

    /// An explicit copy.
    #[napi]
    pub fn copy(&self) -> Self {
        self.clone()
    }

    /// Value equality (name, type, nullability, and metadata all match).
    #[napi]
    pub fn equals(&self, other: &Field) -> bool {
        self.inner == other.inner
    }

    #[napi(js_name = "toString")]
    pub fn text(&self) -> String {
        format!(
            "Field(name={:?}, type={}, nullable={})",
            self.inner.name(),
            self.inner.type_name(),
            self.inner.nullable()
        )
    }
}
