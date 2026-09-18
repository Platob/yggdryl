//! The codec's readers, over the committed dictionary and a real capture.

use super::SoleMessage;
use super::path;

use std::sync::Arc;

use arrow_array::RecordBatch;
use yggdryl::graph::Event;
use yggdryl::media::text::{TextBytes, TextLine};
use yggdryl::types::State;
use yggdryl::{DataType, Field, FixCodec, FixEntry, FixId, FixRegistry, Scalar};

fn registry() -> Arc<FixRegistry> {
    super::committed_registry()
}

fn codec() -> FixCodec {
    super::fixed_codec(registry())
}

/// The codec these fixtures are read under.
///
/// A fixture file of every shape a reader meets holds rows that state no
/// type at all - a bridge row of marked keys, a bare FIXML element, a
/// document - and `DEFAULT_REFUSED_MSGTYPES` is what keeps those out of a
/// live session's read. A suite about the readers is asking for them, and
/// says so here; what the default refuses is pinned by
/// `the_default_refusals_are_the_session_traffic_and_the_typeless_row`.
fn reader() -> FixCodec {
    codec().with_exclude_msgtypes::<[&str; 0], &str>([])
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

/// The capture rows that carry no message at all: a row that
/// opens no frame, states no bridge pair and carries no document states
/// nothing to read. `After Enrichment ->` and `heartbeat emitted seq=7`
/// write their pairs into a sentence, and a sentence names no separator for
/// them, so they are prose that happens to hold an `=`; `Referential(...)`
/// and the two remarks hold no readable pair at all.
const SILENT: [usize; 5] = [7, 8, 11, 12, 13];

#[test]
fn every_capture_row_states_its_messages_and_none_is_skipped() {
    let reader = reader();
    for (at, row) in CAPTURE.iter().enumerate() {
        let messages = reader
            .parse_line(row.as_bytes())
            .unwrap_or_else(|error| panic!("{row}: {error}"));
        let mut read = 0;
        for message in messages {
            let message = message.unwrap_or_else(|error| panic!("{row}: {error}"));
            // A message with no type is named `unknown` rather than refused:
            // every pair that parsed became a field and the entries record
            // the row.
            assert!(!message.as_field().name().is_empty(), "{row}");
            read += 1;
        }
        assert_eq!(read, usize::from(!SILENT.contains(&at)), "{row}");
    }

    // Empty input is the one typed error: a line that merely held nothing to
    // read is `Ok` and says so by yielding nothing.
    assert!(reader.parse_line(b"").is_err());
    assert!(
        reader
            .parse_line(b"no level printed by this plugin")
            .unwrap()
            .next()
            .is_none()
    );
    // `unknown` names a frame that stated no type, which is what a line
    // without a frame no longer answers.
    let typeless = reader.sole_line(b"8=FIX.4.4|49=S|56=T|10=0|").unwrap();
    assert_eq!(typeless.as_field().name(), "unknown");
    // The version and the two comp ids are the header's, so the checksum is
    // the whole of what the row holds.
    assert_eq!(typeless.entries().len(), 1);
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
        let message = reader.sole_line(row.as_bytes()).expect(row);
        assert_eq!(message.as_field().name(), msgtype, "{row}");
    }

    // The prose in front of the frame is not a field, and nothing after the
    // checksum is part of the message.
    let framed = reader
        .sole_line(b"sending >> 8=FIX.4.2|9=176|35=D|10=203| << queued seq=1092")
        .unwrap();
    // The entries are the content the frame carried: the version and the
    // type are the header's, so what is left is the body length, the
    // checksum and the dictionary's own derivation for an order.
    let keys: Vec<&str> = framed.entries().iter().map(|entry| entry.name()).collect();
    assert_eq!(keys, ["bodylength", "checksum"], "{keys:?}");
    assert_eq!(framed.header().beginstring(), "FIX.4.2");
    assert_eq!(framed.header().msgtype(), "D");
}

#[test]
fn message_codes_keep_the_complete_text_the_wire_declares() {
    let reader = reader();
    for code in ["U1", "UABC", "ConfigurationPlugin", "P Report Ack"] {
        for separator in [b'|', 1] {
            let line = format!(
                "MSGTYPE={code}{sep}SYMBOL=AAPL{sep}",
                sep = char::from(separator)
            );
            assert_eq!(FixCodec::infer_msgtype_text(&line), Some(code));
            let message = reader.sole_line(line.as_bytes()).unwrap();
            assert_eq!(message.by_tag(35).unwrap().as_str(), Some(code));
            assert_eq!(message.by_tag(55).unwrap().as_str(), Some("AAPL"));
            // The type is the header's own fact, spelled back under tag 35
            // with every byte the wire declared, beside the version the
            // dictionary's newest states for a row that named none.
            assert_eq!(
                String::from_utf8(message.into_bytes(separator)).unwrap(),
                format!(
                    "8=FIX.4.4{sep}35={code}{sep}55=AAPL{sep}",
                    sep = char::from(separator)
                )
            );
        }
    }
}

#[test]
fn numeric_group_counters_and_nested_occurrences_keep_their_declared_shapes() {
    let reader = reader();
    let wire = b"8=FIX.4.4|35=D|453=2|448=A|447=D|452=1|802=2|523=DESK|803=1|523=CLIENT|803=2|448=B|447=D|452=3|802=1|523=OTHER|803=3|55=AAPL|10=0|";
    let message = reader.parse_fix_line(wire).unwrap();
    assert_eq!(message.by_tag(453).unwrap(), Scalar::from(2_i32));
    assert_eq!(
        message
            .by_path(&path("Parties[0].PartyID"))
            .unwrap()
            .as_str(),
        Some("A")
    );
    assert_eq!(
        message
            .by_path(&path("Parties[1].PartyID"))
            .unwrap()
            .as_str(),
        Some("B")
    );
    assert_eq!(
        message.by_path(&path("Parties[0].NoPartySubIDs")).unwrap(),
        Scalar::from(2_i32)
    );
    assert_eq!(
        message
            .by_path(&path("Parties[0].PtysSubGrp[1].PartySubID"))
            .unwrap()
            .as_str(),
        Some("CLIENT")
    );
    assert_eq!(
        message
            .by_path(&path("Parties[1].PtysSubGrp[0].PartySubID"))
            .unwrap()
            .as_str(),
        Some("OTHER")
    );
    assert_eq!(message.by_tag(55).unwrap().as_str(), Some("AAPL"));
    assert_eq!(message.by_tag(10).unwrap().as_str(), Some("0"));
    assert!(
        message.get_by_tag(448).is_none(),
        "members stay inside the component"
    );
    // The order's `TimeInForce` is the dictionary's own derivation for a
    // `D`, and it is one of the event's facts rather than a child of the
    // content row, so it re-emits in the event's band ahead of the entries.
    assert_eq!(
        String::from_utf8(message.into_bytes(b'|')).unwrap(),
        "8=FIX.4.4|35=D|59=0|453=2|448=A|447=D|452=1|802=2|523=DESK|803=1|523=CLIENT|803=2|448=B|447=D|452=3|802=1|523=OTHER|803=3|55=AAPL|10=0|"
    );

    let schema = yggdryl::fix_schema(message.registry(), "fix").unwrap();
    let row = message.into_row(&schema).unwrap();
    let parties = row
        .get(schema.index_of("parties").unwrap())
        .unwrap()
        .as_sequence()
        .unwrap();
    assert_eq!(parties.len(), 2, "projection retains parsed occurrences");
}

#[test]
fn numeric_group_counts_describe_arrivals_without_allocating_stated_lengths() {
    let reader = reader();
    for (count, members, held) in [
        ("0", "", 0),
        ("0", "448=A|", 1),
        ("2", "448=A|447=D|", 1),
        ("invalid", "448=A|", 1),
        ("2147483648", "448=A|", 1),
        ("-1", "448=A|", 1),
    ] {
        let wire = format!("35=D|453={count}|{members}55=AAPL|10=0|");
        let message = reader.parse_fix_line(wire.as_bytes()).unwrap();
        assert_eq!(
            message
                .by_name("Parties")
                .unwrap()
                .as_sequence()
                .unwrap()
                .len(),
            held,
            "{wire}"
        );
        assert_eq!(message.by_tag(55).unwrap().as_str(), Some("AAPL"));
        // The counter describes the arrival: the group entry carries the
        // occurrences it holds, whatever length the wire claimed, so a
        // miscount, a word and an overflow all re-emit as what came.
        let held = i32::try_from(held).expect("a small count");
        assert_eq!(message.by_tag(453).unwrap(), Scalar::from(held), "{wire}");
        assert_eq!(
            String::from_utf8(message.into_bytes(b'|')).unwrap(),
            format!("8=FIX.4.4|35=D|59=0|453={held}|{members}55=AAPL|10=0|"),
            "{wire}"
        );
    }
}

#[test]
fn an_unknown_numeric_group_member_closes_the_scope_without_losing_pairs() {
    let reader = reader();
    let wire = b"35=D|453=1|448=A|9999=outside|447=D|55=AAPL|10=0|";
    let message = reader.parse_fix_line(wire).unwrap();
    assert_eq!(
        message
            .by_path(&path("Parties[0].PartyID"))
            .unwrap()
            .as_str(),
        Some("A")
    );
    assert_eq!(message.by_tag(9999).unwrap().as_str(), Some("outside"));
    assert_eq!(message.by_tag(447).unwrap().as_str(), Some("D"));
    assert_eq!(
        message
            .entries()
            .iter()
            .map(FixEntry::tag)
            .collect::<Vec<_>>(),
        [453, 0, 447, 55, 10]
    );
    assert_eq!(message.entries()[0].entries()[0].entries()[0].tag(), 448);
    assert_eq!(
        String::from_utf8(message.into_bytes(b'|')).unwrap(),
        "8=FIX.4.4|35=D|59=0|453=1|448=A|9999=outside|447=D|55=AAPL|10=0|"
    );
}

#[test]
fn a_tag_key_and_a_name_key_build_the_same_message() {
    let reader = reader();
    let by_tag = reader
        .sole_line(b"8=FIX.4.4|35=D|55=AAPL|54=1|10=0|")
        .unwrap();
    let by_name = reader
        .sole_line(b"8=FIX.4.4|MsgType=D|Symbol=AAPL|Side=1|10=0|")
        .unwrap();
    assert_eq!(by_tag.as_value(), by_name.as_value());
    assert_eq!(by_tag.as_field().dtype(), by_name.as_field().dtype());

    // Case and separators fold away, so a renderer's spelling still resolves,
    // and every spelling with the tag is the one id the field carries.
    let id = registry().field_by_tag(35).unwrap().as_fix().id().unwrap();
    for spelling in ["MsgType", "msgtype", "MSG_TYPE", "msg-type", "Msg Type"] {
        let row = format!("8=FIX.4.4|{spelling}=D|10=0|");
        let message = reader.sole_line(row.as_bytes()).expect(&row);
        assert_eq!(message.as_field().name(), "D", "{spelling}");
        assert_eq!(FixId::of(35, spelling).ok(), id, "{spelling}");
    }
}

