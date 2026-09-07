//! Classifying a capture read off a text log file, line by line and by column.
//!
//! The corpus is what a bridge's own log holds: framed FIX either side of the
//! process's prose, bridge rows keyed by name, sentences, and the JSON
//! configuration documents a Jolokia read answers with. The measurement is the
//! whole path a reader takes - a `.log` handle read as records, with the three
//! classification columns on - beside the per-line readings those columns are
//! filled from, so what the record surface adds over the readings is visible
//! rather than argued.

use std::hint::black_box;

use criterion::{Criterion, Throughput};
use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::media::text::TextOptions;
use yggdryl::types::{MsgDirection, MsgType};
use yggdryl::{IOMedia, MimeType, Url};

/// How many capture lines one measured run reads.
const ROWS: usize = crate::bench_profile::corpus(4_000, 200);

/// The five shapes a capture line arrives in, one row each.
///
/// Each is a real shape rather than a synthetic one, and each takes a
/// different path through the one shallow scan: a framed tag stream with prose
/// either side, a bare tag stream, a bridge line keyed by name, a sentence
/// nothing matches, and a bridge configuration document.
const TAGGED: &str =
    "sending >> 8=FIX.4.4|9=176|35=D|11=ORDER-1|55=AAPL|54=1|38=100|40=2|10=203| << queued";
const BARE: &str = "8=FIX.4.4|9=176|35=D|11=ORDER-1|55=AAPL|54=1|38=100|40=2|10=203|";
const NAMED: &str =
    "ACCOUNT=A1|MSGTYPE=D|CLORDID=ORDER-1|SYMBOL=AAPL|SIDE=1|ORDERQTY=100|ORDTYPE=2";
const PROSE: &str = "no level printed by this plugin, and no pairs either";

/// One Jolokia read of one session interface, as the bridge answers it.
///
/// Trimmed to what the readings actually touch - the ObjectName the entry is
/// keyed by, the `$type` discriminators, and the wording that would be read as
/// a direction if the document were not bounded - because a benchmark measures
/// the scan rather than the transfer of a fixture.
const ULCONFIG: &str = concat!(
    r#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"#,
    r#""value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,"#,
    r#"plugin-type=FIX,type=ConfigurationPlugin":{"Category":"InterBridge","Name":"ULMSG_BROKER_TO_DMZ","#,
    r#""ExtendedActions":[{"$type":"com.ullink.ulbridge.objects.ExtendedAction","name":"send-test-request","#,
    r#""description":"Send a test request message. This action is available only when the adapter is logged.","#,
    r#""parameters":[{"$type":"com.ullink.ulbridge.objects.ExtendedActionParameter","name":"test-request-id","#,
    r#""description":"The outgoing test request ID to send","defaultValue":"PING"}],"enabled":true}],"#,
    r#""ClassHierarchy":[{"$type":"com.ullink.ulbridge.objects.ClassAndRevision","#,
    r#""className":"com.ullink.ulbridge2.plugins.ULMsg","classRevision":null}],"State":"logged"}},"status":200}"#,
);

/// The shapes, in the order the corpus repeats them.
const SHAPES: [(&str, &str); 5] = [
    ("tagged", TAGGED),
    ("bare", BARE),
    ("named", NAMED),
    ("prose", PROSE),
    ("ulconfig", ULCONFIG),
];

/// A log file holding every shape, in as many rows as the profile asks for.
fn corpus() -> Vec<u8> {
    let mut bytes = Vec::new();
    for row in 0..ROWS {
        let (_, line) = SHAPES[row % SHAPES.len()];
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
    }
    bytes
}

/// A handle whose media type comes from a name, so `.log` reads as records.
fn handle(bytes: Vec<u8>) -> Buffer {
    Buffer::from_bytes(bytes).with_media_type(
        Url::from_str("file:///capture.log")
            .expect("a URL")
            .media_type(),
    )
}

/// Text options with the three classification columns on, or with none.
fn options(classified: bool) -> RecordOptions {
    let mut options = TextOptions::new();
    options.with_mimetype = classified;
    options.with_msgtype = classified;
    options.with_direction = classified;
    options.into()
}

/// Every row the handle answers with, which is what a reader pays for.
fn drain(handle: &Buffer, options: &RecordOptions) -> usize {
    handle
        .read_arrow_reader(options)
        .expect("a reader")
        .map(|batch| batch.expect("a batch").num_rows())
        .sum()
}

pub fn benchmarks(criterion: &mut Criterion) {
    let bytes = corpus();
    let source = handle(bytes.clone());
    let plain = options(false);
    let classified = options(true);
    // Asserted before anything is timed: a benchmark that measured a read
    // answering the wrong number of rows would measure the wrong thing.
    assert_eq!(drain(&source, &plain), ROWS);
    assert_eq!(drain(&source, &classified), ROWS);

    let mut group = criterion.benchmark_group("fix/classify");
    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_function("records/plain", |bencher| {
        bencher.iter(|| drain(black_box(&source), black_box(&plain)));
    });
    group.bench_function("records/classified", |bencher| {
        bencher.iter(|| drain(black_box(&source), black_box(&classified)));
    });
    group.finish();

    // The three readings on their own, per shape, so what one document costs
    // over one frame is a number rather than an impression.
    let mut group = criterion.benchmark_group("fix/infer");
    for (label, line) in SHAPES {
        let line = line.as_bytes();
        group.throughput(Throughput::Bytes(line.len() as u64));
        group.bench_function(format!("mimetype/{label}"), |bencher| {
            bencher.iter(|| MimeType::infer_bytes(black_box(line)));
        });
        group.bench_function(format!("msgtype/{label}"), |bencher| {
            bencher.iter(|| MsgType::infer_bytes(black_box(line)));
        });
        group.bench_function(format!("direction/{label}"), |bencher| {
            bencher.iter(|| MsgDirection::infer_bytes(black_box(line)));
        });
    }
    group.finish();
}
