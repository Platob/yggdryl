//! A capture in, columns out, and back to the wire - through the codec's
//! Arrow twins, and the two converters they compose.

use std::sync::Arc;

use arrow_array::RecordBatch;
use yggdryl::arrow::BatchReader;
use yggdryl::graph::{Element, Event};
use yggdryl::media::text::{TextBytes, TextLine};
use yggdryl::{DataType, FixCodec, FixDedup, FixMsg, FixRegistry, Scalar, fix_schema};

fn registry() -> Arc<FixRegistry> {
    super::committed_registry()
}

fn codec() -> FixCodec {
    super::fixed_codec(registry())
}

const BULK_CONFIG: &[u8] = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=*,plugin-type=FIX,type=Plugin","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,plugin-type=FIX,type=Plugin":{"Name":"A"},"com.ullink.ulbridge.sessioninterfaces.plugins:name=B,plugin-type=FIX,type=Plugin":{"Name":"B"}},"status":200}"#;

/// The crate's own fields beside a typed tag 385: the smallest dictionary a
/// stated direction lands in.
fn direction_registry() -> Arc<FixRegistry> {
    let mut registry = FixRegistry::new();
    // Tag 385 as the dictionary types it: text carrying its code set.
    let mut direction = DataType::utf8().nullable_field("MsgDirection");
    direction.as_fix_mut().set_tag(385).unwrap();
    direction
        .as_fix_mut()
        .set_codes(&[
            yggdryl::FixCode::new("Receive", "R"),
            yggdryl::FixCode::new("Send", "S"),
        ])
        .unwrap();
    registry.insert(direction).unwrap();
    Arc::new(registry)
}

/// Every shape a real capture holds, the corpus the readers are tested on.
const CAPTURE: &[&str] = &[
    "sending >> 8=FIX.4.2|9=176|35=D|11=ORDER-1|55=AAPL|54=1|10=203| << queued seq=1092",
    "raw 8=FIX.4.4|9=224|35=8|17=E1|37=O9|31=12.75|32=50|10=118|",
    "8=FIX.4.4|35=8|58=quoting #A=1 and #B=2|10=1|",
    "sending >> 8=FIX.4.2|35=UL|#SYMBOL=TTF|#SIDE=1|10=044|",
    "8=FIX.4.4|35=D|11=ORDER-1|SYMBOL=AAPL|SIDE=1|10=000",
    "toBridge #ISINCODE=XX|#SYMBOL=TTF|#SIDE=1",
    "ACCOUNT=A1|MSGTYPE=D|CLORDID=ORDER-1|SYMBOL=AAPL|SIDE=1",
    "After Enrichment -> ACCOUNT=ACCT-000117 CLIENTID=MCFP2 VENUE=XPAR",
    "Referential(dbi|equity|dbi;GB00BN7SWP63_XLON_GBX|[quantity-type=])",
    "<Order ClOrdID='XML-1'>body</Order>",
    "Receiving XmlApi: <Execution ExecID='E1'></Execution>",
    "Message rejected because : ignoring OMSSales expiry message",
    "no level printed by this plugin",
    "heartbeat emitted seq=7",
];

/// The capture as the batches a text reader hands the codec: one `body`
/// column of bytes, `rows` lines to an input batch.
fn capture_reader(lines: &[&str], rows: usize) -> BatchReader {
    let field = DataType::from_fields([DataType::binary().required_field("body")])
        .unwrap()
        .required_field("capture");
    let batches: Vec<RecordBatch> = lines
        .chunks(rows.max(1))
        .map(|chunk| {
            let values = Scalar::from_sequence(
                chunk
                    .iter()
                    .map(|line| Scalar::from_sequence([Scalar::from(line.as_bytes().to_vec())]))
                    .collect::<Vec<_>>(),
            );
            yggdryl::arrow::batch_from_value(&field, &values).unwrap()
        })
        .collect();
    let schema = field.into_arrow_schema().unwrap();
    yggdryl::arrow::batch_reader(schema, batches)
}

/// The whole capture, one input batch.
fn source() -> BatchReader {
    capture_reader(CAPTURE, CAPTURE.len())
}

fn batches(reader: BatchReader) -> Vec<RecordBatch> {
    reader.map(|batch| batch.expect("a batch")).collect()
}

fn row_count(batches: &[RecordBatch]) -> usize {
    batches.iter().map(RecordBatch::num_rows).sum()
}

/// One column of one batch, by name.
fn column<'batch>(
    batch: &'batch arrow_array::RecordBatch,
    name: &str,
) -> &'batch dyn arrow_array::Array {
    let index = batch
        .schema()
        .index_of(name)
        .unwrap_or_else(|_| panic!("a {name} column"));
    batch.column(index).as_ref()
}

/// One column of one batch, by the tag its field carries.
fn tag_column(batch: &RecordBatch, tag: i32) -> &dyn arrow_array::Array {
    batch.column(super::tag_index(batch, tag)).as_ref()
}

/// One projected scalar in the first row of a batch, by name.
fn first_value(batch: &RecordBatch, name: &str) -> Scalar {
    let index = batch
        .schema()
        .index_of(name)
        .unwrap_or_else(|_| panic!("a {name} column"));
    first_at(batch, index)
}

/// One projected scalar in the first row of a batch, by the tag its field
/// carries.
fn first_tag_value(batch: &RecordBatch, tag: i32) -> Scalar {
    first_at(batch, super::tag_index(batch, tag))
}

/// One projected scalar in the first row of a batch, by position.
fn first_at(batch: &RecordBatch, index: usize) -> Scalar {
    let rows = yggdryl::arrow::batch_to_value(batch).expect("the projected rows");
    rows.as_sequence().expect("rows")[0]
        .as_sequence()
        .expect("columns")[index]
        .clone()
}

#[test]
fn a_bulk_document_line_is_one_unknown_message_read_before_the_next_line_is_pulled() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let pulled = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&pulled);
    let lines = [BULK_CONFIG.to_vec(), Vec::new()]
        .into_iter()
        .inspect(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
        });
    let codec = super::fixed_codec(direction_registry());
    let mut messages = codec.parse_lines(lines);
    assert_eq!(pulled.load(Ordering::SeqCst), 0);
    // A wildcard answer naming two MBeans is one message, not two: a JSON
    // document is a body the codec does not read, so the row is one
    // `unknown` with no entries whatever the document holds.
    let document = messages.next().unwrap().unwrap();
    assert_eq!(document.as_field().name(), "unknown");
    assert!(document.entries().is_empty());
    assert_eq!(pulled.load(Ordering::SeqCst), 1);
    // An empty line is not a row at all: an `Err` item, and the stream goes on.
    assert!(messages.next().unwrap().is_err());
    assert_eq!(pulled.load(Ordering::SeqCst), 2);
    assert!(messages.next().is_none());
}

#[test]
fn a_document_row_is_one_row_carrying_its_source_columns_and_stated_direction() {
    let field = DataType::from_fields([
        DataType::Int64.required_field("rownum"),
        DataType::utf8().required_field("msgdirection"),
        DataType::binary().required_field("body"),
    ])
    .unwrap()
    .required_field("capture");
    let rows = Scalar::from_sequence([Scalar::from_sequence([
        Scalar::from(42_i64),
        Scalar::from("Receive"),
        Scalar::from(BULK_CONFIG.to_vec()),
    ])]);
    let source = yggdryl::arrow::batch_from_value(&field, &rows).unwrap();
    // One byte a batch: a batch a message, so a row that expanded would be
    // seen as the several batches it made. A bulk document expands into
    // nothing: one `unknown` row, carrying the source row's own columns.
    let reader = super::fixed_codec(direction_registry())
        .with_batch_byte_size(1)
        .parse_text_arrow_reader(yggdryl::arrow::batch_reader(source.schema(), [source]))
        .unwrap();
    let batches = batches(reader);
    assert_eq!(batches.len(), 1, "one row for the one document");
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 1);
    assert_eq!(first_value(batch, "rownum"), Scalar::from(42_i64));
    // A stated column is a spelling of a code of the set, stored as
    // the code.
    assert_eq!(first_tag_value(batch, 385).as_str(), Some("R"));
}

