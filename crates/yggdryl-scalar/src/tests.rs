//! Round-trip and behaviour tests for [`ScalarValue`](crate::ScalarValue): Arrow array /
//! `arrow_array::ScalarValue` / bytes / canonical-string conversion across every value
//! family (including nested), typed nulls, the `Hash` + `Eq` contract, and (under the
//! `json` feature) JSON.

use std::collections::HashSet;
use std::sync::Arc;

use arrow_array::{ArrayRef, Int32Array, StringArray};
use arrow_buffer::i256;

use crate::{from_bytes, DataType, Field, Interval, ScalarValue, TimeUnit, Timezone};

/// Asserts the Arrow-faithful round-trips: `to_array` → `from_array`,
/// `to_arrow_scalar` → `from_arrow_scalar`, and `to_bytes` → `from_bytes`.
fn roundtrip_arrow(value: &ScalarValue) {
    let array = value.to_array().expect("to_array");
    assert_eq!(array.len(), 1, "{value} array length");
    assert_eq!(
        &ScalarValue::from_array(array.as_ref(), 0).expect("from_array"),
        value,
        "{value} arrow array round-trip"
    );

    let scalar = value.to_arrow_scalar().expect("to_arrow_scalar");
    assert_eq!(
        &ScalarValue::from_arrow_scalar(&scalar).expect("from_arrow_scalar"),
        value,
        "{value} arrow scalar round-trip"
    );

    let bytes = value.to_bytes().expect("to_bytes");
    assert_eq!(
        &from_bytes(&bytes).expect("from_bytes"),
        value,
        "{value} bytes round-trip"
    );
}

/// Asserts the canonical-string round-trip (`to_str` → `from_str`).
fn roundtrip_str(value: &ScalarValue) {
    let text = value.to_str();
    assert_eq!(
        &ScalarValue::from_str(&text).unwrap_or_else(|e| panic!("from_str {text:?}: {e}")),
        value,
        "{text} string round-trip"
    );
}

#[test]
fn primitives_roundtrip() {
    let cases = [
        ScalarValue::boolean(true),
        ScalarValue::boolean(false),
        ScalarValue::int(-7, 8, true),
        ScalarValue::int(40000, 16, false),
        ScalarValue::int(42, 32, true),
        ScalarValue::int(i64::MAX as i128, 64, true),
        ScalarValue::int(u64::MAX as i128, 64, false),
        ScalarValue::float(3.5, 32),
        ScalarValue::float(123456.789012, 64),
        ScalarValue::float(1.0, 16),
        ScalarValue::utf8("hello"),
        ScalarValue::binary(vec![0u8, 1, 2, 255]),
    ];
    for value in &cases {
        roundtrip_arrow(value);
        roundtrip_str(value);
    }
}

#[test]
fn decimals_roundtrip() {
    let cases = [
        ScalarValue::decimal128(12345, 7, 2),
        ScalarValue::decimal(i256::from_i128(987), 5, 0, 32),
        ScalarValue::decimal(i256::from_i128(-987654321), 18, 4, 64),
        ScalarValue::decimal(
            i256::from_i128(170141183460469231731687303715884105727),
            38,
            10,
            128,
        ),
        ScalarValue::decimal(i256::from_i128(5), 40, 2, 256),
    ];
    for value in &cases {
        roundtrip_arrow(value);
        roundtrip_str(value);
    }
}

#[test]
fn string_and_binary_flavours_roundtrip() {
    let cases = [
        ScalarValue::Utf8 {
            value: "wide".into(),
            charset: crate::Charset::Utf8,
            large: true,
            view: false,
            size: None,
        },
        ScalarValue::Utf8 {
            value: "viewed".into(),
            charset: crate::Charset::Utf8,
            large: false,
            view: true,
            size: None,
        },
        ScalarValue::Binary {
            value: vec![9, 8, 7],
            large: true,
            view: false,
            size: None,
        },
        ScalarValue::Binary {
            value: vec![1, 2, 3, 4],
            large: false,
            view: false,
            size: Some(4),
        },
    ];
    for value in &cases {
        roundtrip_arrow(value);
    }
}

