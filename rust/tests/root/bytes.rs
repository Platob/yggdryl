//! `rust/src/bytes.rs`.

use super::typed;

mod values {
    use yggdryl::{Bytes, BytesType, DataType, StructType};
    use yggdryl::{Field, Scalar, Scheme};

    /// Bytes bounded to `max` on the `binary` layout.
    fn bounded_binary(max: u32) -> DataType {
        DataType::bytes(BytesType::SizedBinary(max)).unwrap()
    }

    #[test]
    fn serde_and_the_structural_value_round_trip() {
        // One `binary` tag for every byte datatype: the layout and the bound are
        // written only where they are not the default.
        for (dtype, json) in [
            (DataType::binary(), r#"{"type":"binary"}"#),
            (
                DataType::large_binary(),
                r#"{"type":"binary","layout":"large_binary"}"#,
            ),
            (
                DataType::binary_view(),
                r#"{"type":"binary","layout":"binary_view"}"#,
            ),
            (
                bounded_binary(16),
                r#"{"type":"binary","layout":"sized_binary","max":16}"#,
            ),
            (
                DataType::fixed_binary(4).unwrap(),
                r#"{"type":"binary","layout":"fixed_binary","fixed":4}"#,
            ),
        ] {
            assert_eq!(dtype.clone().into_json().unwrap(), json);
            assert_eq!(DataType::from_json(json).unwrap(), dtype);
            let value = dtype.clone().into_value();
            assert_eq!(
                value.get_key_str("type").and_then(Scalar::as_str),
                Some("binary")
            );
            assert_eq!(DataType::from_value(value).unwrap(), dtype);
            let field = Field::new("payload", dtype.clone(), true);
            assert_eq!(
                Field::from_json(&field.clone().into_json().unwrap()).unwrap(),
                field
            );
        }

        // The retired tags name nothing.
        for tag in ["fixed_size_binary", "large_binary", "binary_view"] {
            assert!(
                DataType::from_json(&format!(r#"{{"type":"{tag}","width":4}}"#)).is_err(),
                "{tag}"
            );
        }
        // A count of nothing, and a count under the wrong key, are refused.
        assert!(
            DataType::from_json(r#"{"type":"binary","layout":"fixed_binary","fixed":0}"#).is_err()
        );
        assert!(DataType::from_json(r#"{"type":"binary","max":0}"#).is_err());
        assert!(DataType::from_json(r#"{"type":"binary","fixed":4}"#).is_err());
        assert!(
            DataType::from_json(r#"{"type":"binary","layout":"fixed_binary","max":4}"#).is_err()
        );
        let refused = DataType::from_value(
            DataType::binary()
                .into_value()
                .with_key("fixed", Scalar::from(4_i64))
                .unwrap(),
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("$.fixed"), "{refused}");
    }

    #[test]
    fn only_plain_binary_crosses_a_foreign_target_unchanged() {
        let schema = StructType::from_fields([
            DataType::binary().required_field("plain"),
            bounded_binary(16).required_field("bounded"),
            DataType::large_binary().required_field("large"),
            DataType::binary_view().required_field("view"),
            DataType::fixed_binary(4).unwrap().required_field("fixed"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        for scheme in [Scheme::SPARK, Scheme::POLARS, Scheme::PANDAS] {
            let compat = schema.clone().into_scheme_compat(&scheme).unwrap();
            for name in ["plain", "bounded", "large", "view", "fixed"] {
                assert_eq!(compat[name].dtype(), &DataType::binary(), "{name}");
            }
        }
        // Iceberg names `fixed[n]`, so a fixed width stays.
        let compat = schema.clone().into_scheme_compat(&Scheme::ICEBERG).unwrap();
        for name in ["plain", "bounded", "large", "view"] {
            assert_eq!(compat[name].dtype(), &DataType::binary(), "{name}");
        }
        assert_eq!(compat["fixed"].dtype(), &DataType::fixed_binary(4).unwrap());
        assert_eq!(
            schema.clone().into_scheme_compat(&Scheme::ARROW).unwrap(),
            schema
        );
    }

    #[test]
    fn the_default_is_the_empty_payload_or_the_zero_filled_slot() {
        for dtype in [
            DataType::binary(),
            bounded_binary(16),
            DataType::large_binary(),
            DataType::binary_view(),
        ] {
            let value = dtype.default_value().unwrap();
            assert_eq!(value.as_bytes(), Some(&[][..]), "{dtype}");
            // A value carries the leaf of its column, a maximum included.
            assert_eq!(value.dtype().unwrap(), dtype);
            assert_eq!(value.bytes_parameters(), dtype.bytes_parameters());
            assert!(dtype.is_default_value(&value).unwrap());
        }
        let fixed = DataType::fixed_binary(4).unwrap();
        let value = fixed.default_value().unwrap();
        assert_eq!(value, Scalar::Binary(Bytes::new([0_u8; 4])));
        assert_eq!(value, Scalar::FixedBinary(Bytes::new([0_u8; 4]), 4));
        assert_eq!(value.dtype().unwrap(), fixed);
        assert!(fixed.is_default_value(&value).unwrap());
        assert!(
            !fixed
                .is_default_value(&Scalar::Binary(Bytes::new([0_u8; 3])))
                .unwrap()
        );
    }

    #[test]
    fn a_bound_and_a_layout_are_what_a_diff_reports() {
        assert_eq!(
            DataType::binary().show_diff(&bounded_binary(16), true, false),
            "≠ $.bound: None → Some(16)"
        );
        assert_eq!(
            DataType::fixed_binary(4).unwrap().show_diff(
                &DataType::fixed_binary(8).unwrap(),
                true,
                false
            ),
            "≠ $.bound: Some(4) → Some(8)"
        );
        assert_eq!(
            DataType::binary().show_diff(&DataType::large_binary(), true, false),
            "≠ $.kind: binary → large_binary"
        );
    }
}

mod leaves {
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
        assert_eq!(DataType::binary(), DataType::from(BytesType::default()));
        assert_eq!(DataType::binary(), DataType::Binary);
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

        // The storage of a sized column is the plain binary it fills: no
        // Arrow type states the maximum.
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
            DataType::from(BytesType::FixedBinary(0))
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
    fn a_byte_value_is_the_compact_byte_string_and_carries_its_leaf() {
        // A short payload lives inside the value with no heap behind it, a
        // static one costs nothing, and a longer one is one shared handle.
        let short = Bytes::new([1_u8, 2, 3]);
        assert!(short.is_inline());
        assert!(Bytes::new(vec![0_u8; INLINE_BYTES]).is_inline());
        assert!(!Bytes::new(vec![0_u8; INLINE_BYTES + 1]).is_inline());
        assert_eq!(std::mem::size_of::<Bytes>(), 32);
        assert_eq!(std::mem::size_of::<Scalar>(), 48);
        assert_eq!(Bytes::new_static(&[1, 2, 3]), short);
        assert_eq!(Bytes::default(), &[][..]);
        assert_eq!(short, [1_u8, 2, 3]);
        assert_eq!(short, vec![1_u8, 2, 3]);
        assert_eq!(&*short, &[1_u8, 2, 3][..]);
        assert_eq!(short.to_string(), "010203");
        assert_eq!(Vec::from(short.clone()), vec![1_u8, 2, 3]);
        assert_eq!([1_u8, 2, 3].into_iter().collect::<Bytes>(), short);
        assert_eq!(
            Scalar::from(vec![1_u8, 2, 3]),
            Scalar::Binary(short.clone())
        );
        assert_eq!(
            Scalar::from(&[1_u8, 2, 3][..]),
            Scalar::Binary(short.clone())
        );

        // A value is one value whichever leaf holds it: the leaf is the
        // variant, its width or maximum included, and equality reads the
        // payload alone.
        let plain = Scalar::from(short.clone());
        let large = BytesType::LargeBinary.scalar(short.clone()).unwrap();
        assert_eq!(large, plain);
        assert_eq!(large.as_binary(), Some(&short));
        assert_eq!(large.bytes_parameters(), Some(BytesType::LargeBinary));
        assert_eq!(large.dtype().unwrap(), DataType::large_binary());
        // The payload alone prints no leaf; the value prints its variant.
        assert_eq!(format!("{short:?}"), "0x010203");
        assert_eq!(format!("{large:?}"), "LargeBinary(0x010203)");
        assert_ne!(format!("{large:?}"), format!("{plain:?}"));
        let bounded = BytesType::SizedBinary(4).scalar(short.clone()).unwrap();
        assert_eq!(bounded, plain);
        assert_eq!(bounded.bytes_parameters(), Some(BytesType::SizedBinary(4)));
        assert_eq!(bounded.dtype().unwrap(), DataType::sized_binary(4).unwrap());
        assert!(BytesType::SizedBinary(2).scalar(short.clone()).is_err());
        // Bytes are never padded: a fixed value is exactly its width.
        let fixed = BytesType::FixedBinary(3);
        let held = fixed.scalar(short.clone()).unwrap();
        assert_eq!(held.bytes_parameters().and_then(BytesType::fixed), Some(3));
        assert_eq!(held.dtype().unwrap(), DataType::fixed_binary(3).unwrap());
        assert!(fixed.scalar(Bytes::new([1_u8, 2])).is_err());
        assert!(fixed.scalar(Bytes::new([1_u8, 2, 3, 4])).is_err());

        // The value door reads the same way, and reads text and a UUID as
        // their bytes.
        let value = DataType::from_str("binary(4)")
            .unwrap()
            .scalar(Scalar::from(vec![1_u8, 2, 3]))
            .unwrap();
        assert_eq!(value.dtype().unwrap(), DataType::sized_binary(4).unwrap());
        assert_eq!(value.id(), DataTypeId::SizedBinary);
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
                DataType::from(dtype.bytes_parameters().unwrap().storage()),
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
        // and nothing else, and a layout, a width or a maximum makes the value
        // an object, the number under `fixed` either way.
        assert_eq!(
            serde_json::to_string(&Scalar::from(vec![1_u8, 2, 3])).unwrap(),
            r#"{"type":"bytes","value":[1,2,3]}"#
        );
        for (spelling, json) in [
            ("binary", r#"{"type":"bytes","value":[1,2,3]}"#),
            (
                "binary(16)",
                r#"{"type":"bytes","value":{"layout":"sized_binary","fixed":16,"bytes":[1,2,3]}}"#,
            ),
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
}

mod fields {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, BinaryArray, FixedSizeBinaryArray};
    use yggdryl::{ArrowCastOptions, DataType, DataTypeId, FieldScalar, Scalar, Serie};
    use yggdryl::{Bytes, BytesField, BytesType, bytes};

    use super::typed::assert_typed_marker;

    #[test]
    fn the_bytes_marker_covers_every_layout_and_bound() {
        assert_typed_marker::<bytes::BytesType>(DataType::binary());
        assert_typed_marker::<bytes::BytesType>(DataType::large_binary());
        assert_typed_marker::<bytes::BytesType>(DataType::binary_view());
        assert_typed_marker::<bytes::BytesType>(DataType::fixed_binary(16).unwrap());
        assert_typed_marker::<bytes::BytesType>(DataType::from_str("binary(16)").unwrap());
        assert_typed_marker::<bytes::BytesType>(DataType::from_str("sized_binary(64)").unwrap());

        // The family is parameterized, so the field takes its datatype through
        // `try_new`, and a datatype from another family is refused by name.
        let digest =
            BytesField::try_new("digest", DataType::fixed_binary(32).unwrap(), false).unwrap();
        assert_eq!(digest.dtype(), &DataType::fixed_binary(32).unwrap());
        assert!(BytesField::try_new("digest", DataType::utf8(), false).is_err());
        assert!(BytesField::try_new("digest", DataType::Uuid, false).is_err());
        assert!(BytesField::try_new("digest", DataType::geometry(None).unwrap(), false).is_err());

        // A bound of zero is refused wherever it is stated.
        assert!(DataType::fixed_binary(0).is_err());
        assert!(DataType::sized_binary(0).is_err());
        // A fixed layout built by hand with no width is what `validate` catches.
        assert!(DataType::bytes(BytesType::FixedBinary(0)).is_err());
    }

    #[test]
    fn a_fixed_value_is_exactly_its_width() {
        let field =
            BytesField::try_new("digest", DataType::fixed_binary(4).unwrap(), false).unwrap();
        let field_field = field.to_field();
        let held = FieldScalar::new(&field_field, vec![1_u8, 2, 3, 4]).unwrap();
        assert_eq!(held.as_bytes(), Some(&[1_u8, 2, 3, 4][..]));
        assert_eq!(held.value().id(), DataTypeId::FixedBinary);
        assert_eq!(
            held.value().dtype().unwrap(),
            DataType::fixed_binary(4).unwrap()
        );

        // Bytes are never padded: shorter and longer are both refused, naming
        // the width and what arrived.
        for (payload, held) in [(&[1_u8, 2, 3][..], "3 bytes"), (&[1_u8; 5][..], "5 bytes")] {
            let refused = FieldScalar::new(&field.to_field(), payload)
                .unwrap_err()
                .to_string();
            assert!(refused.contains("exactly 4 bytes"), "{refused}");
            assert!(refused.contains(held), "{refused}");
        }
    }

    #[test]
    fn a_bounded_value_is_at_most_the_maximum_and_carries_it() {
        let field =
            BytesField::try_new("payload", DataType::from_str("binary(4)").unwrap(), true).unwrap();
        let field_field = field.to_field();
        let held = FieldScalar::new(&field_field, vec![7_u8; 4]).unwrap();
        assert_eq!(held.as_bytes(), Some(&[7_u8; 4][..]));
        // The value carries the leaf of its column, the maximum included.
        assert_eq!(held.value().id(), DataTypeId::SizedBinary);
        assert_eq!(
            held.value().dtype().unwrap(),
            DataType::sized_binary(4).unwrap()
        );
        assert!(FieldScalar::new(&field.to_field(), Vec::<u8>::new()).is_ok());

        let refused = FieldScalar::new(&field.to_field(), vec![7_u8; 5])
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 4 bytes"), "{refused}");
        assert!(refused.contains("5 bytes"), "{refused}");

        // The same door on the wider layouts.
        let view = BytesField::try_new(
            "payload",
            DataType::from_str("sized_binary(2)").unwrap(),
            true,
        )
        .unwrap();
        assert!(FieldScalar::new(&view.to_field(), vec![1_u8, 2]).is_ok());
        assert!(FieldScalar::new(&view.to_field(), vec![1_u8, 2, 3]).is_err());
    }

    #[test]
    fn a_value_adopts_the_layout_of_the_column_that_holds_it() {
        let large = BytesField::try_new("payload", DataType::large_binary(), false).unwrap();
        let large_field = large.to_field();
        let held = FieldScalar::new(&large_field, Scalar::from(b"abc")).unwrap();
        let Some(bytes) = held.value().as_binary() else {
            panic!("a byte column holds a byte value");
        };
        assert_eq!(
            held.value().bytes_parameters(),
            Some(BytesType::LargeBinary)
        );
        assert_eq!(
            held.value().bytes_parameters().and_then(BytesType::fixed),
            None
        );
        // The layout is a retag over the same payload, so equality reads the
        // payload alone.
        assert_eq!(*bytes, Bytes::new(b"abc"));

        // Text and a UUID spell their bytes into a byte column; a small value
        // stays inline.
        let plain = BytesField::try_new("payload", DataType::binary(), false).unwrap();
        assert_eq!(
            FieldScalar::new(&plain.to_field(), "abc")
                .unwrap()
                .as_bytes(),
            Some(&b"abc"[..])
        );
        let inline = FieldScalar::new(&plain.to_field(), "abc")
            .unwrap()
            .into_value();
        let Some(inline) = inline.as_binary() else {
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
        let refused = Serie::from_arrow_array(
            Some(&field.to_field()),
            Arc::clone(&source),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("\"payload\""), "{refused}");
        assert!(refused.contains("row 1"), "{refused}");
        assert!(refused.contains("at most 3 bytes"), "{refused}");

        // Safe: the cell that does not fit becomes null.
        let cast =
            Serie::from_arrow_array(Some(&field.to_field()), source, ArrowCastOptions::new())
                .unwrap()
                .require_arrow_array()
                .unwrap();
        let cast = cast.as_any().downcast_ref::<BinaryArray>().unwrap();
        assert_eq!(cast.value(0), b"abc");
        assert!(cast.is_null(1));
        assert!(cast.is_null(2));
    }

    #[test]
    fn a_fixed_column_is_its_own_storage() {
        let field =
            BytesField::try_new("digest", DataType::fixed_binary(4).unwrap(), true).unwrap();
        let source: ArrayRef = Arc::new(
            FixedSizeBinaryArray::try_from_sparse_iter_with_size(
                [Some(&b"abcd"[..]), None].into_iter(),
                4,
            )
            .unwrap(),
        );
        let cast = Serie::from_arrow_array(
            Some(&field.to_field()),
            Arc::clone(&source),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .require_arrow_array()
        .unwrap();
        // A column holds its leaf as its own typed array, so the `Arc` around it
        // is new; the buffers under it are the caller's.
        assert!(cast.to_data().ptr_eq(&source.to_data()));

        // Variable bytes of another length are refused into the width; the
        // width is the storage, so Arrow's own kernel is what refuses them.
        let source: ArrayRef = Arc::new(BinaryArray::from(vec![
            Some(&b"abcd"[..]),
            Some(&b"abc"[..]),
        ]));
        assert!(
            Serie::from_arrow_array(
                Some(&field.to_field()),
                source,
                ArrowCastOptions::new().with_safe(false),
            )
            .is_err()
        );
    }
}
