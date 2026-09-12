//! A mixed capture, end to end: log bytes in, enriched messages out.
//!
//! Every other suite in these phases reads one shape against one entry point.
//! This one is the whole path a desk actually takes, once, on a file holding
//! every shape at once: a `.log` handle read as text records, those records
//! read as messages by the codec, and each message filled with what it
//! implies - row by row and in batches, which have to agree.
//!
//! It is the acceptance test for the layers under it. Where a unit test says
//! one rule works, this says the rules compose: the text reader's row count
//! survives the codec, the codec's dialect choice survives enrichment, and
//! enrichment leaves the wire exactly as it arrived so the round trip still
//! closes.

use super::OneMessage;
use super::path;

use std::sync::Arc;

use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::media::text::{TextBytes, TextLine, TextOptions, read_text_lines};
use yggdryl::{FixCodec, FixRegistry, IOMedia, Scalar, Url};

/// The committed dictionary, plus the bridge's own vocabulary.
fn registry() -> Arc<FixRegistry> {
    super::ulbridge_registry()
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
/// A Jolokia read of a session interface, which is a document rather than pairs.
const ULCONFIG: &str = concat!(
    r#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=ULMSG_BROKER_TO_DMZ,"#,
    r#"plugin-type=FIX,type=Plugin","type":"read"},"value":{"SenderCompID":"ULB_BKRBDG","#,
    r#""TargetCompID":"ULB_PTBDG","BeginString":"FIX.4.2","State":"logged"},"status":200}"#,
);
/// A line that is not a message at all, which is still a row.
const PROSE: &str = "no level printed by this plugin, and no pairs either";

/// Every shape, in the order the capture holds them.
const CAPTURE: [&str; 7] = [TAGGED, WORKING, FILLED, NAMED, FIXML, ULCONFIG, PROSE];

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
    options.parse_direction = true;
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

#[test]
fn a_mixed_capture_reads_row_by_row_and_batched_to_the_same_messages() {
    let registry = registry();
    let source = handle();
    let codec = FixCodec::new(Arc::clone(&registry));

    // Line by line: the text reader answers lines, and every line is read as
    // the message its own body spells - the dialect chosen per line, never
    // per capture.
    let lines = lines_of(&source);
    assert_eq!(lines.len(), CAPTURE.len(), "a line in is a line out");

    let one_at_a_time: Vec<_> = codec
        .parse_text_lines(lines)
        .map(|held| held.and_then(|held| codec.enrich_message(held)))
        .map(|held| held.expect("a message"))
        .collect();
    assert_eq!(one_at_a_time.len(), CAPTURE.len());

    // Batched: the same capture through the batch reader, which is the same
    // read with the rows held in Arrow instead of one at a time, then the
    // same filling over the rows.
    let parsed = codec
        .parse_text_arrow_reader(source.read_arrow_reader(&text()).expect("a reader"))
        .expect("the batch reader opens");
    let batched: Vec<_> = codec
        .enrich_messages_arrow_reader(parsed)
        .expect("the filling reader opens")
        .map(|batch| batch.expect("a batch"))
        .collect();
    let rows: usize = batched.iter().map(arrow_array::RecordBatch::num_rows).sum();
    assert_eq!(rows, CAPTURE.len(), "the batch path keeps the row count");
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
            let alone = message.get_by_tag(tag).cloned().unwrap_or(Scalar::Null);
            assert_eq!(
                rendered(&alone),
                rendered(&column[at]),
                "tag {tag} on row {at} differs between the two paths",
            );
        }
    }

    // And the arrival record - the fact the wire is rebuilt from - is the same
    // on both paths, so the round trip closes either way.
    let entries = column(batch, "nofixentries");
    for (at, message) in one_at_a_time.iter().enumerate() {
        assert_eq!(
            message.entries().len(),
            entries[at]
                .as_sequence()
                .map(<[Scalar]>::len)
                .unwrap_or_default(),
            "row {at} carries a different arrival record",
        );
    }
}

