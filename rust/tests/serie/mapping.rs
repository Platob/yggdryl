//! `rust/src/serie/mapping.rs`: the mapping leaf - a cut over an entries
//! record column, read back as pairs and written as two-cell records.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int64Array, MapArray, StringArray, StructArray};
use arrow_buffer::{NullBuffer, OffsetBuffer};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Fields};
use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie, SerieValue, StructType};

/// The entries record a tag mapping is stored as: a required text key and
/// a nullable int64 value.
fn entries_field() -> Field {
    Field::new(
        "entries",
        DataType::from(
            StructType::from_fields([
                Field::new("key", DataType::utf8(), false),
                Field::new("value", DataType::Int64, true),
            ])
            .unwrap(),
        ),
        false,
    )
}

/// A nullable mapping of tags, keys in no particular order.
fn tags_field() -> Field {
    Field::new("tags", DataType::map(entries_field(), false).unwrap(), true)
}

/// Two tag rows and an absent one: `{a: 1, b: null}`, absent, `{c: 3}`.
fn tag_rows() -> Vec<Scalar> {
    vec![
        Scalar::from_mapping([
            (Scalar::from("a"), Scalar::from(1_i64)),
            (Scalar::from("b"), Scalar::Null),
        ])
        .unwrap(),
        Scalar::Null,
        Scalar::from_mapping([(Scalar::from("c"), Scalar::from(3_i64))]).unwrap(),
    ]
}

/// The Arrow spelling of [`entries_field`]'s children.
fn entry_fields() -> Fields {
    vec![
        Arc::new(ArrowField::new("key", ArrowDataType::Utf8, false)),
        Arc::new(ArrowField::new("value", ArrowDataType::Int64, true)),
    ]
    .into()
}

/// The array [`tag_rows`] lays out as, built by hand.
fn tags() -> MapArray {
    let entries = StructArray::new(
        entry_fields(),
        vec![
            Arc::new(StringArray::from(vec!["a", "b", "c"])),
            Arc::new(Int64Array::from(vec![Some(1), None, Some(3)])),
        ],
        None,
    );
    MapArray::new(
        Arc::new(ArrowField::new(
            "entries",
            ArrowDataType::Struct(entry_fields()),
            false,
        )),
        OffsetBuffer::new(vec![0, 2, 2, 3].into()),
        entries,
        Some(NullBuffer::from(vec![true, false, true])),
        false,
    )
}

