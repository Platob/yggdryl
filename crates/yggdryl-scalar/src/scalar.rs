//! The [`Scalar`] base trait: a single, possibly-null value of a
//! [`DataType`](yggdryl_dtype::DataType).

use std::sync::Arc;

use yggdryl_dtype::{DataError, DataType};

/// A single value of a data type, possibly null — the base trait mirroring an Apache
/// Arrow `Scalar`.
///
/// It carries its [`data_type`](Scalar::data_type), reports whether it
/// [`is_null`](Scalar::is_null), and exposes the native Rust
/// [`value`](Scalar::value) (of the associated [`Value`](Scalar::Value) type) when
/// non-null. Arrow models a scalar as an array of exactly one value, so
/// [`to_arrow_scalar`](Scalar::to_arrow_scalar) builds a one-element
/// [`arrow_array::ArrayRef`] (null when the scalar is null),
/// [`to_arrow_array`](Scalar::to_arrow_array) its array form (the same one-element
/// array for a plain scalar; the element array for a serie), and
/// [`from_arrow`](Scalar::from_arrow) reads one back (the Arrow factory). The data
/// type is the associated
/// [`DataType`](Scalar::DataType) type (rather than a generic parameter or a box) so
/// the concrete type is preserved for zero-cost access, mirroring `yggdryl-field`'s
/// `Field` and `yggdryl-dtype`'s `Logical`; the associated `Value` names the
/// in-memory representation a concrete scalar holds. It shares
/// [`DataType`](yggdryl_dtype::DataType)'s `Debug + Send + Sync` bounds so scalar
/// values are printable and shareable across threads and FFI. The associated
/// [`Value`](Scalar::Value) is `?Sized`, so a string scalar can expose `Value = str`.
/// [`cast_dtype`](Scalar::cast_dtype) re-types the value to another data type through
/// the exact `as_*` contract, and the `unsafe`
/// [`cast_dtype_unchecked`](Scalar::cast_dtype_unchecked) reinterprets its bytes
/// between the fixed-width, `binary` and `utf8` types.
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::{DataError, DataType, Int32Type};
/// use yggdryl_scalar::{arrow_array, Scalar};
/// use arrow_array::Array; // len / is_null on the arrow side
///
/// #[derive(Debug)]
/// struct Int32Scalar {
///     data_type: Int32Type,
///     value: Option<i32>,
/// }
///
/// impl Scalar for Int32Scalar {
///     type DataType = Int32Type;
///     type Value = i32;
///     fn data_type(&self) -> &Int32Type {
///         &self.data_type
///     }
///     fn is_null(&self) -> bool {
///         self.value.is_none()
///     }
///     fn value(&self) -> Option<&i32> {
///         self.value.as_ref()
///     }
///     fn to_arrow_scalar(&self) -> arrow_array::ArrayRef {
///         std::sync::Arc::new(match self.value {
///             Some(value) => arrow_array::Int32Array::from_iter_values([value]),
///             None => arrow_array::Int32Array::new_null(1),
///         })
///     }
///     fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
///         if array.len() != 1 {
///             return Err(DataError::InvalidScalarLength { got: array.len() });
///         }
///         let array = array
///             .as_any()
///             .downcast_ref::<arrow_array::Int32Array>()
///             .ok_or_else(|| DataError::IncompatibleArrowType {
///                 expected: "Int32Type".to_string(),
///                 got: array.data_type().to_string(),
///             })?;
///         Ok(Int32Scalar {
///             data_type: Int32Type,
///             value: (!array.is_null(0)).then(|| array.value(0)),
///         })
///     }
///     // The native type answers directly; wider targets convert.
///     fn as_i32(&self) -> Result<i32, DataError> {
///         self.value.ok_or(DataError::NullValue)
///     }
///     fn as_i64(&self) -> Result<i64, DataError> {
///         self.value.map(i64::from).ok_or(DataError::NullValue)
///     }
/// }
///
/// let answer = Int32Scalar { data_type: Int32Type, value: Some(42) };
/// assert_eq!(answer.data_type().name(), "int32");
/// assert!(!answer.is_null());
/// assert_eq!(answer.value(), Some(&42));
/// assert_eq!(answer.as_i64().unwrap(), 42); // converted access
/// // An int32 has no str conversion (the default): an actionable error.
/// assert!(matches!(answer.as_str(None), Err(DataError::UnsupportedConversion { .. })));
///
/// // Arrow interop: a one-element array, round-tripped.
/// let arrow = answer.to_arrow_scalar();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(Int32Scalar::from_arrow(arrow.as_ref()).unwrap().value(), Some(&42));
///
/// let missing = Int32Scalar { data_type: Int32Type, value: None };
/// assert!(missing.is_null());
/// assert!(missing.to_arrow_scalar().is_null(0));
/// ```
pub trait Scalar: std::fmt::Debug + Send + Sync {
    /// The concrete data type of this scalar.
    type DataType: yggdryl_dtype::DataType;