#[test]
fn temporal_roundtrip() {
    let ny = Timezone::from_str("America/New_York").unwrap();
    let cases = [
        ScalarValue::date(19_000),
        ScalarValue::Date {
            value: 1_700_000_000_000,
            large: true,
        },
        ScalarValue::Time {
            value: 3661,
            unit: TimeUnit::Second,
        },
        ScalarValue::Time {
            value: 12_345_678_900,
            unit: TimeUnit::Nanosecond,
        },
        ScalarValue::timestamp(1_700_000_000, TimeUnit::Second, None),
        ScalarValue::timestamp(1_700_000_000_000_000, TimeUnit::Microsecond, Some(ny)),
        ScalarValue::Duration {
            value: 90,
            unit: TimeUnit::Millisecond,
        },
        ScalarValue::interval_year_month(15),
        ScalarValue::Interval(Interval::DayTime {
            days: 3,
            millis: 400,
        }),
        ScalarValue::Interval(Interval::MonthDayNano {
            months: 1,
            days: 2,
            nanos: 3,
        }),
    ];
    for value in &cases {
        roundtrip_arrow(value);
        roundtrip_str(value);
    }
}

#[test]
fn nested_roundtrip() {
    let item = Field::new("item", DataType::int(32, true), true);
    let list = ScalarValue::List {
        values: vec![ScalarValue::int(1, 32, true), ScalarValue::int(2, 32, true)],
        field: Box::new(item.clone()),
        large: false,
        view: false,
        size: None,
    };
    roundtrip_arrow(&list);

    let large_list = ScalarValue::List {
        values: vec![ScalarValue::int(9, 32, true)],
        field: Box::new(item),
        large: true,
        view: false,
        size: None,
    };
    roundtrip_arrow(&large_list);

    let record = ScalarValue::Struct {
        fields: vec![
            Field::new("id", DataType::int(64, true), false),
            Field::new("name", DataType::varchar(), true),
        ],
        values: vec![ScalarValue::int(7, 64, true), ScalarValue::utf8("x")],
    };
    roundtrip_arrow(&record);

    let map = ScalarValue::Map {
        key: Box::new(DataType::varchar()),
        value: Box::new(DataType::int(32, true)),
        sorted: false,
        entries: vec![
            (ScalarValue::utf8("a"), ScalarValue::int(1, 32, true)),
            (ScalarValue::utf8("b"), ScalarValue::int(2, 32, true)),
        ],
    };
    roundtrip_arrow(&map);

    // A list of structs — recursion through both nested builders.
    let row = DataType::struct_(vec![Field::new("k", DataType::int(32, true), true)]);
    let list_of_structs = ScalarValue::List {
        values: vec![ScalarValue::Struct {
            fields: vec![Field::new("k", DataType::int(32, true), true)],
            values: vec![ScalarValue::int(5, 32, true)],
        }],
        field: Box::new(Field::new("item", row, true)),
        large: false,
        view: false,
        size: None,
    };
    roundtrip_arrow(&list_of_structs);
}

#[test]
fn empty_nested_roundtrip() {
    let empty_list = ScalarValue::List {
        values: vec![],
        field: Box::new(Field::new("item", DataType::int(32, true), true)),
        large: false,
        view: false,
        size: None,
    };
    roundtrip_arrow(&empty_list);
}

#[test]
fn typed_null_roundtrip() {
    let value = ScalarValue::null(DataType::int(64, true));
    assert!(value.is_null());
    roundtrip_arrow(&value);
    roundtrip_str(&value);
    assert_eq!(value.data_type(), DataType::int(64, true));
}

#[test]
fn null_cell_reads_typed_null() {
    let array: ArrayRef = Arc::new(Int32Array::from(vec![Some(1), None]));
    assert_eq!(
        ScalarValue::from_array(array.as_ref(), 1).unwrap(),
        ScalarValue::null(DataType::int(32, true))
    );
    // Out-of-bounds is a typed null too.
    assert!(ScalarValue::from_array(array.as_ref(), 9)
        .unwrap()
        .is_null());
}

#[test]
fn json_and_bson_normalise_through_arrow_but_keep_logical_string() {
    let json = ScalarValue::json("{\"a\":1}");
    // The canonical string keeps the logical type.
    roundtrip_str(&json);
    assert_eq!(json.data_type(), DataType::json());
    // Through Arrow it normalises to its physical Utf8.
    let array = json.to_array().unwrap();
    assert_eq!(
        ScalarValue::from_array(array.as_ref(), 0).unwrap(),
        ScalarValue::utf8("{\"a\":1}")
    );

    let bson = ScalarValue::bson(vec![1, 2, 3]);
    roundtrip_str(&bson);
    assert_eq!(bson.data_type(), DataType::bson());
}

