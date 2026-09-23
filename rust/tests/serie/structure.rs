//! `rust/src/serie/structure.rs`: the record leaf - one child column per
//! child field, beside the record's own validity.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BooleanArray, Int64Array, StructArray};
use arrow_buffer::NullBuffer;
use arrow_data::ArrayData;
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
use yggdryl::{
    ArrowCastOptions, DataType, Field, FieldPath, Nullability, Scalar, Serie, SerieValue,
    StructType,
};

/// A nullable record root over an identifier and a flag.
fn quotes_root() -> Field {
    Field::new(
        "row",
        DataType::from(
            StructType::from_fields([
                Field::new("id", DataType::Int64, false),
                Field::new("live", DataType::Boolean, false),
            ])
            .expect("two named children"),
        ),
        true,
    )
}

/// Two quote rows and an absent one under [`quotes_root`].
fn quotes() -> Serie {
    Serie::from_scalars(
        quotes_root(),
        [
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(true)]),
            Scalar::Null,
            Scalar::from_struct([("id", Scalar::from(2_i64)), ("live", Scalar::from(false))])
                .expect("one record"),
        ],
    )
    .expect("three record rows")
}

/// A nullable record three levels deep: `id`, and `venue.tier.level`.
fn venues_root() -> Field {
    let tier = Field::new(
        "tier",
        DataType::from(
            StructType::from_fields([Field::new("level", DataType::Int64, false)]).unwrap(),
        ),
        false,
    );
    let venue = Field::new(
        "venue",
        DataType::from(StructType::from_fields([tier]).unwrap()),
        true,
    );
    Field::new(
        "row",
        DataType::from(
            StructType::from_fields([Field::new("id", DataType::Int64, false), venue]).unwrap(),
        ),
        true,
    )
}

/// Three rows under [`venues_root`]: a full one, one whose venue is absent,
/// and an absent one.
fn venues() -> Serie {
    Serie::from_scalars(
        venues_root(),
        [
            Scalar::from_sequence([
                Scalar::from(1_i64),
                Scalar::from_sequence([Scalar::from_sequence([Scalar::from(3_i64)])]),
            ]),
            Scalar::from_sequence([Scalar::from(2_i64), Scalar::Null]),
            Scalar::Null,
        ],
    )
    .expect("three rows")
}

/// The pointer of every buffer under `data`, children included.
fn buffer_pointers(data: &ArrayData, pointers: &mut Vec<*const u8>) {
    pointers.extend(data.buffers().iter().map(arrow_buffer::Buffer::as_ptr));
    for child in data.child_data() {
        buffer_pointers(child, pointers);
    }
}

/// The pointer of every buffer under every child of `column`: what "no
/// buffer moved" is checked against, the arrays dropped again so the column
/// holds its buffers alone.
fn child_buffers(column: &Serie) -> Vec<*const u8> {
    let mut pointers = Vec::new();
    for child in column.children() {
        buffer_pointers(
            &child.into_arrow_array().expect("a column").to_data(),
            &mut pointers,
        );
    }
    pointers
}

#[test]
fn set_child_refuses_a_run_a_length_mismatch_and_a_column_that_is_not_a_record() {
    let mut column = quotes();
    let before = child_buffers(&column);

    let refusal = column
        .set_child(Serie::new(vec![Scalar::from(1_i64)]))
        .expect_err("a run declares no field");
    assert!(
        refusal.to_string().contains("declares no field"),
        "{refusal}"
    );

    let short: ArrayRef = Arc::new(Int64Array::from(vec![10]));
    let short = Serie::from_arrow_array(
        Some(&Field::new("volume", DataType::Int64, false)),
        short,
        ArrowCastOptions::new(),
    )
    .expect("an int64 column");
    let refusal = column
        .set_child(short)
        .expect_err("one row does not fit three");
    let text = refusal.to_string();
    assert!(text.contains("row"), "names the column: {text}");
    assert!(
        text.contains('1') && text.contains('3'),
        "both counts: {text}"
    );
    assert_eq!(column.children().len(), 2);
    assert_eq!(child_buffers(&column), before);

    let mut price = Serie::from_scalars(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(1_i64)],
    )
    .unwrap();
    let volume = Serie::from_scalars(
        Field::new("volume", DataType::Int64, false),
        [Scalar::from(1_i64)],
    )
    .unwrap();
    let refusal = price
        .set_child(volume)
        .expect_err("an int64 column holds no child");
    assert!(
        refusal.to_string().contains("price"),
        "names the column: {refusal}"
    );
    assert!(
        Serie::new(vec![Scalar::from(1_i64)])
            .set_child(price.clone())
            .is_err()
    );
}

