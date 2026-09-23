//! `rust/src/serie/bytes.rs`: the byte layouts - offsets into one payload,
//! views, and one fixed width - read as bytes: what they lend where it
//! lies, what a write refuses, and how a write lands in the buffers.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BinaryArray, FixedSizeBinaryArray};
use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie, SerieValue, Uuid};

/// A nullable binary column of three runs, one of them absent, straight off
/// Arrow buffers.
fn raw() -> Serie {
    let array: ArrayRef = Arc::new(BinaryArray::from(vec![
        Some(b"ab".as_slice()),
        None,
        Some(b"cde".as_slice()),
    ]));
    Serie::from_arrow_array(
        Some(&Field::new("raw", DataType::binary(), true)),
        array,
        ArrowCastOptions::new(),
    )
    .expect("a binary column")
}

/// A required two-byte column of two runs.
fn pairs() -> Serie {
    let array: ArrayRef = Arc::new(
        FixedSizeBinaryArray::try_from_iter([b"ab".to_vec(), b"cd".to_vec()].into_iter())
            .expect("two runs of two bytes"),
    );
    Serie::from_arrow_array(
        Some(&Field::new(
            "pair",
            DataType::fixed_binary(2).unwrap(),
            false,
        )),
        array,
        ArrowCastOptions::new(),
    )
    .expect("a fixed-width column")
}

/// The bytes every present row of `column` holds, `None` for an absent one.
fn runs(column: &Serie) -> Vec<Option<Vec<u8>>> {
    column
        .rows()
        .iter()
        .map(|row| row.as_bytes().map(<[u8]>::to_vec))
        .collect()
}

#[test]
fn a_required_column_refuses_an_absent_row_and_leaves_the_column_unchanged() {
    let mut column = Serie::from_scalars(
        Field::new("raw", DataType::binary(), false),
        [Scalar::from(b"ab".as_slice())],
    )
    .expect("one run");

    let refusal = column
        .push(Scalar::Null)
        .expect_err("a required column admits no absent row");
    assert!(
        refusal.to_string().contains("raw"),
        "the refusal names the column: {refusal}"
    );
    assert!(column.set(0, Scalar::Null).is_err());
    // A run of values is not a run of bytes; text coerces into one.
    let nested = Scalar::from_sequence([Scalar::from(1_i64)]);
    assert!(column.set(0, nested.clone()).is_err());
    assert!(column.insert(0, nested).is_err());
    assert_eq!(column.len(), 1);
    assert_eq!(column.scalar(0).unwrap(), Scalar::from(b"ab".as_slice()));
    assert!(column.as_binary().unwrap().nulls().is_none());
}

#[test]
fn a_row_past_the_end_is_refused_naming_the_column_and_both_counts() {
    let column = raw();
    let leaf = column.as_binary().expect("a binary column");

    let refusal = leaf.scalar(3).expect_err("row 3 is past the end");
    let text = refusal.to_string();
    assert!(text.contains("raw"), "names the column: {text}");
    assert!(text.contains('3'), "names the row: {text}");
    assert!(leaf.is_null(3).is_err());
    assert_eq!(leaf.value(3), None);
    assert!(leaf.slice(2, 2).is_err());
    assert!(leaf.slice(usize::MAX, 1).is_err());

    let mut column = raw();
    assert!(column.set(3, Scalar::from(b"x".as_slice())).is_err());
    assert!(column.insert(4, Scalar::from(b"x".as_slice())).is_err());
    let backwards = (2, 1);
    assert!(column.splice(backwards.0..backwards.1, vec![]).is_err());
    assert!(column.splice(1..4, vec![]).is_err());
    assert_eq!(column.len(), 3);
}

