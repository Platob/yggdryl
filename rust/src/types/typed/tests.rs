//! What a field's value contract answers beside the field, and what it refuses.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::types::UncheckedTypedScalar;
use crate::types::temporal::{Interval, Temporal};
use crate::{DataType, Field, Scalar, TimeUnit, Timezone, TypedScalar};

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn a_pairing_holds_only_a_value_its_field_accepts() {
    let field = Field::new("size", DataType::Int64, false);
    let typed = TypedScalar::new(&field, 7_i64).unwrap();
    assert_eq!(typed.name(), "size");
    assert_eq!(typed.dtype(), &DataType::Int64);
    assert!(std::ptr::eq(typed.field(), &field));

    let rejected = TypedScalar::new(&field, "seven").expect_err("a string is not an int64");
    let message = rejected.to_string();
    assert!(
        message.contains("size") && message.contains("int64"),
        "the failure must name the field and the datatype, got {message}"
    );
}

#[test]
fn nullability_is_the_fields_rule() {
    for dtype in [
        DataType::Int64,
        DataType::Utf8,
        DataType::Binary,
        DataType::DateTime64 {
            unit: TimeUnit::Nanosecond,
            timezone: Timezone::NAIVE,
        },
    ] {
        let nullable = Field::new("value", dtype.clone(), true);
        let typed = TypedScalar::new(&nullable, Scalar::Null).unwrap();
        assert!(typed.is_null());
        assert_eq!(typed.value(), &Scalar::Null);
        assert_eq!(typed.dtype(), &dtype);

        let required = Field::new("value", dtype, false);
        let refused = TypedScalar::new(&required, Scalar::Null).unwrap_err();
        assert!(refused.to_string().contains("null"), "{refused}");
    }

    let field = Field::new("value", DataType::Utf8, false);
    assert!(!TypedScalar::new(&field, "").unwrap().is_null());
}

#[test]
fn the_value_is_what_the_field_stores() {
    // The pairing is the value contract: an integer narrows to the declared
    // width, a code is trimmed of its padding, a value that does not fit is
    // refused rather than widened.
    let narrow = Field::new("count", DataType::Int8, false);
    assert_eq!(
        TypedScalar::new(&narrow, 7_i64).unwrap().value(),
        &Scalar::from(7_i8)
    );
    assert!(TypedScalar::new(&narrow, 1_000_i64).is_err());

    let ccy = Field::new("ccy", DataType::Currency, false);
    let typed = TypedScalar::new(&ccy, "USD\0").unwrap();
    assert_eq!(typed.as_str(), Some("USD"));
    assert_eq!(typed.value().id(), crate::DataTypeId::Currency);
}

#[test]
fn text_is_read_under_the_field_through_the_one_text_door() {
    let size = Field::new("size", DataType::Int64, false);
    assert_eq!(
        TypedScalar::parse_str(&size, "42").unwrap().value(),
        &Scalar::from(42_i64)
    );
    assert!(TypedScalar::parse_str(&size, "forty-two").is_err());

    let payload = Field::new("payload", DataType::Binary, false);
    assert_eq!(
        TypedScalar::parse_str(&payload, "QUJD").unwrap().as_bytes(),
        Some(&b"ABC"[..])
    );

    let day = Field::new("day", DataType::Date32, true);
    let typed = TypedScalar::parse_str(&day, "2024-01-01").unwrap();
    assert_eq!(typed.value(), &Scalar::date32(19_723));
    assert_eq!(typed.into_str(), "2024-01-01");
}

#[test]
fn a_value_infers_the_shared_field_of_its_own_datatype() {
    let typed = TypedScalar::infer(Scalar::from(1.5_f64)).unwrap();
    assert_eq!(typed.dtype(), &DataType::Float64);
    assert_eq!(typed.name(), "value");
    assert!(typed.field().is_nullable());
    assert!(std::ptr::eq(
        typed.field(),
        DataType::Float64.shared_field().unwrap()
    ));

    let decimal = TypedScalar::infer(Scalar::d128(150, 2)).unwrap();
    assert_eq!(decimal.dtype().id(), crate::DataTypeId::Decimal128);
    assert_eq!(decimal.as_decimal().map(|(_, scale)| scale), Some(2));

    let nothing = TypedScalar::infer(Scalar::Null).unwrap();
    assert_eq!(nothing.dtype(), &DataType::Null);
    assert!(nothing.is_null());

    // A nested value names a datatype with no shared field.
    let column = Scalar::from_sequence([Scalar::from(1_i64)]);
    let refused = TypedScalar::infer(column).unwrap_err().to_string();
    assert!(refused.contains("list"), "{refused}");
    assert!(refused.contains("TypedScalar::new"), "{refused}");

    // A value that names no single datatype has no pairing to build.
    let mixed = Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]);
    assert!(TypedScalar::infer(mixed).is_err());
}

