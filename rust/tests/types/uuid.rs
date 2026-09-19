//! One identifier datatype over sixteen fixed bytes, and the value it holds.

use yggdryl::FieldValue as _;
use yggdryl::Uuid;
use yggdryl::{DataType, Error, Scalar};

fn assert_round_trips(value: Uuid, version: u8, text: &str) {
    assert_eq!(value.to_string(), text);
    let bytes = value.into_bytes();
    assert_eq!(bytes[6] >> 4, version);
    assert_eq!(bytes[8] >> 6, 2);
    assert_eq!(Uuid::new(value.get()), value);
    assert_eq!(Uuid::from_bytes(&bytes).unwrap(), value);
    assert_eq!(Uuid::from_bytes(text.as_bytes()).unwrap(), value);
    let scalar = Scalar::Uuid(value);
    assert_eq!(DataType::uuid().scalar(scalar.clone()).unwrap(), scalar);
    assert_eq!(DataType::uuid().scalar(Scalar::from(text)).unwrap(), scalar);
    assert_eq!(
        serde_json::from_str::<Uuid>(&serde_json::to_string(&value).unwrap()).unwrap(),
        value
    );
}

#[test]
fn version_7_packs_exact_microsecond_fractions_and_payload_bits() {
    let cases: &[(i64, u64, &str)] = &[
        (0, 0, "00000000-0000-7000-8000-000000000000"),
        (1, 0, "00000000-0000-7004-8000-000000000000"),
        (244, 0, "00000000-0000-73e7-8000-000000000000"),
        (250, 0, "00000000-0000-7400-8000-000000000000"),
        (456, 0, "00000000-0000-774b-8000-000000000000"),
        (500, 0, "00000000-0000-7800-8000-000000000000"),
        (999, 0, "00000000-0000-7ffb-8000-000000000000"),
        (1_000, 0, "00000000-0001-7000-8000-000000000000"),
        (1_001, 0, "00000000-0001-7004-8000-000000000000"),
        (
            1_645_557_742_000_456,
            0xfedc_ba98_7654_3210,
            "017f22e2-79b0-774b-bedc-ba9876543210",
        ),
        (
            281_474_976_710_655_999,
            0,
            "ffffffff-ffff-7ffb-8000-000000000000",
        ),
        (
            281_474_976_710_655_999,
            u64::MAX,
            "ffffffff-ffff-7ffb-bfff-ffffffffffff",
        ),
    ];
    for &(micros, payload, text) in cases {
        assert_round_trips(Uuid::from_v7(micros, payload).unwrap(), 7, text);
    }
}

#[test]
fn version_7_orders_every_microsecond_and_millisecond_rollover_before_payload() {
    for base in [0, 1_645_557_742_000_000, 281_474_976_710_653_000] {
        for offset in 0..2_000 {
            let micros = base + offset;
            let earlier = Uuid::from_v7(micros, u64::MAX).unwrap();
            let later = Uuid::from_v7(micros + 1, 0).unwrap();
            assert!(earlier < later, "{micros}: UUID value order");
            assert!(
                earlier.into_bytes() < later.into_bytes(),
                "{micros}: storage order"
            );
        }
    }
}

#[test]
fn version_7_discards_exactly_the_two_high_payload_bits() {
    let instant = 1_645_557_742_000_456;
    let baseline = Uuid::from_v7(instant, 0).unwrap();
    for bit in 0..64 {
        let value = Uuid::from_v7(instant, 1_u64 << bit).unwrap();
        let difference = value.get() ^ baseline.get();
        assert_eq!(difference, if bit < 62 { 1_u128 << bit } else { 0 });
    }
    let payload = 0x0123_4567_89ab_cdef;
    for high in 0..4 {
        assert_eq!(
            Uuid::from_v7(instant, payload | (high << 62)).unwrap(),
            Uuid::from_v7(instant, payload).unwrap()
        );
    }
}

