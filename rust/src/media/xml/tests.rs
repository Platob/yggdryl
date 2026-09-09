//! XML rows: the shared record surface, and the positional one over it.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};

use super::{Xml, XmlOptions};
use crate::holder::Buffer;
use crate::media::{IORecordOptions, RecordOptions};
use crate::text::{Formatting, Indent};
use crate::{DataType, Field, IOBase, IOMedia, Scalar, Url};

/// A handle whose media type comes from a name, so the encoding is declared.
fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    )
}

/// A document with three rows, written by hand rather than by the writer.
fn stored(name: &str, body: &str) -> Xml<Buffer> {
    let mut handle = handle(name);
    handle.write_all_bytes(body.as_bytes()).unwrap();
    Xml::new(handle)
}

fn rows_document() -> &'static str {
    "<rows>\n  <row><id>1</id><symbol>AAPL</symbol></row>\n  \
     <row><id>2</id><symbol>MSFT</symbol></row>\n  \
     <row><id>3</id><symbol>NVDA</symbol></row>\n</rows>"
}

fn typed_field() -> Field {
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])
    .unwrap()
    .required_field("row")
}

fn batch(ids: Vec<i64>, symbols: Vec<Option<&str>>) -> RecordBatch {
    RecordBatch::try_new(
        crate::arrow::arrow_schema_from_field(&typed_field()).unwrap(),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(symbols)),
        ],
    )
    .unwrap()
}

fn reader(ids: Vec<i64>, symbols: Vec<Option<&str>>) -> crate::arrow::BatchReader {
    crate::arrow::batch_reader(
        crate::arrow::arrow_schema_from_field(&typed_field()).unwrap(),
        [batch(ids, symbols)],
    )
}

/// Read one column of every batch as the values it holds.
fn column(batches: &[RecordBatch], name: &str) -> Vec<Scalar> {
    let mut values = Vec::new();
    for batch in batches {
        let index = batch.schema().index_of(name).unwrap();
        let root = crate::arrow::field_from_arrow_schema("row", &batch.schema()).unwrap();
        let child = root.fields()[index].clone();
        let column = crate::arrow::array_to_value(&child, batch.column(index)).unwrap();
        values.extend(column.as_sequence().unwrap().iter().cloned());
    }
    values
}

/// The values a text column holds, which is what an undeclared read answers.
fn text(values: &[&str]) -> Vec<Scalar> {
    values.iter().copied().map(Scalar::from).collect()
}

fn collected(media: &Xml<Buffer>) -> Vec<RecordBatch> {
    let options = media.record_options().unwrap();
    media
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap())
        .collect()
}

#[test]
fn a_document_round_trips_through_the_shared_record_surface() {
    let mut media = Xml::new(handle("trades.xml")).with_field(typed_field());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(reader(vec![1, 2], vec![Some("AAPL"), None]), &options)
        .unwrap();

    assert_eq!(
        media
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        "<rows>\n  <row><id>1</id><symbol>AAPL</symbol></row>\n  \
         <row><id>2</id><symbol/></row>\n</rows>"
    );
    assert_eq!(media.row_size().unwrap(), 2);
    assert_eq!(media.column_size().unwrap(), 2);
    assert_eq!(media.read_arrow_field(&options).unwrap(), typed_field());

    let batches = collected(&media);
    assert_eq!(
        column(&batches, "id"),
        vec![Scalar::from(1_i64), Scalar::from(2_i64)]
    );
    assert_eq!(
        column(&batches, "symbol"),
        vec![Scalar::from("AAPL"), Scalar::Null]
    );
}

#[test]
fn an_undeclared_read_answers_the_columns_the_rows_prove() {
    let media = stored(
        "trades.xml",
        "<rows><row><id>1</id><symbol>AAPL</symbol></row><row><id>2</id></row></rows>",
    );
    let field = media
        .read_arrow_field(&media.record_options().unwrap())
        .unwrap();
    assert_eq!(
        field,
        Field::new(
            "row",
            DataType::from_fields([
                DataType::Utf8.required_field("id"),
                DataType::Utf8.nullable_field("symbol"),
            ])
            .unwrap(),
            false,
        )
    );
    // The row that leaves `symbol` out reads as absent rather than failing.
    let batches = collected(&media);
    assert_eq!(
        column(&batches, "symbol"),
        vec![Scalar::from("AAPL"), Scalar::Null]
    );
}

