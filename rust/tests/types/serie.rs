//! The column side of the value model: what a [`Serie`] owes the field that
//! types it, what its leaves lend off the buffers they hold, how those
//! buffers are written in place, and how a column reads as the sequence it
//! is.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int64Array, StringArray};
use yggdryl::{
    DataType, Field, Int32Serie, Int64Serie, Scalar, Sequence, Serie, SerieValue, StructType,
    Utf8StringSerie, Value,
};

/// One non-null 64-bit column of two prices, laid out from values.
fn prices() -> Serie {
    Serie::from_scalars(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(125_i64), Scalar::from(126_i64)],
    )
    .expect("two int64 rows")
}

/// The same two prices, taken straight off Arrow buffers.
fn price_buffers() -> Serie {
    let array: ArrayRef = Arc::new(Int64Array::from(vec![125, 126]));
    Serie::from_arrow_array(Field::new("price", DataType::Int64, false), array)
        .expect("an int64 run")
}

/// One non-null record root over an identifier and a symbol.
fn quotes_root() -> Field {
    let fields = StructType::from_fields([
        Field::new("id", DataType::Int64, false),
        Field::new("symbol", DataType::utf8(), false),
    ])
    .expect("two named children");
    Field::new("row", DataType::Struct(fields), false)
}

/// Two rows under [`quotes_root`], named rather than positional.
fn quotes_rows() -> Vec<Scalar> {
    vec![
        Scalar::from_struct([
            ("id", Scalar::from(1_i64)),
            ("symbol", Scalar::from("AAPL")),
        ])
        .expect("one record"),
        Scalar::from_struct([
            ("id", Scalar::from(2_i64)),
            ("symbol", Scalar::from("MSFT")),
        ])
        .expect("one record"),
    ]
}

#[test]
fn a_serie_rewrites_every_row_into_what_its_field_declares() {
    // The field declares int64 and the rows arrive as int8: the field's own
    // value contract rewrites each one, so the column holds the width it
    // declared and not the width it was handed.
    let column = Serie::from_scalars(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(1_i8), Scalar::from(2_i8)],
    )
    .expect("two rows the field accepts");

    assert_eq!(column.scalar(0).unwrap(), Scalar::from(1_i64));
    assert_eq!(column.scalar(1).unwrap(), Scalar::from(2_i64));
    assert_eq!(column.field().dtype(), &DataType::Int64);

    // And the buffers are the declared width, not a second reading of it.
    assert_eq!(
        column.as_int64().expect("an int64 column").values(),
        &[1, 2]
    );
}

#[test]
fn a_row_the_field_refuses_refuses_the_whole_column() {
    let refusal = Serie::from_scalars(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(1_i64), Scalar::Null],
    )
    .expect_err("a required column admits no absent row");
    assert!(
        refusal.to_string().contains("price"),
        "the refusal names the field: {refusal}"
    );

    // Nothing partial survives: the same rows under a nullable field are a
    // column, so it is the declaration that refused and not the value.
    let nullable = Serie::from_scalars(
        Field::new("price", DataType::Int64, true),
        [Scalar::from(1_i64), Scalar::Null],
    )
    .expect("a nullable column");
    assert_eq!(nullable.null_count(), 1);
    assert!(nullable.is_null(1));
}

#[test]
fn a_column_reads_its_rows_off_the_buffers_it_holds() {
    let column = price_buffers();

    // The leaf lends the values buffer itself - no row was built to answer
    // any of this.
    let leaf: &Int64Serie = column.as_int64().expect("an int64 column");
    assert_eq!(leaf.values(), &[125, 126]);
    assert_eq!(leaf.value(0), Some(125));
    assert_eq!(leaf.value(9), None);
    assert!(leaf.nulls().is_none());

    // And a value is built only where one is asked for.
    assert_eq!(column.scalar(1).unwrap(), Scalar::from(126_i64));
    assert_eq!(column.scalar(7).unwrap(), Scalar::Null);
    assert_eq!(column.len(), 2);
    assert!(!column.is_empty());
}

