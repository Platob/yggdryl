//! Tests for the binary data types.

use yggdryl_schema::{
    BinaryType, BinaryViewType, DataType, DataTypeId, FixedSizeBinaryType, LargeBinaryType,
    LargeBinaryViewType, MaxedSizeBinaryType, PhysicalType,
};

fn assert_physical<T: PhysicalType>(_: &T) {}

#[test]
fn names_ids_and_category() {
    assert_eq!(BinaryType.name(), "binary");
    assert_eq!(BinaryType.type_id(), DataTypeId::Binary);
    assert_eq!(LargeBinaryType.name(), "large_binary");
    assert_eq!(LargeBinaryType.type_id(), DataTypeId::LargeBinary);
    assert_eq!(BinaryViewType.name(), "binary_view");
    assert_eq!(BinaryViewType.type_id(), DataTypeId::BinaryView);
    assert_eq!(LargeBinaryViewType.name(), "large_binary_view");
    assert_eq!(LargeBinaryViewType.type_id(), DataTypeId::LargeBinaryView);
    assert_eq!(FixedSizeBinaryType::new(8).name(), "fixed_size_binary");
    assert_eq!(
        FixedSizeBinaryType::new(8).type_id(),
        DataTypeId::FixedSizeBinary
    );
    assert_eq!(MaxedSizeBinaryType::new(8).name(), "maxed_size_binary");
    assert_eq!(
        MaxedSizeBinaryType::new(8).type_id(),
        DataTypeId::MaxedSizeBinary
    );

    for id in [
        DataTypeId::Binary,
        DataTypeId::LargeBinary,
        DataTypeId::BinaryView,
        DataTypeId::LargeBinaryView,
        DataTypeId::FixedSizeBinary,
        DataTypeId::MaxedSizeBinary,
    ] {
        assert!(id.is_physical());
    }
    assert_physical(&BinaryType);
    assert_physical(&FixedSizeBinaryType::new(8));
    assert_physical(&MaxedSizeBinaryType::new(8));
}

#[test]
fn fixed_and_max_size_limits() {
    // Fixed size is an exact width; maxed size is a cap. Both report a max byte size.
    assert!(DataTypeId::FixedSizeBinary.is_fixed_size());
    assert!(!DataTypeId::MaxedSizeBinary.is_fixed_size());
    assert!(!DataTypeId::Binary.is_fixed_size());

    assert_eq!(FixedSizeBinaryType::new(4).max_byte_size(), Some(4));
    assert_eq!(MaxedSizeBinaryType::new(4).max_byte_size(), Some(4));
    assert_eq!(BinaryType.max_byte_size(), None);
}

#[test]
fn fixed_size_accessors() {
    let ty = FixedSizeBinaryType::new(16);
    assert_eq!(ty.byte_size(), 16);
    assert_eq!(ty.large_byte_size(), 16_i64);
    assert_eq!(ty.with_byte_size(4).byte_size(), 4);
    assert_eq!(ty.with_large_byte_size(32).byte_size(), 32);
    assert_eq!(ty.with_large_byte_size(32).large_byte_size(), 32_i64);
    // A 64-bit width beyond i32 clamps to i32::MAX.
    assert_eq!(ty.with_large_byte_size(i64::MAX).byte_size(), i32::MAX);
}

#[cfg(feature = "serde")]
#[test]
fn fixed_size_serde_round_trip() {
    let ty = FixedSizeBinaryType::new(16);
    let json = serde_json::to_string(&ty).unwrap();
    assert_eq!(
        serde_json::from_str::<FixedSizeBinaryType>(&json).unwrap(),
        ty
    );
}

#[cfg(feature = "arrow")]
mod arrow {
    use arrow_schema::DataType as ArrowType;
    use yggdryl_schema::{
        BinaryType, BinaryViewType, DataType, FixedSizeBinaryType, LargeBinaryType,
        LargeBinaryViewType, MaxedSizeBinaryType, Metadata,
    };

    #[test]
    fn map_to_arrow() {
        assert_eq!(BinaryType.to_arrow_type(), ArrowType::Binary);
        assert_eq!(LargeBinaryType.to_arrow_type(), ArrowType::LargeBinary);
        assert_eq!(BinaryViewType.to_arrow_type(), ArrowType::BinaryView);
        // Arrow has no large binary-view: lossy map to BinaryView.
        assert_eq!(LargeBinaryViewType.to_arrow_type(), ArrowType::BinaryView);
        assert_eq!(
            FixedSizeBinaryType::new(12).to_arrow_type(),
            ArrowType::FixedSizeBinary(12)
        );
        // Arrow has no size-capped binary: lossy map to Binary; the cap is stashed
        // in the reserved metadata instead.
        assert_eq!(
            MaxedSizeBinaryType::new(4).to_arrow_type(),
            ArrowType::Binary
        );
        assert_eq!(
            MaxedSizeBinaryType::new(4)
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
            BinaryType
        );
        assert_eq!(
            LargeBinaryType::from_arrow_type(&ArrowType::LargeBinary, &none).unwrap(),
            LargeBinaryType
        );
        assert_eq!(
            BinaryViewType::from_arrow_type(&ArrowType::BinaryView, &none).unwrap(),
            BinaryViewType
        );
        assert_eq!(
            FixedSizeBinaryType::from_arrow_type(&ArrowType::FixedSizeBinary(7), &none).unwrap(),
            FixedSizeBinaryType::new(7)
        );
        // A non-matching Arrow type errors.
        assert!(BinaryType::from_arrow_type(&ArrowType::Utf8, &none).is_err());
        // A maxed-size type needs its cap in the metadata to rebuild.
        let mut metadata = Metadata::new();
        metadata.insert(b"yggdryl:byte_size".to_vec(), b"4".to_vec());
        assert_eq!(
            MaxedSizeBinaryType::from_arrow_type(&ArrowType::Binary, &metadata).unwrap(),
            MaxedSizeBinaryType::new(4)
        );
        assert!(MaxedSizeBinaryType::from_arrow_type(&ArrowType::Binary, &none).is_err());
    }
}
