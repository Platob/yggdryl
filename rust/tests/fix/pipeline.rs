//! A bridge's own log, end to end: text records in, one stable FIX table out.
//!
//! The capture is what a ULBridge writes - each line under the row header
//! [`yggdryl::ULBRIDGE_ROWHEADER`] names, a Jolokia exchange whose answer is
//! a configuration document, framed FIX either side of a plugin's prose, a
//! bridge row keyed by name, and the sentences between them. The text reader
//! frames and classifies every line; the codec reads every framed body into
//! the one row shape the dictionary decides before a byte is read. This is
//! the acceptance test for that composition: the schema never depends on the
//! data, a line in is a row out, the capture's own columns lead each row and
//! the captures named after fields fill them instead, every row is stamped
//! by its header's clock, a document's attributes land typed on the bridge's
//! own tags, and the batched read agrees with the line read on every tag
//! both can answer.

use super::OneMessage;

use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_array::cast::AsArray;
use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::media::text::TextOptions;
use yggdryl::{
    FixBranch, FixCodec, FixEntry, FixRegistry, IOMedia, Scalar, TimeUnit, Timezone, Url,
    fix_schema, fix_schema_carrying,
};

/// The committed dictionary beside the bridge's own vocabulary.
fn registry() -> Arc<FixRegistry> {
    super::ulbridge_registry()
}

/// The Jolokia answer, which is the one line that is a document.
const RESPONSE: &str = concat!(
    r#"2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) Response: {"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=Router_TradeCapture,plugin-type=FIX,type=Plugin","type":"read"},"#,
    r#""value":{"SenderCompID":"CLIENTFIS","TargetCompID":"VENUEADC","BeginString":"FIX.4.2","Category":"Fix TradeCapture","PrimaryHost":"10.20.30.40","PrimaryPort":9726,"BackupHost":null,"BackupPort":-1,"#,
    r#""CurrentHost":"10.20.30.40","CurrentPort":9726,"IncomingMsgSeqNum":4507,"OutgoingMsgSeqNum":571,"LogLevel":-1,"PriorityLevel":5,"NotificationsStatus":false,"NeedReload":false,"#,
    r#""Name":"Router_TradeCapture","Version":"4.7.0","State":"logged","Type":"I","ExtendedActions":[{"name":"send-test-request","enabled":true}],"Enrichments":[]},"timestamp":1755153982,"status":200}"#,
);

/// A heartbeat the bridge sent, under a bracket holding the thread alone.
const HEARTBEAT: &str = "2026-08-14 06:46:30.416 [15261] [OMS_X1_TradeCapture] (DEBUG) Sending : 8=FIX.4.4|9=68|35=0|49=CLIAUDITX1|56=OMSAUDITX1|34=696|52=20260814-04:46:30.415655|10=159|";

/// A fill the bridge received, wide enough to fill the body columns.
const FILL: &str = "2026-08-14 06:46:36.887 [653] [Spot_FX_TradeCapture] (INFO) Receiving : 8=FIX.4.2|9=0322|35=8|34=4507|49=VENUEADC|56=CLIENTFIS|52=20260814-04:46:36|1=client|6=547.771791547861|11=20260814_TP1_CLIENT_1003|14=982|15=INR|17=E-20260814-4507|31=547.77|32=982|37=O-20260814-1003|38=982|39=2|40=1|44=547.771791547861|48=XX0000000001|54=1|55=EXAMPLECO|58=Filled|59=0|60=20260814-04:46:36|75=20260814|150=2|151=0|10=197|";

/// The same fill as the bridge routes it, keyed by name, under a bracket
/// stating the session, the message context and the sequence number.
const ROUTED: &str = "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [Broker_DarkPool_TradeCapture] (DEBUG) RouteMessage : ACCOUNT=client|AVGPX=547.771791547861|CLORDID=20260814_TP1_CLIENT_1003|CUMQTY=982|CURRENCY=INR|EXECBROKER=BRKR|EXECTYPE=2|LASTPX=547.77|LASTQTY=982|LEAVESQTY=0|MSGTYPE=8|ORDERQTY=982|ORDSTATUS=2|ORDTYPE=1|SIDE=1|SYMBOL=EXAMPLECO|TRANSACTTIME=20260814-04:46:36|";

