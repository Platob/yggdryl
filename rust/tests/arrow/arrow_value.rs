//! One Arrow value across every encoding the public surface reaches.

use arrow_array::RecordBatch;
use yggdryl::arrow::{batch_reader, batch_to_value};
use yggdryl::holder::Buffer;
use yggdryl::{
    ArrowCastOptions, ArrowShape, ArrowValue, DataType, Field, IOBase, IOMedia, IOMode, Scalar, Url,
};

/// An in-memory handle whose media type is the one its name implies.
fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .expect("the URL parses")
            .media_type(),
    )
}

fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    DataType::from_fields(fields)
        .expect("the root datatype is valid")
        .required_field("row")
}

fn quote_root() -> Field {
    root([
        DataType::Utf8.required_field("symbol"),
        DataType::Int64.required_field("size"),
    ])
}

fn quote_rows() -> Vec<Scalar> {
    vec![
        Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100_i64)]),
        Scalar::from_sequence([Scalar::from("MSFT"), Scalar::from(250_i64)]),
    ]
}

fn quotes() -> ArrowValue {
    ArrowValue::from_rows(&quote_root(), &Scalar::from_sequence(quote_rows()))
        .expect("the rows materialize")
}

fn quote_batch() -> RecordBatch {
    quotes().into_batch().expect("the rows are already held")
}

/// A reader over `count` copies of the quotes.
fn quote_stream(count: usize) -> ArrowValue {
    let schema = quote_root()
        .into_arrow_schema()
        .expect("the root projects to Arrow");
    let batches: Vec<RecordBatch> = (0..count).map(|_| quote_batch()).collect();
    ArrowValue::from_reader(batch_reader(schema, batches)).expect("the reader names its root")
}

/// One value of each shape, labelled, with the root its rows land under and
/// the rows they spell once laid out.
fn shaped_values() -> Vec<(ArrowShape, ArrowValue, Field, Scalar)> {
    let price = DataType::Int64.required_field("price");
    let price_root = root([price.clone()]);
    let priced = |value: i64| Scalar::from_sequence([Scalar::from(value)]);

    vec![
        (
            ArrowShape::Scalar,
            ArrowValue::from_value(&price, &Scalar::from(125_i64)).expect("one value materializes"),
            price_root.clone(),
            Scalar::from_sequence([priced(125)]),
        ),
        (
            ArrowShape::Array,
            ArrowValue::from_values(
                &price,
                &Scalar::from_sequence([Scalar::from(125_i64), Scalar::from(126_i64)]),
            )
            .expect("the column materializes"),
            price_root,
            Scalar::from_sequence([priced(125), priced(126)]),
        ),
        (
            ArrowShape::Batch,
            quotes(),
            quote_root(),
            Scalar::from_sequence(quote_rows()),
        ),
        (
            ArrowShape::Stream,
            quote_stream(2),
            quote_root(),
            Scalar::from_sequence([quote_rows(), quote_rows()].concat()),
        ),
    ]
}

fn nested_root() -> Field {
    root([
        DataType::from_fields([
            DataType::Utf8.required_field("mic"),
            DataType::Int64.required_field("rank"),
        ])
        .expect("the child datatype is valid")
        .required_field("venue"),
        DataType::list(DataType::Int64.required_field("item")).required_field("sizes"),
        DataType::Utf8.nullable_field("note"),
        DataType::Decimal128 {
            precision: 12,
            scale: 2,
        }
        .required_field("price"),
        DataType::Date32.required_field("day"),
    ])
}

fn nested_rows() -> Vec<Scalar> {
    vec![
        Scalar::from_sequence([
            Scalar::from_sequence([Scalar::from("XPAR"), Scalar::from(1_i64)]),
            Scalar::from_sequence([Scalar::from(100_i64), Scalar::from(250_i64)]),
            Scalar::from("lit"),
            Scalar::d128(12_550, 2),
            Scalar::date32(19_876),
        ]),
        Scalar::from_sequence([
            Scalar::from_sequence([Scalar::from("XNAS"), Scalar::from(2_i64)]),
            Scalar::from_sequence([]),
            Scalar::Null,
            Scalar::d128(1, 2),
            Scalar::date32(0),
        ]),
    ]
}

fn nested() -> ArrowValue {
    ArrowValue::from_rows(&nested_root(), &Scalar::from_sequence(nested_rows()))
        .expect("the rows materialize")
}

mod structured_text {
    use super::{
        ArrowShape, Field, IOBase, IOMedia, IOMode, Scalar, handle, quotes, shaped_values,
    };

    /// The number of values one of `rows`' rows carries.
    fn rows_width(rows: &Scalar) -> usize {
        rows.as_sequence()
            .and_then(<[Scalar]>::first)
            .and_then(Scalar::as_sequence)
            .map_or(0, <[Scalar]>::len)
    }

