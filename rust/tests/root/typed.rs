//! `rust/src/typed.rs`: a field is an enum over its leaves, and each leaf
//! carries its own datatype.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use yggdryl::{DataType, Field};
use yggdryl::{DataTypeValue, FieldValue, Int64Field, Int64Type, StringField, StringType};

fn stable_hash<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn a_field_takes_the_leaf_its_datatype_names() {
    // The variant is not a caller's claim: it follows the datatype, and the
    // leaf then holds that datatype in its own type rather than a `DataType`
    // that something else has to agree with.
    let field = Field::new("id", DataType::Int64, false);
    assert!(matches!(field, Field::Int64(_)));
    assert_eq!(field.dtype(), &DataType::Int64);

    let leaf = Int64Field::from_field(&field).expect("the field is the Int64 leaf");
    assert_eq!(leaf.typed_dtype_ref(), &Int64Type);
    assert_eq!(leaf.name(), "id");

    // A leaf of another family narrows to nothing, which is the whole of what
    // the old marker proved.
    assert!(StringField::from_field(&field).is_none());
}

#[test]
fn a_leaf_and_the_field_it_widens_to_are_the_same_field() {
    let leaf = Int64Field::new("id", Int64Type, true);
    let field = Field::from(leaf.clone());

    assert_eq!(field.name(), leaf.name());
    assert_eq!(field.dtype(), leaf.dtype());
    assert_eq!(field.is_nullable(), leaf.is_nullable());

    // Round-tripping through the root changes nothing, and the leaf reads the
    // same datatype in either spelling.
    let narrowed = Int64Field::from_field(&field).expect("the leaf is still the Int64 one");
    assert_eq!(narrowed.typed_dtype_ref(), leaf.typed_dtype_ref());
    assert_eq!(stable_hash(&field), stable_hash(&Field::from(leaf)));
}

#[test]
fn replacing_the_datatype_moves_the_field_to_the_matching_leaf() {
    let mut field = Field::new("value", DataType::Int64, false);
    assert!(matches!(field, Field::Int64(_)));

    field
        .set_dtype(DataType::utf8())
        .expect("utf8 is a valid datatype");
    assert!(matches!(field, Field::String(_)));
    assert_eq!(field.dtype(), &DataType::utf8());
    assert_eq!(field.name(), "value", "the name survives the move");

    // The leaf follows the datatype, so the two can never disagree.
    let leaf = StringField::from_field(&field).expect("the field is the String leaf now");
    assert_eq!(
        leaf.typed_dtype_ref(),
        &StringType::from_dtype(&DataType::utf8()).expect("utf8 is a string datatype")
    );
}

#[test]
fn a_datatype_payload_reads_back_the_datatype_it_came_from() {
    // Every payload is a DataTypeValue, and widening then narrowing is the
    // identity - that is what lets a leaf store the payload and nothing else.
    for dtype in [DataType::Int64, DataType::utf8(), DataType::Boolean] {
        let field = Field::new("column", dtype.clone(), false);
        assert_eq!(field.dtype(), &dtype);
        assert_eq!(field.id(), dtype.id());
    }
}

/// Asserts that a datatype's payload reads back the datatype it came from.
///
/// The check the old compile-time markers used to make, on the type that
/// replaced them: a payload narrows out of the datatype it belongs to, refuses
/// every other one, and widens back to exactly what it came from.
pub fn assert_typed_marker<D: DataTypeValue>(dtype: DataType) {
    let payload =
        D::from_dtype(&dtype).unwrap_or_else(|| panic!("{dtype} should narrow to {}", D::FAMILY));
    assert_eq!(
        payload.clone().into_dtype(),
        dtype,
        "{} should widen back to the datatype it came from",
        D::FAMILY
    );
    assert_eq!(payload.id(), dtype.id());

    // A field of that datatype carries the payload and nothing else.
    let field = Field::new("column", dtype.clone(), false);
    assert_eq!(field.dtype(), &dtype);
}

mod pairing {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    use yggdryl::UncheckedFieldScalar;
    use yggdryl::interval::Interval;
    use yggdryl::{DataType, Field, FieldScalar, Scalar, StructType, TimeUnit, Timezone};

    fn hash_of<T: Hash>(value: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn a_pairing_holds_only_a_value_its_field_accepts() {
        let field = Field::new("size", DataType::Int64, false);
        let typed = FieldScalar::new(&field, 7_i64).unwrap();
        assert_eq!(typed.name(), "size");
        assert_eq!(typed.dtype(), &DataType::Int64);
        assert!(std::ptr::eq(typed.field(), &field));

        let rejected = FieldScalar::new(&field, "seven").expect_err("a string is not an int64");
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
            DataType::utf8(),
            DataType::binary(),
            DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::NAIVE,
            },
        ] {
            let nullable = Field::new("value", dtype.clone(), true);
            let typed = FieldScalar::new(&nullable, Scalar::Null).unwrap();
            assert!(typed.is_null());
            assert_eq!(typed.value(), &Scalar::Null);
            assert_eq!(typed.dtype(), &dtype);

            let required = Field::new("value", dtype, false);
            let refused = FieldScalar::new(&required, Scalar::Null).unwrap_err();
            assert!(refused.to_string().contains("null"), "{refused}");
        }

