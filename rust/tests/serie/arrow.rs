//! `rust/src/serie/arrow.rs`: the one door buffers take into a column -
//! what it proves, what it refuses by name, and what crosses back out.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int64Array, ListArray, RecordBatch, StringArray, StructArray};
use arrow_buffer::{NullBuffer, OffsetBuffer};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
use yggdryl::{DataType, Field, Scalar, Serie, StructType};

/// A non-null record root over an identifier and a symbol.
fn quotes_root() -> Field {
    Field::new(
        "row",
        DataType::from(
            StructType::from_fields([
                Field::new("id", DataType::Int64, false),
                Field::new("symbol", DataType::utf8(), false),
            ])
            .expect("two named children"),
        ),
        false,
    )
}

/// Two quote rows as one Arrow batch.
fn quote_batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        ArrowField::new("id", ArrowDataType::Int64, false),
        ArrowField::new("symbol", ArrowDataType::Utf8, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
        ],
    )
    .expect("two rows")
}

/// A required int64 list field, and three lists - `[1, 2]`, `[3]`,
/// `[4, 5]` - over one child of five.
fn legs() -> (Field, ListArray) {
    let field = Field::new(
        "legs",
        DataType::list(Field::new("item", DataType::Int64, false)),
        false,
    );
    let lists = ListArray::new(
        Arc::new(ArrowField::new("item", ArrowDataType::Int64, false)),
        OffsetBuffer::new(vec![0_i32, 2, 3, 5].into()),
        Arc::new(Int64Array::from(vec![1_i64, 2, 3, 4, 5])),
        None,
    );
    (field, lists)
}

#[test]
fn the_buffers_cross_in_as_they_are_and_back_out_shared() {
    let array = Int64Array::from((0..1_024_i64).collect::<Vec<_>>());
    let values = array.values().as_ptr();
    let held: ArrayRef = Arc::new(array);
    let column = Serie::from_arrow_array(Field::new("price", DataType::Int64, false), held)
        .expect("an int64 column");

    // The layout is the datatype's whole contract, so no row is read: the
    // column lends the very buffer the array held.
    assert_eq!(column.len(), 1_024);
    assert_eq!(column.as_int64().unwrap().values().as_ptr(), values);
    assert_eq!(column.scalar(1).unwrap(), Scalar::from(1_i64));

    let back = column.into_arrow_array().expect("a column");
    assert_eq!(back.data_type(), &ArrowDataType::Int64);
    assert_eq!(back.len(), 1_024);
    assert!(
        back.to_data().buffers()[0].as_ptr() == values.cast::<u8>(),
        "the way out shares the buffer too"
    );
    assert!(column.require_arrow_array().is_ok());
}

#[test]
fn a_layout_that_is_not_the_fields_is_refused_naming_both() {
    let text: ArrayRef = Arc::new(StringArray::from(vec!["AAPL"]));
    let refusal = Serie::from_arrow_array(Field::new("price", DataType::Int64, false), text)
        .expect_err("utf8 is not int64");
    let text = refusal.to_string();
    assert!(text.contains("price"), "names the column: {text}");
    assert!(text.contains("Int64"), "names the layout it wanted: {text}");
    assert!(text.contains("Utf8"), "names the layout it got: {text}");
}

#[test]
fn an_absent_row_is_refused_under_a_required_field_and_admitted_under_a_nullable_one() {
    let absent: ArrayRef = Arc::new(Int64Array::from(vec![Some(1), None]));
    let refusal = Serie::from_arrow_array(
        Field::new("price", DataType::Int64, false),
        Arc::clone(&absent),
    )
    .expect_err("a required column admits no absent row");
    let text = refusal.to_string();
    assert!(text.contains("price"), "names the column: {text}");
    assert!(text.contains("1 absent"), "counts the absent rows: {text}");

    let column = Serie::from_arrow_array(Field::new("price", DataType::Int64, true), absent)
        .expect("a nullable column admits it");
    assert_eq!(column.null_count(), 1);
    assert!(column.is_null(1).unwrap());
    assert_eq!(column.scalar(1).unwrap(), Scalar::Null);
}

