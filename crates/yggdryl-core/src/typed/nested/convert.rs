//! [`ToValue`] / [`FromValue`] — the **native ↔ erased [`Value`]** bridge that lets the generic
//! `*_any` accessors on the [`Serie`](crate::typed::Serie) trait erase a concrete column's element
//! into a [`Value`] (and back) with a single per-native impl.
//!
//! [`ToValue`] wraps a decoded native scalar into the matching [`Value`] variant (`i64` →
//! [`Value::Int64`], `String` → [`Value::Utf8`], …); [`FromValue`] is its inverse, extracting the
//! native back out of a `Value` (returning `None` on a variant mismatch). The pair is keyed on the
//! **native Rust type**, so the width-sharing types resolve to one canonical variant: `i32` maps to
//! [`Value::Int32`] (a `Decimal32` column, whose native is also `i32`, therefore erases through
//! `Int32` — the erased [`Column`](crate::typed::Column) path keeps the precise decimal variant).
//!
//! The unit type `()` (the value of a [`NullSerie`](crate::typed::NullSerie)) bridges to
//! [`Value::Null`], so an all-null column participates in the same erased surface.

use crate::datatype_id::DataTypeId;
use crate::io::memory::IoError;
use crate::typed::fixedbyte::{F16, I256};
use crate::typed::nested::Value;

/// Erases a concrete **native scalar** into the tagged [`Value`] union — the forward half of the
/// bridge the generic [`get_any_value_at`](crate::typed::Serie::get_any_value_at) uses.
pub trait ToValue {
    /// This native value wrapped in its matching [`Value`] variant.
    fn to_value(self) -> Value;
}

/// Extracts a concrete **native scalar** from the erased [`Value`] — the inverse of [`ToValue`],
/// used by [`set_any_scalar_at`](crate::typed::Serie::set_any_scalar_at). Returns `None` when
/// `value`'s variant does not match the target native (the caller turns that into a guided error).
pub trait FromValue: Sized {
    /// The native extracted from `value`, or `None` on a variant mismatch.
    fn from_value(value: &Value) -> Option<Self>;
}

/// Generates the [`ToValue`] + [`FromValue`] pair for a **`Copy`** native and its [`Value`] variant
/// (the numeric / bool primitives + [`I256`]).
macro_rules! copy_bridge {
    ( $( $native:ty => $variant:ident ),+ $(,)? ) => {
        $(
            impl ToValue for $native {
                fn to_value(self) -> Value {
                    Value::$variant(self)
                }
            }
            impl FromValue for $native {
                fn from_value(value: &Value) -> Option<Self> {
                    match value {
                        Value::$variant(inner) => Some(*inner),
                        _ => None,
                    }
                }
            }
        )+
    };
}

copy_bridge! {
    i8 => Int8,
    u8 => UInt8,
    i16 => Int16,
    u16 => UInt16,
    i32 => Int32,
    u32 => UInt32,
    i64 => Int64,
    u64 => UInt64,
    i128 => Int128,
    u128 => UInt128,
    f32 => Float32,
    f64 => Float64,
    bool => Bool,
    F16 => Float16,
    I256 => Decimal256,
}

impl ToValue for Vec<u8> {
    fn to_value(self) -> Value {
        Value::Binary(self)
    }
}

impl FromValue for Vec<u8> {
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Binary(bytes) => Some(bytes.clone()),
            _ => None,
        }
    }
}

impl ToValue for String {
    fn to_value(self) -> Value {
        Value::Utf8(self)
    }
}

impl FromValue for String {
    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Utf8(text) => Some(text.clone()),
            _ => None,
        }
    }
}

impl ToValue for () {
    /// The all-null column's unit element erases to [`Value::Null`].
    fn to_value(self) -> Value {
        Value::Null
    }
}

/// The guided [`IoError::TypedCast`] for a [`set_any_scalar_at`](crate::typed::Serie::set_any_scalar_at)
/// whose erased [`Value`] does not match the column's element type — names **both** the column's
/// dtype and the value's, and the fix.
pub(crate) fn set_any_dtype_error(column: DataTypeId, value: &Value) -> IoError {
    IoError::TypedCast {
        detail: format!(
            "cannot set a {} value into a {} column: pass a Value whose type matches the column's \
             element type ({})",
            value.data_type_id(),
            column,
            column
        ),
    }
}
