//! `rust/src/arrow/scalars.rs`: what the generic Arrow scalar family holds,
//! and what it narrows to.

use super::root;

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Datum, Int64Array, RecordBatch, StringArray, StructArray};

use yggdryl::arrow::batch_reader;
use yggdryl::{ArrowCastOptions, DataType, Field, StructType};
use yggdryl::{ArrowScalar, ArrowShape};

fn quote_root() -> Field {
    StructType::from_fields([
        DataType::utf8().required_field("symbol"),
        DataType::Int64.required_field("size"),
    ])
    .map(DataType::from)
    .expect("the root datatype is valid")
    .required_field("row")
}

fn quote_batch() -> RecordBatch {
    let schema = quote_root()
        .into_arrow_schema()
        .expect("the root projects to Arrow");
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])) as ArrayRef,
            Arc::new(Int64Array::from(vec![100_i64, 250])),
        ],
    )
    .expect("the columns match the schema")
}

fn prices() -> ArrayRef {
    Arc::new(Int64Array::from(vec![125_i64, 126, 127]))
}

mod shapes {

    use super::{ArrowScalar, ArrowShape, Field, prices, quote_batch, quote_root};
    use yggdryl::StructType;
    use yggdryl::{DataType, Scalar};

    #[test]
    fn every_shape_reports_itself_and_its_width() {
        let field = Field::new("price", DataType::Int64, false);
        let scalar = ArrowScalar::from_value(&field, &Scalar::from(125_i64))
            .expect("one value materializes");
        let array = ArrowScalar::from_array(field, prices()).expect("the column pairs");
        let batch = ArrowScalar::from_batch(quote_batch()).expect("the batch names its root");

        assert_eq!(scalar.shape(), ArrowShape::Scalar);
        assert_eq!(scalar.row_size(), Some(1));
        assert_eq!(scalar.column_size(), 1);

        assert_eq!(array.shape(), ArrowShape::Array);
        assert_eq!(array.row_size(), Some(3));

        assert_eq!(batch.shape(), ArrowShape::Batch);
        assert_eq!(batch.row_size(), Some(2));
        assert_eq!(batch.column_size(), 2);
        assert_eq!(batch.field().name(), "row");
    }

    #[test]
    fn a_stream_states_its_root_without_being_pulled() {
        let reader = yggdryl::arrow::batch_reader(
            quote_root()
                .into_arrow_schema()
                .expect("the root projects to Arrow"),
            [quote_batch()],
        );
        let value = ArrowScalar::from_reader(reader).expect("the reader names its root");

        // Nothing was read: a stream has no length until it is drained.
        assert_eq!(value.shape(), ArrowShape::Stream);
        assert_eq!(value.row_size(), None);
        assert_eq!(value.column_size(), 2);
        assert!(value.shape().is_streamed());
        assert!(value.shape().is_tabular());
    }

    #[test]
    fn shape_names_round_trip_through_their_canonical_spelling() {
        for shape in ArrowShape::ALL {
            assert_eq!(
                ArrowShape::from_str(shape.as_str()).expect("a canonical name parses"),
                shape
            );
            assert_eq!(shape.to_string(), shape.as_str());
        }
        assert!(ArrowShape::from_str("table").is_err());
    }

    #[test]
    fn a_columns_width_is_the_width_of_the_root_its_rows_live_under() {
        let structure = StructType::from_fields([
            DataType::utf8().required_field("symbol"),
            DataType::Int64.required_field("size"),
        ])
        .map(DataType::from)
        .expect("the struct datatype is valid");

        // A non-null struct column is its own root, so its children are the
        // columns...
        let rows = ArrowScalar::from_batch(quote_batch()).expect("the batch names its root");
        let array = rows.into_array().expect("the rows narrow");
        let own = ArrowScalar::from_array(structure.clone().required_field("row"), array.clone())
            .expect("the column pairs");
        assert_eq!(own.column_size(), 2);

        // ...and a nullable one is not a root, so it is one column of a
        // wrapping root, which is also what laying it out as rows produces.
        let wrapped = ArrowScalar::from_array(structure.nullable_field("row"), array)
            .expect("the column pairs");
        assert_eq!(wrapped.column_size(), 1);
        assert_eq!(
            wrapped
                .into_batch()
                .expect("the column lays out as rows")
                .num_columns(),
            1
        );
    }
}