/// Every line the log interleaves, in the order it writes them.
const CAPTURE: [&str; 11] = [
    "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) URI: /jolokia/read/com.ullink.ulbridge.sessioninterfaces.plugins:name=Router_TradeCapture,plugin-type=FIX,type=Plugin",
    "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) Request: JmxReadRequest[attribute=null, objectName = com.ullink.ulbridge.sessioninterfaces.plugins:name=Router_TradeCapture,plugin-type=FIX,type=Plugin]",
    "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) Execution time: 0 ms",
    RESPONSE,
    HEARTBEAT,
    "2026-08-14 06:46:30.947 [402-e7254b20:9f015ed023:935] [ULMSG_BROKER_TO_DMZ] (DEBUG) Receiving : 8=FIX.4.2|9=55|35=0|49=ULB_DMZ|56=ULB_BRK|34=935|52=20260814-04:46:30.967|10=186|",
    FILL,
    ROUTED,
    "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [Broker_DarkPool_TradeCapture] (INFO) Filtering - Message for RiskMonitor",
    "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [EnrichmentManager] (INFO) Enrichment execution[&SetEnv, &Broker_DarkPool_TradeCapture]",
    "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [ULBridge] (INFO) Execution report (ClOrderID : 20260814_TP1_CLIENT_1003) without any route so using not persisted route: [UNDEFINED] --> [Broker_DarkPool_TradeCapture]",
];

/// Where each shape sits in the capture.
const RESPONSE_ROW: usize = 3;
const HEARTBEAT_ROW: usize = 4;
const FILL_ROW: usize = 6;
const ROUTED_ROW: usize = 7;

/// The log as the bytes a `.log` file holds.
fn corpus(lines: &[&str]) -> Buffer {
    let mut bytes = Vec::new();
    for line in lines {
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
    }
    Buffer::from_bytes(bytes).with_media_type(
        Url::from_str("file:///bridge.log")
            .expect("a URL")
            .media_type(),
    )
}

/// The text options a bridge log is read under: the bridge's own row header,
/// its clock read in UTC, and each line numbered, classified and read for
/// its direction.
fn text_options() -> TextOptions {
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_direction = true;
    options.parse_mimetype = true;
    options
}

/// The same, as the record options a read takes.
fn text() -> RecordOptions {
    text_options().into()
}

/// The codec: the bridge's own dialect, pinned for the whole run.
fn codec() -> FixCodec {
    FixCodec::new(registry()).with_branch(&FixBranch::from_str(yggdryl::ULBRIDGE_BRANCH).unwrap())
}

/// The first stage alone, as one batch: what the text reader hands the codec.
fn text_stage(lines: &[&str]) -> RecordBatch {
    let batches: Vec<RecordBatch> = corpus(lines)
        .read_arrow_reader(&text())
        .expect("a reader")
        .map(|batch| batch.expect("a batch"))
        .collect();
    assert_eq!(batches.len(), 1, "one batch, under the byte target");
    batches.into_iter().next().expect("the batch")
}

/// The whole path, as one batch: the capture is far under the byte target.
fn read(lines: &[&str]) -> RecordBatch {
    let batches: Vec<RecordBatch> = codec()
        .parse_text_arrow_reader(corpus(lines).read_arrow_reader(&text()).expect("a reader"))
        .expect("the batch reader opens")
        .map(|batch| batch.expect("a batch"))
        .collect();
    assert_eq!(batches.len(), 1, "one batch, under the byte target");
    batches.into_iter().next().expect("the batch")
}

/// One column of one batch, by position, as the values it holds.
fn column_at(batch: &RecordBatch, at: usize) -> Vec<Scalar> {
    let rows = yggdryl::arrow::batch_to_value(batch).expect("the batch reads");
    rows.as_sequence()
        .expect("rows")
        .iter()
        .map(|row| row.as_sequence().expect("a row")[at].clone())
        .collect()
}

