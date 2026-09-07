//! The three readers, over the committed dictionary and a real capture.

use std::path::PathBuf;
use std::sync::Arc;

use yggdryl::holder::local::Folder;
use yggdryl::{DataType, FixEntry, FixReader, FixRegistry, Scalar, Version};

fn registry() -> Arc<FixRegistry> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("config")
        .join("fix");
    let folder = Folder::new(root).expect("the seed folder is a local path");
    Arc::new(FixRegistry::from_handle(&folder).expect("the committed dictionary loads"))
}

fn reader() -> FixReader {
    FixReader::new(registry())
}

/// Every shape a real capture holds, one row each.
///
/// The classifier labels are a consumer's taxonomy rather than a third
/// reader: one reader takes a tag key or a name key, and the frame already
/// said which splitter runs.
const CAPTURE: &[&str] = &[
    "sending >> 8=FIX.4.2|9=176|35=D|10=203| << queued seq=1092",
    "raw 8=FIX.4.4|9=224|35=8|10=118|",
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

#[test]
fn every_capture_row_builds_and_none_is_skipped() {
    let reader = reader();
    for row in CAPTURE {
        let message = reader
            .text(row)
            .unwrap_or_else(|error| panic!("{row}: {error}"));
        // A row with no type is named `unknown` rather than refused: every
        // pair that parsed became a field and the entries record the row.
        assert!(!message.as_field().name().is_empty(), "{row}");
    }

    // Empty input is the one typed error: a line that merely held nothing is
    // `Ok` and says so.
    assert!(reader.text("").is_err());
    let empty = reader.text("no level printed by this plugin").unwrap();
    assert_eq!(empty.as_field().name(), "unknown");
    assert!(empty.entries().is_empty());
}

#[test]
fn a_framed_row_takes_its_type_from_the_frame_and_its_prefix_is_dropped() {
    let reader = reader();
    for (row, msgtype) in [
        (
            "sending >> 8=FIX.4.2|9=176|35=D|10=203| << queued seq=1092",
            "D",
        ),
        ("raw 8=FIX.4.4|9=224|35=8|10=118|", "8"),
        ("8=FIX.4.4|35=D|11=ORDER-1|SYMBOL=AAPL|SIDE=1|10=000", "D"),
        (
            "ACCOUNT=A1|MSGTYPE=D|CLORDID=ORDER-1|SYMBOL=AAPL|SIDE=1",
            "D",
        ),
    ] {
        let message = reader.text(row).expect(row);
        assert_eq!(message.as_field().name(), msgtype, "{row}");
    }

    // The prose in front of the frame is not a field, and nothing after the
    // checksum is part of the message.
    let framed = reader
        .text("sending >> 8=FIX.4.2|9=176|35=D|10=203| << queued seq=1092")
        .unwrap();
    let keys: Vec<&str> = framed
        .entries()
        .iter()
        .map(yggdryl::FixEntry::key)
        .collect();
    assert_eq!(keys, ["8", "9", "35", "10"], "{keys:?}");
}

#[test]
fn a_tag_key_and_a_name_key_build_the_same_message() {
    let reader = reader();
    let by_tag = reader.text("8=FIX.4.4|35=D|55=AAPL|54=1|10=0|").unwrap();
    let by_name = reader
        .text("8=FIX.4.4|MsgType=D|Symbol=AAPL|Side=1|10=0|")
        .unwrap();
    assert_eq!(by_tag.as_value(), by_name.as_value());
    assert_eq!(by_tag.as_field().dtype(), by_name.as_field().dtype());

    // Case and separators fold away, so a renderer's spelling still resolves.
    for spelling in ["MsgType", "msgtype", "MSG_TYPE", "msg-type", "Msg Type"] {
        let row = format!("8=FIX.4.4|{spelling}=D|10=0|");
        let message = reader.text(&row).expect(&row);
        assert_eq!(message.as_field().name(), "D", "{spelling}");
    }
}

#[test]
fn a_value_is_translated_typed_and_kept_as_it_arrived() {
    let reader = reader();
    let message = reader.text("8=FIX.4.4|35=D|54=Buy|38=100|10=0|").unwrap();

    // The row holds the translated code and the typed number.
    assert_eq!(message.by_name("side").unwrap().as_str(), Some("1"));
    assert_eq!(
        message.by_name("orderqty").unwrap(),
        &Scalar::from(100.0_f64)
    );
    // The entry holds what arrived, untranslated.
    let side = message
        .entries()
        .iter()
        .find(|entry| entry.tag() == 54)
        .expect("tag 54");
    assert_eq!(side.value(), "Buy");
    assert_eq!(side.key(), "54");
    assert_eq!(side.id(), Some(yggdryl::FixId::standard(54)));
}

#[test]
fn an_unknown_key_is_kept_and_a_bad_value_is_null_rather_than_a_failure() {
    let reader = reader();
    let message = reader
        .text("8=FIX.4.4|35=D|VenueOwnThing=x|9999=y|9=abc|10=0|")
        .unwrap();

    // An unknown name is kept under its own folded spelling, and an unknown
    // tag under its decimal one.
    assert_eq!(
        message.by_name("venueownthing").unwrap().as_str(),
        Some("x")
    );
    assert_eq!(message.by_name("9999").unwrap().as_str(), Some("y"));
    // The entry keeps the arrival casing; the built child does not.
    let held = message
        .entries()
        .iter()
        .find(|entry| entry.key() == "VenueOwnThing")
        .expect("the venue's own key");
    assert_eq!(held.tag(), 0, "an unknown key names no tag");
    assert_eq!(held.id(), None);

    // A `BodyLength` of `abc` nulls that field while the raw text stays.
    assert_eq!(message.by_name("bodylength").unwrap(), &Scalar::Null);
    assert!(
        message
            .entries()
            .iter()
            .any(|entry| entry.tag() == 9 && entry.value() == "abc")
    );
}

#[test]
fn a_stated_absence_produces_no_field_and_no_entry() {
    let reader = reader();
    for spelling in ["", "null", "NULL", "<null>"] {
        let row = format!("8=FIX.4.4|35=D|58={spelling}|10=0|");
        let message = reader.text(&row).expect(&row);
        assert!(message.get_by_tag(58).is_none(), "{spelling}");
        assert!(
            !message.entries().iter().any(|entry| entry.tag() == 58),
            "{spelling}"
        );
    }
    // A value that merely contains `null`, and one a venue means literally,
    // both survive - the listing is a convention and has to be overridable.
    let kept = reader.text("8=FIX.4.4|35=D|58=nullable|10=0|").unwrap();
    assert_eq!(kept.by_tag(58).unwrap().as_str(), Some("nullable"));

    let literal = reader
        .clone()
        .null_values::<[&str; 0], &str>([])
        .text("8=FIX.4.4|35=D|58=null|10=0|")
        .unwrap();
    assert_eq!(literal.by_tag(58).unwrap().as_str(), Some("null"));
}

#[test]
fn a_bridge_group_becomes_real_nesting_from_its_indexed_keys() {
    let reader = reader();
    let row = "MSGTYPE=D|#NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=SYNTH-01\u{4}\u{3}PARTYIDSOURCE=D\u{4}\u{3}PARTYROLE=1";
    let message = reader.text(row).unwrap();

    let parties = message.by_name("nopartyids").expect("the group");
    let occurrences = parties.as_sequence().expect("a list of occurrences");
    assert_eq!(occurrences.len(), 1);
    let members = occurrences[0].as_sequence().expect("one item struct");
    assert_eq!(members.len(), 3);
    assert!(
        members
            .iter()
            .any(|value| value.as_str() == Some("SYNTH-01")),
        "{members:?}"
    );

    // The group field is a List of a non-null `item` Struct.
    let field = message
        .as_field()
        .get_field_by_path("nopartyids")
        .expect("the group field");
    let DataType::List(item) = field.dtype() else {
        panic!("a list, got {}", field.dtype());
    };
    assert_eq!(item.name(), "item");
    assert!(!item.is_nullable());
}

#[test]
fn a_bridge_frame_of_raw_bytes_reads_its_types_its_group_and_its_miscount() {
    let reader = reader();
    // One real bridge frame, byte for byte: a leading separator, `#`-prefixed
    // name keys, and one occurrence whose value packs its members behind the
    // two control bytes ULLINK separates them with.
    let line: &[u8] = b"|#SYMBOL=TTF|#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=2\
|#NOPARTYIDS[0]=PARTYID=BUYSIDE\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1|";

    // The dialect is read off the frame, so the bytes entry point and the
    // bridge one answer the same message rather than two spellings of it.
    let message = reader.bytes(line).unwrap();
    assert_eq!(message, reader.ultext(line).unwrap());

    // Names resolve to tags, and each value takes its field's own type: a
    // quantity and a price are numbers, and a side is the packed code.
    assert_eq!(message.by_tag(55).unwrap().as_str(), Some("TTF"));
    assert_eq!(message.by_tag(54).unwrap().as_str(), Some("1"));
    assert_eq!(message.by_tag(38).unwrap().as_f64(), Some(1200.0));
    assert_eq!(message.by_tag(44).unwrap().as_f64(), Some(41.25));

    // The occurrence's packed members became three real fields under one
    // nesting, each resolved to its own tag rather than kept as text.
    let occurrences = message
        .by_tag(453)
        .unwrap()
        .as_sequence()
        .expect("the party group")
        .to_vec();
    assert_eq!(occurrences.len(), 1);
    assert_eq!(
        occurrences[0].as_sequence().map(<[Scalar]>::len),
        Some(3),
        "three members, split on the control bytes"
    );
    let keys: Vec<&str> = message.entries().iter().map(FixEntry::key).collect();
    assert_eq!(
        &keys[5..],
        [
            "NOPARTYIDS[0].PARTYID",
            "NOPARTYIDS[0].PARTYIDSOURCE",
            "NOPARTYIDS[0].PARTYROLE",
        ]
    );

    // Which is enough to lift the party the frame is about.
    let held = message.party("1").expect("the buy-side party");
    assert_eq!(held.id().and_then(Scalar::as_str), Some("BUYSIDE"));
    assert_eq!(held.source().and_then(Scalar::as_str), Some("D"));

    // The counter says two occurrences and one arrived. That is the frame's
    // own contradiction, reported rather than repaired: nothing invents the
    // occurrence that is missing, and nothing rewrites the count that is
    // wrong.
    let anomalies: Vec<String> = message.anomalies().map(|held| held.to_string()).collect();
    assert_eq!(anomalies.len(), 1, "{anomalies:?}");
    assert!(anomalies[0].contains("453"), "{anomalies:?}");
    assert!(anomalies[0].contains('2'), "{anomalies:?}");
}

#[test]
fn indexed_occurrences_are_built_by_index_and_a_gap_is_null() {
    let reader = reader();
    // Out of order and gapped: `[2]` before `[0]`, with `[1]` absent.
    let message = reader
        .text("MSGTYPE=D|PartyID[2]=third|PartyID[0]=first")
        .unwrap();
    let values = message
        .by_name("partyid")
        .expect("the repeated field")
        .as_sequence()
        .expect("occurrences");
    assert_eq!(values.len(), 3);
    assert_eq!(values[0].as_str(), Some("first"));
    assert_eq!(values[1], Scalar::Null);
    assert_eq!(values[2].as_str(), Some("third"));
}

#[test]
fn the_header_orders_first_and_the_trailer_last_whatever_the_input_order() {
    let reader = reader();
    let message = reader
        // Header tags out of canonical order, with a body field interleaved.
        .text("8=FIX.4.4|55=AAPL|35=D|9=100|10=000|")
        .unwrap();
    let names: Vec<&str> = message
        .as_field()
        .dtype()
        .as_fields()
        .expect("a struct root")
        .iter()
        .map(yggdryl::Field::name)
        .collect();
    assert_eq!(
        names,
        ["beginstring", "bodylength", "msgtype", "symbol", "checksum"],
        "{names:?}"
    );
}

#[test]
fn a_message_re_emits_from_its_entries_and_reads_back_equal() {
    let reader = reader();
    let row = "8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|38=100|10=000|";
    let message = reader.text(row).unwrap();

    // The emit is the wire record, so a translated code cannot leak into it.
    let bytes = message.into_bytes(b'|');
    assert_eq!(String::from_utf8(bytes.clone()).unwrap(), row);
    let again = reader.fixtext(&bytes, b'|').unwrap();
    assert_eq!(again.entries(), message.entries());
    assert_eq!(again.as_value(), message.as_value());

    assert_eq!(message.into_text('|').unwrap(), row);
    assert_eq!(message.version(), Some("4.4".parse::<Version>().unwrap()));
}

#[test]
fn a_version_names_and_types_a_field_as_that_version_did() {
    let reader = reader();
    let old = reader
        .clone()
        .source_version("4.2".parse::<Version>().unwrap());
    let newest = reader
        .clone()
        .source_version("5.0SP2".parse::<Version>().unwrap());

    // Tag 32 is `LastShares` in 4.2 and `LastQty` in a newest one, and both
    // answer the same value.
    let at_42 = old.text("8=FIX.4.2|35=8|32=100|10=0|").unwrap();
    let at_new = newest.text("8=FIX.4.4|35=8|32=100|10=0|").unwrap();
    assert!(at_42.get_by_name("lastshares").is_some());
    assert!(at_new.get_by_name("lastqty").is_some());
    assert_eq!(at_42.by_tag(32).unwrap(), at_new.by_tag(32).unwrap());
}

#[test]
fn a_numeric_frame_carrying_a_repeating_group_reads_and_only_its_counter_counts() {
    let reader = reader();
    // A numeric frame states its group members flat, so the occurrences the
    // bridge's indexed keys build are not there to build: the counter keeps
    // the group's own shape and holds nothing, and each member is as many
    // values as arrived. Retyping the counter to the `NumInGroup` its lineage
    // dates would collapse that shape and refuse the whole frame.
    let row = "8=FIX.4.4|35=D|55=AAPL|453=2|448=BUYSIDE|447=D|452=1|448=VENUE|447=D|452=17|10=000|";
    let message = reader.text(row).unwrap();

    let field = message
        .as_field()
        .get_field_by_path("nopartyids")
        .expect("the counter's column");
    let DataType::List(item) = field.dtype() else {
        panic!("the group's own shape, got {}", field.dtype());
    };
    assert!(item.dtype().is_nested(), "a List of `item` Structs");
    assert!(
        message
            .by_tag(453)
            .unwrap()
            .as_sequence()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        message.by_tag(452).unwrap().as_sequence().unwrap().len(),
        2,
        "a member arrived twice, so it is two values"
    );

    // One anomaly, and it is the counter's: a member that merely arrived
    // twice states no count, so `PartyRole=1` is a role and not a group of
    // one.
    let anomalies: Vec<String> = message.anomalies().map(|held| held.to_string()).collect();
    assert_eq!(
        anomalies,
        ["nopartyids (453) states 2 occurrences and holds 0"],
        "{anomalies:?}"
    );
}

#[test]
fn a_group_addressed_by_its_tag_and_one_addressed_by_its_name_reach_one_column() {
    let reader = reader();
    // `FixKey` reads every string as a name, so a numeric group key resolves
    // only tag-first. Resolving it by name alone built a second, tag-less
    // column beside the counter's own and left the counter holding nothing.
    let by_tag = reader
        .text("MSGTYPE=D|453=2|453[0]=448=BUYSIDE|453[1]=448=VENUE")
        .unwrap();
    let by_name = reader
        .text("MSGTYPE=D|NOPARTYIDS=2|NOPARTYIDS[0]=PARTYID=BUYSIDE|NOPARTYIDS[1]=PARTYID=VENUE")
        .unwrap();

    for message in [&by_tag, &by_name] {
        let columns = message
            .as_field()
            .dtype()
            .as_fields()
            .expect("a struct root")
            .len();
        assert_eq!(columns, 2, "msgtype and the group, and nothing beside them");
        let occurrences = message.by_tag(453).unwrap().as_sequence().unwrap();
        assert_eq!(occurrences.len(), 2);
        assert!(message.anomalies().next().is_none());
    }
    assert_eq!(by_tag.by_tag(453).unwrap(), by_name.by_tag(453).unwrap());
}

#[test]
fn an_occurrence_that_named_no_member_is_kept_as_the_value_it_stated() {
    let reader = reader();
    // A bridge writes `NOPARTYIDS[0]=ONE` where it has nothing to name. The
    // occurrence has no member, but it is still an occurrence: dropping it
    // would make the row say the group held nothing, which is the one thing
    // the frame did not say.
    let message = reader
        .text("MSGTYPE=D|NOPARTYIDS=2|NOPARTYIDS[0]=ONE|NOPARTYIDS[1]=TWO")
        .unwrap();

    let occurrences = message.by_tag(453).unwrap().as_sequence().unwrap();
    assert_eq!(occurrences.len(), 2);
    assert_eq!(occurrences[0].as_str(), Some("ONE"));
    assert_eq!(occurrences[1].as_str(), Some("TWO"));
    // Two stated and two held, so there is nothing to report.
    assert!(message.anomalies().next().is_none());
}

#[test]
fn a_renamed_group_builds_one_column_at_the_version_that_renamed_it() {
    let reader = reader()
        .clone()
        .source_version("4.2".parse::<Version>().unwrap());
    // Tag 33 is `LinesOfText` before 4.4 and `NoLinesOfText` after. The
    // counter's slot is projected to the message's version and the members'
    // slot has to be projected with it, or one group builds two columns
    // carrying one tag.
    let message = reader
        .text("MSGTYPE=B|NOLINESOFTEXT=2|NOLINESOFTEXT[0]=TEXT=a|NOLINESOFTEXT[1]=TEXT=b")
        .unwrap();

    let names: Vec<&str> = message
        .as_field()
        .dtype()
        .as_fields()
        .expect("a struct root")
        .iter()
        .map(yggdryl::Field::name)
        .collect();
    assert_eq!(names, ["msgtype", "linesoftext"], "{names:?}");
    assert_eq!(message.by_tag(33).unwrap().as_sequence().unwrap().len(), 2);
    assert!(message.anomalies().next().is_none());
}

#[test]
fn a_group_the_dictionary_holds_as_a_large_list_still_states_its_count() {
    // The FIX layer reads a group as `List` or `LargeList` everywhere it looks
    // at one, so the miscount looks at the same pair: a dictionary that stored
    // its group in the wider variant is still a dictionary of groups.
    let mut party_id = DataType::Utf8.nullable_field("partyid");
    party_id.as_fix_mut().set_tag(448).unwrap();
    let item = DataType::from_fields([party_id])
        .unwrap()
        .required_field("item");
    let mut group = DataType::large_list(item).nullable_field("nopartyids");
    group.as_fix_mut().set_tag(453).unwrap();
    let mut symbol = DataType::Utf8.nullable_field("symbol");
    symbol.as_fix_mut().set_tag(55).unwrap();
    let registry = Arc::new(FixRegistry::from_fields([group, symbol]).unwrap());

    let message = FixReader::new(registry)
        .text("35=D|55=AAPL|453=2|10=0|")
        .unwrap();
    let anomalies: Vec<String> = message.anomalies().map(|held| held.to_string()).collect();
    assert_eq!(
        anomalies,
        ["nopartyids (453) states 2 occurrences and holds 0"],
        "{anomalies:?}"
    );
}

/// A capture that cannot print `0x01` writes it, and the frame is the same.
///
/// Every escaped spelling a log uses reaches the same columns as the byte it
/// stands for: the escape happened on the way into the log, not on the wire.
#[test]
fn a_printed_soh_spelling_reads_as_the_byte_it_stands_for() {
    let reader = reader();
    let wire = reader
        .text("recv 8=FIX.4.4\u{1}9=61\u{1}35=0\u{1}49=XPAR\u{1}10=017\u{1}")
        .expect("a numeric frame");
    for spelling in ["^A", "\\x01", "<SOH>", "{SOH}"] {
        let line = format!(
            "recv 8=FIX.4.4{spelling}9=61{spelling}35=0{spelling}49=XPAR{spelling}10=017{spelling} on session 3"
        );
        let held = reader.text(&line).expect("the same frame, escaped");
        assert_eq!(
            held.get_by_tag(35).and_then(Scalar::as_str),
            wire.get_by_tag(35).and_then(Scalar::as_str),
            "{spelling} lost the message type",
        );
        assert_eq!(
            held.get_by_tag(49).and_then(Scalar::as_str),
            Some("XPAR"),
            "{spelling} lost a body field",
        );
        // The prose after the checksum stays prose.
        assert_eq!(held.get_by_tag(10).and_then(Scalar::as_str), Some("017"));
    }
}

/// A bridge packs one group occurrence's members behind the FIX separator.
///
/// ULLINK writes EOT/ETX; a bridge relaying into a session writes SOH. Both
/// split, because inside an occurrence neither can be part of a value.
#[test]
fn a_group_occurrence_splits_on_either_packed_spelling() {
    let reader = reader();
    for separator in ["\u{4}\u{3}", "\u{1}"] {
        let line = format!(
            "toBridge #SYMBOL=TTF|#NOPARTYIDS=2|\
             #NOPARTYIDS[0]=PARTYID=BUYSIDE{separator}PARTYIDSOURCE=D{separator}PARTYROLE=1|\
             #NOPARTYIDS[1]=PARTYID=XPAR{separator}PARTYIDSOURCE=G{separator}PARTYROLE=17"
        );
        let held = reader.text(&line).expect("a bridge row");
        let parties = held
            .get_by_tag(453)
            .and_then(Scalar::as_sequence)
            .expect("the group");
        assert_eq!(parties.len(), 2);
        let first = parties[0].as_sequence().expect("one occurrence");
        // Three members, not one run under the first member's name.
        assert_eq!(first.len(), 3, "the packed members did not split");
        assert_eq!(first[0].as_str(), Some("BUYSIDE"));
    }
}

/// A row's content can never fail the batch it arrives in.
///
/// A message states the members that occurrence carried; the fixed schema
/// declares the dictionary's. Projecting places them by name and leaves the
/// rest null, so an occurrence that stated one member is a row rather than a
/// refusal.
#[test]
fn a_group_shorter_than_the_schema_declares_still_projects() {
    use yggdryl::{FixProjection, fix_schema};

    let registry = registry();
    let reader = FixReader::new(Arc::clone(&registry));
    let projection = FixProjection::new(&registry, "fix").expect("the fixed schema");
    let held = reader
        .text("toBridge #NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=BUYSIDE")
        .expect("a bridge row");

    let row = held.to_row(&projection);
    let field = fix_schema(&registry, "fix").expect("the fixed root");
    // The completion is what a batch does with the row; it must not refuse.
    field
        .canonicalize_value(row)
        .expect("a short occurrence is nulls, never a refusal");
}
