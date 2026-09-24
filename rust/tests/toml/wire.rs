//! `rust/src/toml/wire.rs`: the natural TOML projection no caller can name.
//!
//! Which temporal values TOML has a native date-time for, and the calendar
//! arithmetic underneath them, are file-private steps of one public codec, so
//! they are reached through `yggdryl::internals`. Everything a caller can
//! observe is pinned through `yggdryl::` like any other test.

use yggdryl::internals::toml_wire::{
    civil_from_days, days_from_civil, native_datetime, split_count,
};
use yggdryl::{Scalar, TimeUnit, Timezone};

#[test]
fn calendar_conversion_is_exact_across_tomls_range() {
    let first = days_from_civil(0, 1, 1);
    let last = days_from_civil(9_999, 12, 31);
    assert_eq!(first, -719_528);
    assert_eq!(last, 2_932_896);
    for days in first..=last {
        let (year, month, day) = civil_from_days(days);
        assert_eq!(days_from_civil(year, month, day), days);
    }
}

#[test]
fn negative_counts_keep_a_positive_subsecond_fraction() {
    assert_eq!(
        split_count(-500, TimeUnit::Millisecond),
        Some((-1, 500_000_000))
    );
}

#[test]
fn temporal_values_use_native_toml_when_the_shape_fits() {
    let date = Scalar::date32(3_433);
    let time = Scalar::time64(27_120_123_456, TimeUnit::Microsecond, Timezone::NAIVE).unwrap();
    let datetime = Scalar::datetime64(296_638_320, TimeUnit::Second, Timezone::UTC).unwrap();

    assert_eq!(native_datetime(&date).unwrap().to_string(), "1979-05-27");
    assert_eq!(
        native_datetime(&time).unwrap().to_string(),
        "07:32:00.123456"
    );
    assert_eq!(
        native_datetime(&datetime).unwrap().to_string(),
        "1979-05-27T07:32:00Z"
    );
}

#[test]
fn named_zones_and_durations_have_no_native_toml_scalar() {
    let datetime =
        Scalar::datetime64(0, TimeUnit::Second, "Europe/Paris".parse().unwrap()).unwrap();
    let duration = Scalar::duration64(90, TimeUnit::Second).unwrap();
    assert!(native_datetime(&datetime).is_none());
    assert!(native_datetime(&duration).is_none());
}

/// A `serie<int64>` value held as a column, and the run of its rows.
fn int64s() -> (Scalar, Scalar) {
    let item = yggdryl::Field::new("item", yggdryl::DataType::Int64, false);
    let rows = [Scalar::from(1_i64), Scalar::from(2_i64)];
    let column = yggdryl::Serie::from_scalars(item, rows.clone()).unwrap();
    (Scalar::from(column), Scalar::from_sequence(rows))
}

#[test]
fn a_serie_column_writes_as_the_run_of_its_rows() {
    let (column, run) = int64s();
    let table = |xs: Scalar| Scalar::from_struct([("xs", xs)]).unwrap();
    for formatting in [
        yggdryl::text::Formatting::default(),
        yggdryl::text::Formatting::indented(2),
    ] {
        assert_eq!(
            yggdryl::toml::into_utf8_with_formatting(&table(column.clone()), formatting).unwrap(),
            yggdryl::toml::into_utf8_with_formatting(&table(run.clone()), formatting).unwrap()
        );
    }
    assert_eq!(
        yggdryl::toml::into_utf8(&table(column)).unwrap(),
        "\"xs\" = [1, 2]\n"
    );
}
