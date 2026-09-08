//! A bridge's own log, read as text records and then as FIX rows.
//!
//! The whole path a desk takes over a day of ULBridge log, measured as the
//! two stages it is: the text reader frames each line under a row header
//! and classifies it, and the codec reads the framed body of every row into
//! the one fixed row shape. The corpus repeats the twelve lines such a log
//! actually interleaves - a Jolokia exchange whose answer is a configuration
//! document, framed FIX either side of a plugin's prose, a bridge row keyed
//! by name, and the sentences a bridge writes between them - so the numbers
//! are per line of a real capture rather than per line of one shape.
//!
//! The codec is pinned to the bridge's own dialect, which is what a capture
//! holding configuration documents needs: a name resolves in that dialect
//! first and in the standard one after, so the framed FIX still lands on
//! FIX's own tags while a document's attributes land on the bridge's.

use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, Throughput};
use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::media::text::TextOptions;
use yggdryl::{FixBatchReader, FixBranch, FixCodec, FixOptions, IOMedia, Timezone, Url};

use super::seed;

/// How many capture lines one measured run reads.
const ROWS: usize = crate::bench_profile::corpus(2_400, 240);

/// The lines a bridge interleaves, in the order it writes them.
///
/// One Jolokia exchange - four lines of prose and the answer, which is the
/// configuration document - then the frames and the rows of the sessions it
/// configured, with the plugin's own sentences between them.
const BLOCK: [&str; 12] = [
    "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) URI: /jolokia/read/com.ullink.ulbridge.sessioninterfaces.plugins:name=Router_TradeCapture,plugin-type=FIX,type=Plugin",
    "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) Path-Info: read/com.ullink.ulbridge.sessioninterfaces.plugins:name=Router_TradeCapture,plugin-type=FIX,type=Plugin",
    "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) Request: JmxReadRequest[attribute=null, objectName = com.ullink.ulbridge.sessioninterfaces.plugins:name=Router_TradeCapture,plugin-type=FIX,type=Plugin]",
    "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) Execution time: 0 ms",
    concat!(
        r#"2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) Response: {"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=Router_TradeCapture,plugin-type=FIX,type=Plugin","type":"read"},"#,
        r#""value":{"SenderCompID":"CLIENTFIS","TargetCompID":"VENUEADC","BeginString":"FIX.4.2","Category":"Fix TradeCapture","PrimaryHost":"10.20.30.40","PrimaryPort":9726,"BackupHost":null,"BackupPort":-1,"#,
        r#""CurrentHost":"10.20.30.40","CurrentPort":9726,"IncomingMsgSeqNum":4507,"OutgoingMsgSeqNum":571,"LogLevel":-1,"PriorityLevel":5,"LoadIsolation":0,"NotificationsStatus":false,"NeedReload":false,"#,
        r#""Name":"Router_TradeCapture","Version":"4.7.0","State":"logged","Type":"I","BinaryName":"ULFix.jar","ClassName":"ULFix","MinimumBridgeRevision":"20050101000000","Comment":"","Prefix":"","Suffix":"","#,
        r#""Guid":"e7254b20-9f01-5ed0-23a1-000000000935","ExtendedActions":[{"name":"send-test-request","enabled":true}],"Enrichments":[],"#,
        r#""ClassHierarchy":[{"className":"com.ullink.ulbridge2.plugins.ULFix","classRevision":"4.7.0"}],"Resources":[]},"timestamp":1755153982,"status":200}"#,
    ),
    "2026-08-14 06:46:30.416 [15261] [OMS_X1_TradeCapture] (DEBUG) Sending : 8=FIX.4.4|9=68|35=0|49=CLIAUDITX1|56=OMSAUDITX1|34=696|52=20260814-04:46:30.415655|10=159|",
    "2026-08-14 06:46:30.947 [402-e7254b20:9f015ed023:935] [ULMSG_BROKER_TO_DMZ] (DEBUG) Receiving : 8=FIX.4.2|9=55|35=0|49=ULB_DMZ|56=ULB_BRK|34=935|52=20260814-04:46:30.967|10=186|",
    "2026-08-14 06:46:36.887 [653] [Spot_FX_TradeCapture] (INFO) Receiving : 8=FIX.4.2|9=0322|35=8|34=4507|49=VENUEADC|56=CLIENTFIS|52=20260814-04:46:36|1=client|6=547.771791547861|11=20260814_TP1_CLIENT_1003|14=982|15=INR|17=E-20260814-4507|20=0|22=4|31=547.77|32=982|37=O-20260814-1003|38=982|39=2|40=1|44=547.771791547861|48=XX0000000001|54=1|55=EXAMPLECO|58=Filled|59=0|60=20260814-04:46:36|75=20260814|76=BRKR|77=O|150=2|151=0|10=197|",
    "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [Broker_DarkPool_TradeCapture] (DEBUG) RouteMessage : ACCOUNT=client|AVGPX=547.771791547861|CLORDID=20260814_TP1_CLIENT_1003|CUMQTY=982|CURRENCY=INR|EXECBROKER=BRKR|EXECTYPE=2|LASTPX=547.77|LASTQTY=982|LEAVESQTY=0|MSGTYPE=8|ORDERQTY=982|ORDSTATUS=2|ORDTYPE=1|SIDE=1|SYMBOL=EXAMPLECO|TRANSACTTIME=20260814-04:46:36|",
    "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [Broker_DarkPool_TradeCapture] (INFO) Filtering - Message for RiskMonitor",
    "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [EnrichmentManager] (INFO) Enrichment execution[&SetEnv, &Broker_DarkPool_TradeCapture]",
    "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [ULBridge] (INFO) Execution report (ClOrderID : 20260814_TP1_CLIENT_1003) without any route so using not persisted route: [UNDEFINED] --> [Broker_DarkPool_TradeCapture]",
];

