//! The broadened numeric family added on top of `u8`…`i64`/`f32`/`f64`: half-precision `f16`,
//! the 128-bit `u128`/`i128`, and the wide non-Arrow-native `u96`/`i96`/`u256`/`i256` `[u8; N]`
//! newtypes. Focus: the little-endian codec + `Scalar`/`Serie` round-trips, the bit-canonical
//! value identity (including the `f16` `NaN`/`±0.0` cases), the `DataTypeId` drill-down, and the
//! closest-Arrow fallback.

use std::collections::HashSet;

use yggdryl_core::io::fixed::{
    f16, Buffer, F16Scalar, F16Serie, I128Scalar, I256Scalar, U128Scalar, U256Scalar, U96Scalar,
    I256, I96, U256, U96,
};
use yggdryl_core::io::{Bytes, DataType, DataTypeId, IOCursor};

// -------------------------------------------------------------------------------------
// f16
// -------------------------------------------------------------------------------------

#[test]
fn f16_round_trips_and_categorizes() {
    let value = F16Scalar::of(f16::from_f32(1.5));
    assert_eq!(value.value(), Some(f16::from_f32(1.5)));
    assert_eq!(value.data_type().type_id(), DataTypeId::F16);
    assert!(value.data_type().is_floating() && value.data_type().is_numeric());
    assert_eq!(value.data_type().byte_width(), 2);

    let mut sink = Bytes::new();
    value.write_to(&mut sink).unwrap();
    sink.rewind();
    assert_eq!(F16Scalar::read_from(&mut sink).unwrap(), value);

    let col = F16Serie::from_options(&[Some(f16::from_f32(1.0)), None, Some(f16::from_f32(2.0))]);
    assert_eq!(col.len(), 3);
    assert_eq!(col.null_count(), 1);
    assert_eq!(col.get(2), Some(f16::from_f32(2.0)));
}

#[test]
fn f16_identity_is_bit_canonical() {
    // NaN == NaN when the bits match (unlike IEEE `==`), so a NaN scalar is a usable key.
    let nan = F16Scalar::of(f16::NAN);
    assert_eq!(nan, F16Scalar::of(f16::NAN));

    // +0.0 and -0.0 have different bytes, so they are distinct keys (bit-canonical).
    let pos_zero = F16Scalar::of(f16::from_f32(0.0));
    let neg_zero = F16Scalar::of(f16::from_f32(-0.0));
    assert_ne!(pos_zero, neg_zero);

    // Hashable everywhere: usable in a set.
    let mut set = HashSet::new();
    set.insert(F16Scalar::of(f16::from_f32(1.0)));
    set.insert(F16Scalar::of(f16::from_f32(1.0)));
    set.insert(F16Scalar::of(f16::NAN));
    set.insert(F16Scalar::of(f16::NAN));
    assert_eq!(set.len(), 2); // {1.0, NaN}
}

// -------------------------------------------------------------------------------------
// u128 / i128 (Rust-native, not Arrow-native)
// -------------------------------------------------------------------------------------

#[test]
fn u128_i128_round_trip_and_predicates() {
    let big = U128Scalar::of(u128::MAX);
    assert!(big.data_type().is_unsigned_integer() && big.data_type().is_integer());
    assert!(!big.data_type().is_signed());
    assert_eq!(big.data_type().byte_width(), 16);
    let mut sink = Bytes::new();
    big.write_to(&mut sink).unwrap();
    sink.rewind();
    assert_eq!(U128Scalar::read_from(&mut sink).unwrap(), big);

    let neg = I128Scalar::of(i128::MIN);
    assert!(neg.data_type().is_signed_integer() && neg.data_type().is_signed());
    assert_eq!(neg.data_type().type_id(), DataTypeId::I128);
    let mut sink = Bytes::new();
    neg.write_to(&mut sink).unwrap();
    sink.rewind();
    assert_eq!(I128Scalar::read_from(&mut sink).unwrap(), neg);
}

// -------------------------------------------------------------------------------------
// Wide [u8; N] newtypes: u96 / i96 / u256 / i256
// -------------------------------------------------------------------------------------