#[test]
fn attributes_and_element_text_are_columns_with_their_own_names() {
    let media = stored(
        "trades.xml",
        "<rows><row id=\"1\">AAPL</row><row id=\"2\">MSFT</row></rows>",
    );
    let batches = collected(&media);
    assert_eq!(column(&batches, "@id"), text(&["1", "2"]));
    assert_eq!(column(&batches, "#text"), text(&["AAPL", "MSFT"]));
}

#[test]
fn a_repeated_element_is_a_list_and_a_single_one_fills_it() {
    let media = stored(
        "trades.xml",
        "<rows><row><leg>1</leg><leg>2</leg></row><row><leg>3</leg></row></rows>",
    )
    .with_field(
        DataType::from_fields([
            DataType::list(DataType::Int64.nullable_field("item")).nullable_field("leg")
        ])
        .unwrap()
        .required_field("row"),
    );
    let batches = collected(&media);
    assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 2);
    let legs = batches[0].column(0);
    assert_eq!(legs.len(), 2);
}

#[test]
fn a_foreign_document_reads_without_being_described_first() {
    let media = stored(
        "orders.xml",
        "<Orders><Order><ClOrdID>A-1</ClOrdID></Order><Order><ClOrdID>A-2</ClOrdID></Order></Orders>",
    );
    assert_eq!(media.row_size().unwrap(), 2);
    let batches = collected(&media);
    assert_eq!(column(&batches, "ClOrdID"), text(&["A-1", "A-2"]));
    let index = media.read_row_index().unwrap();
    assert_eq!(index.root(), Some("Orders"));
    assert_eq!(index.row(), Some("Order"));
}

#[test]
fn an_element_beside_the_rows_is_not_read_as_one() {
    let media = stored(
        "trades.xml",
        "<rows><meta>generated</meta><row><id>1</id></row><row><id>2</id></row></rows>",
    );
    // The first element under the document element names the row, so the
    // header element is what is read and the rows are skipped.
    assert_eq!(media.read_row_index().unwrap().row(), Some("meta"));

    let named = stored(
        "trades.xml",
        "<rows><meta>generated</meta><row><id>1</id></row><row><id>2</id></row></rows>",
    )
    .with_options(XmlOptions::new().with_row("row").unwrap());
    assert_eq!(named.row_size().unwrap(), 2);
}

#[test]
fn an_empty_document_holds_no_rows_and_no_columns() {
    let media = Xml::new(handle("trades.xml"));
    assert_eq!(media.row_size().unwrap(), 0);
    assert_eq!(media.column_size().unwrap(), 0);
    assert_eq!(collected(&media).len(), 0);

    let closed = stored("trades.xml", "<rows/>");
    assert_eq!(closed.row_size().unwrap(), 0);
    assert!(closed.read_row_index().unwrap().is_self_closed());
}

#[test]
fn the_row_index_names_the_bytes_each_row_occupies() {
    let media = stored("trades.xml", rows_document());
    let index = media.read_row_index().unwrap();
    assert_eq!(index.len(), 3);
    for (row, span) in index.spans().iter().enumerate() {
        let bytes = media
            .read_range_bytes(span.start, span.byte_size() as usize)
            .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("<row>"), "{text}");
        assert!(text.ends_with("</row>"), "{text}");
        assert!(text.contains(&format!("<id>{}</id>", row + 1)), "{text}");
    }
    assert_eq!(
        &media.read_range_bytes(index.content_end(), 7).unwrap(),
        b"</rows>"
    );
}

#[test]
fn one_row_reads_out_of_its_own_bytes() {
    let media = stored("trades.xml", rows_document());
    assert_eq!(
        media.read_row_scalar(1).unwrap(),
        Scalar::from_record([("id", Scalar::from("2")), ("symbol", Scalar::from("MSFT")),])
            .unwrap()
    );
    assert_eq!(media.read_range_scalars(1, 2).unwrap().len(), 2);
    assert_eq!(media.read_range_scalars(2, 10).unwrap().len(), 1);

    let error = media.read_row_scalar(3).unwrap_err().to_string();
    assert!(error.contains('3'), "{error}");
}

