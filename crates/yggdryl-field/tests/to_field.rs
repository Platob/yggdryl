//! The buffer → field bridge ([`ToField`]): a typed buffer names itself into its matching
//! typed field from above (a buffer carries no schema of its own).

use yggdryl_buffer::{BooleanBuffer, I64Buffer, U8Buffer};
use yggdryl_field::{Field, ToField, TypedField};

#[test]
fn buffer_bridges_to_the_matching_typed_field() {
    // The associated type is the concrete `I64Field` for `I64Buffer` (compile-time).
    let field: yggdryl_field::I64Field = I64Buffer::from_slice(&[1, 2, 3]).to_field("id", false);
    assert_eq!(field.name(), "id");
    assert!(!field.is_nullable());
    let _ = TypedField::data_type(&field); // the typed data type is reachable

    // The boolean buffer bridges to a `BooleanField`.
    let bfield: yggdryl_field::BooleanField =
        BooleanBuffer::from_bits(&[true, false]).to_field("flag", true);
    assert_eq!(bfield.name(), "flag");
    assert!(bfield.is_nullable());

    // `U8Buffer` is the byte store (an alias of `ByteBuffer`); it bridges to `U8Field`.
    let u: yggdryl_field::U8Field = U8Buffer::from_slice(&[1, 2]).to_field("bytes", false);
    assert_eq!(u.name(), "bytes");
    assert!(!u.is_nullable());
}
