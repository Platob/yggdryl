//! `rust/src/serie/arrow.rs`: the one door buffers take into a column -
//! what it proves, what it refuses by name, and what crosses back out.

use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, Int32Array, Int64Array, ListArray, RecordBatch, StringArray, StructArray,
};
use arrow_buffer::{NullBuffer, OffsetBuffer};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
use yggdryl::arrow::batch_reader;
use yggdryl::{
    ArrowCastOptions, DataType, Field, Nullability, Scalar, Serie, SerieReader, StructType,
};

/// The options a refusal is pinned under: a present value is never nulled
/// and an absent one never repaired.
fn strict() -> ArrowCastOptions {
    ArrowCastOptions::new()
        .with_safe(false)
        .with_nullability(Nullability::Strict)
}

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

/// [`quote_batch`] with its identifiers laid out as int32, which the
/// quotes root declares as int64.
fn narrow_quote_batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        ArrowField::new("id", ArrowDataType::Int32, false),
        ArrowField::new("symbol", ArrowDataType::Utf8, false),
    ]));
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int32Array::from(vec![1, 2])),
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
    let column = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Int64, false)),
        held,
        ArrowCastOptions::new(),
    )
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
fn a_layout_that_is_not_the_fields_is_cast_and_a_value_no_cast_reaches_is_refused() {
    // An int32 array under an int64 field is converted by the one plan the
    // door compiles: the column holds the field's layout, in buffers of its
    // own rather than the array's.
    let narrow = Int32Array::from(vec![1, 2]);
    let values = narrow.values().as_ptr().cast::<u8>();
    let column = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Int64, false)),
        Arc::new(narrow),
        strict(),
    )
    .expect("int32 widens into int64");
    assert_eq!(
        column.as_int64().expect("an int64 column").values(),
        &[1, 2]
    );
    let back = column.into_arrow_array().expect("a column");
    assert_eq!(back.data_type(), &ArrowDataType::Int64);
    assert!(
        back.to_data().buffers()[0].as_ptr() != values,
        "a converted column shares nothing with its input"
    );

    // Text no reading takes as an integer is refused under `safe = false`,
    // naming the value and the datatype it could not reach.
    let text: ArrayRef = Arc::new(StringArray::from(vec!["AAPL"]));
    let refusal = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Int64, false)),
        Arc::clone(&text),
        strict(),
    )
    .expect_err("AAPL is no int64");
    let shown = refusal.to_string();
    assert!(shown.contains("AAPL"), "names the value: {shown}");
    assert!(shown.contains("Int64"), "names the target: {shown}");

    // Under the default options the same value is nulled, and a required
    // field repairs the null to its canonical default.
    let column = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Int64, false)),
        text,
        ArrowCastOptions::new(),
    )
    .expect("nulled, then repaired");
    assert_eq!(column.as_int64().expect("an int64 column").values(), &[0]);
    assert_eq!(column.null_count(), 0);
}

#[test]
fn an_absent_row_is_refused_under_a_required_field_and_admitted_under_a_nullable_one() {
    let absent: ArrayRef = Arc::new(Int64Array::from(vec![Some(1), None]));
    let refusal = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Int64, false)),
        Arc::clone(&absent),
        strict(),
    )
    .expect_err("a strict required column admits no absent row");
    let text = refusal.to_string();
    assert!(text.contains("$.price"), "names the column: {text}");
    assert!(text.contains("1 null"), "counts the absent rows: {text}");

    // Under the default nullability the absent row is repaired to the
    // field's canonical default instead.
    let repaired = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Int64, false)),
        Arc::clone(&absent),
        ArrowCastOptions::new(),
    )
    .expect("the default nullability repairs absence");
    assert_eq!(
        repaired.as_int64().expect("an int64 column").values(),
        &[1, 0]
    );
    assert_eq!(repaired.null_count(), 0);

    let column = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Int64, true)),
        absent,
        ArrowCastOptions::new(),
    )
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
    let column = Serie::from_arrow_array(Some(&root), Arc::clone(&records), strict())
        .expect("a hidden absent child, even strictly");
    assert_eq!(column.len(), 2);
    assert!(column.is_null(1).unwrap());
    assert_eq!(
        column.scalar(0).unwrap(),
        Scalar::from_sequence([Scalar::from(1_i64)])
    );

    // The same records under a required root: the hidden child is still
    // admitted, and it is the root's own absent row that is refused, at
    // the root's path - a required record is the `$` its children hang
    // from.
    let required = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([Field::new("price", DataType::Int64, false)]).unwrap(),
        ),
        false,
    );
    let refusal = Serie::from_arrow_array(Some(&required), records, strict())
        .expect_err("the record row is absent");
    let text = refusal.to_string();
    assert!(text.contains("field $ "), "names the root: {text}");
    assert!(text.contains("1 null"), "counts the absent rows: {text}");
}

