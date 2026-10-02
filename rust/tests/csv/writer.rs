//! `rust/src/csv/writer.rs`: how batches render as CSV - the header, the
//! quoting rule, the null spelling, every leaf family's text - and that
//! each reads back as what it was.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::IOMedia as _;
use yggdryl::csv::{CsvOptions, overwrite_arrow_reader, read_batch_reader};
use yggdryl::holder::Buffer;
use yggdryl::text::LineSep;
use yggdryl::{
    ArrowCastOptions, Charset, DataType, Error, Field, IOBase, MediaType, Scalar, Serie,
    SerieReader, TimeUnit, Timezone,
};

fn buffer(name: &str) -> Buffer {
    Buffer::new().with_media_type(MediaType::from_file_name(name))
}

/// The trades table: a required `id` and a nullable `symbol`.
fn trades_field() -> Field {
    DataType::from_str("struct<id: int64 not null, symbol: utf8>")
        .expect("a valid root")
        .required_field("row")
}

fn batch(rows: &[(i64, Option<&str>)]) -> RecordBatch {
    RecordBatch::try_new(
        trades_field().into_arrow_schema().expect("an Arrow schema"),
        vec![
            Arc::new(Int64Array::from_iter_values(rows.iter().map(|(id, _)| *id))),
            Arc::new(StringArray::from_iter(
                rows.iter().map(|(_, symbol)| *symbol),
            )),
        ],
    )
    .expect("a batch")
}

fn reader(batches: Vec<RecordBatch>) -> yggdryl::arrow::BatchReader {
    yggdryl::arrow::batch_reader(
        trades_field().into_arrow_schema().expect("an Arrow schema"),
        batches,
    )
}

fn text(handle: &Buffer) -> String {
    String::from_utf8(handle.read_all_bytes().expect("the bytes")).expect("UTF-8")
}

/// The path and the reason of an `InvalidRecord` refusal.
fn refusal(error: Error) -> (String, String) {
    match error {
        Error::InvalidRecord { path, reason } => (path.to_string(), reason.to_string()),
        other => panic!("expected an invalid record, got {other:?}"),
    }
}

/// Every row of `handle` read under `field` and `options`, each row its cells.
fn rows_under(handle: &Buffer, field: &Field, options: &CsvOptions) -> Vec<Vec<Scalar>> {
    let reader = read_batch_reader(handle, Some(field), options).expect("a reader");
    let records = SerieReader::from_arrow_reader(None, reader, ArrowCastOptions::default())
        .expect("record columns");
    let mut out = Vec::new();
    for record in records {
        let record = record.expect("a batch");
        for index in 0..record.len() {
            let row = record.scalar(index).expect("a row");
            out.push(row.as_serie().expect("a run").rows().to_vec());
        }
    }
    out
}