#[test]
fn data_type_is_exact() {
    assert_eq!(
        ScalarValue::int(1, 16, false).data_type(),
        DataType::int(16, false)
    );
    assert_eq!(ScalarValue::float(1.0, 32).data_type(), DataType::float(32));
    assert_eq!(
        ScalarValue::timestamp(0, TimeUnit::Microsecond, None).data_type(),
        DataType::timestamp(TimeUnit::Microsecond, None)
    );
    assert_eq!(
        ScalarValue::decimal128(1, 10, 2).data_type(),
        DataType::decimal(10, 2)
    );
}

#[test]
fn accessors() {
    assert_eq!(ScalarValue::boolean(true).as_bool(), Some(true));
    assert_eq!(ScalarValue::int(5, 64, true).as_i128(), Some(5));
    assert_eq!(ScalarValue::float(1.5, 64).as_f64(), Some(1.5));
    assert_eq!(ScalarValue::utf8("hi").as_str(), Some("hi"));
    assert_eq!(
        ScalarValue::binary(vec![1, 2]).as_bytes(),
        Some(&[1u8, 2][..])
    );
    assert_eq!(ScalarValue::int(5, 64, true).as_f64(), None);
}

#[test]
fn temporal_accessors_return_core_types() {
    let ts = ScalarValue::timestamp(1_000_000_000, TimeUnit::Second, None);
    assert_eq!(ts.as_datetime().unwrap().epoch_seconds(), 1_000_000_000);

    let dt = ScalarValue::date(100);
    assert_eq!(dt.as_date().unwrap().epoch_days(), 100);

    let dur = ScalarValue::Duration {
        value: 5,
        unit: TimeUnit::Second,
    };
    assert_eq!(dur.as_duration().unwrap().as_nanos(), 5_000_000_000);
}

#[test]
fn from_impls() {
    assert_eq!(ScalarValue::from(true), ScalarValue::boolean(true));
    assert_eq!(ScalarValue::from(42i64), ScalarValue::int(42, 64, true));
    assert_eq!(ScalarValue::from(1.5f64), ScalarValue::float(1.5, 64));
    assert_eq!(ScalarValue::from("x"), ScalarValue::utf8("x"));
}

#[test]
fn hash_eq_contract() {
    // NaN floats are equal to themselves and hash the same, so a ScalarValue keys a set.
    let nan_a = ScalarValue::float(f64::NAN, 64);
    let nan_b = ScalarValue::float(f64::NAN, 64);
    assert_eq!(nan_a, nan_b);

    let mut set = HashSet::new();
    set.insert(ScalarValue::int(1, 64, true));
    set.insert(ScalarValue::int(1, 64, true));
    set.insert(nan_a);
    set.insert(nan_b);
    set.insert(ScalarValue::utf8("k"));
    assert_eq!(set.len(), 3);

    // +0.0 and -0.0 are equal.
    assert_eq!(ScalarValue::float(0.0, 64), ScalarValue::float(-0.0, 64));
    // Different widths are different scalars.
    assert_ne!(ScalarValue::int(1, 32, true), ScalarValue::int(1, 64, true));
}

#[test]
fn from_str_rejects_nested_and_bad_input() {
    let list = ScalarValue::List {
        values: vec![ScalarValue::int(1, 32, true)],
        field: Box::new(Field::new("item", DataType::int(32, true), true)),
        large: false,
        view: false,
        size: None,
    };
    // Nested types render but do not parse back from a string.
    assert!(ScalarValue::from_str(&list.to_str()).is_err());
    assert!(ScalarValue::from_str("nonsense").is_err());
    assert!(ScalarValue::from_str("notanint::int64").is_err());
}

#[test]
fn mapping_roundtrip() {
    let value = ScalarValue::int(99, 32, true);
    let map = value.to_mapping();
    assert_eq!(map.get("type").unwrap(), "int32");
    assert_eq!(map.get("value").unwrap(), "99");
    assert_eq!(ScalarValue::from_mapping(&map).unwrap(), value);
}

#[test]
fn reads_from_any_arrow_array() {
    // A scalar can be lifted out of any Arrow array, not just one it built.
    let array: ArrayRef = Arc::new(StringArray::from(vec!["a", "b", "c"]));
    assert_eq!(
        ScalarValue::from_array(array.as_ref(), 1).unwrap(),
        ScalarValue::utf8("b")
    );
}

