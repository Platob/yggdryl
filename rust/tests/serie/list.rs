//! `rust/src/serie/list.rs`: the list leaves - an item column under a
//! cut, the cut rebased onto exactly the items it reaches and re-cut by
//! every write.

use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, BooleanArray, FixedSizeListArray, Int64Array, LargeListArray, ListArray,
    ListViewArray, StructArray,
};
use arrow_buffer::{NullBuffer, OffsetBuffer, ScalarBuffer};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie, SerieValue, StructType};

/// The Arrow spelling of a required int64 item.
fn item() -> Arc<ArrowField> {
    Arc::new(ArrowField::new("item", ArrowDataType::Int64, false))
}

/// Three lists of int64 - `[1, 2]`, `[]`, `[3]` - and a fourth, absent.
fn legs() -> ListArray {
    ListArray::new(
        item(),
        OffsetBuffer::new(vec![0, 2, 2, 3, 3].into()),
        Arc::new(Int64Array::from(vec![1, 2, 3])),
        Some(vec![true, true, true, false].into()),
    )
}

/// The field [`legs`] lays out under.
fn legs_field() -> Field {
    Field::new(
        "legs",
        DataType::list(Field::new("item", DataType::Int64, false)),
        true,
    )
}

/// The rows of [`legs`].
fn leg_rows() -> Vec<Scalar> {
    vec![
        Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i64)]),
        Scalar::from_sequence([]),
        Scalar::from_sequence([Scalar::from(3_i64)]),
        Scalar::Null,
    ]
}

/// The column [`legs`] crosses into.
fn legs_column() -> Serie {
    Serie::from_arrow_array(
        Some(&legs_field()),
        Arc::new(legs()),
        ArrowCastOptions::new(),
    )
    .expect("a list column")
}

/// A nullable fixed-size list of two required int64 items.
fn pairs_field() -> Field {
    Field::new(
        "pair",
        DataType::fixed_size_list(Field::new("item", DataType::Int64, false), 2).unwrap(),
        true,
    )
}

