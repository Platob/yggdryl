use arrow_array::{Array as _, Int64Array, StringArray, UInt64Array};

/// One hash of any hashable value, for "equal values hash alike" and for a
/// temporary name that does not collide. The crate's own stable hash is
/// private, and neither use needs it to be stable across runs.
pub(super) fn hash_of<T: std::hash::Hash>(value: &T) -> u64 {
    use std::hash::Hasher as _;

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions as _, RecordOptions};
use yggdryl::text::{LeadingFragment, LineSep, Text, TextLine, TextOptions, read_text_lines};
use yggdryl::{Codec, DataType, Field, StructType, Timezone};
use yggdryl::{IOBase as _, IOMedia as _};

fn named(name: &str, bytes: &[u8]) -> Buffer {
    Buffer::from_bytes(bytes.to_vec()).with_media_type(
        yggdryl::Url::from_str(&format!("file:///{name}"))
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

/// The sixteen event columns every line batch opens with, in front of the
/// line's own: the line is an event of the graph, and a message parsed out
/// of it opens with the same sixteen.
const EVENT_COLUMNS: [&str; 16] = [
    "currunix",
    "creaunix",
    "expirunix",
    "prevunix",
    "snapunix",
    "curruuid",
    "crossuuid",
    "crosscode",
    "currhashcode",
    "crosshashcode",
    "prevuuid",
    "seqnum",
    "parentuuids",
    "srcuuids",
    "identifiers",
    "state",
];

/// The names of a line batch: the event columns, then the line's own.
fn with_event(rest: &[&'static str]) -> Vec<&'static str> {
    EVENT_COLUMNS.iter().chain(rest).copied().collect()
}

fn collect(source: &impl yggdryl::IOBase, options: TextOptions) -> Vec<arrow_array::RecordBatch> {
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
                .downcast_ref::<StringArray>()
                .unwrap()
                .iter()
                .map(|value| value.unwrap().as_bytes().to_vec())
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

/// The alignment a binding can actually give the read it holds.
///
/// A binding does not box what it hands back: PyO3 places this struct inside
/// an object CPython allocated, and CPython's allocator aligns to 16 bytes.
/// A field asking for more - a vector-width searcher is the one that did -
/// makes every use of the read a misaligned dereference, which a release
/// build quietly tolerates on x86 and a debug build aborts the process over.
/// The rule is cheap to keep and invisible to break, so it is pinned here
/// rather than discovered in a binding's test suite.
#[test]
fn a_read_fits_the_alignment_an_object_allocator_gives() {
    assert!(
        align_of::<yggdryl::text::TextLines>() <= 16,
        "a text read asks to be aligned to {} bytes, and an object allocator gives 16",
        align_of::<yggdryl::text::TextLines>()
    );
}

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
    assert!(error.contains(
        "distinct from sourceurl, rownum, body, dropped_byte_size and the event columns the line derives"
    ));
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
    assert_eq!(batch.schema().field(16).name(), "sourceurl");
    assert_eq!(batch.schema().field(17).name(), "rownum");
    assert_eq!(batch.schema().field(18).name(), "mtime");
    assert_eq!(batch.schema().field(19).name(), "body");
    assert_eq!(
        batch.schema().field(21).data_type(),
        &arrow_schema::DataType::Int64
    );
    assert_eq!(
        batch
            .column(17)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .values(),
        &[10, 11, 12]
    );
    // The body is the line as cut - the header included, the edges
    // stripped - and the captures are read off it beside it.
    assert_eq!(
        batch
            .column(19)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        [
            Some("[INFO] id=7 first"),
            Some("[WARN] id=9 second"),
            Some("plain")
        ]
    );
    assert_eq!(
        batch
            .column(20)
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
    assert_eq!(field.field("value").unwrap().dtype(), &DataType::utf8(),);

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
        batch.schema().field(19).data_type(),
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
        batch.schema().field(19).data_type(),
        &arrow_schema::DataType::Utf8
    );
    assert_eq!(
        batch
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>(),
        with_event(&["sourceurl", "mtime", "body", "value"])
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
            .column(17)
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
    use yggdryl::local::{File, Folder};

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
            .column(16)
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
        batch.schema().field(19).data_type(),
        &arrow_schema::DataType::Time32(arrow_schema::TimeUnit::Second)
    );
    assert_eq!(
        batch
            .column(19)
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
        batch.schema().field(19).data_type(),
        &arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, Some("UTC".into()))
    );
    assert_eq!(
        batch
            .column(18)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "2026-08-29 00:00:00.434_958 [77-2f3e6ff7:9f4d2a08b1:128] \
[ModuleFailFastFilterChecker] (DEBUG) Execution report \
(execId: 20260828180000369318, from session:"
    );
    assert_eq!(
        batch
            .column(20)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "77-2f3e6ff7:9f4d2a08b1:128"
    );
    assert_eq!(
        batch
            .column(21)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "ModuleFailFastFilterChecker"
    );
}

#[test]
fn a_comma_fraction_and_a_variable_width_one_are_timestamp_columns() {
    // ISO 8601 names the comma a decimal sign, so a log4j clock is a clock:
    // the capture's own syntax types the column at the width it spells, and
    // the row reads through the header's own match rather than through a cast
    // that would demand a zone the line never carries.
    let source = named("log4j.log", b"2026-08-14 00:05:01,148 [main] started\n");
    let text = TextOptions::new()
        .try_with_rowheader(
            r"^(?<stamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3}) \[(?<thread>[^]]+)\] ",
        )
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
        batch.schema().field(19).data_type(),
        &arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Millisecond, Some("UTC".into()))
    );
    assert_eq!(
        batch
            .column(19)
            .as_any()
            .downcast_ref::<arrow_array::TimestampMillisecondArray>()
            .unwrap()
            .value(0),
        1_786_665_901_148
    );

    // A capture admitting several widths publishes the widest, and a row
    // spelling fewer digits restates into it exactly.
    let source = named(
        "variable.log",
        b"2026-08-14 00:05:01.148 short\n2026-08-14 00:05:01.12345 long\n",
    );
    let text = TextOptions::new()
        .try_with_rowheader(r"^(?<stamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{1,5}) ")
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
        batch.schema().field(19).data_type(),
        &arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, Some("UTC".into()))
    );
    assert_eq!(
        batch
            .column(19)
            .as_any()
            .downcast_ref::<arrow_array::TimestampMicrosecondArray>()
            .unwrap()
            .iter()
            .collect::<Vec<_>>(),
        [Some(1_786_665_901_148_000), Some(1_786_665_901_123_450)]
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
            b"[A] first\ncontinuation one".to_vec(),
            b"[B] second\ncontinuation two".to_vec(),
        ]
    );
    assert_eq!(rownums(&batches), [10, 12]);
}

