//! Round-trip and behaviour tests for [`Scalar`](crate::Scalar): Arrow array /
//! `arrow_array::Scalar` / bytes / canonical-string conversion across every value
//! family (including nested), typed nulls, the `Hash` + `Eq` contract, and (under the
//! `json` feature) JSON.

use std::collections::HashSet;
use std::sync::Arc;

use arrow_array::{ArrayRef, Int32Array, StringArray};
use arrow_buffer::i256;

use crate::{from_bytes, DataType, Field, Interval, Scalar, TimeUnit, Timezone};

/// Asserts the Arrow-faithful round-trips: `to_array` → `from_array`,
/// `to_arrow_scalar` → `from_arrow_scalar`, and `to_bytes` → `from_bytes`.
fn roundtrip_arrow(value: &Scalar) {
    let array = value.to_array().expect("to_array");
    assert_eq!(array.len(), 1, "{value} array length");
    assert_eq!(
        &Scalar::from_array(array.as_ref(), 0).expect("from_array"),
        value,
        "{value} arrow array round-trip"
    );

    let scalar = value.to_arrow_scalar().expect("to_arrow_scalar");
    assert_eq!(
        &Scalar::from_arrow_scalar(&scalar).expect("from_arrow_scalar"),
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
fn roundtrip_str(value: &Scalar) {
    let text = value.to_str();
    assert_eq!(
        &Scalar::from_str(&text).unwrap_or_else(|e| panic!("from_str {text:?}: {e}")),
        value,
        "{text} string round-trip"
    );
}

#[test]
fn primitives_roundtrip() {
    let cases = [
        Scalar::boolean(true),
        Scalar::boolean(false),
        Scalar::int(-7, 8, true),
        Scalar::int(40000, 16, false),
        Scalar::int(42, 32, true),
        Scalar::int(i64::MAX as i128, 64, true),
        Scalar::int(u64::MAX as i128, 64, false),
        Scalar::float(3.5, 32),
        Scalar::float(123456.789012, 64),
        Scalar::float(1.0, 16),
        Scalar::utf8("hello"),
        Scalar::binary(vec![0u8, 1, 2, 255]),
    ];
    for value in &cases {
        roundtrip_arrow(value);
        roundtrip_str(value);
    }
}

#[test]
fn decimals_roundtrip() {
    let cases = [
        Scalar::decimal128(12345, 7, 2),
        Scalar::decimal(i256::from_i128(987), 5, 0, 32),
        Scalar::decimal(i256::from_i128(-987654321), 18, 4, 64),
        Scalar::decimal(
            i256::from_i128(170141183460469231731687303715884105727),
            38,
            10,
            128,
        ),
        Scalar::decimal(i256::from_i128(5), 40, 2, 256),
    ];
    for value in &cases {
        roundtrip_arrow(value);
        roundtrip_str(value);
    }
}

#[test]
fn string_and_binary_flavours_roundtrip() {
    let cases = [
        Scalar::Utf8 {
            value: "wide".into(),
            charset: crate::Charset::Utf8,
            large: true,
            view: false,
            size: None,
        },
        Scalar::Utf8 {
            value: "viewed".into(),
            charset: crate::Charset::Utf8,
            large: false,
            view: true,
            size: None,
        },
        Scalar::Binary {
            value: vec![9, 8, 7],
            large: true,
            view: false,
            size: None,
        },
        Scalar::Binary {
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
        Scalar::date(19_000),
        Scalar::Date {
            value: 1_700_000_000_000,
            large: true,
        },
        Scalar::Time {
            value: 3661,
            unit: TimeUnit::Second,
        },
        Scalar::Time {
            value: 12_345_678_900,
            unit: TimeUnit::Nanosecond,
        },
        Scalar::timestamp(1_700_000_000, TimeUnit::Second, None),
        Scalar::timestamp(1_700_000_000_000_000, TimeUnit::Microsecond, Some(ny)),
        Scalar::Duration {
            value: 90,
            unit: TimeUnit::Millisecond,
        },
        Scalar::interval_year_month(15),
        Scalar::Interval(Interval::DayTime {
            days: 3,
            millis: 400,
        }),
        Scalar::Interval(Interval::MonthDayNano {
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
    let list = Scalar::List {
        values: vec![Scalar::int(1, 32, true), Scalar::int(2, 32, true)],
        field: Box::new(item.clone()),
        large: false,
        view: false,
        size: None,
    };
    roundtrip_arrow(&list);

    let large_list = Scalar::List {
        values: vec![Scalar::int(9, 32, true)],
        field: Box::new(item),
        large: true,
        view: false,
        size: None,
    };
    roundtrip_arrow(&large_list);

    let record = Scalar::Struct {
        fields: vec![
            Field::new("id", DataType::int(64, true), false),
            Field::new("name", DataType::varchar(), true),
        ],
        values: vec![Scalar::int(7, 64, true), Scalar::utf8("x")],
    };
    roundtrip_arrow(&record);

    let map = Scalar::Map {
        key: Box::new(DataType::varchar()),
        value: Box::new(DataType::int(32, true)),
        sorted: false,
        entries: vec![
            (Scalar::utf8("a"), Scalar::int(1, 32, true)),
            (Scalar::utf8("b"), Scalar::int(2, 32, true)),
        ],
    };
    roundtrip_arrow(&map);

    // A list of structs — recursion through both nested builders.
    let row = DataType::struct_(vec![Field::new("k", DataType::int(32, true), true)]);
    let list_of_structs = Scalar::List {
        values: vec![Scalar::Struct {
            fields: vec![Field::new("k", DataType::int(32, true), true)],
            values: vec![Scalar::int(5, 32, true)],
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
    let empty_list = Scalar::List {
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
    let value = Scalar::null(DataType::int(64, true));
    assert!(value.is_null());
    roundtrip_arrow(&value);
    roundtrip_str(&value);
    assert_eq!(value.data_type(), DataType::int(64, true));
}

#[test]
fn null_cell_reads_typed_null() {
    let array: ArrayRef = Arc::new(Int32Array::from(vec![Some(1), None]));
    assert_eq!(
        Scalar::from_array(array.as_ref(), 1).unwrap(),
        Scalar::null(DataType::int(32, true))
    );
    // Out-of-bounds is a typed null too.
    assert!(Scalar::from_array(array.as_ref(), 9).unwrap().is_null());
}

#[test]
fn json_and_bson_normalise_through_arrow_but_keep_logical_string() {
    let json = Scalar::json("{\"a\":1}");
    // The canonical string keeps the logical type.
    roundtrip_str(&json);
    assert_eq!(json.data_type(), DataType::json());
    // Through Arrow it normalises to its physical Utf8.
    let array = json.to_array().unwrap();
    assert_eq!(
        Scalar::from_array(array.as_ref(), 0).unwrap(),
        Scalar::utf8("{\"a\":1}")
    );

    let bson = Scalar::bson(vec![1, 2, 3]);
    roundtrip_str(&bson);
    assert_eq!(bson.data_type(), DataType::bson());
}

#[test]
fn data_type_is_exact() {
    assert_eq!(
        Scalar::int(1, 16, false).data_type(),
        DataType::int(16, false)
    );
    assert_eq!(Scalar::float(1.0, 32).data_type(), DataType::float(32));
    assert_eq!(
        Scalar::timestamp(0, TimeUnit::Microsecond, None).data_type(),
        DataType::timestamp(TimeUnit::Microsecond, None)
    );
    assert_eq!(
        Scalar::decimal128(1, 10, 2).data_type(),
        DataType::decimal(10, 2)
    );
}

#[test]
fn accessors() {
    assert_eq!(Scalar::boolean(true).as_bool(), Some(true));
    assert_eq!(Scalar::int(5, 64, true).as_i128(), Some(5));
    assert_eq!(Scalar::float(1.5, 64).as_f64(), Some(1.5));
    assert_eq!(Scalar::utf8("hi").as_str(), Some("hi"));
    assert_eq!(Scalar::binary(vec![1, 2]).as_bytes(), Some(&[1u8, 2][..]));
    assert_eq!(Scalar::int(5, 64, true).as_f64(), None);
}

#[test]
fn temporal_accessors_return_core_types() {
    let ts = Scalar::timestamp(1_000_000_000, TimeUnit::Second, None);
    assert_eq!(ts.as_datetime().unwrap().epoch_seconds(), 1_000_000_000);

    let dt = Scalar::date(100);
    assert_eq!(dt.as_date().unwrap().epoch_days(), 100);

    let dur = Scalar::Duration {
        value: 5,
        unit: TimeUnit::Second,
    };
    assert_eq!(dur.as_duration().unwrap().as_nanos(), 5_000_000_000);
}

#[test]
fn from_impls() {
    assert_eq!(Scalar::from(true), Scalar::boolean(true));
    assert_eq!(Scalar::from(42i64), Scalar::int(42, 64, true));
    assert_eq!(Scalar::from(1.5f64), Scalar::float(1.5, 64));
    assert_eq!(Scalar::from("x"), Scalar::utf8("x"));
}

#[test]
fn hash_eq_contract() {
    // NaN floats are equal to themselves and hash the same, so a Scalar keys a set.
    let nan_a = Scalar::float(f64::NAN, 64);
    let nan_b = Scalar::float(f64::NAN, 64);
    assert_eq!(nan_a, nan_b);

    let mut set = HashSet::new();
    set.insert(Scalar::int(1, 64, true));
    set.insert(Scalar::int(1, 64, true));
    set.insert(nan_a);
    set.insert(nan_b);
    set.insert(Scalar::utf8("k"));
    assert_eq!(set.len(), 3);

    // +0.0 and -0.0 are equal.
    assert_eq!(Scalar::float(0.0, 64), Scalar::float(-0.0, 64));
    // Different widths are different scalars.
    assert_ne!(Scalar::int(1, 32, true), Scalar::int(1, 64, true));
}

#[test]
fn from_str_rejects_nested_and_bad_input() {
    let list = Scalar::List {
        values: vec![Scalar::int(1, 32, true)],
        field: Box::new(Field::new("item", DataType::int(32, true), true)),
        large: false,
        view: false,
        size: None,
    };
    // Nested types render but do not parse back from a string.
    assert!(Scalar::from_str(&list.to_str()).is_err());
    assert!(Scalar::from_str("nonsense").is_err());
    assert!(Scalar::from_str("notanint::int64").is_err());
}

#[test]
fn mapping_roundtrip() {
    let value = Scalar::int(99, 32, true);
    let map = value.to_mapping();
    assert_eq!(map.get("type").unwrap(), "int32");
    assert_eq!(map.get("value").unwrap(), "99");
    assert_eq!(Scalar::from_mapping(&map).unwrap(), value);
}

#[test]
fn reads_from_any_arrow_array() {
    // A scalar can be lifted out of any Arrow array, not just one it built.
    let array: ArrayRef = Arc::new(StringArray::from(vec!["a", "b", "c"]));
    assert_eq!(
        Scalar::from_array(array.as_ref(), 1).unwrap(),
        Scalar::utf8("b")
    );
}

#[cfg(feature = "json")]
#[test]
fn json_roundtrip() {
    let cases = [
        Scalar::int(i64::MAX as i128, 64, true),
        Scalar::decimal(i256::from_i128(123456789), 18, 3, 64),
        Scalar::utf8("text"),
        Scalar::null(DataType::float(64)),
        Scalar::timestamp(1, TimeUnit::Nanosecond, None),
    ];
    for value in &cases {
        let json = value.to_json();
        assert_eq!(&Scalar::from_json(&json).unwrap(), value, "{json}");
    }
}