#[test]
fn every_dialect_in_one_capture_is_read_as_itself() {
    let registry = registry();
    let codec = FixCodec::new(Arc::clone(&registry));

    // A framed order behind prose: the frame is located and the prose dropped.
    let order = codec.one_line(TAGGED.as_bytes(), true).expect("an order");
    assert_eq!(order.by_tag(11).unwrap().as_str(), Some("ORDER-1"));
    assert_eq!(order.by_tag(55).unwrap().as_str(), Some("AAPL"));

    // A bridge row keyed by name, with its group packed into one occurrence.
    let bridge = codec
        .one_line(NAMED.as_bytes(), true)
        .expect("a bridge row");
    assert_eq!(bridge.by_tag(55).unwrap().as_str(), Some("TTF"));
    // The packed occurrence split into the group the counter heads: one
    // occurrence, carrying the members the bridge packed into its value.
    let parties = bridge
        .by_name("parties")
        .expect("the group the counter heads")
        .as_sequence()
        .expect("a list");
    assert_eq!(parties.len(), 1);
    let party = parties[0].as_sequence().expect("one occurrence");
    assert!(
        party.iter().any(|held| held.as_str() == Some("BUYSIDE")),
        "the occurrence carries the members it packed",
    );

    // A FIXML row, whose fields are attributes rather than pairs.
    let fixml = codec.one_line(FIXML.as_bytes(), true).expect("a FIXML row");
    assert_eq!(fixml.by_tag(11).unwrap().as_str(), Some("ORDER-2"));
    // The same document behind a transport's prose, with whitespace either
    // side: the document opens where the tag opens, whatever was trimmed off
    // the line's end.
    let prosed = format!("  Sending : {FIXML}  \t");
    let behind = codec
        .one_line(prosed.as_bytes(), true)
        .expect("a FIXML row behind prose");
    assert_eq!(behind.by_tag(11).unwrap().as_str(), Some("ORDER-2"));
    assert_eq!(behind.by_tag(54).unwrap(), fixml.by_tag(54).unwrap());
    assert_eq!(behind.entries().len(), fixml.entries().len());

    // A bridge configuration document, read as the document it is. It is read
    // under the bridge's own dialect, which is what gives its envelope fields
    // somewhere to land; a name the specification publishes still resolves,
    // because a name is looked for in the pinned branch and then the standard
    // one.
    let branch = yggdryl::FixBranch::from_str(yggdryl::ULBRIDGE_BRANCH).expect("a branch");
    let bridge = FixCodec::new(Arc::clone(&registry)).with_branch(&branch);
    let config = bridge
        .one_line(ULCONFIG.as_bytes(), true)
        .expect("a configuration document");
    assert_eq!(config.branch(), &branch);
    assert_eq!(config.as_field().as_fix().branch().unwrap(), branch);
    // The envelope is what the exchange was, and it types: a status is a
    // number rather than the text it arrived as.
    assert_eq!(
        config.by_tag(yggdryl::STATUS_TAG).unwrap(),
        &Scalar::from(200_i64)
    );
    // One MBean produces one message, with standard FIX fields on their own tags.
    assert_eq!(
        config.by_path(&path("SenderCompID")).unwrap(),
        &Scalar::from("ULB_BKRBDG"),
    );
    assert_eq!(
        config.by_path(&path("MBeanType")).unwrap(),
        &Scalar::from("Plugin"),
    );

    // A line that is not a message is a message with nothing in it, never an
    // error: one corrupt line must not end a run over ten million.
    let prose = codec.one_line(PROSE.as_bytes(), true).expect("a row");
    assert_eq!(prose.get_by_tag(35), None);
}

#[test]
fn enrichment_fills_the_columns_and_leaves_the_wire_alone() {
    let registry = registry();
    let codec = FixCodec::new(registry);

    // The part-filled report states what was ordered and what was done, so it
    // has stated what is left and what the fill was worth.
    let bare = codec.one_line(WORKING.as_bytes(), false).expect("a report");
    let filled = codec.one_line(WORKING.as_bytes(), true).expect("a report");
    assert_eq!(bare.get_by_tag(151), None);
    assert_eq!(filled.by_tag(151).unwrap(), &Scalar::from(60.0_f64));
    assert_eq!(filled.by_tag(381).unwrap(), &Scalar::from(420.0_f64));

    // A date arrives compact and reads as that day's midnight, stating no zone
    // because a local market date has none.
    let settled = filled.by_tag(64).expect("a settlement date");
    assert_ne!(settled, &Scalar::Null);

    // The closing fill settles in the currency it was dealt in, at the rate it
    // stated - Appendix O read as the implication it is.
    let closed = codec.one_line(FILLED.as_bytes(), true).expect("a report");
    assert_eq!(closed.by_tag(151).unwrap(), &Scalar::from(0.0_f64));
    assert_eq!(closed.by_tag(120).unwrap().as_str(), Some("EUR"));

    // And none of it touched the arrival record, so the capture still
    // re-emits the bytes it was read from.
    for line in [WORKING, FILLED] {
        let plain = codec.one_line(line.as_bytes(), false).expect("a report");
        let held = codec.one_line(line.as_bytes(), true).expect("a report");
        assert_eq!(plain.entries(), held.entries());
        assert_eq!(held.into_bytes(b'|'), line.as_bytes());
    }
}

