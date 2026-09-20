//! The Parquet Variant encoding, and the value that holds it.

use std::sync::Arc;

use yggdryl::{
    DataType, DataTypeId, DataTypeKind, DigestAlgorithm, FamilyValue, Field, Nested, Scalar,
    VARIANT_EXTENSION_NAME, VARIANT_VERSION, Value, Variant,
};

/// The value most of these tests exchange: one object, two leaves.
fn quote() -> Scalar {
    Scalar::from_struct([
        ("symbol", Scalar::from("AAPL")),
        ("size", Scalar::from(100_i64)),
    ])
    .expect("the value builds")
}

#[test]
fn the_metadata_header_states_the_version_and_a_sorted_dictionary() {
    let variant = Variant::encode(&quote()).unwrap();
    let header = variant.metadata()[0];
    assert_eq!(header & 0x0f, VARIANT_VERSION, "the specification version");
    assert_eq!((header >> 4) & 0x01, 1, "sorted_strings");
    assert_eq!(usize::from(header >> 6) + 1, 1, "one-byte offsets");
    // Two keys, sorted: `size` then `symbol`.
    assert_eq!(variant.metadata()[1], 2);
    assert!(
        variant.metadata().ends_with(b"sizesymbol"),
        "{:?}",
        variant.metadata()
    );
    assert_eq!(variant.scalar().unwrap(), quote());
}

#[test]
fn a_short_string_folds_its_length_and_a_long_one_states_it() {
    let short = Variant::encode(&Scalar::from("abc")).unwrap();
    assert_eq!(short.value()[0] & 0x03, 1, "the short-string basic type");
    assert_eq!(short.value()[0] >> 2, 3, "the length in the header");
    assert_eq!(short.value().len(), 4);

    let long = "x".repeat(64);
    let held = Variant::encode(&Scalar::from(long.as_str())).unwrap();
    assert_eq!(held.value()[0] & 0x03, 0, "a primitive");
    assert_eq!(held.value()[0] >> 2, 16, "the string primitive");
    assert_eq!(held.value().len(), 1 + 4 + 64);
    assert_eq!(held.scalar().unwrap(), Scalar::from(long.as_str()));
}

