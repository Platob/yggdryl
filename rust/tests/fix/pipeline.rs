//! A bridge's own log, end to end: text records in, one stable FIX table out.
//!
//! The capture is what a ULBridge writes - each line under one row header,
//! a Jolokia exchange whose answer is a configuration document, framed FIX
//! either side of a plugin's prose, a bridge row keyed by name, and the
//! sentences between them. The text reader frames and classifies every line;
//! the codec reads every framed body into the one row shape the dictionary
//! decides before a byte is read. This is the acceptance test for that
//! composition: the schema never depends on the data, a line in is a row
//! out, the capture's own columns lead each row, a document's attributes land
//! typed on the bridge's own tags, and the batched read agrees with the line
//! read on every tag both can answer.

use super::OneMessage;

use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_array::cast::AsArray;
use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::media::text::TextOptions;
use yggdryl::{
    FixBatchReader, FixBranch, FixCodec, FixEntry, FixOptions, FixRegistry, IOMedia, Scalar,
    Timezone, Url, fix_schema, fix_schema_carrying,
};

/// The committed dictionary beside the bridge's own vocabulary.
fn registry() -> Arc<FixRegistry> {
    super::ulbridge_registry()
}

/// The row header every line of the log opens with.
const ROWHEADER: &str = r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) \[(?P<thread>[^\]]+)\] \[(?P<plugin>[^\]]+)\] \((?P<level>[A-Z]+)\) ";

/// The Jolokia answer, which is the one line that is a document.
const RESPONSE: &str = concat!(
    r#"2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) Response: {"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=SmartTrade_TradeCapture,plugin-type=FIX,type=Plugin","type":"read"},"#,
    r#""value":{"SenderCompID":"PICTETFIS","TargetCompID":"ITGADC","BeginString":"FIX.4.2","Category":"Fix TradeCapture","PrimaryHost":"10.20.30.40","PrimaryPort":9726,"BackupHost":null,"BackupPort":-1,"#,
    r#""CurrentHost":"10.20.30.40","CurrentPort":9726,"IncomingMsgSeqNum":4507,"OutgoingMsgSeqNum":571,"LogLevel":-1,"PriorityLevel":5,"NotificationsStatus":false,"NeedReload":false,"#,
    r#""Name":"SmartTrade_TradeCapture","Version":"4.7.0","State":"logged","Type":"I","ExtendedActions":[{"name":"send-test-request","enabled":true}],"Enrichments":[]},"timestamp":1755153982,"status":200}"#,
);

/// A heartbeat the bridge sent.
const HEARTBEAT: &str = "2026-08-14 06:46:30.416 [15261] [Fidessa_X1_TradeCapture] (DEBUG) Sending : 8=FIX.4.4|9=68|35=0|49=PICAUDITX1|56=FIDAUDITX1|34=696|52=20260814-04:46:30.415655|10=159|";

/// A fill the bridge received, wide enough to fill the body columns.
const FILL: &str = "2026-08-14 06:46:36.887 [653] [EBS_FX_TradeCapture] (INFO) Receiving : 8=FIX.4.2|9=0322|35=8|34=4507|49=ITGADC|56=PICTETFIS|52=20260814-04:46:36|1=pictet|6=547.771791547861|11=20260814_TP1_PICTET_1003|14=982|15=INR|17=E-20260814-4507|31=547.77|32=982|37=O-20260814-1003|38=982|39=2|40=1|44=547.771791547861|48=INE786A01032|54=1|55=JKLAKSHMI|58=Filled|59=0|60=20260814-04:46:36|75=20260814|150=2|151=0|10=197|";

/// The same fill as the bridge routes it, keyed by name.
const ROUTED: &str = "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [Virtu_TritonBlack_TradeCapture] (DEBUG) RouteMessage : ACCOUNT=pictet|AVGPX=547.771791547861|CLORDID=20260814_TP1_PICTET_1003|CUMQTY=982|CURRENCY=INR|EXECBROKER=RJEA|EXECTYPE=2|LASTPX=547.77|LASTQTY=982|LEAVESQTY=0|MSGTYPE=8|ORDERQTY=982|ORDSTATUS=2|ORDTYPE=1|SIDE=1|SYMBOL=JKLAKSHMI|TRANSACTTIME=20260814-04:46:36|";

