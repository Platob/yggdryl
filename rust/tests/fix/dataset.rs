//! A bridge's own log as one dataset, read a line at a time and a batch at a
//! time, and required to agree.
//!
//! `ulbridge.log` is a second of a ULBridge's own capture, anonymized, and
//! then every shape a bridge writes that the second happened not to hold:
//! Jolokia exchanges, FIXML behind a verb, frames spelled with `^A` and
//! `<SOH>`, a FIXT logon, `35=UL` frames carrying a whole row in their
//! `XmlData`, a bridge row with null spellings, a statistics line, an empty
//! body, a warning, and a cancel/reject flow the capture ends on. The text
//! reader frames every line under the bridge's own row header; the FIX
//! reader then answers for each line, and the batch door is required to
//! answer the same.

use std::sync::Arc;

use arrow_array::RecordBatch;
use yggdryl::graph::{Event, MarketElement};
use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::text::{TextLine, TextOptions, read_text_lines};
use yggdryl::{FixCodec, FixMsg, FixRegistry, IOMedia, Scalar, Timezone, Url};

use super::path;

/// The capture, exactly as the bridge wrote it.
const LOG: &[u8] = include_bytes!("ulbridge.log");

/// How many lines the capture holds.
const LINES: usize = 144;

/// How many messages the capture reads as under the codec's own defaults:
/// every frame, bridge row and document the log carries, less the session
/// traffic `DEFAULT_REFUSED_MSGTYPES` names.
const ROWS: usize = 79;

/// How many it reads as when nothing is refused: the same lines plus the
/// heartbeats, the test request and the rows that state no type at all.
const EVERY_ROW: usize = 94;

fn registry() -> Arc<FixRegistry> {
    super::committed_registry()
}

/// The log as the `.log` handle a reader opens.
fn source() -> &'static Buffer {
    static SOURCE: std::sync::OnceLock<Buffer> = std::sync::OnceLock::new();
    SOURCE.get_or_init(|| {
        Buffer::from_bytes(LOG.to_vec()).with_media_type(
            Url::from_str("file:///ulbridge.log")
                .expect("a URL")
                .media_type(),
        )
    })
}

/// The text options a bridge log is read under: its own row header, each
/// line numbered and classified.
fn reading() -> RecordOptions {
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the bridge's row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_mimetype = true;
    options.into()
}

/// What the bridge's row header captures, in the order it declares them.
fn header_captures() -> Vec<String> {
    let RecordOptions::Text(options) = reading() else {
        panic!("a text read")
    };
    options.capture_names().map(ToOwned::to_owned).collect()
}

/// The codec every read uses: the bridge's dictionary, and what the run's
/// captures are called, because a line answers them by position and only
/// this boundary knows what each position means.
fn codec() -> FixCodec {
    super::fixed_codec(registry()).with_capture_names(header_captures())
}

/// Every line the capture holds, as the text reader decodes them.
fn text_lines() -> Vec<TextLine> {
    let RecordOptions::Text(options) = reading() else {
        panic!("a text read")
    };
    read_text_lines(source(), &options)
        .expect("a line reader")
        .map(|line| line.expect("a line"))
        .collect()
}

/// Every message the line door answers for the capture, in line order.
fn line_messages(codec: &FixCodec) -> Vec<FixMsg> {
    codec
        .parse_text_lines(text_lines())
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("every line reads")
}

/// The batch door's answer for the whole capture: a capture row in, one FIX
/// row per message out.
fn batches(codec: &FixCodec) -> Vec<RecordBatch> {
    codec
        .parse_text_arrow_reader(source().read_arrow_reader(&reading()).expect("a reader"))
        .expect("the batch reader opens")
        .map(|batch| batch.expect("a batch"))
        .collect()
}

/// One read's rows, by value, beside the schema they landed under.
fn rows_of(batches: &[RecordBatch]) -> (yggdryl::Field, Vec<Vec<Scalar>>) {
    let schema = yggdryl::Field::from_arrow_schema("row", &batches[0].schema())
        .expect("the batch schema reads");
    let mut rows = Vec::new();
    for batch in batches {
        let held = yggdryl::arrow::batch_to_value(batch).expect("the batch reads");
        for row in held.as_sequence().expect("rows") {
            rows.push(row.as_sequence().expect("a row").to_vec());
        }
    }
    (schema, rows)
}

