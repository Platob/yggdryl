//! A bridge's own log, read as text records and then as FIX rows.
//!
//! The corpus is `rust/tests/fix/ulbridge.log` - a second of a ULBridge's
//! own capture, anonymized, and then every shape a bridge writes that the
//! second happened not to hold: a Jolokia exchange whose answer is a
//! configuration document, FIXML behind a verb, frames spelled with `^A` and
//! `<SOH>`, a `35=UL` frame packing a group inside a group, a bridge row
//! keyed by name, a statistics line, an empty body and a warning - repeated
//! until the release corpus is about eleven megabytes, so the numbers are
//! per byte of a real capture rather than of one shape. Throughput is in
//! bytes of that log.
//!
//! Every stage runs over the same corpus, each on its own: the text reader
//! framing each line under the bridge's row header, the whole path into fixed
//! rows, the codec alone over the framed bodies, the record reader over the
//! same bodies with each row naming the plugin that logged it - so the
//! per-row dialect path is measured on its own - and then what a message
//! costs after it is built - its row, the batch the rows land in, the rules
//! that fill what it implies, the restatement at the dictionary's newest
//! version, the stamp that joins it to its order's life, and its digest.
//!
//! The codec is pinned to the bridge's own dialect, which is what a capture
//! holding configuration documents needs: a name resolves in that dialect
//! first and in the standard one after, so the framed FIX still lands on
//! FIX's own tags while a document's attributes land on the bridge's.

use std::hint::black_box;
use std::sync::Arc;

use criterion::{BatchSize, Criterion, Throughput};
use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::media::text::TextOptions;
use yggdryl::{FixBranch, FixCodec, FixMsg, IOMedia, Scalar, Timezone, Url, fix_schema};

use super::seed;

/// The capture, exactly as the bridge wrote it; it ends in a newline, so
/// repeating it repeats whole lines.
const LOG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fix/ulbridge.log"
));

/// How many times the capture is repeated in one measured run.
const REPEATS: usize = crate::bench_profile::corpus(64, 1);

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
    options.parse_direction = true;
    options.parse_mimetype = true;
    options.into()
}

/// The bodies the text reader hands the codec, framed and stripped.
fn bodies(source: &Buffer) -> Vec<Vec<u8>> {
    use arrow_array::cast::AsArray;

    let mut held = Vec::new();
    for batch in source.read_arrow_reader(&text()).expect("a reader") {
        let batch = batch.expect("a batch");
        let at = batch.schema().index_of("body").expect("the body column");
        let column = batch.column(at).as_binary::<i32>();
        for row in 0..batch.num_rows() {
            held.push(column.value(row).to_vec());
        }
    }
    held
}

pub fn benchmarks(criterion: &mut Criterion) {
    let bytes = corpus();
    let source = handle(&bytes);
    let branch = FixBranch::from_str(yggdryl::ULBRIDGE_BRANCH).expect("a branch");
    let registry = Arc::new(
        seed()
            .with_ulbridge_fields()
            .expect("the bridge's own fields"),
    );
    let codec = FixCodec::new(Arc::clone(&registry)).with_branch(&branch);
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
                .read_arrow_reader(&text())
                .expect("a reader");
            codec
                .parse_text_arrow_reader(read)
                .expect("a reader")
                .map(|batch| batch.expect("a batch").num_rows())
                .sum::<usize>()
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
    // that logged it: every other row an alias of the pinned dialect, the
    // rest a plugin no branch is named after. The alias is declared on a
    // copy of the dictionary so the other cases keep their setup, and the
    // codec stays pinned as they are. A row's dialect resolves off the
    // codec's memo after the first row spelling it, so this is what a row
    // costs to read under a dialect it names for itself.
    let mut aliased = registry.as_ref().clone();
    let alias = aliased
        .branch_named(yggdryl::ULBRIDGE_BRANCH)
        .cloned()
        .expect("the bridge's branch")
        .with_aliases(["ulb"])
        .expect("an alias");
    aliased.set_branch(alias).expect("the alias declares");
    let plugin_codec = FixCodec::new(Arc::new(aliased)).with_branch(&branch);
    let records: Vec<Scalar> = held
        .iter()
        .enumerate()
        .map(|(index, body)| {
            let plugin = if index % 2 == 0 {
                "ULB"
            } else {
                "OMS_X1_TradeCapture"
            };
            Scalar::from_record([
                ("body", Scalar::from(body.clone())),
                ("pluginid", Scalar::from(plugin)),
            ])
            .expect("a record")
        })
        .collect();
    group.bench_function("parse_text_records_pluginid", |bencher| {
        bencher.iter_batched(
            || records.clone(),
            |held| {
                black_box(&plugin_codec)
                    .parse_text_records(held)
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
    group.bench_function("arrow_reader", |bencher| {
        bencher.iter_batched(
            || messages.clone(),
            |held| {
                codec
                    .arrow_reader(schema.clone(), held.into_iter().map(Ok))
                    .expect("a reader")
                    .map(|batch| batch.expect("a batch").num_rows())
                    .sum::<usize>()
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("enrich_messages", |bencher| {
        bencher.iter_batched(
            || messages.clone(),
            |held| {
                codec
                    .enrich_messages(held)
                    .map(|message| message.expect("enriched").entries().len())
                    .sum::<usize>()
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("into_latest", |bencher| {
        bencher.iter_batched(
            || messages.clone(),
            |held| {
                held.into_iter()
                    .map(|message| message.into_latest().expect("restated").entries().len())
                    .sum::<usize>()
            },
            BatchSize::LargeInput,
        );
    });
    group.bench_function("lifecycle", |bencher| {
        bencher.iter_batched(
            || messages.clone(),
            |held| {
                codec
                    .lifecycle(held)
                    .map(|message| message.expect("stamped").entries().len())
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
    group.finish();
}
