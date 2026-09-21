//! A bridge's own log, read as text records and then as FIX rows.
//!
//! The corpus is `rust/tests/fix/ulbridge.log` - a second of a ULBridge's
//! own capture, anonymized, and then every shape a bridge writes that the
//! second happened not to hold: a Jolokia exchange whose answer is a JSON
//! document the codec does not read, FIXML behind a verb, frames spelled with `^A` and
//! `<SOH>`, a `35=UL` frame packing a group inside a group, a bridge row
//! keyed by name, a statistics line, an empty body, a warning and a
//! cancel/reject flow - repeated
//! until the release corpus is about eleven megabytes, so the numbers are
//! per byte of a real capture rather than of one shape. Throughput is in
//! bytes of that log.
//!
//! Every stage runs over the same corpus, each on its own: the text reader
//! framing each line under the bridge's row header, the whole path into fixed
//! rows, the codec alone over the framed bodies, the record reader over the
//! same bodies with each row naming the plugin that logged it - so the
//! `msgpluginid` capture's fill is measured on its own - and then what a message
//! costs after it is built - its row, the batch the rows land in, the one
//! walk that joins it to its order's life, and its digest. A parse settles
//! everything a message derives about itself - the dictionary's latest
//! names, the derivations, the identifiers, the lanes, the identity - so
//! there is no pass after it but the walk.
//!
//! The registry is the shipped dictionary: the framed FIX lands on FIX's own
//! tags, and a JSON document the bridge wrote is one `unknown` row carrying
//! only what the row stated.

use std::hint::black_box;
use std::sync::Arc;

use criterion::{BatchSize, Criterion, Throughput};
use yggdryl::graph::{Element, Event};
use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::text::{TextBytes, TextLine, TextOptions, read_text_lines};
use yggdryl::{
    DataType, Field, FixCodec, FixMsg, IOMedia, State, StructType, Timezone, Url, fix_schema,
};

use super::seed;

/// The capture, exactly as the bridge wrote it; it ends in a newline, so
/// repeating it repeats whole lines.
const LOG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fix/ulbridge.log"
));

/// How many times the capture is repeated in one measured run.
const REPEATS: usize = crate::bench_profile::corpus(64, 1);

/// How many messages one copy of the capture carries.
///
/// Every codec below refuses nothing, so this is the whole capture and not
/// the 79 a live session reads: `DEFAULT_REFUSED_MSGTYPES` holds back the
/// keepalives and the rows that state no type, and those are shapes this
/// corpus exists to measure. `rust/tests/fix/dataset.rs` pins both numbers
/// against each other; every other reader of this capture - the integration
/// suite, the pages, the two bindings' suites - reads it the same way.
const MESSAGES: usize = 94;

/// The log, as the bytes a `.log` file holds.
fn corpus() -> Vec<u8> {
    LOG.repeat(REPEATS)
}

/// A handle whose media type comes from its name, so `.log` reads as records.
fn handle(bytes: &[u8]) -> Buffer {
    Buffer::from_bytes(bytes.to_vec()).with_media_type(
        Url::from_str("file:///ulbridge.log")
            .expect("a URL")
            .media_type(),
    )
}

/// The text options a bridge log is read under: the bridge's own row header
/// framed - its clock stamping each row, its bracket filling the session,
/// context and sequence columns - each line numbered, classified and read
/// for its direction.
fn text() -> RecordOptions {
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_mimetype = true;
    options.into()
}

/// The same bridge reader, bounded to make several input batches available
/// to the ordered parsing pool even in the debug smoke corpus.
fn batched_text(rows: usize) -> RecordOptions {
    let RecordOptions::Text(mut options) = text() else {
        unreachable!("the capture uses text options")
    };
    options.batch_row_size = Some(rows);
    RecordOptions::Text(options)
}

/// The bodies the text reader hands the codec, framed and stripped.
fn bodies(source: &Buffer) -> Vec<Vec<u8>> {
    use arrow_array::cast::AsArray;

    let mut held = Vec::new();
    for batch in source.read_arrow_reader(&text()).expect("a reader") {
        let batch = batch.expect("a batch");
        let at = batch.schema().index_of("body").expect("the body column");
        let column = batch.column(at).as_string::<i32>();
        for row in 0..batch.num_rows() {
            held.push(column.value(row).as_bytes().to_vec());
        }
    }
    held
}

