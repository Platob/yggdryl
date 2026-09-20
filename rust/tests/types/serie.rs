//! The column side of the value model: what a [`Serie`] owes the field that
//! types it, how it reads off the buffers it holds, how it grows, and how it
//! reads as the sequence it is.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int64Array, StringArray};
use yggdryl::{
    Column, DataType, Field, NestedValue, Scalar, Sequence, Serie, SerieValue, StructType, Value,
};

/// One non-null 64-bit column of two prices, held as values.
fn prices() -> Serie {
    Serie::from_rows(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(125_i64), Scalar::from(126_i64)],
    )
    .expect("two int64 rows")
}

/// The same two prices, held as Arrow buffers.
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
    // The rows arrive at three widths; the field is the one value contract,
    // so the column holds exactly what it declares.
    let serie = Serie::from_rows(
        Field::new("price", DataType::Int64, false),
        [
            Scalar::from(125_i32),
            Scalar::from(126_u8),
            Scalar::from(127_i64),
        ],
    )
    .expect("three widths narrow into one");

    assert_eq!(serie.len(), 3);
    assert_eq!(serie.get(0).unwrap(), Scalar::from(125_i64));
    assert_eq!(serie.get(2).unwrap(), Scalar::from(127_i64));
    // Past the end is absence, not a panic, which is what a reader walking
    // two columns of different lengths needs.
    assert_eq!(serie.get(3).unwrap(), Scalar::Null);
    assert!(serie.is_null(3));
}

#[test]
fn a_row_the_field_refuses_refuses_the_whole_column() {
    let refusal = Serie::from_rows(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(1_i64), Scalar::Null],
    )
    .expect_err("a required column holds no null");
    assert!(
        refusal.to_string().contains("price"),
        "the refusal names the field: {refusal}"
    );

    let nullable = Serie::from_rows(
        Field::new("price", DataType::Int64, true),
        [Scalar::from(1_i64), Scalar::Null],
    )
    .expect("a nullable column holds a null");
    assert!(nullable.is_null(1));
    assert_eq!(nullable.get(1).unwrap(), Scalar::Null);
}

#[test]
fn a_column_reads_its_rows_off_the_buffers_it_holds() {
    let serie = price_buffers();

    // Nothing has been decoded, so there is no slice to lend yet.
    assert_eq!(serie.len(), 2);
    assert_eq!(serie.chunk_count(), 1);
    assert_eq!(serie.frozen_len(), 2);
    assert_eq!(serie.as_slice(), None);

    // Random access reads the buffer, and does not decode the column.
    assert_eq!(serie.i64_at(0), Some(125));
    assert_eq!(serie.i128_at(1), Some(126));
    assert_eq!(serie.i64_at(2), None);
    assert!(!serie.is_null(0));
    assert_eq!(serie.as_slice(), None);

    // Asking for the rows decodes them once, and the reading is kept.
    assert_eq!(
        serie.rows().unwrap(),
        [Scalar::from(125_i64), Scalar::from(126_i64)].as_slice()
    );
    assert!(serie.as_slice().is_some());

    // A column of values holds them the other way round.
    let held = prices();
    assert_eq!(held.chunk_count(), 0);
    assert_eq!(held.as_slice().map(<[Scalar]>::len), Some(2));
    assert_eq!(held.i64_at(1), Some(126));
}

