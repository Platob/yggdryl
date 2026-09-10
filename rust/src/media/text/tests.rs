use arrow_array::{Array as _, BinaryArray, Int64Array, StringArray, UInt64Array};

use crate::holder::Buffer;
use crate::media::text::{LeadingFragment, LineSep, Text, TextOptions};
use crate::media::{IORecordOptions as _, RecordOptions};
use crate::{Codec, DataType, Field, Timezone};
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

fn framed(rowheader: &str) -> TextOptions {
    options(rowheader).with_framing(true)
}

fn collect(source: &impl crate::IOBase, options: TextOptions) -> Vec<arrow_array::RecordBatch> {
    source
        .read_arrow_reader(&options.into())
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
}

fn bodies(batches: &[arrow_array::RecordBatch]) -> Vec<Vec<u8>> {
    batches
        .iter()
        .flat_map(|batch| {
            let index = batch.schema().index_of("body").unwrap();
            batch
                .column(index)
                .as_any()
                .downcast_ref::<BinaryArray>()
                .unwrap()
                .iter()
                .map(|value| value.unwrap().to_vec())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn rownums(batches: &[arrow_array::RecordBatch]) -> Vec<i64> {
    batches
        .iter()
        .flat_map(|batch| {
            let index = batch.schema().index_of("rownum").unwrap();
            batch
                .column(index)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .values()
                .to_vec()
        })
        .collect()
}

fn dropped(batches: &[arrow_array::RecordBatch]) -> Vec<Option<u64>> {
    batches
        .iter()
        .flat_map(|batch| {
            let index = batch.schema().index_of("dropped_byte_size").unwrap();
            batch
                .column(index)
                .as_any()
                .downcast_ref::<UInt64Array>()
                .unwrap()
                .iter()
                .collect::<Vec<_>>()
        })
        .collect()
}

fn strings(batches: &[arrow_array::RecordBatch], name: &str) -> Vec<Option<String>> {
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
        .try_with_lstrip([r"^\s+"])
        .unwrap()
        .try_with_rstrip([r"\s+$"])
        .unwrap()
        .with_linesep(LineSep::CRLF)
        .with_framing(true)
        .with_leading_fragment(LeadingFragment::Drop)
        .with_max_record_byte_size(1_024)
        .with_autotype(false)
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(-3);
    options.set_batch_row_size(Some(7));

    assert_eq!(
        options.rowheader(),
        Some(r"\[(?<level>[A-Z]+)\] (?<id>\d+)")
    );
    assert_eq!(options.lstrip().collect::<Vec<_>>(), [r"^\s+"]);
    assert_eq!(options.rstrip().collect::<Vec<_>>(), [r"\s+$"]);
    assert_eq!(options.linesep(), Some(&LineSep::CRLF));
    assert!(options.framing());
    assert_eq!(options.leading_fragment(), LeadingFragment::Drop);
    assert_eq!(options.max_record_byte_size(), Some(1_024));
    assert!(!options.autotype());
    assert_eq!(options.timezone(), Some(&Timezone::UTC));
    assert_eq!(options.start_rownum, Some(-3));
    assert_eq!(options.batch_row_size(), Some(7));

    let error = TextOptions::new()
        .try_with_rowheader(r"(?<body>.+)")
        .unwrap_err()
        .to_string();
    assert!(error.contains("distinct from url, rownum, body, and dropped_byte_size"));
}

#[test]
fn ordinary_record_reading_emits_optional_row_numbers_and_regex_typed_captures() {
    let source = named(
        "app.log",
        b"  [INFO] id=7 first  \r\n[WARN] id=9 second\nplain\r",
    );
    let mut text = options(r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)");
    text.start_rownum = Some(10);
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
    assert_eq!(batch.schema().field(2).name(), "mtime");
    assert_eq!(batch.schema().field(3).name(), "body");
    assert_eq!(
        batch.schema().field(5).data_type(),
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
            .column(3)
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
            .column(4)
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
        batch.schema().field(3).data_type(),
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
        batch.schema().field(3).data_type(),
        &arrow_schema::DataType::Utf8
    );
    assert_eq!(
        batch
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>(),
        ["url", "mtime", "body", "value"]
    );
}

#[test]
fn row_numbers_start_at_the_requested_i64_and_overflow_loudly() {
    let mut options = TextOptions::new();
    options.start_rownum = Some(i64::MAX);
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
fn framing_normalizes_every_physical_terminator_and_keeps_start_rownums() {
    let source = named(
        "mixed.log",
        b"[A] first\ncontinuation one\r\n[B] second\rcontinuation two",
    );
    let mut options = framed(r"^\[(?<kind>[A-Z])\] ");
    options.start_rownum = Some(10);
    options.set_batch_row_size(Some(1));

    let batches = collect(&source, options);
    assert_eq!(batches.len(), 2);
    assert_eq!(
        bodies(&batches),
        [
            b"first\ncontinuation one".to_vec(),
            b"second\ncontinuation two".to_vec(),
        ]
    );
    assert_eq!(rownums(&batches), [10, 12]);
}

#[test]
fn framing_carries_a_record_across_input_windows_and_output_batches() {
    let mut bytes = b"[A] first\n".to_vec();
    bytes.extend(std::iter::repeat_n(
        b'x',
        crate::DEFAULT_STREAM_BATCH_SIZE + 17,
    ));
    bytes.extend_from_slice(b"\nlast continuation\n[B] next\nend\n");
    let source = named("windows.log", &bytes);
    let mut options = framed(r"^\[(?<kind>[A-Z])\] ");
    options.set_batch_row_size(Some(1));

    let batches = collect(&source, options);
    assert_eq!(batches.len(), 2);
    let bodies = bodies(&batches);
    assert_eq!(
        bodies[0].len(),
        6 + crate::DEFAULT_STREAM_BATCH_SIZE + 17 + 18
    );
    assert!(bodies[0].starts_with(b"first\nxxxxxxxx"));
    assert!(bodies[0].ends_with(b"\nlast continuation"));
    assert_eq!(bodies[1], b"next\nend");
}

#[test]
fn text_row_size_counts_logical_records_when_framing_is_enabled() {
    let source = named("count.log", b"[A] first\ncontinued\n[B] second\n");
    let text = Text::new(source).with_options(framed(r"^\[(?<kind>[A-Z])\] "));

    assert_eq!(text.row_size().unwrap(), 2);
}

#[test]
fn text_row_size_ignores_row_value_conversion_and_retains_no_bodies() {
    let source = named("count-raw.log", b"A first\ncontinued\n\xFF second\n");
    let mut options = framed(r"^(?<kind>(?-u:.)) ");
    options.start_rownum = Some(i64::MAX);
    let text = Text::new(source).with_options(options);

    // The second output row cannot be represented by the configured rownum,
    // and its capture is not UTF-8. Neither changes the number of records.
    assert_eq!(text.row_size().unwrap(), 2);
}

#[test]
fn a_result_row_limit_does_not_convert_the_following_record() {
    let source = named("limited-values.log", b"A first\n\xFF invalid\n");
    let mut options = framed(r"^(?<kind>(?-u:.)) ");
    options.start_rownum = Some(i64::MAX);
    options.set_batch_row_size(Some(8));
    options.set_max_row_size(Some(1));

    let batches = collect(&source, options);
    assert_eq!(bodies(&batches), [b"first".to_vec()]);
    assert_eq!(rownums(&batches), [i64::MAX]);
}

#[test]
fn a_physical_row_limit_does_not_convert_the_following_line() {
    let source = named("limited-lines.log", b"A first\n\xFF invalid\n");
    let mut options = options(r"^(?<kind>(?-u:.)) ");
    options.start_rownum = Some(i64::MAX);
    options.set_batch_row_size(Some(8));
    options.set_max_row_size(Some(1));

    let batches = collect(&source, options);
    assert_eq!(bodies(&batches), [b"first".to_vec()]);
    assert_eq!(rownums(&batches), [i64::MAX]);
}

#[test]
fn an_invalid_next_header_follows_the_completed_record_batch_prefix() {
    let source = named("invalid-next-header.log", b"A first\n\xFF invalid\n");
    let mut options = framed(r"^(?<kind>(?-u:.)) ");
    options.set_batch_row_size(Some(8));
    let mut reader = source.read_arrow_reader(&options.into()).unwrap();

    let prefix = reader.next().unwrap().unwrap();
    assert_eq!(bodies(&[prefix]), [b"first".to_vec()]);
    let error = reader.next().unwrap().unwrap_err().to_string();
    assert!(error.contains("UTF-8 row-header capture"), "{error}");
    assert!(reader.next().is_none());
}

#[test]
fn leading_fragments_are_kept_dropped_or_rejected_and_eof_finishes_a_record() {
    let source = named("leading.log", b"before\nstill before\n[A] final");

    let mut keep = framed(r"^\[(?<kind>[A-Z])\] ");
    keep.start_rownum = Some(1);
    let kept = collect(&source, keep);
    assert_eq!(
        bodies(&kept),
        [b"before\nstill before".to_vec(), b"final".to_vec()]
    );
    assert_eq!(rownums(&kept), [1, 3]);
    let kind = kept[0]
        .column(kept[0].schema().index_of("kind").unwrap())
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert!(kind.is_null(0));

    let dropped = collect(
        &source,
        framed(r"^\[(?<kind>[A-Z])\] ").with_leading_fragment(LeadingFragment::Drop),
    );
    assert_eq!(bodies(&dropped), [b"final".to_vec()]);

    let rejected = framed(r"^\[(?<kind>[A-Z])\] ").with_leading_fragment(LeadingFragment::Error);
    let mut reader = source.read_arrow_reader(&rejected.into()).unwrap();
    let error = reader.next().unwrap().unwrap_err().to_string();
    assert!(error.contains("leading physical line"));
    assert!(reader.next().is_none());
}

#[test]
fn record_byte_limit_reports_only_bytes_beyond_the_retained_prefix() {
    let source = named("limit.log", b"[A] abc\ndef\n[B] xyz\n");

    let exact = collect(
        &source,
        framed(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(7),
    );
    assert_eq!(bodies(&exact), [b"abc\ndef".to_vec(), b"xyz".to_vec()]);
    assert_eq!(dropped(&exact), [None, None]);

    let limited = collect(
        &source,
        framed(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(6),
    );
    assert_eq!(bodies(&limited), [b"abc\nde".to_vec(), b"xyz".to_vec()]);
    assert_eq!(dropped(&limited), [Some(1), None]);

    let zero = collect(
        &source,
        framed(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(0),
    );
    assert_eq!(bodies(&zero), [Vec::<u8>::new(), Vec::new()]);
    assert_eq!(dropped(&zero), [Some(7), Some(3)]);
}

#[test]
fn oversized_continuations_are_drained_before_the_following_record() {
    let oversized = crate::DEFAULT_STREAM_BATCH_SIZE * 8 + 31;
    let mut bytes = b"[A] begin\n".to_vec();
    bytes.extend(std::iter::repeat_n(b'x', oversized));
    bytes.extend_from_slice(b"\n[B] after\n");
    let source = named("oversized.log", &bytes);
    let options = framed(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(8);

    let batches = collect(&source, options);
    assert_eq!(bodies(&batches), [b"begin\nxx".to_vec(), b"after".to_vec()]);
    assert_eq!(
        dropped(&batches),
        [Some(u64::try_from(oversized - 2).unwrap()), None]
    );
}

#[test]
fn an_oversized_matching_line_retains_only_its_body_prefix() {
    let oversized = crate::DEFAULT_STREAM_BATCH_SIZE * 8 + 31;
    let mut bytes = b"[A] ".to_vec();
    bytes.extend(std::iter::repeat_n(b'x', oversized));
    bytes.extend_from_slice(b"\n[B] after\n");
    let source = named("oversized-header-line.log", &bytes);
    let options = framed(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(8);

    let batches = collect(&source, options);
    assert_eq!(bodies(&batches), [b"xxxxxxxx".to_vec(), b"after".to_vec()]);
    assert_eq!(
        dropped(&batches),
        [Some(u64::try_from(oversized - 8).unwrap()), None]
    );
}

#[test]
fn capped_header_scanning_matches_complete_regex_semantics() {
    let cases: [(&str, &[u8]); 6] = [
        (r"^(?<h>a+)", b"aaaa body\ncontinued"),
        (r"^(?<h>a+?)", b"aaaa body\ncontinued"),
        (r"^(?<h>ab|a)", b"ab body\ncontinued"),
        (r"^(?<h>a+)$", b"aaaa\ncontinued"),
        (r"(?<h>HDR+)", b"prefix HDRRR suffix\ncontinued"),
        (r"^(?<h>a)?START", b"START body\ncontinued"),
    ];
    for (rowheader, source) in cases {
        let source = named("regex-equivalence.log", source);
        let uncapped = collect(&source, framed(rowheader));
        let capped = collect(&source, framed(rowheader).with_max_record_byte_size(1_024));
        assert_eq!(bodies(&capped), bodies(&uncapped), "{rowheader}");
        assert_eq!(
            strings(&capped, "h"),
            strings(&uncapped, "h"),
            "{rowheader}"
        );
        assert!(dropped(&capped).iter().all(Option::is_none), "{rowheader}");
    }

    let header_size = crate::DEFAULT_STREAM_BATCH_SIZE - 1;
    let mut bytes = vec![b'H'; header_size];
    bytes.extend_from_slice(b" body\ncontinued");
    let source = named("window-header.log", &bytes);
    let uncapped = collect(&source, framed(r"^(?<h>H+) "));
    let capped = collect(
        &source,
        framed(r"^(?<h>H+) ").with_max_record_byte_size(1_024),
    );
    assert_eq!(bodies(&capped), bodies(&uncapped));
    assert_eq!(strings(&capped, "h"), strings(&uncapped, "h"));

    let schema = capped[0].schema();
    assert!(schema.field_with_name("h").unwrap().is_nullable());
    assert!(
        schema
            .field_with_name("dropped_byte_size")
            .unwrap()
            .is_nullable()
    );
}

#[test]
fn gzip_and_zstd_framing_decode_the_same_logical_records() {
    let decoded = b"[A] first\ncontinued\n[B] second\n";
    for (name, codec) in [
        ("records.log.gz", Codec::Gzip),
        ("records.log.zst", Codec::Zstd),
    ] {
        let source = named(name, &codec.dump(decoded).unwrap());
        let batches = collect(&source, framed(r"^\[(?<kind>[A-Z])\] "));
        assert_eq!(
            bodies(&batches),
            [b"first\ncontinued".to_vec(), b"second".to_vec()]
        );
    }
}

#[test]
fn framed_schema_is_complete_before_empty_or_absent_input_is_pulled() {
    use crate::holder::local::File;

    let options = framed(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(10);
    let record_options: RecordOptions = options.clone().into();
    let empty = named("empty.log.gz", &Codec::Gzip.dump(b"").unwrap());
    let reader = empty.read_arrow_reader(&record_options).unwrap();
    assert_eq!(
        reader
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>(),
        ["url", "mtime", "body", "dropped_byte_size", "kind"]
    );
    assert!(
        reader
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
            .is_empty()
    );

    for suffix in ["log.gz", "log.zst"] {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "yggdryl-absent-framed-schema-{}-{}.{}",
            std::process::id(),
            crate::stable_hash_of(&path),
            suffix
        ));
        let mut absent = File::new(&path).unwrap();
        absent.remove(false).unwrap();
        let reader = absent.read_arrow_reader(&options.clone().into()).unwrap();
        assert_eq!(
            reader
                .schema()
                .fields()
                .iter()
                .map(|field| field.name().as_str())
                .collect::<Vec<_>>(),
            ["url", "mtime", "body", "dropped_byte_size", "kind"]
        );
        assert!(
            reader
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            Text::new(absent)
                .with_options(options.clone())
                .row_size()
                .unwrap(),
            0
        );
    }
}

#[test]
fn framing_requires_a_rowheader_before_any_source_read() {
    let source = named("unused.log", b"body\n");
    let options: RecordOptions = TextOptions::new().with_framing(true).into();
    let error = source
        .read_arrow_reader(&options)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("framing requires a rowheader"));
}

#[test]
fn folder_leaves_never_share_framing_state_and_restart_physical_rownums() {
    use crate::holder::local::Folder;

    let mut root = Folder::temporary().unwrap().path().unwrap();
    root.push(format!("yggdryl-framed-folder-{}", std::process::id()));
    let mut folder = Folder::new(&root).unwrap();
    folder.remove(true).unwrap();
    let mut first = folder.child_by_path("a.log").unwrap();
    first.write_all_bytes(b"[A] first\ncontinued in a").unwrap();
    let mut second = folder.child_by_path("b.log").unwrap();
    second.write_all_bytes(b"leading in b\n[B] second").unwrap();

    let mut options = framed(r"^\[(?<kind>[A-Z])\] ");
    options.start_rownum = Some(1);
    let batches = collect(&folder, options);
    assert_eq!(
        bodies(&batches),
        [
            b"first\ncontinued in a".to_vec(),
            b"leading in b".to_vec(),
            b"second".to_vec(),
        ]
    );
    assert_eq!(rownums(&batches), [1, 1, 2]);

    folder.remove(true).unwrap();
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

#[test]
fn a_strip_sequence_takes_one_layer_off_at_a_time() {
    // A capture line routinely carries several layers of prose in front of
    // its payload, and one expression matching all of them at once is the
    // expression nobody can read. Each pattern strips from the edge the one
    // before it left.
    let cases: &[(&[&str], &[u8], &[u8])] = &[
        // An arrow alone.
        (&[r"^-->\s*"], b"--> 8=FIX.4.4|35=D|", b"8=FIX.4.4|35=D|"),
        // A stage label, then an arrow.
        (
            &[r"^After \w+\s*", r"^-->\s*"],
            b"After Enrichment --> ACCOUNT=A1|SIDE=1",
            b"ACCOUNT=A1|SIDE=1",
        ),
        // The same two in the other spelling: a label ending in a colon.
        (
            &[r"^After [\w ]+:\s*", r"^-->\s*"],
            b"After the bridge: --> ACCOUNT=A1",
            b"ACCOUNT=A1",
        ),
        // A timestamp, a level, a plugin name, then the arrow.
        (
            &[
                r"^\d{4}-\d{2}-\d{2} ",
                r"^[A-Z]+ ",
                r"^\[\w+\]\s*",
                r"^-->\s*",
            ],
            b"2026-09-04 INFO [XmlApi] --> 8=FIX.4.2|35=8|",
            b"8=FIX.4.2|35=8|",
        ),
        // A pattern that matches nothing leaves the edge where it was, so a
        // sequence written for the general case still reads the specific one.
        (
            &[r"^After \w+\s*", r"^-->\s*"],
            b"--> ACCOUNT=A1",
            b"ACCOUNT=A1",
        ),
        // A pattern that does not match leaves the edge for the next one, so
        // the sequence is tried in order rather than abandoned at the first
        // miss - which is what lets one listing serve every shape a capture
        // mixes. Here the arrow does not lead, so only the label comes off.
        (
            &[r"^-->\s*", r"^After \w+\s*"],
            b"After Enrichment --> ACCOUNT=A1",
            b"--> ACCOUNT=A1",
        ),
    ];

    for (patterns, line, expected) in cases {
        let options = TextOptions::new()
            .try_with_lstrip(patterns.iter().copied())
            .unwrap();
        let source = Buffer::from_bytes([*line, b"\n"].concat());
        assert_eq!(
            bodies(&collect(&source, options)),
            [expected.to_vec()],
            "{patterns:?}"
        );
    }
}

#[test]
fn a_right_edge_sequence_strips_in_order_too() {
    let options = TextOptions::new()
        .try_with_rstrip([r"\s+$", r"<< queued seq=\d+$", r"\s+$"])
        .unwrap();
    let source = Buffer::from_bytes(b"8=FIX.4.2|35=D|10=203| << queued seq=1092  \n".to_vec());
    assert_eq!(
        bodies(&collect(&source, options)),
        [b"8=FIX.4.2|35=D|10=203|".to_vec()]
    );
}

#[test]
fn the_classification_columns_read_the_line_and_the_direction_leaves_the_body() {
    let source = Buffer::from_bytes(
        [
            b"sending >> 8=FIX.4.2|9=176|35=D|10=203|
"
            .as_slice(),
            b"recv ACCOUNT=A1|MSGTYPE=8|SYMBOL=AAPL
"
            .as_slice(),
            b"level=INFO worker=3 took=12ms
"
            .as_slice(),
            b"<Order ClOrdID='XML-1'/>
"
            .as_slice(),
            b"no level printed by this plugin
"
            .as_slice(),
        ]
        .concat(),
    );
    let mut options = TextOptions::new();
    options.parse_mimetype = true;
    options.parse_direction = true;

    // The columns a classifying read declares, in order.
    let field = options.source_field().unwrap();
    let names: Vec<&str> = field
        .dtype()
        .as_fields()
        .unwrap()
        .iter()
        .map(Field::name)
        .collect();
    assert_eq!(names, ["url", "mtime", "direction", "mimetype", "body"]);

    let batches = collect(&source, options);
    assert_eq!(
        codes(&batches, "direction"),
        [Some("SENT"), Some("RECV"), None, None, None]
    );
    assert_eq!(
        texts(&batches, "mimetype"),
        [
            Some("text/fix"),
            Some("text/ullink"),
            Some("text/key-value"),
            Some("application/xml"),
            Some("application/octet-stream"),
        ]
    );
    // Reading the verb takes exactly the verb off the body: a body that kept
    // it would carry a word no protocol sent. What it does *not* take is the
    // rest of the transport's punctuation - the arrow here - because that was
    // not used to read anything, and removing it is what an lstrip sequence
    // is for.
    assert_eq!(
        bodies(&batches)[..2],
        [
            b">> 8=FIX.4.2|9=176|35=D|10=203|".to_vec(),
            b"ACCOUNT=A1|MSGTYPE=8|SYMBOL=AAPL".to_vec(),
        ]
    );
    assert_eq!(
        bodies(&batches)[4],
        b"no level printed by this plugin".to_vec()
    );
}

/// One packed-ASCII column, read back with its padding gone.
fn codes<'a>(batches: &'a [arrow_array::RecordBatch], name: &str) -> Vec<Option<&'a str>> {
    batches
        .iter()
        .flat_map(|batch| {
            let index = batch.schema().index_of(name).unwrap();
            batch
                .column(index)
                .as_any()
                .downcast_ref::<arrow_array::FixedSizeBinaryArray>()
                .unwrap()
                .iter()
                .map(|value| {
                    value.map(|bytes| std::str::from_utf8(bytes).unwrap().trim_end_matches(' '))
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// One text column.
fn texts<'a>(batches: &'a [arrow_array::RecordBatch], name: &str) -> Vec<Option<&'a str>> {
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
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn adjacent_rows_repeating_a_body_are_dropped_only_when_asked() {
    // `A A B A`: the trailing `A` survives, which is what proves this is one
    // previous digest rather than a set of every body seen.
    let source = Buffer::from_bytes(
        b"8=FIX.4.4|35=D|10=1|\n8=FIX.4.4|35=D|10=1|\n8=FIX.4.4|35=8|10=2|\n8=FIX.4.4|35=D|10=1|\n"
            .to_vec(),
    );

    // Off by default: a row in is a row out.
    assert_eq!(bodies(&collect(&source, TextOptions::new())).len(), 4);

    let mut options = TextOptions::new();
    options.dedup_adjacent = true;
    assert_eq!(
        bodies(&collect(&source, options)),
        [
            b"8=FIX.4.4|35=D|10=1|".to_vec(),
            b"8=FIX.4.4|35=8|10=2|".to_vec(),
            b"8=FIX.4.4|35=D|10=1|".to_vec(),
        ]
    );

    // An empty source drops nothing, and a single row is never its own
    // predecessor.
    let single = Buffer::from_bytes(b"8=FIX.4.4|35=D|10=1|\n".to_vec());
    let mut options = TextOptions::new();
    options.dedup_adjacent = true;
    assert_eq!(bodies(&collect(&single, options)).len(), 1);

    // The digest is of the body after stripping, so two rows differing only
    // in prose a strip removes are one row.
    let prefixed = Buffer::from_bytes(
        b"sending >> 8=FIX.4.4|35=D|10=1|\nrecv >> 8=FIX.4.4|35=D|10=1|\n".to_vec(),
    );
    let mut options = TextOptions::new().try_with_lstrip([r"^>>\s*"]).unwrap();
    options.parse_direction = true;
    options.dedup_adjacent = true;
    assert_eq!(bodies(&collect(&prefixed, options)).len(), 1);
}

/// What one text read costs the store underneath it.
///
/// A record read decodes through one sequential open, so the question a remote
/// store cares about is how many times that open is asked for bytes. The
/// decoder pulls in its own small increments - gzip reads 32 KiB at a time -
/// so without a fetch window between them a gigabyte-scale object would cost
/// tens of thousands of round trips for bytes it is going to read in order
/// anyway.
mod fetching {
    use std::any::Any;
    use std::sync::{Arc, Mutex};

    use crate::holder::fs::{
        BoundLocation, ByteReader, ByteWriter, FileInfo, FileInfos, FileSelector, FileSystem,
        MemoryFileSystem, OutputMetadata, RandomAccessReader,
    };
    use crate::media::text::{Text, TextOptions};
    use crate::{Codec, DEFAULT_FETCH_BYTE_SIZE, IOBase as _, IOMedia as _, Url};

    const ROWHEADER: &str = r"^\[(?<level>[A-Z]+)\] ";

    /// One filesystem that records the size of every read its streams serve.
    struct Counting {
        inner: MemoryFileSystem,
        reads: Arc<Mutex<Vec<usize>>>,
        opens: Arc<Mutex<usize>>,
    }

    struct CountingReader {
        inner: Box<dyn ByteReader>,
        reads: Arc<Mutex<Vec<usize>>>,
    }

    impl ByteReader for CountingReader {
        fn read(&mut self, buffer: &mut [u8]) -> crate::Result<usize> {
            self.reads.lock().unwrap().push(buffer.len());
            self.inner.read(buffer)
        }

        fn tell(&self) -> u64 {
            self.inner.tell()
        }

        fn close(&mut self) -> crate::Result<()> {
            self.inner.close()
        }

        fn closed(&self) -> bool {
            self.inner.closed()
        }

        fn as_any(&self) -> &dyn Any {
            self
        }

        fn into_any(self: Box<Self>) -> Box<dyn Any> {
            self
        }
    }

    impl FileSystem for Counting {
        fn type_name(&self) -> &str {
            "counting"
        }

        fn equals(&self, other: &dyn FileSystem) -> bool {
            other.as_any().downcast_ref::<Self>().is_some()
        }

        fn normalize_path(&self, path: &str) -> crate::Result<String> {
            self.inner.normalize_path(path)
        }

        fn file_info(&self, path: &str) -> crate::Result<FileInfo> {
            self.inner.file_info(path)
        }

        fn list(&self, selector: &FileSelector) -> FileInfos {
            self.inner.list(selector)
        }

        fn create_dir(&self, path: &str, recursive: bool) -> crate::Result<()> {
            self.inner.create_dir(path, recursive)
        }

        fn delete_dir(&self, path: &str) -> crate::Result<()> {
            self.inner.delete_dir(path)
        }

        fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> crate::Result<()> {
            self.inner.delete_dir_contents(path, missing_dir_ok)
        }

        fn delete_root_dir_contents(&self) -> crate::Result<()> {
            self.inner.delete_root_dir_contents()
        }

        fn delete_file(&self, path: &str) -> crate::Result<()> {
            self.inner.delete_file(path)
        }

        fn copy_file(&self, source: &str, target: &str) -> crate::Result<()> {
            self.inner.copy_file(source, target)
        }

        fn move_file(&self, source: &str, target: &str) -> crate::Result<()> {
            self.inner.move_file(source, target)
        }

        fn open_input_file(&self, path: &str) -> crate::Result<Box<dyn RandomAccessReader>> {
            self.inner.open_input_file(path)
        }

        fn open_input_stream(&self, path: &str) -> crate::Result<Box<dyn ByteReader>> {
            *self.opens.lock().unwrap() += 1;
            Ok(Box::new(CountingReader {
                inner: self.inner.open_input_stream(path)?,
                reads: Arc::clone(&self.reads),
            }))
        }

        fn open_output_stream(
            &self,
            path: &str,
            metadata: Option<&OutputMetadata>,
        ) -> crate::Result<Box<dyn ByteWriter>> {
            self.inner.open_output_stream(path, metadata)
        }

        fn open_append_stream(
            &self,
            path: &str,
            metadata: Option<&OutputMetadata>,
        ) -> crate::Result<Box<dyn ByteWriter>> {
            self.inner.open_append_stream(path, metadata)
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    /// Text that gzip cannot shrink away, so the encoded object spans several
    /// fetch windows and records straddle every boundary between them.
    fn payload(lines: usize) -> (Vec<u8>, Vec<Vec<u8>>) {
        const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let mut state = 0x2545_F491_u32;
        let mut plain = Vec::new();
        let mut bodies = Vec::new();
        for index in 0..lines {
            let mut body = format!("id={index} ").into_bytes();
            for _ in 0..1_024 {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                body.push(ALPHABET[state as usize % ALPHABET.len()]);
            }
            plain.extend_from_slice(b"[INFO] ");
            plain.extend_from_slice(&body);
            plain.push(b'\n');
            bodies.push(body);
        }
        (plain, bodies)
    }

    fn framed() -> TextOptions {
        TextOptions::new()
            .try_with_rowheader(ROWHEADER)
            .unwrap()
            .with_framing(true)
    }

    fn located(name: &str, bytes: &[u8]) -> (crate::holder::fs::File, Arc<Counting>) {
        let filesystem = Arc::new(Counting {
            inner: MemoryFileSystem::new(),
            reads: Arc::new(Mutex::new(Vec::new())),
            opens: Arc::new(Mutex::new(0)),
        });
        filesystem
            .inner
            .open_output_stream(name, None)
            .unwrap()
            .write(bytes)
            .unwrap();
        let bound =
            BoundLocation::new(Arc::clone(&filesystem) as Arc<dyn FileSystem>, name, None).unwrap();
        let mut handle = crate::holder::fs::File::new(bound);
        handle.set_media_type(
            Url::from_str(&format!("file:///{name}"))
                .unwrap()
                .media_type(),
        );
        (handle, filesystem)
    }

    #[test]
    fn a_compressed_leaf_streams_one_open_in_whole_fetch_windows() {
        let (plain, expected) = payload(3_000);
        let encoded = Codec::Gzip.dump(&plain).unwrap();
        assert!(
            encoded.len() > 2 * DEFAULT_FETCH_BYTE_SIZE,
            "the object must span several windows, got {} bytes",
            encoded.len()
        );
        let (handle, filesystem) = located("app.log.gz", &encoded);

        let read = super::collect(&handle, framed());
        assert_eq!(super::bodies(&read), expected);

        // One open, and every ask but the last is a whole window: the decoder's
        // own 32 KiB appetite never reaches the store.
        assert_eq!(*filesystem.opens.lock().unwrap(), 1);
        let reads = filesystem.reads.lock().unwrap().clone();
        assert!(
            reads.iter().all(|size| *size == DEFAULT_FETCH_BYTE_SIZE),
            "every fetch asks for a whole window, got {reads:?}"
        );
        let windows = encoded.len().div_ceil(DEFAULT_FETCH_BYTE_SIZE);
        assert!(
            (windows..=windows + 1).contains(&reads.len()),
            "one fetch per window of {} encoded bytes, at most one more to see              the end, got {} fetches",
            encoded.len(),
            reads.len()
        );

        // Counting the rows, rather than materializing them, reads the same
        // way: one open, whole windows.
        filesystem.reads.lock().unwrap().clear();
        *filesystem.opens.lock().unwrap() = 0;
        assert_eq!(
            Text::new(handle).with_options(framed()).row_size().unwrap(),
            3_000
        );
        assert_eq!(*filesystem.opens.lock().unwrap(), 1);
        // Counting asks for a window at a time as well; the shorter final ask
        // is the tail of the last window, not a small request of its own.
        let counted = filesystem.reads.lock().unwrap().clone();
        assert_eq!(counted.first(), Some(&DEFAULT_FETCH_BYTE_SIZE));
        assert!(
            counted.iter().all(|size| *size <= DEFAULT_FETCH_BYTE_SIZE),
            "counting rows never asks beyond one window, got {counted:?}"
        );
        assert!(
            counted.len() <= windows + 1,
            "counting rows fetches one window at a time, got {counted:?}"
        );
    }

    #[test]
    fn an_empty_compressed_leaf_is_not_probed_a_byte_at_a_time() {
        let (handle, filesystem) = located("empty.log.gz", &[]);

        assert_eq!(
            handle.read_arrow_reader(&framed().into()).unwrap().count(),
            0
        );
        assert_eq!(*filesystem.opens.lock().unwrap(), 1);
        assert_eq!(
            filesystem.reads.lock().unwrap().as_slice(),
            [DEFAULT_FETCH_BYTE_SIZE],
            "emptiness is answered by the first window, not by a one-byte read"
        );
    }
}

#[test]
fn the_mtime_column_prefers_the_header_capture_over_the_handles_own_time() {
    use arrow_array::TimestampNanosecondArray;

    // The expression dates the line, so the column is the line's own reading
    // resolved into UTC - not the moment the file happened to be written.
    let source = named("dated.log", b"2020-01-02T03:04:05Z id=7 first\n");
    let batch = collect(&source, options(r"^(?<mtime>\S+) id=(?<id>\d+) "))
        .pop()
        .unwrap();
    let schema = batch.schema();
    let names: Vec<&str> = schema
        .fields()
        .iter()
        .map(|field| field.name().as_str())
        .collect();
    // One column, not two: the capture fills `mtime` rather than sitting
    // beside it under the same name.
    assert_eq!(names, ["url", "mtime", "body", "id"]);
    assert_eq!(
        batch.schema().field(1).data_type(),
        &arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Nanosecond, Some("UTC".into()))
    );
    assert_eq!(
        batch
            .column(1)
            .as_any()
            .downcast_ref::<TimestampNanosecondArray>()
            .unwrap()
            .value(0),
        1_577_934_245_000_000_000
    );
}

#[test]
fn a_handle_with_no_modification_time_leaves_the_mtime_column_null() {
    use arrow_array::Array as _;

    // A buffer records no such fact, and the reader says so rather than
    // inventing a clock reading.
    let batch = collect(&named("plain.log", b"first\nsecond\n"), TextOptions::new())
        .pop()
        .unwrap();
    let mtime = batch.column_by_name("mtime").unwrap();
    assert_eq!(mtime.len(), 2);
    assert_eq!(mtime.null_count(), 2);
}

#[test]
fn the_mtime_column_falls_back_to_the_handles_own_modification_time() {
    use crate::holder::local::File;
    use arrow_array::TimestampNanosecondArray;

    let directory = std::env::temp_dir().join("yggdryl_text_mtime");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("undated.log");
    std::fs::write(&path, b"first\nsecond\n").unwrap();
    let handle = File::new(&path).unwrap();
    let expected = handle.mtime().expect("a filesystem modification time");

    let batch = collect(&handle, TextOptions::new()).pop().unwrap();
    let values = batch
        .column_by_name("mtime")
        .unwrap()
        .as_any()
        .downcast_ref::<TimestampNanosecondArray>()
        .unwrap();
    // Every row shares the handle's answer: one fact about the object, read
    // once and repeated, never one stat per line.
    assert_eq!(values.values(), &[expected, expected]);

    std::fs::remove_file(&path).ok();
}

#[test]
fn the_mtime_column_is_off_when_the_flag_is_and_frees_its_name_for_a_capture() {
    let mut plain = TextOptions::new();
    plain.parse_mtime = false;
    let batch = collect(&named("plain.log", b"first\n"), plain)
        .pop()
        .unwrap();
    assert!(batch.column_by_name("mtime").is_none());

    // With no column of that name, a capture spelled `mtime` is an ordinary
    // one, typed by its own syntax rather than by the column it no longer
    // fills.
    let mut captured = options(r"^(?<mtime>\d+) ");
    captured.parse_mtime = false;
    let batch = collect(&named("counted.log", b"77 first\n"), captured)
        .pop()
        .unwrap();
    assert_eq!(
        batch.schema().field(2).data_type(),
        &arrow_schema::DataType::Int64
    );
}

#[test]
fn captures_are_typed_by_name_whatever_fixed_columns_precede_them() {
    // Every fixed column is optional, so a capture's datatype cannot be found
    // by counting the ones in front of it: with the classification columns on,
    // that count was wrong and a capture was parsed at another column's type.
    let mut options = options(r"^(?<seen>\d{4}-\d{2}-\d{2}) id=(?<id>\d+) ");
    options.parse_mimetype = true;
    options.parse_direction = true;
    let batch = collect(&named("wide.log", b"2020-01-02 id=7 first\n"), options)
        .pop()
        .unwrap();
    assert_eq!(
        batch.schema().field_with_name("seen").unwrap().data_type(),
        &arrow_schema::DataType::Date32
    );
    assert_eq!(
        batch.schema().field_with_name("id").unwrap().data_type(),
        &arrow_schema::DataType::Int64
    );
}

// --- The decoded row value and its parts ---

mod values {
    use std::sync::Arc;

    use crate::media::text::{TextBytes, TextEntries, TextEntry, TextLine};
    use crate::{FieldPath, FieldSegment};

    fn page(bytes: &[u8]) -> Arc<Vec<u8>> {
        Arc::new(bytes.to_vec())
    }

    fn path(text: &str) -> FieldPath {
        FieldPath::from_str(text).expect("path parses")
    }

    fn entry(key: &str, value: &str) -> TextEntry {
        TextEntry::new(
            TextBytes::from_bytes(key).expect("a key"),
            TextBytes::from_bytes(value).expect("a value"),
        )
    }

    #[test]
    fn a_range_borrows_its_page_and_copies_nothing() {
        let page = page(b"alpha beta gamma");
        let middle = TextBytes::from_page(&page, 6, 10).expect("inside the page");
        assert_eq!(middle.as_bytes(), b"beta");
        assert_eq!(middle.len(), 4);
        assert_eq!(middle.start(), 6);
        assert_eq!(middle.end(), 10);
        // The range points into the very page it was taken from.
        assert!(Arc::ptr_eq(middle.page().expect("a page"), &page));
    }

    #[test]
    fn an_empty_range_retains_no_page() {
        let page = page(b"alpha");
        let empty = TextBytes::from_page(&page, 2, 2).expect("an empty range");
        assert!(empty.is_empty());
        assert_eq!(empty.as_bytes(), b"");
        assert!(empty.page().is_none(), "an empty range pins nothing");
        assert_eq!(Arc::strong_count(&page), 1);
    }

    #[test]
    fn a_range_outside_its_page_is_refused() {
        let page = page(b"alpha");
        assert!(TextBytes::from_page(&page, 0, 6).is_err());
        assert!(TextBytes::from_page(&page, 4, 2).is_err());
        assert!(TextBytes::from_page(&page, 0, 5).is_ok());
    }

    #[test]
    fn identity_is_the_bytes_and_never_the_page() {
        let left = TextBytes::from_page(&page(b"xxbetayy"), 2, 6).expect("inside");
        let right = TextBytes::from_page(&page(b"beta"), 0, 4).expect("inside");
        assert_eq!(left, right, "same bytes, different pages");
        assert_eq!(
            crate::stable_hash_of(&left),
            crate::stable_hash_of(&right),
            "equal values must hash alike"
        );
        assert!(left <= right && right <= left);
    }

    #[test]
    fn slicing_stays_inside_the_same_page() {
        let page = page(b"alpha beta gamma");
        let tail = TextBytes::from_page(&page, 6, 16).expect("inside");
        let inner = tail.slice(0, 4).expect("inside the range");
        assert_eq!(inner.as_bytes(), b"beta");
        assert!(Arc::ptr_eq(inner.page().expect("a page"), &page));
        assert!(tail.slice(0, 99).is_err());
    }

    #[test]
    fn an_entry_tree_is_found_by_path_at_every_depth() {
        let nested = TextEntries::from_iter([entry("PartyID", "ACME")]);
        let entries = TextEntries::from_iter([
            entry("35", "D"),
            entry("55", "AAPL"),
            TextEntry::new(
                TextBytes::from_bytes("213").expect("a key"),
                TextBytes::new(),
            )
            .with_entries(nested),
        ]);
        let line =
            TextLine::new(0, TextBytes::from_bytes("body").expect("a body")).with_entries(entries);

        assert_eq!(
            line.get_entry_by_path(&path("55"))
                .and_then(|held| held.value().as_str()),
            Some("AAPL")
        );
        assert_eq!(
            line.get_entry_by_path(&path("\"213\".PartyID"))
                .and_then(|held| held.value().as_str()),
            Some("ACME")
        );
        assert_eq!(
            line.get_entry_by_path(&path("[1]"))
                .and_then(|held| held.key().as_str()),
            Some("55")
        );
        assert_eq!(
            line.get_entry_by_path(&path("[-1]"))
                .and_then(|held| held.key().as_str()),
            Some("213")
        );
    }

    #[test]
    fn a_miss_is_null_and_never_an_error_or_a_panic() {
        let line = TextLine::new(0, TextBytes::new())
            .with_entries(TextEntries::from_iter([entry("a", "1")]));
        for text in ["b", "a.b", "a.b.c", "[9]", "[-9]"] {
            assert!(
                line.get_entry_by_path(&path(text)).is_none(),
                "{text} must miss"
            );
        }
        // A line carrying no tree at all misses the same way.
        let bare = TextLine::new(0, TextBytes::new());
        assert!(bare.get_entry_by_path(&path("a")).is_none());
        // The raising form says why, and names the path.
        let error = bare.entry_by_path(&path("a")).expect_err("raises");
        assert!(error.to_string().contains('a'), "{error}");
    }

    #[test]
    fn a_key_with_no_value_is_found_and_is_not_a_miss() {
        let line = TextLine::new(0, TextBytes::new())
            .with_entries(TextEntries::from_iter([entry("a", "")]));
        let found = line
            .get_entry_by_path(&path("a"))
            .expect("the key is there");
        assert!(found.value().is_empty());
    }

    #[test]
    fn the_setter_creates_what_is_not_there() {
        let mut line = TextLine::new(0, TextBytes::new());
        line.set_entry_by_path(
            &path("order.price"),
            TextBytes::from_bytes("12").expect("a value"),
        )
        .expect("creates both levels");
        assert_eq!(
            line.get_entry_by_path(&path("order.price"))
                .and_then(|held| held.value().as_str()),
            Some("12")
        );
        // Setting again replaces rather than appending a second entry.
        line.set_entry_by_path(
            &path("order.price"),
            TextBytes::from_bytes("13").expect("a value"),
        )
        .expect("replaces");
        assert_eq!(
            line.entries().map(TextEntries::len),
            Some(1),
            "one root entry"
        );
        assert_eq!(
            line.get_entry_by_path(&path("order.price"))
                .and_then(|held| held.value().as_str()),
            Some("13")
        );
    }

    #[test]
    fn a_position_naming_no_entry_refuses_and_changes_nothing() {
        let mut line = TextLine::new(0, TextBytes::new())
            .with_entries(TextEntries::from_iter([entry("a", "1")]));
        let before = line.clone();
        let error = line
            .set_entry_by_path(&path("[4]"), TextBytes::from_bytes("x").expect("a value"))
            .expect_err("a position names an entry that exists");
        assert!(error.to_string().contains("position"), "{error}");
        assert_eq!(line, before, "a refusal leaves the line unchanged");
    }

    #[test]
    fn the_root_path_is_not_a_place_to_set() {
        let mut line = TextLine::new(0, TextBytes::new());
        assert!(
            line.set_entry_by_path(&FieldPath::root(), TextBytes::new())
                .is_err()
        );
    }

    #[test]
    fn removing_takes_one_entry_at_the_path() {
        let mut line = TextLine::new(0, TextBytes::new())
            .with_entries(TextEntries::from_iter([entry("a", "1"), entry("b", "2")]));
        let removed = line.remove_entry_by_path(&path("a")).expect("removes");
        assert_eq!(removed.key().as_str(), Some("a"));
        assert_eq!(line.entries().map(TextEntries::len), Some(1));
        assert!(line.remove_entry_by_path(&path("a")).is_none());
    }

    #[test]
    fn a_repeated_key_is_two_entries_reachable_by_position() {
        let line = TextLine::new(0, TextBytes::new()).with_entries(TextEntries::from_iter([
            entry("tag", "first"),
            entry("tag", "second"),
        ]));
        assert_eq!(
            line.get_entry_by_path(&path("tag"))
                .and_then(|held| held.value().as_str()),
            Some("first"),
            "a name reaches the first"
        );
        assert_eq!(
            line.get_entry_by_path(&path("[1]"))
                .and_then(|held| held.value().as_str()),
            Some("second")
        );
    }

    #[test]
    fn a_line_carries_what_no_other_field_can_recover() {
        let mut line = TextLine::new(7, TextBytes::from_bytes("hello").expect("a body"));
        assert_eq!(line.index(), 7);
        line.set_timestamp(Some(1_700_000_000_000_000_000));
        line.set_dropped_byte_size(Some(12));
        line.set_direction(Some("out"));
        assert_eq!(line.timestamp(), Some(1_700_000_000_000_000_000));
        assert_eq!(line.dropped_byte_size(), Some(12));
        assert_eq!(line.direction(), Some("out"));
        assert_eq!(line.body().as_str(), Some("hello"));
    }

    #[test]
    fn a_path_segment_naming_nothing_addressable_misses_rather_than_panics() {
        let line = TextLine::new(0, TextBytes::new())
            .with_entries(TextEntries::from_iter([entry("a", "1")]));
        let by_index = FieldPath::new([FieldSegment::index(0)]);
        assert!(line.get_entry_by_path(&by_index).is_some());
        let deep = FieldPath::new([FieldSegment::index(0), FieldSegment::field("x")]);
        assert!(line.get_entry_by_path(&deep).is_none());
    }

    /// Every pair a body declares, rendered as the line wrote it.
    fn read(body: &[u8]) -> Vec<String> {
        let body = TextBytes::from_bytes(body).expect("a body");
        crate::media::text::entry::read_entries(&body)
            .as_ref()
            .map(TextEntries::as_slice)
            .unwrap_or_default()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    fn tree(body: &[u8]) -> TextEntries {
        let body = TextBytes::from_bytes(body).expect("a body");
        crate::media::text::entry::read_entries(&body).expect("the line states pairs")
    }

    #[test]
    fn a_tree_read_from_a_frame_keeps_the_whole_value_the_frame_bounded() {
        assert_eq!(
            read(b"8=FIX.4.4|35=D|18=G L|58=quoting #A=1 and #B=2|10=0|"),
            [
                "8=FIX.4.4",
                "35=D",
                "18=G L",
                "58=quoting #A=1 and #B=2",
                "10=0",
            ]
        );
        let entries = tree(b"8=FIX.4.4|35=D|NoAllocs[0].79=ACCT|Symbol[0]=AAPL|10=0|");
        assert_eq!(
            entries.as_slice()[2].key().as_str(),
            Some("NoAllocs[0].79"),
            "an indexed key is a key, and the loose walk found neither of these"
        );
        assert_eq!(entries.as_slice()[3].key().as_str(), Some("Symbol[0]"));
    }

    #[test]
    fn a_text_field_quoting_pairs_is_one_field_carrying_a_tree_of_them() {
        // The frame decides where the Text field ends; what that field's own
        // text says is read under it, which is what a pair-shaped value has
        // always meant here. The two readings do not compete: `58` is one
        // field of the frame, and `A` is a member of what `58` says.
        let entries = tree(b"8=FIX.4.4|35=D|58=quoting #A=1 and #B=2|10=0|");
        let quoting = &entries.as_slice()[2];
        assert_eq!(quoting.value().as_str(), Some("quoting #A=1 and #B=2"));
        let quoted = quoting
            .entries()
            .expect("the value states pairs")
            .as_slice();
        assert_eq!(quoted.len(), 2);
        assert_eq!(quoted[0].key().as_str(), Some("A"));
        assert!(
            quoted[0].marked() && quoted[1].marked(),
            "the quoted keys carry the mark the text wrote in front of them"
        );
        assert_eq!(
            entries
                .get_entry_by_path(&path("58"))
                .and_then(|found| found.value().as_str()),
            Some("quoting #A=1 and #B=2"),
            "and the frame's own field is what the frame's own key reaches"
        );
    }

    #[test]
    fn a_value_a_frame_bounded_is_still_a_range_of_the_page_it_came_from() {
        let page = page(b"8=FIX.4.4|58=a value with spaces|10=0|");
        let body = TextBytes::from_whole_page(Arc::clone(&page)).expect("the whole page");
        let entries = crate::media::text::entry::read_entries(&body).expect("pairs");
        let held = entries.as_slice()[1].value();
        assert_eq!(held.as_str(), Some("a value with spaces"));
        assert!(
            Arc::ptr_eq(held.page().expect("a page"), &page),
            "a wider value is a wider range, never a copy"
        );
    }

    #[test]
    fn an_entry_records_the_mark_the_line_wrote_and_keeps_its_key_stripped() {
        let entries = tree(b"MSGTYPE=D|ORDERID=123|#ORDERID=123|#SIDE=1");
        let held = entries.as_slice();
        assert!(!held[1].marked() && held[2].marked());
        assert_eq!(
            held[1].key().as_str(),
            held[2].key().as_str(),
            "the mark is off the key, so a path still lifts the name the \
             bridge gave the field"
        );
        assert_eq!(
            entries
                .get_entry_by_path(&path("ORDERID"))
                .and_then(|found| found.value().as_str()),
            Some("123"),
            "and the name reaches the pair that arrived first"
        );
        assert!(
            held[3].marked() && !held[3].key().as_bytes().starts_with(b"#"),
            "a marked key with no bare twin is marked all the same"
        );
    }

    #[test]
    fn two_entries_differing_only_in_the_mark_are_two_values() {
        let equal = tree(b"MSGTYPE=D|ORDERID=123|#ORDERID=123");
        let (bare, marked) = (&equal.as_slice()[1], &equal.as_slice()[2]);
        assert_eq!(bare.key(), marked.key());
        assert_eq!(bare.value(), marked.value());
        assert_ne!(bare, marked, "the line wrote two different things");
        assert_ne!(
            crate::stable_hash_of(bare),
            crate::stable_hash_of(marked),
            "unequal values must not be forced to hash alike"
        );
        assert!(
            bare < marked,
            "a bare pair sorts in front of its restatement"
        );
        assert_eq!(marked.to_string(), "#ORDERID=123");
        assert_eq!(bare.to_string(), "ORDERID=123");

        let differing = tree(b"MSGTYPE=D|ORDERID=123|#ORDERID=345");
        assert!(differing.as_slice()[2].marked());
        assert_eq!(differing.as_slice()[2].value().as_str(), Some("345"));
    }

    #[test]
    fn a_marked_stem_and_its_occurrence_arrive_marked_and_nested() {
        let entries = tree(b"MSGTYPE=D|#NOPARTYIDS=3|#NOPARTYIDS[0]=PARTYID=ONE");
        let held = entries.as_slice();
        assert!(held[1].marked() && held[2].marked());
        assert_eq!(held[1].key().as_str(), Some("NOPARTYIDS"));
        assert_eq!(held[2].key().as_str(), Some("NOPARTYIDS[0]"));
        assert_eq!(
            held[2]
                .entries()
                .and_then(|nested| nested.as_slice().first())
                .and_then(|member| member.value().as_str()),
            Some("ONE"),
            "an occurrence's members are read in their own scope"
        );
    }

    #[test]
    fn a_stated_absence_arrives_under_every_spelling_of_it() {
        assert_eq!(
            read(b"MSGTYPE=D|SYMBOL=|SIDE=null|PRICE=<null>|ACCOUNT=A"),
            [
                "MSGTYPE=D",
                "SYMBOL=",
                "SIDE=null",
                "PRICE=<null>",
                "ACCOUNT=A",
            ],
            "the tree carries what the line wrote; which spelling means absent \
             is a dialect's reading of it"
        );
        assert!(
            tree(b"MSGTYPE=D|SYMBOL=|SIDE=1").as_slice()[1]
                .value()
                .is_empty()
        );
    }

    #[test]
    fn an_entry_nothing_wrote_a_mark_for_is_unmarked() {
        let mut entries = TextEntries::new();
        entries
            .set_entry_by_path(
                &path("created"),
                TextBytes::from_bytes("x").expect("a value"),
            )
            .expect("the path names a child");
        assert!(
            !entries.as_slice()[0].marked(),
            "a caller creating an entry states no mark, and none is invented"
        );
    }
}

// --- The one decode path, the plan, and the two new options ---

mod decoding {
    use arrow_array::{Array as _, BinaryArray, StringArray};

    use crate::FieldPath;
    use crate::media::text::{TextOptions, into_arrow_batch, read_text_lines};

    use super::named;

    fn lines(source: &[u8], options: &TextOptions) -> Vec<crate::media::text::TextLine> {
        read_text_lines(&named("app.log", source), options)
            .expect("the configuration is settled")
            .map(|line| line.expect("a line decodes"))
            .collect()
    }

    #[test]
    fn every_line_becomes_one_typed_row() {
        let options = TextOptions::new();
        let decoded = lines(b"first\nsecond\nthird\n", &options);
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0].index(), 0);
        assert_eq!(decoded[2].index(), 2);
        assert_eq!(decoded[1].body().as_str(), Some("second"));
        // The URL is one shared value, not one rebuilt per line.
        assert_eq!(
            decoded[0].url().map(ToString::to_string),
            decoded[2].url().map(ToString::to_string)
        );
    }

    #[test]
    fn a_read_whose_columns_want_no_entry_builds_no_tree() {
        let options = TextOptions::new();
        let decoded = lines(b"35=D|55=AAPL\n", &options);
        assert!(
            decoded[0].entries().is_none(),
            "nothing asked for a tree, so none was built"
        );
        assert!(decoded[0].bodytype().is_none(), "and nothing classified it");
    }

    #[test]
    fn declaring_a_lifted_path_builds_the_tree_and_the_column() {
        let options = TextOptions::new()
            .try_with_lift_names(["55"])
            .expect("the path parses");
        let decoded = lines(b"35=D|55=AAPL\n", &options);
        let entries = decoded[0].entries().expect("a lifted column wants a tree");
        assert!(entries.len() >= 2, "both pairs were read");
        let path = FieldPath::from_str("55").expect("the path parses");
        assert_eq!(
            decoded[0]
                .get_entry_by_path(&path)
                .and_then(|held| held.value().as_str()),
            Some("AAPL")
        );

        let batch = into_arrow_batch(decoded, &options).expect("a batch builds");
        let column = batch
            .column_by_name("55")
            .expect("the lifted column is there");
        assert_eq!(
            column
                .as_any()
                .downcast_ref::<BinaryArray>()
                .expect("lifted values are binary")
                .value(0),
            b"AAPL"
        );
    }

    #[test]
    fn a_lifted_path_no_line_carries_is_null_rather_than_a_refusal() {
        let options = TextOptions::new()
            .try_with_lift_names(["absent"])
            .expect("the path parses");
        let decoded = lines(b"35=D\n", &options);
        let batch = into_arrow_batch(decoded, &options).expect("a batch builds");
        let column = batch.column_by_name("absent").expect("the column exists");
        assert!(column.is_null(0), "a path nothing carried is null");
        // And the column exists in the schema whether or not a row filled it.
        assert!(
            options
                .source_field()
                .expect("a schema")
                .get_field_by_path("absent")
                .is_some()
        );
    }

    #[test]
    fn renaming_changes_what_a_column_is_called_and_nothing_else() {
        let options = TextOptions::new()
            .with_renamed_column("body", "payload")
            .with_renamed_column("url", "source");
        let batch = into_arrow_batch(lines(b"hello\n", &options), &options).expect("a batch");
        assert!(batch.column_by_name("payload").is_some());
        assert!(batch.column_by_name("source").is_some());
        assert!(
            batch.column_by_name("body").is_none(),
            "the old name is gone"
        );
        assert_eq!(
            batch
                .column_by_name("payload")
                .expect("renamed")
                .as_any()
                .downcast_ref::<BinaryArray>()
                .expect("still binary")
                .value(0),
            b"hello"
        );
    }

    #[test]
    fn a_rename_naming_no_column_is_refused_at_options_time() {
        let options = TextOptions::new().with_renamed_column("nosuch", "x");
        let error = options
            .source_field()
            .expect_err("a rename must name a column");
        let rendered = error.to_string();
        assert!(rendered.contains("nosuch"), "{rendered}");
        assert!(rendered.contains("lift_names"), "{rendered}");
    }

    #[test]
    fn two_columns_may_not_emit_one_name() {
        let options = TextOptions::new().with_renamed_column("body", "url");
        let error = options.source_field().expect_err("one column per name");
        assert!(error.to_string().contains("twice"), "{error}");
    }

    #[test]
    fn a_lifted_path_names_its_own_column_with_an_alias() {
        // `as` names the column in the same breath that selects it, so no
        // rename is needed for the common case.
        let options = TextOptions::new()
            .try_with_lift_names(["\"55\" as symbol"])
            .expect("the path parses");
        let batch = into_arrow_batch(
            lines(
                b"55=AAPL
",
                &options,
            ),
            &options,
        )
        .expect("a batch");
        assert!(batch.column_by_name("symbol").is_some());
        assert!(batch.column_by_name("55").is_none());
        assert_eq!(
            batch
                .column_by_name("symbol")
                .expect("aliased")
                .as_any()
                .downcast_ref::<BinaryArray>()
                .expect("lifted values are binary")
                .value(0),
            b"AAPL"
        );
    }

    #[test]
    fn two_lifted_paths_ending_alike_are_told_apart_by_their_aliases() {
        // Without aliases both would take the last segment's name and collide.
        let options = TextOptions::new()
            .try_with_lift_names(["a.id as left_id", "b.id as right_id"])
            .expect("the paths parse");
        let field = options.source_field().expect("a schema");
        let children = field.dtype().as_fields().expect("a struct");
        let names: Vec<&str> = children.iter().map(crate::Field::name).collect();
        assert!(names.contains(&"left_id"));
        assert!(names.contains(&"right_id"));
    }

    #[test]
    fn a_lifted_path_may_be_renamed_like_any_other_column() {
        let options = TextOptions::new()
            .try_with_lift_names(["55"])
            .expect("the path parses")
            .with_renamed_column("55", "symbol");
        let batch = into_arrow_batch(lines(b"55=AAPL\n", &options), &options).expect("a batch");
        assert!(batch.column_by_name("symbol").is_some());
        assert!(batch.column_by_name("55").is_none());
    }

    #[test]
    fn the_classification_column_comes_from_one_scan() {
        let mut options = TextOptions::new();
        options.parse_mimetype = true;
        let decoded = lines(
            b"8=FIX.4.2|35=D|10=001
",
            &options,
        );
        assert!(decoded[0].bodytype().is_some());
        let batch = into_arrow_batch(decoded, &options).expect("a batch");
        let shape = batch
            .column_by_name("mimetype")
            .expect("the column is there")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("utf8")
            .value(0)
            .to_owned();
        assert_eq!(shape, "text/fix");
    }

    #[test]
    fn the_schema_a_batch_carries_is_the_schema_the_options_declared() {
        let mut options = TextOptions::new();
        options.start_rownum = Some(1);
        options.parse_mimetype = true;
        let options = options
            .try_with_lift_names(["55"])
            .expect("the path parses");
        let declared = options.source_field().expect("a schema");
        let batch = into_arrow_batch(lines(b"55=AAPL\n", &options), &options).expect("a batch");
        let schema = batch.schema();
        let names: Vec<&str> = schema
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect();
        let declared_fields = declared.fields();
        let expected: Vec<&str> = declared_fields.iter().map(crate::Field::name).collect();
        assert_eq!(
            names, expected,
            "the plan and the schema are one derivation"
        );
    }

    #[test]
    fn an_empty_object_answers_its_columns_and_no_rows() {
        let options = TextOptions::new();
        let batch = into_arrow_batch(Vec::new(), &options).expect("a batch");
        assert_eq!(batch.num_rows(), 0);
        assert!(batch.column_by_name("body").is_some());
    }

    #[test]
    fn a_lifted_path_with_no_name_to_take_is_refused() {
        let mut options = TextOptions::new();
        options.set_lift_paths(Some(vec![FieldPath::root()]));
        assert!(options.source_field().is_err());
    }
}

// --- Reading Arrow back into lines ---

mod intake {
    use crate::FieldPath;
    use crate::media::text::{
        TextOptions, from_arrow_batch, from_arrow_reader, into_arrow_batch, read_text_lines,
    };

    use super::named;

    fn decode(source: &[u8], options: &TextOptions) -> Vec<crate::media::text::TextLine> {
        read_text_lines(&named("app.log", source), options)
            .expect("a settled configuration")
            .map(|line| line.expect("a line"))
            .collect()
    }

    #[test]
    fn a_batch_round_trips_back_into_its_lines() {
        let mut options = TextOptions::new();
        options.start_rownum = Some(0);
        options.parse_mimetype = true;
        let lines = decode(b"first\nsecond\n", &options);
        let batch = into_arrow_batch(lines.clone(), &options).expect("a batch");
        let back = from_arrow_batch(&batch, &options).expect("lines read back");

        assert_eq!(back.len(), lines.len());
        for (read, original) in back.iter().zip(&lines) {
            assert_eq!(read.body(), original.body());
            assert_eq!(read.index(), original.index());
            assert_eq!(
                read.url().map(ToString::to_string),
                original.url().map(ToString::to_string)
            );
            assert_eq!(read.timestamp(), original.timestamp());
        }
    }

    #[test]
    fn a_column_named_the_way_someone_else_writes_it_is_still_found() {
        let options = TextOptions::new();
        // A producer that calls the body `payload` and the object `source`.
        let renamed = TextOptions::new()
            .with_renamed_column("body", "payload")
            .with_renamed_column("url", "source");
        let lines = decode(b"hello\n", &renamed);
        let foreign = into_arrow_batch(lines, &renamed).expect("a batch");

        // Read under the ordinary names: the aliases carry it.
        let back = from_arrow_batch(&foreign, &options).expect("lines read back");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].body().as_str(), Some("hello"));
        assert!(back[0].url().is_some(), "source resolved to url");
    }

    #[test]
    fn a_case_difference_alone_still_matches() {
        let renamed = TextOptions::new().with_renamed_column("body", "BODY");
        let lines = decode(b"hello\n", &renamed);
        let batch = into_arrow_batch(lines, &renamed).expect("a batch");
        let back = from_arrow_batch(&batch, &TextOptions::new()).expect("lines read back");
        assert_eq!(back[0].body().as_str(), Some("hello"));
    }

    #[test]
    fn a_column_the_batch_does_not_carry_leaves_its_field_at_the_default() {
        let mut with_rownum = TextOptions::new();
        with_rownum.start_rownum = Some(5);
        let lines = decode(b"only\n", &TextOptions::new());
        let batch = into_arrow_batch(lines, &TextOptions::new()).expect("a batch");
        // The reading options want a rownum column the batch has none of.
        let back = from_arrow_batch(&batch, &with_rownum).expect("lines read back");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].index(), 0, "the position it was read at");
        assert_eq!(back[0].body().as_str(), Some("only"));
    }

    #[test]
    fn a_lifted_column_round_trips_back_into_its_entry() {
        let options = TextOptions::new()
            .try_with_lift_names(["55"])
            .expect("the path parses");
        let lines = decode(b"55=AAPL\n", &options);
        let batch = into_arrow_batch(lines, &options).expect("a batch");
        let back = from_arrow_batch(&batch, &options).expect("lines read back");
        let path = FieldPath::from_str("55").expect("the path parses");
        assert_eq!(
            back[0]
                .get_entry_by_path(&path)
                .and_then(|held| held.value().as_str()),
            Some("AAPL")
        );
    }

    #[test]
    fn streamed_batches_read_back_one_at_a_time() {
        let mut options = TextOptions::new();
        options.batch_row_size = Some(1);
        let lines = decode(b"a\nb\nc\n", &options);
        let reader = crate::media::text::into_arrow_reader(
            lines.into_iter().map(Ok).collect::<Vec<_>>(),
            &options,
        )
        .expect("a reader");
        let back: Vec<_> = from_arrow_reader(reader, &options)
            .expect("a settled configuration")
            .map(|line| line.expect("a line"))
            .collect();
        assert_eq!(back.len(), 3);
        assert_eq!(back[2].body().as_str(), Some("c"));
    }

    #[test]
    fn a_timestamp_wider_than_the_column_is_refused_by_name() {
        let options = TextOptions::new();
        let mut lines = decode(b"one\n", &options);
        // The line counts in 128 bits; the column holds 64.
        lines[0].set_timestamp(Some(i128::from(i64::MAX) + 1));
        let error = into_arrow_batch(lines, &options).expect_err("a count that will not fit");
        let rendered = error.to_string();
        assert!(rendered.contains("mtime"), "{rendered}");
        assert!(rendered.contains("64-bit"), "{rendered}");
    }
}
