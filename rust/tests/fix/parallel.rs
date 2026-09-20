//! The threads a codec reads on change when a capture is answered, never
//! what: every door answers on four threads exactly what it answers on one.

use std::sync::Arc;

use arrow_array::RecordBatch;
use yggdryl::arrow::BatchReader;
use yggdryl::graph::{Element, Event};
use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::text::{TextLine, TextOptions, read_text_lines};
use yggdryl::{FixCodec, FixMsg, IOMedia, Timezone, Url, fix_schema};

/// The bridge capture as the bytes a `.log` file holds, and the options
/// its rows are read under.
fn capture() -> (Buffer, TextOptions) {
    let source = Buffer::from_bytes(include_bytes!("ulbridge.log").to_vec()).with_media_type(
        Url::from_str("file:///ulbridge.log")
            .expect("a URL")
            .media_type(),
    );
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the bridge's row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_mimetype = true;
    (source, options)
}

fn lines(source: &Buffer, options: &TextOptions) -> Vec<TextLine> {
    read_text_lines(source, options)
        .expect("a line reader")
        .map(|line| line.expect("a line"))
        .collect()
}

fn messages(read: impl Iterator<Item = yggdryl::Result<FixMsg>>) -> Vec<Result<FixMsg, String>> {
    read.map(|held| held.map_err(|error| error.to_string()))
        .collect()
}

fn batches(reader: BatchReader) -> Vec<Result<RecordBatch, String>> {
    reader
        .map(|held| held.map_err(|error| error.to_string()))
        .collect()
}

/// Two readings of one capture, message for message.
fn same_messages(one: &[Result<FixMsg, String>], four: &[Result<FixMsg, String>]) {
    assert_eq!(one.len(), four.len(), "the same count of messages");
    for (at, (one, four)) in one.iter().zip(four).enumerate() {
        assert!(one == four, "message {at} differs: {one:?} vs {four:?}");
    }
}

/// Two readings of one set of rows, by what each row stated: its identity,
/// its instant and the wire it re-emits.
fn same_read(one: &[Result<FixMsg, String>], four: &[Result<FixMsg, String>]) {
    assert_eq!(one.len(), four.len(), "the same count of messages");
    let stated = |held: &Result<FixMsg, String>| {
        held.as_ref()
            .map(|message| {
                (
                    message.get_curruuid(),
                    message.get_currunix(),
                    message.into_bytes(b'|'),
                )
            })
            .map_err(Clone::clone)
    };
    for (at, (one, four)) in one.iter().zip(four).enumerate() {
        assert!(stated(one) == stated(four), "message {at} differs");
    }
}

/// Two readings of one capture, batch for batch.
fn same_batches(one: &[Result<RecordBatch, String>], four: &[Result<RecordBatch, String>]) {
    assert_eq!(one.len(), four.len(), "the same count of batches");
    for (at, (one, four)) in one.iter().zip(four).enumerate() {
        assert!(one == four, "batch {at} differs");
    }
}

/// Zero threads read as one, and the count is the codec's to state.
#[test]
fn the_threads_are_one_unless_stated_and_never_zero() {
    let codec = FixCodec::new(super::committed_registry());
    assert_eq!(codec.threads(), 1);
    assert_eq!(codec.clone().with_threads(0).threads(), 1);
    assert_eq!(codec.clone().with_threads(4).threads(), 4);
    let mut codec = codec;
    codec.set_threads(3);
    assert_eq!(codec.threads(), 3);
    assert_eq!(FixCodec::PARALLEL_CHUNK, 64);
}

/// Every door answers on four threads what it answers on one: the same
/// messages in the same order, and the same batches closing at the same
/// rows.
#[test]
fn every_door_answers_on_four_threads_what_it_answers_on_one() {
    let registry = super::committed_registry();
    let one =
        super::fixed_codec(Arc::clone(&registry)).with_exclude_msgtypes::<[&str; 0], &str>([]);
    let four = one.clone().with_threads(4);
    let (source, options) = capture();
    let composed = |codec: &FixCodec| codec.clone().with_capture_names(options.capture_names());

    // The line doors: text lines, and their bodies as bytes.
    let held = lines(&source, &options);
    let text_one = messages(composed(&one).parse_text_lines(held.iter()));
    let text_four = messages(composed(&four).parse_text_lines(held.iter()));
    assert!(
        text_one.len() > 50,
        "the capture carries messages: {}",
        text_one.len()
    );
    same_messages(&text_one, &text_four);
    let bodies: Vec<Vec<u8>> = held.iter().map(|line| line.body_bytes().to_vec()).collect();
    let bytes_one = messages(one.parse_lines(&bodies));
    let bytes_four = messages(four.parse_lines(&bodies));
    same_messages(&bytes_one, &bytes_four);
    assert!(text_four.iter().filter(|held| held.is_ok()).count() > 50);

    // The Arrow doors: rows parsed, rows read back as messages, and rows
    // written; the batches close on the same rows because the charging
    // reads the rows and never the threads.
    let record: RecordOptions = options.clone().into();
    let reader = || source.read_arrow_reader(&record).expect("a text reader");
    let parsed_one = batches(one.parse_text_arrow_reader(reader()).expect("a reader"));
    let parsed_four = batches(four.parse_text_arrow_reader(reader()).expect("a reader"));
    same_batches(&parsed_one, &parsed_four);
    let parsed: Vec<RecordBatch> = parsed_one
        .into_iter()
        .map(|held| held.expect("a batch"))
        .collect();
    let schema = parsed[0].schema();
    let rows = || yggdryl::arrow::batch_reader(schema.clone(), parsed.clone());
    // A row stating no `SendingTime` is dated by the clock the read
    // settles, so the header's clock is left out of the comparison and
    // everything the row stated is in it.
    same_read(
        &messages(one.messages(rows())),
        &messages(four.messages(rows())),
    );
    // The row door reads a batch's rows as the array reader answers them,
    // which is what `from_row` answers for the same row canonicalized: one
    // message, whichever way the row was read.
    let target = fix_schema(&registry, "fix").expect("the fixed schema");
    let held: Vec<FixMsg> = one
        .messages(rows())
        .map(|message| message.expect("a message"))
        .collect();
    let canonical: Vec<Result<FixMsg, String>> = held
        .iter()
        .map(|message| {
            let row = message.into_row(&target).expect("a row");
            FixMsg::from_row(Arc::clone(&registry), &target, &row)
                .map_err(|error| error.to_string())
        })
        .collect();
    let via_arrow = messages(
        one.messages(
            one.arrow_reader(target.clone(), held.clone())
                .expect("a reader"),
        ),
    );
    same_read(&via_arrow, &canonical);
    same_batches(
        &batches(one.lifecycle_arrow_reader(rows()).expect("a reader")),
        &batches(four.lifecycle_arrow_reader(rows()).expect("a reader")),
    );
    let written = |codec: &FixCodec| {
        batches(
            codec
                .arrow_reader(
                    target.clone(),
                    messages(one.messages(rows()))
                        .into_iter()
                        .map(|held| held.expect("a message")),
                )
                .expect("a reader"),
        )
    };
    same_batches(&written(&one), &written(&four));
}
