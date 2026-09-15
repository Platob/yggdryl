//! A bridge's own log, end to end: text records in, one stable FIX table out.
//!
//! The capture is what a ULBridge writes - each line under the row header
//! [`yggdryl::ULBRIDGE_ROWHEADER`] names, a Jolokia exchange whose answer is
//! a configuration document, framed FIX either side of a plugin's prose, a
//! bridge row keyed by name, and the sentences between them. The text reader
//! frames and classifies every line; the codec reads every framed body into
//! the one row shape the dictionary decides before a byte is read. This is
//! the acceptance test for that composition: the schema never depends on the
//! data, a message in is a row out - a line carrying none is no row and a
//! line carrying two frames is two (decision 16) - the capture's own columns
//! lead each row and the captures named after fields fill them instead,
//! every row keeps its event clock independently of its header, attributes land
//! typed on the bridge's own tags, and the batched read agrees with the line
//! read on every tag both can answer.

use super::SoleMessage;
use super::path as fpath;
use super::states_no_envelope;
use super::{RETIRED_ENVELOPE_NAMES, RETIRED_ENVELOPE_TAGS, SESSIONINTERFACE_TAG};

use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_array::cast::AsArray;
use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::media::text::TextOptions;
use yggdryl::{
    FixCodec, FixEntry, FixRegistry, IOMedia, Scalar, TimeUnit, Timezone, Url, fix_schema,
    fix_schema_carrying,
};

/// The committed dictionary beside the bridge's own vocabulary.
fn registry() -> Arc<FixRegistry> {
    super::plugin_fields_registry()
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

/// Which capture lines carry a message, in the order they carry them
/// (decision 16). Six of the eleven carry none, and each for the same
/// reason - the line opens no frame, states no bridge pair and carries no
/// document. Lines 1 and 2 hold runs of named pairs -
/// `name=Router_TradeCapture,plugin-type=FIX,type=Plugin` and
/// `attribute=null, objectName = ...` - but a comma and a space are not a
/// separator a line names and the bridge marked no key, so they are prose
/// carrying an `=`; lines 3, 9, 10 and 11 are sentences with no pair in them
/// at all. Every one of the six used to be a row holding an entry-less
/// `unknown`.
const CARRYING: [usize; 5] = [3, 4, 5, 6, 7];

/// How many rows the batch door answers for this capture: one per message,
/// never one per line.
const MESSAGES: usize = CARRYING.len();

/// Where each shape sits among the messages, which is where its row sits in
/// every batch the codec answers.
const RESPONSE_ROW: usize = 0;
const HEARTBEAT_ROW: usize = 1;
const RELAY_ROW: usize = 2;
const FILL_ROW: usize = 3;
const ROUTED_ROW: usize = 4;

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
    options.parse_mimetype = true;
    options
}

/// The same, as the record options a read takes.
fn text() -> RecordOptions {
    text_options().into()
}