/// Write `rows` under `field` through `options`, answering the document.
fn written(name: &str, field: &Field, rows: Vec<Scalar>, options: &CsvOptions) -> Buffer {
    let batch = Serie::from_scalars(field.clone(), rows)
        .expect("rows under the field")
        .into_arrow_batch()
        .expect("a batch");
    let mut held = buffer(name);
    overwrite_arrow_reader(
        &mut held,
        yggdryl::arrow::batch_reader(batch.schema(), [batch]),
        options,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    held
}

// Refusals.

#[test]
fn a_cell_that_needs_quoting_is_refused_by_name_when_the_dialect_quotes_nothing() {
    let options = CsvOptions::new().with_quote(None).expect("no quote");
    let mut held = buffer("trades.csv");
    let (path, reason) = refusal(
        overwrite_arrow_reader(
            &mut held,
            reader(vec![batch(&[(1, Some("a,b"))])]),
            &options,
        )
        .unwrap_err(),
    );
    assert_eq!(path, "$[0].symbol");
    assert!(
        reason.starts_with("expected a cell that needs no quoting"),
        "{reason}"
    );
    assert!(
        reason.ends_with("while quote is unset, got \"a,b\""),
        "{reason}"
    );
    assert_eq!(held.size(), 0, "nothing reached the handle");

    // A header name too.
    let named = DataType::from_str("struct<\"id,x\": int64>")
        .expect("a quoted name")
        .required_field("row");
    let schema = named.clone().into_arrow_schema().expect("a schema");
    let (path, _) = refusal(
        overwrite_arrow_reader(
            &mut held,
            yggdryl::arrow::batch_reader(Arc::clone(&schema), [RecordBatch::new_empty(schema)]),
            &options,
        )
        .unwrap_err(),
    );
    assert_eq!(path, "$.header.id,x");
}

#[test]
fn a_null_with_no_null_spelling_is_refused_by_name() {
    let options = CsvOptions::new()
        .with_null_values::<[&str; 0], &str>([])
        .expect("no spelling");
    let mut held = buffer("trades.csv");
    let (path, reason) = refusal(
        overwrite_arrow_reader(&mut held, reader(vec![batch(&[(1, None)])]), &options).unwrap_err(),
    );
    assert_eq!(path, "$[0].symbol");
    assert_eq!(
        reason,
        "expected a null spelling in null_values to write a null cell, got none"
    );
}

// The rendering.

#[test]
fn the_header_then_one_record_per_row_and_only_what_must_be_is_quoted() {
    let mut held = buffer("trades.csv");
    overwrite_arrow_reader(
        &mut held,
        reader(vec![
            batch(&[(1, Some("AAPL")), (2, None)]),
            batch(&[
                (3, Some("a,b")),
                (4, Some("say \"hi\"")),
                (5, Some("two\nlines")),
                (6, Some(" padded ")),
                (7, Some("")),
                (8, Some("cr\rhere")),
            ]),
        ]),
        &CsvOptions::new(),
    )
    .expect("written");
    assert_eq!(
        text(&held),
        "id,symbol\n1,AAPL\n2,\n3,\"a,b\"\n4,\"say \"\"hi\"\"\"\n5,\"two\nlines\"\n6,\" padded \"\n7,\"\"\n8,\"cr\rhere\"\n"
    );
    assert_eq!(
        rows_under(&held, &trades_field(), &CsvOptions::new()),
        [
            vec![Scalar::from(1_i64), Scalar::from("AAPL")],
            vec![Scalar::from(2_i64), Scalar::Null],
            vec![Scalar::from(3_i64), Scalar::from("a,b")],
            vec![Scalar::from(4_i64), Scalar::from("say \"hi\"")],
            vec![Scalar::from(5_i64), Scalar::from("two\nlines")],
            vec![Scalar::from(6_i64), Scalar::from(" padded ")],
            vec![Scalar::from(7_i64), Scalar::from("")],
            vec![Scalar::from(8_i64), Scalar::from("cr\rhere")],
        ]
    );
}

#[test]
fn a_null_is_the_first_null_spelling_and_a_cell_spelling_one_is_quoted() {
    let options = CsvOptions::new()
        .with_null_values(["NA", ""])
        .expect("spellings");
    let mut held = buffer("trades.csv");
    overwrite_arrow_reader(
        &mut held,
        reader(vec![batch(&[(1, None), (2, Some("NA")), (3, Some(""))])]),
        &options,
    )
    .expect("written");
    assert_eq!(text(&held), "id,symbol\n1,NA\n2,\"NA\"\n3,\"\"\n");
    assert_eq!(
        rows_under(&held, &trades_field(), &options),
        [
            vec![Scalar::from(1_i64), Scalar::Null],
            vec![Scalar::from(2_i64), Scalar::from("NA")],
            vec![Scalar::from(3_i64), Scalar::from("")],
        ]
    );
}

#[test]
fn an_escape_byte_spells_a_quote_and_itself_and_a_comment_byte_opening_a_record_is_quoted() {
    let options = CsvOptions::new()
        .with_escape(Some(b'\\'))
        .expect("an escape")
        .with_comment(Some(b'#'))
        .expect("a comment byte")
        .with_separator(b';')
        .expect("a separator");
    let field = DataType::from_str("struct<symbol: utf8, id: int64>")
        .expect("a root")
        .required_field("row");
    let held = written(
        "trades.csv",
        &field,
        vec![
            Scalar::from_sequence([Scalar::from("say \"hi\" \\ bye"), Scalar::from(1_i64)]),
            Scalar::from_sequence([Scalar::from("#not a comment"), Scalar::from(2_i64)]),
            Scalar::from_sequence([Scalar::from("a;b"), Scalar::from(3_i64)]),
        ],
        &options,
    );
    assert_eq!(
        text(&held),
        "symbol;id\n\"say \\\"hi\\\" \\\\ bye\";1\n\"#not a comment\";2\n\"a;b\";3\n"
    );
    assert_eq!(
        rows_under(&held, &field, &options),
        [
            vec![Scalar::from("say \"hi\" \\ bye"), Scalar::from(1_i64)],
            vec![Scalar::from("#not a comment"), Scalar::from(2_i64)],
            vec![Scalar::from("a;b"), Scalar::from(3_i64)],
        ]
    );
}

#[test]
fn a_crlf_terminator_and_no_header_are_the_options() {
    let options = CsvOptions::new()
        .with_linesep(LineSep::CRLF)
        .with_header(false);
    let mut held = buffer("trades.tsv");
    overwrite_arrow_reader(
        &mut held,
        reader(vec![batch(&[(1, Some("AAPL")), (2, None)])]),
        &CsvOptions::tsv()
            .with_linesep(LineSep::CRLF)
            .with_header(false),
    )
    .expect("written");
    assert_eq!(text(&held), "1\tAAPL\r\n2\t\r\n");
    assert_eq!(
        rows_under(
            &held,
            &trades_field(),
            &options.with_separator(b'\t').expect("tab")
        ),
        [
            vec![Scalar::from(1_i64), Scalar::from("AAPL")],
            vec![Scalar::from(2_i64), Scalar::Null],
        ]
    );
}

#[test]
fn a_windows_1252_handle_is_written_in_its_charset_and_read_back() {
    let mut held = Buffer::new()
        .with_media_type(MediaType::from_file_name("trades.csv").with_charset(Charset::Cp1252));
    overwrite_arrow_reader(
        &mut held,
        reader(vec![batch(&[(1, Some("Soci\u{e9}t\u{e9}"))])]),
        &CsvOptions::new(),
    )
    .expect("written");
    assert_eq!(
        held.read_all_bytes().expect("the bytes"),
        b"id,symbol\n1,Soci\xE9t\xE9\n"
    );
    assert_eq!(
        rows_under(&held, &trades_field(), &CsvOptions::new()),
        [vec![Scalar::from(1_i64), Scalar::from("Soci\u{e9}t\u{e9}")]]
    );
}

#[test]
fn every_leaf_family_renders_as_the_text_it_reads_back_from() {
    let field = DataType::from_str(
        "struct<\
         i8: int8, u64: uint64, f32: float32, f64: float64, \
         d: decimal(10, 2), big: decimal256(40, 3), \
         b: boolean, \
         date: date32, time: time64(us), at: datetime64(ms, UTC), naive: datetime64(ns), \
         dur: duration64(s), \
         s: utf8, large: large_utf8, sized: sized_utf8(8), \
         ccy: ccy, id: uuid, \
         bytes: binary, fixed: fixed_binary(2), \
         tags: serie<utf8>, meta: struct<k: int64, v: utf8>, \
         m: map<utf8, int64>\
         >",
    )
    .expect("a root of every family")
    .required_field("row");
    let row = Scalar::from_sequence([
        Scalar::from(-7_i8),
        Scalar::from(u64::MAX),
        Scalar::from(0.5_f32),
        Scalar::from(-1234.5678_f64),
        Scalar::decimal128(1250, 2),
        Scalar::decimal256(yggdryl::i256::from(-1_234_567_i128), 3),
        Scalar::from(true),
        Scalar::date32(19_724),
        Scalar::time64(3_600_000_001, TimeUnit::Microsecond, Timezone::NAIVE).expect("a time"),
        Scalar::datetime64(1_704_164_645_123, TimeUnit::Millisecond, Timezone::UTC)
            .expect("an instant"),
        Scalar::datetime64(
            1_704_164_645_000_000_007,
            TimeUnit::Nanosecond,
            Timezone::NAIVE,
        )
        .expect("a naive instant"),
        Scalar::duration64(90, TimeUnit::Second).expect("a duration"),
        Scalar::from("plain, text"),
        Scalar::from("large"),
        Scalar::from("sized"),
        DataType::from_str("ccy")
            .expect("a code")
            .scalar(Scalar::from("EUR"))
            .expect("a currency"),
        DataType::from_str("uuid")
            .expect("a uuid")
            .scalar(Scalar::from("6ba7b810-9dad-11d1-80b4-00c04fd430c8"))
            .expect("an identifier"),
        Scalar::from(b"\x00\xFF\x10".to_vec()),
        Scalar::from(b"\x01\x02".to_vec()),
        Scalar::from_sequence([Scalar::from("a"), Scalar::from("b,c")]),
        Scalar::from_struct([("k", Scalar::from(1_i64)), ("v", Scalar::from("x"))])
            .expect("a record"),
        Scalar::from_mapping([(Scalar::from("one"), Scalar::from(1_i64))]).expect("a map"),
    ]);
    let nulls = Scalar::from_sequence(vec![Scalar::Null; 22]);
    let held = written(
        "every.csv",
        &field,
        vec![row.clone(), nulls.clone()],
        &CsvOptions::new(),
    );
    let document = text(&held);
    let lines: Vec<&str> = document.lines().collect();
    assert_eq!(lines.len(), 3, "{document}");
    assert_eq!(
        lines[1],
        "-7,18446744073709551615,0.5,-1234.5678,12.5,-1234.567,true,\
         2024-01-02,01:00:00.000001,2024-01-02T03:04:05.123Z,2024-01-02T03:04:05.000000007,\
         PT90S,\"plain, text\",large,sized,EUR,6ba7b810-9dad-11d1-80b4-00c04fd430c8,\
         AP8Q,AQI=,\"[\"\"a\"\",\"\"b,c\"\"]\",\"{\"\"k\"\":1,\"\"v\"\":\"\"x\"\"}\",\"{\"\"one\"\":1}\"",
    );
    assert_eq!(lines[2], ",,,,,,,,,,,,,,,,,,,,,");

    // Read back under the field, every cell is what was written.
    let read = rows_under(&held, &field, &CsvOptions::new());
    let expected = field.scalar(row).expect("the row canonicalizes");
    assert_eq!(read[0], expected.as_serie().expect("a run").rows().to_vec());
    assert_eq!(read[1], vec![Scalar::Null; 22]);
}

/// A nested cell is the JSON a cast of its column into text spells: a struct
/// keyed in declaration order, a map's keys as the text they read back from.
#[test]
fn a_nested_cell_is_the_json_a_cast_into_text_spells() {
    let field = DataType::from_str("struct<meta: struct<v: utf8, k: int64>, m: map<int64, utf8>>")
        .expect("a root")
        .required_field("row");
    let row = Scalar::from_sequence([
        Scalar::from_struct([("k", Scalar::from(1_i64)), ("v", Scalar::from("x"))])
            .expect("a record"),
        Scalar::from_mapping([(Scalar::from(2_i64), Scalar::from("y"))]).expect("a map"),
    ]);
    let held = written("nested.csv", &field, vec![row.clone()], &CsvOptions::new());
    assert_eq!(
        text(&held),
        "meta,m\n\"{\"\"v\"\":\"\"x\"\",\"\"k\"\":1}\",\"{\"\"2\"\":\"\"y\"\"}\"\n"
    );
    let column = Serie::from_scalars(field.clone(), [row.clone()])
        .expect("a record column")
        .child("meta")
        .expect("the meta column")
        .cast(
            &Field::new("meta", DataType::utf8(), true),
            ArrowCastOptions::default(),
        )
        .expect("a text column");
    assert_eq!(
        column.scalar(0).expect("a cell"),
        Scalar::from(r#"{"v":"x","k":1}"#)
    );
    let expected = field.scalar(row).expect("the row canonicalizes");
    assert_eq!(
        rows_under(&held, &field, &CsvOptions::new())[0],
        expected.as_serie().expect("a run").rows().to_vec()
    );
}

#[test]
fn a_dictionary_encoded_and_a_view_string_column_render_their_text() {
    // What Arrow JS infers for a plain record's string, and the view layout.
    let field = DataType::from_str("struct<symbol: dictionary<int32, utf8>, view: utf8_view>")
        .expect("a root")
        .required_field("row");
    let held = written(
        "trades.csv",
        &field,
        vec![
            Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from("viewed")]),
            Scalar::from_sequence([Scalar::from("AAPL"), Scalar::Null]),
        ],
        &CsvOptions::new(),
    );
    assert_eq!(text(&held), "symbol,view\nAAPL,viewed\nAAPL,\n");
}

