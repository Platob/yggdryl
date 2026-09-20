//! A mixed capture, end to end: log bytes in, filled messages out.
//!
//! Every other suite in these phases reads one shape against one entry point.
//! This one is the whole path a desk actually takes, once, on a file holding
//! every shape at once: a `.log` handle read as text records, those records
//! read as messages by the codec, each filled as it is read with what it
//! implies - row by row and in batches, which have to agree.
//!
//! It is the acceptance test for the layers under it. Where a unit test says
//! one rule works, this says the rules compose: the text reader answers a row
//! per line and the codec a row per message, so the two counts are read
//! apart here; the dialect is chosen per line; and what a message re-emits
//! is what it states, the derived fields beside the arrived ones, so the
//! round trip closes on the message rather than on the line.

use super::SoleMessage;

use std::sync::Arc;

use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::text::{TextBytes, TextLine, TextOptions, read_text_lines};
use yggdryl::{FixCodec, FixRegistry, IOMedia, Scalar, Url};

/// The committed dictionary.
fn registry() -> Arc<FixRegistry> {
    super::committed_registry()
}

/// A framed order behind the prose its process printed around it.
const TAGGED: &str = "sending >> 8=FIX.4.4|9=176|35=D|49=BUYSIDE|56=VENUE|11=ORDER-1|55=AAPL|54=1|38=100|44=10.5|59=0|60=20240102-10:15:30.000|10=203| << queued seq=1092";
/// A part-filled execution report: what is left is implied, never stated.
const WORKING: &str = "8=FIX.4.4|9=224|35=8|49=VENUE|56=BUYSIDE|37=O-9|17=E-1|39=1|150=F|55=AAPL|54=1|38=100|14=40|32=40|31=10.5|64=20240104|10=118|";
/// The fill that closes it, and states nothing more than it must.
const FILLED: &str = "8=FIX.4.4|9=224|35=8|49=VENUE|56=BUYSIDE|37=O-9|17=E-2|39=2|150=F|55=AAPL|54=1|38=100|14=100|32=60|31=10.5|15=EUR|155=1.1|10=119|";
/// A bridge row, keyed by name rather than by tag, with a group packed in.
const NAMED: &str = "recv |MSGTYPE=D|SYMBOL=TTF|SIDE=1|ORDERQTY=1200|#NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=BUYSIDE\u{4}\u{3}PARTYIDSOURCE=D\u{4}\u{3}PARTYROLE=1|";
/// A FIXML row, whose fields are attributes.
const FIXML: &str = r#"<FIXML><Order ClOrdID="ORDER-2" Side="1" OrdQty="50"/></FIXML>"#;
/// A Jolokia read of a session interface: a JSON document rather than pairs,
/// and a body the codec does not read - one `unknown` message, no entries.
const PLUGIN: &str = concat!(
    r#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,"#,
    r#"plugin-type=FIX,type=Plugin","type":"read"},"value":{"SenderCompID":"ULB_BKRBDG","#,
    r#""TargetCompID":"ULB_PTBDG","BeginString":"FIX.4.2","State":"logged"},"status":200}"#,
);
/// A line that is not a message at all, which is still a text row.
///
/// It opens no frame, holds no pair and carries no document, so it states
/// nothing and yields no message - not even the entry-less `unknown` it used
/// to build.
const PROSE: &str = "no level printed by this plugin, and no pairs either";

/// A sentence whose prose carries an `=`, which is not a pair it separated.
///
/// The shape the none-one-or-many rule is named for. The run of named pairs is a bridge row
/// only where the line named a separator for it - a pipe, a `SOH`, one of the
/// spellings a log escapes it with, never whitespace - or the bridge marked a
/// key with `#`. This line does neither, so `seq=7` is prose and the line
/// states no message, where it used to state an `unknown` carrying `seq`.
const CHATTER: &str = "heartbeat emitted seq=7 to VENUE, no reply yet";

/// Every shape, in the order the capture holds them.
const CAPTURE: [&str; 8] = [
    TAGGED, WORKING, FILLED, NAMED, FIXML, PLUGIN, PROSE, CHATTER,
];