#[test]
fn a_row_past_the_end_a_backwards_range_and_a_row_the_field_refuses_leave_the_cut() {
    let mut column = legs_column();

    let refusal = column.scalar(4).expect_err("row 4 is past the end");
    let text = refusal.to_string();
    assert!(text.contains("legs"), "names the column: {text}");
    assert!(text.contains('4'), "names the row: {text}");
    assert!(column.set(4, Scalar::from_sequence([])).is_err());
    let backwards = (3, 2);
    assert!(column.splice(backwards.0..backwards.1, vec![]).is_err());
    assert!(column.splice(2..5, vec![]).is_err());
    assert!(column.insert(5, Scalar::from_sequence([])).is_err());

    // Not a sequence, an item the item field refuses, an absent item under
    // a required item field: each refused before any buffer is touched.
    assert!(column.push(Scalar::from(1_i64)).is_err());
    assert!(
        column
            .push(Scalar::from_sequence([Scalar::from("one")]))
            .is_err()
    );
    assert!(
        column
            .push(Scalar::from_sequence([Scalar::from(1_i64), Scalar::Null]))
            .is_err()
    );
    let leaf = column.as_list().expect("a list column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 2, 2, 3, 3]);
    assert_eq!(leaf.items().len(), 3);
    assert_eq!(column.rows().into_owned(), leg_rows());

    // A fixed-size row of the wrong width is refused naming the width.
    let mut pairs = Serie::empty(pairs_field()).unwrap();
    let refusal = pairs
        .push(Scalar::from_sequence([Scalar::from(1_i64)]))
        .expect_err("one item is not two");
    assert!(refusal.to_string().contains('2'), "{refusal}");
    assert!(pairs.is_empty());
    assert_eq!(pairs.items().map(Serie::len), Some(0));
}

#[test]
fn a_list_row_set_with_a_different_item_count_recuts_the_offsets_and_later_rows_read_unchanged() {
    let mut column = legs_column();

    column
        .set(
            0,
            Scalar::from_sequence([
                Scalar::from(9_i64),
                Scalar::from(8_i64),
                Scalar::from(7_i64),
            ]),
        )
        .expect("two items become three");
    let leaf = column.as_list().expect("a list column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 3, 3, 4, 4]);
    assert_eq!(leaf.items().as_int64().unwrap().values(), &[9, 8, 7, 3]);
    assert_eq!(column.rows()[1..], leg_rows()[1..]);

    // Emptied, filled where it was absent, and absented where it held items.
    column.set(2, Scalar::from_sequence([])).unwrap();
    column
        .set(3, Scalar::from_sequence([Scalar::from(5_i64)]))
        .unwrap();
    column.set(1, Scalar::Null).unwrap();
    let leaf = column.as_list().expect("a list column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 3, 3, 3, 4]);
    assert_eq!(leaf.items().as_int64().unwrap().values(), &[9, 8, 7, 5]);
    assert_eq!(column.null_count(), 1);
    assert!(column.is_null(1).unwrap());
    assert_eq!(leaf.range(3), Some(3..4));

    let expected: ArrayRef = Arc::new(ListArray::new(
        item(),
        OffsetBuffer::new(vec![0, 3, 3, 3, 4].into()),
        Arc::new(Int64Array::from(vec![9, 8, 7, 5])),
        Some(vec![true, false, true, true].into()),
    ));
    assert_eq!(
        column.into_arrow_array().unwrap().as_ref(),
        expected.as_ref()
    );

    // A splice in the middle re-cuts from its start: one row becomes two.
    column
        .splice(
            1..2,
            vec![
                Scalar::from_sequence([Scalar::from(1_i64)]),
                Scalar::from_sequence([Scalar::from(2_i64), Scalar::from(3_i64)]),
            ],
        )
        .unwrap();
    let leaf = column.as_list().expect("a list column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 3, 4, 6, 6, 7]);
    assert_eq!(
        leaf.items().as_int64().unwrap().values(),
        &[9, 8, 7, 1, 2, 3, 5]
    );
    assert_eq!(column.null_count(), 0);
    assert_eq!(column.remove(0).unwrap().len(), 3);
    assert_eq!(
        column.as_list().unwrap().offsets().as_ref(),
        &[0, 1, 3, 3, 4]
    );
    column.truncate(2).unwrap();
    assert_eq!(column.as_list().unwrap().offsets().as_ref(), &[0, 1, 3]);
    assert_eq!(column.items().map(Serie::len), Some(3));
}

#[test]
fn a_column_built_by_pushes_is_the_from_scalars_column_and_the_array_built_from_the_rows() {
    let laid_out = Serie::from_scalars(legs_field(), leg_rows()).unwrap();
    let mut pushed = Serie::empty(legs_field()).unwrap();
    for row in leg_rows() {
        pushed.push(row).expect("a row the field accepts");
    }

    assert_eq!(pushed, laid_out);
    assert_eq!(pushed, legs_column());
    assert_eq!(pushed.into_arrow_array().unwrap().as_ref(), &legs());
    assert_eq!(
        laid_out.into_arrow_array().unwrap().as_ref(),
        pushed.into_arrow_array().unwrap().as_ref()
    );
    assert_eq!(pushed.null_count(), 1);
    assert_eq!(
        pushed.as_list().unwrap().offsets().as_ref(),
        &[0, 2, 2, 3, 3]
    );

    // A 64-bit cut grows the same way.
    let large = Field::new(
        "legs",
        DataType::large_list(Field::new("item", DataType::Int64, false)),
        true,
    );
    let mut pushed = Serie::empty(large.clone()).unwrap();
    pushed.extend(leg_rows()).unwrap();
    let expected: ArrayRef = Arc::new(LargeListArray::new(
        item(),
        OffsetBuffer::new(vec![0, 2, 2, 3, 3].into()),
        Arc::new(Int64Array::from(vec![1, 2, 3])),
        Some(vec![true, true, true, false].into()),
    ));
    assert_eq!(
        pushed.into_arrow_array().unwrap().as_ref(),
        expected.as_ref()
    );
    assert_eq!(pushed, Serie::from_scalars(large, leg_rows()).unwrap());
    assert_eq!(
        pushed.as_large_list().unwrap().offsets().as_ref(),
        &[0, 2, 2, 3, 3]
    );
    assert_eq!(pushed.as_large_list().unwrap().range(2), Some(2..3));
}

#[test]
fn a_fixed_size_list_null_row_holds_width_placeholder_items_and_a_cleared_bit() {
    let rows = vec![
        Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i64)]),
        Scalar::Null,
        Scalar::from_sequence([Scalar::from(3_i64), Scalar::from(4_i64)]),
    ];
    let laid_out = Serie::from_scalars(pairs_field(), rows.clone()).unwrap();
    let mut pushed = Serie::empty(pairs_field()).unwrap();
    for row in rows.clone() {
        pushed.push(row).expect("a row the field accepts");
    }

    let leaf = pushed
        .as_fixed_size_list()
        .expect("a fixed-size list column");
    assert_eq!(leaf.width(), 2);
    assert_eq!(SerieValue::len(leaf), 3);
    assert_eq!(leaf.items().len(), 6);
    assert_eq!(leaf.items().null_count(), 2);
    assert_eq!(leaf.null_count(), 1);
    assert_eq!(leaf.range(1), None);
    assert_eq!(leaf.range(2), Some(4..6));
    assert_eq!(leaf.scalar(1).unwrap(), Scalar::Null);
    assert_eq!(leaf.scalar(2).unwrap(), rows[2]);
    assert_eq!(
        leaf.row(0).map(|row| row.rows().into_owned()),
        Some(vec![Scalar::from(1_i64), Scalar::from(2_i64)])
    );
    assert_eq!(pushed, laid_out);

    let expected: ArrayRef = Arc::new(FixedSizeListArray::new(
        item(),
        2,
        Arc::new(Int64Array::from(vec![
            Some(1),
            Some(2),
            None,
            None,
            Some(3),
            Some(4),
        ])),
        Some(NullBuffer::from(vec![true, false, true])),
    ));
    assert_eq!(
        pushed.into_arrow_array().unwrap().as_ref(),
        expected.as_ref()
    );
    assert_eq!(
        laid_out.into_arrow_array().unwrap().as_ref(),
        expected.as_ref()
    );

    // A set moves exactly `width` items; a slice takes exactly `width` per
    // row; the array crosses back in as it went out.
    pushed
        .set(
            1,
            Scalar::from_sequence([Scalar::from(8_i64), Scalar::from(9_i64)]),
        )
        .unwrap();
    assert_eq!(
        pushed.items().and_then(Serie::as_int64).unwrap().values(),
        &[1, 2, 8, 9, 3, 4]
    );
    assert_eq!(pushed.null_count(), 0);
    let window = pushed.slice(1, 2).unwrap();
    assert_eq!(window.items().map(Serie::len), Some(4));
    assert_eq!(window.scalar(1).unwrap(), rows[2]);
    let back =
        Serie::from_arrow_array(Some(&pairs_field()), expected, ArrowCastOptions::new()).unwrap();
    assert_eq!(back, laid_out);
    assert_eq!(back.rows().into_owned(), rows);
}

#[test]
fn a_list_view_crosses_in_compact_and_every_write_keeps_it_so() {
    let field = Field::new(
        "legs",
        DataType::list_view(Field::new("item", DataType::Int64, false)),
        true,
    );
    // Views out of order, overlapping, and one absent: `[3]`, `[1, 2]`,
    // absent, `[2, 3]`.
    let views: ArrayRef = Arc::new(ListViewArray::new(
        item(),
        ScalarBuffer::from(vec![2, 0, 0, 1]),
        ScalarBuffer::from(vec![1, 2, 0, 2]),
        Arc::new(Int64Array::from(vec![1, 2, 3])),
        Some(NullBuffer::from(vec![true, true, false, true])),
    ));
    let mut column = Serie::from_arrow_array(Some(&field), views, ArrowCastOptions::new())
        .expect("a list-view column");
    let leaf = column.as_list_view().expect("a list-view column");

    // Rebased: contiguous from 0, an absent row viewing nothing, the items
    // exactly the ones the rows reach, in row order.
    assert_eq!(leaf.offsets().as_ref(), &[0, 1, 3, 3]);
    assert_eq!(leaf.sizes().as_ref(), &[1, 2, 0, 2]);
    assert_eq!(leaf.items().as_int64().unwrap().values(), &[3, 1, 2, 2, 3]);
    assert_eq!(leaf.range(1), Some(1..3));
    assert_eq!(leaf.range(2), None);
    assert_eq!(
        leaf.scalar(0).unwrap(),
        Scalar::from_sequence([Scalar::from(3_i64)])
    );
    assert_eq!(leaf.scalar(2).unwrap(), Scalar::Null);
    assert_eq!(
        leaf.scalar(3).unwrap(),
        Scalar::from_sequence([Scalar::from(2_i64), Scalar::from(3_i64)])
    );

    // A push appends into the cut, a set in the middle re-cuts from there,
    // and a slice rebases its window.
    column
        .push(Scalar::from_sequence([Scalar::from(4_i64)]))
        .unwrap();
    column
        .set(
            0,
            Scalar::from_sequence([Scalar::from(7_i64), Scalar::from(8_i64)]),
        )
        .unwrap();
    let leaf = column.as_list_view().expect("a list-view column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 2, 4, 4, 6]);
    assert_eq!(leaf.sizes().as_ref(), &[2, 2, 0, 2, 1]);
    assert_eq!(
        leaf.items().as_int64().unwrap().values(),
        &[7, 8, 1, 2, 2, 3, 4]
    );
    let window = column.slice(1, 2).unwrap();
    let window = window.as_list_view().unwrap();
    assert_eq!(window.offsets().as_ref(), &[0, 2]);
    assert_eq!(window.items().len(), 2);
    assert!(window.is_null(1).unwrap());

    // Out and back in: an already compact array is taken as it is.
    let out = column.into_arrow_array().unwrap();
    let back =
        Serie::from_arrow_array(Some(&field), Arc::clone(&out), ArrowCastOptions::new()).unwrap();
    assert_eq!(back, column);
    assert_eq!(back.into_arrow_array().unwrap().as_ref(), out.as_ref());

    // A sliced compact array is rebased without a gather: the items it
    // reaches, shifted to 0.
    let sliced =
        Serie::from_arrow_array(Some(&field), out.slice(3, 2), ArrowCastOptions::new()).unwrap();
    let leaf = sliced.as_list_view().unwrap();
    assert_eq!(leaf.offsets().as_ref(), &[0, 2]);
    assert_eq!(leaf.items().as_int64().unwrap().values(), &[2, 3, 4]);
    assert_eq!(
        sliced.rows().into_owned(),
        vec![
            Scalar::from_sequence([Scalar::from(2_i64), Scalar::from(3_i64)]),
            Scalar::from_sequence([Scalar::from(4_i64)]),
        ]
    );
}

#[test]
fn a_slice_with_its_original_dropped_grows_into_a_freshly_built_array() {
    let whole = legs_column();
    let mut window = whole.slice(2, 2).expect("rows 2..4");
    drop(whole);

    let leaf = window.as_list().unwrap();
    assert_eq!(leaf.offsets().as_ref(), &[0, 1, 1]);
    assert_eq!(leaf.items().len(), 1);
    window
        .push(Scalar::from_sequence([
            Scalar::from(4_i64),
            Scalar::from(5_i64),
        ]))
        .unwrap();
    let expected: ArrayRef = Arc::new(ListArray::new(
        item(),
        OffsetBuffer::new(vec![0, 1, 1, 3].into()),
        Arc::new(Int64Array::from(vec![3, 4, 5])),
        Some(vec![true, false, true].into()),
    ));
    assert_eq!(
        window.into_arrow_array().unwrap().as_ref(),
        expected.as_ref()
    );
    assert_eq!(window.null_count(), 1);
}

#[test]
fn extend_from_serie_appends_the_cut_and_the_items_and_a_list_of_records_pushes_down() {
    let mut column = legs_column();
    column
        .extend_from_serie(&legs_column())
        .expect("one layout appended");
    let leaf = column.as_list().unwrap();
    assert_eq!(leaf.offsets().as_ref(), &[0, 2, 2, 3, 3, 5, 5, 6, 6]);
    assert_eq!(
        leaf.items().as_int64().unwrap().values(),
        &[1, 2, 3, 1, 2, 3]
    );
    assert_eq!(column.null_count(), 2);
    let mut both = leg_rows();
    both.extend(leg_rows());
    assert_eq!(column.rows().into_owned(), both);

    // A run of the same rows appends through the field's contract.
    column
        .extend_from_serie(&Serie::new(vec![Scalar::from_sequence([Scalar::from(
            9_i64,
        )])]))
        .unwrap();
    assert_eq!(column.len(), 9);
    assert_eq!(column.items().map(Serie::len), Some(7));

    // Items that are records: the write descends into the record's children.
    let orders = Field::new(
        "orders",
        DataType::list(Field::new(
            "item",
            DataType::from(
                StructType::from_fields([
                    Field::new("id", DataType::Int64, false),
                    Field::new("live", DataType::Boolean, false),
                ])
                .unwrap(),
            ),
            true,
        )),
        false,
    );
    let mut nested = Serie::empty(orders).unwrap();
    nested
        .push(Scalar::from_sequence([
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(true)]),
            Scalar::Null,
        ]))
        .unwrap();
    nested.push(Scalar::from_sequence([])).unwrap();
    assert_eq!(nested.len(), 2);
    assert_eq!(
        nested
            .items()
            .and_then(|items| items.child("id"))
            .and_then(Serie::as_int64)
            .unwrap()
            .values(),
        &[1, 0]
    );
    assert_eq!(
        nested.scalar(0).unwrap(),
        Scalar::from_sequence([
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(true)]),
            Scalar::Null,
        ])
    );
    let records: ArrayRef = Arc::new(StructArray::new(
        vec![
            Arc::new(ArrowField::new("id", ArrowDataType::Int64, false)),
            Arc::new(ArrowField::new("live", ArrowDataType::Boolean, false)),
        ]
        .into(),
        vec![
            Arc::new(Int64Array::from(vec![Some(1), None])),
            Arc::new(BooleanArray::from(vec![Some(true), None])),
        ],
        Some(NullBuffer::from(vec![true, false])),
    ));
    let expected: ArrayRef = Arc::new(ListArray::new(
        Arc::new(ArrowField::new("item", records.data_type().clone(), true)),
        OffsetBuffer::new(vec![0, 2, 2].into()),
        records,
        None,
    ));
    assert_eq!(
        nested.into_arrow_array().unwrap().as_ref(),
        expected.as_ref()
    );
}

