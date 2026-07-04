//! The `yggdryl.field` namespace — thin wrappers over the `yggdryl-field` crate.
//!
//! Every integer type is exposed as its field and its optional field (e.g.
//! `Int64Field`, `OptionalInt64Field`), alongside `BinaryField` /
//! `OptionalBinaryField`, `NullField`, `UnionField` and its concrete serie field
//! (e.g. `Int64SerieField`, a column of `Int64SerieType`) — the same
//! globally-unique names as the Rust crate, the namespace carrying the concern (the `…Field`
//! suffix keeps every class distinct in napi's addon-global registry). A field
//! pairs a name with its `yggdryl.dtype` data type and a nullability flag (`true`
//! by default, as an `Option<bool>` default).
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow` / `from_arrow`, and `castDtype` which returns a re-typed
//! `arrow-schema` field — all exchange `arrow-schema` values that cannot cross
//! the FFI boundary; C Data Interface interop is future work) and the dynamic base
//! and typed nested fields (`SerieField` / `TypedSerieField` over a non-integer
//! value type, `MapField` / `TypedMapField`, `StructField`), which have no concrete
//! FFI shape yet.

use napi_derive::napi;
use yggdryl_field::Field;

/// A nullable `union` field: a name paired with a `yggdryl.dtype.UnionType` data
/// type.
#[napi(namespace = "field")]
pub struct UnionField {
    pub(crate) inner: yggdryl_field::UnionField,
}

#[napi(namespace = "field")]
impl UnionField {
    /// A field named `name` of the union type `dataType` (nullable by default).
    #[napi(constructor)]
    pub fn new(name: String, data_type: &crate::dtype::UnionType, nullable: Option<bool>) -> Self {
        Self {
            inner: yggdryl_field::UnionField::new(
                name,
                data_type.inner.clone(),
                nullable.unwrap_or(true),
            ),
        }
    }

    /// The field's name.
    #[napi]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    #[napi]
    pub fn data_type(&self) -> crate::dtype::UnionType {
        crate::dtype::UnionType {
            inner: self.inner.data_type().clone(),
        }
    }

    /// Whether values in this field may be null.
    #[napi]
    pub fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// A `null` field: a name paired with the null data type.
#[napi(namespace = "field")]
pub struct NullField {
    pub(crate) inner: yggdryl_field::NullField,
}

#[napi(namespace = "field")]
impl NullField {
    /// A `null` field named `name` (nullable by default).
    #[napi(constructor)]
    pub fn new(name: String, nullable: Option<bool>) -> Self {
        Self {
            inner: yggdryl_field::NullField::new(name, nullable.unwrap_or(true)),
        }
    }

    /// The field's name.
    #[napi]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    #[napi]
    pub fn data_type(&self) -> crate::dtype::NullType {
        crate::dtype::NullType::default()
    }

    /// Whether values in this field may be null.
    #[napi]
    pub fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// A nullable `binary` field: a name paired with the data type.
#[napi(namespace = "field")]
pub struct BinaryField {
    pub(crate) inner: yggdryl_field::BinaryField,
}

#[napi(namespace = "field")]
impl BinaryField {
    /// A `binary` field named `name` (nullable by default).
    #[napi(constructor)]
    pub fn new(name: String, nullable: Option<bool>) -> Self {
        Self {
            inner: yggdryl_field::BinaryField::new(name, nullable.unwrap_or(true)),
        }
    }

    /// The field's name.
    #[napi]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    #[napi]
    pub fn data_type(&self) -> crate::dtype::BinaryType {
        crate::dtype::BinaryType::default()
    }

    /// Whether values in this field may be null.
    #[napi]
    pub fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// A nullable optional-`binary` field: a name paired with the logical optional
/// data type.
#[napi(namespace = "field")]
pub struct OptionalBinaryField {
    pub(crate) inner: yggdryl_field::TypedOptionalField<yggdryl_dtype::BinaryType>,
}

#[napi(namespace = "field")]
impl OptionalBinaryField {
    /// An optional-`binary` field named `name` (nullable by default).
    #[napi(constructor)]
    pub fn new(name: String, nullable: Option<bool>) -> Self {
        Self {
            inner: yggdryl_field::TypedOptionalField::new(name, nullable.unwrap_or(true)),
        }
    }

    /// The field's name.
    #[napi]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    #[napi]
    pub fn data_type(&self) -> crate::dtype::OptionalBinaryType {
        crate::dtype::OptionalBinaryType::default()
    }

    /// Whether values in this field may be null.
    #[napi]
    pub fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// Generates the two field wrappers of one integer type: the field `$ty` and the
/// optional field `$opt_ty` — each a thin delegation to the `yggdryl-field`
/// types.
macro_rules! int_field_node {
    ($ty:ident, $opt_ty:ident, $dtype:ident, $opt_dtype:ident, $name:literal) => {
        #[doc = concat!("A nullable `", $name, "` field: a name paired with the data type.")]
        #[napi(namespace = "field")]
        pub struct $ty {
            pub(crate) inner: yggdryl_field::$ty,
        }

        #[napi(namespace = "field")]
        impl $ty {
            #[doc = concat!("A `", $name, "` field named `name` (nullable by default).")]
            #[napi(constructor)]
            pub fn new(name: String, nullable: Option<bool>) -> Self {
                Self {
                    inner: yggdryl_field::$ty::new(name, nullable.unwrap_or(true)),
                }
            }

            /// The field's name.
            #[napi]
            pub fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The field's data type.
            #[napi]
            pub fn data_type(&self) -> crate::dtype::$dtype {
                crate::dtype::$dtype::default()
            }

            /// Whether values in this field may be null.
            #[napi]
            pub fn is_nullable(&self) -> bool {
                self.inner.is_nullable()
            }
        }

        #[doc = concat!("A nullable optional-`", $name, "` field: a name paired with the logical optional data type.")]
        #[napi(namespace = "field")]
        pub struct $opt_ty {
            pub(crate) inner: yggdryl_field::TypedOptionalField<yggdryl_dtype::$dtype>,
        }

        #[napi(namespace = "field")]
        impl $opt_ty {
            #[doc = concat!("An optional-`", $name, "` field named `name` (nullable by default).")]
            #[napi(constructor)]
            pub fn new(name: String, nullable: Option<bool>) -> Self {
                Self {
                    inner: yggdryl_field::TypedOptionalField::new(name, nullable.unwrap_or(true)),
                }
            }

            /// The field's name.
            #[napi]
            pub fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The field's data type.
            #[napi]
            pub fn data_type(&self) -> crate::dtype::$opt_dtype {
                crate::dtype::$opt_dtype::default()
            }

            /// Whether values in this field may be null.
            #[napi]
            pub fn is_nullable(&self) -> bool {
                self.inner.is_nullable()
            }
        }
    };
}

int_field_node!(
    Int8Field,
    OptionalInt8Field,
    Int8Type,
    OptionalInt8Type,
    "int8"
);
int_field_node!(
    Int16Field,
    OptionalInt16Field,
    Int16Type,
    OptionalInt16Type,
    "int16"
);
int_field_node!(
    Int32Field,
    OptionalInt32Field,
    Int32Type,
    OptionalInt32Type,
    "int32"
);
int_field_node!(
    Int64Field,
    OptionalInt64Field,
    Int64Type,
    OptionalInt64Type,
    "int64"
);
int_field_node!(
    UInt8Field,
    OptionalUInt8Field,
    UInt8Type,
    OptionalUInt8Type,
    "uint8"
);
int_field_node!(
    UInt16Field,
    OptionalUInt16Field,
    UInt16Type,
    OptionalUInt16Type,
    "uint16"
);
int_field_node!(
    UInt32Field,
    OptionalUInt32Field,
    UInt32Type,
    OptionalUInt32Type,
    "uint32"
);
int_field_node!(
    UInt64Field,
    OptionalUInt64Field,
    UInt64Type,
    OptionalUInt64Type,
    "uint64"
);

/// Generates the concrete serie field of one integer value type: `$ty`, a column
/// of the `yggdryl.dtype` class `$dtype` — a thin delegation to
/// `yggdryl_field::TypedSerieField<$value_ty>`.
macro_rules! int_serie_field_node {
    ($ty:ident, $dtype:ident, $value_ty:ident, $name:literal) => {
        /// A nullable serie field: a name paired with the serie data type.
        #[doc = concat!("This is the `list`-of-`", $name, "` column (`", stringify!($dtype), "`).")]
        #[napi(namespace = "field")]
        pub struct $ty {
            pub(crate) inner: yggdryl_field::TypedSerieField<yggdryl_dtype::$value_ty>,
        }

        #[napi(namespace = "field")]
        impl $ty {
            /// A serie field named `name` (nullable by default).
            #[napi(constructor)]
            pub fn new(name: String, nullable: Option<bool>) -> Self {
                Self {
                    inner: yggdryl_field::TypedSerieField::new(name, nullable.unwrap_or(true)),
                }
            }

            /// The field's name.
            #[napi]
            pub fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The field's data type.
            #[napi]
            pub fn data_type(&self) -> crate::dtype::$dtype {
                crate::dtype::$dtype::default()
            }

            /// Whether values in this field may be null.
            #[napi]
            pub fn is_nullable(&self) -> bool {
                self.inner.is_nullable()
            }
        }
    };
}

int_serie_field_node!(Int8SerieField, Int8SerieType, Int8Type, "int8");
int_serie_field_node!(Int16SerieField, Int16SerieType, Int16Type, "int16");
int_serie_field_node!(Int32SerieField, Int32SerieType, Int32Type, "int32");
int_serie_field_node!(Int64SerieField, Int64SerieType, Int64Type, "int64");
int_serie_field_node!(UInt8SerieField, UInt8SerieType, UInt8Type, "uint8");
int_serie_field_node!(UInt16SerieField, UInt16SerieType, UInt16Type, "uint16");
int_serie_field_node!(UInt32SerieField, UInt32SerieType, UInt32Type, "uint32");
int_serie_field_node!(UInt64SerieField, UInt64SerieType, UInt64Type, "uint64");