#[test]
fn set_cell_writes_three_levels_deep_in_place_and_refuses_what_it_cannot_reach() {
    let mut column = venues();
    let path = FieldPath::from_str("venue.tier.level").unwrap();
    let before = child_buffers(&column);

    // An absent row at the root, and an absent record on the way, each
    // refuse naming the record whose row is absent.
    let refusal = column
        .set_cell(&path, 2, Scalar::from(9_i64))
        .expect_err("row 2 is absent at the root");
    let text = refusal.to_string();
    assert!(text.contains("row 2 of row is absent"), "{text}");
    let refusal = column
        .set_cell(&path, 1, Scalar::from(9_i64))
        .expect_err("row 1 has no venue");
    let text = refusal.to_string();
    assert!(text.contains("row 1 of venue is absent"), "{text}");

    // A stranger segment, a level that is not a record, an index segment, a
    // row past the end, and a value the leaf refuses.
    let refusal = column
        .set_cell(
            &FieldPath::from_str("venue.size").unwrap(),
            0,
            Scalar::from(9_i64),
        )
        .expect_err("venue has no size");
    assert!(
        refusal.to_string().contains("no child named \"size\""),
        "{refusal}"
    );
    let refusal = column
        .set_cell(
            &FieldPath::from_str("id.level").unwrap(),
            0,
            Scalar::from(9_i64),
        )
        .expect_err("id is a leaf");
    assert!(
        refusal.to_string().contains("not a record column"),
        "{refusal}"
    );
    assert!(
        column
            .set_cell(&FieldPath::from_str("[0]").unwrap(), 0, Scalar::from(9_i64))
            .is_err()
    );
    assert!(column.set_cell(&path, 3, Scalar::from(9_i64)).is_err());
    assert!(column.set_cell(&path, 0, Scalar::from("nine")).is_err());
    assert!(column.set_cell(&path, 0, Scalar::Null).is_err());
    assert_eq!(child_buffers(&column), before);
    assert_eq!(column, venues());

    // One present slot over one present slot is one buffer write: the leaf's
    // buffer stays where it was, and every other row reads as before.
    column
        .set_cell(&path, 0, Scalar::from(7_i64))
        .expect("a cell three levels down");
    assert_eq!(child_buffers(&column), before);
    assert_eq!(
        column.scalar(0).unwrap(),
        Scalar::from_sequence([
            Scalar::from(1_i64),
            Scalar::from_sequence([Scalar::from_sequence([Scalar::from(7_i64)])]),
        ])
    );
    assert_eq!(column.scalar(1).unwrap(), venues().scalar(1).unwrap());
    assert_eq!(column.scalar(2).unwrap(), Scalar::Null);
    assert_eq!(
        column
            .get_child_by_path(&path)
            .and_then(Serie::as_int64)
            .map(|leaf| leaf.values().to_vec()),
        Some(vec![7, 0, 0])
    );

    // A leaf one level down is the same write, and a run or a leaf column
    // holds no cell at all.
    column
        .set_cell(&FieldPath::from_str("id").unwrap(), 1, Scalar::from(20_i64))
        .expect("a cell one level down");
    assert_eq!(
        column
            .child("id")
            .and_then(Serie::as_int64)
            .unwrap()
            .values(),
        &[1, 20, 0]
    );
    assert!(
        Serie::new(vec![Scalar::from(1_i64)])
            .set_cell(&path, 0, Scalar::from(1_i64))
            .is_err()
    );
    let mut leaf = Serie::from_scalars(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(1_i64)],
    )
    .unwrap();
    let refusal = leaf
        .set_cell(&path, 0, Scalar::from(1_i64))
        .expect_err("a leaf holds no cell");
    assert!(refusal.to_string().contains("price"), "{refusal}");
}

