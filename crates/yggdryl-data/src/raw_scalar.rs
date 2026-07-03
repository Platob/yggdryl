//! The [`RawScalar`] base trait: a single, possibly-null value of a [`RawDataType`].

use super::{DataError, RawDataType};

/// A single value of a data type, possibly null — the base trait mirroring an Apache
/// Arrow `Scalar`.
///
/// It carries its [`data_type`](RawScalar::data_type) of type `D`, reports whether it
/// [`is_null`](RawScalar::is_null), and exposes the native Rust
/// [`value`](RawScalar::value) (of the associated [`Value`](RawScalar::Value) type)
/// when non-null. Arrow models a scalar as an array of exactly one value, so
/// [`to_arrow`](RawScalar::to_arrow) builds a one-element
/// [`arrow_array::ArrayRef`] (null when the scalar is null) and
/// [`from_arrow`](RawScalar::from_arrow) reads one back. Parameterising by `D` keeps
/// the concrete type available for zero-cost access; the associated `Value` names the
/// in-memory representation a concrete scalar holds. It shares [`RawDataType`]'s
/// `Debug + Send + Sync` bounds so scalar values are printable and shareable across
/// threads and FFI. The associated [`Value`](RawScalar::Value) is `?Sized`, so a
/// string scalar can expose `Value = str`.
///
/// ```
/// use yggdryl_data::{arrow_array, DataError, Int32, RawDataType, RawScalar};
/// use arrow_array::Array; // len / is_null on the arrow side
///
/// #[derive(Debug)]
/// struct Int32Scalar {
///     data_type: Int32,
///     value: Option<i32>,
/// }
///
/// impl RawScalar<Int32> for Int32Scalar {
///     type Value = i32;
///     fn data_type(&self) -> &Int32 {
///         &self.data_type
///     }
///     fn is_null(&self) -> bool {
///         self.value.is_none()
///     }
///     fn value(&self) -> Option<&i32> {
///         self.value.as_ref()
///     }
///     fn to_arrow(&self) -> arrow_array::ArrayRef {
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
///                 expected: "Int32".to_string(),
///                 got: array.data_type().to_string(),
///             })?;
///         Ok(Int32Scalar {
///             data_type: Int32,
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
/// let answer = Int32Scalar { data_type: Int32, value: Some(42) };
/// assert_eq!(answer.data_type().name(), "int32");
/// assert!(!answer.is_null());
/// assert_eq!(answer.value(), Some(&42));
/// assert_eq!(answer.as_i64().unwrap(), 42); // converted access
/// // An int32 has no str conversion (the default): an actionable error.
/// assert!(matches!(answer.as_str(), Err(DataError::UnsupportedConversion { .. })));
///
/// // Arrow interop: a one-element array, round-tripped.
/// let arrow = answer.to_arrow();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(Int32Scalar::from_arrow(arrow.as_ref()).unwrap().value(), Some(&42));
///
/// let missing = Int32Scalar { data_type: Int32, value: None };
/// assert!(missing.is_null());
/// assert!(missing.to_arrow().is_null(0));
/// ```
pub trait RawScalar<D: RawDataType>: std::fmt::Debug + Send + Sync {
    /// The native Rust representation this scalar holds when non-null. May be
    /// unsized (e.g. `str`).
    type Value: ?Sized;

    /// The scalar's data type.
    fn data_type(&self) -> &D;

    /// Whether this scalar holds a null value.
    fn is_null(&self) -> bool;

    /// The scalar's value, or `None` when it [`is_null`](RawScalar::is_null).
    fn value(&self) -> Option<&Self::Value>;

    /// The Apache Arrow form of this scalar: a one-element
    /// [`arrow_array::ArrayRef`] of this scalar's data type, holding the value (or a
    /// null). This is Arrow's own scalar representation — a length-1 array — so it
    /// plugs straight into arrow-rs kernels (wrap it in `arrow_array::Scalar` for a
    /// `Datum`).
    fn to_arrow(&self) -> arrow_array::ArrayRef;