#[test]
fn the_schema_is_decided_before_the_first_row_is_read() {
    // Nothing is pulled to answer this: an empty capture has the same columns
    // a full one does, which is the whole point.
    let reader = codec()
        .parse_text_arrow_reader(capture_reader(&[], 1))
        .unwrap();
    let schema = reader.schema();
    let names: Vec<&str> = schema
        .fields()
        .iter()
        .map(|held| held.name().as_str())
        .collect();

    // The capture's own column leads, then the crate's own clocks; the rest
    // of the fixed columns follow, named by their folded names, each
    // carrying its tag on the field - which is what the row is filled by.
    // The capture's own column leads and the crate's own clocks open the
    // fixed ones, each found by its name rather than by an offset.
    assert_eq!(names.first(), Some(&"body"), "{names:?}");
    let at = |name: &str| {
        names
            .iter()
            .position(|held| *held == name)
            .unwrap_or_else(|| panic!("a {name} column in {names:?}"))
    };
    for pair in ["body", "unix", "creatunix", "prevunix", "expirunix"].windows(2) {
        assert!(at(pair[0]) < at(pair[1]), "{pair:?} in {names:?}");
    }
    assert_eq!(names.last(), Some(&"fixentries"));
    // The standard header, the body a consumer queries, the groups worth
    // keeping whole, the trailer, and this crate's own derived facts - each
    // found by the tag its column carries.
    let fixed = yggdryl::Field::from_arrow_schema("row", &schema).expect("the schema reads");
    for tag in [
        55,
        54,
        44,
        38,
        60, // the trade
        132,
        133,
        134,
        135, // the quote's lanes
        453,
        454,
        768, // the groups
        10,  // the trailer
        yggdryl::HASHCODE_TAG_NAME.0,
        yggdryl::UNIX_TAG_NAME.0,
        yggdryl::CREATUNIX_TAG_NAME.0, // the digest and the clocks
        yggdryl::MSGSESSIONID_TAG_NAME.0,
        yggdryl::MSGCTXID_TAG_NAME.0, // what a bridge's own log states
        yggdryl::MSGDIRECTION_TAG_NAME.0, // which way the line moved
    ] {
        assert!(
            yggdryl::fix_column_of(&fixed, tag).is_some(),
            "tag {tag} missing from {names:?}"
        );
    }

    // Each column carries its tag and the spelling it had, so a renderer can
    // show `MsgType` over the column `msgtype`.
    let msgtype = schema
        .field_with_name("msgtype")
        .expect("the msgtype column");
    assert_eq!(
        msgtype.metadata().get("fix:tag").map(String::as_str),
        Some("35"),
    );
    assert_eq!(
        msgtype.metadata().get("display").map(String::as_str),
        Some("MsgType"),
    );
    // And an empty capture yields no batch at all.
    assert_eq!(reader.count(), 0);
}

#[test]
fn a_capture_answers_one_row_per_message_not_one_per_line() {
    let batches = batches(codec().parse_text_arrow_reader(source()).unwrap());
    // A FIX batch answers one row per *message*, and five of the fourteen
    // lines carry none, so they carry no row either - the text
    // reader is the one that answers a row per line. The silent five, and
    // why each states no message:
    //
    // - `After Enrichment -> ACCOUNT=... CLIENTID=... VENUE=...` separates its
    //   named pairs with whitespace alone and marks no key, so the run is
    //   prose carrying an `=` rather than a bridge row;
    // - `Referential(dbi|equity|...|[quantity-type=])` holds every pipe in
    //   front of its one `=`, so nothing names a separator for a run of pairs;
    // - `Message rejected because : ...` and `no level printed by this plugin`
    //   hold no `=` at all;
    // - `heartbeat emitted seq=7` states its one pair on whitespace, unmarked.
    //
    // None of the five opens a frame or carries a document either, so each is
    // a line and no row.
    assert_eq!(
        row_count(&batches),
        9,
        "nine of the {} lines carry a message",
        CAPTURE.len()
    );

    // A row that states no type is still a row, named `unknown` - but only a
    // bridge row or a document ever is, never a sentence, because a sentence
    // is not a row at all.
    let first = &batches[0];
    let msgtype = tag_column(first, 35);
    assert!(msgtype.is_valid(0), "a framed row states its type");
    assert!(
        !msgtype.is_valid(5),
        "the bridge row `toBridge #ISINCODE=XX|...` states no type"
    );
    assert!(
        !msgtype.is_valid(8),
        "the last row is the `<Execution>` document, which states no type"
    );
}

#[test]
fn the_line_read_and_the_batch_read_agree_on_separatorless_group_inference() {
    const BRIDGE: &str = "|#SYMBOL=TTF|#SIDE=1|#PRICE=41.25|#NOPARTYIDS=1\
|#NOPARTYIDS[0]=PARTYID=BUYSIDEPARTYIDSOURCE=DPARTYROLE=1|";

    let codec = codec();
    let schema = fix_schema(codec.registry(), "fix").unwrap();
    let rows = codec
        .arrow_reader(schema, codec.parse_lines([BRIDGE]))
        .expect("a line reader");
    let row_batch = rows.into_iter().next().unwrap().unwrap();

    let columns = codec
        .parse_text_arrow_reader(capture_reader(&[BRIDGE], 1))
        .expect("a payload-column reader");
    let column_batch = columns.into_iter().next().unwrap().unwrap();

    for batch in [&row_batch, &column_batch] {
        assert_eq!(first_tag_value(batch, 453), Scalar::from(1_i32));
        let group = first_value(batch, "parties");
        let parties = group.as_sequence().expect("the party group");
        assert_eq!(parties.len(), 1);
        let members = parties[0].as_sequence().expect("one occurrence");
        assert_eq!(members[0].as_str(), Some("BUYSIDE"));
        assert_eq!(members[1].as_str(), Some("D"));
        assert_eq!(members[2].as_i64(), Some(1));
    }
}

#[test]
fn the_entries_column_is_the_row_and_the_facets_are_a_convenience() {
    let reader = codec()
        .parse_text_arrow_reader(capture_reader(
            &["8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|38=100|10=0|"],
            1,
        ))
        .unwrap();
    let batch = reader.into_iter().next().unwrap().unwrap();
    assert_eq!(batch.num_rows(), 1);

    // The lifted facets answered.
    let symbol = tag_column(&batch, 55);
    assert!(symbol.is_valid(0));
    // The content code's storage is the `uint64` it is, not a string.
    let digest = tag_column(&batch, yggdryl::HASHCODE_TAG_NAME.0);
    assert_eq!(digest.data_type(), &arrow_schema::DataType::UInt64);
    assert!(digest.is_valid(0));
    // And the arrival record is there in full, which is what makes the batch
    // lossless rather than one reader's summary.
    let entries = column(&batch, "fixentries");
    assert!(entries.is_valid(0));
    assert_eq!(entries.len(), 1);
}

/// Two hundred wide orders, about 450 bytes each.
fn wide() -> Vec<String> {
    (0..200)
        .map(|index| {
            format!(
                "8=FIX.4.4|35=D|11=ORDER-{index:06}|58={}|10=0|",
                "x".repeat(400)
            )
        })
        .collect()
}

