//! One byte family: six leaves, each its own column shape.

use arrow_schema::DataType as ArrowDataType;
use arrow_schema::extension::{EXTENSION_TYPE_METADATA_KEY, EXTENSION_TYPE_NAME_KEY};
use yggdryl::{BYTES_EXTENSION_NAME, Bytes, BytesType, INLINE_BYTES};
use yggdryl::{DataType, DataTypeId, Field, Scalar};

#[test]
fn every_leaf_is_one_datatype_under_every_spelling() {
    for (leaf, id, spelling) in [
        (BytesType::Binary, DataTypeId::Binary, "binary"),
        (
            BytesType::LargeBinary,
            DataTypeId::LargeBinary,
            "large_binary",
        ),
        (BytesType::BinaryView, DataTypeId::BinaryView, "binary_view"),
        (
            BytesType::LargeBinaryView,
            DataTypeId::LargeBinaryView,
            "large_binary_view",
        ),
        (
            BytesType::FixedBinary(16),
            DataTypeId::FixedBinary,
            "fixed_binary",
        ),
        (
            BytesType::SizedBinary(16),
            DataTypeId::SizedBinary,
            "sized_binary",
        ),
    ] {
        assert_eq!(leaf.as_str(), spelling);
        assert_eq!(leaf.id(), id);
        assert_eq!(BytesType::from_id(id, 16), Some(leaf));
        assert_eq!(BytesType::from_str(spelling).unwrap().id(), id);
        // The fold is the grammar's: case, underscores and hyphens all drop.
        assert_eq!(
            BytesType::from_str(&spelling.to_uppercase()).unwrap().id(),
            id
        );
        assert_eq!(
            BytesType::from_str(&spelling.replace('_', "-"))
                .unwrap()
                .id(),
            id
        );
        assert!(id.is_binary(), "{id}");
        assert_eq!(id.fixed_byte_width(), None, "{id}");

        let dtype = DataType::bytes(leaf).unwrap();
        assert!(dtype.to_string().starts_with(spelling), "{dtype}");
        assert_eq!(DataType::from_str(&dtype.to_string()).unwrap(), dtype);
        assert_eq!(dtype.id(), id);
        assert_eq!(dtype.bytes_parameters(), Some(leaf));
        assert_eq!(dtype.string_parameters(), None);
    }

    // The sugar constructors are the same datatypes, and `binary` is the
    // family's default.
    assert_eq!(DataType::binary(), DataType::Bytes(BytesType::default()));
    assert_eq!(
        DataType::large_binary(),
        DataType::bytes(BytesType::LargeBinary).unwrap()
    );
    assert_eq!(
        DataType::binary_view(),
        DataType::bytes(BytesType::BinaryView).unwrap()
    );
    assert_eq!(
        DataType::large_binary_view(),
        DataType::bytes(BytesType::LargeBinaryView).unwrap()
    );
    assert_eq!(
        DataType::fixed_binary(16).unwrap().to_string(),
        "fixed_binary(16)"
    );
    assert_eq!(
        DataType::sized_binary(16).unwrap().to_string(),
        "sized_binary(16)"
    );

    // The accepted aliases render as the canonical spellings, and a number
    // after a plain layout is a maximum - which is its own leaf.
    for (alias, canonical) in [
        ("bytes", "binary"),
        ("blob", "binary"),
        ("bytea", "binary"),
        ("varbinary", "binary"),
        ("varbinary(16)", "sized_binary(16)"),
        ("binary(16)", "sized_binary(16)"),
        ("fixed_binary(16)", "fixed_binary(16)"),
        ("FixedSizeBinary(16)", "fixed_binary(16)"),
        ("largebinary", "large_binary"),
        ("BinaryView", "binary_view"),
        ("large_binary_view", "large_binary_view"),
    ] {
        assert_eq!(
            DataType::from_str(alias).unwrap().to_string(),
            canonical,
            "{alias}"
        );
    }

    // A UUID and a geospatial value are bytes with an identity, not byte
    // columns.
    assert_eq!(DataType::uuid().bytes_parameters(), None);
    assert_eq!(DataType::geometry(None).unwrap().bytes_parameters(), None);
}