/// Every line the log interleaves, in the order it writes them.
const CAPTURE: [&str; 11] = [
    "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) URI: /jolokia/read/com.ullink.ulbridge.sessioninterfaces.plugins:name=SmartTrade_TradeCapture,plugin-type=FIX,type=Plugin",
    "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) Request: JmxReadRequest[attribute=null, objectName = com.ullink.ulbridge.sessioninterfaces.plugins:name=SmartTrade_TradeCapture,plugin-type=FIX,type=Plugin]",
    "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) Execution time: 0 ms",
    RESPONSE,
    HEARTBEAT,
    "2026-08-14 06:46:30.947 [402-e7254b20:9f015ed023:935] [ULMSG_BROKER_TO_DMZ] (DEBUG) Receiving : 8=FIX.4.2|9=55|35=0|49=ULB_DMZ|56=ULB_BRK|34=935|52=20260814-04:46:30.967|10=186|",
    FILL,
    ROUTED,
    "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [Virtu_TritonBlack_TradeCapture] (INFO) Filtering - Message for KRM22",
    "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [EnrichmentManager] (INFO) Enrichment execution[&SetEnv, &Virtu_TritonBlack_TradeCapture]",
    "2026-08-14 06:46:37.153 [15333-e7254b22:9f015ee861:4507] [ULBridge] (INFO) Execution report (ClOrderID : 20260814_TP1_PICTET_1003) without any route so using not persisted route: [UNDEFINED] --> [Virtu_TritonBlack_TradeCapture]",
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

/// The text options a bridge log is read under.
fn text() -> RecordOptions {
    let mut options = TextOptions::new()
        .try_with_rowheader(ROWHEADER)
        .expect("the row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_direction = true;
    options.parse_mimetype = true;
    options.parse_msgtype = true;
    options.into()
}

/// The codec options: the bridge's own dialect, pinned for the whole run.
fn options() -> FixOptions {
    FixOptions::new().with_branch(FixBranch::from_str(yggdryl::ULBRIDGE_BRANCH).unwrap())
}

/// The whole path, as one batch: the capture is far under the byte target.
fn read(lines: &[&str]) -> RecordBatch {
    let batches: Vec<RecordBatch> = FixBatchReader::from_column(
        registry(),
        corpus(lines).read_arrow_reader(&text()).expect("a reader"),
        "body",
        options(),
    )
    .expect("the batch reader opens")
    .map(|batch| batch.expect("a batch"))
    .collect();
    assert_eq!(batches.len(), 1, "one batch, under the byte target");
    batches.into_iter().next().expect("the batch")
}

/// One column of one batch, by name, as the values it holds.
fn column(batch: &RecordBatch, name: &str) -> Vec<Scalar> {
    let at = batch
        .schema()
        .index_of(name)
        .unwrap_or_else(|_| panic!("a {name} column"));
    let rows = yggdryl::arrow::batch_to_value(batch).expect("the batch reads");
    rows.as_sequence()
        .expect("rows")
        .iter()
        .map(|row| row.as_sequence().expect("a row")[at].clone())
        .collect()
}

/// One column's text per row, null where it states none.
fn text_column(batch: &RecordBatch, name: &str) -> Vec<Option<String>> {
    column(batch, name)
        .iter()
        .map(|held| held.as_str().map(ToOwned::to_owned))
        .collect()
}

#[test]
fn the_schema_is_the_captures_columns_then_the_tags_and_never_depends_on_the_data() {
    let registry = registry();
    let batch = read(&CAPTURE);
    let schema = batch.schema();
    let names: Vec<&str> = schema
        .fields()
        .iter()
        .map(|held| held.name().as_str())
        .collect();

    // The text reader's own columns lead the row - where the line came from,
    // which line it was, what it was, the line itself and the four captures
    // the row header typed - and the tags follow.
    assert_eq!(
        &names[..11],
        [
            "url",
            "rownum",
            "mtime",
            "direction",
            "mimetype",
            "msgtype",
            "body",
            "ts",
            "thread",
            "plugin",
            "level"
        ],
        "{names:?}"
    );
    assert_eq!(&names[11..14], ["8", "9", "35"], "{names:?}");
    assert_eq!(
        &names[names.len() - 2..],
        ["nofixentries", "nounmappedfixentries"]
    );

    // The timestamp capture was typed from its pattern before a byte was
    // read, and took the zone the options declared.
    let ts = schema.field_with_name("ts").expect("the timestamp");
    assert!(
        matches!(
            ts.data_type(),
            arrow_schema::DataType::Timestamp(_, Some(_))
        ),
        "{ts:?}"
    );

    // The shape is a function of the options and the dictionary alone: a
    // capture of two lines and one of eleven answer the same schema, and it
    // is exactly the composition the two halves publish.
    let two = read(&CAPTURE[..2]);
    assert_eq!(two.schema(), batch.schema());
    let composed = fix_schema_carrying(
        &TextOptions::new()
            .try_with_rowheader(ROWHEADER)
            .expect("the row header compiles")
            .with_timezone(Timezone::UTC)
            .source_field()
            .expect("the text root"),
        &fix_schema(&registry, "fix").expect("the fixed root"),
    )
    .expect("the composition");
    // `source_field` on fresh options omits the four optional columns the
    // read switched on, so the composition is compared past them.
    let composed: Vec<String> = composed
        .fields()
        .iter()
        .map(|held| held.name().to_owned())
        .collect();
    assert_eq!(composed[0], "url");
    assert_eq!(&composed[1..3], ["mtime", "body"]);
    assert_eq!(&composed[7..], &names[11..], "the tags follow in one order");
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

    // The row header's captures survive the codec untouched.
    assert_eq!(
        text_column(&read, "plugin")[HEARTBEAT_ROW].as_deref(),
        Some("Fidessa_X1_TradeCapture")
    );
    assert_eq!(
        text_column(&read, "level")[FILL_ROW].as_deref(),
        Some("INFO")
    );
    assert_eq!(
        text_column(&read, "thread")[ROUTED_ROW].as_deref(),
        Some("15333-e7254b22:9f015ee861:4507")
    );

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
    // And the column read as a parameter is still carried into the row: the
    // FIX direction column agrees with the text reader's wherever the text
    // reader answered, and fills the default where it did not.
    let fix_direction = text_column(&read, "385");
    assert_eq!(fix_direction[HEARTBEAT_ROW].as_deref(), Some("SENT"));
    assert_eq!(fix_direction[FILL_ROW].as_deref(), Some("RECV"));
    assert_eq!(fix_direction[RESPONSE_ROW].as_deref(), Some("RECV"));
    assert_eq!(fix_direction[2].as_deref(), Some("SENT"));
}

#[test]
fn every_framed_line_fills_its_tag_columns_typed() {
    let read = read(&CAPTURE);

    let msgtype = text_column(&read, "35");
    assert_eq!(msgtype[HEARTBEAT_ROW].as_deref(), Some("0"));
    assert_eq!(msgtype[FILL_ROW].as_deref(), Some("8"));
    assert_eq!(
        msgtype[ROUTED_ROW].as_deref(),
        Some("8"),
        "a bridge row names its type"
    );
    assert_eq!(msgtype[0], None, "prose states no type");

    // Header facts, by tag.
    let sender = text_column(&read, "49");
    assert_eq!(sender[HEARTBEAT_ROW].as_deref(), Some("PICAUDITX1"));
    assert_eq!(sender[FILL_ROW].as_deref(), Some("ITGADC"));
    let seq = column(&read, "34");
    assert_eq!(seq[HEARTBEAT_ROW].as_i64(), Some(696));
    assert_eq!(seq[FILL_ROW].as_i64(), Some(4507));

    // A sending time is an instant, not the text it arrived as.
    let sent = column(&read, "52");
    assert!(
        matches!(sent[HEARTBEAT_ROW], Scalar::Temporal(_)),
        "{:?}",
        sent[HEARTBEAT_ROW]
    );
    assert!(sent[2].is_null(), "prose states no time");

    // The fill's body: symbol, side, quantities and prices, typed.
    assert_eq!(
        text_column(&read, "55")[FILL_ROW].as_deref(),
        Some("JKLAKSHMI")
    );
    assert_eq!(text_column(&read, "54")[FILL_ROW].as_deref(), Some("1"));
    assert_eq!(column(&read, "38")[FILL_ROW].as_f64(), Some(982.0));
    assert_eq!(
        column(&read, "44")[FILL_ROW].as_f64(),
        Some(547.771791547861)
    );
    assert_eq!(column(&read, "151")[FILL_ROW].as_f64(), Some(0.0));
    assert_eq!(text_column(&read, "150")[FILL_ROW].as_deref(), Some("2"));

    // The routed row states the same trade under names, and lands on the
    // same tags.
    assert_eq!(
        text_column(&read, "55")[ROUTED_ROW].as_deref(),
        Some("JKLAKSHMI")
    );
    assert_eq!(
        text_column(&read, "11")[ROUTED_ROW].as_deref(),
        Some("20260814_TP1_PICTET_1003")
    );
    assert_eq!(column(&read, "38")[ROUTED_ROW].as_f64(), Some(982.0));
    assert_eq!(column(&read, "31")[ROUTED_ROW].as_f64(), Some(547.77));

    // The crate's own columns: a digest for every row that carried anything,
    // and the market clock read from the fill's transaction time.
    let digest = column(&read, "30001");
    assert!(
        digest[FILL_ROW]
            .as_bytes()
            .is_some_and(|held| held.len() == 16)
    );
    assert_ne!(
        digest[FILL_ROW], digest[ROUTED_ROW],
        "the routed row omits body fields and changes arrival order, both covered by the digest",
    );
    assert!(matches!(
        column(&read, "30004")[FILL_ROW],
        Scalar::Temporal(_)
    ));
    assert!(matches!(
        column(&read, "30004")[ROUTED_ROW],
        Scalar::Temporal(_)
    ));
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
        ("SenderCompID", Scalar::from("PICTETFIS")),
        ("TargetCompID", Scalar::from("ITGADC")),
        ("BeginString", Scalar::from("FIX.4.2")),
        ("MBeanType", Scalar::from("Plugin")),
        ("PluginType", Scalar::from("FIX")),
        ("Name", Scalar::from("SmartTrade_TradeCapture")),
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
    // runs over them rather than over the raw lines.
    let bodies = column(&read, "body");
    for tag in [8, 35, 49, 56, 34, 11, 55, 54, 38, 44, 31, 32, 150, 151, 60] {
        let held = column(&read, &tag.to_string());
        for (row, body) in bodies.iter().enumerate() {
            let body = body.as_bytes().expect("a body");
            let message = codec
                .one_line(body, false)
                .unwrap_or_else(|error| panic!("row {row}: {error}"));
            let alone = message.get_by_tag(tag).cloned().unwrap_or(Scalar::Null);
            let rendered = |value: &Scalar| match value {
                Scalar::Null => None,
                held => Some(
                    held.as_str()
                        .map_or_else(|| format!("{held:?}"), ToString::to_string),
                ),
            };
            assert_eq!(
                rendered(&alone),
                rendered(&held[row]),
                "tag {tag} on row {row} differs between the line read and the batch",
            );
        }
    }

    // The wire is rebuilt from each row's arrival record: every framed line
    // comes back byte for byte behind the prose the text reader left in
    // front of it.
    let mut written: Vec<u8> = Vec::new();
    let mut emitting = FixOptions::new();
    emitting.separator = b'|';
    let source = FixBatchReader::from_column(
        Arc::clone(&registry),
        corpus(&CAPTURE)
            .read_arrow_reader(&text())
            .expect("a reader"),
        "body",
        options(),
    )
    .expect("the batch reader opens");
    let rows = yggdryl::write_fix(source, &mut written, &emitting).expect("the capture writes");
    assert_eq!(rows, CAPTURE.len() as u64);
    let lines: Vec<&str> = std::str::from_utf8(&written)
        .expect("text")
        .lines()
        .collect();
    for (row, line) in [(HEARTBEAT_ROW, HEARTBEAT), (FILL_ROW, FILL)] {
        let frame = &line[line.find("8=FIX").expect("a frame")..];
        assert_eq!(lines[row], frame, "row {row} re-emits its frame");
    }
}