#[test]
fn a_required_child_under_a_null_record_row_is_admitted() {
    // The child's absent slot is hidden by the record's null, so it is
    // judged only where the record is present.
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
    let column =
        Serie::from_arrow_array(root, Arc::clone(&records)).expect("a hidden absent child");
    assert_eq!(column.len(), 2);
    assert!(column.is_null(1).unwrap());
    assert_eq!(
        column.scalar(0).unwrap(),
        Scalar::from_sequence([Scalar::from(1_i64)])
    );

    // The same records under a required root: the hidden child is still
    // admitted, and it is the root's own absent row that is refused, by
    // the root's name.
    let required = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([Field::new("price", DataType::Int64, false)]).unwrap(),
        ),
        false,
    );
    let refusal = Serie::from_arrow_array(required, records).expect_err("the record row is absent");
    let text = refusal.to_string();
    assert!(text.contains("\"row\""), "names the root: {text}");
    assert!(text.contains("1 absent"), "counts the absent rows: {text}");
}

#[test]
fn a_value_the_fields_contract_refuses_is_refused_at_the_door_naming_the_row() {
    // A code rides Arrow's own text layout, so the layout admits any text
    // and the door reads each row once through the field's contract.
    let text: ArrayRef = Arc::new(StringArray::from(vec!["USD", "EUR", "EURO"]));
    let refusal = Serie::from_arrow_array(
        Field::new("ccy", DataType::Currency, false),
        Arc::clone(&text),
    )
    .expect_err("EURO is not a registered currency");
    let shown = refusal.to_string();
    assert!(shown.contains("ccy"), "names the column: {shown}");
    assert!(shown.contains("row 2"), "names the row: {shown}");

    // Under a nullable code field an absent row is not a value, and is
    // not read.
    let text: ArrayRef = Arc::new(StringArray::from(vec![Some("USD"), None]));
    let column = Serie::from_arrow_array(Field::new("ccy", DataType::Currency, true), text)
        .expect("two rows, one absent");
    assert_eq!(column.null_count(), 1);
    assert!(column.scalar(0).unwrap().is_code());

    // The plain UTF-8 layout is its own contract: nothing is read, and the
    // same bytes cross in.
    let text: ArrayRef = Arc::new(StringArray::from(vec!["AB", "ABCD"]));
    let refusal = Serie::from_arrow_array(
        Field::new("code", DataType::sized_utf8(2).unwrap(), false),
        Arc::clone(&text),
    )
    .expect_err("a value past the size is not one the field accepts");
    assert!(refusal.to_string().contains("row 1"), "{refusal}");
    assert!(Serie::from_arrow_array(Field::new("code", DataType::utf8(), false), text).is_ok());
}

#[test]
fn from_scalars_proves_the_rows_once_and_lays_them_out_once() {
    let column = Serie::from_scalars(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(1_i8), Scalar::from(2_i16)],
    )
    .expect("two rows the field rewrites");
    assert_eq!(
        column.as_int64().expect("an int64 column").values(),
        &[1, 2]
    );

    // A code column's rows are proven by the contract on the way in, and
    // the door does not read them again: the column reads back as codes.
    let codes = Serie::from_scalars(
        Field::new("ccy", DataType::Currency, false),
        [Scalar::from("USD"), Scalar::from("EUR")],
    )
    .expect("two registered currencies");
    assert!(codes.scalar(1).unwrap().is_code());
    assert_eq!(
        codes
            .as_utf8()
            .expect("a code rides utf8")
            .payload()
            .as_slice(),
        b"USDEUR"
    );

    let refusal = Serie::from_scalars(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(1_i64), Scalar::Null],
    )
    .expect_err("a required column admits no absent row");
    assert!(refusal.to_string().contains("price"));
    let refusal = Serie::from_scalars(
        Field::new("ccy", DataType::Currency, false),
        [Scalar::from("USD"), Scalar::from("EURO")],
    )
    .expect_err("EURO is not a registered currency");
    assert!(refusal.to_string().contains("ccy"), "{refusal}");
}

