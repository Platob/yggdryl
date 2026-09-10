//! A bridge's own log as one dataset, read three ways and required to agree.
//!
//! `ulbridge.log` is a second of a ULBridge's own capture, anonymized, and
//! then every shape a bridge writes that the second happened not to hold: a
//! Jolokia exchange whose answer is a configuration document, a wildcard
//! document and an error, FIXML behind a verb, frames spelled with `^A` and
//! `<SOH>`, a FIXT logon, a `35=UL` frame with exact lengths and real
//! control-byte separators, one more carrying a trade capture whose payload
//! packs a group inside a group inside a group and separates their members
//! with the glyphs a viewer prints for those bytes, one carrying a FIXML
//! document in the same field instead, a bridge row with null spellings, a
//! marked frame, a statistics line, an empty body and a warning. The text reader frames every line under the bridge's own row
//! header; each row is then read on its own as a record and as a batch of
//! one shape, with enrichment on, and the two readings are required to agree
//! line for line.

use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::RecordBatch;
use yggdryl::holder::Buffer;
use yggdryl::holder::local::Folder;
use yggdryl::media::RecordOptions;
use yggdryl::media::text::TextOptions;
use yggdryl::types::State;
use yggdryl::{
    DataType, FixBranch, FixCodec, FixDedup, FixEntry, FixMsg, FixRegistry, IOMedia, MimeType,
    Scalar, Timezone, Url,
};

/// The capture, exactly as the bridge wrote it.
const LOG: &[u8] = include_bytes!("ulbridge.log");

/// How many lines the capture holds.
const LINES: usize = 113;

/// The one line that reads as two rows: a wildcard Jolokia read, which
/// answers for two MBeans and so yields one message per MBean.
const WILDCARD: usize = 98;

/// How many rows the capture reads as: a row a line, and the wildcard line's
/// second MBean once more.
const ROWS: usize = LINES + 1;

/// The first row text line `line` was read into.
const fn row_of(line: usize) -> usize {
    if line > WILDCARD { line + 1 } else { line }
}

/// The text line row `row` was read from.
const fn line_of(row: usize) -> usize {
    if row > WILDCARD { row - 1 } else { row }
}

/// The committed dictionary beside the bridge's own vocabulary.
fn registry() -> Arc<FixRegistry> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    let folder = Folder::new(root).expect("the seed folder is a local path");
    let held = FixRegistry::from_handle(&folder).expect("the committed dictionary loads");
    Arc::new(
        held.with_ulbridge_fields()
            .expect("the bridge's own fields"),
    )
}

/// The log as the `.log` handle a reader opens.
fn source() -> Buffer {
    Buffer::from_bytes(LOG.to_vec()).with_media_type(
        Url::from_str("file:///ulbridge.log")
            .expect("a URL")
            .media_type(),
    )
}

/// The text options a bridge log is read under: its own row header, each
/// line numbered, classified and read for its direction.
fn reading() -> RecordOptions {
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the bridge's row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_direction = true;
    options.parse_mimetype = true;
    options.into()
}

/// The codec every read uses: the bridge's own dialect, pinned for the whole
/// run, and batches closing at `bytes` of raw capture where one is stated.
fn codec_batching(bytes: Option<u64>) -> FixCodec {
    let codec = FixCodec::new(registry())
        .with_branch(&FixBranch::from_str(yggdryl::ULBRIDGE_BRANCH).unwrap());
    match bytes {
        Some(bytes) => codec.with_batch_byte_size(bytes),
        None => codec,
    }
}

/// The codec the row-by-row read uses, over the same dictionary and dialect.
fn codec() -> FixCodec {
    codec_batching(None)
}

/// One record read and filled: what the batch read does to every row, done
/// to one, so the two readings are compared with enrichment on both.
trait Enriched {
    fn enriched_record(
        &self,
        record: &Scalar,
    ) -> yggdryl::Result<impl Iterator<Item = yggdryl::Result<FixMsg>>>;
}

impl Enriched for FixCodec {
    fn enriched_record(
        &self,
        record: &Scalar,
    ) -> yggdryl::Result<impl Iterator<Item = yggdryl::Result<FixMsg>>> {
        Ok(self
            .parse_text_record(record)?
            .map(|held| held.and_then(|held| self.enrich_message(held))))
    }
}

/// Every batch the FIX read yields, filled, closing at `bytes` of raw capture.
fn batches(bytes: Option<u64>) -> Vec<RecordBatch> {
    let codec = codec_batching(bytes);
    let parsed = codec
        .parse_text_arrow_reader(source().read_arrow_reader(&reading()).expect("a reader"))
        .expect("the batch reader opens");
    codec
        .enrich_messages_arrow_reader(parsed)
        .expect("the filling reader opens")
        .map(|batch| batch.expect("a batch"))
        .collect()
}

/// One read's rows, as the values each holds, with the column names.
fn rows_of(batches: &[RecordBatch]) -> (Vec<String>, Vec<Vec<Scalar>>) {
    let names: Vec<String> = batches[0]
        .schema()
        .fields()
        .iter()
        .map(|held| held.name().to_owned())
        .collect();
    let mut rows = Vec::new();
    for batch in batches {
        let held = yggdryl::arrow::batch_to_value(batch).expect("the batch reads");
        for row in held.as_sequence().expect("rows") {
            rows.push(row.as_sequence().expect("a row").to_vec());
        }
    }
    (names, rows)
}

