//! The bridge's own capture, read as the messages it carries and then as
//! the rows, the wire and the chains a reader makes of them.
//!
//! The corpus is `rust/tests/fix/ulbridge.log` - a second of a ULBridge's
//! own capture, anonymized: 144 lines the bridge wrote under its row
//! header, 94 of which carry a message. Every stage runs over the same
//! lines, each on its own, and the throughput is per message: the codec
//! over the framed lines; the codec and then the one walk that chains each
//! message to its order's life; the fixed row a message becomes under
//! [`fix_schema`]; the message a row becomes again; and the wire bytes a
//! message emits.
//!
//! The lines are read once, outside every timer, so the row header's
//! captures are a fact the lines carry and never a cost the codec pays.
//! The capture is repeated in the release corpus so a run measures
//! hundreds of messages rather than a hundred; a repeat is the same
//! messages again, which the walk reads as a message logged at another
//! hop, restating the live one.

use std::hint::black_box;
use std::sync::Arc;

use criterion::{BatchSize, Criterion, Throughput};
use yggdryl::graph::Event;
use yggdryl::holder::Buffer;
use yggdryl::media::text::{TextLine, TextOptions, read_text_lines};
use yggdryl::{FixCodec, FixMsg, Scalar, Timezone, Url, fix_schema};

use super::seed;

/// The capture, exactly as the bridge wrote it; it ends in a newline, so
/// repeating it repeats whole lines.
const LOG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fix/ulbridge.log"
));

/// How many times the capture is repeated in one measured run.
const REPEATS: usize = crate::bench_profile::corpus(8, 1);

/// How many messages one copy of the capture carries.
///
/// The codec below refuses nothing, so this is the whole capture and not the
/// 79 a live session reads: `DEFAULT_REFUSED_MSGTYPES` holds back the
/// keepalives and the rows that state no type, and those are shapes this
/// corpus exists to measure.
const MESSAGES: usize = 94;

/// The text options a bridge log is read under: the bridge's own row
/// header framed, its clock read in UTC, each line numbered and classified.
fn options() -> TextOptions {
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_mimetype = true;
    options
}

/// The capture's lines, decoded and framed under the row header, each
/// owning its page.
fn lines(options: &TextOptions) -> Vec<TextLine> {
    let source = Buffer::from_bytes(LOG.repeat(REPEATS)).with_media_type(
        Url::from_str("file:///ulbridge.log")
            .expect("a URL")
            .media_type(),
    );
    read_text_lines(&source, options)
        .expect("a decoded line stream")
        .map(|line| line.expect("a line"))
        .collect()
}

pub fn benchmarks(criterion: &mut Criterion) {
    let registry = Arc::new(seed());
    let options = options();
    let codec = FixCodec::new(Arc::clone(&registry))
        .with_capture_names(options.capture_names())
        .with_exclude_msgtypes::<[&str; 0], &str>([]);
    let schema = fix_schema(&registry, "fix").expect("the fixed schema");
    let lines = lines(&options);

    // The parse, once, outside every timer: what the row and wire stages
    // read, and the check that the capture is what the numbers say it is.
    let parsed: Vec<yggdryl::Result<FixMsg>> = codec.parse_text_lines(lines.iter()).collect();
    assert!(parsed.iter().all(Result::is_ok), "the capture parses whole");
    let messages: Vec<FixMsg> = parsed.into_iter().filter_map(Result::ok).collect();
    assert_eq!(messages.len(), MESSAGES * REPEATS, "the corpus");
    let rows: Vec<Scalar> = messages
        .iter()
        .map(|message| message.into_row(&schema).expect("a row"))
        .collect();
    // The entries round-trip; the wire and the code do not yet for this
    // bridge row, which states no sending clock (see the bench report).
    let back = FixMsg::from_row(Arc::clone(&registry), &schema, &rows[1]).expect("a message");
    assert_eq!(back.entries(), messages[1].entries(), "the row round-trips");
    let chained = codec
        .lifecycle(messages.clone())
        .filter(|message| {
            message
                .as_ref()
                .is_ok_and(|held| held.get_prevuuid().is_some())
        })
        .count();
    assert!(chained > 0, "the walk chains the capture");

    let mut group = criterion.benchmark_group("fix/ulbridge");
    group.throughput(Throughput::Elements(messages.len() as u64));

    // The codec over the framed lines: every message built, settled and
    // identified, and nothing else.
    group.bench_function("parse", |bencher| {
        bencher.iter(|| {
            black_box(&codec)
                .parse_text_lines(black_box(&lines).iter())
                .filter(Result::is_ok)
                .count()
        });
    });

    // The parse and then the walk: each message stated as the one after
    // the live message of its chain, in the order the instants make.
    group.bench_function("parse_lifecycle", |bencher| {
        bencher.iter(|| {
            black_box(&codec)
                .lifecycle(codec.parse_text_lines(black_box(&lines).iter()))
                .filter(Result::is_ok)
                .count()
        });
    });

    // What a message costs after it is built, each over fresh clones set up
    // outside the timer: a clone carries none of what a message derives
    // about itself on its first projection, so each number is the pass
    // over a message the stream just built.
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
    group.bench_function("from_row", |bencher| {
        bencher.iter(|| {
            black_box(&rows)
                .iter()
                .map(|row| {
                    FixMsg::from_row(Arc::clone(&registry), &schema, black_box(row))
                        .expect("a message")
                        .as_value()
                        .as_sequence()
                        .map_or(0, <[Scalar]>::len)
                })
                .sum::<usize>()
        });
    });
    group.bench_function("into_bytes", |bencher| {
        bencher.iter_batched(
            || messages.clone(),
            |held| {
                held.iter()
                    .map(|message| black_box(message).into_bytes(b'|').len())
                    .sum::<usize>()
            },
            BatchSize::LargeInput,
        );
    });

    // Three steps of the parse measured through the doors that expose them,
    // so the whole is attributed: the assembly of a message from a row it
    // already holds - the typed facts lifted, the clocks and the identity
    // settled - which is what `with_registry` costs over the content row;
    // one typed write, which is what settling the identity again costs; and
    // the entries derived from the row, which is what the wire re-emission
    // starts from.
    let rows: Vec<(yggdryl::Field, Scalar)> = messages
        .iter()
        .map(|message| (message.as_field().clone(), message.as_value().clone()))
        .collect();
    group.bench_function("step/with_registry", |bencher| {
        bencher.iter(|| {
            black_box(&rows)
                .iter()
                .map(|(field, value)| {
                    FixMsg::with_registry(Arc::clone(&registry), field.clone(), value.clone())
                        .expect("a message")
                        .entries()
                        .len()
                })
                .sum::<usize>()
        });
    });
    group.bench_function("step/set_text", |bencher| {
        bencher.iter_batched(
            || messages.clone(),
            |mut held| {
                for message in &mut held {
                    message
                        .set(58, Scalar::from("restated"))
                        .expect("a typed write");
                }
                held
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("step/entries", |bencher| {
        bencher.iter_batched(
            || messages.clone(),
            |held| {
                held.iter()
                    .map(|message| black_box(message).entries().len())
                    .sum::<usize>()
            },
            BatchSize::LargeInput,
        );
    });
    group.finish();
}