#[test]
fn a_bounded_range_reads_as_batches_without_parsing_the_rows_before_it() {
    let media = stored("trades.xml", rows_document());
    let batches: Vec<RecordBatch> = media
        .read_range_arrow_reader(1, 2)
        .unwrap()
        .map(|batch| batch.unwrap())
        .collect();
    assert_eq!(column(&batches, "id"), text(&["2", "3"]));
}

#[test]
fn a_replacement_of_the_same_length_is_written_where_the_row_was() {
    let mut media = stored("trades.xml", rows_document());
    let before = media.read_row_index().unwrap();
    let span = before.get(1).unwrap();

    media
        .write_row_scalar(
            1,
            &Scalar::from_record([("id", Scalar::from("2")), ("symbol", Scalar::from("TSLA"))])
                .unwrap(),
        )
        .unwrap();

    let after = media.read_row_index().unwrap();
    assert_eq!(after.get(1), Some(span));
    assert_eq!(after.spans(), before.spans());
    assert_eq!(after.content_end(), before.content_end());
    assert_eq!(
        media
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        rows_document().replace("MSFT", "TSLA")
    );
}

#[test]
fn a_longer_and_a_shorter_replacement_move_only_what_follows() {
    for symbol in ["A", "A-VERY-LONG-SYMBOL"] {
        let mut media = stored("trades.xml", rows_document());
        let before = media.read_row_index().unwrap();
        media
            .write_row_scalar(
                1,
                &Scalar::from_record([("id", Scalar::from("2")), ("symbol", Scalar::from(symbol))])
                    .unwrap(),
            )
            .unwrap();

        let after = media.read_row_index().unwrap();
        assert_eq!(after.len(), 3);
        assert_eq!(after.get(0), before.get(0), "{symbol}");
        assert_eq!(
            media
                .read_all_bytes()
                .map(String::from_utf8)
                .unwrap()
                .unwrap(),
            rows_document().replace("MSFT", symbol),
            "{symbol}"
        );
        // The index the write restated is the one a fresh scan reads.
        assert_eq!(
            after.spans(),
            Xml::new(media.handle().clone())
                .read_row_index()
                .unwrap()
                .spans()
        );
    }
}

#[test]
fn removing_a_row_takes_the_layout_that_introduced_it() {
    let mut media = stored("trades.xml", rows_document());
    media.remove_row(1).unwrap();
    assert_eq!(
        media
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        "<rows>\n  <row><id>1</id><symbol>AAPL</symbol></row>\n  \
         <row><id>3</id><symbol>NVDA</symbol></row>\n</rows>"
    );
    assert_eq!(media.read_row_index().unwrap().len(), 2);
}

#[test]
fn appending_rewrites_the_end_tag_and_nothing_before_it() {
    let mut media = stored("trades.xml", rows_document());
    let added = media
        .append_row_scalars([
            Scalar::from_record([("id", Scalar::from("4"))]).unwrap(),
            Scalar::from_record([("id", Scalar::from("5"))]).unwrap(),
        ])
        .unwrap();
    assert_eq!(added, 2);
    assert_eq!(
        media
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        format!(
            "{}\n  <row><id>4</id></row>\n  <row><id>5</id></row>\n</rows>",
            rows_document().trim_end_matches("\n</rows>")
        )
    );
    assert_eq!(media.row_size().unwrap(), 5);
}

#[test]
fn appending_to_an_empty_document_element_opens_it_first() {
    let mut media = stored("trades.xml", "<rows/>");
    media
        .append_row_scalars([Scalar::from_record([("id", Scalar::from("1"))]).unwrap()])
        .unwrap();
    assert_eq!(
        media
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        "<rows>\n  <row><id>1</id></row>\n</rows>"
    );
}

#[test]
fn appending_to_nothing_writes_the_document_the_options_name() {
    let mut media = Xml::new(handle("trades.xml"));
    media
        .append_row_scalars([Scalar::from_record([("id", Scalar::from("1"))]).unwrap()])
        .unwrap();
    assert_eq!(
        media
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        "<rows>\n  <row><id>1</id></row>\n</rows>"
    );
}