/// The text reader's own rows: the capture framed, classified and numbered,
/// before any message is built.
fn text_rows() -> (Vec<String>, Vec<Vec<Scalar>>) {
    let batches: Vec<RecordBatch> = source()
        .read_arrow_reader(&reading())
        .expect("a reader")
        .map(|batch| batch.expect("a batch"))
        .collect();
    rows_of(&batches)
}

/// One text row as the record the codec reads it from.
fn record(names: &[String], row: &[Scalar]) -> Scalar {
    Scalar::from_record(names.iter().cloned().zip(row.iter().cloned())).expect("a record")
}

/// Where one column sits, by name.
fn at(names: &[String], name: &str) -> usize {
    names
        .iter()
        .position(|held| held == name)
        .unwrap_or_else(|| panic!("a {name} column in {names:?}"))
}

/// One row's body as text.
fn body(names: &[String], row: &[Scalar]) -> String {
    let held = &row[at(names, "body")];
    held.as_bytes()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .or_else(|| held.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

/// A value rendered for comparison across the two reads.
fn rendered(value: &Scalar) -> Option<String> {
    match value {
        Scalar::Null => None,
        held => Some(
            held.as_str()
                .map_or_else(|| format!("{held:?}"), ToString::to_string),
        ),
    }
}

#[test]
fn every_line_is_a_row_whatever_the_batch_size_and_the_batches_share_one_schema() {
    let whole = batches(None);
    assert_eq!(whole.len(), 1, "one batch under the byte target");
    assert_eq!(
        whole[0].num_rows(),
        ROWS,
        "a row a line, and the wildcard line read once per MBean"
    );

    // A target of one byte closes a batch after every row of raw capture.
    let small = batches(Some(1));
    assert_eq!(small.len(), ROWS, "a row a batch");
    assert_eq!(small.iter().map(RecordBatch::num_rows).sum::<usize>(), ROWS);
    for batch in &small {
        assert_eq!(
            batch.schema(),
            whole[0].schema(),
            "one schema for every batch"
        );
    }
    // The rows are the same rows, whichever way the capture was cut - past
    // the URL, which names the buffer each read opened.
    let (names, whole_rows) = rows_of(&whole);
    let (_, small_rows) = rows_of(&small);
    let url = at(&names, "url");
    for (row, (left, right)) in whole_rows.iter().zip(&small_rows).enumerate() {
        assert_eq!(left[..url], right[..url], "row {row}");
        assert_eq!(left[url + 1..], right[url + 1..], "row {row}");
    }

    // The text reader framed every line: the line number is the capture's,
    // and the schema is the capture's own columns then the fixed row.
    let (names, rows) = text_rows();
    assert_eq!(rows.len(), LINES);
    assert_eq!(
        rows.last().unwrap()[at(&names, "rownum")].as_i64(),
        Some(LINES as i64)
    );
}

#[test]
fn the_row_by_row_read_agrees_with_the_batch_read_on_every_tag() {
    let codec = codec();
    let (text_names, text) = text_rows();
    let batch = batches(None);
    let (names, rows) = rows_of(&batch);
    let schema = yggdryl::Field::from_arrow_schema("row", &batch[0].schema()).expect("the schema");
    // Every fixed column the dictionary explains, header, body, groups and
    // the crate's own alike.
    let fixed: Vec<(usize, i32)> = names
        .iter()
        .enumerate()
        .filter_map(|(index, _)| {
            let tag = schema.fields()[index].as_fix().tag().ok().flatten()?;
            Some((index, tag))
        })
        .collect();
    assert!(fixed.len() > 80, "{} fixed columns", fixed.len());
    let direction =
        yggdryl::fix_column_of(&schema, yggdryl::MSGDIRECTION_TAG).expect("the direction");
    let mut next = 0;
    for (line, held) in text.iter().enumerate() {
        // The record is the text reader's row, whole: the body is what the
        // codec parses and every other column is what the row states. A
        // wildcard read answers for two MBeans, so one line is two messages
        // and fills the two rows the batch read gave it.
        let messages = codec
            .enriched_record(&record(&text_names, held))
            .and_then(|messages| messages.collect::<yggdryl::Result<Vec<_>>>())
            .unwrap_or_else(|error| panic!("line {line}: {error}"));
        assert_eq!(next, row_of(line), "line {line} opens at its own row");
        for message in messages {
            let row = next;
            next += 1;
            for &(index, tag) in &fixed {
                if index == direction {
                    // The batch reads the direction column; a record does not.
                    continue;
                }
                let alone = message.get_by_tag(tag).cloned().unwrap_or(Scalar::Null);
                let alone = if alone.is_null() {
                    message
                        .into_row(&schema)
                        .expect("a row")
                        .as_sequence()
                        .expect("cells")[index]
                        .clone()
                } else {
                    alone
                };
                assert_eq!(
                    rendered(&alone).is_some(),
                    rendered(&rows[row][index]).is_some(),
                    "tag {tag} on row {row}: record read {alone:?}, batch read {:?}",
                    rows[row][index]
                );
            }
        }
    }
    assert_eq!(next, rows.len(), "every row was read on its own");
}

#[test]
fn enrichment_fills_what_the_line_implied_and_only_that() {
    let (text_names, text) = text_rows();
    let batch = batches(None);
    let (_, rows) = rows_of(&batch);
    let schema = yggdryl::Field::from_arrow_schema("row", &batch[0].schema()).expect("the schema");
    let column =
        |tag: i32| yggdryl::fix_column_of(&schema, tag).unwrap_or_else(|| panic!("tag {tag}"));

    // The first fill the bridge received: 21 shares at 83.08.
    let fill = text
        .iter()
        .position(|held| {
            let body = body(&text_names, held);
            body.contains("|35=8|") && body.contains("|32=21|")
        })
        .expect("the fill");
    // `GrossTradeAmt` is no fixed column, so the message answers for it;
    // `SettlCurrency` is one, so the row does.
    let enriched = codec()
        .enriched_record(&record(&text_names, &text[fill]))
        .and_then(|mut messages| messages.next().expect("a message"))
        .expect("the fill reads");
    let gross = enriched
        .by_tag(381)
        .unwrap()
        .as_f64()
        .expect("GrossTradeAmt derived");
    assert!((gross - 21.0 * 83.08).abs() < 1e-6, "{gross}");
    assert_eq!(
        rows[fill][column(120)].as_str(),
        Some("CHF"),
        "SettlCurrency from Currency"
    );
    // Stated values are never overwritten: the line said 260 shares remain.
    assert_eq!(rows[fill][column(151)].as_f64(), Some(260.0));

    // Read without enrichment, the same line states neither.
    let plain = codec()
        .parse_line(body(&text_names, &text[fill]).as_bytes())
        .and_then(|mut messages| messages.next().expect("a message"))
        .expect("the fill reads");
    assert!(plain.get_by_tag(381).is_none());
    assert!(plain.get_by_tag(120).is_none());

    // The instrument and the lifecycle the line named, read into the columns
    // a monitor filters on: the ISIN under its stated source, the product the
    // dictionary files its security type under, the market it names first,
    // the state it reports - and the country its ISIN opens with, which is
    // no fixed column, so the message answers for it.
    assert_eq!(
        enriched.by_tag(yggdryl::ISINCODE_TAG).unwrap().as_str(),
        Some("CH0012221716")
    );
    assert_eq!(enriched.by_tag(470).unwrap().as_str(), Some("CH"));
    assert_eq!(
        rows[fill][column(460)].as_i128(),
        Some(5),
        "Product from SecurityType"
    );
    assert_eq!(
        rows[fill][column(yggdryl::MICCODE_TAG)].as_str(),
        Some("XSWX")
    );
    assert_eq!(
        rows[fill][column(yggdryl::STATE_TAG)]
            .as_str()
            .and_then(State::from_spelling),
        State::from_spelling("1"),
        "the state the report stated, as the column spells it"
    );
    for tag in [
        yggdryl::ISINCODE_TAG,
        470,
        460,
        yggdryl::MICCODE_TAG,
        yggdryl::STATE_TAG,
    ] {
        assert!(
            plain.get_by_tag(tag).is_none(),
            "tag {tag} without enrichment"
        );
    }

    // An identifier the check digit does not close is no identifier: the
    // anonymized line names one, and nothing is read off it.
    let masked = text
        .iter()
        .position(|held| body(&text_names, held).contains("XX0000000001"))
        .expect("the anonymized line");
    let masked = codec()
        .enriched_record(&record(&text_names, &text[masked]))
        .and_then(|mut messages| messages.next().expect("a message"))
        .expect("the anonymized line reads");
    for tag in [yggdryl::ISINCODE_TAG, 470] {
        assert!(
            masked.get_by_tag(tag).is_none_or(Scalar::is_null),
            "tag {tag} off a masked ISIN"
        );
    }
    // And the row is dated by the row header, not by the wire's clock.
    let clock = &text[fill][at(&text_names, "timestamp")];
    assert_eq!(&rows[fill][column(yggdryl::TIMESTAMP_TAG)], clock);
}

#[test]
fn every_row_is_dated_versioned_and_named_by_its_bracket() {
    let (text_names, text) = text_rows();
    let batch = batches(None);
    let (_, rows) = rows_of(&batch);
    let schema = yggdryl::Field::from_arrow_schema("row", &batch[0].schema()).expect("the schema");
    let column =
        |tag: i32| yggdryl::fix_column_of(&schema, tag).unwrap_or_else(|| panic!("tag {tag}"));
    for (row, held) in rows.iter().enumerate() {
        // A row is its line's, and the wildcard line's two rows are both its
        // own, so a row is read against the line it came from.
        let clock = &text[line_of(row)][at(&text_names, "timestamp")];
        assert_eq!(
            &held[column(yggdryl::TIMESTAMP_TAG)],
            clock,
            "row {row} is dated by its header"
        );
        assert!(
            !held[column(yggdryl::UNIXPARTITION_TAG)].is_null(),
            "row {row} has a partition"
        );
        assert!(
            held[column(8)]
                .as_str()
                .is_some_and(|held| held.starts_with("FIX")),
            "row {row} says which FIX it was read as: {:?}",
            held[column(8)]
        );
        assert!(
            !held[column(yggdryl::MSGHASH_TAG)].is_null(),
            "row {row} digests"
        );
        // The bracket names the context, which fills `msgctxid`; its session
        // uid is the bridge's own and is carried in front, so `sendersessionid`
        // holds only what the message itself spelled - a bridge row's
        // `SESSIONID`, and nothing on any other line.
        let context = &text[line_of(row)][at(&text_names, "msgCtxId")];
        assert_eq!(
            &held[column(yggdryl::MSGCTXID_TAG)],
            context,
            "row {row} context"
        );
        // The session column reads the message's own statement where the line
        // makes one, and the instance its bridge bracketed in front of the
        // line where it does not - a fill never lands over a stated reading.
        let uid = &text[line_of(row)][at(&text_names, "senderSessionId")];
        let session = &held[column(yggdryl::SENDERSESSIONID_TAG)];
        let line = body(&text_names, &text[line_of(row)]);
        if line.contains("|SESSIONID=") {
            assert!(
                session.as_str().is_some(),
                "row {row} states its own session"
            );
        } else {
            assert_eq!(session, uid, "row {row} session from the bracket");
        }
        // The plugin that logged the line is the plugin session it moved
        // from or to, by the direction it moved - unless the row spelled the
        // session itself, which a bridge row does as `ULFROMSESSIONNAME`.
        let plugin = &text[line_of(row)][at(&text_names, "plugin")];
        let direction = held[column(yggdryl::MSGDIRECTION_TAG)]
            .as_str()
            .map(str::to_owned);
        let (sender, target) = (
            &held[column(yggdryl::SENDERSESSIONNAME_TAG)],
            &held[column(yggdryl::TARGETSESSIONNAME_TAG)],
        );
        match direction.as_deref() {
            Some("SENT") if !line.contains("|ULFROMSESSIONNAME=") => {
                assert_eq!(sender, plugin, "row {row} sent by its plugin");
                assert!(
                    target.is_null() || line.contains("|ULTOSESSIONNAME="),
                    "row {row} target"
                );
            }
            Some("RECV") if !line.contains("|ULTOSESSIONNAME=") => {
                assert_eq!(target, plugin, "row {row} received by its plugin");
                assert!(
                    sender.is_null() || line.contains("|ULFROMSESSIONNAME="),
                    "row {row} sender"
                );
            }
            _ => {}
        }
    }
    // A thread bracket with no session leaves the bracket's columns null -
    // the Jolokia lines - and the bridge's own session name, where a row
    // spells one, is the plugin session rather than the logging plugin.
    let jolokia = text
        .iter()
        .position(|held| body(&text_names, held).starts_with("URI: /jolokia"))
        .expect("the Jolokia read");
    let jolokia = row_of(jolokia);
    assert!(rows[jolokia][column(yggdryl::SENDERSESSIONID_TAG)].is_null());
    assert!(rows[jolokia][column(yggdryl::MSGCTXID_TAG)].is_null());
    let stating = |key: &str| {
        text.iter()
            .position(|held| body(&text_names, held).contains(&format!("|{key}")))
            .unwrap_or_else(|| panic!("a bridge row spelling {key}"))
    };
    let spelled = |row: usize, key: &str| {
        body(&text_names, &text[row])
            .split('|')
            .find_map(|pair| pair.strip_prefix(key))
            .map(str::to_owned)
            .expect(key)
    };
    let bridged = stating("ULFROMSESSIONNAME=");
    assert_eq!(
        rows[row_of(bridged)][column(yggdryl::SENDERSESSIONNAME_TAG)].as_str(),
        Some(spelled(bridged, "ULFROMSESSIONNAME=").as_str())
    );
    assert_eq!(
        rows[row_of(bridged)][column(yggdryl::TARGETSESSIONNAME_TAG)].as_str(),
        Some(spelled(bridged, "ULTOSESSIONNAME=").as_str())
    );
    let sessioned = stating("SESSIONID=");
    assert_eq!(
        rows[row_of(sessioned)][column(yggdryl::SENDERSESSIONID_TAG)].as_str(),
        Some(spelled(sessioned, "SESSIONID=").as_str())
    );
    // And the sequence number the bracket states fills a row that carries
    // no frame, while a frame keeps its own.
    let routed = text
        .iter()
        .position(|held| body(&text_names, held).starts_with("RouteMessage : ACTION=EXECUTION|"))
        .expect("a routed row");
    assert_eq!(
        &rows[row_of(routed)][column(34)],
        &text[routed][at(&text_names, "seqNum")]
    );
    let fill = text
        .iter()
        .position(|held| body(&text_names, held).contains("|35=8|34=40218|"))
        .expect("the fill");
    assert_eq!(rows[row_of(fill)][column(34)].as_i64(), Some(40218));
}

#[test]
fn a_frame_carrying_a_row_in_its_xmldata_fills_the_columns_the_frame_left_unsaid() {
    let codec = codec();
    let (text_names, text) = text_rows();
    let batch = batches(None);
    let (names, rows) = rows_of(&batch);
    let schema = yggdryl::Field::from_arrow_schema("row", &batch[0].schema()).expect("the schema");
    let column =
        |tag: i32| yggdryl::fix_column_of(&schema, tag).unwrap_or_else(|| panic!("tag {tag}"));
    let frames: Vec<usize> = text
        .iter()
        .enumerate()
        .filter(|(_, held)| body(&text_names, held).contains("|35=UL|"))
        .map(|(row, _)| row)
        .collect();
    assert_eq!(
        frames.len(),
        4,
        "two of the bridge's frames, the exact one and the trade capture"
    );
    for &line in &frames {
        let (row, held) = (line, &rows[row_of(line)]);
        // The frame's own statements stay the frame's.
        assert_eq!(held[column(35)].as_str(), Some("UL"), "row {row}");
        assert!(
            held[column(49)].as_str().is_some(),
            "row {row} names a sender"
        );
        // The row inside XmlData fills what the frame never stated.
        assert!(
            held[column(55)].as_str().is_some(),
            "row {row} symbol from the nested row"
        );
        assert!(
            held[column(11)].as_str().is_some(),
            "row {row} ClOrdID from the nested row"
        );
        assert!(
            held[column(31)].as_f64().is_some(),
            "row {row} LastPx typed from the nested row"
        );
        assert!(held[column(60)].is_null() || matches!(held[column(60)], Scalar::Temporal(_)));
        // XmlData itself is the bytes it is, whole. It opens with a bridge
        // key - marked with a `#` by the hop that marks them, bare by the one
        // that does not - and never with a tag or a document.
        let xml = held[column(213)].as_bytes().expect("XmlData bytes");
        assert!(
            xml.starts_with(b"#") || xml[0].is_ascii_uppercase(),
            "row {row}: {}",
            String::from_utf8_lossy(&xml[..40])
        );
        assert!(
            memchr::memmem::find(xml, b"ULTOSESSIONNAME=").is_some(),
            "row {row} carries the whole row"
        );
        // The arrival record is the frame's ten pairs and nothing the row
        // inside one of them said: the nested row fills, and records nothing.
        let entries = held[at(&names, "nofixentries")]
            .as_sequence()
            .expect("entries");
        assert_eq!(entries.len(), 10, "row {row}: {entries:?}");
        let message = codec
            .enriched_record(&record(&text_names, &text[line]))
            .and_then(|mut messages| messages.next().expect("a message"))
            .expect("the frame reads");
        let data = message
            .entries()
            .iter()
            .find(|entry| entry.tag() == 213)
            .expect("the XmlData entry");
        assert!(data.children().is_empty());
        assert_eq!(
            data.value().len(),
            xml.len(),
            "row {row} keeps the whole value"
        );
    }
    // The bridge counted the control bytes its log printed as glyphs, so its
    // stated length is short and the value runs to the trailer instead.
    let printed = row_of(frames[0]);
    let stated = rows[printed][column(212)].as_i64().expect("XmlDataLen");
    let actual = rows[printed][column(213)]
        .as_bytes()
        .expect("XmlData")
        .len() as i64;
    assert!(stated < actual, "{stated} stated, {actual} carried");
    assert_eq!(rows[printed][column(55)].as_str(), Some("HOLN"));
    // The exact frame states its length in bytes and is read to it.
    let exact = row_of(frames[2]);
    assert_eq!(
        rows[exact][column(212)].as_i64().map(|held| held as usize),
        rows[exact][column(213)].as_bytes().map(<[u8]>::len)
    );
    assert_eq!(rows[exact][column(55)].as_str(), Some("EXAMPLECO.S"));
    assert_eq!(rows[exact][column(1)].as_str(), Some("ACCT1"));
}

#[test]
fn a_group_packed_inside_an_occurrence_nests_under_it_and_a_republication_is_dropped() {
    let codec = codec();
    let (text_names, text) = text_rows();
    // The same enrichment result, printed twice by the bridge: once with the
    // viewer's bullets for the two control bytes and the party's
    // sub-identifiers flattened to the row, once with the bridge's own
    // control bytes and the sub-identifiers packed inside the party they
    // belong to.
    let rows: Vec<usize> = text
        .iter()
        .enumerate()
        .filter(|(_, held)| {
            let body = body(&text_names, held);
            body.contains(
                "After Enrichment -> ACTION=EXECUTION|AGGRESSORINDICATOR=Y|ALTEVENTTEXT=Order Fill",
            ) && body.contains("EXECID=00011377089XEEA0|")
                && body.contains("NOPARTYSUBIDS[0]")
        })
        .map(|(row, _)| row)
        .collect();
    assert_eq!(rows.len(), 2, "{rows:?}");
    let flat = codec
        .enriched_record(&record(&text_names, &text[rows[0]]))
        .and_then(|mut messages| messages.next().expect("a message"))
        .expect("the flattened row reads");
    let nested = codec
        .enriched_record(&record(&text_names, &text[rows[1]]))
        .and_then(|mut messages| messages.next().expect("a message"))
        .expect("the packed row reads");
    for message in [&flat, &nested] {
        // Six parties either way, the first the executing firm: the tag the
        // row spelled is the counter's own column, and the occurrences are
        // the group the dictionary files under `Parties`.
        assert_eq!(message.by_tag(453).unwrap(), &Scalar::from(6_i32));
        let parties = message
            .by_path("Parties")
            .unwrap()
            .as_sequence()
            .expect("parties");
        assert_eq!(parties.len(), 6);
        assert_eq!(
            message.by_path("Parties.0.PartyID").unwrap(),
            &Scalar::from("HIGH_TOUCH")
        );
        assert_eq!(
            message.by_path("Parties.1.PartyID").unwrap(),
            &Scalar::from("SWXCCP")
        );
    }
    // Flattened, the sub-identifier group is a group of the row.
    assert_eq!(
        flat.by_path("PtysSubGrp.0.PartySubID").unwrap(),
        &Scalar::from("trader1")
    );
    // Packed inside the party, it is the party's own: reached through it,
    // typed through its own field - `contactname` is the code set's
    // spelling of 9 - and absent from a party that packed none.
    assert_eq!(
        nested.by_path("Parties.0.PtysSubGrp.0.PartySubID").unwrap(),
        &Scalar::from("trader1")
    );
    assert_eq!(
        nested
            .by_path("Parties.0.PtysSubGrp.0.PartySubIDType")
            .unwrap()
            .as_i64(),
        Some(9)
    );
    assert!(
        nested
            .get_by_path("Parties.1.PtysSubGrp")
            .is_none_or(Scalar::is_null)
    );
    assert!(
        nested.get_by_tag(802).is_none() && nested.get_by_path("PtysSubGrp").is_none(),
        "no sub-identifier counter or group at the row level"
    );
    // The arrival record nests three deep: the party under its counter, the
    // sub-identifier under the sub-counter under the party.
    let parties = nested
        .entries()
        .iter()
        .find(|entry| entry.tag() == 453)
        .expect("the party counter");
    let sub = parties
        .children()
        .iter()
        .find(|entry| entry.tag() == 802)
        .expect("the sub-counter under the parties");
    assert!(
        sub.children().iter().any(|entry| entry.tag() == 523),
        "{sub:?}"
    );
    let tags: Vec<i32> = parties.children().iter().map(FixEntry::tag).collect();
    assert!(tags.contains(&448) && tags.contains(&452), "{tags:?}");

    // A line the bridge prints twice in a row digests once, so deduplication
    // drops the republication and keeps the count honest.
    let line = String::from_utf8(LOG.to_vec()).expect("text");
    let line = line
        .lines()
        .nth(rows[1])
        .expect("the packed line")
        .to_owned();
    let twice = Buffer::from_bytes(format!("{line}\n{line}\n").into_bytes()).with_media_type(
        Url::from_str("file:///twice.log")
            .expect("a URL")
            .media_type(),
    );
    let parsed = codec
        .parse_text_arrow_reader(twice.read_arrow_reader(&reading()).expect("a reader"))
        .expect("the batch reader opens");
    let schema = yggdryl::Field::from_arrow_schema("fix", &parsed.schema()).expect("the schema");
    let messages = codec
        .messages(parsed)
        .map(|message| message.expect("a message"));
    let kept: usize = codec
        .arrow_reader(schema, FixDedup::new(messages).map(Ok))
        .expect("the batch reader opens")
        .map(|batch| batch.expect("a batch").num_rows())
        .sum();
    assert_eq!(kept, 1);
}

#[test]
fn every_other_shape_the_bridge_writes_lands_where_it_belongs() {
    let codec = codec();
    let (text_names, text) = text_rows();
    let batch = batches(None);
    let (_, rows) = rows_of(&batch);
    let schema = yggdryl::Field::from_arrow_schema("row", &batch[0].schema()).expect("the schema");
    let column =
        |tag: i32| yggdryl::fix_column_of(&schema, tag).unwrap_or_else(|| panic!("tag {tag}"));
    let mimetype = |row: usize| {
        text[row][at(&text_names, "mimetype")]
            .as_str()
            .map(ToOwned::to_owned)
    };
    let find = |needle: &str| {
        text.iter()
            .position(|held| body(&text_names, held).contains(needle))
            .unwrap_or_else(|| panic!("a line holding {needle:?}"))
    };
    let read = |row: usize| {
        codec
            .enriched_record(&record(&text_names, &text[row]))
            .and_then(|mut messages| messages.next().expect("a message"))
            .expect("the row reads")
    };

    // A configuration document: the exchange on the bridge's tags.
    let document = find(
        r#"Response: {"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=OMS_X1_TradeCapture"#,
    );
    assert_eq!(
        mimetype(document).as_deref(),
        Some(MimeType::ULCONFIG.as_str())
    );
    let message = read(document);
    assert_eq!(
        message.by_tag(yggdryl::STATUS_TAG).unwrap(),
        &Scalar::from(200_i64)
    );
    assert_eq!(
        message.by_path("CurrentPort").unwrap(),
        &Scalar::from(9726_i64)
    );
    // A wildcard read answers for every plugin, one message each, and every
    // message carries its own MBean's attributes flat.
    let wildcard = find(r#""mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*""#);
    let messages = codec
        .enriched_record(&record(&text_names, &text[wildcard]))
        .expect("the wildcard reads")
        .collect::<yggdryl::Result<Vec<_>>>()
        .expect("the wildcard reads");
    assert_eq!(messages.len(), 2, "one message per plugin");
    for (name, port) in [
        ("ULMSG_BROKER_BDG_DMZ_CLI", 9801_i64),
        ("OMS_X1_OrderOut", 9702_i64),
    ] {
        let held = messages
            .iter()
            .find(|message| message.get_by_path("Name").and_then(Scalar::as_str) == Some(name))
            .unwrap_or_else(|| panic!("a message for {name}"));
        assert_eq!(held.by_path("CurrentPort").unwrap(), &Scalar::from(port));
    }
    // An error answer states its error.
    let error = find(r#""error_type":"javax.management.InstanceNotFoundException""#);
    assert!(read(error).get_by_tag(yggdryl::ERROR_TAG).is_some());

    // FIXML behind a verb reads by its attributes.
    let out = find("<FIXML xmlns=");
    assert_eq!(mimetype(out).as_deref(), Some(MimeType::FIXML.as_str()));
    assert_eq!(
        rows[row_of(out)][column(11)].as_str(),
        Some("00026877711XOEA0.1")
    );
    assert_eq!(rows[row_of(out)][column(32)].as_f64(), Some(21.0));
    let inbound = row_of(find("<FIXML><Order ClOrdID=\"OD9EOEDJ401\""));
    assert_eq!(rows[inbound][column(55)].as_str(), Some("HOLN"));
    assert!(
        rows[inbound][column(35)].is_null(),
        "an element is not a MsgType"
    );

    // Frames spelled with `^A` and `<SOH>` split on the byte they spell.
    let heartbeat = row_of(find("8=FIX.4.4^A9=61^A35=0"));
    assert_eq!(rows[heartbeat][column(35)].as_str(), Some("0"));
    assert_eq!(rows[heartbeat][column(34)].as_i64(), Some(3091));
    let test_request = find("8=FIX.4.4<SOH>9=70<SOH>35=1");
    assert_eq!(rows[row_of(test_request)][column(35)].as_str(), Some("1"));
    assert_eq!(
        read(test_request).by_tag(112).unwrap().as_str(),
        Some("PING")
    );

    // A FIXT session keeps the BeginString it stated.
    let logon = row_of(find("8=FIXT.1.1|"));
    assert_eq!(rows[logon][column(8)].as_str(), Some("FIXT.1.1"));
    assert_eq!(rows[logon][column(35)].as_str(), Some("A"));

    // A spelled absence is null; a member run with no separator splits at
    // the names the dictionary declares.
    let nulls = find("LASTPX=null|LASTSHARES=<null>");
    assert!(rows[row_of(nulls)][column(31)].is_null());
    assert!(rows[row_of(nulls)][column(32)].is_null());
    assert_eq!(rows[row_of(nulls)][column(55)].as_str(), Some("HOLN"));
    let message = read(nulls);
    assert_eq!(
        message.by_path("Parties.0.PartyID").unwrap(),
        &Scalar::from("TRADER2")
    );
    // A coded member is the code's value, typed: `11` is an order origination trader.
    assert_eq!(
        message.by_path("Parties.0.PartyRole").unwrap().as_i64(),
        Some(11)
    );

    // A marked frame states its direction in front of it.
    let marked = find("|11=OD9EOEDJ401|55=HOLN|54=1|38=50|");
    assert_eq!(rows[row_of(marked)][column(35)].as_str(), Some("D"));
    assert_eq!(
        text[marked][at(&text_names, "direction")].as_str(),
        Some(yggdryl::types::MsgDirection::SENT)
    );

    // A line with nothing after its header is still a row: dated, versioned,
    // and saying nothing else.
    let empty = text
        .iter()
        .position(|held| body(&text_names, held).is_empty())
        .expect("the empty line");
    assert!(rows[row_of(empty)][column(35)].is_null());
    assert!(!rows[row_of(empty)][column(8)].is_null());
    assert_eq!(
        &rows[row_of(empty)][column(yggdryl::TIMESTAMP_TAG)],
        &text[empty][at(&text_names, "timestamp")]
    );
    // And a sentence is a sentence.
    let warning = find("Unable to resolve destination for OD9EOEDJ401");
    assert_eq!(
        mimetype(warning).as_deref(),
        Some(MimeType::OCTET_STREAM.as_str())
    );
    assert!(rows[row_of(warning)][column(35)].is_null());
}

#[test]
fn the_wire_re_emits_from_the_arrival_record_frames_included() {
    let (text_names, text) = text_rows();
    let mut written: Vec<u8> = Vec::new();
    let codec = codec();
    let parsed = codec
        .parse_text_arrow_reader(source().read_arrow_reader(&reading()).expect("a reader"))
        .expect("the batch reader opens");
    let filled = codec
        .enrich_messages_arrow_reader(parsed)
        .expect("the filling reader opens");
    let rows = codec
        .with_separator(b'|')
        .write_arrow_reader(filled, &mut written)
        .expect("the capture writes");
    assert_eq!(rows, ROWS as u64);
    let lines: Vec<&str> = std::str::from_utf8(&written)
        .expect("text")
        .lines()
        .collect();
    assert_eq!(lines.len(), ROWS);
    // Every frame the bridge wrote with `|` comes back byte for byte, the
    // XmlData rows included, because the entries are the frame and nothing
    // read inside one of its values was recorded as an arrival.
    let mut checked = 0;
    for (line, held) in text.iter().enumerate() {
        let body = body(&text_names, held);
        let Some(start) = body.find("8=FIX") else {
            continue;
        };
        if !body.contains("|10=") {
            continue;
        }
        let frame = body[start..].trim_end_matches(" << queued");
        let row = row_of(line);
        assert_eq!(lines[row], frame, "row {row} re-emits its frame");
        checked += 1;
    }
    assert!(checked >= 9, "{checked} frames checked");
}

/// The members one group occurrence declares, whatever the reader named the
/// occurrence itself.
fn packed_members(group: &yggdryl::Field) -> Vec<&str> {
    let DataType::List(item) = group.dtype() else {
        panic!("a list, got {}", group.dtype());
    };
    item.fields().iter().map(yggdryl::Field::name).collect()
}

fn packed_group<'held>(group: &'held yggdryl::Field, name: &str) -> &'held yggdryl::Field {
    let DataType::List(item) = group.dtype() else {
        panic!("a list, got {}", group.dtype());
    };
    item.fields()
        .iter()
        .find(|held| held.name() == name)
        .unwrap_or_else(|| panic!("{name} inside {}", group.name()))
}

#[test]
fn a_document_in_a_data_field_fills_the_frame_that_carried_it() {
    let codec = codec();
    let (text_names, text) = text_rows();
    let line = text
        .iter()
        .position(|held| body(&text_names, held).contains("213=<FIXML"))
        .expect("the frame carrying a document");
    let message = codec
        .enriched_record(&record(&text_names, &text[line]))
        .and_then(|mut messages| messages.next().expect("a message"))
        .expect("the frame reads");

    // The frame's own statements stay the frame's, and the document inside
    // `XmlData` fills what the frame never said - real tags, typed by the
    // dictionary, a nested element's attributes flattened like any other.
    assert_eq!(message.by_tag(35).unwrap().as_str(), Some("n"));
    assert_eq!(
        message.by_tag(17).unwrap().as_str(),
        Some("00011377096XEEA0")
    );
    assert_eq!(message.by_tag(55).unwrap().as_str(), Some("HOLN"));
    assert_eq!(message.by_tag(32).unwrap().as_f64(), Some(120.0));
    assert_eq!(message.by_tag(452).unwrap().as_i64(), Some(11));

    // The field still holds the bytes it arrived as: a reading of a value is
    // not a second arrival, so the wire re-emits the line exactly.
    let xml = message.by_tag(213).unwrap().as_bytes().expect("XmlData");
    assert!(
        xml.starts_with(b"<FIXML"),
        "{}",
        String::from_utf8_lossy(xml)
    );
    let entries = message.entries().len();
    assert_eq!(
        entries, 10,
        "the frame's own pairs and nothing the document said"
    );
}

#[test]
fn a_trade_capture_frame_nests_every_group_its_payload_packs() {
    let codec = codec();
    let (text_names, text) = text_rows();
    let line = text
        .iter()
        .position(|held| body(&text_names, held).contains("MSGTYPE=tradecapturereport"))
        .expect("the trade capture frame");
    let message = codec
        .enriched_record(&record(&text_names, &text[line]))
        .and_then(|mut messages| messages.next().expect("a message"))
        .expect("the frame reads");

    // The frame's own type is the message's: `35=UL` is what the bridge sent
    // and `MSGTYPE=` inside `XmlData` is what the row it carried calls itself.
    // The frame states it, so the frame keeps it.
    assert_eq!(message.by_tag(35).unwrap().as_str(), Some("UL"));

    // The occurrence packs its members behind the two glyphs a log viewer
    // prints for the bridge's control bytes, and each becomes its own field.
    let hedges = message
        .by_name("nohedgegroups")
        .expect("the hedge group")
        .as_sequence()
        .expect("its occurrences")
        .to_vec();
    assert_eq!(hedges.len(), 1);
    let held = hedges[0].as_sequence().expect("its members").to_vec();
    assert_eq!(held.len(), 6, "{held:?}");
    assert_eq!(held[0].as_str(), Some("XAU"), "HedgeCurrency");
    let root = message.as_field();
    let hedge = root
        .get_field_by_path("nohedgegroups")
        .expect("the hedge group field");
    assert_eq!(packed_members(hedge)[0], "hedgecurrency");

    // A group packed inside an occurrence nests inside it rather than beside
    // it, at every depth the payload packs one: the leg carries its own
    // allocations, and the side its parties, and a party its sub-identifiers.
    let legs = root.get_field_by_path("nolegs").expect("the leg group");
    let allocations = packed_group(legs, "legpreallocgrp");
    assert!(
        packed_members(allocations).contains(&"legallocaccount"),
        "{:?}",
        packed_members(allocations)
    );
    let sides = root.get_field_by_path("nosides").expect("the side group");
    let parties = packed_group(sides, "parties");
    let subs = packed_group(parties, "ptyssubgrp");
    assert!(
        packed_members(subs).contains(&"partysubid"),
        "{:?}",
        packed_members(subs)
    );

    // The bridge counted the control bytes it wrote, so the length it stated
    // is short of the bytes the log carries and the value runs to the trailer.
    let stated = message.by_tag(212).unwrap().as_i64().expect("XmlDataLen");
    let carried = message
        .by_tag(213)
        .unwrap()
        .as_bytes()
        .expect("XmlData")
        .len() as i64;
    assert!(stated < carried, "{stated} stated, {carried} carried");

    // The envelope's `BeginString` is what the session speaks and the row
    // inside it is written to a later FIX: `RegulatoryTradeIDGrp` is tag 1907,
    // which no 4.2 session ever named. The codec here pins no version, so the
    // row is dated by the dictionary's own newest rather than by the frame,
    // and the frame keeps saying what it said.
    assert_eq!(message.by_tag(8).unwrap().as_str(), Some("FIX.4.2"));
    assert!(
        registry().newest().expect("the seed's newest").version()
            > "4.2".parse().expect("a version")
    );
    assert!(
        packed_members(
            root.get_field_by_path("regulatorytradeidgrp")
                .expect("the regulatory trade id group")
        )
        .contains(&"tradeid")
    );

    // A key the dictionary has no field for is kept under its own spelling
    // rather than dropped, dot and all.
    assert_eq!(
        message
            .by_name("metal.loco")
            .expect("the venue's own key")
            .as_str(),
        Some("LN")
    );
}
