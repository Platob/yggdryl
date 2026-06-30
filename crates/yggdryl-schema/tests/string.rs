//! Tests for the string logical types.

use yggdryl_schema::{
    Charset, DataType, DataTypeId, LargeStringType, LargeStringViewType, LogicalType, StringType,
    StringViewType,
};

fn assert_logical<T: LogicalType>(_: &T) {}

#[test]
fn names_ids_and_category() {
    assert_eq!(StringType::new().name(), "string");
    assert_eq!(StringType::new().type_id(), DataTypeId::String);
    assert_eq!(LargeStringType::new().name(), "large_string");
    assert_eq!(LargeStringType::new().type_id(), DataTypeId::LargeString);
    assert_eq!(StringViewType::new().name(), "string_view");
    assert_eq!(StringViewType::new().type_id(), DataTypeId::StringView);
    assert_eq!(LargeStringViewType::new().name(), "large_string_view");
    assert_eq!(
        LargeStringViewType::new().type_id(),
        DataTypeId::LargeStringView
    );

    for id in [
        DataTypeId::String,
        DataTypeId::LargeString,
        DataTypeId::StringView,
        DataTypeId::LargeStringView,
    ] {
        assert!(id.is_logical());
        assert!(!id.is_physical());
    }
    assert_logical(&StringType::new());
}

#[test]
fn charset_default_and_update() {
    let s = StringType::new();
    assert_eq!(s.charset(), Charset::Utf8);
    let latin1 = s.with_charset(Charset::Latin1);
    assert_eq!(latin1.charset(), Charset::Latin1);
    // with_charset is non-mutating.
    assert_eq!(s.charset(), Charset::Utf8);
}

#[test]
fn backing_physical_types() {
    assert_eq!(StringType::new().physical().type_id(), DataTypeId::Binary);
    assert_eq!(
        LargeStringType::new().physical().type_id(),
        DataTypeId::LargeBinary
    );
    assert_eq!(
        StringViewType::new().physical().type_id(),
        DataTypeId::BinaryView
    );
    assert_eq!(
        LargeStringViewType::new().physical().type_id(),
        DataTypeId::LargeBinaryView
    );
}

#[test]
fn metadata_records_identity_and_charset() {
    // The default charset is implied: only the type identity is stored.
    let default = StringType::new().metadata();
    assert_eq!(
        default.get(b"yggdryl:type".as_slice()).map(Vec::as_slice),
        Some(b"string".as_slice())
    );
    assert!(!default.contains_key(b"yggdryl:charset".as_slice()));
    // A non-default charset is stored.
    let latin1 = StringType::new().with_charset(Charset::Latin1).metadata();
    assert_eq!(
        latin1.get(b"yggdryl:charset".as_slice()).map(Vec::as_slice),
        Some(b"latin1".as_slice())
    );
}

#[cfg(feature = "serde")]
#[test]
fn serde_round_trip() {
    let s = StringType::new().with_charset(Charset::Ascii);
    let json = serde_json::to_string(&s).unwrap();
    assert_eq!(serde_json::from_str::<StringType>(&json).unwrap(), s);
}

#[cfg(feature = "arrow")]
mod arrow {
    use arrow_schema::DataType as ArrowType;
    use yggdryl_schema::{
        Charset, DataType, LargeStringType, LargeStringViewType, Metadata, StringType,
        StringViewType,
    };

    #[test]
    fn map_to_arrow() {
        assert_eq!(StringType::new().to_arrow_type(), ArrowType::Utf8);
        assert_eq!(LargeStringType::new().to_arrow_type(), ArrowType::LargeUtf8);
        assert_eq!(StringViewType::new().to_arrow_type(), ArrowType::Utf8View);
        // Arrow has no large string-view: lossy map to Utf8View.
        assert_eq!(
            LargeStringViewType::new().to_arrow_type(),
            ArrowType::Utf8View
        );
    }

    #[test]
    fn round_trip_charset_via_metadata() {
        // The default charset rebuilds from empty metadata.
        assert_eq!(
            StringType::from_arrow_type(&ArrowType::Utf8, &Metadata::new()).unwrap(),
            StringType::new()
        );
        // A non-default charset round-trips through the metadata it produced.
        let latin1 = StringType::new().with_charset(Charset::Latin1);
        let rebuilt = StringType::from_arrow_type(&ArrowType::Utf8, &latin1.metadata()).unwrap();
        assert_eq!(rebuilt, latin1);
        // A wrong Arrow type errors.
        assert!(StringType::from_arrow_type(&ArrowType::Binary, &Metadata::new()).is_err());
    }
}
