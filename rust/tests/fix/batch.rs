//! A capture in, columns out, and back to the wire - through the codec's
//! Arrow twins, and the two converters they compose.

use std::sync::Arc;

use arrow_array::RecordBatch;
use yggdryl::arrow::BatchReader;
use yggdryl::{DataType, FixBranch, FixCodec, FixDedup, FixMsg, FixRegistry, Scalar, fix_schema};

fn registry() -> Arc<FixRegistry> {
    super::committed_registry()
}

fn codec() -> FixCodec {
    FixCodec::new(registry())
}

const BULK_CONFIG: &[u8] = br#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:name=*,plugin-type=FIX,type=Plugin","type":"read"},"value":{"com.ullink.ulbridge.sessioninterfaces.plugins:name=A,plugin-type=FIX,type=Plugin":{"Name":"A"},"com.ullink.ulbridge.sessioninterfaces.plugins:name=B,plugin-type=FIX,type=Plugin":{"Name":"B"}},"status":200}"#;

fn config_registry() -> Arc<FixRegistry> {
    let mut registry = FixRegistry::new().with_ulbridge_fields().unwrap();
    let mut direction = DataType::MsgDirection.nullable_field("MsgDirection");
    direction.as_fix_mut().set_tag(385).unwrap();
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
    let field = DataType::from_fields([DataType::Binary.required_field("body")])
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
fn bulk_configuration_lines_expand_without_pulling_the_next_line() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let pulled = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&pulled);
    let lines = [BULK_CONFIG.to_vec(), Vec::new()]
        .into_iter()
        .inspect(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
        });
    let codec = FixCodec::new(config_registry());
    let mut messages = codec.parse_lines(lines);
    assert_eq!(pulled.load(Ordering::SeqCst), 0);
    assert!(messages.next().unwrap().is_ok());
    assert_eq!(pulled.load(Ordering::SeqCst), 1);
    assert!(messages.next().unwrap().is_ok(), "the second MBean");
    assert_eq!(pulled.load(Ordering::SeqCst), 1);
    // An empty line is not a row at all: an `Err` item, and the stream goes on.
    assert!(messages.next().unwrap().is_err());
    assert_eq!(pulled.load(Ordering::SeqCst), 2);
    assert!(messages.next().is_none());
}

