//! The **buffer → field** bridge. A [buffer](yggdryl_buffer) is pure io+data with no
//! schema of its own; the field layer, sitting above it, names it and sets its
//! nullability — schema is applied **from above** (`CLAUDE.md` re-layer).

use yggdryl_buffer::{
    BooleanBuffer, F32Buffer, F64Buffer, I16Buffer, I32Buffer, I64Buffer, I8Buffer, U16Buffer,
    U32Buffer, U64Buffer, U8Buffer,
};

/// Turns a typed buffer into its matching typed [`Field`](crate::Field), naming it and
/// setting nullability from above.
///
/// The associated [`Field`](ToField::Field) is the buffer's counterpart in this layer
/// (`I64Buffer` → [`I64Field`](crate::I64Field), the byte/`U8Buffer` → [`U8Field`](crate::U8Field),
/// `BooleanBuffer` → [`BooleanField`](crate::BooleanField)).
///
/// ```
/// use yggdryl_buffer::I64Buffer;
/// use yggdryl_field::{Field, ToField};
///
/// let field = I64Buffer::from_slice(&[1, 2, 3]).to_field("ts", true);
/// assert_eq!(field.name(), "ts");
/// assert!(field.is_nullable());
/// ```
pub trait ToField {
    /// The matching field type (`I64Buffer` → `I64Field`).
    type Field;

    /// Builds the matching field named `name`, nullable `nullable`.
    fn to_field(&self, name: impl Into<String>, nullable: bool) -> Self::Field;
}

macro_rules! impl_to_field {
    ($buf:ty, $field:ident) => {
        impl ToField for $buf {
            type Field = crate::$field;

            fn to_field(&self, name: impl Into<String>, nullable: bool) -> crate::$field {
                crate::$field::new(name, nullable)
            }
        }
    };
}

impl_to_field!(I8Buffer, I8Field);
// `U8Buffer` is an alias of `ByteBuffer`, so this is the byte buffer's bridge too.
impl_to_field!(U8Buffer, U8Field);
impl_to_field!(I16Buffer, I16Field);
impl_to_field!(U16Buffer, U16Field);
impl_to_field!(I32Buffer, I32Field);
impl_to_field!(U32Buffer, U32Field);
impl_to_field!(I64Buffer, I64Field);
impl_to_field!(U64Buffer, U64Field);
impl_to_field!(F32Buffer, F32Field);
impl_to_field!(F64Buffer, F64Field);
impl_to_field!(BooleanBuffer, BooleanField);
