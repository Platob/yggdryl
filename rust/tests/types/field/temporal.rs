//! The five temporal families' fields: one marker per family, over every
//! leaf and every unit the family carries.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Date32Array, Date64Array};
use yggdryl::types::{
    DateField, DateTimeField, DateTimeType, DateType, DurationField, DurationType, IntervalField,
    IntervalType, TimeField, TimeType,
};
use yggdryl::{ArrowCastOptions, DataType, DataTypeId, Scalar, TimeUnit, Timezone};

use super::typed::assert_typed_marker;

#[test]
fn every_temporal_marker_covers_every_leaf_of_its_family() {
    assert_typed_marker::<DateTimeType>(
        DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC).unwrap(),
    );
    assert_typed_marker::<DateTimeType>(
        DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
    );
    assert_typed_marker::<DateType>(DataType::date32());
    assert_typed_marker::<DateType>(DataType::date64());
    assert_typed_marker::<TimeType>(DataType::time32(TimeUnit::Second).unwrap());
    assert_typed_marker::<TimeType>(DataType::time64(TimeUnit::Microsecond).unwrap());
    assert_typed_marker::<DurationType>(DataType::duration32(TimeUnit::Millisecond).unwrap());
    assert_typed_marker::<DurationType>(DataType::duration64(TimeUnit::Millisecond).unwrap());
    assert_typed_marker::<IntervalType>(DataType::interval(TimeUnit::MonthDayNano).unwrap());
}

#[test]
fn a_date_field_holds_its_own_leaf_and_the_root_redirects_to_it() {
    let field = DateField::try_new("settlement", DataType::date64(), false).unwrap();
    assert_eq!(*field.typed_dtype_ref(), DateType::Date64);
    assert_eq!(field.typed_dtype_ref().unit(), TimeUnit::Millisecond);
    assert_eq!(field.dtype(), &DataType::date64());
    assert_eq!(field.id(), DataTypeId::Date64);

    // A datatype from another family is refused by name.
    let refused = DateField::try_new(
        "settlement",
        DataType::time32(TimeUnit::Second).unwrap(),
        false,
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("date"), "{refused}");

    // The value door is the family's: text reads as the leaf the column
    // declares, at the unit that width means.
    let held = field.to_field().scalar(Scalar::from("2024-01-02")).unwrap();
    assert_eq!(held.id(), DataTypeId::Date64);
    assert_eq!(
        held.as_date64().map(|(count, ..)| count),
        Some(19_724 * 86_400_000)
    );
    let narrow = DateField::try_new("settlement", DataType::date32(), false).unwrap();
    let held = narrow
        .to_field()
        .scalar(Scalar::from("2024-01-02"))
        .unwrap();
    assert_eq!(held.as_date32().map(|(count, ..)| count), Some(19_724));
}

#[test]
fn a_date_fields_array_is_the_width_its_leaf_declares() {
    // The family has no single array type: a `date32` column casts to a
    // `Date32Array` and a `date64` one to a `Date64Array`, so the typed
    // cast answers `ArrayRef` and the leaf says which to narrow to.
    let days: ArrayRef = Arc::new(Date32Array::from(vec![Some(19_724), None]));
    let narrow = DateField::try_new("day", DataType::date32(), true).unwrap();
    let cast = narrow
        .cast_arrow_array(days, ArrowCastOptions::new().with_safe(false))
        .unwrap();
    let cast = cast
        .as_any()
        .downcast_ref::<Date32Array>()
        .expect("a date32 column is a Date32Array");
    assert_eq!(cast.value(0), 19_724);
    assert!(cast.is_null(1));

    let millis: ArrayRef = Arc::new(Date64Array::from(vec![Some(19_724 * 86_400_000), None]));
    let wide = DateField::try_new("day", DataType::date64(), true).unwrap();
    let cast = wide
        .cast_arrow_array(millis, ArrowCastOptions::new().with_safe(false))
        .unwrap();
    let cast = cast
        .as_any()
        .downcast_ref::<Date64Array>()
        .expect("a date64 column is a Date64Array");
    assert_eq!(cast.value(0), 19_724 * 86_400_000);
    assert!(cast.is_null(1));
}

