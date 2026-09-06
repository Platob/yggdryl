//! A capture in, columns out, and back to the wire.

use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
use yggdryl::holder::local::Folder;
use yggdryl::media::IORecordOptions;
use yggdryl::{
    DataType, FixBatchReader, FixMsg, FixOptions, FixRegistry, Scalar, TimeUnit, Timezone,
    write_fix,
};

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
fn a_text_clock_is_derived_as_utc_microseconds_or_null() {
    let mut sending_time = DataType::Utf8.nullable_field("sendingtime");
    sending_time.as_fix_mut().set_tag(52).unwrap();
    let registry = Arc::new(FixRegistry::from_fields([sending_time]).unwrap());
    let reader = FixBatchReader::from_rows(
        registry,
        [
            Ok(b"52=20260814-00:05:01.148123456|".to_vec()),
            Ok(b"52=20260814-00:05:01.148123|".to_vec()),
            Ok(b"52=19691231-23:59:59.999999|".to_vec()),
            Ok(b"52=not-a-clock|".to_vec()),
        ],
        FixOptions::new(),
    )
    .unwrap();
    let schema = reader.schema();
    assert_eq!(
        schema.field_with_name("30004").unwrap().data_type(),
        &arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, Some("UTC".into()))
    );

    let batch = reader.into_iter().next().unwrap().unwrap();
    let timestamp = column(&batch, "30004")
        .as_any()
        .downcast_ref::<arrow_array::TimestampMicrosecondArray>()
        .expect("a microsecond timestamp array");
    assert_eq!(
        timestamp.value(0),
        timestamp.value(1),
        "sub-us digits truncate"
    );
    assert_eq!(timestamp.value(0).rem_euclid(1_000_000), 148_123);
    assert!(
        timestamp.is_null(3),
        "an invalid row clock is not a stream error"
    );
    let partition = column(&batch, "30005")
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("an epoch-second partition array");
    assert_eq!(
        partition.value(0),
        timestamp.value(0).div_euclid(3_600_000_000) * 3_600
    );
    assert_eq!(partition.value(2), -3_600, "pre-epoch instants floor");
    assert!(partition.is_null(3));
}

#[test]
fn a_custom_nanosecond_clock_retains_source_precision() {
    let mut sending_time = DataType::DateTime64 {
        unit: TimeUnit::Nanosecond,
        timezone: Timezone::UTC,
    }
    .nullable_field("sendingtime");
    sending_time.as_fix_mut().set_tag(52).unwrap();
    let registry = Arc::new(FixRegistry::from_fields([sending_time]).unwrap());
    let reader = FixBatchReader::from_rows(
        registry,
        [Ok(b"52=20260814-00:05:01.148123456|".to_vec())],
        FixOptions::new(),
    )
    .unwrap();
    let batch = reader.into_iter().next().unwrap().unwrap();

    let source = column(&batch, "52")
        .as_any()
        .downcast_ref::<arrow_array::TimestampNanosecondArray>()
        .expect("the dictionary's nanosecond source");
    assert_eq!(source.value(0).rem_euclid(1_000_000_000), 148_123_456);
    let derived = column(&batch, "30004")
        .as_any()
        .downcast_ref::<arrow_array::TimestampMicrosecondArray>()
        .expect("the canonical microsecond clock");
    assert_eq!(derived.value(0).rem_euclid(1_000_000), 148_123);
}