#[test]
fn a_value_is_translated_typed_and_kept_as_it_arrived() {
    let reader = reader();
    let message = reader
        .sole_line(b"8=FIX.4.4|35=D|54=Buy|38=100|10=0|")
        .unwrap();

    // The row holds the translated code and the typed number.
    assert_eq!(message.by_name("side").unwrap().as_str(), Some("BUY"));
    assert_eq!(
        message.by_name("orderqty").unwrap(),
        Scalar::from(100.0_f64)
    );
    // The side is one of the event's own facts, so it is no entry of the
    // row: the wire spells it back under its own tag as the code the set
    // holds, never as the name the column reads it by.
    assert!(!message.entries().iter().any(|entry| entry.tag() == 54));
    assert!(
        String::from_utf8(message.into_bytes(b'|'))
            .unwrap()
            .contains("|54=1|")
    );
    // The identity is the tag with the dictionary field's name: one
    // id for the pair, the same under any spelling of the name, and the one
    // `get_by_id` answers the message through.
    let id = FixId::of(54, "Side").unwrap();
    assert_eq!(
        registry().field_by_tag(54).unwrap().as_fix().id().unwrap(),
        Some(id)
    );
    assert_eq!(FixId::of(54, "side").unwrap(), id);
    assert_eq!(message.get_by_id(id).unwrap().as_str(), Some("BUY"));
}

#[test]
fn an_unknown_key_is_kept_and_a_bad_value_is_null_rather_than_a_failure() {
    let reader = reader();
    let message = reader
        .sole_line(b"8=FIX.4.4|35=D|VenueOwnThing=x|9999=y|9=abc|10=0|")
        .unwrap();

    // An unknown name is kept under its own folded spelling, and an unknown
    // tag under its decimal one.
    assert_eq!(
        message.by_name("venueownthing").unwrap().as_str(),
        Some("x")
    );
    assert_eq!(message.by_name("9999").unwrap().as_str(), Some("y"));
    // The entries are the row read as a tree, so the key is the child's own
    // folded spelling and it names no tag.
    let held = message
        .entries()
        .iter()
        .find(|entry| entry.name().as_bytes() == b"venueownthing")
        .expect("the venue's own key");
    assert_eq!(held.tag(), 0, "an unknown key names no tag");

    // A `BodyLength` of `abc` nulls that field, and a child stating null is
    // no entry: what the row could not hold is what the wire no longer says.
    assert_eq!(message.by_name("bodylength").unwrap(), Scalar::Null);
    assert!(!message.entries().iter().any(|entry| entry.tag() == 9));
}

#[test]
fn a_stated_absence_produces_no_field_and_no_entry() {
    let reader = reader();
    for spelling in ["", "null", "NULL", "<null>"] {
        let row = format!("8=FIX.4.4|35=D|58={spelling}|10=0|");
        let message = reader.sole_line(row.as_bytes()).expect(&row);
        assert!(message.get_by_tag(58).is_none(), "{spelling}");
        assert!(
            !message.entries().iter().any(|entry| entry.tag() == 58),
            "{spelling}"
        );
    }
    // A value that merely contains `null`, and one a venue means literally,
    // both survive - the listing is a convention and has to be overridable.
    let kept = reader
        .sole_line(b"8=FIX.4.4|35=D|58=nullable|10=0|")
        .unwrap();
    assert_eq!(kept.by_tag(58).unwrap().as_str(), Some("nullable"));

    let literal = reader
        .clone()
        .with_null_values::<[&str; 0], &str>([])
        .sole_line(b"8=FIX.4.4|35=D|58=null|10=0|")
        .unwrap();
    assert_eq!(literal.by_tag(58).unwrap().as_str(), Some("null"));
}

#[test]
fn a_bridge_group_becomes_real_nesting_from_its_indexed_keys() {
    let reader = reader();
    let row = "MSGTYPE=D|#NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=SYNTH-01\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1";
    let message = reader.sole_line(row.as_bytes()).unwrap();

    let parties = message.by_name("parties").expect("the group");
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
        .get_field_by_path("parties")
        .expect("the group field");
    let DataType::List(item) = field.dtype() else {
        panic!("a list, got {}", field.dtype());
    };
    assert_eq!(item.name(), "party");
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
    let message = reader.sole_line(line).unwrap();
    assert_eq!(message, reader.parse_ullink_line(line).unwrap());
    let without_member_separators: &[u8] =
        b"|#SYMBOL=TTF|#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=2\
|#NOPARTYIDS[0]=PARTYID=BUYSIDEPARTYIDSOURCE=DPARTYROLE=1|";
    assert_eq!(
        message,
        reader.sole_line(without_member_separators).unwrap()
    );

    // Names resolve to tags, and each value takes its field's own type: a
    // quantity and a price are numbers, and a side is the packed code.
    assert_eq!(message.by_tag(55).unwrap().as_str(), Some("TTF"));
    assert_eq!(message.by_tag(54).unwrap().as_str(), Some("BUY"));
    assert_eq!(message.by_tag(38).unwrap().as_f64(), Some(1200.0));
    assert_eq!(message.by_tag(44).unwrap().as_f64(), Some(41.25));

    // The occurrence's packed members became three real fields under one
    // nesting, each resolved to its own tag rather than kept as text.
    let occurrences = message
        .by_name("parties")
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
    // The entries are the row read as a tree: the group heads its
    // occurrences and carries the count, each occurrence is the component
    // the dictionary declares with no value of its own, and the members the
    // packed pair was read into are nested beneath it.
    let counter = message
        .entries()
        .iter()
        .find(|entry| entry.tag() == 453)
        .expect("the group entry");
    assert_eq!(counter.value(), Some("1"));
    let keys: Vec<&str> = counter.entries().iter().map(|entry| entry.name()).collect();
    assert_eq!(keys, ["party"]);
    assert_eq!(counter.entries()[0].value(), None);
    let members: Vec<&str> = counter.entries()[0]
        .entries()
        .iter()
        .map(|entry| entry.name())
        .collect();
    assert_eq!(members, ["partyid", "partyidsource", "partyrole"]);
    assert!(
        message
            .entries()
            .iter()
            .all(|entry| !entry.name().as_bytes().starts_with(b"NOPARTYIDS[0]")),
        "an occurrence rides under its group, not beside it",
    );

    // Which is enough to reach the party the frame is about, through the
    // group the counter heads.
    assert_eq!(
        message
            .get_by_path(&path("Parties[0].PartyID"))
            .as_ref()
            .and_then(Scalar::as_str),
        Some("BUYSIDE")
    );
    assert_eq!(
        message
            .get_by_path(&path("Parties[0].PartyIDSource"))
            .as_ref()
            .and_then(Scalar::as_str),
        Some("D")
    );
}

#[test]
fn a_hash_key_yields_to_its_bare_twin_and_drops_its_mark_alone() {
    let reader = reader();
    // `ORDERID=123|#ORDERID=345` states one key twice: the bare pair is the
    // wire's word and the marked restatement goes, on whichever side of its
    // twin it arrived.
    for row in [
        b"MSGTYPE=D|ORDERID=123|#ORDERID=345".as_slice(),
        b"MSGTYPE=D|#ORDERID=345|ORDERID=123".as_slice(),
    ] {
        let message = reader.sole_line(row).unwrap();
        let spelled = String::from_utf8_lossy(row);
        assert_eq!(
            message.by_tag(37).unwrap().as_str(),
            Some("123"),
            "{spelled}"
        );
        assert!(message.by_name("#orderid").is_err(), "{spelled}");
        // The row is what remains, and the entries are that row: the
        // dictionary's own column and the day order it derives.
        let keys: Vec<&str> = message.entries().iter().map(|entry| entry.name()).collect();
        assert_eq!(keys, ["orderid"], "{spelled}");

        // Re-reading the emitted line answers the same message: the twin
        // judgment is idempotent.
        let emitted = message.into_bytes(b'|');
        let reread = reader.sole_line(&emitted).expect(&spelled);
        assert_eq!(reread, message, "{spelled}");
    }

    // Alone, the `#` is the bridge's own marker and drops: the key is the
    // dictionary field, exactly as a frame of only `#` keys always read.
    let single = reader.sole_line(b"MSGTYPE=D|#ORDERID=345").unwrap();
    assert_eq!(single.by_tag(37).unwrap().as_str(), Some("345"));
    assert!(single.by_name("#orderid").is_err());

    // A marked pair restating the bare one's bytes is a second spelling of
    // one pair, and one pair is what remains: row and entries alike, on
    // whichever side it arrived, however the bare key was spelled, and with
    // whatever space the row put around the values.
    for row in [
        b"MSGTYPE=D|ORDERID=123|#ORDERID=123".as_slice(),
        b"MSGTYPE=D|#ORDERID=123|ORDERID=123".as_slice(),
        b"MSGTYPE=D|OrderId=123|#ORDERID=123".as_slice(),
        b"MSGTYPE=D|ORDERID=123 |#ORDERID= 123".as_slice(),
    ] {
        let message = reader.sole_line(row).unwrap();
        let spelled = String::from_utf8_lossy(row);
        assert_eq!(
            message.by_tag(37).unwrap().as_str(),
            Some("123"),
            "{spelled}"
        );
        assert!(message.by_name("#orderid").is_err(), "{spelled}");
        let keys: Vec<&str> = message.entries().iter().map(|entry| entry.name()).collect();
        assert_eq!(keys, ["orderid"], "{spelled}");
        assert_eq!(
            String::from_utf8(message.into_bytes(b'|')).unwrap(),
            "8=FIX.4.4|35=D|59=0|37=123|",
            "{spelled}"
        );
    }
    // A value is a value: two spellings of the bytes are two values.
    let cased = reader
        .sole_line(b"MSGTYPE=D|ORDERID=abc|#ORDERID=ABC")
        .unwrap();
    assert_eq!(cased.by_tag(37).unwrap().as_str(), Some("abc"));
    assert!(cased.by_name("#orderid").is_err());

    // A bridge marks the name keys it writes into a numeric frame exactly as
    // it marks a row's, and they are judged by the same rules: alone the
    // mark drops, beside a twin of other bytes it stays, restating a twin it
    // goes.
    let framed = reader
        .sole_line(b"sending >> 8=FIX.4.2|35=UL|#SYMBOL=TTF|#SIDE=1|10=044|")
        .unwrap();
    assert_eq!(framed.by_tag(55).unwrap().as_str(), Some("TTF"));
    assert_eq!(framed.by_tag(54).unwrap().as_str(), Some("BUY"));
    let keys: Vec<&str> = framed.entries().iter().map(|entry| entry.name()).collect();
    assert_eq!(keys, ["symbol", "checksum"]);
    for line in [
        b"8=FIX.4.2|35=UL|ORDERID=123|#ORDERID=345|10=0|".as_slice(),
        b"8=FIX.4.2|35=UL|ORDERID=123|#ORDERID=123|10=0|".as_slice(),
    ] {
        let twinned = reader.sole_line(line).unwrap();
        assert_eq!(twinned.by_tag(37).unwrap().as_str(), Some("123"));
        assert!(twinned.by_name("#orderid").is_err());
        assert_eq!(
            String::from_utf8(twinned.into_bytes(b'|')).unwrap(),
            "8=FIX.4.2|35=UL|37=123|10=0|"
        );
    }

    // The row's type is read after the marks are judged, so a type the
    // bridge marked names the message - and the message is what splits a
    // separator-less occurrence of a counter half the dictionary shares at
    // the members its own group declares.
    let typed = reader
        .sole_line(b"#MSGTYPE=s|#NOSIDES=1|#NOSIDES[0]=SIDE=1CLORDID=X")
        .unwrap();
    assert_eq!(typed.by_tag(35).unwrap().as_str(), Some("s"));
    assert_eq!(
        typed
            .by_path(&path("SideCrossOrdModGrp[0].ClOrdID"))
            .unwrap(),
        Scalar::from("X")
    );
}