#[test]
fn a_write_that_fails_mid_stream_leaves_the_handle_untouched() {
    let mut held = buffer("trades.csv");
    overwrite_arrow_reader(
        &mut held,
        reader(vec![batch(&[(1, Some("AAPL"))])]),
        &CsvOptions::new(),
    )
    .expect("written");
    let before = held.read_all_bytes().expect("the bytes");
    let failing: yggdryl::arrow::BatchReader = Box::new(arrow_array::RecordBatchIterator::new(
        vec![
            Ok(batch(&[(9, Some("IBM"))])),
            Err(arrow_schema::ArrowError::ComputeError(
                "the feed dropped".to_owned(),
            )),
        ],
        trades_field().into_arrow_schema().expect("a schema"),
    ));
    let message = overwrite_arrow_reader(&mut held, failing, &CsvOptions::new())
        .unwrap_err()
        .to_string();
    assert!(message.contains("the feed dropped"), "{message}");
    assert_eq!(held.read_all_bytes().expect("the bytes"), before);
}

/// Write `rows` under `field` through `options` into a fresh handle,
/// answering the refusal where there is one and the handle it left.
fn try_written(
    field: &Field,
    rows: Vec<Scalar>,
    options: &CsvOptions,
) -> (Buffer, Result<(), Error>) {
    let batch = Serie::from_scalars(field.clone(), rows)
        .expect("rows under the field")
        .into_arrow_batch()
        .expect("a batch");
    let mut held = buffer("one.csv");
    let result = overwrite_arrow_reader(
        &mut held,
        yggdryl::arrow::batch_reader(batch.schema(), [batch]),
        options,
    );
    (held, result)
}

