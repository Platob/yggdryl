//! The `yggdryl.decimal` namespace's **columnar** decimal types — one nullable value carrying its
//! column `(precision, scale)` (`D32Scalar` … `D256Scalar`) and one nullable decimal column
//! (`D32Serie` … `D256Serie`), mirroring `yggdryl_core::io::fixed`'s `DecimalScalar<B>` /
//! `DecimalSerie<B>`.
//!
//! A column fixes one `(precision, scale)` (Arrow's model): a value is re-expressed at that scale
//! (a guided error if it does not fit exactly, or exceeds the precision). **Values cross as exact
//! decimal strings** (`"123.45"`) — the same form across Node and Python. A `Scalar` is an
//! immutable value (with `equals` / `hashCode` by its decimal value, so `2.5` equals `2.50`); a
//! `Serie` is a mutable column.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use napi::bindgen_prelude::Buffer;
use napi::{Env, JsUnknown};
use napi_derive::napi;

use yggdryl_core::io::fixed::{
    Dec128, Dec256, Dec32, Dec64, Decimal, DecimalBacking, DecimalScalar, DecimalSerie,
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

/// Parses a decimal literal (`"-123.45"`) into a value of width `B`.
fn parse_dec<B: DecimalBacking>(text: &str) -> napi::Result<Decimal<B>> {
    text.parse::<Decimal<B>>().map_err(to_error)
}

/// Generates the columnar `Scalar` **and** `Serie` napi wrappers for one decimal width.
macro_rules! napi_dec_col {
    ($Scalar:ident, $Serie:ident, $B:ty, $lit:literal) => {
        #[doc = concat!("A single, nullable `", $lit, "` value carrying its column `(precision, scale)`.")]
        #[napi(namespace = "decimal")]
        pub struct $Scalar {
            pub(crate) inner: DecimalScalar<$B>,
        }

        #[napi(namespace = "decimal")]
        impl $Scalar {
            /// A scalar from a decimal string. With no `precision`/`scale` they are inferred from
            /// the value; pass **both** to pin the column type. `value=null` is a null of the given
            /// (or default) `(precision, scale)`.
            #[napi(constructor)]
            pub fn new(
                value: Option<String>,
                precision: Option<u8>,
                scale: Option<i8>,
            ) -> napi::Result<Self> {
                match value {
                    None => Ok(Self {
                        inner: DecimalScalar::null(
                            precision.unwrap_or(<$B as DecimalBacking>::MAX_PRECISION),
                            scale.unwrap_or(0),
                        ),
                    }),
                    Some(text) => {
                        let value = parse_dec::<$B>(&text)?;
                        match (precision, scale) {
                            (Some(precision), Some(scale)) => {
                                DecimalScalar::with_precision_scale(value, precision, scale)
                                    .map(|inner| Self { inner })
                                    .map_err(to_error)
                            }
                            _ => Ok(Self {
                                inner: DecimalScalar::of(value),
                            }),
                        }
                    }
                }
            }

            /// The null scalar of the given `(precision, scale)`.
            #[napi(factory)]
            pub fn null(precision: u8, scale: i8) -> Self {
                Self {
                    inner: DecimalScalar::null(precision, scale),
                }
            }

            /// The value as a decimal string, or `null` if null.
            #[napi(getter)]
            pub fn value(&self) -> Option<String> {
                self.inner.value().map(|value| value.to_string())
            }

            /// Whether the scalar is null.
            #[napi(getter)]
            pub fn is_null(&self) -> bool {
                self.inner.is_null()
            }

            /// The column precision.
            #[napi(getter)]
            pub fn precision(&self) -> u32 {
                self.inner.precision() as u32
            }

            /// The column scale.
            #[napi(getter)]
            pub fn scale(&self) -> i32 {
                self.inner.scale() as i32
            }

            /// The scalar's canonical bytes (`[validity][precision][scale][coefficient]`).
            #[napi]
            pub fn serialize_bytes(&self) -> Buffer {
                self.inner.serialize_bytes().into()
            }

            /// Reconstructs a scalar from [`serializeBytes`](Self::serialize_bytes).
            #[napi(factory)]
            pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
                DecimalScalar::<$B>::deserialize_bytes(&bytes)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }

            /// Value equality (`2.5` equals `2.50`).
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
                match self.inner.value() {
                    Some(value) => format!(
                        "{}(\"{}\", precision={}, scale={})",
                        stringify!($Scalar),
                        value,
                        self.inner.precision(),
                        self.inner.scale()
                    ),
                    None => format!(
                        "{}(null, precision={}, scale={})",
                        stringify!($Scalar),
                        self.inner.precision(),
                        self.inner.scale()
                    ),
                }
            }
        }

        #[doc = concat!("A nullable column of `", $lit, "` values at one `(precision, scale)`.")]
        #[napi(namespace = "decimal")]
        pub struct $Serie {
            pub(crate) inner: DecimalSerie<$B>,
        }

        #[napi(namespace = "decimal")]
        impl $Serie {
            /// A column of `(precision, scale)` from an array of decimal-string-or-`null` (empty by
            /// default). Each value is re-expressed at the column's scale.
            #[napi(constructor)]
            pub fn new(
                precision: u8,
                scale: i8,
                values: Option<Vec<Option<String>>>,
            ) -> napi::Result<Self> {
                match values {
                    None => Ok(Self {
                        inner: DecimalSerie::new(precision, scale),
                    }),
                    Some(values) => {
                        let mut options = Vec::with_capacity(values.len());
                        for value in values {
                            options.push(match value {
                                Some(text) => Some(parse_dec::<$B>(&text)?),
                                None => None,
                            });
                        }
                        DecimalSerie::from_options(precision, scale, &options)
                            .map(|inner| Self { inner })
                            .map_err(to_error)
                    }
                }
            }

            /// A non-null column from an array of present decimal strings.
            #[napi(factory)]
            pub fn from_values(precision: u8, scale: i8, values: Vec<String>) -> napi::Result<Self> {
                let mut owned = Vec::with_capacity(values.len());
                for text in values {
                    owned.push(parse_dec::<$B>(&text)?);
                }
                DecimalSerie::from_values(precision, scale, &owned)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }

            /// A column of `(precision, scale)` from an array of
            /// [`getScalar`](Self::get_scalar)-shaped scalars — a `null` / `undefined` item is the
            /// null scalar. Each value is re-expressed at the column's scale.
            #[napi(factory)]
            pub fn from_scalars(
                precision: u8,
                scale: i8,
                scalars: Vec<Option<&$Scalar>>,
            ) -> napi::Result<Self> {
                let scalars: Vec<DecimalScalar<$B>> = scalars
                    .into_iter()
                    .map(|slot| {
                        slot.map(|scalar| scalar.inner.clone())
                            .unwrap_or_else(|| DecimalScalar::null(precision, scale))
                    })
                    .collect();
                DecimalSerie::from_scalars(precision, scale, &scalars)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }

            /// Appends one element (a decimal string, or `null` for a null).
            #[napi]
            pub fn push(&mut self, value: Option<String>) -> napi::Result<()> {
                let decimal = match value {
                    Some(text) => Some(parse_dec::<$B>(&text)?),
                    None => None,
                };
                self.inner.push(decimal).map_err(to_error)
            }

            /// The value at `index` (a decimal string), or `null` if null or out of range.
            #[napi]
            pub fn get(&self, index: u32) -> Option<String> {
                self.inner.get(index as usize).map(|value| value.to_string())
            }

            /// Element `index` as a scalar (carrying the column's `(precision, scale)`).
            #[napi]
            pub fn get_scalar(&self, index: u32) -> $Scalar {
                $Scalar {
                    inner: self.inner.get_scalar(index as usize),
                }
            }

            /// Overwrites element `index` (a decimal string, or `null`); throws out of range or if
            /// the value does not fit `(precision, scale)`.
            #[napi]
            pub fn set(&mut self, index: u32, value: Option<String>) -> napi::Result<()> {
                let decimal = match value {
                    Some(text) => Some(parse_dec::<$B>(&text)?),
                    None => None,
                };
                self.inner.set(index as usize, decimal).map_err(to_error)
            }

            /// The column precision.
            #[napi(getter)]
            pub fn precision(&self) -> u32 {
                self.inner.precision() as u32
            }

            /// The column scale.
            #[napi(getter)]
            pub fn scale(&self) -> i32 {
                self.inner.scale() as i32
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

            /// The elements as an array of decimal-string-or-`null`, in order.
            #[napi]
            pub fn to_options(&self) -> Vec<Option<String>> {
                (0..self.inner.len())
                    .map(|index| self.inner.get(index).map(|value| value.to_string()))
                    .collect()
            }

            /// The column's canonical bytes (`[len][precision][scale][flags][validity?][values]`).
            #[napi]
            pub fn serialize_bytes(&self) -> Buffer {
                self.inner.serialize_bytes().into()
            }

            /// Reconstructs a column from [`serializeBytes`](Self::serialize_bytes).
            #[napi(factory)]
            pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
                DecimalSerie::<$B>::deserialize_bytes(&bytes)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }

            /// Value equality.
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
                    "{}(len={}, precision={}, scale={}, nullCount={})",
                    stringify!($Serie),
                    self.inner.len(),
                    self.inner.precision(),
                    self.inner.scale(),
                    self.inner.null_count()
                )
            }

            // ---- Phase 8: reshape + row-selection (no arithmetic on a decimal column) --------

            /// A same-`(precision, scale)` column of the rows `mask` keeps (`true` keeps row `i`);
            /// throws if `mask`'s length is not this column's length.
            #[napi]
            pub fn filter(&self, mask: Vec<bool>) -> napi::Result<Self> {
                Ok(Self {
                    inner: crate::ops::filter_into(&self.inner, mask)?,
                })
            }

            /// A same-column with every null replaced by `value` (a JS `null` / `undefined` is a
            /// no-op clone). A decimal has no native JS scalar form, so a real fill value is passed
            /// as a length-1 `Serie` **carrier** of the same `(precision, scale)` — its `value(0)` is
            /// used, and a scale mismatch (or a plain JS number) is a guided error.
            #[napi]
            pub fn fill_null(&self, env: Env, value: JsUnknown) -> napi::Result<Self> {
                Ok(Self {
                    inner: crate::ops::fill_null_into(env, &self.inner, value)?,
                })
            }

            /// This column as a one-field [`StructSerie`](crate::nested::StructSerie) named `name`
            /// (default `"value"`).
            #[napi]
            pub fn to_struct(&self, name: Option<String>) -> crate::nested::StructSerie {
                crate::ops::to_struct_wrapper(&self.inner, name)
            }

            /// This column as a list-of-singletons [`ListSerie`](crate::nested::ListSerie).
            #[napi]
            pub fn to_list(&self) -> crate::nested::ListSerie {
                crate::ops::to_list_wrapper(&self.inner)
            }

            /// This column reshaped toward a map, as its `serializeBytes()` frame (unchanged for a
            /// decimal column; reconstruct with the resulting class's `deserializeBytes`).
            #[napi]
            pub fn to_map(&self) -> napi::Result<Buffer> {
                crate::ops::to_map_frame(&self.inner)
            }
        }
    };
}

napi_dec_col!(D32Scalar, D32Serie, Dec32, "d32");
napi_dec_col!(D64Scalar, D64Serie, Dec64, "d64");
napi_dec_col!(D128Scalar, D128Serie, Dec128, "d128");
napi_dec_col!(D256Scalar, D256Serie, Dec256, "d256");