/// One column of one batch, by name.
fn column(batch: &RecordBatch, name: &str) -> Vec<Scalar> {
    let at = batch
        .schema()
        .index_of(name)
        .unwrap_or_else(|_| panic!("a {name} column"));
    column_at(batch, at)
}

/// One column of one batch, by the tag its field carries.
fn tag_column(batch: &RecordBatch, tag: i32) -> Vec<Scalar> {
    column_at(batch, super::tag_index(batch, tag))
}

/// Each value's text, null where it states none.
fn texts(held: &[Scalar]) -> Vec<Option<String>> {
    held.iter()
        .map(|held| held.as_str().map(ToOwned::to_owned))
        .collect()
}

/// One column's text per row, by name.
fn text_column(batch: &RecordBatch, name: &str) -> Vec<Option<String>> {
    texts(&column(batch, name))
}

/// One column's text per row, by the tag its field carries.
fn tag_text(batch: &RecordBatch, tag: i32) -> Vec<Option<String>> {
    texts(&tag_column(batch, tag))
}

#[test]
fn the_schema_is_the_captures_columns_then_the_fixed_ones_and_never_depends_on_the_data() {
    let registry = registry();
    let held = read(&CAPTURE);
    let schema = held.schema();
    let names: Vec<&str> = schema
        .fields()
        .iter()
        .map(|held| held.name().as_str())
        .collect();

    // The text reader's own columns lead the row - where the line came from,
    // which line it was, when it was written, what it was, the line itself
    // and the header's
    // captures - and the fixed columns follow. A capture whose folded name a
    // fixed column takes is not carried in front, it fills that column: the
    // reader's `msgtype`, and the header's `timestamp` and `msgCtxId`.
    // `senderSessionId` names a fixed column too, so it is not carried either.
    // `seqNum` and `plugin` are, since no fixed column is spelled so; `seqNum`
    // fills `msgseqnum` besides and `plugin` the session the line's direction
    // names.
    assert_eq!(
        &names[..10],
        [
            "url",
            "rownum",
            "mtime",
            "direction",
            "mimetype",
            "body",
            "threadId",
            "seqNum",
            "plugin",
            "level"
        ],
        "{names:?}"
    );
    assert_eq!(
        &names[10..13],
        ["beginstring", "bodylength", "msgtype"],
        "{names:?}"
    );
    for once in [
        "msgtype",
        "timestamp",
        "sendersessionid",
        "msgctxid",
        "msgseqnum",
    ] {
        assert_eq!(
            names.iter().filter(|held| **held == once).count(),
            1,
            "{once} is one column"
        );
    }
    // The text reader's `direction` and FIX's own `msgdirection` are two
    // names, so both are here.
    assert!(names.contains(&"msgdirection"), "{names:?}");
    assert_eq!(
        &names[names.len() - 2..],
        ["nofixentries", "nounmappedfixentries"]
    );

    // The timestamp capture was typed from its pattern before a byte was
    // read and took the zone the options declared; the fixed column it
    // stamps is an instant too, and one every row has.
    let stage = text_stage(&CAPTURE);
    let captured = stage.schema();
    let clock = captured
        .field_with_name("timestamp")
        .expect("the timestamp capture");
    assert!(
        matches!(
            clock.data_type(),
            arrow_schema::DataType::Timestamp(_, Some(_))
        ),
        "{clock:?}"
    );
    let stamp = schema
        .field_with_name("timestamp")
        .expect("the clock column");
    assert!(
        matches!(
            stamp.data_type(),
            arrow_schema::DataType::Timestamp(_, Some(_))
        ),
        "{stamp:?}"
    );
    assert!(!stamp.is_nullable(), "every row is stamped");

    // The shape is a function of the options and the dictionary alone: a
    // capture of two lines and one of eleven answer the same schema, and it
    // is exactly the composition the two halves publish.
    let two = read(&CAPTURE[..2]);
    assert_eq!(two.schema(), held.schema());
    let composed = fix_schema_carrying(
        &text_options().source_field().expect("the text root"),
        &fix_schema(&registry, "fix").expect("the fixed root"),
    )
    .expect("the composition");
    let composed: Vec<&str> = composed.fields().iter().map(yggdryl::Field::name).collect();
    assert_eq!(composed, names, "the composition the two halves publish");
}

