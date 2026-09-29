//! `rust/src/csv/reader.rs`: the RFC 4180 tokenizer and the typed row
//! builder, reached through the public read doors over in-memory buffers.

use yggdryl::csv::{CsvOptions, read_batch_reader, read_field};
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::{
    ArrowCastOptions, Charset, DataType, Error, Field, IOBase, IOMedia, MediaType, Scalar,
    SerieReader, StructType, TimeUnit, Timezone,
};

/// A `.csv` buffer holding `bytes`.
fn document(bytes: &[u8]) -> Buffer {
    named("trades.csv", bytes)
}

/// A buffer named `name` holding `bytes`.
fn named(name: &str, bytes: &[u8]) -> Buffer {
    Buffer::from_bytes(bytes.to_vec()).with_media_type(MediaType::from_file_name(name))
}

/// Every row the document reads as under `options`, each row its cells.
fn rows(handle: &Buffer, options: &CsvOptions) -> Vec<Vec<Scalar>> {
    try_rows(handle, options).unwrap_or_else(|error| panic!("{error}"))
}

/// The rows, or the first failure a batch raised.
fn try_rows(handle: &Buffer, options: &CsvOptions) -> Result<Vec<Vec<Scalar>>, String> {
    let reader = read_batch_reader(handle, None, options).map_err(|error| error.to_string())?;
    let records = SerieReader::from_arrow_reader(None, reader, ArrowCastOptions::default())
        .map_err(|error| error.to_string())?;
    let mut out = Vec::new();
    for record in records {
        let record = record.map_err(|error| error.to_string())?;
        for index in 0..record.len() {
            let row = record.scalar(index).expect("a row");
            out.push(row.as_serie().expect("a run").rows().to_vec());
        }
    }
    Ok(out)
}

/// Every cell as its canonical text, `None` where it is absent.
fn text(rows: &[Vec<Scalar>]) -> Vec<Vec<Option<String>>> {
    rows.iter()
        .map(|row| {
            row.iter()
                .map(|cell| {
                    if cell.is_null() {
                        None
                    } else {
                        Some(cell.as_str().map_or_else(
                            || yggdryl::into_json_scalar(cell).expect("a JSON spelling"),
                            str::to_owned,
                        ))
                    }
                })
                .collect()
        })
        .collect()
}

fn some(values: &[&str]) -> Vec<Option<String>> {
    values
        .iter()
        .map(|value| Some((*value).to_owned()))
        .collect()
}

/// The datatypes the document's columns infer to, as the field spells them.
/// Assert the document's columns infer to the datatype `spelling` names,
/// under the root name the options state.
fn assert_inferred(handle: &Buffer, options: &CsvOptions, spelling: &str) {
    let field = read_field(handle, options).unwrap_or_else(|error| panic!("{error}"));
    let expected = DataType::from_str(spelling).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(field.dtype(), &expected, "{spelling}");
    assert_eq!(field.name(), options.name());
    assert!(!field.is_nullable(), "a root is never nullable");
}

/// The path and the reason of an `InvalidRecord` refusal.
fn refusal(error: Error) -> (String, String) {
    match error {
        Error::InvalidRecord { path, reason } => (path.to_string(), reason.to_string()),
        other => panic!("expected an invalid record, got {other:?}"),
    }
}

fn declared(spelling: &str) -> Field {
    DataType::from_str(spelling)
        .unwrap_or_else(|error| panic!("{error}"))
        .required_field("row")
}

// Refusals.

#[test]
fn a_ragged_record_is_refused_naming_the_row_and_the_location() {
    let held = document(b"a,b\n1,2\n3\n4,5,6\n");
    let message = try_rows(&held, &CsvOptions::new()).unwrap_err();
    let url = held.url().expect("a buffer identity").to_string();
    assert!(
        message.contains(&format!("expected 2 cells, got 1 in row 3 of {url}")),
        "{message}"
    );
    assert!(message.contains("$[1]"), "{message}");

    // The rows before it are answered first, the refusal after.
    let reader = read_batch_reader(&held, None, &CsvOptions::new()).expect("a reader");
    let mut batches = reader;
    assert_eq!(
        batches
            .next()
            .expect("a batch")
            .expect("the prefix")
            .num_rows(),
        1
    );
    assert!(batches.next().expect("the refusal").is_err());
    assert!(batches.next().is_none(), "fused after the refusal");
}