    #[test]
    fn every_shape_lands_as_the_same_rows_in_every_structured_format() {
        for format in ["json", "jsonl", "yaml", "toml"] {
            for (shape, value, root, rows) in shaped_values() {
                assert_eq!(value.shape(), shape);
                let mut target = handle(&format!("shaped.{format}"));
                target
                    .write_arrow_value(value, IOMode::Overwrite)
                    .unwrap_or_else(|error| panic!("a {shape} writes to {format}: {error}"));

                let read = target
                    .read_arrow_value(Some(&root))
                    .unwrap_or_else(|error| panic!("a {shape} reads from {format}: {error}"));
                // A document has no frame to read a prefix of, so the four
                // shapes converge on the one held batch their rows parse into.
                assert_eq!(read.shape(), ArrowShape::Batch, "a {shape} in {format}");
                assert_eq!(
                    read.column_size(),
                    rows_width(&rows),
                    "a {shape} in {format}"
                );
                assert_eq!(
                    read.into_scalar().expect("the rows decode"),
                    rows,
                    "a {shape} in {format}"
                );
            }
        }
    }

    #[test]
    fn an_append_is_refused_naming_the_mode_a_document_cannot_take() {
        let mut target = handle("quotes.yaml");
        let refused = target
            .write_arrow_value(quotes(), IOMode::Append)
            .expect_err("a document has no append");

        assert!(refused.to_string().contains("append"), "{refused}");
        // The refusal precedes the encoding, so no partial document was
        // published under a mode the format cannot honour.
        assert!(target.read_all_bytes().expect("the bytes read").is_empty());
    }

    #[test]
    fn a_document_read_without_a_root_orders_the_columns_the_way_a_record_does() {
        let mut target = handle("quotes.jsonl");
        target
            .write_arrow_value(quotes(), IOMode::Overwrite)
            .expect("the rows write");

        // Nothing is declared, so the root is what the document proves - and a
        // document names its values rather than ordering them, which is why
        // the inferred columns are sorted and not the declaration's order.
        let read = target
            .read_arrow_value(None)
            .expect("the document proves a root");
        let names: Vec<&str> = read
            .dtype()
            .as_fields()
            .expect("a struct root")
            .iter()
            .map(Field::name)
            .collect();
        assert_eq!(names, ["size", "symbol"]);
        assert_eq!(
            read.into_scalar().expect("the rows decode"),
            Scalar::from_sequence([
                Scalar::from_sequence([Scalar::from(100_i64), Scalar::from("AAPL")]),
                Scalar::from_sequence([Scalar::from(250_i64), Scalar::from("MSFT")]),
            ])
        );
    }
}

mod record_encodings {
    use super::{
        ArrowShape, ArrowValue, DataType, IOMedia, IOMode, Scalar, handle, quote_root, quote_rows,
        quotes, root,
    };

    /// The rows `name` holds after `quotes()` was written to it.
    fn stored(name: &str) -> ArrowValue {
        let mut target = handle(name);
        target
            .write_arrow_value(quotes(), IOMode::Overwrite)
            .unwrap_or_else(|error| panic!("{name} writes: {error}"));
        target
            .read_arrow_value(None)
            .unwrap_or_else(|error| panic!("{name} reads: {error}"))
    }

    #[test]
    fn every_unconditional_record_encoding_round_trips_under_its_stored_schema() {
        for name in ["quotes.arrows", "quotes.avro"] {
            let read = stored(name);
            // A record encoding is framed, so the read is a stream that has
            // not been pulled and states no length yet.
            assert_eq!(read.shape(), ArrowShape::Stream, "{name}");
            assert_eq!(read.row_size(), None, "{name}");
            assert_eq!(read.field(), &quote_root(), "{name}");
            assert_eq!(
                read.into_scalar().expect("the rows decode"),
                Scalar::from_sequence(quote_rows()),
                "{name}"
            );
        }
    }

    #[cfg(feature = "parquet")]
    #[test]
    fn parquet_round_trips_the_same_rows_under_the_same_root() {
        let read = stored("quotes.parquet");
        assert_eq!(read.field(), &quote_root());
        assert_eq!(
            read.into_scalar().expect("the rows decode"),
            Scalar::from_sequence(quote_rows())
        );
    }

    #[test]
    fn an_append_keeps_the_rows_a_record_encoding_already_holds() {
        let mut target = handle("quotes.arrows");
        target
            .write_arrow_value(quotes(), IOMode::Overwrite)
            .expect("the rows write");
        target
            .write_arrow_value(quotes(), IOMode::Append)
            .expect("the rows append");

        let read = target.read_arrow_value(None).expect("the rows read");
        assert_eq!(
            read.into_scalar().expect("the rows decode"),
            Scalar::from_sequence([quote_rows(), quote_rows()].concat())
        );
    }

