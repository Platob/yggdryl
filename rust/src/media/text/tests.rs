use arrow_array::{Array as _, BinaryArray, Int64Array, StringArray, UInt64Array};

use crate::holder::Buffer;
use crate::media::text::{LeadingFragment, LineSep, Text, TextOptions};
use crate::media::{IORecordOptions as _, RecordOptions};
use crate::{Codec, DataType, Timezone};
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
        .try_with_lstrip(r"^\s+")
        .unwrap()
        .try_with_rstrip(r"\s+$")
        .unwrap()
        .with_linesep(LineSep::CRLF)
        .with_framing(true)
        .with_leading_fragment(LeadingFragment::Drop)
        .with_max_record_byte_size(1_024)
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
    assert!(options.framing());
    assert_eq!(options.leading_fragment(), LeadingFragment::Drop);
    assert_eq!(options.max_record_byte_size(), Some(1_024));
    assert!(!options.autotype());
    assert_eq!(options.timezone(), Some(&Timezone::UTC));
    assert_eq!(options.with_rownum, Some(-3));
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
fn framing_normalizes_every_physical_terminator_and_keeps_start_rownums() {
    let source = named(
        "mixed.log",
        b"[A] first\ncontinuation one\r\n[B] second\rcontinuation two",
    );
    let mut options = framed(r"^\[(?<kind>[A-Z])\] ");
    options.with_rownum = Some(10);
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
    options.with_rownum = Some(i64::MAX);
    let text = Text::new(source).with_options(options);

    // The second output row cannot be represented by the configured rownum,
    // and its capture is not UTF-8. Neither changes the number of records.
    assert_eq!(text.row_size().unwrap(), 2);
}

#[test]
fn a_result_row_limit_does_not_convert_the_following_record() {
    let source = named("limited-values.log", b"A first\n\xFF invalid\n");
    let mut options = framed(r"^(?<kind>(?-u:.)) ");
    options.with_rownum = Some(i64::MAX);
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
    options.with_rownum = Some(i64::MAX);
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
    keep.with_rownum = Some(1);
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
        ["url", "body", "dropped_byte_size", "kind"]
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
            ["url", "body", "dropped_byte_size", "kind"]
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
    options.with_rownum = Some(1);
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