/// The messages the capture states: one per line but the two silent ones.
///
/// `PROSE` and `CHATTER` state no message - the first holds no pair at all,
/// the second holds one no separator was named for - so a message count is
/// two under the line count. A line is a text row whatever it holds, which is
/// why `CAPTURE.len()` is what a line count is compared against and this is
/// what a message count is.
const MESSAGES: usize = CAPTURE.len() - 2;

/// The capture as the bytes a log file holds.
fn corpus() -> Vec<u8> {
    let mut bytes = Vec::new();
    for line in CAPTURE {
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
    }
    bytes
}

/// A handle whose media type comes from its name, so `.log` reads as records.
fn handle() -> Buffer {
    Buffer::from_bytes(corpus()).with_media_type(
        Url::from_str("file:///capture.log")
            .expect("a URL")
            .media_type(),
    )
}

/// The text options a capture is read under.
fn text() -> RecordOptions {
    let mut options = TextOptions::new();
    options.parse_mimetype = true;
    options.into()
}

/// Every line the capture holds, as the text reader decodes them.
///
/// The one decode entry point, which is what the line door takes: the same
/// lines the batch path is built from, handed over rather than made again,
/// and therefore the honest comparison.
fn lines_of(source: &Buffer) -> Vec<TextLine> {
    let RecordOptions::Text(options) = text() else {
        panic!("a text read")
    };
    read_text_lines(source, &options)
        .expect("a line reader")
        .map(|line| line.expect("a line"))
        .collect()
}

/// One value as the reading it is, independent of how it is stored.
///
/// A packed code and the text it holds are one reading, and a null is one
/// reading whatever column it sits in. This is what lets the row-by-row path
/// and the batched one be compared without comparing the Arrow boundary
/// between them.
fn rendered(value: &Scalar) -> Option<String> {
    match value {
        Scalar::Null => None,
        held => Some(
            held.as_str()
                .map_or_else(|| format!("{held:?}"), ToString::to_string),
        ),
    }
}

/// One column of one batch, by name.
fn column(batch: &arrow_array::RecordBatch, name: &str) -> Vec<Scalar> {
    let at = batch
        .schema()
        .index_of(name)
        .unwrap_or_else(|_| panic!("a {name} column"));
    column_at(batch, at)
}

/// One column of one batch, by the tag its field carries.
fn tag_column(batch: &arrow_array::RecordBatch, tag: i32) -> Vec<Scalar> {
    column_at(batch, super::tag_index(batch, tag))
}

/// One column of one batch, by position.
fn column_at(batch: &arrow_array::RecordBatch, at: usize) -> Vec<Scalar> {
    let held = yggdryl::arrow::batch_to_value(batch).expect("the batch reads");
    held.as_sequence()
        .expect("rows")
        .iter()
        .map(|row| {
            row.as_sequence()
                .expect("a row")
                .get(at)
                .cloned()
                .unwrap_or(Scalar::Null)
        })
        .collect()
}

/// A Jolokia answer as a log line writes it: prose in front, prose behind.
///
/// The shape a bridge actually prints - a timestamp, the reader that logged
/// it, the level, then the document - with a duration written after it, which
/// is what a reader that assumed the document ended the line never saw.
const LOGGED: &str = concat!(
    r#"2026-08-14 06:46:22.150 [Jolokia] (DEBUG) Response: {"request":{"mbean":"#,
    r#""com.ullink.ulbridge.sessioninterfaces.plugins:name=Router_OrderRouting,"#,
    r#"plugin-type=FIX,type=Plugin","type":"read"},"value":{"Name":"Router_OrderRouting","#,
    r#""Version":"4.7.0","Category":"Fix BuySide","SenderCompID":"CLI.PROD.TRD","#,
    r#""TargetCompID":"ST.PROD","BeginString":"FIX.4.4","PrimaryHost":"172.97.127.90","#,
    r#""CurrentPort":9726,"State":"logged","Type":"I","NeedCFBReload":false,"#,
    r#""cm-extension":"4.7.0","IncomingMsgSeqNum":18336},"status":200} (12 ms)"#,
);