#[test]
fn a_twin_is_judged_by_fold_and_by_carrying_a_value() {
    let reader = reader();
    // The twin is the identity a key resolves by - case and separators fold
    // away - and the space-separated row splits into the same judgment.
    for row in [
        b"MSGTYPE=D|OrderId=123|#ORDERID=345".as_slice(),
        b"MSGTYPE=D|ORDER_ID=123|#ORDERID=345".as_slice(),
        b"MSGTYPE=D ORDERID=123 #ORDERID=345".as_slice(),
    ] {
        let message = reader.sole_line(row).unwrap();
        let spelled = String::from_utf8_lossy(row);
        assert_eq!(
            message.by_tag(37).unwrap().as_str(),
            Some("123"),
            "{spelled}"
        );
        assert!(message.by_name("#orderid").is_err(), "{spelled}");
    }

    // A bare twin that stated an absence was never sent, so the `#` is the
    // row's sole spelling and drops: the value lands under the dictionary
    // field exactly as a lone `#` key always did.
    for spelling in ["", "null", "<null>"] {
        let row = format!("MSGTYPE=D|ORDERID={spelling}|#ORDERID=345");
        let message = reader.sole_line(row.as_bytes()).expect(&row);
        assert_eq!(message.by_tag(37).unwrap().as_str(), Some("345"), "{row}");
        assert!(message.by_name("#orderid").is_err(), "{row}");
    }

    // A `#` occurrence numbered as one the bare spelling also numbers is one
    // occurrence of the group, read into the members it packs.
    let row: &[u8] = b"MSGTYPE=D|NOPARTYIDS[0]=whole|#NOPARTYIDS[0]=PARTYID=A\x04\x03PARTYROLE=1";
    let message = reader.sole_line(row).unwrap();
    let keys: Vec<&str> = message.entries().iter().map(|entry| entry.name()).collect();
    assert_eq!(keys, ["parties"], "{keys:?}");
    assert_eq!(
        super::sequence(message.by_name("parties").unwrap()).len(),
        1
    );
    assert_eq!(
        message.by_path(&path("Parties[0].PartyID")).unwrap(),
        Scalar::from("A")
    );

    // A group is one thing however many occurrences it states, so a bare
    // group claims every marked one of it: the marked occurrences are
    // numbered past the bare ones, the group counts what is left, and the
    // list is sorted by what each occurrence states - so the two spellings
    // of one row answer one group whichever side arrived first.
    for row in [
        b"MSGTYPE=D|NOPARTYIDS=1|NOPARTYIDS[0]=PARTYID=BARE\x04\x03PARTYROLE=1\
|#NOPARTYIDS=2|#NOPARTYIDS[0]=PARTYID=A\x04\x03PARTYROLE=1|#NOPARTYIDS[1]=PARTYID=B\x04\x03PARTYROLE=3"
            .as_slice(),
        b"MSGTYPE=D|#NOPARTYIDS=2|#NOPARTYIDS[0]=PARTYID=A\x04\x03PARTYROLE=1|#NOPARTYIDS[1]=PARTYID=B\x04\x03PARTYROLE=3\
|NOPARTYIDS=1|NOPARTYIDS[0]=PARTYID=BARE\x04\x03PARTYROLE=1"
            .as_slice(),
    ] {
        let message = reader.sole_line(row).unwrap();
        let spelled = String::from_utf8_lossy(row);
        assert_eq!(message.by_tag(453).unwrap().as_i64(), Some(3), "{spelled}");
        let ids: Vec<String> = super::sequence(message.by_name("parties").unwrap())
            .iter()
            .map(|occurrence| {
                occurrence.as_sequence().expect("an occurrence")[0]
                    .as_str()
                    .expect("a party id")
                    .to_owned()
            })
            .collect();
        assert_eq!(ids, ["A", "B", "BARE"], "{spelled}");
        // One counter and one group, whichever spelling the row opened with.
        let mut columns: Vec<&str> =
            message.as_field().fields().iter().map(Field::name).collect();
        columns.sort_unstable();
        assert_eq!(columns, ["nopartyids", "parties"], "{spelled}");
    }

    // A marked group restating the bare group pair for pair goes pair for
    // pair, so a row that says one party twice states one party.
    let restated: &[u8] = b"MSGTYPE=D|NOPARTYIDS=1|NOPARTYIDS[0]=PARTYID=A\x04\x03PARTYROLE=1\
|#NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=A\x04\x03PARTYROLE=1";
    let message = reader.sole_line(restated).unwrap();
    assert_eq!(message.by_tag(453).unwrap().as_i64(), Some(1));
    assert_eq!(
        message.by_path(&path("Parties[0].PartyID")).unwrap(),
        Scalar::from("A")
    );
    assert!(message.as_field().index_of("#nopartyids").is_none());
    assert!(
        message
            .entries()
            .iter()
            .all(|entry| !entry.name().as_bytes().starts_with(b"#"))
    );
}

#[test]
fn a_nested_occurrence_ends_at_the_close_the_bridge_wrote_or_at_the_dictionary() {
    let reader = reader();
    // Every packed value ends with the separator, so where the bridge packs
    // an occurrence inside another it writes two in a row: the empty
    // segment between them closes the inner one. A run carrying a close is
    // bounded by its closes alone, so the venue's own `VENUE_SEQ` packed
    // inside the sub-identifier stays inside it and `PARTYID` after the
    // close is the party's.
    let closed: &[u8] = b"MSGTYPE=D|NOPARTYIDS=2\
|NOPARTYIDS[0]=NOPARTYSUBIDS=1\x04\x03NOPARTYSUBIDS[0]=PARTYSUBID=a\x04\x03PARTYSUBIDTYPE=1\x04\x03VENUE_SEQ=7\x04\x03\x04\x03PARTYID=X\x04\x03PARTYROLE=1\x04\x03\
|NOPARTYIDS[1]=PARTYID=Y\x04\x03PARTYROLE=3\x04\x03";
    let message = reader.sole_line(closed).unwrap();
    // A group built from indexed keys is sorted by what each occurrence
    // states, so the party stating nothing but an id and a role comes first
    // and the one carrying the nested group after it.
    assert_eq!(
        message
            .by_path(&path("Parties[1].PtysSubGrp[0].PartySubID"))
            .unwrap(),
        Scalar::from("a")
    );
    assert_eq!(
        message.by_path(&path("Parties[1].PartyID")).unwrap(),
        Scalar::from("X")
    );
    assert_eq!(
        message.by_path(&path("Parties[0].PartyID")).unwrap(),
        Scalar::from("Y")
    );
    let members = |path: &str| -> Vec<String> {
        let group = message.as_field().get_field_by_path(path).expect(path);
        let DataType::List(item) = group.dtype() else {
            panic!("{path}: a list, got {}", group.dtype());
        };
        item.fields()
            .iter()
            .map(|field| field.name().to_owned())
            .collect()
    };
    assert!(
        members("parties.ptyssubgrp").contains(&"venueseq".to_owned()),
        "{:?}",
        members("parties.ptyssubgrp")
    );
    assert!(
        !members("parties").contains(&"venueseq".to_owned()),
        "{:?}",
        members("parties")
    );
    fn keys(entries: &[FixEntry], out: &mut Vec<String>) {
        for entry in entries {
            out.push(entry.name().to_owned());
            keys(entry.entries(), out);
        }
    }
    let mut arrived = Vec::new();
    keys(message.entries(), &mut arrived);
    // The entries are the row read as a tree, so the packed value is the
    // members it was read into: each occurrence under the group that heads
    // it, and the venue's own key inside the sub-occurrence it closed in.
    assert_eq!(
        arrived,
        [
            "parties",
            "party",
            "partyid",
            "partyrole",
            "party",
            "ptyssubgrp",
            "ptyssub",
            "partysubid",
            "partysubidtype",
            "venueseq",
            "partyid",
            "partyrole"
        ]
    );
    let venue = message
        .by_path(&path("Parties[1].PtysSubGrp[0].VenueSeq"))
        .expect("the venue's own key, inside the sub-identifier");
    assert_eq!(venue.as_str(), Some("7"));

    // A run carrying no close is bounded by what the dictionary declares:
    // the pairs after the sub-occurrence belong to it while its group
    // declares them, and the first it does not - the venue's own key here -
    // comes back up to the party, as does everything after it.
    let open: &[u8] = b"MSGTYPE=D|NOPARTYIDS=1\
|NOPARTYIDS[0]=NOPARTYSUBIDS=1\x04\x03NOPARTYSUBIDS[0]=PARTYSUBID=a\x04\x03PARTYSUBIDTYPE=1\x04\x03VENUE_SEQ=7\x04\x03PARTYID=X\x04\x03PARTYROLE=1\x04\x03";
    let message = reader.sole_line(open).unwrap();
    assert_eq!(
        message
            .by_path(&path("Parties[0].PtysSubGrp[0].PartySubID"))
            .unwrap(),
        Scalar::from("a")
    );
    assert_eq!(
        message
            .by_path(&path("Parties[0].PtysSubGrp[0].PartySubIDType"))
            .unwrap()
            .as_i64(),
        Some(1)
    );
    assert_eq!(
        message.by_path(&path("Parties[0].PartyID")).unwrap(),
        Scalar::from("X")
    );
    let party = message
        .as_field()
        .get_field_by_path("parties")
        .expect("parties");
    let DataType::List(item) = party.dtype() else {
        panic!("a list");
    };
    let names: Vec<&str> = item.fields().iter().map(yggdryl::Field::name).collect();
    assert!(names.contains(&"venueseq"), "{names:?}");

    // A close is an empty segment and nothing else: a segment of spaces
    // between two separators is residue, and the run stays bounded by the
    // dictionary.
    let spaced: &[u8] = b"MSGTYPE=D|NOPARTYIDS=1\
|NOPARTYIDS[0]=PARTYID=X\x04\x03 \x04\x03NOPARTYSUBIDS[0]=PARTYSUBID=a\x04\x03PARTYSUBIDTYPE=1\x04\x03PARTYROLE=1\x04\x03";
    let message = reader.sole_line(spaced).unwrap();
    assert_eq!(
        message
            .by_path(&path("Parties[0].PartyRole"))
            .unwrap()
            .as_i64(),
        Some(1)
    );
    assert_eq!(
        message
            .by_path(&path("Parties[0].PtysSubGrp[0].PartySubIDType"))
            .unwrap()
            .as_i64(),
        Some(1)
    );

    // A run of openers nothing closes nests as deep as a schema may and no
    // deeper: past that an opener is one more member, and a line of two
    // thousand of them reads rather than exhausting the stack.
    let mut deep = b"MSGTYPE=D|NOPARTYIDS=1|NOPARTYIDS[0]=\x04\x03".to_vec();
    for _ in 0..2000 {
        deep.extend_from_slice(b"NOPARTYSUBIDS[0]=PARTYSUBID=a\x04\x03");
    }
    let message = reader.sole_line(&deep).unwrap();
    assert_eq!(message.by_tag(453).unwrap().as_i64(), Some(1));
    // The entries are that row read as a tree, so they nest as deep as it
    // does and no deeper: one group, one occurrence, then the sub-group
    // repeated to the bound.
    let mut arrived = Vec::new();
    keys(message.entries(), &mut arrived);
    assert_eq!(&arrived[..3], ["parties", "party", "ptyssubgrp"]);
    assert_eq!(arrived.last().map(String::as_str), Some("ptyssubgrp"));
    assert_eq!(
        arrived.iter().filter(|name| *name == "ptyssubgrp").count(),
        65,
        "one level per opener the schema may nest"
    );
    let mut depth = 0;
    let mut level = message
        .get_by_path(&path("Parties[0].PtysSubGrp"))
        .and_then(|held| held.as_sequence().map(<[Scalar]>::to_vec));
    while let Some(held) = level {
        depth += 1;
        level = held
            .first()
            .and_then(Scalar::as_sequence)
            .and_then(|occurrence| occurrence.iter().find_map(Scalar::as_sequence))
            .map(<[Scalar]>::to_vec);
    }
    // As deep as a schema may nest and no deeper, which is the bound this
    // case exists to pin: the openers past it are members of the deepest
    // occurrence rather than another level of it, so two thousand of them
    // build a wide row instead of exhausting the stack. Sixty-five levels
    // because the party's own occurrence is the one the reader opened at
    // depth zero, and sixty-four more are what it may open under it.
    assert_eq!(depth, 65, "the openers nested to the bound and stopped");
}

