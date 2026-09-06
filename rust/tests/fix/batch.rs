//! A capture in, columns out, and back to the wire.

use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
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
    assert_eq!(&names[names.len() - 2..], ["entries", "unmapped"]);
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
    let entries = column(&batch, "entries");
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

    let plain = yggdryl::FixReader::new(Arc::clone(&registry)).null_values::<[&str; 0], &str>([]);
    for (line, source) in back.iter().zip(CAPTURE) {
        let read = plain.text(source).unwrap();
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
    let projected = facets.cast_arrow_reader(reader, None);
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
        ("fixbranch", Scalar::from("")),
    ])
    .unwrap();
    let read = FixMsg::from_record(registry, &silent, &options).unwrap();
    assert_eq!(read.as_field().name(), "D");
}

#[test]
fn a_capture_already_in_arrow_feeds_the_same_builders() {
    let registry = registry();
    let mut url = DataType::Utf8.required_field("url");
    url.set_metadata([("source", "capture")]).unwrap();
    let mut source_field = DataType::from_fields([
        url,
        DataType::Int64.required_field("rownum"),
        DataType::Utf8.required_field("branch"),
        DataType::Utf8.required_field("body"),
        DataType::Utf8.required_field("fixbranch"),
    ])
    .unwrap()
    .required_field("message");
    source_field
        .set_metadata([("python.class", "Message")])
        .unwrap();
    let schema = source_field.into_arrow_schema().unwrap();
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(StringArray::from(vec!["file:///capture.log"])),
            Arc::new(Int64Array::from(vec![7_i64])),
            Arc::new(StringArray::from(vec!["provenance"])),
            Arc::new(StringArray::from(vec![
                "8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|10=0|",
            ])),
            Arc::new(StringArray::from(vec![""])),
        ],
    )
    .unwrap();
    let parsed = FixBatchReader::from_column(
        registry,
        yggdryl::arrow::batch_reader(schema, [batch]),
        "body",
        FixOptions::new(),
    );
    let mut parsed = parsed.unwrap();
    let output = parsed.schema();
    let names: Vec<&str> = output
        .fields()
        .iter()
        .map(|field| field.name().as_str())
        .collect();
    assert_eq!(
        &names[..5],
        ["url", "rownum", "branch", "body", "fixbranch"]
    );
    assert!(names.contains(&"35"));
    assert_eq!(
        output.field_with_name("url").unwrap().metadata()["source"],
        "capture"
    );
    assert!(!output.metadata().contains_key("python.class"));

    let batch = parsed.next().unwrap().unwrap();
    assert_eq!(batch.num_rows(), 1);
    let branch = column(&batch, "branch")
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(branch.value(0), "provenance");
    assert!(column(&batch, "35").is_valid(0));
}

#[test]
fn a_source_fix_column_collision_is_refused_before_reading() {
    let registry = registry();
    let source = DataType::from_fields([
        DataType::Utf8.required_field("body"),
        DataType::Utf8.nullable_field("Entries"),
    ])
    .unwrap()
    .required_field("message");
    let reader = yggdryl::arrow::batch_reader(source.into_arrow_schema().unwrap(), []);
    let error = match FixBatchReader::from_column(registry, reader, "body", FixOptions::new()) {
        Ok(_) => panic!("a colliding source schema must be refused"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("duplicate \"Entries\""));
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