#[test]
fn several_small_input_batches_accumulate_into_one_output_batch() {
    let lines = wide();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    // Twenty input batches of ten rows, far under the default target.
    let whole = batches(
        codec()
            .parse_text_arrow_reader(capture_reader(&lines, 10))
            .unwrap(),
    );
    assert_eq!(whole.len(), 1, "one batch under the byte target");
    assert_eq!(whole[0].num_rows(), 200);

    // Under a target holding about five input batches, the output batches
    // are fewer than the input ones and no row is lost.
    let target = 5 * 10 * 470;
    let bounded = batches(
        codec()
            .with_batch_byte_size(target)
            .parse_text_arrow_reader(capture_reader(&lines, 10))
            .unwrap(),
    );
    assert!(
        (2..20).contains(&bounded.len()),
        "{} batches",
        bounded.len()
    );
    assert_eq!(row_count(&bounded), 200);
    for batch in &bounded[..bounded.len() - 1] {
        assert!(
            batch.num_rows() > 10,
            "{} rows: an input batch did not close an output batch alone",
            batch.num_rows()
        );
    }
}

#[test]
fn one_large_input_batch_splits_by_rows_in_proportion() {
    let lines = wide();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let raw: usize = lines.iter().map(|line| line.len()).sum();
    const TARGET: u64 = 4_096;
    let batches = batches(
        codec()
            .with_batch_byte_size(TARGET)
            .parse_text_arrow_reader(capture_reader(&lines, lines.len()))
            .unwrap(),
    );
    assert!(batches.len() > 1, "{} batches", batches.len());
    assert_eq!(
        row_count(&batches),
        200,
        "the bound shapes batches, it does not drop rows"
    );
    // One input batch charges every row the same share of its bytes, so the
    // cut is even: every closed batch holds the same number of rows, and
    // that many rows of raw capture is about the target.
    let closed = &batches[..batches.len() - 1];
    let rows = closed[0].num_rows();
    assert!(closed.iter().all(|batch| batch.num_rows() == rows));
    let per_row = raw / lines.len();
    let held = (rows * per_row) as u64;
    assert!(
        (TARGET / 2..=TARGET * 2).contains(&held),
        "{held} raw bytes against a {TARGET} target over {rows} rows",
    );
}

#[test]
fn a_batch_always_holds_at_least_one_row_and_the_default_target_is_stated_once() {
    let lines = wide();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    // A target no row fits under closes a batch after every row, so one
    // enormous line can never produce an empty batch.
    let batches = batches(
        codec()
            .with_batch_byte_size(1)
            .parse_text_arrow_reader(capture_reader(&lines, lines.len()))
            .unwrap(),
    );
    assert!(batches.iter().all(|batch| batch.num_rows() == 1));
    assert_eq!(batches.len(), 200);

    // The default target, stated once and read here so a change to it is a
    // change to this assertion.
    assert_eq!(codec().batch_byte_size(), FixCodec::DEFAULT_BATCH_BYTE_SIZE);
    assert_eq!(FixCodec::DEFAULT_BATCH_BYTE_SIZE, 128 * 1024 * 1024);
}

#[test]
fn messages_to_batches_close_on_the_arrival_records_raw_bytes() {
    let codec = codec();
    let schema = fix_schema(codec.registry(), "fix").unwrap();
    let lines = wide();
    // Under the default target: one batch, whatever the shape.
    let one = batches(
        codec
            .arrow_reader(schema.clone(), codec.parse_lines(lines.clone()))
            .unwrap(),
    );
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].num_rows(), 200);

    // A bound of about ten lines of pairs cuts the stream into batches of
    // about ten, and every row survives the cut.
    let bounded = codec.clone().with_batch_byte_size(10 * 115);
    let many = batches(
        bounded
            .arrow_reader(schema.clone(), codec.parse_lines(lines.clone()))
            .unwrap(),
    );
    assert!((10..40).contains(&many.len()), "{} batches", many.len());
    assert_eq!(row_count(&many), 200);
    for batch in &many[..many.len() - 1] {
        assert!(
            (5..=20).contains(&batch.num_rows()),
            "{} rows a batch",
            batch.num_rows()
        );
    }

    // A target of one byte is a batch a message.
    let each = batches(
        codec
            .clone()
            .with_batch_byte_size(1)
            .arrow_reader(schema, codec.parse_lines(lines))
            .unwrap(),
    );
    assert_eq!(each.len(), 200);
}

#[test]
fn messages_with_no_arrival_record_are_charged_by_their_row() {
    let codec = codec();
    let schema = fix_schema(codec.registry(), "fix").unwrap();
    let whole = batches(
        codec
            .arrow_reader(schema, codec.parse_lines(wide()))
            .unwrap(),
    );
    assert_eq!(whole.len(), 1);
    // The lifted columns alone: the projection a consumer keeps, whose rows
    // come back as messages holding no entries.
    let batch = &whole[0];
    let lifted: Vec<usize> = (0..batch.num_columns())
        .filter(|at| batch.schema().field(*at).name() != yggdryl::fix::FIXENTRIES_COLUMN)
        .collect();
    let projected = batch.project(&lifted).unwrap();
    assert_eq!(projected.num_rows(), 200);
    let reader = || yggdryl::arrow::batch_reader(projected.schema(), [projected.clone()]);

    // Under a bound of about ten rows of leaves, the stream is cut into
    // batches of about ten - it is not one batch of everything, which is
    // what charging a message with no wire the bare row width would make.
    let bounded = codec.clone().with_batch_byte_size(10 * 450);
    let many = batches(bounded.lifecycle_arrow_reader(reader()).unwrap());
    assert!((5..60).contains(&many.len()), "{} batches", many.len());
    assert_eq!(row_count(&many), 200);

    // A target of one byte is still a batch a message.
    let each = batches(
        codec
            .clone()
            .with_batch_byte_size(1)
            .lifecycle_arrow_reader(reader())
            .unwrap(),
    );
    assert_eq!(each.len(), 200);
}

#[test]
fn a_source_without_a_readable_payload_column_is_refused_before_a_row_is_read() {
    let codec = codec();
    let refusal = |error: yggdryl::Error| {
        assert!(
            matches!(error, yggdryl::Error::InvalidRecord { .. }),
            "{error}"
        );
        error.to_string()
    };
    let source = |field: yggdryl::Field, rows: Scalar| {
        let batch = yggdryl::arrow::batch_from_value(&field, &rows).unwrap();
        yggdryl::arrow::batch_reader(batch.schema(), [batch])
    };
    let frame = Scalar::from(b"8=FIX.4.4|35=D|11=A|10=0|".to_vec());

    // The column is named otherwise: refused, naming what was asked for and
    // what the source carries.
    let named_line = DataType::from_fields([DataType::binary().required_field("line")])
        .unwrap()
        .required_field("capture");
    let rows = Scalar::from_sequence([Scalar::from_sequence([frame.clone()])]);
    let message = refusal(
        codec
            .parse_text_arrow_reader(source(named_line.clone(), rows.clone()))
            .map(drop)
            .unwrap_err(),
    );
    assert!(
        message.contains("body") && message.contains("line"),
        "{message}"
    );
    // Pinned to that name, the same source reads.
    let pinned = codec.clone().with_payload_column("line");
    let read = batches(
        pinned
            .parse_text_arrow_reader(source(named_line, rows))
            .unwrap(),
    );
    assert_eq!(row_count(&read), 1);
    assert_eq!(first_tag_value(&read[0], 11).as_str(), Some("A"));

    // The column holds neither text nor bytes: refused, naming its type.
    let numbered = DataType::from_fields([DataType::Int64.required_field("body")])
        .unwrap()
        .required_field("capture");
    let numbers = Scalar::from_sequence([Scalar::from_sequence([Scalar::from(7_i64)])]);
    let message = refusal(
        codec
            .parse_text_arrow_reader(source(numbered, numbers))
            .map(drop)
            .unwrap_err(),
    );
    assert!(message.contains("int64"), "{message}");

    // Neither refusal exists at the line door, and that is the point of it: a
    // line holds its bytes as a typed field, so there is no column to name
    // wrongly and no cell that could hold a number instead. A line carrying
    // no bytes carries no message either: an empty payload column is a row
    // that had nothing to read, not a row holding an empty message.
    let mut silent = codec
        .parse_text_line(&TextLine::from_bytes(0, TextBytes::default()).unwrap())
        .unwrap();
    assert!(silent.next().is_none(), "no payload, no message");

    // The empty `unknown` survives for the one case that is not this: a
    // payload that was there and would not parse, which a batch must not fail
    // on.
    let malformed = TextBytes::from_bytes(b"<Order ClOrdID='X'></Nope>").unwrap();
    let mut broken = codec
        .parse_text_line(&TextLine::from_bytes(0, malformed).unwrap())
        .unwrap();
    let empty = broken.next().unwrap().unwrap();
    assert!(broken.next().is_none());
    assert_eq!(empty.as_field().name(), "unknown");
    assert!(empty.entries().is_empty());

    // And empty bytes at the byte door are still the one typed refusal: they
    // are not a row at all, where an empty payload column is a row that
    // carried nothing.
    let refused = codec.parse_line(b"").map(drop).unwrap_err();
    assert!(matches!(refused, yggdryl::Error::Parse { .. }), "{refused}");
}