#[test]
fn an_implicit_run_nests_a_declared_group_at_every_depth_and_lifts_what_no_level_declares() {
    let reader = reader();
    // No close anywhere, three levels deep: the party is bounded by what the
    // side's party group declares, and the sub-identifier it declares is
    // skipped whole inside it, so the party's own `PARTYID` lands on the
    // party and the second party opens after it.
    let row: &[u8] = b"MSGTYPE=AE|NOSIDES=1\
|NOSIDES[0]=NOPARTYIDS=2\x04\x03NOPARTYIDS[0]=NOPARTYSUBIDS=1\x04\x03NOPARTYSUBIDS[0]=PARTYSUBID=a\x04\x03PARTYSUBIDTYPE=1\x04\x03PARTYID=X\x04\x03PARTYROLE=1\x04\x03NOPARTYIDS[1]=PARTYID=Y\x04\x03PARTYROLE=3\x04\x03";
    let message = reader.sole_line(row).unwrap();
    assert_eq!(
        message
            .by_path(&path(
                "TrdCapRptSideGrp[0].Parties[1].PtysSubGrp[0].PartySubID"
            ))
            .unwrap(),
        Scalar::from("a")
    );
    assert_eq!(
        message
            .by_path(&path("TrdCapRptSideGrp[0].Parties[1].PartyID"))
            .unwrap(),
        Scalar::from("X")
    );
    assert_eq!(
        message
            .by_path(&path("TrdCapRptSideGrp[0].Parties[1].PartyRole"))
            .unwrap()
            .as_i64(),
        Some(1)
    );
    assert_eq!(
        message
            .by_path(&path("TrdCapRptSideGrp[0].Parties[0].PartyID"))
            .unwrap(),
        Scalar::from("Y")
    );

    // A pair no level declares ends the sub-identifier, then the party, and
    // lands on the side - the packed value's own occurrence - with
    // everything after it, because without a close nothing says where the
    // bridge meant it to go.
    let lifted: &[u8] = b"MSGTYPE=AE|NOSIDES=1\
|NOSIDES[0]=NOPARTYIDS=1\x04\x03NOPARTYIDS[0]=NOPARTYSUBIDS=1\x04\x03NOPARTYSUBIDS[0]=PARTYSUBID=a\x04\x03VENUE_SEQ=7\x04\x03PARTYID=X\x04\x03";
    let message = reader.sole_line(lifted).unwrap();
    let members = |path: &str| -> Vec<String> {
        let group = message.as_field().get_field_by_path(path).expect(path);
        let DataType::List(item) = group.dtype() else {
            panic!("{path}: a list, got {}", group.dtype());
        };
        item.fields()
            .iter()
            .map(|field| field.name().to_owned())
            .collect()
    };
    let side = members("trdcaprptsidegrp");
    assert!(
        side.contains(&"venueseq".to_owned()) && side.contains(&"partyid".to_owned()),
        "{side:?}"
    );
    assert!(
        !members("trdcaprptsidegrp.parties").contains(&"venueseq".to_owned()),
        "{:?}",
        members("trdcaprptsidegrp.parties")
    );
}

#[test]
fn a_frame_with_a_data_field_judges_its_marks_and_a_key_marked_twice_is_judged_once_more() {
    let reader = reader();
    // The frame's own marks are judged, and the row inside its data field by
    // its own bare spellings: the frame's `#SYMBOL` drops its mark, the
    // row's `#ORDERID` restates the row's bare one and goes.
    let nested = "MSGTYPE=D|ORDERID=9|#ORDERID=9|#SIDE=1";
    let frame = format!(
        "8=FIX.4.2|35=UL|#SYMBOL=TTF|212={}|213={nested}|10=0|",
        nested.len()
    );
    let message = reader.sole_line(frame.as_bytes()).unwrap();
    let keys: Vec<&str> = message.entries().iter().map(|entry| entry.name()).collect();
    assert_eq!(
        keys,
        ["xmldatalen", "xmldata", "symbol", "orderid", "checksum"]
    );
    assert_eq!(message.by_tag(55).unwrap().as_str(), Some("TTF"));
    assert_eq!(message.by_tag(37).unwrap().as_str(), Some("9"));
    assert_eq!(message.by_tag(54).unwrap().as_str(), Some("BUY"));
    assert!(message.by_name("#orderid").is_err());

    // A key marked twice is judged one mark at a time: `##ORDERID` twins
    // `#ORDERID` as `#ORDERID` twins `ORDERID`.
    let restated = reader
        .sole_line(b"MSGTYPE=D|#ORDERID=123|##ORDERID=123")
        .unwrap();
    assert_eq!(restated.by_tag(37).unwrap().as_str(), Some("123"));
    let keys: Vec<&str> = restated
        .entries()
        .iter()
        .map(|entry| entry.name())
        .collect();
    assert_eq!(keys, ["orderid"]);
    let beside = reader
        .sole_line(b"MSGTYPE=D|#ORDERID=123|##ORDERID=345")
        .unwrap();
    assert_eq!(beside.by_tag(37).unwrap().as_str(), Some("123"));
    let columns: Vec<&str> = beside
        .as_field()
        .fields()
        .iter()
        .map(|f| f.name())
        .collect();
    assert_eq!(columns, ["orderid"]);
    let alone = reader.sole_line(b"MSGTYPE=D|##ORDERID=345").unwrap();
    assert!(alone.get_by_tag(37).is_none());
    let at = alone
        .as_field()
        .index_of("#orderid")
        .expect("one mark gone");
    assert_eq!(
        alone.as_value().as_sequence().expect("a row")[at].as_str(),
        Some("345")
    );
}

#[test]
fn a_stated_length_is_honoured_only_where_it_ends_a_segment_the_frame_cut() {
    let reader = reader();
    let value = |frame: &[u8], tag: i32| -> Vec<u8> {
        let message = reader
            .parse_fix_line(frame)
            .unwrap_or_else(|error| panic!("{error}"));
        message
            .entries()
            .iter()
            .find(|entry| entry.tag() == tag)
            .unwrap_or_else(|| panic!("tag {tag}"))
            .value()
            .unwrap_or_default()
            .as_bytes()
            .to_vec()
    };

    // The length is what a data field is for: the frame's separator stands
    // inside the value and the count is what reaches past it.
    assert_eq!(
        value(b"8=FIX.4.4|9=0|35=D|95=5|96=AB|CD|10=000|", 96),
        b"AB|CD"
    );

    // A count that stops in the middle of the value is honoured by nothing:
    // the frame's own cut is kept, because widening to a byte the frame never
    // separated at would truncate the field and drop what stood past it from
    // the message and from the wire alike.
    assert_eq!(value(b"8=FIX.4.4|35=D|95=2|96=ABCDE|10=000|", 96), b"ABCDE");
    // Nor one that reaches past everything the line wrote.
    assert_eq!(
        value(b"8=FIX.4.4|35=D|95=99|96=ABCDE|10=000|", 96),
        b"ABCDE"
    );

    // `XmlData` takes the trailer instead where the count lands nowhere,
    // because a bridge writes it last and a log that printed each control
    // byte as a glyph carries more bytes than the bridge counted.
    assert_eq!(
        value(b"8=FIX.4.4|35=D|212=5|213=ABCDEFGH|10=0|", 213),
        b"ABCDEFGH"
    );
    // Including a count landing inside the checksum's own key, which is the
    // one that costs a message its trailer: the entry after any span at all
    // is the tag-keyed `10` the frame closes with, so `10` being a tag is no
    // evidence that the span ended a segment.
    let trailered: &[u8] = b"8=FIX.4.4|35=D|212=7|213=A|B|C|10=000|";
    assert_eq!(value(trailered, 213), b"A|B|C");
    assert_eq!(
        reader
            .parse_fix_line(trailered)
            .unwrap()
            .by_tag(10)
            .unwrap()
            .as_str(),
        Some("000"),
        "the checksum is still the message's"
    );
}

#[test]
fn a_frame_reader_takes_the_frame_and_leaves_the_transports_own_pairs() {
    let reader = reader();
    // One bound for every door: a log line printing its own `k=v` in front of
    // the frame it quoted states neither field, whether the line arrives at
    // the row reader or here. The line still said them - `TextEntries` keeps
    // every pair it saw - but a message that swallowed them would answer them
    // by name and re-emit them as its own.
    let line: &[u8] = b"ts=12|thread=7|8=FIX.4.4|35=D|11=A1|10=000|";
    let message = reader.parse_fix_line(line).unwrap();
    let keys: Vec<&str> = message.entries().iter().map(|entry| entry.name()).collect();
    assert_eq!(keys, ["clordid", "checksum"]);
    assert!(message.get_by_name("ts").is_none());
    assert_eq!(
        String::from_utf8(message.into_bytes(b'|')).unwrap(),
        "8=FIX.4.4|35=D|59=0|11=A1|10=000|"
    );

    // A body that opens at its own first pair is bounded at zero, so nothing
    // a caller hands in whole is dropped.
    let bare: &[u8] = b"8=FIX.4.4|35=D|11=A1|10=000|";
    assert_eq!(
        reader.parse_fix_line(bare).unwrap().into_bytes(b'|'),
        message.into_bytes(b'|')
    );
}

#[test]
fn a_mark_is_judged_where_a_row_is_split_and_nowhere_else() {
    let reader = reader();
    // A member a rendered occurrence carries marked is a member like any
    // other: the reader opens a marked occurrence packed inside a party,
    // bounded by its close, and the builder nests it as its key says - an
    // unknown group of the party's own.
    let packed: &[u8] = b"MSGTYPE=D|NOPARTYIDS=1\
|NOPARTYIDS[0]=PARTYID=a\x04\x03#NOPARTYSUBIDS[0]=PARTYSUBID=s\x04\x03PARTYSUBIDTYPE=1\x04\x03\x04\x03PARTYROLE=1\x04\x03";
    let message = reader.sole_line(packed).unwrap();
    assert_eq!(
        message
            .by_path(&path("Parties[0].PartyRole"))
            .unwrap()
            .as_i64(),
        Some(1)
    );
    let party = message
        .as_field()
        .get_field_by_path("parties")
        .expect("parties");
    let DataType::List(item) = party.dtype() else {
        panic!("a list");
    };
    let marked = item.field("#nopartysubids").expect("the marked group");
    let DataType::List(sub) = marked.dtype() else {
        panic!("a list, got {}", marked.dtype());
    };
    let names: Vec<&str> = sub.fields().iter().map(yggdryl::Field::name).collect();
    assert_eq!(names, ["partysubid", "partysubidtype"]);

    // Pairs a caller split are built as they are: a mark is neither judged
    // nor dropped, and a marked occurrence is one flat child.
    let built = reader
        .parse_pairs([
            (b"MSGTYPE".as_slice(), b"D".as_slice()),
            (b"SYMBOL", b"S"),
            (b"#SYMBOL", b"S"),
            (b"#NOPARTYIDS[0]", b"whole"),
        ])
        .unwrap();
    assert_eq!(built.by_tag(55).unwrap().as_str(), Some("S"));
    assert_eq!(built.by_name("#symbol").unwrap().as_str(), Some("S"));
    let at = built
        .as_field()
        .index_of("#nopartyids[0]")
        .expect("the marked occurrence");
    assert_eq!(
        built.as_value().as_sequence().expect("a row")[at].as_str(),
        Some("whole")
    );
}

