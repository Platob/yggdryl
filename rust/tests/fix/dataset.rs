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
//! marked frame, a statistics line, an empty body and a warning - and then
//! fifteen more lines of the bridge's own, a cancel/reject flow: a cancel
//! request and its reject, the reject routed, enriched with its regulatory
//! clocks and forwarded as a bridge row of ten parties, the `35=UL` frame
//! that row goes out in, and two heartbeats. The text reader frames every line under the bridge's own row
//! header; each row is then read on its own as a record and as a batch of
//! one shape, with enrichment on, and the two readings are required to agree
//! line for line.

use super::path;

use std::sync::Arc;

use arrow_array::RecordBatch;
use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::media::text::{TextLine, TextOptions, read_text_lines};
use yggdryl::types::State;
use yggdryl::{
    DataType, FixCodec, FixDedup, FixMsg, FixRegistry, IOMedia, MimeType, Scalar, Timezone, Url,
};

/// The capture, exactly as the bridge wrote it.
const LOG: &[u8] = include_bytes!("ulbridge.log");

/// How many lines the capture holds.
const LINES: usize = 144;

/// The lines that read as more than one row: the wildcard Jolokia reads,
/// each answering for several plugins and so yielding one message per
/// configuration the answer named.
/// Each is the line and how many rows it yields beyond its first.
const WILDCARDS: [(usize, usize); 2] = [(98, 1), (128, 2)];

/// The lines that read as no row at all.
///
/// Mostly the bridge's own prose, which states no frame, no bridge pair and
/// no document, so it carries no message (decision 16). Line 99 is the one
/// that is not prose: a Jolokia answer that came back with an error and no
/// `value`, which names no plugin - and a read that answers no plugin answers
/// no message, rather than an envelope with nothing inside it (decision 17).
/// It used to be a row carrying `Error` on tag 20004 and nothing else.
///
/// A line is still a line - the text reader answers every one of them - and
/// this is what the FIX reader answers for it.
const SILENT: [usize; 52] = [
    0, 2, 12, 16, 17, 18, 19, 22, 28, 29, 31, 40, 41, 42, 46, 48, 49, 50, 54, 61, 65, 66, 67, 68,
    71, 78, 79, 80, 84, 86, 87, 88, 92, 93, 94, 95, 96, 99, 108, 109, 110, 115, 116, 117, 118, 120,
    124, 126, 127, 136, 137, 139,
];

/// How many rows the capture reads as: a row for every message, so a row a
/// line but for the wildcards, which answer for each configuration they
/// named, and the silent lines, which answer for nothing.
const ROWS: usize = LINES - SILENT.len() + WILDCARDS[0].1 + WILDCARDS[1].1;

/// Whether text line `line` carries no message.
const fn silent(line: usize) -> bool {
    let mut at = 0;
    while at < SILENT.len() {
        if SILENT[at] == line {
            return true;
        }
        at += 1;
    }
    false
}

/// How many rows beyond its first text line `line` answers.
const fn beyond(line: usize) -> usize {
    let mut at = 0;
    while at < WILDCARDS.len() {
        if WILDCARDS[at].0 == line {
            return WILDCARDS[at].1;
        }
        at += 1;
    }
    0
}

/// The first row text line `line` was read into; where the line is silent,
/// the row the line after it opens.
const fn row_of(line: usize) -> usize {
    let mut row = 0;
    let mut at = 0;
    while at < line {
        if !silent(at) {
            row += 1 + beyond(at);
        }
        at += 1;
    }
    row
}

/// The text line row `row` was read from.
const fn line_of(row: usize) -> usize {
    let mut line = 0;
    let mut seen = 0;
    while line < LINES {
        if !silent(line) {
            let held = 1 + beyond(line);
            if row < seen + held {
                return line;
            }
            seen += held;
        }
        line += 1;
    }
    LINES
}