    /// The native Rust representation this scalar holds when non-null. May be
    /// unsized (e.g. `str`).
    type Value: ?Sized;

    /// The scalar's data type.
    fn data_type(&self) -> &Self::DataType;

    /// Whether this scalar holds a null value.
    fn is_null(&self) -> bool;

    /// The scalar's value, or `None` when it [`is_null`](Scalar::is_null).
    fn value(&self) -> Option<&Self::Value>;

    /// The Apache Arrow **scalar** form of this value: a one-element
    /// [`arrow_array::ArrayRef`] of this scalar's data type, holding the value (or a
    /// null). This is Arrow's own scalar representation — a length-1 array — so it
    /// plugs straight into arrow-rs kernels (wrap it in `arrow_array::Scalar` for a
    /// `Datum`). Its exact inverse is [`from_arrow`](Scalar::from_arrow).
    ///
    /// A concrete scalar names the concrete array type it produces as the
    /// [`TypedScalar`](crate::TypedScalar) `ArrowScalar` parameter.
    fn to_arrow_scalar(&self) -> arrow_array::ArrayRef;

    /// The Apache Arrow **array** form of this value. For a plain scalar this is the
    /// same one-element array as [`to_arrow_scalar`](Scalar::to_arrow_scalar) — a
    /// scalar *is* a length-1 array — so it defaults to it; a sequence scalar (a
    /// serie) overrides this to hand back its element array instead (empty when the
    /// serie is null, told apart from an empty serie by [`is_null`](Scalar::is_null)).
    ///
    /// A concrete scalar names the concrete array type it produces as the
    /// [`TypedScalar`](crate::TypedScalar) `ArrowArray` parameter (which defaults to
    /// `ArrowScalar`).
    fn to_arrow_array(&self) -> arrow_array::ArrayRef {
        self.to_arrow_scalar()
    }

    /// Build this scalar from its one-element Apache Arrow array — the exact inverse
    /// of [`to_arrow_scalar`](Scalar::to_arrow_scalar), and the Arrow factory. An
    /// array whose length is not exactly 1 errors with
    /// [`DataError::InvalidScalarLength`]; an array of a different Arrow type errors
    /// with [`DataError::IncompatibleArrowType`].
    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError>
    where
        Self: Sized;