mod pairing {

    use super::{ArrowScalar, Field, prices, quote_batch, quote_root};
    use yggdryl::StructType;
    use yggdryl::{DataType, Scalar};

    #[test]
    fn a_scalar_holds_exactly_one_row() {
        let field = Field::new("price", DataType::Int64, false);
        assert!(ArrowScalar::from_scalar_array(field.clone(), prices()).is_err());

        let one = prices().slice(0, 1);
        assert!(ArrowScalar::from_scalar_array(field, one).is_ok());
    }

    #[test]
    fn a_column_must_carry_the_declared_layout() {
        // int64 values under a utf8 field are refused at the pairing, not at
        // the first read.
        let wrong = Field::new("price", DataType::utf8(), false);
        assert!(ArrowScalar::from_array(wrong, prices()).is_err());
    }

    #[test]
    fn a_declared_root_must_be_exactly_the_rows_own_schema() {
        let widened = StructType::from_fields([
            DataType::utf8().required_field("symbol"),
            DataType::Int64.nullable_field("size"),
        ])
        .map(DataType::from)
        .expect("the root datatype is valid")
        .required_field("row");

        // Nullability is part of the schema, so this is a mismatch and not a
        // silent conversion.
        assert!(ArrowScalar::from_batch_as(widened, quote_batch()).is_err());
        assert!(ArrowScalar::from_batch_as(quote_root(), quote_batch()).is_ok());
    }

    #[test]
    fn native_rows_cross_through_the_one_value_boundary() {
        let rows = Scalar::from_sequence([
            Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100_i64)]),
            Scalar::from_sequence([Scalar::from("MSFT"), Scalar::from(250_i64)]),
        ]);
        let value = ArrowScalar::from_rows(&quote_root(), &rows).expect("the rows materialize");

        assert_eq!(value.row_size(), Some(2));
        assert_eq!(value.into_scalar().expect("the rows decode"), rows);
    }
}

mod narrowing {
    use super::{
        Array, ArrowScalar, Datum, Field, StructArray, batch_reader, prices, quote_batch,
        quote_root,
    };
    use yggdryl::{DataType, Scalar};

    #[test]
    fn every_shape_widens_to_the_one_reader_a_record_write_takes() {
        let field = Field::new("price", DataType::Int64, false);
        let cases = [
            ArrowScalar::from_array(field, prices()).expect("the column pairs"),
            ArrowScalar::from_batch(quote_batch()).expect("the batch names its root"),
        ];
        let expected = [3, 2];
        for (value, rows) in cases.into_iter().zip(expected) {
            let reader = value.into_reader().expect("the value widens");
            assert_eq!(
                reader
                    .map(|batch| batch.expect("a batch reads").num_rows())
                    .sum::<usize>(),
                rows
            );
        }
    }