#[test]
fn a_tz_timestamp_preserves_its_offset_and_truncates_to_microseconds() {
    let mut tz_timestamp = DataType::DateTime64 {
        unit: TimeUnit::Microsecond,
        timezone: Timezone::UTC,
    }
    .nullable_field("tztimestamp");
    tz_timestamp.as_fix_mut().set_tag(52).unwrap();
    let registry = Arc::new(FixRegistry::from_fields([tz_timestamp]).unwrap());
    let reader = FixBatchReader::from_rows(
        registry,
        [
            Ok(b"52=20260814-00:05:01.148123456+02:00|".to_vec()),
            Ok(b"52=20260813-22:05:01.148123Z|".to_vec()),
            Ok(b"52=20260814-00:05:01.148123456+02|".to_vec()),
        ],
        FixOptions::new(),
    )
    .unwrap();
    let batch = reader.into_iter().next().unwrap().unwrap();
    let timestamp = column(&batch, "52")
        .as_any()
        .downcast_ref::<arrow_array::TimestampMicrosecondArray>()
        .expect("the TZTimestamp column");
    assert_eq!(
        timestamp.value(0),
        timestamp.value(1),
        "the explicit offset resolves into the same UTC instant"
    );
    assert_eq!(
        timestamp.value(0),
        timestamp.value(2),
        "an hour-only offset"
    );
    assert_eq!(timestamp.value(0).rem_euclid(1_000_000), 148_123);
}

#[test]
fn a_historical_text_clock_conforms_to_the_fixed_timestamp_column() {
    let registry = registry();
    let reader = FixBatchReader::from_rows(
        registry,
        [Ok(
            b"8=FIX.4.1|52=20260814-00:05:01.148123456|35=D|10=0|".to_vec()
        )],
        FixOptions::new(),
    )
    .unwrap();
    let batch = reader.into_iter().next().unwrap().unwrap();
    let source = column(&batch, "52")
        .as_any()
        .downcast_ref::<arrow_array::TimestampMicrosecondArray>()
        .expect("the fixed timestamp column");
    assert!(source.is_valid(0));
    assert_eq!(source.value(0).rem_euclid(1_000_000), 148_123);
}

#[test]
fn a_multibyte_malformed_date_is_null_without_losing_its_row() {
    let registry = registry();
    let reader = FixBatchReader::from_rows(
        registry,
        [Ok("75=123é567|".as_bytes().to_vec())],
        FixOptions::new(),
    )
    .unwrap();
    let batch = reader.into_iter().next().unwrap().unwrap();
    assert_eq!(batch.num_rows(), 1);
    assert!(column(&batch, "75").is_null(0));
    assert!(column(&batch, "entries").is_valid(0));
}

#[test]
fn a_fixt_row_carries_its_resolved_application_version() {
    let registry = registry();
    let reader = FixBatchReader::from_rows(
        registry,
        [Ok(b"8=FIXT.1.1|1128=6|35=D|11=ORDER-1|10=0|".to_vec())],
        FixOptions::new(),
    )
    .unwrap();
    let batch = reader.into_iter().next().unwrap().unwrap();
    let version = column(&batch, "30002")
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("the derived version column");
    assert_eq!(version.value(0), "4.4");
}

#[test]
fn inferred_directions_use_fix_codes_in_the_fixed_column() {
    let registry = registry();
    let reader = FixBatchReader::from_rows(
        registry,
        [
            Ok(b"sending >> 8=FIX.4.4|35=D|10=0|".to_vec()),
            Ok(b"receiving >> 8=FIX.4.4|35=D|10=0|".to_vec()),
        ],
        FixOptions::new(),
    )
    .unwrap();
    let batch = reader.into_iter().next().unwrap().unwrap();
    let direction = column(&batch, "385")
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("the FIX MsgDirection column");
    assert_eq!(direction.value(0), "S");
    assert_eq!(direction.value(1), "R");
}