#[test]
fn a_code_declared_under_another_name_is_a_second_message_and_the_bare_code_answers_the_first() {
    // Message codes live in one namespace under the same rule as fields: a
    // definition re-declaring a code under another name is a second message,
    // reached by its name, while the bare code keeps answering the first
    // holder; re-declaring it under the same folded name folds into the
    // stored one. The codec reads a frame under the first holder's grammar.
    let declare = |name: &str| {
        let mut allocation = DataType::Int32.nullable_field("AllocQty");
        allocation.as_fix_mut().set_tag(80).unwrap();
        let mut message = DataType::from_fields([allocation])
            .unwrap()
            .required_field(name);
        message.as_fix_mut().set_msgtype("J").unwrap();
        message
    };
    let frame: &[u8] = b"8=FIX.4.4|35=J|70=A1|78=1|79=ACC|80=5|10=0|";
    let grouped = |message: &yggdryl::FixMsg| message.get_by_name("allocgrp").is_some();

    // Undeclared: the standard AllocationInstruction holds the code and
    // folds the allocation group the frame states flat.
    let mut registry = registry().as_ref().clone();
    let standard = registry
        .msgtype("J")
        .expect("the standard holder")
        .name()
        .to_owned();
    assert_eq!(
        registry.msgtype("allocationinstruction").unwrap().as_str(),
        "J"
    );
    let messages = super::msgtypes(&registry).count();
    let none = super::fixed_codec(Arc::new(registry.clone()));
    assert!(grouped(&none.sole_line(frame).unwrap()));

    // Declared under another name: a second message, reached by its name and
    // carrying the code, while the bare code still answers the first holder.
    registry.insert(declare("AllocIn")).unwrap();
    assert_eq!(super::msgtypes(&registry).count(), messages + 1);
    assert_eq!(registry.msgtype("J").unwrap().name(), standard);
    assert_eq!(registry.msgtype("AllocIn").unwrap().as_str(), "J");
    assert_eq!(registry.msgtype("alloc_in").unwrap().name(), "AllocIn");
    let once = super::fixed_codec(Arc::new(registry.clone()));
    let message = once.sole_line(frame).unwrap();
    assert!(grouped(&message), "the first holder's grammar");
    let occurrences = super::sequence(
        message
            .get_by_name("allocgrp")
            .expect("the group's occurrences"),
    );
    assert_eq!(occurrences.len(), 1, "{occurrences:?}");
    assert!(
        message.get_by_tag(80).is_none(),
        "tag 80 nests, as the holder declares"
    );

    // Re-declared under the same folded name: it folds into the stored one,
    // and no third message appears.
    let added = registry.add_field(declare("Alloc_In")).unwrap();
    assert!(!added, "merged, not added");
    assert_eq!(super::msgtypes(&registry).count(), messages + 1);
    assert_eq!(registry.msgtype("J").unwrap().name(), standard);
    assert_eq!(registry.msgtype("allocin").unwrap().as_str(), "J");
}

#[test]
fn indexed_occurrences_are_built_by_index_and_a_gap_is_null() {
    let reader = reader();
    // Out of order and gapped: `[2]` before `[0]`, with `[1]` absent.
    let message = reader
        .sole_line(b"MSGTYPE=D|PartyID[2]=third|PartyID[0]=first")
        .unwrap();
    let held = message.by_name("partyid").expect("the repeated field");
    let values = held.as_sequence().expect("occurrences");
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
        .sole_line(b"8=FIX.4.4|55=AAPL|35=D|9=100|10=000|")
        .unwrap();
    let names: Vec<&str> = message
        .as_field()
        .dtype()
        .as_fields()
        .expect("a struct root")
        .iter()
        .map(yggdryl::Field::name)
        .collect();
    // The row holds the content alone, in header/body/trailer order: the
    // version and the type are the header's, held typed beside it, and the
    // dictionary's own derivation closes the order.
    assert_eq!(names, ["bodylength", "symbol", "checksum"], "{names:?}");
    assert_eq!(message.header().beginstring(), "FIX.4.4");
    assert_eq!(message.header().msgtype(), "D");
}

#[test]
fn a_message_re_emits_from_its_entries_and_reads_back_equal() {
    let reader = reader();
    let row = "8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|38=100|10=000|";
    let message = reader.sole_line(row.as_bytes()).unwrap();

    // The wire is the message as the crate holds it, not the line it came
    // from: the header's tags lead, then the event's own, then the row -
    // the lane a buy of a hundred filled and the day order the dictionary
    // derives among them - each coded fact spelled as the wire spells it.
    let emitted = "8=FIX.4.4|35=D|38=100|54=1|59=0|134=100|11=ORDER-1|55=AAPL|10=000|";
    let bytes = message.into_bytes(b'|');
    assert_eq!(String::from_utf8(bytes.clone()).unwrap(), emitted);
    assert_eq!(message.into_text('|').unwrap(), emitted);
    assert_eq!(message.header().beginstring(), "FIX.4.4");

    // And reading that wire back answers the same message: what it emits is
    // what it is.
    let again = reader
        .clone()
        .with_separator(b'|')
        .parse_fix_line(&bytes)
        .unwrap();
    assert_eq!(again.entries(), message.entries());
    assert_eq!(again.as_value(), message.as_value());
}

#[test]
fn a_version_never_renames_or_retypes_the_column_a_tag_lands_in() {
    let reader = reader();

    // Tag 32 is `LastShares` typed `int` in 4.0, `LastShares` typed `Qty` in
    // 4.2 and `LastQty` from 4.3 on. A row read at 4.2 still builds the
    // dictionary's own column, because a tag that renamed itself per version
    // is a tag no two captures of one venue could be read together on.
    let at_42 = super::dated_line(&reader, b"8=FIX.4.2|35=8|32=100|10=0|", "4.2").unwrap();
    let at_new = super::dated_line(&reader, b"8=FIX.4.4|35=8|32=100|10=0|", "5.0.2").unwrap();
    // Tag 32 is one of the facts the event holds typed, so neither reading
    // puts a column of either spelling on the content row, and both answer
    // the one fact - by its tag, by its current name, and by the name 4.2
    // spelled it with.
    for held in [&at_42, &at_new] {
        assert!(held.as_field().index_of("lastqty").is_none());
        assert!(held.as_field().index_of("lastshares").is_none());
    }
    assert_eq!(at_42.by_tag(32).unwrap(), at_new.by_tag(32).unwrap());
    assert_eq!(at_42.by_name("lastqty").unwrap(), at_42.by_tag(32).unwrap());
    assert_eq!(
        at_42.by_name("lastshares").unwrap(),
        at_42.by_tag(32).unwrap(),
        "nothing the version knew is lost"
    );
}

#[test]
fn a_numeric_frame_nests_its_group_members_as_the_dictionary_declares_them() {
    let reader = reader();
    // A numeric frame states no structure: the counter arrives and its
    // members follow. The dictionary's declaration is what says where one
    // occurrence ends and the next begins - the first declared member opens
    // an occurrence, a member the occurrence already holds opens the next,
    // and a tag the group does not declare closes it - so the frame lands in
    // the shape a bridge's indexed keys would have built, with the numeric
    // delimiter opening each Party while NoPartyIDs keeps the count.
    let row =
        "8=FIX.4.4|35=D|55=AAPL|453=2|448=BUYSIDE|447=D|452=1|448=VENUE|447=D|452=17|54=1|10=000|";
    let message = reader.sole_line(row.as_bytes()).unwrap();

    let field = message
        .as_field()
        .get_field_by_path("parties")
        .expect("the separate logical group");
    let DataType::List(item) = field.dtype() else {
        panic!("the group's own shape, got {}", field.dtype());
    };
    assert!(item.dtype().is_nested(), "a List of `item` Structs");
    let occurrences = super::sequence(message.by_name("parties").unwrap());
    assert_eq!(occurrences.len(), 2, "one occurrence per delimiter");
    let ids: Vec<&str> = occurrences
        .iter()
        .map(|occurrence| occurrence.as_sequence().unwrap()[0].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["BUYSIDE", "VENUE"]);
    // The counter is a scalar column of its own and keeps what the frame
    // stated, beside the group the occurrences reach.
    assert_eq!(message.by_tag(453).unwrap(), Scalar::from(2_i32));
    assert_eq!(
        message.by_path(&path("Parties[0].PartyRole")).unwrap(),
        Scalar::from(1_i32)
    );
    assert_eq!(
        message.by_path(&path("Parties[1].PartyRole")).unwrap(),
        Scalar::from(17_i32)
    );
    // A member lives in its occurrence and nowhere else: nothing is flat at
    // the root, and the field after the group is the order's own again.
    assert!(message.get_by_tag(448).is_none(), "no flat party id");
    assert!(message.get_by_tag(452).is_none(), "no flat party role");
    assert_eq!(message.by_tag(54).unwrap().as_str(), Some("BUY"));
    // The entries are the row read as a tree: the group heads its two
    // occurrences and each member is a child of the one it arrived in.
    let counter = message
        .entries()
        .iter()
        .find(|entry| entry.tag() == 453)
        .expect("the group entry");
    assert_eq!(counter.entries().len(), 2);
    assert_eq!(
        counter
            .entries()
            .iter()
            .map(|occurrence| occurrence.entries().len())
            .collect::<Vec<_>>(),
        [3, 3]
    );
}

#[test]
fn a_counter_a_numeric_frame_states_twice_at_one_level_appends_to_its_group() {
    let reader = reader();
    // A dictionary that does not nest one group inside another reads a
    // frame that does as two statements of the inner counter at one level.
    // Nothing is lost for it: the second counter appends to what the first
    // gathered, and the miscount says the group holds more than one
    // counter stated.
    let row = "8=FIX.4.4|35=x|320=R1|146=2|55=AAPL|454=1|455=US0378331005|456=4|55=MSFT|454=2|455=US5949181045|456=4|455=MSFT.O|456=5|10=0|";
    let message = reader.sole_line(row.as_bytes()).unwrap();
    let alternates = super::sequence(message.by_name("secaltidgrp").unwrap());
    let ids: Vec<&str> = alternates
        .iter()
        .map(|occurrence| occurrence.as_sequence().unwrap()[0].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["US0378331005", "US5949181045", "MSFT.O"]);
}