#[test]
fn an_enriched_capture_still_writes_back_the_wire_it_was_read_from() {
    let registry = registry();
    let source = handle();
    let codec = FixCodec::new(Arc::clone(&registry)).with_separator(b'|');

    let parsed = codec
        .parse_text_arrow_reader(source.read_arrow_reader(&text()).expect("a reader"))
        .expect("the batch reader opens");
    let reader = codec
        .enrich_messages_arrow_reader(parsed)
        .expect("the filling reader opens");

    // The wire is rebuilt from each row's arrival record, never from its
    // columns, so a filled column cannot leak into a re-emitted frame.
    let mut written: Vec<u8> = Vec::new();
    let rows = codec
        .write_arrow_reader(reader, &mut written)
        .expect("the capture writes");
    assert_eq!(rows, CAPTURE.len() as u64);

    // The two framed reports come back exactly as they arrived, filled or not.
    let held = String::from_utf8(written).expect("the wire is text here");
    let lines: Vec<&str> = held.lines().collect();
    assert_eq!(lines.len(), CAPTURE.len());
    assert!(lines.contains(&WORKING), "the report re-emits exactly");
    assert!(lines.contains(&FILLED), "the fill re-emits exactly");
}

#[test]
fn the_batch_states_what_each_line_was_and_which_way_it_moved() {
    let registry = registry();
    let source = handle();
    let codec = FixCodec::new(registry);

    let parsed = codec
        .parse_text_arrow_reader(source.read_arrow_reader(&text()).expect("a reader"))
        .expect("the batch reader opens");
    let batch = codec
        .enrich_messages_arrow_reader(parsed)
        .expect("the filling reader opens")
        .next()
        .expect("a batch")
        .expect("a batch");

    // The capture's own columns lead the row and are carried through, which is
    // what lets a monitor join a parsed capture back to its source by
    // position.
    let directions = column(&batch, "direction");
    assert_eq!(directions.len(), CAPTURE.len());
    // The bridge row wrote `recv` in front of its frame, and a verb the
    // transport wrote wins over everything else.
    assert_eq!(directions[3].as_str(), Some("RECV"));
    // The configuration document echoes back the request it answers, so it
    // came back rather than went out.
    assert_eq!(directions[5].as_str(), Some("RECV"));

    // And the enrichment is visible in the columns, not just on the message.
    let leaves = tag_column(&batch, 151);
    assert_eq!(leaves[1], Scalar::from(60.0_f64), "the part-filled report");
    assert_eq!(leaves[2], Scalar::from(0.0_f64), "the closing fill");
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

#[test]
fn a_document_is_read_out_of_the_line_that_carries_it() {
    // The classifier already read past the prose in front; the reader reads to
    // the document's own close rather than to the end of the line, so what a
    // transport writes behind it is prose too.
    assert_eq!(
        yggdryl::MimeType::infer_bytes(LOGGED.as_bytes()),
        yggdryl::MimeType::ULCONFIG
    );
    assert_eq!(
        FixCodec::infer_msgtype_bytes(LOGGED.as_bytes()),
        Some(&b"Plugin"[..])
    );

    let branch = yggdryl::FixBranch::from_str(yggdryl::ULBRIDGE_BRANCH).unwrap();
    let codec = FixCodec::new(registry()).with_branch(&branch);
    let message = codec
        .one_ulconfig_line(LOGGED.as_bytes(), false)
        .expect("the document the line carries");
    // Standard FIX and bridge attributes share the configuration's flat row.
    assert_eq!(
        message
            .get_by_path(&path("SenderCompID"))
            .and_then(Scalar::as_str),
        Some("CLI.PROD.TRD")
    );
    assert_eq!(
        message
            .get_by_path(&path("Version"))
            .and_then(Scalar::as_str),
        Some("4.7.0")
    );
    // A `[Jolokia]` in the prose opens no document: only an object whose first
    // member is quoted, or an array of those, does.
    assert!(message.get_by_tag(yggdryl::MBEAN_TAG).is_some());
}

#[test]
fn every_plugin_a_document_answers_for_crosses_both_ways() {
    // A wildcard read answers a plugin per key; a single read answers one, and
    // the request's own MBean names it. Both are the same walk.
    let held: Vec<yggdryl::UlPlugin> = yggdryl::UlPlugin::from_json_bytes(WILDCARD.as_bytes())
        .expect("a readable answer")
        .collect();
    assert_eq!(held.len(), 2);
    assert_eq!(held[0].name(), Some("ULMSG_BROKER_BDG_DMZ_PCO"));
    assert_eq!(held[0].mbean_type(), Some("ConfigurationPlugin"));
    assert_eq!(held[0].plugin_type(), Some("FIX"));
    assert_eq!(held[0].category(), Some("InterBridge"));
    assert_eq!(held[0].version(), Some("2.0.3"));
    assert_eq!(held[1].name(), Some("ULMSG_BROKER_TO_DMZ"));
    assert_eq!(held[1].mbean_type(), Some("Plugin"));

    // The spelling is folded the way every other name in this crate is.
    assert_eq!(
        held[0].get("priority_level").and_then(Scalar::as_i64),
        Some(5)
    );

    let single: Vec<yggdryl::UlPlugin> = yggdryl::UlPlugin::from_json_bytes(LOGGED.as_bytes())
        .expect("a readable line")
        .collect();
    assert_eq!(single.len(), 1);
    assert_eq!(single[0].name(), Some("Router_OrderRouting"));
    assert_eq!(single[0].state(), Some("logged"));

    // And back to a typed message, and out of one again: the crossing keeps
    // the ObjectName, the attributes and their types.
    let branch = yggdryl::FixBranch::from_str(yggdryl::ULBRIDGE_BRANCH).unwrap();
    let codec = FixCodec::new(registry()).with_branch(&branch);
    let message = held[0].into_fixmsg(&codec).expect("a typed message");
    assert_eq!(
        message
            .get_by_path(&path("PriorityLevel"))
            .and_then(Scalar::as_i64),
        Some(5)
    );
    let back = yggdryl::UlPlugin::from_fixmsg(&message).expect("one flat configuration");
    assert_eq!(back.mbean(), held[0].mbean());
    assert_eq!(back.name(), held[0].name());
    assert_eq!(back.version(), held[0].version());
}

#[test]
fn a_wildcard_capture_expands_messages_and_repeats_its_source_columns() {
    let branch = yggdryl::FixBranch::from_str(yggdryl::ULBRIDGE_BRANCH).unwrap();
    let codec = FixCodec::new(registry()).with_branch(&branch);
    let messages = codec
        .parse_line(WILDCARD.as_bytes())
        .expect("a wildcard response")
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("each configuration converts");
    assert_eq!(messages.len(), 2);
    for (message, expected) in messages
        .iter()
        .zip(["ULMSG_BROKER_BDG_DMZ_PCO", "ULMSG_BROKER_TO_DMZ"])
    {
        assert_eq!(message.by_name("Name").unwrap().as_str(), Some(expected));
        assert_eq!(
            message.by_tag(yggdryl::STATUS_TAG).unwrap(),
            &Scalar::from(200_i64)
        );
        assert_eq!(
            message.by_tag(yggdryl::MBEAN_TAG).unwrap().as_str(),
            Some("com.ullink.ulbridge.sessioninterfaces.plugins:*"),
        );
        assert!(
            message
                .by_name("SessionInterface")
                .unwrap()
                .as_str()
                .unwrap()
                .contains(expected)
        );
        assert!(message.get_by_name("SessionInterfaces").is_none());
    }

    let lines: Vec<TextLine> = [WILDCARD, WORKING]
        .map(|body| TextLine::new(0, TextBytes::from_bytes(body.as_bytes()).unwrap()))
        .into();
    let read = codec
        .parse_text_lines(lines)
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("line iteration expands each configuration");
    assert_eq!(read.len(), 3);
    assert_eq!(read[0].as_value(), messages[0].as_value());
    assert_eq!(read[1].as_value(), messages[1].as_value());
    assert_eq!(read[2].by_tag(37).unwrap().as_str(), Some("O-9"));

    let field = yggdryl::DataType::from_fields([
        yggdryl::DataType::utf8().required_field("url"),
        yggdryl::DataType::Int64.required_field("rownum"),
        yggdryl::DataType::binary().required_field("body"),
    ])
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
        .expect("bulk responses expand batch rows")
        .next()
        .expect("one batch")
        .expect("a batch");
    assert_eq!(batch.num_rows(), 3);
    assert_eq!(
        column(&batch, "rownum"),
        [
            Scalar::from(41_i64),
            Scalar::from(41_i64),
            Scalar::from(42_i64)
        ]
    );
    assert_eq!(
        column(&batch, "url"),
        vec![Scalar::from("file:///bulk.log"); 3]
    );
    assert_eq!(
        column(&batch, "body"),
        [
            Scalar::from(WILDCARD.as_bytes().to_vec()),
            Scalar::from(WILDCARD.as_bytes().to_vec()),
            Scalar::from(WORKING.as_bytes().to_vec()),
        ]
    );
}