#[test]
fn a_line_in_is_a_row_out_and_the_captures_own_columns_ride_in_front() {
    let read = read(&CAPTURE);
    assert_eq!(read.num_rows(), CAPTURE.len());

    // The line number is the capture's, one-based as the options said.
    let rownum = read
        .column(read.schema().index_of("rownum").expect("rownum"))
        .as_primitive::<arrow_array::types::Int64Type>();
    assert_eq!(
        rownum.values().iter().copied().collect::<Vec<_>>(),
        (1..=11).collect::<Vec<i64>>()
    );

    // The row header's captures survive the codec untouched: the thread that
    // wrote the line, the plugin and the level, and the bracket's sequence
    // number - null where the bracket held only the thread.
    assert_eq!(
        text_column(&read, "plugin")[HEARTBEAT_ROW].as_deref(),
        Some("OMS_X1_TradeCapture")
    );
    assert_eq!(
        text_column(&read, "level")[FILL_ROW].as_deref(),
        Some("INFO")
    );
    let thread = column(&read, "threadId");
    assert_eq!(thread[ROUTED_ROW].as_i64(), Some(15_333));
    assert_eq!(thread[HEARTBEAT_ROW].as_i64(), Some(15_261));
    let seq = column(&read, "seqNum");
    assert_eq!(seq[ROUTED_ROW].as_i64(), Some(4_507));
    assert!(seq[HEARTBEAT_ROW].is_null(), "no session, no sequence");

    // Both halves of the bracket are captures named after the crate's own
    // fields, so both land in those columns rather than in front: the routed
    // row's bracket stated them, the heartbeat's did not. No line here spells a
    // session of its own, so the bracket's instance is what `sendersessionid`
    // reads.
    let session = tag_text(&read, yggdryl::SENDERSESSIONID_TAG);
    let context = tag_text(&read, yggdryl::MSGCTXID_TAG);
    assert_eq!(session[ROUTED_ROW].as_deref(), Some("e7254b22"));
    assert_eq!(context[ROUTED_ROW].as_deref(), Some("9f015ee861"));
    assert_eq!(session[HEARTBEAT_ROW], None);
    assert_eq!(context[HEARTBEAT_ROW], None);

    // The plugin that logged a line is the plugin session it moved from or
    // to, by the direction the line took: the heartbeat was sent, the fill
    // received, and the routed row - no verb - takes the default, sent.
    let sender = tag_text(&read, yggdryl::SENDERSESSIONNAME_TAG);
    let target = tag_text(&read, yggdryl::TARGETSESSIONNAME_TAG);
    assert_eq!(
        sender[HEARTBEAT_ROW].as_deref(),
        Some("OMS_X1_TradeCapture")
    );
    assert_eq!(target[HEARTBEAT_ROW], None);
    assert_eq!(target[FILL_ROW].as_deref(), Some("Spot_FX_TradeCapture"));
    assert_eq!(sender[FILL_ROW], None);
    assert_eq!(
        sender[ROUTED_ROW].as_deref(),
        Some("Broker_DarkPool_TradeCapture")
    );

    // The bracket's sequence number fills `MsgSeqNum` where the line stated
    // none - the routed row is keyed by name and carries no 34 - and never
    // where it did: the heartbeat keeps its own.
    let msgseqnum = tag_column(&read, 34);
    assert_eq!(msgseqnum[ROUTED_ROW].as_i64(), Some(4_507));
    assert_eq!(msgseqnum[HEARTBEAT_ROW].as_i64(), Some(696));

    // What each line was, read once by the text reader and carried through.
    let mimetype = text_column(&read, "mimetype");
    assert_eq!(mimetype[RESPONSE_ROW].as_deref(), Some("text/ulconfig"));
    assert_eq!(mimetype[HEARTBEAT_ROW].as_deref(), Some("text/fix"));
    assert_eq!(mimetype[FILL_ROW].as_deref(), Some("text/fix"));
    assert_eq!(mimetype[ROUTED_ROW].as_deref(), Some("text/ullink"));
    assert_eq!(mimetype[0].as_deref(), Some("application/octet-stream"));
    assert_eq!(mimetype[1].as_deref(), Some("text/key-value"));

    // Which way each line moved: the verb in front of the frame, and the
    // document's own statement that it came back.
    let direction = text_column(&read, "direction");
    assert_eq!(direction[HEARTBEAT_ROW].as_deref(), Some("SENT"));
    assert_eq!(direction[FILL_ROW].as_deref(), Some("RECV"));
    assert_eq!(direction[RESPONSE_ROW].as_deref(), Some("RECV"));
    assert_eq!(direction[2], None, "a sentence states no direction");
    // And the column read as a parameter is still carried into the row: FIX's
    // own `msgdirection` - a name of its own beside the text reader's
    // `direction` - agrees with the text reader's wherever the text reader
    // answered, and fills the default where it did not.
    let fix_direction = tag_text(&read, yggdryl::MSGDIRECTION_TAG);
    assert_eq!(fix_direction[HEARTBEAT_ROW].as_deref(), Some("SENT"));
    assert_eq!(fix_direction[FILL_ROW].as_deref(), Some("RECV"));
    assert_eq!(fix_direction[RESPONSE_ROW].as_deref(), Some("RECV"));
    assert_eq!(fix_direction[2].as_deref(), Some("SENT"));
}