#[test]
fn a_refused_line_ends_the_batch_stream_after_the_completed_prefix() {
    let codec = codec();
    let schema = fix_schema(codec.registry(), "fix").unwrap();
    let lines = ["8=FIX.4.4|35=D|11=A|10=0|", "", "8=FIX.4.4|35=D|11=B|10=0|"];

    // The line door yields the refusal as an item and goes on past it.
    let read: Vec<_> = codec.parse_lines(lines).collect();
    assert_eq!(read.len(), 3);
    assert!(read[0].is_ok() && read[1].is_err() && read[2].is_ok());

    // Composed into batches, the refusal is the reader's error: the prefix,
    // then the error, then nothing.
    let mut reader = codec
        .arrow_reader(schema, codec.parse_lines(lines))
        .unwrap();
    let prefix = reader.next().unwrap().unwrap();
    assert_eq!(prefix.num_rows(), 1);
    assert_eq!(first_tag_value(&prefix, 11).as_str(), Some("A"));
    assert!(reader.next().unwrap().is_err());
    assert!(reader.next().is_none(), "fused");

    // The capture door has no refusal to make and no empty row to answer
    // with: the empty cell is a row that carried no payload, so it yields no
    // row at all and the two framed lines come through as two.
    let read: Vec<_> = codec
        .parse_text_arrow_reader(capture_reader(&lines, lines.len()))
        .unwrap()
        .collect();
    assert!(read.iter().all(|batch| batch.is_ok()), "no line refused");
    let rows: Vec<RecordBatch> = read.into_iter().map(Result::unwrap).collect();
    assert_eq!(row_count(&rows), 2);
    assert_eq!(first_tag_value(&rows[0], 11).as_str(), Some("A"));
}

#[test]
fn decoded_line_intake_accepts_owned_borrowed_and_both_fallible_forms() {
    let codec = codec();
    let line = TextLine::from_bytes(
        0,
        TextBytes::from_bytes(b"8=FIX.4.4|35=D|11=A|10=0|8=FIX.4.4|35=D|11=B|10=0|").unwrap(),
    )
    .unwrap();
    let expected = codec
        .parse_text_line(&line)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(expected.len(), 2);
    for actual in [
        codec
            .parse_text_lines([line.clone()])
            .collect::<yggdryl::Result<Vec<_>>>(),
        codec
            .parse_text_lines([&line])
            .collect::<yggdryl::Result<Vec<_>>>(),
        codec
            .parse_text_lines([Ok::<_, yggdryl::Error>(line.clone())])
            .collect::<yggdryl::Result<Vec<_>>>(),
        codec
            .parse_text_lines([Ok::<_, yggdryl::Error>(&line)])
            .collect::<yggdryl::Result<Vec<_>>>(),
    ] {
        assert_eq!(actual.unwrap(), expected);
    }
    let schema = fix_schema(codec.registry(), "fix").unwrap();
    let owned = batches(
        codec
            .arrow_reader(schema.clone(), expected.clone())
            .unwrap(),
    );
    let fallible = batches(
        codec
            .arrow_reader(schema, expected.into_iter().map(Ok))
            .unwrap(),
    );
    assert_eq!(owned, fallible);
}

#[derive(Debug)]
struct SourceFailure(Arc<()>);

impl std::fmt::Display for SourceFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("source failure")
    }
}

impl std::error::Error for SourceFailure {}

pub(super) fn source_failure(marker: &Arc<()>) -> yggdryl::Error {
    std::io::Error::other(SourceFailure(Arc::clone(marker))).into()
}

pub(super) fn same_source_failure(error: yggdryl::Error, marker: &Arc<()>) {
    let yggdryl::Error::Io(error) = error else {
        panic!("the source error changed type: {error}");
    };
    let held = error
        .get_ref()
        .unwrap()
        .downcast_ref::<SourceFailure>()
        .unwrap();
    assert!(Arc::ptr_eq(&held.0, marker));
}

#[test]
fn composed_fallible_stages_are_lazy_preserve_errors_and_fuse_exhaustion() {
    let codec = codec();
    let line = |body: &[u8]| TextLine::from_bytes(0, TextBytes::from_bytes(body).unwrap()).unwrap();
    let first = line(b"8=FIX.4.4|35=D|11=A|65024=stream|52=20260102-10:15:30|10=0|");
    let last = line(b"8=FIX.4.4|35=D|11=A|65024=stream|52=20260102-10:15:31|10=0|");
    let marker = Arc::new(());
    let mut items = [Ok(first), Err(source_failure(&marker)), Ok(last)].into_iter();
    let pulls = std::rc::Rc::new(std::cell::Cell::new(0));
    let count = std::rc::Rc::clone(&pulls);
    let source = std::iter::from_fn(move || {
        count.set(count.get() + 1);
        assert!(count.get() <= 4, "the exhausted source was pulled again");
        items.next()
    });
    let mut pipeline = codec.lifecycle(codec.parse_text_lines(source));
    drop(codec);
    // The parse is lazy; the walk is not, because a chain is read in instant
    // order and no order is known until the last message is in - so the walk
    // drains the source it was handed, once, and fuses it there.
    assert_eq!(pulls.get(), 4);
    let mut read = Vec::new();
    while let Some(held) = pipeline.next() {
        match held {
            // The source's own failure moves through as itself.
            Err(error) => same_source_failure(error, &marker),
            Ok(message) => read.push(message),
        }
    }
    assert_eq!(read.len(), 2);
    assert_eq!(read[1].get_prevuuid(), Some(read[0].get_curruuid()));
    assert_eq!(read[1].get_creatunix(), read[0].get_creatunix());
    assert!(pipeline.next().is_none());
    assert!(pipeline.next().is_none());
    assert_eq!(pulls.get(), 4);
}

/// A source that answers `item`, then `None` once, then resumes a bounded
/// number of times: a door that did not fuse would read the resumed items.
fn resuming<T: Clone>(item: T) -> impl Iterator<Item = T> {
    let mut pulls = 0;
    std::iter::from_fn(move || {
        pulls += 1;
        (pulls != 2 && pulls < 8).then(|| item.clone())
    })
}