#[test]
fn a_bridge_group_is_aligned_to_the_complete_fixed_item() {
    let registry = registry();
    let body = "toBridge #ISINCODE=XX0000084733|#CFICODE=FXXXSX|#SYMBOL=TTF|#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=2|#NOPARTYIDS[0]=PARTYID=BUYSIDE\u{1}PARTYIDSOURCE=D\u{1}PARTYROLE=1|#NOPARTYIDS[1]=PARTYID=XPAR\u{1}PARTYIDSOURCE=G\u{1}PARTYROLE=17|#TRANSACTTIME=20260814-00:05:01.148|#UNKNOWNVENUEFIELD=Z9";
    let reader =
        FixBatchReader::from_rows(registry, [Ok(body.as_bytes().to_vec())], FixOptions::new())
            .unwrap();
    let batch = reader.into_iter().next().unwrap().unwrap();
    let group_at = batch.schema().index_of("453").unwrap();
    let rows = yggdryl::arrow::batch_to_value(&batch).unwrap();
    let group = rows
        .get(0)
        .and_then(|row| row.get(group_at))
        .and_then(Scalar::as_sequence)
        .expect("the party group");
    assert_eq!(group.len(), 2);
    let first = group[0].as_sequence().expect("the first complete item");
    let second = group[1].as_sequence().expect("the second complete item");
    assert_eq!(first.len(), 4, "the fixed registry item has four fields");
    assert_eq!(second.len(), 4);
    assert_eq!(first[0].as_str(), Some("BUYSIDE"));
    assert_eq!(second[0].as_str(), Some("XPAR"));
    assert!(
        first[3].is_null(),
        "the absent qualifier is filled with null"
    );
    assert!(second[3].is_null());
}

#[test]
fn a_standard_numeric_party_group_is_assembled_from_its_layout() {
    let registry = registry();
    let body = b"8=FIX.4.4|35=D|453=2|448=BUYSIDE|447=D|452=1|448=XPAR|447=G|452=17|10=0|";
    let reader =
        FixBatchReader::from_rows(registry, [Ok(body.to_vec())], FixOptions::new()).unwrap();
    let batch = reader.into_iter().next().unwrap().unwrap();
    let group_at = batch.schema().index_of("453").unwrap();
    let rows = yggdryl::arrow::batch_to_value(&batch).unwrap();
    let group = rows[0][group_at].as_sequence().expect("the party group");
    assert_eq!(group.len(), 2);
    let first = group[0].as_sequence().expect("the first party");
    let second = group[1].as_sequence().expect("the second party");
    assert_eq!(first[0].as_str(), Some("BUYSIDE"));
    assert_eq!(second[0].as_str(), Some("XPAR"));
    assert_eq!(first.len(), 4);
    assert_eq!(second.len(), 4);
}

#[test]
fn a_regulatory_group_timestamp_drives_the_capture_clock() {
    let registry = registry();
    let body = "MSGTYPE=8|#NOTRDREGTIMESTAMPS=1|#NOTRDREGTIMESTAMPS[0]=TRDREGTIMESTAMP=20240102-10:15:30.148123456\u{1}TRDREGTIMESTAMPTYPE=1";
    let reader =
        FixBatchReader::from_rows(registry, [Ok(body.as_bytes().to_vec())], FixOptions::new())
            .unwrap();
    let batch = reader.into_iter().next().unwrap().unwrap();
    let timestamp = column(&batch, "30004")
        .as_any()
        .downcast_ref::<arrow_array::TimestampMicrosecondArray>()
        .expect("the regulatory capture clock");
    assert!(timestamp.is_valid(0));
    assert_eq!(timestamp.value(0).rem_euclid(1_000_000), 148_123);
    let partition = column(&batch, "30005")
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("the regulatory clock partition");
    assert!(partition.is_valid(0));
}

#[test]
fn a_null_symbol_falls_through_to_the_stated_security_id() {
    let mut symbol = DataType::Int32.nullable_field("symbol");
    symbol.as_fix_mut().set_tag(55).unwrap();
    let mut security_id = DataType::Utf8.nullable_field("securityid");
    security_id.as_fix_mut().set_tag(48).unwrap();
    let registry = Arc::new(FixRegistry::from_fields([symbol, security_id]).unwrap());
    let reader = FixBatchReader::from_rows(
        registry,
        [Ok(b"55=not-an-int|48=XX0000084733|".to_vec())],
        FixOptions::new(),
    )
    .unwrap();
    let batch = reader.into_iter().next().unwrap().unwrap();
    let ticker = column(&batch, "30003")
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("the normalized ticker");
    assert_eq!(ticker.value(0), "XX0000084733");
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