#[test]
fn a_text_column_lends_its_offsets_and_its_characters_where_they_lie() {
    let array: ArrayRef = Arc::new(StringArray::from(vec!["AAPL", "MSFT", "NVDA"]));
    let column = Serie::from_arrow_array(Field::new("symbol", DataType::utf8(), false), array)
        .expect("a utf8 run");

    let leaf: &Utf8StringSerie = column.as_utf8().expect("a utf8 column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 4, 8, 12]);
    assert_eq!(leaf.payload().as_slice(), b"AAPLMSFTNVDA");
    assert_eq!(leaf.value(1), Some("MSFT"));

    // The value side still answers, and it answers the same characters.
    assert_eq!(column.scalar(2).unwrap(), Scalar::from("NVDA"));
}

#[test]
fn the_typed_readers_span_the_widths_their_family_spans() {
    let narrow: ArrayRef = Arc::new(arrow_array::Int32Array::from(vec![1, 2, 3]));
    let column = Serie::from_arrow_array(Field::new("count", DataType::Int32, false), narrow)
        .expect("an int32 run");

    // One leaf per width, and a leaf answers only for the width it is.
    let leaf: &Int32Serie = column.as_int32().expect("an int32 column");
    assert_eq!(leaf.values(), &[1, 2, 3]);
    assert!(column.as_int64().is_none());
    assert!(column.as_utf8().is_none());

    // The family is one step above the leaf, and it is the same column.
    let family = column.as_integer().expect("the integer family");
    assert_eq!(SerieValue::len(family), 3);
    assert_eq!(family.scalar(2).unwrap(), Scalar::from(3_i32));
}

#[test]
fn a_column_grows_into_the_buffer_it_already_holds() {
    let mut column = price_buffers();
    let before = column.len();

    // A typed append writes the values buffer; nothing is rebuilt from rows.
    column
        .as_int64_mut()
        .expect("an int64 column")
        .push_value(Some(127));
    assert_eq!(column.len(), before + 1);
    assert_eq!(
        column.as_int64().expect("an int64 column").values(),
        &[125, 126, 127]
    );

    // And the value door writes the same buffer, through the field.
    column
        .push(Scalar::from(128_i16))
        .expect("the field narrows");
    assert_eq!(
        column.as_int64().expect("an int64 column").values(),
        &[125, 126, 127, 128]
    );

    // A value the field refuses leaves the column as it was.
    assert!(column.push(Scalar::from("AAPL")).is_err());
    assert_eq!(column.len(), 4);
}

#[test]
fn a_slot_is_rewritten_in_the_buffer_that_holds_it() {
    let mut column = price_buffers();

    column.set(0, Scalar::from(999_i64)).expect("a set row");
    assert_eq!(
        column.as_int64().expect("an int64 column").values(),
        &[999, 126]
    );

    // Past the end is refused by name rather than silently appended.
    let refusal = column
        .set(9, Scalar::from(1_i64))
        .expect_err("row 9 is past the end");
    assert!(
        refusal.to_string().contains("price"),
        "the refusal names the column: {refusal}"
    );

    // A required column refuses an absent value, and keeps what it had.
    assert!(column.set(0, Scalar::Null).is_err());
    assert_eq!(column.scalar(0).unwrap(), Scalar::from(999_i64));
}

#[test]
fn a_null_slot_is_written_and_read_back_as_absent() {
    let mut column = Serie::from_scalars(
        Field::new("price", DataType::Int64, true),
        [Scalar::from(1_i64), Scalar::from(2_i64)],
    )
    .expect("a nullable column");

    column.set(1, Scalar::Null).expect("an absent row");
    assert!(column.is_null(1));
    assert_eq!(column.null_count(), 1);
    assert_eq!(column.scalar(1).unwrap(), Scalar::Null);

    // And back again: the validity bitmap is rebuilt, the values stay.
    column.set(1, Scalar::from(3_i64)).expect("a present row");
    assert_eq!(column.null_count(), 0);
    assert_eq!(
        column.as_int64().expect("an int64 column").values(),
        &[1, 3]
    );
}

#[test]
fn a_run_whose_layout_is_not_the_fields_is_refused() {
    let text: ArrayRef = Arc::new(StringArray::from(vec!["AAPL"]));
    let refusal = Serie::from_arrow_array(Field::new("price", DataType::Int64, false), text)
        .expect_err("utf8 is not int64");
    assert!(
        refusal.to_string().to_lowercase().contains("int64"),
        "the refusal names the layout it wanted: {refusal}"
    );
}

#[test]
fn a_record_column_lends_its_children_and_takes_them_away() {
    let column = Serie::from_scalars(quotes_root(), quotes_rows()).expect("two records");
    let records = column.as_struct().expect("a record column");

    // Each child is a column of its own, with its own buffers.
    assert_eq!(records.children().len(), 2);
    assert_eq!(
        records
            .child("id")
            .expect("a named child")
            .as_int64()
            .expect("an int64 child")
            .values(),
        &[1, 2]
    );
    assert_eq!(
        records
            .child_at(1)
            .expect("a positional child")
            .field()
            .name(),
        "symbol"
    );
    assert!(records.child("volume").is_none());

    // Dropping one child drops it from the field too, and leaves the others'
    // buffers exactly where they were.
    let without = records.without_child("symbol").expect("a dropped child");
    assert_eq!(without.children().len(), 1);
    assert!(without.field().dtype().as_fields().unwrap().len() == 1);
    assert_eq!(
        without
            .child("id")
            .expect("the kept child")
            .as_int64()
            .expect("an int64 child")
            .values(),
        &[1, 2]
    );

    // And adding one back is the same move in reverse.
    let volumes: ArrayRef = Arc::new(Int64Array::from(vec![10, 20]));
    let child = Serie::from_arrow_array(Field::new("volume", DataType::Int64, false), volumes)
        .expect("an int64 child");
    let widened = without.with_child(&child).expect("a added child");
    assert_eq!(widened.children().len(), 2);
    assert_eq!(
        widened.scalar(0).unwrap(),
        Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(10_i64)])
    );

    // A child whose rows do not line up with the record's is refused.
    let short: ArrayRef = Arc::new(Int64Array::from(vec![1]));
    let mismatched =
        Serie::from_arrow_array(Field::new("bid", DataType::Int64, false), short).expect("one row");
    assert!(without.with_child(&mismatched).is_err());
}

