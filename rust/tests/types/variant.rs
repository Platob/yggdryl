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