#[test]
fn a_positional_write_keeps_a_stored_document_s_own_names() {
    let mut media = stored(
        "orders.xml",
        "<Orders>\n  <Order><ClOrdID>A-1</ClOrdID></Order>\n</Orders>",
    );
    media
        .append_row_scalars([Scalar::from_record([("ClOrdID", Scalar::from("A-2"))]).unwrap()])
        .unwrap();
    assert_eq!(
        media
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        "<Orders>\n  <Order><ClOrdID>A-1</ClOrdID></Order>\n  \
         <Order><ClOrdID>A-2</ClOrdID></Order>\n</Orders>"
    );
}

#[test]
fn a_positional_range_write_replaces_exactly_the_rows_it_names() {
    let mut media = stored("trades.xml", rows_document()).with_field(typed_field());
    media
        .write_range_arrow_reader(1, 2, reader(vec![9], vec![Some("ZM")]))
        .unwrap();
    assert_eq!(
        media
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        "<rows>\n  <row><id>1</id><symbol>AAPL</symbol></row>\n  \
         <row><id>9</id><symbol>ZM</symbol></row>\n</rows>"
    );
}

#[test]
fn an_append_through_the_record_surface_keeps_the_stored_rows() {
    let mut media = Xml::new(handle("trades.xml")).with_field(typed_field());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(reader(vec![1], vec![Some("AAPL")]), &options)
        .unwrap();
    media
        .append_arrow_reader(reader(vec![2], vec![Some("MSFT")]), &options)
        .unwrap();
    assert_eq!(media.row_size().unwrap(), 2);
    assert_eq!(
        media
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        "<rows>\n  <row><id>1</id><symbol>AAPL</symbol></row>\n  \
         <row><id>2</id><symbol>MSFT</symbol></row>\n</rows>"
    );
}

#[test]
fn a_merge_updates_a_stored_row_and_appends_the_rest() {
    let mut media = Xml::new(handle("trades.xml")).with_field(typed_field());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(
            reader(vec![1, 2], vec![Some("AAPL"), Some("MSFT")]),
            &options,
        )
        .unwrap();

    let mut merging = media.record_options().unwrap();
    merging.set_merge_by_names(vec!["id".to_owned()]);
    media
        .merge_arrow_reader(
            reader(vec![2, 3], vec![Some("TSLA"), Some("NVDA")]),
            &merging,
        )
        .unwrap();

    assert_eq!(media.row_size().unwrap(), 3);
    let batches = collected(&media);
    assert_eq!(
        column(&batches, "symbol"),
        vec![
            Scalar::from("AAPL"),
            Scalar::from("TSLA"),
            Scalar::from("NVDA")
        ]
    );
}

#[test]
fn a_content_coding_reads_and_writes_but_has_no_row_addresses() {
    let mut media = Xml::new(handle("trades.xml.gz")).with_field(typed_field());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(reader(vec![1, 2], vec![Some("AAPL"), None]), &options)
        .unwrap();
    assert_eq!(media.row_size().unwrap(), 2);
    assert_eq!(collected(&media).len(), 1);

    let error = media.read_row_index().unwrap_err().to_string();
    assert!(error.contains("gzip"), "{error}");
}

#[test]
fn the_layout_a_write_uses_is_the_one_the_options_ask_for() {
    let mut media = Xml::new(handle("trades.xml"))
        .with_field(typed_field())
        .with_options(
            XmlOptions::new()
                .with_field(typed_field())
                .with_formatting(Formatting::default().with_indent(Indent::None)),
        );
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(reader(vec![1], vec![Some("AAPL")]), &options)
        .unwrap();
    assert_eq!(
        media
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        "<rows><row><id>1</id><symbol>AAPL</symbol></row></rows>"
    );

    let mut indented = Xml::new(handle("trades.xml")).with_options(
        XmlOptions::new()
            .with_field(typed_field())
            .with_formatting(Formatting::indented(2)),
    );
    let options = indented.record_options().unwrap();
    indented
        .overwrite_arrow_reader(reader(vec![1], vec![Some("AAPL")]), &options)
        .unwrap();
    assert_eq!(
        indented
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        "<rows>\n  <row>\n    <id>1</id>\n    <symbol>AAPL</symbol>\n  </row>\n</rows>"
    );
}