#[test]
fn every_framed_line_fills_its_tag_columns_typed() {
    let read = read(&CAPTURE);

    let msgtype = tag_text(&read, 35);
    assert_eq!(msgtype[HEARTBEAT_ROW].as_deref(), Some("0"));
    assert_eq!(msgtype[FILL_ROW].as_deref(), Some("8"));
    assert_eq!(
        msgtype[ROUTED_ROW].as_deref(),
        Some("8"),
        "a bridge row names its type"
    );
    assert_eq!(msgtype[0], None, "prose states no type");

    // Header facts, by tag.
    let sender = tag_text(&read, 49);
    assert_eq!(sender[HEARTBEAT_ROW].as_deref(), Some("CLIAUDITX1"));
    assert_eq!(sender[FILL_ROW].as_deref(), Some("VENUEADC"));
    let seq = tag_column(&read, 34);
    assert_eq!(seq[HEARTBEAT_ROW].as_i64(), Some(696));
    assert_eq!(seq[FILL_ROW].as_i64(), Some(4507));

    // A sending time is an instant, not the text it arrived as.
    let sent = tag_column(&read, 52);
    assert!(
        matches!(sent[HEARTBEAT_ROW], Scalar::Temporal(_)),
        "{:?}",
        sent[HEARTBEAT_ROW]
    );
    assert!(sent[2].is_null(), "prose states no time");

    // The fill's body: symbol, side, quantities and prices, typed.
    assert_eq!(tag_text(&read, 55)[FILL_ROW].as_deref(), Some("EXAMPLECO"));
    assert_eq!(tag_text(&read, 54)[FILL_ROW].as_deref(), Some("1"));
    assert_eq!(tag_column(&read, 38)[FILL_ROW].as_f64(), Some(982.0));
    assert_eq!(
        tag_column(&read, 44)[FILL_ROW].as_f64(),
        Some(547.771791547861)
    );
    assert_eq!(tag_column(&read, 151)[FILL_ROW].as_f64(), Some(0.0));
    // A state column holds the ranked spelling the code names, never the
    // code: `2` is a filled order, and sorts after every live state.
    assert_eq!(tag_text(&read, 39)[FILL_ROW].as_deref(), Some("80FILLED"));

    // The routed row states the same trade under names, and lands on the
    // same tags.
    assert_eq!(
        tag_text(&read, 55)[ROUTED_ROW].as_deref(),
        Some("EXAMPLECO")
    );
    assert_eq!(
        tag_text(&read, 11)[ROUTED_ROW].as_deref(),
        Some("20260814_TP1_CLIENT_1003")
    );
    assert_eq!(tag_column(&read, 38)[ROUTED_ROW].as_f64(), Some(982.0));
    assert_eq!(tag_column(&read, 31)[ROUTED_ROW].as_f64(), Some(547.77));

    // The crate's own columns: a digest for every row that carried anything -
    // the framed fill and the routed row state different tag sets, so they
    // digest apart - and the clock every row is stamped with.
    let digest = tag_column(&read, yggdryl::MSGHASH_TAG);
    assert!(
        digest[FILL_ROW]
            .as_bytes()
            .is_some_and(|held| held.len() == 16)
    );
    assert!(
        digest[ROUTED_ROW]
            .as_bytes()
            .is_some_and(|held| held.len() == 16)
    );
    assert_ne!(digest[FILL_ROW], digest[ROUTED_ROW]);
    let stamp = tag_column(&read, yggdryl::TIMESTAMP_TAG);
    assert!(matches!(stamp[FILL_ROW], Scalar::Temporal(_)));
    assert!(matches!(stamp[ROUTED_ROW], Scalar::Temporal(_)));
}