#[test]
fn the_typed_readers_span_the_widths_their_family_spans() {
    let text: ArrayRef = Arc::new(StringArray::from(vec![Some("AAPL"), None]));
    let symbols = Serie::from_arrow_array(Field::new("symbol", DataType::utf8(), true), text)
        .expect("a utf8 run");
    assert_eq!(symbols.str_at(0), Some("AAPL"));
    assert_eq!(symbols.str_at(1), None, "a null row reads as no value");
    assert!(symbols.is_null(1));
    assert_eq!(symbols.i64_at(0), None, "a text column is not an integer");

    // One reader, every integer width.
    let widths: [(DataType, ArrayRef); 3] = [
        (
            DataType::Int16,
            Arc::new(arrow_array::Int16Array::from(vec![7_i16])),
        ),
        (
            DataType::UInt32,
            Arc::new(arrow_array::UInt32Array::from(vec![7_u32])),
        ),
        (DataType::Int64, Arc::new(Int64Array::from(vec![7_i64]))),
    ];
    for (dtype, array) in widths {
        let column = Serie::from_arrow_array(Field::new("n", dtype, false), array).unwrap();
        assert_eq!(column.i128_at(0), Some(7));
        assert_eq!(column.i64_at(0), Some(7));
    }

    // One reader, every float width.
    let floats: ArrayRef = Arc::new(arrow_array::Float32Array::from(vec![1.5_f32]));
    let column =
        Serie::from_arrow_array(Field::new("f", DataType::Float32, false), floats).unwrap();
    assert_eq!(column.f64_at(0), Some(1.5));

    let flags: ArrayRef = Arc::new(arrow_array::BooleanArray::from(vec![true, false]));
    let column = Serie::from_arrow_array(Field::new("b", DataType::Boolean, false), flags).unwrap();
    assert_eq!(column.bool_at(0), Some(true));
    assert_eq!(column.bool_at(1), Some(false));

    // And the readers answer for a column still holding values, too.
    let held = prices();
    assert_eq!(held.i64_at(0), Some(125));
    assert_eq!(held.str_at(0), None);
}

#[test]
fn a_column_grows_without_rebuilding_what_it_already_holds() {
    let mut serie = price_buffers();

    // A pushed row lands in the tail: still one run of buffers.
    serie.push(Scalar::from(127_i64)).unwrap();
    assert_eq!(serie.len(), 3);
    assert_eq!(serie.chunk_count(), 1);
    assert_eq!(serie.frozen_len(), 2);
    assert_eq!(serie.i64_at(2), Some(127));
    assert_eq!(serie.get(2).unwrap(), Scalar::from(127_i64));

    // A pushed row meets the field, like every other row.
    assert!(serie.push(Scalar::Null).is_err());
    assert_eq!(serie.len(), 3, "a refused row changes nothing");

    // Extending refuses as a whole rather than half-appending.
    let mut refused = serie.clone();
    assert!(refused.extend([Scalar::from(1_i64), Scalar::Null]).is_err());
    assert_eq!(refused.len(), 3);

    // A whole run appends as one chunk, no row touched.
    let more: ArrayRef = Arc::new(Int64Array::from(vec![200, 201]));
    serie.append_arrow_array(more).unwrap();
    assert_eq!(serie.len(), 5);
    assert_eq!(
        serie.chunk_count(),
        3,
        "the tail froze, then the run landed"
    );
    assert_eq!(serie.i64_at(3), Some(200));
    assert_eq!(serie.i64_at(4), Some(201));

    // And the rows read back in order across every run.
    let rows = serie.rows().unwrap().to_vec();
    assert_eq!(
        rows.iter().filter_map(Scalar::as_i64).collect::<Vec<_>>(),
        vec![125, 126, 127, 200, 201]
    );

    // Gathering the runs keeps the rows and their order.
    let mut gathered = serie.clone();
    gathered.compact().unwrap();
    assert_eq!(gathered.chunk_count(), 1);
    assert_eq!(gathered.rows().unwrap(), rows.as_slice());
    assert_eq!(gathered, serie, "how a column is cut is not its value");
}

#[test]
fn a_run_whose_layout_is_not_the_fields_is_refused() {
    let field = Field::new("price", DataType::Int64, false);
    let text: ArrayRef = Arc::new(StringArray::from(vec!["AAPL"]));
    assert!(Serie::from_arrow_array(field.clone(), text).is_err());

    // A null under a required column is refused at the door, not at a read.
    let holes: ArrayRef = Arc::new(Int64Array::from(vec![Some(1), None]));
    let refusal =
        Serie::from_arrow_array(field, ArrayRef::clone(&holes)).expect_err("a required column");
    assert!(
        refusal.to_string().contains("price"),
        "the refusal names the field: {refusal}"
    );

    let nullable = Serie::from_arrow_array(Field::new("price", DataType::Int64, true), holes)
        .expect("a nullable column takes it");
    assert!(nullable.is_null(1));
}

