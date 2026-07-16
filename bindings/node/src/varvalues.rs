//! The `yggdryl.types` namespace's **variable-length value layer** — one nullable value
//! (`Utf8Scalar` / `BinaryScalar`) and one nullable column (`Utf8Serie` / `BinarySerie`) per
//! variable-length kind, mirroring `yggdryl_core::io::var`'s generic `ByteScalar<E>` /
//! `ByteSerie<E>`.
//!
//! A **UTF-8** value crosses as a JS `string`; a **binary** value as a `Buffer`. A `Utf8` value is
//! validated (a bad decode throws); binary accepts any bytes. A `Scalar` is an immutable value
//! (with `equals` / `hashCode` and a byte codec); a `Serie` is a mutable column whose per-element
//! `set` may rewrite trailing offsets. `null` / `undefined` is a null throughout.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

use yggdryl_core::io::fixed::f16;
use yggdryl_core::io::fixed::Field as CoreField;
use yggdryl_core::io::var::{Binary, ByteScalar, ByteSerie, Utf8, VarElement};

use crate::types::{DataType, Field};
use crate::values::{
    F16Scalar, F32Scalar, F64Scalar, I128Scalar, I16Scalar, I32Scalar, I64Scalar, I8Scalar,
    U16Scalar, U32Scalar, U64Scalar, U8Scalar,
};

/// Maps any core error to a thrown JS `Error` (its guided text passes through unchanged).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// A Java-style `i32` content hash of a value, folding the 64-bit hash halves.
fn java_hash<T: Hash>(value: &T) -> i32 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    let hash = hasher.finish();
    (hash as u32 ^ (hash >> 32) as u32) as i32
}

/// The `Field` a variable-length column of kind `E` names.
fn var_field<E: VarElement>(name: &str, nullable: bool) -> Field {
    let id = <E as VarElement>::TYPE_ID;
    Field {
        inner: CoreField::of(name, id, id.fixed_byte_width().unwrap_or(0), nullable),
    }
}

// ---- per-kind value marshaling (stored bytes <-> JS) ----------------------------------------

/// UTF-8 — the stored bytes are known-valid, so `to_js` never re-checks.
fn utf8_to_js(bytes: &[u8]) -> String {
    std::str::from_utf8(bytes).unwrap_or_default().to_string()
}
fn utf8_from_js(value: String) -> Vec<u8> {
    value.into_bytes()
}
/// Binary — any bytes are valid.
fn binary_to_js(bytes: &[u8]) -> Buffer {
    bytes.to_vec().into()
}
fn binary_from_js(value: Buffer) -> Vec<u8> {
    value.to_vec()
}