/// The committed dictionary beside the bridge's own vocabulary.
fn registry() -> Arc<FixRegistry> {
    super::plugin_fields_registry()
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
/// line numbered, classified and read for its direction.
fn reading() -> RecordOptions {
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the bridge's row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_mimetype = true;
    options.into()
}

/// The codec every read uses: the bridge's dictionary, whose fields resolve in
/// the one namespace, and batches closing at `bytes` of raw capture where one
/// is stated.
fn codec_batching(bytes: Option<u64>) -> FixCodec {
    let codec = super::fixed_codec(registry());
    match bytes {
        Some(bytes) => codec.with_batch_byte_size(bytes),
        None => codec,
    }
}

/// The codec the row-by-row read uses, over the same dictionary.
///
/// It is also told what the run's captures are called, because a line answers
/// them by position and only this boundary knows what each position means.
pub(super) fn codec() -> FixCodec {
    codec_batching(None).with_capture_names(header_captures())
}

/// What the bridge's row header captures, in the order it declares them.
fn header_captures() -> Vec<String> {
    let RecordOptions::Text(options) = reading() else {
        panic!("a text read")
    };
    options.capture_names().map(ToOwned::to_owned).collect()
}

/// One record read and filled: what the batch read does to every row, done
/// to one, so the two readings are compared with enrichment on both.
trait Enriched {
    fn enriched_line(
        &self,
        line: &TextLine,
    ) -> yggdryl::Result<impl Iterator<Item = yggdryl::Result<FixMsg>>>;
}

impl Enriched for FixCodec {
    fn enriched_line(
        &self,
        line: &TextLine,
    ) -> yggdryl::Result<impl Iterator<Item = yggdryl::Result<FixMsg>>> {
        Ok(self.enrich_messages(self.parse_text_line(line)?))
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

/// Every line the capture holds, as the text reader decodes them.
///
/// The one decode entry point, which is the door the codec takes: a line in
/// is a row out, so these line up with the batch's rows by position.
pub(super) fn text_lines() -> Vec<TextLine> {
    let RecordOptions::Text(options) = reading() else {
        panic!("a text read")
    };
    read_text_lines(source(), &options)
        .expect("a line reader")
        .map(|line| line.expect("a line"))
        .collect()
}

#[test]
fn the_full_capture_composes_through_both_arrow_boundaries() {
    let codec = codec();
    let RecordOptions::Text(options) = reading() else {
        panic!("a text read")
    };
    let schema = yggdryl::fix_schema(codec.registry(), "fix").unwrap();
    let expected = codec
        .enrich_messages(codec.parse_text_lines(text_lines()))
        .map(|message| message.and_then(|message| message.into_row(&schema)))
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(expected.len(), ROWS);

    let direct = codec
        .arrow_reader(
            schema.clone(),
            codec.enrich_messages(
                codec.parse_text_lines(read_text_lines(source(), &options).unwrap()),
            ),
        )
        .unwrap();
    let text_batches = yggdryl::media::text::into_arrow_reader(
        read_text_lines(source(), &options).unwrap(),
        &options,
    )
    .unwrap();
    let restored_lines = yggdryl::media::text::from_arrow_reader(text_batches, &options).unwrap();
    let restored = codec
        .arrow_reader(
            schema.clone(),
            codec.enrich_messages(codec.parse_text_lines(restored_lines)),
        )
        .unwrap();
    for reader in [direct, restored] {
        let actual = codec
            .messages(reader)
            .map(|message| message.and_then(|message| message.into_row(&schema)))
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(actual, expected);
    }

    // The reverse reader restores each physical row number through the
    // configured start, every body, and every capture's presence at its index.
    let original = text_lines();
    let restored = yggdryl::media::text::from_arrow_reader(
        yggdryl::media::text::into_arrow_reader(original.clone(), &options).unwrap(),
        &options,
    )
    .unwrap()
    .collect::<yggdryl::Result<Vec<_>>>()
    .unwrap();
    assert_eq!(restored.len(), LINES);
    for (line, back) in original.iter().zip(&restored) {
        assert_eq!(back.index(), line.index());
        assert_eq!(back.body(), line.body());
        assert_eq!(
            back.captures()
                .iter()
                .map(Option::is_some)
                .collect::<Vec<_>>(),
            line.captures()
                .iter()
                .map(Option::is_some)
                .collect::<Vec<_>>(),
            "line {}",
            line.index()
        );
    }
}

#[test]
fn a_source_error_inside_the_capture_moves_through_and_the_stream_fuses() {
    const FAILED_AFTER: usize = 5;
    let codec = codec();
    let lines = text_lines();
    let before = codec
        .enrich_messages(codec.parse_text_lines(&lines[..FAILED_AFTER]))
        .count();
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
    let mut stream = codec.enrich_messages(codec.parse_text_lines(source));
    // Nothing is pulled before the first message is asked for.
    assert_eq!(pulls.get(), 0);
    for _ in 0..before {
        stream.next().unwrap().unwrap();
    }
    super::batch::same_source_failure(stream.next().unwrap().unwrap_err(), &marker);
    assert_eq!(pulls.get(), FAILED_AFTER + 1);
    let after = stream
        .by_ref()
        .try_fold(0_usize, |read, message| message.map(|_| read + 1))
        .unwrap();
    assert_eq!(before + after, ROWS);
    let exhausted = pulls.get();
    assert_eq!(exhausted, LINES + 2);
    assert!(stream.next().is_none());
    assert_eq!(pulls.get(), exhausted);
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
        "a row a line, and the wildcard line read once per configuration"
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
    // The same immutable source has the same captured URL and content UUID,
    // whichever way its rows are batched.
    let (_, whole_rows) = rows_of(&whole);
    let (_, small_rows) = rows_of(&small);
    for (row, (left, right)) in whole_rows.iter().zip(&small_rows).enumerate() {
        assert_eq!(left, right, "row {row}");
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

/// The one reading a row cannot carry back, as the row and the tag it loses.
///
/// Row 71 is line 111: a trade capture report whose body arrived as a FIXML
/// document behind `XmlData(213)`. The reader lifts the document's fields
/// into the message, so the line door holds `ExecBroker(76)` and
/// `ClientID(109)` and restates both into the `parties` group and its
/// `NoPartyIDs(453)` counter. Neither 76 nor 109 has a column of its own -
/// `fix_schema` names 49 body tags and these are not among them - and
/// neither is an arrival entry either, because the record of a document-bodied
/// message is the ten pairs of its envelope and the document whole, not the
/// fields a reader found inside it. So the row carries them nowhere, and the
/// batch door, reading the row, has nothing to restate: two parties on the
/// line door, none on the batch door.
///
/// This is the one shape it happens to, and the reason it is only one: where
/// a lifted field is an arrival entry, the enriching pass takes it back off
/// the record before restating, which is what closes lines 122, 123 and 125.
/// A document's fields are the case the record cannot answer for, and closing
/// it means recording what a document stated as entries of its own - which
/// moves the arrival record every `.entry[i]` in the equivalence snapshot
/// pins, and so is its own decision rather than a corner of this one.
const LIFTED_OUT_OF_A_DOCUMENT: [(usize, i32); 2] = [(71, 453), (71, 209_321)];

#[test]
fn the_row_by_row_read_agrees_with_the_batch_read_on_every_tag() {
    let codec = codec();
    let lines = text_lines();
    let batch = batches(None);
    let (names, rows) = rows_of(&batch);
    let schema = yggdryl::Field::from_arrow_schema("row", &batch[0].schema()).expect("the schema");
    let projection = yggdryl::fix_schema(codec.registry(), "row").expect("FIX-only columns");
    // Every fixed column the dictionary explains, header, body, groups and
    // the crate's own alike.
    let fixed: Vec<(usize, i32, usize)> = names
        .iter()
        .enumerate()
        .filter_map(|(index, _)| {
            let tag = schema.fields()[index].as_fix().tag().ok().flatten()?;
            Some((index, tag, yggdryl::fix_column_of(&projection, tag)?))
        })
        .collect();
    assert!(fixed.len() > 80, "{} fixed columns", fixed.len());
    let direction =
        yggdryl::fix_column_of(&schema, yggdryl::MSGDIRECTION_TAG_NAME.0).expect("the direction");
    let source =
        yggdryl::fix_column_of(&schema, yggdryl::SOURCEURL_TAG_NAME.0).expect("the source url");
    let mut next = 0;
    let mut unread: Vec<(usize, i32)> = Vec::new();
    for (line, held) in lines.iter().enumerate() {
        // The line is the text reader's own, whole: the body is what the
        // codec parses and every capture is what the row states. A wildcard
        // read answers for two MBeans, so one line is two messages and fills
        // the two rows the batch read gave it.
        let messages = codec
            .enriched_line(held)
            .and_then(|messages| messages.collect::<yggdryl::Result<Vec<_>>>())
            .unwrap_or_else(|error| panic!("line {line}: {error}"));
        assert_eq!(next, row_of(line), "line {line} opens at its own row");
        for message in messages {
            let row = next;
            next += 1;
            let projected = message.into_row(&projection).expect("a FIX-only row");
            let projected = projected.as_sequence().expect("cells");
            for &(index, tag, projected_at) in &fixed {
                if index == direction {
                    // The batch door fills the codec's pin where a line states
                    // no direction; the line door leaves it unsaid (decision 14).
                    continue;
                }
                if index == source {
                    // Where a line was read from is the capture's answer, not
                    // the line's: the batch door has an object to name and a
                    // line handed over on its own does not.
                    continue;
                }
                let alone = message.get_by_tag(tag).cloned().unwrap_or(Scalar::Null);
                let alone = if alone.is_null() {
                    projected[projected_at].clone()
                } else {
                    alone
                };
                let agreed = rendered(&alone).is_some() == rendered(&rows[row][index]).is_some();
                if !agreed && LIFTED_OUT_OF_A_DOCUMENT.contains(&(row, tag)) {
                    unread.push((row, tag));
                    continue;
                }
                assert!(
                    agreed,
                    "tag {tag} on row {row}: record read {alone:?}, batch read {:?}",
                    rows[row][index]
                );
            }
        }
    }
    assert_eq!(next, rows.len(), "every row was read on its own");
    // The bound is a list, not a licence: every pair named has to be a pair
    // the two doors really did read differently, so the day a row carries
    // one of them the list stops being true and this says so.
    assert_eq!(
        unread,
        LIFTED_OUT_OF_A_DOCUMENT.to_vec(),
        "what a row cannot carry is exactly what is written down"
    );
}

#[test]
fn enrichment_fills_what_the_line_implied_and_only_that() {
    let (text_names, text) = text_rows();
    let lines = text_lines();
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
    let fill_row = row_of(fill);
    // `GrossTradeAmt` is no fixed column, so the message answers for it;
    // `SettlCurrency` is one, so the row does.
    let enriched = codec()
        .enriched_line(&lines[fill])
        .and_then(|mut messages| messages.next().expect("a message"))
        .expect("the fill reads");
    let gross = enriched
        .by_tag(381)
        .unwrap()
        .as_f64()
        .expect("GrossTradeAmt derived");
    assert!((gross - 21.0 * 83.08).abs() < 1e-6, "{gross}");
    assert_eq!(
        rows[fill_row][column(120)].as_str(),
        Some("CHF"),
        "SettlCurrency from Currency"
    );
    // Stated values are never overwritten: the line said 260 shares remain.
    assert_eq!(rows[fill_row][column(151)].as_f64(), Some(260.0));

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
        enriched
            .by_tag(yggdryl::ISINCODE_TAG_NAME.0)
            .unwrap()
            .as_str(),
        Some("CH0012221716")
    );
    assert_eq!(enriched.by_tag(470).unwrap().as_str(), Some("CH"));
    assert_eq!(
        rows[fill_row][column(460)].as_i128(),
        Some(5),
        "Product from SecurityType"
    );
    assert_eq!(
        rows[fill_row][column(yggdryl::MICCODE_TAG_NAME.0)].as_str(),
        Some("XSWX")
    );
    assert_eq!(
        rows[fill_row][column(yggdryl::STATE_TAG_NAME.0)]
            .as_str()
            .and_then(State::from_spelling),
        State::from_spelling("1"),
        "the state the report stated, as the column spells it"
    );
    for tag in [
        yggdryl::ISINCODE_TAG_NAME.0,
        470,
        460,
        yggdryl::MICCODE_TAG_NAME.0,
        yggdryl::STATE_TAG_NAME.0,
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
        .enriched_line(&lines[masked])
        .and_then(|mut messages| messages.next().expect("a message"))
        .expect("the anonymized line reads");
    for tag in [yggdryl::ISINCODE_TAG_NAME.0, 470] {
        assert!(
            masked.get_by_tag(tag).is_none_or(Scalar::is_null),
            "tag {tag} off a masked ISIN"
        );
    }
    // The wire's event clock settles the message; capture time remains context.
    let clock = &text[fill][at(&text_names, "timestamp")];
    assert_eq!(
        &rows[fill_row][column(yggdryl::UPDATEDAT_TAG_NAME.0)],
        plain.updatedat()
    );
    assert_ne!(plain.updatedat(), clock);
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
            &held[schema.index_of("timestamp").expect("capture clock")],
            clock,
            "row {row} carries its header clock"
        );
        let transact = &held[column(60)];
        let event = if transact.is_null() {
            &held[column(52)]
        } else {
            transact
        };
        assert_eq!(&held[column(yggdryl::UPDATEDAT_TAG_NAME.0)], event);
        assert_eq!(&held[column(yggdryl::SNAPSHOTAT_TAG_NAME.0)], event);
        assert_eq!(&held[column(yggdryl::CREATEDAT_TAG_NAME.0)], event);
        assert!(
            !held[column(yggdryl::TIMEPARTITION_TAG_NAME.0)].is_null(),
            "row {row} has a partition"
        );
        assert!(
            held[column(8)]
                .as_str()
                .is_some_and(|held| held.starts_with("FIX")),
            "row {row} says which FIX it was read as: {:?}",
            held[column(8)]
        );
        assert_eq!(
            super::identity_bytes(&held[column(yggdryl::UUID_TAG_NAME.0)]).len(),
            16,
            "row {row} has a sixteen-byte content identity"
        );
        // The bracket names the context, which fills `msgctxid`; its session
        // uid is the bridge's own and is carried in front, so `sendersessionid`
        // holds only what the message itself spelled - a bridge row's
        // `SESSIONID`, and nothing on any other line.
        let context = &text[line_of(row)][at(&text_names, "msgCtxId")];
        assert_eq!(
            &held[column(yggdryl::MSGCTXID_TAG_NAME.0)],
            context,
            "row {row} context"
        );
        // The session column reads the message's own statement where the line
        // makes one, and the instance its bridge bracketed in front of the
        // line where it does not - a fill never lands over a stated reading.
        let uid = &text[line_of(row)][at(&text_names, "senderSessionId")];
        let session = &held[column(yggdryl::SENDERSESSIONID_TAG_NAME.0)];
        let line = body(&text_names, &text[line_of(row)]);
        if line.contains("|SESSIONID=") {
            assert!(
                session.as_str().is_some(),
                "row {row} states its own session"
            );
        } else {
            assert_eq!(session, uid, "row {row} session from the bracket");
        }
        // The plugin that logged the line is the bracket's own capture, on
        // every row the line read into - and it is never anything else: the
        // session names a line moved between are what the line spells, as a
        // bridge row does with `ULFROMSESSIONNAME` and `ULTOSESSIONNAME`, and
        // nothing derives them from the plugin, nor the plugin the message
        // came through before.
        let plugin = &text[line_of(row)][at(&text_names, "pluginid")];
        assert_eq!(
            &held[column(yggdryl::PLUGINID_TAG_NAME.0)],
            plugin,
            "row {row} names the plugin that logged it"
        );
        assert!(
            held[column(yggdryl::PREVPLUGINID_TAG_NAME.0)].is_null(),
            "row {row} states no previous plugin"
        );
        if !line.contains("ULFROMSESSIONNAME=") {
            assert!(
                held[column(yggdryl::SENDERSESSIONNAME_TAG_NAME.0)].is_null(),
                "row {row} states no sender session name"
            );
        }
        if !line.contains("ULTOSESSIONNAME=") {
            assert!(
                held[column(yggdryl::TARGETSESSIONNAME_TAG_NAME.0)].is_null(),
                "row {row} states no target session name"
            );
        }
    }
    // A thread bracket with no session leaves the bracket's columns null -
    // the Jolokia lines - and the bridge's own session name, where a row
    // spells one, is the row's own statement.
    let jolokia = text
        .iter()
        .position(|held| body(&text_names, held).starts_with("URI: /jolokia"))
        .expect("the Jolokia read");
    let jolokia = row_of(jolokia);
    assert!(rows[jolokia][column(yggdryl::SENDERSESSIONID_TAG_NAME.0)].is_null());
    assert!(rows[jolokia][column(yggdryl::MSGCTXID_TAG_NAME.0)].is_null());
    let stating = |key: &str| {
        text.iter()
            .position(|held| body(&text_names, held).contains(&format!("|{key}")))
            .unwrap_or_else(|| panic!("a bridge row spelling {key}"))
    };
    // What the line spelled, minus the paren a log wrapped the row in: the
    // capture writes `ULTOSESSIONNAME=ULMSG_BROKER_BDG_DMZ_CLI)` inside a
    // parenthesised remark, and the session name is the name and not the
    // paren, so one column holds one spelling of one value however the line
    // that carried it was decorated. Only the `)` is given back, because only
    // the `)` is what this capture closed a value with - a value that ended
    // anywhere else would be a truncation and has to fail here.
    let spelled = |row: usize, key: &str| {
        body(&text_names, &text[row])
            .split('|')
            .find_map(|pair| pair.strip_prefix(key))
            .map(|value| value.trim_end_matches(')').to_owned())
            .expect(key)
    };
    let bridged = stating("ULFROMSESSIONNAME=");
    assert_eq!(
        rows[row_of(bridged)][column(yggdryl::SENDERSESSIONNAME_TAG_NAME.0)].as_str(),
        Some(spelled(bridged, "ULFROMSESSIONNAME=").as_str())
    );
    assert_eq!(
        rows[row_of(bridged)][column(yggdryl::TARGETSESSIONNAME_TAG_NAME.0)].as_str(),
        Some(spelled(bridged, "ULTOSESSIONNAME=").as_str())
    );
    let sessioned = stating("SESSIONID=");
    assert_eq!(
        rows[row_of(sessioned)][column(yggdryl::SENDERSESSIONID_TAG_NAME.0)].as_str(),
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
    let lines = text_lines();
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
        5,
        "two of the bridge's frames, the exact one, the trade capture and the cancel reject"
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
        // Typed by the dictionary where the row states one: the four
        // executions carry a `LASTPX`, the cancel reject carries none and
        // fills nothing there.
        assert_eq!(
            held[column(31)].as_f64().is_some(),
            body(&text_names, &text[line]).contains("LASTPX="),
            "row {row} LastPx typed from the nested row"
        );
        assert!(held[column(60)].is_null() || held[column(60)].is_temporal());
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
        let entries = held[at(&names, "fixentries")]
            .as_sequence()
            .expect("entries");
        assert_eq!(entries.len(), 10, "row {row}: {entries:?}");
        let message = codec
            .enriched_line(&lines[line])
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
    // The frame the bridge forwards the cancel reject in states its length
    // in bytes too, and is read to it; the row it carries names itself a
    // `cancelreject`, and the frame keeps saying `UL` - its own type, as the
    // trade capture frame's does - while the row fills every column the
    // frame left unsaid: the order's chain, the rejection, the reason.
    let reject = row_of(frames[4]);
    assert_eq!(
        rows[reject][column(212)].as_i64().map(|held| held as usize),
        rows[reject][column(213)].as_bytes().map(<[u8]>::len)
    );
    let message = codec
        .enriched_line(&lines[frames[4]])
        .and_then(|mut messages| messages.next().expect("a message"))
        .expect("the reject frame reads");
    assert_eq!(message.as_field().name(), "UL");
    assert_eq!(message.by_tag(35).unwrap().as_str(), Some("UL"));
    assert_eq!(rows[reject][column(11)].as_str(), Some("0102000452788802"));
    assert_eq!(rows[reject][column(41)].as_str(), Some("0102000452788801"));
    assert_eq!(rows[reject][column(55)].as_str(), Some("2454"));
    assert_eq!(
        rows[reject][column(39)]
            .as_str()
            .and_then(State::from_spelling),
        State::from_spelling("8"),
        "the row's `ORDSTATUS=rejected`, typed"
    );
    assert_eq!(
        rows[reject][column(yggdryl::STATE_TAG_NAME.0)],
        rows[reject][column(39)]
    );
    assert_eq!(
        rows[reject][column(58)].as_str(),
        Some("Counterparty unavailable - please contact support")
    );
    assert_eq!(rows[reject][column(385)].as_str(), Some("S"));
    // The groups the row packs replicate inside the frame exactly as they
    // do on the bridge row the frame carries, the stated count included.
    assert_eq!(rows[reject][column(453)], Scalar::from(10_i32));
    assert_eq!(rows[reject][column(768)], Scalar::from(4_i32));
    assert_eq!(
        message
            .by_path(&path("Parties"))
            .unwrap()
            .as_sequence()
            .map(<[Scalar]>::len),
        Some(10)
    );
    assert_eq!(
        message
            .by_path(&path("TrdRegTimestamps"))
            .unwrap()
            .as_sequence()
            .map(<[Scalar]>::len),
        Some(5)
    );
}

#[test]
fn a_group_packed_inside_an_occurrence_nests_under_it_and_a_republication_is_dropped() {
    let codec = codec();
    let (text_names, text) = text_rows();
    let lines = text_lines();
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
        .enriched_line(&lines[rows[0]])
        .and_then(|mut messages| messages.next().expect("a message"))
        .expect("the flattened row reads");
    let nested = codec
        .enriched_line(&lines[rows[1]])
        .and_then(|mut messages| messages.next().expect("a message"))
        .expect("the packed row reads");
    for message in [&flat, &nested] {
        // Six parties either way, the first the executing firm: the tag the
        // row spelled is the counter's own column, and the occurrences are
        // the group the dictionary files under `Parties`.
        assert_eq!(message.by_tag(453).unwrap(), &Scalar::from(6_i32));
        let parties = message
            .by_path(&path("Parties"))
            .unwrap()
            .as_sequence()
            .expect("parties");
        assert_eq!(parties.len(), 6);
        assert_eq!(
            message.by_path(&path("Parties[0].PartyID")).unwrap(),
            &Scalar::from("HIGH_TOUCH")
        );
        assert_eq!(
            message.by_path(&path("Parties[1].PartyID")).unwrap(),
            &Scalar::from("SWXCCP")
        );
    }
    // Flattened, the sub-identifier group is a group of the row.
    assert_eq!(
        flat.by_path(&path("PtysSubGrp[0].PartySubID")).unwrap(),
        &Scalar::from("trader1")
    );
    // Packed inside the party, it is the party's own: reached through it,
    // typed through its own field - `contactname` is the code set's
    // spelling of 9 - and absent from a party that packed none.
    assert_eq!(
        nested
            .by_path(&path("Parties[0].PtysSubGrp[0].PartySubID"))
            .unwrap(),
        &Scalar::from("trader1")
    );
    assert_eq!(
        nested
            .by_path(&path("Parties[0].PtysSubGrp[0].PartySubIDType"))
            .unwrap()
            .as_i64(),
        Some(9)
    );
    assert!(
        nested
            .get_by_path(&path("Parties[1].PtysSubGrp"))
            .is_none_or(Scalar::is_null)
    );
    assert!(
        nested.get_by_tag(802).is_none() && nested.get_by_path(&path("PtysSubGrp")).is_none(),
        "no sub-identifier counter or group at the row level"
    );
    // The row nests three deep; the arrival record nests as the bridge wrote
    // it, which is one occurrence under the counter that heads it. Everything
    // above - the sub-counter, the sub-identifier, the party's own members -
    // is this reader's reading of the value that occurrence packed, and no
    // range of the line spells any of their keys.
    let parties = nested
        .entries()
        .iter()
        .find(|entry| entry.tag() == 453)
        .expect("the party counter");
    let keys: Vec<&str> = parties
        .children()
        .iter()
        .map(|entry| entry.key().as_str().unwrap_or_default())
        .collect();
    assert!(
        keys.iter().all(|key| key.starts_with("NOPARTYIDS[")),
        "{keys:?}"
    );
    let packed = parties.children()[0]
        .value()
        .as_str()
        .expect("the packed value");
    assert!(packed.contains("NOPARTYSUBIDS"), "{packed}");
    assert!(packed.contains("PARTYID="), "{packed}");

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
    let lines = text_lines();
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
            .enriched_line(&lines[row])
            .and_then(|mut messages| messages.next().expect("a message"))
            .expect("the row reads")
    };

    // A configuration document: the exchange on the bridge's tags.
    let document = find(
        r#"Response: {"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=OMS_X1_TradeCapture"#,
    );
    // The line classifies as the JSON it is: what makes a document *this*
    // reader's is a shape the codec recognizes rather than a name the
    // classifier gives it (decision 17).
    assert_eq!(mimetype(document).as_deref(), Some(MimeType::JSON.as_str()));
    let message = read(document);
    // The message is the plugin's attributes and nothing the Jolokia answer
    // wrapped them in: the four envelope tags hold nothing, and the ObjectName
    // the read answered for is the `SessionInterface` attribute.
    super::states_no_envelope(&message);
    assert_eq!(
        message
            .by_tag(super::SESSIONINTERFACE_TAG)
            .unwrap()
            .as_str()
            .unwrap_or_default(),
        "com.ullink.ulbridge.sessioninterfaces.plugins:name=OMS_X1_TradeCapture,\
         plugin-type=FIX,type=Plugin"
    );
    assert_eq!(
        message.by_path(&path("CurrentPort")).unwrap(),
        &Scalar::from(9726_i64)
    );
    // A wildcard read answers for every plugin, one message each, and every
    // message carries its own MBean's attributes flat.
    let wildcard = find(r#""mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*""#);
    let messages = codec
        .enriched_line(&lines[wildcard])
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
            .find(|message| {
                message.get_by_path(&path("Name")).and_then(Scalar::as_str) == Some(name)
            })
            .unwrap_or_else(|| panic!("a message for {name}"));
        assert_eq!(
            held.by_path(&path("CurrentPort")).unwrap(),
            &Scalar::from(port)
        );
    }
    // An error answer names no plugin, so it states no message: what failed
    // is how the asking went and never a configuration, and a read that
    // answers no plugin answers nothing rather than an envelope with nothing
    // inside it (decision 17). The line is still a line, still classified as
    // JSON, and it is the FIX reader that has nothing to say about it.
    let error = find(r#""error_type":"javax.management.InstanceNotFoundException""#);
    assert_eq!(mimetype(error).as_deref(), Some(MimeType::JSON.as_str()));
    assert!(
        codec
            .enriched_line(&lines[error])
            .expect("the error answer reads")
            .next()
            .is_none(),
        "an error-only answer names no plugin",
    );
    assert!(silent(error), "and so it is no row of the batch");

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
        message.by_path(&path("Parties[0].PartyID")).unwrap(),
        &Scalar::from("TRADER2")
    );
    // A coded member is the code's value, typed: `11` is an order origination trader.
    assert_eq!(
        message
            .by_path(&path("Parties[0].PartyRole"))
            .unwrap()
            .as_i64(),
        Some(11)
    );

    // A marked frame states its direction in front of it, and the row
    // carries it as FIX's own tag 385 (decision 14).
    let marked = find("|11=OD9EOEDJ401|55=HOLN|54=1|38=50|");
    assert_eq!(rows[row_of(marked)][column(35)].as_str(), Some("D"));
    assert_eq!(rows[row_of(marked)][column(385)].as_str(), Some("S"));

    // A line with nothing after its header carries no message, so the FIX
    // batch holds no row for it at all (decision 16): what a line was is the
    // text reader's answer, and it still holds every one of them.
    let empty = text
        .iter()
        .position(|held| body(&text_names, held).is_empty())
        .expect("the empty line");
    assert!(silent(empty), "a line with no payload carries no message");
    assert_ne!(line_of(row_of(empty)), empty, "no row is that line's");
    assert!(!text[empty][at(&text_names, "timestamp")].is_null());
    // And a sentence is a sentence: classified as one, and no message.
    let warning = find("Unable to resolve destination for OD9EOEDJ401");
    assert_eq!(
        mimetype(warning).as_deref(),
        Some(MimeType::OCTET_STREAM.as_str())
    );
    assert!(silent(warning), "a sentence carries no message");
    assert!(
        codec
            .parse_text_line(&lines[warning])
            .expect("a line")
            .next()
            .is_none()
    );
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
        .clone()
        .with_separator(b'|')
        .write_arrow_reader(filled, &mut written)
        .expect("the capture writes");
    assert_eq!(rows, ROWS as u64);
    // A row in is a line out: every row's wire, the codec's separator, a
    // newline. The wires are read once more through the line door, which
    // the equivalence identity says answers the same entries, so the written
    // bytes are compared whole - a configuration document's
    // `InitFileContent` carries newlines of its own inside one value, and a
    // split of the output on newlines would count those as rows.
    let mut wires: Vec<Vec<u8>> = Vec::with_capacity(ROWS);
    for line in &text_lines() {
        for message in codec.parse_text_line(line).expect("the line reads") {
            wires.push(message.expect("a message").into_bytes(b'|'));
        }
    }
    assert_eq!(wires.len(), ROWS);
    let mut expected = Vec::with_capacity(written.len());
    for wire in &wires {
        expected.extend_from_slice(wire);
        expected.push(b'\n');
    }
    assert_eq!(
        String::from_utf8_lossy(&written),
        String::from_utf8_lossy(&expected),
        "the writer re-emits every row's wire, one line each"
    );
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
        assert_eq!(
            std::str::from_utf8(&wires[row]).expect("text"),
            frame,
            "row {row} re-emits its frame"
        );
        checked += 1;
    }
    assert!(checked >= 11, "{checked} frames checked");
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
    let lines = text_lines();
    let line = text
        .iter()
        .position(|held| body(&text_names, held).contains("213=<FIXML"))
        .expect("the frame carrying a document");
    let message = codec
        .enriched_line(&lines[line])
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
    let lines = text_lines();
    let line = text
        .iter()
        .position(|held| body(&text_names, held).contains("MSGTYPE=tradecapturereport"))
        .expect("the trade capture frame");
    let message = codec
        .enriched_line(&lines[line])
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

    // The row calls itself a trade capture report. The dialect declares no
    // message of that name, so the standard one answers, and its grammar is
    // what places `NoLegs` under `TrdInstrmtLegGrp` and `NoSides` under
    // `TrdCapRptSideGrp` - two counters half the dictionary shares, which no
    // registry-wide lookup could place. Each count keeps its own column and
    // the group it heads sits beside it.
    assert_eq!(message.by_tag(555).unwrap().as_i64(), Some(1));
    assert_eq!(message.by_tag(552).unwrap().as_i64(), Some(1));

    // A group packed inside an occurrence nests inside it rather than beside
    // it, at every depth the payload packs one: the leg carries its own
    // allocations, and the side its parties, and a party its sub-identifiers.
    let legs = root
        .get_field_by_path("trdinstrmtleggrp")
        .expect("the leg group");
    let allocations = packed_group(legs, "legpreallocgrp");
    assert!(
        packed_members(allocations).contains(&"legallocaccount"),
        "{:?}",
        packed_members(allocations)
    );
    // Where an inner occurrence ends the bridge writes two separators in a
    // row, and that close is what bounds it: the venue's own
    // `TR_LEGCALCULATEDALLOCQTY` packed before the close is the
    // allocation's, and the leg's `OPTIONSTRATEGY` after it is the leg's.
    assert!(
        packed_members(allocations).contains(&"trlegcalculatedallocqty"),
        "{:?}",
        packed_members(allocations)
    );
    assert!(
        packed_members(legs).contains(&"optionstrategy")
            && !packed_members(legs).contains(&"trlegcalculatedallocqty"),
        "{:?}",
        packed_members(legs)
    );
    assert_eq!(
        message
            .by_path(&path("TrdInstrmtLegGrp[0].LegPreAllocGrp[0].LegAllocQty"))
            .unwrap()
            .as_f64(),
        Some(600.0)
    );
    let sides = root
        .get_field_by_path("trdcaprptsidegrp")
        .expect("the side group");
    let parties = packed_group(sides, "parties");
    let subs = packed_group(parties, "ptyssubgrp");
    assert!(
        packed_members(subs).contains(&"partysubid"),
        "{:?}",
        packed_members(subs)
    );
    // Seven parties, each holding its own members and its own
    // sub-identifiers, and nothing of theirs on the side: a party packed
    // after one carrying sub-identifiers is still a party of its own.
    let party = |index: usize, member: &str| {
        message
            .by_path(&path(&format!(
                "TrdCapRptSideGrp[0].Parties[{index}].{member}"
            )))
            .unwrap_or_else(|error| panic!("party {index} {member}: {error}"))
            .clone()
    };
    assert_eq!(
        message
            .by_path(&path("TrdCapRptSideGrp[0].Parties"))
            .unwrap()
            .as_sequence()
            .map(<[Scalar]>::len),
        Some(7)
    );
    for (index, id, role) in [
        (0, "trader1", 11),
        (1, "trader1", 12),
        (2, "CITP", 1),
        (3, "PICT", 3),
        (5, "DEFAULT", 72),
        (6, "trader1", 122),
    ] {
        assert_eq!(party(index, "PartyID").as_str(), Some(id), "party {index}");
        assert_eq!(
            party(index, "PartyRole").as_i64(),
            Some(role),
            "party {index}"
        );
    }
    assert_eq!(party(4, "PartyID").as_str(), Some("DGVG"));
    for (index, sub) in [(1, "TRADER ONE"), (2, "EXAMPLEBK"), (3, "EXAMPLECO")] {
        assert_eq!(
            party(index, "PtysSubGrp[0].PartySubID").as_str(),
            Some(sub),
            "party {index}"
        );
    }
    assert_eq!(
        party(5, "PtysSubGrp").as_sequence().map(<[Scalar]>::len),
        Some(2)
    );
    assert_eq!(
        party(5, "PtysSubGrp[1].PartySubID").as_str(),
        Some("5493000EXAMPLE00000H")
    );
    assert!(
        !packed_members(sides).contains(&"partyid")
            && !packed_members(sides).contains(&"ptyssubgrp"),
        "{:?}",
        packed_members(sides)
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

/// The text line whose body holds `needle`.
fn line_holding(text_names: &[String], text: &[Vec<Scalar>], needle: &str) -> usize {
    text.iter()
        .position(|held| body(text_names, held).contains(needle))
        .unwrap_or_else(|| panic!("a line holding {needle:?}"))
}

/// The fifteen lines the capture ends on, as the bridge logged them: a
/// cancel request going out and its reject coming back, the reject routed,
/// enriched with its regulatory clocks and forwarded as a bridge row, the
/// `35=UL` frame that row goes out in, and two heartbeats. This test and the
/// two after it pin what those lines carry that no earlier line did; the
/// suites above already read every one of them both ways and require the
/// two doors to agree.
#[test]
fn a_cancel_request_and_its_reject_are_typed_and_the_reject_states_its_state() {
    let codec = codec();
    let (text_names, text) = text_rows();
    let lines = text_lines();
    let batch = batches(None);
    let (_, rows) = rows_of(&batch);
    let schema = yggdryl::Field::from_arrow_schema("row", &batch[0].schema()).expect("the schema");
    let column =
        |tag: i32| yggdryl::fix_column_of(&schema, tag).unwrap_or_else(|| panic!("tag {tag}"));
    let read = |line: usize| {
        codec
            .enriched_line(&lines[line])
            .and_then(|mut messages| messages.next().expect("a message"))
            .expect("the line reads")
    };

    // The request: a frame typed by its code, sent by the bridge, naming the
    // order it cancels and the order it replaces.
    let request = line_holding(&text_names, &text, "|35=F|");
    let message = read(request);
    assert_eq!(message.as_field().name(), "F");
    assert_eq!(message.by_tag(35).unwrap().as_str(), Some("F"));
    assert_eq!(
        rows[row_of(request)][column(11)].as_str(),
        Some("0102000452788802")
    );
    assert_eq!(
        rows[row_of(request)][column(41)].as_str(),
        Some("0102000452788801")
    );
    assert_eq!(rows[row_of(request)][column(54)].as_str(), Some("2"));
    assert_eq!(rows[row_of(request)][column(38)].as_f64(), Some(10000.0));
    assert_eq!(rows[row_of(request)][column(385)].as_str(), Some("S"));
    // A request states no status, so the crate's `state` has nothing to read.
    assert!(rows[row_of(request)][column(yggdryl::STATE_TAG_NAME.0)].is_null());

    // The reject: typed by its code, received, answering the cancel request
    // (`434=1`), stating the order rejected and why.
    let reject = line_holding(&text_names, &text, "|35=9|");
    let message = read(reject);
    assert_eq!(message.as_field().name(), "9");
    assert_eq!(message.by_tag(35).unwrap().as_str(), Some("9"));
    assert_eq!(message.by_tag(434).unwrap().as_str(), Some("1"));
    assert_eq!(
        message
            .by_tag(39)
            .unwrap()
            .as_str()
            .and_then(State::from_spelling),
        State::from_spelling("8")
    );
    assert_eq!(
        message.by_tag(58).unwrap().as_str(),
        Some("Counterparty unavailable - please contact support")
    );
    let row = &rows[row_of(reject)];
    assert_eq!(row[column(385)].as_str(), Some("R"));
    assert_eq!(
        row[column(39)].as_str().and_then(State::from_spelling),
        State::from_spelling("8")
    );
    // The crate's `state` is the status the message stated, on the row and
    // on the message alike, and `CxlRejResponseTo` is no fixed column, so
    // the message alone answers for it.
    assert_eq!(row[column(yggdryl::STATE_TAG_NAME.0)], row[column(39)]);
    assert_eq!(
        message.by_tag(yggdryl::STATE_TAG_NAME.0).unwrap(),
        message.by_tag(39).unwrap()
    );
    assert!(yggdryl::fix_column_of(&schema, 434).is_none());
    assert_eq!(
        row[column(58)].as_str(),
        Some("Counterparty unavailable - please contact support")
    );
    // Both the request and the reject re-emit byte for byte from their
    // arrival record.
    for line in [request, reject] {
        let frame = body(&text_names, &text[line]);
        let frame = &frame[frame.find("8=FIX").expect("a frame")..];
        assert_eq!(
            String::from_utf8(read(line).into_bytes(b'|')).expect("text"),
            frame
        );
    }
}

/// The bridge's own `#`-marked, packed and composed spellings are read by the
/// rules the codec documents on `parse_ullink_line`, so this is those rules
/// held against a real row of a hundred keys rather than a fixture of three.
#[test]
fn the_forwarded_reject_row_reads_its_marked_packed_and_composed_keys() {
    let codec = codec();
    let (text_names, text) = text_rows();
    let lines = text_lines();
    let batch = batches(None);
    let (_, rows) = rows_of(&batch);
    let schema = yggdryl::Field::from_arrow_schema("row", &batch[0].schema()).expect("the schema");
    let column =
        |tag: i32| yggdryl::fix_column_of(&schema, tag).unwrap_or_else(|| panic!("tag {tag}"));
    let read = |line: usize| {
        codec
            .enriched_line(&lines[line])
            .and_then(|mut messages| messages.next().expect("a message"))
            .expect("the line reads")
    };

    // The row sits in parentheses after prose on the line the bridge logs
    // its forwarding with, and bare on the line the next hop pushes: one
    // row, read twice, is one message twice - same entries, same digest.
    let received = line_holding(
        &text_names,
        &text,
        "Message received: Message type [cancel/replace reject]",
    );
    let pushed = line_holding(
        &text_names,
        &text,
        "PushMessage : #BLOOMBERGCODE=2454 TT Equity|",
    );
    let message = read(received);
    let bare = read(pushed);
    assert_eq!(message.entries(), bare.entries());
    assert_eq!(message.digest(), bare.digest());
    assert_eq!(message.as_field().name(), "cancelreject");
    assert_eq!(
        message
            .by_tag(yggdryl::TARGETSESSIONNAME_TAG_NAME.0)
            .unwrap()
            .as_str(),
        Some("ULMSG_DMZ_FIRM"),
        "the paren closing the remark is not the session's name"
    );

    // A `#`-marked key with no bare twin is the key it marks: the bridge's
    // `#DELIVERTOCOMPID`, `#SECURITYEXCHANGE`, `#SECURITYID`, `#MINTRADEVOL`
    // and `#IDSOURCE` - the 4.2 spelling of `SecurityIDSource` - land on
    // their tags, mark dropped. A marked twin stating the same bytes as the
    // bare key is dropped (`#CFICODE` beside `CFICODE`); one stating other
    // bytes stays verbatim beside it, its own key and its own entry
    // (`#SYMBOL=TW0002454006` beside `SYMBOL=2454`).
    assert_eq!(message.by_tag(128).unwrap().as_str(), Some("ITGA"));
    assert_eq!(message.by_tag(207).unwrap().as_str(), Some("XTAI"));
    assert_eq!(message.by_tag(48).unwrap().as_str(), Some("TW0002454006"));
    assert_eq!(message.by_tag(22).unwrap().as_str(), Some("4"));
    assert_eq!(message.by_tag(562).unwrap().as_f64(), Some(1.0));
    assert_eq!(message.by_tag(461).unwrap().as_str(), Some("ESXXXX"));
    assert_eq!(
        message
            .entries()
            .iter()
            .filter(|entry| entry.tag() == 461)
            .count(),
        1
    );
    assert_eq!(message.by_tag(55).unwrap().as_str(), Some("2454"));
    assert_eq!(
        message.by_name("#SYMBOL").unwrap().as_str(),
        Some("TW0002454006")
    );
    let marked = message
        .entries()
        .iter()
        .find(|entry| entry.key().as_str() == Some("#SYMBOL"))
        .expect("the marked twin's own entry");
    assert_eq!(marked.tag(), 0);
    assert_eq!(
        message
            .entries()
            .iter()
            .filter(|entry| entry.tag() == 55)
            .count(),
        1
    );
    let row = &rows[row_of(received)];
    assert_eq!(row[column(55)].as_str(), Some("2454"));
    assert_eq!(row[column(207)].as_str(), Some("XTAI"));
    assert_eq!(
        row[column(yggdryl::MICCODE_TAG_NAME.0)].as_str(),
        Some("XTAI")
    );
    assert_eq!(
        row[column(yggdryl::ISINCODE_TAG_NAME.0)].as_str(),
        Some("TW0002454006")
    );

    // Ten packed parties replicate into the `parties` group, every member
    // typed where the dictionary's code set spells the bridge's word and
    // kept as text where the member is text; a role FIX has no code for
    // (`orderoriginatorsystem`) and a qualifier it has none for (`buyside`)
    // are null. `party` addresses a party by role, and answers none where
    // two bear it.
    assert_eq!(message.by_tag(453).unwrap(), &Scalar::from(10_i32));
    assert_eq!(row[column(453)], Scalar::from(10_i32));
    let parties = message
        .by_path(&path("Parties"))
        .unwrap()
        .as_sequence()
        .expect("parties");
    assert_eq!(parties.len(), 10);
    let party = |index: usize, member: &str| {
        message
            .get_by_path(&path(&format!("Parties[{index}].{member}")))
            .cloned()
            .unwrap_or(Scalar::Null)
    };
    for (index, id, source, role) in [
        (0, "3000090.019", "proprietary/customcode", Some(3)),
        (1, "BU/SGP", "shortcodeid", Some(3)),
        (2, "FIRMA9120", "proprietary/customcode", Some(5)),
        (3, "0101", "proprietary/customcode", Some(36)),
        (4, "0102TRADER3", "proprietary/customcode", Some(12)),
        (5, "0101", "proprietary/customcode", Some(122)),
        (6, "2000657.018", "proprietary/customcode", Some(1)),
        (
            7,
            "ITGA_TRITON@2",
            "generallyacceptedmarketparticipantidentifier",
            Some(1),
        ),
        (8, "OMSVENDOR0102", "proprietary/customcode", None),
        (9, "Autex", "proprietary/customcode", None),
    ] {
        assert_eq!(party(index, "PartyID").as_str(), Some(id), "party {index}");
        assert_eq!(
            party(index, "PartyIDSource").as_str(),
            Some(source),
            "party {index}"
        );
        assert_eq!(party(index, "PartyRole").as_i64(), role, "party {index}");
    }
    assert!(party(8, "PartyRoleQualifier").is_null());
    let trader = message
        .party("ExecutingTrader")
        .expect("one executing trader");
    assert_eq!(trader.id().and_then(Scalar::as_str), Some("0102TRADER3"));
    assert_eq!(
        trader.source().and_then(Scalar::as_str),
        Some("proprietary/customcode")
    );
    assert_eq!(trader.role().and_then(Scalar::as_i64), Some(12));
    assert!(trader.qualifier().is_none_or(Scalar::is_null));
    assert_eq!(
        message
            .party("InvestorID")
            .and_then(|party| party.id())
            .and_then(Scalar::as_str),
        Some("FIRMA9120")
    );
    assert!(
        message.party("ExecutingFirm").is_none(),
        "two parties bear the role, so none is the executing firm"
    );

    // Two alternate identifiers, their sources kept as the bridge spelled
    // them.
    assert_eq!(message.by_tag(454).unwrap(), &Scalar::from(2_i32));
    for (index, id, source) in [(0, "2454 TT Equity", "bloomberg"), (1, "2454.TW", "ric")] {
        assert_eq!(
            message
                .by_path(&path(&format!("SecAltIDGrp[{index}].SecurityAltID")))
                .unwrap()
                .as_str(),
            Some(id)
        );
        assert_eq!(
            message
                .by_path(&path(&format!("SecAltIDGrp[{index}].SecurityAltIDSource")))
                .unwrap()
                .as_str(),
            Some(source)
        );
    }

    // The regulatory clocks: the bridge states four and indexes five. The
    // occurrences are what it indexed, all five; the counter is what it
    // stated, four, never renumbered; and the disagreement is reported,
    // not repaired. Only the submission time bears a type FIX spells; the
    // bridge's own kinds are null, and `trd_reg_timestamp` reads by FIX's.
    assert_eq!(message.by_tag(768).unwrap(), &Scalar::from(4_i32));
    assert_eq!(row[column(768)], Scalar::from(4_i32));
    let stamps = message
        .by_path(&path("TrdRegTimestamps"))
        .unwrap()
        .as_sequence()
        .expect("the timestamps");
    assert_eq!(stamps.len(), 5);
    let stamp = |index: usize, member: &str| {
        message
            .get_by_path(&path(&format!("TrdRegTimestamps[{index}].{member}")))
            .cloned()
            .unwrap_or(Scalar::Null)
    };
    // The bridge's compact spelling, `20260814011345549`, read as the
    // millisecond it is: 2026-08-14T01:13:45.549Z and 21:59:46.479Z.
    let clock = |seconds: i64, millis: i64| {
        Scalar::datetime64(
            (seconds * 1_000 + millis) * 1_000_000,
            yggdryl::TimeUnit::Nanosecond,
            Timezone::UTC,
        )
        .expect("a clock")
    };
    const SUBMITTED: i64 = 1786670025;
    const CANCELLED: i64 = 1786744786;
    for (index, when, kind, origin) in [
        (0, clock(SUBMITTED, 0), None, "CLIENT"),
        (1, clock(SUBMITTED, 549), None, "ULBRIDGE"),
        (2, clock(SUBMITTED, 162), Some(10), "CLIENT"),
        (3, clock(CANCELLED, 416), None, "ULBRIDGE"),
        (4, clock(CANCELLED, 479), None, "CLIENT"),
    ] {
        assert_eq!(stamp(index, "TrdRegTimestamp"), when, "stamp {index}");
        assert_eq!(
            stamp(index, "TrdRegTimestampType").as_i64(),
            kind,
            "stamp {index}"
        );
        assert_eq!(
            stamp(index, "TrdRegTimestampOrigin").as_str(),
            Some(origin),
            "stamp {index}"
        );
    }
    assert_eq!(
        message.trd_reg_timestamp("OrderSubmissionTime"),
        Some(&clock(SUBMITTED, 162))
    );
    assert!(message.trd_reg_timestamp("ExecutionTime").is_none());
    let anomalies: Vec<yggdryl::FixAnomaly<'_>> = message.anomalies().collect();
    assert!(
        anomalies.contains(&yggdryl::FixAnomaly::Miscounted {
            tag: 768,
            name: "trdregtimestamps",
            stated: 4,
            held: 5,
        }),
        "{anomalies:?}"
    );
    // The enriched result two hops earlier states five and indexes five,
    // and reports no miscount.
    let enriched = read(line_holding(
        &text_names,
        &text,
        "Result of message post-enrichment : #NOTRDREGTIMESTAMPS=5|",
    ));
    assert_eq!(enriched.by_tag(768).unwrap(), &Scalar::from(5_i32));
    assert!(
        enriched
            .anomalies()
            .all(|anomaly| !matches!(anomaly, yggdryl::FixAnomaly::Miscounted { .. })),
        "{:?}",
        enriched.anomalies().collect::<Vec<_>>()
    );

    // Composed keys (decision 20): a namespace's last segment fills the
    // absent dictionary field it names - `OMSVENDOR.ORDERQTY` fills
    // `OrderQty`, `OMSVENDOR.TIMEINFORCE` fills `TimeInForce` - and the
    // filled field takes its tag while the pair the bridge wrote stays the
    // arrival, under its own name and no tag. A segment naming no field -
    // `FIRM.ACRONYM`, `ULLINK.INSTRUMENTID`, `ULLINK.BYPASSRISK` - stays
    // under its namespace, verbatim.
    assert_eq!(message.by_tag(38).unwrap().as_f64(), Some(10000.0));
    assert_eq!(row[column(38)].as_f64(), Some(10000.0));
    assert_eq!(message.by_tag(59).unwrap().as_str(), Some("day"));
    for (key, value) in [
        ("omsvendor.orderqty", "10000"),
        ("omsvendor.timeinforce", "day"),
        ("firm.acronym", "FIRMA9120"),
        ("firm.last_instrument_lookup_date", "20260814"),
        ("ullink.instrumentid", "dbi;TW0002454006_XTAI_TWD"),
        ("ullink.bypassrisk", "yes"),
    ] {
        assert_eq!(message.by_name(key).unwrap().as_str(), Some(value), "{key}");
        let entry = message
            .entries()
            .iter()
            .find(|entry| {
                entry
                    .key()
                    .as_str()
                    .is_some_and(|held| held.eq_ignore_ascii_case(key))
            })
            .unwrap_or_else(|| panic!("the arrival of {key}"));
        assert_eq!(entry.tag(), 0, "{key}");
    }
    assert!(message.get_by_name("acronym").is_none());
    assert!(message.get_by_name("instrumentid").is_none());

    // The rejection the row states, typed as the frame's was, and the
    // response-to spelled as the bridge spells it, since the code set has
    // no such word and the field is text.
    assert_eq!(
        message
            .by_tag(39)
            .unwrap()
            .as_str()
            .and_then(State::from_spelling),
        State::from_spelling("8")
    );
    assert_eq!(
        row[column(yggdryl::STATE_TAG_NAME.0)]
            .as_str()
            .and_then(State::from_spelling),
        State::from_spelling("8")
    );
    assert_eq!(message.by_tag(434).unwrap().as_str(), Some("cancel"));
    assert_eq!(message.by_tag(167).unwrap().as_str(), Some("equity"));
}

#[test]
fn two_heartbeats_close_the_capture_and_one_says_it_is_no_resend() {
    let codec = codec();
    let (text_names, text) = text_rows();
    let lines = text_lines();
    let batch = batches(None);
    let (_, rows) = rows_of(&batch);
    let schema = yggdryl::Field::from_arrow_schema("row", &batch[0].schema()).expect("the schema");
    let column =
        |tag: i32| yggdryl::fix_column_of(&schema, tag).unwrap_or_else(|| panic!("tag {tag}"));
    let sent = line_holding(&text_names, &text, "|35=0|49=FIRMB|56=FIDESSA|");
    let received = line_holding(
        &text_names,
        &text,
        "|35=0|49=FIDESSA_BOPRD|56=FIRMAPRD|43=N|",
    );
    assert_eq!(received, LINES - 1, "the capture ends on it");
    let read = |line: usize| {
        codec
            .enriched_line(&lines[line])
            .and_then(|mut messages| messages.next().expect("a message"))
            .expect("the heartbeat reads")
    };
    let (sent_beat, received_beat) = (read(sent), read(received));
    for (line, message) in [(sent, &sent_beat), (received, &received_beat)] {
        assert_eq!(message.as_field().name(), "0");
        assert_eq!(rows[row_of(line)][column(35)].as_str(), Some("0"));
    }
    assert_eq!(rows[row_of(sent)][column(385)].as_str(), Some("S"));
    assert_eq!(rows[row_of(received)][column(385)].as_str(), Some("R"));
    // `43=N` is the boolean it is: this transmission is the original, and
    // the `resent` facet says so; the heartbeat that states nothing
    // states nothing.
    assert_eq!(received_beat.by_tag(43).unwrap(), &Scalar::from(false));
    assert_eq!(rows[row_of(received)][column(43)], Scalar::from(false));
    assert_eq!(received_beat.lifted("resent"), Some(&Scalar::from(false)));
    assert!(sent_beat.get_by_tag(43).is_none());
    assert!(rows[row_of(sent)][column(43)].is_null());
    assert!(sent_beat.lifted("resent").is_none());
    // A digest reads past the header and the trailer, and a heartbeat is
    // nothing else: two sessions' heartbeats digest equal, and only their
    // wires tell them apart.
    assert_eq!(sent_beat.digest(), received_beat.digest());
    assert_ne!(sent_beat.into_bytes(b'|'), received_beat.into_bytes(b'|'));
}
