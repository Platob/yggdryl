//! `rust/src/serie/primitive.rs`: the fixed-width leaves - what they lend
//! off the values buffer, what a typed writer validates, and how a write
//! lands in the buffer the column holds.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int32Array, Int64Array};
use yggdryl::{
    ArrowCastOptions, DataType, Decimal128, Field, Int32Serie, Scalar, Serie, SerieValue,
};

/// A nullable int64 column of six rows, two of them absent, straight off
/// Arrow buffers.
fn counts() -> Serie {
    let array: ArrayRef = Arc::new(Int64Array::from(vec![
        Some(1),
        None,
        Some(3),
        Some(4),
        None,
        Some(6),
    ]));
    Serie::from_arrow_array(
        Some(&Field::new("count", DataType::Int64, true)),
        array,
        ArrowCastOptions::new(),
    )
    .expect("an int64 column")
}

/// The hash a value writes into the default hasher.
fn hashed(value: &impl Hash) -> u64 {
    let mut state = DefaultHasher::new();
    value.hash(&mut state);
    state.finish()
}

#[test]
fn a_typed_writer_refuses_an_absent_row_under_a_required_field_and_leaves_the_column_unchanged() {
    let mut column = Serie::from_scalars(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(125_i64), Scalar::from(126_i64)],
    )
    .expect("two int64 rows");
    let leaf = column.get_int64_mut().expect("an int64 column");

    let refusal = leaf
        .push_value(None)
        .expect_err("a required column admits no absent row");
    assert!(
        refusal.to_string().contains("price"),
        "the refusal names the column: {refusal}"
    );
    assert_eq!(leaf.values(), &[125, 126]);
    assert!(leaf.nulls().is_none());

    assert!(leaf.set_value(0, None).is_err());
    assert!(leaf.splice_values(1..1, vec![Some(1), None]).is_err());
    assert_eq!(leaf.values(), &[125, 126]);

    // And a present value lands in the buffer, no value built.
    leaf.push_value(Some(127)).expect("a present row");
    leaf.set_value(0, Some(124)).expect("a present row");
    leaf.extend_values(vec![Some(128)]).expect("present rows");
    assert_eq!(leaf.values(), &[124, 126, 127, 128]);
}

#[test]
fn a_row_past_the_end_is_refused_naming_the_column_and_both_counts() {
    let column = counts();
    let leaf = column.as_int64().expect("an int64 column");

    let refusal = leaf.scalar(6).expect_err("row 6 is past the end");
    let text = refusal.to_string();
    assert!(text.contains("count"), "names the column: {text}");
    assert!(text.contains('6'), "names the row: {text}");
    assert!(leaf.is_null(6).is_err());
    assert_eq!(leaf.value(6), None);
    assert!(leaf.slice(4, 3).is_err());
    assert!(leaf.slice(usize::MAX, 2).is_err());
}

#[test]
fn the_typed_readers_lend_the_buffers_and_read_absence_off_the_bitmap() {
    let column = counts();
    let leaf = column.as_int64().expect("an int64 column");

    assert_eq!(leaf.values().len(), 6);
    assert_eq!(leaf.value(0), Some(1));
    assert_eq!(leaf.value(1), None);
    assert_eq!(leaf.nulls().map(|nulls| nulls.null_count()), Some(2));
    assert_eq!(leaf.array().len(), 6);
    assert_eq!(SerieValue::len(leaf), 6);
    assert_eq!(leaf.null_count(), 2);
    assert!(leaf.is_null(1).unwrap());
    assert!(!leaf.is_null(2).unwrap());
    assert_eq!(leaf.scalar(1).unwrap(), Scalar::Null);
    assert_eq!(leaf.scalar(2).unwrap(), Scalar::from(3_i64));
    assert_eq!(SerieValue::field(leaf).name(), "count");
    assert_eq!(leaf.field_ref().name(), "count");
    assert!(column.as_int32().is_none());
}

#[test]
fn a_slice_with_its_original_dropped_grows_into_a_freshly_built_array() {
    let whole = counts();
    let mut window = whole.slice(3, 3).expect("rows 3..6");
    drop(whole);

    window.push(Scalar::from(7_i64)).expect("a present row");
    window.push(Scalar::Null).expect("an absent row");

    let expected: ArrayRef = Arc::new(Int64Array::from(vec![
        Some(4),
        None,
        Some(6),
        Some(7),
        None,
    ]));
    let held = window.into_arrow_array().expect("a column");
    assert_eq!(held.as_ref(), expected.as_ref());
    assert_eq!(window.len(), 5);
    assert_eq!(window.null_count(), 2);
}

