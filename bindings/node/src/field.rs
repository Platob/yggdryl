//! The `yggdryl.field` namespace — thin wrappers over the `yggdryl-field` crate.
//!
//! Every integer type is exposed as its field and its optional field (e.g.
//! `Int64`, `OptionalInt64`), alongside `Binary` / `OptionalBinary`, `Null` and
//! `Union` — the same bare names as the Rust crate, the namespace carrying the
//! concern. A field pairs a name with its `yggdryl.dtype` data type and a
//! nullability flag (`true` by default, as an `Option<bool>` default).
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow` / `from_arrow` exchange `arrow-schema` values that cannot cross
//! the FFI boundary; C Data Interface interop is future work) and the generic
//! nested fields (`List` / `Map` / `Struct`), which have no concrete FFI shape
//! yet.

use napi_derive::napi;
use yggdryl_field::RawField;

/// A nullable `union` field: a name paired with a `yggdryl.dtype.Union` data type.
#[napi]
pub struct FieldUnion {
    pub(crate) inner: yggdryl_field::Union,
}

#[napi]
impl FieldUnion {
    /// A field named `name` of the union type `dataType` (nullable by default).
    #[napi(constructor)]
    pub fn new(name: String, data_type: &crate::dtype::DtypeUnion, nullable: Option<bool>) -> Self {
        Self {
            inner: yggdryl_field::Union::new(
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
    pub fn data_type(&self) -> crate::dtype::DtypeUnion {
        crate::dtype::DtypeUnion {
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
#[napi]
pub struct FieldNull {
    pub(crate) inner: yggdryl_field::Null,
}

#[napi]
impl FieldNull {
    /// A `null` field named `name` (nullable by default).
    #[napi(constructor)]
    pub fn new(name: String, nullable: Option<bool>) -> Self {
        Self {
            inner: yggdryl_field::Null::new(name, nullable.unwrap_or(true)),
        }
    }

    /// The field's name.
    #[napi]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    #[napi]
    pub fn data_type(&self) -> crate::dtype::DtypeNull {
        crate::dtype::DtypeNull::default()
    }

    /// Whether values in this field may be null.
    #[napi]
    pub fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// A nullable `binary` field: a name paired with the data type.
#[napi]
pub struct FieldBinary {
    pub(crate) inner: yggdryl_field::Binary,
}

#[napi]
impl FieldBinary {
    /// A `binary` field named `name` (nullable by default).
    #[napi(constructor)]
    pub fn new(name: String, nullable: Option<bool>) -> Self {
        Self {
            inner: yggdryl_field::Binary::new(name, nullable.unwrap_or(true)),
        }
    }

    /// The field's name.
    #[napi]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    #[napi]
    pub fn data_type(&self) -> crate::dtype::DtypeBinary {
        crate::dtype::DtypeBinary::default()
    }

    /// Whether values in this field may be null.
    #[napi]
    pub fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// A nullable optional-`binary` field: a name paired with the logical optional
/// data type.
#[napi]
pub struct FieldOptionalBinary {
    pub(crate) inner: yggdryl_field::Optional<yggdryl_dtype::Binary>,
}

#[napi]
impl FieldOptionalBinary {
    /// An optional-`binary` field named `name` (nullable by default).
    #[napi(constructor)]
    pub fn new(name: String, nullable: Option<bool>) -> Self {
        Self {
            inner: yggdryl_field::Optional::new(name, nullable.unwrap_or(true)),
        }
    }

    /// The field's name.
    #[napi]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    #[napi]
    pub fn data_type(&self) -> crate::dtype::DtypeOptionalBinary {
        crate::dtype::DtypeOptionalBinary::default()
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
    ($ty:ident, $opt_ty:ident, $inner:ident, $dtype:ident, $opt_dtype:ident, $name:literal) => {
        #[doc = concat!("A nullable `", $name, "` field: a name paired with the data type.")]
        #[napi]
        pub struct $ty {
            pub(crate) inner: yggdryl_field::$inner,
        }

        #[napi]
        impl $ty {
            #[doc = concat!("A `", $name, "` field named `name` (nullable by default).")]
            #[napi(constructor)]
            pub fn new(name: String, nullable: Option<bool>) -> Self {
                Self {
                    inner: yggdryl_field::$inner::new(name, nullable.unwrap_or(true)),
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
        #[napi]
        pub struct $opt_ty {
            pub(crate) inner: yggdryl_field::Optional<yggdryl_dtype::$inner>,
        }

        #[napi]
        impl $opt_ty {
            #[doc = concat!("An optional-`", $name, "` field named `name` (nullable by default).")]
            #[napi(constructor)]
            pub fn new(name: String, nullable: Option<bool>) -> Self {
                Self {
                    inner: yggdryl_field::Optional::new(name, nullable.unwrap_or(true)),
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
    FieldInt8,
    FieldOptionalInt8,
    Int8,
    DtypeInt8,
    DtypeOptionalInt8,
    "int8"
);
int_field_node!(
    FieldInt16,
    FieldOptionalInt16,
    Int16,
    DtypeInt16,
    DtypeOptionalInt16,
    "int16"
);
int_field_node!(
    FieldInt32,
    FieldOptionalInt32,
    Int32,
    DtypeInt32,
    DtypeOptionalInt32,
    "int32"
);
int_field_node!(
    FieldInt64,
    FieldOptionalInt64,
    Int64,
    DtypeInt64,
    DtypeOptionalInt64,
    "int64"
);
int_field_node!(
    FieldUInt8,
    FieldOptionalUInt8,
    UInt8,
    DtypeUInt8,
    DtypeOptionalUInt8,
    "uint8"
);
int_field_node!(
    FieldUInt16,
    FieldOptionalUInt16,
    UInt16,
    DtypeUInt16,
    DtypeOptionalUInt16,
    "uint16"
);
int_field_node!(
    FieldUInt32,
    FieldOptionalUInt32,
    UInt32,
    DtypeUInt32,
    DtypeOptionalUInt32,
    "uint32"
);
int_field_node!(
    FieldUInt64,
    FieldOptionalUInt64,
    UInt64,
    DtypeUInt64,
    DtypeOptionalUInt64,
    "uint64"
);