#[test]
fn a_width_and_a_maximum_are_two_leaves_and_never_one_column() {
    // `sized_binary(16)` is a maximum of sixteen bytes and
    // `fixed_binary(16)` the exact width; the two readings never both answer.
    let bounded = DataType::from_str("binary(16)").unwrap();
    let parameters = bounded.bytes_parameters().unwrap();
    assert_eq!(parameters, BytesType::SizedBinary(16));
    assert_eq!(parameters.max(), Some(16));
    assert_eq!(parameters.fixed(), None);
    assert_eq!(parameters.bound(), Some(16));
    assert!(parameters.is_bounded());
    assert!(!parameters.is_fixed());
    assert_eq!(bounded.fixed_byte_width(), None);
    assert_ne!(bounded, DataType::binary());

    let fixed = DataType::fixed_binary(16).unwrap();
    let parameters = fixed.bytes_parameters().unwrap();
    assert_eq!(parameters, BytesType::FixedBinary(16));
    assert_eq!(parameters.fixed(), Some(16));
    assert_eq!(parameters.max(), None);
    assert!(parameters.is_fixed());
    assert_eq!(fixed.fixed_byte_width(), Some(16));
    assert_ne!(bounded, fixed);

    // The value a sized column holds is the plain binary it fills: the
    // maximum is the column's rule and never the value's.
    assert_eq!(BytesType::SizedBinary(16).storage(), BytesType::Binary);
    assert_eq!(
        BytesType::FixedBinary(16).storage(),
        BytesType::FixedBinary(16)
    );
    assert_eq!(BytesType::LargeBinary.storage(), BytesType::LargeBinary);

    // A column of no bytes is not a column, whichever leaf states the number.
    assert!(DataType::from_str("binary(0)").is_err());
    assert!(DataType::from_str("fixed_binary(0)").is_err());
    assert!(DataType::fixed_binary(0).is_err());
    assert!(DataType::sized_binary(0).is_err());
    assert!(BytesType::FixedBinary(0).validate().is_err());
    assert!(BytesType::SizedBinary(0).validate().is_err());
    assert!(
        DataType::Bytes(BytesType::FixedBinary(0))
            .validate()
            .is_err()
    );

    // A bare fixed spelling is a question rather than a declaration.
    assert!(DataType::from_str("fixed_size_binary").is_err());

    // Which offsets and whether it is viewed are the leaf's, and answered
    // without a match.
    assert!(BytesType::LargeBinaryView.is_view());
    assert!(BytesType::LargeBinaryView.is_large());
    assert!(BytesType::BinaryView.is_view());
    assert!(!BytesType::BinaryView.is_large());
    assert!(!BytesType::SizedBinary(16).is_view());
}