#[test]
fn a_sliced_list_array_is_rebased_onto_the_items_it_reaches() {
    let (field, lists) = legs();

    // Arrow slices a list by slicing its offsets and keeping the whole
    // child: the middle row is offsets [2, 3] over a child of five. The door
    // rebases the cut onto exactly the items it reaches, so a column that
    // grows knows where its items end.
    let middle: ArrayRef = Arc::new(lists.slice(1, 1));
    let mut column = Serie::from_arrow_array(field.clone(), middle).expect("a sliced list");
    assert_eq!(column.len(), 1);
    assert_eq!(
        column.scalar(0).unwrap(),
        Scalar::from_sequence([Scalar::from(3_i64)])
    );
    let leaf = column.as_list().expect("a list column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 1]);
    assert_eq!(leaf.items().as_int64().unwrap().values(), &[3]);
    column
        .push(Scalar::from_sequence([Scalar::from(99_i64)]))
        .expect("one more row");
    assert_eq!(
        column.rows().as_ref(),
        &[
            Scalar::from_sequence([Scalar::from(3_i64)]),
            Scalar::from_sequence([Scalar::from(99_i64)]),
        ]
    );

    // A tail slice is rebased the same way, its items sliced to match.
    let tail: ArrayRef = Arc::new(lists.slice(1, 2));
    let column = Serie::from_arrow_array(field.clone(), tail).expect("a sliced list");
    let leaf = column.as_list().expect("a list column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 1, 3]);
    assert_eq!(leaf.items().as_int64().unwrap().values(), &[3, 4, 5]);

    // An unsliced array is already in that shape and is taken untouched:
    // the offsets buffer is the array's own.
    let offsets = lists.offsets().inner().inner().clone();
    let whole: ArrayRef = Arc::new(lists);
    let column = Serie::from_arrow_array(field, whole).expect("a list column");
    let leaf = column.as_list().expect("a list column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 2, 3, 5]);
    assert!(leaf.offsets().inner().inner().ptr_eq(&offsets));
    assert_eq!(leaf.items().len(), 5);
}

#[test]
fn an_empty_column_names_its_datatype_and_a_reserved_one_is_still_empty() {
    let empty = Serie::empty(Field::new("price", DataType::Int64, false)).unwrap();
    assert!(empty.is_empty());
    assert_eq!(
        empty.dtype().unwrap(),
        DataType::list(Field::new("item", DataType::Int64, false))
    );

    let reserved = Serie::with_capacity(Field::new("price", DataType::Int64, true), 64).unwrap();
    assert!(reserved.is_empty());
    assert!(reserved.as_int64().is_some());

    let nested = Serie::with_capacity(quotes_root(), 8).unwrap();
    assert!(nested.is_empty());
    assert_eq!(nested.children().len(), 2);

    let (field, _) = legs();
    let lists = Serie::with_capacity(field, 8).unwrap();
    assert!(lists.is_empty());
    assert_eq!(lists.items().map(Serie::len), Some(0));
}

#[test]
fn a_record_root_crosses_from_a_batch_and_back_into_one() {
    let batch = quote_batch();
    let column = Serie::from_arrow_batch(&batch).expect("a record column");
    assert_eq!(column.field().map(Field::name), Some("row"));
    assert_eq!(column.len(), 2);
    assert_eq!(
        column.scalar(1).unwrap(),
        Scalar::from_sequence([Scalar::from(2_i64), Scalar::from("MSFT")])
    );
    // The batch's own columns are this column's children, shared.
    assert!(
        column
            .child("id")
            .and_then(Serie::into_arrow_array)
            .is_some_and(
                |ids| ids.to_data().buffers()[0].ptr_eq(&batch.column(0).to_data().buffers()[0])
            )
    );

    let back = column.into_arrow_batch().expect("a batch");
    assert_eq!(back.num_rows(), 2);
    assert_eq!(back.num_columns(), 2);
    assert_eq!(back.schema().field(1).name(), "symbol");

    let mut reader = column.into_arrow_reader().expect("a reader");
    let first = reader.next().expect("one batch").expect("a batch");
    assert_eq!(first.num_rows(), 2);
    assert!(reader.next().is_none());

    let drained = Serie::from_arrow_reader(column.into_arrow_reader().unwrap()).unwrap();
    assert_eq!(drained, column);
    assert_eq!(drained.field().map(Field::name), Some("row"));

    // A column that is not a record is not a table.
    let prices: ArrayRef = Arc::new(Int64Array::from(vec![1]));
    let prices =
        Serie::from_arrow_array(Field::new("price", DataType::Int64, false), prices).unwrap();
    let refusal = prices
        .into_arrow_batch()
        .expect_err("a leaf is not a table");
    assert!(refusal.to_string().contains("price"), "{refusal}");
    assert!(
        Serie::new(vec![Scalar::from(1_i64)])
            .into_arrow_batch()
            .is_err()
    );
    assert!(
        Serie::new(vec![Scalar::from(1_i64)])
            .into_arrow_reader()
            .is_err()
    );
}