#[test]
fn a_slice_shares_the_buffers_it_spans() {
    let mut serie = price_buffers();
    serie
        .append_arrow_array(Arc::new(Int64Array::from(vec![200, 201, 202])))
        .unwrap();
    assert_eq!(serie.len(), 5);

    let window = serie.slice(1, 3);
    assert_eq!(window.len(), 3);
    assert_eq!(
        window
            .rows()
            .unwrap()
            .iter()
            .filter_map(Scalar::as_i64)
            .collect::<Vec<_>>(),
        vec![126, 200, 201]
    );

    // Past the end clamps rather than refusing.
    assert_eq!(serie.slice(4, 99).len(), 1);
    assert_eq!(serie.slice(99, 1).len(), 0);

    // Truncating keeps the runs it can and cuts the one it must.
    let mut cut = serie.clone();
    cut.truncate(3);
    assert_eq!(cut.len(), 3);
    assert_eq!(cut.i64_at(2), Some(200));
    assert_eq!(cut.get(3).unwrap(), Scalar::Null);
}

#[test]
fn a_record_column_lends_its_children_and_takes_them_away() {
    let records = Serie::from_rows(quotes_root(), quotes_rows()).unwrap();

    // One child is one array per run, and a column of its own field.
    let symbols = records.child("symbol").unwrap();
    assert_eq!(symbols.len(), 2);
    assert_eq!(symbols.field().name(), "symbol");
    assert_eq!(symbols.str_at(0), Some("AAPL"));
    assert_eq!(symbols.str_at(1), Some("MSFT"));
    assert_eq!(records.child_at(0).unwrap().i64_at(1), Some(2));
    assert!(records.child("missing").is_err());

    // Dropping one keeps the other, and its field goes with it.
    let without = records.without_child("symbol").unwrap();
    assert_eq!(without.len(), 2);
    assert_eq!(without.field().field_len(), 1);
    assert!(without.child("symbol").is_err());
    assert_eq!(without.child("id").unwrap().i64_at(0), Some(1));

    // Adding one back reaches the same shape.
    let again = without.with_child(&symbols).unwrap();
    assert_eq!(again.field().field_len(), 2);
    assert_eq!(again.child("symbol").unwrap().str_at(1), Some("MSFT"));

    // A child of the wrong length does not fit.
    let short = Serie::from_rows(
        Field::new("symbol", DataType::utf8(), false),
        [Scalar::from("AAPL")],
    )
    .unwrap();
    assert!(records.with_child(&short).is_err());

    // A column of a leaf field has no children to lend.
    assert!(prices().child("anything").is_err());
}

#[test]
fn a_serie_reads_as_the_sequence_it_is() {
    let value = Scalar::from(prices());

    assert_eq!(value.kind(), "serie");
    assert!(value.is_container());
    assert_eq!(value.len(), 2);
    assert_eq!(
        value.as_sequence(),
        Some([Scalar::from(125_i64), Scalar::from(126_i64)].as_slice())
    );
    assert_eq!(value.get(1), Some(&Scalar::from(126_i64)));
    assert_eq!(value[0], Scalar::from(125_i64));
    assert_eq!(value.iter().count(), 2);
    assert_eq!(value.id(), yggdryl::DataTypeId::List);
    assert_eq!(value.family(), yggdryl::DataTypeKind::Nested);

    // And so does one still holding its buffers: borrowing its rows as
    // values decodes them once.
    let buffered = Scalar::from(price_buffers());
    assert_eq!(buffered.len(), 2);
    assert_eq!(buffered.iter().count(), 2);
    assert_eq!(buffered[1], Scalar::from(126_i64));

    // A schema-free run keeps its own word.
    assert_eq!(
        Scalar::from_sequence([Scalar::from(1_i64)]).kind(),
        "sequence"
    );
}

#[test]
fn a_serie_names_its_datatype_where_an_empty_sequence_cannot() {
    let declared = DataType::list(Field::new("price", DataType::Int64, false));
    assert_eq!(prices().dtype().unwrap(), declared);
    assert_eq!(Scalar::from(prices()).dtype().unwrap(), declared);
    assert_eq!(price_buffers().dtype().unwrap(), declared);

    let empty = Serie::empty(Field::new("price", DataType::Int64, false));
    assert!(empty.is_empty());
    assert_eq!(empty.dtype().unwrap(), declared);
    assert_eq!(empty.rows().unwrap(), &[] as &[Scalar]);

    // An empty run has no rows to read a datatype off, so it can only name
    // the ambiguous one; a caller asking it for an item Field is refused.
    assert_eq!(
        Scalar::from_sequence([]).dtype().unwrap(),
        DataType::list(Field::new("item", DataType::Null, true))
    );
    assert!(Scalar::from_sequence([]).inferred_array_field().is_err());
}

