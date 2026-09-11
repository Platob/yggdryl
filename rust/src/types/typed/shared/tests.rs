//! Which datatypes keep a shared field, and that each keeps exactly one.

use crate::{DataType, DataTypeId, Field, Scalar, TimeUnit, Timezone};

#[test]
fn every_parameter_free_datatype_keeps_one_nullable_value_field() {
    for id in DataTypeId::ALL {
        if id.is_parameterized() {
            continue;
        }
        let Ok(dtype) = DataType::from_str(id.as_str()) else {
            // The 128-bit integer widths name no datatype of their own.
            assert!(
                matches!(id, DataTypeId::Int128 | DataTypeId::UInt128),
                "{id:?}"
            );
            continue;
        };
        if dtype.id() != id {
            assert!(
                matches!(id, DataTypeId::Int128 | DataTypeId::UInt128),
                "{id:?}"
            );
            continue;
        }
        let shared = dtype
            .shared_field()
            .unwrap_or_else(|| panic!("{id:?} keeps no shared field"));
        assert_eq!(shared.name(), "value");
        assert_eq!(shared.dtype(), &dtype);
        assert!(shared.is_nullable());
        assert!(shared.as_metadata().is_empty());
        assert!(
            std::ptr::eq(shared, dtype.shared_field().unwrap()),
            "{id:?} answered two fields"
        );
    }
}

#[test]
fn a_parameterized_leaf_is_interned_once_per_distinct_datatype() {
    let cases = [
        DataType::FixedAscii(4),
        DataType::FixedSizeBinary(16),
        DataType::decimal32(9, 2).unwrap(),
        DataType::decimal64(18, 4).unwrap(),
        DataType::decimal128(38, 10).unwrap(),
        DataType::decimal256(76, 20).unwrap(),
        DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC).unwrap(),
        DataType::datetime64(TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
        DataType::Time32(TimeUnit::Second),
        DataType::Time64(TimeUnit::Nanosecond),
        DataType::Duration32(TimeUnit::Millisecond),
        DataType::Duration64(TimeUnit::Nanosecond),
        DataType::Interval(TimeUnit::MonthDayNano),
    ];
    for dtype in &cases {
        let first = dtype
            .shared_field()
            .unwrap_or_else(|| panic!("{dtype} keeps no shared field"));
        assert_eq!(first.dtype(), dtype);
        assert!(first.is_nullable());
        assert_eq!(first.name(), "value");
        assert!(std::ptr::eq(first, dtype.shared_field().unwrap()));
    }
    // Two datatypes differing only in a parameter keep two fields.
    assert!(!std::ptr::eq(
        DataType::FixedAscii(4).shared_field().unwrap(),
        DataType::FixedAscii(8).shared_field().unwrap()
    ));
    assert!(!std::ptr::eq(
        cases[6].shared_field().unwrap(),
        cases[7].shared_field().unwrap()
    ));
}

#[test]
fn an_unbounded_or_invalid_datatype_keeps_no_shared_field() {
    let item = Field::new("item", DataType::Int64, true);
    for dtype in [
        DataType::list(item.clone()),
        DataType::large_list(item.clone()),
        DataType::from_fields([item.clone()]).unwrap(),
        DataType::map(
            Field::new(
                "entries",
                DataType::from_fields([
                    Field::new("key", DataType::Utf8, false),
                    Field::new("value", DataType::Int64, true),
                ])
                .unwrap(),
                false,
            ),
            false,
        )
        .unwrap(),
        DataType::geometry(None).unwrap(),
        DataType::geography(None, None).unwrap(),
    ] {
        assert!(dtype.shared_field().is_none(), "{dtype}");
    }
    // A parameter the datatype refuses never earns a permanent field.
    assert!(DataType::FixedSizeBinary(-1).shared_field().is_none());
    assert!(
        DataType::Decimal32 {
            precision: 99,
            scale: 0
        }
        .shared_field()
        .is_none()
    );
    assert!(
        DataType::Time32(TimeUnit::Nanosecond)
            .shared_field()
            .is_none()
    );
}

#[test]
fn a_value_names_the_shared_field_of_its_own_datatype() {
    assert_eq!(
        Scalar::from(7_i64).shared_field().map(Field::dtype),
        Some(&DataType::Int64)
    );
    assert_eq!(
        Scalar::d128(150, 2)
            .shared_field()
            .map(|field| field.dtype().id()),
        Some(DataTypeId::Decimal128)
    );
    assert_eq!(
        Scalar::Null.shared_field().map(Field::dtype),
        Some(&DataType::Null)
    );
    assert!(
        Scalar::from_sequence([Scalar::from(1_i64)])
            .shared_field()
            .is_none()
    );
    let mixed = Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]);
    assert!(mixed.shared_field().is_none());
}