#[test]
fn a_numeric_frame_nests_a_group_inside_an_occurrence_of_another() {
    // A dictionary declaring one group inside another reads the inner
    // counter as a member: it opens its group inside the occurrence being
    // filled, the members that follow fill that group first, and a member of
    // the outer group closes it.
    let mut sub_id = DataType::utf8().nullable_field("partysubid");
    sub_id.as_fix_mut().set_tag(523).unwrap();
    let mut sub_type = DataType::Int32.nullable_field("partysubidtype");
    sub_type.as_fix_mut().set_tag(803).unwrap();
    let sub_item = DataType::from_fields([sub_id.clone(), sub_type.clone()])
        .unwrap()
        .required_field("partysub");
    let mut sub_count = DataType::Int32.nullable_field("nopartysubids");
    sub_count.as_fix_mut().set_tag(802).unwrap();
    let mut subs = DataType::list(sub_item).nullable_field("ptyssubgrp");
    subs.as_fix_mut().set_counter(802).unwrap();
    let mut party_id = DataType::utf8().nullable_field("partyid");
    party_id.as_fix_mut().set_tag(448).unwrap();
    let mut role = DataType::Int32.nullable_field("partyrole");
    role.as_fix_mut().set_tag(452).unwrap();
    let item = DataType::from_fields([
        party_id.clone(),
        role.clone(),
        sub_count.clone(),
        subs.clone(),
    ])
    .unwrap()
    .required_field("party");
    let mut count = DataType::Int32.nullable_field("nopartyids");
    count.as_fix_mut().set_tag(453).unwrap();
    let mut parties = DataType::list(item).nullable_field("parties");
    parties.as_fix_mut().set_counter(453).unwrap();
    let mut symbol = DataType::utf8().nullable_field("symbol");
    symbol.as_fix_mut().set_tag(55).unwrap();
    // As the generated dictionary does, every member is also a field of its
    // own by tag and every group is a named definition headed by its
    // counter's own field: the item declares the shape, the tag resolves the
    // member.
    let mut registry =
        FixRegistry::from_fields([count, sub_count, party_id, role, sub_id, sub_type, symbol])
            .unwrap();
    registry.insert(subs).unwrap();
    registry.insert(parties).unwrap();
    let registry = Arc::new(registry);

    let row = "35=D|55=AAPL|453=2|448=A|452=1|802=2|523=S1|803=1|523=S2|803=2|448=B|452=3|10=0|";
    let message = super::fixed_codec(registry)
        .sole_line(row.as_bytes())
        .unwrap();
    let occurrences = super::sequence(message.by_name("parties").unwrap());
    assert_eq!(occurrences.len(), 2);
    let first = occurrences[0].as_sequence().unwrap();
    assert_eq!(first[0].as_str(), Some("A"));
    let subs = first[3]
        .as_sequence()
        .expect("the nested group in the first occurrence");
    let sub_ids: Vec<&str> = subs
        .iter()
        .map(|occurrence| occurrence.as_sequence().unwrap()[0].as_str().unwrap())
        .collect();
    assert_eq!(sub_ids, ["S1", "S2"]);
    let second = occurrences[1].as_sequence().unwrap();
    assert_eq!(second[0].as_str(), Some("B"));
    assert!(second[3].is_null(), "the second party stated no sub-ids");
    assert!(
        message.by_tag(802).is_err(),
        "the inner counter is no root column"
    );
}

#[test]
fn a_state_code_is_the_letter_the_wire_wrote_and_the_crates_state_ranks_it() {
    let reader = reader();
    // `D` is one letter in two code sets: Restated as an `ExecType` and
    // AcceptedForBidding as an `OrdStatus`. The dictionary's own columns keep
    // the letter the wire wrote, and the crate's `state` is the one typed
    // column: it takes `OrdStatus`, else `ExecType`, and ranks the code it
    // finds, so a letter two sets share ranks once whichever carried it.
    let restated = reader.sole_line(b"8=FIX.4.4|35=8|150=D|10=0|").unwrap();
    assert_eq!(restated.by_tag(150).unwrap().as_str(), Some("D"));
    let bidding = reader.sole_line(b"8=FIX.4.4|35=8|39=D|10=0|").unwrap();
    assert_eq!(bidding.by_tag(39).unwrap().as_str(), Some("D"));
    let ranked = State::from_spelling("AcceptedForBidding").unwrap();
    for held in [&restated, &bidding] {
        assert_eq!(
            held.by_tag(yggdryl::STATE_TAG_NAME.0).unwrap().as_str(),
            Some(ranked.as_str())
        );
    }
    // A code both sets spell alike reads alike, whichever field carries it.
    let new = reader
        .sole_line(b"8=FIX.4.4|35=8|39=0|150=0|10=0|")
        .unwrap();
    assert_eq!(new.by_tag(39).unwrap(), new.by_tag(150).unwrap());
}

#[test]
fn a_group_addressed_by_its_tag_and_one_addressed_by_its_name_reach_one_column() {
    let reader = reader();
    // `FixKey` reads every string as a name, so a numeric group key resolves
    // only tag-first. Resolving it by name alone built a second, tag-less
    // column beside the counter's own and left the counter holding nothing.
    let by_tag = reader
        .sole_line(b"MSGTYPE=D|453=2|453[0]=448=BUYSIDE|453[1]=448=VENUE")
        .unwrap();
    let by_name = reader
        .sole_line(
            b"MSGTYPE=D|NOPARTYIDS=2|NOPARTYIDS[0]=PARTYID=BUYSIDE|NOPARTYIDS[1]=PARTYID=VENUE",
        )
        .unwrap();

    for message in [&by_tag, &by_name] {
        let columns: Vec<&str> = message
            .as_field()
            .dtype()
            .as_fields()
            .expect("a struct root")
            .iter()
            .map(yggdryl::Field::name)
            .collect();
        assert_eq!(
            columns,
            ["nopartyids", "parties"],
            "one counter, one group and the day order the dictionary derives"
        );
        assert_eq!(message.by_tag(453).unwrap(), Scalar::from(2_i32));
        let occurrences = super::sequence(message.by_name("parties").unwrap());
        assert_eq!(occurrences.len(), 2);
    }
    assert_eq!(
        by_tag.by_name("parties").unwrap(),
        by_name.by_name("parties").unwrap()
    );
}

#[test]
fn an_unnamed_occurrence_opens_the_declared_component_under_its_counter() {
    let message = reader()
        .sole_line(b"MSGTYPE=D|NOPARTYIDS=2|NOPARTYIDS[0]=ONE|NOPARTYIDS[1]=TWO")
        .unwrap();
    assert_eq!(message.by_tag(453).unwrap(), Scalar::from(2_i32));
    let occurrences = super::sequence(message.by_name("parties").unwrap());
    assert_eq!(occurrences, [Scalar::Null, Scalar::Null]);
    let group = message.as_field().get_field_by_path("parties").unwrap();
    let DataType::List(item) = group.dtype() else {
        panic!("{}", group.dtype());
    };
    assert_eq!(item.name(), "party");
    assert!(matches!(item.dtype(), DataType::Structure(_)));
    assert!(item.is_nullable());
    // The group is what the wire says of it: two occurrences under their
    // count, each one the declared component states nothing in.
    assert_eq!(
        String::from_utf8(message.into_bytes(b'|')).unwrap(),
        "8=FIX.4.4|35=D|59=0|453=2|"
    );
}

#[test]
fn a_renamed_group_builds_one_column_under_the_name_the_dictionary_holds() {
    let reader = reader();
    // Tag 33 is `LinesOfText` before 4.4 and `NoLinesOfText` after, and the
    // message is read at 4.2 - but a field is one column under the one name
    // the dictionary holds it by, whatever version the row is read at. What
    // 4.2 called it stays readable as an alias beside that name.
    let message = super::dated_line(
        &reader,
        b"MSGTYPE=B|NOLINESOFTEXT=2|NOLINESOFTEXT[0]=TEXT=a|NOLINESOFTEXT[1]=TEXT=b",
        "4.2",
    )
    .unwrap();

    let names: Vec<&str> = message
        .as_field()
        .dtype()
        .as_fields()
        .expect("a struct root")
        .iter()
        .map(yggdryl::Field::name)
        .collect();
    assert_eq!(names, ["nolinesoftext", "linesoftextgrp"], "{names:?}");
    // One group, one column: the counter and its members reach one slot.
    assert_eq!(
        message
            .by_name("linesoftextgrp")
            .unwrap()
            .as_sequence()
            .unwrap()
            .len(),
        2
    );
    let group = message
        .as_field()
        .field("nolinesoftext")
        .expect("the counter's column");
    assert!(
        group.as_fix().names().any(|held| held == "linesoftext"),
        "the 4.2 spelling is still readable off the column",
    );
}