#[test]
fn every_stream_door_fuses_its_own_source() {
    let codec = codec();
    let line = TextLine::from_bytes(
        0,
        TextBytes::from_bytes(b"8=FIX.4.4|35=D|11=A|10=0|").unwrap(),
    )
    .unwrap();
    let message = codec
        .parse_text_line(&line)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let mut parsed = codec.parse_text_lines(resuming(line));
    let mut lived = codec.lifecycle(resuming(message.clone()));
    for stream in [
        &mut parsed as &mut dyn Iterator<Item = yggdryl::Result<FixMsg>>,
        &mut lived,
    ] {
        assert!(stream.next().unwrap().is_ok());
        assert!(stream.next().is_none());
        assert!(stream.next().is_none());
    }
    let schema = fix_schema(codec.registry(), "fix").unwrap();
    let rows: usize = codec
        .arrow_reader(schema, resuming(message))
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(rows, 1);
}

#[test]
fn a_stream_carries_nothing_from_a_document_to_the_rows_after_it() {
    let codec = super::fixed_codec(Arc::new(FixRegistry::new())).with_capture_names(["pluginid"]);
    let line = |body: &[u8]| {
        TextLine::from_bytes(0, TextBytes::from_bytes(body).unwrap())
            .unwrap()
            .with_captures(vec![Some(TextBytes::from_bytes(b"STREAM").unwrap())])
            .unwrap()
    };
    // A Jolokia answer naming the plugin every line here names, and the two
    // ends of its session: a document, which the codec does not read.
    let document = line(br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=STREAM,plugin-type=FIX,type=Plugin","type":"read"},"value":{"Name":"STREAM","SenderCompID":"SOURCE","TargetCompID":"SINK"},"status":200}"#);
    let message = line(b"8=FIX.4.4|35=0|10=0|");
    let marker = Arc::new(());
    let mut stream =
        codec.parse_text_lines([Ok(document), Err(source_failure(&marker)), Ok(message)]);
    // The document is one `unknown` row carrying what its row stated - the
    // plugin capture - and nothing the document did.
    let unknown = stream.next().unwrap().unwrap();
    assert_eq!(unknown.as_field().name(), "unknown");
    assert!(unknown.entries().is_empty());
    assert_eq!(
        unknown
            .by_tag(yggdryl::PLUGINID_TAG_NAME.0)
            .unwrap()
            .as_str(),
        Some("STREAM")
    );
    assert!(unknown.get_by_tag(49).is_none_or(|held| held.is_null()));
    // The source error moves through, and the stream goes on past it.
    same_source_failure(stream.next().unwrap().unwrap_err(), &marker);
    // The heartbeat after it names the same plugin and gains nothing from
    // the document: the stream remembers no configuration, so the two ends
    // it stated fill no `SenderCompID` and no `TargetCompID` here.
    let filled = stream.next().unwrap().unwrap();
    assert_eq!(
        filled
            .by_tag(yggdryl::PLUGINID_TAG_NAME.0)
            .unwrap()
            .as_str(),
        Some("STREAM")
    );
    assert!(
        filled.get_by_tag(49).is_none_or(|held| held.is_null()),
        "{filled:?}"
    );
    assert!(
        filled.get_by_tag(56).is_none_or(|held| held.is_null()),
        "{filled:?}"
    );
    assert!(stream.next().is_none());
}

#[test]
fn the_batch_door_fills_what_a_parse_fills_and_leaves_the_record_alone() {
    const REPORT: &str = "8=FIX.4.4|35=8|39=1|150=F|38=100|14=40|32=40|31=10.5|54=1|10=0|";
    let codec = codec();

    // A parse fills what the message implies, so the one batch door states
    // the columns the message pass fills with the values it fills.
    let filled = codec
        .parse_text_arrow_reader(capture_reader(&[REPORT], 1))
        .unwrap()
        .map(std::result::Result::unwrap)
        .next()
        .expect("one batch");
    let message = codec
        .parse_lines([REPORT])
        .next()
        .expect("one message")
        .expect("a filled message");
    for tag in [151, 6, 381] {
        let held = message.get_by_tag(tag).unwrap_or(Scalar::Null);
        match yggdryl::fix_column_of(
            &yggdryl::Field::from_arrow_schema("row", &filled.schema()).unwrap(),
            tag,
        ) {
            Some(at) => assert_eq!(first_at(&filled, at), held, "tag {tag}"),
            // A derived tag the fixed row does not carry - `GrossTradeAmt(381)`
            // is one - is filled on the message and simply has no column to
            // appear in. The row is a projection of the message, not the whole
            // of it.
            None => assert_eq!(held, Scalar::from(420.0_f64), "tag {tag}"),
        }
    }
    assert_eq!(first_tag_value(&filled, 151), Scalar::from(60.0_f64));
    // One fill, so the average is that fill's price.
    assert_eq!(first_tag_value(&filled, 6), Scalar::from(10.5_f64));
    // The record is the message read as a tree, so the row's arrival record
    // is what the line read emits.
    assert_eq!(
        first_value(&filled, "fixentries")
            .as_sequence()
            .map(<[Scalar]>::len),
        Some(message.entries().len()),
    );
}

#[test]
fn messages_and_arrow_reader_invert_each_other() {
    let codec = codec().with_null_values::<[&str; 0], &str>([]);
    let schema = fix_schema(codec.registry(), "fix").unwrap();
    let parsed: Vec<FixMsg> = codec
        .parse_lines(CAPTURE)
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    // The stream is over messages, not lines: the five lines of the corpus
    // that state no message contribute none to it.
    assert_eq!(
        parsed.len(),
        9,
        "nine messages from {} lines",
        CAPTURE.len()
    );

    let reader = codec
        .arrow_reader(schema.clone(), parsed.clone().into_iter().map(Ok))
        .unwrap();
    let again: Vec<FixMsg> = codec
        .messages(reader)
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(again.len(), parsed.len());
    for (held, message) in again.iter().zip(&parsed) {
        // The same message: the same arrival record, the same wire, the same
        // digest, the same stated values by tag - a row read back holds every
        // column as a child, null where the parsed message had none - and the
        // same row again.
        assert_eq!(held.entries(), message.entries());
        assert_eq!(held.into_bytes(b'|'), message.into_bytes(b'|'));
        assert_eq!(held.digest(), message.digest());
        for tag in [35, 11, 55, 54, 17, 37] {
            fn stated(message: &FixMsg, tag: i32) -> Option<Scalar> {
                message.get_by_tag(tag).filter(|held| !held.is_null())
            }
            assert_eq!(stated(held, tag), stated(message, tag), "tag {tag}");
        }
        assert_eq!(
            held.into_row(&schema).unwrap(),
            message.into_row(&schema).unwrap()
        );
    }

    // And the batches the second pass makes are the batches the first made.
    let first = batches(
        codec
            .arrow_reader(schema.clone(), parsed.into_iter().map(Ok))
            .unwrap(),
    );
    let second = batches(
        codec
            .arrow_reader(schema, again.into_iter().map(Ok))
            .unwrap(),
    );
    assert_eq!(first, second);
}

#[test]
fn byte_in_byte_out_over_the_whole_corpus() {
    // The convention that drops a stated absence is deliberately not
    // byte-preserving, so it is turned off to measure the reader rather than
    // the convention.
    let codec = codec()
        .with_null_values::<[&str; 0], &str>([])
        .with_separator(b'|');
    let reader = codec.parse_text_arrow_reader(source()).unwrap();

    let mut written = Vec::new();
    let count = codec.write_arrow_reader(reader, &mut written).unwrap();

    // One written line per *message*, not per source line: the five lines of
    // the corpus that open no frame, state no bridge pair and carry no
    // document are silent, so nine of the fourteen come back.
    // The line each message came from, paired with the pairs it should be
    // written as, so the written lines zip against the lines that carried a
    // message rather than against every line.
    let plain = super::fixed_codec(registry()).with_null_values::<[&str; 0], &str>([]);
    let expected: Vec<(&str, String)> = CAPTURE
        .iter()
        .flat_map(|source| {
            plain
                .parse_line(source.as_bytes())
                .unwrap()
                .map(|read| {
                    let bytes = read.unwrap().into_bytes(b'|');
                    (*source, String::from_utf8(bytes).unwrap())
                })
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(expected.len(), 9, "nine of the fourteen lines are messages");
    assert_eq!(count as usize, expected.len());

    // Every message comes back as the pairs it arrived with, in arrival order.
    let back: Vec<&str> = std::str::from_utf8(&written).unwrap().lines().collect();
    assert_eq!(back.len(), expected.len());
    for (line, (source, pairs)) in back.iter().zip(&expected) {
        assert_eq!(line, pairs, "{source}");
    }
}

#[test]
fn a_batch_with_no_arrival_record_cannot_be_written() {
    let codec = codec();
    let batch = codec
        .parse_text_arrow_reader(source())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    // Project the facets alone, which is exactly what a caller may do - and
    // then the wire is no longer reconstructible, which has to be said rather
    // than guessed at.
    let facets = batch
        .project(&[super::tag_index(&batch, 55), super::tag_index(&batch, 54)])
        .unwrap();
    let projected = yggdryl::arrow::batch_reader(facets.schema(), [facets]);
    let mut sink = Vec::new();
    let refused = codec.write_arrow_reader(projected, &mut sink).unwrap_err();
    assert!(
        matches!(refused, yggdryl::Error::InvalidRecord { .. }),
        "{refused}"
    );
    assert!(sink.is_empty(), "refused before a row was read");
}

#[test]
fn a_line_is_read_by_its_captures_and_a_bare_body_reads_as_the_byte_reader_does() {
    let codec = codec().with_capture_names(["beginstring", "pluginid"]);
    let page = |bytes: &[u8]| TextBytes::from_bytes(bytes).unwrap();
    // A line and the captures its header declared, in that order; `None` is a
    // capture the header stated and this line did not match.
    let line = |body: &[u8], captures: [Option<&str>; 2]| {
        TextLine::from_bytes(0, page(body))
            .unwrap()
            .with_captures(
                captures
                    .iter()
                    .map(|held| held.map(|text| page(text.as_bytes())))
                    .collect(),
            )
            .unwrap()
    };
    let one = |line: &TextLine| -> FixMsg {
        let mut messages = codec.parse_text_line(line).unwrap();
        let message = messages.next().unwrap().unwrap();
        assert!(messages.next().is_none(), "one message");
        message
    };

    // Only a body: exactly what the byte reader does.
    let message = one(&line(b"8=FIX.4.4|35=D|11=ORDER-1|10=0|", [None, None]));
    assert_eq!(message.as_field().name(), "D");
    assert_eq!(message.by_tag(11).unwrap().as_str(), Some("ORDER-1"));

    // A capture speaks per row and outranks the codec, which speaks per
    // stream. What a version settles is how a value is read - which dated
    // code spelling answers - and never what a field is called: tag 32 is the
    // dictionary's own column whatever version the row states.
    let old = one(&line(
        b"8=FIX.4.4|35=8|32=100|10=0|",
        [Some("FIX.4.2"), None],
    ));
    assert!(old.as_field().index_of("lastqty").is_some());
    assert!(
        old.get_by_name("lastshares").is_some(),
        "the 4.2 spelling reaches it"
    );

    // A capture absent, unmatched or empty is silence, never an instruction
    // and never an error.
    let quiet = one(&line(b"8=FIX.4.4|35=D|11=A|10=0|", [None, Some("")]));
    assert_eq!(quiet.as_field().name(), "D");
    assert_eq!(
        quiet.as_field().as_fix().branches().count(),
        0,
        "a plugin names no dialect: a message is not a dictionary member"
    );

    // A bulk document is one `unknown` message, whatever it holds, and the
    // stream door yields that one.
    let read: Vec<_> = super::fixed_codec(direction_registry())
        .parse_text_lines([TextLine::from_bytes(0, page(BULK_CONFIG)).unwrap()])
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].as_field().name(), "unknown");
    assert!(read[0].entries().is_empty());
}

/// The committed dictionary beside a venue's one field of its own on tag
/// 5001, a member of the `venue` dictionary, so a bridge row's `VENUETAG`
/// resolves in the one namespace whatever plugin the row names.
fn plugin_registry() -> Arc<FixRegistry> {
    let mut registry = registry().as_ref().clone();
    let mut field = DataType::utf8().nullable_field("VenueTag");
    field.as_fix_mut().set_tag(5001).unwrap();
    field.as_fix_mut().set_branches(["venue"]).unwrap();
    registry.insert(field).unwrap();
    Arc::new(registry)
}

/// The capture a bridge row header declares: the plugin that logged the line.
const PLUGIN_CAPTURES: [&str; 1] = ["pluginid"];

/// A bridge line naming the plugin that logged it, where it names one.
fn plugin_line(body: &[u8], plugin: Option<&str>) -> TextLine {
    let page = |text: &str| TextBytes::from_bytes(text.as_bytes()).unwrap();
    TextLine::from_bytes(0, TextBytes::from_bytes(body).unwrap())
        .unwrap()
        .with_captures(vec![plugin.map(page)])
        .unwrap()
}

/// The one message one line reads as.
fn one_of(codec: &FixCodec, line: &TextLine) -> FixMsg {
    let mut messages = codec.parse_text_line(line).unwrap();
    let message = messages.next().unwrap().unwrap();
    assert!(messages.next().is_none(), "one message");
    message
}

#[test]
fn a_rows_pluginid_fills_its_own_column_and_selects_no_dialect() {
    let registry = plugin_registry();
    let codec = super::fixed_codec(Arc::clone(&registry)).with_capture_names(PLUGIN_CAPTURES);
    let body: &[u8] = b"MSGTYPE=D|CLORDID=A|VENUETAG=dark";

    // Membership is provenance on the dictionary's field, never a namespace
    // the row is read under: the venue's field says which dictionary spoke
    // it, and the message root says nothing, because a message is not a
    // dictionary member.
    let venue = registry.field_by_tag(5001).unwrap().as_fix();
    assert!(venue.has_branch("venue"));
    assert_eq!(venue.branches().collect::<Vec<_>>(), ["venue"]);

    // Every way a row can name its plugin - the dictionary's own name, an
    // alias in another case, a plugin no dictionary is named after, and a
    // name too long to be one - reads the same: the venue's field resolves
    // in the one namespace, and the column fills the crate's own field
    // exactly as it was spelled.
    let long = "x".repeat(300);
    for spelled in ["venue", "VNU", "OMS_X1_TradeCapture", long.as_str()] {
        let message = one_of(&codec, &plugin_line(body, Some(spelled)));
        assert_eq!(
            message.as_field().as_fix().branches().count(),
            0,
            "{spelled}: a message root carries no membership"
        );
        assert_eq!(
            message.by_tag(5001).unwrap().as_str(),
            Some("dark"),
            "{spelled}: the venue's own field, in the one namespace"
        );
        assert_eq!(
            message.by_name("venuetag").unwrap().as_str(),
            Some("dark"),
            "{spelled}: and under its own spelling"
        );
        assert_eq!(
            message
                .by_tag(yggdryl::PLUGINID_TAG_NAME.0)
                .unwrap()
                .as_str(),
            Some(spelled),
            "{spelled}: the fill is the text as spelled"
        );
        assert!(
            message
                .entries()
                .iter()
                .all(|entry| entry.tag() != yggdryl::PLUGINID_TAG_NAME.0),
            "a fill is never an entry"
        );
    }

    // A plugin unstated fills nothing, and the row still reads the same.
    let message = one_of(&codec, &plugin_line(body, None));
    assert!(
        message
            .get_by_tag(yggdryl::PLUGINID_TAG_NAME.0)
            .is_none_or(|held| held.is_null()),
        "nothing to fill from"
    );
    assert_eq!(message.by_tag(5001).unwrap().as_str(), Some("dark"));
}

#[test]
fn the_batch_reader_and_the_line_reader_agree_on_a_rows_plugin() {
    let registry = plugin_registry();
    let codec = super::fixed_codec(registry).with_capture_names(PLUGIN_CAPTURES);
    // Every way a row can name its plugin: a dictionary's name, an alias of
    // it, a plugin no dictionary is named after, nothing, and an empty string.
    let rows: [(&[u8], Option<&str>); 5] = [
        (b"MSGTYPE=D|CLORDID=A|VENUETAG=dark", Some("venue")),
        (b"MSGTYPE=D|CLORDID=B|VENUETAG=lit", Some("VNU")),
        (
            b"MSGTYPE=D|CLORDID=C|VENUETAG=none",
            Some("OMS_X1_TradeCapture"),
        ),
        (b"MSGTYPE=D|CLORDID=D|VENUETAG=none", None),
        (b"MSGTYPE=D|CLORDID=E|VENUETAG=none", Some("")),
    ];
    let text = |held: Option<&str>| held.map_or(Scalar::Null, Scalar::from);
    let lines: Vec<TextLine> = rows
        .iter()
        .map(|(body, plugin)| plugin_line(body, *plugin))
        .collect();
    let capture = DataType::from_fields([
        DataType::binary().required_field("body"),
        DataType::utf8().nullable_field("pluginid"),
    ])
    .unwrap()
    .required_field("capture");
    let values = Scalar::from_sequence(
        rows.iter()
            .map(|(body, plugin)| {
                Scalar::from_sequence([Scalar::from(body.to_vec()), text(*plugin)])
            })
            .collect::<Vec<_>>(),
    );
    let batch = yggdryl::arrow::batch_from_value(&capture, &values).unwrap();
    let read = batches(
        codec
            .parse_text_arrow_reader(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap(),
    );
    assert_eq!(row_count(&read), rows.len());
    let again: Vec<FixMsg> = codec
        .messages(yggdryl::arrow::batch_reader(read[0].schema(), read.clone()))
        .collect::<yggdryl::Result<_>>()
        .unwrap();

    // Row for row, the batch read is the line read: the plugin lands in its
    // column, and the arrival record carries the tag each key resolved to -
    // 5001 on every row, because the venue's field is in the one namespace
    // whatever plugin the row names.
    for (row, line) in lines.iter().enumerate() {
        let alone = one_of(&codec, line);
        let streamed = &again[row];
        let tag = yggdryl::PLUGINID_TAG_NAME.0;
        assert_eq!(
            alone.get_by_tag(tag).filter(|held| !held.is_null()),
            streamed.get_by_tag(tag).filter(|held| !held.is_null()),
            "row {row} tag {tag}"
        );
        assert_eq!(alone.entries(), streamed.entries(), "row {row}");
        assert!(
            alone.entries().iter().any(|entry| entry.tag() == 5001),
            "row {row} resolves the venue's field whatever plugin it names"
        );
    }
    assert_eq!(
        again[1]
            .by_tag(yggdryl::PLUGINID_TAG_NAME.0)
            .unwrap()
            .as_str(),
        Some("VNU")
    );
}

#[test]
fn a_capture_already_in_arrow_feeds_the_same_builders() {
    let codec = codec();
    let first = codec
        .parse_text_arrow_reader(capture_reader(
            &[
                "8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|10=0|",
                "8=FIX.4.4|35=D|11=ORDER-2|55=MSFT|10=0|",
            ],
            2,
        ))
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(first.num_rows(), 2);

    // A settled FIX row replays through messages without parsing its body.
    let schema = first.schema();
    assert_eq!(
        codec
            .messages(yggdryl::arrow::batch_reader(
                schema.clone(),
                [first.clone()]
            ))
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap()
            .len(),
        2
    );
    // Re-parsing the body under the projected row's UUID asserts a different
    // named content shape and must refuse instead of silently replacing it.
    let mut again = codec
        .clone()
        .parse_text_arrow_reader(yggdryl::arrow::batch_reader(
            schema.clone(),
            [first.clone()],
        ))
        .unwrap();
    // The identity is settled by the read rather than carried in from the
    // row, so the same body under the projected row answers its rows again.
    assert_eq!(
        again
            .by_ref()
            .map(|batch| batch.expect("a batch").num_rows())
            .sum::<usize>(),
        2
    );
    assert!(again.next().is_none());
    // An explicit body-only projection is a fresh capture, with no stale claim.
    let bodies = first.project(&[schema.index_of("body").unwrap()]).unwrap();
    let again = codec
        .parse_text_arrow_reader(yggdryl::arrow::batch_reader(bodies.schema(), [bodies]))
        .unwrap();
    let rows: usize = again.map(|batch| batch.unwrap().num_rows()).sum();
    assert_eq!(rows, 2, "the frames the batch carried, read again");

    // Named at a column that is not a payload, the same source answers
    // nothing: `msgtype` holds `D`, which opens no frame, states no bridge
    // pair and carries no document, so neither row is a row.
    let typed = codec
        .clone()
        .with_payload_column("msgtype")
        .parse_text_arrow_reader(yggdryl::arrow::batch_reader(
            schema.clone(),
            [first.clone()],
        ))
        .unwrap();
    let rows: usize = typed.map(|batch| batch.unwrap().num_rows()).sum();
    assert_eq!(rows, 0, "a payload of `D` carries no message");

    // The entries column is a list, not a payload: naming it is refused
    // before a row is read rather than answered as rows of nothing.
    let refused = codec
        .with_payload_column("fixentries")
        .parse_text_arrow_reader(yggdryl::arrow::batch_reader(schema, [first]))
        .map(drop)
        .unwrap_err();
    assert!(
        matches!(refused, yggdryl::Error::InvalidRecord { .. }),
        "{refused}"
    );
}

#[test]
fn the_captures_own_columns_lead_the_row_and_a_clash_yields_to_fix() {
    // Shaped the way the text line reader shapes a capture: where the line was
    // read from, which line it was, what stamped it, and the frame itself.
    let capture = DataType::from_fields([
        DataType::utf8().required_field("url"),
        DataType::Int64.required_field("rownum"),
        DataType::utf8().nullable_field("threadname"),
        DataType::binary().required_field("body"),
        // A name a FIX column already takes, which yields to it: one column
        // per name, and the FIX one is what a reader spelling it means.
        DataType::utf8().nullable_field("fixentries"),
    ])
    .expect("a capture root")
    .required_field("line");

    let rows = Scalar::from_sequence([Scalar::from_sequence([
        Scalar::from("file:///capture.log"),
        Scalar::from(7_i64),
        Scalar::from("session-a"),
        Scalar::from(b"8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|10=0|".to_vec()),
        Scalar::from("ignored"),
    ])]);
    let batch = yggdryl::arrow::batch_from_value(&capture, &rows).expect("a capture batch");
    let schema = batch.schema();
    let source = yggdryl::arrow::batch_reader(schema, [batch]);

    let read = codec()
        .parse_text_arrow_reader(source)
        .expect("a carried reader");
    let columns: Vec<String> = read
        .schema()
        .fields()
        .iter()
        .map(|held| held.name().clone())
        .collect();
    assert_eq!(
        &columns[..4],
        ["url", "rownum", "threadname", "body"],
        "the capture leads the row"
    );
    assert_eq!(
        columns.iter().filter(|held| *held == "fixentries").count(),
        1,
        "the clashing capture column yielded to the FIX one"
    );
    assert!(
        columns.contains(&"msgtype".to_owned()),
        "the fixed columns follow it"
    );

    let read = batches(read);
    assert_eq!(row_count(&read), 1);
    let held = yggdryl::arrow::batch_to_value(&read[0]).expect("a value");
    let row = held.as_sequence().expect("one row")[0]
        .as_sequence()
        .expect("its columns")
        .to_vec();
    assert_eq!(row[0].as_str(), Some("file:///capture.log"));
    assert_eq!(row[1].as_i64(), Some(7));
    assert_eq!(row[2].as_str(), Some("session-a"));
    let at = columns
        .iter()
        .position(|held| held == "msgtype")
        .expect("the msgtype column");
    assert_eq!(row[at].as_str(), Some("D"), "and FIX filled its own");

    // Read back as messages, the carried columns are children of their own
    // names, and the same schema takes the row back whole.
    let codec = codec();
    let field = yggdryl::Field::from_arrow_schema("fix", &read[0].schema()).unwrap();
    let message = codec
        .messages(yggdryl::arrow::batch_reader(read[0].schema(), read.clone()))
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(message.by_name("rownum").unwrap(), Scalar::from(7_i64));
    assert_eq!(message.by_tag(55).unwrap().as_str(), Some("AAPL"));
    let again = batches(codec.arrow_reader(field, [Ok(message)]).unwrap());
    assert_eq!(again, read);
}

#[test]
fn dedup_composes_over_the_messages_a_batch_holds() {
    let codec = codec();
    let published = [
        "8=FIX.4.4|35=D|11=A|10=0|",
        "8=FIX.4.4|35=D|11=A|10=0|",
        "8=FIX.4.4|35=D|11=B|10=0|",
    ];
    let reader = codec
        .parse_text_arrow_reader(capture_reader(&published, 3))
        .unwrap();
    // Every row is a row, because with a filter a batch no longer aligns with
    // its input by position.
    let schema = yggdryl::Field::from_arrow_schema("fix", &reader.schema()).unwrap();
    let messages: Vec<FixMsg> = codec
        .messages(reader)
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(messages.len(), 3);

    // The stage is a call: the dedup adapter over the messages, then batches.
    let mut dedup = FixDedup::new(messages.into_iter());
    let kept = batches(
        codec
            .arrow_reader(schema, dedup.by_ref().map(Ok).collect::<Vec<_>>())
            .unwrap(),
    );
    assert_eq!(row_count(&kept), 2, "the adjacent republication went");
    assert_eq!(dedup.dropped(), 1);
}

#[test]
fn a_pluginid_capture_with_no_field_to_fill_is_silence() {
    // A `pluginid` capture fills the crate's own field and selects nothing:
    // a registry holding no `pluginid` field of its own still reads its rows
    // the same, in the one namespace, with nothing filled and no error.
    let registry = plugin_registry();
    let mut without = registry.as_ref().clone();
    assert!(without.remove(yggdryl::PLUGINID_TAG_NAME.0).is_some());
    let codec = super::fixed_codec(Arc::new(without));
    let body: &[u8] = b"MSGTYPE=D|CLORDID=A|VENUETAG=dark";

    let alone = one_of(
        &codec.clone().with_capture_names(PLUGIN_CAPTURES),
        &plugin_line(body, Some("VNU")),
    );
    assert_eq!(alone.as_field().as_fix().branches().count(), 0);
    assert_eq!(alone.by_tag(5001).unwrap().as_str(), Some("dark"));
    assert!(
        alone.get_by_tag(yggdryl::PLUGINID_TAG_NAME.0).is_none(),
        "no field of that name is there to fill"
    );

    let capture = DataType::from_fields([
        DataType::binary().required_field("body"),
        DataType::utf8().nullable_field("pluginid"),
    ])
    .unwrap()
    .required_field("capture");
    let values = Scalar::from_sequence([Scalar::from_sequence([
        Scalar::from(body.to_vec()),
        Scalar::from("VNU"),
    ])]);
    let batch = yggdryl::arrow::batch_from_value(&capture, &values).unwrap();
    let read = batches(
        codec
            .parse_text_arrow_reader(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap(),
    );
    assert_eq!(row_count(&read), 1);
    let streamed: Vec<FixMsg> = codec
        .messages(yggdryl::arrow::batch_reader(read[0].schema(), read.clone()))
        .collect::<yggdryl::Result<_>>()
        .unwrap();

    // The two readers read one row one way. A row read back off a batch
    // carries the batch's own root, and its arrival record resolved to the
    // venue's own tag in the one namespace.
    assert_eq!(streamed.len(), 1);
    assert_eq!(streamed[0].entries(), alone.entries());
    assert!(
        streamed[0]
            .entries()
            .iter()
            .any(|entry| entry.tag() == 5001),
        "the venue's field resolved"
    );
}

#[test]
fn a_payload_column_spelled_pluginid_is_the_payload_and_fills_no_plugin() {
    // The column [`FixCodec::with_payload_column`] names is the payload and
    // nothing else, whatever it is spelled: its text never fills the crate's
    // own `pluginid` field, or a capture whose payload column happened to be
    // spelled so would stamp every line with whatever its first bytes were.
    let registry = plugin_registry();
    let codec = super::fixed_codec(registry).with_payload_column("pluginid");
    // One payload spelling a dictionary's name exactly, and one that is a
    // frame, so the column is proven to be read as the payload.
    let bodies = ["venue", "8=FIX.4.4|35=D|11=A|10=0|"];

    // The line door has no payload column to spell at all: a line's body is a
    // typed field, so the same two bodies read as bodies there and nothing
    // about them can reach the plugin.
    let lines: Vec<TextLine> = bodies
        .iter()
        .map(|body| {
            TextLine::from_bytes(0, TextBytes::from_bytes(body.as_bytes()).unwrap()).unwrap()
        })
        .collect();
    // Read as the payload it is, `venue` is one word: it opens no frame,
    // states no bridge pair and carries no document, so it states no message
    // at all. It used to answer an empty `unknown`; that it now
    // answers nothing is the same proof, that the column was read as a
    // payload and never as the plugin its spelling names.
    assert!(
        codec.parse_text_line(&lines[0]).unwrap().next().is_none(),
        "a payload of one word carries no message"
    );
    let alone = one_of(&codec, &lines[1]);

    let capture = DataType::from_fields([DataType::utf8().required_field("pluginid")])
        .unwrap()
        .required_field("capture");
    let values = Scalar::from_sequence(
        bodies
            .iter()
            .map(|body| Scalar::from_sequence([Scalar::from(*body)]))
            .collect::<Vec<_>>(),
    );
    let batch = yggdryl::arrow::batch_from_value(&capture, &values).unwrap();
    let read = batches(
        codec
            .parse_text_arrow_reader(yggdryl::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap(),
    );
    // The batch door reads the column the same way: one row for the frame,
    // and none for the word, which is a payload carrying no message.
    assert_eq!(row_count(&read), 1);
    let streamed: Vec<FixMsg> = codec
        .messages(yggdryl::arrow::batch_reader(read[0].schema(), read.clone()))
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(streamed.len(), 1);

    for message in [&alone, &streamed[0]] {
        assert_eq!(message.as_field().as_fix().branches().count(), 0);
        assert!(
            message
                .get_by_tag(yggdryl::PLUGINID_TAG_NAME.0)
                .is_none_or(|held| held.is_null()),
            "the payload never fills the plugin"
        );
        assert!(message.get_by_tag(5001).is_none());
    }
    assert_eq!(alone.entries(), streamed[0].entries());
    // The frame was read as the body; the dictionary's name held nothing to read.
    assert_eq!(alone.by_tag(11).unwrap().as_str(), Some("A"));
}