#[test]
fn a_column_and_the_run_it_holds_order_by_their_rows_and_are_not_one_value() {
    let serie = Scalar::from(prices());
    let run = Scalar::from_sequence([Scalar::from(125_i64), Scalar::from(126_i64)]);

    assert_ne!(serie, run, "a column knows its field and a run does not");
    assert!(
        run < serie,
        "equal rows tie, and the run is the plainer leaf"
    );

    let smaller = Scalar::from_sequence([Scalar::from(1_i64)]);
    assert!(smaller < serie);

    // How a column is held is not what it is: values and buffers over the
    // same rows under the same field are one value.
    assert_eq!(Scalar::from(price_buffers()), serie);

    // Two columns over one run order by the field that types them.
    let renamed = prices()
        .with_field(Field::new("size", DataType::Int64, false))
        .unwrap();
    assert_ne!(Scalar::from(renamed.clone()), serie);
    assert_eq!(renamed.rows().unwrap(), prices().rows().unwrap());
}

#[test]
fn the_family_narrows_and_widens_through_the_serie_root() {
    let serie = prices();
    let column = serie.as_column().expect("the one leaf today").clone();

    assert_eq!(Column::from_serie(&serie), Some(&column));
    assert_eq!(column.clone().into_serie(), serie);
    assert_eq!(Serie::from_serie(&serie), Some(&serie));

    let value = Scalar::from(serie.clone());
    assert_eq!(Serie::from_scalar(&value), Some(&serie));
    assert_eq!(Column::from_scalar(&value), Some(&column));
    assert_eq!(Value::into_scalar(serie.clone()), value);
    assert_eq!(Serie::from_scalar(&Scalar::Null), None);

    let Scalar::Sequence(held) = &value else {
        panic!("a serie is a sequence value");
    };
    assert_eq!(held.as_serie(), Some(&serie));
    assert_eq!(held.as_list(), None);
    assert_eq!(held.row_count(), 2);
    assert_eq!(Sequence::from(serie.clone()).as_serie(), Some(&serie));
    assert_eq!(NestedValue::len(&serie), 2);
    assert_eq!(serie.children().count(), 2);
    assert_eq!(SerieValue::get(&serie, 0).unwrap(), Scalar::from(125_i64));

    // Dropping the field is spelled, never implied.
    assert_eq!(
        serie.into_sequence().unwrap(),
        Scalar::from_sequence([Scalar::from(125_i64), Scalar::from(126_i64)])
    );
}

#[test]
fn a_column_survives_the_value_contract_until_that_contract_rewrites_it() {
    let value = Scalar::from(prices());
    let item = Field::new("price", DataType::Int64, false);

    let exact = Field::new("prices", DataType::list(item.clone()), false);
    assert_eq!(exact.scalar(value.clone()).unwrap(), value);

    let renamed = Field::new(
        "prices",
        DataType::list(Field::new("item", DataType::Int64, false)),
        false,
    );
    assert_eq!(renamed.scalar(value.clone()).unwrap(), value);

    let narrower = Field::new(
        "prices",
        DataType::list(Field::new("price", DataType::Int32, false)),
        false,
    );
    let rewritten = narrower.scalar(value.clone()).unwrap();
    assert_eq!(rewritten.kind(), "sequence");
    assert_eq!(
        rewritten,
        Scalar::from_sequence([Scalar::from(125_i32), Scalar::from(126_i32)])
    );

    let root = Field::new(
        "row",
        DataType::Struct(
            StructType::from_fields([Field::new("prices", DataType::list(item), false)]).unwrap(),
        ),
        false,
    );
    let row = root
        .scalar(Scalar::from_struct([("prices", value)]).unwrap())
        .unwrap();
    assert_eq!(row.get(0).map(Scalar::kind), Some("serie"));
}

#[test]
fn a_column_crosses_into_one_arrow_array_and_back() {
    let serie = prices();
    let array = serie.into_arrow_array().unwrap();

    assert_eq!(array.len(), 2);
    assert_eq!(array.data_type(), &arrow_schema::DataType::Int64);
    assert_eq!(
        array
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .values(),
        &[125, 126]
    );

    let read = Serie::from_arrow_array(serie.field().clone(), ArrayRef::clone(&array)).unwrap();
    assert_eq!(read, serie);

    // A column already holding one run lends it rather than rebuilding it.
    let buffered = price_buffers();
    let lent = buffered.into_arrow_array().unwrap();
    assert!(Arc::ptr_eq(&lent, buffered.chunk(0).unwrap()));

    // The leaf answers the same doors as the root it widens to.
    let column = serie.as_column().unwrap();
    assert_eq!(column.into_arrow_array().unwrap().as_ref(), array.as_ref());
}