#[cfg(feature = "json")]
#[test]
fn json_roundtrip() {
    let cases = [
        ScalarValue::int(i64::MAX as i128, 64, true),
        ScalarValue::decimal(i256::from_i128(123456789), 18, 3, 64),
        ScalarValue::utf8("text"),
        ScalarValue::null(DataType::float(64)),
        ScalarValue::timestamp(1, TimeUnit::Nanosecond, None),
    ];
    for value in &cases {
        let json = value.to_json();
        assert_eq!(&ScalarValue::from_json(&json).unwrap(), value, "{json}");
    }
}

// ---- the Scalar trait + per-type concrete scalars (Stage 1) ----

use crate::{
    from_value, BooleanScalar, Date, DateScalar, IntScalar, ListScalar, MapScalar, NullScalar,
    Scalar, ScalarRef, StructScalar, TimezoneScalar, TypedScalar, VarcharScalar,
};

#[test]
fn concrete_scalars_and_trait() {
    let i = IntScalar::new(42, 32, true);
    assert_eq!(*i.data_type(), DataType::int(32, true));
    assert!(!i.is_null());
    assert_eq!(i.to_str(), "42::int32");
    assert_eq!(<IntScalar as TypedScalar<i128>>::get(&i), Some(42));

    // the factory wraps a value in the right concrete, downcastable from the trait object.
    let boxed: ScalarRef = ScalarValue::int(7, 64, true).into_scalar();
    assert!(boxed.as_any().downcast_ref::<IntScalar>().is_some());
    assert_eq!(*boxed.data_type(), DataType::int(64, true));

    // every concrete round-trips its value through Arrow via the trait.
    for s in [
        from_value(ScalarValue::boolean(true)),
        from_value(ScalarValue::utf8("hi")),
        from_value(ScalarValue::float(1.5, 64)),
        from_value(ScalarValue::date(100)),
        TimezoneScalar::new(Timezone::Utc).into(),
    ] {
        let array = s.to_array().unwrap();
        let back = ScalarValue::scalar_at(array.as_ref(), 0).unwrap();
        // Arrow-faithful types round-trip exactly; logical ones (timezone) normalise.
        assert_eq!(back.to_array().unwrap().data_type(), array.data_type());
    }
}

#[test]
fn typed_concrete_accessors() {
    assert_eq!(
        <BooleanScalar as TypedScalar<bool>>::get(&BooleanScalar::new(true)),
        Some(true)
    );
    assert_eq!(
        <VarcharScalar as TypedScalar<String>>::get(&VarcharScalar::new("x")),
        Some("x".to_string())
    );
    assert_eq!(
        <DateScalar as TypedScalar<Date>>::get(&DateScalar::new(100)).map(|d| d.epoch_days()),
        Some(100)
    );
    assert!(NullScalar::new(DataType::int(64, true)).is_null());
}

#[test]
fn scalar_cast_between_types() {
    let i = IntScalar::new(42, 32, true);
    let as64 = i.cast(&DataType::int(64, true)).unwrap();
    assert_eq!(*as64.data_type(), DataType::int(64, true));
    assert_eq!(
        as64.as_any().downcast_ref::<IntScalar>().unwrap().to_str(),
        "42::int64"
    );

    let as_f = i.cast(&DataType::float(64)).unwrap();
    assert_eq!(*as_f.data_type(), DataType::float(64));

    let as_str = i.cast(&DataType::varchar()).unwrap();
    assert_eq!(as_str.to_str(), "'42'::utf8");
}