/// The log, as the bytes a `.log` file holds.
fn corpus() -> Vec<u8> {
    let mut bytes = Vec::new();
    for row in 0..ROWS {
        bytes.extend_from_slice(BLOCK[row % BLOCK.len()].as_bytes());
        bytes.push(b'\n');
    }
    bytes
}

/// A handle whose media type comes from its name, so `.log` reads as records.
fn handle(bytes: &[u8]) -> Buffer {
    Buffer::from_bytes(bytes.to_vec()).with_media_type(
        Url::from_str("file:///bridge.log")
            .expect("a URL")
            .media_type(),
    )
}

/// The text options a bridge log is read under: the bridge's own row header
/// framed - its clock stamping each row, its bracket filling the session,
/// context and sequence columns - each line numbered, classified and read
/// for its direction.
fn text(classify: bool) -> RecordOptions {
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_direction = classify;
    options.parse_mimetype = classify;
    options.parse_msgtype = classify;
    options.into()
}

/// The bodies the text reader hands the codec, framed and stripped.
fn bodies(source: &Buffer) -> Vec<Vec<u8>> {
    use arrow_array::cast::AsArray;

    let mut held = Vec::with_capacity(ROWS);
    for batch in source.read_arrow_reader(&text(true)).expect("a reader") {
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
    let options = FixOptions::new().with_branch(branch.clone());

    let mut group = criterion.benchmark_group("fix/pipeline");
    group.throughput(Throughput::Bytes(bytes.len() as u64));

    // The first stage alone: lines framed under the row header, numbered,
    // and - in the second case - classified and read for their direction.
    for (label, classify) in [("text_read", false), ("text_read_classified", true)] {
        group.bench_function(label, |bencher| {
            bencher.iter(|| {
                black_box(&source)
                    .read_arrow_reader(&text(classify))
                    .expect("a reader")
                    .map(|batch| batch.expect("a batch").num_rows())
                    .sum::<usize>()
            });
        });
    }

    // Both stages: the text reader's batches read straight into FIX rows,
    // the capture's own columns carried in front of the tags.
    group.bench_function("text_read_into_fix", |bencher| {
        bencher.iter(|| {
            let read = black_box(&source)
                .read_arrow_reader(&text(true))
                .expect("a reader");
            FixBatchReader::from_column(Arc::clone(&registry), read, "body", options.clone())
                .expect("a reader")
                .map(|batch| batch.expect("a batch").num_rows())
                .sum::<usize>()
        });
    });

    // The second stage alone, per line: what the codec costs over the framed
    // bodies, without the batch it lands them in.
    let held = bodies(&source);
    let codec = FixCodec::new(Arc::clone(&registry)).with_branch(&branch);
    group.bench_function("codec_lines", |bencher| {
        bencher.iter(|| {
            held.iter()
                .map(|body| {
                    black_box(&codec)
                        .transform_line(black_box(body), false)
                        .expect("a readable row")
                        .entries()
                        .len()
                })
                .sum::<usize>()
        });
    });

    // The second stage alone, batched: the framed bodies already in Arrow,
    // read into FIX rows - so what the text stage costs is the difference
    // between this and the whole path.
    let capture = {
        let read = source.read_arrow_reader(&text(true)).expect("a reader");
        read.map(|batch| batch.expect("a batch"))
            .collect::<Vec<_>>()
    };
    let schema = capture[0].schema();
    group.bench_function("fix_batches", |bencher| {
        bencher.iter(|| {
            let read = yggdryl::arrow::batch_reader(Arc::clone(&schema), capture.clone());
            FixBatchReader::from_column(Arc::clone(&registry), read, "body", options.clone())
                .expect("a reader")
                .map(|batch| batch.expect("a batch").num_rows())
                .sum::<usize>()
        });
    });
    group.finish();
}

/// The batch path split into its stages, so where a row's cost goes is a
/// number rather than an argument.
///
/// Over the same messages: filling the fixed row, canonicalizing it under
/// the schema, assembling one Arrow batch from every row, the digest each
/// row pays for its `msghash` column, and the two arrival-record columns on
/// their own - the one nested shape in the row, and the part of it that the
/// generic value machinery walks entry by entry.
pub fn stages(criterion: &mut Criterion) {
    use yggdryl::{FixMsg, Scalar, fix_schema};

    let bytes = corpus();
    let source = handle(&bytes);
    let branch = FixBranch::from_str(yggdryl::ULBRIDGE_BRANCH).expect("a branch");
    let registry = Arc::new(seed().with_ulbridge_fields().expect("fields"));
    let codec = FixCodec::new(Arc::clone(&registry)).with_branch(&branch);
    let held = bodies(&source);
    let messages: Vec<FixMsg> = held
        .iter()
        .map(|body| codec.transform_line(body, false).expect("a row"))
        .collect();
    let schema = fix_schema(&registry, "fix").expect("the fixed schema");
    let mut group = criterion.benchmark_group("fix/pipeline_stages");
    group.bench_function("to_row", |bencher| {
        bencher.iter(|| {
            messages
                .iter()
                .map(|message| black_box(message).to_row(&schema).expect("a row").len())
                .sum::<usize>()
        });
    });
    let rows: Vec<Scalar> = messages
        .iter()
        .map(|message| message.to_row(&schema).expect("a row"))
        .collect();
    group.bench_function("canonicalize", |bencher| {
        bencher.iter(|| {
            rows.iter()
                .map(|row| {
                    schema
                        .canonicalize_value(black_box(row).clone())
                        .expect("canonical")
                        .len()
                })
                .sum::<usize>()
        });
    });
    let all = Scalar::from_sequence(rows.clone());
    group.bench_function("batch_from_value", |bencher| {
        bencher.iter(|| {
            yggdryl::arrow::batch_from_value(&schema, black_box(&all))
                .expect("a batch")
                .num_rows()
        });
    });
    group.bench_function("digest", |bencher| {
        bencher.iter(|| {
            messages
                .iter()
                .map(|message| black_box(message).digest())
                .fold(0_u128, |folded, digest| folded ^ digest)
        });
    });
    group.bench_function("entries_columns", |bencher| {
        let narrow = yggdryl::DataType::from_fields([
            schema
                .get_field_by_path("nofixentries")
                .expect("entries")
                .clone(),
            schema
                .get_field_by_path("nounmappedfixentries")
                .expect("unmapped")
                .clone(),
        ])
        .expect("a struct")
        .required_field("narrow");
        bencher.iter(|| {
            messages
                .iter()
                .map(|message| black_box(message).to_row(&narrow).expect("a row").len())
                .sum::<usize>()
        });
    });
    group.finish();
}