#[test]
fn a_row_past_the_end_and_a_row_the_field_refuses_leave_the_cut() {
    let mut column = Serie::from_scalars(tags_field(), tag_rows()).unwrap();

    let refusal = column.scalar(3).expect_err("row 3 is past the end");
    let text = refusal.to_string();
    assert!(text.contains("tags"), "names the column: {text}");
    assert!(text.contains('3'), "names the row: {text}");
    assert!(column.set(3, Scalar::Null).is_err());
    let backwards = (2, 1);
    assert!(column.splice(backwards.0..backwards.1, vec![]).is_err());

    // Not a mapping, a key the key field refuses, an absent key under a
    // required key field: each refused before any buffer is touched.
    assert!(column.push(Scalar::from(1_i64)).is_err());
    assert!(
        column
            .push(Scalar::from_sequence([Scalar::from("a")]))
            .is_err()
    );
    assert!(
        column
            .push(Scalar::from_mapping([(Scalar::Null, Scalar::from(1_i64))]).unwrap())
            .is_err()
    );
    assert!(
        column
            .push(Scalar::from_mapping([(Scalar::from("d"), Scalar::from("four"))]).unwrap())
            .is_err()
    );
    let leaf = column.as_map().expect("a mapping column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 2, 2, 3]);
    assert_eq!(leaf.entries().len(), 3);
    assert_eq!(column.rows().into_owned(), tag_rows());
}

#[test]
fn a_mapping_round_trips_through_arrow_and_a_column_built_by_pushes_is_the_laid_out_one() {
    let laid_out = Serie::from_scalars(tags_field(), tag_rows()).unwrap();
    let mut pushed = Serie::empty(tags_field()).unwrap();
    for row in tag_rows() {
        pushed.push(row).expect("a row the field accepts");
    }

    assert_eq!(pushed, laid_out);
    assert_eq!(pushed.into_arrow_array().unwrap().as_ref(), &tags());
    assert_eq!(laid_out.into_arrow_array().unwrap().as_ref(), &tags());
    assert_eq!(pushed.null_count(), 1);

    // Out through the door and back in, the rows are the rows.
    let out = laid_out.into_arrow_array().unwrap();
    let back = Serie::from_arrow_array(
        Some(&tags_field()),
        Arc::clone(&out),
        ArrowCastOptions::new(),
    )
    .unwrap();
    assert_eq!(back, laid_out);
    assert_eq!(back.rows().into_owned(), tag_rows());
    assert_eq!(back.into_arrow_array().unwrap().as_ref(), out.as_ref());
    let leaf = back.as_map().unwrap();
    assert_eq!(
        leaf.keys().rows().into_owned(),
        vec![Scalar::from("a"), Scalar::from("b"), Scalar::from("c")]
    );
    assert_eq!(leaf.values().scalar(1).unwrap(), Scalar::Null);
    assert_eq!(leaf.entries().child("key").map(Serie::len), Some(3));

    // A sliced array is rebased onto the entries it reaches.
    let sliced: ArrayRef = Arc::new(tags().slice(2, 1));
    let window =
        Serie::from_arrow_array(Some(&tags_field()), sliced, ArrowCastOptions::new()).unwrap();
    let leaf = window.as_map().unwrap();
    assert_eq!(leaf.offsets().as_ref(), &[0, 1]);
    assert_eq!(leaf.entries().len(), 1);
    assert_eq!(window.scalar(0).unwrap(), tag_rows()[2]);
}

#[test]
fn a_mapping_row_set_with_a_different_entry_count_recuts_and_later_rows_read_unchanged() {
    let mut column = Serie::from_scalars(tags_field(), tag_rows()).unwrap();

    column
        .set(
            0,
            Scalar::from_mapping([(Scalar::from("z"), Scalar::from(26_i64))]).unwrap(),
        )
        .expect("two entries become one");
    let leaf = column.as_map().unwrap();
    assert_eq!(leaf.offsets().as_ref(), &[0, 1, 1, 2]);
    assert_eq!(leaf.entries().len(), 2);
    assert_eq!(leaf.keys().scalar(0).unwrap(), Scalar::from("z"));
    assert_eq!(column.rows()[1..], tag_rows()[1..]);

    // Filled where it was absent, absented where it held entries, and
    // spliced in the middle: the cut follows.
    column
        .set(
            1,
            Scalar::from_mapping([
                (Scalar::from("x"), Scalar::Null),
                (Scalar::from("y"), Scalar::from(25_i64)),
            ])
            .unwrap(),
        )
        .unwrap();
    column.set(2, Scalar::Null).unwrap();
    let leaf = column.as_map().unwrap();
    assert_eq!(leaf.offsets().as_ref(), &[0, 1, 3, 3]);
    assert_eq!(leaf.range(1), Some(1..3));
    assert_eq!(leaf.range(2), None);
    assert_eq!(column.null_count(), 1);
    column
        .insert(
            1,
            Scalar::from_mapping([(Scalar::from("m"), Scalar::from(13_i64))]).unwrap(),
        )
        .unwrap();
    assert_eq!(
        column.as_map().unwrap().offsets().as_ref(),
        &[0, 1, 2, 4, 4]
    );
    assert_eq!(
        column.as_map().unwrap().keys().rows().into_owned(),
        vec![
            Scalar::from("z"),
            Scalar::from("m"),
            Scalar::from("x"),
            Scalar::from("y")
        ]
    );
    assert_eq!(column.remove(0).unwrap().len(), 1);
    assert_eq!(column.pop().unwrap(), Some(Scalar::Null));
    assert_eq!(column.as_map().unwrap().offsets().as_ref(), &[0, 1, 3]);
    assert_eq!(column.into_arrow_array().unwrap().len(), 2);
}

#[test]
fn extend_from_serie_appends_the_cut_and_the_entries_and_a_slice_rebases() {
    let mut column = Serie::from_scalars(tags_field(), tag_rows()).unwrap();
    column
        .extend_from_serie(&Serie::from_scalars(tags_field(), tag_rows()).unwrap())
        .expect("one layout appended");
    let leaf = column.as_map().unwrap();
    assert_eq!(leaf.offsets().as_ref(), &[0, 2, 2, 3, 5, 5, 6]);
    assert_eq!(leaf.entries().len(), 6);
    assert_eq!(column.null_count(), 2);
    let mut both = tag_rows();
    both.extend(tag_rows());
    assert_eq!(column.rows().into_owned(), both);

    // A run of mappings appends through the field's contract.
    column
        .extend_from_serie(&Serie::new(vec![
            Scalar::from_mapping([(Scalar::from("q"), Scalar::from(17_i64))]).unwrap(),
        ]))
        .unwrap();
    assert_eq!(column.len(), 7);
    assert_eq!(column.items().map(Serie::len), Some(7));

    // A window's cut is rebased onto the entries it reaches, and grows on
    // its own once the original is dropped.
    let mut window = column.slice(2, 2).unwrap();
    drop(column);
    let leaf = window.as_map().unwrap();
    assert_eq!(leaf.offsets().as_ref(), &[0, 1, 3]);
    assert_eq!(leaf.entries().len(), 3);
    window.push(Scalar::Null).unwrap();
    assert_eq!(window.as_map().unwrap().offsets().as_ref(), &[0, 1, 3, 3]);
    assert_eq!(window.scalar(0).unwrap(), tag_rows()[2]);
    assert_eq!(window.scalar(1).unwrap(), tag_rows()[0]);
    assert_eq!(window.into_arrow_array().unwrap().len(), 3);
}

#[test]
fn keys_sorted_is_read_off_the_field_and_laid_out_on_the_array() {
    let sorted = Field::new("tags", DataType::map(entries_field(), true).unwrap(), true);
    let column = Serie::from_scalars(sorted.clone(), tag_rows()).unwrap();
    assert!(column.as_map().unwrap().keys_sorted());
    assert!(matches!(
        column.into_arrow_array().unwrap().data_type(),
        ArrowDataType::Map(_, true)
    ));

    let unsorted = Serie::from_scalars(tags_field(), tag_rows()).unwrap();
    assert!(!unsorted.as_map().unwrap().keys_sorted());
    assert!(matches!(
        unsorted.into_arrow_array().unwrap().data_type(),
        ArrowDataType::Map(_, false)
    ));

    // The layout that says unsorted is cast into the one that says sorted:
    // the same rows, now laid out under the sorted field.
    let cast = Serie::from_arrow_array(Some(&sorted), Arc::new(tags()), ArrowCastOptions::new())
        .expect("keys already in order cast into the sorted layout");
    assert!(cast.as_map().unwrap().keys_sorted());
    assert!(matches!(
        cast.into_arrow_array().unwrap().data_type(),
        ArrowDataType::Map(_, true)
    ));
    assert_eq!(cast.rows().as_ref(), tag_rows().as_slice());

    // Keys out of order do not become sorted by being cast: the row that
    // breaks the order is refused.
    let unordered = MapArray::new(
        Arc::new(ArrowField::new(
            "entries",
            ArrowDataType::Struct(entry_fields()),
            false,
        )),
        OffsetBuffer::new(vec![0, 2].into()),
        StructArray::new(
            entry_fields(),
            vec![
                Arc::new(StringArray::from(vec!["b", "a"])),
                Arc::new(Int64Array::from(vec![Some(1), Some(2)])),
            ],
            None,
        ),
        None,
        false,
    );
    let refusal =
        Serie::from_arrow_array(Some(&sorted), Arc::new(unordered), ArrowCastOptions::new())
            .expect_err("b before a is not sorted");
    assert!(refusal.to_string().contains("row 0"), "{refusal}");
}

#[test]
fn a_mapping_column_holds_its_entries_as_a_record_column_and_pairs_them_back() {
    let first = tag_rows().swap_remove(0);
    let column =
        Serie::from_scalars(tags_field(), [first.clone(), Scalar::Null]).expect("two mappings");
    let leaf = column.as_map().expect("a mapping column");

    assert_eq!(leaf.offsets().as_ref(), &[0, 2, 2]);
    assert_eq!(leaf.entries().len(), 2);
    assert_eq!(leaf.keys().scalar(1).unwrap(), Scalar::from("b"));
    assert_eq!(leaf.values().scalar(0).unwrap(), Scalar::from(1_i64));
    assert!(!leaf.keys_sorted());
    assert_eq!(leaf.range(0), Some(0..2));
    assert_eq!(leaf.range(1), None);
    assert_eq!(leaf.row(0).map(|row| row.len()), Some(2));
    assert_eq!(leaf.scalar(0).unwrap(), first);
    assert_eq!(leaf.scalar(1).unwrap(), Scalar::Null);
    assert!(leaf.scalar(2).is_err());
    assert_eq!(leaf.null_count(), 1);
    assert_eq!(column.items().map(Serie::len), Some(2));
    assert_eq!(leaf.slice(1, 1).unwrap().entries().len(), 0);
    assert_eq!(leaf.into_arrow_array().len(), 2);
}