#[test]
fn the_codec_refuses_the_session_traffic_and_reads_every_other_line() {
    let codec = codec();
    let lines = text_lines();
    assert_eq!(lines.len(), LINES);

    let read = line_messages(&codec);
    assert_eq!(read.len(), ROWS);
    for message in &read {
        let msgtype = message.header().msgtype();
        assert!(
            codec.reads_msgtype(msgtype),
            "the codec refuses {msgtype} and still read one"
        );
        assert!(!yggdryl::DEFAULT_REFUSED_MSGTYPES.contains(&msgtype));
    }

    // A caller that says it wants everything gets the session traffic too,
    // and that is the whole of the difference: the fifteen more lines are
    // heartbeats, one test request, and the rows that state no type.
    let every = codec.clone().with_exclude_msgtypes::<[&str; 0], &str>([]);
    let read_all = every
        .parse_text_lines(text_lines())
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("every line reads");
    assert_eq!(read_all.len(), EVERY_ROW);
    let mut refused: Vec<&str> = read_all
        .iter()
        .map(|message| message.header().msgtype())
        .filter(|msgtype| !codec.reads_msgtype(msgtype))
        .collect();
    refused.sort_unstable();
    assert_eq!(refused.len(), EVERY_ROW - ROWS);
    assert_eq!(refused[..4], ["", "", "", ""], "the rows stating no type");
    assert_eq!(refused[10..], ["0", "0", "0", "0", "1"]);
}

#[test]
fn the_line_door_and_the_batch_door_land_the_same_rows() {
    let codec = codec();
    let held = super::format_target(codec.registry());
    let expected = line_messages(&codec)
        .iter()
        .map(|message| message.into_row(&held).expect("the fixed row"))
        .collect::<Vec<_>>();

    let batches = batches(&codec);
    let (schema, rows) = rows_of(&batches);
    assert_eq!(rows.len(), expected.len());
    // The batch door keeps the capture's own columns in front of the fixed
    // ones, so the two rows are lined up by the tag each column carries
    // rather than by position.
    for tag in yggdryl::fix_schema_tags() {
        // Tag 385 is the one column the two doors are allowed to disagree
        // on: an unmarked line takes the codec's own direction on the batch
        // door and states nothing on the line door.
        // Two columns the two doors are allowed to disagree on, both
        // capture facts rather than the message's: an unmarked line takes
        // the codec's own direction on the batch door and states none on
        // the line door, and only a read through a handle knows what object
        // the line came off.
        if tag == yggdryl::MSGDIRECTION_TAG_NAME.0 || tag == yggdryl::SOURCEURL_TAG_NAME.0 {
            continue;
        }
        let Some(mine) = yggdryl::fix_column_of(&held, tag) else {
            continue;
        };
        let theirs = yggdryl::fix_column_of(&schema, tag).expect("the batch keeps every column");
        for (at, row) in rows.iter().enumerate() {
            let want = expected[at].as_sequence().expect("a row")[mine].clone();
            assert_eq!(row[theirs], want, "row {at}, tag {tag}");
        }
    }
}

#[test]
fn a_batch_bound_changes_the_batches_and_never_the_rows() {
    let codec = codec();
    let one_batch = batches(&codec);
    let (_, expected) = rows_of(&one_batch);
    assert_eq!(one_batch.len(), 1, "the whole capture fits one batch");
    assert_eq!(expected.len(), ROWS);

    // Either bound closes a batch, whichever it reaches first, and neither
    // changes what the rows hold.
    for split in [
        codec.clone().with_batch_byte_size(4 * 1024),
        codec.clone().with_batch_row_size(8),
    ] {
        let batches = batches(&split);
        assert!(batches.len() > 1, "{} batches", batches.len());
        let (_, rows) = rows_of(&batches);
        assert_eq!(rows, expected);
        for batch in &batches {
            assert_eq!(batch.schema(), batches[0].schema());
        }
    }
}

#[test]
fn a_source_error_moves_through_the_line_stream_which_then_fuses() {
    const FAILED_AFTER: usize = 5;
    let codec = codec();
    let lines = text_lines();
    let before = codec.parse_text_lines(&lines[..FAILED_AFTER]).count();
    let marker = Arc::new(());
    let mut items = lines
        .into_iter()
        .map(Ok)
        .collect::<Vec<yggdryl::Result<TextLine>>>();
    items.insert(FAILED_AFTER, Err(super::batch::source_failure(&marker)));
    let mut items = items.into_iter();
    let pulls = std::rc::Rc::new(std::cell::Cell::new(0_usize));
    let counted = std::rc::Rc::clone(&pulls);
    let source = std::iter::from_fn(move || {
        counted.set(counted.get() + 1);
        items.next()
    });
    let mut stream = codec.parse_text_lines(source);
    // Nothing is pulled before the first message is asked for.
    assert_eq!(pulls.get(), 0);
    for _ in 0..before {
        stream.next().unwrap().unwrap();
    }
    super::batch::same_source_failure(stream.next().unwrap().unwrap_err(), &marker);
    assert_eq!(pulls.get(), FAILED_AFTER + 1);
    // The source's own error moves through and never advances the walk: the
    // lines behind it are read as if it had not been there.
    let after = stream
        .by_ref()
        .try_fold(0_usize, |read, message| message.map(|_| read + 1))
        .unwrap();
    assert_eq!(before + after, ROWS);
    // Exhaustion is what fuses it, and a fused stream pulls nothing more.
    let exhausted = pulls.get();
    assert_eq!(exhausted, LINES + 2);
    assert!(stream.next().is_none());
    assert_eq!(pulls.get(), exhausted);
}