#[test]
fn a_nested_value_is_validated_against_the_field_it_claims() {
    let row = Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]);
    let schema = DataType::from_fields([
        Field::new("id", DataType::Int64, false),
        Field::new("symbol", DataType::Utf8, false),
    ])
    .unwrap()
    .required_field("row");
    let typed = TypedScalar::new(&schema, row.clone()).unwrap();
    assert_eq!(typed.as_sequence(), row.as_sequence());
    assert_eq!(typed.get(1), Some(&Scalar::from("AAPL")));

    let wrong = Scalar::from_sequence([Scalar::from("one"), Scalar::from("AAPL")]);
    let error = TypedScalar::new(&schema, wrong).expect_err("id is not text");
    assert!(
        error.to_string().contains("id"),
        "the failure must locate the child, got {error}"
    );
}

#[test]
fn the_accessors_are_the_values_own() {
    let integer = TypedScalar::infer(Scalar::from(-7_i32)).unwrap();
    assert_eq!(integer.as_i64(), Some(-7));
    assert_eq!(integer.as_i128(), Some(-7));
    assert_eq!(integer.as_u64(), None);
    assert_eq!(integer.as_integer(), Scalar::from(-7_i32).as_integer());
    assert_eq!(integer.as_bool(), None);
    assert_eq!(integer.as_f64(), None);

    let float = TypedScalar::infer(Scalar::from(2.5_f32)).unwrap();
    assert_eq!(float.as_f64(), Some(2.5));
    assert_eq!(float.as_float(), Scalar::from(2.5_f32).as_float());

    let flag = TypedScalar::infer(Scalar::from(true)).unwrap();
    assert_eq!(flag.as_bool(), Some(true));

    let instant = Scalar::datetime64(0, TimeUnit::Microsecond, Timezone::UTC).unwrap();
    let typed = TypedScalar::infer(instant.clone()).unwrap();
    assert_eq!(typed.as_temporal(), instant.as_temporal());

    let record = Scalar::from_record([("id", Scalar::from(1_i64))]).unwrap();
    let field = record.inferred_scalar_field().unwrap();
    let typed = TypedScalar::new(&field, record).unwrap();
    assert!(
        typed.as_sequence().is_some(),
        "a record canonicalizes to a row"
    );
    assert_eq!(typed.as_record(), None);
    assert_eq!(typed.get(0), Some(&Scalar::from(1_i64)));
    assert_eq!(typed.as_ref(), typed.value());

    let mapping = Scalar::from_mapping([(Scalar::from("id"), Scalar::from(1_i64))]).unwrap();
    let field = mapping.inferred_scalar_field().unwrap();
    let mapped = TypedScalar::new(&field, mapping).unwrap();
    assert_eq!(mapped.get_key_str("id"), Some(&Scalar::from(1_i64)));
    assert_eq!(mapped.get_key_str("absent"), None);
}

#[test]
fn both_halves_come_back_out() {
    let field = Field::new("symbol", DataType::Utf8, false);
    let (borrowed, value) = TypedScalar::new(&field, "AAPL").unwrap().into_parts();
    assert!(std::ptr::eq(borrowed, &field));
    assert_eq!(value, Scalar::from("AAPL"));
    assert_eq!(
        Scalar::from(TypedScalar::new(&field, "AAPL").unwrap()),
        Scalar::from("AAPL")
    );
    assert_eq!(
        TypedScalar::new(&field, "AAPL").unwrap().into_value(),
        Scalar::from("AAPL")
    );
}

#[test]
fn the_text_is_the_canonical_spelling_the_display_writes() {
    let cases = [
        (TypedScalar::infer(Scalar::from(7_i64)).unwrap(), "7"),
        (TypedScalar::infer(Scalar::from("AAPL")).unwrap(), "AAPL"),
        (TypedScalar::infer(Scalar::from(true)).unwrap(), "true"),
        (TypedScalar::infer(Scalar::d128(150, 2)).unwrap(), "1.50"),
        (
            TypedScalar::infer(Scalar::date32(19_723)).unwrap(),
            "2024-01-01",
        ),
        (
            TypedScalar::infer(Scalar::from(&b"ABC"[..])).unwrap(),
            "ABC",
        ),
        (TypedScalar::infer(Scalar::Null).unwrap(), "null"),
    ];
    for (typed, expected) in cases {
        assert_eq!(typed.to_string(), expected);
        assert_eq!(typed.into_str(), expected);
    }

    // A value with no text of its own writes its family's own form: a row as
    // its sequence, an interval as its components, a payload the text
    // spelling refused - bytes that are not UTF-8 - as its hex.
    let row = Scalar::from_sequence([Scalar::from(1_i64)]);
    let field = row.inferred_scalar_field().unwrap();
    let typed = TypedScalar::new(&field, row.clone()).unwrap();
    assert_eq!(
        typed.to_string(),
        format!("{:?}", row.as_sequence().unwrap())
    );
    let interval = Scalar::Temporal(Temporal::Interval(
        Interval::new(1, 2, 3, TimeUnit::MonthDayNano).unwrap(),
    ));
    let typed = TypedScalar::infer(interval).unwrap();
    assert_eq!(typed.to_string(), "1mo:2d:3ns@month_day_nano");
    assert_eq!(typed.into_str(), "1mo:2d:3ns@month_day_nano");
    let typed = TypedScalar::infer(Scalar::from(&[0xff_u8, 0x00][..])).unwrap();
    assert_eq!(typed.to_string(), "ff00");
    assert_eq!(typed.into_str(), "ff00");
}

