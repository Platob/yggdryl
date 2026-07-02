//! `PrimitiveArray` construction, access, zero-copy views and round-trips.

use arrow_buffer::{NullBuffer, ScalarBuffer};
use yggdryl_array::{Array, ArrayError, PrimitiveArray};
use yggdryl_scalar::Scalar;
use yggdryl_schema::{Decimal128, Float64, Int32, Millisecond, Timestamp, UInt8};

#[test]
fn construction_and_access() {
    let column = PrimitiveArray::from_native(Int32, vec![1, 2, 3]);
    assert_eq!(column.len(), 3);
    assert!(!column.is_empty());
    assert_eq!(column.null_count(), 0);
    assert!(column.validity().is_none());
    assert_eq!(column.value(1), Some(2));
    assert_eq!(column.value(9), None);

    let sparse = PrimitiveArray::from_options(Int32, vec![Some(1), None, Some(3)]);
    assert_eq!(sparse.null_count(), 1);
    assert_eq!(sparse.is_null(1), Some(true));
    assert_eq!(sparse.value(1), None);
    assert_eq!(sparse.value(2), Some(3));
    // An all-valid options vector normalizes to no bitmap at all.
    assert!(PrimitiveArray::from_options(Int32, vec![Some(1)])
        .validity()
        .is_none());

    let timestamps = PrimitiveArray::from_native(
        Timestamp::from_parts(Millisecond, Some("UTC".into())),
        vec![1_700_000_000_000i64],
    );
    assert_eq!(timestamps.value(0), Some(1_700_000_000_000));
}

#[test]
fn from_parts_validates_lengths() {
    let values = ScalarBuffer::from(vec![1i32, 2, 3]);
    let validity: NullBuffer = [true, false].iter().copied().collect();
    assert_eq!(
        PrimitiveArray::from_parts(Int32, values, Some(validity)),
        Err(ArrayError::LengthMismatch {
            values: 3,
            validity: 2
        })
    );
}

#[test]
fn slices_are_zero_copy_views() {
    let column = PrimitiveArray::from_options(Int32, vec![Some(1), None, Some(3), Some(4)]);
    let view = column.slice(1, 3).unwrap();
    assert_eq!(view.len(), 3);
    assert_eq!(view.value(0), None);
    assert_eq!(view.value(1), Some(3));
    // The view shares the parent's allocation instead of copying it.
    assert_eq!(
        view.values().inner().as_ptr(),
        column.values().inner().as_slice()[4..].as_ptr(),
    );
    assert!(matches!(
        column.slice(3, 2),
        Err(ArrayError::SliceOutOfBounds { .. })
    ));
}

#[test]
fn scalars_are_zero_copy_slices() {
    let decimal = Decimal128::from_parts(38, 2).unwrap();
    let column = PrimitiveArray::from_options(decimal, vec![Some(123i128), None]);

    assert_eq!(column.scalar(0), Some(Scalar::from_native(decimal, 123)));
    assert_eq!(column.scalar(1), Some(Scalar::null(decimal)));
    assert_eq!(column.scalar(2), None);
    // The extracted scalar's buffer points into the array's allocation.
    let scalar = column.scalar(0).unwrap();
    assert_eq!(
        scalar.buffer().unwrap().as_ptr(),
        column.values().inner().as_ptr(),
    );
}

#[test]
fn arrays_roundtrip_through_bytes() {
    let dense = PrimitiveArray::from_native(UInt8, vec![1, 2, 3]);
    assert_eq!(PrimitiveArray::from_bytes(&dense.to_bytes()), Ok(dense));

    let sparse = PrimitiveArray::from_options(
        Float64,
        vec![Some(1.5), None, Some(f64::NAN), None, Some(0.0)],
    );
    let decoded = PrimitiveArray::from_bytes(&sparse.to_bytes()).unwrap();
    assert_eq!(decoded, sparse); // bit-wise, so the NaN slot compares equal

    let empty = PrimitiveArray::from_native(Int32, vec![]);
    assert_eq!(PrimitiveArray::from_bytes(&empty.to_bytes()), Ok(empty));

    // Corrupted payloads are rejected with typed errors.
    let encoded = PrimitiveArray::from_native(Int32, vec![7]).to_bytes();
    assert!(matches!(
        PrimitiveArray::<Int32>::from_bytes(&encoded[..encoded.len() - 1]),
        Err(ArrayError::InvalidByteLength { .. })
    ));
    assert!(PrimitiveArray::<Int32>::from_bytes(&[1, 2]).is_err());
}

#[test]
fn equality_and_hashing_are_content_based() {
    use std::collections::HashSet;

    let a = PrimitiveArray::from_options(Int32, vec![Some(1), None]);
    let b = PrimitiveArray::from_bytes(&a.to_bytes()).unwrap();
    assert_eq!(a, b);
    assert_ne!(
        a,
        PrimitiveArray::from_options(Int32, vec![Some(1), Some(2)])
    );
    assert_ne!(a, PrimitiveArray::from_options(Int32, vec![None, Some(1)]));

    let mut set = HashSet::new();
    set.insert(a);
    assert!(set.contains(&b));
}
