//! The `yggdryl.field` submodule — thin wrappers over the `yggdryl-field` crate.
//!
//! Every integer and float type is exposed as its field and its optional field
//! (e.g. `Int64Field`, `OptionalInt64Field`, and the `float16` family), alongside
//! `BinaryField` / `OptionalBinaryField`, `Utf8Field` / `OptionalUtf8Field`
//! (the `utf8` field), `NullField`, `UnionField`, `StructField` (taking a
//! `yggdryl.dtype.StructType`, like `UnionField` takes its dynamic type) and its
//! concrete serie field (e.g. `Int64SerieField`, a column of `Int64SerieType`) —
//! the same suffixed names as the Rust crate, the submodule carrying the concern.
//! A field pairs a name with its `yggdryl.dtype` data type and a nullability flag
//! (`True` by default, as a keyword default); a data type also builds its field
//! directly through its `field(name, nullable=True)` factory.
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow` / `from_arrow`, and `cast_dtype` which returns a re-typed
//! `arrow-schema` field — all exchange `arrow-schema` values that cannot cross
//! the FFI boundary; C Data Interface interop is future work) and the dynamic base
//! and typed nested fields (`SerieField` / `TypedSerieField` over a non-integer
//! value type, `MapField` / `TypedMapField`), which have no concrete FFI shape
//! yet.

use pyo3::prelude::*;
use yggdryl_field::Field;

/// A nullable `union` field: a name paired with a `yggdryl.dtype.UnionType` data
/// type.
#[pyclass]
pub struct UnionField {
    pub(crate) inner: yggdryl_field::UnionField,
}

#[pymethods]
impl UnionField {
    /// A field named `name` of the union type `data_type`.
    #[new]
    #[pyo3(signature = (name, data_type, nullable = true))]
    fn new(name: String, data_type: &crate::dtype::UnionType, nullable: bool) -> Self {
        Self {
            inner: yggdryl_field::UnionField::new(name, data_type.inner.clone(), nullable),
        }
    }

    /// The field's name.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    fn data_type(&self) -> crate::dtype::UnionType {
        crate::dtype::UnionType {
            inner: self.inner.data_type().clone(),
        }
    }

    /// Whether values in this field may be null.
    fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// A nullable `struct` field: a name paired with a `yggdryl.dtype.StructType`
/// data type.
#[pyclass]
pub struct StructField {
    pub(crate) inner: yggdryl_field::StructField,
}

#[pymethods]
impl StructField {
    /// A field named `name` of the struct type `data_type`.
    #[new]
    #[pyo3(signature = (name, data_type, nullable = true))]
    fn new(name: String, data_type: &crate::dtype::StructType, nullable: bool) -> Self {
        Self {
            inner: yggdryl_field::StructField::new(name, data_type.inner.clone(), nullable),
        }
    }

    /// The field's name.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    fn data_type(&self) -> crate::dtype::StructType {
        crate::dtype::StructType {
            inner: self.inner.data_type().clone(),
        }
    }

    /// Whether values in this field may be null.
    fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// A `null` field: a name paired with the null data type.
#[pyclass]
pub struct NullField {
    pub(crate) inner: yggdryl_field::NullField,
}

#[pymethods]
impl NullField {
    /// A `null` field named `name`.
    #[new]
    #[pyo3(signature = (name, nullable = true))]
    fn new(name: String, nullable: bool) -> Self {
        Self {
            inner: yggdryl_field::NullField::new(name, nullable),
        }
    }

    /// The field's name.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    fn data_type(&self) -> crate::dtype::NullType {
        crate::dtype::NullType::default()
    }

    /// Whether values in this field may be null.
    fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// A nullable `binary` field: a name paired with the data type.
#[pyclass]
pub struct BinaryField {
    pub(crate) inner: yggdryl_field::BinaryField,
}

#[pymethods]
impl BinaryField {
    /// A `binary` field named `name`.
    #[new]
    #[pyo3(signature = (name, nullable = true))]
    fn new(name: String, nullable: bool) -> Self {
        Self {
            inner: yggdryl_field::BinaryField::new(name, nullable),
        }
    }

    /// The field's name.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    fn data_type(&self) -> crate::dtype::BinaryType {
        crate::dtype::BinaryType::default()
    }

    /// Whether values in this field may be null.
    fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// A nullable optional-`binary` field: a name paired with the logical optional
/// data type.
#[pyclass]
pub struct OptionalBinaryField {
    pub(crate) inner: yggdryl_field::TypedOptionalField<yggdryl_dtype::BinaryType>,
}

#[pymethods]
impl OptionalBinaryField {
    /// An optional-`binary` field named `name`.
    #[new]
    #[pyo3(signature = (name, nullable = true))]
    fn new(name: String, nullable: bool) -> Self {
        Self {
            inner: yggdryl_field::TypedOptionalField::new(name, nullable),
        }
    }

    /// The field's name.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    fn data_type(&self) -> crate::dtype::OptionalBinaryType {
        crate::dtype::OptionalBinaryType::default()
    }

    /// Whether values in this field may be null.
    fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// A nullable `utf8` field: a name paired with the data type.
#[pyclass]
pub struct Utf8Field {
    pub(crate) inner: yggdryl_field::Utf8Field,
}

#[pymethods]
impl Utf8Field {
    /// A `utf8` field named `name`.
    #[new]
    #[pyo3(signature = (name, nullable = true))]
    fn new(name: String, nullable: bool) -> Self {
        Self {
            inner: yggdryl_field::Utf8Field::new(name, nullable),
        }
    }

    /// The field's name.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    fn data_type(&self) -> crate::dtype::Utf8Type {
        crate::dtype::Utf8Type::default()
    }

    /// Whether values in this field may be null.
    fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// A nullable optional-`utf8` field: a name paired with the logical optional data
/// type.
#[pyclass]
pub struct OptionalUtf8Field {
    pub(crate) inner: yggdryl_field::TypedOptionalField<yggdryl_dtype::Utf8Type>,
}

#[pymethods]
impl OptionalUtf8Field {
    /// An optional-`utf8` field named `name`.
    #[new]
    #[pyo3(signature = (name, nullable = true))]
    fn new(name: String, nullable: bool) -> Self {
        Self {
            inner: yggdryl_field::TypedOptionalField::new(name, nullable),
        }
    }

    /// The field's name.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    fn data_type(&self) -> crate::dtype::OptionalUtf8Type {
        crate::dtype::OptionalUtf8Type::default()
    }

    /// Whether values in this field may be null.
    fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// Generates the two field wrappers of one integer type: the field `$ty` and the
/// optional field `$opt_ty` — each a thin delegation to the `yggdryl-field`
/// types.
macro_rules! int_field_py {
    ($ty:ident, $opt_ty:ident, $dtype:ident, $opt_dtype:ident, $name:literal) => {
        #[doc = concat!("A nullable `", $name, "` field: a name paired with the data type.")]
        #[pyclass]
        pub struct $ty {
            pub(crate) inner: yggdryl_field::$ty,
        }

        #[pymethods]
        impl $ty {
            #[doc = concat!("A `", $name, "` field named `name`.")]
            #[new]
            #[pyo3(signature = (name, nullable = true))]
            fn new(name: String, nullable: bool) -> Self {
                Self {
                    inner: yggdryl_field::$ty::new(name, nullable),
                }
            }

            /// The field's name.
            fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The field's data type.
            fn data_type(&self) -> crate::dtype::$dtype {
                crate::dtype::$dtype::default()
            }

            /// Whether values in this field may be null.
            fn is_nullable(&self) -> bool {
                self.inner.is_nullable()
            }
        }

        #[doc = concat!("A nullable optional-`", $name, "` field: a name paired with the logical optional data type.")]
        #[pyclass]
        pub struct $opt_ty {
            pub(crate) inner: yggdryl_field::TypedOptionalField<yggdryl_dtype::$dtype>,
        }

        #[pymethods]
        impl $opt_ty {
            #[doc = concat!("An optional-`", $name, "` field named `name`.")]
            #[new]
            #[pyo3(signature = (name, nullable = true))]
            fn new(name: String, nullable: bool) -> Self {
                Self {
                    inner: yggdryl_field::TypedOptionalField::new(name, nullable),
                }
            }

            /// The field's name.
            fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The field's data type.
            fn data_type(&self) -> crate::dtype::$opt_dtype {
                crate::dtype::$opt_dtype::default()
            }

            /// Whether values in this field may be null.
            fn is_nullable(&self) -> bool {
                self.inner.is_nullable()
            }
        }
    };
}

int_field_py!(
    Int8Field,
    OptionalInt8Field,
    Int8Type,
    OptionalInt8Type,
    "int8"
);
int_field_py!(
    Int16Field,
    OptionalInt16Field,
    Int16Type,
    OptionalInt16Type,
    "int16"
);
int_field_py!(
    Int32Field,
    OptionalInt32Field,
    Int32Type,
    OptionalInt32Type,
    "int32"
);
int_field_py!(
    Int64Field,
    OptionalInt64Field,
    Int64Type,
    OptionalInt64Type,
    "int64"
);
int_field_py!(
    UInt8Field,
    OptionalUInt8Field,
    UInt8Type,
    OptionalUInt8Type,
    "uint8"
);
int_field_py!(
    UInt16Field,
    OptionalUInt16Field,
    UInt16Type,
    OptionalUInt16Type,
    "uint16"
);
int_field_py!(
    UInt32Field,
    OptionalUInt32Field,
    UInt32Type,
    OptionalUInt32Type,
    "uint32"
);
int_field_py!(
    UInt64Field,
    OptionalUInt64Field,
    UInt64Type,
    OptionalUInt64Type,
    "uint64"
);
int_field_py!(
    Float32Field,
    OptionalFloat32Field,
    Float32Type,
    OptionalFloat32Type,
    "float32"
);
int_field_py!(
    Float16Field,
    OptionalFloat16Field,
    Float16Type,
    OptionalFloat16Type,
    "float16"
);
int_field_py!(
    Float64Field,
    OptionalFloat64Field,
    Float64Type,
    OptionalFloat64Type,
    "float64"
);

/// Generates the concrete serie field of one integer value type: `$ty`, a column
/// of the `yggdryl.dtype` class `$dtype` — a thin delegation to
/// `yggdryl_field::TypedSerieField<$value_ty>`.
macro_rules! int_serie_field_py {
    ($ty:ident, $dtype:ident, $value_ty:ident, $name:literal) => {
        #[doc = concat!("A nullable `list`-of-`", $name, "` field: a name paired with the `", stringify!($dtype), "` data type.")]
        #[pyclass]
        pub struct $ty {
            pub(crate) inner: yggdryl_field::TypedSerieField<yggdryl_dtype::$value_ty>,
        }

        #[pymethods]
        impl $ty {
            #[doc = concat!("A `list`-of-`", $name, "` field named `name`.")]
            #[new]
            #[pyo3(signature = (name, nullable = true))]
            fn new(name: String, nullable: bool) -> Self {
                Self {
                    inner: yggdryl_field::TypedSerieField::new(name, nullable),
                }
            }

            /// The field's name.
            fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The field's data type.
            fn data_type(&self) -> crate::dtype::$dtype {
                crate::dtype::$dtype::default()
            }

            /// Whether values in this field may be null.
            fn is_nullable(&self) -> bool {
                self.inner.is_nullable()
            }
        }
    };
}

int_serie_field_py!(Int8SerieField, Int8SerieType, Int8Type, "int8");
int_serie_field_py!(Int16SerieField, Int16SerieType, Int16Type, "int16");
int_serie_field_py!(Int32SerieField, Int32SerieType, Int32Type, "int32");
int_serie_field_py!(Int64SerieField, Int64SerieType, Int64Type, "int64");
int_serie_field_py!(UInt8SerieField, UInt8SerieType, UInt8Type, "uint8");
int_serie_field_py!(UInt16SerieField, UInt16SerieType, UInt16Type, "uint16");
int_serie_field_py!(UInt32SerieField, UInt32SerieType, UInt32Type, "uint32");
int_serie_field_py!(UInt64SerieField, UInt64SerieType, UInt64Type, "uint64");
int_serie_field_py!(Float16SerieField, Float16SerieType, Float16Type, "float16");
int_serie_field_py!(Float32SerieField, Float32SerieType, Float32Type, "float32");
int_serie_field_py!(Float64SerieField, Float64SerieType, Float64Type, "float64");

/// Populates the `field` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<UnionField>()?;
    module.add_class::<StructField>()?;
    module.add_class::<NullField>()?;
    module.add_class::<BinaryField>()?;
    module.add_class::<OptionalBinaryField>()?;
    module.add_class::<Utf8Field>()?;
    module.add_class::<OptionalUtf8Field>()?;
    module.add_class::<Int8Field>()?;
    module.add_class::<OptionalInt8Field>()?;
    module.add_class::<Int16Field>()?;
    module.add_class::<OptionalInt16Field>()?;
    module.add_class::<Int32Field>()?;
    module.add_class::<OptionalInt32Field>()?;
    module.add_class::<Int64Field>()?;
    module.add_class::<OptionalInt64Field>()?;
    module.add_class::<UInt8Field>()?;
    module.add_class::<OptionalUInt8Field>()?;
    module.add_class::<UInt16Field>()?;
    module.add_class::<OptionalUInt16Field>()?;
    module.add_class::<UInt32Field>()?;
    module.add_class::<OptionalUInt32Field>()?;
    module.add_class::<UInt64Field>()?;
    module.add_class::<OptionalUInt64Field>()?;
    module.add_class::<Float16Field>()?;
    module.add_class::<OptionalFloat16Field>()?;
    module.add_class::<Float32Field>()?;
    module.add_class::<OptionalFloat32Field>()?;
    module.add_class::<Float64Field>()?;
    module.add_class::<OptionalFloat64Field>()?;
    module.add_class::<Int8SerieField>()?;
    module.add_class::<Int16SerieField>()?;
    module.add_class::<Int32SerieField>()?;
    module.add_class::<Int64SerieField>()?;
    module.add_class::<UInt8SerieField>()?;
    module.add_class::<UInt16SerieField>()?;
    module.add_class::<UInt32SerieField>()?;
    module.add_class::<UInt64SerieField>()?;
    module.add_class::<Float16SerieField>()?;
    module.add_class::<Float32SerieField>()?;
    module.add_class::<Float64SerieField>()?;
    Ok(())
}