        let field = Field::new("value", DataType::utf8(), false);
        assert!(!FieldScalar::new(&field, "").unwrap().is_null());
    }

    #[test]
    fn the_value_is_what_the_field_stores() {
        // The pairing is the value contract: an integer narrows to the declared
        // width, a code is trimmed of its padding, a value that does not fit is
        // refused rather than widened.
        let narrow = Field::new("count", DataType::Int8, false);
        assert_eq!(
            FieldScalar::new(&narrow, 7_i64).unwrap().value(),
            &Scalar::from(7_i8)
        );
        assert!(FieldScalar::new(&narrow, 1_000_i64).is_err());

        let ccy = Field::new("ccy", DataType::Currency, false);
        let typed = FieldScalar::new(&ccy, "USD\0").unwrap();
        assert_eq!(typed.as_str(), Some("USD"));
        assert_eq!(typed.value().id(), yggdryl::DataTypeId::Currency);
    }

    #[test]
    fn text_is_read_under_the_field_through_the_one_text_door() {
        let size = Field::new("size", DataType::Int64, false);
        assert_eq!(
            FieldScalar::parse_str(&size, "42").unwrap().value(),
            &Scalar::from(42_i64)
        );
        assert!(FieldScalar::parse_str(&size, "forty-two").is_err());

        let payload = Field::new("payload", DataType::binary(), false);
        assert_eq!(
            FieldScalar::parse_str(&payload, "QUJD").unwrap().as_bytes(),
            Some(&b"ABC"[..])
        );

        let day = Field::new("day", DataType::date32(), true);
        let typed = FieldScalar::parse_str(&day, "2024-01-01").unwrap();
        assert_eq!(typed.value(), &Scalar::date32(19_723));
        assert_eq!(typed.into_str(), "2024-01-01");
    }

    #[test]
    fn a_value_infers_the_shared_field_of_its_own_datatype() {
        let typed = FieldScalar::infer(Scalar::from(1.5_f64)).unwrap();
        assert_eq!(typed.dtype(), &DataType::Float64);
        assert_eq!(typed.name(), "value");
        assert!(typed.field().is_nullable());
        assert!(std::ptr::eq(
            typed.field(),
            DataType::Float64.shared_field().unwrap()
        ));

        let decimal = FieldScalar::infer(Scalar::d128(150, 2)).unwrap();
        assert_eq!(decimal.field().id(), yggdryl::DataTypeId::Decimal128);
        assert_eq!(decimal.as_decimal().map(|(_, scale)| scale), Some(2));

        let nothing = FieldScalar::infer(Scalar::Null).unwrap();
        assert_eq!(nothing.dtype(), &DataType::Null);
        assert!(nothing.is_null());

        // A nested value names a datatype with no shared field.
        let column = Scalar::from_sequence([Scalar::from(1_i64)]);
        let refused = FieldScalar::infer(column).unwrap_err().to_string();
        assert!(refused.contains("list"), "{refused}");
        assert!(refused.contains("FieldScalar::new"), "{refused}");

        // A value that names no single datatype has no pairing to build.
        let mixed = Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]);
        assert!(FieldScalar::infer(mixed).is_err());
    }

    #[test]
    fn a_nested_value_is_validated_against_the_field_it_claims() {
        let row = Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]);
        let schema = StructType::from_fields([
            Field::new("id", DataType::Int64, false),
            Field::new("symbol", DataType::utf8(), false),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        let typed = FieldScalar::new(&schema, row.clone()).unwrap();
        assert_eq!(typed.as_sequence(), row.as_sequence());
        assert_eq!(typed.get(1).as_deref(), Some(&Scalar::from("AAPL")));

        let wrong = Scalar::from_sequence([Scalar::from("one"), Scalar::from("AAPL")]);
        let error = FieldScalar::new(&schema, wrong).expect_err("id is not text");
        assert!(
            error.to_string().contains("id"),
            "the failure must locate the child, got {error}"
        );
    }

    #[test]
    fn the_accessors_are_the_values_own() {
        let integer = FieldScalar::infer(Scalar::from(-7_i32)).unwrap();
        assert_eq!(integer.as_i64(), Some(-7));
        assert_eq!(integer.as_i128(), Some(-7));
        assert_eq!(integer.as_u64(), None);
        assert_eq!(integer.as_bool(), None);
        assert_eq!(integer.as_f64(), None);

        let float = FieldScalar::infer(Scalar::from(2.5_f32)).unwrap();
        assert_eq!(float.as_f64(), Some(2.5));

        let flag = FieldScalar::infer(Scalar::from(true)).unwrap();
        assert_eq!(flag.as_bool(), Some(true));

        let record = Scalar::from_struct([("id", Scalar::from(1_i64))]).unwrap();
        let field = record.inferred_scalar_field().unwrap();
        let typed = FieldScalar::new(&field, record).unwrap();
        assert!(
            typed.as_sequence().is_some(),
            "a record canonicalizes to a row"
        );
        assert_eq!(typed.as_struct(), None);
        assert_eq!(typed.get(0).as_deref(), Some(&Scalar::from(1_i64)));
        assert_eq!(typed.as_ref(), typed.value());

        let mapping = Scalar::from_mapping([(Scalar::from("id"), Scalar::from(1_i64))]).unwrap();
        let field = mapping.inferred_scalar_field().unwrap();
        let mapped = FieldScalar::new(&field, mapping).unwrap();
        assert_eq!(mapped.get_key_str("id"), Some(&Scalar::from(1_i64)));
        assert_eq!(mapped.get_key_str("absent"), None);
    }

    #[test]
    fn both_halves_come_back_out() {
        let field = Field::new("symbol", DataType::utf8(), false);
        let (borrowed, value) = FieldScalar::new(&field, "AAPL").unwrap().into_parts();
        assert!(std::ptr::eq(borrowed, &field));
        assert_eq!(value, Scalar::from("AAPL"));
        assert_eq!(
            Scalar::from(FieldScalar::new(&field, "AAPL").unwrap()),
            Scalar::from("AAPL")
        );
        assert_eq!(
            FieldScalar::new(&field, "AAPL").unwrap().into_value(),
            Scalar::from("AAPL")
        );
    }

    #[test]
    fn the_text_is_the_canonical_spelling_the_display_writes() {
        let cases = [
            (FieldScalar::infer(Scalar::from(7_i64)).unwrap(), "7"),
            (FieldScalar::infer(Scalar::from("AAPL")).unwrap(), "AAPL"),
            (FieldScalar::infer(Scalar::from(true)).unwrap(), "true"),
            (FieldScalar::infer(Scalar::d128(150, 2)).unwrap(), "1.50"),
            (
                FieldScalar::infer(Scalar::date32(19_723)).unwrap(),
                "2024-01-01",
            ),
            (
                FieldScalar::infer(Scalar::from(&b"ABC"[..])).unwrap(),
                "ABC",
            ),
            (FieldScalar::infer(Scalar::Null).unwrap(), "null"),
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
        let typed = FieldScalar::new(&field, row.clone()).unwrap();
        assert_eq!(
            typed.to_string(),
            format!("{:?}", row.as_sequence().unwrap())
        );
        let interval = Scalar::Interval(Interval::new(1, 2, 3, TimeUnit::MonthDayNano).unwrap());
        let typed = FieldScalar::infer(interval).unwrap();
        assert_eq!(typed.to_string(), "1mo:2d:3ns@month_day_nano");
        assert_eq!(typed.into_str(), "1mo:2d:3ns@month_day_nano");
        let typed = FieldScalar::infer(Scalar::from(&[0xff_u8, 0x00][..])).unwrap();
        assert_eq!(typed.to_string(), "ff00");
        assert_eq!(typed.into_str(), "ff00");
    }

    #[test]
    fn pairings_compare_by_datatype_and_value_and_never_by_the_field_around_them() {
        let price = Field::new("price", DataType::Int32, false);
        let size = Field::new("size", DataType::Int32, true);
        let first = FieldScalar::new(&price, 7).unwrap();
        let same_value_other_field = FieldScalar::new(&size, 7).unwrap();
        let later_value = FieldScalar::new(&price, 8).unwrap();
        let wide = Field::new("price", DataType::Int64, false);
        let later_type = FieldScalar::new(&wide, 7).unwrap();

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
        let typed = FieldScalar::new(&field, 7_i64).unwrap();
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
        let unchecked = UncheckedFieldScalar::from_str(&size, "42");
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
        assert_eq!(unchecked.as_f64(), None);
        assert_eq!(unchecked.as_bool(), None);
        let checked = unchecked.clone().checked().unwrap();
        assert_eq!(checked.value(), &Scalar::from(42_i64));
        assert_eq!(unchecked.into_value(), Scalar::from("42"));

        let wrong = UncheckedFieldScalar::from_str(&size, "forty-two");
        assert_eq!(wrong.as_i64(), None);
        assert!(wrong.checked().is_err());

        // A reading follows the field's datatype whatever the held shape.
        let price = Field::new("price", DataType::decimal128(10, 2).unwrap(), false);
        let unchecked = UncheckedFieldScalar::new(&price, "1.5");
        assert_eq!(
            unchecked.as_decimal(),
            Some((yggdryl::i256::from_i128(150), 2))
        );
        let ratio = Field::new("ratio", DataType::Float64, false);
        assert_eq!(UncheckedFieldScalar::new(&ratio, "2.5").as_f64(), Some(2.5));
        // An integer is not a float value, so the reading is no reading at all.
        assert_eq!(UncheckedFieldScalar::new(&ratio, 2_i64).as_f64(), None);
        // A boolean is borrowed as held, like text and bytes: a spelling of one
        // is not read until the pairing is proven.
        let flag = Field::new("flag", DataType::Boolean, false);
        assert_eq!(UncheckedFieldScalar::new(&flag, true).as_bool(), Some(true));
        let spelled = UncheckedFieldScalar::from_str(&flag, "true");
        assert_eq!(spelled.as_bool(), None);
        assert_eq!(spelled.checked().unwrap().as_bool(), Some(true));
        assert_eq!(
            UncheckedFieldScalar::new(&flag, false).as_bool(),
            Some(false)
        );

        // A null is held and read as the absence it is.
        let held = UncheckedFieldScalar::new(&size, Scalar::Null);
        assert!(held.is_null());
        assert_eq!(held.as_i64(), None);
        assert!(held.checked().is_err(), "the field is required");
        assert!(std::ptr::eq(
            UncheckedFieldScalar::new(&size, 1).field(),
            &size
        ));
    }

    mod arrow {
        use std::sync::Arc;

        use arrow_array::ArrayRef;
        use yggdryl::StructType;
        use yggdryl::{ArrowCastOptions, DataType, Field, FieldScalar, Scalar, Serie};

        /// Lay a pairing out as the one-row column of its field.
        fn lay_out(typed: &FieldScalar<'_>) -> yggdryl::Result<ArrayRef> {
            Serie::from_scalars(typed.field().clone(), [typed.value().clone()])?
                .require_arrow_array()
        }

        /// Read row 0 of an Arrow array back as the pairing `field` types.
        fn read_back(field: &Field, array: ArrayRef) -> yggdryl::Result<FieldScalar<'_>> {
            let column = Serie::from_arrow_array(Some(field), array, ArrowCastOptions::default())?;
            FieldScalar::new(field, column.scalar(0)?)
        }

        #[test]
        fn a_pairing_round_trips_through_its_one_row_arrow_array() {
            let field = Field::new("size", DataType::Int64, false);
            let typed = FieldScalar::new(&field, 7_i64).unwrap();
            let array = lay_out(&typed).unwrap();
            assert_eq!(array.len(), 1);
            assert_eq!(read_back(&field, array).unwrap(), typed);
        }

        #[test]
        fn a_read_refuses_a_missing_row_and_casts_a_foreign_layout() {
            let field = Field::new("size", DataType::Int64, false);
            let array = lay_out(&FieldScalar::new(&field, 7_i64).unwrap()).unwrap();
            // Zero rows land as the empty column: it is not one Arrow scalar,
            // and it holds no row to pair.
            let empty = Serie::from_arrow_array(
                Some(&field),
                array.slice(0, 0),
                ArrowCastOptions::default(),
            )
            .unwrap();
            let error = empty.into_arrow_scalar().unwrap_err().to_string();
            assert!(error.contains("exactly one row, got 0"), "{error}");
            let error = read_back(&field, array.slice(0, 0))
                .expect_err("zero rows hold no row to pair")
                .to_string();
            assert!(error.contains("row 0 is past the 0 rows"), "{error}");
            // Another layout is cast into the field, so an int64 that fits
            // pairs as the int32 it narrows to...
            let narrow = Field::new("size", DataType::Int32, false);
            assert_eq!(
                read_back(&narrow, Arc::clone(&array)).unwrap(),
                FieldScalar::new(&narrow, 7_i32).unwrap()
            );
            // ...and one that does not is refused by name, never truncated.
            let wide = lay_out(&FieldScalar::new(&field, i64::MAX).unwrap()).unwrap();
            let error = Serie::from_arrow_array(
                Some(&narrow),
                wide,
                ArrowCastOptions::new().with_safe(false),
            )
            .expect_err("an int64 past the int32 range is not an int32")
            .to_string();
            assert!(error.contains("field $.size"), "{error}");
            assert!(
                error.contains("Can't cast value 9223372036854775807 to type Int32"),
                "{error}"
            );
        }

        #[test]
        fn a_null_projects_under_a_nullable_field_and_nowhere_else() {
            let nullable = Field::new("size", DataType::Int64, true);
            let absent = FieldScalar::new(&nullable, Scalar::Null).unwrap();
            let array = lay_out(&absent).unwrap();
            assert!(arrow_array::Array::is_null(array.as_ref(), 0));
            assert_eq!(read_back(&nullable, array).unwrap(), absent);
            // A required field never holds the null, so nothing projects - not
            // even under the Null datatype, whose only value it is: the pairing
            // is the field's own contract, with no canonical-default exception.
            let required = Field::new("size", DataType::Int64, false);
            assert!(FieldScalar::new(&required, Scalar::Null).is_err());
            assert!(
                FieldScalar::new(&Field::new("nothing", DataType::Null, false), Scalar::Null)
                    .is_err()
            );
            let nothing = DataType::Null.shared_field().unwrap();
            let typed = FieldScalar::new(nothing, Scalar::Null).unwrap();
            assert_eq!(lay_out(&typed).unwrap().len(), 1);
        }

        #[test]
        fn a_struct_pairing_decodes_and_reprojects_its_canonical_row_spelling() {
            let structure = StructType::from_fields([
                Field::new("id", DataType::Int64, false),
                Field::new("name", DataType::utf8(), true),
            ])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
            let row = Scalar::from_sequence([Scalar::from(7_i64), Scalar::from("XNAS")]);
            let typed = FieldScalar::new(&structure, row.clone()).unwrap();
            let array = lay_out(&typed).unwrap();
            // Arrow and the validator use the same schema-ordered row sequence.
            let decoded = read_back(&structure, Arc::clone(&array)).unwrap();
            assert_eq!(decoded.value(), &row);
            assert_eq!(lay_out(&decoded).unwrap().as_ref(), array.as_ref());
        }

        #[test]
        fn an_arrow_reading_is_canonicalized_before_the_pairing_holds_it() {
            // A float16 column reads back physically; the pairing holds what
            // the field stores, which is what a second projection expects.
            let field = Field::new("ratio", DataType::Float16, true);
            let typed = FieldScalar::new(&field, 1.5_f64).unwrap();
            let array = lay_out(&typed).unwrap();
            let decoded = read_back(&field, array).unwrap();
            assert_eq!(decoded, typed);
            assert_eq!(decoded.value().id(), yggdryl::DataTypeId::Float16);
        }
    }
}