#[test]
fn a_required_child_under_a_null_record_row_is_admitted_at_the_door_and_by_a_push() {
    // At the door: the child's absent slot is hidden by the record's null.
    let child = ArrowField::new("price", ArrowDataType::Int64, false);
    let records: ArrayRef = Arc::new(StructArray::new(
        vec![Arc::new(child)].into(),
        vec![Arc::new(Int64Array::from(vec![Some(1), None]))],
        Some(NullBuffer::from(vec![true, false])),
    ));
    let root = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([Field::new("price", DataType::Int64, false)]).unwrap(),
        ),
        true,
    );
    let strict = ArrowCastOptions::new()
        .with_safe(false)
        .with_nullability(Nullability::Strict);
    let column = Serie::from_arrow_array(Some(&root), records, strict)
        .expect("a hidden absent child, even strictly");
    assert!(column.is_null(1).unwrap());
    assert_eq!(column.child("price").unwrap().null_count(), 1);

    // By a push: the same slot, written as the child's placeholder.
    let mut pushed = Serie::empty(root).unwrap();
    pushed
        .push(Scalar::from_sequence([Scalar::from(1_i64)]))
        .unwrap();
    pushed.push(Scalar::Null).expect("an absent record row");
    assert_eq!(pushed, column);
    assert_eq!(
        pushed.into_arrow_array().unwrap().as_ref(),
        column.into_arrow_array().unwrap().as_ref()
    );

    // A required root admits no absent row, and says so by name.
    let required = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([Field::new("price", DataType::Int64, false)]).unwrap(),
        ),
        false,
    );
    let refusal = Serie::from_scalars(required.clone(), [Scalar::Null])
        .expect_err("a required record admits no absent row");
    assert!(refusal.to_string().contains("row"), "{refusal}");
    let mut empty = Serie::empty(required).unwrap();
    assert!(empty.push(Scalar::Null).is_err());
    assert!(empty.is_empty());
}

#[test]
fn a_column_built_by_pushes_is_the_from_scalars_column_and_the_array_built_from_the_rows() {
    let laid_out = quotes();
    let mut pushed = Serie::empty(quotes_root()).unwrap();
    for row in laid_out.rows().iter().cloned() {
        pushed.push(row).expect("a row the field accepts");
    }

    // A null record row clears the record's bit and gives every child one
    // placeholder slot: the array built from the same rows by hand.
    let expected: ArrayRef = Arc::new(StructArray::new(
        vec![
            Arc::new(ArrowField::new("id", ArrowDataType::Int64, false)),
            Arc::new(ArrowField::new("live", ArrowDataType::Boolean, false)),
        ]
        .into(),
        vec![
            Arc::new(Int64Array::from(vec![Some(1), None, Some(2)])),
            Arc::new(BooleanArray::from(vec![Some(true), None, Some(false)])),
        ],
        Some(NullBuffer::from(vec![true, false, true])),
    ));
    assert_eq!(pushed, laid_out);
    assert_eq!(
        pushed.into_arrow_array().unwrap().as_ref(),
        expected.as_ref()
    );
    assert_eq!(
        laid_out.into_arrow_array().unwrap().as_ref(),
        expected.as_ref()
    );
    assert_eq!(pushed.null_count(), 1);

    // A row that does not fit the children is refused, and nothing moved.
    let before = child_buffers(&pushed);
    assert!(
        pushed
            .push(Scalar::from_sequence([Scalar::from(3_i64)]))
            .is_err()
    );
    assert!(pushed.push(Scalar::from(true)).is_err());
    assert_eq!(pushed.len(), 3);
    assert_eq!(child_buffers(&pushed), before);

    // The other writes are the one splice spelled differently.
    pushed
        .insert(
            0,
            Scalar::from_sequence([Scalar::from(0_i64), Scalar::from(false)]),
        )
        .unwrap();
    assert_eq!(pushed.remove(2).unwrap(), Scalar::Null);
    assert_eq!(
        pushed.pop().unwrap(),
        Some(Scalar::from_sequence([
            Scalar::from(2_i64),
            Scalar::from(false)
        ]))
    );
    assert_eq!(
        pushed
            .child("id")
            .and_then(Serie::as_int64)
            .unwrap()
            .values(),
        &[0, 1]
    );
    assert_eq!(pushed.null_count(), 0);
    pushed
        .resize(4, Scalar::Null)
        .expect("two absent rows appended");
    assert_eq!(pushed.null_count(), 2);
    assert_eq!(pushed.child("live").unwrap().len(), 4);
    pushed.clear().unwrap();
    assert!(pushed.is_empty());
    assert_eq!(pushed.child("id").map(Serie::len), Some(0));
}