#[test]
fn version_7_refuses_negative_and_overflow_instants_at_the_value_root() {
    for micros in [i64::MIN, -1, 281_474_976_710_656_000, i64::MAX] {
        let error = Uuid::from_v7(micros, 0).unwrap_err();
        let Error::InvalidRecord { path, reason } = error else {
            panic!("expected a located UUID value refusal, got {error}");
        };
        assert_eq!(path, "$");
        assert_eq!(
            reason.as_str(),
            format!(
                "expected a UUIDv7 Unix microsecond instant in 0..=281474976710655999, got {micros}"
            )
        );
    }
}

#[test]
fn version_8_keeps_the_rfc_illustrative_vector_and_extremes() {
    // RFC 9562 Appendix B.2: the leading 128 bits of its example SHA-256.
    const RFC_VALUE: Uuid = Uuid::from_v8(0x5c14_6b14_3c52_4afd_938a_375d_0df1_fbf6);
    assert_round_trips(RFC_VALUE, 8, "5c146b14-3c52-8afd-938a-375d0df1fbf6");
    for (payload, text) in [
        (0, "00000000-0000-8000-8000-000000000000"),
        (u128::MAX, "ffffffff-ffff-8fff-bfff-ffffffffffff"),
        (
            0x0123_4567_89ab_cdef_0123_4567_89ab_cdef,
            "01234567-89ab-8def-8123-456789abcdef",
        ),
    ] {
        assert_round_trips(Uuid::from_v8(payload), 8, text);
    }
}

#[test]
fn version_8_replaces_only_the_six_version_and_variant_bits() {
    let baseline = Uuid::from_v8(0);
    for bit in 0..128 {
        let value = Uuid::from_v8(1_u128 << bit);
        assert_eq!(
            value.get() ^ baseline.get(),
            if matches!(bit, 62 | 63 | 76..=79) {
                0
            } else {
                1_u128 << bit
            },
            "bit {bit}"
        );
        assert_eq!(Uuid::from_v8(value.get()), value);
    }
}