#[test]
fn a_serie_reads_as_the_sequence_it_is() {
    let column = prices();
    let value = Scalar::from(column.clone());

    // A column is a sequence value, so everything a sequence answers, it
    // answers - the rows decoded once, on the first ask.
    assert_eq!(value.len(), 2);
    assert_eq!(
        value.as_sequence().expect("a sequence"),
        &[Scalar::from(125_i64), Scalar::from(126_i64)]
    );
    assert_eq!(value.iter().count(), 2);
    assert_eq!(value.kind(), "serie");

    // The leaf is reachable both ways round.
    let sequence = match &value {
        Scalar::Sequence(sequence) => sequence,
        other => panic!("a column is a sequence value, got {other:?}"),
    };
    assert!(sequence.as_list().is_none());
    assert_eq!(sequence.as_serie().expect("a column").len(), 2);
    assert_eq!(sequence.row_count(), 2);
}

#[test]
fn the_rows_are_decoded_once_and_the_column_is_unchanged_by_being_read() {
    let value = Scalar::from(price_buffers());
    let sequence = match &value {
        Scalar::Sequence(sequence) => sequence,
        other => panic!("a column is a sequence value, got {other:?}"),
    };
    let rows = sequence.as_serie_rows().expect("a column read as rows");

    // Nothing is decoded until a row is asked for.
    assert!(rows.as_slice().is_none());
    assert_eq!(rows.len(), 2);

    // The first ask decodes, and every ask after it lends the same rows.
    let first = rows.rows().expect("readable rows").as_ptr();
    assert_eq!(rows.rows().expect("readable rows").as_ptr(), first);
    assert_eq!(rows.as_slice().expect("decoded rows").len(), 2);

    // And the column itself still holds buffers, not rows.
    assert_eq!(
        rows.column().as_int64().expect("an int64 column").values(),
        &[125, 126]
    );
}

