use arrow_array::{Array as _, BinaryArray, Int64Array, StringArray};

use crate::holder::Buffer;
use crate::media::text::{LineSep, Text, TextOptions};
use crate::media::{IORecordOptions as _, RecordOptions};
use crate::{DataType, Timezone};
use crate::{IOBase as _, IOMedia as _};

fn named(name: &str, bytes: &[u8]) -> Buffer {
    Buffer::from_bytes(bytes.to_vec()).with_media_type(
        crate::Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    )
}

fn options(rowheader: &str) -> TextOptions {
    TextOptions::new().try_with_rowheader(rowheader).unwrap()
}

fn assert_text_buffer(_: &Text<Buffer>) {}

#[test]
fn repeated_text_conversion_reconfigures_one_wrapper() {
    let text = named("app.log", b"body\n")
        .into_text()
        .into_text()
        .into_text_with(options(r"(?<value>body)"));

    assert_text_buffer(&text);
    assert_eq!(text.options().rowheader(), Some(r"(?<value>body)"));
}

#[test]
fn options_are_flat_and_validate_rowheader_names() {
    let mut options = TextOptions::new()
        .try_with_rowheader(r"\[(?<level>[A-Z]+)\] (?<id>\d+)")
        .unwrap()
        .try_with_lstrip(r"^\s+")
        .unwrap()
        .try_with_rstrip(r"\s+$")
        .unwrap()
        .with_linesep(LineSep::CRLF)
        .with_autotype(false)
        .with_timezone(Timezone::UTC);
    options.set_batch_row_size(Some(7));

    assert_eq!(
        options.rowheader(),
        Some(r"\[(?<level>[A-Z]+)\] (?<id>\d+)")
    );
    assert_eq!(options.lstrip(), Some(r"^\s+"));
    assert_eq!(options.rstrip(), Some(r"\s+$"));
    assert_eq!(options.linesep(), Some(&LineSep::CRLF));
    assert!(!options.autotype());
    assert_eq!(options.timezone(), Some(&Timezone::UTC));
    assert_eq!(options.batch_row_size(), Some(7));

    let error = TextOptions::new()
        .try_with_rowheader(r"(?<body>.+)")
        .unwrap_err()
        .to_string();
    assert!(error.contains("distinct from url, rownum, and body"));
}

#[test]
fn ordinary_record_reading_emits_base_columns_and_adaptive_captures() {
    let source = named(
        "app.log",
        b"  [INFO] id=7 first  \r\n[WARN] id=9 second\nplain\r",
    );
    let mut text = options(r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)");
    text.set_lstrip(Some(r"^\s+")).unwrap();
    text.set_rstrip(Some(r"\s+$")).unwrap();
    let options = text.into();

    let batches = source
        .read_arrow_reader(&options)
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(batches.len(), 1);
    let batch = &batches[0];
    assert_eq!(batch.schema().field(0).name(), "url");
    assert_eq!(batch.schema().field(1).name(), "rownum");
    assert_eq!(batch.schema().field(2).name(), "body");
    assert_eq!(
        batch.schema().field(4).data_type(),
        &arrow_schema::DataType::Int64
    );
    assert_eq!(
        batch
            .column(1)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .values(),
        &[1, 2, 3]
    );
    assert_eq!(
        batch
            .column(2)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        [
            Some(&b"first"[..]),
            Some(&b"second"[..]),
            Some(&b"plain"[..])
        ]
    );
    assert_eq!(
        batch
            .column(3)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        [Some("INFO"), Some("WARN"), None]
    );
}

#[test]
fn autotype_can_be_disabled_and_is_fixed_after_the_first_batch() {
    let mut strings = options(r"(?<value>\S+)");
    strings.set_autotype(false);
    let strings = strings.into();
    let batch = named("values.log", b"1\nword\n")
        .read_arrow_reader(&strings)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(
        batch.schema().field(3).data_type(),
        &arrow_schema::DataType::Utf8
    );

    let mut adaptive = options(r"(?<value>\S+)");
    adaptive.set_batch_row_size(Some(1));
    let adaptive = adaptive.into();
    let mut reader = named("values.log", b"1\nword\n")
        .read_arrow_reader(&adaptive)
        .unwrap();
    assert!(reader.next().unwrap().is_ok());
    let error = reader.next().unwrap().unwrap_err().to_string();
    assert!(error.contains("inferred datatype int64"));
}

#[test]
fn autotyping_reads_a_session_clock_past_the_end_of_its_day() {
    // Extended session hours spell the small hours as 24 and up; the capture
    // is a time of day, so each reading folds into its day.
    let batch = named("session.log", b"08:00:00\n25:30:00\n")
        .read_arrow_reader(&options(r"(?<clock>\S+)").into())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();

    assert_eq!(
        batch.schema().field(3).data_type(),
        &arrow_schema::DataType::Time32(arrow_schema::TimeUnit::Second)
    );
    assert_eq!(
        batch
            .column(3)
            .as_any()
            .downcast_ref::<arrow_array::Time32SecondArray>()
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        [Some(28_800), Some(5_400)]
    );
}

#[test]
fn a_real_log_row_captures_a_microsecond_timestamp_and_binary_body() {
    let source = named(
        "execution.log",
        b"2026-08-29 00:00:00.434_958 [77-2f3e6ff7:9f4d2a08b1:128] \
[ModuleFailFastFilterChecker] (DEBUG) Execution report \
(execId: 20260828180000369318, from session:\n",
    );
    let text = TextOptions::new()
        .try_with_rowheader(concat!(
            r"^(?<stamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}_\d{3}) ",
            r"\[(?<thread>[^]]+)\] \[(?<module>[^]]+)\] \((?<level>[A-Z]+)\) ",
        ))
        .unwrap();
    let mut options: RecordOptions = text.into();
    options.set_timezone(Some(Timezone::UTC)).unwrap();

    let batch = source
        .read_arrow_reader(&options)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(
        batch.schema().field(3).data_type(),
        &arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, Some("UTC".into()))
    );
    assert_eq!(
        batch
            .column(2)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap()
            .value(0),
        b"Execution report (execId: 20260828180000369318, from session:"
    );
    assert_eq!(
        batch
            .column(4)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "77-2f3e6ff7:9f4d2a08b1:128"
    );
    assert_eq!(
        batch
            .column(5)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "ModuleFailFastFilterChecker"
    );
}

#[test]
fn generic_record_writes_use_only_the_binary_body() {
    let mut target = named("out.txt", b"old");
    let mut options: RecordOptions = TextOptions::new().into();
    let field = DataType::from_fields([
        DataType::Utf8.required_field("url"),
        DataType::Int64.required_field("rownum"),
        DataType::Binary.required_field("body"),
    ])
    .unwrap()
    .required_field("row");
    options.set_field(field);
    let rows = [
        crate::Scalar::from_record([
            ("url", crate::Scalar::from("input")),
            ("rownum", crate::Scalar::from(1_i64)),
            ("body", crate::Scalar::from(&b"first"[..])),
        ])
        .unwrap(),
        crate::Scalar::from_record([
            ("url", crate::Scalar::from("input")),
            ("rownum", crate::Scalar::from(2_i64)),
            ("body", crate::Scalar::from(&b"second"[..])),
        ])
        .unwrap(),
    ];
    target.overwrite_records(rows, &options).unwrap();
    assert_eq!(target.read_all_bytes().unwrap(), b"first\nsecond\n");
}