#[test]
fn a_splice_whose_utf8_child_would_overflow_its_offsets_is_refused_and_no_buffer_moves() {
    let root = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([
                Field::new("id", DataType::Int64, false),
                Field::new("symbol", DataType::utf8(), false),
            ])
            .unwrap(),
        ),
        false,
    );
    let mut column = Serie::from_scalars(
        root,
        [
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]),
            Scalar::from_sequence([Scalar::from(2_i64), Scalar::from("MSFT")]),
        ],
    )
    .expect("two rows");
    let before = child_buffers(&column);

    // Refused by the proof, before any child is asked: a required child
    // admits no absent cell.
    assert!(
        column
            .push(Scalar::from_sequence([Scalar::from(3_i64), Scalar::Null]))
            .is_err()
    );
    assert_eq!(child_buffers(&column), before);

    // Refused by the utf8 child's check: 129 rows of 16 MiB reach past the
    // 2 GiB a 32-bit offset addresses. The text is one shared allocation
    // and each row a pointer bump, so nothing near that is allocated.
    let text = Scalar::from("x".repeat(16 << 20));
    let wide = Scalar::from_sequence([Scalar::from(3_i64), text]);
    let refusal = column
        .extend(vec![wide; 129])
        .expect_err("the payload would overflow int32 offsets");
    let text = refusal.to_string();
    assert!(text.contains("symbol"), "names the child: {text}");
    assert_eq!(column.len(), 2);
    assert_eq!(child_buffers(&column), before);
    assert_eq!(
        column
            .child("id")
            .and_then(Serie::as_int64)
            .unwrap()
            .values(),
        &[1, 2]
    );
}

#[test]
fn extend_from_serie_appends_child_by_child_and_a_slice_shares_the_children() {
    let mut column = quotes();
    let more = Serie::from_scalars(
        quotes_root(),
        [
            Scalar::from_sequence([Scalar::from(4_i64), Scalar::from(true)]),
            Scalar::Null,
        ],
    )
    .unwrap();

    column
        .extend_from_serie(&more)
        .expect("one layout appended");
    assert_eq!(column.len(), 5);
    assert_eq!(column.null_count(), 2);
    assert_eq!(
        column
            .child("id")
            .and_then(Serie::as_int64)
            .unwrap()
            .values(),
        &[1, 0, 2, 4, 0]
    );
    assert_eq!(column.scalar(4).unwrap(), Scalar::Null);
    assert_eq!(column.rows()[..3], quotes().rows()[..]);

    // A required root reads the other column's rows and refuses its absent
    // one, leaving itself as it was.
    let mut required = Serie::from_scalars(
        quotes_root().with_nullable(false),
        [Scalar::from_sequence([
            Scalar::from(9_i64),
            Scalar::from(true),
        ])],
    )
    .unwrap();
    assert!(required.extend_from_serie(&more).is_err());
    assert_eq!(required.len(), 1);

    // A window slices every child to the same rows, and grows on its own.
    let mut window = column.slice(1, 2).unwrap();
    assert_eq!(window.len(), 2);
    assert!(window.is_null(0).unwrap());
    assert_eq!(window.children()[1].len(), 2);
    drop(column);
    window
        .push(Scalar::from_sequence([
            Scalar::from(5_i64),
            Scalar::from(false),
        ]))
        .unwrap();
    assert_eq!(
        window
            .child("id")
            .and_then(Serie::as_int64)
            .unwrap()
            .values(),
        &[0, 2, 5]
    );
    assert_eq!(window.null_count(), 1);
}