#[test]
fn a_serie_names_its_datatype_where_an_empty_sequence_cannot() {
    let empty = Serie::empty(Field::new("price", DataType::Int64, false)).expect("an empty column");
    assert!(empty.is_empty());
    assert_eq!(
        empty.dtype().unwrap(),
        DataType::list(Field::new("price", DataType::Int64, false))
    );

    // A schema-free run of nothing cannot say the same.
    let run = Scalar::from_sequence([]);
    assert_ne!(run.dtype().unwrap(), empty.dtype().unwrap());
}

#[test]
fn a_column_and_the_run_it_holds_order_by_their_rows_and_are_not_one_value() {
    let column = Sequence::from(prices());
    let run = Sequence::new(vec![Scalar::from(125_i64), Scalar::from(126_i64)]);

    // A run lends its rows where it holds them; a column has none to lend
    // until one is asked for.
    assert!(run.as_slice().is_some());
    assert!(column.as_slice().is_none());
    assert_eq!(column.rows().unwrap().len(), 2);
    assert!(column.as_slice().is_some(), "the first ask decodes");

    // The rows are the same, so neither sorts away from the other; the leaf
    // only breaks the tie, and they are not equal.
    assert_ne!(column, run);
    assert!(run < column);
    assert_eq!(run.rows().unwrap(), column.rows().unwrap());
}

#[test]
fn the_family_narrows_and_widens_through_the_serie_root() {
    let column = price_buffers();

    // Widen a leaf to the root and narrow it back: one column throughout.
    let leaf = column.as_int64().expect("an int64 column").clone();
    let widened = leaf.clone().into_serie();
    assert_eq!(widened, column);
    assert_eq!(Int64Serie::from_serie(&widened), Some(&leaf));
    assert_eq!(Int32Serie::from_serie(&widened), None);
}

#[test]
fn a_column_survives_the_value_contract_until_that_contract_rewrites_it() {
    let column = prices();
    let value = Scalar::from(column.clone());

    // A column carried as a value is the same column on the way back.
    assert_eq!(
        Sequence::from_scalar(&value).and_then(Sequence::as_serie),
        Some(&column)
    );

    // Dropping to the schema-free run is the one direction that loses the
    // field, and it is spelled rather than implied.
    let run = column.clone().into_sequence().expect("readable rows");
    assert_eq!(
        run,
        Scalar::from_sequence([Scalar::from(125_i64), Scalar::from(126_i64)])
    );
    assert!(matches!(run, Scalar::Sequence(Sequence::List(_))));
}

#[test]
fn a_column_crosses_into_one_arrow_array_and_back() {
    let column = price_buffers();
    let array = column.into_arrow_array();
    assert_eq!(array.len(), 2);

    let back = Serie::from_arrow_array(column.field().clone(), array).expect("the same buffers");
    assert_eq!(back, column);
}

#[test]
fn a_record_root_column_crosses_into_one_batch_and_back() {
    let column = Serie::from_scalars(quotes_root(), quotes_rows()).expect("two records");
    let batch = column.into_arrow_batch().expect("a record root");

    assert_eq!(batch.num_rows(), 2);
    assert_eq!(batch.num_columns(), 2);

    let back = Serie::from_arrow_batch(&batch).expect("two columns of one length");
    assert_eq!(back.scalars().unwrap(), column.scalars().unwrap());
}

#[test]
fn the_column_wire_carries_the_field_beside_the_rows() {
    let column = prices();
    let document = serde_json::to_string(&column).expect("a column document");
    assert!(
        document.contains("price"),
        "the field travels with the rows: {document}"
    );

    let back: Serie = serde_json::from_str(&document).expect("the same column");
    assert_eq!(back, column);
}

#[test]
fn the_sequence_family_reads_back_whichever_leaf_a_document_holds() {
    let column = Scalar::from(prices());
    let run = Scalar::from_sequence([Scalar::from(125_i64), Scalar::from(126_i64)]);

    for value in [column, run] {
        let document = serde_json::to_string(&value).expect("a sequence document");
        let back: Scalar = serde_json::from_str(&document).expect("the same sequence");
        assert_eq!(back, value, "through {document}");
    }
}