#[test]
fn a_byte_column_lends_its_offsets_and_its_payload_where_they_lie() {
    let column = raw();
    let leaf = column.as_binary().expect("a binary column");

    assert_eq!(leaf.offsets().as_ref(), &[0, 2, 2, 5]);
    assert_eq!(leaf.payload().as_slice(), b"abcde");
    assert_eq!(leaf.value(0), Some(b"ab".as_slice()));
    assert_eq!(leaf.value(1), None);
    assert_eq!(leaf.value(3), None);
    assert_eq!(leaf.nulls().map(|nulls| nulls.null_count()), Some(1));
    assert_eq!(leaf.null_count(), 1);
    assert_eq!(leaf.array().len(), 3);
    assert_eq!(leaf.scalar(2).unwrap(), Scalar::from(b"cde".as_slice()));
    assert_eq!(leaf.scalar(1).unwrap(), Scalar::Null);
    assert_eq!(leaf.slice(1, 2).unwrap().value(1), Some(b"cde".as_slice()));
    assert!(column.as_binary_string().is_none());
    assert!(column.as_large_binary().is_none());
    assert!(column.as_bytes().is_some());
    assert!(column.as_string().is_none());
}

#[test]
fn a_set_in_the_middle_moves_every_later_offset() {
    let mut column = raw();

    column
        .set(0, Scalar::from(b"abcd".as_slice()))
        .expect("a longer run in front");
    let leaf = column.as_binary().expect("a binary column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 4, 4, 7]);
    assert_eq!(leaf.payload().as_slice(), b"abcdcde");
    assert_eq!(leaf.value(2), Some(b"cde".as_slice()));

    column
        .set(1, Scalar::from(b"x".as_slice()))
        .expect("a filled slot");
    column.set(2, Scalar::Null).expect("an emptied slot");
    let leaf = column.as_binary().expect("a binary column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 4, 5, 5]);
    assert_eq!(leaf.payload().as_slice(), b"abcdx");
    assert_eq!(column.null_count(), 1);
    assert_eq!(
        runs(&column),
        vec![Some(b"abcd".to_vec()), Some(b"x".to_vec()), None]
    );
}

#[test]
fn every_write_round_trips_through_the_rows() {
    let mut column = raw();

    column
        .splice(
            0..2,
            vec![
                Scalar::from(b"1".as_slice()),
                Scalar::Null,
                Scalar::from(b"22".as_slice()),
            ],
        )
        .expect("two rows replaced by three");
    column
        .insert(1, Scalar::from(b"333".as_slice()))
        .expect("a row inserted");
    assert_eq!(
        runs(&column),
        vec![
            Some(b"1".to_vec()),
            Some(b"333".to_vec()),
            None,
            Some(b"22".to_vec()),
            Some(b"cde".to_vec()),
        ]
    );
    assert_eq!(
        column.as_binary().unwrap().offsets().as_ref(),
        &[0, 1, 4, 4, 6, 9]
    );

    assert_eq!(column.remove(1).unwrap(), Scalar::from(b"333".as_slice()));
    assert_eq!(column.pop().unwrap(), Some(Scalar::from(b"cde".as_slice())));
    column.truncate(2).expect("rows 2.. dropped");
    assert_eq!(runs(&column), vec![Some(b"1".to_vec()), None]);

    column
        .resize(4, Scalar::from(b"zz".as_slice()))
        .expect("two clones appended");
    assert_eq!(column.as_binary().unwrap().payload().as_slice(), b"1zzzz");
    column.extend(vec![Scalar::Null]).expect("one absent row");
    assert_eq!(column.len(), 5);
    assert_eq!(column.null_count(), 2);
    column.clear().expect("every row dropped");
    assert!(column.is_empty());
    assert_eq!(column.field().map(Field::name), Some("raw"));
}

#[test]
fn a_slice_with_its_original_dropped_grows_into_a_freshly_built_array() {
    let whole = raw();
    let mut window = whole.slice(1, 2).expect("rows 1..3");
    drop(whole);

    window
        .push(Scalar::from(b"f".as_slice()))
        .expect("a present row");
    window.push(Scalar::Null).expect("an absent row");
    window
        .set(0, Scalar::from(b"gh".as_slice()))
        .expect("a filled slot");

    let expected: ArrayRef = Arc::new(BinaryArray::from(vec![
        Some(b"gh".as_slice()),
        Some(b"cde".as_slice()),
        Some(b"f".as_slice()),
        None,
    ]));
    let held = window.into_arrow_array().expect("a column");
    assert_eq!(held.as_ref(), expected.as_ref());
    assert_eq!(
        window.as_binary().unwrap().offsets().as_ref(),
        &[0, 2, 5, 6, 6]
    );
}

#[test]
fn a_column_built_by_pushes_is_the_from_scalars_column_buffer_for_buffer() {
    let field = Field::new("raw", DataType::binary(), true);
    let rows = vec![
        Scalar::from(b"ab".as_slice()),
        Scalar::Null,
        Scalar::from(b"cde".as_slice()),
    ];

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
fn two_columns_of_one_field_append_buffer_to_buffer() {
    let mut column = raw();
    let more = Serie::from_scalars(
        Field::new("more", DataType::binary(), false),
        [
            Scalar::from(b"f".as_slice()),
            Scalar::from(b"gh".as_slice()),
        ],
    )
    .expect("two runs");

    column.extend_from_serie(&more).expect("one layout");
    let leaf = column.as_binary().expect("a binary column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 2, 2, 5, 6, 8]);
    assert_eq!(leaf.payload().as_slice(), b"abcdefgh");
    assert_eq!(column.null_count(), 1);

    // A required column takes a nullable one through the rows instead, and
    // refuses the absent row by name.
    let mut required = Serie::from_scalars(
        Field::new("raw", DataType::binary(), false),
        [Scalar::from(b"a".as_slice())],
    )
    .unwrap();
    assert!(required.extend_from_serie(&column).is_err());
    assert_eq!(required.len(), 1);
}

#[test]
fn the_large_and_view_layouts_write_through_the_same_verbs() {
    let mut large = Serie::from_scalars(
        Field::new("raw", DataType::large_binary(), true),
        [Scalar::from(b"ab".as_slice()), Scalar::Null],
    )
    .expect("two runs");
    large.push(Scalar::from(b"cde".as_slice())).expect("a run");
    large
        .set(1, Scalar::from(b"x".as_slice()))
        .expect("a filled slot");
    let leaf = large.as_large_binary().expect("a large binary column");
    assert_eq!(leaf.offsets().as_ref(), &[0_i64, 2, 3, 6]);
    assert_eq!(leaf.payload().as_slice(), b"abxcde");

    let mut views = Serie::from_scalars(
        Field::new("raw", DataType::binary_view(), true),
        [Scalar::from(b"ab".as_slice()), Scalar::Null],
    )
    .expect("two runs");
    views
        .push(Scalar::from(b"a run longer than twelve bytes".as_slice()))
        .expect("a run held out of line");
    views
        .set(1, Scalar::from(b"x".as_slice()))
        .expect("a filled slot");
    views.remove(0).expect("the first row");
    let leaf = views.as_binary_view().expect("a binary view column");
    assert_eq!(leaf.views().len(), 2);
    assert_eq!(leaf.value(0), Some(b"x".as_slice()));
    assert_eq!(
        leaf.value(1),
        Some(b"a run longer than twelve bytes".as_slice())
    );
    assert!(leaf.nulls().is_none());
    assert_eq!(leaf.array().len(), 2);
    assert_eq!(
        runs(&views),
        vec![
            Some(b"x".to_vec()),
            Some(b"a run longer than twelve bytes".to_vec())
        ]
    );
}

#[test]
fn a_fixed_width_byte_column_lends_one_payload_and_its_width() {
    let column = pairs();
    let leaf = column.as_fixed_bytes().expect("a fixed-width column");

    assert_eq!(leaf.width(), 2);
    assert_eq!(leaf.payload().as_slice(), b"abcd");
    assert_eq!(leaf.value(1), Some(b"cd".as_slice()));
    assert_eq!(leaf.value(2), None);
    assert!(leaf.nulls().is_none());
    assert_eq!(leaf.scalar(0).unwrap(), Scalar::from(b"ab".as_slice()));
    assert!(leaf.scalar(2).is_err());
    assert_eq!(SerieValue::len(leaf), 2);
    assert_eq!(leaf.slice(1, 1).unwrap().value(0), Some(b"cd".as_slice()));
    assert!(column.as_fixed_string().is_none());
}

#[test]
fn a_fixed_width_write_lands_at_its_slot_and_a_run_of_another_width_is_refused() {
    let mut column = pairs();

    column
        .set(0, Scalar::from(b"xy".as_slice()))
        .expect("one slot");
    column.push(Scalar::from(b"ef".as_slice())).expect("a row");
    column
        .insert(1, Scalar::from(b"12".as_slice()))
        .expect("a row in the middle");
    let leaf = column.as_fixed_bytes().expect("a fixed-width column");
    assert_eq!(leaf.payload().as_slice(), b"xy12cdef");
    assert_eq!(leaf.width(), 2);

    let refusal = column
        .push(Scalar::from(b"abc".as_slice()))
        .expect_err("three bytes are not two");
    assert!(
        refusal.to_string().contains("pair"),
        "the refusal names the column: {refusal}"
    );
    assert!(column.set(0, Scalar::Null).is_err());
    assert_eq!(
        column.as_fixed_bytes().unwrap().payload().as_slice(),
        b"xy12cdef"
    );

    assert_eq!(column.remove(1).unwrap(), Scalar::from(b"12".as_slice()));
    column.truncate(2).expect("rows 2.. dropped");
    assert_eq!(
        column.as_fixed_bytes().unwrap().payload().as_slice(),
        b"xycd"
    );
}

#[test]
fn a_fixed_width_slice_with_its_original_dropped_grows_into_a_freshly_built_array() {
    let field = Field::new("pair", DataType::fixed_binary(2).unwrap(), true);
    let rows = vec![
        Scalar::from(b"ab".as_slice()),
        Scalar::Null,
        Scalar::from(b"cd".as_slice()),
    ];
    let whole = Serie::from_scalars(field.clone(), rows.clone()).expect("three rows");
    let mut window = whole.slice(1, 2).expect("rows 1..3");
    drop(whole);

    window
        .push(Scalar::from(b"ef".as_slice()))
        .expect("a present row");
    window.push(Scalar::Null).expect("an absent row");
    window
        .set(0, Scalar::from(b"gh".as_slice()))
        .expect("a filled slot");
    assert_eq!(
        runs(&window),
        vec![
            Some(b"gh".to_vec()),
            Some(b"cd".to_vec()),
            Some(b"ef".to_vec()),
            None
        ]
    );
    assert_eq!(window.null_count(), 1);

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
fn a_uuid_column_is_a_fixed_width_byte_leaf_of_sixteen() {
    let one = Uuid::from_bytes(&[1_u8; 16]).expect("sixteen bytes");
    let two = Uuid::from_bytes(&[2_u8; 16]).expect("sixteen bytes");
    let mut column =
        Serie::from_scalars(Field::new("id", DataType::Uuid, false), [Scalar::Uuid(one)])
            .expect("one identity");

    column.push(Scalar::Uuid(two)).expect("a second identity");
    let leaf = column.as_fixed_bytes().expect("a fixed-width column");
    assert_eq!(leaf.width(), 16);
    assert_eq!(leaf.value(1), Some([2_u8; 16].as_slice()));
    assert_eq!(column.scalar(0).unwrap(), Scalar::Uuid(one));
    assert_eq!(column.pop().unwrap(), Some(Scalar::Uuid(two)));
    assert_eq!(column.len(), 1);
}