/// Every string and byte datatype a UUID column reaches, and what it reads
/// back as.
///
/// A UUID is sixteen bytes of identity and a 36-character spelling of them,
/// and both readings have to survive every layout, charset and bound the two
/// families offer - not only the plain `utf8` one arm used to answer.
#[test]
fn a_uuid_column_reads_into_every_string_and_byte_datatype() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, RecordBatch, StringArray};
    use yggdryl::{ArrowCastOptions, Field};

    const TEXT: &str = "01912d68-783e-7c9a-b1f2-0123456789ab";
    let raw: [u8; 16] = [
        0x01, 0x91, 0x2d, 0x68, 0x78, 0x3e, 0x7c, 0x9a, 0xb1, 0xf2, 0x01, 0x23, 0x45, 0x67, 0x89,
        0xab,
    ];

    let strict = || ArrowCastOptions::new().with_safe(false);
    let row = |field: Field| Field::new("row", DataType::from_fields([field]).unwrap(), false);
    let id = Field::new("id", DataType::Uuid, false);
    let stored = id
        .cast_arrow_array(
            Arc::new(StringArray::from(vec![TEXT])) as ArrayRef,
            strict(),
        )
        .unwrap();
    // The column carries `arrow.uuid`, which is what a reading reads it under.
    let batch = RecordBatch::try_new(
        row(id.clone()).into_arrow_schema().unwrap(),
        vec![Arc::clone(&stored)],
    )
    .unwrap();
    let into = |target: DataType| -> ArrayRef {
        Arc::clone(
            row(Field::new("id", target, false))
                .cast_arrow_batch(batch.clone(), strict())
                .unwrap()
                .column(0),
        )
    };

    // Text: the canonical spelling, under every layout, charset and bound the
    // string family declares. A bound or a charset used to fall through to a
    // reading of the raw bytes, which is not text at all.
    for spelling in [
        "utf8",
        "large_utf8",
        "utf8_view",
        "ascii",
        "utf8(36)",
        "large_ascii",
        "fixed_ascii(36)",
        "string(windows-1252)",
    ] {
        let target = DataType::from_str(spelling).unwrap();
        let read = into(target.clone());
        let back = Field::new("id", DataType::Uuid, false)
            .cast_arrow_array(read, strict())
            .unwrap_or_else(|error| panic!("{spelling} does not read back: {error}"));
        assert_eq!(back.as_ref(), stored.as_ref(), "{spelling}");
    }

    // A bound the spelling outgrows is refused rather than truncated, and the
    // refusal names the field and the row.
    let refused = row(Field::new(
        "id",
        DataType::from_str("utf8(8)").unwrap(),
        false,
    ))
    .cast_arrow_batch(batch.clone(), strict())
    .unwrap_err()
    .to_string();
    assert!(refused.contains("row 0"), "{refused}");

    // Bytes: the sixteen stored bytes, under every variable framing and the
    // fixed width that is exactly them.
    for spelling in [
        "binary",
        "large_binary",
        "binary_view",
        "fixed_binary(16)",
        "binary(16)",
    ] {
        let read = into(DataType::from_str(spelling).unwrap());
        let back = Field::new("id", DataType::Uuid, false)
            .cast_arrow_array(read, strict())
            .unwrap_or_else(|error| panic!("{spelling} does not read back: {error}"));
        assert_eq!(back.as_ref(), stored.as_ref(), "{spelling}");
    }

    // A byte width that is not sixteen holds a different payload, and each
    // refusal names both sides rather than leaving Arrow's builder to
    // complain about a slice length.
    for (spelling, expected) in [
        ("fixed_binary(8)", "a fixed binary of 16 bytes"),
        ("binary(8)", "at most 8 bytes"),
    ] {
        let refused = row(Field::new(
            "id",
            DataType::from_str(spelling).unwrap(),
            false,
        ))
        .cast_arrow_batch(batch.clone(), strict())
        .unwrap_err()
        .to_string();
        assert!(refused.contains(expected), "{spelling}: {refused}");
    }

    // And the same two readings hold one value at a time.
    let value = Scalar::Uuid(Uuid::from_bytes(&raw).unwrap());
    assert_eq!(
        DataType::utf8().scalar(value.clone()).unwrap().as_str(),
        Some(TEXT)
    );
    assert_eq!(
        DataType::binary().scalar(value.clone()).unwrap().as_bytes(),
        Some(raw.as_slice())
    );
    assert_eq!(
        DataType::uuid().scalar(Scalar::from(raw.to_vec())).unwrap(),
        value
    );
    assert_eq!(DataType::uuid().scalar(Scalar::from(TEXT)).unwrap(), value);
}

/// What the identifier is, how it is stored, and what is refused - the value's
/// own contract, beside the layouts above that generate one.
mod value {
    use arrow_array::{Array, FixedSizeBinaryArray};
    use arrow_schema::DataType as ArrowDataType;

    use yggdryl::{DataType, DataTypeId, DataTypeKind};
    use yggdryl::{Field, Scalar};

    const TEXT: &str = "01912d68-783e-7c9a-b1f2-0123456789ab";
    const PACKED: u128 = 0x0191_2d68_783e_7c9a_b1f2_0123_4567_89ab;