mod records {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    use yggdryl::{DataType, Field, FieldRecord, FieldScalar, Scalar, StructType};

    fn hash_of<T: Hash>(value: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    fn schema() -> Field {
        StructType::from_fields([
            Field::new("id", DataType::Int64, false),
            Field::new("symbol", DataType::utf8(), true),
            Field::new("price", DataType::decimal128(10, 2).unwrap(), true),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
    }

    fn row() -> Scalar {
        Scalar::from_sequence([
            Scalar::from(7_i64),
            Scalar::from("AAPL"),
            Scalar::d128(150, 2),
        ])
    }

    #[test]
    fn a_row_pairs_every_cell_with_its_child() {
        let schema = schema();
        let record = FieldRecord::new(&schema, row()).unwrap();
        assert_eq!(record.len(), 3);
        assert!(!record.is_empty());
        assert!(std::ptr::eq(record.field(), &schema));
        for (index, (cell, child)) in record.iter().zip(schema.fields()).enumerate() {
            assert!(std::ptr::eq(cell.field(), child), "cell {index}");
            assert_eq!(cell.name(), child.name());
        }
        assert_eq!(
            record.names().collect::<Vec<_>>(),
            ["id", "symbol", "price"]
        );
        assert_eq!(record[0].as_i64(), Some(7));
        assert_eq!(record["symbol"].as_str(), Some("AAPL"));
        assert_eq!(record.as_str("symbol"), Some("AAPL"));
        assert_eq!(record.as_str(0), None);
        assert_eq!(record.as_str("absent"), None);
        assert_eq!(
            record.get(2).and_then(FieldScalar::as_decimal),
            Some((yggdryl::i256::from_i128(150), 2))
        );
        assert_eq!(record.get("price"), record.get_by_index(2));
        assert!(record.get(3).is_none());
        assert!(record.get_by_name("volume").is_none());
    }

    #[test]
    fn a_name_resolves_exactly_as_the_field_resolves_it() {
        let schema = StructType::from_fields([
            Field::new("Symbol", DataType::utf8(), false),
            Field::new("symbol", DataType::utf8(), false),
            StructType::from_fields([Field::new("px", DataType::Float64, false)])
                .map(DataType::from)
                .unwrap()
                .required_field("leg"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        let record = FieldRecord::new(
            &schema,
            Scalar::from_sequence([
                Scalar::from("upper"),
                Scalar::from("lower"),
                Scalar::from_sequence([Scalar::from(1.5_f64)]),
            ]),
        )
        .unwrap();
        // The field's own child lookup owns the rule: every key answers the same
        // child on both sides, and a folded name misses on both.
        for key in [
            "symbol", "Symbol", "SYMBOL", "leg", "LEG", "px", "leg.px", "absent",
        ] {
            assert_eq!(
                record.get_by_name(key).map(FieldScalar::name),
                schema
                    .index_of(key)
                    .map(|index| schema.fields()[index].name()),
                "{key}"
            );
        }
        assert_eq!(record.as_str("symbol"), Some("lower"));
        assert_eq!(record.as_str("Symbol"), Some("upper"));
        assert!(record.get("SYMBOL").is_none());
        // A dotted path names a descendant the field walks to, never a cell.
        assert_eq!(schema.get_field("leg.px").map(Field::name), Some("px"));
        assert!(record.get("leg.px").is_none());
        assert_eq!(record["leg"].get(0).and_then(|px| px.as_f64()), Some(1.5));
    }

    #[test]
    fn a_named_record_and_an_ordered_sequence_read_alike() {
        let schema = schema();
        let named = Scalar::from_struct([
            ("price", Scalar::d128(150, 2)),
            ("symbol", Scalar::from("AAPL")),
            ("id", Scalar::from(7)),
        ])
        .unwrap();
        let from_struct = FieldRecord::new(&schema, named).unwrap();
        let from_sequence = FieldRecord::new(&schema, row()).unwrap();
        assert_eq!(from_struct, from_sequence);
        assert_eq!(hash_of(&from_struct), hash_of(&from_sequence));
        // The cells are what the field stores: the id was narrowed on the way in.
        assert_eq!(from_struct[0].value(), &Scalar::from(7_i64));
        assert_eq!(from_struct.clone().into_scalar(), row());
        assert_eq!(Scalar::from(from_struct.clone()), row());
        assert_eq!(
            from_struct.into_values(),
            row().as_sequence().unwrap().to_vec()
        );
    }

    #[test]
    fn typing_a_named_row_preserves_its_source_and_fills_defaults() {
        let schema = schema();
        let source = Scalar::from_struct([("id", Scalar::from(7_i32))]).unwrap();

        let record = FieldRecord::new(&schema, source.clone()).unwrap();

        assert_eq!(record[0].value(), &Scalar::from(7_i64));
        assert!(record["symbol"].is_null());
        assert!(record["price"].is_null());
        assert!(matches!(
            source.as_struct().and_then(|values| values.get("id")),
            Some(Scalar::Int32(_))
        ));
        assert_eq!(
            source
                .as_struct()
                .and_then(|values| values.get("id"))
                .and_then(Scalar::as_i64),
            Some(7)
        );
        assert_eq!(source.as_struct().map(|values| values.len()), Some(1));
    }

    #[test]
    fn typing_a_named_row_keeps_nested_canonicalization_errors() {
        let line = StructType::from_fields([DataType::Int64.required_field("price")])
            .map(DataType::from)
            .unwrap()
            .required_field("line");
        let schema = StructType::from_fields([line])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let invalid_leaf = Scalar::from_struct([(
            "line",
            Scalar::from_struct([("price", Scalar::from("not a price"))]).unwrap(),
        )])
        .unwrap();
        let unknown = Scalar::from_struct([("unknown", Scalar::from(7))]).unwrap();

        for source in [invalid_leaf, unknown] {
            let unchanged = source.clone();
            let canonical = schema
                .canonicalize_value(source.clone())
                .unwrap_err()
                .to_string();
            let typed = FieldRecord::new(&schema, source.clone())
                .unwrap_err()
                .to_string();

            assert_eq!(typed, canonical);
            assert_eq!(source, unchanged);
        }
    }

    #[test]
    fn the_root_must_be_a_required_struct_and_the_row_must_fit_it() {
        let nullable = schema().with_nullable(true);
        let refused = FieldRecord::new(&nullable, row()).unwrap_err().to_string();
        assert!(refused.contains("non-null struct root"), "{refused}");

        let leaf = Field::new("id", DataType::Int64, false);
        let refused = FieldRecord::new(&leaf, row()).unwrap_err().to_string();
        assert!(refused.contains("struct root"), "{refused}");

        let schema = schema();
        let short = Scalar::from_sequence([Scalar::from(7_i64)]);
        let refused = FieldRecord::new(&schema, short).unwrap_err().to_string();
        assert!(refused.contains("3"), "{refused}");

        let wrong = Scalar::from_sequence([Scalar::from("seven"), Scalar::Null, Scalar::Null]);
        let refused = FieldRecord::new(&schema, wrong).unwrap_err().to_string();
        assert!(refused.contains("id"), "{refused}");

        let missing = Scalar::from_sequence([Scalar::Null, Scalar::Null, Scalar::Null]);
        assert!(
            FieldRecord::new(&schema, missing).is_err(),
            "id is required"
        );

        let extra =
            Scalar::from_struct([("id", Scalar::from(1)), ("volume", Scalar::from(1))]).unwrap();
        assert!(FieldRecord::new(&schema, extra).is_err());
    }

    #[test]
    fn an_empty_struct_reads_an_empty_row() {
        let schema = DataType::from(StructType::from_fields([]).unwrap()).required_field("row");
        let record = FieldRecord::new(&schema, Scalar::from_sequence([])).unwrap();
        assert!(record.is_empty());
        assert_eq!(record.names().count(), 0);
        assert_eq!(record.to_string(), "{}");
        assert_eq!(record.into_scalar(), Scalar::from_sequence([]));
    }

    #[test]
    fn a_row_iterates_by_value_and_by_reference() {
        let schema = schema();
        let record = FieldRecord::new(&schema, row()).unwrap();
        let names: Vec<&str> = (&record).into_iter().map(FieldScalar::name).collect();
        assert_eq!(names, ["id", "symbol", "price"]);
        let values: Vec<Scalar> = record.into_iter().map(FieldScalar::into_value).collect();
        assert_eq!(values, row().as_sequence().unwrap());
    }

    #[test]
    fn rows_compare_by_datatype_and_cells_and_never_by_the_root_around_them() {
        let first = schema();
        let renamed = schema().with_name("trade");
        let left = FieldRecord::new(&first, row()).unwrap();
        let right = FieldRecord::new(&renamed, row()).unwrap();
        assert_eq!(left, right);
        assert_eq!(hash_of(&left), hash_of(&right));

        let other = FieldRecord::new(
            &first,
            Scalar::from_sequence([Scalar::from(8_i64), Scalar::Null, Scalar::Null]),
        )
        .unwrap();
        assert_ne!(left, other);

        let widened = StructType::from_fields([
            Field::new("id", DataType::Int64, false),
            Field::new("symbol", DataType::large_utf8(), true),
            Field::new("price", DataType::decimal128(10, 2).unwrap(), true),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        let wide = FieldRecord::new(&widened, row()).unwrap();
        assert_ne!(left, wide, "another datatype is another row");
    }

    #[test]
    fn a_row_displays_its_named_cells() {
        let schema = schema();
        let record = FieldRecord::new(&schema, row()).unwrap();
        assert_eq!(record.to_string(), "{id=7, symbol=AAPL, price=1.50}");
        let absent = FieldRecord::new(
            &schema,
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::Null, Scalar::Null]),
        )
        .unwrap();
        assert_eq!(absent.to_string(), "{id=1, symbol=null, price=null}");
        let debug = format!("{record:?}");
        assert!(debug.starts_with("FieldRecord {"), "{debug}");
    }

    #[test]
    fn a_row_serializes_as_its_field_and_cells() {
        let schema = schema();
        let record = FieldRecord::new(&schema, row()).unwrap();
        let encoded: serde_json::Value = serde_json::to_value(&record).unwrap();
        assert_eq!(encoded["field"], serde_json::to_value(&schema).unwrap());
        assert_eq!(encoded["values"].as_array().map(Vec::len), Some(3));
        assert_eq!(
            encoded["values"][1]["value"],
            serde_json::to_value(Scalar::from("AAPL")).unwrap()
        );
    }

    #[test]
    #[should_panic(expected = "is not a child of the field")]
    fn subscripting_an_absent_name_panics_like_a_field_does() {
        let schema = schema();
        let record = FieldRecord::new(&schema, row()).unwrap();
        let _ = &record["volume"];
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn subscripting_a_position_past_the_row_panics_like_a_field_does() {
        let schema = schema();
        let record = FieldRecord::new(&schema, row()).unwrap();
        let _ = &record[3];
    }

    mod arrow {
        use arrow_array::{Array, RecordBatch};

        use super::{DataType, Field, FieldRecord, Scalar, row, schema};
        use yggdryl::{ArrowCastOptions, Serie, StructType};

        /// Lay rows out as the table of `root`.
        fn lay_out(
            root: &Field,
            rows: impl IntoIterator<Item = Scalar>,
        ) -> yggdryl::Result<RecordBatch> {
            Ok(Serie::from_scalars(root.clone(), rows)?.into_arrow_batch()?)
        }

        /// Read row `row` of a table back as the record `root` types.
        fn read_back<'a>(
            root: &'a Field,
            batch: &RecordBatch,
            row: usize,
        ) -> yggdryl::Result<FieldRecord<'a>> {
            let column = Serie::from_arrow_batch(Some(root), batch, ArrowCastOptions::default())?;
            FieldRecord::new(root, column.scalar(row)?)
        }

        #[test]
        fn a_row_round_trips_through_a_one_row_batch() {
            let schema = schema();
            let record = FieldRecord::new(&schema, row()).unwrap();
            let batch = lay_out(&schema, [record.clone().into_scalar()]).unwrap();
            assert_eq!(batch.num_rows(), 1);
            assert_eq!(batch.num_columns(), 3);
            let decoded = read_back(&schema, &batch, 0).unwrap();
            assert_eq!(decoded, record);
            assert_eq!(decoded.into_scalar(), row());
        }

        #[test]
        fn a_typed_row_lays_out_as_its_source_row_keeping_arrow_contracts() {
            let nested = StructType::from_fields([DataType::Int64.required_field("price")])
                .map(DataType::from)
                .unwrap()
                .nullable_field("line");
            let schema = Field::from_parts(
                "row",
                StructType::from_fields([
                    DataType::Currency.required_field("currency"),
                    Field::new(
                        "symbol",
                        DataType::dictionary(DataType::Int8, DataType::utf8()).unwrap(),
                        true,
                    ),
                    nested,
                ])
                .map(DataType::from)
                .unwrap(),
                false,
                [("owner", "trading")],
            )
            .unwrap();
            let raw =
                Scalar::from_sequence([Scalar::from("USD"), Scalar::from("AAPL"), Scalar::Null]);
            let record = FieldRecord::new(&schema, raw.clone()).unwrap();
            // The typed row lays out as the row it was typed from: the root's
            // contract, applied once, is what both columns hold.
            let typed = lay_out(&schema, [record.clone().into_scalar()]).unwrap();
            let untyped = lay_out(&schema, [raw]).unwrap();

            assert_eq!(typed, untyped);
            assert_eq!(
                typed.schema().metadata().get("owner").map(String::as_str),
                Some("trading")
            );
            assert!(typed.column(2).is_null(0));
            assert_eq!(read_back(&schema, &typed, 0).unwrap(), record);
        }

        #[test]
        fn a_typed_empty_struct_row_lays_out_as_one_row() {
            let schema = DataType::from(StructType::from_fields([]).unwrap()).required_field("row");
            let record = FieldRecord::new(&schema, Scalar::from_sequence([])).unwrap();

            let batch = lay_out(&schema, [record.clone().into_scalar()]).unwrap();

            assert_eq!(batch.num_rows(), 1);
            assert_eq!(batch.num_columns(), 0);
            assert_eq!(read_back(&schema, &batch, 0).unwrap(), record);
        }

        #[test]
        fn a_typed_row_keeps_physical_materialization_limits() {
            let large =
                DataType::fixed_size_list(DataType::Int64.required_field("item"), 1_000_001)
                    .unwrap()
                    .nullable_field("large");
            let schema =
                DataType::from(StructType::from_fields([large]).unwrap()).required_field("row");
            let raw = Scalar::from_sequence([Scalar::Null]);
            let record = FieldRecord::new(&schema, raw.clone()).unwrap();
            // The typed row is refused by the budget exactly as the row it was
            // typed from, before anything is allocated.
            let typed = lay_out(&schema, [record.into_scalar()]).unwrap_err();
            let untyped = lay_out(&schema, [raw]).unwrap_err();
            assert_eq!(typed.to_string(), untyped.to_string());
            let message = typed.to_string();
            assert!(message.contains("expanded slots"), "{message}");
            assert!(message.contains("expected at most 1000000"), "{message}");
            assert!(message.contains("got 1000001"), "{message}");
        }

        #[test]
        fn a_batch_row_is_read_under_the_field_the_batch_was_written_under() {
            let schema = schema();
            let rows = [
                row(),
                Scalar::from_sequence([Scalar::from(8_i64), Scalar::Null, Scalar::d128(1, 2)]),
            ];
            let batch = lay_out(&schema, rows).unwrap();
            let second = read_back(&schema, &batch, 1).unwrap();
            assert_eq!(second["id"].as_i64(), Some(8));
            assert!(second["symbol"].is_null());
            assert_eq!(second.as_str("symbol"), None);
            // A physically spelled reading is held canonically: the decimal is
            // at the field's scale, so it is the value the row was built from.
            assert_eq!(second["price"].value(), &Scalar::d128(1, 2));

            let past = read_back(&schema, &batch, 2).unwrap_err().to_string();
            assert!(past.contains("row 2 is past the 2 rows"), "{past}");

            // A narrower root is a projection of the table by name, so the row
            // it reads holds only the columns it names.
            let narrower = StructType::from_fields([Field::new("id", DataType::Int64, false)])
                .map(DataType::from)
                .unwrap()
                .required_field("row");
            let projected = read_back(&narrower, &batch, 1).unwrap();
            assert_eq!(projected.len(), 1);
            assert_eq!(projected["id"].as_i64(), Some(8));

            let nullable = schema.clone().with_nullable(true);
            let refused = read_back(&nullable, &batch, 0).unwrap_err().to_string();
            assert!(
                refused.contains("cast target Struct Field must be non-nullable"),
                "{refused}"
            );
        }
    }
}