#[test]
fn a_bridge_frame_carrying_a_row_is_the_type_that_row_states() {
    let codec = codec();
    // `35=UL` is the envelope the bridge sent and `MSGTYPE=` inside its
    // `XmlData` is what the row it carried calls itself. The row's type is
    // the message's, which is also the type the codec filters on.
    let message = line_messages(&codec)
        .into_iter()
        .find(|message| {
            message
                .get_by_tag(213)
                .and_then(|held| held.as_bytes().map(<[u8]>::to_vec))
                .is_some_and(|held| {
                    String::from_utf8_lossy(&held).contains("MSGTYPE=tradecapturereport")
                })
        })
        .expect("the trade capture frame");
    assert_eq!(message.by_tag(35).unwrap().as_str(), Some("AE"));
    // The envelope's own version still says what the session speaks.
    assert_eq!(message.by_tag(8).unwrap().as_str(), Some("FIX.4.2"));

    // The bridge packs an occurrence's members behind the two glyphs a log
    // viewer prints for its control bytes, and a group packed inside an
    // occurrence nests inside it rather than beside it, at every depth:
    // the leg carries its own allocations, the side its parties, and a
    // party its sub-identifiers.
    assert_eq!(message.by_tag(555).unwrap().as_i64(), Some(1), "NoLegs");
    assert_eq!(message.by_tag(552).unwrap().as_i64(), Some(1), "NoSides");
    assert_eq!(
        message
            .by_path(&path("TrdInstrmtLegGrp[0].LegPreAllocGrp[0].LegAllocQty"))
            .unwrap(),
        super::decimal("600")
    );
    assert_eq!(
        message
            .by_path(&path("TrdCapRptSideGrp[0].Parties"))
            .unwrap()
            .as_sequence()
            .map(<[Scalar]>::len),
        Some(7)
    );
    // One of the seven packs two sub-identifiers of its own, and they are
    // the party's rather than the side's.
    let subs: Vec<usize> = (0..7)
        .map(|index| {
            message
                .get_by_path(&path(&format!(
                    "TrdCapRptSideGrp[0].Parties[{index}].PtysSubGrp"
                )))
                .and_then(|held| held.as_sequence().map(<[Scalar]>::len))
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(
        subs,
        [1, 2, 0, 1, 0, 1, 0],
        "the sub-identifiers each party packs"
    );
    // A key the venue spelled under a namespace of its own is the bridge's
    // metadata rather than a child of the row, dot and all.
    assert_eq!(
        message
            .metadata()
            .get("metal.loco")
            .map(smol_str::SmolStr::as_str),
        Some("LN")
    );
}

#[test]
fn a_parse_fills_the_crate_columns_the_line_only_implied() {
    let codec = codec();
    // The first fill the bridge received: 21 shares at 83.08.
    let fill = line_messages(&codec)
        .into_iter()
        .find(|message| {
            message.header().msgtype() == "8"
                && message.get_by_tag(32) == Some(super::decimal("21"))
        })
        .expect("the fill of 21 shares");

    // A parse enriches, so the derived facts are on the message the line
    // door answered and no second pass adds them: the amount the fill comes
    // to, the currency it settles in, the instrument's ISIN under its stated
    // source and the country that ISIN opens with, the product the
    // dictionary files the security type under, the market the line names
    // first, and the ranked state it reports.
    // Exact, because a quantity times a price is an exact number and no
    // longer a float that has to be compared within a tolerance.
    assert_eq!(fill.by_tag(381).unwrap(), super::decimal("1744.68"));
    assert_eq!(fill.by_tag(120).unwrap().as_str(), Some("CHF"));
    assert_eq!(
        fill.by_tag(44).unwrap().as_decimal(),
        Some((yggdryl::i256::from_i128(83_080_000_000_000_000_000), 18)),
        "the price the line stated, exact"
    );
    assert_eq!(
        fill.get_px().to_string(),
        "83.08",
        "and the price the message is about, read off it"
    );
    assert_eq!(
        fill.get_isincode().map(|held| held.as_str()),
        Some("CH0012221716")
    );
    assert_eq!(fill.by_tag(470).unwrap().as_str(), Some("CH"));
    assert_eq!(fill.by_tag(460).unwrap().as_i128(), Some(5), "Product");
    assert_eq!(fill.get_miccode().map(|held| held.as_str()), Some("XSWX"));
    assert_eq!(
        Some(fill.get_state().clone()),
        yggdryl::State::from_spelling("1"),
    );
    // A stated value is never a derived one: the line said 260 remain.
    assert_eq!(fill.by_tag(151).unwrap(), super::decimal("260"));

    // An identifier the check digit does not close is no identifier: the
    // anonymized line names one, and nothing is read off it.
    let masked = line_messages(&codec)
        .into_iter()
        .find(|message| {
            message
                .get_by_tag(48)
                .and_then(|held| held.as_str().map(ToOwned::to_owned))
                == Some("XX0000000001".to_owned())
        })
        .expect("the anonymized line");
    assert_eq!(masked.get_isincode(), None, "no ISIN off a masked one");
    assert!(
        masked.get_by_tag(470).is_none_or(|held| held.is_null()),
        "and no country either"
    );
}

/// The rows whose parties nest a sub-group, which is the one shape
/// `FixMsg::from_row` names as inexact: a repeating group whose occurrences
/// nest a second group only some of them state comes back with the nested
/// occurrences behind the parties rather than inside the one that stated
/// them, so the wire moves `NoPartySubIDs(802)` and its members.
const PARTIES_NESTING_A_SUBGROUP: [usize; 4] = [4, 6, 37, 64];

/// The rows whose bridge spelled a coded value in its own words.
///
/// `TimeInForce(59)` is a coded field, and this capture's bridge writes
/// `TIMEINFORCE=day` where FIX's code set says `0`. The column holds the
/// code, the arrival record holds the word, and only the line door still
/// has the word to re-emit - which is what a row round trip costs for a
/// venue's own spelling of a code.
const A_VENUES_OWN_WORD_FOR_A_CODE: [usize; 3] = [76, 77, 78];

#[test]
fn the_writer_re_emits_each_rows_own_wire_and_none_of_the_captures_columns() {
    let codec = codec();
    let mut written: Vec<u8> = Vec::new();
    let filled = codec
        .parse_text_arrow_reader(source().read_arrow_reader(&reading()).expect("a reader"))
        .expect("the batch reader opens");
    let rows = codec
        .clone()
        .with_separator(b'|')
        .write_arrow_reader(filled, &mut written)
        .expect("the capture writes");
    assert_eq!(rows, ROWS as u64);

    // A row in is a line out: every row's wire, the codec's separator, a
    // newline - and the line door emits the same bytes for the same line.
    let wires: Vec<String> = line_messages(&codec)
        .iter()
        .map(|message| String::from_utf8_lossy(&message.into_bytes(b'|')).into_owned())
        .collect();
    assert_eq!(wires.len(), ROWS);
    let written: Vec<&str> = std::str::from_utf8(&written)
        .expect("the wire is text here")
        .lines()
        .collect();
    assert_eq!(written.len(), ROWS);
    for (at, wire) in wires.iter().enumerate() {
        // The capture's columns are the capture's: the body the line was
        // read from, its place in the object and the bridge's row header
        // are not content, so none of them reaches a counterparty.
        for carried in ["|body=", "|rownum=", "|mimetype=", "|url=", "|msgthreadid="] {
            assert!(!written[at].contains(carried), "row {at}: {}", written[at]);
        }
        if PARTIES_NESTING_A_SUBGROUP.contains(&at) {
            // Named, not skipped: what moves is the nested occurrence, and
            // the two wires still hold the same bytes elsewhere.
            assert!(wire.contains("|802="), "row {at} nests a sub-group");
            assert_ne!(written[at], wire, "row {at}");
            continue;
        }
        if A_VENUES_OWN_WORD_FOR_A_CODE.contains(&at) {
            // The venue's own word for a coded value is what the arrival
            // record keeps, and a row round trip types it: the line door
            // re-emits `59=day` because that is what the bridge wrote, and
            // the batch door re-emits `59=0` because the column it came
            // back through holds the code. Everything else is byte for byte.
            assert_eq!(
                written[at].replace("|59=0|", "|59=day|"),
                *wire,
                "row {at} differs in more than the coded word"
            );
            continue;
        }
        assert_eq!(written[at], wire, "row {at} re-emits its own wire");
    }

    // Every frame the bridge wrote with `|` comes back byte for byte, the
    // XmlData rows included, because the entries are the frame and nothing
    // read inside one of its values was recorded as an arrival.
    let framed = wires
        .iter()
        .enumerate()
        .filter(|(at, wire)| !PARTIES_NESTING_A_SUBGROUP.contains(at) && wire.contains("|10="))
        .count();
    assert!(framed >= 11, "{framed} frames checked");
}