#[test]
fn a_variant_is_a_value_the_generic_vocabulary_knows() {
    let variant = Variant::encode(&quote()).unwrap();
    let value = variant.clone().into_scalar();

    assert!(matches!(value, Scalar::Variant(_)));
    assert_eq!(value.id(), DataTypeId::Variant);
    assert_eq!(value.family(), DataTypeKind::Nested);
    assert_eq!(value.kind(), "variant");
    assert_eq!(Value::dtype(&variant).unwrap(), DataType::Variant);
    assert_eq!(Variant::from_scalar(&value), Some(&variant));
    assert_eq!(Variant::from_scalar(&quote()), None);
    assert_eq!(Scalar::from(variant.clone()), value);

    // It is one leaf of the nested family, beside the three containers.
    let held = Nested::from_scalar(&value).expect("a nested value");
    assert!(matches!(held, Nested::Variant(_)));
    assert_eq!(FamilyValue::dtype(&held).unwrap(), DataType::Variant);
    assert_eq!(held.into_scalar(), value);

    // Equality is the bytes, and the display is the JSON the value spells.
    assert_eq!(value, variant.clone().into_scalar());
    assert_eq!(variant.to_string(), r#"{"size":100,"symbol":"AAPL"}"#);
}

#[test]
fn an_encoded_variant_keeps_its_foreign_bytes_when_already_typed() {
    // A valid foreign dictionary need not claim sorted strings. Its object
    // still states fields in name order: dictionary ids 1 (`a`), then 0 (`b`).
    let variant = Variant::new(
        vec![0x01, 2, 0, 1, 2, b'b', b'a'],
        vec![2, 2, 1, 0, 0, 1, 2, 0, 8],
    )
    .unwrap();
    let scalar = Scalar::Variant(variant.clone());

    let scalar_encoded = scalar.into_variant().unwrap();
    let typed_encoded = DataType::Variant.encode_variant(&scalar).unwrap();
    let value_encoded = Value::into_variant(&variant).unwrap();
    let value_decoded = <Variant as Value>::from_variant(&variant).unwrap();
    for held in [scalar_encoded, typed_encoded, value_encoded, value_decoded] {
        assert_eq!(held, variant);
        assert_eq!(held.metadata().as_ptr(), variant.metadata().as_ptr());
        assert_eq!(held.value().as_ptr(), variant.value().as_ptr());
    }
}

#[test]
fn casting_into_a_variant_encodes_and_casting_out_decodes() {
    // A value entering a variant column is encoded, canonically.
    let held = DataType::Variant.scalar(quote()).unwrap();
    assert_eq!(held, Scalar::Variant(Variant::encode(&quote()).unwrap()));
    // And a value already encoded is answered untouched.
    assert_eq!(DataType::Variant.scalar(held.clone()).unwrap(), held);

    // Leaving the type decodes first, so every cast a value answers a
    // variant answers too.
    let seven = Scalar::Variant(Variant::encode(&Scalar::from(7_i32)).unwrap());
    assert_eq!(
        DataType::Int64.scalar(seven.clone()).unwrap(),
        Scalar::from(7_i64)
    );
    assert_eq!(
        DataType::utf8().scalar(seven).unwrap(),
        Scalar::from("7"),
        "a variant casts as the value it holds"
    );

    // A null is a value a variant spells, so a required column holds it.
    let null = DataType::Variant.scalar(Scalar::Null).unwrap();
    let Scalar::Variant(held) = &null else {
        panic!("a variant value, got {null:?}");
    };
    assert_eq!(held.value(), [0], "the encoding's own null");
    assert_eq!(held.scalar().unwrap(), Scalar::Null);
}

#[test]
fn a_variant_column_crosses_arrow_as_the_two_binaries() {
    let field = Field::new("payload", DataType::variant(), true);
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.extension_type_name(), Some(VARIANT_EXTENSION_NAME));
    let arrow_schema::DataType::Struct(children) = arrow.data_type() else {
        panic!("a struct storage, got {}", arrow.data_type());
    };
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].name(), "metadata");
    assert_eq!(children[1].name(), "value");

    let root = Field::new(
        "row",
        DataType::from(yggdryl::StructType::from_fields([field]).unwrap()),
        false,
    );
    let rows = Scalar::from_sequence([
        Scalar::from_struct([("payload", quote())]).unwrap(),
        Scalar::from_struct([("payload", Scalar::from(7_i64))]).unwrap(),
    ]);
    let batch = yggdryl::arrow::batch_from_value(&root, &rows).unwrap();
    assert_eq!(batch.num_rows(), 2);

    let read = yggdryl::arrow::batch_to_value(&batch).unwrap();
    let read = read.as_sequence().unwrap();
    let held: Vec<Scalar> = read
        .iter()
        .map(|row| row.as_sequence().unwrap()[0].clone())
        .collect();
    for (value, expected) in held.iter().zip([quote(), Scalar::from(7_i64)]) {
        let Scalar::Variant(variant) = value else {
            panic!("a variant value, got {value:?}");
        };
        assert_eq!(variant.scalar().unwrap(), expected);
    }
}

#[test]
fn variant_fields_distinguish_absence_from_encoded_null() {
    let optional = DataType::Variant.nullable_field("payload");
    let required = DataType::Variant.required_field("payload");
    let present = DataType::Variant.scalar(Scalar::Null).unwrap();
    assert!(matches!(present, Scalar::Variant(_)));
    assert_eq!(optional.scalar(Scalar::Null).unwrap(), Scalar::Null);
    assert_eq!(optional.cast_scalar(&Scalar::Null).unwrap(), Scalar::Null);
    assert!(required.scalar(Scalar::Null).is_err());
    assert!(required.cast_scalar(&Scalar::Null).is_err());
    assert_eq!(required.scalar(present.clone()).unwrap(), present);
    assert_eq!(optional.default_value().unwrap(), Scalar::Null);
    assert_eq!(required.default_value().unwrap(), present);
    assert!(DataType::Variant.is_default_value(&present).unwrap());
    assert!(!DataType::Variant.is_default_value(&Scalar::Null).unwrap());
}