    #[test]
    fn the_identity_is_the_sixteen_bytes_and_the_spelling_is_a_rendering() {
        let uuid = DataType::uuid();
        assert_eq!(uuid, DataType::Uuid);
        assert_eq!(uuid.id(), DataTypeId::Uuid);
        assert_eq!(uuid.kind(), DataTypeKind::Uuid);
        assert_eq!(uuid.name(), "uuid");
        assert_eq!(uuid.to_string(), "uuid");
        assert_eq!(DataTypeId::Uuid.fixed_byte_width(), Some(16));
        assert!(!uuid.is_nested());
        uuid.validate().unwrap();

        // The one spelling parses and displays unchanged.
        assert_eq!("uuid".parse::<DataType>().unwrap(), uuid);
        assert_eq!(uuid.to_string().parse::<DataType>().unwrap(), uuid);

        // The packed integer is the identifier, not a code for it.
        assert_eq!(uuid.uuid_packed(TEXT.as_bytes()).unwrap(), PACKED);
        assert_eq!(
            uuid.uuid_packed(TEXT.to_uppercase().as_bytes()).unwrap(),
            PACKED
        );
        assert_eq!(
            uuid.uuid_packed(TEXT.replace('-', "").as_bytes()).unwrap(),
            PACKED
        );
        assert_eq!(uuid.uuid_packed(&PACKED.to_be_bytes()).unwrap(), PACKED);
        assert_eq!(uuid.uuid_value(PACKED).unwrap(), TEXT);

        // Every accepted rendering canonicalizes to the exact packed UUID leaf.
        let field = uuid.clone().required_field("id");
        let row = DataType::from_fields([field.clone()])
            .unwrap()
            .required_field("row");
        let canonical = |value: Scalar| {
            row.canonicalize_value(Scalar::from_sequence([value]))
                .unwrap()
        };
        let exact = Scalar::Uuid(yggdryl::Uuid::new(PACKED));
        let expected = Scalar::from_sequence([exact]);
        assert_eq!(canonical(Scalar::from(TEXT)), expected);
        assert_eq!(canonical(Scalar::from(TEXT.to_uppercase())), expected);
        assert_eq!(
            canonical(Scalar::from(PACKED.to_be_bytes().to_vec())),
            expected
        );
        assert_eq!(
            uuid.default_value().unwrap(),
            Scalar::Uuid(yggdryl::Uuid::new(0))
        );
        assert!(
            uuid.is_default_value(&Scalar::from([0_u8; 16].to_vec()))
                .unwrap()
        );
    }

    #[test]
    fn storage_is_the_canonical_arrow_uuid_extension_over_sixteen_bytes() {
        let field = Field::new("id", DataType::Uuid, false);
        let arrow = field.clone().into_arrow_field().unwrap();

        assert_eq!(arrow.data_type(), &ArrowDataType::FixedSizeBinary(16));
        assert_eq!(arrow.metadata()["ARROW:extension:name"], "arrow.uuid");
        assert_eq!(arrow.metadata()["ARROW:extension:metadata"], "");
        assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);

        // The stored bytes are the identifier; the value reads back exact.
        let array = yggdryl::arrow::scalar_array(&field, &Scalar::from(TEXT)).unwrap();
        let stored = array
            .as_any()
            .downcast_ref::<FixedSizeBinaryArray>()
            .unwrap();
        assert_eq!(stored.value(0), PACKED.to_be_bytes());
        assert_eq!(
            yggdryl::arrow::scalar_value(&field, array.as_ref()).unwrap(),
            Scalar::Uuid(yggdryl::Uuid::new(PACKED))
        );
    }

    #[test]
    fn what_is_not_an_identifier_is_refused_by_the_one_rule() {
        let uuid = DataType::uuid();
        for spelling in [
            "not-a-uuid",
            "",
            "01912d68-783e-7c9a-b1f2-0123456789a",
            "01912d68-783e-7c9a-b1f2-0123456789abc",
            "01912d68783e7c9ab1f20123456789ab0",
            "01912d68-783e-7c9a-b1f2+0123456789ab",
            "0191_d68-783e-7c9a-b1f2-0123456789ab",
        ] {
            assert!(uuid.uuid_packed(spelling.as_bytes()).is_err(), "{spelling}");
        }
        let refused = uuid.uuid_packed(b"not-a-uuid").unwrap_err().to_string();
        assert!(refused.contains("sixteen bytes"), "{refused}");
        assert!(refused.contains("36-character"), "{refused}");

        // The type answers only for itself.
        assert!(DataType::utf8().uuid_packed(TEXT.as_bytes()).is_err());
        assert!(DataType::utf8().uuid_value(PACKED).is_err());
        assert!(
            DataType::fixed_binary(16)
                .unwrap()
                .uuid_packed(TEXT.as_bytes())
                .is_err()
        );
    }
}