/// Generates the `Scalar` **and** `Serie` napi wrappers for one variable-length kind.
///
/// `$js` is the JS-facing type; `$to` / `$from` marshal the stored bytes to/from it.
macro_rules! napi_var {
    ($Scalar:ident, $Serie:ident, $E:ty, $js:ty, $to:path, $from:path, $lit:literal,
     scalar_extra = { $($scalar_extra:tt)* }) => {
        #[doc = concat!("A single, nullable `", $lit, "` value.")]
        #[napi(namespace = "types")]
        pub struct $Scalar {
            pub(crate) inner: ByteScalar<$E>,
        }

        #[napi(namespace = "types")]
        impl $Scalar {
            /// A scalar from a value (`null` / `undefined` is null).
            #[napi(constructor)]
            pub fn new(value: Option<$js>) -> napi::Result<Self> {
                let bytes = value.map($from);
                ByteScalar::<$E>::new(bytes.as_deref())
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }

            /// The null scalar.
            #[napi(factory)]
            pub fn null() -> Self {
                Self {
                    inner: ByteScalar::null(),
                }
            }

            /// The value, or `null` if null.
            #[napi(getter)]
            pub fn value(&self) -> Option<$js> {
                self.inner.value_bytes().map($to)
            }

            /// Whether the scalar is null.
            #[napi(getter)]
            pub fn is_null(&self) -> bool {
                self.inner.is_null()
            }

            /// The element type's name (`"utf8"` / `"binary"`).
            #[napi(getter)]
            pub fn type_name(&self) -> &'static str {
                <$E as VarElement>::NAME
            }

            /// This scalar's [`DataType`].
            #[napi(getter)]
            pub fn data_type(&self) -> DataType {
                DataType::of(<$E as VarElement>::TYPE_ID)
            }

            /// A [`Field`] naming a column of this scalar's type (default nullable).
            #[napi]
            pub fn field(&self, name: String, nullable: Option<bool>) -> Field {
                var_field::<$E>(&name, nullable.unwrap_or(true))
            }

            /// The scalar's canonical bytes (validity byte, then `[len][bytes]` if present).
            #[napi]
            pub fn serialize_bytes(&self) -> Buffer {
                self.inner.serialize_bytes().into()
            }

            /// Reconstructs a scalar from [`serializeBytes`](Self::serialize_bytes).
            #[napi(factory)]
            pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
                ByteScalar::<$E>::deserialize_bytes(&bytes)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }

            /// Value equality.
            #[napi]
            pub fn equals(&self, other: &$Scalar) -> bool {
                self.inner == other.inner
            }

            /// A content hash consistent with [`equals`](Self::equals).
            #[napi]
            pub fn hash_code(&self) -> i32 {
                java_hash(&self.inner)
            }

            /// An explicit copy.
            #[napi]
            pub fn copy(&self) -> Self {
                Self {
                    inner: self.inner.clone(),
                }
            }

            #[napi(js_name = "toString")]
            pub fn text(&self) -> String {
                if self.inner.is_null() {
                    format!("{}(null)", stringify!($Scalar))
                } else {
                    format!("{}(value)", stringify!($Scalar))
                }
            }

            $($scalar_extra)*
        }

        #[doc = concat!("A nullable column of `", $lit, "` values.")]
        #[napi(namespace = "types")]
        pub struct $Serie {
            pub(crate) inner: ByteSerie<$E>,
        }

        #[napi(namespace = "types")]
        impl $Serie {
            /// A column from an array of value-or-`null` (empty by default).
            #[napi(constructor)]
            pub fn new(values: Option<Vec<Option<$js>>>) -> napi::Result<Self> {
                match values {
                    None => Ok(Self {
                        inner: ByteSerie::new(),
                    }),
                    Some(values) => {
                        let owned: Vec<Option<Vec<u8>>> =
                            values.into_iter().map(|value| value.map($from)).collect();
                        let refs: Vec<Option<&[u8]>> = owned.iter().map(|o| o.as_deref()).collect();
                        ByteSerie::<$E>::from_options(&refs)
                            .map(|inner| Self { inner })
                            .map_err(to_error)
                    }
                }
            }

            /// A column from an array of [`getScalar`](Self::get_scalar)-shaped scalars — a
            /// `null` / `undefined` item is the null scalar. Round-trips a column through its own
            /// scalars.
            #[napi(factory)]
            pub fn from_scalars(scalars: Vec<Option<&$Scalar>>) -> napi::Result<Self> {
                let scalars: Vec<ByteScalar<$E>> = scalars
                    .into_iter()
                    .map(|slot| {
                        slot.map(|scalar| scalar.inner.clone())
                            .unwrap_or_else(ByteScalar::<$E>::null)
                    })
                    .collect();
                ByteSerie::<$E>::from_scalars(&scalars)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }

            /// Appends one element (`null` / `undefined` is a null).
            #[napi]
            pub fn push(&mut self, value: Option<$js>) -> napi::Result<()> {
                let bytes = value.map($from);
                self.inner.push_bytes(bytes.as_deref()).map_err(to_error)
            }

            /// The element at `index`, or `null` if it is null or out of range.
            #[napi]
            pub fn get(&self, index: u32) -> Option<$js> {
                self.inner.get_bytes(index as usize).map($to)
            }

            /// The element at `index` as a scalar (null if null or out of range).
            #[napi]
            pub fn get_scalar(&self, index: u32) -> $Scalar {
                $Scalar {
                    inner: self.inner.get_scalar(index as usize),
                }
            }

            /// Overwrites element `index` (`null` writes a null); a length change rewrites the
            /// trailing offsets. Throws if out of range.
            #[napi]
            pub fn set(&mut self, index: u32, value: Option<$js>) -> napi::Result<()> {
                let bytes = value.map($from);
                self.inner
                    .set_bytes(index as usize, bytes.as_deref())
                    .map_err(to_error)
            }

            /// The number of elements.
            #[napi(getter)]
            pub fn length(&self) -> u32 {
                self.inner.len() as u32
            }

            /// The number of null elements.
            #[napi(getter)]
            pub fn null_count(&self) -> u32 {
                self.inner.null_count() as u32
            }

            /// Whether the column carries any nulls.
            #[napi(getter)]
            pub fn has_nulls(&self) -> bool {
                self.inner.has_nulls()
            }

            /// Whether the column is empty.
            #[napi]
            pub fn is_empty(&self) -> bool {
                self.inner.is_empty()
            }

            /// The total number of value bytes (excluding offsets / validity).
            #[napi(getter)]
            pub fn data_len(&self) -> u32 {
                self.inner.data_len() as u32
            }

            /// The elements as an array of value-or-`null`, in order.
            #[napi]
            pub fn to_options(&self) -> Vec<Option<$js>> {
                (0..self.inner.len())
                    .map(|index| self.inner.get_bytes(index).map($to))
                    .collect()
            }

            /// This column's [`DataType`].
            #[napi(getter)]
            pub fn data_type(&self) -> DataType {
                DataType::of(<$E as VarElement>::TYPE_ID)
            }

            /// A [`Field`] naming this column with explicit nullability (default nullable).
            #[napi]
            pub fn field(&self, name: String, nullable: Option<bool>) -> Field {
                var_field::<$E>(&name, nullable.unwrap_or(true))
            }

            /// A [`Field`] naming this column, nullability **inferred** from whether it holds nulls.
            #[napi]
            pub fn to_field(&self, name: String) -> Field {
                var_field::<$E>(&name, self.inner.has_nulls())
            }

            /// The column's canonical bytes (`[len][flags][validity?][offsets][data_len][data]`).
            #[napi]
            pub fn serialize_bytes(&self) -> Buffer {
                self.inner.serialize_bytes().into()
            }

            /// Reconstructs a column from [`serializeBytes`](Self::serialize_bytes).
            #[napi(factory)]
            pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
                ByteSerie::<$E>::deserialize_bytes(&bytes)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }

            /// Value equality (content, nulls included).
            #[napi]
            pub fn equals(&self, other: &$Serie) -> bool {
                self.inner == other.inner
            }

            /// An explicit copy.
            #[napi]
            pub fn copy(&self) -> Self {
                Self {
                    inner: self.inner.clone(),
                }
            }

            #[napi(js_name = "toString")]
            pub fn text(&self) -> String {
                format!(
                    "{}(len={}, nullCount={})",
                    stringify!($Serie),
                    self.inner.len(),
                    self.inner.null_count()
                )
            }
        }
    };
}