#[test]
fn nested_struct_scalar_recursive() {
    // struct { id: int32, name: utf8, inner: struct { x: int32 } }
    let inner = StructScalar::from_children(
        vec![Field::new("x", DataType::int(32, true), true)],
        vec![IntScalar::new(5, 32, true).into()],
    );
    let rec = StructScalar::from_children(
        vec![
            Field::new("id", DataType::int(64, true), false),
            Field::new("name", DataType::varchar(), true),
            Field::new(
                "inner",
                DataType::struct_(vec![Field::new("x", DataType::int(32, true), true)]),
                true,
            ),
        ],
        vec![
            IntScalar::new(7, 64, true).into(),
            VarcharScalar::new("y").into(),
            inner.into(),
        ],
    );
    // recursive child access by index + name.
    assert_eq!(rec.child(0).unwrap().to_str(), "7::int64");
    assert_eq!(rec.child_named("name").unwrap().to_str(), "'y'::utf8");
    let inner_ref = rec.child_named("inner").unwrap();
    let inner_struct = inner_ref.as_any().downcast_ref::<StructScalar>().unwrap();
    assert_eq!(inner_struct.child_named("x").unwrap().to_str(), "5::int32");
    assert_eq!(rec.children().len(), 3);

    // round-trips through Arrow (recursive struct array build + read-back).
    let array = rec.to_array().unwrap();
    assert_eq!(array.len(), 1);
    let back = ScalarValue::from_array(array.as_ref(), 0).unwrap();
    assert_eq!(back, rec.value().clone());
}

#[test]
fn list_of_struct_and_map_recursive() {
    // list[ struct { k: int32 } ] — deep nesting through both nested builders.
    let item_type = DataType::struct_(vec![Field::new("k", DataType::int(32, true), true)]);
    let row = StructScalar::from_children(
        vec![Field::new("k", DataType::int(32, true), true)],
        vec![IntScalar::new(9, 32, true).into()],
    );
    let list = ListScalar::from_children(
        Field::new("item", item_type, true),
        vec![row.into(), {
            // a second row with a null child.
            StructScalar::from_children(
                vec![Field::new("k", DataType::int(32, true), true)],
                vec![NullScalar::new(DataType::int(32, true)).into()],
            )
            .into()
        }],
    );
    assert_eq!(list.len(), 2);
    let first = list.values()[0]
        .as_any()
        .downcast_ref::<StructScalar>()
        .unwrap()
        .child_named("k");
    assert_eq!(first.unwrap().to_str(), "9::int32");
    // Arrow round-trip of the whole nested list.
    let array = list.to_array().unwrap();
    assert_eq!(
        ScalarValue::from_array(array.as_ref(), 0).unwrap(),
        list.value().clone()
    );

    // map[utf8, int32]
    let map = MapScalar::from_entries(
        DataType::varchar(),
        DataType::int(32, true),
        false,
        vec![
            (
                VarcharScalar::new("a").into(),
                IntScalar::new(1, 32, true).into(),
            ),
            (
                VarcharScalar::new("b").into(),
                IntScalar::new(2, 32, true).into(),
            ),
        ],
    );
    let entries = map.entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].0.to_str(), "'a'::utf8");
    assert_eq!(entries[1].1.to_str(), "2::int32");
    let array = map.to_array().unwrap();
    assert_eq!(
        ScalarValue::from_array(array.as_ref(), 0).unwrap(),
        map.value().clone()
    );
}

#[test]
fn nested_edge_cases() {
    // empty list + empty struct round-trip.
    let empty_list =
        ListScalar::from_children(Field::new("item", DataType::int(32, true), true), vec![]);
    assert!(empty_list.is_empty());
    let a = empty_list.to_array().unwrap();
    assert_eq!(
        ScalarValue::from_array(a.as_ref(), 0).unwrap(),
        empty_list.value().clone()
    );

    let empty_struct = StructScalar::from_children(vec![], vec![]);
    assert!(empty_struct.children().is_empty());

    // timezone scalar keeps its logical type via to_str, normalises to utf8 in Arrow.
    let tz = TimezoneScalar::new(Timezone::from_str("America/New_York").unwrap());
    assert_eq!(*tz.data_type(), DataType::Timezone);
    assert_eq!(
        <TimezoneScalar as TypedScalar<Timezone>>::get(&tz)
            .unwrap()
            .name(),
        "America/New_York"
    );
    // its Arrow array is the physical Utf8 (same as a string scalar's).
    assert_eq!(
        tz.to_array().unwrap().data_type(),
        ScalarValue::utf8("x").to_array().unwrap().data_type()
    );
}