#[test]
fn a_header_naming_a_column_twice_is_refused() {
    let held = document(b"a,b,a\n1,2,3\n");
    let (path, reason) = refusal(read_field(&held, &CsvOptions::new()).unwrap_err());
    assert_eq!(path, "$.header");
    assert!(
        reason.starts_with("expected each column name once, got \"a\" twice in the header of "),
        "{reason}"
    );
    let message = read_batch_reader(&held, None, &CsvOptions::new())
        .err()
        .expect("the same refusal before a batch")
        .to_string();
    assert!(message.contains("got \"a\" twice"), "{message}");
}

#[test]
fn an_empty_document_declares_no_schema_and_reads_as_no_rows() {
    let held = document(b"");
    let (path, reason) = refusal(read_field(&held, &CsvOptions::new()).unwrap_err());
    assert_eq!(path, "$.csv");
    assert_eq!(reason, "an empty document declares no schema");
    let reader = read_batch_reader(&held, None, &CsvOptions::new()).expect("a reader");
    assert!(reader.schema().fields().is_empty());
    assert_eq!(reader.count(), 0);

    // Declared, the empty document is the declared schema and no rows.
    let field = declared("struct<a: int64>");
    let reader = read_batch_reader(&held, Some(&field), &CsvOptions::new()).expect("a reader");
    assert_eq!(
        reader.schema(),
        field.clone().into_arrow_schema().expect("a schema")
    );
    assert_eq!(reader.count(), 0);
    assert_eq!(
        read_field(&held, &CsvOptions::new().with_field(field.clone())).expect("declared"),
        field
    );
}

#[test]
fn a_required_declared_column_the_header_does_not_state_is_refused() {
    let held = document(b"a,b\n1,2\n");
    let field = declared("struct<a: int64, c: utf8 not null>");
    let (path, reason) = refusal(
        read_batch_reader(&held, Some(&field), &CsvOptions::new())
            .err()
            .expect("a refusal")
            .into(),
    );
    assert_eq!(path, "$.header");
    assert!(
        reason.starts_with(
            "expected a column named \"c\" for the declared not-null field, got [\"a\", \"b\"] in the header of "
        ),
        "{reason}"
    );
}

#[test]
fn a_cell_a_required_column_cannot_read_is_refused_naming_the_cell_and_the_row() {
    let held = document(b"n,s\n1,x\nseven,y\n");
    let field = declared("struct<n: int64 not null, s: utf8>");
    let message = try_rows(&held, &CsvOptions::new().with_field(field)).unwrap_err();
    assert!(message.contains("$[1].n"), "{message}");
    assert!(message.contains("\"seven\""), "{message}");
    assert!(message.contains("in row 3 of"), "{message}");
}

#[test]
fn safe_nulls_a_cell_a_nullable_column_cannot_read_and_strict_refuses_it() {
    let held = document(b"n\n1\nseven\n");
    let field = declared("struct<n: int64>");
    assert_eq!(
        rows(&held, &CsvOptions::new().with_field(field.clone())),
        [vec![Scalar::from(1_i64)], vec![Scalar::Null]]
    );
    let message =
        try_rows(&held, &CsvOptions::new().with_field(field).with_safe(false)).unwrap_err();
    assert!(message.contains("$[1].n"), "{message}");
    assert!(message.contains("in row 3 of"), "{message}");
}

