//! One byte family: four layouts, one bound.

use arrow_schema::DataType as ArrowDataType;
use arrow_schema::extension::{EXTENSION_TYPE_METADATA_KEY, EXTENSION_TYPE_NAME_KEY};
use yggdryl::types::{BYTES_EXTENSION_NAME, Bytes, BytesLayout, BytesParameters, INLINE_BYTES};
use yggdryl::{DataType, DataTypeId, Field, Scalar};

/// Every layout, with its identifier and its canonical spelling.
const LAYOUTS: [(BytesLayout, DataTypeId, &str); 4] = [
    (BytesLayout::Binary, DataTypeId::Binary, "binary"),
    (
        BytesLayout::FixedSizeBinary,
        DataTypeId::FixedSizeBinary,
        "fixed_size_binary",
    ),
    (
        BytesLayout::LargeBinary,
        DataTypeId::LargeBinary,
        "large_binary",
    ),
    (
        BytesLayout::BinaryView,
        DataTypeId::BinaryView,
        "binary_view",
    ),
];

#[test]
fn every_layout_is_one_datatype_under_every_spelling() {
    for (layout, id, spelling) in LAYOUTS {
        assert_eq!(layout.as_str(), spelling);
        assert_eq!(layout.id(), id);
        assert_eq!(BytesLayout::from_id(id), Some(layout));
        assert_eq!(BytesLayout::from_str(spelling).unwrap(), layout);
        // The fold is the grammar's: case, underscores and hyphens all drop.
        assert_eq!(
            BytesLayout::from_str(&spelling.to_uppercase()).unwrap(),
            layout
        );
        assert_eq!(
            BytesLayout::from_str(&spelling.replace('_', "-")).unwrap(),
            layout
        );
        // Every byte identifier is parameterized: the layout alone is not a
        // datatype, and the width is a parameter rather than the id's.
        assert!(id.is_parameterized(), "{id}");
        assert!(id.is_binary(), "{id}");
        assert_eq!(id.fixed_byte_width(), None, "{id}");

        let mut parameters = BytesParameters::new(layout);
        if layout.is_fixed() {
            parameters = parameters.try_with_bound(16).unwrap();
        }
        let dtype = DataType::bytes(parameters).unwrap();
        assert!(dtype.to_string().starts_with(spelling), "{dtype}");
        assert_eq!(DataType::from_str(&dtype.to_string()).unwrap(), dtype);
        assert_eq!(dtype.id(), id);
        assert_eq!(dtype.bytes_parameters(), Some(parameters));
        assert!(dtype.id().is_binary());
        assert_eq!(dtype.string_parameters(), None);
    }

    // The sugar constructors are the same datatypes.
    assert_eq!(
        DataType::binary(),
        DataType::Bytes(BytesParameters::default())
    );
    assert_eq!(
        DataType::large_binary(),
        DataType::bytes(BytesLayout::LargeBinary).unwrap()
    );
    assert_eq!(
        DataType::binary_view(),
        DataType::bytes(BytesLayout::BinaryView).unwrap()
    );
    assert_eq!(
        DataType::fixed_size_binary(16).unwrap().to_string(),
        "fixed_size_binary(16)"
    );

    // The accepted aliases render as the canonical spellings.
    for (alias, canonical) in [
        ("bytes", "binary"),
        ("blob", "binary"),
        ("bytea", "binary"),
        ("varbinary", "binary"),
        ("varbinary(16)", "binary(16)"),
        ("fixed_binary(16)", "fixed_size_binary(16)"),
        ("FixedSizeBinary(16)", "fixed_size_binary(16)"),
        ("largebinary", "large_binary"),
        ("BinaryView", "binary_view"),
    ] {
        assert_eq!(
            DataType::from_str(alias).unwrap().to_string(),
            canonical,
            "{alias}"
        );
    }

    // A UUID and a geospatial value are bytes with an identity, not byte
    // columns.
    assert_eq!(DataType::Uuid.bytes_parameters(), None);
    assert_eq!(DataType::geometry(None).unwrap().bytes_parameters(), None);
}