#[test]
fn every_row_is_stamped_by_its_header_clock_and_says_which_fix_it_was_read_as() {
    let read = read(&CAPTURE);
    let stage = text_stage(&CAPTURE);

    // The row's own clock outranks every clock the message carries - the fill
    // states 04:46:36 and is stamped when the bridge wrote its line,
    // 06:46:36.887 - and a line carrying no clock at all is stamped too. The
    // text read declared UTC, so the capture and the stamp are one instant,
    // on every row.
    let clock = column(&stage, "timestamp");
    let stamp = tag_column(&read, yggdryl::TIMESTAMP_TAG);
    for row in 0..CAPTURE.len() {
        assert!(!stamp[row].is_null(), "row {row} is stamped");
        assert_eq!(
            stamp[row].temporal_count_at(TimeUnit::Nanosecond),
            clock[row].temporal_count_at(TimeUnit::Nanosecond),
            "row {row} is stamped by its header clock",
        );
    }
    let millis = |row: usize| stamp[row].temporal_count_at(TimeUnit::Millisecond);
    assert_eq!(millis(HEARTBEAT_ROW), Some(1_786_689_990_416));
    assert_eq!(millis(FILL_ROW), Some(1_786_689_996_887));
    assert_eq!(millis(ROUTED_ROW), Some(1_786_689_997_153));

    // The partition the stamp falls in, floored from the clock's own
    // nanoseconds so a millisecond clock still has one - on every row.
    let partition = tag_column(&read, yggdryl::UNIXPARTITION_TAG);
    for (row, held) in partition.iter().enumerate() {
        assert!(!held.is_null(), "row {row} has a partition");
    }
    let seconds = 1_786_689_996_i64;
    assert_eq!(
        partition[FILL_ROW].as_i64(),
        Some(seconds - seconds % yggdryl::DEFAULT_PARTITION_SECONDS)
    );

    // Every row says which FIX it was read as: the wire's own `BeginString`
    // where the frame stated one, and `FIX.` and the version the row was read
    // at where it did not - the routed row keyed by name, and the prose.
    let version = tag_text(&read, 8);
    for (row, held) in version.iter().enumerate() {
        assert!(held.is_some(), "row {row} states a version");
    }
    assert_eq!(version[HEARTBEAT_ROW].as_deref(), Some("FIX.4.4"));
    assert_eq!(version[FILL_ROW].as_deref(), Some("FIX.4.2"));
    assert!(
        version[ROUTED_ROW]
            .as_deref()
            .is_some_and(|held| held.starts_with("FIX.")),
        "{:?}",
        version[ROUTED_ROW]
    );
}