    #[test]
    fn a_bare_column_becomes_the_single_column_of_a_default_root() {
        let field = Field::new("price", DataType::Int64, false);
        let value = ArrowScalar::from_array(field, prices()).expect("the column pairs");

        let batch = value.into_batch().expect("the column lays out as rows");
        assert_eq!(batch.num_columns(), 1);
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.schema().field(0).name(), "price");
    }

    #[test]
    fn a_batch_narrows_to_the_struct_column_its_rows_already_are() {
        let value = ArrowScalar::from_batch(quote_batch()).expect("the batch names its root");
        let array = value.into_array().expect("the rows narrow");
        let structs = array
            .as_any()
            .downcast_ref::<StructArray>()
            .expect("rows are a struct column");

        assert_eq!(structs.len(), 2);
        assert_eq!(structs.num_columns(), 2);
    }

    #[test]
    fn draining_a_stream_concatenates_every_batch_it_yielded() {
        let schema = quote_root()
            .into_arrow_schema()
            .expect("the root projects to Arrow");
        let reader = batch_reader(schema, [quote_batch(), quote_batch()]);
        let value = ArrowScalar::from_reader(reader).expect("the reader names its root");

        assert_eq!(value.into_batch().expect("the stream drains").num_rows(), 4);
    }

    #[test]
    fn only_a_one_row_value_becomes_an_arrow_datum() {
        let field = Field::new("price", DataType::Int64, false);
        let many = ArrowScalar::from_array(field.clone(), prices()).expect("the column pairs");
        assert!(many.into_arrow_scalar().is_err());

        let one = ArrowScalar::from_value(&field, &Scalar::from(125_i64))
            .expect("one value materializes");
        assert_eq!(
            one.into_arrow_scalar()
                .expect("one row is a datum")
                .get()
                .0
                .len(),
            1
        );
    }

    #[test]
    fn a_stream_of_rows_decodes_as_one_sequence() {
        let schema = quote_root()
            .into_arrow_schema()
            .expect("the root projects to Arrow");
        let reader = batch_reader(schema, [quote_batch()]);
        let value = ArrowScalar::from_reader(reader).expect("the reader names its root");
        let rows = value.into_scalar().expect("the stream decodes");

        assert_eq!(rows.as_sequence().map(<[Scalar]>::len), Some(2));
    }
}

mod casting {

    use super::{ArrowCastOptions, ArrowScalar, Field, batch_reader, prices};
    use yggdryl::StructType;
    use yggdryl::{DataType, Scalar};

    #[test]
    fn a_column_is_reshaped_and_keeps_its_shape() {
        let source = Field::new("price", DataType::Int64, false);
        let target = Field::new("price", DataType::Float64, false);
        let value = ArrowScalar::from_array(source, prices()).expect("the column pairs");

        let cast = value
            .cast(&target, ArrowCastOptions::new())
            .expect("int64 widens to float64");
        assert!(cast.is_array());
        assert_eq!(cast.field().dtype(), &DataType::Float64);
        assert_eq!(cast.row_size(), Some(3));
    }

    #[test]
    fn a_stream_is_cast_one_batch_at_a_time_under_one_plan() {
        let source = StructType::from_fields([DataType::Int64.required_field("size")])
            .map(DataType::from)
            .expect("the root datatype is valid")
            .required_field("row");
        let target = StructType::from_fields([DataType::Float64.required_field("size")])
            .map(DataType::from)
            .expect("the root datatype is valid")
            .required_field("row");
        let batch = yggdryl::arrow::batch_from_value(
            &source,
            &Scalar::from_sequence([Scalar::from_sequence([Scalar::from(100_i64)])]),
        )
        .expect("the row materializes");
        let reader = batch_reader(batch.schema(), [batch]);

        let value = ArrowScalar::from_reader(reader)
            .expect("the reader names its root")
            .cast(&target, ArrowCastOptions::new())
            .expect("the plan compiles from the reader schema");

        assert!(value.is_stream());
        assert_eq!(
            value
                .into_reader()
                .expect("a stream is already a reader")
                .map(|batch| batch.expect("a batch reads").num_rows())
                .sum::<usize>(),
            1
        );
    }
}

mod encodings {
    use super::root;
    use arrow_array::RecordBatch;
    use yggdryl::DecimalType;
    use yggdryl::arrow::{batch_reader, batch_to_value};
    use yggdryl::holder::Buffer;
    use yggdryl::{
        ArrowCastOptions, ArrowScalar, ArrowShape, DataType, Field, IOBase, IOMedia, IOMode,
        Scalar, StructType, Url,
    };

    /// An in-memory handle whose media type is the one its name implies.
    fn handle(name: &str) -> Buffer {
        Buffer::new().with_media_type(
            Url::from_str(&format!("file:///{name}"))
                .expect("the URL parses")
                .media_type(),
        )
    }

    fn quote_root() -> Field {
        root([
            DataType::utf8().required_field("symbol"),
            DataType::Int64.required_field("size"),
        ])
    }

