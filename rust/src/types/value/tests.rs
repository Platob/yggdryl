use super::*;
use crate::types::Integer;

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    DataType::from_fields(fields).unwrap().required_field("row")
}

#[test]
fn a_record_maps_names_to_schema_order_and_fills_field_defaults() {
    let schema = root([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ]);
    let record = Scalar::from_record([("id", Scalar::from(7))]).unwrap();

    schema.validate_value(&record).unwrap();
    assert_eq!(
        schema.canonicalize_value(record).unwrap(),
        Scalar::from_sequence([Scalar::from(7), Scalar::Null])
    );
}

#[test]
fn a_record_refuses_unknown_names() {
    let schema = root([DataType::Int64.required_field("id")]);
    let record =
        Scalar::from_record([("id", Scalar::from(7)), ("unknown", Scalar::from(1))]).unwrap();

    let validation = schema.validate_value(&record).unwrap_err().to_string();
    let canonical = schema.canonicalize_value(record).unwrap_err().to_string();
    assert!(validation.contains("unknown field"), "{validation}");
    assert!(canonical.contains("unknown field"), "{canonical}");
}

#[test]
fn integer_canonicalization_preserves_every_declared_width() {
    let schema = root([
        DataType::Int8.required_field("i8"),
        DataType::Int16.required_field("i16"),
        DataType::Int32.required_field("i32"),
        DataType::Int64.required_field("i64"),
        DataType::UInt8.required_field("u8"),
        DataType::UInt16.required_field("u16"),
        DataType::UInt32.required_field("u32"),
        DataType::UInt64.required_field("u64"),
    ]);
    let natural = Scalar::from_sequence([
        Scalar::from(-1),
        Scalar::from(2),
        Scalar::from(-3),
        Scalar::from(-4),
        Scalar::from(1),
        Scalar::from(2),
        Scalar::from(3),
        Scalar::from(4),
    ]);
    let canonical = schema.canonicalize_value(natural).unwrap();
    let values = canonical.as_sequence().unwrap();
    assert!(matches!(values[0], Scalar::Integer(Integer::I8(_))));
    assert!(matches!(values[1], Scalar::Integer(Integer::I16(_))));
    assert!(matches!(values[2], Scalar::Integer(Integer::I32(_))));
    assert!(matches!(values[3], Scalar::Integer(Integer::I64(_))));
    assert!(matches!(values[4], Scalar::Integer(Integer::U8(_))));
    assert!(matches!(values[5], Scalar::Integer(Integer::U16(_))));
    assert!(matches!(values[6], Scalar::Integer(Integer::U32(_))));
    assert!(matches!(values[7], Scalar::Integer(Integer::U64(_))));
}

#[test]
fn year_month_interval_canonicalizes_to_the_exact_interval_leaf() {
    let schema = root([DataType::Interval(TimeUnit::YearMonth).required_field("months")]);

    let canonical = schema
        .canonicalize_value(Scalar::from_sequence([Scalar::from(18)]))
        .unwrap();
    let value = &canonical.as_sequence().unwrap()[0];
    assert!(matches!(
        value,
        Scalar::Temporal(Temporal::Interval(interval))
            if interval.months() == 18 && interval.unit() == TimeUnit::YearMonth
    ));
}