#[test]
fn expanded_configurations_repeat_the_source_columns_and_stated_direction() {
    let field = DataType::from_fields([
        DataType::Int64.required_field("rownum"),
        DataType::Utf8.required_field("direction"),
        DataType::Binary.required_field("body"),
    ])
    .unwrap()
    .required_field("capture");
    let rows = Scalar::from_sequence([Scalar::from_sequence([
        Scalar::from(42_i64),
        Scalar::from("RECV"),
        Scalar::from(BULK_CONFIG.to_vec()),
    ])]);
    let source = yggdryl::arrow::batch_from_value(&field, &rows).unwrap();
    // One byte a batch: a batch a message, so each expanded row is seen alone.
    let reader = FixCodec::new(config_registry())
        .with_batch_byte_size(1)
        .parse_text_arrow_reader(yggdryl::arrow::batch_reader(source.schema(), [source]))
        .unwrap();
    let batches = batches(reader);
    assert_eq!(batches.len(), 2);
    for batch in batches {
        assert_eq!(batch.num_rows(), 1);
        assert_eq!(first_value(&batch, "rownum"), Scalar::from(42_i64));
        assert_eq!(first_tag_value(&batch, 385).as_str(), Some("RECV"));
    }
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

    // The capture's own column leads; the fixed columns follow, named by
    // their folded names, each carrying its tag on the field - which is what
    // the row is filled by.
    assert_eq!(
        &names[..6],
        [
            "body",
            "beginstring",
            "bodylength",
            "msgtype",
            "sendercompid",
            "targetcompid"
        ],
        "{names:?}"
    );
    assert_eq!(
        &names[names.len() - 2..],
        ["nofixentries", "nounmappedfixentries"],
    );
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
        yggdryl::MSGHASH_TAG,
        yggdryl::TIMESTAMP_TAG,
        yggdryl::UNIXPARTITION_TAG, // the digest, the clock, the partition
        yggdryl::SENDERSESSIONID_TAG,
        yggdryl::MSGCTXID_TAG,     // what a bridge's own log states
        yggdryl::MSGDIRECTION_TAG, // which way the line moved
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
fn a_row_in_is_a_row_out() {
    let batches = batches(codec().parse_text_arrow_reader(source()).unwrap());
    assert_eq!(
        row_count(&batches),
        CAPTURE.len(),
        "every line, including the ones that are not messages at all",
    );

    // The rows that were not FIX are still rows, named `unknown`.
    let first = &batches[0];
    let msgtype = tag_column(first, 35);
    assert!(msgtype.is_valid(0), "a framed row states its type");
    assert!(
        !msgtype.is_valid(CAPTURE.len() - 1),
        "a sentence states no type"
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
    // The digest is sixteen bytes, not a string.
    let digest = tag_column(&batch, yggdryl::MSGHASH_TAG);
    assert_eq!(
        digest.data_type(),
        &arrow_schema::DataType::FixedSizeBinary(16)
    );
    assert!(digest.is_valid(0));
    // And the arrival record is there in full, which is what makes the batch
    // lossless rather than one reader's summary.
    let entries = column(&batch, "nofixentries");
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
    let bounded = codec.clone().with_batch_byte_size(10 * 450);
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
        .filter(|at| batch.schema().field(*at).name() != yggdryl::fix::ENTRIES_COLUMN)
        .collect();
    let projected = batch.project(&lifted).unwrap();
    assert_eq!(projected.num_rows(), 200);
    let reader = || yggdryl::arrow::batch_reader(projected.schema(), [projected.clone()]);

    // Under a bound of about ten rows of leaves, the stream is cut into
    // batches of about ten - it is not one batch of everything, which is
    // what charging a message with no wire the bare row width would make.
    let bounded = codec.clone().with_batch_byte_size(10 * 450);
    let many = batches(bounded.enrich_messages_arrow_reader(reader()).unwrap());
    assert!((5..60).contains(&many.len()), "{} batches", many.len());
    assert_eq!(row_count(&many), 200);

    // A target of one byte is still a batch a message.
    let each = batches(
        codec
            .clone()
            .with_batch_byte_size(1)
            .enrich_messages_arrow_reader(reader())
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
    let named_line = DataType::from_fields([DataType::Binary.required_field("line")])
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

    // The record door refuses the same two records the same way, and reads a
    // null payload as a row holding an empty message.
    let record = |name: &str, value: Scalar| Scalar::from_record([(name, value)]).unwrap();
    let message = refusal(
        codec
            .parse_text_record(&record("line", frame.clone()))
            .map(drop)
            .unwrap_err(),
    );
    assert!(
        message.contains("body") && message.contains("line"),
        "{message}"
    );
    let message = refusal(
        codec
            .parse_text_record(&record("body", Scalar::from(7_i64)))
            .map(drop)
            .unwrap_err(),
    );
    assert!(message.contains("int64"), "{message}");
    let mut silent = codec
        .parse_text_record(&record("body", Scalar::Null))
        .unwrap();
    let empty = silent.next().unwrap().unwrap();
    assert!(silent.next().is_none());
    assert_eq!(empty.as_field().name(), "unknown");
    assert!(empty.entries().is_empty());
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

    // The capture door answers every line as a row, the refused one empty.
    let rows = batches(
        codec
            .parse_text_arrow_reader(capture_reader(&lines, lines.len()))
            .unwrap(),
    );
    assert_eq!(row_count(&rows), 3);
}

#[test]
fn the_filling_reader_fills_what_the_filling_pass_fills_and_leaves_the_record_alone() {
    const REPORT: &str = "8=FIX.4.4|35=8|39=1|150=F|38=100|14=40|32=40|31=10.5|54=1|10=0|";
    let codec = codec();

    // The reader that does not fill leaves the implied columns null.
    let bare = codec
        .parse_text_arrow_reader(capture_reader(&[REPORT], 1))
        .unwrap()
        .map(std::result::Result::unwrap)
        .next()
        .expect("one batch");
    assert_eq!(first_tag_value(&bare, 151), Scalar::Null);

    // The same rows through the filling reader state what the message
    // implied - the columns the message pass fills, with the values it fills.
    let filled = codec
        .enrich_messages_arrow_reader(
            codec
                .parse_text_arrow_reader(capture_reader(&[REPORT], 1))
                .unwrap(),
        )
        .unwrap()
        .map(std::result::Result::unwrap)
        .next()
        .expect("one batch");
    let message = codec
        .enrich_messages(codec.parse_lines([REPORT]).map(Result::unwrap))
        .next()
        .expect("one message")
        .expect("a filled message");
    for tag in [151, 6, 381] {
        let held = message.get_by_tag(tag).cloned().unwrap_or(Scalar::Null);
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
    // The schema is the same schema: the carried column still leads.
    assert_eq!(filled.schema(), bare.schema());
    // The arrival record is untouched either way, so the wire re-emits the
    // same bytes whether the row was filled or not.
    assert_eq!(
        first_value(&bare, "nofixentries"),
        first_value(&filled, "nofixentries"),
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
            fn stated(message: &FixMsg, tag: i32) -> Option<&Scalar> {
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
    assert_eq!(count as usize, CAPTURE.len());

    // Every line comes back as the pairs it arrived with, in arrival order.
    let back: Vec<&str> = std::str::from_utf8(&written).unwrap().lines().collect();
    assert_eq!(back.len(), CAPTURE.len());

    let plain = FixCodec::new(registry()).with_null_values::<[&str; 0], &str>([]);
    for (line, source) in back.iter().zip(CAPTURE) {
        let read = plain
            .parse_line(source.as_bytes())
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let expected = String::from_utf8(read.into_bytes(b'|')).unwrap();
        assert_eq!(*line, expected, "{source}");
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
fn a_record_is_read_by_its_columns_and_a_bare_payload_reads_as_the_byte_reader_does() {
    let codec = codec();
    let one = |record: &Scalar| -> FixMsg {
        let mut messages = codec.parse_text_record(record).unwrap();
        let message = messages.next().unwrap().unwrap();
        assert!(messages.next().is_none(), "one message");
        message
    };

    // Only a payload: exactly what the byte reader does.
    let bare = Scalar::from_record([(
        "body",
        Scalar::from(b"8=FIX.4.4|35=D|11=ORDER-1|10=0|".to_vec()),
    )])
    .unwrap();
    let message = one(&bare);
    assert_eq!(message.as_field().name(), "D");
    assert_eq!(message.by_tag(11).unwrap().as_str(), Some("ORDER-1"));

    // A column speaks per row and outranks the codec, which speaks per
    // stream. What a version settles is how a value is read - which dated
    // code spelling answers - and never what a field is called: tag 32 is the
    // dictionary's own column whatever version the row states.
    let dated = Scalar::from_record([
        (
            "body",
            Scalar::from(b"8=FIX.4.4|35=8|32=100|10=0|".to_vec()),
        ),
        ("beginstring", Scalar::from("FIX.4.2")),
    ])
    .unwrap();
    let old = one(&dated);
    assert!(old.as_field().index_of("lastqty").is_some());
    assert!(
        old.get_by_name("lastshares").is_some(),
        "the 4.2 spelling reaches it"
    );

    // A column absent, null or empty is silence, never an instruction and
    // never an error.
    let silent = Scalar::from_record([
        ("body", Scalar::from(b"8=FIX.4.4|35=D|11=A|10=0|".to_vec())),
        ("beginstring", Scalar::Null),
        ("pluginid", Scalar::from("")),
    ])
    .unwrap();
    let quiet = one(&silent);
    assert_eq!(quiet.as_field().name(), "D");
    assert!(
        quiet.branch().is_standard(),
        "an empty plugin names no dialect"
    );

    // A bulk document is many messages, and the stream door yields each.
    let bulk = Scalar::from_record([("body", Scalar::from(BULK_CONFIG.to_vec()))]).unwrap();
    let read: Vec<_> = FixCodec::new(config_registry())
        .parse_text_records([bulk])
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(read.len(), 2);
}

/// The committed dictionary beside a dialect declared under a plugin's name
/// and an alias, with one field of its own on user tag 5001, so a bridge
/// row's `VENUETAG` resolves under that dialect and under nothing else.
fn plugin_registry() -> (Arc<FixRegistry>, FixBranch) {
    let venue = FixBranch::from_str("venue")
        .unwrap()
        .with_aliases(["vnu"])
        .unwrap();
    let mut registry = registry().as_ref().clone();
    registry.set_branch(venue.clone()).unwrap();
    let mut field = DataType::Utf8.nullable_field("VenueTag");
    field.as_fix_mut().set_id(&venue, 5001).unwrap();
    registry.insert(field).unwrap();
    (Arc::new(registry), venue)
}

/// A bridge row naming its plugin, and the plugin the message came through
/// before it where the row states one.
fn plugin_record(body: &[u8], plugin: Scalar, previous: Scalar) -> Scalar {
    Scalar::from_record([
        ("body", Scalar::from(body.to_vec())),
        ("pluginid", plugin),
        ("prevpluginid", previous),
    ])
    .unwrap()
}

/// The one message one record reads as.
fn one_of(codec: &FixCodec, record: &Scalar) -> FixMsg {
    let mut messages = codec.parse_text_record(record).unwrap();
    let message = messages.next().unwrap().unwrap();
    assert!(messages.next().is_none(), "one message");
    message
}

#[test]
fn a_rows_pluginid_names_the_dialect_it_is_read_under_and_fills_its_own_column() {
    let (registry, venue) = plugin_registry();
    let codec = FixCodec::new(Arc::clone(&registry));
    let body: &[u8] = b"MSGTYPE=D|CLORDID=A|VENUETAG=dark";

    // The dialect's name, and its alias in another case: the row reads under
    // the dialect - its own field resolves, the root's `fix:branch` says so
    // and the message reports the registry's declaration, aliases and all -
    // and the column fills the crate's own field exactly as it was spelled.
    for spelled in ["venue", "VNU"] {
        let message = one_of(
            &codec,
            &plugin_record(body, Scalar::from(spelled), Scalar::Null),
        );
        assert_eq!(message.branch().name(), "venue", "{spelled}");
        assert!(message.branch().has_alias("vnu"), "{spelled}");
        assert_eq!(
            message.as_field().as_fix().branch().unwrap().name(),
            "venue",
            "{spelled}"
        );
        assert_eq!(
            message.by_tag(5001).unwrap().as_str(),
            Some("dark"),
            "{spelled}: the dialect's own field"
        );
        assert_eq!(
            message.by_tag(yggdryl::PLUGINID_TAG).unwrap().as_str(),
            Some(spelled)
        );
        assert!(
            message
                .entries()
                .iter()
                .all(|entry| entry.tag() != yggdryl::PLUGINID_TAG),
            "a fill is never an entry"
        );
    }

    // Any other plugin keeps the codec's pin, then the standard branch: a
    // plugin no branch is declared under, a null, an empty string, and a
    // name too long to spell a branch at all.
    let long = "x".repeat(FixBranch::MAX_LENGTH + 1);
    let silent = [
        Scalar::from("OMS_X1_TradeCapture"),
        Scalar::Null,
        Scalar::from(""),
        Scalar::from(long.as_str()),
    ];
    for plugin in &silent {
        let message = one_of(&codec, &plugin_record(body, plugin.clone(), Scalar::Null));
        assert!(message.branch().is_standard(), "{plugin:?}");
        assert!(
            message.get_by_tag(5001).is_none(),
            "{plugin:?}: the dialect's field is unknown to the standard"
        );
        assert_eq!(
            message.by_name("venuetag").unwrap().as_str(),
            Some("dark"),
            "{plugin:?}: kept under its own spelling"
        );
    }
    let pinned = codec.clone().with_branch(&venue);
    for plugin in &silent {
        let message = one_of(&pinned, &plugin_record(body, plugin.clone(), Scalar::Null));
        assert_eq!(message.branch().name(), "venue", "{plugin:?}");
        assert_eq!(message.by_tag(5001).unwrap().as_str(), Some("dark"));
    }
    // And the row outranks the pin, because it is the more specific
    // statement: a codec pinned elsewhere still reads this row as the venue.
    let elsewhere = codec
        .clone()
        .with_branch(&FixBranch::from_str("elsewhere").unwrap());
    let message = one_of(
        &elsewhere,
        &plugin_record(body, Scalar::from("vnu"), Scalar::Null),
    );
    assert_eq!(message.branch().name(), "venue");
    assert_eq!(message.by_tag(5001).unwrap().as_str(), Some("dark"));
    let message = one_of(
        &elsewhere,
        &plugin_record(body, Scalar::from("ULBridge"), Scalar::Null),
    );
    assert_eq!(message.branch().name(), "elsewhere");
}

#[test]
fn a_prevpluginid_column_fills_its_field_and_nothing_derives_it() {
    let codec = codec();
    let body: &[u8] = b"8=FIX.4.4|35=D|11=A|10=0|";
    let stated = plugin_record(
        body,
        Scalar::from("OMS_X1_TradeCapture"),
        Scalar::from("ULFilter"),
    );
    let message = one_of(&codec, &stated);
    assert_eq!(
        message.by_tag(yggdryl::PREVPLUGINID_TAG).unwrap().as_str(),
        Some("ULFilter")
    );
    assert_eq!(
        message.by_tag(yggdryl::PLUGINID_TAG).unwrap().as_str(),
        Some("OMS_X1_TradeCapture")
    );
    assert!(
        message.entries().iter().all(|entry| {
            entry.tag() != yggdryl::PREVPLUGINID_TAG && entry.tag() != yggdryl::PLUGINID_TAG
        }),
        "a fill is never an entry"
    );

    // Without the column nothing fills it - not the plugin, not the
    // direction the line moved - and the same holds of the session names,
    // which are only ever what the line spells.
    let unstated = Scalar::from_record([
        ("body", Scalar::from(body.to_vec())),
        ("pluginid", Scalar::from("OMS_X1_TradeCapture")),
        ("direction", Scalar::from("RECV")),
    ])
    .unwrap();
    let message = one_of(&codec, &unstated);
    for tag in [
        yggdryl::PREVPLUGINID_TAG,
        yggdryl::SENDERSESSIONNAME_TAG,
        yggdryl::TARGETSESSIONNAME_TAG,
    ] {
        assert!(
            message.get_by_tag(tag).is_none_or(Scalar::is_null),
            "tag {tag} is stated or nothing"
        );
    }
    // What the line spells still lands, through the field's own alias.
    let spelled = one_of(
        &codec,
        &Scalar::from_record([
            (
                "body",
                Scalar::from("MSGTYPE=D|CLORDID=A|ULFROMSESSIONNAME=OMS_X1_OrderOut"),
            ),
            ("pluginid", Scalar::from("ULFilter")),
        ])
        .unwrap(),
    );
    assert_eq!(
        spelled
            .by_tag(yggdryl::SENDERSESSIONNAME_TAG)
            .unwrap()
            .as_str(),
        Some("OMS_X1_OrderOut")
    );
    assert!(
        spelled
            .get_by_tag(yggdryl::TARGETSESSIONNAME_TAG)
            .is_none_or(Scalar::is_null)
    );
}

#[test]
fn the_batch_reader_and_the_record_reader_agree_on_a_rows_plugin() {
    let (registry, _) = plugin_registry();
    let codec = FixCodec::new(registry);
    // Every way a row can name its plugin: the dialect's name, its alias, a
    // plugin no branch is named after, nothing, and an empty string - beside
    // a previous plugin stated or not.
    let rows: [(&[u8], Option<&str>, Option<&str>); 5] = [
        (
            b"MSGTYPE=D|CLORDID=A|VENUETAG=dark",
            Some("venue"),
            Some("ULFilter"),
        ),
        (b"MSGTYPE=D|CLORDID=B|VENUETAG=lit", Some("VNU"), None),
        (
            b"MSGTYPE=D|CLORDID=C|VENUETAG=none",
            Some("OMS_X1_TradeCapture"),
            None,
        ),
        (b"MSGTYPE=D|CLORDID=D|VENUETAG=none", None, Some("ULBridge")),
        (b"MSGTYPE=D|CLORDID=E|VENUETAG=none", Some(""), None),
    ];
    let text = |held: Option<&str>| held.map_or(Scalar::Null, Scalar::from);
    let records: Vec<Scalar> = rows
        .iter()
        .map(|(body, plugin, previous)| plugin_record(body, text(*plugin), text(*previous)))
        .collect();
    let capture = DataType::from_fields([
        DataType::Binary.required_field("body"),
        DataType::Utf8.nullable_field("pluginid"),
        DataType::Utf8.nullable_field("prevpluginid"),
    ])
    .unwrap()
    .required_field("capture");
    let values = Scalar::from_sequence(
        rows.iter()
            .map(|(body, plugin, previous)| {
                Scalar::from_sequence([Scalar::from(body.to_vec()), text(*plugin), text(*previous)])
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

    // Row for row, the batch read is the record read: the plugin and the
    // previous plugin land in their columns, and the arrival record carries
    // the tag and the branch each key resolved to - 5001 under the venue,
    // nothing anywhere else - which is the dialect the row was read under.
    for (row, record) in records.iter().enumerate() {
        let alone = one_of(&codec, record);
        let streamed = &again[row];
        for tag in [yggdryl::PLUGINID_TAG, yggdryl::PREVPLUGINID_TAG] {
            assert_eq!(
                alone.get_by_tag(tag).filter(|held| !held.is_null()),
                streamed.get_by_tag(tag).filter(|held| !held.is_null()),
                "row {row} tag {tag}"
            );
        }
        assert_eq!(alone.entries(), streamed.entries(), "row {row}");
        let under_venue = matches!(rows[row].1, Some("venue" | "VNU"));
        assert_eq!(
            alone.entries().iter().any(|entry| entry.tag() == 5001),
            under_venue,
            "row {row} resolves the venue's field under the venue alone"
        );
        assert_eq!(alone.branch().name() == "venue", under_venue, "row {row}");
    }
    assert_eq!(
        again[0].by_tag(yggdryl::PREVPLUGINID_TAG).unwrap().as_str(),
        Some("ULFilter")
    );
    assert_eq!(
        again[1].by_tag(yggdryl::PLUGINID_TAG).unwrap().as_str(),
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

    // Feed the batch back through as a capture under one of its own text
    // columns: one implementation serves both, so a column source builds the
    // same schema a row source does - the capture's own columns first, then
    // the fixed ones under their own names - and a row in is a row out.
    let schema = first.schema();
    let again = codec
        .clone()
        .with_payload_column("msgtype")
        .parse_text_arrow_reader(yggdryl::arrow::batch_reader(
            schema.clone(),
            [first.clone()],
        ))
        .unwrap();
    let rows: usize = again.map(|batch| batch.unwrap().num_rows()).sum();
    assert_eq!(rows, 2, "a row in is still a row out");

    // The entries column is a list, not a payload: naming it is refused
    // before a row is read rather than answered as rows of nothing.
    let refused = codec
        .with_payload_column("nofixentries")
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
        DataType::Utf8.required_field("url"),
        DataType::Int64.required_field("rownum"),
        DataType::Utf8.nullable_field("threadname"),
        DataType::Binary.required_field("body"),
        // A name a FIX column already takes, which yields to it: one column
        // per name, and the FIX one is what a reader spelling it means.
        DataType::Utf8.nullable_field("nofixentries"),
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
        columns
            .iter()
            .filter(|held| *held == "nofixentries")
            .count(),
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
    assert_eq!(message.by_name("rownum").unwrap(), &Scalar::from(7_i64));
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
fn a_rows_dialect_is_read_from_its_pluginid_column_with_no_field_to_fill() {
    // The dialect a row names and the field the same column fills are two
    // answers to one cell, and only the first is the dictionary's business:
    // a registry holding no `pluginid` field of its own still reads its rows
    // under the dialect they name for themselves.
    let (registry, _) = plugin_registry();
    let mut without = registry.as_ref().clone();
    assert!(without.remove(yggdryl::PLUGINID_TAG).is_some());
    let codec = FixCodec::new(Arc::new(without));
    let body: &[u8] = b"MSGTYPE=D|CLORDID=A|VENUETAG=dark";

    let alone = one_of(
        &codec,
        &Scalar::from_record([
            ("body", Scalar::from(body.to_vec())),
            ("pluginid", Scalar::from("VNU")),
        ])
        .unwrap(),
    );
    assert_eq!(alone.branch().name(), "venue");
    assert_eq!(alone.by_tag(5001).unwrap().as_str(), Some("dark"));
    assert!(
        alone.get_by_tag(yggdryl::PLUGINID_TAG).is_none(),
        "no field of that name is there to fill"
    );

    let capture = DataType::from_fields([
        DataType::Binary.required_field("body"),
        DataType::Utf8.nullable_field("pluginid"),
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
    // carries the batch's own root, so the dialect it was read under is what
    // its arrival record resolved to: the venue's own tag, under the venue.
    assert_eq!(streamed.len(), 1);
    assert_eq!(streamed[0].entries(), alone.entries());
    assert!(
        streamed[0]
            .entries()
            .iter()
            .any(|entry| entry.tag() == 5001),
        "the venue's field resolved, so the row was read under the venue"
    );
}

#[test]
fn a_payload_column_spelled_pluginid_is_the_payload_and_names_no_dialect() {
    // The column [`FixCodec::with_payload_column`] names is the payload and
    // nothing else, whatever it is spelled: its text never picks a dialect
    // and never fills the crate's own `pluginid` field, or a capture whose
    // payload column happened to be spelled so would read every line under
    // whatever dialect the line's first bytes resembled.
    let (registry, _) = plugin_registry();
    let codec = FixCodec::new(registry).with_payload_column("pluginid");
    // One payload spelling a registered branch's name exactly, and one that
    // is a frame, so the column is proven to be read as the payload.
    let bodies = ["venue", "8=FIX.4.4|35=D|11=A|10=0|"];

    let records: Vec<Scalar> = bodies
        .iter()
        .map(|body| Scalar::from_record([("pluginid", Scalar::from(*body))]).unwrap())
        .collect();
    let alone: Vec<FixMsg> = records
        .iter()
        .map(|record| one_of(&codec, record))
        .collect();

    let capture = DataType::from_fields([DataType::Utf8.required_field("pluginid")])
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
    assert_eq!(row_count(&read), bodies.len());
    let streamed: Vec<FixMsg> = codec
        .messages(yggdryl::arrow::batch_reader(read[0].schema(), read.clone()))
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(streamed.len(), bodies.len());

    for (row, message) in alone.iter().enumerate() {
        for read in [message, &streamed[row]] {
            assert!(read.branch().is_standard(), "row {row}");
            assert!(
                read.get_by_tag(yggdryl::PLUGINID_TAG)
                    .is_none_or(Scalar::is_null),
                "row {row}: the payload never fills the plugin"
            );
            assert!(read.get_by_tag(5001).is_none(), "row {row}");
        }
        assert_eq!(message.entries(), streamed[row].entries(), "row {row}");
    }
    // The frame was read as the payload; the branch name held nothing to read.
    assert_eq!(alone[1].by_tag(11).unwrap().as_str(), Some("A"));
    assert!(alone[0].get_by_tag(11).is_none());
    assert!(alone[0].entries().is_empty());
}