/// A wildcard read: one answer, a plugin per key, each named by its ObjectName.
const WILDCARD: &str = concat!(
    r#"{"request": {"mbean": "com.ullink.ulbridge.sessioninterfaces.plugins:*", "type": "read"},"#,
    r#" "value": {"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_BDG_DMZ_PCO,"#,
    r#"plugin-type=FIX,type=ConfigurationPlugin": {"Comment": "", "Category": "InterBridge","#,
    r#" "Prefix": "", "Name": "ULMSG_BROKER_BDG_DMZ_PCO", "LoadIsolation": 0, "Suffix": "","#,
    r#" "PriorityLevel": 5, "Version": "2.0.3"},"#,
    r#" "com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,"#,
    r#"plugin-type=FIX,type=Plugin": {"Name": "ULMSG_BROKER_TO_DMZ", "Version": "4.7.0"}},"#,
    r#" "status": 200}"#,
);

/// A JSON body that is not a Jolokia answer: a row's own bytes, and nothing
/// in them a FIX reader can read.
const STRANGER: &str = r#"{"a":1}"#;

/// The codec every shape in this capture is read under.
///
/// A capture of every dialect holds rows that state no type - a document,
/// and a FIXML element whose name no message code spells - and those are
/// what `DEFAULT_REFUSED_MSGTYPES` keeps out of a live session's read. A
/// fixture written to hold them is asking for them, and says so here; what
/// the default refuses is pinned on its own below.
fn codec() -> FixCodec {
    super::fixed_codec(registry()).with_exclude_msgtypes::<[&str; 0], &str>([])
}

#[test]
fn the_default_read_refuses_every_row_that_states_no_type() {
    let default = super::fixed_codec(registry());
    // Four of the six messages this capture states name a type. The FIXML
    // `Order` element and the Jolokia answer name none - `unknown` is what
    // the codec calls a row that states no type - and `unknown` is one of
    // the three types a codec refuses until a caller says otherwise.
    assert!(yggdryl::DEFAULT_REFUSED_MSGTYPES.contains(&"unknown"));
    assert_eq!(default.parse_lines(CAPTURE).count(), MESSAGES - 2);
    assert_eq!(codec().parse_lines(CAPTURE).count(), MESSAGES);
    for body in [FIXML, PLUGIN] {
        assert!(
            default
                .parse_line(body.as_bytes())
                .expect("a readable line")
                .next()
                .is_none(),
            "{body} states no type and is refused",
        );
        assert_eq!(
            codec()
                .sole_line(body.as_bytes())
                .expect("a message")
                .header()
                .msgtype(),
            "",
            "and states none when it is read",
        );
    }
}