    fn quote_rows() -> Vec<Scalar> {
        vec![
            Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100_i64)]),
            Scalar::from_sequence([Scalar::from("MSFT"), Scalar::from(250_i64)]),
        ]
    }

    /// The one section a structured document reads off record options: the
    /// declared field, carried by any record encoding's options.
    fn declaring(field: &Field) -> yggdryl::media::RecordOptions {
        use yggdryl::media::IORecordOptions;

        let mut options = yggdryl::media::RecordOptions::for_media_type(&yggdryl::MediaType::new(
            yggdryl::MimeType::ARROW_STREAM,
        ))
        .expect("the IPC encoding is built in");
        options.set_field(field.clone());
        options
    }

    fn quotes() -> ArrowScalar {
        ArrowScalar::from_rows(&quote_root(), &Scalar::from_sequence(quote_rows()))
            .expect("the rows materialize")
    }

    fn quote_batch() -> RecordBatch {
        quotes().into_batch().expect("the rows are already held")
    }

    /// A reader over `count` copies of the quotes.
    fn quote_stream(count: usize) -> ArrowScalar {
        let schema = quote_root()
            .into_arrow_schema()
            .expect("the root projects to Arrow");
        let batches: Vec<RecordBatch> = (0..count).map(|_| quote_batch()).collect();
        ArrowScalar::from_reader(batch_reader(schema, batches)).expect("the reader names its root")
    }

    /// One value of each shape, labelled, with the root its rows land under and
    /// the rows they spell once laid out.
    fn shaped_values() -> Vec<(ArrowShape, ArrowScalar, Field, Scalar)> {
        let price = DataType::Int64.required_field("price");
        let price_root = root([price.clone()]);
        let priced = |value: i64| Scalar::from_sequence([Scalar::from(value)]);

        vec![
            (
                ArrowShape::Scalar,
                ArrowScalar::from_value(&price, &Scalar::from(125_i64))
                    .expect("one value materializes"),
                price_root.clone(),
                Scalar::from_sequence([priced(125)]),
            ),
            (
                ArrowShape::Array,
                ArrowScalar::from_values(
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
            StructType::from_fields([
                DataType::utf8().required_field("mic"),
                DataType::Int64.required_field("rank"),
            ])
            .map(DataType::from)
            .expect("the child datatype is valid")
            .required_field("venue"),
            DataType::list(DataType::Int64.required_field("item")).required_field("sizes"),
            DataType::utf8().nullable_field("note"),
            DataType::Decimal(DecimalType::Decimal128 {
                precision: 12,
                scale: 2,
            })
            .required_field("price"),
            DataType::date32().required_field("day"),
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

    fn nested() -> ArrowScalar {
        ArrowScalar::from_rows(&nested_root(), &Scalar::from_sequence(nested_rows()))
            .expect("the rows materialize")
    }

    mod structured_text {
        use super::{
            ArrowShape, Field, IOBase, IOMedia, IOMode, Scalar, declaring, handle, quotes,
            shaped_values,
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
            // The fixture table is the claim that all four shapes are covered, so
            // it is checked against the shape list itself rather than against the
            // label written beside each value.
            let covered: Vec<_> = shaped_values().into_iter().map(|entry| entry.0).collect();
            assert_eq!(covered, ArrowShape::ALL);

            for format in ["json", "jsonl", "yaml", "toml"] {
                for (shape, value, root, rows) in shaped_values() {
                    assert_eq!(value.shape(), shape, "the {shape} fixture");
                    let mut target = handle(&format!("shaped.{format}"));
                    target
                        .write_arrow(value, IOMode::Overwrite, None)
                        .unwrap_or_else(|error| panic!("a {shape} writes to {format}: {error}"));

                    let read = target
                        .read_arrow(Some(&declaring(&root)))
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
            target
                .write_arrow(quotes(), IOMode::Overwrite, None)
                .expect("the rows write");
            let published = target.read_all_bytes().expect("the bytes read");
            assert!(!published.is_empty());

            let refused = target
                .write_arrow(quotes(), IOMode::Append, None)
                .expect_err("a document has no append");
            assert!(refused.to_string().contains("append"), "{refused}");

            // The refusal precedes the encoding, so the document that was already
            // there is byte-for-byte what it was - not truncated, not appended to.
            assert_eq!(target.read_all_bytes().expect("the bytes read"), published);
        }

        #[test]
        fn a_document_read_without_a_root_orders_the_columns_the_way_a_record_does() {
            let mut target = handle("quotes.jsonl");
            target
                .write_arrow(quotes(), IOMode::Overwrite, None)
                .expect("the rows write");

            // Nothing is declared, so the root is what the document proves - and a
            // document names its values rather than ordering them, which is why
            // the inferred columns are sorted and not the declaration's order.
            let read = target.read_arrow(None).expect("the document proves a root");
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
            ArrowScalar, ArrowShape, DataType, DecimalType, IOMedia, IOMode, Scalar, declaring,
            handle, quote_root, quote_rows, quotes, root,
        };

        /// The rows `name` holds after `quotes()` was written to it.
        fn stored(name: &str) -> ArrowScalar {
            let mut target = handle(name);
            target
                .write_arrow(quotes(), IOMode::Overwrite, None)
                .unwrap_or_else(|error| panic!("{name} writes: {error}"));
            target
                .read_arrow(None)
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
                .write_arrow(quotes(), IOMode::Overwrite, None)
                .expect("the rows write");
            target
                .write_arrow(quotes(), IOMode::Append, None)
                .expect("the rows append");

            let read = target.read_arrow(None).expect("the rows read");
            assert_eq!(
                read.into_scalar().expect("the rows decode"),
                Scalar::from_sequence([quote_rows(), quote_rows()].concat())
            );
        }

        #[test]
        fn a_declared_root_casts_the_rows_a_record_encoding_stored() {
            let mut target = handle("quotes.arrows");
            target
                .write_arrow(quotes(), IOMode::Overwrite, None)
                .expect("the rows write");

            let declared = root([
                DataType::utf8().required_field("symbol"),
                DataType::Decimal(DecimalType::Decimal128 {
                    precision: 12,
                    scale: 4,
                })
                .required_field("size"),
            ]);
            let read = target
                .read_arrow(Some(&declaring(&declared)))
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
            let empty = ArrowScalar::from_rows(&quote_root(), &Scalar::from_sequence([]))
                .expect("no rows still materialize");
            assert_eq!(empty.row_size(), Some(0));

            let mut target = handle("empty.arrows");
            target
                .write_arrow(empty, IOMode::Overwrite, None)
                .expect("the rows write");

            let read = target.read_arrow(None).expect("the rows read");
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
            let value = ArrowScalar::from_rows(&quote_root(), &Scalar::from_sequence(rows.clone()))
                .expect("the rows materialize");

            let mut target = handle("wide.arrows");
            target
                .write_arrow(value, IOMode::Overwrite, None)
                .expect("the rows write");

            let read = target.read_arrow(None).expect("the rows read");
            assert_eq!(
                read.into_scalar().expect("the rows decode"),
                Scalar::from_sequence(rows)
            );
        }
    }

    mod nesting {
        use super::{IOMedia, IOMode, Scalar, declaring, handle, nested, nested_root, nested_rows};

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
                    .write_arrow(nested(), IOMode::Overwrite, None)
                    .unwrap_or_else(|error| panic!("{name} writes: {error}"));

                let read = target
                    .read_arrow(Some(&declaring(&nested_root())))
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
                .write_arrow(nested(), IOMode::Overwrite, None)
                .expect("the rows write");

            // Nothing is declared on the read: the struct child, the list item,
            // the nullable column, the decimal, and the temporal all come back
            // named and parameterized by the schema the write stored.
            let read = target
                .read_arrow(None)
                .expect("the stored schema names the columns");
            assert_eq!(read.field(), &nested_root());
        }
    }

    mod casting {
        use super::{ArrowCastOptions, DataType, Scalar, batch_to_value, quote_stream, root};

        #[test]
        fn one_compiled_plan_casts_every_batch_a_stream_yields() {
            let target = root([
                DataType::utf8().required_field("symbol"),
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
}
