//! Tests for the dynamic [`AnyType`] carrier.

use yggdryl_schema::{AnyType, BinaryType, DataType, DataTypeId, LargeBinaryViewType, StringType};

#[test]
fn wraps_and_delegates_every_type() {
    // Each concrete type converts into the matching variant and reports its own
    // name, id and category through the enum.
    let cases: [(AnyType, &str, DataTypeId); 3] = [
        (BinaryType::new().into(), "binary", DataTypeId::Binary),
        (
            LargeBinaryViewType::new().into(),
            "large_binary_view",
            DataTypeId::LargeBinaryView,
        ),
        (StringType::new().into(), "string", DataTypeId::String),
    ];
    for (any, name, id) in cases {
        assert_eq!(any.name(), name);
        assert_eq!(any.type_id(), id);
    }
    assert!(AnyType::from(BinaryType::new()).is_physical());
    assert!(AnyType::from(StringType::new()).is_logical());
}

#[test]
fn delegates_byte_size_and_metadata() {
    let any = AnyType::from(BinaryType::new().with_byte_size(8));
    assert_eq!(any.max_byte_size(), Some(8));
    // The metadata is the inner type's — identity plus the cap.
    assert_eq!(
        any.metadata()
            .get(b"yggdryl:type".as_slice())
            .map(Vec::as_slice),
        Some(b"binary".as_slice())
    );
    assert_eq!(
        any.metadata()
            .get(b"yggdryl:byte_size".as_slice())
            .map(Vec::as_slice),
        Some(b"8".as_slice())
    );
}

#[cfg(feature = "serde")]
#[test]
fn serde_round_trip() {
    use yggdryl_schema::Charset;

    let any = AnyType::from(
        StringType::new()
            .with_charset(Charset::Latin1)
            .with_byte_size(4),
    );
    let json = serde_json::to_string(&any).unwrap();
    assert_eq!(serde_json::from_str::<AnyType>(&json).unwrap(), any);
}

#[cfg(feature = "arrow")]
mod arrow {
    use arrow_schema::DataType as ArrowType;
    use yggdryl_schema::{
        AnyType, BinaryType, DataType, LargeBinaryViewType, Metadata, StringType,
    };

    #[test]
    fn to_arrow_type_delegates() {
        assert_eq!(
            AnyType::from(BinaryType::new()).to_arrow_type(),
            ArrowType::Binary
        );
        // The lossy large-binary-view still maps to BinaryView through the enum.
        assert_eq!(
            AnyType::from(LargeBinaryViewType::new()).to_arrow_type(),
            ArrowType::BinaryView
        );
        assert_eq!(
            AnyType::from(StringType::new()).to_arrow_type(),
            ArrowType::Utf8
        );
    }

    #[test]
    fn from_arrow_type_uses_the_type_metadata() {
        // The reserved `yggdryl:type` name selects the exact variant, even when the
        // Arrow type alone would be ambiguous (BinaryView ← large_binary_view).
        let original = AnyType::from(LargeBinaryViewType::new().with_byte_size(16));
        let rebuilt =
            AnyType::from_arrow_type(&original.to_arrow_type(), &original.metadata()).unwrap();
        assert_eq!(rebuilt, original);
    }

    #[test]
    fn from_arrow_type_infers_without_metadata() {
        let none = Metadata::new();
        // A bare Arrow type (no yggdryl metadata) infers the non-large variant.
        assert_eq!(
            AnyType::from_arrow_type(&ArrowType::Binary, &none).unwrap(),
            AnyType::from(BinaryType::new())
        );
        assert_eq!(
            AnyType::from_arrow_type(&ArrowType::Utf8, &none).unwrap(),
            AnyType::from(StringType::new())
        );
        // An Arrow type with no yggdryl equivalent errors.
        assert!(AnyType::from_arrow_type(&ArrowType::Int32, &none).is_err());
    }
}