#[test]
fn a_null_cell_under_a_required_column_is_refused() {
    let held = document(b"n\n1\n\n2\n,\n");
    // The blank line is a separator; the `,` line is two empty cells, one
    // too many for the header.
    let field = declared("struct<n: int64 not null>");
    let message = try_rows(&held, &CsvOptions::new().with_field(field)).unwrap_err();
    assert!(
        message.contains("expected 1 cells, got 2 in row 5"),
        "{message}"
    );
    let held = document(b"n\n1\nNA\n");
    let field = declared("struct<n: int64 not null>");
    let message = try_rows(
        &held,
        &CsvOptions::new()
            .with_field(field)
            .with_null_values(["NA"])
            .unwrap(),
    )
    .unwrap_err();
    assert!(message.contains("$[1].n"), "{message}");
}

// The cut.

#[test]
fn rfc_4180_quoting_reads_the_separator_line_breaks_and_doubled_quotes_as_content() {
    let held =
        document(b"a,b\n\"x, y\",\"line1\nline2\"\n\"say \"\"hi\"\"\",plain\n\"\",\"\"\"\"\n");
    assert_eq!(
        text(&rows(&held, &CsvOptions::new())),
        [
            some(&["x, y", "line1\nline2"]),
            some(&["say \"hi\"", "plain"]),
            some(&["", "\""]),
        ]
    );
}

#[test]
fn a_quote_in_an_unquoted_cell_is_content_and_bytes_after_a_closing_quote_are_too() {
    let held = document(b"a,b\n5\" tall,\"x\"y\n");
    assert_eq!(
        text(&rows(&held, &CsvOptions::new())),
        [some(&["5\" tall", "xy"])]
    );
}

#[test]
fn crlf_and_lf_both_end_a_record_and_a_lone_cr_ends_nothing() {
    let held = document(b"a,b\r\n1,2\n3,4\r\n5\r6,7");
    assert_eq!(
        text(&rows(&held, &CsvOptions::new())),
        [some(&["1", "2"]), some(&["3", "4"]), some(&["5\r6", "7"])]
    );
}

#[test]
fn the_last_record_may_lack_a_terminator_and_a_quoted_one_may_end_the_stream() {
    assert_eq!(
        text(&rows(&document(b"a,b\n1,2"), &CsvOptions::new())),
        [some(&["1", "2"])]
    );
    assert_eq!(
        text(&rows(&document(b"a\n\"open"), &CsvOptions::new())),
        [some(&["open"])]
    );
}

#[test]
fn blank_and_comment_records_are_skipped_and_never_counted() {
    let options = CsvOptions::new().with_comment(Some(b'#')).unwrap();
    let held = document(b"# a comment first\na,b\n\n# note\n1,2\n\r\n#3,4\n5,6\n\n");
    assert_eq!(
        text(&rows(&held, &options)),
        [some(&["1", "2"]), some(&["5", "6"])]
    );
    let media = yggdryl::csv::Csv::new(held).with_options(options);
    assert_eq!(
        media.row_size().expect("the rows"),
        2,
        "the count skips them too"
    );
    // Without a comment byte, `#` opens an ordinary cell.
    assert_eq!(
        text(&rows(&document(b"a\n#x\n"), &CsvOptions::new())),
        [some(&["#x"])]
    );
}

#[test]
fn a_byte_order_mark_is_framing_and_not_the_first_name() {
    let held = document(b"\xEF\xBB\xBFa,b\n1,2\n");
    assert_inferred(&held, &CsvOptions::new(), "struct<a: int64, b: int64>");
}

#[test]
fn a_tsv_name_reads_tabs_and_an_option_reads_any_other_separator() {
    let held = named("trades.tsv", b"a\tb\n1,5\t2\n");
    let options = match held.record_options().expect("options") {
        yggdryl::media::RecordOptions::Csv(options) => options,
        other => panic!("expected CSV options, got {other:?}"),
    };
    assert_eq!(options.separator(), b'\t');
    assert_eq!(text(&rows(&held, &options)), [some(&["1,5", "2"])]);

    for separator in [b';', b'|'] {
        let mut bytes = b"a?b\n1,5?\"x?y\"\n".to_vec();
        for byte in &mut bytes {
            if *byte == b'?' {
                *byte = separator;
            }
        }
        let held = document(&bytes);
        let options = CsvOptions::new().with_separator(separator).unwrap();
        assert_eq!(
            text(&rows(&held, &options)),
            [some(&["1,5", &format!("x{}y", char::from(separator))])],
            "{}",
            char::from(separator)
        );
    }
}