#[test]
fn a_byte_value_is_the_compact_byte_string_and_carries_no_maximum() {
    // A short payload lives inside the value with no heap behind it, a
    // static one costs nothing, and a longer one is one shared handle.
    let short = Bytes::new([1_u8, 2, 3]);
    assert!(short.is_inline());
    assert!(Bytes::new(vec![0_u8; INLINE_BYTES]).is_inline());
    assert!(!Bytes::new(vec![0_u8; INLINE_BYTES + 1]).is_inline());
    assert_eq!(std::mem::size_of::<Bytes>(), 40);
    assert_eq!(std::mem::size_of::<Scalar>(), 48);
    assert_eq!(Bytes::new_static(&[1, 2, 3]), short);
    assert_eq!(Bytes::default(), &[][..]);
    assert_eq!(short, [1_u8, 2, 3]);
    assert_eq!(short, vec![1_u8, 2, 3]);
    assert_eq!(&*short, &[1_u8, 2, 3][..]);
    assert_eq!(short.to_string(), "010203");
    assert_eq!(Vec::from(short.clone()), vec![1_u8, 2, 3]);
    assert_eq!([1_u8, 2, 3].into_iter().collect::<Bytes>(), short);
    assert_eq!(Scalar::from(vec![1_u8, 2, 3]), Scalar::Bytes(short.clone()));
    assert_eq!(
        Scalar::from(&[1_u8, 2, 3][..]),
        Scalar::Bytes(short.clone())
    );

    // A value is one value whichever layout holds it; the layout and the
    // width ride beside the payload, and a maximum never does.
    let large = short
        .clone()
        .try_with_parameters(BytesType::LargeBinary)
        .unwrap();
    assert_eq!(large, short);
    assert_eq!(large.layout(), BytesType::LargeBinary);
    assert_eq!(large.dtype().unwrap(), DataType::large_binary());
    assert_ne!(format!("{large:?}"), format!("{short:?}"));
    let bounded = short
        .clone()
        .try_with_parameters(BytesType::SizedBinary(4))
        .unwrap();
    assert_eq!(bounded.parameters(), BytesType::default());
    assert!(
        short
            .clone()
            .try_with_parameters(BytesType::SizedBinary(2))
            .is_err()
    );
    // Bytes are never padded: a fixed value is exactly its width.
    let fixed = BytesType::FixedBinary(3);
    let held = short.clone().try_with_parameters(fixed).unwrap();
    assert_eq!(held.fixed(), Some(3));
    assert_eq!(held.dtype().unwrap(), DataType::fixed_binary(3).unwrap());
    assert!(Bytes::new([1_u8, 2]).try_with_parameters(fixed).is_err());
    assert!(
        Bytes::new([1_u8, 2, 3, 4])
            .try_with_parameters(fixed)
            .is_err()
    );

    // The value door reads the same way, and reads text and a UUID as
    // their bytes.
    let value = DataType::from_str("binary(4)")
        .unwrap()
        .scalar(Scalar::from(vec![1_u8, 2, 3]))
        .unwrap();
    assert_eq!(value.dtype().unwrap(), DataType::binary());
    assert_eq!(value.id(), DataTypeId::Binary);
    assert_eq!(
        DataType::fixed_binary(3)
            .unwrap()
            .scalar(Scalar::from(vec![1_u8, 2, 3]))
            .unwrap()
            .id(),
        DataTypeId::FixedBinary
    );
    assert_eq!(
        DataType::binary().scalar("abc").unwrap().as_bytes(),
        Some(&b"abc"[..])
    );
    assert_eq!(
        DataType::binary()
            .scalar(
                DataType::uuid()
                    .scalar("00000000-0000-0000-0000-000000000001")
                    .unwrap()
            )
            .unwrap()
            .as_bytes()
            .map(<[u8]>::len),
        Some(16)
    );
}

