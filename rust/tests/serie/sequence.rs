//! `rust/src/serie/sequence.rs`: the serie leaves - an item column under
//! a cut, the cut rebased onto exactly the items it reaches and re-cut by
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

/// Three series of int64 - `[1, 2]`, `[]`, `[3]` - and a fourth, absent.
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
        DataType::serie(Field::new("item", DataType::Int64, false)),
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
    .expect("a serie column")
}

/// A nullable fixed-size serie of two required int64 items.
fn pairs_field() -> Field {
    Field::new(
        "pair",
        DataType::fixed_size_serie(Field::new("item", DataType::Int64, false), 2).unwrap(),
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
    let leaf = column.as_serie().expect("a serie column");
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
fn a_serie_row_set_with_a_different_item_count_recuts_the_offsets_and_later_rows_read_unchanged() {
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
    let leaf = column.as_serie().expect("a serie column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 3, 3, 4, 4]);
    assert_eq!(leaf.items().as_int64().unwrap().values(), &[9, 8, 7, 3]);
    assert_eq!(column.rows()[1..], leg_rows()[1..]);

    // Emptied, filled where it was absent, and absented where it held items.
    column.set(2, Scalar::from_sequence([])).unwrap();
    column
        .set(3, Scalar::from_sequence([Scalar::from(5_i64)]))
        .unwrap();
    column.set(1, Scalar::Null).unwrap();
    let leaf = column.as_serie().expect("a serie column");
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
    let leaf = column.as_serie().expect("a serie column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 3, 4, 6, 6, 7]);
    assert_eq!(
        leaf.items().as_int64().unwrap().values(),
        &[9, 8, 7, 1, 2, 3, 5]
    );
    assert_eq!(column.null_count(), 0);
    assert_eq!(column.remove(0).unwrap().len(), 3);
    assert_eq!(
        column.as_serie().unwrap().offsets().as_ref(),
        &[0, 1, 3, 3, 4]
    );
    column.truncate(2).unwrap();
    assert_eq!(column.as_serie().unwrap().offsets().as_ref(), &[0, 1, 3]);
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
        pushed.as_serie().unwrap().offsets().as_ref(),
        &[0, 2, 2, 3, 3]
    );

    // A 64-bit cut grows the same way.
    let large = Field::new(
        "legs",
        DataType::large_serie(Field::new("item", DataType::Int64, false)),
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
        pushed.as_large_serie().unwrap().offsets().as_ref(),
        &[0, 2, 2, 3, 3]
    );
    assert_eq!(pushed.as_large_serie().unwrap().range(2), Some(2..3));
}

#[test]
fn a_fixed_size_serie_null_row_holds_width_placeholder_items_and_a_cleared_bit() {
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
        .as_fixed_size_serie()
        .expect("a fixed-size serie column");
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
fn a_serie_view_crosses_in_compact_and_every_write_keeps_it_so() {
    let field = Field::new(
        "legs",
        DataType::serie_view(Field::new("item", DataType::Int64, false)),
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
        .expect("a serie-view column");
    let leaf = column.as_serie_view().expect("a serie-view column");

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
    let leaf = column.as_serie_view().expect("a serie-view column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 2, 4, 4, 6]);
    assert_eq!(leaf.sizes().as_ref(), &[2, 2, 0, 2, 1]);
    assert_eq!(
        leaf.items().as_int64().unwrap().values(),
        &[7, 8, 1, 2, 2, 3, 4]
    );
    let window = column.slice(1, 2).unwrap();
    let window = window.as_serie_view().unwrap();
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
    let leaf = sliced.as_serie_view().unwrap();
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

    let leaf = window.as_serie().unwrap();
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
fn extend_from_serie_appends_the_cut_and_the_items_and_a_serie_of_records_pushes_down() {
    let mut column = legs_column();
    column
        .extend_from_serie(&legs_column())
        .expect("one layout appended");
    let leaf = column.as_serie().unwrap();
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
        DataType::serie(Field::new(
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
fn a_serie_column_cuts_one_item_column_and_reads_a_row_off_it() {
    let column = legs_column();
    let leaf = column.as_serie().expect("a serie column");

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
        .expect("a sliced serie column");
    let leaf = column.as_serie().expect("a serie column");

    assert_eq!(leaf.offsets().as_ref(), &[0, 1, 1]);
    assert_eq!(leaf.items().len(), 1);
    assert_eq!(
        leaf.scalar(0).unwrap(),
        Scalar::from_sequence([Scalar::from(3_i64)])
    );
    assert!(leaf.is_null(1).unwrap());

    // And a column's own slice rebases the same way.
    let window = legs_column().slice(0, 1).unwrap();
    let window = window.as_serie().unwrap();
    assert_eq!(window.offsets().as_ref(), &[0, 2]);
    assert_eq!(window.items().len(), 2);
}

#[test]
fn narrow_span_gathers_preserve_nested_zero_width_rows_and_view_order() {
    let empty: ArrayRef = Arc::new(
        FixedSizeListArray::try_new_with_length(
            item(),
            0,
            Arc::new(Int64Array::from(Vec::<i64>::new())),
            None,
            3,
        )
        .expect("three zero-width rows"),
    );
    let values: ArrayRef = Arc::new(StructArray::new(
        vec![
            Arc::new(ArrowField::new("empty", empty.data_type().clone(), false)),
            DataType::IsinCode
                .required_field("code")
                .into_arrow_field_ref()
                .unwrap(),
        ]
        .into(),
        vec![
            empty,
            Arc::new(arrow_array::StringArray::from(vec![
                "US0378331005",
                "BAD",
                "GB0002634946",
            ])),
        ],
        None,
    ));
    let item = Arc::new(ArrowField::new("item", values.data_type().clone(), false));
    let present = Some(NullBuffer::from(vec![true, false, true]));
    let offsets: ArrayRef = Arc::new(ListArray::new(
        Arc::clone(&item),
        OffsetBuffer::new(vec![0_i32, 1, 2, 3].into()),
        Arc::clone(&values),
        present.clone(),
    ));
    let views: ArrayRef = Arc::new(ListViewArray::new(
        item,
        ScalarBuffer::from(vec![2_i32, 1, 0]),
        ScalarBuffer::from(vec![1_i32, 1, 1]),
        values,
        present,
    ));

    for (array, expected) in [
        (offsets, ["US0378331005", "GB0002634946"]),
        (views, ["GB0002634946", "US0378331005"]),
    ] {
        array
            .to_data()
            .validate_full()
            .expect("legal required child arrays");
        let column = Serie::from_arrow_array(None, array, ArrowCastOptions::new())
            .expect("gathering a narrow struct preserves its zero-width sibling");
        assert_eq!(column.len(), 3);
        assert_eq!(column.scalar(1).unwrap(), Scalar::Null);
        let items = column.items().expect("the gathered item column");
        assert_eq!(items.len(), 2);
        assert_eq!(items.rows().len(), 2, "the hidden invalid ISIN was removed");
        let empty = items.child("empty").unwrap().as_fixed_size_serie().unwrap();
        assert_eq!(empty.width(), 0);
        assert_eq!(empty.len(), 2);
        for row in 0..2 {
            assert_eq!(empty.scalar(row).unwrap(), Scalar::from_sequence([]));
        }
        let codes = items.child("code").unwrap();
        for (row, code) in expected.into_iter().enumerate() {
            assert_eq!(codes.scalar(row).unwrap().as_str(), Some(code));
        }
        match column.as_serie() {
            Some(list) => assert_eq!(list.offsets().as_ref(), &[0, 1, 1, 2]),
            None => {
                let view = column.as_serie_view().expect("the other input is a view");
                assert_eq!(view.offsets().as_ref(), &[0, 1, 1]);
                assert_eq!(view.sizes().as_ref(), &[1, 0, 1]);
            }
        }
        column
            .require_arrow_array()
            .unwrap()
            .to_data()
            .validate_full()
            .expect("the gathered struct and zero-width child have the same length");
    }
}

#[test]
fn narrow_sibling_compactions_share_one_materialization_budget() {
    // Each child copies 600K present ISIN slots across two disjoint spans.
    // The copied payload is well below the byte limit; their aggregate
    // exceeds the million-slot limit while either column alone fits.
    let values = arrow_array::StringArray::from_iter_values((0..600_001).map(|row| {
        if row == 300_000 {
            "BAD"
        } else {
            "US0378331005"
        }
    }));
    let array: ArrayRef = Arc::new(ListArray::new(
        DataType::IsinCode
            .required_field("item")
            .into_arrow_field_ref()
            .unwrap(),
        OffsetBuffer::new(vec![0_i32, 300_000, 300_001, 600_001].into()),
        Arc::new(values),
        Some(NullBuffer::from(vec![true, false, true])),
    ));
    array
        .to_data()
        .validate_full()
        .expect("legal foreign buffers, including the opaque hidden slot");
    for name in ["left", "right"] {
        let schema = Arc::new(arrow_schema::Schema::new(vec![ArrowField::new(
            name,
            array.data_type().clone(),
            true,
        )]));
        let batch = arrow_array::RecordBatch::try_new(schema, vec![Arc::clone(&array)]).unwrap();
        let root = Serie::from_arrow_batch(None, &batch, ArrowCastOptions::new())
            .expect("either 600K-slot compaction fits independently");
        let column = root.child(name).unwrap();
        let list = column.as_serie().unwrap();
        assert_eq!(list.offsets().as_ref(), &[0, 300_000, 300_000, 600_000]);
        assert_eq!(list.items().len(), 600_000);
        assert_eq!(column.scalar(1).unwrap(), Scalar::Null);
        assert_eq!(
            list.items().scalar(599_999).unwrap().as_str(),
            Some("US0378331005")
        );
    }
    let schema = Arc::new(arrow_schema::Schema::new(vec![
        ArrowField::new("left", array.data_type().clone(), true),
        ArrowField::new("right", array.data_type().clone(), true),
    ]));
    let batch = arrow_array::RecordBatch::try_new(schema, vec![Arc::clone(&array), array]).unwrap();
    let refusal = Serie::from_arrow_batch(None, &batch, ArrowCastOptions::new())
        .expect_err("the second output exceeds the shared slot budget");
    let message = refusal.to_string();
    assert!(
        message.contains("expanded slots"),
        "names the resource: {message}"
    );
    assert!(
        message.contains("1000000"),
        "states the slot bound: {message}"
    );
    assert!(
        message.contains("right"),
        "locates the second sibling: {message}"
    );
}

#[test]
fn hidden_spans_preserve_zero_width_items_without_gathering() {
    let empty: ArrayRef = Arc::new(
        FixedSizeListArray::try_new_with_length(
            item(),
            0,
            Arc::new(Int64Array::from(Vec::<i64>::new())),
            None,
            3,
        )
        .expect("three present zero-width items"),
    );
    let records: ArrayRef = Arc::new(StructArray::new(
        vec![
            Arc::new(ArrowField::new("empty", empty.data_type().clone(), false)),
            Arc::new(ArrowField::new("id", ArrowDataType::Int64, false)),
        ]
        .into(),
        vec![
            Arc::clone(&empty),
            Arc::new(Int64Array::from(vec![7_i64, 999, 9])),
        ],
        None,
    ));

    for values in [empty, records] {
        let nested_record = matches!(values.data_type(), ArrowDataType::Struct(_));
        let array = ListArray::new(
            Arc::new(ArrowField::new("item", values.data_type().clone(), false)),
            OffsetBuffer::new(vec![0_i32, 1, 2, 3].into()),
            values,
            Some(NullBuffer::from(vec![true, false, true])),
        );
        let offsets = array.value_offsets().as_ptr();
        array
            .to_data()
            .validate_full()
            .expect("legal foreign buffers");
        let column = Serie::from_arrow_array(None, Arc::new(array), ArrowCastOptions::new())
            .expect("the two visible zero-width items land");
        let leaf = column.as_serie().expect("the outer list");
        assert_eq!(leaf.offsets().as_ref(), &[0, 1, 2, 3]);
        assert_eq!(leaf.offsets().as_ptr(), offsets);
        assert_eq!(leaf.items().len(), 3);
        assert_eq!(column.scalar(1).unwrap(), Scalar::Null);

        let empty_items = if nested_record {
            let items = leaf.items();
            assert_eq!(
                items
                    .child("id")
                    .and_then(Serie::as_int64)
                    .unwrap()
                    .values(),
                &[7, 999, 9],
                "the physical middle record is retained under its null parent"
            );
            items.child("empty").expect("the zero-width child")
        } else {
            leaf.items()
        };
        let empty_items = empty_items.as_fixed_size_serie().expect("zero-width items");
        assert_eq!(empty_items.width(), 0);
        assert_eq!(empty_items.len(), 3);
        for row in 0..3 {
            assert_eq!(empty_items.scalar(row).unwrap(), Scalar::from_sequence([]));
        }
        for row in [0, 2] {
            let visible = column.scalar(row).expect("a present list row");
            assert_eq!(visible.as_sequence().map(<[Scalar]>::len), Some(1));
        }

        let exported = column
            .require_arrow_array()
            .expect("an aligned Arrow output");
        exported
            .to_data()
            .validate_full()
            .expect("all descendants remain aligned");
        let lists = exported.as_any().downcast_ref::<ListArray>().unwrap();
        assert_eq!(lists.values().len(), 3);
        assert_eq!(lists.len(), 3);
    }
}

#[test]
fn hidden_spans_borrow_sibling_columns_larger_than_the_materialization_limit() {
    // The million-slot bound concerns materialized output. These two columns
    // borrow their 600001 physical items, so only temporary proof masks cost.
    let values = Int64Array::from_iter_values(0_i64..600_001);
    let payload = values.values().as_ptr();
    let array: ArrayRef = Arc::new(ListArray::new(
        item(),
        OffsetBuffer::new(vec![0_i32, 300_000, 300_001, 600_001].into()),
        Arc::new(values),
        Some(NullBuffer::from(vec![true, false, true])),
    ));
    array
        .to_data()
        .validate_full()
        .expect("legal foreign buffers");
    let one = Serie::from_arrow_array(None, Arc::clone(&array), ArrowCastOptions::new())
        .expect("one physical child column remains borrowed");
    assert_eq!(one.items().map(Serie::len), Some(600_001));
    drop(one);

    let schema = Arc::new(arrow_schema::Schema::new(vec![
        ArrowField::new("left", array.data_type().clone(), true),
        ArrowField::new("right", array.data_type().clone(), true),
    ]));
    let batch = arrow_array::RecordBatch::try_new(schema, vec![Arc::clone(&array), array])
        .expect("two legal list columns");
    let root = Serie::from_arrow_batch(None, &batch, ArrowCastOptions::new())
        .expect("both physical child columns remain borrowed");
    for name in ["left", "right"] {
        let column = root.child(name).unwrap();
        let lists = column.as_serie().unwrap();
        assert_eq!(lists.items().len(), 600_001);
        assert_eq!(lists.items().as_int64().unwrap().values().as_ptr(), payload);
        assert_eq!(column.scalar(1).unwrap(), Scalar::Null);
        assert_eq!(lists.row(0).unwrap().len(), 300_000);
        assert_eq!(lists.row(2).unwrap().len(), 300_000);
    }
}

#[test]
fn inherited_nulls_reach_view_and_fixed_items_through_two_records() {
    for fixed in [false, true] {
        for visible_bad in [false, true] {
            let item = DataType::IsinCode
                .required_field("item")
                .into_arrow_field_ref()
                .unwrap();
            let values: ArrayRef = Arc::new(arrow_array::StringArray::from(vec![
                "US0378331005",
                "BAD",
                if visible_bad { "BAD" } else { "GB0002634946" },
            ]));
            let sequence: ArrayRef = if fixed {
                Arc::new(FixedSizeListArray::new(item, 1, values, None))
            } else {
                Arc::new(ListViewArray::new(
                    item,
                    ScalarBuffer::from(vec![0_i32, 1, 2]),
                    ScalarBuffer::from(vec![1_i32, 1, 1]),
                    values,
                    None,
                ))
            };
            let inner: ArrayRef = Arc::new(StructArray::new(
                vec![Arc::new(ArrowField::new(
                    "codes",
                    sequence.data_type().clone(),
                    false,
                ))]
                .into(),
                vec![sequence],
                None,
            ));
            let outer: ArrayRef = Arc::new(StructArray::new(
                vec![Arc::new(ArrowField::new(
                    "inner",
                    inner.data_type().clone(),
                    false,
                ))]
                .into(),
                vec![inner],
                Some(NullBuffer::from(vec![true, false, true])),
            ));
            outer
                .to_data()
                .validate_full()
                .expect("legal nested foreign buffers");
            let landed = Serie::from_arrow_array(None, outer, ArrowCastOptions::new());
            if visible_bad {
                let refusal = landed.expect_err("a visible invalid ISIN still refuses");
                let message = refusal.to_string();
                assert!(message.contains("codes"), "names the child: {message}");
                assert!(message.contains("[2]"), "names the visible row: {message}");
                continue;
            }
            let column = landed.expect("both record ancestors reach the item proof");
            assert_eq!(column.scalar(1).unwrap(), Scalar::Null);
            for row in [0, 2] {
                assert!(column.scalar(row).is_ok());
            }
            column
                .require_arrow_array()
                .unwrap()
                .to_data()
                .validate_full()
                .expect("physical required children remain valid Arrow arrays");
        }
    }
}

#[test]
fn replacing_a_null_offset_row_replaces_its_retained_physical_span() {
    let array: ArrayRef = Arc::new(ListArray::new(
        item(),
        OffsetBuffer::new(vec![0_i32, 1, 3, 4].into()),
        Arc::new(Int64Array::from(vec![7_i64, 998, 999, 9])),
        Some(NullBuffer::from(vec![true, false, true])),
    ));
    let mut column = Serie::from_arrow_array(None, array, ArrowCastOptions::new()).unwrap();
    let shared = column.clone();
    column
        .set(1, Scalar::from_sequence([Scalar::from(8_i64)]))
        .unwrap();
    let lists = column.as_serie().unwrap();
    assert_eq!(lists.offsets().as_ref(), &[0, 1, 2, 3]);
    assert_eq!(lists.items().as_int64().unwrap().values(), &[7, 8, 9]);
    assert_eq!(shared.scalar(1).unwrap(), Scalar::Null);
    assert_eq!(
        shared
            .as_serie()
            .unwrap()
            .items()
            .as_int64()
            .unwrap()
            .values(),
        &[7, 998, 999, 9]
    );
    column
        .require_arrow_array()
        .unwrap()
        .to_data()
        .validate_full()
        .unwrap();
}

#[test]
fn null_sequence_rows_mask_their_physical_items_during_inferred_batch_landing() {
    fn values() -> ArrayRef {
        Arc::new(arrow_array::StringArray::from(vec![
            "US0378331005",
            "BAD",
            "GB0002634946",
        ]))
    }

    fn validity() -> Option<NullBuffer> {
        Some(NullBuffer::from(vec![true, false, true]))
    }

    fn assert_visible_neighbors(column: &Serie) {
        let first = column.scalar(0).expect("the first row");
        assert_eq!(
            first.as_sequence().and_then(|row| row[0].as_str()),
            Some("US0378331005")
        );
        assert_eq!(column.scalar(1).unwrap(), Scalar::Null);
        let third = column.scalar(2).expect("the third row");
        assert_eq!(
            third.as_sequence().and_then(|row| row[0].as_str()),
            Some("GB0002634946")
        );
    }

    let item = DataType::IsinCode
        .required_field("item")
        .into_arrow_field_ref()
        .expect("ISIN projects");
    let arrays: Vec<(&str, ArrayRef)> = vec![
        (
            "list32",
            Arc::new(ListArray::new(
                Arc::clone(&item),
                OffsetBuffer::new(vec![0_i32, 1, 2, 3].into()),
                values(),
                validity(),
            )),
        ),
        (
            "list64",
            Arc::new(LargeListArray::new(
                Arc::clone(&item),
                OffsetBuffer::new(vec![0_i64, 1, 2, 3].into()),
                values(),
                validity(),
            )),
        ),
        (
            "view32",
            Arc::new(ListViewArray::new(
                Arc::clone(&item),
                ScalarBuffer::from(vec![0_i32, 1, 2]),
                ScalarBuffer::from(vec![1_i32, 1, 1]),
                values(),
                validity(),
            )),
        ),
        (
            "view64",
            Arc::new(arrow_array::LargeListViewArray::new(
                Arc::clone(&item),
                ScalarBuffer::from(vec![0_i64, 1, 2]),
                ScalarBuffer::from(vec![1_i64, 1, 1]),
                values(),
                validity(),
            )),
        ),
        (
            "fixed",
            Arc::new(FixedSizeListArray::new(
                Arc::clone(&item),
                1,
                values(),
                validity(),
            )),
        ),
    ];

    // The child list is present at every row; the enclosing struct masks its
    // invalid middle item instead.
    let nested = ListArray::new(
        item,
        OffsetBuffer::new(vec![0_i32, 1, 2, 3].into()),
        values(),
        None,
    );
    let nested_fields = vec![Arc::new(ArrowField::new(
        "codes",
        nested.data_type().clone(),
        false,
    ))]
    .into();
    let wrapped: ArrayRef = Arc::new(StructArray::new(
        nested_fields,
        vec![Arc::new(nested)],
        validity(),
    ));

    let mut columns = arrays;
    columns.push(("wrapped", wrapped));
    let schema = Arc::new(arrow_schema::Schema::new(
        columns
            .iter()
            .map(|(name, array)| ArrowField::new(*name, array.data_type().clone(), true))
            .collect::<Vec<_>>(),
    ));
    let batch = arrow_array::RecordBatch::try_new(
        schema,
        columns.iter().map(|(_, array)| Arc::clone(array)).collect(),
    )
    .expect("one batch");
    let root = Serie::from_arrow_batch(None, &batch, ArrowCastOptions::new())
        .expect("null ancestors hide their physical items");

    for name in ["list32", "list64", "view32", "view64", "fixed"] {
        assert_visible_neighbors(root.child(name).expect("the inferred child"));
    }
    let wrapped = root.child("wrapped").expect("the wrapped child");
    assert_eq!(wrapped.scalar(1).unwrap(), Scalar::Null);
    for (row, expected) in [(0, "US0378331005"), (2, "GB0002634946")] {
        let record = wrapped.scalar(row).expect("a visible wrapper row");
        assert_eq!(
            record
                .as_sequence()
                .and_then(|record| record[0].as_sequence())
                .and_then(|codes| codes[0].as_str()),
            Some(expected)
        );
    }
}