#[test]
fn an_escape_byte_spells_the_next_byte_as_content() {
    let options = CsvOptions::new().with_escape(Some(b'\\')).unwrap();
    let held = document(b"a,b\n\"x\\\"y\\\\z\",\"p\\,q\"\n");
    assert_eq!(text(&rows(&held, &options)), [some(&["x\"y\\z", "p,q"])]);
    // With an escape, a doubled quote closes the cell, and the two after
    // it are content after the closing quote.
    let held = document(b"a\n\"\"\"\"\n");
    assert_eq!(text(&rows(&held, &options)), [some(&["\"\""])]);
}

#[test]
fn null_spellings_decide_absence_and_a_quoted_cell_is_never_absent() {
    let held = document(b"a,b,c\n,\"\",NA\nNA,\"NA\",\n");
    let options = CsvOptions::new().with_null_values(["", "NA"]).unwrap();
    assert_eq!(
        text(&rows(&held, &options)),
        [
            vec![None, Some(String::new()), None],
            vec![None, Some("NA".to_owned()), None],
        ]
    );
    // With no null spelling at all, an empty cell is the empty text.
    let none = CsvOptions::new()
        .with_null_values::<[&str; 0], &str>([])
        .unwrap();
    assert_eq!(
        text(&rows(&document(b"a,b\n,\n"), &none)),
        [some(&["", ""])]
    );
}

#[test]
fn trim_drops_the_blanks_around_a_cell_and_keeps_a_quoted_cells_own() {
    let held = document(b"a,b\n x , \" y \" \n");
    assert_eq!(
        text(&rows(&held, &CsvOptions::new())),
        [some(&[" x ", " \" y \" "])]
    );
    assert_eq!(
        text(&rows(&held, &CsvOptions::new().with_trim(true))),
        [some(&["x", " y "])]
    );
    // A trimmed empty cell is the empty spelling, so it is null.
    assert_eq!(
        rows(&document(b"a\n  \n"), &CsvOptions::new().with_trim(true)),
        [vec![Scalar::Null]]
    );
}

#[test]
fn without_a_header_the_columns_are_numbered_and_every_record_is_a_row() {
    let held = document(b"1,x\n2,y\n");
    let options = CsvOptions::new().with_header(false);
    assert_inferred(&held, &options, "struct<column_1: int64, column_2: utf8>");
    assert_eq!(
        rows(&held, &options),
        [
            vec![Scalar::from(1_i64), Scalar::from("x")],
            vec![Scalar::from(2_i64), Scalar::from("y")]
        ]
    );
    assert_eq!(
        held.row_size().expect("the rows"),
        1,
        "the first record is the header under the default"
    );
    let media = yggdryl::csv::Csv::new(held).with_options(options);
    assert_eq!(media.row_size().expect("the rows"), 2);
}

#[test]
fn an_empty_header_name_is_numbered() {
    let held = document(b"a,,c\n1,2,3\n");
    assert_inferred(
        &held,
        &CsvOptions::new(),
        "struct<a: int64, column_2: int64, c: int64>",
    );
}

#[test]
fn a_declared_charset_is_decoded_by_the_transport_and_a_stray_byte_reads_as_windows_1252() {
    let mut declared = named("names.csv", b"name\nSoci\xE9t\xE9\n");
    declared.set_media_type(declared.media_type().clone().with_charset(Charset::Cp1252));
    assert_eq!(
        text(&rows(&declared, &CsvOptions::new())),
        [some(&["Soci\u{e9}t\u{e9}"])]
    );
    let undeclared = named("names.csv", b"name\nSoci\xE9t\xE9\n");
    assert_eq!(
        text(&rows(&undeclared, &CsvOptions::new())),
        [some(&["Soci\u{e9}t\u{e9}"])]
    );
}