#[test]
fn bytes_ride_their_own_arrow_layout_and_a_maximum_rides_the_document() {
    // The storage is the layout; only a maximum on a variable layout needs
    // the `yggdryl.bytes` document, because no Arrow type can state it.
    let cases: [(&str, ArrowDataType, Option<&str>); 6] = [
        ("binary", ArrowDataType::Binary, None),
        ("large_binary", ArrowDataType::LargeBinary, None),
        ("binary_view", ArrowDataType::BinaryView, None),
        ("fixed_binary(16)", ArrowDataType::FixedSizeBinary(16), None),
        (
            "binary(16)",
            ArrowDataType::Binary,
            Some(r#"{"layout":"sized_binary","max":16}"#),
        ),
        (
            "sized_binary(64)",
            ArrowDataType::Binary,
            Some(r#"{"layout":"sized_binary","max":64}"#),
        ),
    ];
    for (spelling, storage, document) in cases {
        let dtype = DataType::from_str(spelling).unwrap();
        assert_eq!(
            dtype.clone().into_arrow_datatype().unwrap(),
            storage,
            "{spelling}"
        );
        // A bare storage is the layout it names, with no maximum.
        assert_eq!(
            DataType::from_arrow_datatype(&storage).unwrap(),
            DataType::Bytes(dtype.bytes_parameters().unwrap().storage()),
            "{spelling}"
        );

        let field = dtype.clone().nullable_field("payload");
        let arrow = field.clone().into_arrow_field().unwrap();
        assert_eq!(arrow.data_type(), &storage, "{spelling}");
        assert_eq!(
            arrow
                .metadata()
                .get(EXTENSION_TYPE_NAME_KEY)
                .map(String::as_str),
            document.map(|_| BYTES_EXTENSION_NAME),
            "{spelling}"
        );
        assert_eq!(
            arrow
                .metadata()
                .get(EXTENSION_TYPE_METADATA_KEY)
                .map(String::as_str),
            document,
            "{spelling}"
        );
        assert_eq!(
            Field::from_arrow_field(&arrow).unwrap(),
            field,
            "{spelling}"
        );
    }

    // The document round-trips through its own door, and a document over a
    // storage it does not describe imports as the storage.
    let bounded = DataType::from_str("binary(16)")
        .unwrap()
        .bytes_parameters()
        .unwrap();
    assert_eq!(
        BytesType::from_extension_json(&bounded.extension_json()).unwrap(),
        bounded
    );
    let foreign = arrow_schema::Field::new("payload", ArrowDataType::LargeBinary, true)
        .with_metadata(
            [
                (
                    EXTENSION_TYPE_NAME_KEY.to_owned(),
                    BYTES_EXTENSION_NAME.to_owned(),
                ),
                (
                    EXTENSION_TYPE_METADATA_KEY.to_owned(),
                    bounded.extension_json(),
                ),
            ]
            .into_iter()
            .collect(),
        );
    assert_eq!(
        Field::from_arrow_field(&foreign).unwrap().dtype(),
        &DataType::large_binary()
    );
}

#[test]
fn a_byte_datatype_and_value_survive_the_serde_doors_under_one_tag() {
    // One `binary` tag for every byte datatype, with the layout when not
    // `binary` and the bound under the reading its layout gives it.
    for (spelling, json) in [
        ("binary", r#"{"type":"binary"}"#),
        (
            "binary(16)",
            r#"{"type":"binary","layout":"sized_binary","max":16}"#,
        ),
        (
            "large_binary",
            r#"{"type":"binary","layout":"large_binary"}"#,
        ),
        (
            "sized_binary(64)",
            r#"{"type":"binary","layout":"sized_binary","max":64}"#,
        ),
        (
            "fixed_binary(16)",
            r#"{"type":"binary","layout":"fixed_binary","fixed":16}"#,
        ),
    ] {
        let dtype = DataType::from_str(spelling).unwrap();
        assert_eq!(dtype.clone().into_json().unwrap(), json, "{spelling}");
        assert_eq!(DataType::from_json(json).unwrap(), dtype, "{spelling}");
        assert_eq!(
            serde_json::from_str::<DataType>(&serde_json::to_string(&dtype).unwrap()).unwrap(),
            dtype,
            "{spelling}"
        );
        let value = dtype.clone().into_value();
        assert_eq!(DataType::from_value(value).unwrap(), dtype, "{spelling}");
    }
    for retired in [
        r#"{"type":"fixed_size_binary","width":16}"#,
        r#"{"type":"large_binary"}"#,
        r#"{"type":"binary_view"}"#,
        r#"{"type":"binary","layout":"fixed_binary"}"#,
        r#"{"type":"binary","layout":"fixed_binary","max":16}"#,
        r#"{"type":"binary","fixed":16}"#,
        r#"{"type":"binary","max":0}"#,
    ] {
        assert!(DataType::from_json(retired).is_err(), "{retired}");
    }

    // One `bytes` tag for every byte value: the plain one writes its payload
    // and nothing else, and a layout or a width makes the value an object.
    assert_eq!(
        serde_json::to_string(&Scalar::from(vec![1_u8, 2, 3])).unwrap(),
        r#"{"type":"bytes","value":[1,2,3]}"#
    );
    for (spelling, json) in [
        ("binary(16)", r#"{"type":"bytes","value":[1,2,3]}"#),
        (
            "large_binary",
            r#"{"type":"bytes","value":{"layout":"large_binary","bytes":[1,2,3]}}"#,
        ),
        (
            "fixed_binary(3)",
            r#"{"type":"bytes","value":{"layout":"fixed_binary","fixed":3,"bytes":[1,2,3]}}"#,
        ),
    ] {
        let value = DataType::from_str(spelling)
            .unwrap()
            .scalar(Scalar::from(vec![1_u8, 2, 3]))
            .unwrap();
        assert_eq!(serde_json::to_string(&value).unwrap(), json, "{spelling}");
        let read: Scalar = serde_json::from_str(json).unwrap();
        assert_eq!(read, value, "{spelling}");
        assert_eq!(read.id(), value.id(), "{spelling}");
    }
    for retired in [
        r#"{"type":"fixed_size_binary","value":[1,2,3]}"#,
        r#"{"type":"large_binary","value":[1,2,3]}"#,
        r#"{"type":"binary_view","value":[1,2,3]}"#,
        r#"{"type":"bytes","value":{"layout":"fixed_binary","fixed":4,"bytes":[1,2,3]}}"#,
    ] {
        assert!(
            serde_json::from_str::<Scalar>(retired).is_err(),
            "{retired}"
        );
    }
}
