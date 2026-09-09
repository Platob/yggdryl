use arrow_array::{Array as _, Int64Array, RecordBatch, StringArray};

use crate::holder::Buffer;
use crate::media::csv::{Csv, CsvOptions};
use crate::media::text::LineSep;
use crate::media::{IORecordOptions as _, RecordOptions};
use crate::{Codec, DataType, Scalar, Timezone};
use crate::{IOBase as _, IOMedia as _};

fn named(name: &str, bytes: &[u8]) -> Buffer {
    Buffer::from_bytes(bytes.to_vec()).with_media_type(
        crate::Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    )
}

fn csv(bytes: &[u8]) -> Csv<Buffer> {
    Csv::new(named("trades.csv", bytes))
}

fn collect(source: &impl crate::IOBase, options: &CsvOptions) -> Vec<RecordBatch> {
    source
        .read_arrow_reader(&options.clone().into())
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
}

fn column_text(batches: &[RecordBatch], name: &str) -> Vec<Option<String>> {
    batches
        .iter()
        .flat_map(|batch| {
            let index = batch.schema().index_of(name).unwrap();
            batch
                .column(index)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .iter()
                .map(|value| value.map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn column_int(batches: &[RecordBatch], name: &str) -> Vec<Option<i64>> {
    batches
        .iter()
        .flat_map(|batch| {
            let index = batch.schema().index_of(name).unwrap();
            batch
                .column(index)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .iter()
                .collect::<Vec<_>>()
        })
        .collect()
}

fn dtype(source: &impl crate::IOBase, options: &CsvOptions, column: usize) -> DataType {
    crate::media::csv::read_field(source, options)
        .unwrap()
        .get_field(column)
        .unwrap()
        .dtype()
        .clone()
}

#[test]
fn a_header_names_the_columns_and_the_cells_type_them() {
    let source = named(
        "trades.csv",
        b"symbol,quantity,price,traded_on,settled\nBRN,120,81.5,2024-01-02,true\nWTI,80,77.25,2024-01-03,false\n",
    );
    let options = CsvOptions::new();
    assert_eq!(dtype(&source, &options, 0), DataType::Utf8);
    assert_eq!(dtype(&source, &options, 1), DataType::Int64);
    assert_eq!(dtype(&source, &options, 2), DataType::Float64);
    assert_eq!(dtype(&source, &options, 3), DataType::Date32);
    assert_eq!(dtype(&source, &options, 4), DataType::Boolean);

    let batches = collect(&source, &options);
    assert_eq!(
        column_text(&batches, "symbol"),
        [Some("BRN".to_owned()), Some("WTI".to_owned())]
    );
    assert_eq!(column_int(&batches, "quantity"), [Some(120), Some(80)]);
}

#[test]
fn a_mixed_column_meets_where_both_readings_fit() {
    let widened = named("t.csv", b"value\n1\n1.5\n");
    assert_eq!(dtype(&widened, &CsvOptions::new(), 0), DataType::Float64);

    let text = named("t.csv", b"value\n1\nAAPL\n");
    assert_eq!(dtype(&text, &CsvOptions::new(), 0), DataType::Utf8);

    // A null cell yields to whatever the column otherwise holds.
    let nulled = named("t.csv", b"value\n\n7\n");
    assert_eq!(dtype(&nulled, &CsvOptions::new(), 0), DataType::Int64);

    // A column of nothing but absence is text, the floor.
    let empty = named("t.csv", b"value\n\n\n");
    assert_eq!(dtype(&empty, &CsvOptions::new(), 0), DataType::Utf8);
}

#[test]
fn a_zero_padded_numeric_keeps_its_padding() {
    let source = named("t.csv", b"account\n007\n042\n");
    assert_eq!(dtype(&source, &CsvOptions::new(), 0), DataType::Utf8);
    let batches = collect(&source, &CsvOptions::new());
    assert_eq!(
        column_text(&batches, "account"),
        [Some("007".to_owned()), Some("042".to_owned())]
    );

    // A whole number too wide for the column stays text rather than losing
    // digits to a double.
    let wide = named("t.csv", b"id\n123456789012345678901234567890\n");
    assert_eq!(dtype(&wide, &CsvOptions::new(), 0), DataType::Utf8);
}

#[test]
fn a_quoted_cell_carries_separators_quotes_and_terminators() {
    let source = named(
        "t.csv",
        b"symbol,note\nBRN,\"held, then sold\"\nWTI,\"said \"\"buy\"\"\"\nAPI,\"first\r\nsecond\"\n",
    );
    let batches = collect(&source, &CsvOptions::new());
    assert_eq!(
        column_text(&batches, "note"),
        [
            Some("held, then sold".to_owned()),
            Some("said \"buy\"".to_owned()),
            Some("first\r\nsecond".to_owned()),
        ]
    );
}

#[test]
fn a_custom_separator_quote_and_escape_are_read_and_written() {
    let options = CsvOptions::new()
        .try_with_separator(b';')
        .unwrap()
        .try_with_quote(b'\'')
        .unwrap()
        .try_with_escape(b'\\')
        .unwrap();
    let source = named("t.csv", b"symbol;note\nBRN;'held\\'s; sold'\n");
    let batches = collect(&source, &options);
    assert_eq!(
        column_text(&batches, "note"),
        [Some("held's; sold".to_owned())]
    );

    let mut target = named("out.csv", b"");
    target
        .overwrite_arrow_reader(
            crate::arrow::batch_reader(batches[0].schema(), batches.clone()),
            &options.clone().into(),
        )
        .unwrap();
    assert_eq!(
        target.read_all_bytes().unwrap(),
        b"symbol;note\nBRN;'held\\'s; sold'\n"
    );
}

#[test]
fn one_byte_cannot_carry_two_meanings() {
    let message = CsvOptions::new()
        .try_with_quote(b',')
        .unwrap_err()
        .to_string();
    assert!(message.contains("the separator"), "{message}");
    assert!(
        CsvOptions::new().try_with_separator(b'\n').is_err(),
        "a terminator byte cannot also separate cells"
    );
    // A quote that escapes itself is RFC 4180's doubling, not a collision.
    assert!(CsvOptions::new().try_with_escape(b'"').is_ok());
}

#[test]
fn comments_blank_lines_and_a_pinned_terminator_are_honored() {
    let options = CsvOptions::new()
        .try_with_comment(b'#')
        .unwrap()
        .try_with_linesep(LineSep::CRLF)
        .unwrap();
    // A lone LF is content under a pinned CRLF, so it stays inside the cell.
    let source = named(
        "t.csv",
        b"# a note\r\nsymbol,note\r\nBRN,one\ntwo\r\n\r\nWTI,three\r\n",
    );
    let batches = collect(&source, &options);
    assert_eq!(
        column_text(&batches, "note"),
        [Some("one\ntwo".to_owned()), Some("three".to_owned())]
    );
}

#[test]
fn a_headerless_resource_names_its_columns_by_position() {
    let options = CsvOptions::new().with_header(false);
    let source = named("t.csv", b"BRN,120\nWTI,80\n");
    let field = crate::media::csv::read_field(&source, &options).unwrap();
    assert_eq!(field.get_field(0).unwrap().name(), "column_1");
    assert_eq!(field.get_field(1).unwrap().name(), "column_2");
    assert_eq!(
        column_int(&collect(&source, &options), "column_2"),
        [Some(120), Some(80)]
    );
}

#[test]
fn a_ragged_record_is_refused_by_name() {
    let source = named("t.csv", b"symbol,quantity\nBRN,120\nWTI\n");
    let message = source
        .read_arrow_reader(&CsvOptions::new().into())
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(message.contains("expected 2 cells"), "{message}");
    assert!(message.contains("in row 1 of"), "{message}");
}

#[test]
fn absence_is_the_null_spelling_and_nothing_else() {
    // Under an explicit spelling, an empty cell is the empty string: only the
    // spelling is absence.
    let options = CsvOptions::new().with_null("NULL");
    let source = named("t.csv", b"symbol,note\nBRN,NULL\nWTI,\n");
    let batches = collect(&source, &options);
    assert_eq!(column_text(&batches, "note"), [None, Some(String::new())]);

    let mut target = named("out.csv", b"");
    target
        .overwrite_arrow_reader(
            crate::arrow::batch_reader(batches[0].schema(), batches.clone()),
            &options.clone().into(),
        )
        .unwrap();
    assert_eq!(
        target.read_all_bytes().unwrap(),
        b"symbol,note\nBRN,NULL\nWTI,\n"
    );
}

#[test]
fn a_quoted_empty_cell_is_a_value_and_a_bare_one_is_absence() {
    // The default spelling is the empty cell, so the two are told apart by the
    // quoting a write puts there and a read reads back.
    let source = named("t.csv", b"symbol,note\nBRN,\nWTI,\"\"\n");
    let batches = collect(&source, &CsvOptions::new());
    assert_eq!(column_text(&batches, "note"), [None, Some(String::new())]);

    let mut target = named("out.csv", b"");
    target
        .overwrite_arrow_reader(
            crate::arrow::batch_reader(batches[0].schema(), batches.clone()),
            &CsvOptions::new().into(),
        )
        .unwrap();
    assert_eq!(
        target.read_all_bytes().unwrap(),
        b"symbol,note\nBRN,\nWTI,\"\"\n"
    );

    // And the round trip is stable: reading what was written answers the same
    // two values rather than collapsing them.
    let round_trip = collect(&target, &CsvOptions::new());
    assert_eq!(
        column_text(&round_trip, "note"),
        [None, Some(String::new())]
    );
}

#[test]
fn a_declared_schema_reads_the_cells_the_header_names() {
    // The declaration names the columns in another order than the file does.
    // The header says which cell is which, so the values still land right.
    let source = named("t.csv", b"quantity,symbol\n120,BRN\n80,WTI\n");
    let field =
        crate::Field::from_str("row: struct<symbol: utf8, quantity: int64> not null").unwrap();
    let mut options = CsvOptions::new();
    options.set_field(field);
    let batches = collect(&source, &options);
    assert_eq!(
        column_text(&batches, "symbol"),
        [Some("BRN".to_owned()), Some("WTI".to_owned())]
    );
    assert_eq!(column_int(&batches, "quantity"), [Some(120), Some(80)]);
}

#[test]
fn trimming_removes_edge_whitespace_from_unquoted_cells_only() {
    let options = CsvOptions::new().with_trim(true);
    let source = named("t.csv", b"symbol, note\nBRN,  spaced  \nWTI,\"  kept  \"\n");
    let batches = collect(&source, &options);
    assert_eq!(
        column_text(&batches, "note"),
        [Some("spaced".to_owned()), Some("  kept  ".to_owned())]
    );
}

#[test]
fn dimensions_come_from_the_bytes_without_decoding_rows() {
    let source = named("t.csv", b"symbol,quantity\nBRN,120\nWTI,80\n");
    assert_eq!(
        crate::media::csv::row_size(&source, &CsvOptions::new()).unwrap(),
        2
    );
    let media = Csv::new(source);
    assert_eq!(crate::IOMedia::row_size(&media).unwrap(), 2);
    assert_eq!(media.column_size().unwrap(), 2);
    assert!(media.is_tabular());
}

#[test]
fn rows_round_trip_through_overwrite_and_append() {
    let source = named("t.csv", b"symbol,quantity\nBRN,120\n");
    let options = CsvOptions::new();
    let batches = collect(&source, &options);

    let mut target = named("out.csv", b"");
    let record_options: RecordOptions = options.clone().into();
    target
        .overwrite_arrow_reader(
            crate::arrow::batch_reader(batches[0].schema(), batches.clone()),
            &record_options,
        )
        .unwrap();
    assert_eq!(
        target.read_all_bytes().unwrap(),
        b"symbol,quantity\nBRN,120\n"
    );

    // An append adds rows and no second header.
    target
        .append_arrow_reader(
            crate::arrow::batch_reader(batches[0].schema(), batches.clone()),
            &record_options,
        )
        .unwrap();
    assert_eq!(
        target.read_all_bytes().unwrap(),
        b"symbol,quantity\nBRN,120\nBRN,120\n"
    );
    assert_eq!(crate::media::csv::row_size(&target, &options).unwrap(), 2);
}

#[test]
fn an_append_lands_in_the_columns_it_names() {
    let mut target = named("out.csv", b"symbol,quantity\nBRN,120\n");
    let options = CsvOptions::new();
    // The incoming batch names the same columns in the other order.
    let swapped = named("in.csv", b"quantity,symbol\n80,WTI\n");
    let batches = collect(&swapped, &options);
    target
        .append_arrow_reader(
            crate::arrow::batch_reader(batches[0].schema(), batches.clone()),
            &options.clone().into(),
        )
        .unwrap();
    assert_eq!(
        target.read_all_bytes().unwrap(),
        b"symbol,quantity\nBRN,120\nWTI,80\n"
    );

    // Reached directly, without the write pipeline's completion cast, a batch
    // that does not carry the stored columns is refused rather than written
    // short into the wrong ones.
    let short = named("in.csv", b"symbol\nAPI\n");
    let short_batches = collect(&short, &options);
    let refused = crate::media::csv::append_arrow_reader(
        &mut target,
        crate::arrow::batch_reader(short_batches[0].schema(), short_batches),
        &options,
    );
    let message = refused
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(message.contains("stored columns"), "{message}");
}

#[test]
fn rows_merge_on_a_key_because_a_row_has_identity() {
    let mut target = named("out.csv", b"symbol,quantity\nBRN,120\nWTI,80\n");
    let options = CsvOptions::new();
    let incoming = named("in.csv", b"symbol,quantity\nWTI,95\nAPI,10\n");
    let batches = collect(&incoming, &options);
    let mut merging: RecordOptions = options.clone().into();
    merging.set_merge_by_names(vec!["symbol".to_owned()]);
    target
        .merge_arrow_reader(
            crate::arrow::batch_reader(batches[0].schema(), batches),
            &merging,
        )
        .unwrap();
    assert_eq!(
        target.read_all_bytes().unwrap(),
        b"symbol,quantity\nBRN,120\nWTI,95\nAPI,10\n"
    );
}

#[test]
fn a_row_and_a_cell_are_read_positionally() {
    let media = csv(b"symbol,quantity,traded_on\nBRN,120,2024-01-02\nWTI,80,2024-01-03\n");
    assert_eq!(media.read_cell_text(1, 0).unwrap().as_deref(), Some("WTI"));
    assert_eq!(
        media.read_cell_bytes(0, 1).unwrap().as_deref(),
        Some(b"120".as_slice())
    );
    assert_eq!(
        media.read_cell_scalar(1, 1).unwrap(),
        Some(Scalar::from(80_i64))
    );
    assert_eq!(
        media
            .read_row_scalar(0)
            .unwrap()
            .unwrap()
            .as_sequence()
            .unwrap()[0],
        Scalar::from("BRN")
    );
    assert_eq!(media.read_row_byte_range(1).unwrap(), Some(45..63));
    assert_eq!(media.read_row_scalar(2).unwrap(), None);
    let message = media
        .read_cell_text(0, 9)
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(message.contains("column 9"), "{message}");
}

#[test]
fn a_cell_is_written_in_place_whatever_its_new_width() {
    let mut media = csv(b"symbol,quantity\nBRN,120\nWTI,80\n");

    // Same width: one positional write.
    media.write_cell_text(0, 1, "999").unwrap();
    assert_eq!(
        media.handle().read_all_bytes().unwrap(),
        b"symbol,quantity\nBRN,999\nWTI,80\n"
    );

    // Wider: the tail moves up.
    media
        .write_cell_scalar(1, 1, &Scalar::from(1_000_000_i64))
        .unwrap();
    assert_eq!(
        media.handle().read_all_bytes().unwrap(),
        b"symbol,quantity\nBRN,999\nWTI,1000000\n"
    );

    // Narrower: the tail moves down and the resource shrinks.
    media.write_cell_text(0, 0, "B").unwrap();
    assert_eq!(
        media.handle().read_all_bytes().unwrap(),
        b"symbol,quantity\nB,999\nWTI,1000000\n"
    );

    // A value needing quotes gets them.
    media.write_cell_text(0, 0, "a,b").unwrap();
    assert_eq!(
        media.handle().read_all_bytes().unwrap(),
        b"symbol,quantity\n\"a,b\",999\nWTI,1000000\n"
    );
    assert_eq!(media.read_cell_text(0, 0).unwrap().as_deref(), Some("a,b"));
}

#[test]
fn a_row_is_replaced_added_and_removed_in_place() {
    let mut media = csv(b"symbol,quantity\nBRN,120\nWTI,80\n");
    let row = Scalar::from_sequence([Scalar::from("API"), Scalar::from(7_i64)]);
    media.write_row_scalar(0, &row).unwrap();
    assert_eq!(
        media.handle().read_all_bytes().unwrap(),
        b"symbol,quantity\nAPI,7\nWTI,80\n"
    );

    media
        .append_row_scalar(&Scalar::from_sequence([
            Scalar::from("GAS"),
            Scalar::from(3_i64),
        ]))
        .unwrap();
    assert_eq!(
        media.handle().read_all_bytes().unwrap(),
        b"symbol,quantity\nAPI,7\nWTI,80\nGAS,3\n"
    );
    assert_eq!(media.row_size().unwrap(), 3);

    media.remove_row(1).unwrap();
    assert_eq!(
        media.handle().read_all_bytes().unwrap(),
        b"symbol,quantity\nAPI,7\nGAS,3\n"
    );
    assert_eq!(media.row_size().unwrap(), 2);
}

#[test]
fn positional_reads_stay_exact_past_the_index_stride() {
    // More rows than the index holds anchors, so the stride has doubled and a
    // lookup is an anchor plus a bounded forward scan.
    let mut bytes = Vec::from(b"symbol,quantity\n".as_slice());
    for row in 0..12_000_u32 {
        bytes.extend_from_slice(format!("S{row},{row}\n").as_bytes());
    }
    let media = csv(&bytes);
    assert_eq!(media.row_size().unwrap(), 12_000);
    assert_eq!(media.read_cell_text(0, 0).unwrap().as_deref(), Some("S0"));
    assert_eq!(
        media.read_cell_text(6_543, 1).unwrap().as_deref(),
        Some("6543")
    );
    assert_eq!(
        media.read_cell_text(11_999, 0).unwrap().as_deref(),
        Some("S11999")
    );
    assert_eq!(media.read_cell_text(12_000, 0).unwrap(), None);
}

#[test]
fn a_multiline_record_is_addressable_and_replaceable() {
    let mut media = csv(b"symbol,note\nBRN,\"one\ntwo\"\nWTI,three\n");
    assert_eq!(
        media.read_cell_text(0, 1).unwrap().as_deref(),
        Some("one\ntwo")
    );
    assert_eq!(media.read_cell_text(1, 0).unwrap().as_deref(), Some("WTI"));
    media.write_cell_text(0, 1, "flat").unwrap();
    assert_eq!(
        media.handle().read_all_bytes().unwrap(),
        b"symbol,note\nBRN,flat\nWTI,three\n"
    );
}

#[test]
fn a_declared_schema_reads_the_columns_it_declares() {
    let source = named("t.csv", b"symbol,quantity\nBRN,120\n");
    let field =
        crate::Field::from_str("row: struct<symbol: utf8, quantity: decimal128(12, 2)> not null")
            .unwrap();
    let mut options = CsvOptions::new();
    options.set_field(field.clone());
    assert_eq!(
        crate::media::csv::read_field(&source, &options).unwrap(),
        field
    );
    let batches = collect(&source, &options);
    assert_eq!(
        batches[0].schema().field(1).data_type(),
        &arrow_schema::DataType::Decimal128(12, 2)
    );
}

#[test]
fn a_coded_resource_reads_rows_and_refuses_a_positional_write() {
    let plain = named("t.csv", b"symbol,quantity\nBRN,120\nWTI,80\n");
    let mut coded = named("t.csv.gz", b"");
    plain.compress_into(&mut coded, Codec::Gzip).unwrap();
    assert_eq!(coded.media_type().base(), &crate::MimeType::CSV);

    let options = CsvOptions::new();
    assert_eq!(crate::media::csv::row_size(&coded, &options).unwrap(), 2);
    let batches = collect(&coded, &options);
    assert_eq!(column_int(&batches, "quantity"), [Some(120), Some(80)]);

    let mut media = Csv::new(coded);
    // A decoded offset is still addressable for reading.
    assert_eq!(media.read_cell_text(1, 0).unwrap().as_deref(), Some("WTI"));
    let message = media.write_cell_text(1, 0, "API").unwrap_err().to_string();
    assert!(message.contains("uncoded resource"), "{message}");
}

#[test]
fn the_encoding_comes_from_the_media_type() {
    let options = RecordOptions::for_media_type(
        &crate::Url::from_str("file:///trades.csv")
            .unwrap()
            .media_type(),
    )
    .unwrap();
    assert!(matches!(options, RecordOptions::Csv(_)));
    assert_eq!(options.mime_type(), crate::MimeType::CSV);

    let media = crate::media::Media::open(crate::holder::Holder::buffer(named(
        "trades.csv",
        b"symbol\nBRN\n",
    )))
    .unwrap();
    assert!(matches!(media, crate::media::Media::Csv(_)));
    assert_eq!(media.row_size().unwrap(), 1);

    // Options of another encoding are refused before a write pulls a batch.
    let mut wrong = Csv::new(named("t.csv", b""));
    let refused = wrong.append_arrow_reader(
        crate::arrow::batch_reader(std::sync::Arc::new(arrow_schema::Schema::empty()), []),
        &RecordOptions::for_mime_type(&crate::MimeType::AVRO).unwrap(),
    );
    let message = refused
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(message.contains("CSV record options"), "{message}");
}

#[test]
fn an_offset_free_instant_takes_the_configured_zone() {
    let zoned = CsvOptions::new().with_timezone(Timezone::from_str("UTC").unwrap());
    let source = named("t.csv", b"seen_at\n2024-01-02T03:04:05\n");
    assert_eq!(
        dtype(&source, &zoned, 0),
        DataType::DateTime64 {
            unit: crate::TimeUnit::Microsecond,
            timezone: Timezone::from_str("UTC").unwrap(),
        }
    );
    assert_eq!(
        dtype(&source, &CsvOptions::new(), 0),
        DataType::DateTime64 {
            unit: crate::TimeUnit::Microsecond,
            timezone: Timezone::NAIVE,
        }
    );
}

#[test]
fn inference_reads_only_the_rows_it_is_asked_to() {
    // The sample stops before the row that would have widened the column, so
    // the bound is observable rather than advisory.
    let source = named("t.csv", b"value\n1\n2\nAAPL\n");
    let sampled = CsvOptions::new().with_infer_row_size(2);
    assert_eq!(dtype(&source, &sampled, 0), DataType::Int64);
    assert_eq!(dtype(&source, &CsvOptions::new(), 0), DataType::Utf8);
}

#[test]
fn typing_off_reads_every_column_as_text() {
    let source = named("t.csv", b"symbol,quantity\nBRN,120\n");
    let options = CsvOptions::new().with_autotype(false);
    assert_eq!(dtype(&source, &options, 1), DataType::Utf8);
}

#[test]
fn an_added_row_follows_the_final_record_rather_than_continuing_it() {
    // The stored final record never got its terminator.
    let mut media = csv(b"symbol,quantity\nBRN,120");
    media
        .append_row_scalar(&Scalar::from_sequence([
            Scalar::from("WTI"),
            Scalar::from(80_i64),
        ]))
        .unwrap();
    assert_eq!(
        media.handle().read_all_bytes().unwrap(),
        b"symbol,quantity\nBRN,120\nWTI,80\n"
    );
    assert_eq!(media.row_size().unwrap(), 2);
}

#[test]
fn an_append_to_a_resource_holding_no_records_writes_the_header() {
    // The resource has bytes but no record, so there is no header to append
    // under and the rows would otherwise become one.
    let mut target = named("out.csv", b"\n");
    let options = CsvOptions::new();
    let source = named("in.csv", b"symbol,quantity\nBRN,120\n");
    let batches = collect(&source, &options);
    crate::media::csv::append_arrow_reader(
        &mut target,
        crate::arrow::batch_reader(batches[0].schema(), batches),
        &options,
    )
    .unwrap();
    assert_eq!(
        target.read_all_bytes().unwrap(),
        b"symbol,quantity\nBRN,120\n"
    );
}

#[test]
fn writing_the_empty_string_does_not_delete_the_value() {
    let mut media = csv(b"symbol,note\nBRN,one\n");
    media.write_cell_text(0, 1, "").unwrap();
    assert_eq!(
        media.handle().read_all_bytes().unwrap(),
        b"symbol,note\nBRN,\"\"\n"
    );
    assert_eq!(media.read_cell_text(0, 1).unwrap().as_deref(), Some(""));

    // And absence is still writable, as the spelling rather than as a value.
    media.write_cell_scalar(0, 1, &Scalar::Null).unwrap();
    assert_eq!(
        media.handle().read_all_bytes().unwrap(),
        b"symbol,note\nBRN,\n"
    );
    assert_eq!(media.read_cell_scalar(0, 1).unwrap(), Some(Scalar::Null));
}

#[test]
fn every_row_is_reachable_after_the_index_has_halved_its_resolution() {
    // Enough rows to force two compactions, so a lookup lands through a stride
    // of four rather than one. Rows vary in width, so an off-by-one anywhere in
    // the anchor arithmetic shows up as the wrong value rather than the wrong
    // offset.
    let mut bytes = Vec::from(b"id,symbol\n".as_slice());
    for row in 0..12_000_u32 {
        bytes.extend_from_slice(format!("{row},S{}\n", "x".repeat((row % 7) as usize)).as_bytes());
    }
    let mut media = csv(&bytes);
    media.open().unwrap();
    assert_eq!(media.row_size().unwrap(), 12_000);

    let mut previous: Option<std::ops::Range<u64>> = None;
    for row in 0..12_000_u64 {
        assert_eq!(
            media.read_cell_text(row, 0).unwrap().as_deref(),
            Some(row.to_string().as_str()),
            "row {row}"
        );
        let range = media.read_row_byte_range(row).unwrap().unwrap();
        if let Some(previous) = previous {
            // The records tile the resource: no byte is skipped or counted twice.
            assert_eq!(previous.end, range.start, "row {row}");
        }
        previous = Some(range);
    }
    assert_eq!(previous.unwrap().end, bytes.len() as u64);
}

#[test]
fn a_folder_of_leaves_reads_as_one_table_and_each_header_stays_a_header() {
    let mut root = crate::holder::local::Folder::temporary()
        .unwrap()
        .path()
        .unwrap();
    root.push(format!("yggdryl-csv-folder-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("part-0.csv"), b"symbol,quantity\nBRN,120\n").unwrap();
    // The second leaf names the same columns in the other order, which the
    // header is what settles.
    std::fs::write(root.join("part-1.csv"), b"quantity,symbol\n80,WTI\n").unwrap();

    let folder = crate::holder::Holder::folder(&root).unwrap();
    let options = folder.record_options().unwrap();
    assert!(matches!(options, RecordOptions::Csv(_)));

    let batches = folder
        .read_arrow_reader(&options)
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        column_text(&batches, "symbol"),
        [Some("BRN".to_owned()), Some("WTI".to_owned())]
    );
    assert_eq!(column_int(&batches, "quantity"), [Some(120), Some(80)]);
    assert_eq!(folder.row_size().unwrap(), 2);

    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_coded_resource_round_trips_overwrite_and_append() {
    let options = CsvOptions::new();
    let source = named("t.csv", b"symbol,quantity\nBRN,120\n");
    let batches = collect(&source, &options);
    let schema = batches[0].schema();

    let mut coded = named("t.csv.gz", b"");
    assert_eq!(coded.codec(), Codec::Gzip);
    coded
        .overwrite_arrow_reader(
            crate::arrow::batch_reader(schema.clone(), batches.clone()),
            &options.clone().into(),
        )
        .unwrap();
    // The stored bytes are gzip framing; the rows are what the encoding reads.
    assert_eq!(&coded.read_range_bytes(0, 2).unwrap(), &[0x1F, 0x8B]);
    assert_eq!(crate::media::csv::row_size(&coded, &options).unwrap(), 1);

    coded
        .append_arrow_reader(
            crate::arrow::batch_reader(schema, batches),
            &options.clone().into(),
        )
        .unwrap();
    assert_eq!(crate::media::csv::row_size(&coded, &options).unwrap(), 2);
    let read = collect(&coded, &options);
    assert_eq!(column_int(&read, "quantity"), [Some(120), Some(120)]);
}

#[test]
fn an_empty_cell_is_a_value_only_where_the_column_holds_one() {
    // The quoted empty cell falls outside the inference sample, so the column
    // is typed from the numbers and the empty reading has nowhere to go.
    let source = named("t.csv", b"quantity\n120\n80\n\"\"\n");
    let options = CsvOptions::new().with_infer_row_size(2);
    assert_eq!(dtype(&source, &options, 0), DataType::Int64);
    let batches = collect(&source, &options);
    assert_eq!(
        column_int(&batches, "quantity"),
        [Some(120), Some(80), None]
    );
}

#[test]
fn trimming_reaches_the_absence_spelling_too() {
    let options = CsvOptions::new().with_trim(true).with_null("NA");
    let source = named("t.csv", b"symbol,note\nBRN, NA \nWTI, kept \n");
    let batches = collect(&source, &options);
    assert_eq!(
        column_text(&batches, "note"),
        [None, Some("kept".to_owned())]
    );
}

#[test]
fn a_write_reads_the_resource_with_the_dialect_it_was_given() {
    // The stored resource is semicolon-separated. A write that probed it with
    // default options would read its header as one column and put the rows in
    // that one column instead.
    let options = CsvOptions::new().try_with_separator(b';').unwrap();
    let mut target = named("out.csv", b"symbol;quantity\nBRN;120\n");
    let source = named("in.csv", b"symbol;quantity\nWTI;80\n");
    let batches = collect(&source, &options);
    target
        .append_arrow_reader(
            crate::arrow::batch_reader(batches[0].schema(), batches),
            &options.clone().into(),
        )
        .unwrap();
    assert_eq!(
        target.read_all_bytes().unwrap(),
        b"symbol;quantity\nBRN;120\nWTI;80\n"
    );
    assert_eq!(crate::media::csv::row_size(&target, &options).unwrap(), 2);
}

#[test]
fn an_append_leaves_the_stored_records_exactly_as_they_were() {
    // A comment line, CRLF terminators, and quoting the writer would not have
    // chosen: an append that re-rendered the resource would lose all three.
    let options = CsvOptions::new().try_with_comment(b'#').unwrap();
    let stored = b"# a note\r\nsymbol,quantity\r\n\"BRN\",120\r\n";
    let mut target = named("out.csv", stored);
    let source = named("in.csv", b"symbol,quantity\nWTI,80\n");
    let batches = collect(&source, &options);
    target
        .append_arrow_reader(
            crate::arrow::batch_reader(batches[0].schema(), batches),
            &options.clone().into(),
        )
        .unwrap();
    let written = target.read_all_bytes().unwrap();
    assert!(
        written.starts_with(stored),
        "{:?}",
        String::from_utf8_lossy(&written)
    );
    assert_eq!(crate::media::csv::row_size(&target, &options).unwrap(), 2);
}

#[test]
fn a_sidecar_does_not_retype_a_folder_of_data_files() {
    let mut root = crate::holder::local::Folder::temporary()
        .unwrap()
        .path()
        .unwrap();
    root.push(format!("yggdryl-csv-sidecar-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    // The sidecar sorts first, so listing order alone would have it win.
    std::fs::write(root.join("_manifest.csv"), b"name\npart-0\n").unwrap();

    let field = crate::DataType::from_fields([crate::DataType::Int64.required_field("id")])
        .unwrap()
        .required_field("row");
    let mut leaf = crate::holder::Holder::folder(&root)
        .unwrap()
        .child_by_path("part-0.arrows")
        .unwrap();
    let ipc = RecordOptions::for_mime_type(&crate::MimeType::ARROW_STREAM)
        .unwrap()
        .with_field(field.clone());
    leaf.overwrite_arrow_reader(
        crate::arrow::batch_reader(field.clone().into_arrow_schema().unwrap(), []),
        &ipc,
    )
    .unwrap();

    let folder = crate::holder::Holder::folder(&root).unwrap();
    assert!(matches!(
        folder.record_options().unwrap(),
        RecordOptions::Ipc(_)
    ));

    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_one_byte_terminator_is_quoted_like_any_other() {
    for terminator in [LineSep::NUL, LineSep::RS, LineSep::new("|").unwrap()] {
        let options = CsvOptions::new()
            .try_with_linesep(terminator.clone())
            .unwrap();
        let mut stored = Vec::from(b"symbol".as_slice());
        stored.extend_from_slice(terminator.as_bytes());
        stored.extend_from_slice(b"BRN");
        stored.extend_from_slice(terminator.as_bytes());
        let mut media = Csv::new(named("t.csv", &stored)).with_options(options.clone());

        let mut value = Vec::from(b"a".as_slice());
        value.extend_from_slice(terminator.as_bytes());
        value.extend_from_slice(b"b");
        media.write_cell_bytes(0, 0, &value).unwrap();

        assert_eq!(media.row_size().unwrap(), 1, "{terminator}");
        assert_eq!(
            media.read_cell_bytes(0, 0).unwrap().as_deref(),
            Some(value.as_slice()),
            "{terminator}"
        );
    }
}

#[test]
fn a_value_the_dialect_cannot_spell_is_refused_rather_than_written() {
    let mut options = CsvOptions::new();
    options.set_quote(None).unwrap();
    let mut media = Csv::new(named("t.csv", b"symbol\nBRN\n")).with_options(options);
    let message = media
        .write_cell_bytes(0, 0, b"a,b")
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(message.contains("no quote byte"), "{message}");
    // The refusal is the whole outcome: nothing was written.
    assert_eq!(media.handle().read_all_bytes().unwrap(), b"symbol\nBRN\n");
}

#[test]
fn a_single_column_row_survives_being_empty_or_absent() {
    let field = crate::DataType::from_fields([crate::DataType::Utf8.nullable_field("note")])
        .unwrap()
        .required_field("row");
    let schema = field.clone().into_arrow_schema().unwrap();
    let values: arrow_array::ArrayRef = std::sync::Arc::new(StringArray::from(vec![
        Some("a"),
        Some(""),
        None,
        Some("b"),
    ]));
    let batch = RecordBatch::try_new(schema.clone(), vec![values]).unwrap();

    let mut target = named("out.csv", b"");
    target
        .overwrite_arrow_reader(
            crate::arrow::batch_reader(schema, [batch]),
            &CsvOptions::new().into(),
        )
        .unwrap();
    // Four rows in, four rows out: a blank line would have been two of them.
    assert_eq!(
        crate::media::csv::row_size(&target, &CsvOptions::new()).unwrap(),
        4
    );
    // The one column has no room left to say which empty reading it holds, so
    // absence reads back as the empty string. The row itself is never lost.
    assert_eq!(
        column_text(&collect(&target, &CsvOptions::new()), "note"),
        [
            Some("a".to_owned()),
            Some(String::new()),
            Some(String::new()),
            Some("b".to_owned())
        ]
    );

    // Naming the spelling gives the column that room back, and both round trip.
    let named_null = CsvOptions::new().with_null("NULL");
    let values: arrow_array::ArrayRef = std::sync::Arc::new(StringArray::from(vec![
        Some("a"),
        Some(""),
        None,
        Some("b"),
    ]));
    let schema = field.into_arrow_schema().unwrap();
    let batch = RecordBatch::try_new(schema.clone(), vec![values]).unwrap();
    let mut spelled = named("out.csv", b"");
    spelled
        .overwrite_arrow_reader(
            crate::arrow::batch_reader(schema, [batch]),
            &named_null.clone().into(),
        )
        .unwrap();
    assert_eq!(
        spelled.read_all_bytes().unwrap(),
        b"note\na\n\"\"\nNULL\nb\n"
    );
    assert_eq!(
        column_text(&collect(&spelled, &named_null), "note"),
        [
            Some("a".to_owned()),
            Some(String::new()),
            None,
            Some("b".to_owned())
        ]
    );
}

#[test]
fn a_truncated_quoted_cell_reads_as_the_value_it_was_carrying() {
    // The resource ends inside a quoted cell: what is there is the value, not
    // the quote that opened it, and its doubled quotes still spell one quote.
    let source = named("t.csv", b"symbol,note\nBRN,\"said \"\"buy\"\" and");
    let batches = collect(&source, &CsvOptions::new());
    assert_eq!(
        column_text(&batches, "note"),
        [Some("said \"buy\" and".to_owned())]
    );
}

#[test]
fn a_configured_escape_still_reads_a_doubled_quote_as_one() {
    let options = CsvOptions::new().try_with_escape(b'\\').unwrap();
    let source = named("t.csv", b"note\n\"said \"\"buy\"\" then \\\"sold\\\"\"\n");
    let batches = collect(&source, &options);
    assert_eq!(
        column_text(&batches, "note"),
        [Some("said \"buy\" then \"sold\"".to_owned())]
    );
}

#[test]
fn trimming_never_eats_the_separator_or_one_edge_of_a_quoted_cell() {
    // A whitespace separator with trimming on: the empty cells stay cells.
    let spaced = CsvOptions::new()
        .try_with_separator(b' ')
        .unwrap()
        .with_trim(true);
    let source = named("t.csv", b"a b c\n1  3\n");
    let batches = collect(&source, &spaced);
    assert_eq!(column_text(&batches, "b"), [None]);

    // And quoting is symmetric: whitespace on either side of a quoted cell is
    // edge whitespace, not content on one side and not the other.
    let options = CsvOptions::new().with_trim(true);
    let source = named("t.csv", b"note,other\n  \"kept\"  ,x\n");
    let batches = collect(&source, &options);
    assert_eq!(column_text(&batches, "note"), [Some("kept".to_owned())]);
}

#[test]
fn a_zoned_instant_is_written_by_the_reading_that_produced_it() {
    // Inference answers a zoned column for a reading that carries an offset,
    // and Arrow's formatter cannot name that zone without a zone database. The
    // column is still writable, through this crate's own reading of it.
    let source = named("t.csv", b"seen_at\n2024-01-02T03:04:05+02:00\n");
    let options = CsvOptions::new();
    assert!(matches!(
        dtype(&source, &options, 0),
        DataType::DateTime64 { .. }
    ));
    let batches = collect(&source, &options);
    let mut target = named("out.csv", b"");
    target
        .overwrite_arrow_reader(
            crate::arrow::batch_reader(batches[0].schema(), batches),
            &options.clone().into(),
        )
        .unwrap();
    let written = target.read_all_bytes().unwrap();
    assert!(
        written.starts_with(b"seen_at\n2024-01-02T01:04:05"),
        "{}",
        String::from_utf8_lossy(&written)
    );
    // And it reads back as the same instant.
    assert_eq!(collect(&target, &options)[0].num_rows(), 1);
}

#[test]
fn a_clock_before_ten_is_a_clock_and_a_double_keeps_its_readings() {
    let clocks = named("t.csv", b"at\n09:30:00\n11:45:00\n");
    assert_eq!(
        dtype(&clocks, &CsvOptions::new(), 0),
        DataType::Time64(crate::TimeUnit::Microsecond)
    );

    // A double column round trips the readings a double has.
    let field = crate::DataType::from_fields([crate::DataType::Float64.nullable_field("v")])
        .unwrap()
        .required_field("row");
    let schema = field.into_arrow_schema().unwrap();
    let values: arrow_array::ArrayRef = std::sync::Arc::new(arrow_array::Float64Array::from(vec![
        1.5,
        f64::NAN,
        f64::INFINITY,
    ]));
    let batch = RecordBatch::try_new(schema.clone(), vec![values]).unwrap();
    let mut target = named("out.csv", b"");
    target
        .overwrite_arrow_reader(
            crate::arrow::batch_reader(schema, [batch]),
            &CsvOptions::new().into(),
        )
        .unwrap();
    let declared = crate::Field::from_str("row: struct<v: float64> not null").unwrap();
    let mut typed = CsvOptions::new();
    typed.set_field(declared);
    let read = collect(&target, &typed);
    let column = read[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow_array::Float64Array>()
        .unwrap();
    assert_eq!(column.value(0), 1.5);
    assert!(column.value(1).is_nan());
    assert!(column.value(2).is_infinite());
}

#[test]
fn a_repeated_header_name_is_made_unique_rather_than_refusing_the_resource() {
    let source = named("t.csv", b"a,a,a\n1,2,3\n");
    let field = crate::media::csv::read_field(&source, &CsvOptions::new()).unwrap();
    assert_eq!(field.get_field(0).unwrap().name(), "a");
    assert_eq!(field.get_field(1).unwrap().name(), "a_2");
    assert_eq!(field.get_field(2).unwrap().name(), "a_3");
    assert_eq!(
        column_int(&collect(&source, &CsvOptions::new()), "a_3"),
        [Some(3)]
    );
}

#[test]
fn a_declared_root_that_is_not_a_row_shape_is_refused_by_name() {
    let mut options = CsvOptions::new();
    options.set_dtype(Some(crate::DataType::Int64));
    let source = named("t.csv", b"v\n1\n");
    let message = crate::media::csv::read_field(&source, &options)
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(message.contains("struct root datatype"), "{message}");
}

#[test]
fn the_columns_are_discovered_even_when_no_row_is_sampled() {
    let options = CsvOptions::new().with_header(false).with_infer_row_size(0);
    let source = named("t.csv", b"BRN,120\n");
    let field = crate::media::csv::read_field(&source, &options).unwrap();
    assert_eq!(field.field_len(), 2);
}

#[test]
fn the_positional_surface_answers_the_column_that_was_declared() {
    let field =
        crate::Field::from_str("row: struct<id: int64, px: decimal128(10, 2)> not null").unwrap();
    let mut media = csv(b"id,px\n1,10.25\n2,11.50\n").with_field(field);

    // Read: the declared column, not the text the cells were read as.
    assert_eq!(
        media.read_cell_scalar(0, 1).unwrap(),
        Some(Scalar::d128(1_025, 2))
    );

    // Write: the caller's own declared value, rendered as that column spells it.
    media
        .write_cell_scalar(1, 1, &Scalar::d128(1_275, 2))
        .unwrap();
    assert_eq!(
        media.handle().read_all_bytes().unwrap(),
        b"id,px\n1,10.25\n2,12.75\n"
    );
    assert_eq!(
        media.read_cell_scalar(1, 1).unwrap(),
        Some(Scalar::d128(1_275, 2))
    );
}

#[test]
fn counting_rows_answers_the_same_wrapped_or_not() {
    // A ragged resource: counting records is not typing them, so both answer.
    let source = named("t.csv", b"a,b\n1,2\n3,4,5\n6,7\n");
    let options = CsvOptions::new();
    assert_eq!(crate::media::csv::row_size(&source, &options).unwrap(), 3);
    assert_eq!(source.row_size().unwrap(), 3);
    assert_eq!(Csv::new(source).row_size().unwrap(), 3);
}

#[test]
fn a_row_is_not_added_where_a_header_belongs() {
    let mut media = csv(b"");
    let message = media
        .append_row_scalar(&Scalar::from_sequence([Scalar::from("BRN")]))
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(message.contains("header"), "{message}");
    assert_eq!(media.handle().size(), 0);
}

#[test]
fn an_added_row_is_terminated_the_way_the_resource_already_is() {
    let mut media = csv(b"symbol,quantity\r\nBRN,120\r\n");
    media
        .append_row_scalar(&Scalar::from_sequence([
            Scalar::from("WTI"),
            Scalar::from(80_i64),
        ]))
        .unwrap();
    assert_eq!(
        media.handle().read_all_bytes().unwrap(),
        b"symbol,quantity\r\nBRN,120\r\nWTI,80\r\n"
    );
    assert_eq!(media.row_size().unwrap(), 2);
}
