//! `rust/src/serie/layout.rs`: the buffer edits every leaf shares, reached
//! through `yggdryl::internals::serie_layout`.

use arrow_buffer::{BooleanBuffer, NullBuffer, OffsetBuffer, ScalarBuffer};
use yggdryl::internals::serie_layout::{
    require_offset, splice_bits, splice_nulls, splice_offsets, splice_scalars,
};

#[test]
fn a_validity_splice_answers_none_when_no_row_is_absent_afterwards() {
    assert!(splice_nulls(None, 3, 3..3, &[true, true]).is_none());

    let nulls = NullBuffer::from(vec![true, false, true]);
    assert!(splice_nulls(Some(nulls), 3, 1..2, &[true]).is_none());
}

#[test]
fn a_validity_splice_appends_rebuilds_and_fills_a_missing_bitmap() {
    let nulls = NullBuffer::from(vec![true, false, true]);
    let appended = splice_nulls(Some(nulls), 3, 3..3, &[false, true]).expect("absent rows");
    assert_eq!(
        appended.iter().collect::<Vec<bool>>(),
        vec![true, false, true, false, true]
    );

    let nulls = NullBuffer::from(vec![true, false, true, true]);
    let rebuilt = splice_nulls(Some(nulls), 4, 1..3, &[true, false, false]).expect("absent rows");
    assert_eq!(
        rebuilt.iter().collect::<Vec<bool>>(),
        vec![true, true, false, false, true]
    );

    let filled = splice_nulls(None, 2, 1..1, &[false]).expect("an absent row");
    assert_eq!(
        filled.iter().collect::<Vec<bool>>(),
        vec![true, false, true]
    );
}

#[test]
fn a_bit_splice_writes_one_bit_appends_and_rebuilds() {
    let bits = BooleanBuffer::from(vec![true, false, true]);
    let one = splice_bits(bits.clone(), 3, 1..2, &BooleanBuffer::from(vec![true]));
    assert_eq!(one.iter().collect::<Vec<bool>>(), vec![true, true, true]);

    let appended = splice_bits(
        bits.clone(),
        3,
        3..3,
        &BooleanBuffer::from(vec![false, true]),
    );
    assert_eq!(
        appended.iter().collect::<Vec<bool>>(),
        vec![true, false, true, false, true]
    );

    let rebuilt = splice_bits(bits, 3, 0..2, &BooleanBuffer::from(vec![false]));
    assert_eq!(rebuilt.iter().collect::<Vec<bool>>(), vec![false, true]);
}

#[test]
fn an_offsets_splice_recuts_the_rows_and_answers_the_items_they_occupied() {
    let offsets = OffsetBuffer::<i32>::new(ScalarBuffer::from(vec![0, 2, 5, 6]));

    let (recut, replaced) = splice_offsets(offsets.clone(), 1..2, &[1, 1]);
    assert_eq!(recut.as_ref(), &[0, 2, 3, 4, 5]);
    assert_eq!(replaced, 2..5);

    let (appended, replaced) = splice_offsets(offsets.clone(), 3..3, &[4]);
    assert_eq!(appended.as_ref(), &[0, 2, 5, 6, 10]);
    assert_eq!(replaced, 6..6);

    let (dropped, replaced) = splice_offsets(offsets, 0..3, &[]);
    assert_eq!(dropped.as_ref(), &[0]);
    assert_eq!(replaced, 0..6);
}

#[test]
fn an_item_total_past_the_offset_type_is_refused_naming_the_column() {
    assert!(require_offset::<i32>("items", 1 << 20).is_ok());
    let refusal = require_offset::<i32>("items", 1 << 40).expect_err("past i32");
    assert!(
        refusal.to_string().contains("items"),
        "the refusal names the column: {refusal}"
    );
    assert!(require_offset::<i64>("items", 1 << 40).is_ok());
}

#[test]
fn a_typed_splice_appends_and_overwrites_in_the_buffer_it_holds_and_rebuilds_otherwise() {
    let held = ScalarBuffer::<i8>::from(vec![1, 2, 3]);
    let appended = splice_scalars(held, 3..3, &[4, 5]);
    assert_eq!(appended.as_ref(), &[1, 2, 3, 4, 5]);

    let before = appended.as_ptr();
    let overwritten = splice_scalars(appended, 1..3, &[7, 8]);
    assert_eq!(overwritten.as_ref(), &[1, 7, 8, 4, 5]);
    assert_eq!(
        overwritten.as_ptr(),
        before,
        "as many slots as are written are written where they lie"
    );

    let shared = overwritten.clone();
    let copied = splice_scalars(overwritten, 5..5, &[6]);
    assert_eq!(copied.as_ref(), &[1, 7, 8, 4, 5, 6]);
    assert_eq!(
        shared.as_ref(),
        &[1, 7, 8, 4, 5],
        "a shared buffer is copied, never written"
    );

    let rebuilt = splice_scalars(shared, 0..2, &[9]);
    assert_eq!(rebuilt.as_ref(), &[9, 8, 4, 5]);

    let sliced = ScalarBuffer::<i32>::from(vec![1, 2, 3, 4]).slice(1, 2);
    assert_eq!(splice_scalars(sliced, 2..2, &[9]).as_ref(), &[2, 3, 9]);
}
