//! What a datatype accepts beside it, and what it refuses.

use crate::{DataType, Field, Scalar, TimeUnit, Timezone, TypedScalar};

#[test]
fn a_pairing_holds_only_a_value_its_datatype_accepts() {
    assert!(TypedScalar::from_parts(DataType::Int64, Scalar::from(7_i64)).is_ok());

    let rejected = TypedScalar::from_parts(DataType::Int64, Scalar::from("seven"))
        .expect_err("a string is not an int64");
    let message = rejected.to_string();
    assert!(
        message.contains("int64") && message.contains("string"),
        "the failure must name both sides, got {message}"
    );
}

#[test]
fn a_null_is_accepted_by_every_datatype_because_that_is_what_a_column_stores() {
    for data_type in [
        DataType::Int64,
        DataType::Utf8,
        DataType::Binary,
        DataType::Timestamp(TimeUnit::Nanosecond, None),
    ] {
        let typed = TypedScalar::from_parts(data_type.clone(), Scalar::Null).unwrap();
        assert!(typed.is_null());
        assert_eq!(typed.value(), &Scalar::Null);
        assert_eq!(typed.data_type(), &data_type);
    }

    // A value that is there is not null, whatever its datatype.
    assert!(
        !TypedScalar::from_parts(DataType::Int64, Scalar::from(0_i64))
            .unwrap()
            .is_null()
    );
    assert!(
        !TypedScalar::from_parts(DataType::Utf8, Scalar::from(""))
            .unwrap()
            .is_null()
    );
}

#[test]
fn a_narrow_datatype_rejects_a_value_that_does_not_fit_it() {
    // The pairing validates through the same walk a column value takes, so the
    // range of the declared width is enforced here too.
    assert!(TypedScalar::from_parts(DataType::Int8, Scalar::from(7_i64)).is_ok());
    assert!(TypedScalar::from_parts(DataType::Int8, Scalar::from(1_000_i64)).is_err());
}

#[test]
fn a_nested_value_is_validated_against_the_datatype_it_claims() {
    let row = Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]);
    let schema = DataType::from_fields([
        Field::new("id", DataType::Int64, false),
        Field::new("symbol", DataType::Utf8, false),
    ])
    .unwrap();
    assert!(TypedScalar::from_parts(schema.clone(), row).is_ok());

    let wrong = Scalar::from_sequence([Scalar::from("one"), Scalar::from("AAPL")]);
    let error = TypedScalar::from_parts(schema, wrong).expect_err("id is not text");
    assert!(
        error.to_string().contains("id"),
        "the failure must locate the child, got {error}"
    );
}

#[test]
fn a_value_can_name_its_own_datatype() {
    let typed = TypedScalar::from_value(Scalar::from(1.5_f64)).unwrap();
    assert_eq!(typed.data_type(), &DataType::Float64);
    assert_eq!(typed.value(), &Scalar::F64(1.5.into()));

    let column = TypedScalar::from_value(Scalar::from_sequence([Scalar::from(1_i64)])).unwrap();
    assert_eq!(
        column.data_type(),
        &DataType::list(Field::new("item", DataType::Int64, false)),
    );

    // A value that names no single datatype has no pairing to build.
    let mixed = Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]);
    assert!(TypedScalar::from_value(mixed).is_err());
}

#[test]
fn both_halves_come_back_out() {
    let (data_type, value) = TypedScalar::from_parts(DataType::Utf8, Scalar::from("AAPL"))
        .unwrap()
        .into_parts();

    assert_eq!(data_type, DataType::Utf8);
    assert_eq!(value, Scalar::from("AAPL"));
}

#[test]
fn pairings_order_and_stably_hash_both_halves() {
    let first = TypedScalar::from_parts(DataType::Int32, Scalar::I32(7)).unwrap();
    let equal = first.clone();
    let later_value = TypedScalar::from_parts(DataType::Int32, Scalar::I32(8)).unwrap();
    let later_type = TypedScalar::from_parts(DataType::Int64, Scalar::I64(7)).unwrap();

    assert_eq!(first.stable_hash(), equal.stable_hash());
    assert!(first < later_value);
    assert!(first < later_type);
}

#[test]
fn serde_reads_a_pairing_back_through_the_validating_constructor() {
    let typed = TypedScalar::from_parts(DataType::Int64, Scalar::from(7_i64)).unwrap();
    let encoded = serde_json::to_vec(&typed).unwrap();
    assert_eq!(
        serde_json::from_slice::<TypedScalar>(&encoded).unwrap(),
        typed
    );

    // A pairing that never agreed is refused on the way in, not stored.
    let contradiction =
        br#"{"data_type":{"type":"int64"},"value":{"type":"string","value":"seven"}}"#;
    assert!(serde_json::from_slice::<TypedScalar>(contradiction).is_err());
}