fn one(value: Scalar) -> Scalar {
    Scalar::from_sequence([value])
}

#[test]
fn a_one_column_record_is_never_a_blank_line() {
    // A blank line is the separator between records, never one, so a
    // one-column record whose cell spells nothing is written as `""`: the
    // empty text, which a column that is not text reads as the null it was.
    let int = DataType::from_str("struct<a: int64>")
        .expect("a root")
        .required_field("row");
    let default = CsvOptions::new();
    let values = vec![
        one(Scalar::from(1_i64)),
        one(Scalar::Null),
        one(Scalar::from(3_i64)),
    ];
    let held = written("one.csv", &int, values.clone(), &default);
    assert_eq!(text(&held), "a\n1\n\"\"\n3\n");
    let rows: Vec<Scalar> = rows_under(&held, &int, &default)
        .into_iter()
        .map(Scalar::from_sequence)
        .collect();
    assert_eq!(rows, values);
    assert_eq!(
        yggdryl::csv::read_field(&held, &default).expect("the field"),
        DataType::from_str("struct<a: int64>")
            .expect("a root")
            .required_field("row"),
        "the empty text proves no datatype, so the column still infers"
    );
    assert_eq!(
        yggdryl::csv::Csv::new(held).row_size().expect("the rows"),
        3
    );

    // A text column: the empty text is `""`, and a null has no spelling a
    // reader tells from it under the default, so it is refused by name.
    let utf8 = DataType::from_str("struct<a: utf8>")
        .expect("a root")
        .required_field("row");
    let held = written(
        "one.csv",
        &utf8,
        vec![one(Scalar::from("x")), one(Scalar::from(""))],
        &default,
    );
    assert_eq!(text(&held), "a\nx\n\"\"\n");
    assert_eq!(
        rows_under(&held, &utf8, &default),
        [vec![Scalar::from("x")], vec![Scalar::from("")]]
    );
    let (held, result) = try_written(
        &utf8,
        vec![one(Scalar::from("x")), one(Scalar::Null)],
        &default,
    );
    let (path, reason) = refusal(result.unwrap_err());
    assert_eq!(path, "$[1].a");
    assert_eq!(
        reason,
        "expected a null spelling in null_values that is not empty, to write a null as the only \
         cell of a record, got [\"\"]: an empty record is a blank line, which is no record, and \
         `\"\"` does not read as a null under utf8"
    );
    assert_eq!(held.size(), 0, "nothing reached the handle");

    // Under a spelling that is not empty, the null and the empty text both
    // round trip in a text column and in any other.
    let na = CsvOptions::new()
        .with_null_values(["NA"])
        .expect("a spelling");
    let values = vec![
        one(Scalar::from("x")),
        one(Scalar::Null),
        one(Scalar::from("")),
    ];
    let held = written("one.csv", &utf8, values.clone(), &na);
    assert_eq!(text(&held), "a\nx\nNA\n\"\"\n");
    let rows: Vec<Scalar> = rows_under(&held, &utf8, &na)
        .into_iter()
        .map(Scalar::from_sequence)
        .collect();
    assert_eq!(rows, values);
    let held = written(
        "one.csv",
        &int,
        vec![one(Scalar::from(1_i64)), one(Scalar::Null)],
        &na,
    );
    assert_eq!(text(&held), "a\n1\nNA\n");
    assert_eq!(
        rows_under(&held, &int, &na),
        [vec![Scalar::from(1_i64)], vec![Scalar::Null]]
    );
}