#[test]
fn temporal_casts_preserve_family_and_timezone() {
    let schema = root([
        DataType::DateTime64 {
            unit: TimeUnit::Millisecond,
            timezone: Timezone::UTC,
        }
        .required_field("at"),
        DataType::Time32(TimeUnit::Second).required_field("clock"),
        DataType::Duration32(TimeUnit::Millisecond).required_field("elapsed"),
    ]);
    let valid = Scalar::from_sequence([
        Scalar::datetime64(1, TimeUnit::Second, Timezone::UTC).unwrap(),
        Scalar::time32(2, TimeUnit::Second, Timezone::NAIVE).unwrap(),
        Scalar::duration64(3, TimeUnit::Second).unwrap(),
    ]);
    assert_eq!(
        schema.canonicalize_value(valid).unwrap(),
        Scalar::from_sequence([
            Scalar::datetime64(1_000, TimeUnit::Millisecond, Timezone::UTC).unwrap(),
            Scalar::time32(2, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::duration32(3_000, TimeUnit::Millisecond).unwrap(),
        ])
    );

    for invalid in [
        Scalar::from_sequence([
            Scalar::datetime64(1, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::time32(2, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::duration32(3, TimeUnit::Millisecond).unwrap(),
        ]),
        Scalar::from_sequence([
            Scalar::duration64(1, TimeUnit::Second).unwrap(),
            Scalar::time32(2, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            Scalar::duration32(3, TimeUnit::Millisecond).unwrap(),
        ]),
        Scalar::from_sequence([
            Scalar::datetime64(1, TimeUnit::Second, Timezone::UTC).unwrap(),
            Scalar::from("not a time"),
            Scalar::duration32(3, TimeUnit::Millisecond).unwrap(),
        ]),
    ] {
        assert!(schema.validate_value(&invalid).is_err());
        assert!(schema.canonicalize_value(invalid).is_err());
    }
}

/// The spellings a value takes on the way into a datatype, and the ones it
/// prints on the way out - the same readings a column takes and prints.
mod readings {
    use crate::types::{Mapping, Nested};
    use crate::{DataType, Scalar};

    fn dtype(expression: &str) -> DataType {
        expression.parse().unwrap()
    }

    #[test]
    fn text_reads_into_every_family_a_text_column_reads_into() {
        assert_eq!(DataType::Int64.scalar("42").unwrap(), Scalar::from(42_i64));
        assert_eq!(DataType::UInt8.scalar(" 7 ").unwrap(), Scalar::from(7_u8));
        assert_eq!(
            DataType::Float64.scalar("4.5").unwrap(),
            Scalar::from(4.5_f64)
        );
        assert_eq!(
            DataType::Boolean.scalar("TRUE").unwrap(),
            Scalar::from(true)
        );
        assert_eq!(
            dtype("decimal128(10, 2)").scalar("10.50").unwrap(),
            Scalar::d128(1_050, 2)
        );
        assert_eq!(
            DataType::Date32.scalar("1970-01-02").unwrap(),
            Scalar::date32(1)
        );

        // A magnitude the declared width cannot hold is refused by the width,
        // not wrapped by the reading.
        assert!(DataType::Int8.scalar("200").is_err());
        // A digit the declared scale cannot hold is refused rather than rounded.
        let refused = dtype("decimal128(10, 2)")
            .scalar("1.005")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("fractional digits"), "{refused}");
    }

    #[test]
    fn every_value_with_a_spelling_prints_it_into_a_text_column() {
        assert_eq!(DataType::Utf8.scalar(7_i64).unwrap(), Scalar::from("7"));
        assert_eq!(
            DataType::Utf8.scalar(Scalar::date32(0)).unwrap(),
            Scalar::from("1970-01-01")
        );
        assert_eq!(
            DataType::Utf8
                .scalar(DataType::Currency.scalar("USD").unwrap())
                .unwrap(),
            Scalar::from("USD")
        );
        assert_eq!(
            DataType::Utf8
                .scalar(Scalar::from(b"AAPL".to_vec()))
                .unwrap(),
            Scalar::from("AAPL")
        );

        // A payload that was read and refused names what was wrong with it,
        // rather than which kind arrived.
        let refused = DataType::Utf8
            .scalar(Scalar::from(vec![0xFF_u8]))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("not UTF-8"), "{refused}");
    }

    #[test]
    fn a_byte_column_stores_the_payload_a_value_spells() {
        assert_eq!(
            DataType::Binary.scalar("hi").unwrap(),
            Scalar::from(b"hi".to_vec())
        );
        // An ASCII value at a declared width spells that width, padded, which
        // is the payload the fixed column stores.
        let code = dtype("ascii(4)").scalar("US").unwrap();
        assert_eq!(
            DataType::FixedSizeBinary(4)
                .scalar(code)
                .unwrap()
                .as_bytes(),
            Some(b"US\0\0".as_slice())
        );
        let uuid = DataType::Uuid
            .scalar("00000000-0000-0000-0000-000000000001")
            .unwrap();
        assert_eq!(
            DataType::FixedSizeBinary(16)
                .scalar(uuid)
                .unwrap()
                .as_bytes(),
            Some(
                [0_u8; 15]
                    .iter()
                    .copied()
                    .chain([1])
                    .collect::<Vec<_>>()
                    .as_slice()
            )
        );

        // The declared width is part of the layout on every path.
        let refused = DataType::FixedSizeBinary(4)
            .scalar(Scalar::from(vec![1_u8, 2]))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("requires 4 bytes"), "{refused}");
    }

    #[test]
    fn a_record_is_the_entries_a_map_column_holds() {
        let map = dtype("map<utf8, int32>");
        let record = Scalar::from_record([("a", Scalar::from(1_i32))]).unwrap();

        assert_eq!(
            map.scalar(record).unwrap(),
            Scalar::from_mapping([(Scalar::from("a"), Scalar::from(1_i32))]).unwrap()
        );
    }

    #[test]
    fn a_map_carries_its_invariants_however_its_entries_were_built() {
        // `Mapping::new` takes already-unique entries on trust, so the value
        // contract is what refuses a map that is not a function.
        let duplicates = Scalar::Nested(Nested::Mapping(Mapping::new(vec![
            (Scalar::from("a"), Scalar::from(1_i32)),
            (Scalar::from("a"), Scalar::from(2_i32)),
        ])));
        let refused = dtype("map<utf8, int32>")
            .scalar(duplicates)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("collide"), "{refused}");

        // A declared ordering is checked whether or not anything was restated.
        let unsorted = Scalar::Nested(Nested::Mapping(Mapping::new(vec![
            (Scalar::from("b"), Scalar::from(1_i32)),
            (Scalar::from("a"), Scalar::from(2_i32)),
        ])));
        let sorted = DataType::map_of(DataType::Utf8, DataType::Int32, true).unwrap();
        let refused = sorted.scalar(unsorted.clone()).unwrap_err().to_string();
        assert!(refused.contains("not sorted"), "{refused}");
        DataType::map_of(DataType::Utf8, DataType::Int32, false)
            .unwrap()
            .scalar(unsorted)
            .unwrap();
    }

    #[test]
    fn a_union_type_id_has_one_canonical_representation() {
        let union = dtype("union<0: int32>");
        let canonical = Scalar::from_sequence([Scalar::from(0_i64), Scalar::from(1_i32)]);

        // The id does not depend on whether the payload also needed restating.
        assert_eq!(
            union
                .scalar(Scalar::from_sequence([
                    Scalar::from(0_i8),
                    Scalar::from(1_i32)
                ]))
                .unwrap(),
            canonical
        );
        assert_eq!(union.scalar(canonical.clone()).unwrap(), canonical);
    }
}

