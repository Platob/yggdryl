//! The column side of the value model: what a [`Serie`] owes the field that
//! types it, how it reads as a sequence, and both directions of its Arrow
//! interop.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int64Array, RecordBatch, RecordBatchReader};

use yggdryl::{
    Column, DataType, Field, NestedValue, Scalar, Sequence, Serie, SerieValue, StructType, Value,
};

/// One non-null 64-bit column of two prices.
fn prices() -> Serie {
    Serie::from_rows(
        Field::new("price", DataType::Int64, false),
        [Scalar::from(125_i64), Scalar::from(126_i64)],
    )
    .expect("two int64 rows")
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
    // The rows arrive at three widths and one spelling; the field is the one
    // value contract, so the column holds exactly what it declares.
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
    assert_eq!(serie.get(0), Some(&Scalar::from(125_i64)));
    assert_eq!(serie.get(1), Some(&Scalar::from(126_i64)));
    assert_eq!(serie.get(2), Some(&Scalar::from(127_i64)));
    assert_eq!(serie.get(3), None);
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

    // The same rows under a nullable field are a column.
    let nullable = Serie::from_rows(
        Field::new("price", DataType::Int64, true),
        [Scalar::from(1_i64), Scalar::Null],
    )
    .expect("a nullable column holds a null");
    assert_eq!(nullable.get(1), Some(&Scalar::Null));
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

    // A schema-free run keeps its own word, so an error message still tells
    // a caller which of the two it was handed.
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

    // Empty, and still exact: the field is carried, not inferred.
    let empty = Serie::from_rows(Field::new("price", DataType::Int64, false), []).unwrap();
    assert!(empty.is_empty());
    assert_eq!(empty.dtype().unwrap(), declared);
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

    // The rows lead, so a column never sorts away from the run it holds.
    let smaller = Scalar::from_sequence([Scalar::from(1_i64)]);
    assert!(smaller < serie);

    // Two columns over one run order by the field that types them.
    let renamed = prices()
        .with_field(Field::new("size", DataType::Int64, false))
        .unwrap();
    assert_ne!(Scalar::from(renamed.clone()), serie);
    assert_eq!(renamed.as_slice(), prices().as_slice());
}

#[test]
fn the_family_narrows_and_widens_through_the_serie_root() {
    let serie = prices();
    let column = serie.as_column().expect("the one leaf today").clone();

    assert_eq!(Column::from_serie(&serie), Some(&column));
    assert_eq!(column.clone().into_serie(), serie);
    assert_eq!(Serie::from_serie(&serie), Some(&serie));

    // And through the value root, at both levels.
    let value = Scalar::from(serie.clone());
    assert_eq!(Serie::from_scalar(&value), Some(&serie));
    assert_eq!(Column::from_scalar(&value), Some(&column));
    assert_eq!(Value::into_scalar(serie.clone()), value);
    assert_eq!(Serie::from_scalar(&Scalar::Null), None);

    // The sequence family holds it, and says which leaf it is.
    let Scalar::Sequence(held) = &value else {
        panic!("a serie is a sequence value");
    };
    assert_eq!(held.as_serie(), Some(&serie));
    assert_eq!(held.as_list(), None);
    assert_eq!(Sequence::from(serie.clone()).as_serie(), Some(&serie));
    assert_eq!(NestedValue::len(&serie), 2);
    assert_eq!(serie.children().count(), 2);

    // Dropping the field is spelled, never implied.
    assert_eq!(
        serie.into_sequence(),
        Scalar::from_sequence([Scalar::from(125_i64), Scalar::from(126_i64)])
    );
}