#[test]
fn a_null_spelling_a_record_cannot_hold_as_it_stands_is_refused_by_name() {
    // A null is written verbatim, since a quoted cell is never absent: a
    // spelling holding the separator, the quote or a line break, or one
    // opening the record with the comment byte, would read back as
    // something else, so it is refused where a null is written.
    let field = DataType::from_str("struct<a: utf8, b: utf8>")
        .expect("a root")
        .required_field("row");
    let rows = vec![
        Scalar::from_sequence([Scalar::from("x"), Scalar::Null]),
        Scalar::from_sequence([Scalar::Null, Scalar::from("y")]),
    ];
    for (options, at, spelling, holds) in [
        (
            CsvOptions::new().with_null_values(["N,A"]).unwrap(),
            "$[0].b",
            "\"N,A\"",
            "which holds the separator ','",
        ),
        (
            CsvOptions::new().with_null_values(["\"NA\""]).unwrap(),
            "$[0].b",
            "\"\\\"NA\\\"\"",
            "which holds the quote '\"'",
        ),
        (
            CsvOptions::new().with_null_values(["N\nA"]).unwrap(),
            "$[0].b",
            "\"N\\nA\"",
            "which holds a line break",
        ),
        (
            CsvOptions::new()
                .with_comment(Some(b'#'))
                .unwrap()
                .with_null_values(["#N/A"])
                .unwrap(),
            "$[1].a",
            "\"#N/A\"",
            "which opens the record with the comment byte '#'",
        ),
    ] {
        let (held, result) = try_written(&field, rows.clone(), &options);
        let (path, reason) = refusal(result.unwrap_err());
        assert_eq!(path, "$.null_values", "{spelling}");
        assert_eq!(
            reason,
            format!(
                "expected a null spelling a record holds as it stands - no separator, quote or \
                 line break, and no comment byte opening the record - to write the null at {at}, \
                 got {spelling}, {holds}"
            )
        );
        assert_eq!(held.size(), 0, "nothing reached the handle");
    }
    // The comment byte past the first cell opens nothing, so it is written.
    let options = CsvOptions::new()
        .with_comment(Some(b'#'))
        .unwrap()
        .with_null_values(["#N/A"])
        .unwrap();
    let held = written("two.csv", &field, rows[..1].to_vec(), &options);
    assert_eq!(text(&held), "a,b\nx,#N/A\n");
    assert_eq!(
        rows_under(&held, &field, &options),
        [vec![Scalar::from("x"), Scalar::Null]]
    );
}