#[test]
fn a_set_in_the_middle_rewrites_one_slot_and_a_validity_change_rebuilds_the_bitmap() {
    let mut column = counts();

    column.set(2, Scalar::from(30_i64)).expect("one slot");
    assert_eq!(
        column.as_int64().expect("an int64 column").value(2),
        Some(30)
    );
    assert_eq!(column.null_count(), 2);

    column.set(1, Scalar::from(20_i64)).expect("a filled slot");
    column.set(0, Scalar::Null).expect("an emptied slot");
    assert_eq!(column.null_count(), 2);
    assert!(column.is_null(0).unwrap());
    assert_eq!(column.scalar(1).unwrap(), Scalar::from(20_i64));

    // A refusal leaves the column exactly as it was.
    assert!(column.set(3, Scalar::from("AAPL")).is_err());
    assert_eq!(column.scalar(3).unwrap(), Scalar::from(4_i64));
}

#[test]
fn every_edit_keeps_the_fields_own_arrow_datatype() {
    let field = Field::new("amount", DataType::decimal128(10, 2).unwrap(), false);
    let projected = field.as_arrow_field_ref().unwrap().data_type().clone();
    let mut column =
        Serie::from_scalars(field, [Scalar::Decimal128(Decimal128::new(12_500, 2))]).unwrap();

    column
        .push(Scalar::Decimal128(Decimal128::new(13_000, 2)))
        .expect("a second row");
    column
        .set(0, Scalar::Decimal128(Decimal128::new(12_000, 2)))
        .expect("a slot");
    column
        .insert(0, Scalar::Decimal128(Decimal128::new(11_000, 2)))
        .expect("a row in front");

    let held = column.into_arrow_array().expect("a column");
    assert_eq!(held.data_type(), &projected);
    assert_eq!(
        column
            .as_decimal128()
            .expect("a decimal128 column")
            .values(),
        &[11_000, 12_000, 13_000]
    );
}

#[test]
fn two_columns_of_equal_rows_are_one_value_whichever_field_types_them() {
    let a: ArrayRef = Arc::new(Int32Array::from(vec![1, 2, 3]));
    let b: ArrayRef = Arc::new(Int32Array::from(vec![1, 2, 3]));
    let under_a = Serie::from_arrow_array(
        Some(&Field::new("a", DataType::Int32, false)),
        a,
        ArrowCastOptions::new(),
    )
    .unwrap();
    let under_b = Serie::from_arrow_array(
        Some(&Field::new("b", DataType::Int32, true)),
        b,
        ArrowCastOptions::new(),
    )
    .unwrap();
    let (left, right) = (
        under_a.as_int32().expect("int32"),
        under_b.as_int32().expect("int32"),
    );

    assert_eq!(left, right);
    assert_eq!(hashed(left), hashed(right));
    assert_eq!(under_a, under_b);
    assert_eq!(hashed(&under_a), hashed(&under_b));

    // The run of the same rows is the same value again, and hashes alike.
    let run = Serie::new(vec![
        Scalar::from(1_i32),
        Scalar::from(2_i32),
        Scalar::from(3_i32),
    ]);
    assert_eq!(under_a, run);
    assert_eq!(hashed(&under_a), hashed(&run));
}

#[test]
fn a_sorted_vec_of_leaves_and_the_sorted_vec_of_series_holding_them_agree() {
    let columns: Vec<Serie> = [vec![3, 1], vec![1, 2, 3], vec![1, 2]]
        .into_iter()
        .map(|values| {
            let array: ArrayRef = Arc::new(Int32Array::from(values));
            Serie::from_arrow_array(
                Some(&Field::new("n", DataType::Int32, false)),
                array,
                ArrowCastOptions::new(),
            )
            .unwrap()
        })
        .collect();
    let mut leaves: Vec<Int32Serie> = columns
        .iter()
        .map(|column| column.as_int32().expect("int32").clone())
        .collect();
    let mut roots = columns.clone();

    leaves.sort();
    roots.sort();

    let by_leaf: Vec<Vec<i32>> = leaves.iter().map(|leaf| leaf.values().to_vec()).collect();
    let by_root: Vec<Vec<i32>> = roots
        .iter()
        .map(|root| root.as_int32().expect("int32").values().to_vec())
        .collect();
    assert_eq!(by_leaf, by_root);
    assert_eq!(by_leaf, vec![vec![1, 2], vec![1, 2, 3], vec![3, 1]]);
}

#[test]
fn a_column_built_by_pushes_is_the_from_scalars_column_buffer_for_buffer() {
    let field = Field::new("count", DataType::Int64, true);
    let rows = vec![Scalar::from(1_i64), Scalar::Null, Scalar::from(3_i64)];

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

#[test]
fn the_general_splice_rebuilds_from_prefix_replacement_and_suffix() {
    let mut column = counts();

    column
        .splice(1..4, vec![Scalar::from(10_i64), Scalar::Null])
        .expect("three rows replaced by two");
    assert_eq!(
        column.rows().as_ref(),
        &[
            Scalar::from(1_i64),
            Scalar::from(10_i64),
            Scalar::Null,
            Scalar::Null,
            Scalar::from(6_i64),
        ]
    );

    // A reversed range and one past the end are refused by name.
    let backwards = (3, 2);
    assert!(column.splice(backwards.0..backwards.1, vec![]).is_err());
    assert!(column.splice(4..9, vec![]).is_err());
    assert_eq!(column.len(), 5);
}
