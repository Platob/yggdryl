//! A bridge's own log, end to end: text records in, one stable FIX table out.
//!
//! The capture is what a ULBridge writes - each line under the row header
//! [`yggdryl::ULBRIDGE_ROWHEADER`] names, a Jolokia exchange whose answer is
//! a JSON document, framed FIX either side of a plugin's prose, a bridge row
//! keyed by name, and the sentences between them. The text reader
//! frames and classifies every line; the codec reads every framed body into
//! the one row shape the dictionary decides before a byte is read. This is
//! the acceptance test for that composition: the schema never depends on the
//! data, a message in is a row out - a line carrying none is no row, a line
//! carrying two frames is two and a line carrying a document is one row
//! stating no type and no entries - the capture's own columns
//! lead each row and the captures named after fields fill them instead,
//! every row keeps its event clock independently of its header, attributes land
//! typed on the bridge's own tags, and the batched read agrees with the line
//! read on every tag both can answer.

use super::SoleMessage;

use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_array::cast::AsArray;
use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::text::TextOptions;
use yggdryl::{
    FixCodec, FixRegistry, IOMedia, Scalar, TimeUnit, Timezone, Url, fix_schema,
    fix_schema_carrying,
};

/// The committed dictionary, in which every line of the bridge's resolves.
fn registry() -> Arc<FixRegistry> {
    super::committed_registry()
}

/// The Jolokia answer, which is the one line that is a document: a body the
/// codec does not read, and so one `unknown` row with no entries.
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

/// Which capture lines carry a message, in the order they carry them.
/// Six of the eleven carry none, and each for the same
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