    /// Build this scalar from its one-element Apache Arrow array — the exact inverse
    /// of [`to_arrow`](RawScalar::to_arrow). An array whose length is not exactly 1
    /// errors with [`DataError::InvalidScalarLength`]; an array of a different Arrow
    /// type errors with [`DataError::IncompatibleArrowType`].
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
    /// See [`as_i8`](RawScalar::as_i8) for the shared contract.
    fn as_i16(&self) -> Result<i16, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "i16",
        })
    }

    /// The value as an `i32`, when exactly representable.
    /// See [`as_i8`](RawScalar::as_i8) for the shared contract.
    fn as_i32(&self) -> Result<i32, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "i32",
        })
    }

    /// The value as an `i64`, when exactly representable.
    /// See [`as_i8`](RawScalar::as_i8) for the shared contract.
    fn as_i64(&self) -> Result<i64, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "i64",
        })
    }

    /// The value as a `u8`, when exactly representable.
    /// See [`as_i8`](RawScalar::as_i8) for the shared contract.
    fn as_u8(&self) -> Result<u8, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "u8",
        })
    }

    /// The value as a `u16`, when exactly representable.
    /// See [`as_i8`](RawScalar::as_i8) for the shared contract.
    fn as_u16(&self) -> Result<u16, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "u16",
        })
    }

    /// The value as a `u32`, when exactly representable.
    /// See [`as_i8`](RawScalar::as_i8) for the shared contract.
    fn as_u32(&self) -> Result<u32, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "u32",
        })
    }

    /// The value as a `u64`, when exactly representable.
    /// See [`as_i8`](RawScalar::as_i8) for the shared contract.
    fn as_u64(&self) -> Result<u64, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "u64",
        })
    }

    /// The value as an `f32`, when exactly representable.
    /// See [`as_i8`](RawScalar::as_i8) for the shared contract.
    fn as_f32(&self) -> Result<f32, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "f32",
        })
    }

    /// The value as an `f64`, when exactly representable.
    /// See [`as_i8`](RawScalar::as_i8) for the shared contract.
    fn as_f64(&self) -> Result<f64, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "f64",
        })
    }

    /// The value as a `bool`, when the value is a boolean.
    /// See [`as_i8`](RawScalar::as_i8) for the shared contract.
    fn as_bool(&self) -> Result<bool, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "bool",
        })
    }

    /// The value as a borrowed `&str`, when the value is a string (or bytes that
    /// are valid UTF-8) — borrowed directly, never copied.
    /// See [`as_i8`](RawScalar::as_i8) for the shared contract.
    fn as_str(&self) -> Result<&str, DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "str",
        })
    }

    /// The value as borrowed bytes, when the value is a byte sequence — borrowed
    /// directly, never copied.
    /// See [`as_i8`](RawScalar::as_i8) for the shared contract.
    fn as_bytes(&self) -> Result<&[u8], DataError> {
        Err(DataError::UnsupportedConversion {
            data_type: self.data_type().name().to_string(),
            target: "bytes",
        })
    }
}

/// One child array holding every element's one-element Arrow form, in order (an
/// empty array of `value_type` for an empty sequence) — the shared builder behind
/// the nested scalars' `to_arrow`.
pub(crate) fn concat_scalar_arrays(
    elements: Vec<arrow_array::ArrayRef>,
    value_type: &arrow_schema::DataType,
) -> arrow_array::ArrayRef {
    if elements.is_empty() {
        return arrow_array::new_empty_array(value_type);
    }
    let refs: Vec<&dyn arrow_array::Array> =
        elements.iter().map(std::convert::AsRef::as_ref).collect();
    arrow_select::concat::concat(&refs).expect("one-element arrays of one type concatenate")
}

/// Every element of `elements` read back through the scalar's own `from_arrow` —
/// the shared reader behind the nested scalars' `from_arrow`.
pub(crate) fn scalars_from_elements<D: RawDataType, S: RawScalar<D>>(
    elements: &dyn arrow_array::Array,
) -> Result<Vec<S>, DataError> {
    (0..arrow_array::Array::len(elements))
        .map(|index| {
            let element = arrow_array::Array::slice(elements, index, 1);
            S::from_arrow(element.as_ref())
        })
        .collect()
}
