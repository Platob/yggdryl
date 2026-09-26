//! `rust/src/media/structured.rs`: what a structured text document carries
//! into a record column, and back out.

use yggdryl::holder::Buffer;
use yggdryl::{
    ArrowCastOptions, DataType, Field, IOBase, IOMedia, IOMode, Scalar, Serie, SerieReader,
    StructType, Url,
};

fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .expect("the URL parses")
            .media_type(),
    )
}

/// The one section a structured document reads off record options: the
/// declared field. A document has no encoding options of its own, so the
/// options are any record encoding's, carrying the field.
fn options_declaring(field: &Field) -> yggdryl::media::RecordOptions {
    use yggdryl::media::IORecordOptions;

    let mut options = yggdryl::media::RecordOptions::for_media_type(&yggdryl::MediaType::new(
        yggdryl::MimeType::ARROW_STREAM,
    ))
    .expect("the IPC encoding is built in");
    options.set_field(field.clone());
    options
}

/// The non-null record root a document's rows land under.
fn record(fields: impl IntoIterator<Item = Field>) -> Field {
    StructType::from_fields(fields)
        .map(DataType::from)
        .expect("the root datatype is valid")
        .required_field("row")
}

fn quote_root() -> Field {
    record([
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

/// The quotes as the one record column a write takes.
fn quote_column() -> Serie {
    Serie::from_scalars(quote_root(), quote_rows()).expect("the rows materialize")
}

/// The quotes as the stream of one record column a write takes.
fn quotes() -> SerieReader {
    SerieReader::from_serie(quote_column()).expect("a record column is one stream")
}

/// The one record column a document reads as.
///
/// A document has no frame to read a prefix of, so whatever was written
/// converges on the one column its rows parse into.
fn the_one_column(reader: SerieReader, context: &str) -> Serie {
    let columns = reader
        .collect::<Result<Vec<Serie>, _>>()
        .unwrap_or_else(|error| panic!("{context} lands: {error}"));
    assert_eq!(columns.len(), 1, "{context} reads as one column");
    columns.into_iter().next().expect("one column")
}

#[test]
fn every_structured_format_round_trips_arrow_rows() {
    for name in [
        "quotes.json",
        "quotes.jsonl",
        "quotes.yaml",
        "quotes.toml",
        "quotes.xml",
    ] {
        let mut target = handle(name);
        target
            .write_arrow(quotes(), IOMode::Overwrite, None)
            .unwrap_or_else(|error| panic!("{name} writes: {error}"));

        let read = target
            .read_arrow(Some(&options_declaring(&quote_root())))
            .unwrap_or_else(|error| panic!("{name} reads: {error}"));
        let column = the_one_column(read, name);
        assert_eq!(column.len(), 2, "{name}");
        assert_eq!(Scalar::from(column), Scalar::from(quote_column()), "{name}");
    }
}

#[test]
fn every_stream_lands_as_the_same_rows_in_every_structured_format() {
    let price = DataType::Int64.required_field("price");
    let price_root = record([price.clone()]);
    let priced = |value: i64| Scalar::from_sequence([Scalar::from(value)]);
    let prices = |values: &[i64]| {
        let rows = values.iter().map(|value| Scalar::from(*value));
        Serie::from_scalars(price.clone(), rows).expect("the column materializes")
    };
    let quote_batch = quote_column()
        .into_arrow_batch()
        .expect("a record column is one table");

    // Every way a stream of record columns is built: a held column that is
    // not a record - one row, or many - is the one child of a record, a held
    // record column is itself, and a stream is each of its batches.
    let sources = || {
        vec![
            (
                "one value",
                SerieReader::from_serie(prices(&[125])).expect("a column is one stream"),
                price_root.clone(),
                Scalar::from_sequence([priced(125)]),
            ),
            (
                "a column",
                SerieReader::from_serie(prices(&[125, 126])).expect("a column is one stream"),
                price_root.clone(),
                Scalar::from_sequence([priced(125), priced(126)]),
            ),
            (
                "a record column",
                quotes(),
                quote_root(),
                Scalar::from_sequence(quote_rows()),
            ),
            (
                "a stream",
                SerieReader::from_arrow_reader(
                    None,
                    yggdryl::arrow::batch_reader(
                        quote_batch.schema(),
                        [quote_batch.clone(), quote_batch.clone()],
                    ),
                    ArrowCastOptions::default(),
                )
                .expect("the reader names its root"),
                quote_root(),
                Scalar::from_sequence([quote_rows(), quote_rows()].concat()),
            ),
        ]
    };

    for format in ["json", "jsonl", "yaml", "toml", "xml"] {
        for (source, value, root, rows) in sources() {
            let mut target = handle(&format!("sourced.{format}"));
            target
                .write_arrow(value, IOMode::Overwrite, None)
                .unwrap_or_else(|error| panic!("{source} writes to {format}: {error}"));

            let read = target
                .read_arrow(Some(&options_declaring(&root)))
                .unwrap_or_else(|error| panic!("{source} reads from {format}: {error}"));
            let column = the_one_column(read, &format!("{source} in {format}"));
            let width = rows
                .as_sequence()
                .and_then(<[Scalar]>::first)
                .and_then(Scalar::as_sequence)
                .map_or(0, <[Scalar]>::len);
            assert_eq!(column.children().len(), width, "{source} in {format}");
            assert_eq!(Scalar::from(column), rows, "{source} in {format}");
        }
    }
}

#[test]
fn rows_are_written_with_the_names_their_field_declares() {
    let mut target = handle("quotes.jsonl");
    target
        .write_arrow(quotes(), IOMode::Overwrite, None)
        .expect("the rows write");

    let text = String::from_utf8(target.read_all_bytes().expect("the bytes read"))
        .expect("JSON Lines is UTF-8");
    // A canonical row is positional; the document names it, one row per line.
    assert_eq!(text.lines().count(), 2);
    assert!(text.contains(r#""symbol":"AAPL""#), "{text}");
    assert!(text.contains(r#""size":100"#), "{text}");
}

#[test]
fn a_declared_root_types_the_documents_natural_strings() {
    let mut source = handle("quotes.jsonl");
    // The size is a string, which is the point: a document spells a decimal in
    // text and the declared column is what reads it at its own scale. A bare
    // JSON number is deliberately not used here - `Field::scalar` reads one as
    // an unscaled coefficient rather than a whole value, which is a value
    // contract question this surface does not own.
    source
        .write_all_bytes(br#"{"symbol": "AAPL", "size": "100.00"}"#)
        .expect("the bytes write");

    let widened = record([
        DataType::utf8().required_field("symbol"),
        DataType::Decimal128 {
            precision: 12,
            scale: 2,
        }
        .required_field("size"),
    ]);

    let value = source
        .read_arrow(Some(&options_declaring(&widened)))
        .expect("the declared root types the document");
    assert_eq!(value.field(), &widened);
    let column = the_one_column(value, "the document");
    assert_eq!(column.field(), Some(&widened));
    assert_eq!(
        Scalar::from(column),
        Scalar::from_sequence([Scalar::from_sequence([
            Scalar::from("AAPL"),
            Scalar::d128(10_000, 2),
        ])])
    );
}

#[test]
fn an_undeclared_read_names_the_root_the_document_proves() {
    let mut source = handle("quotes.json");
    source
        .write_all_bytes(br#"[{"symbol": "AAPL", "size": 100}]"#)
        .expect("the bytes write");

    let value = source.read_arrow(None).expect("the document proves a root");
    let column = the_one_column(value, "the document");
    assert_eq!(column.children().len(), 2);
    assert_eq!(column.len(), 1);
}

#[test]
fn a_document_read_without_a_root_orders_the_columns_the_way_a_record_does() {
    let mut target = handle("quotes.jsonl");
    target
        .write_arrow(quotes(), IOMode::Overwrite, None)
        .expect("the rows write");

    // Nothing is declared, so the root is what the document proves - and a
    // document names its values rather than ordering them, which is why the
    // inferred columns are sorted and not the declaration's order.
    let read = target.read_arrow(None).expect("the document proves a root");
    let names: Vec<&str> = read
        .field()
        .dtype()
        .as_fields()
        .expect("a struct root")
        .iter()
        .map(Field::name)
        .collect();
    assert_eq!(names, ["size", "symbol"]);
    assert_eq!(
        Scalar::from(the_one_column(read, "the document")),
        Scalar::from_sequence([
            Scalar::from_sequence([Scalar::from(100_i64), Scalar::from("AAPL")]),
            Scalar::from_sequence([Scalar::from(250_i64), Scalar::from("MSFT")]),
        ])
    );
}

#[test]
fn one_document_that_is_not_a_sequence_is_one_row() {
    let mut source = handle("quote.yaml");
    source
        .write_all_bytes(b"symbol: AAPL\nsize: 100\n")
        .expect("the bytes write");

    let value = source
        .read_arrow(Some(&options_declaring(&quote_root())))
        .expect("one document is one row");
    assert_eq!(the_one_column(value, "the document").len(), 1);
}

#[test]
fn a_toml_table_travels_under_the_roots_own_name() {
    let mut target = handle("quotes.toml");
    target
        .write_arrow(quotes(), IOMode::Overwrite, None)
        .expect("the rows write");

    let text =
        String::from_utf8(target.read_all_bytes().expect("the bytes read")).expect("TOML is UTF-8");
    // TOML has no top-level sequence, so the rows travel under the root's own
    // name - which is also how the read finds them again.
    assert!(text.starts_with(r#""row" = ["#), "{text}");
    assert!(text.contains(r#""symbol" = "AAPL""#), "{text}");
}

#[test]
fn an_xml_document_holds_one_row_element_per_row_under_the_data_element() {
    let mut target = handle("quotes.xml");
    target
        .write_arrow(quotes(), IOMode::Overwrite, None)
        .expect("the rows write");

    let text =
        String::from_utf8(target.read_all_bytes().expect("the bytes read")).expect("XML is UTF-8");
    // XML has no top-level sequence either: the rows are one element each,
    // named after the root, under one document element - and that is how the
    // read finds them again.
    // A natural record names its values rather than ordering them, so the
    // columns come sorted, as every document here writes them.
    assert_eq!(
        text,
        "<data><row><size>100</size><symbol>AAPL</symbol></row>\
         <row><size>250</size><symbol>MSFT</symbol></row></data>"
    );

    // No rows is the document element with nothing in it, and reads back as
    // no rows.
    let mut empty = handle("empty.xml");
    let none = Serie::from_scalars(quote_root(), Vec::<Scalar>::new()).expect("no rows");
    empty
        .write_arrow(
            SerieReader::from_serie(none).expect("a record column is one stream"),
            IOMode::Overwrite,
            None,
        )
        .expect("no rows write");
    assert_eq!(
        empty.read_all_bytes().expect("the bytes read"),
        b"<data></data>"
    );
    let read = empty
        .read_arrow(Some(&options_declaring(&quote_root())))
        .expect("no rows read");
    assert_eq!(the_one_column(read, "no rows").len(), 0);

    // A document whose root is the row itself is one row.
    let mut single = handle("one.xml");
    single
        .write_all_bytes(b"<row><symbol>AAPL</symbol><size>100</size></row>")
        .expect("the bytes write");
    let read = single
        .read_arrow(Some(&options_declaring(&quote_root())))
        .expect("one row reads");
    assert_eq!(
        Scalar::from(the_one_column(read, "one row")),
        Scalar::from_sequence([quote_rows()[0].clone()])
    );

    // Without a field the columns are the text the document proves.
    let read = target.read_arrow(None).expect("the document proves a root");
    let column = the_one_column(read, "the document");
    assert_eq!(column.len(), 2);
    assert_eq!(
        Scalar::from(column),
        Scalar::from_sequence([
            Scalar::from_sequence([Scalar::from("100"), Scalar::from("AAPL")]),
            Scalar::from_sequence([Scalar::from("250"), Scalar::from("MSFT")]),
        ])
    );
}

#[test]
fn a_document_is_written_whole_so_only_an_overwrite_applies() {
    let mut target = handle("quotes.json");
    let refused = target
        .write_arrow(quotes(), IOMode::Append, None)
        .expect_err("a document has no append");
    assert!(refused.to_string().contains("overwrite"), "{refused}");
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
fn a_compressed_document_reads_and_writes_through_its_coding() {
    let mut target = handle("quotes.jsonl.gz");
    target
        .write_arrow(quotes(), IOMode::Overwrite, None)
        .expect("the rows write");

    // The bytes on the handle are gzip, not JSON Lines.
    let bytes = target.read_all_bytes().expect("the bytes read");
    assert_eq!(&bytes[..2], &[0x1F, 0x8B]);
    let read = target
        .read_arrow(Some(&options_declaring(&quote_root())))
        .expect("the coding is transparent");
    assert_eq!(the_one_column(read, "the document").len(), 2);
}

#[test]
fn nested_children_keep_their_values_in_every_document_that_carries_them() {
    let nested_root = record([
        StructType::from_fields([
            DataType::utf8().required_field("mic"),
            DataType::Int64.required_field("rank"),
        ])
        .map(DataType::from)
        .expect("the child datatype is valid")
        .required_field("venue"),
        DataType::serie(DataType::Int64.required_field("item")).required_field("sizes"),
        DataType::utf8().nullable_field("note"),
        DataType::Decimal128 {
            precision: 12,
            scale: 2,
        }
        .required_field("price"),
        DataType::date32().required_field("day"),
    ]);
    let nested_rows = vec![
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
    ];

    // TOML is absent deliberately: it has no null, and these rows have one.
    // XML spells the null as an empty element and the empty sequence as no
    // element at all, and the declared root reads both back.
    for name in ["nested.json", "nested.jsonl", "nested.yaml", "nested.xml"] {
        let nested = Serie::from_scalars(nested_root.clone(), nested_rows.clone())
            .expect("the rows materialize");
        let mut target = handle(name);
        target
            .write_arrow(
                SerieReader::from_serie(nested).expect("a record column is one stream"),
                IOMode::Overwrite,
                None,
            )
            .unwrap_or_else(|error| panic!("{name} writes: {error}"));

        let read = target
            .read_arrow(Some(&options_declaring(&nested_root)))
            .unwrap_or_else(|error| panic!("{name} reads: {error}"));
        assert_eq!(
            Scalar::from(the_one_column(read, name)),
            Scalar::from_sequence(nested_rows.clone()),
            "{name}"
        );
    }
}

#[test]
fn xml_rows_are_the_children_named_after_the_root_field_or_the_one_child_name_a_document_repeats() {
    let mut source = handle("quotes.xml");
    source
        .write_all_bytes(
            b"<quotes><quote><symbol>AAPL</symbol><size>100</size></quote>\
              <quote><symbol>MSFT</symbol><size>250</size></quote></quotes>",
        )
        .expect("the bytes write");
    let natural_rows = Scalar::from_sequence([
        Scalar::from_sequence([Scalar::from("100"), Scalar::from("AAPL")]),
        Scalar::from_sequence([Scalar::from("250"), Scalar::from("MSFT")]),
    ]);

    // Without a field, the one child name the root repeats is the row.
    let read = source.read_arrow(None).expect("the document proves a root");
    assert_eq!(
        Scalar::from(the_one_column(read, "the repeated child")),
        natural_rows
    );

    // A field names the rows, and finds them under that name alone.
    let quote = StructType::from_fields([
        DataType::utf8().required_field("symbol"),
        DataType::Int64.required_field("size"),
    ])
    .map(DataType::from)
    .expect("the root datatype is valid")
    .required_field("quote");
    let read = source
        .read_arrow(Some(&options_declaring(&quote)))
        .expect("the rows read under their name");
    assert_eq!(
        Scalar::from(the_one_column(read, "typed rows")),
        Scalar::from_sequence(quote_rows())
    );
    let outcome = source
        .read_arrow(Some(&options_declaring(&quote_root())))
        .map_err(|error| error.to_string())
        .and_then(|reader| {
            reader
                .collect::<Result<Vec<Serie>, _>>()
                .map_err(|error| error.to_string())
        });
    let error = outcome.expect_err("a root named otherwise is one row the field cannot type");
    assert!(error.contains("quote"), "{error}");

    // One such child is one row; a root whose one child is a leaf is itself
    // the row.
    let mut one = handle("one.xml");
    one.write_all_bytes(b"<quotes><quote><symbol>AAPL</symbol><size>100</size></quote></quotes>")
        .expect("the bytes write");
    let read = one.read_arrow(None).expect("one row reads");
    assert_eq!(
        Scalar::from(the_one_column(read, "one child")),
        Scalar::from_sequence([Scalar::from_sequence([
            Scalar::from("100"),
            Scalar::from("AAPL"),
        ])])
    );
    let mut leaf = handle("leaf.xml");
    leaf.write_all_bytes(b"<quotes><n>1</n></quotes>")
        .expect("the bytes write");
    let read = leaf.read_arrow(None).expect("the root reads");
    assert_eq!(
        Scalar::from(the_one_column(read, "a leaf child")),
        Scalar::from_sequence([Scalar::from_sequence([Scalar::from("1")])])
    );
}

#[test]
fn a_nested_sequence_column_travels_under_its_items_name_in_xml() {
    let inner = DataType::serie(DataType::Int64.required_field("value")).required_field("item");
    let root = record([
        DataType::serie(inner).required_field("matrix"),
        DataType::serie(DataType::utf8().required_field("tag")).required_field("tags"),
    ]);
    let rows = vec![
        Scalar::from_sequence([
            Scalar::from_sequence([
                Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i64)]),
                Scalar::from_sequence([Scalar::from(3_i64)]),
            ]),
            Scalar::from_sequence([Scalar::from("")]),
        ]),
        Scalar::from_sequence([
            Scalar::from_sequence([
                Scalar::from_sequence([]),
                Scalar::from_sequence([Scalar::from(4_i64)]),
            ]),
            Scalar::from_sequence([]),
        ]),
    ];
    let column = Serie::from_scalars(root.clone(), rows.clone()).expect("the rows materialize");
    let mut target = handle("matrix.xml");
    target
        .write_arrow(
            SerieReader::from_serie(column).expect("a record column is one stream"),
            IOMode::Overwrite,
            None,
        )
        .expect("the rows write");
    assert_eq!(
        String::from_utf8(target.read_all_bytes().expect("the bytes read")).expect("UTF-8"),
        "<data><row><matrix><value>1</value><value>2</value></matrix><matrix><value>3</value></matrix>\
         <tags></tags></row><row><matrix></matrix><matrix><value>4</value></matrix></row></data>",
        "an inner sequence is an element holding its items under the inner item's name, \
         an empty one an element with an empty body, an empty text item the same"
    );
    let read = target
        .read_arrow(Some(&options_declaring(&root)))
        .expect("the rows read");
    assert_eq!(
        Scalar::from(the_one_column(read, "the matrix")),
        Scalar::from_sequence(rows)
    );
}
