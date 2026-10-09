//! `rust/src/fix/batch.rs`: a capture in, columns out, and back to the
//! wire - through the codec's Arrow twins, and the two converters they
//! compose.

use std::sync::Arc;

use arrow_array::RecordBatch;
use yggdryl::arrow::BatchReader;

use super::SoleMessage;
use yggdryl::graph::{Element, Event, Market, Operation};
use yggdryl::text::{TextBytes, TextLine};
use yggdryl::{
    DataType, FixCodec, FixDedup, FixField, FixFieldMut, FixMsg, FixRegistry, IdKey, IdSource,
    IdType, Scalar, StructType, fix_schema,
};

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
    FixFieldMut::new(&mut direction).set_tag(385).unwrap();
    registry
        .set_codeset(
            "msgdirectioncodeset",
            &[
                yggdryl::FixCode::new("Receive", "R"),
                yggdryl::FixCode::new("Send", "S"),
            ],
        )
        .unwrap();
    FixFieldMut::new(&mut direction)
        .set_codeset("msgdirectioncodeset")
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
    let field = StructType::from_fields([DataType::binary().required_field("body")])
        .map(DataType::from)
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
            lay_out(&field, &values)
        })
        .collect();
    let schema = field.into_arrow_schema().unwrap();
    yggdryl::arrow::batch_reader(schema, batches)
}

/// Lay a run of rows out as the Arrow table of `root`'s column.
fn lay_out(root: &yggdryl::Field, rows: &Scalar) -> RecordBatch {
    let rows = rows.as_sequence().expect("a run of rows").to_vec();
    yggdryl::Serie::from_scalars(root.clone(), rows)
        .expect("rows the root accepts")
        .into_arrow_batch()
        .expect("a record column is a table")
}