#[test]
fn a_mixed_capture_reads_row_by_row_and_batched_to_the_same_messages() {
    let source = handle();
    let codec = codec();

    // Line by line: the text reader answers lines, and every line is read for
    // the messages its own body spells - none, one or many - the dialect
    // chosen per line, never per capture.
    let lines = lines_of(&source);
    assert_eq!(lines.len(), CAPTURE.len(), "a line in is a line out");

    let one_at_a_time: Vec<_> = codec
        .parse_text_lines(lines)
        .map(|held| held.expect("a message"))
        .collect();
    // A message per line but the last two: the prose opens no frame, states
    // no bridge pair and carries no document, and the chatter's `seq=7` is a
    // pair the line named no separator for - so both state nothing at all.
    assert_eq!(one_at_a_time.len(), MESSAGES);
    // The document's row is the one entry-less `unknown` - a body the codec
    // does not read, stating nothing - and every other row arrived with
    // entries, the FIXML row's `Order` among them.
    for (at, held) in one_at_a_time.iter().enumerate() {
        assert_eq!(held.entries().is_empty(), CAPTURE[at] == PLUGIN, "row {at}");
    }

    // Batched: the same capture through the batch door, which is the same
    // read with the rows held in Arrow instead of one at a time.
    let batched: Vec<_> = codec
        .parse_text_arrow_reader(source.read_arrow_reader(&text()).expect("a reader"))
        .expect("the batch reader opens")
        .map(|batch| batch.expect("a batch"))
        .collect();
    let rows: usize = batched.iter().map(arrow_array::RecordBatch::num_rows).sum();
    assert_eq!(rows, MESSAGES, "the batch path reads the same messages");
    // One batch, because the capture is far under the 128 MiB target.
    assert_eq!(batched.len(), 1);

    // The two paths agree on what each line said. They are compared on the
    // reading rather than on the storage: a batch value has crossed the Arrow
    // boundary and a message's has not, so logical values may differ in
    // physical representation while retaining the same declared reading.
    let batch = &batched[0];
    for tag in [11, 55, 37, 17, 39, 151, 6, 120] {
        let column = tag_column(batch, tag);
        for (at, message) in one_at_a_time.iter().enumerate() {
            let alone = message.get_by_tag(tag).unwrap_or(Scalar::Null);
            assert_eq!(
                rendered(&alone),
                rendered(&column[at]),
                "tag {tag} on row {at} differs between the two paths",
            );
        }
    }

    // The batch retains only arrival content the fixed columns cannot state;
    // the typed-column checks above prove represented facts arrive identically.
    let entries = column(batch, "fixentries");
    assert_eq!(entries.len(), one_at_a_time.len());
    assert!(entries.iter().all(|entry| entry.as_sequence().is_some()));

    // Which way a message moved is FIX's own tag 385, a code of its set,
    // retained once per row and shared by every message that row states.
    let directions = tag_column(batch, 385);
    assert_eq!(directions.len(), MESSAGES);
    // The bridge row wrote `recv` in front of its frame, and a verb the
    // transport wrote wins over everything else; a bare document states
    // nothing of which way it moved, so the batch door's own pin fills it.
    assert_eq!(directions[3].as_str(), Some("R"));
    assert_eq!(directions[5].as_str(), Some("S"));
}

#[test]
fn every_dialect_in_one_capture_is_read_as_itself() {
    let codec = codec();

    // A framed order behind prose: the frame is located and the prose dropped.
    let order = codec.sole_line(TAGGED.as_bytes()).expect("an order");
    assert_eq!(order.by_tag(11).unwrap().as_str(), Some("ORDER-1"));
    assert_eq!(order.by_tag(55).unwrap().as_str(), Some("AAPL"));

    // A bridge row keyed by name, with its group packed into one occurrence.
    let bridge = codec.sole_line(NAMED.as_bytes()).expect("a bridge row");
    assert_eq!(bridge.by_tag(55).unwrap().as_str(), Some("TTF"));
    let group = bridge
        .by_name("parties")
        .expect("the group the counter heads");
    let parties = group.as_sequence().expect("a list");
    assert_eq!(parties.len(), 1);
    let party = parties[0].as_sequence().expect("one occurrence");
    assert!(
        party.iter().any(|held| held.as_str() == Some("BUYSIDE")),
        "the occurrence carries the members it packed",
    );
    // What makes that run of named keys a bridge row rather than prose
    // carrying an `=` is the separator the line named for it - the pipe - and
    // the `#` the bridge marked its counter with. Named neither way, the same
    // shape of run is prose: whitespace names no separator, so the line states
    // no message at all.
    let paired = codec
        .sole_line(b"ACCOUNT=A1|SIDE=1")
        .expect("a bridge row the pipe separated");
    assert_eq!(paired.by_tag(1).unwrap().as_str(), Some("A1"));
    assert!(
        codec
            .parse_line(b"After Enrichment -> ACCOUNT=A1 CLIENTID=B2")
            .expect("a readable line")
            .next()
            .is_none(),
        "a run of named pairs the line separated with whitespace is prose",
    );

    // A FIXML row, whose fields are attributes rather than pairs.
    let fixml = codec.sole_line(FIXML.as_bytes()).expect("a FIXML row");
    assert_eq!(fixml.by_tag(11).unwrap().as_str(), Some("ORDER-2"));
    // The same document behind a transport's prose, with whitespace either
    // side: the document opens where the tag opens, whatever was trimmed off
    // the line's end.
    let prosed = format!("  Sending : {FIXML}  \t");
    let behind = codec
        .sole_line(prosed.as_bytes())
        .expect("a FIXML row behind prose");
    assert_eq!(behind.by_tag(11).unwrap().as_str(), Some("ORDER-2"));
    assert_eq!(behind.by_tag(54).unwrap(), fixml.by_tag(54).unwrap());
    assert_eq!(behind.entries().len(), fixml.entries().len());

    // A line that is not a message states no message at all - never an error:
    // it opens no frame, states no bridge pair and carries no document, so
    // there is nothing in it to read. It reads without failing, which is the
    // fact that matters: one such line must not end a run over ten million.
    for line in [PROSE, CHATTER] {
        assert!(
            codec
                .parse_line(line.as_bytes())
                .expect("a readable line")
                .next()
                .is_none(),
            "{line:?} states no message",
        );
    }
}