    /// The value as an `i8`, when exactly representable.
    ///
    /// The `as_*` accessors share one contract: the value whenever the target type
    /// represents it exactly, and an actionable [`DataError`] otherwise —
    /// [`NullValue`](DataError::NullValue) for a null scalar,
    /// [`InexactConversion`](DataError::InexactConversion) when converting would
    /// change the value (a narrowing or sign change out of range, a float that
    /// would round, non-UTF-8 bytes read as `str`), and
    /// [`UnsupportedConversion`](DataError::UnsupportedConversion) when the
    /// scalar's type has no conversion to the target at all — the default for
    /// every accessor, so a concrete scalar overrides only the targets its value
    /// converts to. A scalar whose native type *is* the target answers directly,
    /// without conversion; `str` and byte access borrow without copying.
    fn as_i8(&self) -> Result<i8, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "i8",
        })
    }

    /// The value as an `i16`, when exactly representable.
    /// See [`as_i8`](Scalar::as_i8) for the shared contract.
    fn as_i16(&self) -> Result<i16, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "i16",
        })
    }

    /// The value as an `i32`, when exactly representable.
    /// See [`as_i8`](Scalar::as_i8) for the shared contract.
    fn as_i32(&self) -> Result<i32, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "i32",
        })
    }

    /// The value as an `i64`, when exactly representable.
    /// See [`as_i8`](Scalar::as_i8) for the shared contract.
    fn as_i64(&self) -> Result<i64, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "i64",
        })
    }

    /// The value as a `u8`, when exactly representable.
    /// See [`as_i8`](Scalar::as_i8) for the shared contract.
    fn as_u8(&self) -> Result<u8, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "u8",
        })
    }

    /// The value as a `u16`, when exactly representable.
    /// See [`as_i8`](Scalar::as_i8) for the shared contract.
    fn as_u16(&self) -> Result<u16, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "u16",
        })
    }

    /// The value as a `u32`, when exactly representable.
    /// See [`as_i8`](Scalar::as_i8) for the shared contract.
    fn as_u32(&self) -> Result<u32, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "u32",
        })
    }

    /// The value as a `u64`, when exactly representable.
    /// See [`as_i8`](Scalar::as_i8) for the shared contract.
    fn as_u64(&self) -> Result<u64, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "u64",
        })
    }

    /// The value as an [`f16`](half::f16), when exactly representable.
    /// See [`as_i8`](Scalar::as_i8) for the shared contract.
    fn as_f16(&self) -> Result<half::f16, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "f16",
        })
    }

    /// The value as an `f32`, when exactly representable.
    /// See [`as_i8`](Scalar::as_i8) for the shared contract.
    fn as_f32(&self) -> Result<f32, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "f32",
        })
    }

    /// The value as an `f64`, when exactly representable.
    /// See [`as_i8`](Scalar::as_i8) for the shared contract.
    fn as_f64(&self) -> Result<f64, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "f64",
        })
    }

    /// The value as a `bool`, when the value is a boolean.
    /// See [`as_i8`](Scalar::as_i8) for the shared contract.
    fn as_bool(&self) -> Result<bool, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "bool",
        })
    }

    /// The value as a `str`, when the value is a string or decodable bytes.
    ///
    /// `charset` picks the `yggdryl-core` [`Charset`](yggdryl_core::Charset)
    /// decoding the bytes; `None` defaults to UTF-8 and *borrows* without
    /// copying (`Cow::Borrowed`), while an explicit charset decodes through
    /// [`decode_bytes`](yggdryl_core::Charset::decode_bytes) into an owned
    /// string. See [`as_i8`](Scalar::as_i8) for the shared contract.
    fn as_str(
        &self,
        charset: Option<&dyn yggdryl_core::Charset>,
    ) -> Result<std::borrow::Cow<'_, str>, DataError> {
        let _ = charset;
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "str",
        })
    }

    /// The value as borrowed bytes, when the value is a byte sequence — borrowed
    /// directly, never copied.
    /// See [`as_i8`](Scalar::as_i8) for the shared contract.
    fn as_bytes(&self) -> Result<&[u8], DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "bytes",
        })
    }

    /// The value as a dynamic [`Serie`](crate::Serie), when the value is a
    /// sequence — the serie scalars answer with a zero-copy handle over their item
    /// serie (a reference-count bump, not a copy).
    /// See [`as_i8`](Scalar::as_i8) for the shared contract.
    fn as_serie(&self) -> Result<crate::Serie, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "serie",
        })
    }

    /// The value as a dynamic [`MapScalar`](crate::MapScalar), when the value is a
    /// key–value sequence — the map scalars answer with a zero-copy handle over
    /// their entries serie.
    /// See [`as_i8`](Scalar::as_i8) for the shared contract.
    fn as_map(&self) -> Result<crate::MapScalar, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "map",
        })
    }

    /// The value as a [`RecordScalar`](crate::RecordScalar), when the value is a
    /// struct row — the struct scalars answer with a zero-copy handle over their
    /// column series, giving the generic per-child scalar access.
    /// See [`as_i8`](Scalar::as_i8) for the shared contract.
    fn as_struct(&self) -> Result<crate::RecordScalar, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "struct",
        })
    }

    /// The value's raw little-endian byte encoding — the fixed-width type's own
    /// layout (`i64` as its 8 bytes, `binary` as its bytes), the source of the
    /// [`cast_dtype_unchecked`](Scalar::cast_dtype_unchecked) reinterpret cast.
    ///
    /// It defaults to the borrowed [`as_bytes`](Scalar::as_bytes) (so a byte
    /// sequence answers directly); a fixed-width scalar overrides it with its
    /// little-endian value bytes, and a wrapper (an optional) delegates to its
    /// inner scalar. A null scalar errors with [`DataError::NullValue`]; a type
    /// with no byte encoding errors with [`DataError::UnsupportedConversion`].
    fn value_le_bytes(&self) -> Result<Vec<u8>, DataError> {
        self.as_bytes().map(<[u8]>::to_vec)
    }

    /// A compact, human-readable rendering for fast debugging, with the default
    /// [`DisplayOptions`](crate::DisplayOptions) (10 rows, ~100 columns wide) — see
    /// [`display_with`](Scalar::display_with).
    fn display(&self) -> String {
        self.display_with(crate::DisplayOptions::default())
    }

    /// A compact, human-readable rendering for fast debugging. An **atomic** scalar
    /// is its value (`42`, `1.5`, `"hi"`, `0x0102`, `null`); a **serie** is a table
    /// headed by its field (name and type) with the first
    /// [`max_rows`](crate::DisplayOptions::max_rows) elements; a **struct** serie or
    /// record is a multi-column table (one column per field), each nested value shown
    /// compactly so the whole tries to fit [`max_width`](crate::DisplayOptions::max_width).
    ///
    /// The default renders the atomic value (through the one-element Arrow form); the
    /// serie and nested scalars override it with their tables.
    ///
    /// ```
    /// use yggdryl_scalar::{Int64Scalar, Scalar};
    /// assert_eq!(Int64Scalar::new(42).display(), "42");
    /// assert_eq!(Int64Scalar::null().display(), "null");
    /// ```
    fn display_with(&self, options: crate::DisplayOptions) -> String {
        let _ = options;
        crate::display::format_any(&crate::AnyScalar::from_arrow(self.to_arrow_scalar()))
    }

    /// Cast this scalar to `dtype`, returning the value re-typed as a one-element
    /// [`arrow_array::ArrayRef`] of the target type (rehydrate it with the target
    /// scalar's [`from_arrow`](Scalar::from_arrow)).
    ///
    /// The cast is **exact-or-error**, reusing the [`as_*`](Scalar::as_i8) contract:
    /// a null casts to a null of the target type, a value of the target type is
    /// returned unchanged, a numeric target reads the value through the matching
    /// `as_*` accessor (erroring with [`InexactConversion`](DataError::InexactConversion)
    /// when it would not represent the value exactly), a `utf8` target reads
    /// [`as_str`](Scalar::as_str) (validated UTF-8), and a `binary` target reads
    /// [`as_bytes`](Scalar::as_bytes). A target the source has no exact conversion to
    /// errors with [`UnsupportedConversion`](DataError::UnsupportedConversion) or, for
    /// a target outside the castable set (e.g. a nested type), with
    /// [`UnsupportedCast`](DataError::UnsupportedCast). For lossy byte-level casts
    /// between fixed-width, `binary` and `utf8` types, see
    /// [`cast_dtype_unchecked`](Scalar::cast_dtype_unchecked).
    ///
    /// ```
    /// use yggdryl_scalar::arrow_array::Array; // is_null on the arrow side
    /// use yggdryl_scalar::yggdryl_dtype::{Int32Type, Int64Type};
    /// use yggdryl_scalar::{Int32Scalar, Int64Scalar, Scalar};
    ///
    /// // int64 → int32, exact.
    /// let cast = Int64Scalar::new(42).cast_dtype(&Int32Type).unwrap();
    /// assert_eq!(Int32Scalar::from_arrow(cast.as_ref()).unwrap(), Int32Scalar::new(42));
    ///
    /// // A value that would not fit errors, exact-or-nothing.
    /// assert!(Int64Scalar::new(1 << 40).cast_dtype(&Int32Type).is_err());
    /// // A null casts to a null of the target type.
    /// assert!(Int64Scalar::null().cast_dtype(&Int32Type).unwrap().is_null(0));
    /// ```
    fn cast_dtype(
        &self,
        dtype: &dyn yggdryl_dtype::DataType,
    ) -> Result<arrow_array::ArrayRef, DataError> {
        let target = dtype.to_arrow();
        if self.is_null() {
            return Ok(arrow_array::new_null_array(&target, 1));
        }
        if target == self.data_type().to_arrow() {
            return Ok(self.to_arrow_scalar());
        }
        use arrow_schema::DataType as A;
        let array: arrow_array::ArrayRef = match &target {
            A::Int8 => Arc::new(arrow_array::Int8Array::from_iter_values([self.as_i8()?])),
            A::Int16 => Arc::new(arrow_array::Int16Array::from_iter_values([self.as_i16()?])),
            A::Int32 => Arc::new(arrow_array::Int32Array::from_iter_values([self.as_i32()?])),
            A::Int64 => Arc::new(arrow_array::Int64Array::from_iter_values([self.as_i64()?])),
            A::UInt8 => Arc::new(arrow_array::UInt8Array::from_iter_values([self.as_u8()?])),
            A::UInt16 => Arc::new(arrow_array::UInt16Array::from_iter_values([self.as_u16()?])),
            A::UInt32 => Arc::new(arrow_array::UInt32Array::from_iter_values([self.as_u32()?])),
            A::UInt64 => Arc::new(arrow_array::UInt64Array::from_iter_values([self.as_u64()?])),
            A::Float16 => Arc::new(arrow_array::Float16Array::from_iter_values(
                [self.as_f16()?],
            )),
            A::Float32 => Arc::new(arrow_array::Float32Array::from_iter_values(
                [self.as_f32()?],
            )),
            A::Float64 => Arc::new(arrow_array::Float64Array::from_iter_values(
                [self.as_f64()?],
            )),
            A::Boolean => Arc::new(arrow_array::BooleanArray::from(vec![self.as_bool()?])),
            A::Utf8 => Arc::new(arrow_array::StringArray::from_iter_values([self
                .as_str(None)?
                .into_owned()])),
            A::Binary => Arc::new(arrow_array::BinaryArray::from_iter_values([
                self.as_bytes()?
            ])),
            other => {
                return Err(DataError::UnsupportedCast {
                    from: self.data_type().name().to_string(),
                    to: other.to_string(),
                });
            }
        };
        Ok(array)
    }

    /// Cast this scalar to `dtype` by **reinterpreting its raw bytes**, returning the
    /// value re-typed as a one-element [`arrow_array::ArrayRef`] of the target type.
    ///
    /// Unlike the exact [`cast_dtype`](Scalar::cast_dtype), this bridges *every*
    /// fixed-width, `binary` and `utf8` type through
    /// [`value_le_bytes`](Scalar::value_le_bytes): a fixed-width target reads the
    /// source's little-endian bytes back with `from_le_bytes` (the widths must match,
    /// else [`InvalidByteLength`](DataError::InvalidByteLength)), a `binary` target
    /// takes the bytes as-is, and a `utf8` target reads them **without UTF-8
    /// validation**. A null casts to a null of the target type.
    ///
    /// # Safety
    ///
    /// The reinterpret may not round-trip and does not preserve the value's meaning
    /// (an `int32`'s bits read as an `f32`, `int64` bytes read as raw text). A `utf8`
    /// target is built with [`String::from_utf8_unchecked`], so the caller must accept
    /// that the resulting `str` may hold invalid UTF-8, breaking `str`'s invariant for
    /// any downstream reader.
    ///
    /// ```
    /// use yggdryl_scalar::yggdryl_dtype::{BinaryType, Int64Type};
    /// use yggdryl_scalar::{BinaryScalar, Int64Scalar, Scalar};
    ///
    /// // int64 → binary: its eight little-endian bytes...
    /// let bytes = unsafe { Int64Scalar::new(1).cast_dtype_unchecked(&BinaryType) }.unwrap();
    /// assert_eq!(
    ///     BinaryScalar::from_arrow(bytes.as_ref()).unwrap(),
    ///     BinaryScalar::new(1i64.to_le_bytes().to_vec()),
    /// );
    ///
    /// // ...and back, reinterpreting the bytes as an int64.
    /// let round = unsafe {
    ///     BinaryScalar::new(1i64.to_le_bytes().to_vec()).cast_dtype_unchecked(&Int64Type)
    /// }
    /// .unwrap();
    /// assert_eq!(Int64Scalar::from_arrow(round.as_ref()).unwrap(), Int64Scalar::new(1));
    /// ```
    unsafe fn cast_dtype_unchecked(
        &self,
        dtype: &dyn yggdryl_dtype::DataType,
    ) -> Result<arrow_array::ArrayRef, DataError> {
        let target = dtype.to_arrow();
        if self.is_null() {
            return Ok(arrow_array::new_null_array(&target, 1));
        }
        let bytes = self.value_le_bytes()?;
        use arrow_schema::DataType as A;
        let array: arrow_array::ArrayRef = match &target {
            A::Int8 => Arc::new(arrow_array::Int8Array::from_iter_values([
                i8::from_le_bytes(le_exact(&bytes)?),
            ])),
            A::Int16 => Arc::new(arrow_array::Int16Array::from_iter_values([
                i16::from_le_bytes(le_exact(&bytes)?),
            ])),
            A::Int32 => Arc::new(arrow_array::Int32Array::from_iter_values([
                i32::from_le_bytes(le_exact(&bytes)?),
            ])),
            A::Int64 => Arc::new(arrow_array::Int64Array::from_iter_values([
                i64::from_le_bytes(le_exact(&bytes)?),
            ])),
            A::UInt8 => Arc::new(arrow_array::UInt8Array::from_iter_values([
                u8::from_le_bytes(le_exact(&bytes)?),
            ])),
            A::UInt16 => Arc::new(arrow_array::UInt16Array::from_iter_values([
                u16::from_le_bytes(le_exact(&bytes)?),
            ])),
            A::UInt32 => Arc::new(arrow_array::UInt32Array::from_iter_values([
                u32::from_le_bytes(le_exact(&bytes)?),
            ])),
            A::UInt64 => Arc::new(arrow_array::UInt64Array::from_iter_values([
                u64::from_le_bytes(le_exact(&bytes)?),
            ])),
            A::Float16 => Arc::new(arrow_array::Float16Array::from_iter_values([
                half::f16::from_le_bytes(le_exact(&bytes)?),
            ])),
            A::Float32 => Arc::new(arrow_array::Float32Array::from_iter_values([
                f32::from_le_bytes(le_exact(&bytes)?),
            ])),
            A::Float64 => Arc::new(arrow_array::Float64Array::from_iter_values([
                f64::from_le_bytes(le_exact(&bytes)?),
            ])),
            A::Binary => Arc::new(arrow_array::BinaryArray::from_iter_values([
                bytes.as_slice()
            ])),
            A::Utf8 => {
                // SAFETY: the caller of this `unsafe fn` accepts that the bytes may
                // not be valid UTF-8 — the whole point of the unchecked reinterpret.
                let text = unsafe { String::from_utf8_unchecked(bytes) };
                Arc::new(arrow_array::StringArray::from_iter_values([text]))
            }
            other => {
                return Err(DataError::UnsupportedCast {
                    from: self.data_type().name().to_string(),
                    to: other.to_string(),
                });
            }
        };
        Ok(array)
    }
}