#[test]
fn a_value_the_fields_contract_refuses_is_refused_at_the_door_naming_the_row() {
    // A code rides Arrow's own text layout, so the layout admits any text
    // and the door reads each row once through the field's contract.
    let text: ArrayRef = Arc::new(StringArray::from(vec!["USD", "EUR", "EURO"]));
    let refusal = Serie::from_arrow_array(
        Some(&Field::new("ccy", DataType::Currency, false)),
        Arc::clone(&text),
        strict(),
    )
    .expect_err("EURO is not a registered currency");
    let shown = refusal.to_string();
    assert!(shown.contains("ccy"), "names the column: {shown}");
    assert!(shown.contains("row 2"), "names the row: {shown}");

    // Under `safe` the refused value is nulled rather than refused.
    let nulled = Serie::from_arrow_array(
        Some(&Field::new("ccy", DataType::Currency, true)),
        Arc::clone(&text),
        ArrowCastOptions::new(),
    )
    .expect("EURO is nulled");
    assert_eq!(nulled.null_count(), 1);
    assert!(nulled.is_null(2).unwrap());

    // Under a nullable code field an absent row is not a value, and is
    // not read.
    let text: ArrayRef = Arc::new(StringArray::from(vec![Some("USD"), None]));
    let column = Serie::from_arrow_array(
        Some(&Field::new("ccy", DataType::Currency, true)),
        text,
        strict(),
    )
    .expect("two rows, one absent");
    assert_eq!(column.null_count(), 1);
    assert!(column.scalar(0).unwrap().is_code());

    // The plain UTF-8 layout is its own contract: nothing is read, and the
    // same bytes cross in.
    let text: ArrayRef = Arc::new(StringArray::from(vec!["AB", "ABCD"]));
    let refusal = Serie::from_arrow_array(
        Some(&Field::new("code", DataType::sized_utf8(2).unwrap(), false)),
        Arc::clone(&text),
        strict(),
    )
    .expect_err("a value past the size is not one the field accepts");
    assert!(refusal.to_string().contains("row 1"), "{refusal}");
    assert!(
        Serie::from_arrow_array(
            Some(&Field::new("code", DataType::utf8(), false)),
            text,
            strict()
        )
        .is_ok()
    );
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
    let mut column = Serie::from_arrow_array(Some(&field), middle, ArrowCastOptions::new())
        .expect("a sliced list");
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
    let column = Serie::from_arrow_array(Some(&field), tail, ArrowCastOptions::new())
        .expect("a sliced list");
    let leaf = column.as_list().expect("a list column");
    assert_eq!(leaf.offsets().as_ref(), &[0, 1, 3]);
    assert_eq!(leaf.items().as_int64().unwrap().values(), &[3, 4, 5]);

    // An unsliced array is already in that shape and is taken untouched:
    // the offsets buffer is the array's own.
    let offsets = lists.offsets().inner().inner().clone();
    let whole: ArrayRef = Arc::new(lists);
    let column = Serie::from_arrow_array(Some(&field), whole, ArrowCastOptions::new())
        .expect("a list column");
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
    let column =
        Serie::from_arrow_batch(None, &batch, ArrowCastOptions::new()).expect("a record column");
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

    let drained = Serie::from_arrow_reader(
        None,
        column.into_arrow_reader().unwrap(),
        ArrowCastOptions::new(),
    )
    .unwrap();
    assert_eq!(drained, column);
    assert_eq!(drained.field().map(Field::name), Some("row"));

    // A column that is not a record is the one column of a row root.
    let prices: ArrayRef = Arc::new(Int64Array::from(vec![1]));
    let prices = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Int64, false)),
        prices,
        ArrowCastOptions::new(),
    )
    .unwrap();
    let table = prices.into_arrow_batch().expect("a leaf is one column");
    assert_eq!(table.schema().field(0).name(), "price");
    assert_eq!(table.num_rows(), 1);
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

