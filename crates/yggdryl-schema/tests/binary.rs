//! Tests for the binary data types.

use yggdryl_schema::{
    BinaryType, BinaryViewType, DataType, DataTypeId, LargeBinaryType, LargeBinaryViewType,
    PhysicalType,
};

fn assert_physical<T: PhysicalType>(_: &T) {}

#[test]
fn names_ids_and_category() {
    assert_eq!(BinaryType::new().name(), "binary");
    assert_eq!(BinaryType::new().type_id(), DataTypeId::Binary);
    assert_eq!(LargeBinaryType::new().name(), "large_binary");
    assert_eq!(LargeBinaryType::new().type_id(), DataTypeId::LargeBinary);
    assert_eq!(BinaryViewType::new().name(), "binary_view");
    assert_eq!(BinaryViewType::new().type_id(), DataTypeId::BinaryView);
    assert_eq!(LargeBinaryViewType::new().name(), "large_binary_view");
    assert_eq!(
        LargeBinaryViewType::new().type_id(),
        DataTypeId::LargeBinaryView
    );

    for id in [
        DataTypeId::Binary,
        DataTypeId::LargeBinary,
        DataTypeId::BinaryView,
        DataTypeId::LargeBinaryView,
    ] {
        assert!(id.is_physical());
    }
    assert_physical(&BinaryType::new());
    assert_physical(&BinaryType::new().with_byte_size(8));
}

#[test]
fn byte_size_cap() {
    // Unbounded by default; a cap reports a max byte size.
    assert_eq!(BinaryType::new().max_byte_size(), None);
    assert_eq!(BinaryType::new().byte_size(), None);
    assert_eq!(BinaryType::new().with_byte_size(4).max_byte_size(), Some(4));
    assert_eq!(BinaryType::new().with_byte_size(4).byte_size(), Some(4));
}

#[test]
fn byte_size_accessors() {
    let ty = BinaryType::new().with_byte_size(16);
    assert_eq!(ty.byte_size(), Some(16));
    assert_eq!(ty.with_byte_size(4).byte_size(), Some(4));
    assert_eq!(ty.without_byte_size().byte_size(), None);
}

#[cfg(feature = "serde")]
#[test]
fn serde_round_trip() {
    let ty = BinaryType::new().with_byte_size(16);
    let json = serde_json::to_string(&ty).unwrap();
    assert_eq!(serde_json::from_str::<BinaryType>(&json).unwrap(), ty);
}

#[cfg(feature = "arrow")]
mod arrow {
    use arrow_schema::DataType as ArrowType;
    use yggdryl_schema::{
        BinaryType, BinaryViewType, DataType, LargeBinaryType, LargeBinaryViewType, Metadata,
    };

    #[test]
    fn map_to_arrow() {
        assert_eq!(BinaryType::new().to_arrow_type(), ArrowType::Binary);
        assert_eq!(
            LargeBinaryType::new().to_arrow_type(),
            ArrowType::LargeBinary
        );
        assert_eq!(BinaryViewType::new().to_arrow_type(), ArrowType::BinaryView);
        // Arrow has no large binary-view: lossy map to BinaryView.
        assert_eq!(
            LargeBinaryViewType::new().to_arrow_type(),
            ArrowType::BinaryView
        );
        // Arrow has no size-capped binary: a cap maps to the same variable Binary,
        // with the cap stashed in the reserved metadata instead.
        assert_eq!(
            BinaryType::new().with_byte_size(4).to_arrow_type(),
            ArrowType::Binary
        );
        assert_eq!(
            BinaryType::new()
                .with_byte_size(4)
                .metadata()
                .get(b"yggdryl:byte_size".as_slice())
                .map(Vec::as_slice),
            Some(b"4".as_slice())
        );
    }

    #[test]
    fn round_trip_from_arrow() {
        let none = Metadata::new();
        assert_eq!(
            BinaryType::from_arrow_type(&ArrowType::Binary, &none).unwrap(),
            BinaryType::new()
        );
        assert_eq!(
            LargeBinaryType::from_arrow_type(&ArrowType::LargeBinary, &none).unwrap(),
            LargeBinaryType::new()
        );
        assert_eq!(
            BinaryViewType::from_arrow_type(&ArrowType::BinaryView, &none).unwrap(),
            BinaryViewType::new()
        );
        // A non-matching Arrow type errors.
        assert!(BinaryType::from_arrow_type(&ArrowType::Utf8, &none).is_err());
        // A byte-size cap rebuilds from the reserved metadata.
        let mut metadata = Metadata::new();
        metadata.insert(b"yggdryl:byte_size".to_vec(), b"4".to_vec());
        assert_eq!(
            BinaryType::from_arrow_type(&ArrowType::Binary, &metadata).unwrap(),
            BinaryType::new().with_byte_size(4)
        );
        // Without the cap metadata, the type rebuilds as unbounded.
        assert_eq!(
            BinaryType::from_arrow_type(&ArrowType::Binary, &none).unwrap(),
            BinaryType::new()
        );
    }
}