#[test]
fn a_marker_narrows_a_pairing_to_one_datatype_at_compile_time() {
    use crate::generic::{Int64Scalar, Utf8Scalar};

    let price = Int64Scalar::new(Scalar::from(7_i64)).unwrap();
    assert_eq!(price.data_type(), &DataType::Int64);
    assert_eq!(price.value(), &Scalar::I64(7));

    // The marker is checked, and the value is still checked against it.
    assert!(Int64Scalar::try_from_parts(DataType::Int64, Scalar::from(7_i64)).is_ok());
    assert!(Int64Scalar::new(Scalar::from("seven")).is_err());
    let wrong = Int64Scalar::try_from_parts(DataType::Utf8, Scalar::from("seven"))
        .expect_err("utf8 is not the int64 marker");
    let message = wrong.to_string();
    assert!(
        message.contains("int64") && message.contains("utf8"),
        "the failure must name both markers, got {message}"
    );

    // A value that names its own datatype still has to name this one.
    assert_eq!(
        Utf8Scalar::try_from_value(Scalar::from("AAPL"))
            .unwrap()
            .data_type(),
        &DataType::Utf8
    );
    assert!(Utf8Scalar::try_from_value(Scalar::from(7_i64)).is_err());
}

#[test]
fn the_newest_markers_narrow_their_pairings_like_every_other() {
    use crate::generic::{GeographyScalar, GeometryScalar, VariantScalar};

    // A variant accepts any value: the datatype is the self-describing one.
    let anything = VariantScalar::new(Scalar::from("seven")).unwrap();
    assert_eq!(anything.data_type(), &DataType::Variant);

    // A geospatial pairing validates its bytes as WKB on construction.
    let point: &[u8] = &[
        1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 240, 63, 0, 0, 0, 0, 0, 0, 0, 64,
    ];
    let shape =
        GeometryScalar::try_from_parts(DataType::geometry(None).unwrap(), Scalar::from(point))
            .unwrap();
    assert_eq!(shape.data_type().id(), crate::generic::DataTypeId::Geometry);
    assert!(
        GeometryScalar::try_from_parts(DataType::geometry(None).unwrap(), Scalar::from("no wkb"))
            .is_err()
    );
    assert!(
        GeographyScalar::try_from_parts(DataType::geography(None, None).unwrap(), Scalar::Null)
            .unwrap()
            .is_null()
    );

    // Each marker still refuses the other family's datatype.
    let wrong = GeometryScalar::try_from_parts(DataType::Variant, Scalar::Null)
        .unwrap_err()
        .to_string();
    assert!(wrong.contains("geometry"), "{wrong}");
}