pub fn benchmarks(criterion: &mut Criterion) {
    let bytes = corpus();
    let source = handle(&bytes);
    let registry = Arc::new(seed());
    let codec = FixCodec::new(Arc::clone(&registry))
        .with_threads(1)
        .with_exclude_msgtypes::<[&str; 0], &str>([]);
    let schema = fix_schema(&registry, "fix").expect("the fixed schema");

    let mut group = criterion.benchmark_group("fix/pipeline");
    group.throughput(Throughput::Bytes(bytes.len() as u64));

    // The first stage alone: lines framed under the row header, numbered,
    // classified and read for their direction.
    group.bench_function("text_read", |bencher| {
        bencher.iter(|| {
            black_box(&source)
                .read_arrow_reader(&text())
                .expect("a reader")
                .map(|batch| batch.expect("a batch").num_rows())
                .sum::<usize>()
        });
    });

    // The whole path: the text reader's batches read straight into FIX rows,
    // the capture's own columns carried in front of the tags.
    group.bench_function("parse_text_arrow_reader", |bencher| {
        bencher.iter(|| {
            let read = black_box(&source)
                .read_arrow_reader(&batched_text(8))
                .expect("a reader");
            codec
                .parse_text_arrow_reader(read)
                .expect("a reader")
                .map(|batch| batch.expect("a batch").num_rows())
                .sum::<usize>()
        });
    });
    // The same reader boundary without rebuilding Arrow batches: its small
    // input batches let the ordered parser pool receive real parallel work.
    group.bench_function("parse_arrow_messages", |bencher| {
        bencher.iter(|| {
            let read = black_box(&source)
                .read_arrow_reader(&batched_text(16))
                .expect("a reader");
            codec
                .parse_arrow_messages(read)
                .expect("messages")
                .filter(Result::is_ok)
                .count()
        });
    });
    let RecordOptions::Text(options) = text() else {
        unreachable!("the capture uses text options")
    };
    let composed = codec.clone().with_capture_names(options.capture_names());
    // The decoded line stream, read straight into messages, and then the
    // same stream walked: each message stated as the one after the live
    // message of its chain.
    let read_composed = || {
        composed
            .parse_text_lines(read_text_lines(&source, &options).expect("a decoded line stream"))
            .try_fold(0_usize, |read, message: yggdryl::Result<FixMsg>| {
                message.map(|_| read + 1)
            })
            .expect("a parsed message")
    };
    assert_eq!(read_composed(), MESSAGES * REPEATS);
    group.bench_function("decoded_lines", |bencher| {
        bencher.iter(|| black_box(read_composed()));
    });
    group.bench_function("decoded_lines_lifecycle", |bencher| {
        bencher.iter(|| {
            composed
                .lifecycle(composed.parse_text_lines(
                    read_text_lines(&source, &options).expect("a decoded line stream"),
                ))
                .try_fold(0_usize, |read, message: yggdryl::Result<FixMsg>| {
                    message.map(|_| read + 1)
                })
                .expect("a walked message")
        });
    });

    // The codec alone, over the framed bodies: what a message costs to
    // build, without the frame it was cut from or the batch it lands in. A
    // line the reader refuses - the empty body - is an item the count skips.
    let held = bodies(&source);
    group.bench_function("parse_lines", |bencher| {
        bencher.iter(|| {
            black_box(&codec)
                .parse_lines(black_box(&held))
                .filter(Result::is_ok)
                .count()
        });
    });

    // The record reader over the same bodies, each row naming the plugin
    // that logged it: the capture fills the crate's `msgpluginid` field and
    // selects nothing, so this is what a row costs to read with one more
    // capture on every line.
    let plugin_codec = FixCodec::new(Arc::clone(&registry))
        .with_threads(1)
        .with_capture_names(["msgpluginid"])
        .with_exclude_msgtypes::<[&str; 0], &str>([]);
    let lines: Vec<TextLine> = held
        .iter()
        .enumerate()
        .map(|(index, body)| {
            let plugin = if index % 2 == 0 {
                "ULB"
            } else {
                "OMS_X1_TradeCapture"
            };
            TextLine::from_bytes(
                index as u64,
                TextBytes::from_bytes(body.as_slice()).expect("a page"),
                std::sync::Arc::new(yggdryl::text::TextOptions::new()),
            )
            .expect("a line")
            .with_captures(vec![Some(
                TextBytes::from_bytes(plugin.as_bytes()).expect("a page"),
            )])
            .expect("captures")
        })
        .collect();
    group.bench_function("parse_text_lines_msgpluginid", |bencher| {
        bencher.iter_batched(
            || lines.clone(),
            |held| {
                black_box(&plugin_codec)
                    .parse_text_lines(held)
                    .filter(Result::is_ok)
                    .count()
            },
            BatchSize::LargeInput,
        );
    });

    // What a message costs after it is built. Every pass runs over fresh
    // clones, set up outside the measured routine: a clone carries none of
    // what a message derives about itself on its first projection, so each
    // number is the pass over a message the stream just built, and a pass
    // that takes the message by value is the pass alone.
    let messages: Vec<FixMsg> = codec.parse_lines(&held).filter_map(Result::ok).collect();
    const MINUTE: i64 = 60_000_000_000;
    const SNAPSHOT_BASE: i64 = 1_700_000_000_000_000_000;
    let snapshot_messages: Vec<FixMsg> = messages
        .iter()
        .filter(|message| message.header().stated_sendingtime())
        .take(16)
        .cloned()
        .enumerate()
        .map(|(index, mut message)| {
            message
                .set_currunix(SNAPSHOT_BASE + i64::try_from(index).expect("sixteen rows") * MINUTE);
            message.set_crosscode(format!("SNAPSHOT-{index}"));
            message.set_state(State::read("new").expect("the shipped new state"));
            message.set_exprtime(Some(SNAPSHOT_BASE + 60 * MINUTE));
            message.set_seqnum(0);
            message.set_prevunix(None);
            message.set_prevuuid(None);
            message.set_parentuuids(Vec::new());
            message.set_snapunix(None);
            message.finalize();
            message
        })
        .collect();
    assert_eq!(
        snapshot_messages.len(),
        16,
        "the corpus carries the fixture"
    );
    group.bench_function("into_row", |bencher| {
        bencher.iter_batched(
            || messages.clone(),
            |held| {
                held.iter()
                    .map(|message| black_box(message).into_row(&schema).expect("a row").len())
                    .sum::<usize>()
            },
            BatchSize::LargeInput,
        );
    });
    // This is the same row shape without the residual arrival record. It
    // retains every ordinary column and all root/child metadata, isolating
    // the direct final-sequence path from residual coverage.
    let columns = StructType::from_fields(
        schema
            .fields()
            .iter()
            .filter(|field| {
                field.name() != yggdryl::fix::FIXENTRIES_COLUMN
                    && field.name() != yggdryl::NOFIXENTRIES_TAG_NAME.1
            })
            .cloned(),
    )
    .expect("the fixed columns remain unique");
    let columns = Field::new(schema.name(), DataType::from(columns), schema.is_nullable())
        .try_with_metadata_entries(schema.as_metadata().iter())
        .expect("the fixed metadata remains valid");
    group.bench_function("into_row_columns", |bencher| {
        bencher.iter(|| {
            messages
                .iter()
                .map(|message| {
                    black_box(message)
                        .into_row(&columns)
                        .expect("a projected row")
                        .len()
                })
                .sum::<usize>()
        });
    });
    // Rows are prepared once: the measured inverse is only the semantic
    // reconstruction from the fixed row, not a second projection.
    let rows: Vec<_> = messages
        .iter()
        .map(|message| message.into_row(&schema).expect("a row"))
        .collect();
    group.bench_function("from_row", |bencher| {
        bencher.iter(|| {
            for row in &rows {
                black_box(
                    FixMsg::from_row(Arc::clone(&registry), &schema, black_box(row))
                        .expect("a rebuilt message"),
                );
            }
        });
    });
    // The parse settled the identifiers a message goes by; the walk states
    // each message's place in its chain, and the rows carry both.
    assert!(
        messages
            .iter()
            .any(|message| !message.get_identifiers().is_empty()),
        "the parse fills the identifiers"
    );
    let walked = codec
        .lifecycle(messages.clone())
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("the capture walks");
    assert!(
        walked
            .iter()
            .any(|message| message.get_prevuuid().is_some()),
        "the walk chains the capture"
    );
    for (name, rows) in [
        ("arrow_reader", &messages),
        ("arrow_reader_walked", &walked),
    ] {
        group.bench_function(name, |bencher| {
            bencher.iter_batched(
                || rows.clone(),
                |held| {
                    codec
                        .arrow_reader(schema.clone(), held)
                        .expect("a reader")
                        .map(|batch| batch.expect("a batch").num_rows())
                        .sum::<usize>()
                },
                BatchSize::LargeInput,
            );
        });
    }
    group.bench_function("lifecycle", |bencher| {
        bencher.iter_batched(
            || messages.clone(),
            |held| {
                codec
                    .lifecycle(held)
                    .map(|message| message.expect("walked").entries().len())
                    .sum::<usize>()
            },
            BatchSize::LargeInput,
        );
    });
    // The same message a thousand times: the corpus above is every shape a
    // bridge writes, this is one report logged at every hop it passed, and
    // the finite capture's delivery set removes every republication after the first.
    let report = messages
        .iter()
        .find(|message| message.header().msgtype() == "8")
        .expect("the corpus carries an execution report")
        .clone();
    let same_shape: Vec<FixMsg> = std::iter::repeat_n(report, 1_000).collect();
    group.bench_function("lifecycle_same_shape", |bencher| {
        bencher.iter_batched(
            || same_shape.clone(),
            |held| {
                codec
                    .lifecycle(held)
                    .map(|message| message.expect("walked").entries().len())
                    .sum::<usize>()
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("digest", |bencher| {
        bencher.iter_batched(
            || messages.clone(),
            |held| {
                held.iter()
                    .map(|message| black_box(message).digest())
                    .fold(0_u128, |folded, digest| folded ^ digest)
            },
            BatchSize::LargeInput,
        );
    });

    // The same doors on several threads, against the one-thread rows above:
    // what the machine's cores buy each door, and what each door leaves on
    // the thread that pulls it - the text reader in front of the line
    // doors, the batches closing behind the Arrow ones.
    for threads in [2, 4] {
        let spread = codec.clone().with_threads(threads);
        let composed = composed.clone().with_threads(threads);
        group.bench_function(format!("parse_lines/threads={threads}"), |bencher| {
            bencher.iter(|| {
                black_box(&spread)
                    .parse_lines(black_box(&held))
                    .filter(Result::is_ok)
                    .count()
            });
        });
        group.bench_function(format!("decoded_lines/threads={threads}"), |bencher| {
            bencher.iter(|| {
                composed
                    .parse_text_lines(
                        read_text_lines(&source, &options).expect("a decoded line stream"),
                    )
                    .filter(Result::is_ok)
                    .count()
            });
        });
        group.bench_function(
            format!("parse_text_arrow_reader/threads={threads}"),
            |bencher| {
                bencher.iter(|| {
                    let read = black_box(&source)
                        .read_arrow_reader(&batched_text(8))
                        .expect("a reader");
                    spread
                        .parse_text_arrow_reader(read)
                        .expect("a reader")
                        .map(|batch| batch.expect("a batch").num_rows())
                        .sum::<usize>()
                });
            },
        );
        group.bench_function(
            format!("parse_arrow_messages/threads={threads}"),
            |bencher| {
                bencher.iter(|| {
                    let read = black_box(&source)
                        .read_arrow_reader(&batched_text(16))
                        .expect("a reader");
                    spread
                        .parse_arrow_messages(read)
                        .expect("messages")
                        .filter(Result::is_ok)
                        .count()
                });
            },
        );
        group.bench_function(format!("arrow_reader/threads={threads}"), |bencher| {
            bencher.iter_batched(
                || messages.clone(),
                |held| {
                    spread
                        .arrow_reader(schema.clone(), held)
                        .expect("a reader")
                        .map(|batch| batch.expect("a batch").num_rows())
                        .sum::<usize>()
                },
                BatchSize::LargeInput,
            );
        });
    }
    group.finish();

    let snapshot_codec = codec.with_snapshot_ns(MINUTE);
    let mut snapshots = criterion.benchmark_group("fix/pipeline/lifecycle_snapshots");
    snapshots.throughput(Throughput::Elements(snapshot_messages.len() as u64));
    snapshots.bench_function("16x60", |bencher| {
        bencher.iter_batched(
            || snapshot_messages.clone(),
            |held| {
                snapshot_codec
                    .lifecycle(held)
                    .map(|message| message.expect("walked").entries().len())
                    .sum::<usize>()
            },
            BatchSize::LargeInput,
        );
    });
    snapshots.finish();
}

/// One line of the capture, stripped of its row header, checked to be the
/// shape the benchmark names so an edit to the corpus fails here rather than
/// silently measuring something else.
fn capture_body(index: usize, expects: &[u8]) -> Vec<u8> {
    let line = LOG
        .split(|byte| *byte == b'\n')
        .nth(index)
        .expect("a line of the capture");
    // The row header closes on the level in parentheses and one space.
    let at = line
        .windows(2)
        .position(|pair| pair == b") ")
        .expect("a row header")
        + 2;
    let body = line[at..].to_vec();
    assert!(
        memchr::memmem::find(&body, expects).is_some(),
        "line {index} of the capture no longer holds {}",
        String::from_utf8_lossy(expects)
    );
    body
}

/// What one line costs the codec, one shape at a time.
///
/// The shapes a capture actually mixes, each measured through the one door
/// `parse_lines` takes - a bridge row of a hundred named keys, a numeric
/// frame on pipes, the same frame on raw SOH, a `35=UL` frame packing a
/// bridge row inside its `XmlData`, and frames spelled `^A` and `<SOH>` -
/// beside the scan alone, so what the message costs after its pairs are
/// read is the difference. Per shape rather than over the corpus, so a
/// change to the codec is attributed to the shape it moved.
pub fn line_benchmarks(criterion: &mut Criterion) {
    let registry = Arc::new(seed());
    // Nothing is refused: two of the six shapes below are the session's own
    // - a Heartbeat and a TestRequest - and a codec on its defaults answers
    // no message for either, so those two rows would time an empty parse
    // while still reporting a throughput per byte of the line they skipped.
    let codec = FixCodec::new(Arc::clone(&registry))
        .with_threads(1)
        .with_exclude_msgtypes::<[&str; 0], &str>([]);
    let frame_pipe = capture_body(72, b"8=FIX.4.4|9=886|35=8|");
    let frame_soh: Vec<u8> = frame_pipe
        .iter()
        .map(|byte| if *byte == b'|' { 0x01 } else { *byte })
        .collect();
    let shapes: [(&str, Vec<u8>); 6] = [
        (
            "bridge_pipe",
            capture_body(1, b"MSGTYPE=executionreport|NOPARTYIDS=2|"),
        ),
        ("frame_pipe", frame_pipe),
        ("frame_soh", frame_soh),
        (
            "frame_packed",
            capture_body(111, b"8=FIX.4.2|9=3430|35=UL|"),
        ),
        ("frame_caret", capture_body(102, b"8=FIX.4.4^A9=61^A35=0^A")),
        (
            "frame_marker",
            capture_body(103, b"8=FIX.4.4<SOH>9=70<SOH>35=1<SOH>"),
        ),
    ];

    let mut group = criterion.benchmark_group("fix/line");
    for (shape, body) in &shapes {
        group.throughput(Throughput::Bytes(body.len() as u64));
        group.bench_function(format!("{shape}/parse_line"), |bencher| {
            bencher.iter(|| {
                black_box(&codec)
                    .parse_line(black_box(body))
                    .expect("messages")
                    .count()
            });
        });
        let page = TextBytes::from_bytes(body).expect("a page");
        group.bench_function(format!("{shape}/scan"), |bencher| {
            bencher.iter(|| {
                yggdryl::text::TextEntries::from_bytes_direct(black_box(&page))
                    .map_or(0, |held| held.len())
            });
        });
    }
    group.finish();
}
