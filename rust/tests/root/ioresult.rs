//! `rust/src/ioresult.rs`.

use yggdryl::IOResult;

#[test]
fn what_was_read_and_not_written_is_skipped() {
    let result = IOResult::new(10, 8);

    assert_eq!(result.read_rows, 10);
    assert_eq!(result.written_rows, 8);
    assert_eq!(result.skipped_rows, 2);
    assert!(!result.is_empty());
}

#[test]
fn a_write_of_more_rows_than_it_read_skips_none() {
    // A `select` that unnests writes more rows than its source held, and a
    // skipped count never goes below zero.
    let result = IOResult::new(2, 6);

    assert_eq!(result.skipped_rows, 0);
}

#[test]
fn the_default_is_the_write_of_an_empty_source() {
    let result = IOResult::default();

    assert_eq!(result, IOResult::new(0, 0));
    assert!(result.is_empty());
    // A source whose every row was kept out was read: it is not empty.
    assert!(!IOResult::new(3, 0).is_empty());
}

#[test]
fn results_add_count_by_count_and_saturate() {
    let mut total = IOResult::new(10, 8) + IOResult::new(5, 5);
    assert_eq!(total, IOResult::new(15, 13));

    total += IOResult::new(1, 0);
    assert_eq!(
        (total.read_rows, total.written_rows, total.skipped_rows),
        (16, 13, 3)
    );

    let summed: IOResult = [
        IOResult::new(1, 1),
        IOResult::new(2, 1),
        IOResult::new(4, 4),
    ]
    .into_iter()
    .sum();
    assert_eq!(summed, IOResult::new(7, 6));
    assert_eq!(
        std::iter::empty::<IOResult>().sum::<IOResult>(),
        IOResult::default()
    );

    let full = IOResult::new(u64::MAX, u64::MAX) + IOResult::new(1, 1);
    assert_eq!(full.read_rows, u64::MAX);
    assert_eq!(full.written_rows, u64::MAX);
}

#[test]
fn a_result_displays_orders_hashes_and_serializes_as_its_three_counts() {
    let result = IOResult::new(10, 8);

    assert_eq!(result.to_string(), "read 10 rows, wrote 8, skipped 2");
    assert_eq!(
        serde_json::to_string(&result).unwrap(),
        r#"{"read_rows":10,"written_rows":8,"skipped_rows":2}"#
    );
    assert_eq!(
        serde_json::from_str::<IOResult>(&serde_json::to_string(&result).unwrap()).unwrap(),
        result
    );

    assert!(IOResult::new(1, 1) < IOResult::new(2, 0));
    assert_eq!(result.stable_hash(), IOResult::new(10, 8).stable_hash());
    assert_ne!(result.stable_hash(), IOResult::new(10, 9).stable_hash());
}