napi_var!(
    Utf8Scalar,
    Utf8Serie,
    Utf8,
    String,
    utf8_to_js,
    utf8_from_js,
    "utf8",
    scalar_extra = {
        #[napi]
        pub fn to_u8(&self) -> napi::Result<U8Scalar> {
            self.inner
                .parse_to::<u8>()
                .map(|inner| U8Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_u16(&self) -> napi::Result<U16Scalar> {
            self.inner
                .parse_to::<u16>()
                .map(|inner| U16Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_u32(&self) -> napi::Result<U32Scalar> {
            self.inner
                .parse_to::<u32>()
                .map(|inner| U32Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_u64(&self) -> napi::Result<U64Scalar> {
            self.inner
                .parse_to::<u64>()
                .map(|inner| U64Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_i8(&self) -> napi::Result<I8Scalar> {
            self.inner
                .parse_to::<i8>()
                .map(|inner| I8Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_i16(&self) -> napi::Result<I16Scalar> {
            self.inner
                .parse_to::<i16>()
                .map(|inner| I16Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_i32(&self) -> napi::Result<I32Scalar> {
            self.inner
                .parse_to::<i32>()
                .map(|inner| I32Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_i64(&self) -> napi::Result<I64Scalar> {
            self.inner
                .parse_to::<i64>()
                .map(|inner| I64Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_i128(&self) -> napi::Result<I128Scalar> {
            self.inner
                .parse_to::<i128>()
                .map(|inner| I128Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_f16(&self) -> napi::Result<F16Scalar> {
            self.inner
                .parse_to::<f16>()
                .map(|inner| F16Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_f32(&self) -> napi::Result<F32Scalar> {
            self.inner
                .parse_to::<f32>()
                .map(|inner| F32Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_f64(&self) -> napi::Result<F64Scalar> {
            self.inner
                .parse_to::<f64>()
                .map(|inner| F64Scalar { inner })
                .map_err(to_error)
        }
    }
);
napi_var!(
    BinaryScalar,
    BinarySerie,
    Binary,
    Buffer,
    binary_to_js,
    binary_from_js,
    "binary",
    scalar_extra = {
        #[napi]
        pub fn to_u8(&self) -> napi::Result<U8Scalar> {
            self.inner
                .read_to::<u8>()
                .map(|inner| U8Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_u16(&self) -> napi::Result<U16Scalar> {
            self.inner
                .read_to::<u16>()
                .map(|inner| U16Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_u32(&self) -> napi::Result<U32Scalar> {
            self.inner
                .read_to::<u32>()
                .map(|inner| U32Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_u64(&self) -> napi::Result<U64Scalar> {
            self.inner
                .read_to::<u64>()
                .map(|inner| U64Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_i8(&self) -> napi::Result<I8Scalar> {
            self.inner
                .read_to::<i8>()
                .map(|inner| I8Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_i16(&self) -> napi::Result<I16Scalar> {
            self.inner
                .read_to::<i16>()
                .map(|inner| I16Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_i32(&self) -> napi::Result<I32Scalar> {
            self.inner
                .read_to::<i32>()
                .map(|inner| I32Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_i64(&self) -> napi::Result<I64Scalar> {
            self.inner
                .read_to::<i64>()
                .map(|inner| I64Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_i128(&self) -> napi::Result<I128Scalar> {
            self.inner
                .read_to::<i128>()
                .map(|inner| I128Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_f16(&self) -> napi::Result<F16Scalar> {
            self.inner
                .read_to::<f16>()
                .map(|inner| F16Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_f32(&self) -> napi::Result<F32Scalar> {
            self.inner
                .read_to::<f32>()
                .map(|inner| F32Scalar { inner })
                .map_err(to_error)
        }
        #[napi]
        pub fn to_f64(&self) -> napi::Result<F64Scalar> {
            self.inner
                .read_to::<f64>()
                .map(|inner| F64Scalar { inner })
                .map_err(to_error)
        }
    }
);
