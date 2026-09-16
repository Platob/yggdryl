//! The third verb: a message as the rows one message field holds them.
//!
//! A parse lands a capture in the fixed row and an enrichment fills what each
//! message implies. This is where the same messages answer under a *message
//! field* a consumer names - the fixed row itself, which keeps every column a
//! capture lands in, a venue's own message type, or a projection of either -
//! and where a column the source row did not carry is lifted back out of the
//! arrival record.

use std::sync::Arc;

use super::SoleMessage;

use yggdryl::{DataType, Field, FixCodec, FixMsg, FixRegistry, Scalar, fix_schema};

fn reader() -> (Arc<FixRegistry>, FixCodec) {
    let registry = super::committed_registry();
    let codec = super::fixed_codec(Arc::clone(&registry));
    (registry, codec)
}

/// One column of one row, by name.
#[track_caller]
fn at<'row>(row: &'row Scalar, field: &Field, name: &str) -> &'row Scalar {
    let at = field
        .index_of(name)
        .unwrap_or_else(|| panic!("a {name} column"));
    &row.as_sequence().expect("a row")[at]
}

const ORDER: &[u8] = b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|44=12.5|38=100|207=XLON|10=0|";

#[test]
fn format_messages_answers_one_row_per_message_under_the_field() {
    let (registry, codec) = reader();
    let target = fix_schema(&registry, "fix").unwrap();
    let messages = codec.parse_line(ORDER).unwrap();

    let rows: Vec<Scalar> = codec
        .format_messages(messages, &target)
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(at(&rows[0], &target, "symbol").as_str(), Some("AAPL"));
    assert_eq!(at(&rows[0], &target, "price").as_f64(), Some(12.5));
    assert_eq!(at(&rows[0], &target, "orderqty").as_f64(), Some(100.0));
    // The record closes a formatted row exactly as it closes a parsed one.
    assert!(!at(&rows[0], &target, "fixentries").is_null());
}

/// A target of a caller's own, and the columns it does not name are gone.
#[test]
fn a_format_target_is_any_message_field_a_caller_names() {
    let (registry, codec) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let wanted = [
        "symbol",
        "side",
        "updatedat",
        "createdat",
        "msghash",
        "msgphash",
        "code",
        "snapshotat",
        "sendingtime",
        "beginstring",
    ];
    let columns: Vec<Field> = wanted
        .iter()
        .map(|name| schema.fields()[schema.index_of(name).expect("a column")].clone())
        .collect();
    let narrow = DataType::from_fields(columns)
        .unwrap()
        .required_field("blotter");

    let rows: Vec<Scalar> = codec
        .format_messages(codec.parse_line(ORDER).unwrap(), &narrow)
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(rows[0].as_sequence().expect("a row").len(), wanted.len());
    assert_eq!(at(&rows[0], &narrow, "symbol").as_str(), Some("AAPL"));
    assert!(
        narrow.index_of("price").is_none(),
        "what it did not name is gone"
    );
}

/// A row formatted from a narrower one reads the record for what that one lost.
#[test]
fn a_column_the_source_row_dropped_is_lifted_out_of_the_record() {
    let (registry, codec) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();

    // A row carrying the record and the settled bundle, and nothing of the
    // message's own fields - what a capture stored as little as it could.
    let keep = [
        "beginstring",
        "msgtype",
        "version",
        "updatedat",
        "msghash",
        "msgphash",
        "createdat",
        "code",
        "snapshotat",
        "sendingtime",
        "fixentries",
        "nofixentries",
    ];
    let columns: Vec<Field> = keep
        .iter()
        .map(|name| schema.fields()[schema.index_of(name).expect("a column")].clone())
        .collect();
    let narrow = DataType::from_fields(columns)
        .unwrap()
        .required_field("fix");

    let parsed = codec.sole_line(ORDER, false).unwrap();
    let stored = parsed.into_row(&narrow).unwrap();
    assert!(
        narrow.index_of("symbol").is_none(),
        "the stored row has no symbol"
    );

    // Read back and formatted into the full message field, the columns the
    // narrow row never carried come back off the record, typed.
    let held = FixMsg::from_row(Arc::clone(&registry), &narrow, &stored).unwrap();
    let row = codec
        .format_messages([Ok(held)], &schema)
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(at(&row, &schema, "symbol").as_str(), Some("AAPL"));
    assert_eq!(at(&row, &schema, "price").as_f64(), Some(12.5));
    assert_eq!(at(&row, &schema, "securityexchange").as_str(), Some("XLON"));
    assert_eq!(at(&row, &schema, "side"), parsed.by_tag(54).unwrap());
}

/// A group comes back off the record whole, occurrence by occurrence.
#[test]
fn a_group_is_lifted_out_of_the_record_with_its_members() {
    let (registry, codec) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let line =
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|453=2|448=P1|447=D|452=1|448=P2|447=D|452=11|10=0|";

    let keep = [
        "beginstring",
        "msgtype",
        "version",
        "updatedat",
        "msghash",
        "msgphash",
        "createdat",
        "code",
        "snapshotat",
        "sendingtime",
        "fixentries",
        "nofixentries",
    ];
    let columns: Vec<Field> = keep
        .iter()
        .map(|name| schema.fields()[schema.index_of(name).expect("a column")].clone())
        .collect();
    let narrow = DataType::from_fields(columns)
        .unwrap()
        .required_field("fix");

    let stored = codec
        .sole_line(line, false)
        .unwrap()
        .into_row(&narrow)
        .unwrap();
    let held = FixMsg::from_row(Arc::clone(&registry), &narrow, &stored).unwrap();
    let row = codec
        .format_messages([Ok(held)], &schema)
        .next()
        .unwrap()
        .unwrap();

    // Two parties, each member where the group definition puts it, and the
    // counter counting them.
    assert_eq!(at(&row, &schema, "nopartyids").as_i128(), Some(2));
    let parties = at(&row, &schema, "parties")
        .as_sequence()
        .expect("the parties");
    assert_eq!(parties.len(), 2);
    let identifiers: Vec<Option<&str>> = parties
        .iter()
        .map(|party| party.as_sequence().expect("a party")[0].as_str())
        .collect();
    assert_eq!(identifiers, [Some("P1"), Some("P2")]);
}

/// The Arrow twin: the same rows, one batch at a time.
#[test]
fn format_arrow_reader_answers_the_batches_format_messages_answers_rows() {
    let (registry, codec) = reader();
    let target = fix_schema(&registry, "fix").unwrap();
    let messages: Vec<FixMsg> = codec
        .parse_line(ORDER)
        .unwrap()
        .collect::<yggdryl::Result<Vec<_>>>()
        .unwrap();

    let source = codec
        .arrow_reader(target.clone(), messages.clone().into_iter().map(Ok))
        .unwrap();
    let formatted = codec.format_arrow_reader(source, &target).unwrap();

    // The schema is decided before the first row, from the field alone.
    let held = Field::from_arrow_schema("fix", &formatted.schema()).unwrap();
    assert_eq!(held.fields().len(), target.fields().len());

    let batches: Vec<_> = formatted.map(|batch| batch.unwrap()).collect();
    let rows = yggdryl::arrow::batch_to_value(&batches[0]).unwrap();
    let batched = &rows.as_sequence().expect("rows")[0];
    let direct = codec
        .format_messages(messages.into_iter().map(Ok).collect::<Vec<_>>(), &target)
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(batched, &direct, "the two doors answer one row");
}