#[test]
fn the_document_and_row_names_a_write_uses_are_the_declared_ones() {
    let mut media = Xml::new(handle("trades.xml")).with_options(
        XmlOptions::new()
            .with_field(typed_field())
            .with_root("Trades")
            .unwrap()
            .with_row("Trade")
            .unwrap(),
    );
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(reader(vec![1], vec![Some("AAPL")]), &options)
        .unwrap();
    assert_eq!(
        media
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        "<Trades>\n  <Trade><id>1</id><symbol>AAPL</symbol></Trade>\n</Trades>"
    );
    assert_eq!(media.row_size().unwrap(), 1);
}

#[test]
fn a_declared_element_name_has_to_be_one_xml_can_carry() {
    let error = XmlOptions::new()
        .with_row("1st row")
        .unwrap_err()
        .to_string();
    assert!(error.contains("XML element"), "{error}");
}

#[test]
fn an_opened_document_reuses_one_index_across_positional_writes() {
    let mut media = stored("trades.xml", rows_document());
    media.open().unwrap();
    assert!(media.opened());
    let index = media.read_row_index().unwrap();
    assert!(Arc::ptr_eq(&index, &media.read_row_index().unwrap()));

    media
        .write_row_scalar(
            0,
            &Scalar::from_record([("id", Scalar::from("7"))]).unwrap(),
        )
        .unwrap();
    let after = media.read_row_index().unwrap();
    assert_eq!(after.len(), 3);
    assert_eq!(
        after.spans(),
        Xml::new(media.handle().clone())
            .read_row_index()
            .unwrap()
            .spans()
    );
    media.close().unwrap();
    assert!(!media.opened());
}

#[test]
fn options_of_another_encoding_are_refused_by_name() {
    let media = Xml::new(handle("trades.xml"));
    let error = media
        .read_arrow_field(&RecordOptions::Ipc(crate::media::ipc::IpcOptions::new()))
        .unwrap_err()
        .to_string();
    assert!(error.contains("XML record options"), "{error}");
}

/// One text column named `a`, which is what a hand-written document holds.
fn text_field() -> Field {
    DataType::from_fields([DataType::Utf8.nullable_field("a")])
        .unwrap()
        .required_field("row")
}

fn text_reader(values: Vec<&str>) -> crate::arrow::BatchReader {
    let arrow = crate::arrow::arrow_schema_from_field(&text_field()).unwrap();
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow),
        vec![Arc::new(StringArray::from(values))],
    )
    .unwrap();
    crate::arrow::batch_reader(arrow, [batch])
}

#[test]
fn a_record_append_keeps_the_document_it_was_given() {
    let mut media = stored(
        "catalog.xml",
        "<catalog>\n  <item><a>1</a></item>\n</catalog>",
    )
    .with_field(text_field());
    let options = media.record_options().unwrap();
    media
        .append_arrow_reader(text_reader(vec!["z"]), &options)
        .unwrap();

    // The stored document keeps its own names, keeps its stored row, and gains
    // exactly one element: the append is not a rewrite under other names.
    assert_eq!(
        media
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        "<catalog>\n  <item><a>1</a></item>\n  <item><a>z</a></item>\n</catalog>"
    );
    assert_eq!(media.row_size().unwrap(), 2);
}

#[test]
fn a_record_overwrite_keeps_the_document_it_was_given() {
    let mut media = stored(
        "catalog.xml",
        "<catalog>\n  <item><a>1</a></item>\n</catalog>",
    )
    .with_field(text_field());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(text_reader(vec!["z"]), &options)
        .unwrap();
    assert_eq!(
        media
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        "<catalog>\n  <item><a>z</a></item>\n</catalog>"
    );
}

#[test]
fn a_declared_row_element_survives_the_stored_shape_probe() {
    let mut media = stored(
        "trades.xml",
        "<rows>\n  <meta><generated>x</generated></meta>\n  <row><a>1</a></row>\n</rows>",
    )
    .with_options(
        XmlOptions::new()
            .with_row("row")
            .unwrap()
            .with_field(text_field()),
    );
    let options = media.record_options().unwrap();
    media
        .append_arrow_reader(text_reader(vec!["z"]), &options)
        .unwrap();

    // The element beside the rows is not the row shape, and it is still there.
    assert_eq!(
        media
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        "<rows>\n  <meta><generated>x</generated></meta>\n  <row><a>1</a></row>\n  \
         <row><a>z</a></row>\n</rows>"
    );
    assert_eq!(media.row_size().unwrap(), 2);
}