#[test]
fn a_column_clone_shares_its_buffers_rather_than_copying_them() {
    let column = price_buffers();
    let copy = column.clone();

    let one = column.into_arrow_array();
    let other = copy.into_arrow_array();
    assert!(
        std::ptr::eq(
            one.to_data().buffers()[0].as_ptr(),
            other.to_data().buffers()[0].as_ptr(),
        ),
        "a clone shares the values buffer"
    );
}

#[test]
fn a_record_column_writes_its_own_validity_and_leaves_its_children_alone() {
    let nullable = Field::new("row", quotes_root().dtype().clone(), true);
    let mut records =
        Serie::from_scalars(nullable, quotes_rows()).expect("two records under a nullable root");

    // A null record row is the record's own validity bit; Arrow leaves the
    // children's slots unspecified under it, so nothing below is rewritten.
    records.set(0, Scalar::Null).expect("an absent record");
    assert!(records.is_null(0));
    assert_eq!(records.scalar(0).unwrap(), Scalar::Null);
    assert_eq!(
        records
            .as_struct()
            .expect("a record column")
            .child("id")
            .expect("a named child")
            .as_int64()
            .expect("an int64 child")
            .values(),
        &[1, 2],
        "the child buffers are untouched by the record's own absence"
    );

    // And a null row appends as one slot in every child.
    records.push(Scalar::Null).expect("an absent record");
    assert_eq!(records.len(), 3);
    assert!(records.is_null(2));
    assert_eq!(
        records
            .as_struct()
            .expect("a record column")
            .child("id")
            .expect("a named child")
            .len(),
        3,
        "every child is as long as the record is"
    );

    // The buffers still cross as one batch, absent rows and all.
    let array = records.into_arrow_array();
    assert_eq!(array.len(), 3);
    assert_eq!(array.null_count(), 2);

    // A required root refuses the same row rather than writing a bit.
    let mut required = Serie::from_scalars(quotes_root(), quotes_rows()).expect("two records");
    assert!(required.set(0, Scalar::Null).is_err());
    assert!(required.push(Scalar::Null).is_err());
}

#[test]
fn a_record_row_that_does_not_fit_the_children_is_refused_by_name() {
    let mut records = Serie::from_scalars(quotes_root(), quotes_rows()).expect("two records");

    // A row of the wrong width never reaches a child: the record refuses it
    // whole, naming itself.
    let refusal = records
        .push(Scalar::from_sequence([Scalar::from(3_i64)]))
        .expect_err("one cell does not fit two children");
    assert!(
        refusal.to_string().contains("row"),
        "the refusal names the record: {refusal}"
    );
    assert_eq!(records.len(), 2);
}

#[test]
fn a_list_column_cuts_one_item_column_and_writes_its_own_validity() {
    let item = Field::new("item", DataType::Int64, false);
    let mut lists = Serie::from_scalars(
        Field::new("prices", DataType::list(item), true),
        [
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i64)]),
            Scalar::from_sequence([Scalar::from(3_i64)]),
        ],
    )
    .expect("two list rows");

    // Every item of every row is one column, and the offsets say where each
    // row starts - so reading them all reads one buffer.
    let column = lists.as_list().expect("a list column");
    assert_eq!(
        column
            .items()
            .as_int64()
            .expect("an int64 item column")
            .values(),
        &[1, 2, 3]
    );
    assert_eq!(column.range(0), Some((0, 2)));
    assert_eq!(column.range(1), Some((2, 3)));

    // An absent row is the validity bit; an empty present row is the same
    // cut, which is what makes the bitmap the only thing telling them apart.
    lists.push(Scalar::Null).expect("an absent row");
    lists
        .push(Scalar::from_sequence([]))
        .expect("an empty present row");
    assert_eq!(lists.len(), 4);
    assert!(lists.is_null(2));
    assert!(!lists.is_null(3));
    assert_eq!(lists.scalar(2).unwrap(), Scalar::Null);
    assert_eq!(lists.scalar(3).unwrap(), Scalar::from_sequence([]));

    // And the buffers still cross, absent rows and all.
    let array = lists.into_arrow_array();
    assert_eq!(array.len(), 4);
    assert_eq!(array.null_count(), 1);
}