#[test]
fn wide_newtypes_byte_codec_and_identity() {
    let a = U96::from_le_bytes([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    assert_eq!(a.to_le_bytes(), [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);

    // Byte-wise equality/hash — usable as a set key.
    let mut set = HashSet::new();
    set.insert(a);
    set.insert(U96::from_le_bytes([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12])); // equal
    set.insert(U96::default()); // all zero
    assert_eq!(set.len(), 2);

    // Round-trips as a Scalar through a byte sink.
    let s = U96Scalar::of(a);
    let mut sink = Bytes::new();
    s.write_to(&mut sink).unwrap();
    sink.rewind();
    assert_eq!(U96Scalar::read_from(&mut sink).unwrap(), s);

    // i256 (32 bytes) likewise.
    let mut b = [0u8; 32];
    b[31] = 0x80; // a "high bit set" pattern
    let big = I256Scalar::of(I256::from_le_bytes(b));
    let mut sink = Bytes::new();
    big.write_to(&mut sink).unwrap();
    sink.rewind();
    assert_eq!(I256Scalar::read_from(&mut sink).unwrap(), big);
}

#[test]
fn wide_newtypes_buffer_zero_copy_slice() {
    // Buffer construction + the zero-copy typed view work for the align-1 newtypes (as_slice
    // is total — its element-alignment assert can never fire for align-1 types).
    let values = [
        I96::from_le_bytes([1; 12]),
        I96::from_le_bytes([2; 12]),
        I96::from_le_bytes([3; 12]),
    ];
    let buffer = Buffer::<I96>::from_slice(&values);
    assert_eq!(buffer.count(), 3);
    assert_eq!(buffer.get(1), Some(I96::from_le_bytes([2; 12])));
    assert_eq!(buffer.as_slice(), &values);

    // A nullable column of wide values.
    let col = yggdryl_core::io::fixed::U256Serie::from_options(&[
        Some(U256::from_le_bytes([9; 32])),
        None,
    ]);
    assert_eq!(col.len(), 2);
    assert_eq!(col.get(0), Some(U256::from_le_bytes([9; 32])));
    assert_eq!(col.get(1), None);
    let _ = U256Scalar::null();
}

#[test]
fn wide_newtypes_predicates() {
    fn dt<T: yggdryl_core::io::fixed::NativeType>() -> yggdryl_core::io::fixed::PrimitiveType<T> {
        yggdryl_core::io::fixed::PrimitiveType::new()
    }
    assert!(dt::<U96>().is_unsigned_integer() && dt::<U96>().is_fixed_width());
    assert!(dt::<I96>().is_signed_integer());
    assert!(dt::<U256>().is_unsigned_integer());
    assert!(dt::<I256>().is_signed_integer());
    assert_eq!(dt::<U256>().byte_width(), 32);
    assert_eq!(dt::<U96>().byte_width(), 12);
    // None of the wide ints is variable-length or a float.
    for is_var in [
        dt::<U96>().is_variable_length(),
        dt::<I256>().is_variable_length(),
        dt::<U96>().is_floating(),
    ] {
        assert!(!is_var);
    }
}

// -------------------------------------------------------------------------------------
// Closest-Arrow fallback (feature `arrow`)
// -------------------------------------------------------------------------------------

#[cfg(feature = "arrow")]
#[test]
fn closest_arrow_representation() {
    use arrow_schema::DataType as A;
    use yggdryl_core::io::fixed::PrimitiveType;
    // The Arrow-native f16 maps exactly.
    assert_eq!(PrimitiveType::<f16>::new().to_arrow(), A::Float16);
    // The wide types map to their closest representation.
    assert_eq!(
        PrimitiveType::<u128>::new().to_arrow(),
        A::FixedSizeBinary(16)
    );
    assert_eq!(
        PrimitiveType::<i128>::new().to_arrow(),
        A::Decimal128(38, 0)
    );
    assert_eq!(
        PrimitiveType::<U96>::new().to_arrow(),
        A::FixedSizeBinary(12)
    );
    assert_eq!(
        PrimitiveType::<I96>::new().to_arrow(),
        A::FixedSizeBinary(12)
    );
    assert_eq!(
        PrimitiveType::<U256>::new().to_arrow(),
        A::FixedSizeBinary(32)
    );
    assert_eq!(
        PrimitiveType::<I256>::new().to_arrow(),
        A::Decimal256(76, 0)
    );
}