#[test]
fn repeated_appends_do_not_stack_layout_between_the_rows() {
    let mut media = Xml::new(handle("trades.xml")).with_field(text_field());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(text_reader(vec!["1"]), &options)
        .unwrap();
    for value in ["2", "3", "4"] {
        media
            .append_arrow_reader(text_reader(vec![value]), &options)
            .unwrap();
    }
    assert_eq!(
        media
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        "<rows>\n  <row><a>1</a></row>\n  <row><a>2</a></row>\n  \
         <row><a>3</a></row>\n  <row><a>4</a></row>\n</rows>"
    );
    assert_eq!(media.row_size().unwrap(), 4);
}

#[test]
fn a_declared_read_answers_without_scanning_for_a_shape() {
    // A declared field is a projection: the column it does not name is not
    // read, and the one the document does not store is filled by the cast.
    let media = stored(
        "trades.xml",
        "<rows><row><a>1</a><b>x</b></row><row><a>2</a></row></rows>",
    )
    .with_field(
        DataType::from_fields([
            DataType::Int64.nullable_field("a"),
            DataType::Utf8.nullable_field("c"),
        ])
        .unwrap()
        .required_field("row"),
    );
    let batches = collected(&media);
    assert_eq!(
        column(&batches, "a"),
        vec![Scalar::from(1_i64), Scalar::from(2_i64)]
    );
    assert_eq!(column(&batches, "c"), vec![Scalar::Null, Scalar::Null]);
    assert_eq!(batches[0].schema().fields().len(), 2);
}