/// The codec: over the bridge's dictionary, whose fields resolve in the one
/// namespace without a pin.
fn codec() -> FixCodec {
    super::fixed_codec(registry())
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

/// The whole path, as the batches it answers - which is none where the lines
/// carry no message at all.
fn read_batches(lines: &[&str]) -> Vec<RecordBatch> {
    super::parsed_and_formatted(
        &codec(),
        corpus(lines).read_arrow_reader(&text()).expect("a reader"),
    )
    .map(|batch| batch.expect("a batch"))
    .collect()
}

/// The whole path, as one batch: the capture is far under the byte target.
fn read(lines: &[&str]) -> RecordBatch {
    let batches = read_batches(lines);
    assert_eq!(batches.len(), 1, "one batch, under the byte target");
    batches.into_iter().next().expect("the batch")
}

/// How many messages each capture line carries, read by the codec over the
/// very bodies the text reader hands it.
fn messages_per_line(lines: &[&str]) -> Vec<usize> {
    let codec = codec();
    column(&text_stage(lines), "body")
        .iter()
        .map(|body| {
            codec
                .parse_line(body.as_str().expect("a body").as_bytes())
                .expect("the line reads")
                .count()
        })
        .collect()
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

    // The text reader's own columns lead the row - which line it was, when it
    // was written, what it was, the line itself and the header's captures -
    // and the fixed columns follow. A capture whose folded name a fixed
    // column takes is not carried in front, it fills that column: the
    // reader's `msgtype` and `sourceurl`, and the header's `msgCtxId` and
    // `pluginid`. `senderSessionId` names a fixed column too, so it is not
    // carried either. `seqNum` is, since no fixed column is spelled so, and
    // it fills `msgseqnum` besides.
    assert_eq!(
        &names[..8],
        [
            "rownum",
            "mtime",
            "mimetype",
            "body",
            "timestamp",
            "threadId",
            "seqNum",
            "level"
        ],
        "{names:?}"
    );
    assert_eq!(
        &names[8..11],
        ["beginstring", "bodylength", "msgtype"],
        "{names:?}"
    );
    for once in [
        "msgtype",
        "sourceurl",
        "timestamp",
        "sendersessionid",
        "msgctxid",
        "pluginid",
        "msgseqnum",
    ] {
        assert_eq!(
            names.iter().filter(|held| **held == once).count(),
            1,
            "{once} is one column"
        );
    }
    // Which way a line moved is FIX's own `msgdirection` (decision 14).
    assert!(names.contains(&"msgdirection"), "{names:?}");
    assert!(!names.contains(&"direction"), "{names:?}");
    assert_eq!(names.last(), Some(&"fixentries"));

    // The timestamp capture was typed from its pattern before a byte was
    // read and took the zone the options declared; updatedat is independently
    // settled as an exact nanosecond UTC instant on every row.
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
        .field_with_name("updatedat")
        .expect("the clock column");
    assert!(
        matches!(
            stamp.data_type(),
            arrow_schema::DataType::Timestamp(_, Some(_))
        ),
        "{stamp:?}"
    );
    assert!(!stamp.is_nullable(), "every row is stamped");

    // A capture whose lines carry no message is no rows and so no batch at
    // all (decision 16): the first two lines state runs of named pairs whose
    // only separators are a comma and a space - never a separator a line
    // names - and open no frame, so the batch door answers nothing for
    // either. They used to be two rows holding an entry-less `unknown`.
    assert_eq!(messages_per_line(&CAPTURE[..2]), [0, 0]);
    assert!(
        read_batches(&CAPTURE[..2]).is_empty(),
        "no message, no row, and with no row no batch"
    );

    // The shape is a function of the options and the dictionary alone: a
    // capture of two lines and one of eleven answer the same schema, and it
    // is exactly the composition the two halves publish.
    let two = read(&CAPTURE[CARRYING[RESPONSE_ROW]..=CARRYING[HEARTBEAT_ROW]]);
    assert_eq!(two.num_rows(), 2, "the document and the heartbeat");
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
fn a_message_in_is_a_row_out_and_the_captures_own_columns_ride_in_front() {
    let read = read(&CAPTURE);
    let stage = text_stage(&CAPTURE);

    // Per line, what the line carries (decision 16): the Jolokia answer's
    // document, three framed messages and the bridge row, and nothing at all
    // for the other six. The `URI:` and `Request:` lines name no separator
    // for their runs of pairs - a comma and a space are never one, and the
    // bridge marked no key - and the four sentences hold no pair and no
    // frame; none of the six opens a frame or carries a document, so none of
    // them states a message.
    assert_eq!(
        messages_per_line(&CAPTURE),
        [0, 0, 0, 1, 1, 1, 1, 1, 0, 0, 0]
    );

    // The text reader is what answers one row per line; the batch door
    // answers one row per message, so eleven lines are five rows.
    assert_eq!(stage.num_rows(), CAPTURE.len());
    assert_eq!(read.num_rows(), MESSAGES);

    // The line number is the capture's, one-based as the options said - and
    // it is the number of the line the message was read from, so the six
    // silent lines are simply missing from it.
    let rownum = read
        .column(read.schema().index_of("rownum").expect("rownum"))
        .as_primitive::<arrow_array::types::Int64Type>();
    assert_eq!(
        rownum.values().iter().copied().collect::<Vec<_>>(),
        CARRYING
            .iter()
            .map(|line| *line as i64 + 1)
            .collect::<Vec<i64>>()
    );

    // The row header's captures survive the codec untouched: the thread that
    // wrote the line and the level, and the bracket's sequence number - null
    // where the bracket held only the thread.
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
    let session = tag_text(&read, yggdryl::SENDERSESSIONID_TAG_NAME.0);
    let context = tag_text(&read, yggdryl::MSGCTXID_TAG_NAME.0);
    assert_eq!(session[ROUTED_ROW].as_deref(), Some("e7254b22"));
    assert_eq!(context[ROUTED_ROW].as_deref(), Some("9f015ee861"));
    assert_eq!(session[HEARTBEAT_ROW], None);
    assert_eq!(context[HEARTBEAT_ROW], None);

    // The plugin that logged a line is a capture named after the crate's own
    // column, so it lands there rather than in front - on every framed line,
    // as the bracket spells it - and it is never anything else: the session
    // names a line moved between are what the line itself spells, and no
    // line here spells one, nor which plugin the message came through
    // before.
    let plugin = tag_text(&read, yggdryl::PLUGINID_TAG_NAME.0);
    assert_eq!(
        plugin[HEARTBEAT_ROW].as_deref(),
        Some("OMS_X1_TradeCapture")
    );
    assert_eq!(plugin[FILL_ROW].as_deref(), Some("Spot_FX_TradeCapture"));
    assert_eq!(
        plugin[ROUTED_ROW].as_deref(),
        Some("Broker_DarkPool_TradeCapture")
    );
    assert_eq!(plugin[RESPONSE_ROW].as_deref(), Some("Jolokia"));
    let sender = tag_text(&read, yggdryl::SENDERSESSIONNAME_TAG_NAME.0);
    let target = tag_text(&read, yggdryl::TARGETSESSIONNAME_TAG_NAME.0);
    let previous = tag_text(&read, yggdryl::PREVPLUGINID_TAG_NAME.0);
    for row in 0..MESSAGES {
        assert_eq!(sender[row], None, "row {row} states no sender session name");
        assert_eq!(target[row], None, "row {row} states no target session name");
        assert_eq!(previous[row], None, "row {row} states no previous plugin");
    }

    // The bracket's sequence number fills `MsgSeqNum` where the line stated
    // none - the routed row is keyed by name and carries no 34 - and never
    // where it did: the heartbeat keeps its own.
    let msgseqnum = tag_column(&read, 34);
    assert_eq!(msgseqnum[ROUTED_ROW].as_i64(), Some(4_507));
    assert_eq!(msgseqnum[HEARTBEAT_ROW].as_i64(), Some(696));

    // What each line was, read once by the text reader and carried through.
    let mimetype = text_column(&read, "mimetype");
    // A bridge configuration line is JSON, which is what it is: a classifier
    // that named it anything else would have read the body far enough to know
    // it was a Jolokia answer, and that reading is the codec's rather than
    // the classifier's (decision 17).
    assert_eq!(mimetype[RESPONSE_ROW].as_deref(), Some("application/json"));
    assert_eq!(mimetype[HEARTBEAT_ROW].as_deref(), Some("text/fix"));
    assert_eq!(mimetype[FILL_ROW].as_deref(), Some("text/fix"));
    assert_eq!(mimetype[RELAY_ROW].as_deref(), Some("text/fix"));
    assert_eq!(mimetype[ROUTED_ROW].as_deref(), Some("text/ullink"));
    // What a line is classified as and whether it carries a message are two
    // different answers (decision 16). The text reader still calls line 1
    // `application/octet-stream` and line 2 `text/key-value` - a run of named
    // pairs is what a classifier can see without parsing - and neither line
    // carries a message, so neither reaches the batch: the five rows here are
    // the document, the three frames and the bridge row, and nothing else.
    let classified = text_column(&stage, "mimetype");
    assert_eq!(classified[0].as_deref(), Some("application/octet-stream"));
    assert_eq!(classified[1].as_deref(), Some("text/key-value"));
    assert_eq!(mimetype.len(), MESSAGES);

    // Which way each message moved is FIX's own tag 385 (decision 14): the
    // verb in front of the frame, and the `Response:` Jolokia wrote in front
    // of the document (decision 15). The codec's pin for a line that states
    // no direction has nothing left to fill on this capture - the lines that
    // stated none were the sentences, and a sentence is no row (decision 16)
    // - so every row here states the direction its own line spelled.
    let fix_direction = tag_text(&read, yggdryl::MSGDIRECTION_TAG_NAME.0);
    assert_eq!(fix_direction[HEARTBEAT_ROW].as_deref(), Some("S"));
    assert_eq!(fix_direction[FILL_ROW].as_deref(), Some("R"));
    assert_eq!(fix_direction[RESPONSE_ROW].as_deref(), Some("R"));
    assert_eq!(fix_direction[RELAY_ROW].as_deref(), Some("R"));
    assert_eq!(fix_direction[ROUTED_ROW].as_deref(), Some("S"));
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
    // `unknown` names a frame, a bridge row or a document that states no
    // type - never a line that states no frame (decision 16). The capture's
    // sentences used to be `unknown` rows and are now no rows at all, which
    // is what the five-row count says. The Jolokia answer used to be one
    // too: it states no type of its own, and now the crate states one for
    // it, because a plugin configuration is a message the crate registered
    // (decision 19).
    assert_eq!(
        msgtype[RESPONSE_ROW].as_deref(),
        Some("UCFG"),
        "a configuration is typed by the crate, not by the document"
    );
    assert_eq!(msgtype.len(), MESSAGES, "no sentence is a row");

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
        sent[HEARTBEAT_ROW].is_temporal(),
        "{:?}",
        sent[HEARTBEAT_ROW]
    );
    assert!(sent[RELAY_ROW].is_temporal(), "{:?}", sent[RELAY_ROW]);
    assert_eq!(
        sent[RESPONSE_ROW].temporal_count_at(TimeUnit::Nanosecond),
        Some(1_704_190_530_000_000_000),
        "an unstated sending time uses the explicit codec default"
    );

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

    // Every projected row carries its sixteen content identity bytes.
    // Distinct real messages remain distinct, independently of the separate
    // arrival digest.
    let identities = tag_column(&read, yggdryl::MSGHASH_TAG_NAME.0);
    for (row, held) in identities.iter().enumerate() {
        assert_eq!(
            super::identity_bytes(held).len(),
            16,
            "row {row} has sixteen identity bytes"
        );
    }
    assert_ne!(identities[FILL_ROW], identities[ROUTED_ROW]);
    let stamp = tag_column(&read, yggdryl::UPDATEDAT_TAG_NAME.0);
    assert!(stamp[FILL_ROW].is_temporal());
    assert!(stamp[ROUTED_ROW].is_temporal());
}

#[test]
fn every_row_keeps_its_event_clock_capture_clock_and_fix_version() {
    let read = read(&CAPTURE);
    let stage = text_stage(&CAPTURE);

    // A capture instant is ordinary context. TransactTime, else SendingTime,
    // settles the real event independently of when the bridge logged it.
    let clock = column(&stage, "timestamp");
    let carried = column(&read, "timestamp");
    let stamp = tag_column(&read, yggdryl::UPDATEDAT_TAG_NAME.0);
    let snapshot = tag_column(&read, yggdryl::SNAPSHOTAT_TAG_NAME.0);
    let created = tag_column(&read, yggdryl::CREATEDAT_TAG_NAME.0);
    assert_eq!(stamp.len(), MESSAGES);
    assert_eq!(clock.len(), CAPTURE.len());
    for (row, line) in CARRYING.into_iter().enumerate() {
        assert_eq!(carried[row], clock[line], "capture context for row {row}");
        assert_eq!(stamp[row], snapshot[row]);
        assert_eq!(created[row], snapshot[row]);
        assert_ne!(stamp[row], clock[line]);
    }
    let millis = |row: usize| stamp[row].temporal_count_at(TimeUnit::Millisecond);
    assert_eq!(millis(RESPONSE_ROW), Some(1_704_190_530_000));
    assert_eq!(
        stamp[HEARTBEAT_ROW].temporal_count_at(TimeUnit::Nanosecond),
        Some(1_786_682_790_415_655_000)
    );
    assert_eq!(millis(FILL_ROW), Some(1_786_682_796_000));
    assert_eq!(millis(ROUTED_ROW), Some(1_786_682_796_000));

    // The partition the stamp falls in, floored from the clock's own
    // nanoseconds so a millisecond clock still has one - on every row.
    let partition = tag_column(&read, yggdryl::TIMEPARTITION_TAG_NAME.0);
    for (row, held) in partition.iter().enumerate() {
        assert!(!held.is_null(), "row {row} has a partition");
    }
    let seconds = 1_786_682_796_i64;
    assert_eq!(
        partition[FILL_ROW].temporal_count_at(TimeUnit::Second),
        Some(seconds - seconds % yggdryl::DEFAULT_PARTITION_SECONDS)
    );
    assert_eq!(
        partition[FILL_ROW].dtype().unwrap(),
        stamp[FILL_ROW].dtype().unwrap()
    );

    // Every row says which FIX it was read as: the wire's own `BeginString`
    // where the frame stated one, and `FIX.` and the version the row was read
    // at where it did not - the routed row, keyed by name, and the Jolokia
    // document. A sentence answers no version because it answers no row
    // (decision 16).
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
fn the_bridges_own_fields_carry_its_membership_and_resolve_beside_the_standard() {
    let registry = registry();
    // The bridge's dictionary is a membership on the fields it contributed,
    // not a namespace of its own: every field from `PLUGIN_TAG_MIN` says
    // the bridge speaks it, a standard field it never touched says nothing,
    // and both are reached by tag or by name from the one registry.
    assert_eq!(registry.dialects(), [yggdryl::PLUGIN_DIALECT.to_owned()]);
    // `PLUGIN_TAG_MIN` is the floor of the range this dictionary claims,
    // not the smallest tag it happens to define: 20001 to 20004 carried the
    // Jolokia envelope, which decision 17 deleted, and they are retired
    // rather than reused - a capture written before it holds `MBean` on
    // 20001, so nothing else may answer to that tag. The smallest tag defined
    // is `SessionInterface` on 20010, above the floor and not equal to it.
    let (tag, name) = (SESSIONINTERFACE_TAG, "SessionInterface");
    assert!(tag > yggdryl::PLUGIN_TAG_MIN, "{tag}");
    for retired in RETIRED_ENVELOPE_TAGS {
        assert!(registry.field_by_tag(retired).is_err(), "{retired}");
    }
    for retired in RETIRED_ENVELOPE_NAMES {
        assert!(registry.field_by_name(retired).is_err(), "{retired}");
    }
    let smallest = yggdryl::fix_plugin_fields()
        .unwrap()
        .iter()
        .filter_map(|field| field.as_fix().tag().ok().flatten())
        .min()
        .expect("the bridge defines fields");
    assert_eq!(smallest, tag);
    assert!(
        smallest >= yggdryl::PLUGIN_TAG_MIN,
        "the floor is a floor: {smallest}"
    );
    let first = registry
        .field_by_tag(tag)
        .expect("the bridge's first field");
    assert!(first.as_fix().has_branch(yggdryl::PLUGIN_DIALECT));
    assert_eq!(
        first.as_fix().branches().collect::<Vec<_>>(),
        [yggdryl::PLUGIN_DIALECT]
    );
    assert_eq!(
        first.as_fix().id().unwrap(),
        Some(yggdryl::FixId::of(tag, name).unwrap())
    );
    assert_eq!(registry.field_by_name(name).unwrap().name(), first.name());
    let msgtype = registry
        .field_by_tag(yggdryl::fix::MSGTYPE_TAG_NAME.0)
        .expect("MsgType");
    assert_eq!(msgtype.as_fix().branches().count(), 0);
    assert_eq!(
        registry
            .field_by_name(yggdryl::fix::MSGTYPE_TAG_NAME.1)
            .unwrap()
            .name(),
        msgtype.name()
    );
    // Every field the bridge defines is held, and every one of them says so
    // in its membership; the specification's own fields in the same tag
    // range say nothing of the bridge.
    let mut bridged = 0;
    for defined in yggdryl::fix_plugin_fields().unwrap() {
        let held = registry
            .field_by_id(defined.as_fix().id().unwrap().unwrap())
            .unwrap();
        bridged += 1;
        assert!(
            held.as_fix().has_branch(yggdryl::PLUGIN_DIALECT),
            "{} says nothing of the bridge",
            held.name()
        );
    }
    assert!(bridged > 0, "the bridge's block is held");
    assert!(
        !registry
            .field_by_name("NoAdditionalTermBondRefs")
            .unwrap()
            .as_fix()
            .has_branch(yggdryl::PLUGIN_DIALECT)
    );
}

#[test]
fn a_configuration_document_lands_typed_on_the_bridges_own_tags() {
    let registry = registry();
    let codec = super::fixed_codec(Arc::clone(&registry));

    // The line as the text reader hands it to the codec: the row header
    // gone, the `Response:` prose still in front of the document.
    let body = &RESPONSE[ROWHEADER_WIDTH..];
    assert!(body.starts_with("Response: {"), "{body}");
    let message = codec
        .sole_line(body.as_bytes(), false)
        .expect("the document the line carries");

    // A configuration message is the plugin's attributes and nothing the
    // Jolokia answer wrapped them in: what the transport asked (`MBean`,
    // `Operation`) and how the asking went (`Status`, `Error`) state nothing
    // about the plugin, so their four tags hold nothing here (decision 17).
    states_no_envelope(&message);
    // What the read named this plugin by is the `SessionInterface` attribute,
    // which is where it always belonged, and it types like every other one.
    assert_eq!(
        message
            .by_tag(SESSIONINTERFACE_TAG)
            .unwrap()
            .as_str()
            .unwrap_or_default(),
        "com.ullink.ulbridge.sessioninterfaces.plugins:name=Router_TradeCapture,\
         plugin-type=FIX,type=Plugin"
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
        // The document's `State` is the plugin's, held under the bridge's
        // own name beside the crate's order `state`.
        ("PluginState", Scalar::from("logged")),
    ] {
        assert_eq!(message.by_path(&fpath(path)).unwrap(), &expected, "{path}");
    }
    // A stated null is an absence, and an array is kept as the JSON it is.
    assert!(message.get_by_path(&fpath("BackupHost")).is_none());
    assert!(
        message
            .by_path(&fpath("ExtendedActions"))
            .unwrap()
            .as_str()
            .is_some_and(|held| held.contains("send-test-request"))
    );
    // The configuration's scalar fields carry the tags they resolved to, so a
    // reader filtering the arrival record by tag finds them.
    let tags: Vec<i32> = message.entries().iter().map(FixEntry::tag).collect();
    assert!(tags.contains(&49), "{tags:?}");
    assert!(tags.contains(&SESSIONINTERFACE_TAG), "{tags:?}");
    assert!(tags.contains(&20_027), "CurrentPort: {tags:?}");
    for retired in RETIRED_ENVELOPE_TAGS {
        assert!(!tags.contains(&retired), "{retired}: {tags:?}");
    }

    // In the batch the same document is the same row: the attributes in the
    // arrival record, one entry per field under the key the document spelled
    // it by, and nothing the answer wrapped them in.
    let read = read(&CAPTURE);
    let entries = column(&read, "fixentries");
    let held = entries[RESPONSE_ROW].as_sequence().expect("the entries");
    let keyed: Vec<(i32, String)> = held
        .iter()
        .map(|entry| {
            let entry = entry.as_sequence().expect("an entry");
            (
                entry[0].as_i64().map_or(0, |tag| tag as i32),
                entry[3].as_str().unwrap_or_default().to_owned(),
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
    assert!(keyed.iter().all(|(tag, _)| *tag > 0), "{keyed:?}");
}

/// How many bytes the row header takes off the front of every line here.
const ROWHEADER_WIDTH: usize = "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) ".len();

#[test]
fn the_batched_read_agrees_with_the_line_read_and_re_emits_the_wire() {
    let registry = registry();
    let codec = super::fixed_codec(Arc::clone(&registry));
    let read = read(&CAPTURE);

    // The text reader's bodies are what the codec reads, so the line read
    // runs over them rather than over the raw lines. Every row of the batch
    // came from a line carrying exactly one message here, so `sole_line` is
    // the line read for all five - and it is the line the row came from,
    // `CARRYING[row]`, whose columns the batch filled from. Where the line
    // alone answers nothing, what the batch answers is what that row's own
    // columns stated - here the header's `seqNum`. The reader states no
    // message type of its own: a line's type is what its frame says, read by
    // the codec.
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
            let body = body.as_str().expect("a body").as_bytes();
            let message = codec
                .sole_line(body, false)
                .unwrap_or_else(|error| panic!("row {row}: {error}"));
            let alone = message.get_by_tag(tag).cloned().unwrap_or(Scalar::Null);
            let expected = match (tag, &alone) {
                (34, Scalar::Null) => rendered(&sequenced[CARRYING[row]]),
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
    // context, the plugin or the clock - so the arrival record is still the
    // line alone.
    let routed = bodies[ROUTED_ROW].as_str().expect("a body").as_bytes();
    let alone = codec.sole_line(routed, false).expect("the routed row");
    assert!(alone.get_by_tag(34).is_none());
    assert_eq!(tag_column(&read, 34)[ROUTED_ROW].as_i64(), Some(4_507));
    let entries = column(&read, "fixentries");
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
        yggdryl::MSGCTXID_TAG_NAME.0,
        yggdryl::PLUGINID_TAG_NAME.0,
        yggdryl::UPDATEDAT_TAG_NAME.0,
    ] {
        assert!(
            !recorded.contains(&i64::from(filled)),
            "tag {filled} is a fill, never an entry"
        );
    }

    // The wire is rebuilt from each row's arrival record: every framed line
    // comes back byte for byte behind the prose the text reader left in
    // front of it - the routed row too, because what the row filled from its
    // header is not an entry and so is not re-emitted. One line is written
    // per message, not per source line (decision 16), so the six lines that
    // carried none write nothing and eleven lines come back as five.
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
    assert_eq!(rows, MESSAGES as u64);
    let lines: Vec<&str> = std::str::from_utf8(&written)
        .expect("text")
        .lines()
        .collect();
    assert_eq!(lines.len(), MESSAGES, "{lines:?}");
    for (row, line, opens) in [
        (HEARTBEAT_ROW, HEARTBEAT, "8=FIX"),
        (FILL_ROW, FILL, "8=FIX"),
        (ROUTED_ROW, ROUTED, "ACCOUNT="),
    ] {
        let frame = &line[line.find(opens).expect("a frame")..];
        assert_eq!(lines[row], frame, "row {row} re-emits its frame");
    }
}

/// A relay that batched two frames into one log write, and a sentence the
/// bridge wrote after it: two lines carrying two messages between them.
const BATCHED: [&str; 2] = [
    "2026-08-14 06:46:30.947 [402-e7254b20:9f015ed023:935] [ULMSG_BROKER_TO_DMZ] (DEBUG) Receiving : 8=FIX.4.4|9=68|35=0|49=CLIAUDITX1|56=OMSAUDITX1|34=696|52=20260814-04:46:30.415655|10=159|8=FIX.4.2|9=55|35=0|49=ULB_DMZ|56=ULB_BRK|34=935|52=20260814-04:46:30.967|10=186|",
    "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [ULBridge] (INFO) Filtering - Message for RiskMonitor",
];

/// The two frames of that one line, as the bytes each carries.
const FIRST_FRAME: &str =
    "8=FIX.4.4|9=68|35=0|49=CLIAUDITX1|56=OMSAUDITX1|34=696|52=20260814-04:46:30.415655|10=159|";
const SECOND_FRAME: &str =
    "8=FIX.4.2|9=55|35=0|49=ULB_DMZ|56=ULB_BRK|34=935|52=20260814-04:46:30.967|10=186|";

#[test]
fn a_line_of_two_frames_is_two_rows_and_a_sentence_is_none() {
    let read = read(&BATCHED);

    // A row yields none, one or many (decision 16). The first line holds two
    // frames - the second opens at the `8=` behind the first's `10=` checksum
    // - and the second line is a sentence with no frame and no pair in it, so
    // it holds none. Two lines in, two rows out, and neither count is the
    // other's: the text reader still reads both lines.
    assert_eq!(messages_per_line(&BATCHED), [2, 0]);
    assert_eq!(text_stage(&BATCHED).num_rows(), BATCHED.len());
    assert_eq!(read.num_rows(), 2);

    // Both rows came from line 1, because line 2 contributed none.
    let rownum = read
        .column(read.schema().index_of("rownum").expect("rownum"))
        .as_primitive::<arrow_array::types::Int64Type>();
    assert_eq!(rownum.values().iter().copied().collect::<Vec<_>>(), [1, 1]);

    // Each message owns the entries of its own frame and none of its
    // neighbour's: two versions, two senders, two sequence numbers.
    assert_eq!(
        tag_text(&read, 8),
        [Some("FIX.4.4".to_owned()), Some("FIX.4.2".to_owned())]
    );
    assert_eq!(
        tag_text(&read, 49),
        [Some("CLIAUDITX1".to_owned()), Some("ULB_DMZ".to_owned())]
    );
    assert_eq!(tag_column(&read, 34)[0].as_i64(), Some(696));
    assert_eq!(tag_column(&read, 34)[1].as_i64(), Some(935));

    // Capture context is shared, while each frame keeps its own event clock.
    let stamp = tag_column(&read, yggdryl::UPDATEDAT_TAG_NAME.0);
    assert_ne!(stamp[0], stamp[1]);
    assert_eq!(stamp, tag_column(&read, 52));
    let captured = column(&read, "timestamp");
    assert_eq!(captured[0], captured[1]);
    assert_eq!(
        captured[0].temporal_count_at(TimeUnit::Millisecond),
        Some(1_786_689_990_947)
    );
    assert_eq!(
        tag_text(&read, yggdryl::PLUGINID_TAG_NAME.0),
        vec![Some("ULMSG_BROKER_TO_DMZ".to_owned()); 2]
    );
    assert_eq!(
        tag_text(&read, yggdryl::MSGDIRECTION_TAG_NAME.0),
        vec![Some("R".to_owned()); 2]
    );

    // The classifier describes the line's first frame, which is what it can
    // know without parsing, and both rows of that line carry its answer.
    assert_eq!(
        text_column(&read, "mimetype"),
        vec![Some("text/fix".to_owned()); 2]
    );

    // Written back out, each message re-emits only its own bytes, and the
    // sentence writes nothing.
    let emitting = codec().with_separator(b'|');
    let mut written: Vec<u8> = Vec::new();
    let source = emitting
        .parse_text_arrow_reader(
            corpus(&BATCHED)
                .read_arrow_reader(&text())
                .expect("a reader"),
        )
        .expect("the batch reader opens");
    let rows = emitting
        .write_arrow_reader(source, &mut written)
        .expect("the capture writes");
    assert_eq!(rows, 2);
    let lines: Vec<&str> = std::str::from_utf8(&written)
        .expect("text")
        .lines()
        .collect();
    assert_eq!(lines, [FIRST_FRAME, SECOND_FRAME]);

    // The single-frame door refuses a body holding a second: a caller
    // holding two frames has a row, not a frame.
    let body = column(&text_stage(&BATCHED), "body");
    let held = body[0].as_str().expect("a body");
    let refused = codec()
        .parse_fix_line(held.as_bytes())
        .expect_err("the door refuses a second frame");
    assert!(
        refused
            .to_string()
            .contains("expected one frame, got a second"),
        "{refused}"
    );
}