// Types.

#[test]
fn a_declared_field_reads_each_cell_under_its_column() {
    let held = document(
        b"id,price,ok,at,name\n1,12.50,TRUE,2024-01-02T03:04:05Z,x\n2,,false,2024-01-02 03:04:05,\n",
    );
    let field = declared(
        "struct<id: int32 not null, price: decimal(10, 2), ok: boolean, at: datetime64(ms, UTC), name: utf8>",
    );
    let read = rows(&held, &CsvOptions::new().with_field(field));
    let instant = Scalar::datetime64(1_704_164_645_000, TimeUnit::Millisecond, Timezone::UTC)
        .expect("an instant");
    assert_eq!(
        read,
        [
            vec![
                Scalar::from(1_i32),
                Scalar::decimal128(1250, 2),
                Scalar::from(true),
                instant.clone(),
                Scalar::from("x"),
            ],
            vec![
                Scalar::from(2_i32),
                Scalar::Null,
                Scalar::from(false),
                instant,
                Scalar::Null,
            ],
        ]
    );
}

#[test]
fn a_declared_field_reads_columns_by_name_whatever_the_header_order() {
    let held = document(b"b,a,extra\nx,1,ignored\ny,2,ignored\n");
    let field = declared("struct<a: int64 not null, b: utf8, c: utf8>");
    assert_eq!(
        rows(&held, &CsvOptions::new().with_field(field.clone())),
        [
            vec![Scalar::from(1_i64), Scalar::from("x"), Scalar::Null],
            vec![Scalar::from(2_i64), Scalar::from("y"), Scalar::Null],
        ]
    );
    // The declared field is what the schema answers.
    assert_eq!(
        read_field(&held, &CsvOptions::new().with_field(field.clone())).expect("declared"),
        field
    );
}

#[test]
fn without_a_header_a_declared_field_names_the_columns_positionally() {
    let held = document(b"1,x\n2,y\n");
    let field = declared("struct<id: int64 not null, name: utf8>");
    assert_eq!(
        rows(
            &held,
            &CsvOptions::new().with_header(false).with_field(field)
        ),
        [
            vec![Scalar::from(1_i64), Scalar::from("x")],
            vec![Scalar::from(2_i64), Scalar::from("y")],
        ]
    );
    // A declared column past the record's width is absent.
    let wide = declared("struct<id: int64, name: utf8, venue: utf8>");
    assert_eq!(
        rows(
            &held,
            &CsvOptions::new().with_header(false).with_field(wide)
        ),
        [
            vec![Scalar::from(1_i64), Scalar::from("x"), Scalar::Null],
            vec![Scalar::from(2_i64), Scalar::from("y"), Scalar::Null],
        ]
    );
}

#[test]
fn a_nested_declared_column_reads_compact_json() {
    let held = document(b"tags,meta\n\"[\"\"a\"\",\"\"b\"\"]\",\"{\"\"k\"\":1}\"\n");
    let field = declared("struct<tags: serie<utf8>, meta: struct<k: int64>>");
    let read = rows(&held, &CsvOptions::new().with_field(field));
    assert_eq!(read.len(), 1);
    assert_eq!(
        read[0][0].as_serie().expect("a serie").rows().to_vec(),
        [Scalar::from("a"), Scalar::from("b")]
    );
    assert_eq!(
        read[0][1]
            .as_serie()
            .expect("an ordered struct")
            .rows()
            .to_vec(),
        [Scalar::from(1_i64)]
    );
}