#[test]
fn a_marker_is_a_view_of_the_same_pairing_and_costs_nothing() {
    use crate::generic::{Int64Scalar, TimestampScalar};

    assert_eq!(
        std::mem::size_of::<Int64Scalar>(),
        std::mem::size_of::<TypedScalar>()
    );

    // Widening and narrowing move the same two halves between markers.
    let dynamic = TypedScalar::from_parts(DataType::Int64, Scalar::from(7_i64)).unwrap();
    let narrowed: Int64Scalar = dynamic.clone().try_into_typed().unwrap();
    assert_eq!(narrowed.clone().into_any(), dynamic);
    assert!(
        dynamic
            .clone()
            .try_into_typed::<crate::field::binary::Utf8>()
            .is_err()
    );
    assert_eq!(narrowed.into_value(), Scalar::I64(7));

    // A parameterized datatype keeps its parameters in the pairing, not the marker.
    let stamp = TimestampScalar::try_from_parts(
        DataType::Timestamp(TimeUnit::Microsecond, None),
        Scalar::datetime64(0, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
    )
    .unwrap();
    assert_eq!(
        stamp.data_type(),
        &DataType::Timestamp(TimeUnit::Microsecond, None)
    );
    assert!(
        TimestampScalar::try_from_parts(DataType::Date32, Scalar::date32(0)).is_err(),
        "a date is not a timestamp"
    );
}

#[test]
fn a_narrowed_pairing_serializes_as_the_two_halves_and_reads_back_checked() {
    use crate::generic::Int64Scalar;

    let typed = Int64Scalar::new(Scalar::from(7_i64)).unwrap();
    let encoded = serde_json::to_vec(&typed).unwrap();
    assert_eq!(
        serde_json::from_slice::<TypedScalar>(&encoded).unwrap(),
        typed.clone().into_any()
    );
    assert_eq!(
        serde_json::from_slice::<Int64Scalar>(&encoded).unwrap(),
        typed
    );

    // The marker is a compile-time fact, so a datatype it refuses never loads.
    let text = br#"{"data_type":{"type":"utf8"},"value":{"type":"string","value":"seven"}}"#;
    assert!(serde_json::from_slice::<TypedScalar>(text).is_ok());
    assert!(serde_json::from_slice::<Int64Scalar>(text).is_err());
}

#[cfg(feature = "arrow")]
mod arrow {
    use super::{DataType, Field, Scalar, TypedScalar};

    #[test]
    fn a_pairing_round_trips_through_its_one_row_arrow_array() {
        let typed = TypedScalar::from_parts(DataType::Int64, Scalar::from(7_i64)).unwrap();
        let array = typed.clone().into_arrow_array().unwrap();
        assert_eq!(array.len(), 1);
        assert_eq!(
            TypedScalar::from_arrow_array(DataType::Int64, array.as_ref()).unwrap(),
            typed
        );
    }

    #[test]
    fn the_narrowed_decode_checks_the_marker_before_the_array() {
        use crate::generic::{Int64Scalar, Utf8Scalar};

        let array = Int64Scalar::new(Scalar::from(7_i64))
            .unwrap()
            .into_arrow_array()
            .unwrap();
        let typed = Int64Scalar::try_from_arrow_array(DataType::Int64, array.as_ref()).unwrap();
        assert_eq!(typed.value(), &Scalar::I64(7));

        let refused = Utf8Scalar::try_from_arrow_array(DataType::Int64, array.as_ref())
            .expect_err("an int64 is not a utf8");
        let message = refused.to_string();
        assert!(
            message.contains("utf8") && message.contains("int64"),
            "the failure must name both markers, got {message}"
        );
    }

    #[test]
    fn a_decode_refuses_a_foreign_array_that_is_not_one_exact_row() {
        let array = TypedScalar::from_parts(DataType::Int64, Scalar::from(7_i64))
            .unwrap()
            .into_arrow_array()
            .unwrap();
        // Two rows are not a scalar, and neither is a different datatype.
        let error = TypedScalar::from_arrow_array(DataType::Int64, array.slice(0, 0).as_ref())
            .expect_err("zero rows are not a scalar")
            .to_string();
        assert!(error.contains("exactly one value"), "{error}");
        let error = TypedScalar::from_arrow_array(DataType::Int32, array.as_ref())
            .expect_err("an int64 array is not an int32 scalar")
            .to_string();
        assert!(error.contains("differs from expected"), "{error}");
    }

    #[test]
    fn a_null_projects_only_when_the_datatype_default_spells_it() {
        // Null's canonical default is null, so the projection holds...
        let nothing = TypedScalar::from_parts(DataType::Null, Scalar::Null).unwrap();
        let array = nothing.clone().into_arrow_array().unwrap();
        assert_eq!(array.len(), 1);
        assert_eq!(
            TypedScalar::from_arrow_array(DataType::Null, array.as_ref()).unwrap(),
            nothing
        );

        // ...while an int64 null belongs to a nullable column, which is a
        // Field's business rather than a bare datatype's.
        let absent = TypedScalar::from_parts(DataType::Int64, Scalar::Null).unwrap();
        assert!(absent.clone().into_arrow_array().is_err());
        let column = Field::new("value", DataType::Int64, true);
        let array = crate::arrow::scalar_array(&column, &Scalar::Null).unwrap();
        assert_eq!(
            TypedScalar::from_arrow_array(DataType::Int64, array.as_ref()).unwrap(),
            absent
        );
    }

    #[test]
    fn a_struct_pairing_decodes_and_reprojects_its_canonical_row_spelling() {
        let structure = DataType::from_fields([
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, true),
        ])
        .unwrap();
        let row = Scalar::from_sequence([Scalar::from(7_i64), Scalar::from("XNAS")]);
        let typed = TypedScalar::from_parts(structure.clone(), row.clone()).unwrap();
        let array = typed.into_arrow_array().unwrap();
        // Arrow and the validator use the same schema-ordered row sequence.
        let decoded = TypedScalar::from_arrow_array(structure, array.as_ref()).unwrap();
        assert_eq!(decoded.value(), &row);
        assert_eq!(decoded.into_arrow_array().unwrap().as_ref(), array.as_ref());
    }
}