/// Every row of one batch, each the run of its columns, read back under the
/// batch's own schema.
fn rows_of(batch: &RecordBatch) -> Vec<Scalar> {
    yggdryl::Serie::from_arrow_batch(None, batch, yggdryl::ArrowCastOptions::default())
        .expect("the projected rows")
        .rows()
        .into_owned()
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
    rows_of(batch)[0].as_sequence().expect("columns")[index].clone()
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

    // The columns every generated schema opens with lead, the capture's
    // own column follows them, then the message's own columns, named by
    // their folded names, each carrying its tag on the field - which is what
    // the row is filled by - each found by its name rather than by an offset.
    assert_eq!(names.first(), Some(&"curruuid"), "{names:?}");
    let at = |name: &str| {
        names
            .iter()
            .position(|held| *held == name)
            .unwrap_or_else(|| panic!("a {name} column in {names:?}"))
    };
    for pair in [
        "curruuid",
        "currunix",
        "creaunix",
        "prevunix",
        "snapunix",
        "partyids",
        "body",
        "sendingtime",
    ]
    .windows(2)
    {
        assert!(at(pair[0]) < at(pair[1]), "{pair:?} in {names:?}");
    }
    assert_eq!(
        at("body"),
        at("partyids") + 1,
        "right after the shared columns"
    );
    assert_eq!(names.last(), Some(&"fixentries"));
    // The standard header, the body a consumer queries, the groups worth
    // keeping whole, the trailer, and this crate's own derived facts - each
    // found by the tag its column carries.
    let fixed = yggdryl::Field::from_arrow_schema("row", &schema).expect("the schema reads");
    for tag in [
        55,
        54,
        60, // the instrument, the side and the clock
        44,
        38, // the price and the quantity, FIX's own fields
        132,
        133,
        134,
        135, // the quote's bid and offer
        768,
        1907, // the groups
        10,   // the trailer
        yggdryl::SECURITYIDS_TAG_NAME.0,
        yggdryl::PARTYIDS_TAG_NAME.0, // the identifiers the groups state
        yggdryl::CURRHASHCODE_TAG_NAME.0,
        yggdryl::CURRUNIX_TAG_NAME.0,
        yggdryl::CREAUNIX_TAG_NAME.0, // the digest and the clocks
        yggdryl::MSGSESSIONID_TAG_NAME.0,
        yggdryl::MSGCTXID_TAG_NAME.0, // what a bridge's own log states
        yggdryl::MSGSESSEVENTID_TAG_NAME.0, // the session event it joins to
        yggdryl::MSGDIRECTION_TAG_NAME.0, // which way the line moved
    ] {
        assert!(
            yggdryl::fix_column_of(&fixed, tag).is_some(),
            "tag {tag} missing from {names:?}"
        );
    }
    // The wire's own identifier fields are no columns: what a message
    // states of them is its entries, and its identifiers are the logical
    // columns above.
    for tag in [22, 48, 453, 454] {
        assert!(
            yggdryl::fix_column_of(&fixed, tag).is_none(),
            "tag {tag} in {names:?}"
        );
    }

    // Each column carries its tag and the spelling it had, so a renderer can
    // show `MsgType` over the column `msgtype`.
    let msgtype = schema
        .field_with_name("msgtype")
        .expect("the msgtype column");
    assert_eq!(
        msgtype.metadata().get("FIX:tag").map(String::as_str),
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
fn the_entries_column_keeps_only_content_the_columns_did_not_represent() {
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
    let digest = tag_column(&batch, yggdryl::CURRHASHCODE_TAG_NAME.0);
    assert_eq!(digest.data_type(), &arrow_schema::DataType::UInt64);
    assert!(digest.is_valid(0));
    // Every scalar this fixture states has a fixed column, so its residual
    // record is present but empty rather than duplicating the row.
    let entries = column(&batch, "fixentries");
    assert!(entries.is_valid(0));
    assert_eq!(
        first_value(&batch, "fixentries")
            .as_mapping()
            .map(<[(Scalar, Scalar)]>::len),
        Some(0)
    );
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
    // Every row of one shape lands as the same bytes, so the cut is even:
    // every closed batch holds the same number of rows. A fixed row lands
    // wider than the line it was parsed from - its typed columns stand
    // beside its record - so about half the target of raw capture is what
    // closes a batch.
    let closed = &batches[..batches.len() - 1];
    let rows = closed[0].num_rows();
    assert!(closed.iter().all(|batch| batch.num_rows() == rows));
    let per_row = raw / lines.len();
    let held = (rows * per_row) as u64;
    assert!(
        (TARGET / 4..=TARGET).contains(&held),
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
fn messages_to_batches_close_on_the_bytes_their_rows_land_as() {
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

    // A bound of about ten rows cuts the stream into batches of about ten,
    // and every row survives the cut. A row is measured by what it lands
    // as - the leaves of every column, the four hundred bytes of text among
    // them - and never by the line it was parsed from.
    let bounded = codec.clone().with_batch_byte_size(10 * 470);
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
fn messages_without_residual_entries_are_charged_and_rebuilt_by_their_columns() {
    let codec = codec();
    let schema = fix_schema(codec.registry(), "fix").unwrap();
    let whole = batches(
        codec
            .arrow_reader(schema, codec.parse_lines(wide()))
            .unwrap(),
    );
    assert_eq!(whole.len(), 1);
    // The projected columns alone still state ordinary message content.
    let batch = &whole[0];
    let lifted: Vec<usize> = (0..batch.num_columns())
        .filter(|at| batch.schema().field(*at).name() != yggdryl::fix::FIXENTRIES_COLUMN)
        .collect();
    let projected = batch.project(&lifted).unwrap();
    assert_eq!(projected.num_rows(), 200);
    let reader = || yggdryl::arrow::batch_reader(projected.schema(), [projected.clone()]);
    let restored: Vec<FixMsg> = codec
        .messages(reader())
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(restored.len(), 200);
    assert_eq!(
        restored[0].by_tag(11).unwrap().as_str(),
        Some("ORDER-000000")
    );
    let text = "x".repeat(400);
    assert_eq!(
        restored[0].by_tag(58).unwrap().as_str(),
        Some(text.as_str())
    );

    // Under a bound of about ten rows of leaves, the stream is cut into
    // batches of about ten - it is not one batch of everything, which is
    // what charging a row the bare row width would make.
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

/// The FIX-row door stamps the codec's plugin role the way the line doors
/// do: a row rebuilt through `messages` - and so through every reader
/// composed over it - states the source the codec reads under where its
/// schema holds no `msgpluginside` column, and a row carrying the cell is
/// the row's word over the codec's, a codec told no source included.
#[test]
fn the_row_door_stamps_the_codecs_plugin_role_where_the_row_states_none() {
    use yggdryl::{FixSource, Side};

    let mut registry = registry().as_ref().clone();
    for (id, side) in [("buy", Side::Buy), ("sell", Side::Sell)] {
        assert!(registry.add_source(FixSource::new(id).unwrap().with_pluginside(side)));
    }
    let registry = Arc::new(registry);
    let lines = [
        "8=FIX.4.4|35=D|11=A|55=AAPL|54=1|38=5|40=2|44=100|52=20240102-10:15:30|10=0|",
        "8=FIX.4.4|35=D|11=B|55=AAPL|54=2|38=5|40=2|44=100|52=20240102-10:15:31|10=0|",
    ];
    // The rows laid out under the Buy-Side source carry `BUYS`.
    let laid = batches(
        super::fixed_codec(Arc::clone(&registry))
            .with_source("buy")
            .unwrap()
            .parse_text_arrow_reader(capture_reader(&lines, 2))
            .unwrap(),
    );
    assert_eq!(laid.len(), 1);
    let whole = &laid[0];
    let carrying = || yggdryl::arrow::batch_reader(whole.schema(), [whole.clone()]);
    let kept: Vec<usize> = (0..whole.num_columns())
        .filter(|at| whole.schema().field(*at).name() != "msgpluginside")
        .collect();
    let projected = whole.project(&kept).unwrap();
    let stating_none = || yggdryl::arrow::batch_reader(projected.schema(), [projected.clone()]);
    for (source, stamped) in [(Some("sell"), Side::Sell), (None, Side::Unknown)] {
        let mut codec = super::fixed_codec(Arc::clone(&registry));
        if let Some(id) = source {
            codec = codec.with_source(id).unwrap();
        }
        let read = |reader| {
            codec
                .messages(reader)
                .collect::<yggdryl::Result<Vec<FixMsg>>>()
                .unwrap()
        };
        let carried = read(carrying());
        assert_eq!(carried.len(), 2);
        for message in &carried {
            assert_eq!(message.msgpluginside(), Side::Buy, "{source:?}");
        }
        let rebuilt = read(stating_none());
        assert_eq!(rebuilt.len(), 2);
        for message in &rebuilt {
            assert_eq!(message.msgpluginside(), stamped, "{source:?}");
        }
        // A reader composed over `messages` lays the stamp out again under
        // a schema holding the column: `lifecycle_arrow_reader` writes the
        // source's own schema, which the projection left the column out
        // of, and `format_arrow_reader` under the fixed row states it.
        let fixed = fix_schema(codec.registry(), "fix").unwrap();
        let formatted = batches(codec.format_arrow_reader(stating_none(), &fixed).unwrap());
        let cells = column(&formatted[0], "msgpluginside")
            .as_any()
            .downcast_ref::<arrow_array::UInt8Array>()
            .expect("the enum's uint8 codes");
        assert_eq!(cells.values().as_ref(), [stamped.code(), stamped.code()]);
    }
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
        let batch = lay_out(&field, &rows);
        yggdryl::arrow::batch_reader(batch.schema(), [batch])
    };
    let frame = Scalar::from(b"8=FIX.4.4|35=D|11=A|10=0|".to_vec());

    // The column is named otherwise: refused, naming what was asked for and
    // what the source carries.
    let named_line = StructType::from_fields([DataType::binary().required_field("line")])
        .map(DataType::from)
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
    let numbered = StructType::from_fields([DataType::Int64.required_field("body")])
        .map(DataType::from)
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
    // wrongly and no cell that could hold a number instead. Nor is there a
    // line carrying no bytes to ask about: the door that makes one refuses an
    // empty body by name, so an empty payload column stays a row that had
    // nothing to read rather than becoming a line holding an empty message.
    let refused = TextLine::from_bytes(
        0,
        TextBytes::default(),
        std::sync::Arc::new(yggdryl::text::TextOptions::new()),
    )
    .map(drop)
    .unwrap_err();
    assert!(
        refused.to_string().contains("body"),
        "no bytes, no line: {refused}"
    );

    // The empty `unknown` survives for the one case that is not this: a
    // payload that was there and would not parse, which a batch must not fail
    // on.
    let malformed = TextBytes::from_bytes(b"<Order ClOrdID='X'></Nope>").unwrap();
    let mut broken = codec
        .parse_text_line(
            &TextLine::from_bytes(
                0,
                malformed,
                std::sync::Arc::new(yggdryl::text::TextOptions::new()),
            )
            .unwrap(),
        )
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

/// The tag-11 values of every row of `batches`, in order.
fn clordids(batches: &[RecordBatch]) -> Vec<Option<String>> {
    batches
        .iter()
        .flat_map(|batch| {
            let at = super::tag_index(batch, 11);
            rows_of(batch)
                .into_iter()
                .map(move |row| {
                    row.as_sequence().expect("columns")[at]
                        .as_str()
                        .map(str::to_owned)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A line that is not a row, and a frame whose clock will not type, between
/// two good lines: what a capture holds in its middle.
const MALFORMED_MIDDLE: [&str; 4] = [
    "8=FIX.4.4|35=D|11=A|10=0|",
    "",
    "8=FIX.4.4|35=D|52=not a clock|11=M|10=0|",
    "8=FIX.4.4|35=D|11=B|10=0|",
];

#[test]
fn a_refused_line_is_passed_over_and_every_line_after_it_reads() {
    let codec = codec();
    let schema = fix_schema(codec.registry(), "fix").unwrap();

    // The line door passes over what cannot stand and goes on past it: no
    // item for the empty line. A frame whose clock will not type stands with
    // its clock unstated - dated as one stating none is - beside the anomaly
    // naming the text, as every other value that will not type does.
    let messages = codec
        .parse_lines(MALFORMED_MIDDLE)
        .collect::<yggdryl::Result<Vec<FixMsg>>>()
        .expect("nothing the lines hold is an error");
    let read: Vec<_> = messages
        .iter()
        .map(|message| message.by_tag(11).unwrap().as_str().map(str::to_owned))
        .collect();
    let every = [
        Some("A".to_owned()),
        Some("M".to_owned()),
        Some("B".to_owned()),
    ];
    assert_eq!(read, every);
    assert!(!messages[1].header().stated_sendingtime());
    assert!(
        messages[1]
            .anomalies()
            .iter()
            .any(|anomaly| anomaly.to_string().contains("not a clock")),
        "{:?}",
        messages[1].anomalies()
    );

    // Composed into batches, the rows before and after it are the batches.
    let rows = batches(
        codec
            .arrow_reader(schema, codec.parse_lines(MALFORMED_MIDDLE))
            .unwrap(),
    );
    assert_eq!(clordids(&rows), every);

    // The capture door reads the same: the empty cell is a row that carried
    // no payload, passed over, and the frame stands without its clock.
    let rows = batches(
        codec
            .parse_text_arrow_reader(capture_reader(&MALFORMED_MIDDLE, 2))
            .unwrap(),
    );
    assert_eq!(clordids(&rows), every);
    // And so does the walk over what the capture door wrote.
    let schema = rows[0].schema();
    let walked = batches(
        codec
            .lifecycle_arrow_reader(yggdryl::arrow::batch_reader(schema, rows))
            .unwrap(),
    );
    assert_eq!(row_count(&walked), 3);
}

#[test]
fn a_walk_written_under_a_table_schema_keeps_its_enum_columns_as_their_codes() {
    // An Iceberg table stores an enum column as the plain integer its code
    // is, so a walk over what such a table answers writes back under a
    // schema whose `state`, `side`, `marketdatakind` and `marketdatatype`
    // are `int32`: each member lands as its code, never as a null.
    use arrow_array::Array;

    let codec = codec();
    let schema = fix_schema(codec.registry(), "fix").unwrap();
    let stored = schema
        .clone()
        .into_scheme_compat(&yggdryl::Scheme::ICEBERG)
        .unwrap();
    assert_eq!(stored.get_field("state").unwrap().dtype(), &DataType::Int32);
    const LINE: &str = "8=FIX.4.4|35=D|11=A|55=AAPL|54=1|38=5|44=10|10=0|";
    let rows = batches(
        codec
            .arrow_reader(schema.clone(), codec.parse_lines(&[LINE]))
            .unwrap(),
    );
    // What the walk states of the message, which the stored row must hold.
    let walked_message = codec
        .lifecycle(codec.parse_lines(&[LINE]))
        .next()
        .expect("one walked message")
        .expect("a message");
    let as_stored = yggdryl::StreamChunkedSerie::from_arrow_reader(
        Some(&stored),
        yggdryl::arrow::batch_reader(rows[0].schema(), rows),
        yggdryl::ArrowCastOptions::new(),
    )
    .unwrap()
    .into_arrow_reader();
    let walked = batches(codec.lifecycle_arrow_reader(as_stored).unwrap());
    assert_eq!(row_count(&walked), 1);
    for (name, code) in [
        ("state", i32::from(walked_message.get_state().code())),
        ("side", i32::from(walked_message.get_side().code())),
        (
            "marketdatakind",
            i32::from(walked_message.marketdatakind().code()),
        ),
    ] {
        assert_ne!(code, 0, "{name} states a member");
        let column = walked[0].column_by_name(name).unwrap();
        let column = column
            .as_any()
            .downcast_ref::<arrow_array::Int32Array>()
            .unwrap_or_else(|| panic!("{name} stored as int32"));
        assert_eq!(column.null_count(), 0, "{name}");
        assert_eq!(column.value(0), code, "{name}");
    }
}

#[test]
fn a_source_failure_ends_the_batch_stream_after_the_completed_prefix() {
    let codec = codec();
    let schema = fix_schema(codec.registry(), "fix").unwrap();
    let parsed = |body: &[u8]| codec.sole_line(body).expect("a message");
    let marker = Arc::new(());
    let messages = [
        Ok(parsed(b"8=FIX.4.4|35=D|11=A|10=0|")),
        Err(source_failure(&marker)),
        Ok(parsed(b"8=FIX.4.4|35=D|11=B|10=0|")),
    ];
    // The prefix, then the source's own failure, then nothing.
    let mut reader = codec.arrow_reader(schema, messages).unwrap();
    let prefix = reader.next().unwrap().unwrap();
    assert_eq!(prefix.num_rows(), 1);
    assert_eq!(first_tag_value(&prefix, 11).as_str(), Some("A"));
    let failed = yggdryl::arrow::from_reader_error(reader.next().unwrap().unwrap_err());
    same_source_failure(failed.into(), &marker);
    assert!(reader.next().is_none(), "fused");
}

#[test]
fn a_reader_failure_ends_the_rows_after_the_messages_read_before_it() {
    use arrow_array::RecordBatchIterator;

    let codec = codec();
    let written = batches(codec.parse_text_arrow_reader(source()).unwrap());
    let batch = &written[0];
    let marker = Arc::new(());
    let halves = [batch.slice(0, 2), batch.slice(2, batch.num_rows() - 2)];
    let failing = || -> BatchReader {
        Box::new(RecordBatchIterator::new(
            [
                Ok(halves[0].clone()),
                Err(arrow_schema::ArrowError::ExternalError(Box::new(
                    source_failure(&marker),
                ))),
                Ok(halves[1].clone()),
            ],
            batch.schema(),
        ))
    };
    // The rows before the failure, then the failure, then nothing: a reader
    // that failed is not read again.
    let mut messages = codec.messages(failing());
    for _ in 0..2 {
        messages
            .next()
            .unwrap()
            .expect("a row read before the failure");
    }
    same_source_failure(messages.next().unwrap().unwrap_err(), &marker);
    assert!(messages.next().is_none());

    // The walk yields every message read before it, then the failure.
    let mut walked = codec.lifecycle_arrow_reader(failing()).unwrap();
    let prefix = walked.next().unwrap().unwrap();
    assert_eq!(prefix.num_rows(), 2);
    let failed = yggdryl::arrow::from_reader_error(walked.next().unwrap().unwrap_err());
    same_source_failure(failed.into(), &marker);
    assert!(walked.next().is_none());
}

#[test]
fn decoded_line_intake_accepts_owned_borrowed_and_both_fallible_forms() {
    let codec = codec();
    let line = TextLine::from_bytes(
        0,
        TextBytes::from_bytes(b"8=FIX.4.4|35=D|11=A|10=0|8=FIX.4.4|35=D|11=B|10=0|").unwrap(),
        std::sync::Arc::new(yggdryl::text::TextOptions::new()),
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
    let line = |body: &[u8]| {
        TextLine::from_bytes(
            0,
            TextBytes::from_bytes(body).unwrap(),
            std::sync::Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap()
    };
    let first = line(b"8=FIX.4.4|35=D|11=A|65099=stream|52=20260102-10:15:30|10=0|");
    let last = line(b"8=FIX.4.4|35=D|11=A|65099=stream|52=20260102-10:15:31|10=0|");
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
    // reads the source it was handed, once, up to its own failure, which
    // ends the reading: nothing past a source that failed is read.
    assert_eq!(pulls.get(), 2);
    let first = pipeline.next().expect("an item").expect("the message read");
    assert_eq!(first.get_creaunix(), Some(first.get_currunix()));
    // The source's own failure moves through as itself, after the messages
    // read before it, and ends the walk.
    same_source_failure(pipeline.next().expect("the failure").unwrap_err(), &marker);
    assert!(pipeline.next().is_none());
    assert!(pipeline.next().is_none());
    assert_eq!(pulls.get(), 2);
}

/// A walked message names its predecessor and keeps its own source.
///
/// `prevuuid` names the immediate predecessor, and nothing records the
/// chain further back. Its sources stay its own line's: provenance names
/// what a node was read from and never what it follows, so nothing of a
/// predecessor's source reaches its successor. Walked again, the chain is
/// the same chain under the same identities.
#[test]
fn a_walked_message_names_its_predecessor_and_keeps_its_own_source() {
    const LINES: [&[u8]; 3] = [
        b"8=FIX.4.4|35=D|11=CHAIN-1|55=AAPL|54=1|38=100|52=20260102-10:15:30|10=0|",
        b"8=FIX.4.4|35=8|11=CHAIN-1|37=O-1|17=E-1|39=1|150=F|55=AAPL|54=1|38=100|14=40|32=40|31=10.5|52=20260102-10:15:31|10=0|",
        b"8=FIX.4.4|35=8|11=CHAIN-1|37=O-1|17=E-2|39=2|150=F|55=AAPL|54=1|38=100|14=100|32=60|31=10.5|52=20260102-10:15:32|10=0|",
    ];
    let codec = codec();
    let lines: Vec<TextLine> = LINES
        .iter()
        .enumerate()
        .map(|(index, body)| {
            TextLine::from_bytes(
                index as u64,
                TextBytes::from_bytes(body).unwrap(),
                Arc::new(yggdryl::text::TextOptions::new()),
            )
            .unwrap()
            .with_handle_mtime(1_704_190_530_000_000_000 + index as i64)
        })
        .collect();
    let sources: Vec<_> = lines.iter().map(Element::get_curruuid).collect();
    let every: Vec<FixMsg> = codec
        .lifecycle(codec.parse_text_lines(lines.into_iter().map(Ok::<TextLine, yggdryl::Error>)))
        .map(Result::unwrap)
        .collect();
    // Each fill's report is its order's, and the execution split off it is
    // a chain of its own, naming the report beside the line (A12).
    assert_eq!(every.len(), 5);
    let (executions, walked): (Vec<FixMsg>, Vec<FixMsg>) = every
        .iter()
        .cloned()
        .partition(|message| message.marketdatakind() == yggdryl::MarketDataKind::Execution);
    assert_eq!(walked.len(), 3);
    let identities: Vec<_> = walked.iter().map(Element::get_curruuid).collect();
    assert_eq!(
        walked[0].get_prevuuid(),
        None,
        "the first of a chain follows nothing"
    );
    assert_eq!(walked[1].get_prevuuid(), Some(identities[0]));
    assert_eq!(walked[2].get_prevuuid(), Some(identities[1]));
    for (message, source) in walked.iter().zip(&sources) {
        assert_eq!(
            message.get_srcuuids(),
            [*source],
            "a source never travels along the chain"
        );
    }
    for (execution, (report, source)) in
        executions.iter().zip(walked[1..].iter().zip(&sources[1..]))
    {
        assert_eq!(
            execution.get_prevuuid(),
            None,
            "an execution follows nothing"
        );
        assert!(execution.get_srcuuids().contains(source));
        assert!(execution.get_srcuuids().contains(&report.get_srcuuids()[0]));
    }
    let walked = every;
    let again: Vec<FixMsg> = codec
        .lifecycle(walked.clone())
        .map(Result::unwrap)
        .collect();
    for (before, after) in walked.iter().zip(&again) {
        assert_eq!(after.get_srcuuids(), before.get_srcuuids());
        assert_eq!(after.get_prevuuid(), before.get_prevuuid());
        assert_eq!(after.get_curruuid(), before.get_curruuid());
    }
}

#[test]
fn lifecycle_drops_republications_and_true_retransmissions_but_keeps_distinct_deliveries() {
    let codec = codec();
    let original = b"8=FIX.4.4|35=D|49=S|56=T|34=7|52=20260102-10:15:30|11=REPLAY-1|55=AAPL|10=0|";
    let replay = b"8=FIX.4.4|35=D|49=S|56=T|34=7|43=Y|52=20260102-10:15:31|122=20260102-10:15:30|11=REPLAY-1|55=AAPL|10=0|";
    let distinct = b"8=FIX.4.4|35=D|49=S|56=T|34=8|52=20260102-10:15:32|11=REPLAY-1|55=AAPL|10=0|";
    let messages = [
        original.as_slice(),
        original.as_slice(),
        replay.as_slice(),
        distinct.as_slice(),
    ]
    .into_iter()
    .map(|line| codec.parse_fix_line(line));

    let walked: Vec<_> = codec
        .lifecycle(messages)
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(walked.len(), 2);
    assert_eq!(walked[0].header().msgseqnum(), Some(7));
    assert_eq!(walked[1].header().msgseqnum(), Some(8));
    // The distinct delivery stands at its own later instant, so following
    // the settled original leaves its place alone: dropped duplicates never
    // borrowed one for it to inherit.
    assert_eq!(
        walked[1].get_seqnum(),
        0,
        "a later delivery keeps its own place"
    );
    assert_eq!(walked[1].get_prevuuid(), Some(walked[0].get_curruuid()));
}

#[test]
fn lifecycle_delivery_identity_survives_arrow_reconstruction() {
    let codec = codec();
    let original = b"8=FIX.4.4|35=D|49=S|56=T|34=7|52=20260102-10:15:30|1=ACCOUNT|55=AAPL|10=0|";
    // The two ordinary row fields have distinct wire order but one content ID.
    let reordered = b"8=FIX.4.4|35=D|49=S|56=T|34=7|52=20260102-10:15:30|55=AAPL|1=ACCOUNT|10=0|";
    let replay = b"8=FIX.4.4|35=D|49=S|56=T|34=7|43=Y|52=20260102-10:15:31|122=20260102-10:15:30|1=ACCOUNT|55=AAPL|10=0|";
    // Delivery headers alone do not collapse changed content.
    let changed_content =
        b"8=FIX.4.4|35=D|49=S|56=T|34=7|52=20260102-10:15:30|1=ACCOUNT|55=MSFT|10=0|";
    let distinct = b"8=FIX.4.4|35=D|49=S|56=T|34=8|52=20260102-10:15:32|1=ACCOUNT|55=AAPL|10=0|";
    // No SendingTime or TransactTime: the fixed intake clock dates it, while
    // the distinct delivery sequence keeps the otherwise same event.
    let unstated_time = b"8=FIX.4.4|35=D|49=S|56=T|34=9|1=ACCOUNT|55=AAPL|10=0|";
    let original_message = codec.parse_fix_line(original).unwrap();
    let reordered_message = codec.parse_fix_line(reordered).unwrap();
    assert_ne!(original_message.digest(), reordered_message.digest());
    assert_eq!(
        original_message.get_currhashcode(),
        reordered_message.get_currhashcode()
    );
    let messages = vec![
        original_message.clone(),
        original_message,
        reordered_message,
        codec.parse_fix_line(replay).unwrap(),
        codec.parse_fix_line(changed_content).unwrap(),
        codec.parse_fix_line(distinct).unwrap(),
        codec.parse_fix_line(unstated_time).unwrap(),
        codec.parse_fix_line(unstated_time).unwrap(),
    ];

    let direct: Vec<FixMsg> = codec
        .lifecycle(messages.clone())
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(
        direct.len(),
        4,
        "repeat, reordered body, replay and duplicate unstated clock deduplicate"
    );
    assert_eq!(
        direct
            .iter()
            .filter(|message| message.header().msgseqnum() == Some(7))
            .count(),
        2,
        "changed content under one delivery header stays"
    );
    assert!(
        direct
            .iter()
            .any(|message| message.header().msgseqnum() == Some(8)),
        "a distinct delivery sequence stays"
    );
    assert!(
        direct
            .iter()
            .any(|message| message.header().msgseqnum() == Some(9)),
        "an unstated SendingTime without TransactTime stays"
    );
    assert_eq!(
        codec
            .lifecycle(direct.clone())
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap()
            .len(),
        direct.len(),
        "a second walk retains the settled deliveries"
    );

    let schema = fix_schema(codec.registry(), "fix").unwrap();
    let rows = codec.arrow_reader(schema, messages).unwrap();
    let arrow: Vec<FixMsg> = codec
        .messages(codec.lifecycle_arrow_reader(rows).unwrap())
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(arrow.len(), direct.len());
    for (direct, arrow) in direct.iter().zip(&arrow) {
        assert_eq!(arrow.get_curruuid(), direct.get_curruuid());
        assert_eq!(arrow.get_currhashcode(), direct.get_currhashcode());
        assert_eq!(arrow.get_prevuuid(), direct.get_prevuuid());
        assert_eq!(arrow.get_state(), direct.get_state());
        assert_eq!(arrow.get_seqnum(), direct.get_seqnum());
        assert_eq!(arrow.get_currunix(), direct.get_currunix());
        assert_eq!(arrow.get_recdunix(), direct.get_recdunix());
        assert_eq!(arrow.get_execunix(), direct.get_execunix());
    }
}

/// `row`, a record of `fields`, as a table storing a null list of structs
/// as an empty one - PyIceberg's - reads it back: every group the row
/// states no occurrence of, at the root and inside each occurrence, reads
/// back stating `[]`.
fn with_absent_groups_emptied(fields: &[yggdryl::Field], row: &Scalar) -> Scalar {
    let cells = row.as_sequence().expect("a record");
    Scalar::from_sequence(fields.iter().zip(cells).map(|(field, cell)| {
        let (DataType::Serie(item) | DataType::LargeSerie(item)) = field.dtype() else {
            return cell.clone();
        };
        if !matches!(item.dtype(), DataType::Struct(_)) {
            return cell.clone();
        }
        match cell.as_sequence() {
            Some(occurrences) => Scalar::from_sequence(occurrences.iter().map(|occurrence| {
                if occurrence.is_null() {
                    occurrence.clone()
                } else {
                    with_absent_groups_emptied(item.fields(), occurrence)
                }
            })),
            None if cell.is_null() => Scalar::from_sequence(Vec::<Scalar>::new()),
            None => cell.clone(),
        }
    }))
}

#[test]
fn lifecycle_over_rows_reading_an_absent_group_back_empty_yields_the_identities_written() {
    // The bridge logs one execution report at two hops of one conversation:
    // twice as the session event its row header brackets - folded before the
    // walk, so settled again from its content - and once framed by the next
    // session, whose row keeps the content code the parse recorded. A table
    // storing a null list as an empty one reads every absent group back as
    // `[]` - each party's `partysubids`, and at the root every group a
    // message states none of; a list read back empty where the row held null
    // is the group absent, so the walk over the rows
    // read back yields, message for message, the identity and the content
    // code the walk over the rows written yields.
    let codec = codec();
    let source = yggdryl::holder::Buffer::from_bytes(include_bytes!("ulbridge.log").to_vec())
        .with_media_type(
            yggdryl::Url::from_str("file:///ulbridge.log")
                .expect("a URL")
                .media_type(),
        );
    let mut options = yggdryl::text::TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the bridge's row header compiles")
        .with_timezone(yggdryl::Timezone::UTC);
    options.start_rownum = Some(1);
    let messages: Vec<FixMsg> = yggdryl::text::read_text_lines(&source, &options)
        .expect("a line reader")
        .map(|line| line.expect("a line"))
        .filter_map(|line| codec.parse_text_line(&line).ok())
        .flatten()
        .filter_map(Result::ok)
        .collect();
    let schema = fix_schema(codec.registry(), "fix").unwrap();
    let written: Vec<Scalar> = messages
        .iter()
        .map(|message| message.into_row(&schema).unwrap())
        .collect();
    let read_back: Vec<Scalar> = written
        .iter()
        .map(|row| with_absent_groups_emptied(schema.fields(), row))
        .collect();
    assert_ne!(read_back, written, "the capture states absent groups");
    let walk = |rows: Vec<Scalar>| -> Vec<(yggdryl::Uuid, u64)> {
        let batch = lay_out(&schema, &Scalar::from_sequence(rows));
        let rows = yggdryl::arrow::batch_reader(batch.schema(), [batch]);
        codec
            .messages(codec.lifecycle_arrow_reader(rows).unwrap())
            .map(|message| {
                let message = message.expect("a walked message");
                (message.get_curruuid(), message.get_currhashcode())
            })
            .collect()
    };
    assert_eq!(walk(read_back), walk(written));
}

#[test]
fn a_session_event_merged_from_rows_reading_an_absent_group_back_empty_writes_no_count() {
    // Two observations of one session event, each stating a party with no
    // `NoPartySubIDs(802)`, differing in what the merge fills, so their
    // content is merged. A table storing a null list as an empty one reads
    // the absent subgroup, and every absent group at the root, back as `[]`,
    // and the merge writes no count for a group holding no occurrence that no
    // observation stated: the merge over the rows read back is the merge over
    // the rows written.
    let codec = codec().with_capture_names(["msgsessionid", "msgctxid", "msgseqnum"]);
    let captured = |body: &[u8], recdunix: i64| {
        let line = TextLine::from_bytes(
            recdunix as u64,
            TextBytes::from_bytes(body).unwrap(),
            Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap()
        .with_captures(vec![
            Some(TextBytes::from_bytes(b"SESSION-A").unwrap()),
            Some(TextBytes::from_bytes(b"CONTEXT-A").unwrap()),
            Some(TextBytes::from_bytes(b"7").unwrap()),
        ])
        .unwrap()
        .with_handle_mtime(recdunix);
        codec
            .parse_text_line(&line)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
    };
    let observations = [
        captured(
            b"8=FIX.4.4|35=D|52=20260102-10:15:30|55=AAPL|453=1|448=X|447=D|452=1|10=0|",
            100,
        ),
        captured(
            b"8=FIX.4.4|35=D|52=20260102-10:15:30|55=AAPL|38=5|453=1|448=X|447=D|452=1|10=0|",
            200,
        ),
    ];
    let schema = fix_schema(codec.registry(), "fix").unwrap();
    let written: Vec<Scalar> = observations
        .iter()
        .map(|message| message.into_row(&schema).unwrap())
        .collect();
    let read_back: Vec<Scalar> = written
        .iter()
        .map(|row| with_absent_groups_emptied(schema.fields(), row))
        .collect();
    assert_ne!(read_back, written, "each observation states absent groups");
    let walk = |rows: Vec<Scalar>| -> Vec<FixMsg> {
        let batch = lay_out(&schema, &Scalar::from_sequence(rows));
        let rows = yggdryl::arrow::batch_reader(batch.schema(), [batch]);
        codec
            .messages(codec.lifecycle_arrow_reader(rows).unwrap())
            .collect::<yggdryl::Result<_>>()
            .unwrap()
    };
    let merged = walk(written.clone());
    assert_eq!(merged.len(), 1, "one session event");
    // Read back whole, or one observation read back beside one written.
    for rows in [
        read_back.clone(),
        vec![written[0].clone(), read_back[1].clone()],
        vec![read_back[0].clone(), written[1].clone()],
    ] {
        let again = walk(rows);
        assert_eq!(again.len(), 1, "one session event");
        assert_eq!(again[0].get_currhashcode(), merged[0].get_currhashcode());
        assert_eq!(again[0].get_curruuid(), merged[0].get_curruuid());
        let wire = String::from_utf8(again[0].into_bytes(b'|')).unwrap();
        assert!(wire.contains("|38=5|"), "the merged content: {wire}");
        assert!(!wire.contains("802="), "{wire}");
    }
}

#[test]
fn lifecycle_fully_merges_one_session_event_on_the_latest_recording_base() {
    let codec = codec().with_capture_names(["msgsessionid", "msgctxid", "msgseqnum"]);
    let captured = |context: &[u8], body: &[u8], recdunix: i64, execunix: i64| {
        let line = TextLine::from_bytes(
            recdunix as u64,
            TextBytes::from_bytes(body).unwrap(),
            Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap()
        .with_captures(vec![
            Some(TextBytes::from_bytes(b"SESSION-A").unwrap()),
            Some(TextBytes::from_bytes(context).unwrap()),
            Some(TextBytes::from_bytes(b"7").unwrap()),
        ])
        .unwrap()
        .with_handle_mtime(recdunix);
        let mut message = codec
            .parse_text_line(&line)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        message.set_execunix(Some(execunix), true);
        message
    };

    let older = captured(
        b"CONTEXT-A",
        b"8=FIX.4.4|35=D|52=20260102-10:15:30|55=AAPL|isincode=US0378331005|miccode=XNYS|10=0|",
        100,
        90,
    );
    let newer = captured(
        b"CONTEXT-A",
        b"8=FIX.4.4|35=D|52=20260102-10:15:30|55=MSFT|10=0|",
        200,
        110,
    );
    assert_ne!(older.get_curruuid(), newer.get_curruuid());
    let older_source = older.get_srcuuids()[0];
    let newer_source = newer.get_srcuuids()[0];
    // The four parts joined as stated, on the capture and no identifier.
    assert_eq!(
        older.capture().msgsesseventid(),
        Some("D:SESSION-A:CONTEXT-A:7")
    );
    assert!(
        !older
            .get_identifiers()
            .contains_kind(&"msgsesseventid".parse::<IdType>().unwrap())
    );

    let directly_merged = newer
        .clone()
        .with_previous(&older)
        .expect("one session event forces a full merge");
    assert!(directly_merged.get_prevuuid().is_none());
    assert_eq!(directly_merged.get_by_tag(55), Some(Scalar::from("MSFT")));
    assert_eq!(
        directly_merged.get_securityids().get(&IdType::Isin),
        Some("US0378331005")
    );
    // The later recording (200) is the reference, and the merged event
    // keeps the earliest recording either statement knows.
    assert_eq!(directly_merged.get_recdunix(), Some(100));
    assert_eq!(directly_merged.get_execunix(), Some(90));
    // Either way round: the reference is the later recording, not the
    // statement merged into.
    let reversed = older
        .clone()
        .with_previous(&newer)
        .expect("one session event forces a full merge");
    assert_eq!(reversed.get_by_tag(55), Some(Scalar::from("MSFT")));
    assert_eq!(reversed.get_recdunix(), Some(100));

    // Recorded at one instant, the later event instant is the reference; a
    // stated recording leads an unstated one whatever its instant; and an
    // exact tie keeps the statement merged into.
    let early = captured(
        b"CONTEXT-T",
        b"8=FIX.4.4|35=D|52=20260102-10:15:30|55=AAPL|10=0|",
        100,
        90,
    );
    let late = captured(
        b"CONTEXT-T",
        b"8=FIX.4.4|35=D|52=20260102-10:15:31|55=MSFT|10=0|",
        100,
        95,
    );
    for (into, from) in [(&early, &late), (&late, &early)] {
        let merged = into.clone().with_previous(from).expect("one event");
        assert_eq!(merged.get_by_tag(55), Some(Scalar::from("MSFT")));
        assert_eq!(merged.get_recdunix(), Some(100));
    }
    let mut unrecorded = late.clone();
    unrecorded.set_recdunix(None);
    for (into, from) in [(&early, &unrecorded), (&unrecorded, &early)] {
        let merged = into.clone().with_previous(from).expect("one event");
        assert_eq!(merged.get_by_tag(55), Some(Scalar::from("AAPL")));
        assert_eq!(merged.get_recdunix(), Some(100));
    }
    let twin = captured(
        b"CONTEXT-T",
        b"8=FIX.4.4|35=D|52=20260102-10:15:30|55=GOOG|10=0|",
        100,
        90,
    );
    assert_eq!(twin.get_currunix(), early.get_currunix());
    for (into, from) in [(&early, &twin), (&twin, &early)] {
        let merged = into.clone().with_previous(from).expect("one event");
        assert_eq!(merged.get_by_tag(55), into.get_by_tag(55));
    }

    for source in [
        vec![older.clone(), newer.clone()],
        vec![newer.clone(), older.clone()],
    ] {
        let walked = codec
            .lifecycle(source)
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(walked.len(), 1);
        let message = &walked[0];
        assert_eq!(message.header().msgseqnum(), Some(7));
        assert_eq!(message.capture().msgsessionid(), Some("SESSION-A"));
        assert_eq!(message.capture().msgctxid(), Some("CONTEXT-A"));
        assert_eq!(message.get_by_tag(55), Some(Scalar::from("MSFT")));
        assert_eq!(
            message.get_securityids().get(&IdType::Isin),
            Some("US0378331005"),
            "the reference keeps its row and the full graph merge fills a missing market fact"
        );
        let mut expected_sources = [newer_source, older_source];
        expected_sources.sort_unstable();
        assert_eq!(message.get_srcuuids(), expected_sources);
        assert_eq!(message.get_execunix(), Some(90));
        assert_eq!(
            message.get_recdunix(),
            Some(100),
            "the latest recording selects the base while the merged fact remains the earliest"
        );
        assert_eq!(message.get_seqnum(), 0, "one delivery takes one place");
        assert!(message.get_prevuuid().is_none());
    }

    let middle = captured(
        b"CONTEXT-A",
        b"8=FIX.4.4|35=D|52=20260102-10:15:30|55=GOOG|miccode=XPAR|10=0|",
        150,
        100,
    );
    let observations = [newer.clone(), older.clone(), middle];
    // One walk collects every observation of the delivery and folds them
    // latest-recorded first (200, 150, 100): the fold's earliest recording
    // is then never earlier than the statement folded next, so the latest
    // recording stays the reference whatever order the rows arrived in.
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let walked = codec
            .lifecycle(order.map(|index| observations[index].clone()))
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(walked.len(), 1);
        assert_eq!(
            walked[0].get_by_tag(55),
            Some(Scalar::from("MSFT")),
            "the maximum recording remains the base after its recording fact folds to the minimum"
        );
        assert_eq!(
            walked[0].get_miccode().map(|code| code.as_str()),
            Some("XPAR"),
            "the next-latest observation fills a fact the reference omitted"
        );
        assert_eq!(walked[0].get_recdunix(), Some(100));
    }

    let premerged = codec
        .lifecycle([older.clone(), newer.clone()])
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap()
        .pop()
        .expect("one premerged delivery");
    assert_eq!(premerged.get_by_tag(55), Some(Scalar::from("MSFT")));
    assert_eq!(premerged.get_recdunix(), Some(100));
    // A folded delivery keeps no trace of the recording its reference was
    // chosen by: it ranks by the earliest recording it keeps (100) against
    // a third observation, so the middle one (150) leads a replay. Folding
    // is not associative in its reference - of three statements, the
    // reference is the latest-recorded of the pair folded last - and the
    // one walk above is what picks the latest of all three.
    for source in [
        vec![premerged.clone(), observations[2].clone()],
        vec![observations[2].clone(), premerged.clone()],
    ] {
        let replayed = codec
            .lifecycle(source)
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(replayed.len(), 1);
        assert_eq!(
            replayed[0].get_by_tag(55),
            Some(Scalar::from("GOOG")),
            "the later recording of the pair folded last is the reference"
        );
        assert_eq!(
            replayed[0].get_securityids().get(&IdType::Isin),
            Some("US0378331005"),
            "the folded pair still fills what the new reference omits"
        );
        assert_eq!(
            replayed[0].get_miccode().map(|code| code.as_str()),
            Some("XPAR"),
            "the reference's own fact stands"
        );
        assert_eq!(replayed[0].get_recdunix(), Some(100));
        assert_eq!(replayed[0].get_execunix(), Some(90));
    }

    let other_type = captured(
        b"CONTEXT-A",
        b"8=FIX.4.4|35=F|52=20260102-10:15:30|55=TSLA|10=0|",
        300,
        120,
    );
    let walked = codec
        .lifecycle([older.clone(), other_type, newer.clone()])
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        walked.len(),
        2,
        "one session/context/sequence under two message types is two deliveries"
    );
    let order = walked
        .iter()
        .find(|message| message.header().msgtype() == "D")
        .expect("the order delivery");
    assert_eq!(order.get_by_tag(55), Some(Scalar::from("MSFT")));
    let mut expected_sources = [newer_source, older_source];
    expected_sources.sort_unstable();
    assert_eq!(order.get_srcuuids(), expected_sources);
    assert_eq!(order.get_recdunix(), Some(100));
    let cancel = walked
        .iter()
        .find(|message| message.header().msgtype() == "F")
        .expect("the cancel delivery");
    assert_eq!(cancel.get_by_tag(55), Some(Scalar::from("TSLA")));

    let first_context = captured(
        b"CONTEXT-A",
        b"8=FIX.4.4|35=D|49=S|56=T|52=20260102-10:15:30|55=MSFT|10=0|",
        300,
        120,
    );
    let other_context = captured(
        b"CONTEXT-B",
        b"8=FIX.4.4|35=D|49=S|56=T|52=20260102-10:15:30|55=MSFT|10=0|",
        400,
        130,
    );
    // Two deliveries, which the walk answers under one identity: the
    // second restates the first, and the window yields that identity once.
    let contexts = [first_context, other_context];
    let every = codec
        .clone()
        .with_dedup_window_ms(0)
        .lifecycle(contexts.clone())
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(every.len(), 2, "all four capture identity facts must match");
    assert_eq!(every[1].get_curruuid(), every[0].get_curruuid());
    assert_eq!(
        codec
            .lifecycle(contexts)
            .collect::<yggdryl::Result<Vec<_>>>()
            .unwrap()
            .len(),
        1
    );

    let expiring = captured(
        b"CONTEXT-C",
        b"8=FIX.4.4|35=D|49=S|56=T|52=20260102-10:15:30|126=20260102-10:15:31|55=AAPL|10=0|",
        500,
        140,
    );
    let walked = codec
        .lifecycle([expiring])
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(walked.len(), 2, "the deadline emits one expiry");
    let live = &walked[0];
    let expired = &walked[1];
    assert_eq!(expired.get_currunix(), live.get_exprunix().unwrap());
    assert_eq!(expired.get_prevuuid(), Some(live.get_curruuid()));
    // The deadline (10:15:31) falls after the live event's own instant
    // (10:15:30), so the expiry keeps the place it started at, zero.
    assert_eq!(expired.get_seqnum(), 0);
    assert_eq!(*expired.get_state(), yggdryl::State::Expired);
    assert_eq!(
        live.capture().msgsesseventid(),
        Some("D:SESSION-A:CONTEXT-C:7")
    );
    assert_eq!(
        expired.capture().msgsesseventid(),
        live.capture().msgsesseventid()
    );
    assert!(
        expired.clone().with_previous(live).is_none(),
        "a keyed synthetic expiry replays as its existing successor"
    );
}

#[test]
fn lifecycle_merges_overlapping_bridge_groups_by_sorted_occurrence_index() {
    let codec = codec().with_capture_names(["msgsessionid", "msgctxid", "msgseqnum"]);
    let captured = |body: &[u8], recdunix: i64| {
        let line = TextLine::from_bytes(
            recdunix as u64,
            TextBytes::from_bytes(body).unwrap(),
            Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap()
        .with_captures(vec![
            Some(TextBytes::from_bytes(b"FIDESSA-X1").unwrap()),
            Some(TextBytes::from_bytes(b"HOCHE-BAINS-XPAR").unwrap()),
            Some(TextBytes::from_bytes(b"42").unwrap()),
        ])
        .unwrap()
        .with_handle_mtime(recdunix);
        codec
            .parse_text_line(&line)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
    };
    // Each observation uses the marked and bare spelling at the same raw
    // index. The bridge parser makes their stable A/B order; the delivery
    // fold must merge A with A and B with B, not append four occurrences.
    let older = captured(
        b"MSGTYPE=D|NOPARTYIDS=1|NOPARTYIDS[0]=PARTYID=A\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|#NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=B\x04\x03PARTYIDSOURCE=C\x04\x03PARTYROLE=12",
        100,
    );
    let newer = captured(
        b"MSGTYPE=D|#NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=A\x04\x03PARTYROLE=3|NOPARTYIDS=1|NOPARTYIDS[0]=PARTYID=B\x04\x03PARTYIDSOURCE=D",
        200,
    );
    let merged = codec
        .lifecycle([older, newer])
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(merged.len(), 1);
    let message = &merged[0];
    assert_eq!(
        message
            .by_name("parties")
            .unwrap()
            .as_sequence()
            .map(<[Scalar]>::len),
        Some(2)
    );
    let parties = message
        .entries()
        .iter()
        .find(|entry| entry.tag() == 453)
        .expect("one merged Parties group");
    assert_eq!(parties.entries().len(), 2);
    let ids: Vec<&str> = parties
        .entries()
        .iter()
        .map(|occurrence| {
            occurrence
                .entries()
                .iter()
                .find(|member| member.tag() == 448)
                .and_then(|member| member.value())
                .expect("a PartyID")
        })
        .collect();
    assert_eq!(ids, ["A", "B"], "occurrence order is deterministic");
    for occurrence in parties.entries() {
        let tags: Vec<i32> = occurrence
            .entries()
            .iter()
            .map(|member| member.tag())
            .collect();
        assert_eq!(tags, [447, 448, 452], "members are sorted and unique");
    }
    let value = |occurrence: usize, tag: i32| {
        parties.entries()[occurrence]
            .entries()
            .iter()
            .find(|member| member.tag() == tag)
            .and_then(|member| member.value())
    };
    assert_eq!(value(0, 447), Some("D"), "older fills the missing source");
    assert_eq!(value(0, 452), Some("3"), "latest recording wins a conflict");
    assert_eq!(value(1, 447), Some("D"), "latest recording wins a conflict");
    assert_eq!(value(1, 452), Some("12"), "older fills the missing role");
    assert_eq!(message.get_recdunix(), Some(100));
    assert_eq!(
        message.capture().msgsesseventid(),
        Some("D:FIDESSA-X1:HOCHE-BAINS-XPAR:42")
    );
}

#[test]
fn lifecycle_inherits_the_client_order_and_the_parents_from_the_exact_predecessor() {
    let codec = codec();
    let previous = codec
        .parse_fix_line(
            b"8=FIX.4.4|35=8|52=20260102-10:15:30|11=CLIENT-A|37=ORDER-A|198=SHARED|39=0|10=0|",
        )
        .unwrap();
    // A report of a partial fill: the stream door's report, its order's,
    // the execution it splits off walking a chain of its own (A12).
    let current = codec
        .sole_line(
            b"8=FIX.4.4|35=8|52=20260102-10:15:31|37=ORDER-B|198=SHARED|parentorderid=|39=1|10=0|",
        )
        .unwrap();
    let before = current.get_curruuid();
    let walked = codec
        .lifecycle([previous, current])
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(walked.len(), 2);
    let (previous, current) = (&walked[0], &walked[1]);
    assert_eq!(current.get_prevuuid(), Some(previous.get_curruuid()));
    assert_eq!(
        current.get_by_tag(11).as_ref().and_then(Scalar::as_str),
        Some("CLIENT-A")
    );
    // The parents travel in the identifiers alone: the order identifier
    // that changed has the value it held before as its parent, and its
    // chain's first as its origin, and no field is written for them - the
    // spelling the line stated empty stays as it arrived.
    assert_eq!(
        current
            .get_by_name("parentorderid")
            .as_ref()
            .and_then(Scalar::as_str)
            .filter(|held| !held.is_empty()),
        None
    );
    for parent in ["parentorderid", "origorderid"] {
        assert_eq!(
            current
                .get_identifiers()
                .get_from(&IdKey::base(parent.parse::<IdType>().unwrap())),
            Some("ORDER-A"),
            "{parent}"
        );
    }
    assert_ne!(
        current.get_curruuid(),
        before,
        "the inherited FIX facts restate identity"
    );
    assert_eq!(current.get_curruuid(), current.time_uuid().unwrap());

    let previous = codec
        .parse_fix_line(
            b"8=FIX.4.4|35=8|52=20260102-10:16:30|11=CLIENT-PREV|37=ORDER-PREV|198=SHARED-STATED|39=0|10=0|",
        )
        .unwrap();
    // A report of a partial fill: the stream door's report, its order's,
    // the execution it splits off walking a chain of its own (A12).
    let current = codec
        .sole_line(
            b"8=FIX.4.4|35=8|52=20260102-10:16:31|11=CLIENT-STATED|37=ORDER-NEXT|198=SHARED-STATED|parentorderid=PARENT-STATED|39=1|10=0|",
        )
        .unwrap();
    let walked = codec
        .lifecycle([previous, current])
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    let current = &walked[1];
    assert_eq!(
        current.get_by_tag(11).as_ref().and_then(Scalar::as_str),
        Some("CLIENT-STATED")
    );
    assert_eq!(
        current
            .get_by_name("parentorderid")
            .as_ref()
            .and_then(Scalar::as_str),
        Some("PARENT-STATED"),
        "stated links are never overwritten"
    );
}

#[test]
fn lifecycle_headerless_reordered_content_is_one_delivery_through_arrow() {
    let codec = codec();
    let first = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|52=20260102-10:15:30|1=ACCOUNT|55=AAPL|10=0|")
        .unwrap();
    let reordered = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|52=20260102-10:15:30|55=AAPL|1=ACCOUNT|10=0|")
        .unwrap();
    assert_ne!(first.digest(), reordered.digest());
    assert_eq!(first.get_curruuid(), reordered.get_curruuid());

    let direct: Vec<FixMsg> = codec
        .lifecycle([first.clone(), reordered.clone()])
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(direct.len(), 1);
    let schema = fix_schema(codec.registry(), "fix").unwrap();
    let rows = codec.arrow_reader(schema, [first, reordered]).unwrap();
    let arrow: Vec<FixMsg> = codec
        .messages(codec.lifecycle_arrow_reader(rows).unwrap())
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(arrow.len(), 1);
    assert_eq!(arrow[0].get_curruuid(), direct[0].get_curruuid());
    assert_eq!(arrow[0].get_currhashcode(), direct[0].get_currhashcode());
}

#[test]
fn lifecycle_remembers_deliveries_across_the_whole_finite_capture() {
    // The delivery set is what this pins, so the window stays out of it: the
    // deliveries restate one event, which a windowed walk yields once.
    let codec = codec().with_dedup_window_ms(0);
    let first = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|11=LONG-CAPTURE|10=0|")
        .unwrap();
    let mut messages = vec![first.clone()];
    for sequence in 2_u64..=4_097 {
        let mut message = first.clone();
        message.set(34, Scalar::from(sequence)).unwrap();
        messages.push(message);
    }
    messages.push(first);
    assert_eq!(
        codec
            .lifecycle(messages.clone())
            .map(Result::unwrap)
            .count(),
        4_097,
        "a repeated delivery remains a repeat after many distinct deliveries"
    );
    let windowed = codec.with_dedup_window_ms(FixCodec::DEFAULT_DEDUP_WINDOW_MS);
    assert_eq!(windowed.lifecycle(messages).map(Result::unwrap).count(), 1);
}

#[test]
fn lifecycle_deduplicates_headerless_repeats_within_one_capture_context_only() {
    // The delivery set is what this pins; the window, which yields the one
    // identity both sessions' deliveries restate once, stays out of it.
    let codec = codec()
        .with_capture_names(["msgsessionid"])
        .with_dedup_window_ms(0);
    let captured = |session: &[u8]| {
        let line = TextLine::from_bytes(
            0,
            TextBytes::from_bytes(
                b"8=FIX.4.4|35=D|52=20260102-10:15:30|11=HEADERLESS|55=AAPL|10=0|",
            )
            .unwrap(),
            Arc::new(yggdryl::text::TextOptions::new()),
        )
        .unwrap()
        .with_captures(vec![Some(TextBytes::from_bytes(session).unwrap())])
        .unwrap();
        codec
            .parse_text_line(&line)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
    };
    let first = captured(b"SESSION-A");
    let repeated = captured(b"SESSION-A");
    let other_session = captured(b"SESSION-B");
    let walked: Vec<_> = codec
        .lifecycle([first.clone(), repeated, other_session.clone()])
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(walked.len(), 2);
    assert_eq!(walked[0].capture().msgsessionid(), Some("SESSION-A"));
    assert_eq!(walked[1].capture().msgsessionid(), Some("SESSION-B"));
    assert_eq!(walked[1].get_curruuid(), walked[0].get_curruuid());
    let windowed = codec
        .clone()
        .with_dedup_window_ms(FixCodec::DEFAULT_DEDUP_WINDOW_MS)
        .lifecycle([first, other_session])
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(windowed.len(), 1);
    assert_eq!(windowed[0].capture().msgsessionid(), Some("SESSION-A"));

    let sequenced = [
        codec.parse_fix_line(
            b"8=FIX.4.4|35=D|34=1|52=20260102-10:15:30|11=HEADERLESS|55=AAPL|10=0|",
        ),
        codec.parse_fix_line(
            b"8=FIX.4.4|35=D|34=2|52=20260102-10:15:30|11=HEADERLESS|55=AAPL|10=0|",
        ),
    ];
    let walked: Vec<_> = codec
        .lifecycle(sequenced)
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(
        walked
            .iter()
            .map(|message| message.header().msgseqnum())
            .collect::<Vec<_>>(),
        [Some(1), Some(2)],
        "different delivery sequence numbers remain distinct"
    );
}

#[test]
fn lifecycle_learns_in_event_order_and_fills_only_later_missing_instrument_codes() {
    let codec = codec();
    let later =
        b"8=FIX.4.4|35=D|49=S|56=T|34=2|52=20260102-10:15:31|11=LATER|isincode=US0378331005|10=0|";
    let earlier = b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|11=EARLIER|isincode=US0378331005|bloombergcode=AAPL US Equity|10=0|";

    let walked: Vec<_> = codec
        .lifecycle([codec.parse_fix_line(later), codec.parse_fix_line(earlier)])
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(walked.len(), 2);
    assert_eq!(walked[0].header().msgseqnum(), Some(1));
    assert_eq!(
        walked[1].get_securityids().get(&IdType::Bloomberg),
        Some("AAPL US Equity")
    );
    assert_eq!(
        codec.clone().with_snapshot_ns(1_000_000).snapshot_ns(),
        Some(1_000_000)
    );
    assert_eq!(codec.snapshot_ns(), None);
}

#[test]
fn lifecycle_learns_figi_by_isin_and_the_latest_statement_updates_it() {
    let codec = codec();
    let line = |seq: i32, body: &str| {
        format!(
            "8=FIX.4.4|35=D|49=S|56=T|34={seq}|52=20260102-10:15:{seq:02}|11={seq}|isincode=US0378331005|{body}|10=0|"
        )
    };
    let learned: Vec<_> = codec
        .lifecycle([
            codec.parse_fix_line(line(2, "").as_bytes()),
            codec.parse_fix_line(line(1, "figicode=BBG000BLNQ16").as_bytes()),
        ])
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(
        learned[1].get_securityids().get(&IdType::Figi),
        Some("BBG000BLNQ16")
    );

    // Two statements of the instrument disagreeing: the later one in event
    // order updates what the walk learned, whichever arrived first.
    let updated: Vec<_> = codec
        .lifecycle([
            codec.parse_fix_line(line(3, "").as_bytes()),
            codec.parse_fix_line(line(2, "figicode=BCG000000005").as_bytes()),
            codec.parse_fix_line(line(1, "figicode=BBG000BLNQ16").as_bytes()),
        ])
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(
        updated[2].get_securityids().get(&IdType::Figi),
        Some("BCG000000005"),
        "the latest statement updates the association"
    );
}

#[test]
fn a_shared_isin_registry_carries_what_one_walk_learned_into_the_next() {
    use std::sync::Mutex;
    use yggdryl::IsinRegistry;

    let line = |seq: i32, body: &str| {
        format!(
            "8=FIX.4.4|35=D|49=S|56=T|34={seq}|52=20260102-10:15:{seq:02}|11={seq}|isincode=US0378331005|{body}|10=0|"
        )
    };
    let walk = |codec: &FixCodec, seq: i32, body: &str| {
        let walked: Vec<_> = codec
            .lifecycle([codec.parse_fix_line(line(seq, body).as_bytes())])
            .collect::<yggdryl::Result<_>>()
            .unwrap();
        walked[0]
            .get_securityids()
            .get(&IdType::Figi)
            .map(ToOwned::to_owned)
    };
    let instruments = Arc::new(Mutex::new(IsinRegistry::new()));
    let shared = codec().with_isin_registry(Arc::clone(&instruments));
    assert_eq!(
        walk(&shared, 1, "figicode=BBG000BLNQ16").as_deref(),
        Some("BBG000BLNQ16")
    );
    assert_eq!(
        walk(&shared, 2, "").as_deref(),
        Some("BBG000BLNQ16"),
        "the second walk starts from what the first learned"
    );
    assert_eq!(
        instruments
            .lock()
            .unwrap()
            .get("US0378331005")
            .and_then(|entry| entry.get(&IdType::Figi)),
        Some("BBG000BLNQ16")
    );
    // Without one, each walk learns into its own, starting empty.
    let own = codec();
    assert_eq!(
        walk(&own, 1, "figicode=BBG000BLNQ16").as_deref(),
        Some("BBG000BLNQ16")
    );
    assert_eq!(walk(&own, 2, ""), None);
}

#[test]
fn lifecycle_learns_no_listing_a_bridge_names_its_instrument_by() {
    // `OMSINSTRUMENTID` states a listing - the ISIN, its market and its
    // currency - and one ISIN has as many listings as markets: what one
    // message names it by is no association to fill onto another. The
    // Bloomberg code beside it is the instrument's and is learned.
    let codec = codec();
    let line = |seq: i32, body: &str| {
        format!(
            "8=FIX.4.4|35=D|49=S|56=T|34={seq}|52=20260102-10:15:{seq:02}|11={seq}|isincode=CH0012214059|{body}|10=0|"
        )
    };
    let walked: Vec<_> = codec
        .lifecycle([
            codec.parse_fix_line(line(2, "").as_bytes()),
            codec.parse_fix_line(
                line(
                    1,
                    "bloombergcode=HOLN SW|OMSINSTRUMENTID=dbi;CH0012214059_XSWX_CHF",
                )
                .as_bytes(),
            ),
        ])
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(
        walked[0].get_securityids().get_from(&IdKey::new(
            "oms".parse::<IdSource>().unwrap(),
            IdType::InstrumentId
        )),
        Some("dbi;CH0012214059_XSWX_CHF"),
        "the message stating it keeps it"
    );
    assert_eq!(
        walked[1].get_securityids().get(&IdType::Bloomberg),
        Some("HOLN SW")
    );
    assert_eq!(walked[1].get_securityids().get(&IdType::InstrumentId), None);
}

#[test]
fn lifecycle_expiry_keeps_fix_content_and_retires_at_the_exact_deadline() {
    let codec = codec();
    let original = codec
        .parse_fix_line(
            b"8=FIX.4.4|35=D|49=S|56=T|34=1|52=20260102-10:15:30|126=20260102-10:15:32|11=EXP-1|55=AAPL|10=0|",
        )
        .unwrap();
    let walked: Vec<_> = codec
        .lifecycle([original.clone()])
        .collect::<yggdryl::Result<_>>()
        .unwrap();

    assert_eq!(walked.len(), 2);
    let (source, expired) = (&walked[0], &walked[1]);
    assert_eq!(source.get_curruuid(), original.get_curruuid());
    assert_eq!(
        source.entries(),
        original.entries(),
        "the yielded source is immutable"
    );
    assert_eq!(expired.get_currunix(), source.get_exprunix().unwrap());
    assert!(expired.get_state().is_failed());
    assert_eq!(expired.get_prevuuid(), Some(source.get_curruuid()));
    // The deadline (10:15:32) falls after the source's own instant
    // (10:15:30), so the expiry keeps the place it started at, zero.
    assert_eq!(expired.get_seqnum(), 0);
    assert!(Arc::ptr_eq(expired.registry(), source.registry()));
    assert_eq!(
        expired.entries(),
        source.entries(),
        "expiry changes no FIX content"
    );
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
        std::sync::Arc::new(yggdryl::text::TextOptions::new()),
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
            None => assert_eq!(held, super::decimal("420"), "tag {tag}"),
        }
    }
    assert_eq!(first_tag_value(&filled, 151), super::decimal("60"));
    // One fill, so the average is that fill's price.
    assert_eq!(first_tag_value(&filled, 6), super::decimal("10.5"));
    // Projected fields do not also remain in the residual record. The one
    // derived field this fixed schema does not project stays there instead,
    // so reconstructing the row cannot lose it.
    let residual = first_value(&filled, "fixentries");
    let entries = residual.as_mapping().expect("the residual map");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0.as_str(), Some("381:grosstradeamt"));
    assert_eq!(entries[0].1.as_str(), Some("420"));
}

/// The batch door reads a row as the line it is, and states that line as the
/// message's one source - the same identity the line door states for the same
/// bytes, instant, sequence and source-derived seed, so a message names its
/// line whichever door read it.
#[test]
fn the_batch_door_states_the_row_as_the_source_the_line_door_states() {
    const REPORT: &str = "8=FIX.4.4|35=8|39=1|150=F|38=100|14=40|32=40|31=10.5|54=1|10=0|";
    let codec = codec();
    let filled = codec
        .parse_text_arrow_reader(capture_reader(&[REPORT], 1))
        .unwrap()
        .map(std::result::Result::unwrap)
        .next()
        .expect("one batch");
    let schema = yggdryl::Field::from_arrow_schema("row", &filled.schema()).unwrap();
    let at =
        yggdryl::fix_column_of(&schema, yggdryl::SRCUUIDS_TAG_NAME.0).expect("a srcuuids column");
    let sources: Vec<yggdryl::Uuid> = first_at(&filled, at)
        .as_sequence()
        .expect("a list of sources")
        .iter()
        .map(|item| match item {
            Scalar::Uuid(uuid) => *uuid,
            other => panic!("a uuid, got {other:?}"),
        })
        .collect();
    // The row states no `mtime`, so the line is dated at the epoch - as the
    // line door dates one built from the same bytes under no handle.
    let line = TextLine::from_bytes(
        0,
        TextBytes::from_bytes(REPORT).unwrap(),
        Arc::new(yggdryl::text::TextOptions::new()),
    )
    .unwrap();
    assert_eq!(sources, [line.get_curruuid()]);
    let through_line = codec
        .parse_text_line(&line)
        .unwrap()
        .next()
        .expect("one message")
        .unwrap();
    assert_eq!(through_line.get_srcuuids(), sources.as_slice());
    let through_row = codec
        .messages(yggdryl::arrow::batch_reader(
            filled.schema(),
            [filled.clone()],
        ))
        .next()
        .expect("one message")
        .unwrap();
    assert_eq!(through_row.get_srcuuids(), sources.as_slice());
    assert_eq!(through_row.get_curruuid(), through_line.get_curruuid());
}

/// The three steps - the lines as a batch, the messages parsed out of it,
/// the lifecycle's rows - each open with the fifteen columns every event is
/// stated in, under one name and one datatype, and join on them: a
/// message's `srcuuids` is the `curruuid` its line's batch states, read off
/// that column rather than recomputed, and a chained message's `prevuuid`
/// is the `curruuid` of the message before it, its `state` the furthest the
/// chain reached.
#[test]
fn the_three_steps_join_on_the_columns_every_event_states() {
    use yggdryl::graph::{ElementColumn, EventColumn};

    const PLACED: &str = "8=FIX.4.4|35=D|11=C-1|37=A|39=0|54=1|38=100|10=0|";
    const FILLED: &str = "8=FIX.4.4|35=8|11=C-1|37=A|39=2|150=F|54=1|38=100|14=100|65009=20240102-10:15:30.100|10=0|";
    let codec = codec();
    let options = Arc::new(yggdryl::text::TextOptions::new());
    let mut lines: Vec<TextLine> = [PLACED, FILLED]
        .iter()
        .enumerate()
        .map(|(index, wire)| {
            let mut line = TextLine::from_bytes(
                index as u64,
                TextBytes::from_bytes(wire).unwrap(),
                Arc::clone(&options),
            )
            .unwrap();
            line.set_handle_mtime(Some(1_704_190_530_000_000_000 + index as i64));
            line
        })
        .collect();
    // An identity the carrier states and no recomputation of the bytes
    // would ever derive: what the message names is what the batch said.
    lines[1].set_curruuid(yggdryl::Uuid::from_v8(7));
    let identities: Vec<yggdryl::Uuid> = lines.iter().map(Element::get_curruuid).collect();

    // Step 1: the lines as a batch, opening with the fifteen as fields.
    let carrier = yggdryl::text::into_arrow_batch(lines.clone(), &options).unwrap();
    let stated = yggdryl::Field::from_arrow_schema("lines", &carrier.schema()).unwrap();
    let mut expected = ElementColumn::fields().unwrap();
    expected.extend(EventColumn::fields().unwrap());
    for (held, column) in stated.fields().iter().zip(&expected) {
        assert_eq!(held.name(), column.name());
        assert_eq!(held.dtype(), column.dtype(), "{}", column.name());
        assert_eq!(
            held.is_nullable(),
            column.is_nullable(),
            "{}",
            column.name()
        );
    }

    // Step 2: the messages parsed out of the batch, each stating the line
    // the carrier said it was as its one source, and the same fifteen under
    // the row's own names and datatypes.
    let parsed = codec
        .parse_text_arrow_reader(yggdryl::arrow::batch_reader(
            carrier.schema(),
            [carrier.clone()],
        ))
        .unwrap()
        .map(std::result::Result::unwrap)
        .next()
        .expect("one batch");
    let row = yggdryl::Field::from_arrow_schema("row", &parsed.schema()).unwrap();
    let columns = ElementColumn::ALL
        .into_iter()
        .map(|column| (column.name(), column.datatype()))
        .chain(
            EventColumn::ALL
                .into_iter()
                .map(|column| (column.name(), column.datatype())),
        );
    for (name, dtype) in columns {
        let held = row
            .field(name)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(held.dtype(), &dtype, "{name}");
    }
    let messages: Vec<FixMsg> = codec
        .messages(yggdryl::arrow::batch_reader(
            parsed.schema(),
            [parsed.clone()],
        ))
        .map(std::result::Result::unwrap)
        .collect();
    // The fill is its order's report and the execution split off it, which
    // names the report beside the line (A12).
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].get_srcuuids(), &identities[..1]);
    assert_eq!(messages[1].get_srcuuids(), [yggdryl::Uuid::from_v8(7)]);
    assert!(
        messages[2]
            .get_srcuuids()
            .contains(&yggdryl::Uuid::from_v8(7))
    );
    assert_eq!(messages[0].get_recdunix(), Some(1_704_190_530_000_000_000));
    assert_eq!(messages[1].get_recdunix(), Some(1_704_190_530_100_000_000));
    // The row's own seventeen name the message, never the line it came from.
    assert_ne!(messages[0].get_curruuid(), identities[0]);
    assert_eq!(messages[0].get_currunix(), 1_704_190_530_000_000_000);

    // Step 3: the lifecycle's rows carry the chain and the state it reached,
    // and a message read back off them keeps both.
    let walked = codec
        .lifecycle_arrow_reader(yggdryl::arrow::batch_reader(parsed.schema(), [parsed]))
        .unwrap()
        .map(std::result::Result::unwrap)
        .next()
        .expect("one batch");
    let chained: Vec<FixMsg> = codec
        .messages(yggdryl::arrow::batch_reader(walked.schema(), [walked]))
        .map(std::result::Result::unwrap)
        .collect();
    assert_eq!(
        chained.len(),
        3,
        "the order, its fill's report, the execution"
    );
    assert_eq!(chained[1].get_prevuuid(), Some(chained[0].get_curruuid()));
    // The report's line stamped one nanosecond after the order's, a later
    // instant of its own, so following the order leaves its place alone.
    assert_eq!(chained[1].get_seqnum(), 0);
    assert!(
        chained[1].get_state().is_done(),
        "{}",
        chained[1].get_state()
    );
    assert!(
        chained[0].get_state().is_live(),
        "{}",
        chained[0].get_state()
    );
    // Provenance travels along no chain: each keeps its own line.
    assert_eq!(chained[0].get_srcuuids(), &identities[..1]);
    assert_eq!(chained[1].get_srcuuids(), [yggdryl::Uuid::from_v8(7)]);
    assert_eq!(chained[0].get_recdunix(), Some(1_704_190_530_000_000_000));
    assert_eq!(
        chained[1].get_recdunix(),
        Some(1_704_190_530_100_000_000),
        "the recording clock survives lifecycle Arrow reread"
    );
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

/// The committed dictionary beside a venue's one field of its own on tag
/// 5001, a member of the `venue` dictionary, so a bridge row's `VENUETAG`
/// resolves in the one namespace whatever plugin the row names.
fn plugin_registry() -> Arc<FixRegistry> {
    let mut registry = registry().as_ref().clone();
    let mut field = DataType::utf8().nullable_field("VenueTag");
    FixFieldMut::new(&mut field).set_tag(5001).unwrap();
    FixFieldMut::new(&mut field).set_sources(["venue"]).unwrap();
    registry.insert(field).unwrap();
    Arc::new(registry)
}

/// The capture a bridge row header declares: the plugin that logged the line.
const PLUGIN_CAPTURES: [&str; 1] = ["msgpluginid"];

/// A bridge line naming the plugin that logged it, where it names one.
fn plugin_line(body: &[u8], plugin: Option<&str>) -> TextLine {
    let page = |text: &str| TextBytes::from_bytes(text.as_bytes()).unwrap();
    TextLine::from_bytes(
        0,
        TextBytes::from_bytes(body).unwrap(),
        std::sync::Arc::new(yggdryl::text::TextOptions::new()),
    )
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
fn a_rows_msgpluginid_fills_its_own_column_and_selects_no_dialect() {
    let registry = plugin_registry();
    let codec = super::fixed_codec(Arc::clone(&registry)).with_capture_names(PLUGIN_CAPTURES);
    let body: &[u8] = b"MSGTYPE=D|CLORDID=A|VENUETAG=dark";

    // Membership is provenance on the dictionary's field, never a namespace
    // the row is read under: the venue's field says which dictionary spoke
    // it, and the message root says nothing, because a message is not a
    // dictionary member.
    let venue = FixField::new(registry.field_by_tag(5001).unwrap());
    assert!(venue.has_source("venue"));
    assert_eq!(venue.sources().collect::<Vec<_>>(), ["venue"]);

    // Every way a row can name its plugin - the dictionary's own name, an
    // alias in another case, a plugin no dictionary is named after, and a
    // name too long to be one - reads the same: the venue's field resolves
    // in the one namespace, and the column fills the crate's own field
    // exactly as it was spelled.
    let long = "x".repeat(300);
    for spelled in ["venue", "VNU", "OMS_X1_TradeCapture", long.as_str()] {
        let message = one_of(&codec, &plugin_line(body, Some(spelled)));
        assert_eq!(
            FixField::new(message.as_field()).sources().count(),
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
                .by_tag(yggdryl::MSGPLUGINID_TAG_NAME.0)
                .unwrap()
                .as_str(),
            Some(spelled),
            "{spelled}: the fill is the text as spelled"
        );
        assert!(
            message
                .entries()
                .iter()
                .all(|entry| entry.tag() != yggdryl::MSGPLUGINID_TAG_NAME.0),
            "a fill is never an entry"
        );
    }

    // A plugin unstated fills nothing, and the row still reads the same.
    let message = one_of(&codec, &plugin_line(body, None));
    assert!(
        message
            .get_by_tag(yggdryl::MSGPLUGINID_TAG_NAME.0)
            .is_none_or(|held| held.is_null()),
        "nothing to fill from"
    );
    assert_eq!(message.by_tag(5001).unwrap().as_str(), Some("dark"));
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

    // The entries column is a map, not a payload: naming it is refused
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

/// A capture's own cells follow the message, not the row's position.
///
/// The walk answers messages in their own order, which a capture's lines are
/// routinely not in, and no message holds anything about the reading it
/// arrived through. So the door has to carry each row's own cells beside the
/// message it made and state them again where that message lands: pairing by
/// position would hand row two's `body` to row one's message.
#[test]
fn a_walk_keeps_each_rows_own_cells_with_its_own_message() {
    let codec = codec();
    // Three messages of one order, written to the log in the reverse of the
    // order they happened in - which is what makes the walk reorder them.
    let lines = [
        "line-2 8=FIX.4.4|35=8|11=WALK-1|37=O1|17=E2|150=F|39=2|55=AAPL|52=20260102-10:15:32.000|10=0|",
        "line-1 8=FIX.4.4|35=8|11=WALK-1|37=O1|17=E1|150=0|39=0|55=AAPL|52=20260102-10:15:31.000|10=0|",
        "line-0 8=FIX.4.4|35=D|11=WALK-1|55=AAPL|54=1|38=100|52=20260102-10:15:30.000|10=0|",
    ];
    let parsed = codec
        .parse_text_arrow_reader(capture_reader(&lines, lines.len()))
        .unwrap();
    let schema = parsed.schema();
    let held = batches(parsed);
    // The fill's line is two rows: its report and its execution (A12).
    assert_eq!(row_count(&held), 4);

    // The reader stated the body of each line in the row it made: the parse
    // door is the one door that can, because the message holds none of it.
    let body_of = |batch: &RecordBatch, row: usize| {
        let columns = rows_of(batch)[row].clone();
        let columns = columns.as_sequence().expect("columns").to_vec();
        let at = batch.schema().index_of("body").expect("a body column");
        let exec = batch.schema().index_of("execid").expect("an execid column");
        let ordtype = batch
            .schema()
            .index_of("clordid")
            .expect("a clordid column");
        (
            String::from_utf8_lossy(columns[at].as_bytes().expect("bytes")).into_owned(),
            columns[exec].as_str().map(ToOwned::to_owned),
            columns[ordtype].as_str().map(ToOwned::to_owned),
        )
    };
    let read: Vec<_> = (0..3).map(|row| body_of(&held[0], row)).collect();
    assert!(read[0].0.starts_with("line-2"), "{:?}", read[0]);

    // Walked, the rows come back in the order the messages happened in - and
    // each still carries the line it was read from.
    let walked = codec
        .lifecycle_arrow_reader(yggdryl::arrow::batch_reader(schema.clone(), held.clone()))
        .unwrap();
    assert_eq!(walked.schema(), schema, "the same schema in and out");
    let after = batches(walked);
    assert_eq!(row_count(&after), 4);
    let after: Vec<_> = (0..4).map(|row| body_of(&after[0], row)).collect();
    assert_eq!(
        after.iter().map(|held| held.0.clone()).collect::<Vec<_>>(),
        vec![
            "line-0 ".to_owned() + lines[2].trim_start_matches("line-0 "),
            "line-1 ".to_owned() + lines[1].trim_start_matches("line-1 "),
            "line-2 ".to_owned() + lines[0].trim_start_matches("line-2 "),
            "line-2 ".to_owned() + lines[0].trim_start_matches("line-2 "),
        ],
        "the walk reordered the rows",
    );
    // And the line each row holds is the line its own message came off: the
    // order's row carries no ExecID, the two reports carry their own, and
    // the execution split off the fill carries its report's line.
    assert_eq!(after[0].1, None, "the order states no ExecID");
    assert_eq!(after[1].1.as_deref(), Some("E1"));
    assert_eq!(after[2].1.as_deref(), Some("E2"));
    assert_eq!(after[3].1.as_deref(), Some("E2"));
    for held in &after {
        assert_eq!(held.2.as_deref(), Some("WALK-1"));
    }
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
fn a_payload_column_spelled_msgpluginid_is_the_payload_and_fills_no_plugin() {
    // The column [`FixCodec::with_payload_column`] names is the payload and
    // nothing else, whatever it is spelled: its text never fills the crate's
    // own `msgpluginid` field, or a capture whose payload column happened to be
    // spelled so would stamp every line with whatever its first bytes were.
    let registry = plugin_registry();
    let codec = super::fixed_codec(registry).with_payload_column("msgpluginid");
    // One payload spelling a dictionary's name exactly, and one that is a
    // frame, so the column is proven to be read as the payload.
    let bodies = ["venue", "8=FIX.4.4|35=D|11=A|10=0|"];

    // The line door has no payload column to spell at all: a line's body is a
    // typed field, so the same two bodies read as bodies there and nothing
    // about them can reach the plugin.
    let lines: Vec<TextLine> = bodies
        .iter()
        .map(|body| {
            TextLine::from_bytes(
                0,
                TextBytes::from_bytes(body.as_bytes()).unwrap(),
                std::sync::Arc::new(yggdryl::text::TextOptions::new()),
            )
            .unwrap()
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

    let capture = StructType::from_fields([DataType::utf8().required_field("msgpluginid")])
        .map(DataType::from)
        .unwrap()
        .required_field("capture");
    let values = Scalar::from_sequence(
        bodies
            .iter()
            .map(|body| Scalar::from_sequence([Scalar::from(*body)]))
            .collect::<Vec<_>>(),
    );
    let batch = lay_out(&capture, &values);
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
        assert_eq!(FixField::new(message.as_field()).sources().count(), 0);
        assert!(
            message
                .get_by_tag(yggdryl::MSGPLUGINID_TAG_NAME.0)
                .is_none_or(|held| held.is_null()),
            "the payload never fills the plugin"
        );
        assert!(message.get_by_tag(5001).is_none());
    }
    assert_eq!(alone.entries(), streamed[0].entries());
    // The frame was read as the body; the dictionary's name held nothing to read.
    assert_eq!(alone.by_tag(11).unwrap().as_str(), Some("A"));
}

/// The codec a fixture of every shape is read under.
///
/// `DEFAULT_REFUSED_MSGTYPES` is what a capture of live session traffic
/// wants and the opposite of what a fixture of every shape wants: a corpus
/// written to hold a heartbeat and a row that states no type at all is
/// asking for those to be read, and says so here.
fn every_msgtype() -> FixCodec {
    codec().with_exclude_msgtypes::<[&str; 0], &str>([])
}

#[test]
fn the_batch_door_refuses_the_types_the_line_door_refuses() {
    // The corpus holds three rows that state no type - two bridge rows and
    // the `<Execution>` document - and a codec refuses `unknown` until a
    // caller says otherwise, at the batch door as at the line door.
    let refused = batches(codec().parse_text_arrow_reader(source()).unwrap());
    let read = batches(every_msgtype().parse_text_arrow_reader(source()).unwrap());
    assert_eq!(row_count(&refused), 6);
    assert_eq!(row_count(&read), 9);
    assert!(yggdryl::DEFAULT_REFUSED_MSGTYPES.contains(&"unknown"));
    assert!(!codec().reads_msgtype("unknown"));
    assert!(every_msgtype().reads_msgtype("unknown"));

    // The two doors refuse the same lines: what the batch door dropped is
    // what the line door drops.
    let lines = codec().parse_lines(CAPTURE).count();
    assert_eq!(lines, row_count(&refused));
}

#[test]
fn a_capture_answers_one_row_per_message_and_a_sentence_is_no_row() {
    let batches = batches(every_msgtype().parse_text_arrow_reader(source()).unwrap());
    // A FIX batch answers one row per *message*, and five of the fourteen
    // lines carry none, so they carry no row either - the text reader is the
    // one that answers a row per line. The silent five, and why each states
    // no message:
    //
    // - `After Enrichment -> ACCOUNT=... CLIENTID=... VENUE=...` separates its
    //   named pairs with whitespace alone and marks no key, so the run is
    //   prose carrying an `=` rather than a bridge row;
    // - `Referential(dbi|equity|...|[quantity-type=])` holds every pipe in
    //   front of its one `=`, so nothing names a separator for a run of pairs;
    // - `Message rejected because : ...` and `no level printed by this plugin`
    //   hold no `=` at all;
    // - `heartbeat emitted seq=7` states its one pair on whitespace, unmarked.
    assert_eq!(
        row_count(&batches),
        9,
        "nine of the {} lines carry a message",
        CAPTURE.len()
    );

    // A row that states no type is still a row, named `unknown` - but only a
    // bridge row or a document ever is, because a sentence is not a row.
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
fn a_document_row_is_one_unknown_row_carrying_its_source_columns_and_stated_direction() {
    let field = StructType::from_fields([
        DataType::Int64.required_field("rownum"),
        DataType::utf8().required_field("msgdirection"),
        DataType::binary().required_field("body"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("capture");
    let rows = Scalar::from_sequence([Scalar::from_sequence([
        Scalar::from(42_i64),
        Scalar::from("Receive"),
        Scalar::from(BULK_CONFIG.to_vec()),
    ])]);
    let source = lay_out(&field, &rows);
    // One byte a batch: a batch a message, so a row that expanded would be
    // seen as the several batches it made. A bulk document expands into
    // nothing: one `unknown` row, carrying the source row's own columns.
    let reader = super::fixed_codec(direction_registry())
        .with_exclude_msgtypes::<[&str; 0], &str>([])
        .with_batch_byte_size(1)
        .parse_text_arrow_reader(yggdryl::arrow::batch_reader(source.schema(), [source]))
        .unwrap();
    let batches = batches(reader);
    assert_eq!(batches.len(), 1, "one row for the one document");
    let batch = &batches[0];
    assert_eq!(batch.num_rows(), 1);
    assert_eq!(first_value(batch, "rownum"), Scalar::from(42_i64));
    // A stated column is a spelling of a code of the set, stored as the code.
    assert_eq!(first_tag_value(batch, 385).as_str(), Some("R"));
}

#[test]
fn a_captures_own_columns_follow_the_shared_ones_and_a_clash_yields_to_fix() {
    // Shaped the way the text line reader shapes a capture: where the line was
    // read from, which line it was, what stamped it, and the frame itself.
    let capture = StructType::from_fields([
        DataType::utf8().required_field("url"),
        DataType::Int64.required_field("rownum"),
        DataType::utf8().nullable_field("threadname"),
        DataType::binary().required_field("body"),
        // A name a FIX column already takes, which yields to it: one column
        // per name, and the FIX one is what a reader spelling it means.
        DataType::utf8().nullable_field("fixentries"),
    ])
    .map(DataType::from)
    .expect("a capture root")
    .required_field("line");

    let rows = Scalar::from_sequence([Scalar::from_sequence([
        Scalar::from("file:///capture.log"),
        Scalar::from(7_i64),
        Scalar::from("session-a"),
        Scalar::from(b"8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|10=0|".to_vec()),
        Scalar::from("ignored"),
    ])]);
    let batch = lay_out(&capture, &rows);
    let schema = batch.schema();
    let read = codec()
        .parse_text_arrow_reader(yggdryl::arrow::batch_reader(schema, [batch]))
        .expect("a carried reader");
    let columns: Vec<String> = read
        .schema()
        .fields()
        .iter()
        .map(|held| held.name().clone())
        .collect();
    let shared = columns
        .iter()
        .position(|held| held == "partyids")
        .expect("the shared columns")
        + 1;
    assert_eq!(columns[0], "curruuid", "the shared columns open the row");
    assert_eq!(
        &columns[shared..shared + 4],
        ["url", "rownum", "threadname", "body"],
        "the capture follows the shared columns"
    );
    assert_eq!(
        columns.iter().filter(|held| *held == "fixentries").count(),
        1,
        "the clashing capture column yielded to the FIX one"
    );

    let read = batches(read);
    assert_eq!(row_count(&read), 1);
    let held = rows_of(&read[0]);
    let row = held[0].as_sequence().expect("its columns").to_vec();
    assert_eq!(row[shared].as_str(), Some("file:///capture.log"));
    assert_eq!(row[shared + 1].as_i64(), Some(7));
    assert_eq!(row[shared + 2].as_str(), Some("session-a"));
    let at = columns
        .iter()
        .position(|held| held == "msgtype")
        .expect("the msgtype column");
    assert_eq!(row[at].as_str(), Some("D"), "and FIX filled its own");
}

/// A batch lands before its first row is read: a `state` cell no `State`
/// accepts is refused at the landing, and costs its own row alone - the
/// row is excluded with a warning naming its column, never read as a
/// message, and every other row of the batch reads.
#[test]
fn a_fix_batch_row_with_an_invalid_state_code_is_excluded_at_the_landing() {
    use arrow_array::cast::AsArray as _;

    // A code no member of the state enum takes.
    const FOREIGN: u16 = 7;
    let codec = codec();
    let read = batches(codec.parse_text_arrow_reader(source()).unwrap());
    let batch = &read[0];
    assert!(batch.num_rows() > 1, "a second row to refuse");
    // Untouched, the batch reads back as its messages.
    let intact = codec
        .messages(yggdryl::arrow::batch_reader(
            batch.schema(),
            [batch.clone()],
        ))
        .collect::<yggdryl::Result<Vec<FixMsg>>>()
        .expect("the batch's own rows");
    assert_eq!(intact.len(), batch.num_rows());

    // The state column is `uint16` codes under the state extension, so a
    // code no member takes is an integer the layout holds and the datatype
    // refuses.
    let at = batch.schema().index_of("state").expect("a state column");
    let mut codes: Vec<Option<u16>> = batch
        .column(at)
        .as_primitive::<arrow_array::types::UInt16Type>()
        .iter()
        .collect();
    codes[1] = Some(FOREIGN);
    let mut columns = batch.columns().to_vec();
    columns[at] = Arc::new(arrow_array::UInt16Array::from(codes));
    let forged = RecordBatch::try_new(batch.schema(), columns).unwrap();
    let read = codec
        .messages(yggdryl::arrow::batch_reader(forged.schema(), [forged]))
        .collect::<yggdryl::Result<Vec<FixMsg>>>()
        .expect("a refused row is no error");
    let kept: Vec<_> = intact
        .iter()
        .enumerate()
        .filter(|(at, _)| *at != 1)
        .map(|(_, message)| message.clone())
        .collect();
    assert_eq!(read, kept, "every row but the refused one");

    // A capture carrying a state column lands the same way at the parse door.
    let capture = StructType::from_fields([
        DataType::binary().required_field("body"),
        DataType::State.nullable_field("state"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("capture");
    let frame = b"8=FIX.4.4|35=D|11=A|10=0|".as_slice();
    let carried = RecordBatch::try_new(
        capture.into_arrow_schema().unwrap(),
        vec![
            Arc::new(arrow_array::BinaryArray::from_vec(vec![frame; 2])),
            Arc::new(arrow_array::UInt16Array::from(vec![
                yggdryl::State::New.code(),
                FOREIGN,
            ])),
        ],
    )
    .unwrap();
    let parsed = codec
        .parse_arrow_messages(yggdryl::arrow::batch_reader(carried.schema(), [carried]))
        .unwrap()
        .collect::<yggdryl::Result<Vec<FixMsg>>>()
        .expect("a refused row is no error");
    assert_eq!(parsed.len(), 1, "the first row alone");
    assert_eq!(parsed[0].by_tag(11).unwrap().as_str(), Some("A"));
}

#[test]
fn the_line_read_and_the_batch_read_agree_on_separatorless_group_inference() {
    const BRIDGE: &str = "|#SYMBOL=TTF|#SIDE=1|#PRICE=41.25|#NOPARTYIDS=1\
|#NOPARTYIDS[0]=PARTYID=BUYSIDEPARTYIDSOURCE=DPARTYROLE=1|";

    // The row states no type, so the fixture says it wants it read.
    let codec = every_msgtype();
    let schema = fix_schema(codec.registry(), "fix").unwrap();
    let rows = codec
        .arrow_reader(schema, codec.parse_lines([BRIDGE]))
        .expect("a line reader");
    let row_batch = rows.into_iter().next().unwrap().unwrap();

    let columns = codec
        .parse_text_arrow_reader(capture_reader(&[BRIDGE], 1))
        .expect("a payload-column reader");
    let column_batch = columns.into_iter().next().unwrap().unwrap();

    // Both reads infer the one occurrence the separatorless group holds:
    // the party it names, and the group itself among the row's entries as
    // the wire stated it.
    for batch in [&row_batch, &column_batch] {
        let partyids = yggdryl::Identifiers::from_scalar(&first_value(batch, "partyids"))
            .expect("the partyids column");
        assert_eq!(
            partyids.to_string(),
            "[executingfirm=BUYSIDE, proprietary:executingfirm=BUYSIDE]"
        );
        let entries = first_value(batch, yggdryl::fix::FIXENTRIES_COLUMN);
        let group = entries
            .mapping_iter()
            .find(|(key, _)| key.as_str() == Some("453:parties"))
            .map(|(_, value)| value.as_str().expect("the group as JSON").to_owned());
        assert_eq!(
            group.as_deref(),
            Some(r#"[{"447:partyidsource":"D","448:partyid":"BUYSIDE","452:partyrole":"1"}]"#)
        );
    }
}

#[test]
fn a_message_through_the_arrow_reader_and_back_states_the_same_semantic_row() {
    let codec = every_msgtype().with_null_values::<[&str; 0], &str>([]);
    let schema = fix_schema(codec.registry(), "fix").unwrap();
    let parsed: Vec<FixMsg> = codec
        .parse_lines(CAPTURE)
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(parsed.len(), 9);

    let reader = codec
        .arrow_reader(schema.clone(), parsed.clone().into_iter().map(Ok))
        .unwrap();
    let again: Vec<FixMsg> = codec
        .messages(reader)
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(again.len(), parsed.len());
    for (held, message) in again.iter().zip(&parsed) {
        // Represented fields rebuild from their columns; the residual keeps
        // only content those columns could not state.
        assert_eq!(
            held.into_row(&schema).unwrap(),
            message.into_row(&schema).unwrap(),
            "semantic row"
        );
        assert_eq!(held.get_curruuid(), message.get_curruuid());
        assert_eq!(held.get_crossuuid(), message.get_crossuuid());
        assert_eq!(held.get_currhashcode(), message.get_currhashcode());
        assert_eq!(held.get_crosshashcode(), message.get_crosshashcode());
        for tag in [35, 11, 55, 54, 17, 37] {
            fn stated(message: &FixMsg, tag: i32) -> Option<Scalar> {
                message.get_by_tag(tag).filter(|held| !held.is_null())
            }
            assert_eq!(stated(held, tag), stated(message, tag), "tag {tag}");
        }
    }
}

#[test]
fn a_stream_carries_nothing_from_a_document_to_the_rows_after_it() {
    let codec = super::fixed_codec(Arc::new(FixRegistry::new()))
        .with_exclude_msgtypes::<[&str; 0], &str>([])
        .with_capture_names(["msgpluginid"]);
    let line = |body: &[u8]| {
        TextLine::from_bytes(
            0,
            TextBytes::from_bytes(body).unwrap(),
            std::sync::Arc::new(yggdryl::text::TextOptions::new()),
        )
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
            .by_tag(yggdryl::MSGPLUGINID_TAG_NAME.0)
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
            .by_tag(yggdryl::MSGPLUGINID_TAG_NAME.0)
            .unwrap()
            .as_str(),
        Some("STREAM")
    );
    assert!(filled.get_by_tag(49).is_none_or(|held| held.is_null()));
    assert!(filled.get_by_tag(56).is_none_or(|held| held.is_null()));
    assert!(stream.next().is_none());
}

#[test]
fn a_dated_capture_reads_a_retired_spelling_and_the_fact_it_names_is_the_events() {
    let codec = codec().with_capture_names(["beginstring", "msgpluginid"]);
    let page = |bytes: &[u8]| TextBytes::from_bytes(bytes).unwrap();
    let line = |body: &[u8], captures: [Option<&str>; 2]| {
        TextLine::from_bytes(
            0,
            page(body),
            std::sync::Arc::new(yggdryl::text::TextOptions::new()),
        )
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
    // code spelling answers - and never what a fact is called: `LastShares`
    // is 4.2's spelling of tag 32, and tag 32 is the event's own quantity
    // whatever version the row states, so it is read off the event rather
    // than found as a child of the content row.
    let old = one(&line(
        b"8=FIX.4.4|35=8|32=100|10=0|",
        [Some("FIX.4.2"), None],
    ));
    assert!(old.as_field().index_of("lastqty").is_none());
    assert_eq!(old.by_tag(32).unwrap(), super::decimal("100"));

    // A capture absent, unmatched or empty is silence, never an instruction
    // and never an error.
    let quiet = one(&line(b"8=FIX.4.4|35=D|11=A|10=0|", [None, Some("")]));
    assert_eq!(quiet.as_field().name(), "D");
    assert_eq!(
        FixField::new(quiet.as_field()).sources().count(),
        0,
        "a plugin names no dialect: a message is not a dictionary member"
    );
}

/// The bundled bridge capture as the text rows a read answers: its own row
/// header, each line numbered, the clock read in UTC.
fn bridge_capture() -> (yggdryl::holder::Buffer, yggdryl::media::RecordOptions) {
    let source = yggdryl::holder::Buffer::from_bytes(include_bytes!("ulbridge.log").to_vec())
        .with_media_type(
            yggdryl::Url::from_str("file:///ulbridge.log")
                .expect("a URL")
                .media_type(),
        );
    let mut options = yggdryl::text::TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the bridge's row header compiles")
        .with_timezone(yggdryl::Timezone::UTC);
    options.start_rownum = Some(1);
    (source, options.into())
}

/// Every record a serie face answered, as the batch it is.
fn serie_batches(reader: yggdryl::StreamChunkedSerie) -> Vec<RecordBatch> {
    reader
        .into_chunks()
        .map(|record| {
            record
                .expect("a record")
                .into_arrow_batch()
                .expect("a record column is a table")
        })
        .collect()
}

/// What identifies a message and everything it states.
fn stated(messages: impl Iterator<Item = yggdryl::Result<FixMsg>>) -> Vec<(yggdryl::Uuid, u128)> {
    messages
        .map(|message| {
            let message = message.expect("a message");
            (message.get_curruuid(), message.digest())
        })
        .collect()
}

#[test]
fn each_serie_face_yields_exactly_the_rows_its_arrow_door_yields() {
    use yggdryl::IOMedia;
    let codec = codec();
    let (source, options) = bridge_capture();
    let text = || source.read_arrow_reader(&options).expect("a text reader");
    let read = || {
        yggdryl::StreamChunkedSerie::from_serie(
            source.read_serie(Some(&options)).expect("a text serie"),
        )
        .expect("native record stream")
    };

    // Text rows in, FIX rows out: the root the Arrow door writes, declaring
    // no order, and its batches row for row.
    let parsed = batches(codec.parse_text_arrow_reader(text()).unwrap());
    assert!(row_count(&parsed) > 0, "the capture parses");
    let faced = codec.parse_text_serie(read()).unwrap();
    let root = yggdryl::Field::from_arrow_schema("fix", &parsed[0].schema()).unwrap();
    assert_eq!(faced.field(), &root);
    assert!(!faced.field().as_sort().declares_order());
    assert_eq!(serie_batches(faced), parsed);
    let fix_rows = || yggdryl::arrow::batch_reader(parsed[0].schema(), parsed.clone());

    // FIX rows in, walked FIX rows out - fed by the face before it, which
    // crosses as the door's own reader.
    let walked = batches(codec.lifecycle_arrow_reader(fix_rows()).unwrap());
    let faced = codec
        .lifecycle_serie(codec.parse_text_serie(read()).unwrap())
        .unwrap();
    assert_eq!(faced.field(), &root);
    assert_eq!(serie_batches(faced), walked);

    // FIX rows in, market data rows out.
    let market = batches(codec.market_data_arrow_reader(fix_rows()).unwrap());
    assert!(row_count(&market) > 0, "the capture holds market data");
    let faced = codec
        .market_data_serie(codec.parse_text_serie(read()).unwrap())
        .unwrap();
    assert_eq!(faced.field(), &yggdryl::graph::MarketData::field().unwrap());
    assert!(!faced.field().as_sort().declares_order());
    assert_eq!(serie_batches(faced), market);

    // FIX rows in, messages out: what a table read back feeds the walk.
    let messages = stated(codec.messages(fix_rows()));
    assert_eq!(messages.len(), row_count(&parsed));
    let faced = codec
        .messages_serie(codec.parse_text_serie(read()).unwrap())
        .unwrap();
    assert_eq!(stated(faced), messages);

    // Messages in, FIX rows out, under the root the caller names.
    let written = batches(
        codec
            .arrow_reader(root.clone(), codec.messages(fix_rows()))
            .unwrap(),
    );
    let faced = codec
        .chunked_stream(root.clone(), codec.messages(fix_rows()))
        .unwrap();
    assert_eq!(faced.field(), &root);
    assert_eq!(serie_batches(faced), written);

    // Messages in, market data and books out.
    let market = batches(
        codec
            .market_arrow_reader(codec.messages(fix_rows()))
            .unwrap(),
    );
    let faced = codec.market_serie(codec.messages(fix_rows())).unwrap();
    assert_eq!(serie_batches(faced), market);
    let books = batches(
        codec
            .book_arrow_reader(codec.messages(fix_rows()), 900_000, None)
            .unwrap(),
    );
    let faced = codec
        .book_serie(codec.messages(fix_rows()), 900_000, None)
        .unwrap();
    assert_eq!(serie_batches(faced), books);
}

#[test]
fn the_serie_faces_answer_alike_on_several_threads() {
    use yggdryl::IOMedia;
    let one = codec();
    let four = codec().with_threads(4);
    let (source, options) = bridge_capture();
    let read = || {
        yggdryl::StreamChunkedSerie::from_serie(
            source.read_serie(Some(&options)).expect("a text serie"),
        )
        .expect("native record stream")
    };
    let walk = |codec: &FixCodec| {
        serie_batches(
            codec
                .lifecycle_serie(codec.parse_text_serie(read()).unwrap())
                .unwrap(),
        )
    };
    assert_eq!(walk(&four), walk(&one));
}

#[test]
fn a_walk_states_no_order_its_source_declared() {
    // The walk answers in its own order and may date a message again, so a
    // `SORT:by` on the rows it read is not a fact of the rows it writes.
    let codec = codec();
    let parsed = batches(codec.parse_text_arrow_reader(source()).unwrap());
    let mut root = yggdryl::Field::from_arrow_schema("fix", &parsed[0].schema()).unwrap();
    root.as_sort_mut().set_by_texts(["currunix"]).unwrap();
    let schema = root.clone().into_arrow_schema().unwrap();
    let declared: Vec<RecordBatch> = parsed
        .iter()
        .map(|batch| batch.clone().with_schema(schema.clone()).unwrap())
        .collect();

    let walked = codec
        .lifecycle_arrow_reader(yggdryl::arrow::batch_reader(
            schema.clone(),
            declared.clone(),
        ))
        .unwrap();
    assert!(!walked.schema().metadata().contains_key("SORT:by"));
    assert!(row_count(&batches(walked)) > 0);

    let rows = yggdryl::StreamChunkedSerie::from_arrow_reader(
        None,
        yggdryl::arrow::batch_reader(schema, declared),
        yggdryl::ArrowCastOptions::new(),
    )
    .unwrap();
    assert!(rows.field().as_sort().declares_order());
    let faced = codec.lifecycle_serie(rows).unwrap();
    assert!(!faced.field().as_sort().declares_order());
    assert!(!serie_batches(faced).is_empty());
}

#[test]
fn a_serie_face_refuses_a_run_before_a_row_is_read() {
    let run = yggdryl::Serie::from(yggdryl::Run::new(vec![Scalar::from(1_i64)]));
    assert!(codec().parse_text_serie(run.clone()).is_err());
    assert!(codec().lifecycle_serie(run.clone()).is_err());
    assert!(codec().market_data_serie(run.clone()).is_err());
    assert!(codec().messages_serie(run).is_err());
}

/// A bridge line's thread and level are the capture's own columns: no
/// dictionary field is named for either, so the batch door carries both in
/// front of every message the row answers for, as the row's own cells -
/// outside the content, the entries, the wire and the digest - and the
/// rows read back carry them again.
#[test]
fn a_bridge_lines_thread_and_level_ride_its_fix_row_and_fill_nothing() {
    use yggdryl::IOMedia;
    let codec = codec();
    for own in ["msgthreadid", "loglevel"] {
        assert!(
            codec.registry().get_field_by_name(own).is_none(),
            "{own} names no field, which is what keeps it out of every fill"
        );
    }
    let (source, options) = bridge_capture();
    let text = source.read_arrow_reader(&options).unwrap();
    let text_schema = text.schema();
    assert_eq!(
        text_schema
            .field_with_name("msgthreadid")
            .unwrap()
            .data_type(),
        &arrow_schema::DataType::Int64
    );
    assert_eq!(
        text_schema.field_with_name("loglevel").unwrap().data_type(),
        &arrow_schema::DataType::Utf8
    );
    let parsed = codec.parse_text_arrow_reader(text).unwrap();
    let root = yggdryl::Field::from_arrow_schema("fix", &parsed.schema()).unwrap();
    let body = root.index_of("body").unwrap();
    assert_eq!(root.index_of("msgthreadid"), Some(body + 1));
    assert_eq!(root.index_of("loglevel"), Some(body + 2));
    let tags = yggdryl::fix_column_tags(&root);
    for own in ["msgthreadid", "loglevel"] {
        let at = root.index_of(own).unwrap();
        assert_eq!(tags[at], None, "{own}");
        assert!(root.fields()[at].is_nullable(), "{own}");
    }
    // Every message carries both cells, and neither reaches the wire.
    let messages: Vec<FixMsg> = codec
        .messages(parsed)
        .map(|message| message.expect("a message"))
        .collect();
    assert!(!messages.is_empty());
    for message in &messages {
        let carried: Vec<&str> = message
            .carried()
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert!(carried.contains(&"msgthreadid"), "{carried:?}");
        assert!(carried.contains(&"loglevel"), "{carried:?}");
        let wire = String::from_utf8(message.into_bytes(b'|')).unwrap();
        assert!(
            !wire.contains("msgthreadid") && !wire.contains("loglevel"),
            "{wire}"
        );
    }
}

/// The Arrow batch door stamps the codec's plugin role as the line doors
/// do: every message read out of a capture batch under a source states the
/// source's side as its `msgpluginside`, the fixed row the text reader lays
/// out carries the cell, and a codec told no source stamps `UKNW`.
#[test]
fn the_arrow_batch_door_stamps_the_codecs_plugin_role_on_every_message() {
    use yggdryl::{FixSource, MSGPLUGINSIDE_TAG_NAME, Side};

    let mut registry = registry().as_ref().clone();
    assert!(registry.add_source(FixSource::new("ms").unwrap().with_pluginside(Side::Sell)));
    let registry = Arc::new(registry);
    let lines = [
        "8=FIX.4.4|35=D|11=A|55=AAPL|54=1|38=5|40=2|44=100|52=20240102-10:15:30|10=0|",
        "8=FIX.4.4|35=D|11=B|55=AAPL|54=2|38=5|40=2|44=100|52=20240102-10:15:31|10=0|",
    ];
    for (source, side) in [(Some("MS"), Side::Sell), (None, Side::Unknown)] {
        let mut codec = super::fixed_codec(Arc::clone(&registry));
        if let Some(id) = source {
            codec = codec.with_source(id).expect("a held source");
        }
        let messages = codec
            .parse_arrow_messages(capture_reader(&lines, 1))
            .expect("a readable capture")
            .collect::<yggdryl::Result<Vec<FixMsg>>>()
            .expect("the messages");
        assert_eq!(messages.len(), 2);
        for message in &messages {
            assert_eq!(message.msgpluginside(), side, "{source:?}");
            assert_eq!(
                message.get_by_tag(MSGPLUGINSIDE_TAG_NAME.0),
                Some(Scalar::from(side))
            );
        }
        let parsed = batches(
            codec
                .parse_text_arrow_reader(capture_reader(&lines, 2))
                .expect("a readable capture"),
        );
        assert_eq!(row_count(&parsed), 2);
        let cells = column(&parsed[0], "msgpluginside")
            .as_any()
            .downcast_ref::<arrow_array::UInt8Array>()
            .expect("the enum's uint8 codes");
        assert_eq!(cells.values().as_ref(), [side.code(), side.code()]);
    }
}