#[test]
fn a_parse_fills_the_columns_and_the_wire_re_emits_them() {
    let codec = codec();

    // The part-filled report states what was ordered and what was done, so it
    // has stated what is left and what the fill was worth - filled as it is
    // read, with no second pass.
    let filled = codec.sole_line(WORKING.as_bytes()).expect("a report");
    assert_eq!(filled.by_tag(151).unwrap(), super::decimal("60"));
    assert_eq!(filled.by_tag(381).unwrap(), super::decimal("420"));

    // A date arrives compact and reads as that day's midnight, stating no zone
    // because a local market date has none.
    assert_ne!(filled.by_tag(64).expect("a settlement date"), Scalar::Null);

    // The closing fill settles in the currency it was dealt in, at the rate it
    // stated - Appendix O read as the implication it is.
    let closed = codec.sole_line(FILLED.as_bytes()).expect("a report");
    assert_eq!(closed.by_tag(151).unwrap(), super::decimal("0"));
    assert_eq!(closed.by_tag(120).unwrap().as_str(), Some("EUR"));

    // What was derived is the message's, so the wire carries it beside what
    // arrived: a re-emitted report states the leaves neither line stated.
    // `LeavesQty(151)` is one of the event's own facts rather than a child
    // of the content row, so it is on the wire without being an entry.
    for line in [WORKING, FILLED] {
        let held = codec.sole_line(line.as_bytes()).expect("a report");
        assert!(
            !held.entries().iter().any(|entry| entry.tag() == 151),
            "{line}"
        );
        let wire = held.into_text('|').unwrap();
        assert!(
            wire.starts_with("8=FIX.4.4|35=8|49=VENUE|56=BUYSIDE|"),
            "{wire}"
        );
        assert!(wire.contains("|151="), "{wire}");
    }
}