#[test]
fn a_configuration_document_lands_typed_on_the_bridges_own_tags() {
    let registry = registry();
    let branch = FixBranch::from_str(yggdryl::ULBRIDGE_BRANCH).unwrap();
    let codec = FixCodec::new(Arc::clone(&registry)).with_branch(&branch);

    // The line as the text reader hands it to the codec: the row header
    // gone, the `Response:` prose still in front of the document.
    let body = &RESPONSE[ROWHEADER_WIDTH..];
    assert!(body.starts_with("Response: {"), "{body}");
    let message = codec
        .one_line(body.as_bytes(), false)
        .expect("the document the line carries");

    // The envelope is what the exchange was, and it types.
    assert_eq!(
        message.by_tag(yggdryl::OPERATION_TAG).unwrap(),
        &Scalar::from("read")
    );
    assert_eq!(
        message.by_tag(yggdryl::STATUS_TAG).unwrap(),
        &Scalar::from(200_i64)
    );
    // A session interface is one flat message. Standard and bridge attributes
    // retain their own tags and datatypes.
    for (path, expected) in [
        ("SenderCompID", Scalar::from("CLIENTFIS")),
        ("TargetCompID", Scalar::from("VENUEADC")),
        ("BeginString", Scalar::from("FIX.4.2")),
        ("MBeanType", Scalar::from("Plugin")),
        ("PluginType", Scalar::from("FIX")),
        ("Name", Scalar::from("Router_TradeCapture")),
        ("CurrentPort", Scalar::from(9726_i64)),
        ("BackupPort", Scalar::from(-1_i64)),
        ("IncomingMsgSeqNum", Scalar::from(4507_i64)),
        ("NeedReload", Scalar::from(false)),
        ("State", Scalar::from("logged")),
    ] {
        assert_eq!(message.by_path(path).unwrap(), &expected, "{path}");
    }
    // A stated null is an absence, and an array is kept as the JSON it is.
    assert!(message.get_by_path("BackupHost").is_none());
    assert!(
        message
            .by_path("ExtendedActions")
            .unwrap()
            .as_str()
            .is_some_and(|held| held.contains("send-test-request"))
    );
    // The configuration's scalar fields carry the tags they resolved to, so a
    // reader filtering the arrival record by tag finds them.
    let tags: Vec<i32> = message.entries().iter().map(FixEntry::tag).collect();
    assert!(tags.contains(&49), "{tags:?}");
    assert!(tags.contains(&yggdryl::MBEAN_TAG), "{tags:?}");
    assert!(tags.contains(&20_027), "CurrentPort: {tags:?}");

    // In the batch the same document is the same row: the envelope on its
    // tags, and the attributes in the arrival record, one entry per field
    // under the key the document spelled it by.
    let read = read(&CAPTURE);
    let entries = column(&read, "nofixentries");
    let held = entries[RESPONSE_ROW].as_sequence().expect("the entries");
    let keyed: Vec<(i32, String)> = held
        .iter()
        .map(|entry| {
            let entry = entry.as_sequence().expect("an entry");
            (
                entry[0].as_i64().map_or(0, |tag| tag as i32),
                entry[2].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    assert!(
        keyed.contains(&(49, "SenderCompID".to_owned())),
        "{keyed:?}"
    );
    assert!(
        keyed.contains(&(20_027, "CurrentPort".to_owned())),
        "{keyed:?}"
    );
    // Nothing in the document went unexplained on a dictionary that has the
    // bridge's own fields.
    let unmapped = column(&read, "nounmappedfixentries");
    assert_eq!(
        unmapped[RESPONSE_ROW]
            .as_sequence()
            .map(<[Scalar]>::len)
            .unwrap_or_default(),
        0,
        "{:?}",
        unmapped[RESPONSE_ROW]
    );
}

/// How many bytes the row header takes off the front of every line here.
const ROWHEADER_WIDTH: usize = "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) ".len();

#[test]
fn the_batched_read_agrees_with_the_line_read_and_re_emits_the_wire() {
    let registry = registry();
    let branch = FixBranch::from_str(yggdryl::ULBRIDGE_BRANCH).unwrap();
    let codec = FixCodec::new(Arc::clone(&registry)).with_branch(&branch);
    let read = read(&CAPTURE);

    // The text reader's bodies are what the codec reads, so the line read
    // runs over them rather than over the raw lines. Where the line alone
    // answers nothing, what the batch answers is what the row's own columns
    // stated - here the header's `seqNum`. The reader states no message type
    // of its own: a line's type is what its frame says, read by the codec.
    let stage = text_stage(&CAPTURE);
    let sequenced = column(&stage, "seqNum");
    let bodies = column(&read, "body");
    let rendered = |value: &Scalar| match value {
        Scalar::Null => None,
        held => Some(
            held.as_str()
                .map_or_else(|| format!("{held:?}"), ToString::to_string),
        ),
    };
    for tag in [8, 35, 49, 56, 34, 11, 55, 54, 38, 44, 31, 32, 150, 151, 60] {
        let held = tag_column(&read, tag);
        for (row, body) in bodies.iter().enumerate() {
            let body = body.as_bytes().expect("a body");
            let message = codec
                .one_line(body, false)
                .unwrap_or_else(|error| panic!("row {row}: {error}"));
            let alone = message.get_by_tag(tag).cloned().unwrap_or(Scalar::Null);
            let expected = match (tag, &alone) {
                (34, Scalar::Null) => rendered(&sequenced[row]),
                _ => rendered(&alone),
            };
            assert_eq!(
                expected,
                rendered(&held[row]),
                "tag {tag} on row {row} differs between the line read and the batch",
            );
        }
    }

    // The tags the batch answers and the line read cannot are fills from the
    // row's own columns: the sequence number the header stated for the routed
    // row, which carried none. A fill is never an entry - and neither are the
    // session, the context or the clock - so the arrival record is still the
    // line alone.
    let routed = bodies[ROUTED_ROW].as_bytes().expect("a body");
    let alone = codec.one_line(routed, false).expect("the routed row");
    assert!(alone.get_by_tag(34).is_none());
    assert_eq!(tag_column(&read, 34)[ROUTED_ROW].as_i64(), Some(4_507));
    let entries = column(&read, "nofixentries");
    let recorded: Vec<i64> = entries[ROUTED_ROW]
        .as_sequence()
        .expect("the entries")
        .iter()
        .map(|entry| {
            entry.as_sequence().expect("an entry")[0]
                .as_i64()
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(recorded.len(), alone.entries().len());
    for filled in [
        34,
        yggdryl::MSGCTXID_TAG,
        yggdryl::SENDERSESSIONNAME_TAG,
        yggdryl::TIMESTAMP_TAG,
    ] {
        assert!(
            !recorded.contains(&i64::from(filled)),
            "tag {filled} is a fill, never an entry"
        );
    }

    // The wire is rebuilt from each row's arrival record: every framed line
    // comes back byte for byte behind the prose the text reader left in
    // front of it - the routed row too, because what the row filled from its
    // header is not an entry and so is not re-emitted.
    let mut written: Vec<u8> = Vec::new();
    let emitting = codec.clone().with_separator(b'|');
    let source = emitting
        .parse_text_arrow_reader(
            corpus(&CAPTURE)
                .read_arrow_reader(&text())
                .expect("a reader"),
        )
        .expect("the batch reader opens");
    let rows = emitting
        .write_arrow_reader(source, &mut written)
        .expect("the capture writes");
    assert_eq!(rows, CAPTURE.len() as u64);
    let lines: Vec<&str> = std::str::from_utf8(&written)
        .expect("text")
        .lines()
        .collect();
    for (row, line, opens) in [
        (HEARTBEAT_ROW, HEARTBEAT, "8=FIX"),
        (FILL_ROW, FILL, "8=FIX"),
        (ROUTED_ROW, ROUTED, "ACCOUNT="),
    ] {
        let frame = &line[line.find(opens).expect("a frame")..];
        assert_eq!(lines[row], frame, "row {row} re-emits its frame");
    }
}
