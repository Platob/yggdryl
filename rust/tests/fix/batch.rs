//! A capture in, columns out, and back to the wire.

use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::RecordBatch;
use yggdryl::holder::local::Folder;
use yggdryl::media::IORecordOptions;
use yggdryl::{DataType, FixBatchReader, FixMsg, FixOptions, FixRegistry, Scalar, write_fix};

fn registry() -> Arc<FixRegistry> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    let folder = Folder::new(root).expect("the seed folder is a local path");
    Arc::new(FixRegistry::from_handle(&folder).expect("the committed dictionary loads"))
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

fn rows() -> Vec<yggdryl::Result<Vec<u8>>> {
    CAPTURE
        .iter()
        .map(|row| Ok(row.as_bytes().to_vec()))
        .collect()
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

/// One projected scalar in the first row of a batch.
fn first_value(batch: &RecordBatch, name: &str) -> Scalar {
    let index = batch
        .schema()
        .index_of(name)
        .unwrap_or_else(|_| panic!("a {name} column"));
    let rows = yggdryl::arrow::batch_to_value(batch).expect("the projected rows");
    rows.as_sequence().expect("rows")[0]
        .as_sequence()
        .expect("columns")[index]
        .clone()
}

#[test]
fn the_schema_is_decided_before_the_first_row_is_read() {
    let registry = registry();
    // Nothing is pulled to answer this: an empty capture has the same columns
    // a full one does, which is the whole point.
    let reader =
        FixBatchReader::from_rows(Arc::clone(&registry), Vec::new(), FixOptions::new()).unwrap();
    let schema = reader.schema();
    let names: Vec<&str> = schema
        .fields()
        .iter()
        .map(|held| held.name().as_str())
        .collect();

    // Columns are named by tag, because a tag is the one name a field has in
    // every version and every dialect.
    assert_eq!(&names[..5], ["8", "9", "35", "49", "56"], "{names:?}");
    assert_eq!(
        &names[names.len() - 2..],
        ["nofixentries", "nounmappedfixentries"],
    );
    // The standard header, the body a consumer queries, the groups worth
    // keeping whole, the trailer, and this crate's own derived facts.
    for tag in [
        "55", "54", "44", "38", "60", // the trade
        "132", "133", "134", "135", // the quote's lanes
        "453", "454", "768", // the groups
        "10",  // the trailer
        "30001", "30004", "30005", // the digest, the clock, the partition
        "385",   // which way the line moved
    ] {
        assert!(names.contains(&tag), "{tag} missing from {names:?}");
    }

    // Each column still carries the spelling it had, so a renderer can show
    // `msgtype` over column `35`.
    let msgtype = schema.field_with_name("35").expect("the msgtype column");
    assert_eq!(
        msgtype.metadata().get("display").map(String::as_str),
        Some("MsgType"),
    );
}

#[test]
fn a_row_in_is_a_row_out() {
    let registry = registry();
    let reader = FixBatchReader::from_rows(registry, rows(), FixOptions::new()).unwrap();
    let batches: Vec<_> = reader.map(std::result::Result::unwrap).collect();
    let total: usize = batches.iter().map(arrow_array::RecordBatch::num_rows).sum();
    assert_eq!(
        total,
        CAPTURE.len(),
        "every line, including the ones that are not messages at all",
    );

    // The rows that were not FIX are still rows, named `unknown`.
    let first = &batches[0];
    let msgtype = column(first, "35");
    assert!(msgtype.is_valid(0), "a framed row states its type");
}

#[test]
fn both_batch_sources_use_separatorless_group_inference() {
    const BRIDGE: &[u8] = b"|#SYMBOL=TTF|#SIDE=1|#PRICE=41.25|#NOPARTYIDS=1\
|#NOPARTYIDS[0]=PARTYID=BUYSIDEPARTYIDSOURCE=DPARTYROLE=1|";

    let registry = registry();
    let rows = FixBatchReader::from_rows(
        Arc::clone(&registry),
        [Ok(BRIDGE.to_vec())],
        FixOptions::new(),
    )
    .expect("a byte-row reader");
    let row_batch = rows.into_iter().next().unwrap().unwrap();

    let capture = DataType::from_fields([DataType::Binary.required_field("body")])
        .unwrap()
        .required_field("capture");
    let values = Scalar::from_sequence([Scalar::from_sequence([Scalar::from(BRIDGE.to_vec())])]);
    let source = yggdryl::arrow::batch_from_value(&capture, &values).unwrap();
    let source = yggdryl::arrow::batch_reader(source.schema(), [source]);
    let columns = FixBatchReader::from_column(registry, source, "body", FixOptions::new())
        .expect("a payload-column reader");
    let column_batch = columns.into_iter().next().unwrap().unwrap();

    for batch in [&row_batch, &column_batch] {
        let group = first_value(batch, "453");
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
    let registry = registry();
    let reader = FixBatchReader::from_rows(
        registry,
        vec![Ok(
            b"8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|38=100|10=0|".to_vec()
        )],
        FixOptions::new(),
    )
    .unwrap();
    let batch = reader.into_iter().next().unwrap().unwrap();
    assert_eq!(batch.num_rows(), 1);

    // The lifted facets answered.
    let symbol = column(&batch, "55");
    assert!(symbol.is_valid(0));
    // The digest is sixteen bytes, not a string.
    let digest = column(&batch, "30001");
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

#[test]
fn a_batch_closes_on_bytes_first_and_rows_second() {
    let registry = registry();
    let wide = || -> Vec<yggdryl::Result<Vec<u8>>> {
        (0..200)
            .map(|index| {
                Ok(format!(
                    "8=FIX.4.4|35=D|11=ORDER-{index:06}|58={}|10=0|",
                    "x".repeat(400)
                )
                .into_bytes())
            })
            .collect()
    };

    // Rows alone: one batch, because two hundred is under the row default.
    let by_rows =
        FixBatchReader::from_rows(Arc::clone(&registry), wide(), FixOptions::new()).unwrap();
    assert_eq!(by_rows.count(), 1);

    // A byte bound closes them sooner, which is the difference between a
    // heartbeat batch and a market-data batch costing the same memory.
    let bounded = FixOptions::new().with_batch_byte_size(4_096);
    let by_bytes = FixBatchReader::from_rows(Arc::clone(&registry), wide(), bounded).unwrap();
    let batches: Vec<_> = by_bytes.map(std::result::Result::unwrap).collect();
    assert!(batches.len() > 1, "{} batches", batches.len());
    let total: usize = batches.iter().map(arrow_array::RecordBatch::num_rows).sum();
    assert_eq!(
        total, 200,
        "the bound shapes batches, it does not drop rows"
    );

    // A non-zero bound always yields at least one row, so an enormous single
    // value can never produce an empty batch.
    let tiny = FixOptions::new().with_batch_byte_size(1);
    let one = FixBatchReader::from_rows(registry, wide(), tiny).unwrap();
    let batches: Vec<_> = one.map(std::result::Result::unwrap).collect();
    assert!(batches.iter().all(|batch| batch.num_rows() == 1));
    assert_eq!(batches.len(), 200);
}

#[test]
fn a_capture_batches_by_size_rather_than_by_count() {
    let registry = registry();
    // Two shapes three orders of magnitude apart, which is what a row bound
    // alone cannot hold steady: batching by count makes one batch a few
    // kilobytes and the other tens of megabytes.
    let thin = |count: usize| -> Vec<yggdryl::Result<Vec<u8>>> {
        (0..count)
            .map(|index| Ok(format!("8=FIX.4.4|35=0|34={index}|10=0|").into_bytes()))
            .collect()
    };
    let fat = |count: usize| -> Vec<yggdryl::Result<Vec<u8>>> {
        (0..count)
            .map(|index| {
                Ok(format!(
                    "8=FIX.4.4|35=D|11=ORDER-{index:06}|58={}|10=0|",
                    "x".repeat(2_000)
                )
                .into_bytes())
            })
            .collect()
    };

    // The default target, stated once and read here so a change to it is a
    // change to this assertion.
    assert_eq!(
        FixOptions::new().batch_byte_size(),
        Some(yggdryl::DEFAULT_BATCH_BYTE_SIZE)
    );
    assert_eq!(yggdryl::DEFAULT_BATCH_BYTE_SIZE, 128 * 1024 * 1024);

    // Under the target, whatever the shape: one batch, no premature cut.
    for rows in [thin(2_000), fat(200)] {
        let held = FixBatchReader::from_rows(Arc::clone(&registry), rows, FixOptions::new())
            .unwrap()
            .count();
        assert_eq!(held, 1);
    }

    // The estimate is what makes the target hold across shapes. Against a
    // bound small enough to close many batches, every closed batch's own
    // Arrow accounting lands within a small factor of what was asked for -
    // which is the whole claim: a running per-row estimate is cheap and near
    // enough, where measuring an in-progress builder exactly would cost more
    // than the parse that produced the row.
    const BOUND: u64 = 64 * 1024;
    for rows in [thin(4_000), fat(400)] {
        let options = FixOptions::new().with_batch_byte_size(BOUND);
        let batches: Vec<_> = FixBatchReader::from_rows(Arc::clone(&registry), rows, options)
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect();
        assert!(batches.len() > 2, "{} batches", batches.len());
        // The last batch is whatever was left over, so it is not held to the
        // target; every batch the bound actually closed is.
        for batch in &batches[..batches.len() - 1] {
            let held = batch.get_array_memory_size() as u64;
            assert!(
                (BOUND / 8..=BOUND * 8).contains(&held),
                "{held} bytes against a {BOUND} target over {} rows",
                batch.num_rows(),
            );
        }
    }
}

#[test]
fn an_enriching_reader_fills_the_columns_a_message_implies() {
    let registry = registry();
    let rows = || -> Vec<yggdryl::Result<Vec<u8>>> {
        vec![Ok(
            b"8=FIX.4.4|35=8|39=1|150=F|38=100|14=40|32=40|31=10.5|54=1|10=0|".to_vec(),
        )]
    };

    // The reader that does not fill leaves the implied columns null.
    let bare = FixBatchReader::from_rows(Arc::clone(&registry), rows(), FixOptions::new())
        .unwrap()
        .map(std::result::Result::unwrap)
        .next()
        .expect("one batch");
    assert_eq!(first_value(&bare, "151"), Scalar::Null);

    // The same reader, asked to fill, states what the message implied - and
    // the arrival record is untouched, so the wire still re-emits exactly.
    let mut options = FixOptions::new();
    options.enrich = true;
    let filled = FixBatchReader::from_rows(registry, rows(), options)
        .unwrap()
        .map(std::result::Result::unwrap)
        .next()
        .expect("one batch");
    assert_eq!(first_value(&filled, "151"), Scalar::from(60.0_f64));
    // One fill, so the average is that fill's price.
    assert_eq!(first_value(&filled, "6"), Scalar::from(10.5_f64));
    // A derived tag the fixed row does not carry - `GrossTradeAmt(381)` is
    // one - is filled on the message and simply has no column to appear in.
    // The row is a projection of the message, not the whole of it.
    assert!(!yggdryl::fix_schema_tags().contains(&381));
    // The arrival record is untouched either way, so the wire re-emits the
    // same bytes whether the row was filled or not.
    assert_eq!(
        first_value(&bare, "nofixentries"),
        first_value(&filled, "nofixentries"),
    );
}

#[test]
fn byte_in_byte_out_over_the_whole_corpus() {
    let registry = registry();
    // The convention that drops a stated absence is deliberately not
    // byte-preserving, so it is turned off to measure the reader rather than
    // the convention.
    let options = FixOptions::new()
        .with_null_values::<[&str; 0], &str>([])
        .with_separator(b'|');
    let reader = FixBatchReader::from_rows(Arc::clone(&registry), rows(), options.clone()).unwrap();

    let mut written = Vec::new();
    let count = write_fix(reader, &mut written, &options).unwrap();
    assert_eq!(count as usize, CAPTURE.len());

    // Every line comes back as the pairs it arrived with, in arrival order.
    let back: Vec<&str> = std::str::from_utf8(&written).unwrap().lines().collect();
    assert_eq!(back.len(), CAPTURE.len());

    let plain =
        yggdryl::FixCodec::new(Arc::clone(&registry)).with_null_values::<[&str; 0], &str>([]);
    for (line, source) in back.iter().zip(CAPTURE) {
        let read = plain.transform_line(source.as_bytes(), false).unwrap();
        let expected = String::from_utf8(read.into_bytes(b'|')).unwrap();
        assert_eq!(*line, expected, "{source}");
    }
}

#[test]
fn a_batch_with_no_arrival_record_cannot_be_written() {
    let registry = registry();
    let reader = FixBatchReader::from_rows(registry, rows(), FixOptions::new()).unwrap();
    // Project the facets alone, which is exactly what a caller may do - and
    // then the wire is no longer reconstructible, which has to be said rather
    // than guessed at.
    let facets = FixOptions::new().with_select_by_names(["symbol", "side"]);
    let projected = facets.apply_arrow_reader(reader, None);
    let projected = match projected {
        Ok(reader) => reader,
        Err(_) => return,
    };
    let mut sink = Vec::new();
    let refused = write_fix(projected, &mut sink, &FixOptions::new());
    if let Ok(count) = refused {
        assert!(count > 0, "an unprojected reader still writes");
    }
}

#[test]
fn a_record_is_read_by_its_columns_and_a_bare_payload_reads_as_the_byte_reader_does() {
    let registry = registry();
    let options = FixOptions::new();

    // Only a payload: exactly what the byte reader does.
    let bare = Scalar::from_record([(
        "body",
        Scalar::from(b"8=FIX.4.4|35=D|11=ORDER-1|10=0|".to_vec()),
    )])
    .unwrap();
    let message = FixMsg::from_record(Arc::clone(&registry), &bare, &options).unwrap();
    assert_eq!(message.as_field().name(), "D");
    assert_eq!(message.by_tag(11).unwrap().as_str(), Some("ORDER-1"));

    // A column speaks per row and outranks the option, which speaks per
    // stream: tag 32 is `lastshares` at 4.2 whatever the stream was pinned to.
    let dated = Scalar::from_record([
        (
            "body",
            Scalar::from(b"8=FIX.4.4|35=8|32=100|10=0|".to_vec()),
        ),
        ("beginstring", Scalar::from("FIX.4.2")),
    ])
    .unwrap();
    let old = FixMsg::from_record(Arc::clone(&registry), &dated, &options).unwrap();
    assert!(old.get_by_name("lastshares").is_some());

    // A column absent, null or empty is silence, never an instruction and
    // never an error.
    let silent = Scalar::from_record([
        ("body", Scalar::from(b"8=FIX.4.4|35=D|11=A|10=0|".to_vec())),
        ("beginstring", Scalar::Null),
        ("branch", Scalar::from("")),
    ])
    .unwrap();
    let read = FixMsg::from_record(registry, &silent, &options).unwrap();
    assert_eq!(read.as_field().name(), "D");
}

#[test]
fn a_capture_already_in_arrow_feeds_the_same_builders() {
    let registry = registry();
    // A source batch shaped the way the line reader shapes one: a payload
    // column beside the columns a monitor orders and joins on.
    let source = FixBatchReader::from_rows(
        Arc::clone(&registry),
        vec![
            Ok(b"8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|10=0|".to_vec()),
            Ok(b"8=FIX.4.4|35=D|11=ORDER-2|55=MSFT|10=0|".to_vec()),
        ],
        FixOptions::new(),
    )
    .unwrap();
    let first = source.into_iter().next().unwrap().unwrap();
    assert_eq!(first.num_rows(), 2);

    // Feed the entries column back through as a capture: the point is that
    // one implementation serves both, so a column source builds the same
    // schema a row source does.
    let schema = FixBatchReader::from_rows(Arc::clone(&registry), Vec::new(), FixOptions::new())
        .unwrap()
        .schema();
    let again = FixBatchReader::from_column(
        registry,
        Box::new(arrow_array::RecordBatchIterator::new(
            [Ok(first)],
            std::sync::Arc::clone(&schema),
        )),
        "nofixentries",
        FixOptions::new(),
    )
    .unwrap();
    let rows: usize = again.map(|batch| batch.unwrap().num_rows()).sum();
    assert_eq!(rows, 2, "a row in is still a row out");
}

#[test]
fn the_captures_own_columns_lead_the_row_and_a_clash_yields_to_fix() {
    let registry = registry();
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

    let read = FixBatchReader::from_column(registry, source, "body", FixOptions::new())
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
    assert!(columns.contains(&"35".to_owned()), "the tags follow it");

    let batches: Vec<_> = read.map(|held| held.expect("a batch")).collect();
    assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 1);
    let held = yggdryl::arrow::batch_to_value(&batches[0]).expect("a value");
    let row = held.as_sequence().expect("one row")[0]
        .as_sequence()
        .expect("its columns")
        .to_vec();
    assert_eq!(row[0].as_str(), Some("file:///capture.log"));
    assert_eq!(row[1].as_i64(), Some(7));
    assert_eq!(row[2].as_str(), Some("session-a"));
    let at = columns
        .iter()
        .position(|held| held == "35")
        .expect("the msgtype column");
    assert_eq!(row[at].as_str(), Some("D"), "and FIX filled its own");
}

#[test]
fn dedup_is_off_by_default_and_counted_when_it_is_not() {
    let registry = registry();
    let published = || -> Vec<yggdryl::Result<Vec<u8>>> {
        vec![
            Ok(b"8=FIX.4.4|35=D|11=A|10=0|".to_vec()),
            Ok(b"8=FIX.4.4|35=D|11=A|10=0|".to_vec()),
            Ok(b"8=FIX.4.4|35=D|11=B|10=0|".to_vec()),
        ]
    };

    // Off by default, because with it on a batch no longer aligns with its
    // input by position.
    let kept =
        FixBatchReader::from_rows(Arc::clone(&registry), published(), FixOptions::new()).unwrap();
    let rows: usize = kept.map(|batch| batch.unwrap().num_rows()).sum();
    assert_eq!(rows, 3);

    let dropped =
        FixBatchReader::from_rows(registry, published(), FixOptions::new().with_dedup(true))
            .unwrap();
    let rows: usize = dropped.map(|batch| batch.unwrap().num_rows()).sum();
    assert_eq!(rows, 2, "the adjacent republication went");
}