#[test]
fn pairings_compare_by_datatype_and_value_and_never_by_the_field_around_them() {
    let price = Field::new("price", DataType::Int32, false);
    let size = Field::new("size", DataType::Int32, true);
    let first = TypedScalar::new(&price, 7).unwrap();
    let same_value_other_field = TypedScalar::new(&size, 7).unwrap();
    let later_value = TypedScalar::new(&price, 8).unwrap();
    let wide = Field::new("price", DataType::Int64, false);
    let later_type = TypedScalar::new(&wide, 7).unwrap();

    assert_eq!(first, same_value_other_field);
    assert_eq!(hash_of(&first), hash_of(&same_value_other_field));
    assert_eq!(first.stable_hash(), Scalar::from(7).stable_hash());
    assert_eq!(first, first.clone());
    assert!(first < later_value);
    assert!(first < later_type);
    assert_ne!(first, later_type);
}

#[test]
fn a_pairing_serializes_as_its_field_and_value() {
    let field = Field::new("size", DataType::Int64, false);
    let typed = TypedScalar::new(&field, 7_i64).unwrap();
    let encoded: serde_json::Value = serde_json::to_value(&typed).unwrap();
    assert_eq!(encoded["field"], serde_json::to_value(&field).unwrap());
    assert_eq!(
        encoded["value"],
        serde_json::to_value(Scalar::from(7_i64)).unwrap()
    );
}

#[test]
fn an_unchecked_pairing_reads_through_the_field_without_committing() {
    let size = Field::new("size", DataType::Int64, false);
    let unchecked = UncheckedTypedScalar::from_str(&size, "42");
    assert_eq!(unchecked.name(), "size");
    assert_eq!(unchecked.dtype(), &DataType::Int64);
    assert_eq!(unchecked.value(), &Scalar::from("42"));
    assert!(!unchecked.is_null());
    // The held text is borrowed as it is...
    assert_eq!(unchecked.as_str(), Some("42"));
    assert_eq!(unchecked.as_bytes(), None);
    // ...and cast on read as a number.
    assert_eq!(unchecked.as_i64(), Some(42));
    assert_eq!(unchecked.as_u64(), Some(42));
    assert_eq!(unchecked.as_i128(), Some(42));
    assert_eq!(unchecked.as_integer(), Scalar::from(42_i64).as_integer());
    assert_eq!(unchecked.as_f64(), None);
    assert_eq!(unchecked.as_bool(), None);
    let checked = unchecked.clone().checked().unwrap();
    assert_eq!(checked.value(), &Scalar::from(42_i64));
    assert_eq!(unchecked.into_value(), Scalar::from("42"));

    let wrong = UncheckedTypedScalar::from_str(&size, "forty-two");
    assert_eq!(wrong.as_i64(), None);
    assert!(wrong.checked().is_err());

    // A reading follows the field's datatype whatever the held shape.
    let price = Field::new("price", DataType::decimal128(10, 2).unwrap(), false);
    let unchecked = UncheckedTypedScalar::new(&price, "1.5");
    assert_eq!(
        unchecked.as_decimal(),
        Some((crate::I256::from_i128(150), 2))
    );
    let ratio = Field::new("ratio", DataType::Float64, false);
    assert_eq!(UncheckedTypedScalar::new(&ratio, "2.5").as_f64(), Some(2.5));
    assert_eq!(
        UncheckedTypedScalar::from_str(&ratio, "2").as_float(),
        Scalar::from(2.0_f64).as_float()
    );
    // An integer is not a float value, so the reading is no reading at all.
    assert_eq!(UncheckedTypedScalar::new(&ratio, 2_i64).as_f64(), None);
    // A boolean is borrowed as held, like text and bytes: a spelling of one
    // is not read until the pairing is proven.
    let flag = Field::new("flag", DataType::Boolean, false);
    assert_eq!(UncheckedTypedScalar::new(&flag, true).as_bool(), Some(true));
    let spelled = UncheckedTypedScalar::from_str(&flag, "true");
    assert_eq!(spelled.as_bool(), None);
    assert_eq!(spelled.checked().unwrap().as_bool(), Some(true));
    assert_eq!(
        UncheckedTypedScalar::new(&flag, false).as_bool(),
        Some(false)
    );
    let day = Field::new("day", DataType::Date32, false);
    assert_eq!(
        UncheckedTypedScalar::from_str(&day, "2024-01-01").as_temporal(),
        Scalar::date32(19_723).as_temporal().copied()
    );

    // A null is held and read as the absence it is.
    let held = UncheckedTypedScalar::new(&size, Scalar::Null);
    assert!(held.is_null());
    assert_eq!(held.as_i64(), None);
    assert!(held.checked().is_err(), "the field is required");
    assert!(std::ptr::eq(
        UncheckedTypedScalar::new(&size, 1).field(),
        &size
    ));
}