#[test]
fn a_folder_of_documents_reads_as_one_table() {
    let mut root = crate::holder::local::Folder::temporary()
        .unwrap()
        .path()
        .unwrap();
    root.push(format!("yggdryl-xml-lake-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    for (venue, id) in [("XLON", "2"), ("XNAS", "1")] {
        let mut leaf = crate::holder::Holder::folder(root.join(format!("venue={venue}")))
            .unwrap()
            .child_by_path("part-0.xml")
            .unwrap();
        leaf.write_all_bytes(format!("<rows><row><id>{id}</id></row></rows>").as_bytes())
            .unwrap();
    }

    let folder = crate::holder::Holder::folder(&root).unwrap();
    let options = folder.record_options().unwrap();
    assert!(matches!(options, RecordOptions::Xml(_)));
    assert_eq!(folder.row_size().unwrap(), 2);

    let batches: Vec<RecordBatch> = folder
        .read_arrow_reader(&options)
        .unwrap()
        .map(|batch| batch.unwrap())
        .collect();
    // The directory the leaf sits in is a column of the table it belongs to.
    let mut venues = column(&batches, "venue");
    venues.sort();
    assert_eq!(venues, text(&["XLON", "XNAS"]));
    let mut ids = column(&batches, "id");
    ids.sort();
    assert_eq!(ids, text(&["1", "2"]));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_handle_becomes_xml_media_and_stays_one() {
    // The trait method wraps any handle, and the inherent one is idempotent.
    let media = handle("trades.xml").into_xml().into_xml();
    assert!(media.is_tabular());
    assert!(!media.is_atomic());

    // A holder promotes itself to the implementation its name declares.
    let mut holder = crate::holder::Holder::buffer(handle("trades.xml"));
    holder.write_all_bytes(rows_document().as_bytes()).unwrap();
    holder.open().unwrap();
    assert!(matches!(
        &holder,
        crate::holder::Holder::Media(media)
            if matches!(media.as_ref(), crate::media::Media::Xml(_))
    ));
    assert_eq!(holder.row_size().unwrap(), 3);
    assert_eq!(
        crate::IOMedia::read_arrow_reader(&holder, &holder.record_options().unwrap())
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum::<usize>(),
        3
    );
}

/// A handle that counts what actually reaches the bytes underneath it.
///
/// The positional write claims to touch one row's bytes and nothing else, and
/// a byte comparison cannot tell that from a rewrite that happens to produce
/// the same document. Counting can.
#[derive(Debug)]
struct Counted {
    handle: Buffer,
    writes: std::sync::atomic::AtomicUsize,
    written: std::sync::atomic::AtomicU64,
    read: std::sync::atomic::AtomicU64,
}

impl Counted {
    fn new(document: &str) -> Self {
        let mut handle = handle("trades.xml");
        handle.write_all_bytes(document.as_bytes()).unwrap();
        Self {
            handle,
            writes: std::sync::atomic::AtomicUsize::new(0),
            written: std::sync::atomic::AtomicU64::new(0),
            read: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Answer the three counters and start them again.
    fn take(&self) -> (usize, u64, u64) {
        use std::sync::atomic::Ordering::Relaxed;
        (
            self.writes.swap(0, Relaxed),
            self.written.swap(0, Relaxed),
            self.read.swap(0, Relaxed),
        )
    }
}

impl IOMedia for Counted {
    // Not `delegate_iomedia!`: that answers with the handle underneath, and a
    // double that is not asked its own questions counts nothing.
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    fn overwrite_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> crate::Result<()> {
        crate::iobase::overwrite_arrow_reader_default(self, batches, options)
    }
}

impl IOBase for Counted {
    crate::delegate_iobase!(handle: read_all_bytes, pstream_bytes, size, capacity, reserve,
        truncate, url, bound_location, media_type, set_media_type, flush, open, opened, close,
        parent, child_by_path, ls, kind, clear, remove, is_atomic, is_tabular, is_io);

    fn pread(&self, offset: u64, buffer: &mut [u8]) -> crate::Result<usize> {
        let read = self.handle.pread(offset, buffer)?;
        self.read
            .fetch_add(read as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(read)
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> crate::Result<Vec<u8>> {
        let bytes = self.handle.read_range_bytes(offset, length)?;
        self.read
            .fetch_add(bytes.len() as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(bytes)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
        self.writes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.written
            .fetch_add(bytes.len() as u64, std::sync::atomic::Ordering::Relaxed);
        self.handle.pwrite(offset, bytes)
    }
}

/// A document of `rows` rows, each the same width.
fn wide_document(rows: usize) -> String {
    let mut document = String::from("<rows>");
    for index in 0..rows {
        document.push_str(&format!(
            "\n  <row><id>{index:04}</id><symbol>AAPL</symbol></row>"
        ));
    }
    document.push_str("\n</rows>");
    document
}

#[test]
fn a_same_length_replacement_writes_that_row_and_reads_almost_nothing() {
    let document = wide_document(200);
    let size = document.len() as u64;
    let mut media = Xml::new(Counted::new(&document));
    media.open().unwrap();
    let span = media.read_row_index().unwrap().require(100).unwrap();
    media.handle().take();

    media
        .write_row_scalar(
            100,
            &Scalar::from_record([
                ("id", Scalar::from("0100")),
                ("symbol", Scalar::from("MSFT")),
            ])
            .unwrap(),
        )
        .unwrap();

    let (writes, written, read) = media.handle().take();
    // One write, of exactly the row that changed and the layout that leads it.
    assert_eq!(writes, 1);
    assert_eq!(written, span.byte_size() + "\n  ".len() as u64);
    // And nothing read but the whitespace that introduces it.
    assert!(read <= 64, "read {read} bytes of a {size}-byte document");
}

#[test]
fn a_longer_replacement_moves_only_what_follows_the_row() {
    let document = wide_document(200);
    let size = document.len() as u64;
    let mut media = Xml::new(Counted::new(&document));
    media.open().unwrap();
    let last = media.read_row_index().unwrap().len() - 1;
    media.handle().take();

    media
        .write_row_scalar(
            last,
            &Scalar::from_record([
                ("id", Scalar::from("0199")),
                ("symbol", Scalar::from("A-MUCH-LONGER-SYMBOL")),
            ])
            .unwrap(),
        )
        .unwrap();

    let (_, written, read) = media.handle().take();
    // The last row has no tail, so the write is the row and the read is its
    // layout - both a small fraction of the document either way.
    assert!(written < size / 8, "wrote {written} of {size} bytes");
    assert!(read < size / 8, "read {read} of {size} bytes");

    // Changing the same one row through the whole-document surface writes
    // every row instead, which is the difference an index buys.
    let values: Vec<String> = (0..200).map(|index| format!("{index:04}")).collect();
    let mut whole = Xml::new(Counted::new(&document)).with_field(text_field());
    let options = whole.record_options().unwrap();
    whole.handle().take();
    whole
        .overwrite_arrow_reader(
            text_reader(values.iter().map(String::as_str).collect()),
            &options,
        )
        .unwrap();
    let (_, rewritten, _) = whole.handle().take();
    assert!(
        rewritten > size / 2,
        "an overwrite writes the document, got {rewritten} of {size} bytes"
    );
}

#[test]
fn an_append_writes_the_rows_it_adds_and_the_end_tag() {
    let document = wide_document(200);
    let size = document.len() as u64;
    let mut media = Xml::new(Counted::new(&document));
    media.open().unwrap();
    media.read_row_index().unwrap();
    media.handle().take();

    media
        .append_row_scalars([Scalar::from_record([("id", Scalar::from("0200"))]).unwrap()])
        .unwrap();

    let (_, written, read) = media.handle().take();
    assert!(written < size / 8, "wrote {written} of {size} bytes");
    assert!(read < size / 8, "read {read} of {size} bytes");
    assert_eq!(media.row_size().unwrap(), 201);
}

#[test]
fn a_declared_column_matches_its_element_the_way_a_cast_matches_names() {
    let media = stored(
        "trades.xml",
        "<rows><row><Symbol>AAPL</Symbol><ID>7</ID></row></rows>",
    )
    .with_field(
        DataType::from_fields([
            DataType::Utf8.nullable_field("symbol"),
            DataType::Int64.nullable_field("id"),
        ])
        .unwrap()
        .required_field("row"),
    );
    let batches = collected(&media);
    assert_eq!(column(&batches, "symbol"), vec![Scalar::from("AAPL")]);
    assert_eq!(column(&batches, "id"), vec![Scalar::from(7_i64)]);

    // The positional read answers the same rows.
    let ranged: Vec<RecordBatch> = media
        .read_range_arrow_reader(0, 1)
        .unwrap()
        .map(|batch| batch.unwrap())
        .collect();
    assert_eq!(column(&ranged, "symbol"), vec![Scalar::from("AAPL")]);
}

#[test]
fn a_declared_element_name_is_what_a_write_uses() {
    // A declared row the document does not use is still what is written, and
    // the columns are the ones the caller handed over.
    let mut media = stored(
        "catalog.xml",
        "<catalog>\n  <item><a>1</a></item>\n</catalog>",
    )
    .with_options(
        XmlOptions::new()
            .with_row("line")
            .unwrap()
            .with_field(text_field()),
    );
    let options = media.record_options().unwrap();
    media
        .append_arrow_reader(text_reader(vec!["z"]), &options)
        .unwrap();
    assert_eq!(
        media
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        "<catalog>\n  <item><a>1</a></item>\n  <line><a>z</a></line>\n</catalog>"
    );

    // A declared root renames the document an overwrite replaces.
    let mut renamed = stored(
        "catalog.xml",
        "<catalog>\n  <item><a>1</a></item>\n</catalog>",
    )
    .with_options(
        XmlOptions::new()
            .with_root("shipment")
            .unwrap()
            .with_field(text_field()),
    );
    let options = renamed.record_options().unwrap();
    renamed
        .overwrite_arrow_reader(text_reader(vec!["z"]), &options)
        .unwrap();
    assert_eq!(
        renamed
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        "<shipment>\n  <item><a>z</a></item>\n</shipment>"
    );
}

#[test]
fn an_overwrite_replaces_bytes_it_could_not_have_read() {
    // The encoding's own overwrite keeps a stored document's names, so it asks
    // what they are - and bytes that answer nothing are still replaced, because
    // a write that replaces them has no reason to care what they used to say.
    let mut handle = handle("trades.xml");
    handle.write_all_bytes(b"not xml at all <<<").unwrap();
    super::overwrite_arrow_reader(&mut handle, text_reader(vec!["z"]), &XmlOptions::new()).unwrap();
    assert_eq!(
        handle
            .read_all_bytes()
            .map(String::from_utf8)
            .unwrap()
            .unwrap(),
        "<rows>\n  <row><a>z</a></row>\n</rows>"
    );

    // The record surface completes onto the stored shape first, so it reports
    // a document it cannot read rather than replacing it unasked.
    let media = stored("trades.xml", "not xml at all <<<").with_field(text_field());
    let options = media.record_options().unwrap();
    assert!(media.read_arrow_field(&options).is_ok());
    assert!(
        crate::iobase::stored_field(media.handle(), &options).is_err(),
        "a document that cannot be read has no stored shape to complete onto"
    );
}
