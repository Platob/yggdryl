use super::{Differences, OwnedDifferences};
use crate::{DataType, Field};

fn wide_struct(prefix: &str) -> DataType {
    DataType::from_fields(
        (0..1_024).map(|index| Field::new(format!("{prefix}_{index:04}"), DataType::Int64, false)),
    )
    .unwrap()
}

#[test]
fn wide_slice_work_stays_bounded_before_the_first_difference() {
    let left = Field::new("root", wide_struct("left"), false);
    let right = Field::new("root", wide_struct("right"), false);
    let mut differences = Differences::from_fields(&left, &right, true, false);
    assert_eq!(differences.engine.work.len(), 1);
    assert!(differences.engine.pending.is_empty());

    assert_eq!(
        differences.next().as_deref(),
        Some("≠ $.dtype.fields[0].name: \"left_0000\" → \"right_0000\"")
    );
    assert!(differences.engine.work.len() <= 2);
    assert!(differences.engine.pending.is_empty());

    let left = Field::new(
        "root",
        DataType::from_fields(std::iter::empty()).unwrap(),
        false,
    );
    let right = Field::new("root", wide_struct("added"), false);
    let mut differences = Differences::from_fields(&left, &right, true, false);
    assert_eq!(
        differences.next().as_deref(),
        Some("≠ $.dtype.field_count: 0 → 1024")
    );
    assert_eq!(differences.engine.work.len(), 1);
    assert!(differences.engine.pending.is_empty());
}

#[test]
fn physical_first_difference_does_not_scan_equal_wide_metadata() {
    let entries = (0..1_024)
        .map(|index| (format!("key_{index:04}"), format!("value_{index:04}")))
        .collect::<Vec<_>>();
    let left = Field::from_parts("root", DataType::Int32, false, entries.clone()).unwrap();
    let right = Field::from_parts("root", DataType::Int64, false, entries).unwrap();
    assert!(!left.metadata.shares_storage_with(&right.metadata));

    let mut differences = Differences::from_fields(&left, &right, true, false);
    assert_eq!(differences.engine.work.len(), 1);
    assert_eq!(
        differences.next().as_deref(),
        Some("≠ $.dtype.kind: int32 → int64")
    );
    assert_eq!(differences.engine.work.len(), 1);
    assert!(differences.engine.pending.is_empty());
}

#[test]
fn shared_deep_snapshots_complete_without_traversal() {
    let mut dtype = DataType::Int64;
    for depth in 0..64 {
        dtype = DataType::list(Field::new(format!("item_{depth}"), dtype, false));
    }
    let left = Field::new("root", dtype, false);
    let right = left.clone();
    let mut differences = Differences::from_fields(&left, &right, true, false);
    assert!(differences.engine.work.is_empty());
    assert_eq!(differences.next(), None);
}

#[test]
fn owned_cursor_outlives_source_snapshots() {
    let mut differences = {
        let left = Field::new("left", wide_struct("left"), false);
        let right = Field::new("right", wide_struct("right"), false);
        OwnedDifferences::from_fields(&left, &right, true, false)
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