#[test]
fn a_record_column_holding_an_absent_row_is_not_a_table() {
    let child = Field::new("id", DataType::Int64, false);
    let root = Field::new(
        "row",
        DataType::from(StructType::from_fields([child]).expect("one child")),
        true,
    );
    let records = StructArray::new(
        vec![ArrowField::new("id", ArrowDataType::Int64, false)].into(),
        vec![Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef],
        Some(NullBuffer::from(vec![true, false])),
    );
    let serie = Serie::from_arrow_array(Some(&root), Arc::new(records), ArrowCastOptions::new())
        .expect("a nullable record column");
    let refusal = serie
        .into_arrow_batch()
        .expect_err("a batch states no row validity");
    assert!(refusal.to_string().contains("1 absent rows"), "{refusal}");
}

#[test]
fn a_column_that_is_not_a_record_crosses_as_the_one_column_of_a_row_root() {
    let field = Field::new("price", DataType::Int64, true);
    let serie = Serie::from_scalars(field, [Scalar::from(1_i64), Scalar::Null]).expect("two rows");
    let batch = serie.into_arrow_batch().expect("one column");
    assert_eq!(batch.num_columns(), 1);
    assert_eq!(batch.schema().field(0).name(), "price");
    assert_eq!(batch.num_rows(), 2);
    let reader = serie.into_arrow_reader().expect("one batch");
    assert_eq!(reader.schema(), batch.schema());
}

#[test]
fn null_is_absence_under_a_required_field_except_where_it_is_the_datatypes_own_default() {
    // A required `null` column holds only nulls: null is its one value.
    let nothing = Field::new("nothing", DataType::Null, false);
    let nulls: ArrayRef = Arc::new(arrow_array::NullArray::new(3));
    let serie = Serie::from_arrow_array(Some(&nothing), nulls, ArrowCastOptions::new())
        .expect("null is null's default");
    assert_eq!(serie.len(), 3);

    // Every other required column still refuses one.
    let ids: ArrayRef = Arc::new(Int64Array::from(vec![Some(1), None]));
    let refusal = Serie::from_arrow_array(
        Some(&Field::new("id", DataType::Int64, false)),
        ids,
        strict(),
    )
    .expect_err("an int64 is not absent by default");
    assert!(refusal.to_string().contains("$.id"), "{refusal}");
    let symbols: ArrayRef = Arc::new(StringArray::from(vec![Some("AAPL"), None]));
    assert!(
        Serie::from_arrow_array(
            Some(&Field::new("symbol", DataType::utf8(), false)),
            symbols,
            strict()
        )
        .is_err()
    );
}

#[test]
fn the_default_column_repeats_one_row_the_field_lays_out() {
    let required =
        Serie::from_default(Field::new("id", DataType::Int64, false), 3).expect("zero three times");
    assert_eq!(
        required.as_int64().expect("an int64 column").values(),
        &[0, 0, 0]
    );
    let nullable =
        Serie::from_default(Field::new("id", DataType::Int64, true), 2).expect("null twice");
    assert_eq!(nullable.null_count(), 2);
    assert_eq!(
        Serie::from_default(Field::new("id", DataType::Int64, false), 0)
            .expect("no rows")
            .len(),
        0
    );
}

#[test]
fn one_row_is_an_arrow_scalar_and_any_other_length_is_refused() {
    let field = Field::new("id", DataType::Int64, false);
    let one = Serie::from_scalars(field.clone(), [Scalar::from(7_i64)]).expect("one row");
    let datum = one.into_arrow_scalar().expect("one row is a scalar");
    let (array, is_scalar) = arrow_array::Datum::get(&datum);
    assert!(is_scalar);
    assert_eq!(array.len(), 1);
    let two = Serie::from_scalars(field, [Scalar::from(1_i64), Scalar::from(2_i64)]).expect("two");
    let refusal = two
        .into_arrow_scalar()
        .expect_err("two rows are not a scalar");
    assert!(refusal.to_string().contains("got 2"), "{refusal}");
    assert!(
        Serie::new(vec![Scalar::from(1_i64)])
            .into_arrow_scalar()
            .is_err()
    );
}

