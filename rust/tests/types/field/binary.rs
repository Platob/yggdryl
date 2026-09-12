//! The byte family's field: one marker over every layout, and the value door.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, FixedSizeBinaryArray};
use yggdryl::types::{Bytes, BytesField, BytesLayout, BytesParameters, bytes};
use yggdryl::{ArrowCastOptions, DataType, DataTypeId, FieldScalar, Scalar};

use super::typed::assert_typed_marker;

#[test]
fn the_bytes_marker_covers_every_layout_and_bound() {
    assert_typed_marker::<bytes::BytesType>(DataType::binary());
    assert_typed_marker::<bytes::BytesType>(DataType::large_binary());
    assert_typed_marker::<bytes::BytesType>(DataType::binary_view());
    assert_typed_marker::<bytes::BytesType>(DataType::fixed_size_binary(16).unwrap());
    assert_typed_marker::<bytes::BytesType>(DataType::from_str("binary(16)").unwrap());
    assert_typed_marker::<bytes::BytesType>(DataType::from_str("large_binary(64)").unwrap());

    // The family is parameterized, so the field takes its datatype through
    // `try_new`, and a datatype from another family is refused by name.
    let digest =
        BytesField::try_new("digest", DataType::fixed_size_binary(32).unwrap(), false).unwrap();
    assert_eq!(digest.dtype(), &DataType::fixed_size_binary(32).unwrap());
    assert!(BytesField::try_new("digest", DataType::utf8(), false).is_err());
    assert!(BytesField::try_new("digest", DataType::Uuid, false).is_err());
    assert!(BytesField::try_new("digest", DataType::geometry(None).unwrap(), false).is_err());

    // A bound of zero is refused wherever it is stated.
    assert!(DataType::fixed_size_binary(0).is_err());
    assert!(
        BytesParameters::new(BytesLayout::Binary)
            .try_with_bound(0)
            .is_err()
    );
    // A fixed layout built by hand with no width is what `validate` catches.
    assert!(DataType::bytes(BytesParameters::new(BytesLayout::FixedSizeBinary)).is_err());
}

#[test]
fn a_fixed_value_is_exactly_its_width() {
    let field =
        BytesField::try_new("digest", DataType::fixed_size_binary(4).unwrap(), false).unwrap();
    let held = FieldScalar::new(field.as_field(), vec![1_u8, 2, 3, 4]).unwrap();
    assert_eq!(held.as_bytes(), Some(&[1_u8, 2, 3, 4][..]));
    assert_eq!(held.value().id(), DataTypeId::FixedSizeBinary);
    assert_eq!(
        held.value().dtype().unwrap(),
        DataType::fixed_size_binary(4).unwrap()
    );

    // Bytes are never padded: shorter and longer are both refused, naming
    // the width and what arrived.
    for (payload, held) in [(&[1_u8, 2, 3][..], "3 bytes"), (&[1_u8; 5][..], "5 bytes")] {
        let refused = FieldScalar::new(field.as_field(), payload)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("exactly 4 bytes"), "{refused}");
        assert!(refused.contains(held), "{refused}");
    }
}

#[test]
fn a_bounded_value_is_at_most_the_maximum_and_never_carries_it() {
    let field =
        BytesField::try_new("payload", DataType::from_str("binary(4)").unwrap(), true).unwrap();
    let held = FieldScalar::new(field.as_field(), vec![7_u8; 4]).unwrap();
    assert_eq!(held.as_bytes(), Some(&[7_u8; 4][..]));
    // The maximum is the column's rule: the value answers the layout alone.
    assert_eq!(held.value().dtype().unwrap(), DataType::binary());
    assert!(FieldScalar::new(field.as_field(), Vec::<u8>::new()).is_ok());

    let refused = FieldScalar::new(field.as_field(), vec![7_u8; 5])
        .unwrap_err()
        .to_string();
    assert!(refused.contains("at most 4 bytes"), "{refused}");
    assert!(refused.contains("5 bytes"), "{refused}");

    // The same door on the wider layouts.
    let view = BytesField::try_new(
        "payload",
        DataType::from_str("binary_view(2)").unwrap(),
        true,
    )
    .unwrap();
    assert!(FieldScalar::new(view.as_field(), vec![1_u8, 2]).is_ok());
    assert!(FieldScalar::new(view.as_field(), vec![1_u8, 2, 3]).is_err());
}

#[test]
fn a_value_adopts_the_layout_of_the_column_that_holds_it() {
    let large = BytesField::try_new("payload", DataType::large_binary(), false).unwrap();
    let held = FieldScalar::new(large.as_field(), Scalar::from(b"abc")).unwrap();
    let Scalar::Bytes(bytes) = held.value() else {
        panic!("a byte column holds a byte value");
    };
    assert_eq!(bytes.layout(), BytesLayout::LargeBinary);
    assert_eq!(bytes.fixed(), None);
    // The layout is a retag over the same payload, so equality reads the
    // payload alone.
    assert_eq!(*bytes, Bytes::new(b"abc"));

    // Text and a UUID spell their bytes into a byte column; a small value
    // stays inline.
    let plain = BytesField::try_new("payload", DataType::binary(), false).unwrap();
    assert_eq!(
        FieldScalar::new(plain.as_field(), "abc")
            .unwrap()
            .as_bytes(),
        Some(&b"abc"[..])
    );
    let Scalar::Bytes(inline) = FieldScalar::new(plain.as_field(), "abc")
        .unwrap()
        .into_value()
    else {
        panic!("a byte column holds a byte value");
    };
    assert!(inline.is_inline());
}

#[test]
fn a_bounded_column_checks_each_cell_on_the_way_in() {
    let field =
        BytesField::try_new("payload", DataType::from_str("binary(3)").unwrap(), true).unwrap();
    let source: ArrayRef = Arc::new(BinaryArray::from(vec![
        Some(&b"abc"[..]),
        Some(&b"abcd"[..]),
        None,
    ]));

    // Strict: the refusal names the field and the row.
    let refused = field
        .cast_arrow_array(
            Arc::clone(&source),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap_err()
        .to_string();
    assert!(refused.contains("\"payload\""), "{refused}");
    assert!(refused.contains("row 1"), "{refused}");
    assert!(refused.contains("at most 3 bytes"), "{refused}");

    // Safe: the cell that does not fit becomes null.
    let cast = field
        .cast_arrow_array(source, ArrowCastOptions::new())
        .unwrap();
    let cast = cast.as_any().downcast_ref::<BinaryArray>().unwrap();
    assert_eq!(cast.value(0), b"abc");
    assert!(cast.is_null(1));
    assert!(cast.is_null(2));
}

#[test]
fn a_fixed_column_is_its_own_storage() {
    let field =
        BytesField::try_new("digest", DataType::fixed_size_binary(4).unwrap(), true).unwrap();
    let source: ArrayRef = Arc::new(
        FixedSizeBinaryArray::try_from_sparse_iter_with_size(
            [Some(&b"abcd"[..]), None].into_iter(),
            4,
        )
        .unwrap(),
    );
    let cast = field
        .cast_arrow_array(
            Arc::clone(&source),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
    assert!(Arc::ptr_eq(&cast, &source));

    // Variable bytes of another length are refused into the width; the
    // width is the storage, so Arrow's own kernel is what refuses them.
    let source: ArrayRef = Arc::new(BinaryArray::from(vec![
        Some(&b"abcd"[..]),
        Some(&b"abc"[..]),
    ]));
    assert!(
        field
            .cast_arrow_array(source, ArrowCastOptions::new().with_safe(false))
            .is_err()
    );
}
