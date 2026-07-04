//! Integration tests for [`AnyScalar`], the type-erased atomic scalar, and the
//! [`AnySerie::get_scalar`] bridge that reads one out of a column.

use std::sync::Arc;

use yggdryl_scalar::yggdryl_dtype::arrow_schema;
use yggdryl_scalar::{
    arrow_array, AnyScalar, AnySerie, BinaryScalar, Int32Scalar, Int64Scalar, Int64Serie,
    NullScalar, Scalar,
};

#[test]
fn decomposes_integers_and_falls_back_for_others() {
    // An integer value decomposes to its concrete scalar.
    let integer = AnyScalar::from_arrow(Arc::new(arrow_array::Int64Array::from(vec![42])));
    assert!(matches!(integer, AnyScalar::Int64(_)));
    assert!(!integer.is_null());
    assert_eq!(integer.data_type(), arrow_schema::DataType::Int64);

    // Any other type keeps its one-element Arrow array in the fallback.
    let bytes = AnyScalar::from_arrow(BinaryScalar::new(vec![1, 2, 3]).to_arrow_scalar());
    assert!(matches!(bytes, AnyScalar::Arrow(_)));
    assert_eq!(bytes.data_type(), arrow_schema::DataType::Binary);
}

#[test]
fn from_concrete_and_from_arrow_agree() {
    let arrow: arrow_array::ArrayRef = Arc::new(arrow_array::Int64Array::from(vec![7]));
    assert_eq!(
        AnyScalar::from(Int64Scalar::new(7)),
        AnyScalar::from_arrow(arrow.clone())
    );
    // to_arrow_scalar reconstitutes the same one-element array (shared buffers).
    assert_eq!(
        AnyScalar::from(Int64Scalar::new(7))
            .to_arrow_scalar()
            .as_ref(),
        arrow.as_ref()
    );
}

#[test]
fn null_survives_both_representations() {
    let integer = AnyScalar::from_arrow(Int64Scalar::null().to_arrow_scalar());
    assert!(integer.is_null());
    assert!(matches!(integer, AnyScalar::Int64(_)));

    let nothing = AnyScalar::from_arrow(NullScalar::default().to_arrow_scalar());
    assert!(nothing.is_null());
    assert!(matches!(nothing, AnyScalar::Arrow(_)));
}

#[test]
fn equality_bridges_representations() {
    // A decomposed value equals its zero-copy passthrough twin.
    let decomposed = AnyScalar::from(Int64Scalar::new(5));
    let passthrough = AnyScalar::Arrow(Int64Scalar::new(5).to_arrow_scalar());
    assert_eq!(decomposed, passthrough);

    assert_ne!(decomposed, AnyScalar::from(Int64Scalar::new(6)));
    // Different widths of the same numeric value are distinct.
    assert_ne!(
        AnyScalar::from(Int64Scalar::new(5)),
        AnyScalar::from(Int32Scalar::new(5))
    );
}

#[test]
fn any_serie_get_scalar_reads_one_element() {
    // A decomposed column reads the element straight from its buffer.
    let column = AnySerie::from(Int64Serie::from(vec![10, 20, 30]));
    assert_eq!(
        column.get_scalar(1),
        Some(AnyScalar::from(Int64Scalar::new(20)))
    );
    assert!(column.get_scalar(3).is_none()); // out of bounds

    // An Arrow-fallback column slices one element and decomposes it.
    let bytes = AnySerie::from_arrow(Arc::new(arrow_array::BinaryArray::from_iter_values([
        b"a".as_ref(),
        b"bc".as_ref(),
    ])));
    let first = bytes.get_scalar(0).unwrap();
    assert!(matches!(first, AnyScalar::Arrow(_)));
    assert_eq!(
        BinaryScalar::from_arrow(first.to_arrow_scalar().as_ref()).unwrap(),
        BinaryScalar::new(b"a".to_vec())
    );
}