#[test]
fn a_column_survives_the_value_contract_until_that_contract_rewrites_it() {
    let value = Scalar::from(prices());
    let item = Field::new("price", DataType::Int64, false);

    // Nothing to rewrite: the column comes back the column it was, the field
    // it carries included.
    let exact = Field::new("prices", DataType::list(item.clone()), false);
    assert_eq!(exact.scalar(value.clone()).unwrap(), value);

    // A name is the column's, not the rows', so it does not rewrite them.
    let renamed = Field::new(
        "prices",
        DataType::list(Field::new("item", DataType::Int64, false)),
        false,
    );
    assert_eq!(renamed.scalar(value.clone()).unwrap(), value);

    // A narrower width does rewrite them, and from there the declaring field
    // is the authority on the column.
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

    // And a column nested in a record row is a column still.
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

    let read = Serie::from_arrow_array(serie.field().clone(), array.as_ref()).unwrap();
    assert_eq!(read, serie);

    // The leaf answers the same two doors as the root it widens to.
    let column = serie.as_column().unwrap();
    assert_eq!(column.into_arrow_array().unwrap().as_ref(), array.as_ref());
    assert_eq!(
        Column::from_arrow_array(column.field().clone(), array.as_ref()).unwrap(),
        *column
    );

    // An array of another datatype is refused rather than reinterpreted.
    let wrong: ArrayRef = Arc::new(arrow_array::StringArray::from(vec!["AAPL"]));
    assert!(Serie::from_arrow_array(serie.field().clone(), wrong.as_ref()).is_err());
}

#[test]
fn a_record_root_column_crosses_into_one_batch_and_back() {
    let serie = Serie::from_rows(quotes_root(), quotes_rows()).unwrap();
    let batch = serie.into_arrow_batch().unwrap();

    assert_eq!(batch.num_rows(), 2);
    assert_eq!(batch.num_columns(), 2);
    assert_eq!(batch.schema().field(0).name(), "id");
    assert_eq!(batch.schema().field(1).name(), "symbol");

    let read = Serie::from_arrow_batch(&batch).unwrap();
    assert_eq!(read.len(), 2);
    assert_eq!(read.as_slice(), serie.as_slice());
    assert_eq!(read.field().dtype(), serie.field().dtype());

    // A column of a leaf field is not a table, and says so rather than
    // guessing a root around it.
    assert!(prices().into_arrow_batch().is_err());
}

#[test]
fn a_record_root_column_streams_as_one_reader_and_reads_back_from_one() {
    let serie = Serie::from_rows(quotes_root(), quotes_rows()).unwrap();
    let reader = serie.into_arrow_reader().unwrap();

    // The schema is stated before a batch is pulled, which is what a record
    // write reads it by.
    assert_eq!(reader.schema().fields().len(), 2);

    let batches: Vec<RecordBatch> = reader.map(|batch| batch.unwrap()).collect();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 2);

    let drained = Serie::from_arrow_reader(serie.into_arrow_reader().unwrap()).unwrap();
    assert_eq!(drained.as_slice(), serie.as_slice());
    assert_eq!(drained.field().dtype(), serie.field().dtype());
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

    // A schema-free run still writes and reads back as one.
    let run = Scalar::from_sequence([Scalar::from(1_i64)]);
    let written = serde_json::to_string(&run).unwrap();
    assert!(written.contains("\"sequence\""), "{written}");
    assert_eq!(serde_json::from_str::<Scalar>(&written).unwrap(), run);
}

#[test]
fn the_sequence_family_reads_back_whichever_leaf_a_document_holds() {
    // The family's own wire, rather than the scalar wire above: a run writes
    // its values and a column writes the field beside them, and reading one
    // back tells them apart by the shape of the document alone.
    let serie = Sequence::from(prices());
    let run = Sequence::new([Scalar::from(1_i64), Scalar::from(2_i64)]);

    for held in [serie, run] {
        let document = serde_json::to_string(&held).unwrap();
        let read: Sequence = serde_json::from_str(&document).unwrap();
        assert_eq!(read, held, "{document}");
        assert_eq!(read.as_slice(), held.as_slice());
    }

    // A column's rows are rechecked against the field the document carries,
    // so a document whose rows that field refuses is refused.
    let refused = r#"{"field":"price: int64","rows":[null]}"#;
    assert!(serde_json::from_str::<Sequence>(refused).is_err());
}

#[test]
fn a_column_clone_shares_its_rows_rather_than_copying_them() {
    let serie = prices();
    let copy = serie.clone();
    assert!(std::ptr::eq(serie.as_slice(), copy.as_slice()));
    assert_eq!(serie, copy);
}