#[test]
fn a_record_column_lends_its_children_and_reads_a_row_out_of_them() {
    let column = quotes();
    let records = column.as_struct().expect("a record column");

    assert_eq!(records.children().len(), 2);
    assert_eq!(
        records.child("id").unwrap().as_int64().unwrap().values(),
        &[1, 0, 2]
    );
    assert_eq!(
        records.child_at(1).and_then(Serie::field).map(Field::name),
        Some("live")
    );
    assert!(records.child("volume").is_none());
    assert_eq!(records.nulls().map(|nulls| nulls.null_count()), Some(1));
    assert_eq!(records.null_count(), 1);
    assert!(records.is_null(1).unwrap());
    assert_eq!(records.scalar(1).unwrap(), Scalar::Null);
    assert_eq!(
        records.scalar(2).unwrap(),
        Scalar::from_sequence([Scalar::from(2_i64), Scalar::from(false)])
    );
    assert!(records.scalar(3).is_err());

    let window = records.slice(1, 2).unwrap();
    assert_eq!(SerieValue::len(&window), 2);
    assert!(window.is_null(0).unwrap());
    assert_eq!(window.children()[0].len(), 2);
    assert_eq!(records.into_arrow_array().len(), 3);
}

#[test]
fn a_child_is_dropped_added_and_replaced_by_name() {
    let column = quotes();
    let records = column.as_struct().expect("a record column");

    let without = records.without_child("live").expect("a dropped child");
    assert_eq!(without.children().len(), 1);
    assert_eq!(SerieValue::field(&without).fields().len(), 1);
    assert!(records.without_child("volume").is_err());

    let volumes: ArrayRef = Arc::new(Int64Array::from(vec![10, 20, 30]));
    let volume = Serie::from_arrow_array(
        Some(&Field::new("volume", DataType::Int64, false)),
        volumes,
        ArrowCastOptions::new(),
    )
    .unwrap();
    let mut widened = without.clone();
    widened.set_child(volume).expect("a child added");
    assert_eq!(widened.children().len(), 2);
    assert_eq!(
        widened.scalar(0).unwrap(),
        Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(10_i64)])
    );

    // The same name replaces, and the field follows the child's own.
    let volumes: ArrayRef = Arc::new(Int64Array::from(vec![Some(11), None, Some(33)]));
    let volume = Serie::from_arrow_array(
        Some(&Field::new("volume", DataType::Int64, true)),
        volumes,
        ArrowCastOptions::new(),
    )
    .unwrap();
    widened.set_child(volume).expect("a child replaced");
    assert_eq!(widened.children().len(), 2);
    assert!(
        SerieValue::field(&widened)
            .fields()
            .iter()
            .find(|child| child.name() == "volume")
            .is_some_and(Field::is_nullable)
    );
    assert_eq!(
        widened.scalar(2).unwrap(),
        Scalar::from_sequence([Scalar::from(2_i64), Scalar::from(33_i64)])
    );
    assert_eq!(widened.into_arrow_array().len(), 3);
}