/// Absence is a value only where the layout stores it beside the values.
mod absence {
    use crate::{DataType, Field, Scalar};

    #[test]
    fn a_union_and_a_run_end_spell_absence_through_a_child() {
        // Both layouts carry absence inside a child rather than beside the
        // values, so a bare null is not a value either of them holds - the
        // same rule the nested walk has always applied to a child.
        let union: DataType = "union<0: int32>".parse().unwrap();
        let refused = union.scalar(Scalar::Null).unwrap_err().to_string();
        assert!(refused.contains("[type_id, payload]"), "{refused}");
        assert!(
            Field::new("u", union.clone(), true)
                .scalar(Scalar::Null)
                .is_err()
        );
        // The pair that does spell it is accepted.
        union
            .scalar(Scalar::from_sequence([Scalar::from(0_i64), Scalar::Null]))
            .unwrap();

        let required = DataType::run_end_encoded(
            DataType::Int32.required_field("run_ends"),
            DataType::Int32.required_field("values"),
        )
        .unwrap();
        let refused = required.scalar(Scalar::Null).unwrap_err().to_string();
        assert!(refused.contains("non-nullable"), "{refused}");
        // A run-end layout whose values child holds a null does hold one.
        DataType::run_end_encoded(
            DataType::Int32.required_field("run_ends"),
            DataType::Int32.nullable_field("values"),
        )
        .unwrap()
        .scalar(Scalar::Null)
        .unwrap();

        // Every other datatype still takes absence as a value of its own.
        assert_eq!(DataType::Int32.scalar(Scalar::Null).unwrap(), Scalar::Null);
    }
}