#[test]
fn a_record_root_column_crosses_into_one_batch_and_back() {
    let serie = Serie::from_rows(quotes_root(), quotes_rows()).unwrap();
    let batch = serie.into_arrow_batch().unwrap();

    assert_eq!(batch.num_rows(), 2);
    assert_eq!(batch.num_columns(), 2);
    assert_eq!(batch.schema().field(0).name(), "id");

    let read = Serie::from_arrow_batch(&batch).unwrap();
    assert_eq!(read.len(), 2);
    assert_eq!(read.rows().unwrap(), serie.rows().unwrap());
    assert_eq!(read.field().dtype(), serie.field().dtype());

    // A column of a leaf field is not a table, and says so rather than
    // guessing a root around it.
    assert!(prices().into_arrow_batch().is_err());
}

#[test]
fn a_record_column_streams_one_batch_per_run() {
    let mut serie = Serie::from_rows(quotes_root(), quotes_rows()).unwrap();
    serie.freeze().unwrap();
    let second = serie.into_arrow_array().unwrap();
    serie.append_arrow_array(second).unwrap();
    assert_eq!(serie.len(), 4);
    assert_eq!(serie.chunk_count(), 2);

    // Nothing is gathered on the way out: a column of two runs is two
    // batches.
    let rows: Vec<usize> = serie
        .into_arrow_reader()
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .collect();
    assert_eq!(rows, vec![2, 2]);

    // And back in, one run per batch, with no row decoded.
    let read = Serie::from_arrow_reader(serie.into_arrow_reader().unwrap()).unwrap();
    assert_eq!(read.chunk_count(), 2);
    assert_eq!(read.len(), 4);
    assert_eq!(read.as_slice(), None);
}

#[test]
fn the_column_wire_carries_the_field_beside_the_rows() {
    let value = Scalar::from(prices());
    let document = serde_json::to_string(&value).unwrap();

    assert!(
        document.contains("\"serie\""),
        "a column writes under its own tag: {document}"
    );
    assert!(
        document.contains("price"),
        "and carries the field that types it: {document}"
    );

    let read: Scalar = serde_json::from_str(&document).unwrap();
    assert_eq!(read, value);
    assert_eq!(read.kind(), "serie");

    let run = Scalar::from_sequence([Scalar::from(1_i64)]);
    let written = serde_json::to_string(&run).unwrap();
    assert!(written.contains("\"sequence\""), "{written}");
    assert_eq!(serde_json::from_str::<Scalar>(&written).unwrap(), run);
}

#[test]
fn the_sequence_family_reads_back_whichever_leaf_a_document_holds() {
    let serie = Sequence::from(prices());
    let run = Sequence::new([Scalar::from(1_i64), Scalar::from(2_i64)]);

    for held in [serie, run] {
        let document = serde_json::to_string(&held).unwrap();
        let read: Sequence = serde_json::from_str(&document).unwrap();
        assert_eq!(read, held, "{document}");
        assert_eq!(read.rows().unwrap(), held.rows().unwrap());
    }

    let refused = r#"{"field":"price: int64","rows":[null]}"#;
    assert!(serde_json::from_str::<Sequence>(refused).is_err());
}

#[test]
fn a_column_clone_shares_its_buffers_rather_than_copying_them() {
    let serie = price_buffers();
    let copy = serie.clone();
    assert!(Arc::ptr_eq(serie.chunk(0).unwrap(), copy.chunk(0).unwrap()));
    assert_eq!(serie, copy);

    // And a mutation copies the body rather than the buffers it keeps.
    let mut grown = copy.clone();
    grown.push(Scalar::from(127_i64)).unwrap();
    assert_eq!(serie.len(), 2, "the original is untouched");
    assert_eq!(grown.len(), 3);
    assert!(Arc::ptr_eq(
        serie.chunk(0).unwrap(),
        grown.chunk(0).unwrap()
    ));
}