#[test]
fn a_list_column_cuts_one_item_column_and_reads_a_row_off_it() {
    let column = legs_column();
    let leaf = column.as_list().expect("a list column");

    assert_eq!(leaf.offsets().as_ref(), &[0, 2, 2, 3, 3]);
    assert_eq!(leaf.items().as_int64().unwrap().values(), &[1, 2, 3]);
    assert_eq!(leaf.range(0), Some(0..2));
    assert_eq!(leaf.range(3), None);
    assert_eq!(leaf.range(4), None);
    assert_eq!(
        leaf.row(2).map(|row| row.rows().into_owned()),
        Some(vec![Scalar::from(3_i64)])
    );
    assert_eq!(leaf.nulls().map(|nulls| nulls.null_count()), Some(1));
    assert_eq!(SerieValue::len(leaf), 4);
    assert_eq!(
        leaf.scalar(0).unwrap(),
        Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i64)])
    );
    assert_eq!(leaf.scalar(1).unwrap(), Scalar::from_sequence([]));
    assert_eq!(leaf.scalar(3).unwrap(), Scalar::Null);
    assert!(leaf.scalar(4).is_err());
    assert_eq!(column.items().map(Serie::len), Some(3));
    assert_eq!(leaf.into_arrow_array().len(), 4);
}

#[test]
fn a_sliced_cut_is_rebased_onto_the_items_it_reaches() {
    let sliced: ArrayRef = Arc::new(legs().slice(2, 2));
    let column = Serie::from_arrow_array(Some(&legs_field()), sliced, ArrowCastOptions::new())
        .expect("a sliced list column");
    let leaf = column.as_list().expect("a list column");

    assert_eq!(leaf.offsets().as_ref(), &[0, 1, 1]);
    assert_eq!(leaf.items().len(), 1);
    assert_eq!(
        leaf.scalar(0).unwrap(),
        Scalar::from_sequence([Scalar::from(3_i64)])
    );
    assert!(leaf.is_null(1).unwrap());

    // And a column's own slice rebases the same way.
    let window = legs_column().slice(0, 1).unwrap();
    let window = window.as_list().unwrap();
    assert_eq!(window.offsets().as_ref(), &[0, 2]);
    assert_eq!(window.items().len(), 2);
}