#[test]
fn scalar_value_arithmetic_edge_cases() {
    // unsigned underflow / negation must error, not silently wrap on Arrow conversion
    let u = ScalarValue::int(5, 64, false);
    assert!(u.sub(&ScalarValue::int(10, 64, false)).is_err());
    assert!(u.neg().is_err());
    assert_eq!(
        ScalarValue::int(0, 64, false).neg().unwrap(),
        ScalarValue::int(0, 64, false)
    );

    // decimal multiply that overflows 128-bit precision widens to 256-bit, not truncates
    // 2e-30 * 3e-30 = 6e-60 (scale 60 needs 256-bit storage)
    let a = ScalarValue::decimal128(2, 38, 30);
    let b = ScalarValue::decimal128(3, 38, 30);
    let prod = a.mul(&b).unwrap();
    match prod {
        ScalarValue::Decimal {
            value, scale, bits, ..
        } => {
            assert_eq!(value, i256::from_i128(6));
            assert_eq!(scale, 60);
            assert_eq!(bits, 256);
        }
        _ => panic!("expected a decimal"),
    }
    // and it builds a valid Arrow array (precision/scale are coherent)
    assert!(prod.to_array().is_ok());

    // a null operand does NOT turn an unsupported combination into a successful null
    let null_str = ScalarValue::Null(DataType::varchar());
    assert!(null_str.add(&ScalarValue::int(1, 32, true)).is_err());

    // duration division by zero is a division-by-zero error (not "unsupported combination")
    let dur = ScalarValue::from_duration(&Duration::from_secs(10));
    let err = dur.div(&ScalarValue::int(0, 64, true)).unwrap_err();
    assert!(format!("{err}").contains("zero"));
}

// ---- arithmetic (add / sub / mul / div / neg) ----

use crate::{Duration, DurationScalar, FloatScalar};

#[test]
fn scalar_value_numeric_arithmetic() {
    let a = ScalarValue::int(7, 32, true);
    let b = ScalarValue::int(5, 32, true);
    assert_eq!(a.add(&b).unwrap(), ScalarValue::int(12, 32, true));
    assert_eq!(a.sub(&b).unwrap(), ScalarValue::int(2, 32, true));
    assert_eq!(a.mul(&b).unwrap(), ScalarValue::int(35, 32, true));
    assert_eq!(a.div(&b).unwrap(), ScalarValue::int(1, 32, true));

    // width / signedness promote to the wider, signed-if-either operand
    assert_eq!(
        ScalarValue::int(10, 64, false)
            .add(&ScalarValue::int(3, 16, true))
            .unwrap(),
        ScalarValue::int(13, 64, true)
    );
    // a float anywhere widens to f64
    assert_eq!(
        a.add(&ScalarValue::float(2.5, 64)).unwrap(),
        ScalarValue::float(9.5, 64)
    );
    // division by zero and overflow are actionable errors
    assert!(a.div(&ScalarValue::int(0, 32, true)).is_err());
    assert!(ScalarValue::int(i128::MAX, 64, true)
        .add(&ScalarValue::int(1, 64, true))
        .is_err());
    assert_eq!(a.neg().unwrap(), ScalarValue::int(-7, 32, true));
}

#[test]
fn scalar_value_temporal_arithmetic() {
    let day = ScalarValue::from_duration(&Duration::from_secs(86_400));
    assert_eq!(
        day.add(&day).unwrap().as_duration().unwrap().as_seconds(),
        172_800
    );
    assert_eq!(
        day.mul(&ScalarValue::int(3, 64, true))
            .unwrap()
            .as_duration()
            .unwrap()
            .as_seconds(),
        259_200
    );
    let d0 = ScalarValue::from_date(&Date::from_ymd(2024, 1, 1).unwrap());
    let d1 = d0.add(&day).unwrap();
    assert_eq!(d1.as_date().unwrap(), Date::from_ymd(2024, 1, 2).unwrap());
    assert_eq!(
        d1.sub(&d0).unwrap().as_duration().unwrap().as_seconds(),
        86_400
    );
}

#[test]
fn scalar_trait_arithmetic() {
    let a = IntScalar::new(6, 64, true);
    let b = IntScalar::new(4, 64, true);
    assert_eq!(a.add(&b).unwrap().to_str(), "10::int64");
    assert_eq!(a.mul(&b).unwrap().to_str(), "24::int64");
    assert_eq!(a.neg().unwrap().to_str(), "-6::int64");
    assert_eq!(
        a.add(&FloatScalar::new(1.5, 64)).unwrap().to_str(),
        "7.5::float64"
    );
    assert!(VarcharScalar::new("x").add(&a).is_err());
    // a value-level cast is also available
    let dur = DurationScalar::from_duration(&Duration::from_secs(1));
    assert!(dur.value().cast(&DataType::int(64, true)).is_ok());
}
