//! Tests for the broadened fixed family: the full numeric primitive set (widths 1–8, signed,
//! and IEEE-754 float), the `Serie` ⇄ `Scalar` interop, and the optimized `dtype` / `field`
//! accessors.

use yggdryl_core::io::fixed::{
    F64Buffer, F64Scalar, F64Serie, Field, I64Buffer, I64Scalar, PrimitiveType, Scalar, Serie,
    U16Buffer, U16Serie,
};
use yggdryl_core::io::{Bytes, DataType, IOBase, IOCursor};

// -------------------------------------------------------------------------------------
// The whole numeric family: little-endian codec across widths + serialization
// -------------------------------------------------------------------------------------

#[test]
fn u16_two_byte_codec_and_round_trip() {
    let b = U16Buffer::from_slice(&[1, 0x0102, 0xffff]);
    assert_eq!(b.len(), 6); // 3 × 2 bytes
    assert_eq!(b.as_bytes(), &[1, 0, 2, 1, 0xff, 0xff]);
    assert_eq!(b.get(1), Some(0x0102));

    let col = U16Serie::from_options(&[Some(10), None, Some(0xffff)]);
    let mut sink = Bytes::new();
    col.write_to(&mut sink).unwrap();
    sink.rewind();
    assert_eq!(U16Serie::read_from(&mut sink).unwrap(), col);
}

#[test]
fn i64_eight_byte_signed_codec() {
    let b = I64Buffer::from_slice(&[-1, 0, i64::MIN, i64::MAX]);
    assert_eq!(b.len(), 32); // 4 × 8 bytes
    assert_eq!(b.get(0), Some(-1));
    assert_eq!(b.get(2), Some(i64::MIN));
    assert_eq!(b.get(3), Some(i64::MAX));

    // Scalar round-trip preserves the sign and full width.
    for value in [
        I64Scalar::of(-9_000_000_000),
        I64Scalar::of(i64::MIN),
        I64Scalar::null(),
    ] {
        let mut sink = Bytes::new();
        value.write_to(&mut sink).unwrap();
        assert_eq!(sink.len(), 9); // 1 validity + 8 value
        sink.rewind();
        assert_eq!(I64Scalar::read_from(&mut sink).unwrap(), value);
    }
}

#[test]
fn f64_float_codec_and_column() {
    let b = F64Buffer::from_slice(&[1.5, -2.25, 3.0]);
    assert_eq!(b.get(1), Some(-2.25)); // exact — these are representable

    // Float series compare by content (bytes) and round-trip through a sink.
    let col = F64Serie::from_options(&[Some(1.5), None, Some(-2.25)]);
    assert_eq!(col.null_count(), 1);
    let mut sink = Bytes::new();
    col.write_to(&mut sink).unwrap();
    sink.rewind();
    let read = F64Serie::read_from(&mut sink).unwrap();
    assert_eq!(read.to_options(), col.to_options());

    // A float scalar carries its value (Eq/Hash aren't available for floats, PartialEq is).
    assert_eq!(F64Scalar::of(2.5).value(), Some(2.5));
}

// -------------------------------------------------------------------------------------
// Serie ⇄ Scalar interop
// -------------------------------------------------------------------------------------

#[test]
fn serie_yields_and_is_built_from_scalars() {
    let col = Serie::from_options(&[Some(1i32), None, Some(3)]);
    assert_eq!(col.get_scalar(0), Scalar::of(1));
    assert_eq!(col.get_scalar(1), Scalar::null()); // a null element -> null scalar
    assert_eq!(col.get_scalar(9), Scalar::null()); // out of range -> null scalar

    // A length-1 column is usable as a scalar; a longer one is not.
    assert_eq!(
        Serie::from_values(&[42i32]).as_scalar(),
        Some(Scalar::of(42))
    );
    assert_eq!(col.as_scalar(), None);
    assert_eq!(Serie::<i32>::new().as_scalar(), None);

    // Scalar -> Serie -> Scalar round-trips (both present and null).
    for scalar in [Scalar::of(7i32), Scalar::null()] {
        let broadcast = scalar.to_serie();
        assert_eq!(broadcast.len(), 1);
        assert_eq!(broadcast.as_scalar(), Some(scalar.clone()));
        assert_eq!(Serie::from_scalar(scalar), broadcast);
    }
}

// -------------------------------------------------------------------------------------
// Optimized dtype / field accessors
// -------------------------------------------------------------------------------------

#[test]
fn typed_data_type_const_accessors() {
    // Compile-time constants — no method call.
    const NAME: &str = PrimitiveType::<i32>::NAME;
    const WIDTH: usize = PrimitiveType::<i32>::BYTE_WIDTH;
    assert_eq!(NAME, "i32");
    assert_eq!(WIDTH, 4);
    assert_eq!(PrimitiveType::<f64>::BYTE_WIDTH, 8);

    // const fn accessors work in const context too.
    const DT: PrimitiveType<u16> = PrimitiveType::new();
    assert_eq!(DT.type_name(), "u16");
    assert_eq!(DT.width(), 2);
}

#[test]
fn values_expose_data_type_and_field() {
    let buffer = I64Buffer::from_vec(vec![1, 2, 3]);
    assert_eq!(buffer.data_type().name(), "i64");
    assert_eq!(buffer.field("id", false).erase().type_name(), "i64");

    let scalar = F64Scalar::of(1.0);
    assert_eq!(scalar.data_type().byte_width(), 8);
    assert!(scalar.typed_field("x", true).nullable());

    // Serie::to_field infers nullability from the column's actual nulls.
    let dense = Serie::from_values(&[1i32, 2, 3]);
    assert!(!dense.to_field("c").nullable());
    let sparse = Serie::from_options(&[Some(1i32), None]);
    assert!(sparse.to_field("c").nullable());
}

#[test]
fn field_typed_check() {
    let field = Field::new("id", &<PrimitiveType<i32>>::new(), false);
    assert!(field.is::<i32>());
    assert!(!field.is::<u16>());
    assert!(!field.is::<f64>());
}