#[test]
fn framing_carries_a_record_across_input_windows_and_output_batches() {
    let mut bytes = b"[A] first\n".to_vec();
    bytes.extend(std::iter::repeat_n(
        b'x',
        yggdryl::DEFAULT_STREAM_BATCH_SIZE + 17,
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
        4 + 6 + yggdryl::DEFAULT_STREAM_BATCH_SIZE + 17 + 18
    );
    assert!(bodies[0].starts_with(b"[A] first\nxxxxxxxx"));
    assert!(bodies[0].ends_with(b"\nlast continuation"));
    assert_eq!(bodies[1], b"[B] next\nend");
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
    // and its capture is a byte the decode would have to repair. Neither
    // changes the number of records, because counting converts nothing.
    assert_eq!(text.row_size().unwrap(), 2);
}

#[test]
fn a_result_row_limit_does_not_convert_the_following_record() {
    // The second row's number cannot be represented, so converting it would
    // be a refusal; the limit stops the read before that.
    let source = named("limited-values.log", b"A first\nB second\n");
    let mut options = framed(r"^(?<kind>(?-u:.)) ");
    options.start_rownum = Some(i64::MAX);
    options.set_batch_row_size(Some(8));
    options.set_max_row_size(Some(1));

    let batches = collect(&source, options);
    assert_eq!(bodies(&batches), [b"A first".to_vec()]);
    assert_eq!(rownums(&batches), [i64::MAX]);
}

#[test]
fn a_physical_row_limit_does_not_convert_the_following_line() {
    let source = named("limited-lines.log", b"A first\nB second\n");
    let mut options = options(r"^(?<kind>(?-u:.)) ");
    options.start_rownum = Some(i64::MAX);
    options.set_batch_row_size(Some(8));
    options.set_max_row_size(Some(1));

    let batches = collect(&source, options);
    assert_eq!(bodies(&batches), [b"A first".to_vec()]);
    assert_eq!(rownums(&batches), [i64::MAX]);
}

#[test]
fn an_unconvertible_next_record_follows_the_completed_record_batch_prefix() {
    // The second record's number cannot be represented, and the refusal
    // arrives after the batch the first record completed - never inside it.
    let source = named("invalid-next-header.log", b"A first\nB second\n");
    let mut options = framed(r"^(?<kind>(?-u:.)) ");
    options.start_rownum = Some(i64::MAX);
    options.set_batch_row_size(Some(8));
    let mut reader = source.read_arrow_reader(&options.into()).unwrap();

    let prefix = reader.next().unwrap().unwrap();
    assert_eq!(bodies(&[prefix]), [b"A first".to_vec()]);
    let error = reader.next().unwrap().unwrap_err().to_string();
    assert!(
        error.contains("text row number exceeds i64::MAX"),
        "{error}"
    );
    assert!(reader.next().is_none());
}

#[test]
fn a_line_that_was_not_utf_8_reaches_its_row_decoded_and_says_so() {
    // One Latin-1 byte in the body and one in a capture: each reads as the
    // character Windows-1252 gives it, the row's body is text, and the line
    // counts the two bytes it repaired. The counts the reader took before
    // the line existed - the record's bytes over its limit - are counts of
    // the bytes as read.
    let source = named("latin1.log", b"[caf\xE9] first \xE9 line\n[plain] second\n");
    // A byte class, because a Unicode class matches characters and a byte
    // that is not one is not matched by it.
    let mut options = framed(r"^\[(?<kind>(?-u:[^\]]+))\] ");
    options.set_max_record_byte_size(Some(7));
    let lines: Vec<TextLine> = read_text_lines(&source, &options)
        .unwrap()
        .map(|line| line.unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    // The header is retained whole and the limit bounds what follows it:
    // `[café] ` then seven of the twelve wire bytes past it.
    assert_eq!(lines[0].body(), "[caf\u{e9}] first \u{e9}");
    assert_eq!(lines[0].capture(0), Some("caf\u{e9}"));
    assert_eq!(lines[0].decoded_byte_size(), 2);
    assert_eq!(
        lines[0].dropped_byte_size(),
        Some(5),
        "the limit and the count are wire bytes past the header: 12 read, 7 kept"
    );
    assert_eq!(lines[1].body(), "[plain] second");
    assert_eq!(lines[1].decoded_byte_size(), 0);

    let batches = collect(&source, framed(r"^\[(?<kind>(?-u:[^\]]+))\] "));
    assert_eq!(
        bodies(&batches),
        [
            "[caf\u{e9}] first \u{e9} line".as_bytes().to_vec(),
            b"[plain] second".to_vec()
        ]
    );
    let batch = &batches[0];
    assert_eq!(
        batch.schema().field_with_name("body").unwrap().data_type(),
        &arrow_schema::DataType::Utf8
    );
    let kinds = batch
        .column_by_name("kind")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(kinds.value(0), "caf\u{e9}");
}

#[test]
fn a_record_cut_inside_a_character_reads_the_bytes_that_are_left() {
    // The three-byte euro sign, cut after two of its bytes by the limit: the
    // orphans read as the two Windows-1252 characters they are, not as a
    // replacement character, and both are counted as decoded.
    let source = named("cut.log", "[A] x\u{20AC}\n".as_bytes());
    let mut options = framed(r"^\[(?<kind>[A-Z])\] ");
    options.set_max_record_byte_size(Some(3));
    let lines: Vec<TextLine> = read_text_lines(&source, &options)
        .unwrap()
        .map(|line| line.unwrap())
        .collect();
    assert_eq!(lines[0].body(), "[A] x\u{e2}\u{201a}");
    assert_eq!(lines[0].decoded_byte_size(), 2);
    assert_eq!(lines[0].dropped_byte_size(), Some(1));
}

#[test]
fn leading_fragments_are_kept_dropped_or_rejected_and_eof_finishes_a_record() {
    let source = named("leading.log", b"before\nstill before\n[A] final");

    let mut keep = framed(r"^\[(?<kind>[A-Z])\] ");
    keep.start_rownum = Some(1);
    let kept = collect(&source, keep);
    assert_eq!(
        bodies(&kept),
        [b"before\nstill before".to_vec(), b"[A] final".to_vec()]
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
    assert_eq!(bodies(&dropped), [b"[A] final".to_vec()]);

    let rejected = framed(r"^\[(?<kind>[A-Z])\] ").with_leading_fragment(LeadingFragment::Error);
    let mut reader = source.read_arrow_reader(&rejected.into()).unwrap();
    let error = reader.next().unwrap().unwrap_err().to_string();
    assert!(error.contains("leading physical line"));
    assert!(reader.next().is_none());
}

#[test]
fn record_byte_limit_reports_only_bytes_beyond_the_retained_prefix() {
    let source = named("limit.log", b"[A] abc\ndef\n[B] xyz\n");

    // The limit bounds what follows the header, which is retained whole: a
    // record is known by its header, so a limit of nothing still leaves it.
    let exact = collect(
        &source,
        framed(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(7),
    );
    assert_eq!(
        bodies(&exact),
        [b"[A] abc\ndef".to_vec(), b"[B] xyz".to_vec()]
    );
    assert_eq!(dropped(&exact), [None, None]);

    let limited = collect(
        &source,
        framed(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(6),
    );
    assert_eq!(
        bodies(&limited),
        [b"[A] abc\nde".to_vec(), b"[B] xyz".to_vec()]
    );
    assert_eq!(dropped(&limited), [Some(1), None]);

    let zero = collect(
        &source,
        framed(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(0),
    );
    assert_eq!(bodies(&zero), [b"[A] ".to_vec(), b"[B] ".to_vec()]);
    assert_eq!(dropped(&zero), [Some(7), Some(3)]);
}

#[test]
fn oversized_continuations_are_drained_before_the_following_record() {
    let oversized = yggdryl::DEFAULT_STREAM_BATCH_SIZE * 8 + 31;
    let mut bytes = b"[A] begin\n".to_vec();
    bytes.extend(std::iter::repeat_n(b'x', oversized));
    bytes.extend_from_slice(b"\n[B] after\n");
    let source = named("oversized.log", &bytes);
    let options = framed(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(8);

    let batches = collect(&source, options);
    assert_eq!(
        bodies(&batches),
        [b"[A] begin\nxx".to_vec(), b"[B] after".to_vec()]
    );
    assert_eq!(
        dropped(&batches),
        [Some(u64::try_from(oversized - 2).unwrap()), None]
    );
}

#[test]
fn an_oversized_matching_line_retains_only_its_body_prefix() {
    let oversized = yggdryl::DEFAULT_STREAM_BATCH_SIZE * 8 + 31;
    let mut bytes = b"[A] ".to_vec();
    bytes.extend(std::iter::repeat_n(b'x', oversized));
    bytes.extend_from_slice(b"\n[B] after\n");
    let source = named("oversized-header-line.log", &bytes);
    let options = framed(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(8);

    let batches = collect(&source, options);
    assert_eq!(
        bodies(&batches),
        [b"[A] xxxxxxxx".to_vec(), b"[B] after".to_vec()]
    );
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

    let header_size = yggdryl::DEFAULT_STREAM_BATCH_SIZE - 1;
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
            [b"[A] first\ncontinued".to_vec(), b"[B] second".to_vec()]
        );
    }
}

#[test]
fn framed_schema_is_complete_before_empty_or_absent_input_is_pulled() {
    use yggdryl::local::File;

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
        with_event(&["sourceurl", "mtime", "body", "dropped_byte_size", "kind"])
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
            hash_of(&path),
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
            with_event(&["sourceurl", "mtime", "body", "dropped_byte_size", "kind"])
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
    use yggdryl::local::Folder;

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
            b"[A] first\ncontinued in a".to_vec(),
            b"leading in b".to_vec(),
            b"[B] second".to_vec(),
        ]
    );
    assert_eq!(rownums(&batches), [1, 1, 2]);

    folder.remove(true).unwrap();
}

#[test]
fn generic_record_writes_use_only_the_text_body() {
    let mut target = named("out.txt", b"old");
    let mut options: RecordOptions = TextOptions::new().into();
    let field = StructType::from_fields([
        DataType::utf8().required_field("sourceurl"),
        DataType::Int64.required_field("rownum"),
        DataType::utf8().required_field("body"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    options.set_field(field);
    let rows = [
        yggdryl::Scalar::from_struct([
            ("sourceurl", yggdryl::Scalar::from("input")),
            ("rownum", yggdryl::Scalar::from(1_i64)),
            ("body", yggdryl::Scalar::from("first")),
        ])
        .unwrap(),
        yggdryl::Scalar::from_struct([
            ("sourceurl", yggdryl::Scalar::from("input")),
            ("rownum", yggdryl::Scalar::from(2_i64)),
            ("body", yggdryl::Scalar::from("second")),
        ])
        .unwrap(),
    ];
    target.overwrite_records(rows, &options).unwrap();
    assert_eq!(target.read_all_bytes().unwrap(), b"first\nsecond\n");
}

#[test]
fn a_dictionary_encoded_body_is_a_text_body_a_write_unpacks_once() {
    // Arrow JS infers `Dictionary<Int32, Utf8>` for a plain record's string,
    // so a body column in that layout is the same text under another
    // spelling, unpacked once per batch rather than refused.
    use std::sync::Arc;

    use arrow_array::{DictionaryArray, Int32Array, RecordBatch};

    let mut target = named("dictionary.txt", b"old");
    let options: RecordOptions = TextOptions::new().into();
    let keys = Int32Array::from(vec![0, 1, 0]);
    let values = Arc::new(StringArray::from(vec!["one", "two"]));
    let body = DictionaryArray::<arrow_array::types::Int32Type>::try_new(keys, values).unwrap();
    let schema = Arc::new(arrow_schema::Schema::new(vec![
        arrow_schema::Field::new("sourceurl", arrow_schema::DataType::Utf8, false),
        arrow_schema::Field::new(
            "body",
            arrow_schema::DataType::Dictionary(
                Box::new(arrow_schema::DataType::Int32),
                Box::new(arrow_schema::DataType::Utf8),
            ),
            false,
        ),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["input", "input", "input"])),
            Arc::new(body),
        ],
    )
    .unwrap();
    target.overwrite_arrow_batch(batch, &options).unwrap();
    assert_eq!(target.read_all_bytes().unwrap(), b"one\ntwo\none\n");
}

#[test]
fn a_binary_body_is_refused_by_a_write_naming_what_it_expected() {
    // A text row's body is text, and a column that may hold anything is not
    // written as if it were: the refusal names the column and the layout.
    let mut target = named("refused.txt", b"old");
    let mut options: RecordOptions = TextOptions::new().into();
    let field = StructType::from_fields([
        DataType::utf8().required_field("sourceurl"),
        DataType::binary().required_field("body"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    options.set_field(field);
    let rows = [yggdryl::Scalar::from_struct([
        ("sourceurl", yggdryl::Scalar::from("input")),
        ("body", yggdryl::Scalar::from(&b"first"[..])),
    ])
    .unwrap()];
    let error = target
        .overwrite_records(rows, &options)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("expected a utf8 body column, got Binary"),
        "{error}"
    );
    assert_eq!(target.read_all_bytes().unwrap(), b"old");
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
fn the_classification_column_reads_the_line_and_leaves_the_body() {
    let source = Buffer::from_bytes(
        [
            b"sending >> 8=FIX.4.2|9=176|35=D|10=203|\n".as_slice(),
            b"recv ACCOUNT=A1|MSGTYPE=8|SYMBOL=AAPL\n".as_slice(),
            b"level=INFO worker=3 took=12ms\n".as_slice(),
            b"<Order ClOrdID='XML-1'/>\n".as_slice(),
            b"no level printed by this plugin\n".as_slice(),
        ]
        .concat(),
    );
    let mut options = TextOptions::new();
    options.parse_mimetype = true;

    // The columns a classifying read declares, in order. A direction is
    // FIX's fact and not the reader's: no column carries one.
    let field = options.source_field().unwrap();
    let names: Vec<&str> = field
        .dtype()
        .as_fields()
        .unwrap()
        .iter()
        .map(Field::name)
        .collect();
    assert_eq!(
        names,
        with_event(&["sourceurl", "mtime", "mimetype", "body"])
    );

    let batches = collect(&source, options);
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
    // The body is the line as read, the transport's verb included: it is
    // prose in front of the payload, and the codec reads it there.
    assert_eq!(
        bodies(&batches)[..2],
        [
            b"sending >> 8=FIX.4.2|9=176|35=D|10=203|".to_vec(),
            b"recv ACCOUNT=A1|MSGTYPE=8|SYMBOL=AAPL".to_vec(),
        ]
    );
    assert_eq!(
        bodies(&batches)[4],
        b"no level printed by this plugin".to_vec()
    );
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
    let mut options = TextOptions::new()
        .try_with_lstrip([r"^(?:sending|recv) >>\s*"])
        .unwrap();
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
    use yggdryl::text::{Text, TextOptions};

    use yggdryl::fs::{
        BoundLocation, ByteReader, ByteWriter, FileInfo, FileInfos, FileSelector, FileSystem,
        MemoryFileSystem, OutputMetadata, RandomAccessReader,
    };
    use yggdryl::{Codec, DEFAULT_FETCH_BYTE_SIZE, IOBase as _, IOMedia as _, Url};

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
        fn read(&mut self, buffer: &mut [u8]) -> yggdryl::Result<usize> {
            self.reads.lock().unwrap().push(buffer.len());
            self.inner.read(buffer)
        }

        fn tell(&self) -> u64 {
            self.inner.tell()
        }

        fn close(&mut self) -> yggdryl::Result<()> {
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

        fn normalize_path(&self, path: &str) -> yggdryl::Result<String> {
            self.inner.normalize_path(path)
        }

        fn file_info(&self, path: &str) -> yggdryl::Result<FileInfo> {
            self.inner.file_info(path)
        }

        fn list(&self, selector: &FileSelector) -> FileInfos {
            self.inner.list(selector)
        }

        fn create_dir(&self, path: &str, recursive: bool) -> yggdryl::Result<()> {
            self.inner.create_dir(path, recursive)
        }

        fn delete_dir(&self, path: &str) -> yggdryl::Result<()> {
            self.inner.delete_dir(path)
        }

        fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> yggdryl::Result<()> {
            self.inner.delete_dir_contents(path, missing_dir_ok)
        }

        fn delete_root_dir_contents(&self) -> yggdryl::Result<()> {
            self.inner.delete_root_dir_contents()
        }

        fn delete_file(&self, path: &str) -> yggdryl::Result<()> {
            self.inner.delete_file(path)
        }

        fn copy_file(&self, source: &str, target: &str) -> yggdryl::Result<()> {
            self.inner.copy_file(source, target)
        }

        fn move_file(&self, source: &str, target: &str) -> yggdryl::Result<()> {
            self.inner.move_file(source, target)
        }

        fn open_input_file(&self, path: &str) -> yggdryl::Result<Box<dyn RandomAccessReader>> {
            self.inner.open_input_file(path)
        }

        fn open_input_stream(&self, path: &str) -> yggdryl::Result<Box<dyn ByteReader>> {
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
        ) -> yggdryl::Result<Box<dyn ByteWriter>> {
            self.inner.open_output_stream(path, metadata)
        }

        fn open_append_stream(
            &self,
            path: &str,
            metadata: Option<&OutputMetadata>,
        ) -> yggdryl::Result<Box<dyn ByteWriter>> {
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
            bodies.push([b"[INFO] ".as_slice(), &body].concat());
        }
        (plain, bodies)
    }

    fn framed() -> TextOptions {
        TextOptions::new()
            .try_with_rowheader(ROWHEADER)
            .unwrap()
            .with_framing(true)
    }

    fn located(name: &str, bytes: &[u8]) -> (yggdryl::fs::File, Arc<Counting>) {
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
        let mut handle = yggdryl::fs::File::new(bound);
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
    assert_eq!(names, with_event(&["sourceurl", "mtime", "body", "id"]));
    assert_eq!(
        batch.schema().field(17).data_type(),
        &arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Nanosecond, Some("UTC".into()))
    );
    assert_eq!(
        batch
            .column(17)
            .as_any()
            .downcast_ref::<TimestampNanosecondArray>()
            .unwrap()
            .value(0),
        1_577_934_245_000_000_000
    );
}

#[test]
fn an_mtime_capture_that_states_only_a_date_dates_the_line_at_midnight() {
    use arrow_array::TimestampNanosecondArray;

    // A log that dates its lines by the day states no clock, and a date is
    // the instant that day opens: the capture reads midnight in the column's
    // zone rather than failing the row for want of a clock it never had.
    let source = named("dated.log", b"2020-01-02 id=7 first\n");
    let batch = collect(&source, options(r"^(?<mtime>\S+) id=(?<id>\d+) "))
        .pop()
        .unwrap();
    assert_eq!(
        batch
            .column(17)
            .as_any()
            .downcast_ref::<TimestampNanosecondArray>()
            .unwrap()
            .value(0),
        1_577_923_200_000_000_000
    );

    // The compact spelling a wire writes is the same day and the same
    // reading, where it used to be no reading at all.
    let source = named("dated.log", b"20200102 id=7 first\n");
    let batch = collect(&source, options(r"^(?<mtime>\d+) id=(?<id>\d+) "))
        .pop()
        .unwrap();
    assert_eq!(
        batch
            .column(17)
            .as_any()
            .downcast_ref::<TimestampNanosecondArray>()
            .unwrap()
            .value(0),
        1_577_923_200_000_000_000
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
    use arrow_array::TimestampNanosecondArray;
    use yggdryl::local::File;

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
        batch.schema().field(18).data_type(),
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
    use super::hash_of;
    use std::sync::Arc;

    use yggdryl::text::{TextBytes, TextEntries, TextEntry, TextLine};
    use yggdryl::{FieldPath, FieldSegment};

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
    fn an_entry_answers_text_borrowed_from_its_page_and_its_range_beside_it() {
        use std::borrow::Cow;

        let body = TextBytes::from_bytes("8=FIX.4.4|58=caf\u{e9}|10=0|".as_bytes()).unwrap();
        let entries = TextEntries::from_bytes(&body).unwrap();
        let text = &entries.as_slice()[1];
        assert!(matches!(text.key(), Cow::Borrowed("58")));
        assert!(matches!(text.value(), Cow::Borrowed("caf\u{e9}")));
        assert_eq!(text.value_bytes().as_bytes(), "caf\u{e9}".as_bytes());
        assert!(
            std::sync::Arc::ptr_eq(text.value_bytes().page().unwrap(), body.page().unwrap()),
            "the range is the line's own page"
        );
        assert_eq!(text.to_string(), "58=caf\u{e9}");

        // A range a caller built from bytes that are not text is the one
        // case the answer is owned: the lossy decode, and the range intact.
        let raw = TextEntry::new(
            TextBytes::from_bytes(b"96").unwrap(),
            TextBytes::from_bytes(b"\xff\xfe A").unwrap(),
        );
        assert!(matches!(raw.value(), Cow::Owned(_)));
        assert_eq!(raw.value(), "\u{fffd}\u{fffd} A");
        assert_eq!(raw.value_bytes().as_bytes(), b"\xff\xfe A");
        assert!(
            TextEntries::from_iter([raw.clone()])
                .get_entry_by_path(&path("96"))
                .is_some(),
            "and its key, which is text, is reachable by name"
        );
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
            hash_of(&left),
            hash_of(&right),
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
        let line = TextLine::from_bytes(
            0,
            TextBytes::from_bytes("body").expect("a body"),
            std::sync::Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap()
        .with_entries(entries);

        assert_eq!(
            line.get_entry_by_path(&path("55"))
                .and_then(|held| held.value_bytes().as_str()),
            Some("AAPL")
        );
        assert_eq!(
            line.get_entry_by_path(&path("\"213\".PartyID"))
                .and_then(|held| held.value_bytes().as_str()),
            Some("ACME")
        );
        assert_eq!(
            line.get_entry_by_path(&path("[1]"))
                .and_then(|held| held.key_bytes().as_str()),
            Some("55")
        );
        assert_eq!(
            line.get_entry_by_path(&path("[-1]"))
                .and_then(|held| held.key_bytes().as_str()),
            Some("213")
        );
    }

    #[test]
    fn a_miss_is_null_and_never_an_error_or_a_panic() {
        let line = TextLine::from_bytes(
            0,
            TextBytes::from_bytes("body").expect("a body"),
            std::sync::Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap()
        .with_entries(TextEntries::from_iter([entry("a", "1")]));
        for text in ["b", "a.b", "a.b.c", "[9]", "[-9]"] {
            assert!(
                line.get_entry_by_path(&path(text)).is_none(),
                "{text} must miss"
            );
        }
        // A line carrying no tree at all misses the same way.
        let bare = TextLine::from_bytes(
            0,
            TextBytes::from_bytes("body").expect("a body"),
            std::sync::Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap();
        assert!(bare.get_entry_by_path(&path("a")).is_none());
        // The raising form says why, and names the path.
        let error = bare.entry_by_path(&path("a")).expect_err("raises");
        assert!(error.to_string().contains('a'), "{error}");
    }

    #[test]
    fn a_key_with_no_value_is_found_and_is_not_a_miss() {
        let line = TextLine::from_bytes(
            0,
            TextBytes::from_bytes("body").expect("a body"),
            std::sync::Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap()
        .with_entries(TextEntries::from_iter([entry("a", "")]));
        let found = line
            .get_entry_by_path(&path("a"))
            .expect("the key is there");
        assert!(found.value().is_empty());
    }

    #[test]
    fn the_setter_creates_what_is_not_there() {
        let mut line = TextLine::from_bytes(
            0,
            TextBytes::from_bytes("body").expect("a body"),
            std::sync::Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap();
        line.set_entry_by_path(
            &path("order.price"),
            TextBytes::from_bytes("12").expect("a value"),
        )
        .expect("creates both levels");
        assert_eq!(
            line.get_entry_by_path(&path("order.price"))
                .and_then(|held| held.value_bytes().as_str()),
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
                .and_then(|held| held.value_bytes().as_str()),
            Some("13")
        );
    }

    #[test]
    fn a_position_naming_no_entry_refuses_and_changes_nothing() {
        let mut line = TextLine::from_bytes(
            0,
            TextBytes::from_bytes("body").expect("a body"),
            std::sync::Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap()
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
        let mut line = TextLine::from_bytes(
            0,
            TextBytes::from_bytes("body").expect("a body"),
            std::sync::Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap();
        assert!(
            line.set_entry_by_path(&FieldPath::root(), TextBytes::new())
                .is_err()
        );
    }

    #[test]
    fn removing_takes_one_entry_at_the_path() {
        let mut line = TextLine::from_bytes(
            0,
            TextBytes::from_bytes("body").expect("a body"),
            std::sync::Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap()
        .with_entries(TextEntries::from_iter([entry("a", "1"), entry("b", "2")]));
        let removed = line.remove_entry_by_path(&path("a")).expect("removes");
        assert_eq!(removed.key_bytes().as_str(), Some("a"));
        assert_eq!(line.entries().map(TextEntries::len), Some(1));
        assert!(line.remove_entry_by_path(&path("a")).is_none());
    }

    #[test]
    fn a_repeated_key_is_two_entries_reachable_by_position() {
        let line = TextLine::from_bytes(
            0,
            TextBytes::from_bytes("body").expect("a body"),
            std::sync::Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap()
        .with_entries(TextEntries::from_iter([
            entry("tag", "first"),
            entry("tag", "second"),
        ]));
        assert_eq!(
            line.get_entry_by_path(&path("tag"))
                .and_then(|held| held.value_bytes().as_str()),
            Some("first"),
            "a name reaches the first"
        );
        assert_eq!(
            line.get_entry_by_path(&path("[1]"))
                .and_then(|held| held.value_bytes().as_str()),
            Some("second")
        );
    }

    #[test]
    fn a_line_carries_what_no_other_field_can_recover() {
        let mut line = TextLine::from_bytes(
            7,
            TextBytes::from_bytes("hello").expect("a body"),
            std::sync::Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap();
        assert_eq!(line.index(), 7);
        line.set_handle_mtime(Some(1_700_000_000_000_000_000));
        line.set_dropped_byte_size(Some(12));
        assert_eq!(line.mtime().unwrap(), Some(1_700_000_000_000_000_000));
        assert_eq!(line.dropped_byte_size(), Some(12));
        assert_eq!(line.body(), "hello");
    }

    #[test]
    fn a_path_segment_naming_nothing_addressable_misses_rather_than_panics() {
        let line = TextLine::from_bytes(
            0,
            TextBytes::from_bytes("body").expect("a body"),
            std::sync::Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap()
        .with_entries(TextEntries::from_iter([entry("a", "1")]));
        let by_index = FieldPath::new([FieldSegment::index(0)]);
        assert!(line.get_entry_by_path(&by_index).is_some());
        let deep = FieldPath::new([FieldSegment::index(0), FieldSegment::field("x")]);
        assert!(line.get_entry_by_path(&deep).is_none());
    }

    /// Every pair a body declares, rendered as the line wrote it.
    fn read(body: &[u8]) -> Vec<String> {
        let body = TextBytes::from_bytes(body).expect("a body");
        yggdryl::text::TextEntries::from_bytes(&body)
            .as_ref()
            .map(TextEntries::as_slice)
            .unwrap_or_default()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    fn tree(body: &[u8]) -> TextEntries {
        let body = TextBytes::from_bytes(body).expect("a body");
        yggdryl::text::TextEntries::from_bytes(&body).expect("the line states pairs")
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
            entries.as_slice()[2].key_bytes().as_str(),
            Some("NoAllocs[0].79"),
            "an indexed key is a key, and the loose walk found neither of these"
        );
        assert_eq!(
            entries.as_slice()[3].key_bytes().as_str(),
            Some("Symbol[0]")
        );
    }

    #[test]
    fn a_text_field_quoting_pairs_is_one_field_carrying_a_tree_of_them() {
        // The frame decides where the Text field ends; what that field's own
        // text says is read under it, which is what a pair-shaped value has
        // always meant here. The two readings do not compete: `58` is one
        // field of the frame, and `A` is a member of what `58` says.
        let entries = tree(b"8=FIX.4.4|35=D|58=quoting #A=1 and #B=2|10=0|");
        let quoting = &entries.as_slice()[2];
        assert_eq!(
            quoting.value_bytes().as_str(),
            Some("quoting #A=1 and #B=2")
        );
        let quoted = quoting
            .entries()
            .expect("the value states pairs")
            .as_slice();
        assert_eq!(quoted.len(), 2);
        assert_eq!(quoted[0].key_bytes().as_str(), Some("A"));
        assert!(
            quoted[0].marked() && quoted[1].marked(),
            "the quoted keys carry the mark the text wrote in front of them"
        );
        assert_eq!(
            entries
                .get_entry_by_path(&path("58"))
                .and_then(|found| found.value_bytes().as_str()),
            Some("quoting #A=1 and #B=2"),
            "and the frame's own field is what the frame's own key reaches"
        );
    }

    #[test]
    fn a_value_a_frame_bounded_is_still_a_range_of_the_page_it_came_from() {
        let page = page(b"8=FIX.4.4|58=a value with spaces|10=0|");
        let body = TextBytes::from_whole_page(Arc::clone(&page)).expect("the whole page");
        let entries = yggdryl::text::TextEntries::from_bytes(&body).expect("pairs");
        let held = &entries.as_slice()[1];
        assert_eq!(held.value(), "a value with spaces");
        assert!(
            Arc::ptr_eq(held.value_bytes().page().expect("a page"), &page),
            "a wider value is a wider range, never a copy"
        );
    }

    #[test]
    fn an_entry_records_the_mark_the_line_wrote_and_keeps_its_key_stripped() {
        let entries = tree(b"MSGTYPE=D|ORDERID=123|#ORDERID=123|#SIDE=1");
        let held = entries.as_slice();
        assert!(!held[1].marked() && held[2].marked());
        assert_eq!(
            held[1].key_bytes().as_str(),
            held[2].key_bytes().as_str(),
            "the mark is off the key, so a path still lifts the name the \
             bridge gave the field"
        );
        assert_eq!(
            entries
                .get_entry_by_path(&path("ORDERID"))
                .and_then(|found| found.value_bytes().as_str()),
            Some("123"),
            "and the name reaches the pair that arrived first"
        );
        assert!(
            held[3].marked() && !held[3].key_bytes().as_bytes().starts_with(b"#"),
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
            hash_of(bare),
            hash_of(marked),
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
        assert_eq!(differing.as_slice()[2].value_bytes().as_str(), Some("345"));
    }

    #[test]
    fn a_marked_stem_and_its_occurrence_arrive_marked_and_nested() {
        let entries = tree(b"MSGTYPE=D|#NOPARTYIDS=3|#NOPARTYIDS[0]=PARTYID=ONE");
        let held = entries.as_slice();
        assert!(held[1].marked() && held[2].marked());
        assert_eq!(held[1].key_bytes().as_str(), Some("NOPARTYIDS"));
        assert_eq!(held[2].key_bytes().as_str(), Some("NOPARTYIDS[0]"));
        assert_eq!(
            held[2]
                .entries()
                .and_then(|nested| nested.as_slice().first())
                .and_then(|member| member.value_bytes().as_str()),
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

// --- The line as an event: every reading resolved from the body on ask ---

mod event {
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event, EventIterator};
    use yggdryl::text::{
        DEFAULT_TEXT_BATCH_BYTE_SIZE, DEFAULT_TEXT_BATCH_ROW_SIZE, TextBytes, TextEntries,
        TextLine, TextOptions, into_arrow_batch, read_text_lines,
    };
    use yggdryl::{FieldPath, Scalar, Uuid};

    use super::named;

    /// The header naming every fact an event reads off a capture.
    const HEADER: &str = concat!(
        r"^(?<mtime>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z) ",
        r"\[(?<state>[A-Za-z]+)\] (?<seqnum>\d+) (?<prevuuid>[0-9a-f-]{36}) (?<crosscode>\S+) ",
    );
    /// The same facts, each capture admitting any spelling, so a line that
    /// spells one wrongly still matches and the fact's own reading refuses.
    const LOOSE: &str = r"^(?<mtime>\S+) \[(?<state>[^\]]+)\] (?<seqnum>\S+) (?<prevuuid>\S+) ";
    /// A chain's lines: an instant, a state and the code every line shares.
    const CHAIN: &str = r"^(?<mtime>\S+) \[(?<state>[A-Za-z]+)\] (?<crosscode>\S+) ";
    const INSTANT: i64 = 1_767_348_930_000_000_000;
    const PREVIOUS: &str = "0198a3b2-1c4d-7e5f-8a9b-0c1d2e3f4a5b";

    fn options() -> Arc<TextOptions> {
        Arc::new(
            TextOptions::new()
                .try_with_rowheader(HEADER)
                .expect("a header"),
        )
    }

    fn line(body: &str, options: &Arc<TextOptions>) -> TextLine {
        TextLine::from_bytes(
            0,
            TextBytes::from_bytes(body).expect("a page"),
            Arc::clone(options),
        )
        .expect("a line")
    }

    #[test]
    fn a_new_value_batches_on_rows_or_bytes_whichever_binds_first() {
        let options = TextOptions::new();
        assert_eq!(options.batch_row_size, Some(DEFAULT_TEXT_BATCH_ROW_SIZE));
        assert_eq!(options.batch_byte_size, Some(DEFAULT_TEXT_BATCH_BYTE_SIZE));
        assert_eq!(DEFAULT_TEXT_BATCH_ROW_SIZE, 35 * 1024);
        assert_eq!(DEFAULT_TEXT_BATCH_BYTE_SIZE, 64 * 1024 * 1024);
    }

    #[test]
    fn every_reading_resolves_from_the_body_under_the_header() {
        let options = options();
        let body = format!("2026-01-02T10:15:30Z [Filled] 7 {PREVIOUS} O-100 k=v|x=y");
        let line = line(&body, &options);
        // The body is the whole line; the payload is what follows the header.
        assert_eq!(line.body(), body);
        assert_eq!(line.payload_bytes().as_str(), Some("k=v|x=y"));
        assert_eq!(line.entries().map(yggdryl::text::TextEntries::len), Some(2));
        // The captures, in the order the header declares them, and the
        // identifiers the named ones make.
        assert_eq!(line.capture(0), Some("2026-01-02T10:15:30Z"));
        assert_eq!(line.capture(4), Some("O-100"));
        assert_eq!(line.get_identifiers()["state"], "Filled");
        assert_eq!(line.get_identifiers()["prevuuid"], PREVIOUS);
        assert_eq!(line.get_identifiers().len(), 5);
        // Each event fact, read off the capture of its name at its own type.
        assert_eq!(line.mtime().unwrap(), Some(INSTANT));
        assert_eq!(line.get_currunix(), INSTANT);
        assert!(line.get_state().is_done());
        assert_eq!(line.get_seqnum(), 7);
        let Scalar::Uuid(previous) = yggdryl::DataType::Uuid
            .scalar(Scalar::from(PREVIOUS))
            .expect("a uuid")
        else {
            panic!("a uuid")
        };
        assert_eq!(line.get_prevuuid(), Some(previous));
        assert_eq!(line.get_crosscode(), "O-100");
        assert_eq!(line.get_crosshashcode(), yggdryl::xxhash::xxh3(b"O-100"));
        assert_eq!(line.get_crossuuid(), line.cross_uuid());
        assert_ne!(line.get_crossuuid(), line.get_curruuid());
        assert_eq!((line.get_creaunix(), line.get_expirunix()), (None, None));
        assert_eq!((line.get_prevunix(), line.get_snapunix()), (None, None));
        // The identity: the body's own digest, coupled with the instant.
        assert_eq!(
            line.get_currhashcode(),
            yggdryl::xxhash::xxh3(body.as_bytes())
        );
        assert_eq!(line.get_curruuid(), line.time_uuid().expect("an identity"));
        // A line is read from a handle: no source, and no parent until a
        // walk states one.
        assert!(line.get_srcuuids().is_empty() && line.get_parentuuids().is_empty());
        // Two lines stating the same bytes at the same instant are one identity.
        assert_eq!(
            line.get_curruuid(),
            self::line(&body, &options).get_curruuid()
        );
    }

    #[test]
    fn a_line_the_header_does_not_date_stands_at_the_handles_time_else_the_epoch() {
        let options = Arc::new(TextOptions::new());
        let mut line = line("plain", &options);
        assert_eq!(line.mtime().unwrap(), None);
        assert_eq!(line.get_currunix(), 0);
        assert!(line.captures().is_empty());
        assert!(line.get_identifiers().is_empty());
        assert_eq!(line.get_crosscode(), "");
        assert_eq!(line.get_crosshashcode(), 0);
        assert_eq!(line.get_crossuuid(), line.get_curruuid());
        assert_eq!(line.get_state().as_str(), "00UNKNOWN");
        // The place in the chain is the row number under `start_rownum`,
        // else the physical line number.
        assert_eq!(line.get_seqnum(), 0);
        line.set_index(3);
        assert_eq!(line.get_seqnum(), 3);
        let mut numbered = TextOptions::new();
        numbered.start_rownum = Some(10);
        let numbered = self::line("plain", &Arc::new(numbered));
        assert_eq!(numbered.get_seqnum(), 10);
        // The handle's own time dates it, and the identity follows.
        let before = line.get_curruuid();
        line.set_handle_mtime(Some(INSTANT));
        assert_eq!(line.mtime().unwrap(), Some(INSTANT));
        assert_eq!(line.get_currunix(), INSTANT);
        assert_ne!(line.get_curruuid(), before);
        assert_eq!(line.get_curruuid(), line.time_uuid().expect("an identity"));
    }

    #[test]
    fn a_capture_that_does_not_read_as_its_fact_refuses_by_name() {
        let options = Arc::new(
            TextOptions::new()
                .try_with_rowheader(LOOSE)
                .expect("a header"),
        );
        let line = line("nope [what] x y body", &options);
        for (name, refused) in [
            ("mtime", line.mtime().err()),
            ("state", line.state().err()),
            ("seqnum", line.seqnum().err()),
            ("prevuuid", line.prevuuid().err()),
        ] {
            let refused = refused
                .unwrap_or_else(|| panic!("{name} refuses"))
                .to_string();
            assert!(
                refused.contains(&format!("$[0].{name}")),
                "{name}: {refused}"
            );
            assert!(refused.contains("physical line 1"), "{name}: {refused}");
        }
        // The trait door cannot refuse: it answers each fact's default.
        assert_eq!(line.get_currunix(), 0);
        assert_eq!(line.get_state().as_str(), "00UNKNOWN");
        assert_eq!(line.get_seqnum(), 0);
        assert_eq!(line.get_prevuuid(), None);
        // And the column built from the reading refuses the same way.
        let error = into_arrow_batch([line], &options)
            .expect_err("the mtime column refuses")
            .to_string();
        assert!(error.contains("$[0].mtime"), "{error}");
    }

    #[test]
    fn a_stated_fact_wins_and_a_new_body_drops_every_reading() {
        let options = options();
        let body = format!("2026-01-02T10:15:30Z [New] 1 {PREVIOUS} O-100 k=v");
        let mut line = line(&body, &options);
        let resolved = line.get_curruuid();
        line.set_state(yggdryl::State::from_spelling("Filled").expect("a state"));
        line.set_seqnum(9);
        line.set_srcuuids(vec![Uuid::from_v8(70)]);
        assert!(line.get_state().is_done());
        assert_eq!(line.get_seqnum(), 9);
        assert_eq!(
            line.get_curruuid(),
            resolved,
            "a stated fact is not the identity's"
        );
        // A new body: the readings resolve afresh from it, and the stated
        // facts stand.
        line.set_body(TextBytes::from_bytes("plain").expect("a page"))
            .expect("a body");
        assert_ne!(line.get_curruuid(), resolved);
        assert_eq!(line.get_currhashcode(), yggdryl::xxhash::xxh3(b"plain"));
        assert_eq!(line.mtime().unwrap(), None, "the header no longer matches");
        assert!(line.get_state().is_done(), "stated, so it stands");
        assert_eq!(line.get_seqnum(), 9);
        assert_eq!(line.get_srcuuids(), [Uuid::from_v8(70)]);
        // The stated identity is dropped by finalizing, which derives it.
        line.set_curruuid(Uuid::from_v8(1));
        assert_eq!(line.get_curruuid(), Uuid::from_v8(1));
        line.finalize();
        assert_eq!(line.get_curruuid(), line.time_uuid().expect("an identity"));
    }

    /// A line is read across threads as any event is: the slots its readings
    /// resolve into are shared and sent with it.
    #[test]
    fn a_line_is_sent_and_shared_across_threads() {
        fn sent_and_shared<T: Send + Sync>() {}
        sent_and_shared::<TextLine>();
    }

    #[test]
    fn a_stated_tree_stands_over_a_new_body_and_a_matched_header_does_not() {
        let options = TextOptions::new()
            .try_with_rowheader(r"^\[(?<level>[A-Z]+)\] ")
            .expect("a header")
            .with_max_record_byte_size(64);
        let page = |text: &str| TextBytes::from_bytes(text).expect("a page");
        // A line the cut matched: the header's end and captures were read
        // over the body the reader cut, so a new body reads them afresh, and
        // the tree with them.
        let mut cut = read_text_lines(&named("cut.log", b"[INFO] a=1|b=2\n"), &options)
            .expect("a reader")
            .next()
            .expect("a line")
            .expect("a line");
        assert_eq!(cut.capture(0), Some("INFO"));
        assert_eq!(cut.payload_bytes().as_str(), Some("a=1|b=2"));
        cut.set_body(page("[DEBUG] c=3")).expect("a body");
        assert_eq!(cut.capture(0), Some("DEBUG"), "read off the new body");
        assert_eq!(cut.payload_bytes().as_str(), Some("c=3"));
        assert_eq!(cut.entries().map(TextEntries::len), Some(1));
        // Captures a caller stated are the line's word, and stand over every
        // body: the payload is then the whole of the new one.
        let options = Arc::new(options);
        let mut line =
            TextLine::from_bytes(0, page("[INFO] a=1|b=2"), Arc::clone(&options)).expect("a line");
        line.set_captures(vec![Some(page("WARN"))])
            .expect("captures");
        assert_eq!(line.capture(0), Some("WARN"), "stated, the captures win");
        assert_eq!(line.entries().map(TextEntries::len), Some(2));
        line.set_body(page("[DEBUG] c=3")).expect("a body");
        assert_eq!(line.capture(0), Some("WARN"), "stated, the captures stand");
        assert_eq!(line.body(), "[DEBUG] c=3");
        assert_eq!(line.payload_bytes().as_str(), Some("[DEBUG] c=3"));
        assert_eq!(line.entries().map(TextEntries::len), Some(1));
        // A stated tree stands over every body, and so does a stated absence.
        line.set_entries(TextEntries::from_bytes(&page("z=9")));
        line.set_body(page("[DEBUG] c=3|d=4")).expect("a body");
        assert_eq!(
            line.get_entry_by_path(&FieldPath::from_str("z").unwrap())
                .map(|held| held.value().into_owned()),
            Some("9".to_owned()),
            "stated, the tree stands"
        );
        line.set_entries(None);
        assert!(line.entries().is_none(), "cleared, no tree stands");
        // A tree handed out for mutation is the line's word from then on.
        let mut mutated =
            TextLine::from_bytes(1, page("[INFO] a=1"), Arc::clone(&options)).expect("a line");
        mutated
            .set_entry_by_path(&FieldPath::from_str("b").unwrap(), page("2"))
            .expect("an entry");
        mutated.set_body(page("[INFO] c=3")).expect("a body");
        assert_eq!(
            mutated.entries().map(TextEntries::len),
            Some(2),
            "mutated, the tree stands"
        );
    }

    #[test]
    fn the_identity_never_reads_the_cross_hash_the_cross_element_or_a_source() {
        let options = options();
        let body = format!("2026-01-02T10:15:30Z [New] 1 {PREVIOUS} O-100 k=v");
        let stated = line(&body, &options);
        let mut crossed = line(&body, &options);
        crossed.set_crosshashcode(0xCD);
        crossed.set_crossuuid(Uuid::from_v8(77));
        crossed.set_srcuuids(vec![Uuid::from_v8(70)]);
        assert_eq!(crossed.get_currhashcode(), stated.get_currhashcode());
        assert_eq!(crossed.get_curruuid(), stated.get_curruuid());
        assert_eq!(
            crossed.get_crossuuid(),
            Uuid::from_v8(77),
            "stated, so answered"
        );
        crossed.finalize();
        assert_eq!(crossed.get_curruuid(), stated.get_curruuid());
        assert_eq!(crossed.get_crosshashcode(), stated.get_crosshashcode());
        assert_eq!(crossed.get_crossuuid(), stated.get_crossuuid());
    }

    #[test]
    fn equality_reads_the_stated_facts_and_never_a_resolved_slot() {
        let options = options();
        let body = format!("2026-01-02T10:15:30Z [New] 1 {PREVIOUS} O-100 k=v");
        let left = line(&body, &options);
        let mut right = line(&body, &options);
        assert_eq!(left, right);
        let _ = right.captures();
        let _ = right.get_curruuid();
        let _ = right.entries();
        assert_eq!(left, right, "a resolved slot is not a fact");
        right.set_seqnum(4);
        assert_ne!(left, right, "a stated one is");
    }

    #[test]
    fn lines_walk_as_events_and_carry_their_lineage() {
        let options = Arc::new(
            TextOptions::new()
                .try_with_rowheader(CHAIN)
                .expect("a header"),
        );
        let read =
            |instant: &str, state: &str| line(&format!("{instant} [{state}] O-100 k=v"), &options);
        let arrived = vec![
            read("2026-01-02T10:15:30Z", "New"),
            read("2026-01-02T10:15:31Z", "PartiallyFilled"),
            read("2026-01-02T10:15:32Z", "Filled"),
        ];
        let walked: Vec<TextLine> = EventIterator::new(arrived, true).collect();
        let [first, second, third] = walked.as_slice() else {
            panic!("three lines")
        };
        assert_eq!((first.get_seqnum(), first.get_prevuuid()), (0, None));
        assert_eq!(second.get_prevuuid(), Some(first.get_curruuid()));
        assert_eq!(second.get_prevunix(), Some(first.get_currunix()));
        assert_eq!(second.get_seqnum(), 1);
        assert_eq!(second.get_parentuuids(), [first.get_curruuid()]);
        assert_eq!(
            third.get_parentuuids(),
            [first.get_curruuid(), second.get_curruuid()]
        );
        assert!(third.get_state().is_done());
        assert!(walked.iter().all(|line| line.get_srcuuids().is_empty()));
        assert!(
            walked
                .iter()
                .all(|line| line.get_crossuuid() == first.get_crossuuid())
        );
    }

    #[test]
    fn a_read_line_reads_as_the_batch_reads_it() {
        let options = TextOptions::new()
            .try_with_rowheader(HEADER)
            .expect("a header");
        let text = format!("2026-01-02T10:15:30Z [Filled] 7 {PREVIOUS} O-100 k=v\n");
        let source = named("events.log", text.as_bytes());
        let lines: Vec<TextLine> = read_text_lines(&source, &options)
            .expect("a reader")
            .map(|line| line.expect("a line"))
            .collect();
        assert_eq!(lines[0].get_currunix(), INSTANT);
        assert_eq!(lines[0].get_seqnum(), 7);
        assert!(lines[0].sourceurl().is_some());
        let batch = into_arrow_batch(lines.clone(), &options).expect("a batch");
        let schema = batch.schema();
        let names: Vec<&str> = schema
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect();
        // The batch opens with the sixteen event columns the line is stated
        // in, and a capture named for an event fact - `state`, `seqnum`,
        // `prevuuid`, `crosscode` - feeds that column rather than trailing
        // beside it as a column of its own.
        assert_eq!(names, super::with_event(&["sourceurl", "mtime", "body"]));
        // Every fact the line read off its captures is what the column
        // states, at the fact's own datatype rather than as the text the
        // capture held.
        let cell = |name: &str| batch.column_by_name(name).expect(name).clone();
        assert_eq!(
            cell("seqnum")
                .as_any()
                .downcast_ref::<arrow_array::UInt64Array>()
                .expect("a count")
                .value(0),
            7
        );
        assert_eq!(
            cell("crosscode")
                .as_any()
                .downcast_ref::<arrow_array::StringArray>()
                .expect("a code")
                .value(0),
            "O-100"
        );
        assert_eq!(
            cell("state")
                .as_any()
                .downcast_ref::<arrow_array::StringArray>()
                .expect("a state")
                .value(0),
            lines[0].get_state().as_str()
        );
        // And a line read back out of the batch states every one of them
        // again, the identity a message named as its source included.
        let back = yggdryl::text::from_arrow_batch(&batch, &options).expect("lines read back");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].get_currunix(), INSTANT);
        assert_eq!(back[0].get_seqnum(), 7);
        assert!(back[0].get_state().is_done());
        assert_eq!(back[0].get_prevuuid(), lines[0].get_prevuuid());
        assert_eq!(back[0].get_crosscode(), "O-100");
        assert_eq!(back[0].get_curruuid(), lines[0].get_curruuid());
        assert_eq!(back[0].get_crossuuid(), lines[0].get_crossuuid());
        assert_eq!(back[0].get_currhashcode(), lines[0].get_currhashcode());
        assert_eq!(back[0].get_identifiers(), lines[0].get_identifiers());
    }

    #[test]
    fn a_batch_refuses_a_capture_the_event_column_cannot_hold_by_the_facts_name() {
        // The column states the fact at the fact's own datatype, so a
        // capture spelled as the fact and not readable as it refuses the
        // batch by the fact's name, where a capture column would have
        // carried the text: one owner per fact, and one type.
        let options = TextOptions::new()
            .try_with_rowheader(LOOSE)
            .expect("a header");
        let text = format!("2026-01-02T10:15:30Z [Filled] seven {PREVIOUS} k=v\n");
        let source = named("events.log", text.as_bytes());
        let lines: Vec<TextLine> = read_text_lines(&source, &options)
            .expect("a reader")
            .map(|line| line.expect("a line"))
            .collect();
        // The trait door answers the default over the refusal...
        assert_eq!(lines[0].get_seqnum(), 0);
        // ...and the batch, built from the refusing reading, names the fact.
        let error = into_arrow_batch(lines, &options)
            .expect_err("a seqnum that is not a count")
            .to_string();
        assert!(error.contains("seqnum"), "{error}");
        assert!(error.contains("seven"), "{error}");
        assert!(error.contains("physical line 1"), "{error}");
    }
}

// --- The one decode path, the plan, and the two new options ---

mod decoding {

    use arrow_array::{Array as _, StringArray};
    use yggdryl::text::{TextEntries, TextOptions};

    use yggdryl::FieldPath;
    use yggdryl::text::{into_arrow_batch, read_text_lines};

    use super::named;

    fn lines(source: &[u8], options: &TextOptions) -> Vec<yggdryl::text::TextLine> {
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
        assert_eq!(decoded[1].body(), "second");
        // The URL is one shared value, not one rebuilt per line.
        assert_eq!(
            decoded[0].sourceurl().map(ToString::to_string),
            decoded[2].sourceurl().map(ToString::to_string)
        );
    }

    #[test]
    fn a_read_asking_for_no_entry_resolves_the_tree_on_the_first_ask_alone() {
        // Nothing is resolved until asked, and the ask resolves it: the tree
        // the line states, and what the line is classified as. That a read
        // never pays for what it never asks is the allocation suite's pin,
        // `a_line_built_and_read_allocates_nothing_and_its_captures_once`.
        let options = TextOptions::new();
        let decoded = lines(b"35=D|55=AAPL\n", &options);
        assert_eq!(
            decoded[0].entries().map(TextEntries::len),
            Some(2),
            "asked, the tree is read off the body"
        );
        assert_eq!(decoded[0].bodytype().as_str(), "text/fix");
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
                .and_then(|held| held.value_bytes().as_str()),
            Some("AAPL")
        );

        let batch = into_arrow_batch(decoded, &options).expect("a batch builds");
        let column = batch
            .column_by_name("55")
            .expect("the lifted column is there");
        assert_eq!(
            column
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("lifted values are text")
                .value(0),
            "AAPL"
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
            .with_renamed_column("sourceurl", "source");
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
                .downcast_ref::<StringArray>()
                .expect("still text")
                .value(0),
            "hello"
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
        let options = TextOptions::new().with_renamed_column("body", "sourceurl");
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
                .downcast_ref::<StringArray>()
                .expect("lifted values are text")
                .value(0),
            "AAPL"
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
        let names: Vec<&str> = children.iter().map(yggdryl::Field::name).collect();
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
        assert_eq!(decoded[0].bodytype().as_str(), "text/fix");
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
        let expected: Vec<&str> = declared_fields.iter().map(yggdryl::Field::name).collect();
        assert_eq!(
            names, expected,
            "the plan and the schema are one derivation"
        );
    }

    #[test]
    fn an_empty_object_answers_its_columns_and_no_rows() {
        let options = TextOptions::new();
        let batch =
            into_arrow_batch(Vec::<yggdryl::text::TextLine>::new(), &options).expect("a batch");
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

    use yggdryl::FieldPath;
    use yggdryl::graph::Event as _;
    use yggdryl::text::TextOptions;
    use yggdryl::text::{from_arrow_batch, from_arrow_reader};
    use yggdryl::text::{into_arrow_batch, read_text_lines};

    use super::named;

    fn decode(source: &[u8], options: &TextOptions) -> Vec<yggdryl::text::TextLine> {
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
                read.sourceurl().map(ToString::to_string),
                original.sourceurl().map(ToString::to_string)
            );
            assert_eq!(read.mtime().unwrap(), original.mtime().unwrap());
        }
    }

    #[test]
    fn a_column_named_the_way_someone_else_writes_it_is_still_found() {
        let options = TextOptions::new();
        // A producer that calls the body `payload` and the object `source`.
        let renamed = TextOptions::new()
            .with_renamed_column("body", "payload")
            .with_renamed_column("sourceurl", "source");
        let lines = decode(b"hello\n", &renamed);
        let foreign = into_arrow_batch(lines, &renamed).expect("a batch");

        // Read under the ordinary names: the aliases carry it.
        let back = from_arrow_batch(&foreign, &options).expect("lines read back");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].body(), "hello");
        assert!(back[0].sourceurl().is_some(), "source resolved to url");
    }

    #[test]
    fn a_case_difference_alone_still_matches() {
        let renamed = TextOptions::new().with_renamed_column("body", "BODY");
        let lines = decode(b"hello\n", &renamed);
        let batch = into_arrow_batch(lines, &renamed).expect("a batch");
        let back = from_arrow_batch(&batch, &TextOptions::new()).expect("lines read back");
        assert_eq!(back[0].body(), "hello");
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
        assert_eq!(back[0].body(), "only");
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
                .and_then(|held| held.value_bytes().as_str()),
            Some("AAPL")
        );
    }

    #[test]
    fn streamed_batches_read_back_one_at_a_time() {
        let mut options = TextOptions::new();
        options.batch_row_size = Some(1);
        let lines = decode(b"a\nb\nc\n", &options);
        let reader = yggdryl::text::into_arrow_reader(
            lines.into_iter().map(Ok).collect::<Vec<_>>(),
            &options,
        )
        .expect("a reader");
        let back: Vec<_> = from_arrow_reader(reader, &options)
            .expect("a settled configuration")
            .map(|line| line.expect("a line"))
            .collect();
        assert_eq!(back.len(), 3);
        assert_eq!(back[2].body(), "c");
    }

    #[test]
    fn a_stated_instant_is_the_rows_word_over_the_headers() {
        // The mtime column states the instant a line read back answers, over
        // whatever its own header would read: the row's word is the fact.
        let options = TextOptions::new()
            .try_with_rowheader(r"^(?<mtime>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z) ")
            .expect("a header");
        let lines = decode(b"2026-01-02T10:15:30Z body\n", &options);
        let batch = into_arrow_batch(lines.clone(), &options).expect("a batch");
        let back = from_arrow_batch(&batch, &options).expect("lines read back");
        assert_eq!(back[0].mtime().unwrap(), lines[0].mtime().unwrap());
        assert_eq!(back[0].body(), lines[0].body());
        let mut restated = back[0].clone();
        restated.set_currunix(7);
        assert_eq!(restated.mtime().unwrap(), Some(7));
        assert_eq!(restated.get_currunix(), 7);
    }
}

#[test]
fn a_text_read_is_shaped_by_its_select_and_where_sections() {
    // The select casts, aliases and reads the row header's captures; the
    // where clause names an alias, so it runs after the projection.
    let source = named(
        "app.log",
        b"[INFO] id=7 first\n[WARN] id=9 second\n[INFO] id=11 third\nplain\n",
    );
    let mut options = options(r"\[(?<level>[A-Z]+)\] id=(?<id>\d+)");
    options.start_rownum = Some(1);
    let options = options
        .with_select(
            "sourceurl, cast(rownum as int32) as n, trim(body) as line, level, id * 10 as tenfold int64",
        )
        .unwrap()
        .with_filter("n > 1 and line like '%d' and level is not null")
        .unwrap();
    let batches = collect(&source, options);
    let batch = arrow_select::concat::concat_batches(&batches[0].schema(), &batches).unwrap();
    let names: Vec<_> = batch
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect();
    assert_eq!(names, ["sourceurl", "n", "line", "level", "tenfold"]);
    assert_eq!(batch.column(1).data_type(), &arrow_schema::DataType::Int32);
    let lines = batch
        .column(2)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap()
        .iter()
        .map(|line| line.unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(lines, ["[WARN] id=9 second", "[INFO] id=11 third"]);
    let tenfold = batch
        .column(4)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap()
        .values()
        .to_vec();
    assert_eq!(tenfold, [90, 110]);
    // A buffer is located in memory, and the column says so.
    let url = batch
        .column(0)
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert!(url.value(0).starts_with("mem://"), "{}", url.value(0));
}

/// A line with no body is no line, in either direction.
///
/// The `body` column is not nullable, and a cell holding the empty string
/// would make that promise hollow: a row stating nothing is a row nobody can
/// read back as the line it came from. So the reader never cuts one - a
/// blank line, or one the strips take whole, is a separator between records
/// and not a record - the line's own doors refuse one, and a write refuses a
/// row that carries one.
mod body {
    use super::{EVENT_COLUMNS, bodies, collect, named, options, rownums, strings, with_event};
    use yggdryl::IOMedia as _;
    use yggdryl::text::{Text, TextBytes, TextLine, TextOptions};

    fn line(body: &[u8]) -> yggdryl::Result<TextLine> {
        TextLine::from_bytes(
            7,
            TextBytes::from_bytes(body).expect("a page"),
            std::sync::Arc::new(TextOptions::new()),
        )
    }

    #[test]
    fn a_blank_line_is_a_separator_and_never_a_row() {
        let source = named("blanks.log", b"alpha\n\nbeta\n\n\n");
        let mut read = TextOptions::new();
        read.start_rownum = Some(0);
        let batches = collect(&source, read);
        assert_eq!(bodies(&batches), [b"alpha".to_vec(), b"beta".to_vec()]);
        // The numbering is the physical line's own, so the gap the blank
        // line left is visible rather than closed over.
        assert_eq!(rownums(&batches), [0, 2]);
    }

    #[test]
    fn a_line_the_strips_take_whole_is_no_record_either() {
        let source = named("stripped.log", b"xxx\nkeep\nxxx\n");
        let mut read = TextOptions::new();
        read.set_lstrip(["^x+"]).expect("an lstrip");
        read.start_rownum = Some(1);
        let batches = collect(&source, read);
        assert_eq!(bodies(&batches), [b"keep".to_vec()]);
        assert_eq!(rownums(&batches), [2]);
    }

    #[test]
    fn a_count_and_a_read_drop_the_same_lines() {
        // The counting pass keeps no body at all, so the two can only agree
        // if what makes a line a record is what it cut and never what the
        // retained limit kept.
        let source = named("blanks.log", b"alpha\n\nbeta\n\n\n");
        let counted = Text::new(source.clone()).with_options(TextOptions::new());
        assert_eq!(counted.row_size().unwrap(), 2);
        let batches = collect(&source, TextOptions::new());
        let rows: usize = batches.iter().map(arrow_array::RecordBatch::num_rows).sum();
        assert_eq!(rows, 2);
    }

    #[test]
    fn the_line_doors_refuse_a_body_that_states_nothing() {
        let refusal = line(b"").expect_err("no body, no line");
        assert!(refusal.to_string().contains("$[7].body"), "{refusal}");
        assert!(
            refusal.to_string().contains("got an empty one"),
            "{refusal}"
        );
        let mut held = line(b"alpha").expect("a line");
        let refusal = held
            .set_body(TextBytes::new())
            .expect_err("no body, no line");
        assert!(refusal.to_string().contains("$[7].body"), "{refusal}");
        // And the line is the line it was: a refused write changes nothing.
        assert_eq!(held.body(), "alpha");
    }

    #[test]
    fn a_retained_limit_that_keeps_no_body_is_refused_before_a_byte_is_read() {
        // With no row header there is nothing a record is known by, so a
        // limit of zero would answer a line with no body on every row.
        let refusal = named("any.log", b"alpha\n")
            .read_arrow_reader(&TextOptions::new().with_max_record_byte_size(0).into())
            .map(drop)
            .expect_err("a limit that keeps nothing");
        assert!(
            refusal.to_string().contains("max_record_byte_size"),
            "{refusal}"
        );
        // Under a row header the header is always retained, so the same
        // limit keeps the bytes the record is known by and reads.
        let batches = collect(
            &named("headed.log", b"[A] alpha\n[B] beta\n"),
            options(r"^\[(?<kind>[A-Z])\] ").with_max_record_byte_size(0),
        );
        assert_eq!(bodies(&batches), [b"[A] ".to_vec(), b"[B] ".to_vec()]);
        assert_eq!(
            strings(&batches, "kind"),
            [Some("A".into()), Some("B".into())]
        );
    }

    #[test]
    fn a_row_that_states_no_body_is_refused_on_the_way_back_out() {
        let source = named("round.log", b"alpha\nbeta\n");
        let batches = collect(&source, TextOptions::new());
        let schema = batches[0].schema();
        let bodies_at = schema.index_of("body").unwrap();
        let mut columns = batches[0].columns().to_vec();
        columns[bodies_at] = std::sync::Arc::new(arrow_array::StringArray::from(vec![
            Some("alpha"),
            Some(""),
        ]));
        let hollow =
            arrow_array::RecordBatch::try_new(std::sync::Arc::clone(&schema), columns).unwrap();
        let mut target = named("written.log", b"");
        let refusal = target
            .write_arrow_reader(
                yggdryl::arrow::batch_reader(schema, vec![hollow]),
                yggdryl::IOMode::Overwrite,
                &TextOptions::new().into(),
            )
            .map(drop)
            .expect_err("a row stating no line");
        assert!(refusal.to_string().contains("$[1].body"), "{refusal}");
    }

    #[test]
    fn every_emitted_column_states_its_nullability_and_says_what_it_holds() {
        let source = named("columns.log", b"2026-01-02T03:04:05Z INFO k=v hello\n");
        let mut read = options(r"^(?<mtime>\S+) (?<level>\w+) ")
            .with_max_record_byte_size(1_024)
            .try_with_lift_names(["k"])
            .expect("a lift");
        read.start_rownum = Some(1);
        read.parse_mimetype = true;
        let batches = collect(&source, read);
        let schema = batches[0].schema();
        assert_eq!(
            schema
                .fields()
                .iter()
                .map(|field| field.name().as_str())
                .collect::<Vec<_>>(),
            with_event(&[
                "sourceurl",
                "rownum",
                "mtime",
                "mimetype",
                "body",
                "dropped_byte_size",
                "level",
                "k",
            ])
        );
        // The five facts every event settles are the five a line always
        // states; everything a line may leave unsaid is nullable, and the
        // three the reader itself answers - which line it was, what it was
        // classified as, and the line - are not.
        let required = [
            "currunix",
            "curruuid",
            "crossuuid",
            "currhashcode",
            "crosshashcode",
        ];
        for field in schema.fields() {
            let expected = !(required.contains(&field.name().as_str())
                || matches!(field.name().as_str(), "rownum" | "mimetype" | "body"));
            assert_eq!(
                field.is_nullable(),
                expected,
                "{} nullability",
                field.name()
            );
            // Every column says what it holds, the ones a caller named
            // included, so a catalog reading this schema needs nothing else.
            assert!(
                field.metadata().contains_key("description"),
                "{} has no description",
                field.name()
            );
        }
        // And the sixteen a line opens with carry the spelling their own
        // column states, so a line's row and a message's row name one fact
        // one way.
        for name in EVENT_COLUMNS {
            let display = schema.field_with_name(name).unwrap().metadata()["display"].clone();
            assert!(display.eq_ignore_ascii_case(name), "{name} shows {display}");
        }
        // A column that cannot be null never is, whatever the line left
        // unsaid: this row states no level and no `k`.
        let bare = collect(&named("bare.log", b"unmatched\n"), {
            let mut read = options(r"^(?<mtime>\S+) (?<level>\w+) ");
            read.parse_mimetype = true;
            read.start_rownum = Some(1);
            read
        });
        for field in bare[0].schema().fields() {
            if !field.is_nullable() {
                assert_eq!(
                    bare[0].column_by_name(field.name()).unwrap().null_count(),
                    0,
                    "{} holds a null",
                    field.name()
                );
            }
        }
    }
}