#[cfg(feature = "arrow")]
mod arrow {
    use super::{DataType, Field, Scalar, TypedScalar};

    #[test]
    fn a_pairing_round_trips_through_its_one_row_arrow_array() {
        let field = Field::new("size", DataType::Int64, false);
        let typed = TypedScalar::new(&field, 7_i64).unwrap();
        let array = typed.clone().into_arrow_array().unwrap();
        assert_eq!(array.len(), 1);
        assert_eq!(
            TypedScalar::from_arrow_array(&field, array.as_ref()).unwrap(),
            typed
        );
    }

    #[test]
    fn a_decode_refuses_a_foreign_array_that_is_not_one_exact_row() {
        let field = Field::new("size", DataType::Int64, false);
        let array = TypedScalar::new(&field, 7_i64)
            .unwrap()
            .into_arrow_array()
            .unwrap();
        // Zero rows are not a scalar, and neither is another datatype.
        let error = TypedScalar::from_arrow_array(&field, array.slice(0, 0).as_ref())
            .expect_err("zero rows are not a scalar")
            .to_string();
        assert!(error.contains("exactly one value"), "{error}");
        let narrow = Field::new("size", DataType::Int32, false);
        let error = TypedScalar::from_arrow_array(&narrow, array.as_ref())
            .expect_err("an int64 array is not an int32 scalar")
            .to_string();
        assert!(error.contains("differs from expected"), "{error}");
    }

    #[test]
    fn a_null_projects_under_a_nullable_field_and_nowhere_else() {
        let nullable = Field::new("size", DataType::Int64, true);
        let absent = TypedScalar::new(&nullable, Scalar::Null).unwrap();
        let array = absent.clone().into_arrow_array().unwrap();
        assert!(arrow_array::Array::is_null(array.as_ref(), 0));
        assert_eq!(
            TypedScalar::from_arrow_array(&nullable, array.as_ref()).unwrap(),
            absent
        );
        // A required field never holds the null, so nothing projects - not
        // even under the Null datatype, whose only value it is: the pairing
        // is the field's own contract, with no canonical-default exception.
        let required = Field::new("size", DataType::Int64, false);
        assert!(TypedScalar::new(&required, Scalar::Null).is_err());
        assert!(
            TypedScalar::new(&Field::new("nothing", DataType::Null, false), Scalar::Null).is_err()
        );
        let nothing = DataType::Null.shared_field().unwrap();
        let typed = TypedScalar::new(nothing, Scalar::Null).unwrap();
        assert_eq!(typed.into_arrow_array().unwrap().len(), 1);
    }

    #[test]
    fn a_struct_pairing_decodes_and_reprojects_its_canonical_row_spelling() {
        let structure = DataType::from_fields([
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ])
        .unwrap()
        .required_field("row");
        let row = Scalar::from_sequence([Scalar::from(7_i64), Scalar::from("XNAS")]);
        let typed = TypedScalar::new(&structure, row.clone()).unwrap();
        let array = typed.into_arrow_array().unwrap();
        // Arrow and the validator use the same schema-ordered row sequence.
        let decoded = TypedScalar::from_arrow_array(&structure, array.as_ref()).unwrap();
        assert_eq!(decoded.value(), &row);
        assert_eq!(decoded.into_arrow_array().unwrap().as_ref(), array.as_ref());
    }

    #[test]
    fn an_arrow_reading_is_canonicalized_before_the_pairing_holds_it() {
        // A float16 column reads back physically; the pairing holds what
        // the field stores, which is what a second projection expects.
        let field = Field::new("ratio", DataType::Float16, true);
        let typed = TypedScalar::new(&field, 1.5_f64).unwrap();
        let array = typed.clone().into_arrow_array().unwrap();
        let decoded = TypedScalar::from_arrow_array(&field, array.as_ref()).unwrap();
        assert_eq!(decoded, typed);
        assert_eq!(decoded.value().id(), crate::DataTypeId::Float16);
    }
}
