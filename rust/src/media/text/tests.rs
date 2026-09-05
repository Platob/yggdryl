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
    options.with_rownum = Some(-3);
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
    assert_eq!(options.with_rownum, Some(-3));
    assert_eq!(options.batch_row_size(), Some(7));

    let error = TextOptions::new()
        .try_with_rowheader(r"(?<body>.+)")
        .unwrap_err()
        .to_string();
    assert!(error.contains("distinct from url, rownum, and body"));
}

#[test]
fn ordinary_record_reading_emits_optional_row_numbers_and_regex_typed_captures() {
    let source = named(
        "app.log",
        b"  [INFO] id=7 first  \r\n[WARN] id=9 second\nplain\r",
    );
    let mut text = options(r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)");
    text.with_rownum = Some(10);
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
        &[10, 11, 12]
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
fn capture_schema_is_derived_from_regex_before_reading() {
    let mut strings = options(r"(?<value>\d+)");
    strings.set_autotype(false);
    let field = named("empty.log", b"")
        .read_arrow_field(&strings.into())
        .unwrap();
    assert_eq!(field.field("value").unwrap().dtype(), &DataType::Utf8,);

    let typed = options(r"(?<value>\d+)");
    let field = named("empty.log", b"")
        .read_arrow_field(&typed.clone().into())
        .unwrap();
    assert_eq!(field.field("value").unwrap().dtype(), &DataType::Int64);

    let batch = named("values.log", b"1\n2\n")
        .read_arrow_reader(&typed.into())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(
        batch.schema().field(2).data_type(),
        &arrow_schema::DataType::Int64
    );

    let broad = options(r"(?<value>\S+)");
    let batch = named("values.log", b"1\nword\n")
        .read_arrow_reader(&broad.into())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(
        batch.schema().field(2).data_type(),
        &arrow_schema::DataType::Utf8
    );
    assert_eq!(
        batch
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>(),
        ["url", "body", "value"]
    );
}

#[test]
fn row_numbers_start_at_the_requested_i64_and_overflow_loudly() {
    let mut options = TextOptions::new();
    options.with_rownum = Some(i64::MAX);
    options.set_batch_row_size(Some(1));
    let mut reader = named("rows.log", b"first\nsecond\n")
        .read_arrow_reader(&options.into())
        .unwrap();

    let first = reader.next().unwrap().unwrap();
    assert_eq!(
        first
            .column(1)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .value(0),
        i64::MAX
    );
    let error = reader.next().unwrap().unwrap_err().to_string();
    assert!(error.contains("text row number exceeds i64::MAX"));
}

#[test]
fn url_column_is_rendered_from_the_handlers_real_url() {
    use crate::holder::local::{File, Folder};

    let mut path = Folder::temporary().unwrap().path().unwrap();
    path.push(format!("yggdryl-text-url-{}.log", std::process::id()));
    let mut source = File::new(&path).unwrap();
    source.remove(false).unwrap();
    source.write_all_bytes(b"body\n").unwrap();
    let expected = source.url().unwrap().to_string();

    let batch = source
        .read_arrow_reader(&TextOptions::new().into())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(
        batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        expected
    );

    source.remove(false).unwrap();
}

#[test]
fn autotyping_reads_a_session_clock_past_the_end_of_its_day() {
    // Extended session hours spell the small hours as 24 and up; the capture
    // is a time of day, so each reading folds into its day.
    let batch = named("session.log", b"08:00:00\n25:30:00\n")
        .read_arrow_reader(&options(r"(?<clock>\d{2}:\d{2}:\d{2})").into())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();

    assert_eq!(
        batch.schema().field(2).data_type(),
        &arrow_schema::DataType::Time32(arrow_schema::TimeUnit::Second)
    );
    assert_eq!(
        batch
            .column(2)
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
        batch.schema().field(2).data_type(),
        &arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, Some("UTC".into()))
    );
    assert_eq!(
        batch
            .column(1)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap()
            .value(0),
        b"Execution report (execId: 20260828180000369318, from session:"
    );
    assert_eq!(
        batch
            .column(3)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "77-2f3e6ff7:9f4d2a08b1:128"
    );
    assert_eq!(
        batch
            .column(4)
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