#[test]
fn a_group_the_dictionary_holds_as_a_large_list_still_states_its_count() {
    // The FIX layer reads a group as `List` or `LargeList` everywhere it looks
    // at one, so the miscount looks at the same pair: a dictionary that stored
    // its group in the wider variant is still a dictionary of groups.
    let mut party_id = DataType::utf8().nullable_field("partyid");
    party_id.as_fix_mut().set_tag(448).unwrap();
    let item = DataType::from_fields([party_id])
        .unwrap()
        .required_field("item");
    let mut group = DataType::large_list(item.clone()).nullable_field("parties");
    group.as_fix_mut().set_counter(453).unwrap();
    group.as_fix_mut().set_component(item.name()).unwrap();
    let mut counter = DataType::Int32.nullable_field("nopartyids");
    counter.as_fix_mut().set_tag(453).unwrap();
    let mut symbol = DataType::utf8().nullable_field("symbol");
    symbol.as_fix_mut().set_tag(55).unwrap();
    let mut registry = FixRegistry::from_fields([counter, symbol]).unwrap();
    registry.insert(item).unwrap();
    registry.insert(group).unwrap();
    let registry = Arc::new(registry);

    let message = super::fixed_codec(registry)
        .sole_line(b"35=D|55=AAPL|453=2|10=0|")
        .unwrap();
    // The counter describes the group: it holds no occurrence, so the count
    // is zero however wide a list the dictionary stores the group in.
    assert_eq!(message.by_tag(453).unwrap(), Scalar::from(0_i32));
    assert!(
        message
            .get_by_name("parties")
            .is_none_or(|held| held.as_sequence().is_none_or(<[Scalar]>::is_empty))
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
        .sole_line(b"recv 8=FIX.4.4\x019=61\x0135=0\x0149=XPAR\x0110=017\x01")
        .expect("a numeric frame");
    for spelling in ["^A", "\\x01", "<SOH>", "{SOH}"] {
        let line = format!(
            "recv 8=FIX.4.4{spelling}9=61{spelling}35=0{spelling}49=XPAR{spelling}10=017{spelling} on session 3"
        );
        let held = reader
            .sole_line(line.as_bytes())
            .expect("the same frame, escaped");
        assert_eq!(
            held.get_by_tag(35).as_ref().and_then(Scalar::as_str),
            wire.get_by_tag(35).as_ref().and_then(Scalar::as_str),
            "{spelling} lost the message type",
        );
        assert_eq!(
            held.get_by_tag(49).as_ref().and_then(Scalar::as_str),
            Some("XPAR"),
            "{spelling} lost a body field",
        );
        // The prose after the checksum stays prose.
        assert_eq!(
            held.get_by_tag(10).as_ref().and_then(Scalar::as_str),
            Some("017")
        );
    }
}

/// A bridge packs one group occurrence's members behind a control separator,
/// or concatenates them without one.
///
/// ULLINK writes EOT/ETX; a bridge relaying into a session writes SOH. Both
/// are authoritative when present. With neither, the addressed group's four
/// declared names provide the boundaries, including the longer
/// `PartyRoleQualifier` beside `PartyRole`.
#[test]
fn a_group_occurrence_splits_on_explicit_or_declared_boundaries() {
    let reader = reader();
    let mut messages = Vec::new();
    for separator in ["\x04\x03", "\x01", ""] {
        let line = format!(
            "toBridge #SYMBOL=TTF|#NOPARTYIDS=1|\
             #NOPARTYIDS[0]=PARTYID=BUYSIDE{separator}PARTYIDSOURCE=D{separator}\
             PARTYROLE=1{separator}PARTYROLEQUALIFIER=0"
        );
        let held = reader.sole_line(line.as_bytes()).expect("a bridge row");
        let parties = super::sequence(
            held.get_by_name("parties")
                .unwrap_or_else(|| panic!("the group with packed separator {separator:?}")),
        );
        assert_eq!(parties.len(), 1);
        let first = parties[0].as_sequence().expect("one occurrence");
        assert_eq!(first.len(), 4, "the packed members did not split");
        assert_eq!(first[0].as_str(), Some("BUYSIDE"));
        messages.push(held);
    }
    assert_eq!(messages[0], messages[1]);
    assert_eq!(messages[0], messages[2]);
}

/// Separator-less inference is local to the addressed group.
///
/// A globally known `Symbol` inside an unknown member's value is not a
/// boundary. The later direct `PartyRole` member is, and the unknown residue
/// remains an ordinary entry with its embedded `=` intact.
#[test]
fn separatorless_group_inference_uses_only_direct_members() {
    let reader = reader();
    let message = reader
        .sole_line(
            b"MSGTYPE=D|NOPARTYIDS=1|\
             NOPARTYIDS[0]=VENUEFLAG=XSymbol=TTFPARTYROLE=1",
        )
        .expect("a bridge row");

    // The group heads the occurrence the packed pair was read into, and the
    // residue is one member of it under its own name.
    let counter = message
        .entries()
        .iter()
        .find(|entry| entry.tag() == 453)
        .expect("the group entry");
    let occurrence = counter
        .entries()
        .iter()
        .find(|entry| entry.name().as_bytes() == b"party")
        .expect("the occurrence the bridge wrote");
    assert_eq!(occurrence.tag(), 0);
    assert_eq!(occurrence.value(), None);
    let residue = occurrence
        .entries()
        .iter()
        .find(|entry| entry.name().as_bytes() == b"venueflag")
        .expect("the residue the group kept");
    assert_eq!(residue.value(), Some("XSymbol=TTF"));
    // The residue keeps its own value, and nothing invented a `Symbol` out of
    // the bytes inside it.
    assert_eq!(
        message
            .by_path(&path("parties[0].venueflag"))
            .expect("the residue the group kept")
            .as_str(),
        Some("XSymbol=TTF")
    );
    assert!(
        message
            .as_field()
            .get_field_by_path("parties.symbol")
            .is_none()
    );
    let parties = super::sequence(message.get_by_name("parties").expect("the group"));
    let members = parties[0].as_sequence().expect("one occurrence");
    assert_eq!(members.len(), 2);

    // A selected message's direct members outrank the wider global Parties
    // definition, even when the key addresses its numeric counter.
    let mut scoped = registry().as_ref().clone();
    let item = DataType::from_fields([scoped.field_by_tag(448).unwrap().clone()])
        .unwrap()
        .required_field("minimalparty");
    let mut group = DataType::list(item).nullable_field("minimalparties");
    group.as_fix_mut().set_counter(453).unwrap();
    scoped.insert(group.clone()).unwrap();
    let mut definition = DataType::from_fields([scoped.field_by_tag(453).unwrap().clone(), group])
        .unwrap()
        .required_field("minimalpartiesmessage");
    definition.as_fix_mut().set_msgtype("ZMIN").unwrap();
    scoped.insert(definition).unwrap();
    let numeric_name = super::fixed_codec(Arc::new(scoped))
        .sole_line(b"MSGTYPE=ZMIN|#453=1|#453[0]=PARTYID=BUYSIDEPARTYROLE=1")
        .expect("a bridge row");
    // One member, unsplit, because the group declares no `PARTYROLE` to split
    // at - which the row says, while the arrival record says what the bridge
    // wrote and nothing about how it was read.
    assert_eq!(
        numeric_name
            .by_path(&path("minimalparties[0].partyid"))
            .expect("the one unsplit member")
            .as_str(),
        Some("BUYSIDEPARTYROLE=1")
    );
    assert!(
        numeric_name
            .as_field()
            .get_field_by_path("minimalparties.partyrole")
            .is_none()
    );
    let group = numeric_name
        .entries()
        .iter()
        .find(|entry| entry.tag() == 453)
        .expect("the group the counter opens");
    let party = &group.entries()[0];
    assert_eq!(party.name(), "minimalparty");
    assert_eq!(party.value(), None);
    assert_eq!(party.entries()[0].name(), "partyid");
    assert_eq!(party.entries()[0].value(), Some("BUYSIDEPARTYROLE=1"));
}

/// A row's content can never fail the batch it arrives in.
///
/// A message states the members that occurrence carried; the fixed schema
/// declares the dictionary's. Projecting places them by name and leaves the
/// rest null, so an occurrence that stated one member is a row rather than a
/// refusal.
#[test]
fn a_group_shorter_than_the_schema_declares_still_projects() {
    use yggdryl::fix_schema;

    let registry = registry();
    let reader = reader();
    let schema = fix_schema(&registry, "fix").expect("the fixed schema");
    let held = reader
        .sole_line(b"toBridge #NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=BUYSIDE")
        .expect("a bridge row");

    let row = held.into_row(&schema).unwrap();
    // The completion is what a batch does with the row; it must not refuse.
    schema
        .canonicalize_value(row)
        .expect("a short occurrence is nulls, never a refusal");
}

/// The three FIX datatypes that land on `datetime64(ns,"UTC")`, read from the
/// committed dictionary and decoded to the instant each states.
///
/// `TZTimeOnly` states no date, so it lands on the epoch day with its offset
/// resolved in; `TZTimestamp` states its own zone, which is kept rather than
/// written over; `UTCTimestamp` states none and is UTC by saying nothing.
#[test]
fn every_fix_datatype_that_is_an_instant_decodes_to_one() {
    use yggdryl::{TimeUnit, Timezone};

    let reader = reader();
    let instant = |count: i64| {
        Scalar::datetime64(count, TimeUnit::Nanosecond, Timezone::UTC).expect("a nanosecond count")
    };
    // FIXT.1.1 with `ApplVerID(1128)=9`, because MaturityTime and
    // TZTransactTime are 5.0 fields and a 4.4 frame does not know them.
    let read = |frame: &str, value: &str, tag: i32| {
        reader
            .sole_line(format!("{frame}|35=D|{tag}={value}|10=0|").as_bytes())
            .expect("a readable message")
            .by_tag(tag)
            .expect("the tag")
            .clone()
    };
    let latest = |value: &str, tag: i32| read("8=FIXT.1.1|1128=9", value, tag);

    // MaturityTime(1079) is a TZTimeOnly. 07:39:12 is 27552s into the day and
    // the offset takes 19800 of them back, so the instant is 7752.123s past
    // the epoch - and a reading before the offset is applied goes behind it.
    assert_eq!(
        latest("07:39:12.123+05:30", 1079),
        instant(7_752_123_000_000)
    );
    assert_eq!(latest("07:39Z", 1079), instant(27_540_000_000_000));
    assert_eq!(latest("00:30+05:30", 1079), instant(-18_000_000_000_000));
    // FIX means local time by stating no offset, which an instant cannot
    // hold, so that reads as nothing rather than as a guessed UTC. It is the
    // same declared datatype rule that rejects a dateless `UTCTimestamp`.
    // The latter is a critical event clock, so its refusal fails intake.
    assert_eq!(latest("07:39:12", 1079), Scalar::Null);
    let refused = reader
        .parse_fix_line(b"8=FIX.4.4|35=D|60=10:15:30.000|10=0|")
        .unwrap_err();
    assert!(refused.to_string().contains("transacttime"));

    // TZTransactTime(1132) is a TZTimestamp: it states the zone, so writing
    // `Z` over it would spell one twice and read as nothing.
    assert_eq!(
        latest("20240102-10:15:30.000-05:00", 1132),
        instant(1_704_208_530_000_000_000),
    );

    // TransactTime(60) is a UTCTimestamp: no zone stated, and UTC meant.
    assert_eq!(
        read("8=FIX.4.4", "20240102-10:15:30.000", 60),
        instant(1_704_190_530_000_000_000),
    );

    // FIX spells a fraction with the full stop and nothing else, but the wire
    // spelling is handed to this crate's shared ISO reader rather than to a
    // second parser of FIX's own, and that reader takes either decimal sign.
    // So a bridge writing the comma its locale writes reads the same instant
    // where it used to be Null, without FIX itself gaining a spelling.
    assert_eq!(
        read("8=FIX.4.4", "20240102-10:15:30,000", 60),
        instant(1_704_190_530_000_000_000),
    );

    // The same reader takes a date that states no clock, so the compact wire
    // date is read whole rather than rewritten into a clock it does not have.
    // SettlDate(64) is a `LocalMktDate` and states no zone either, so it is
    // that day's midnight stating none, while TransactTime is a UTC column
    // and is owed the `Z` its name states - the one thing FIX leaves out.
    let local = |count: i64| {
        Scalar::datetime64(count, TimeUnit::Nanosecond, Timezone::NAIVE)
            .expect("a nanosecond count")
    };
    assert_eq!(
        read("8=FIX.4.4", "20240102", 64),
        local(1_704_153_600_000_000_000),
    );
    assert_eq!(
        read("8=FIX.4.4", "20240102", 60),
        instant(1_704_153_600_000_000_000),
    );
    // A day the calendar does not have is null, not a neighbouring one.
    assert_eq!(read("8=FIX.4.4", "20240230", 64), Scalar::Null);
}

/// A date column reads the compact wire date through the shared reader.
///
/// No committed FIX field is declared a date - every FIX date lands on
/// `datetime64` - so this is the hand-declared column, and it reads the FIX
/// spelling because the reader reads it, not because the codec rewrites the
/// text first. Eight bytes that are not eight digits are null, the lossily
/// decoded ones included, where a rewriter would have to slice them.
#[test]
fn a_date_column_reads_the_compact_wire_date_and_nulls_what_is_not_one() {
    let mut narrow = FixRegistry::new();
    let mut settled = DataType::Date32.nullable_field("settldate");
    settled.as_fix_mut().set_tag(64).unwrap();
    narrow.insert(settled).unwrap();
    let reader = super::fixed_codec(Arc::new(narrow));
    let read = |line: &[u8]| {
        reader
            .parse_fix_line(line)
            .expect("a readable message")
            .by_tag(64)
            .expect("the tag")
            .clone()
    };

    assert_eq!(
        read(b"8=FIX.4.4|35=D|64=20240102|10=0|"),
        Scalar::date32(19_724)
    );
    assert_eq!(
        read(b"8=FIX.4.4|35=D|64=2024-01-02|10=0|"),
        Scalar::date32(19_724)
    );
    assert_eq!(read(b"8=FIX.4.4|35=D|64=ab\xFFxyz|10=0|"), Scalar::Null);
    assert_eq!(read(b"8=FIX.4.4|35=D|64=20240230|10=0|"), Scalar::Null);
}

/// Critical clocks are typed at intake; no text-typed interior fallback remains.
#[test]
fn clock_intake_keeps_the_declared_datatypes_contract_and_refuses_wrong_layouts() {
    let reader = super::fixed_codec(Arc::new(FixRegistry::new()));
    // A native ns/UTC datetime accepts time with an offset on the epoch day.
    // The seed has no stricter FIX UTCTimestamp metadata, pinned above.
    let dateless = reader
        .parse_fix_line(b"8=FIX.4.4|35=D|60=07:39:12.123+05:30|10=0|")
        .unwrap();
    assert_eq!(dateless.get_unix(), 7_752_123_000_000);
    // A parse is not a snapshot, so the snapshot clock is a fact the
    // message does not state: a row that said it was taken at a moment
    // nothing took it at would be a fact nobody stated.
    assert!(dateless.get_by_tag(yggdryl::SNAPUNIX_TAG_NAME.0).is_none());
    let dated = reader
        .parse_fix_line(b"8=FIX.4.4|35=D|60=20240102-10:15:30.000|10=0|")
        .unwrap();
    assert_eq!(dated.get_unix(), 1_704_190_530_000_000_000);

    let mut narrow = FixRegistry::new();
    let mut clock = DataType::utf8().nullable_field("transacttime");
    clock.as_fix_mut().set_tag(60).unwrap();
    narrow.insert(clock).unwrap();
    let reader = super::fixed_codec(Arc::new(narrow));
    for value in ["07:39:12.123+05:30", "20240102-10:15:30.000"] {
        let error = reader
            .parse_fix_line(format!("8=FIX.4.4|35=D|60={value}|10=0|").as_bytes())
            .unwrap_err();
        assert!(error.to_string().contains("transacttime"));
    }
}

/// Each reader on its own, over the one row shape it owns.
///
/// [`FixCodec::parse_line`] is the door and picks between them; these are the
/// three it picks, addressed directly, so a caller who already knows what a
/// row is pays for no classification and a reader's own contract is pinned
/// where the dispatcher cannot mask it.
#[test]
fn every_reader_answers_for_the_one_row_shape_it_owns() {
    let codec = codec();

    // A numeric frame, split on the byte it actually uses. No separator is
    // pinned, so the frame's own is inferred.
    let numeric = codec
        .parse_fix_line(b"8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|10=0|")
        .expect("a numeric frame");
    assert_eq!(numeric.as_field().name(), "D");
    assert_eq!(numeric.by_tag(11).unwrap().as_str(), Some("ORDER-1"));
    assert_eq!(numeric.by_name("symbol").unwrap().as_str(), Some("AAPL"));

    // A bridge row keys by name, and `#` marks a group rather than a field.
    let bridge = codec
        .parse_ullink_line(b"MSGTYPE=D|CLORDID=ORDER-1|SYMBOL=AAPL|SIDE=1")
        .expect("a bridge row");
    assert_eq!(bridge.as_field().name(), "D");
    assert_eq!(bridge.by_name("clordid").unwrap().as_str(), Some("ORDER-1"));

    // FIXML spells a field as an attribute and a component as a nested
    // element, so both levels contribute and neither element name becomes a
    // tag of its own.
    let fixml = codec
        .parse_fixml_line(br#"<Order ClOrdID="XML-1" Side="1"><Instrmt Sym="AAPL"/></Order>"#)
        .expect("a FIXML row");
    assert_eq!(fixml.by_name("clordid").unwrap().as_str(), Some("XML-1"));
    assert_eq!(fixml.by_name("sym").unwrap().as_str(), Some("AAPL"));

    // A row that is not well-formed XML is a refusal naming its position,
    // where a row that is merely unfamiliar is read and kept.
    let refused = codec.parse_fixml_line(b"<Order ClOrdID=").unwrap_err();
    assert!(refused.to_string().contains("fixml"), "{refused}");
}

/// The door picks the reader, and picks the same one every time.
#[test]
fn read_line_picks_the_reader_the_row_shape_names() {
    // The FIXML element states no type, so this fixture asks for it.
    let codec = reader();
    let named = |row: &[u8]| {
        codec
            .sole_line(row)
            .expect("a readable row")
            .as_field()
            .name()
            .to_owned()
    };

    // The prefix a process printed is located and dropped before the choice.
    assert_eq!(named(b"sending >> 8=FIX.4.4|35=D|11=A|10=0|"), "D");
    assert_eq!(named(b"MSGTYPE=D|CLORDID=A"), "D");

    // An opening `<` is FIXML, and the attribute reaches the field where the
    // bridge splitter would have made one key of the whole element.
    let xml = codec
        .sole_line(br#"<Order ClOrdID="XML-1"/>"#)
        .expect("a FIXML row");
    assert_eq!(xml.by_name("clordid").unwrap().as_str(), Some("XML-1"));
}

/// The batch readers, each over the one shape it takes.
///
/// A capture arrives as lines - a text reader answers one per line, with the
/// body beside the `url` and `rownum` it came from - so the codec takes that
/// shape at two widths: one line at a time, and a stream of Arrow batches.
/// Both are the same read, which is what these pin: the message a stream
/// answers is the message a line answers.
#[test]
fn every_batch_reader_answers_what_the_single_reader_answers() {
    let codec = codec();
    let rows = [
        b"8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|10=0|".to_vec(),
        b"8=FIX.4.4|35=8|37=O-9|55=MSFT|10=0|".to_vec(),
    ];
    let lines: Vec<TextLine> = rows
        .iter()
        .map(|row| {
            TextLine::from_bytes(0, TextBytes::from_bytes(row).expect("a capture page")).unwrap()
        })
        .collect();

    // One line at a time, lazily: the iterator is the stream.
    let read: Vec<_> = codec
        .parse_text_lines(lines)
        .collect::<Result<Vec<_>, _>>()
        .expect("readable lines");
    assert_eq!(read.len(), 2);
    assert_eq!(read[0].as_field().name(), "D");
    assert_eq!(read[1].by_name("symbol").unwrap().as_str(), Some("MSFT"));

    // The same rows as one Arrow batch in and one Arrow batch out, with the
    // row count preserved: a capture joins back to its source by position.
    let capture = DataType::from_fields([DataType::binary().required_field("body")])
        .expect("a capture shape")
        .required_field("capture");
    let values = Scalar::from_sequence(
        rows.iter()
            .map(|row| Scalar::from_sequence([Scalar::from(row.clone())]))
            .collect::<Vec<_>>(),
    );
    let batch = yggdryl::arrow::batch_from_value(&capture, &values).expect("an Arrow batch");
    let source = yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]);
    let streamed: Vec<_> = codec
        .parse_text_arrow_reader(source)
        .expect("a readable stream")
        .collect::<Result<Vec<_>, _>>()
        .expect("readable batches");
    assert_eq!(streamed.iter().map(RecordBatch::num_rows).sum::<usize>(), 2);
    // And the same rows back as messages, without a parse: the batch holds
    // what the record read said, and the capture's own columns stay the
    // capture's rather than becoming content of the message.
    let again: Vec<_> = codec
        .messages(yggdryl::arrow::batch_reader(
            streamed[0].schema(),
            streamed.clone(),
        ))
        .collect::<Result<Vec<_>, _>>()
        .expect("readable rows");
    assert_eq!(again.len(), 2);
    assert_eq!(again[0].by_tag(11).unwrap(), read[0].by_tag(11).unwrap());
    assert_eq!(again[1].entries(), read[1].entries());
    assert_eq!(again[1].into_bytes(b'|'), read[1].into_bytes(b'|'));
}

#[test]
fn a_fixml_document_in_a_data_field_fills_the_line_that_carried_it() {
    let reader = reader();
    // A bridge relaying an execution report writes the whole document into
    // `XmlData(213)`, which is the field FIX names for exactly that.
    let document = br#"<FIXML v="5.0 SP2"><ExecRpt ExecID="E1" ClOrdID="ORDER-1" LastQty="21" LastPx="83.08"><Instrmt Symbol="HOLN" /></ExecRpt></FIXML>"#;
    let mut frame = Vec::new();
    frame.extend_from_slice(b"8=FIX.4.2|9=0|35=n|212=");
    frame.extend_from_slice(document.len().to_string().as_bytes());
    frame.push(b'|');
    frame.extend_from_slice(b"213=");
    frame.extend_from_slice(document);
    frame.extend_from_slice(b"|10=0|");
    let message = reader.sole_line(&frame).unwrap();

    // The frame's own statements stay the frame's, and the document inside
    // fills what the frame never said - resolved to real tags, typed by the
    // dictionary rather than kept as one opaque value.
    assert_eq!(message.by_tag(35).unwrap().as_str(), Some("n"));
    assert_eq!(message.by_tag(17).unwrap().as_str(), Some("E1"));
    assert_eq!(message.by_tag(11).unwrap().as_str(), Some("ORDER-1"));
    assert_eq!(message.by_tag(32).unwrap().as_f64(), Some(21.0));
    assert_eq!(message.by_tag(31).unwrap().as_f64(), Some(83.08));
    // A nested element's attributes are the same pairs, flattened - FIXML
    // spells a component as an element and a field as an attribute.
    assert_eq!(message.by_tag(55).unwrap().as_str(), Some("HOLN"));

    // `XmlData` itself is still the bytes it arrived as, and the wire
    // re-emits the message as the crate holds it: the frame's pairs, the
    // document among them, and the fields the document filled beside them.
    assert_eq!(message.by_tag(213).unwrap().as_bytes(), Some(&document[..]));
    // The trade the document reported is the event's own fact, and a
    // message that reports a trade and states no price or quantity of its
    // own settles on the trade's, so the event band carries four numbers
    // where the document spelled two.
    let mut emitted = Vec::new();
    emitted.extend_from_slice(b"8=FIX.4.2|35=n|31=83.08|32=21|38=21|44=83.08|9=0|212=");
    emitted.extend_from_slice(document.len().to_string().as_bytes());
    emitted.push(b'|');
    emitted.extend_from_slice(b"213=");
    emitted.extend_from_slice(document);
    emitted.extend_from_slice(b"|v=5.0 SP2|17=E1|11=ORDER-1|55=HOLN|10=0|");
    assert_eq!(
        String::from_utf8(message.into_bytes(b'|')).unwrap(),
        String::from_utf8(emitted).unwrap()
    );
}

#[test]
fn the_default_refusals_are_the_session_traffic_and_the_typeless_row() {
    // Three types a codec refuses until a caller says otherwise: the two the
    // session layer fills a capture with, and the row that states no type.
    assert_eq!(yggdryl::DEFAULT_REFUSED_MSGTYPES, ["0", "1", "unknown"]);
    let codec = codec();
    assert_eq!(codec.exclude_msgtypes(), yggdryl::DEFAULT_REFUSED_MSGTYPES);
    assert!(codec.include_msgtypes().is_empty());
    for refused in ["0", "1", "unknown", "Heartbeat", "TestRequest"] {
        assert!(!codec.reads_msgtype(refused), "{refused}");
    }
    for read in ["D", "8", "AE", "UL"] {
        assert!(codec.reads_msgtype(read), "{read}");
    }

    // The filter runs on the type a row states, before a message is built:
    // a heartbeat and a row that states no type are no row at all.
    for body in [
        &b"8=FIX.4.4|35=0|49=S|56=T|10=0|"[..],
        b"8=FIX.4.4|35=1|112=T1|10=0|",
        b"8=FIX.4.4|49=S|56=T|10=0|",
    ] {
        assert!(
            codec
                .parse_line(body)
                .expect("a readable line")
                .next()
                .is_none(),
            "{}",
            String::from_utf8_lossy(body)
        );
        assert!(
            reader()
                .parse_line(body)
                .expect("a readable line")
                .next()
                .is_some()
        );
    }

    // Naming what a caller wants says what it does not: an include list
    // clears the refusals, and an exclude list replaces them outright.
    let only_orders = codec.clone().with_include_msgtypes(["D"]);
    assert!(only_orders.exclude_msgtypes().is_empty());
    assert!(only_orders.reads_msgtype("D"));
    assert!(!only_orders.reads_msgtype("8"));
    assert!(
        codec
            .clone()
            .with_exclude_msgtypes(["8"])
            .reads_msgtype("0")
    );
}