#[test]
fn the_inference_ladder_types_each_column_by_the_first_rung_every_cell_fits() {
    let held = document(
        b"b,i,f,d,t,s,m,n,q\n\
          true,1,1.5,2024-01-02,2024-01-02T03:04:05Z,x,1,,\"1\"\n\
          FALSE,-2,2,2024-02-03,2024-02-03 04:05:06,y,x,,\"2\"\n\
          ,+3,1e3,,,z,2,,3\n",
    );
    assert_inferred(
        &held,
        &CsvOptions::new(),
        "struct<b: boolean, i: int64, f: float64, d: date32, t: datetime64(ns, UTC), \
         s: utf8, m: utf8, n: utf8, q: int64>",
    );
    let read = rows(&held, &CsvOptions::new());
    assert_eq!(read[0][0], Scalar::from(true));
    assert_eq!(read[1][0], Scalar::from(false));
    assert_eq!(read[2][0], Scalar::Null);
    assert_eq!(read[2][1], Scalar::from(3_i64));
    assert_eq!(read[2][2], Scalar::from(1000_f64));
    assert_eq!(read[0][3], Scalar::date32(19_724), "2024-01-02");
    assert_eq!(
        read[1][4],
        Scalar::datetime64(
            1_706_933_106_000_000_000,
            TimeUnit::Nanosecond,
            Timezone::UTC
        )
        .unwrap()
    );
    assert_eq!(read[2][4], Scalar::Null);
    assert_eq!(read[0][6], Scalar::from("1"));
    assert_eq!(read[0][7], Scalar::Null);
    assert_eq!(
        read[0][8],
        Scalar::from(1_i64),
        "a quoted number is a number"
    );
}

#[test]
fn the_sampled_rows_come_first_and_the_rest_streams_under_the_inferred_field() {
    let held = document(b"i\n1\n2\n3\n4\nx\n");
    let options = CsvOptions::new().with_infer_row_size(2).unwrap();
    assert_inferred(&held, &options, "struct<i: int64>");
    // Under `safe`, the row past the sample that does not fit is null.
    assert_eq!(
        rows(&held, &options),
        [
            vec![Scalar::from(1_i64)],
            vec![Scalar::from(2_i64)],
            vec![Scalar::from(3_i64)],
            vec![Scalar::from(4_i64)],
            vec![Scalar::Null],
        ]
    );
    let message = try_rows(&held, &options.clone().with_safe(false)).unwrap_err();
    assert!(message.contains("$[4].i"), "{message}");
    // A wider sample sees the text and types the column as such.
    assert_inferred(
        &held,
        &CsvOptions::new().with_infer_row_size(5).unwrap(),
        "struct<i: utf8>",
    );
}

#[test]
fn the_inferred_field_carries_the_options_name_and_the_declared_one_its_own() {
    let held = document(b"a\n1\n");
    let mut options = CsvOptions::new();
    options.set_name("trade".into());
    assert_inferred(&held, &options, "struct<a: int64>");
    let field =
        DataType::from(StructType::from_fields([DataType::Int64.nullable_field("a")]).unwrap())
            .required_field("quote");
    assert_eq!(
        read_field(&held, &CsvOptions::new().with_field(field.clone())).unwrap(),
        field
    );
}

#[test]
fn trim_leaves_a_blank_that_is_the_separator_alone() {
    // A tab is the separator of a TSV and a space of a space-separated
    // document: trimming the blanks around a cell must not eat the empty
    // cells between two separators, or a three-cell record reads as two.
    // An empty cell, trimmed or not, spells the default null.
    let tsv = named("trades.tsv", b"a\tb\tc\n1\t\t3\n \t2\t \n");
    assert_eq!(
        text(&rows(&tsv, &CsvOptions::tsv().with_trim(true))),
        [
            vec![Some("1".to_owned()), None, Some("3".to_owned())],
            vec![None, Some("2".to_owned()), None]
        ]
    );
    let spaced = document(b"a b c\n1  3\n");
    assert_eq!(
        text(&rows(
            &spaced,
            &CsvOptions::new()
                .with_separator(b' ')
                .expect("a space separator")
                .with_trim(true)
        )),
        [vec![Some("1".to_owned()), None, Some("3".to_owned())]]
    );
}