#[test]
fn a_datetime_field_reads_its_zone_and_resolution_off_its_leaf() {
    let dtype = DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC).unwrap();
    let field = DateTimeField::try_new("transact", dtype.clone(), false).unwrap();
    assert_eq!(field.typed_dtype_ref().unit(), TimeUnit::Microsecond);
    assert!(field.typed_dtype_ref().timezone().is_utc());
    assert_eq!(field.dtype(), &dtype);
    assert_eq!(field.id(), DataTypeId::DateTime64);
    assert!(DateTimeField::try_new("transact", DataType::date32(), false).is_err());

    // A zoned column reads a zoned instant at its own resolution.
    let held = field
        .to_field()
        .scalar(Scalar::from("2024-01-02T10:15:30Z"))
        .unwrap();
    assert_eq!(held.id(), DataTypeId::DateTime64);
    assert_eq!(
        held.as_datetime64().map(|(count, unit, _)| (count, unit)),
        Some((1_704_190_530_000_000, TimeUnit::Microsecond))
    );
    assert_eq!(held.temporal_timezone(), Some(Timezone::UTC));
}

#[test]
fn a_time_field_holds_the_width_its_resolution_fits_in() {
    let field =
        TimeField::try_new("open", DataType::time32(TimeUnit::Second).unwrap(), false).unwrap();
    assert_eq!(*field.typed_dtype_ref(), TimeType::Time32(TimeUnit::Second));
    assert_eq!(field.typed_dtype_ref().bit_width(), 32);
    assert_eq!(field.dtype(), &DataType::time32(TimeUnit::Second).unwrap());
    assert_eq!(field.id(), DataTypeId::Time32);
    assert!(
        TimeField::try_new(
            "open",
            DataType::duration32(TimeUnit::Second).unwrap(),
            false
        )
        .is_err()
    );

    let held = field.to_field().scalar(Scalar::from("09:30:00")).unwrap();
    assert_eq!(held.id(), DataTypeId::Time32);
    assert_eq!(held.as_time32().map(|(count, ..)| count), Some(34_200));
}

#[test]
fn a_duration_field_holds_a_fixed_length_unit_at_its_width() {
    let field = DurationField::try_new(
        "elapsed",
        DataType::duration64(TimeUnit::Millisecond).unwrap(),
        false,
    )
    .unwrap();
    assert_eq!(
        *field.typed_dtype_ref(),
        DurationType::Duration64(TimeUnit::Millisecond)
    );
    assert_eq!(field.typed_dtype_ref().unit(), TimeUnit::Millisecond);
    assert_eq!(
        field.dtype(),
        &DataType::duration64(TimeUnit::Millisecond).unwrap()
    );
    assert_eq!(field.id(), DataTypeId::Duration64);
    assert!(
        DurationField::try_new(
            "elapsed",
            DataType::time32(TimeUnit::Second).unwrap(),
            false
        )
        .is_err()
    );

    let held = field.to_field().scalar(Scalar::from("PT90S")).unwrap();
    assert_eq!(held.id(), DataTypeId::Duration64);
    assert_eq!(held.as_duration64().map(|(count, ..)| count), Some(90_000));
}

#[test]
fn an_interval_field_holds_its_layout() {
    let field = IntervalField::try_new(
        "tenor",
        DataType::interval(TimeUnit::MonthDayNano).unwrap(),
        false,
    )
    .unwrap();
    assert_eq!(
        *field.typed_dtype_ref(),
        IntervalType::Interval(TimeUnit::MonthDayNano)
    );
    assert_eq!(field.typed_dtype_ref().unit(), TimeUnit::MonthDayNano);
    assert_eq!(
        field.dtype(),
        &DataType::interval(TimeUnit::MonthDayNano).unwrap()
    );
    assert_eq!(field.id(), DataTypeId::Interval);
    assert!(
        IntervalField::try_new(
            "tenor",
            DataType::duration64(TimeUnit::Millisecond).unwrap(),
            false
        )
        .is_err()
    );

    // A value already in the column's layout comes back untouched.
    let span = Scalar::interval(1, 2, 3, TimeUnit::MonthDayNano).unwrap();
    let held = field.to_field().scalar(span.clone()).unwrap();
    assert_eq!(held, span);
    assert_eq!(held.id(), DataTypeId::Interval);
}