#[test]
fn a_document_is_one_unknown_row_at_every_door() {
    // The classifier reads past the prose in front and answers
    // `application/json`, which is what the document is, and it names no
    // message type for it: what a document says is not read. The codec
    // locates the document to its own close rather than to the end of the
    // line, so what a transport writes behind it is prose too, and the row
    // is one `unknown` message - no entries, no type - carrying what the
    // prose in front stated: the half `Response:` names.
    assert_eq!(
        yggdryl::MimeType::infer_bytes(LOGGED.as_bytes()),
        yggdryl::MimeType::JSON
    );
    assert_eq!(FixCodec::infer_msgtype_bytes(LOGGED.as_bytes()), None);

    let codec = codec();
    let message = codec
        .sole_line(LOGGED.as_bytes())
        .expect("the line carries one document, and so one message");
    assert_eq!(message.as_field().name(), "unknown");
    assert!(message.entries().is_empty());
    assert!(message.get_by_tag(35).is_none_or(|held| held.is_null()));
    assert!(
        message
            .get_by_name("SenderCompID")
            .is_none_or(|held| held.is_null())
    );
    assert_eq!(message.by_tag(385).unwrap().as_str(), Some("R"));

    // A stranger's `{"a":1}` answers exactly what a Jolokia answer does, at
    // the byte door and at the line door alike: one message stating no type,
    // no entries, and nothing on the wire but the version every built
    // message states.
    let line = TextLine::from_bytes(
        0,
        TextBytes::from_bytes(STRANGER.as_bytes()).unwrap(),
        std::sync::Arc::new(yggdryl::text::TextOptions::new()),
    )
    .unwrap();
    let unknown = |message: yggdryl::FixMsg, door: &str| {
        assert_eq!(message.as_field().name(), "unknown", "{door}");
        assert!(message.entries().is_empty(), "{door}");
        assert_eq!(message.into_bytes(b'|'), b"8=FIX.4.4|", "{door}");
        assert!(
            message.get_by_tag(35).is_none_or(|held| held.is_null()),
            "{door}"
        );
    };
    unknown(
        codec
            .sole_line(STRANGER.as_bytes())
            .expect("the byte door reads one message"),
        "the byte door",
    );
    unknown(
        super::sole_message(codec.parse_text_line(&line).expect("a readable row"))
            .expect("the line door reads one message"),
        "the line door",
    );

    // A wildcard read answers for two plugins, and the row is one message all
    // the same: a document is not read, so there is no per-plugin expansion.
    let wildcard = codec
        .sole_line(WILDCARD.as_bytes())
        .expect("one message, not one per plugin");
    assert_eq!(wildcard.as_field().name(), "unknown");
    assert!(wildcard.get_by_name("Name").is_none());

    // And on the batch door a row that carried a document is a row carrying
    // its own source columns, exactly as it is at the other two.
    let field = yggdryl::StructType::from_fields([
        yggdryl::DataType::utf8().required_field("url"),
        yggdryl::DataType::Int64.required_field("rownum"),
        yggdryl::DataType::binary().required_field("body"),
    ])
    .map(yggdryl::DataType::from)
    .unwrap()
    .required_field("capture");
    let value = Scalar::from_sequence([(41_i64, WILDCARD), (42_i64, WORKING)].map(
        |(rownum, body)| {
            Scalar::from_sequence([
                Scalar::from("file:///bulk.log"),
                Scalar::from(rownum),
                Scalar::from(body.as_bytes().to_vec()),
            ])
        },
    ));
    let source = yggdryl::arrow::batch_from_value(&field, &value).unwrap();
    let batch = codec
        .parse_text_arrow_reader(yggdryl::arrow::batch_reader(source.schema(), [source]))
        .expect("the batch door opens")
        .next()
        .expect("one batch")
        .expect("a batch");
    assert_eq!(batch.num_rows(), 2);
    assert_eq!(
        column(&batch, "rownum"),
        [Scalar::from(41_i64), Scalar::from(42_i64)]
    );
    assert_eq!(
        column(&batch, "url"),
        vec![Scalar::from("file:///bulk.log"); 2]
    );
    assert_eq!(
        column(&batch, "body"),
        [
            Scalar::from(WILDCARD.as_bytes().to_vec()),
            Scalar::from(WORKING.as_bytes().to_vec()),
        ]
    );
    assert_eq!(tag_column(&batch, 35), [Scalar::Null, Scalar::from("8")]);
}

#[test]
fn a_json_document_is_one_unknown_and_any_other_unreadable_body_is_none() {
    // Reading is not refusing. Every one of these is a body a row really
    // carried, and none has a `Result` left to unwrap. What makes a body a
    // document is its shape alone: an object, or an array of objects,
    // opening before any `=` on the line and closing on its own last byte.
    // Such a body is one `unknown`, whatever it says - an error-only
    // answer, a request not yet answered, a wildcard that selected nothing
    // and a bulk answer of one object alike.
    let codec = codec();
    for body in [
        STRANGER,
        r#"[{"request":{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}},2]"#,
        r#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=Gone,plugin-type=FIX,type=Plugin","type":"read"},"error":"InstanceNotFoundException","status":404}"#,
        r#"{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"}"#,
        r#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"value":{},"status":200}"#,
    ] {
        let message = codec
            .sole_line(body.as_bytes())
            .unwrap_or_else(|error| panic!("{body:?}: {error}"));
        assert_eq!(message.as_field().name(), "unknown", "{body:?}");
        assert!(message.entries().is_empty(), "{body:?}");
    }
    // Everything else JSON could spell is no document, and a body that
    // opens a brace without a member is not JSON at all: each opens no
    // frame, states no bridge pair and carries no document, so the row
    // states no message - never an error.
    for body in [
        "null",
        "true",
        "1",
        r#""text""#,
        "[]",
        "[1]",
        "{not json at all",
    ] {
        assert!(
            codec
                .parse_line(body.as_bytes())
                .unwrap_or_else(|error| panic!("{body:?}: {error}"))
                .next()
                .is_none(),
            "{body:?} states a message",
        );
    }
}

