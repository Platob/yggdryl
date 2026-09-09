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
fn absence_is_spelled_by_the_null_setting() {
    let options = CsvOptions::new().with_null("NULL");
    let source = named("t.csv", b"symbol,note\nBRN,NULL\nWTI,\n");
    let batches = collect(&source, &options);
    assert_eq!(column_text(&batches, "note"), [None, None]);

    let mut target = named("out.csv", b"");
    target
        .overwrite_arrow_reader(
            crate::arrow::batch_reader(batches[0].schema(), batches.clone()),
            &options.clone().into(),
        )
        .unwrap();
    assert_eq!(
        target.read_all_bytes().unwrap(),
        b"symbol,note\nBRN,NULL\nWTI,NULL\n"
    );
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