#[test]
fn a_batch_casts_into_a_declared_root_and_one_already_in_it_shares_its_columns() {
    let root = quotes_root();
    let batch = quote_batch();
    let exact = Serie::from_arrow_batch(Some(&root), &batch, strict()).expect("the root's layout");
    assert_eq!(exact.field(), Some(&root));
    assert!(
        exact
            .child("id")
            .and_then(Serie::into_arrow_array)
            .is_some_and(
                |ids| ids.to_data().buffers()[0].ptr_eq(&batch.column(0).to_data().buffers()[0])
            ),
        "an exact batch lands on its own buffers"
    );

    // Int32 identifiers are widened by the one plan into the declared
    // int64, and the rows are the rows of the exact batch.
    let widened = Serie::from_arrow_batch(Some(&root), &narrow_quote_batch(), strict())
        .expect("int32 widens into int64");
    assert_eq!(widened, exact);
    assert_eq!(
        widened
            .child("id")
            .and_then(Serie::as_int64)
            .map(|ids| ids.values().to_vec()),
        Some(vec![1, 2])
    );

    // A table has no row validity, so a nullable root is refused before a
    // row is read.
    let refusal = Serie::from_arrow_batch(
        Some(&root.clone().with_nullable(true)),
        &batch,
        ArrowCastOptions::new(),
    )
    .expect_err("a batch lands under a non-null record");
    assert!(refusal.to_string().contains("non-nullable"), "{refusal}");
}

#[test]
fn a_stream_is_one_record_column_per_batch_under_one_plan() {
    let root = quotes_root();
    let narrow = narrow_quote_batch();
    let stream = || batch_reader(narrow.schema(), [narrow.clone(), narrow.slice(1, 1)]);

    let series = SerieReader::from_arrow_reader(Some(&root), stream(), strict()).expect("one plan");
    assert_eq!(series.field(), &root);
    let landed = series
        .collect::<Result<Vec<Serie>, _>>()
        .expect("two batches");
    assert_eq!(landed.iter().map(Serie::len).collect::<Vec<_>>(), [2, 1]);
    assert_eq!(
        landed[1].scalar(0).unwrap(),
        Scalar::from_sequence([Scalar::from(2_i64), Scalar::from("MSFT")])
    );

    // Drained, the stream is one column of every row it carried.
    let drained = Serie::from_arrow_reader(Some(&root), stream(), strict()).expect("three rows");
    assert_eq!(drained.len(), 3);
    assert_eq!(drained.field(), Some(&root));

    // The transport face reconciles each batch to the root, landing none.
    let mut transport = SerieReader::from_arrow_reader(Some(&root), stream(), strict())
        .expect("one plan")
        .into_arrow_reader();
    assert_eq!(
        transport.schema().field(0).data_type(),
        &ArrowDataType::Int64
    );
    let first = transport
        .next()
        .expect("a batch")
        .expect("a reconciled batch");
    assert_eq!(first.column(0).data_type(), &ArrowDataType::Int64);
    assert_eq!(first.num_rows(), 2);

    // A stream no plan reaches the root from is refused by the constructor,
    // before a batch is pulled: no source column carries `id`.
    let symbols = quote_batch().project(&[1]).expect("the symbol column");
    let refusal = SerieReader::from_arrow_reader(
        Some(&root),
        batch_reader(symbols.schema(), [symbols]),
        strict(),
    )
    .expect_err("a required identifier no column carries");
    assert!(refusal.to_string().contains("id"), "{refusal}");
}

#[test]
fn a_column_in_hand_casts_once_and_one_under_its_own_field_is_itself() {
    let field = Field::new("id", DataType::Int32, true);
    let ids =
        Serie::from_scalars(field.clone(), [Scalar::from(1_i32), Scalar::Null]).expect("two rows");
    let values = ids.as_int32().expect("an int32 column").values().as_ptr();

    let same = ids.cast(&field, strict()).expect("its own field");
    assert_eq!(same.as_int32().unwrap().values().as_ptr(), values);

    let wide = ids
        .cast(&Field::new("id", DataType::Int64, true), strict())
        .expect("int32 widens into int64");
    assert_eq!(wide.as_int64().expect("an int64 column").values()[0], 1);
    assert!(wide.is_null(1).unwrap());

    // The absent row is judged again under a required field: refused
    // strictly, repaired to the default otherwise.
    let required = Field::new("id", DataType::Int32, false);
    let refusal = ids
        .cast(&required, strict())
        .expect_err("a strict required column admits no absent row");
    assert!(refusal.to_string().contains("id"), "{refusal}");
    let repaired = ids
        .cast(&required, ArrowCastOptions::new())
        .expect("the default nullability repairs absence");
    assert_eq!(repaired.as_int32().unwrap().values(), &[1, 0]);

    // A run lays out no buffers for a plan to read.
    assert!(
        Serie::new(vec![Scalar::from(1_i32)])
            .cast(&field, strict())
            .is_err()
    );
}