/// The codec: over the committed dictionary, with nothing pinned and
/// nothing refused.
///
/// Four of this capture's five messages are session traffic or a document -
/// two heartbeats and a Jolokia answer - which is what a live read refuses
/// through `DEFAULT_REFUSED_MSGTYPES`. A capture written to interleave every
/// shape a bridge writes is asking for all of them, and says so here; what
/// the default leaves out is pinned by
/// `the_default_read_answers_only_the_two_business_messages`.
fn codec() -> FixCodec {
    super::fixed_codec(registry()).with_exclude_msgtypes::<[&str; 0], &str>([])
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
    codec()
        .parse_text_arrow_reader(corpus(lines).read_arrow_reader(&text()).expect("a reader"))
        .expect("the batch reader opens")
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

    // The text reader's own columns lead the row - where the line was read
    // from, which line it was, when it was written, what it was, the line
    // itself and the header's captures - and the fixed columns follow. A
    // capture whose folded name a fixed column takes is not carried in
    // front, it fills that column: the reader's `msgtype`, and the header's
    // `bridgesessionid`, `msgctxid` and `msgseqnum`, each named for the
    // field it fills. What is left in front is what no column is spelled
    // for - the object the line came out of, which is the reader's word and
    // not the message's, the thread that wrote the line and its level.
    assert_eq!(
        &names[..8],
        [
            "sourceurl",
            "rownum",
            "mtime",
            "mimetype",
            "body",
            "timestamp",
            "msgthreadid",
            "level"
        ],
        "{names:?}"
    );
    // The crate's own clocks open the fixed columns; the standard header
    // follows them.
    let at = |name: &str| {
        names
            .iter()
            .position(|held| *held == name)
            .unwrap_or_else(|| panic!("a {name} column in {names:?}"))
    };
    for pair in ["level", "currunix", "creaunix", "prevunix"].windows(2) {
        assert!(at(pair[0]) < at(pair[1]), "{pair:?} in {names:?}");
    }
    let header = names
        .iter()
        .position(|held| *held == "beginstring")
        .expect("the header opens");
    assert_eq!(
        &names[header..header + 4],
        ["beginstring", "msgtype", "msgcat", "msgseqnum"],
        "{names:?}"
    );
    for once in [
        "msgtype",
        "sourceurl",
        "timestamp",
        "msgsessionid",
        "msgctxid",
        "msgpluginid",
        "msgseqnum",
    ] {
        assert_eq!(
            names.iter().filter(|held| **held == once).count(),
            1,
            "{once} is one column"
        );
    }
    // Which way a line moved is FIX's own `msgdirection`.
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
        .field_with_name("currunix")
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
    // all: the first two lines state runs of named pairs whose
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

    // Per line, what the line carries: the Jolokia answer's
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
    let thread = column(&read, "msgthreadid");
    assert_eq!(thread[ROUTED_ROW].as_i64(), Some(15_333));
    assert_eq!(thread[HEARTBEAT_ROW].as_i64(), Some(15_261));
    // The bracket's sequence number is FIX's own `MsgSeqNum(34)`, so it fills
    // that column rather than riding in front of the row - and only where the
    // message states none. The routed line is keyed by name and spells no
    // `34`, so the bracket's is what it reads; the heartbeat spells its own
    // and keeps it; the Jolokia response has neither.
    let seq = tag_column(&read, 34);
    assert_eq!(seq[ROUTED_ROW].as_i64(), Some(4_507));
    assert_eq!(seq[HEARTBEAT_ROW].as_i64(), Some(696));
    assert!(seq[RESPONSE_ROW].is_null(), "no session, no sequence");

    // All three parts of the bracket are captures named after the fields they
    // fill, so all three land in those columns rather than in front: the
    // routed row's bracket stated them, the heartbeat's did not.
    let session = tag_text(&read, yggdryl::MSGSESSIONID_TAG_NAME.0);
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
    let plugin = tag_text(&read, yggdryl::MSGPLUGINID_TAG_NAME.0);
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
    // The session instance the bridge handled a line on is the bracket's
    // own, and no line here spells one.
    let session = tag_text(&read, yggdryl::MSGSESSIONID_TAG_NAME.0);
    assert_eq!(session[HEARTBEAT_ROW], None);

    // The bracket's sequence number fills `MsgSeqNum` where the line stated
    // none - the routed row is keyed by name and carries no 34 - and never
    // where it did: the heartbeat keeps its own.
    let msgseqnum = tag_column(&read, 34);
    assert_eq!(msgseqnum[ROUTED_ROW].as_i64(), Some(4_507));
    assert_eq!(msgseqnum[HEARTBEAT_ROW].as_i64(), Some(696));

    // What each line was, read once by the text reader and carried through.
    let mimetype = text_column(&read, "mimetype");
    // A Jolokia answer is JSON, which is what it is: the classifier locates
    // the document behind the prose and names nothing about what it is for.
    assert_eq!(mimetype[RESPONSE_ROW].as_deref(), Some("application/json"));
    assert_eq!(mimetype[HEARTBEAT_ROW].as_deref(), Some("text/fix"));
    assert_eq!(mimetype[FILL_ROW].as_deref(), Some("text/fix"));
    assert_eq!(mimetype[RELAY_ROW].as_deref(), Some("text/fix"));
    assert_eq!(mimetype[ROUTED_ROW].as_deref(), Some("text/ullink"));
    // What a line is classified as and whether it carries a message are two
    // different answers. The text reader still calls line 1
    // `application/octet-stream` and line 2 `text/key-value` - a run of named
    // pairs is what a classifier can see without parsing - and neither line
    // carries a message, so neither reaches the batch: the five rows here are
    // the document, the three frames and the bridge row, and nothing else.
    let classified = text_column(&stage, "mimetype");
    assert_eq!(classified[0].as_deref(), Some("application/octet-stream"));
    assert_eq!(classified[1].as_deref(), Some("text/key-value"));
    assert_eq!(mimetype.len(), MESSAGES);

    // Which way each message moved is FIX's own tag 385: the
    // verb in front of the frame, and the `Response:` Jolokia wrote in front
    // of the document. The codec's pin for a line that states
    // no direction has nothing left to fill on this capture - the lines that
    // stated none were the sentences, and a sentence is no row
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
    // type - never a line that states no frame. The capture's
    // sentences used to be `unknown` rows and are now no rows at all, which
    // is what the five-row count says. The Jolokia answer is one: a
    // document is a body the codec does not read, so nothing states a type
    // for it - not the document, and not the crate - and tag 35 is null.
    assert_eq!(
        msgtype[RESPONSE_ROW], None,
        "a document states no type, and nothing states one for it"
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
    // A row states tag 52 only where the message did: the explicit codec
    // default is intake's stand-in for a line that named no clock, and it
    // lands in the event's own instant rather than in the header's column.
    assert!(sent[RESPONSE_ROW].is_null(), "{:?}", sent[RESPONSE_ROW]);
    assert_eq!(
        tag_column(&read, yggdryl::CURRUNIX_TAG_NAME.0)[RESPONSE_ROW]
            .temporal_count_at(TimeUnit::Nanosecond),
        Some(1_704_190_530_000_000_000),
        "an unstated sending time stands in as the event's instant"
    );

    // The fill's body: symbol, side, quantities and prices, typed.
    assert_eq!(tag_text(&read, 55)[FILL_ROW].as_deref(), Some("EXAMPLECO"));
    assert_eq!(tag_text(&read, 54)[FILL_ROW].as_deref(), Some("BUY"));
    // `OrderQty(38)` and `Price(44)` are columns of their own, exact at the
    // one width this crate keeps a number at.
    assert_eq!(
        tag_column(&read, 38)[FILL_ROW].as_decimal(),
        Some((yggdryl::i256::from_i128(982_000_000_000_000_000_000), 18))
    );
    assert_eq!(
        tag_column(&read, 44)[FILL_ROW].as_decimal(),
        Some((yggdryl::i256::from_i128(547_771_791_547_861_000_000), 18))
    );
    assert_eq!(tag_column(&read, 151)[FILL_ROW], super::decimal("0"));
    // The row carries the code the wire wrote, and the ranked state the
    // traits answer is read off it rather than columned beside it.
    assert_eq!(tag_text(&read, 39)[FILL_ROW].as_deref(), Some("2"));

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
    assert_eq!(
        tag_column(&read, 38)[ROUTED_ROW],
        tag_column(&read, 38)[FILL_ROW]
    );
    assert_eq!(tag_column(&read, 31)[ROUTED_ROW], super::decimal("547.77"));

    // Every projected row carries the code it settled on its content.
    // Distinct real messages remain distinct, independently of the separate
    // arrival digest.
    let identities = tag_column(&read, yggdryl::CURRHASHCODE_TAG_NAME.0);
    for (row, held) in identities.iter().enumerate() {
        assert!(held.as_u64().is_some(), "row {row} states a content code");
    }
    assert_ne!(identities[FILL_ROW], identities[ROUTED_ROW]);
    let stamp = tag_column(&read, yggdryl::CURRUNIX_TAG_NAME.0);
    assert!(stamp[FILL_ROW].is_temporal());
    assert!(stamp[ROUTED_ROW].is_temporal());
}

#[test]
fn every_row_keeps_its_event_clock_capture_clock_and_fix_version() {
    let read = read(&CAPTURE);
    let stage = text_stage(&CAPTURE);

    // A capture instant is ordinary context. `SendingTime` dates the event
    // independently of when the bridge logged it, and a row stating none -
    // the routed fill, keyed by name - takes the codec's clock: what its
    // `TransactTime` says is the lifecycle's to read.
    let clock = column(&stage, "timestamp");
    let carried = column(&read, "timestamp");
    let stamp = tag_column(&read, yggdryl::CURRUNIX_TAG_NAME.0);
    let snapshot = tag_column(&read, yggdryl::SNAPUNIX_TAG_NAME.0);
    let created = tag_column(&read, yggdryl::CREAUNIX_TAG_NAME.0);
    assert_eq!(stamp.len(), MESSAGES);
    assert_eq!(clock.len(), CAPTURE.len());
    for (row, line) in CARRYING.into_iter().enumerate() {
        assert_eq!(carried[row], clock[line], "capture context for row {row}");
        // No snapshot was taken of any of these, so the column stays empty
        // and the event instant is readable as `createdat`.
        assert!(snapshot[row].is_null());
        assert_eq!(created[row], stamp[row]);
        assert_ne!(stamp[row], clock[line]);
    }
    let millis = |row: usize| stamp[row].temporal_count_at(TimeUnit::Millisecond);
    assert_eq!(millis(RESPONSE_ROW), Some(1_704_190_530_000));
    assert_eq!(
        stamp[HEARTBEAT_ROW].temporal_count_at(TimeUnit::Nanosecond),
        Some(1_786_682_790_415_655_000)
    );
    assert_eq!(millis(FILL_ROW), Some(1_786_682_796_000));
    assert_eq!(millis(ROUTED_ROW), Some(1_704_190_530_000));

    // Every row says which FIX it was read as: the wire's own `BeginString`
    // where the frame stated one, and `FIX.` and the version the row was read
    // at where it did not - the routed row, keyed by name, and the Jolokia
    // document. A sentence answers no version because it answers no row.
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
fn a_json_document_is_one_unknown_row_carrying_only_what_the_row_stated() {
    let codec = codec();

    // The line as the text reader hands it to the codec: the row header
    // gone, the `Response:` prose still in front of the document.
    let body = &RESPONSE[ROWHEADER_WIDTH..];
    assert!(body.starts_with("Response: {"), "{body}");
    let message = codec
        .sole_line(body.as_bytes())
        .expect("the line carries one document, and so one message");

    // A JSON document is a body this codec does not read: the row said
    // something, and what it said is one message named `unknown` with no
    // entries - nothing the document spelled reaches a field, a tag or the
    // wire - carrying only what the row stated around it: the half the
    // `Response:` prose names.
    assert_eq!(message.as_field().name(), "unknown");
    assert!(message.entries().is_empty());
    // The version the row was read at is the whole of what it re-emits.
    assert_eq!(
        String::from_utf8(message.into_bytes(b'|')).unwrap(),
        "8=FIX.4.4|"
    );
    assert!(message.get_by_tag(35).is_none_or(|held| held.is_null()));
    for spelled in ["SenderCompID", "TargetCompID", "Name", "CurrentPort"] {
        assert!(
            message
                .get_by_name(spelled)
                .is_none_or(|held| held.is_null()),
            "{spelled}: nothing the document spelled reaches the message"
        );
    }
    // Tag 8 is filled from the version the row was read at, as on every
    // built message - the crate's default here, since the row states none
    // - and never from the `FIX.4.2` the document spelled.
    assert_eq!(message.by_tag(8).unwrap().as_str(), Some("FIX.4.4"));
    assert_eq!(message.by_tag(385).unwrap().as_str(), Some("R"));

    // In the batch the same document is the same row: no type, an empty
    // arrival record, and the capture's own columns filled - the clock the
    // header stated, the plugin that logged it, the direction the prose
    // named.
    let read = read(&CAPTURE);
    let stage = text_stage(&CAPTURE);
    assert_eq!(tag_text(&read, 35)[RESPONSE_ROW], None);
    let entries = column(&read, "fixentries");
    assert_eq!(
        entries[RESPONSE_ROW]
            .as_sequence()
            .map(<[Scalar]>::len)
            .unwrap_or_default(),
        0,
        "{:?}",
        entries[RESPONSE_ROW]
    );
    assert_eq!(
        column(&read, "timestamp")[RESPONSE_ROW],
        column(&stage, "timestamp")[CARRYING[RESPONSE_ROW]]
    );
    assert_eq!(
        tag_text(&read, yggdryl::MSGPLUGINID_TAG_NAME.0)[RESPONSE_ROW].as_deref(),
        Some("Jolokia")
    );
    assert_eq!(
        tag_text(&read, yggdryl::MSGDIRECTION_TAG_NAME.0)[RESPONSE_ROW].as_deref(),
        Some("R")
    );
    assert_eq!(tag_text(&read, 49)[RESPONSE_ROW], None);
}

/// How many bytes the row header takes off the front of every line here.
const ROWHEADER_WIDTH: usize = "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) ".len();

#[test]
fn the_batched_read_agrees_with_the_line_read_and_re_emits_the_wire() {
    let codec = codec();
    let read = read(&CAPTURE);

    // The text reader's bodies are what the codec reads, so the line read
    // runs over them rather than over the raw lines. Every row of the batch
    // came from a line carrying exactly one message here, so `sole_line` is
    // the line read for all five - and it is the line the row came from,
    // `CARRYING[row]`, whose columns the batch filled from. Where the line
    // alone answers nothing, what the batch answers is what that row's own
    // columns stated - here the header's `msgseqnum`. The reader states no
    // message type of its own: a line's type is what its frame says, read by
    // the codec.
    let stage = text_stage(&CAPTURE);
    let sequenced = column(&stage, "msgseqnum");
    let bodies = column(&read, "body");
    let rendered = |value: &Scalar| match value {
        Scalar::Null => None,
        held => Some(
            held.as_str()
                .map_or_else(|| format!("{held:?}"), ToString::to_string),
        ),
    };
    for tag in [8, 35, 49, 56, 34, 11, 55, 54, 150, 151, 60] {
        let held = tag_column(&read, tag);
        for (row, body) in bodies.iter().enumerate() {
            let body = body.as_str().expect("a body").as_bytes();
            let message = codec
                .sole_line(body)
                .unwrap_or_else(|error| panic!("row {row}: {error}"));
            let alone = message.get_by_tag(tag).unwrap_or(Scalar::Null);
            let expected = match (tag, &alone) {
                // The header's own column is an `int64` in the capture and a
                // `uint64` on the message, so the count is what is compared.
                (34, Scalar::Null) => sequenced[CARRYING[row]]
                    .as_i128()
                    .map(|count| count.to_string()),
                (34, held) => held.as_i128().map(|count| count.to_string()),
                _ => rendered(&alone),
            };
            let held = match tag {
                34 => &held[row].as_i128().map_or(Scalar::Null, Scalar::from),
                _ => &held[row],
            };
            let held = match tag {
                34 => held.as_i128().map(|count| count.to_string()),
                _ => rendered(held),
            };
            assert_eq!(
                expected, held,
                "tag {tag} on row {row} differs between the line read and the batch",
            );
        }
    }

    // The tags the batch answers and the line read cannot are fills from the
    // row's own columns: the sequence number the header stated for the routed
    // row, which carried none. The residual record excludes those fills and
    // every content field successfully projected into a typed column.
    let routed = bodies[ROUTED_ROW].as_str().expect("a body").as_bytes();
    let alone = codec.sole_line(routed).expect("the routed row");
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
    // This projection leaves ExecBroker, GrossTradeAmt and CurrencyCodeSource
    // in the residual record; its other content fields have typed columns.
    assert_eq!(recorded, [76, 381, 2897]);
    for filled in [
        34,
        yggdryl::MSGCTXID_TAG_NAME.0,
        yggdryl::MSGPLUGINID_TAG_NAME.0,
        yggdryl::CURRUNIX_TAG_NAME.0,
    ] {
        assert!(
            !recorded.contains(&i64::from(filled)),
            "tag {filled} is a fill, never an entry"
        );
    }

    // Written back out, the wire is rebuilt from each row's arrival record
    // and the facts it holds typed: what the row filled from its header is
    // not an entry, and the capture's own columns are the capture's, so
    // neither is re-emitted. One line is written per message, not per
    // source line, so the six lines that carried none write nothing and
    // eleven lines come back as five.
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
    // The document's row arrived with no entries and stated no type, so it
    // re-emits nothing but the version every built message states.
    assert_eq!(lines[RESPONSE_ROW], "8=FIX.4.4|");
    // The prose the text reader framed the line in is gone, and so are the
    // columns the capture put in front of the row.
    for line in &lines {
        for carried in ["|body=", "|rownum=", "|mimetype=", "|url=", "Receiving :"] {
            assert!(!line.contains(carried), "{line}");
        }
    }
    // Every message states its own header band first, then its entries: a
    // `BodyLength(9)` the frame opened with is an entry like any other, so
    // it re-emits behind the header rather than where the frame wrote it.
    assert_eq!(
        lines[HEARTBEAT_ROW],
        "8=FIX.4.4|35=0|49=CLIAUDITX1|56=OMSAUDITX1|34=696|52=20260814-04:46:30.415655|9=68|10=159|"
    );
    assert!(
        lines[ROUTED_ROW].starts_with("8=FIX.4.4|35=8|"),
        "{}",
        lines[ROUTED_ROW]
    );
    assert!(lines[FILL_ROW].contains("|11=20260814_TP1_CLIENT_1003|"));
}

/// A relay that batched two frames into one log write, and a sentence the
/// bridge wrote after it: two lines carrying two messages between them.
const BATCHED: [&str; 2] = [
    "2026-08-14 06:46:30.947 [402-e7254b20:9f015ed023:935] [ULMSG_BROKER_TO_DMZ] (DEBUG) Receiving : 8=FIX.4.4|9=68|35=0|49=CLIAUDITX1|56=OMSAUDITX1|34=696|52=20260814-04:46:30.415655|10=159|8=FIX.4.2|9=55|35=0|49=ULB_DMZ|56=ULB_BRK|34=935|52=20260814-04:46:30.967|10=186|",
    "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [ULBridge] (INFO) Filtering - Message for RiskMonitor",
];

#[test]
fn a_line_of_two_frames_is_two_rows_and_a_sentence_is_none() {
    let read = read(&BATCHED);

    // A row yields none, one or many. The first line holds two
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
    let stamp = tag_column(&read, yggdryl::CURRUNIX_TAG_NAME.0);
    assert_ne!(stamp[0], stamp[1]);
    assert_eq!(stamp, tag_column(&read, 52));
    let captured = column(&read, "timestamp");
    assert_eq!(captured[0], captured[1]);
    assert_eq!(
        captured[0].temporal_count_at(TimeUnit::Millisecond),
        Some(1_786_689_990_947)
    );
    assert_eq!(
        tag_text(&read, yggdryl::MSGPLUGINID_TAG_NAME.0),
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
    // Each frame re-emits its own bytes with its header band in front: the
    // `BodyLength(9)` the frame opened with is an entry, so it follows the
    // header rather than leading it.
    assert_eq!(
        lines,
        [
            "8=FIX.4.4|35=0|49=CLIAUDITX1|56=OMSAUDITX1|34=696|52=20260814-04:46:30.415655|9=68|10=159|",
            "8=FIX.4.2|35=0|49=ULB_DMZ|56=ULB_BRK|34=935|52=20260814-04:46:30.967|9=55|10=186|"
        ]
    );

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

#[test]
fn the_default_read_answers_only_the_two_business_messages() {
    // A live read of this capture answers two rows, not five: the two
    // heartbeats are `Heartbeat`, the Jolokia answer states no type, and
    // those are three of the types `DEFAULT_REFUSED_MSGTYPES` names.
    let default = super::fixed_codec(registry());
    let batches: Vec<RecordBatch> = default
        .parse_text_arrow_reader(
            corpus(&CAPTURE)
                .read_arrow_reader(&text())
                .expect("a reader"),
        )
        .expect("the batch reader opens")
        .map(|batch| batch.expect("a batch"))
        .collect();
    let rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
    assert_eq!(rows, 2);
    assert_eq!(
        tag_text(&batches[0], 35),
        vec![Some("8".to_owned()); 2],
        "the fill and the row it was routed as"
    );
    // The line door refuses the same lines the batch door did.
    assert_eq!(
        column(&text_stage(&CAPTURE), "body")
            .iter()
            .filter(|body| default
                .parse_line(body.as_str().expect("a body").as_bytes())
                .expect("the line reads")
                .next()
                .is_some())
            .count(),
        rows
    );
}