/// The datatype carries no parameter: a version is a fact about a value,
/// which the value answers, and never about a column.
mod parameters {
    use std::collections::HashMap;

    use arrow_schema::extension::{EXTENSION_TYPE_METADATA_KEY, EXTENSION_TYPE_NAME_KEY};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};

    use yggdryl::{DataType, DataTypeId, Field, Scalar, Uuid};

    #[test]
    fn every_version_stands_in_the_one_column_and_the_value_answers_which() {
        let v4 = Uuid::new(0x6ba7_b810_9dad_41d1_80b4_00c0_4fd4_30c8);
        let v7 = Uuid::from_v7(1_645_557_742_000_456, 0xfedc_ba98_7654_3210).unwrap();
        let v8 = Uuid::from_v8(0x5c14_6b14_3c52_4afd_938a_375d_0df1_fbf6);
        for (value, version) in [(v4, 4), (v7, 7), (v8, 8)] {
            assert_eq!(value.version(), version);
            assert_eq!(
                DataType::uuid().scalar(Scalar::Uuid(value)).unwrap(),
                Scalar::Uuid(value)
            );
            assert_eq!(
                DataType::uuid()
                    .scalar(Scalar::from(value.to_string()))
                    .unwrap(),
                Scalar::Uuid(value)
            );
        }
        // A field adds nullability to the datatype's rule and nothing else.
        let column = DataType::uuid().required_field("id");
        assert_eq!(column.scalar(Scalar::Uuid(v7)).unwrap(), Scalar::Uuid(v7));
        assert!(column.scalar(Scalar::Null).is_err());
        assert!(
            DataType::uuid()
                .nullable_field("id")
                .scalar(Scalar::Null)
                .is_ok()
        );
    }

    #[test]
    fn a_versioned_spelling_is_no_datatype_at_any_door() {
        for spelling in ["uuidv4", "uuidv7", "uuidv8", "UUIDV7", "uuid(7)"] {
            assert!(DataType::from_str(spelling).is_err(), "{spelling}");
            assert!(spelling.parse::<DataTypeId>().is_err(), "{spelling}");
            assert!(
                DataType::from_json(&format!("{{\"type\":{spelling:?}}}")).is_err(),
                "{spelling}"
            );
            let mapping = DataType::uuid()
                .into_value()
                .with_key("type", Scalar::from(spelling))
                .unwrap();
            assert!(DataType::from_value(mapping).is_err(), "{spelling}");
        }
        // The identifier numbers the retired leaves held stay unused.
        assert_eq!(DataTypeId::ALL.len(), 83);
        assert!(
            DataTypeId::ALL
                .iter()
                .all(|id| !(69..=71).contains(&id.as_u8()))
        );
    }

    #[test]
    fn only_the_canonical_extension_imports_as_an_identifier() {
        // Sixteen bytes under a name this crate does not write are the
        // storage they are, exactly as any foreign extension is.
        let mut metadata = HashMap::new();
        metadata.insert(
            EXTENSION_TYPE_NAME_KEY.to_string(),
            "yggdryl.uuid".to_string(),
        );
        metadata.insert(
            EXTENSION_TYPE_METADATA_KEY.to_string(),
            "{\"version\":7}".to_string(),
        );
        let foreign = ArrowField::new("id", ArrowDataType::FixedSizeBinary(16), false)
            .with_metadata(metadata);
        assert_eq!(
            Field::from_arrow_field(&foreign).unwrap().dtype(),
            &DataType::fixed_binary(16).unwrap()
        );

        // The canonical name with its empty document is the identifier.
        let arrow = Field::new("id", DataType::uuid(), false)
            .into_arrow_field()
            .unwrap();
        assert_eq!(arrow.metadata()[EXTENSION_TYPE_NAME_KEY], "arrow.uuid");
        assert_eq!(arrow.metadata()[EXTENSION_TYPE_METADATA_KEY], "");
        assert_eq!(
            Field::from_arrow_field(&arrow).unwrap().dtype(),
            &DataType::uuid()
        );
    }
}