#[test]
fn a_capture_writes_back_what_each_message_emits() {
    let source = handle();
    let codec = codec().with_separator(b'|');
    let messages = codec
        .parse_lines(CAPTURE)
        .map(|message| message.expect("a source message"))
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), MESSAGES);

    let reader = codec
        .parse_text_arrow_reader(source.read_arrow_reader(&text()).expect("a reader"))
        .expect("the batch reader opens");

    // The wire is rebuilt from each row's arrival record and the facts it
    // holds typed, which is what the line read emits for the same line.
    let mut written: Vec<u8> = Vec::new();
    let rows = codec
        .write_arrow_reader(reader, &mut written)
        .expect("the capture writes");
    // One line written per message, and the two lines that state none reach
    // the writer as no row at all, so the capture comes back two lines
    // shorter than it went in.
    assert_eq!(rows, MESSAGES as u64);

    let held = String::from_utf8(written).expect("the wire is text here");
    let lines: Vec<&str> = held.lines().collect();
    assert_eq!(lines.len(), MESSAGES);
    let target = super::format_target(codec.registry());
    let group_columns = target
        .fields()
        .iter()
        .enumerate()
        .filter_map(|(at, field)| {
            field
                .as_fix()
                .counter()
                .expect("valid FIX metadata")
                .filter(|counter| {
                    !(yggdryl::CRATE_TAG_MIN..=yggdryl::CRATE_TAG_MAX).contains(counter)
                })
                .map(|_| (at, field.name()))
        })
        .collect::<Vec<_>>();
    assert!(
        !group_columns.is_empty(),
        "the fixed row has protocol groups"
    );
    for (at, (message, line)) in messages.iter().zip(&lines).enumerate() {
        let emitted = message.into_text('|').expect("the source message emits");
        let mut source_tokens = emitted
            .split('|')
            .filter(|token| !token.is_empty())
            .collect::<Vec<_>>();
        let mut written_tokens = line
            .split('|')
            .filter(|token| !token.is_empty())
            .collect::<Vec<_>>();
        source_tokens.sort_unstable();
        written_tokens.sort_unstable();
        assert_eq!(written_tokens, source_tokens, "row {at} changed a token");

        // Root fields may move into schema order, but header/trailer bands and
        // every repeating group's member and occurrence order remain wire
        // structure rather than a token-set property.
        if emitted.contains("|10=") {
            assert!(line.starts_with("8="), "row {at}: {line}");
            assert!(
                line.rsplit('|')
                    .nth(1)
                    .is_some_and(|token| token.starts_with("10=")),
                "row {at}: {line}"
            );
        }
        let reparsed = codec
            .sole_line(line.as_bytes())
            .unwrap_or_else(|error| panic!("row {at} did not parse: {error}"));
        let source_row = message.into_row(&target).expect("the source row");
        let reparsed_row = reparsed.into_row(&target).expect("the reparsed row");
        let source_cells = source_row.as_sequence().expect("source columns");
        let reparsed_cells = reparsed_row.as_sequence().expect("reparsed columns");
        for (column, name) in &group_columns {
            assert_eq!(
                reparsed_cells[*column], source_cells[*column],
                "row {at} changed {name} occurrence order"
            );
        }
    }
    for silent in [PROSE, CHATTER] {
        assert!(
            !lines.contains(&silent),
            "a line that stated no message writes back none",
        );
    }
    // And the capture's own columns are the capture's: a row read back off
    // a capture states the message, never the body it was read from.
    for line in &lines {
        assert!(!line.contains("|body="), "{line}");
        assert!(!line.contains("|url="), "{line}");
    }
}