#[test]
fn variant_arrow_nulls_keep_their_outer_validity_when_rematerialized() {
    use arrow_array::Array;
    let field = DataType::Variant.nullable_field("payload");
    let values = Scalar::from_sequence([
        Scalar::Null,
        Scalar::Variant(Scalar::Null.into_variant().unwrap()),
        Scalar::Variant(Scalar::from(7_i64).into_variant().unwrap()),
    ]);
    let array = yggdryl::arrow::array_from_value(&field, &values).unwrap();
    assert!(array.is_null(0));
    assert!(array.is_valid(1));
    assert!(array.is_valid(2));
    let decoded = yggdryl::arrow::array_to_value(&field, array.as_ref()).unwrap();
    assert_eq!(decoded, values);
    let restored = yggdryl::arrow::array_from_value(&field, &decoded).unwrap();
    assert_eq!(restored.to_data(), array.to_data());

    let single = yggdryl::arrow::scalar_array(&field, &Scalar::Null).unwrap();
    assert!(single.is_null(0));
    let root =
        DataType::from(yggdryl::StructType::from_fields([field]).unwrap()).required_field("row");
    let row = yggdryl::FieldRecord::new(&root, Scalar::from_sequence([Scalar::Null])).unwrap();
    assert!(row.into_arrow_batch().unwrap().column(0).is_null(0));
}

#[test]
fn a_variant_digests_and_writes_as_the_value_it_holds() {
    let variant = Scalar::Variant(Variant::encode(&quote()).unwrap());
    assert_eq!(
        variant.digest(DigestAlgorithm::Xxh3),
        quote().digest(DigestAlgorithm::Xxh3),
        "one value digests alike however it crossed"
    );
    assert_eq!(
        yggdryl::json::into_json_scalar(&variant).unwrap(),
        yggdryl::json::into_json_scalar(&quote()).unwrap(),
    );
    assert_eq!(
        yggdryl::yaml::into_utf8(&variant).unwrap(),
        yggdryl::yaml::into_utf8(&quote()).unwrap(),
    );
}

#[test]
fn a_variant_serializes_as_its_two_buffers_and_reads_back() {
    let variant = Scalar::Variant(Variant::encode(&quote()).unwrap());
    let document = serde_json::to_string(&variant).unwrap();
    assert!(document.contains(r#""type":"variant""#), "{document}");
    let read: Scalar = serde_json::from_str(&document).unwrap();
    assert_eq!(read, variant, "the bytes survive, not a re-encoding");
}

#[test]
fn metadata_offsets_keys_and_sorted_claim_are_validated() {
    let malformed = [
        ("first offset", vec![0x01, 1, 1, 1, b'x'], "zero"),
        ("trailing key bytes", vec![0x01, 0, 0, b'x'], "key"),
        ("invalid key text", vec![0x01, 1, 0, 1, 0xff], "UTF-8"),
        (
            "key offset inside UTF-8",
            vec![0x01, 2, 0, 1, 2, 0xc3, 0xa9],
            "UTF-8",
        ),
        (
            "unsorted keys",
            vec![0x11, 2, 0, 1, 2, b'b', b'a'],
            "sorted",
        ),
        (
            "duplicate keys",
            vec![0x11, 2, 0, 1, 2, b'a', b'a'],
            "unique",
        ),
    ];

    for (case, metadata, expected) in malformed {
        let refused = Variant::new(metadata, vec![0]).unwrap_err().to_string();
        assert!(refused.contains(expected), "{case}: {refused}");
    }

    // Reserved metadata bits do not change the version-one dictionary.
    Variant::new(vec![0x31, 0, 0], vec![0]).unwrap();

    let refused = Variant::new(vec![0xd1, 0xff, 0xff, 0xff, 0xff], vec![0])
        .unwrap_err()
        .to_string();
    assert!(refused.contains("offset"), "{refused}");
}

#[test]
fn nested_values_fill_their_declared_physical_segments() {
    let metadata = Arc::<[u8]>::from(vec![0x11, 0, 0]);
    // The outer array grants five bytes to an inner array whose own final
    // offset says that its child occupies no bytes.
    let refused = Variant::new(metadata, vec![3, 1, 0, 5, 3, 1, 0, 0, 0])
        .unwrap()
        .scalar()
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("segment") || refused.contains("offset"),
        "{refused}"
    );
}

#[test]
fn child_offsets_may_store_values_outside_logical_order() {
    let metadata = || Arc::<[u8]>::from(vec![0x11, 2, 0, 1, 2, b'a', b'b']);

    let array = Variant::new(metadata(), vec![3, 2, 1, 0, 2, 8, 0]).unwrap();
    assert_eq!(
        array.scalar().unwrap(),
        Scalar::from_sequence([Scalar::Null, Scalar::from(false)])
    );

    let object = Variant::new(metadata(), vec![2, 2, 0, 1, 1, 0, 2, 8, 0]).unwrap();
    assert_eq!(
        object.scalar().unwrap(),
        Scalar::from_struct([("a", Scalar::Null), ("b", Scalar::from(false)),]).unwrap()
    );
}

#[test]
fn object_fields_are_encoded_in_unsigned_utf8_name_order() {
    let metadata = Arc::<[u8]>::from(vec![0x11, 2, 0, 1, 2, b'a', b'b']);
    // The field IDs spell b, a while their physical values each spell null.
    let refused = Variant::new(metadata, vec![2, 2, 1, 0, 0, 1, 2, 0, 0])
        .unwrap()
        .scalar()
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("order") || refused.contains("sorted"),
        "{refused}"
    );
}

