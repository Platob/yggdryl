//! `rust/src/diff.rs`: the borrowed field-pair walk no caller can reach.
//!
//! The field-pair walk an integration test cannot reach.
//!
//! `Differences::from_fields` is crate-private: it is the borrowed walk
//! [`OwnedDifferences::from_fields`] owns its answers from, so the wide
//! cases have to be measured from inside. What a caller can observe lives
//! in `tests/types/field/comparison.rs`.

use yggdryl::internals::diff::{
    differences_from_fields, owned_differences_from_fields, pending_is_empty, work_len,
};
use yggdryl::internals::metadata::shares_storage_with;
use yggdryl::{DataType, Field, StructType};

fn wide_struct(prefix: &str) -> DataType {
    StructType::from_fields(
        (0..1_024).map(|index| Field::new(format!("{prefix}_{index:04}"), DataType::Int64, false)),
    )
    .map(DataType::from)
    .unwrap()
}

#[test]
fn wide_slice_work_stays_bounded_before_the_first_difference() {
    let left = Field::new("root", wide_struct("left"), false);
    let right = Field::new("root", wide_struct("right"), false);
    let mut differences = differences_from_fields(&left, &right, true, false);
    assert_eq!(work_len(&differences), 1);
    assert!(pending_is_empty(&differences));

    assert_eq!(
        differences.next().as_deref(),
        Some("≠ $.dtype.fields[0].name: \"left_0000\" → \"right_0000\"")
    );
    assert!(work_len(&differences) <= 2);
    assert!(pending_is_empty(&differences));

    let left = Field::new(
        "root",
        DataType::from(StructType::from_fields(std::iter::empty()).unwrap()),
        false,
    );
    let right = Field::new("root", wide_struct("added"), false);
    let mut differences = differences_from_fields(&left, &right, true, false);
    assert_eq!(
        differences.next().as_deref(),
        Some("≠ $.dtype.field_count: 0 → 1024")
    );
    assert_eq!(work_len(&differences), 1);
    assert!(pending_is_empty(&differences));
}

#[test]
fn physical_first_difference_does_not_scan_equal_wide_metadata() {
    let entries = (0..1_024)
        .map(|index| (format!("key_{index:04}"), format!("value_{index:04}")))
        .collect::<Vec<_>>();
    let left = Field::from_parts("root", DataType::Int32, false, entries.clone()).unwrap();
    let right = Field::from_parts("root", DataType::Int64, false, entries).unwrap();
    assert!(!shares_storage_with(
        left.as_metadata(),
        right.as_metadata()
    ));

    let mut differences = differences_from_fields(&left, &right, true, false);
    assert_eq!(work_len(&differences), 1);
    assert_eq!(
        differences.next().as_deref(),
        Some("≠ $.dtype.kind: int32 → int64")
    );
    assert_eq!(work_len(&differences), 1);
    assert!(pending_is_empty(&differences));
}

#[test]
fn shared_deep_snapshots_complete_without_traversal() {
    let mut dtype = DataType::Int64;
    for depth in 0..64 {
        dtype = DataType::list(Field::new(format!("item_{depth}"), dtype, false));
    }
    let left = Field::new("root", dtype, false);
    let right = left.clone();
    let mut differences = differences_from_fields(&left, &right, true, false);
    assert!(work_len(&differences) == 0);
    assert_eq!(differences.next(), None);
}

#[test]
fn owned_cursor_outlives_source_snapshots() {
    let mut differences = {
        let left = Field::new("left", wide_struct("left"), false);
        let right = Field::new("right", wide_struct("right"), false);
        owned_differences_from_fields(&left, &right, true, false)
    };
    assert_eq!(
        differences.next().as_deref(),
        Some("≠ $.name: \"left\" → \"right\"")
    );
    assert_eq!(
        differences.next().as_deref(),
        Some("≠ $.dtype.fields[0].name: \"left_0000\" → \"right_0000\"")
    );
}
