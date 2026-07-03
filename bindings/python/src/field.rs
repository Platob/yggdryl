//! The `yggdryl.field` submodule — thin wrappers over the `yggdryl-field` crate.
//!
//! Every integer type is exposed as its field and its optional field (e.g.
//! `Int64`, `OptionalInt64`), alongside `Binary` / `OptionalBinary`, `Null` and
//! `Union` — the same bare names as the Rust crate, the submodule carrying the
//! concern. A field pairs a name with its `yggdryl.dtype` data type and a
//! nullability flag (`True` by default, as a keyword default).
//!
//! Rust-only (stated here and on the docs site): the Arrow interop surface
//! (`to_arrow` / `from_arrow` exchange `arrow-schema` values that cannot cross
//! the FFI boundary; C Data Interface interop is future work) and the generic
//! nested fields (`List` / `Map` / `Struct`), which have no concrete FFI shape
//! yet.

use pyo3::prelude::*;
use yggdryl_field::RawField;

/// A nullable `union` field: a name paired with a `yggdryl.dtype.Union` data type.
#[pyclass]
pub struct Union {
    pub(crate) inner: yggdryl_field::Union,
}

#[pymethods]
impl Union {
    /// A field named `name` of the union type `data_type`.
    #[new]
    #[pyo3(signature = (name, data_type, nullable = true))]
    fn new(name: String, data_type: &crate::dtype::Union, nullable: bool) -> Self {
        Self {
            inner: yggdryl_field::Union::new(name, data_type.inner.clone(), nullable),
        }
    }

    /// The field's name.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    fn data_type(&self) -> crate::dtype::Union {
        crate::dtype::Union {
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
pub struct Null {
    pub(crate) inner: yggdryl_field::Null,
}

#[pymethods]
impl Null {
    /// A `null` field named `name`.
    #[new]
    #[pyo3(signature = (name, nullable = true))]
    fn new(name: String, nullable: bool) -> Self {
        Self {
            inner: yggdryl_field::Null::new(name, nullable),
        }
    }

    /// The field's name.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    fn data_type(&self) -> crate::dtype::Null {
        crate::dtype::Null::default()
    }

    /// Whether values in this field may be null.
    fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// A nullable `binary` field: a name paired with the data type.
#[pyclass]
pub struct Binary {
    pub(crate) inner: yggdryl_field::Binary,
}

#[pymethods]
impl Binary {
    /// A `binary` field named `name`.
    #[new]
    #[pyo3(signature = (name, nullable = true))]
    fn new(name: String, nullable: bool) -> Self {
        Self {
            inner: yggdryl_field::Binary::new(name, nullable),
        }
    }

    /// The field's name.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    fn data_type(&self) -> crate::dtype::Binary {
        crate::dtype::Binary::default()
    }

    /// Whether values in this field may be null.
    fn is_nullable(&self) -> bool {
        self.inner.is_nullable()
    }
}

/// A nullable optional-`binary` field: a name paired with the logical optional
/// data type.
#[pyclass]
pub struct OptionalBinary {
    pub(crate) inner: yggdryl_field::Optional<yggdryl_dtype::Binary>,
}

#[pymethods]
impl OptionalBinary {
    /// An optional-`binary` field named `name`.
    #[new]
    #[pyo3(signature = (name, nullable = true))]
    fn new(name: String, nullable: bool) -> Self {
        Self {
            inner: yggdryl_field::Optional::new(name, nullable),
        }
    }

    /// The field's name.
    fn name(&self) -> String {
        self.inner.name().to_string()
    }

    /// The field's data type.
    fn data_type(&self) -> crate::dtype::OptionalBinary {
        crate::dtype::OptionalBinary::default()
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
    ($ty:ident, $opt_ty:ident, $name:literal) => {
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
            fn data_type(&self) -> crate::dtype::$ty {
                crate::dtype::$ty::default()
            }

            /// Whether values in this field may be null.
            fn is_nullable(&self) -> bool {
                self.inner.is_nullable()
            }
        }

        #[doc = concat!("A nullable optional-`", $name, "` field: a name paired with the logical optional data type.")]
        #[pyclass]
        pub struct $opt_ty {
            pub(crate) inner: yggdryl_field::Optional<yggdryl_dtype::$ty>,
        }

        #[pymethods]
        impl $opt_ty {
            #[doc = concat!("An optional-`", $name, "` field named `name`.")]
            #[new]
            #[pyo3(signature = (name, nullable = true))]
            fn new(name: String, nullable: bool) -> Self {
                Self {
                    inner: yggdryl_field::Optional::new(name, nullable),
                }
            }

            /// The field's name.
            fn name(&self) -> String {
                self.inner.name().to_string()
            }

            /// The field's data type.
            fn data_type(&self) -> crate::dtype::$opt_ty {
                crate::dtype::$opt_ty::default()
            }

            /// Whether values in this field may be null.
            fn is_nullable(&self) -> bool {
                self.inner.is_nullable()
            }
        }
    };
}

int_field_py!(Int8, OptionalInt8, "int8");
int_field_py!(Int16, OptionalInt16, "int16");
int_field_py!(Int32, OptionalInt32, "int32");
int_field_py!(Int64, OptionalInt64, "int64");
int_field_py!(UInt8, OptionalUInt8, "uint8");
int_field_py!(UInt16, OptionalUInt16, "uint16");
int_field_py!(UInt32, OptionalUInt32, "uint32");
int_field_py!(UInt64, OptionalUInt64, "uint64");

/// Populates the `field` submodule.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<Union>()?;
    module.add_class::<Null>()?;
    module.add_class::<Binary>()?;
    module.add_class::<OptionalBinary>()?;
    module.add_class::<Int8>()?;
    module.add_class::<OptionalInt8>()?;
    module.add_class::<Int16>()?;
    module.add_class::<OptionalInt16>()?;
    module.add_class::<Int32>()?;
    module.add_class::<OptionalInt32>()?;
    module.add_class::<Int64>()?;
    module.add_class::<OptionalInt64>()?;
    module.add_class::<UInt8>()?;
    module.add_class::<OptionalUInt8>()?;
    module.add_class::<UInt16>()?;
    module.add_class::<OptionalUInt16>()?;
    module.add_class::<UInt32>()?;
    module.add_class::<OptionalUInt32>()?;
    module.add_class::<UInt64>()?;
    module.add_class::<OptionalUInt64>()?;
    Ok(())
}