#[test]
fn decimal_physical_types_obey_their_precision_limits() {
    for (scalar, physical) in [
        (Scalar::Decimal32(yggdryl::Decimal32::new(i32::MAX, 0)), 9),
        (Scalar::Decimal64(yggdryl::Decimal64::new(i64::MAX, 0)), 10),
    ] {
        let encoded = Variant::encode(&scalar).unwrap();
        assert_eq!(encoded.value()[0] >> 2, physical, "{scalar:?}");
        assert_eq!(encoded.scalar().unwrap(), scalar);
    }

    let metadata = || Arc::<[u8]>::from(vec![0x11, 0, 0]);
    let malformed = [
        (8, i32::MAX.to_le_bytes().to_vec(), "decimal4"),
        (9, i64::MAX.to_le_bytes().to_vec(), "decimal8"),
        (10, i128::MAX.to_le_bytes().to_vec(), "decimal16"),
    ];
    for (physical, coefficient, expected) in malformed {
        let mut value = vec![physical << 2, 0];
        value.extend(coefficient);
        let refused = Variant::new(metadata(), value)
            .unwrap()
            .scalar()
            .unwrap_err()
            .to_string();
        assert!(refused.contains(expected), "{refused}");
    }

    // A wider physical type may carry a coefficient of narrower precision.
    let mut value = vec![10 << 2, 0];
    value.extend(1_i128.to_le_bytes());
    assert_eq!(
        Variant::new(metadata(), value).unwrap().scalar().unwrap(),
        Scalar::Decimal128(yggdryl::Decimal128::new(1, 0))
    );
}

#[test]
fn the_refusals_name_the_byte() {
    // Another version.
    let refused = Variant::new(vec![0x02_u8], vec![0_u8])
        .unwrap_err()
        .to_string();
    assert!(refused.contains("version 2"), "{refused}");

    // A dictionary the metadata ends before.
    let refused = Variant::new(vec![0x11_u8, 4], vec![0_u8])
        .unwrap_err()
        .to_string();
    assert!(refused.contains("offset"), "{refused}");

    // A payload cut short, and a primitive this version does not name.
    let empty = || Arc::<[u8]>::from(vec![0x11_u8, 0, 0]);
    let refused = Variant::new(empty(), vec![(5 << 2), 1])
        .unwrap()
        .scalar()
        .unwrap_err()
        .to_string();
    assert!(refused.contains("4 bytes announced"), "{refused}");
    let refused = Variant::new(empty(), vec![60 << 2])
        .unwrap()
        .scalar()
        .unwrap_err()
        .to_string();
    assert!(refused.contains("primitive type 60"), "{refused}");

    // Bytes left over after the value.
    let refused = Variant::new(empty(), vec![0, 0])
        .unwrap()
        .scalar()
        .unwrap_err()
        .to_string();
    assert!(refused.contains("1 bytes left"), "{refused}");

    // And what the standard cannot spell is refused by name.
    let zoned = Scalar::time64(
        1,
        yggdryl::TimeUnit::Microsecond,
        yggdryl::Timezone::from_str("UTC").unwrap(),
    );
    if let Ok(zoned) = zoned {
        let refused = Variant::encode(&zoned).unwrap_err().to_string();
        assert!(refused.contains("zone"), "{refused}");
    }
    // A coefficient of thirty-nine digits is past what a variant decimal
    // holds, whatever width carried it here.
    let refused = Variant::encode(&Scalar::Decimal128(yggdryl::Decimal128::new(i128::MAX, 0)))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("38"), "{refused}");
}