#[test]
fn one_number_carries_the_maximum_or_the_width() {
    // `binary(16)` is a maximum of sixteen bytes and `fixed_size_binary(16)`
    // the exact width; the two readings never both answer.
    let bounded = DataType::from_str("binary(16)").unwrap();
    let parameters = bounded.bytes_parameters().unwrap();
    assert_eq!(parameters.layout(), BytesLayout::Binary);
    assert_eq!(parameters.max(), Some(16));
    assert_eq!(parameters.fixed(), None);
    assert_eq!(parameters.bound(), Some(16));
    assert!(parameters.is_bounded());
    assert!(!parameters.is_fixed());
    assert_eq!(bounded.fixed_byte_width(), None);
    assert_ne!(bounded, DataType::binary());

    let fixed = DataType::fixed_size_binary(16).unwrap();
    let parameters = fixed.bytes_parameters().unwrap();
    assert_eq!(parameters.layout(), BytesLayout::FixedSizeBinary);
    assert_eq!(parameters.fixed(), Some(16));
    assert_eq!(parameters.max(), None);
    assert!(parameters.is_fixed());
    assert_eq!(fixed.fixed_byte_width(), Some(16));
    assert_ne!(bounded, fixed);

    // The bound follows the layout it is restated into, and comes off with
    // the maximum but not the width.
    let restated = BytesParameters::new(BytesLayout::Binary)
        .try_with_bound(16)
        .unwrap()
        .with_layout(BytesLayout::FixedSizeBinary);
    assert_eq!(restated.fixed(), Some(16));
    assert_eq!(restated.without_max().fixed(), Some(16));
    assert_eq!(restated.without_bound().bound(), None);
    assert_eq!(
        BytesParameters::new(BytesLayout::LargeBinary)
            .try_with_bound(16)
            .unwrap()
            .without_max()
            .bound(),
        None
    );

    // A bound of no bytes is a column of one value; the fixed layout with
    // no width is a question, not a declaration.
    assert!(DataType::from_str("binary(0)").is_err());
    assert!(DataType::from_str("fixed_size_binary(0)").is_err());
    assert!(DataType::from_str("fixed_size_binary").is_err());
    assert!(DataType::fixed_size_binary(0).is_err());
    assert!(
        BytesParameters::new(BytesLayout::FixedSizeBinary)
            .validate()
            .is_err()
    );
    assert!(DataType::bytes(BytesLayout::FixedSizeBinary).is_err());
    let unwidened = DataType::Bytes(BytesParameters::new(BytesLayout::FixedSizeBinary));
    assert!(unwidened.validate().is_err());
    assert!(unwidened.into_arrow().is_err());
    // Every other spelling with a bound is a maximum.
    for spelling in ["large_binary(64)", "binary_view(64)"] {
        let parameters = DataType::from_str(spelling)
            .unwrap()
            .bytes_parameters()
            .unwrap();
        assert_eq!(parameters.max(), Some(64), "{spelling}");
        assert_eq!(parameters.fixed(), None, "{spelling}");
    }
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
        .try_with_parameters(BytesParameters::new(BytesLayout::LargeBinary))
        .unwrap();
    assert_eq!(large, short);
    assert_eq!(large.layout(), BytesLayout::LargeBinary);
    assert_eq!(large.dtype().unwrap(), DataType::large_binary());
    assert_ne!(format!("{large:?}"), format!("{short:?}"));
    let bounded = short
        .clone()
        .try_with_parameters(
            BytesParameters::new(BytesLayout::Binary)
                .try_with_bound(4)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(bounded.parameters(), BytesParameters::default());
    assert!(
        short
            .clone()
            .try_with_parameters(
                BytesParameters::new(BytesLayout::Binary)
                    .try_with_bound(2)
                    .unwrap()
            )
            .is_err()
    );
    // Bytes are never padded: a fixed value is exactly its width.
    let fixed = BytesParameters::new(BytesLayout::FixedSizeBinary)
        .try_with_bound(3)
        .unwrap();
    let held = short.clone().try_with_parameters(fixed).unwrap();
    assert_eq!(held.fixed(), Some(3));
    assert_eq!(
        held.dtype().unwrap(),
        DataType::fixed_size_binary(3).unwrap()
    );
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
        DataType::fixed_size_binary(3)
            .unwrap()
            .scalar(Scalar::from(vec![1_u8, 2, 3]))
            .unwrap()
            .id(),
        DataTypeId::FixedSizeBinary
    );
    assert_eq!(
        DataType::binary().scalar("abc").unwrap().as_bytes(),
        Some(&b"abc"[..])
    );
    assert_eq!(
        DataType::binary()
            .scalar(
                DataType::Uuid
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
        (
            "fixed_size_binary(16)",
            ArrowDataType::FixedSizeBinary(16),
            None,
        ),
        (
            "binary(16)",
            ArrowDataType::Binary,
            Some(r#"{"layout":"binary","max":16}"#),
        ),
        (
            "large_binary(64)",
            ArrowDataType::LargeBinary,
            Some(r#"{"layout":"large_binary","max":64}"#),
        ),
    ];
    for (spelling, storage, document) in cases {
        let dtype = DataType::from_str(spelling).unwrap();
        assert_eq!(dtype.clone().into_arrow().unwrap(), storage, "{spelling}");
        // A bare storage is the layout it names, with no maximum.
        assert_eq!(
            DataType::from_arrow(&storage).unwrap(),
            DataType::Bytes(dtype.bytes_parameters().unwrap().without_max()),
            "{spelling}"
        );

        let field = dtype.clone().nullable_field("payload");
        let arrow = field.clone().into_arrow().unwrap();
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
        assert_eq!(Field::from_arrow(&arrow).unwrap(), field, "{spelling}");
    }

    // The document round-trips through its own door, and a document over a
    // storage it does not describe imports as the storage.
    let bounded = DataType::from_str("binary(16)")
        .unwrap()
        .bytes_parameters()
        .unwrap();
    assert_eq!(
        BytesParameters::from_extension_json(&bounded.extension_json()).unwrap(),
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
        Field::from_arrow(&foreign).unwrap().dtype(),
        &DataType::large_binary()
    );
}

#[test]
fn a_byte_datatype_and_value_survive_the_serde_doors_under_one_tag() {
    // One `binary` tag for every byte datatype, with the layout when not
    // `binary` and the bound under the reading its layout gives it.
    for (spelling, json) in [
        ("binary", r#"{"type":"binary"}"#),
        ("binary(16)", r#"{"type":"binary","max":16}"#),
        (
            "large_binary",
            r#"{"type":"binary","layout":"large_binary"}"#,
        ),
        (
            "binary_view(64)",
            r#"{"type":"binary","layout":"binary_view","max":64}"#,
        ),
        (
            "fixed_size_binary(16)",
            r#"{"type":"binary","layout":"fixed_size_binary","fixed":16}"#,
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
        r#"{"type":"binary","layout":"fixed_size_binary"}"#,
        r#"{"type":"binary","layout":"fixed_size_binary","max":16}"#,
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
            "fixed_size_binary(3)",
            r#"{"type":"bytes","value":{"layout":"fixed_size_binary","fixed":3,"bytes":[1,2,3]}}"#,
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
        r#"{"type":"bytes","value":{"layout":"fixed_size_binary","fixed":4,"bytes":[1,2,3]}}"#,
    ] {
        assert!(
            serde_json::from_str::<Scalar>(retired).is_err(),
            "{retired}"
        );
    }
}