    #[test]
    fn a_declared_root_casts_the_rows_a_record_encoding_stored() {
        let mut target = handle("quotes.arrows");
        target
            .write_arrow_value(quotes(), IOMode::Overwrite)
            .expect("the rows write");

        let declared = root([
            DataType::Utf8.required_field("symbol"),
            DataType::Decimal128 {
                precision: 12,
                scale: 4,
            }
            .required_field("size"),
        ]);
        let read = target
            .read_arrow_value(Some(&declared))
            .expect("the declared root casts the stored int64 column");

        assert_eq!(read.field(), &declared);
        assert_eq!(
            read.into_scalar().expect("the rows decode"),
            Scalar::from_sequence([
                Scalar::from_sequence([Scalar::from("AAPL"), Scalar::d128(1_000_000, 4)]),
                Scalar::from_sequence([Scalar::from("MSFT"), Scalar::d128(2_500_000, 4)]),
            ])
        );
    }

    #[test]
    fn an_empty_table_round_trips_and_keeps_the_columns_it_declared() {
        let empty = ArrowValue::from_rows(&quote_root(), &Scalar::from_sequence([]))
            .expect("no rows still materialize");
        assert_eq!(empty.row_size(), Some(0));

        let mut target = handle("empty.arrows");
        target
            .write_arrow_value(empty, IOMode::Overwrite)
            .expect("the rows write");

        let read = target.read_arrow_value(None).expect("the rows read");
        // The schema is what an empty table carries, so it is the whole claim.
        assert_eq!(read.field(), &quote_root());
        assert_eq!(read.into_batch().expect("the stream drains").num_rows(), 0);
    }

    #[test]
    fn a_thousand_rows_survive_the_round_trip_in_order() {
        let rows: Vec<Scalar> = (0..1_000_i64)
            .map(|index| {
                Scalar::from_sequence([Scalar::from(format!("S{index}")), Scalar::from(index)])
            })
            .collect();
        let value = ArrowValue::from_rows(&quote_root(), &Scalar::from_sequence(rows.clone()))
            .expect("the rows materialize");

        let mut target = handle("wide.arrows");
        target
            .write_arrow_value(value, IOMode::Overwrite)
            .expect("the rows write");

        let read = target.read_arrow_value(None).expect("the rows read");
        assert_eq!(
            read.into_scalar().expect("the rows decode"),
            Scalar::from_sequence(rows)
        );
    }
}

mod nesting {
    use super::{IOMedia, IOMode, Scalar, handle, nested, nested_root, nested_rows};

    #[test]
    fn nested_children_keep_their_values_in_every_encoding_that_carries_them() {
        // TOML is absent deliberately: it has no null, and these rows have one.
        for name in [
            "nested.arrows",
            "nested.json",
            "nested.jsonl",
            "nested.yaml",
        ] {
            let mut target = handle(name);
            target
                .write_arrow_value(nested(), IOMode::Overwrite)
                .unwrap_or_else(|error| panic!("{name} writes: {error}"));

            let read = target
                .read_arrow_value(Some(&nested_root()))
                .unwrap_or_else(|error| panic!("{name} reads: {error}"));
            assert_eq!(
                read.into_scalar().expect("the rows decode"),
                Scalar::from_sequence(nested_rows()),
                "{name}"
            );
        }
    }

    #[test]
    fn a_record_encoding_names_every_nested_child_it_stored() {
        let mut target = handle("nested.arrows");
        target
            .write_arrow_value(nested(), IOMode::Overwrite)
            .expect("the rows write");

        // Nothing is declared on the read: the struct child, the list item,
        // the nullable column, the decimal, and the temporal all come back
        // named and parameterized by the schema the write stored.
        let read = target
            .read_arrow_value(None)
            .expect("the stored schema names the columns");
        assert_eq!(read.field(), &nested_root());
    }
}

mod casting {
    use super::{ArrowCastOptions, DataType, Scalar, batch_to_value, quote_stream, root};

    #[test]
    fn one_compiled_plan_casts_every_batch_a_stream_yields() {
        let target = root([
            DataType::Utf8.required_field("symbol"),
            DataType::Float64.required_field("size"),
        ]);
        let cast = quote_stream(3)
            .cast(&target, ArrowCastOptions::new())
            .expect("int64 widens to float64");
        assert!(cast.is_stream());

        let expected = target
            .clone()
            .into_arrow_schema()
            .expect("the root projects to Arrow");
        let batches: Vec<_> = cast
            .into_reader()
            .expect("a stream is already a reader")
            .map(|batch| batch.expect("a batch reads"))
            .collect();

        // The plan is compiled once from the reader schema, so every batch -
        // not only the first - arrives under the declared root.
        assert_eq!(batches.len(), 3);
        for batch in &batches {
            assert_eq!(batch.schema(), expected);
            assert_eq!(
                batch_to_value(batch).expect("the rows decode"),
                Scalar::from_sequence([
                    Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100.0_f64)]),
                    Scalar::from_sequence([Scalar::from("MSFT"), Scalar::from(250.0_f64)]),
                ])
            );
        }
    }
}