#[test]
fn a_list_row_that_does_not_fit_its_cut_is_refused_rather_than_moving_every_later_row() {
    let item = Field::new("item", DataType::Int64, false);
    let mut lists = Serie::from_scalars(
        Field::new("prices", DataType::list(item), false),
        [Scalar::from_sequence([
            Scalar::from(1_i64),
            Scalar::from(2_i64),
        ])],
    )
    .expect("one list row");

    // Rewriting in place is what a cut allows; a longer row would move every
    // later row, so it is refused by name instead.
    lists
        .set(
            0,
            Scalar::from_sequence([Scalar::from(9_i64), Scalar::from(8_i64)]),
        )
        .expect("the same length");
    assert_eq!(
        lists
            .as_list()
            .expect("a list column")
            .items()
            .as_int64()
            .expect("an int64 item column")
            .values(),
        &[9, 8]
    );

    let refusal = lists
        .set(0, Scalar::from_sequence([Scalar::from(1_i64)]))
        .expect_err("a shorter row does not fit the cut");
    assert!(
        refusal.to_string().contains("prices"),
        "the refusal names the column: {refusal}"
    );
}

#[test]
fn a_mapping_column_holds_its_entries_as_a_record_column() {
    let pair = Field::new(
        "entries",
        DataType::Struct(
            StructType::from_fields([
                Field::new("key", DataType::utf8(), false),
                Field::new("value", DataType::Int64, false),
            ])
            .expect("a key and a value"),
        ),
        false,
    );
    let entries = DataType::map(pair, false).expect("a mapping datatype");
    let column = Serie::from_scalars(
        Field::new("weights", entries, false),
        [Scalar::from_mapping([(Scalar::from("AAPL"), Scalar::from(1_i64))]).expect("one entry")],
    )
    .expect("one mapping row");

    let maps = column.as_map().expect("a mapping column");
    assert_eq!(maps.range(0), Some((0, 1)));

    // The entries are a record column, so the keys and the values are each a
    // column of their own.
    let pairs = maps.entries().as_struct().expect("a record entry column");
    assert_eq!(
        pairs
            .child("key")
            .expect("the key column")
            .as_utf8()
            .expect("a utf8 key column")
            .value(0),
        Some("AAPL")
    );
    assert_eq!(
        pairs
            .child("value")
            .expect("the value column")
            .as_int64()
            .expect("an int64 value column")
            .values(),
        &[1]
    );

    // And a row reads back as the mapping it is.
    assert_eq!(
        column.scalar(0).unwrap(),
        Scalar::from_mapping([(Scalar::from("AAPL"), Scalar::from(1_i64))]).unwrap()
    );
}

#[test]
fn a_variant_column_lends_the_run_it_encoded_and_decodes_only_on_demand() {
    let mut column = Serie::from_scalars(
        Field::new("payload", DataType::Variant, true),
        [Scalar::from(1_i64), Scalar::from("AAPL")],
    )
    .expect("two variant rows");

    let leaf = column.as_variant().expect("a variant column");
    let first = leaf.bytes(0).expect("an encoded run").to_vec();
    assert!(!first.is_empty());
    assert!(!leaf.payload().is_empty());
    assert!(leaf.offsets().is_some(), "the default cut is 32-bit");

    // A row is a value only when one is asked for, and it comes back what it
    // went in as.
    assert_eq!(column.scalar(0).unwrap(), Scalar::from(1_i64));
    assert_eq!(column.scalar(1).unwrap(), Scalar::from("AAPL"));

    // An already-encoded run is forwarded without being decoded.
    column
        .as_variant_mut()
        .expect("a variant column")
        .push_bytes(Some(&first));
    assert_eq!(column.len(), 3);
    assert_eq!(column.scalar(2).unwrap(), Scalar::from(1_i64));

    // And an absent row is a slot with no run in it.
    column.push(Scalar::Null).expect("an absent row");
    assert!(column.is_null(3));
    assert_eq!(
        column.as_variant().expect("a variant column").bytes(3),
        None
    );
}
