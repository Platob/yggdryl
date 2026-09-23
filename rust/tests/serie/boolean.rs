//! `rust/src/serie/boolean.rs`: the boolean leaf - two bitmaps, lent as
//! they are, and written a bit at a time.

use std::sync::Arc;

use arrow_array::{ArrayRef, BooleanArray};
use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie, SerieValue};

/// A nullable boolean column of five rows, one of them absent.
fn flags() -> Serie {
    let array: ArrayRef = Arc::new(BooleanArray::from(vec![
        Some(true),
        None,
        Some(false),
        Some(true),
        Some(false),
    ]));
    Serie::from_arrow_array(
        Some(&Field::new("flag", DataType::Boolean, true)),
        array,
        ArrowCastOptions::new(),
    )
    .expect("a boolean column")
}

#[test]
fn a_typed_writer_refuses_an_absent_row_under_a_required_field_and_leaves_the_column_unchanged() {
    let mut column = Serie::from_scalars(
        Field::new("flag", DataType::Boolean, false),
        [Scalar::from(true), Scalar::from(false)],
    )
    .expect("two boolean rows");
    let leaf = column.get_boolean_mut().expect("a boolean column");

    let refusal = leaf
        .push_value(None)
        .expect_err("a required column admits no absent row");
    assert!(
        refusal.to_string().contains("flag"),
        "the refusal names the column: {refusal}"
    );
    assert!(leaf.set_value(1, None).is_err());
    assert_eq!(
        leaf.values().iter().collect::<Vec<bool>>(),
        vec![true, false]
    );
    assert!(leaf.nulls().is_none());

    leaf.push_value(Some(true)).expect("a present row");
    leaf.set_value(1, Some(true)).expect("one bit");
    assert_eq!(
        leaf.values().iter().collect::<Vec<bool>>(),
        vec![true, true, true]
    );
}

#[test]
fn the_typed_readers_lend_the_bitmaps() {
    let column = flags();
    let leaf = column.as_boolean().expect("a boolean column");

    assert_eq!(leaf.value(0), Some(true));
    assert_eq!(leaf.value(1), None);
    assert_eq!(leaf.value(9), None);
    assert_eq!(leaf.values().len(), 5);
    assert_eq!(leaf.nulls().map(|nulls| nulls.null_count()), Some(1));
    assert_eq!(leaf.array().len(), 5);
    assert_eq!(leaf.scalar(2).unwrap(), Scalar::from(false));
    assert_eq!(leaf.scalar(1).unwrap(), Scalar::Null);
    assert!(leaf.scalar(5).is_err());
    assert_eq!(leaf.null_count(), 1);
}

#[test]
fn a_slice_with_its_original_dropped_grows_into_a_freshly_built_array() {
    let whole = flags();
    let mut window = whole.slice(3, 2).expect("rows 3..5");
    drop(whole);

    window.push(Scalar::from(true)).expect("a present row");
    window.push(Scalar::Null).expect("an absent row");

    let expected: ArrayRef = Arc::new(BooleanArray::from(vec![
        Some(true),
        Some(false),
        Some(true),
        None,
    ]));
    assert_eq!(
        window.into_arrow_array().expect("a column").as_ref(),
        expected.as_ref()
    );
}

#[test]
fn a_set_flips_one_bit_and_a_validity_change_rebuilds_the_bitmap() {
    let mut column = flags();

    column.set(2, Scalar::from(true)).expect("one bit");
    column.set(1, Scalar::from(false)).expect("a filled slot");
    column.set(0, Scalar::Null).expect("an emptied slot");

    assert_eq!(
        column.rows().as_ref(),
        &[
            Scalar::Null,
            Scalar::from(false),
            Scalar::from(true),
            Scalar::from(true),
            Scalar::from(false),
        ]
    );
    assert_eq!(column.null_count(), 1);

    // A refusal leaves the column exactly as it was.
    assert!(column.set(3, Scalar::from(1_i64)).is_err());
    assert_eq!(column.scalar(3).unwrap(), Scalar::from(true));
}

#[test]
fn a_column_built_by_pushes_is_the_from_scalars_column_buffer_for_buffer() {
    let field = Field::new("flag", DataType::Boolean, true);
    let rows = vec![Scalar::from(true), Scalar::Null, Scalar::from(false)];

    let laid_out = Serie::from_scalars(field.clone(), rows.clone()).unwrap();
    let mut pushed = Serie::empty(field).unwrap();
    for row in rows {
        pushed.push(row).expect("a row the field accepts");
    }

    assert_eq!(
        pushed.into_arrow_array().unwrap().as_ref(),
        laid_out.into_arrow_array().unwrap().as_ref()
    );
    assert_eq!(pushed, laid_out);
}