/// The exactly-`N`-byte little-endian window of `bytes`, or a
/// [`DataError::InvalidByteLength`] when the source is not the target's width — the
/// width check behind the fixed-width arms of
/// [`cast_dtype_unchecked`](Scalar::cast_dtype_unchecked).
fn le_exact<const N: usize>(bytes: &[u8]) -> Result<[u8; N], DataError> {
    bytes.try_into().map_err(|_| DataError::InvalidByteLength {
        expected: N,
        got: bytes.len(),
    })
}

/// One child array holding every element's one-element Arrow form, in order (an
/// empty array of `value_type` for an empty sequence) — the shared builder behind
/// the nested scalars' `to_arrow_scalar`. `value_type` is a closure so it is only built
/// for the empty sequence: a non-empty `concat` reads the type from the elements.
pub(crate) fn concat_scalar_arrays(
    elements: Vec<arrow_array::ArrayRef>,
    value_type: impl FnOnce() -> arrow_schema::DataType,
) -> arrow_array::ArrayRef {
    if elements.is_empty() {
        return arrow_array::new_empty_array(&value_type());
    }
    let refs: Vec<&dyn arrow_array::Array> =
        elements.iter().map(std::convert::AsRef::as_ref).collect();
    arrow_select::concat::concat(&refs).expect("one-element arrays of one type concatenate")
}

/// Every element of `elements` read back through the scalar's own `from_arrow` —
/// the shared reader behind the nested scalars' `from_arrow`.
pub(crate) fn scalars_from_elements<S: Scalar>(
    elements: &dyn arrow_array::Array,
) -> Result<Vec<S>, DataError> {
    (0..arrow_array::Array::len(elements))
        .map(|index| {
            let element = arrow_array::Array::slice(elements, index, 1);
            S::from_arrow(element.as_ref())
        })
        .collect()
}