#[test]
fn an_extension_label_is_not_a_proof() {
    // The column claims to be a URL; the landing still reads what it holds.
    let url = Field::new("u", DataType::Url, false);
    let labelled = url
        .clone()
        .into_arrow_field_ref()
        .expect("a url projects")
        .as_ref()
        .clone();
    let schema = Arc::new(Schema::new(vec![labelled]));
    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(StringArray::from(vec!["not a url"])) as ArrayRef],
    )
    .expect("one row");
    let root = Field::new(
        "row",
        DataType::from(StructType::from_fields([url]).expect("one child")),
        false,
    );
    let refusal =
        Serie::from_arrow_batch(Some(&root), &batch, strict()).expect_err("a label proves nothing");
    let message = refusal.to_string();
    assert!(
        message.contains("\"u\"") && message.contains("row 0"),
        "{message}"
    );
}

#[test]
fn a_kernel_into_a_rule_governed_leaf_lands_no_row_its_field_refuses() {
    // Arrow reads any int64 as a date64; a date64 here is a whole day.
    let millis: ArrayRef = Arc::new(Int64Array::from(vec![1_i64]));
    let day = Field::new("day", DataType::Date64, false);
    assert!(Serie::from_arrow_array(Some(&day), millis, strict()).is_err());
    let midnight: ArrayRef = Arc::new(Int64Array::from(vec![86_400_000_i64]));
    let serie = Serie::from_arrow_array(Some(&day), midnight, strict()).expect("a whole day lands");
    assert_eq!(serie.len(), 1);
}

#[test]
fn a_serie_reader_casts_each_batch_by_one_plan_and_fuses_after_a_failure() {
    let schema = Arc::new(Schema::new(vec![ArrowField::new(
        "id",
        ArrowDataType::Int32,
        true,
    )]));
    let good = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(arrow_array::Int32Array::from(vec![Some(1)])) as ArrayRef],
    )
    .unwrap();
    let absent = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(arrow_array::Int32Array::from(vec![None::<i32>])) as ArrayRef],
    )
    .unwrap();
    let root = Field::new(
        "row",
        DataType::from(
            StructType::from_fields([Field::new("id", DataType::Int64, false)]).unwrap(),
        ),
        false,
    );
    let reader = yggdryl::arrow::batch_reader(Arc::clone(&schema), [good.clone(), absent, good]);
    let mut series =
        yggdryl::SerieReader::from_arrow_reader(Some(&root), reader, strict()).expect("one plan");
    assert_eq!(series.field(), &root);
    let first = series
        .next()
        .expect("a batch")
        .expect("the first batch casts");
    assert_eq!(first.len(), 1);
    assert!(series.next().expect("a batch").is_err());
    assert!(series.next().is_none(), "fused after the failure");
}

#[test]
fn an_identity_serie_reader_hands_its_inner_reader_back() {
    let batch = quote_batch();
    let root = Field::from_arrow_schema("row", batch.schema().as_ref()).expect("the root");
    let reader = yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]);
    let transport =
        yggdryl::SerieReader::from_arrow_reader(Some(&root), reader, ArrowCastOptions::new())
            .expect("an identity plan")
            .into_arrow_reader();
    let back: Vec<RecordBatch> = transport.map(Result::unwrap).collect();
    assert_eq!(back.len(), 1);
    assert!(Arc::ptr_eq(back[0].column(0), batch.column(0)));
}

#[test]
fn a_nullable_record_over_an_uninhabited_child_defaults_to_an_absent_row() {
    // The child can hold nothing, so it has no default and no null default;
    // the record's own absence is what its default row is.
    let inner = Field::new(
        "inner",
        DataType::from(
            StructType::from_fields([Field::new("required_null", DataType::Null, false)])
                .expect("one child"),
        ),
        false,
    );
    let outer = Field::new(
        "outer",
        DataType::from(StructType::from_fields([inner]).expect("one child")),
        true,
    );
    let serie = Serie::from_default(outer.clone(), 2).expect("two absent rows");
    assert_eq!(serie.null_count(), 2);
    let rows = Serie::from_scalars(outer, [Scalar::Null]).expect("an absent row");
    assert_eq!(rows.null_count(), 1);
}

#[test]
fn a_zero_width_list_default_keeps_its_row_count() {
    let empty = Field::new(
        "empty",
        DataType::fixed_size_list(Field::new("item", DataType::Int32, true), 0).expect("width 0"),
        false,
    );
    let rows = Serie::from_default(empty, 3).expect("three rows");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows.require_arrow_array().expect("an array").len(), 3);
    let one = rows.slice(0, 1).expect("one row");
    assert!(one.into_arrow_scalar().is_ok());
}