#[test]
fn a_cell_the_writer_cannot_spell_is_refused_naming_the_column_and_the_row() {
    // A top-level float spells NaN as text, but a nested cell is JSON, which
    // has no spelling for one.
    let field = DataType::from_str("struct<k: utf8, xs: serie<float64>>")
        .expect("a root")
        .required_field("row");
    let (held, result) = try_written(
        &field,
        vec![
            Scalar::from_sequence([Scalar::from("a"), Scalar::Null]),
            Scalar::from_sequence([
                Scalar::from("b"),
                Scalar::from_sequence([Scalar::from(f64::NAN)]),
            ]),
        ],
        &CsvOptions::new(),
    );
    let (path, reason) = refusal(result.unwrap_err());
    assert_eq!(path, "$[1].xs");
    assert!(reason.contains("non-finite"), "{reason}");
    assert_eq!(held.size(), 0, "nothing reached the handle");
}

#[test]
fn not_a_number_and_the_infinities_read_back_as_written() {
    let field = DataType::from_str("struct<x: float64 not null, y: float32, z: float64>")
        .expect("a root")
        .required_field("row");
    let rows = vec![
        Scalar::from_sequence([
            Scalar::from(f64::NAN),
            Scalar::from(f32::NAN),
            Scalar::from(1.5_f64),
        ]),
        Scalar::from_sequence([
            Scalar::from(f64::INFINITY),
            Scalar::from(f32::NEG_INFINITY),
            Scalar::from(f64::NEG_INFINITY),
        ]),
    ];
    let held = written("floats.csv", &field, rows.clone(), &CsvOptions::new());
    assert_eq!(text(&held), "x,y,z\nNaN,NaN,1.5\ninf,-inf,-inf\n");
    let read: Vec<Scalar> = rows_under(&held, &field, &CsvOptions::new())
        .into_iter()
        .map(Scalar::from_sequence)
        .collect();
    assert_eq!(read, rows);
}
