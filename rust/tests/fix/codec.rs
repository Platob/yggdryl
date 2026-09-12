//! The codec's readers, over the committed dictionary and a real capture.

use super::OneMessage;
use super::path;

use std::sync::Arc;

use arrow_array::RecordBatch;
use yggdryl::media::text::{TextBytes, TextLine};
use yggdryl::types::State;
use yggdryl::{DataType, FixBranch, FixCategory, FixCodec, FixEntry, FixRegistry, Scalar, Version};

fn registry() -> Arc<FixRegistry> {
    super::committed_registry()
}

fn codec() -> FixCodec {
    FixCodec::new(registry())
}

fn reader() -> FixCodec {
    codec()
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
            .one_line(row.as_bytes(), false)
            .unwrap_or_else(|error| panic!("{row}: {error}"));
        // A row with no type is named `unknown` rather than refused: every
        // pair that parsed became a field and the entries record the row.
        assert!(!message.as_field().name().is_empty(), "{row}");
    }

    // Empty input is the one typed error: a line that merely held nothing is
    // `Ok` and says so.
    assert!(reader.one_line(b"", false).is_err());
    let empty = reader
        .one_line(b"no level printed by this plugin", false)
        .unwrap();
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
        let message = reader.one_line(row.as_bytes(), false).expect(row);
        assert_eq!(message.as_field().name(), msgtype, "{row}");
    }

    // The prose in front of the frame is not a field, and nothing after the
    // checksum is part of the message.
    let framed = reader
        .one_line(
            b"sending >> 8=FIX.4.2|9=176|35=D|10=203| << queued seq=1092",
            false,
        )
        .unwrap();
    let keys: Vec<&str> = framed
        .entries()
        .iter()
        .map(|entry| entry.key().as_str().unwrap_or_default())
        .collect();
    assert_eq!(keys, ["8", "9", "35", "10"], "{keys:?}");
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
            let message = reader.one_line(line.as_bytes(), false).unwrap();
            assert_eq!(message.by_tag(35).unwrap().as_str(), Some(code));
            assert_eq!(message.by_tag(55).unwrap().as_str(), Some("AAPL"));
            assert_eq!(
                message
                    .as_field()
                    .fields()
                    .iter()
                    .find(|field| field.as_fix().tag().unwrap() == Some(35))
                    .unwrap()
                    .dtype(),
                &DataType::utf8(),
            );
            assert_eq!(message.into_bytes(separator), line.as_bytes());
        }
    }
}

#[test]
fn numeric_group_counters_and_nested_occurrences_keep_their_declared_shapes() {
    let reader = reader();
    let wire = b"8=FIX.4.4|35=D|453=2|448=A|447=D|452=1|802=2|523=DESK|803=1|523=CLIENT|803=2|448=B|447=D|452=3|802=1|523=OTHER|803=3|55=AAPL|10=0|";
    let message = reader.parse_fix_line(wire).unwrap();
    assert_eq!(message.by_tag(453).unwrap(), &Scalar::from(2_i32));
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
        &Scalar::from(2_i32)
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
    assert_eq!(message.into_bytes(b'|'), wire);
    assert!(message.anomalies().next().is_none());

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
fn numeric_group_member_anomalies_keep_omitted_and_repeated_occurrences_aligned() {
    let wire = b"35=D|453=5|448=OMITTED|448=INVALID|452=bogus|448=VALID|452=1|448=OMITTED2|448=OVERFLOW|452=2147483648|10=0|";
    let message = reader().parse_fix_line(wire).unwrap();
    assert!(
        message
            .by_path(&path("Parties[0].PartyRole"))
            .unwrap()
            .is_null()
    );
    assert!(
        message
            .by_path(&path("Parties[1].PartyRole"))
            .unwrap()
            .is_null()
    );
    assert_eq!(
        message.by_path(&path("Parties[2].PartyRole")).unwrap(),
        &Scalar::from(1_i32)
    );
    assert!(
        message
            .by_path(&path("Parties[3].PartyRole"))
            .unwrap()
            .is_null()
    );
    assert!(
        message
            .by_path(&path("Parties[4].PartyRole"))
            .unwrap()
            .is_null()
    );
    let anomalies: Vec<_> = message.anomalies().collect();
    assert_eq!(
        anomalies,
        [
            yggdryl::FixAnomaly::Untyped {
                tag: 452,
                key: "452",
                value: "bogus",
            },
            yggdryl::FixAnomaly::Untyped {
                tag: 452,
                key: "452",
                value: "2147483648",
            },
        ]
    );
    assert_eq!(message.into_bytes(b'|'), wire);
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
        assert_eq!(message.into_bytes(b'|'), wire.as_bytes());
        match count.parse::<i32>() {
            Ok(stated) => {
                assert_eq!(message.by_tag(453).unwrap(), &Scalar::from(stated));
                assert_eq!(
                    message.anomalies().any(|anomaly| matches!(
                        anomaly,
                        yggdryl::FixAnomaly::Miscounted { tag: 453, .. }
                    )),
                    i64::from(stated) != held as i64,
                    "{wire}",
                );
            }
            Err(_) => {
                assert!(message.by_tag(453).unwrap().is_null());
                assert!(
                    message.anomalies().any(|anomaly| matches!(
                        anomaly,
                        yggdryl::FixAnomaly::Untyped { tag: 453, .. }
                    )),
                    "{wire}"
                );
            }
        }
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
    assert_eq!(message.into_bytes(b'|'), wire);
}

#[test]
fn nested_counter_anomalies_follow_each_counter_across_reordered_siblings() {
    let mut scoped = registry().as_ref().clone();
    let mut left = DataType::list(
        DataType::from_fields([scoped.field_by_tag(523).unwrap().clone()])
            .unwrap()
            .required_field("leftparty"),
    )
    .nullable_field("leftparties");
    left.as_fix_mut().set_counter(802).unwrap();
    let mut right = DataType::list(
        DataType::from_fields([scoped.field_by_tag(524).unwrap().clone()])
            .unwrap()
            .required_field("rightparty"),
    )
    .nullable_field("rightparties");
    right.as_fix_mut().set_counter(539).unwrap();
    scoped
        .insert_definition(FixCategory::Groups, left.clone())
        .unwrap();
    scoped
        .insert_definition(FixCategory::Groups, right.clone())
        .unwrap();
    let mut parties = DataType::list(
        DataType::from_fields([
            scoped.field_by_tag(448).unwrap().clone(),
            scoped.field_by_tag(802).unwrap().clone(),
            left,
            scoped.field_by_tag(539).unwrap().clone(),
            right,
        ])
        .unwrap()
        .required_field("scopedparty"),
    )
    .nullable_field("scopedparties");
    parties.as_fix_mut().set_counter(453).unwrap();
    scoped
        .insert_definition(FixCategory::Groups, parties.clone())
        .unwrap();
    let mut message_type =
        DataType::from_fields([scoped.field_by_tag(453).unwrap().clone(), parties])
            .unwrap()
            .required_field("scopedcountermessage");
    message_type.as_fix_mut().set_msgtype("ZCNT").unwrap();
    scoped
        .insert_definition(FixCategory::Messages, message_type)
        .unwrap();
    let reader = FixCodec::new(Arc::new(scoped));
    let wire = b"35=ZCNT|453=2|448=A|539=2|524=RIGHT|802=invalid|523=LEFT|448=B|802=1|523=BLEFT|539=0|524=BRIGHT|55=AAPL|10=0|";
    let message = reader.parse_fix_line(wire).unwrap();
    let anomalies: Vec<_> = message.anomalies().collect();
    assert!(
        matches!(
            anomalies.as_slice(),
            [
                yggdryl::FixAnomaly::Miscounted {
                    tag: 539,
                    stated: 2,
                    held: 1,
                    ..
                },
                yggdryl::FixAnomaly::Untyped { tag: 802, .. },
                yggdryl::FixAnomaly::Miscounted {
                    tag: 539,
                    stated: 0,
                    held: 1,
                    ..
                },
            ]
        ),
        "{anomalies:?}"
    );
    assert_eq!(message.into_bytes(b'|'), wire);
}

#[test]
fn a_tag_key_and_a_name_key_build_the_same_message() {
    let reader = reader();
    let by_tag = reader
        .one_line(b"8=FIX.4.4|35=D|55=AAPL|54=1|10=0|", false)
        .unwrap();
    let by_name = reader
        .one_line(b"8=FIX.4.4|MsgType=D|Symbol=AAPL|Side=1|10=0|", false)
        .unwrap();
    assert_eq!(by_tag.as_value(), by_name.as_value());
    assert_eq!(by_tag.as_field().dtype(), by_name.as_field().dtype());

    // Case and separators fold away, so a renderer's spelling still resolves.
    for spelling in ["MsgType", "msgtype", "MSG_TYPE", "msg-type", "Msg Type"] {
        let row = format!("8=FIX.4.4|{spelling}=D|10=0|");
        let message = reader.one_line(row.as_bytes(), false).expect(&row);
        assert_eq!(message.as_field().name(), "D", "{spelling}");
    }
}

#[test]
fn a_value_is_translated_typed_and_kept_as_it_arrived() {
    let reader = reader();
    let message = reader
        .one_line(b"8=FIX.4.4|35=D|54=Buy|38=100|10=0|", false)
        .unwrap();

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
    assert_eq!(side.value().as_str(), Some("Buy"));
    assert_eq!(side.key().as_str(), Some("54"));
    // The identity is the entry's tag under the message's own dialect: one
    // branch for the message, never one copy per pair.
    assert_eq!(
        yggdryl::FixId::from_parts(message.branch(), side.tag()).unwrap(),
        yggdryl::FixId::standard(54)
    );
}

#[test]
fn an_unknown_key_is_kept_and_a_bad_value_is_null_rather_than_a_failure() {
    let reader = reader();
    let message = reader
        .one_line(b"8=FIX.4.4|35=D|VenueOwnThing=x|9999=y|9=abc|10=0|", false)
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
        .find(|entry| entry.key().as_bytes() == b"VenueOwnThing")
        .expect("the venue's own key");
    assert_eq!(held.tag(), 0, "an unknown key names no tag");

    // A `BodyLength` of `abc` nulls that field while the raw text stays.
    assert_eq!(message.by_name("bodylength").unwrap(), &Scalar::Null);
    assert!(
        message
            .entries()
            .iter()
            .any(|entry| entry.tag() == 9 && entry.value().as_bytes() == b"abc")
    );
}

#[test]
fn a_stated_absence_produces_no_field_and_no_entry() {
    let reader = reader();
    for spelling in ["", "null", "NULL", "<null>"] {
        let row = format!("8=FIX.4.4|35=D|58={spelling}|10=0|");
        let message = reader.one_line(row.as_bytes(), false).expect(&row);
        assert!(message.get_by_tag(58).is_none(), "{spelling}");
        assert!(
            !message.entries().iter().any(|entry| entry.tag() == 58),
            "{spelling}"
        );
    }
    // A value that merely contains `null`, and one a venue means literally,
    // both survive - the listing is a convention and has to be overridable.
    let kept = reader
        .one_line(b"8=FIX.4.4|35=D|58=nullable|10=0|", false)
        .unwrap();
    assert_eq!(kept.by_tag(58).unwrap().as_str(), Some("nullable"));

    let literal = reader
        .clone()
        .with_null_values::<[&str; 0], &str>([])
        .one_line(b"8=FIX.4.4|35=D|58=null|10=0|", false)
        .unwrap();
    assert_eq!(literal.by_tag(58).unwrap().as_str(), Some("null"));
}

#[test]
fn a_bridge_group_becomes_real_nesting_from_its_indexed_keys() {
    let reader = reader();
    let row = "MSGTYPE=D|#NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=SYNTH-01\x04\x03PARTYIDSOURCE=D\x04\x03PARTYROLE=1";
    let message = reader.one_line(row.as_bytes(), false).unwrap();

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
    let message = reader.one_line(line, false).unwrap();
    assert_eq!(message, reader.parse_ullink_line(line).unwrap());
    let without_member_separators: &[u8] =
        b"|#SYMBOL=TTF|#SIDE=1|#ORDERQTY=1200|#PRICE=41.2500|#NOPARTYIDS=2\
|#NOPARTYIDS[0]=PARTYID=BUYSIDEPARTYIDSOURCE=DPARTYROLE=1|";
    assert_eq!(
        message,
        reader.one_line(without_member_separators, false).unwrap()
    );

    // Names resolve to tags, and each value takes its field's own type: a
    // quantity and a price are numbers, and a side is the packed code.
    assert_eq!(message.by_tag(55).unwrap().as_str(), Some("TTF"));
    assert_eq!(message.by_tag(54).unwrap().as_str(), Some("1"));
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
    // The occurrence arrived under the counter pair that heads it, so the
    // arrival record nests exactly as the wire stated: the counter entry
    // carries it and the top level does not repeat it. What it carries is the
    // pair the bridge wrote - the members above are a reading of that pair,
    // and a key like `NOPARTYIDS[0].PARTYID` appears nowhere in the line.
    let counter = message
        .entries()
        .iter()
        .find(|entry| entry.tag() == 453)
        .expect("the counter pair");
    let keys: Vec<&str> = counter
        .children()
        .iter()
        .map(|entry| entry.key().as_str().unwrap_or_default())
        .collect();
    assert_eq!(keys, ["NOPARTYIDS[0]"]);
    assert_eq!(
        counter.children()[0].value().as_str(),
        Some("PARTYID=BUYSIDE\u{4}\u{3}PARTYIDSOURCE=D\u{4}\u{3}PARTYROLE=1"),
    );
    assert_eq!(
        counter.children()[0].tag(),
        0,
        "an occurrence names no field"
    );
    assert!(
        message
            .entries()
            .iter()
            .all(|entry| !entry.key().as_bytes().starts_with(b"NOPARTYIDS[0]")),
        "an occurrence rides under its counter, not beside it",
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
fn a_hash_key_keeps_its_hash_only_beside_its_bare_twin() {
    let reader = reader();
    // `ORDERID=123|#ORDERID=345` states two keys: dropping the `#` would
    // merge two values under one name. The bare spelling resolves to the
    // dictionary's `OrderID`, and the `#` one stays its own column - on
    // whichever side of its twin it arrived.
    for row in [
        b"MSGTYPE=D|ORDERID=123|#ORDERID=345".as_slice(),
        b"MSGTYPE=D|#ORDERID=345|ORDERID=123".as_slice(),
    ] {
        let message = reader.one_line(row, false).unwrap();
        let spelled = String::from_utf8_lossy(row);
        assert_eq!(
            message.by_tag(37).unwrap().as_str(),
            Some("123"),
            "{spelled}"
        );
        assert_eq!(
            message.by_name("#orderid").unwrap().as_str(),
            Some("345"),
            "{spelled}"
        );
        // The entries are the wire record, so both arrival spellings stay.
        let keys: Vec<&str> = message
            .entries()
            .iter()
            .map(|entry| entry.key().as_str().unwrap_or_default())
            .collect();
        assert!(keys.contains(&"ORDERID"), "{spelled}: {keys:?}");
        assert!(keys.contains(&"#ORDERID"), "{spelled}: {keys:?}");

        // The wire rebuilds from the entries, `#` included, and re-reading
        // the emitted line answers the same message: the twin judgment is
        // idempotent.
        let emitted = message.into_bytes(b'|');
        let reread = reader.one_line(&emitted, false).expect(&spelled);
        assert_eq!(reread, message, "{spelled}");
    }

    // Alone, the `#` is the bridge's own marker and drops: the key is the
    // dictionary field, exactly as a frame of only `#` keys always read.
    let single = reader.one_line(b"MSGTYPE=D|#ORDERID=345", false).unwrap();
    assert_eq!(single.by_tag(37).unwrap().as_str(), Some("345"));
    assert!(single.by_name("#orderid").is_err());

    // A marked pair restating the bare one's bytes is a second spelling of
    // one pair, and one pair is what remains: row and entries alike, on
    // whichever side it arrived, however the bare key was spelled, and with
    // whatever space the row put around the values.
    for (row, bare) in [
        (b"MSGTYPE=D|ORDERID=123|#ORDERID=123".as_slice(), "ORDERID"),
        (b"MSGTYPE=D|#ORDERID=123|ORDERID=123".as_slice(), "ORDERID"),
        (b"MSGTYPE=D|OrderId=123|#ORDERID=123".as_slice(), "OrderId"),
        (
            b"MSGTYPE=D|ORDERID=123 |#ORDERID= 123".as_slice(),
            "ORDERID",
        ),
    ] {
        let message = reader.one_line(row, false).unwrap();
        let spelled = String::from_utf8_lossy(row);
        assert_eq!(
            message.by_tag(37).unwrap().as_str(),
            Some("123"),
            "{spelled}"
        );
        assert!(message.by_name("#orderid").is_err(), "{spelled}");
        let keys: Vec<&str> = message
            .entries()
            .iter()
            .map(|entry| entry.key().as_str().unwrap_or_default())
            .collect();
        assert_eq!(keys, ["MSGTYPE", bare], "{spelled}");
        assert_eq!(
            message.into_bytes(b'|'),
            format!("MSGTYPE=D|{bare}=123|").as_bytes(),
            "{spelled}"
        );
    }
    // A value is a value: two spellings of the bytes are two values.
    let cased = reader
        .one_line(b"MSGTYPE=D|ORDERID=abc|#ORDERID=ABC", false)
        .unwrap();
    assert_eq!(cased.by_tag(37).unwrap().as_str(), Some("abc"));
    assert_eq!(cased.by_name("#orderid").unwrap().as_str(), Some("ABC"));

    // A bridge marks the name keys it writes into a numeric frame exactly as
    // it marks a row's, and they are judged by the same rules: alone the
    // mark drops, beside a twin of other bytes it stays, restating a twin it
    // goes.
    let framed = reader
        .one_line(
            b"sending >> 8=FIX.4.2|35=UL|#SYMBOL=TTF|#SIDE=1|10=044|",
            false,
        )
        .unwrap();
    assert_eq!(framed.by_tag(55).unwrap().as_str(), Some("TTF"));
    assert_eq!(framed.by_tag(54).unwrap().as_str(), Some("1"));
    let keys: Vec<&str> = framed
        .entries()
        .iter()
        .map(|entry| entry.key().as_str().unwrap_or_default())
        .collect();
    assert_eq!(keys, ["8", "35", "SYMBOL", "SIDE", "10"]);
    let twinned = reader
        .one_line(b"8=FIX.4.2|35=UL|ORDERID=123|#ORDERID=345|10=0|", false)
        .unwrap();
    assert_eq!(twinned.by_tag(37).unwrap().as_str(), Some("123"));
    assert_eq!(twinned.by_name("#orderid").unwrap().as_str(), Some("345"));
    let restated = reader
        .one_line(b"8=FIX.4.2|35=UL|ORDERID=123|#ORDERID=123|10=0|", false)
        .unwrap();
    assert_eq!(restated.by_tag(37).unwrap().as_str(), Some("123"));
    assert!(restated.by_name("#orderid").is_err());
    assert_eq!(
        restated.into_bytes(b'|'),
        b"8=FIX.4.2|35=UL|ORDERID=123|10=0|"
    );

    // The row's type is read after the marks are judged, so a type the
    // bridge marked names the message - and the message is what splits a
    // separator-less occurrence of a counter half the dictionary shares at
    // the members its own group declares.
    let typed = reader
        .one_line(b"#MSGTYPE=s|#NOSIDES=1|#NOSIDES[0]=SIDE=1CLORDID=X", false)
        .unwrap();
    assert_eq!(typed.by_tag(35).unwrap().as_str(), Some("s"));
    assert_eq!(
        typed
            .by_path(&path("SideCrossOrdModGrp[0].ClOrdID"))
            .unwrap(),
        &Scalar::from("X")
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
        let message = reader.one_line(row, false).unwrap();
        let spelled = String::from_utf8_lossy(row);
        assert_eq!(
            message.by_tag(37).unwrap().as_str(),
            Some("123"),
            "{spelled}"
        );
        assert_eq!(
            message.by_name("#orderid").unwrap().as_str(),
            Some("345"),
            "{spelled}"
        );
    }

    // A bare twin that stated an absence was never sent, so the `#` is the
    // row's sole spelling and drops: the value lands under the dictionary
    // field exactly as a lone `#` key always did.
    for spelling in ["", "null", "<null>"] {
        let row = format!("MSGTYPE=D|ORDERID={spelling}|#ORDERID=345");
        let message = reader.one_line(row.as_bytes(), false).expect(&row);
        assert_eq!(message.by_tag(37).unwrap().as_str(), Some("345"), "{row}");
        assert!(message.by_name("#orderid").is_err(), "{row}");
    }

    // A `#` occurrence kept beside its bare twin stays verbatim and whole:
    // one key, one value, nothing rewritten under a group name no registry
    // resolves.
    let row: &[u8] = b"MSGTYPE=D|NOPARTYIDS[0]=whole|#NOPARTYIDS[0]=PARTYID=A\x04\x03PARTYROLE=1";
    let message = reader.one_line(row, false).unwrap();
    let keys: Vec<&str> = message
        .entries()
        .iter()
        .map(|entry| entry.key().as_str().unwrap_or_default())
        .collect();
    assert!(keys.contains(&"NOPARTYIDS[0]"), "{keys:?}");
    assert!(keys.contains(&"#NOPARTYIDS[0]"), "{keys:?}");
    assert!(
        !keys.iter().any(|key| key.starts_with("#NOPARTYIDS[0].")),
        "{keys:?}"
    );

    // A group is one thing however many occurrences it states, so a bare
    // group claims every marked key of it: the count and the occurrences the
    // bare spelling never numbered stay whole beside it, each its own child
    // under its own name, and none of them lands in the dictionary's group.
    for row in [
        b"MSGTYPE=D|NOPARTYIDS=1|NOPARTYIDS[0]=PARTYID=BARE\x04\x03PARTYROLE=1\
|#NOPARTYIDS=2|#NOPARTYIDS[0]=PARTYID=A\x04\x03PARTYROLE=1|#NOPARTYIDS[1]=PARTYID=B\x04\x03PARTYROLE=3"
            .as_slice(),
        b"MSGTYPE=D|#NOPARTYIDS=2|#NOPARTYIDS[0]=PARTYID=A\x04\x03PARTYROLE=1|#NOPARTYIDS[1]=PARTYID=B\x04\x03PARTYROLE=3\
|NOPARTYIDS=1|NOPARTYIDS[0]=PARTYID=BARE\x04\x03PARTYROLE=1"
            .as_slice(),
    ] {
        let message = reader.one_line(row, false).unwrap();
        let spelled = String::from_utf8_lossy(row);
        assert_eq!(message.by_tag(453).unwrap().as_i64(), Some(1), "{spelled}");
        let parties = message.by_name("parties").unwrap().as_sequence().unwrap().to_vec();
        assert_eq!(parties.len(), 1, "{spelled}: {parties:?}");
        assert_eq!(
            message.by_path(&path("Parties[0].PartyID")).unwrap().as_str(),
            Some("BARE"),
            "{spelled}"
        );
        let child = |name: &str| {
            let at = message.as_field().index_of(name).expect(name);
            message.as_value().as_sequence().expect("a row")[at].clone()
        };
        assert_eq!(child("#nopartyids").as_str(), Some("2"), "{spelled}");
        // The row types the packed text with the bridge's control bytes
        // gone, as it types any value; the entry keeps the bytes as they
        // arrived.
        for (name, key, id, role) in [
            ("#nopartyids[0]", "#NOPARTYIDS[0]", "A", "1"),
            ("#nopartyids[1]", "#NOPARTYIDS[1]", "B", "3"),
        ] {
            let held = child(name);
            let text = held.as_str().expect(name);
            assert!(
                text.starts_with(&format!("PARTYID={id}"))
                    && text.ends_with(&format!("PARTYROLE={role}")),
                "{spelled}: {text}"
            );
            let entry = message
                .entries()
                .iter()
                .find(|entry| entry.key().as_bytes() == key.as_bytes())
                .unwrap_or_else(|| panic!("{spelled}: {key}"));
            assert_eq!(
                entry.value().as_str(),
                Some(format!("PARTYID={id}\x04\x03PARTYROLE={role}").as_str()),
                "{spelled}"
            );
            assert!(entry.children().is_empty(), "{spelled}");
        }
        let keys: Vec<&str> = message.entries().iter().map(|entry| entry.key().as_str().unwrap_or_default()).collect();
        assert!(keys.contains(&"#NOPARTYIDS"), "{spelled}: {keys:?}");
        assert!(
            !message
                .entries()
                .iter()
                .flat_map(|entry| entry.children().iter().chain(std::iter::once(entry)))
                .any(|entry| entry.key().as_bytes().starts_with(b"NOPARTYIDS[1]")
                    || entry.key().as_bytes().starts_with(b"#NOPARTYIDS[0].")),
            "{spelled}: {keys:?}"
        );
        // The row and the group agree, so there is no miscount to report.
        assert_eq!(message.anomalies().count(), 0, "{spelled}");
    }

    // A marked group goes only whole. A marked count restating the bare one
    // beside occurrences the bare group never numbered is no second spelling
    // of the bare count - it counts the marked occurrences - so it stays
    // verbatim with them; a marked group restating the bare group pair for
    // pair goes pair for pair.
    let counted: &[u8] = b"MSGTYPE=D|NOPARTYIDS=1|NOPARTYIDS[0]=PARTYID=B\x04\x03PARTYROLE=1\
|#NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=A\x04\x03PARTYROLE=1";
    let message = reader.one_line(counted, false).unwrap();
    assert_eq!(message.by_tag(453).unwrap().as_i64(), Some(1));
    assert_eq!(
        message.by_path(&path("Parties[0].PartyID")).unwrap(),
        &Scalar::from("B")
    );
    let at = message
        .as_field()
        .index_of("#nopartyids")
        .expect("the marked count");
    assert_eq!(
        message.as_value().as_sequence().expect("a row")[at].as_str(),
        Some("1")
    );
    let keys: Vec<&str> = message
        .entries()
        .iter()
        .map(|entry| entry.key().as_str().unwrap_or_default())
        .collect();
    assert!(
        keys.contains(&"#NOPARTYIDS") && keys.contains(&"#NOPARTYIDS[0]"),
        "{keys:?}"
    );
    assert_eq!(message.anomalies().count(), 0);
    let restated: &[u8] = b"MSGTYPE=D|NOPARTYIDS=1|NOPARTYIDS[0]=PARTYID=A\x04\x03PARTYROLE=1\
|#NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=A\x04\x03PARTYROLE=1";
    let message = reader.one_line(restated, false).unwrap();
    assert_eq!(
        message.by_path(&path("Parties[0].PartyID")).unwrap(),
        &Scalar::from("A")
    );
    assert!(message.as_field().index_of("#nopartyids").is_none());
    assert!(
        message
            .entries()
            .iter()
            .all(|entry| !entry.key().as_bytes().starts_with(b"#"))
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
    let message = reader.one_line(closed, false).unwrap();
    assert_eq!(
        message
            .by_path(&path("Parties[0].PtysSubGrp[0].PartySubID"))
            .unwrap(),
        &Scalar::from("a")
    );
    assert_eq!(
        message.by_path(&path("Parties[0].PartyID")).unwrap(),
        &Scalar::from("X")
    );
    assert_eq!(
        message.by_path(&path("Parties[1].PartyID")).unwrap(),
        &Scalar::from("Y")
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
            out.push(entry.key().as_str().unwrap_or_default().to_owned());
            keys(entry.children(), out);
        }
    }
    let mut arrived = Vec::new();
    keys(message.entries(), &mut arrived);
    // The arrival record keeps the occurrence the bridge wrote, packed value
    // and all - the sub-occurrence above is a reading of that value, and no
    // range of the line spells `NOPARTYIDS[0].NOPARTYSUBIDS[0].VENUE_SEQ`.
    assert_eq!(
        arrived,
        ["MSGTYPE", "NOPARTYIDS", "NOPARTYIDS[0]", "NOPARTYIDS[1]"]
    );
    let occurrence = message
        .entries()
        .iter()
        .find(|entry| entry.tag() == 453)
        .expect("the counter pair")
        .children()[0]
        .value()
        .as_str()
        .expect("the packed value");
    assert!(occurrence.contains("VENUE_SEQ=7"), "{occurrence}");

    // A run carrying no close is bounded by what the dictionary declares:
    // the pairs after the sub-occurrence belong to it while its group
    // declares them, and the first it does not - the venue's own key here -
    // comes back up to the party, as does everything after it.
    let open: &[u8] = b"MSGTYPE=D|NOPARTYIDS=1\
|NOPARTYIDS[0]=NOPARTYSUBIDS=1\x04\x03NOPARTYSUBIDS[0]=PARTYSUBID=a\x04\x03PARTYSUBIDTYPE=1\x04\x03VENUE_SEQ=7\x04\x03PARTYID=X\x04\x03PARTYROLE=1\x04\x03";
    let message = reader.one_line(open, false).unwrap();
    assert_eq!(
        message
            .by_path(&path("Parties[0].PtysSubGrp[0].PartySubID"))
            .unwrap(),
        &Scalar::from("a")
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
        &Scalar::from("X")
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
    let message = reader.one_line(spaced, false).unwrap();
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
    let message = reader.one_line(&deep, false).unwrap();
    assert_eq!(message.by_tag(453).unwrap().as_i64(), Some(1));
    // The row is what the two thousand openers built; the arrival record is
    // the one pair the bridge wrote, which is what a range of the line can
    // name.
    let mut arrived = Vec::new();
    keys(message.entries(), &mut arrived);
    assert_eq!(arrived, ["MSGTYPE", "NOPARTYIDS", "NOPARTYIDS[0]"]);
    let mut depth = 0;
    let mut level = message
        .get_by_path(&path("Parties[0].PtysSubGrp"))
        .and_then(Scalar::as_sequence);
    while let Some(held) = level {
        depth += 1;
        level = held
            .first()
            .and_then(Scalar::as_sequence)
            .and_then(|occurrence| occurrence.iter().find_map(Scalar::as_sequence));
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
    let message = reader.one_line(row, false).unwrap();
    assert_eq!(
        message
            .by_path(&path(
                "TrdCapRptSideGrp[0].Parties[0].PtysSubGrp[0].PartySubID"
            ))
            .unwrap(),
        &Scalar::from("a")
    );
    assert_eq!(
        message
            .by_path(&path("TrdCapRptSideGrp[0].Parties[0].PartyID"))
            .unwrap(),
        &Scalar::from("X")
    );
    assert_eq!(
        message
            .by_path(&path("TrdCapRptSideGrp[0].Parties[0].PartyRole"))
            .unwrap()
            .as_i64(),
        Some(1)
    );
    assert_eq!(
        message
            .by_path(&path("TrdCapRptSideGrp[0].Parties[1].PartyID"))
            .unwrap(),
        &Scalar::from("Y")
    );

    // A pair no level declares ends the sub-identifier, then the party, and
    // lands on the side - the packed value's own occurrence - with
    // everything after it, because without a close nothing says where the
    // bridge meant it to go.
    let lifted: &[u8] = b"MSGTYPE=AE|NOSIDES=1\
|NOSIDES[0]=NOPARTYIDS=1\x04\x03NOPARTYIDS[0]=NOPARTYSUBIDS=1\x04\x03NOPARTYSUBIDS[0]=PARTYSUBID=a\x04\x03VENUE_SEQ=7\x04\x03PARTYID=X\x04\x03";
    let message = reader.one_line(lifted, false).unwrap();
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
    let message = reader.one_line(frame.as_bytes(), false).unwrap();
    let keys: Vec<&str> = message
        .entries()
        .iter()
        .map(|entry| entry.key().as_str().unwrap_or_default())
        .collect();
    assert_eq!(keys, ["8", "35", "SYMBOL", "212", "213", "10"]);
    assert_eq!(message.by_tag(55).unwrap().as_str(), Some("TTF"));
    assert_eq!(message.by_tag(37).unwrap().as_str(), Some("9"));
    assert_eq!(message.by_tag(54).unwrap().as_str(), Some("1"));
    assert!(message.by_name("#orderid").is_err());

    // A key marked twice is judged one mark at a time: `##ORDERID` twins
    // `#ORDERID` as `#ORDERID` twins `ORDERID`.
    let restated = reader
        .one_line(b"MSGTYPE=D|#ORDERID=123|##ORDERID=123", false)
        .unwrap();
    assert_eq!(restated.by_tag(37).unwrap().as_str(), Some("123"));
    let keys: Vec<&str> = restated
        .entries()
        .iter()
        .map(|entry| entry.key().as_str().unwrap_or_default())
        .collect();
    assert_eq!(keys, ["MSGTYPE", "ORDERID"]);
    let beside = reader
        .one_line(b"MSGTYPE=D|#ORDERID=123|##ORDERID=345", false)
        .unwrap();
    assert_eq!(beside.by_tag(37).unwrap().as_str(), Some("123"));
    let at = beside
        .as_field()
        .index_of("##orderid")
        .expect("the twice-marked key");
    assert_eq!(
        beside.as_value().as_sequence().expect("a row")[at].as_str(),
        Some("345")
    );
    let alone = reader.one_line(b"MSGTYPE=D|##ORDERID=345", false).unwrap();
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
        assert_eq!(
            message.into_bytes(b'|'),
            frame,
            "a data field re-emits the bytes the frame carried"
        );
        message
            .entries()
            .iter()
            .find(|entry| entry.tag() == tag)
            .unwrap_or_else(|| panic!("tag {tag}"))
            .value()
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
    let keys: Vec<&str> = message
        .entries()
        .iter()
        .map(|entry| entry.key().as_str().unwrap_or_default())
        .collect();
    assert_eq!(keys, ["8", "35", "11", "10"]);
    assert!(message.get_by_name("ts").is_none());
    assert_eq!(message.into_bytes(b'|'), b"8=FIX.4.4|35=D|11=A1|10=000|");

    // A body that opens at its own first pair is bounded at zero, so nothing
    // a caller hands in whole is dropped.
    let bare: &[u8] = b"8=FIX.4.4|35=D|11=A1|10=000|";
    assert_eq!(reader.parse_fix_line(bare).unwrap().into_bytes(b'|'), bare);
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
    let message = reader.one_line(packed, false).unwrap();
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
fn a_dialect_declaring_a_code_twice_names_no_message_and_the_standard_does_not_answer() {
    // A dialect's own message wins where it declares the code once, the
    // standard one answers where it declares it not at all, and a code it
    // declares twice names nothing: ambiguity is an answer, not an absence,
    // so the standard grammar never stands in for a dialect's doubled one.
    let dual = FixBranch::from_str("dual").unwrap();
    let declare = |registry: &mut FixRegistry, name: &str| {
        let mut allocation = DataType::Int32.nullable_field("AllocQty");
        allocation.as_fix_mut().set_tag(80).unwrap();
        let mut message = DataType::from_fields([allocation])
            .unwrap()
            .required_field(name);
        message.as_fix_mut().set_branch(&dual).unwrap();
        message.as_fix_mut().set_msgtype("J").unwrap();
        registry
            .create_definition(FixCategory::Messages, message)
            .unwrap();
    };
    let frame: &[u8] = b"8=FIX.4.4|35=J|70=A1|78=1|79=ACC|80=5|10=0|";
    let grouped = |message: &yggdryl::FixMsg| message.get_by_name("allocgrp").is_some();

    // Undeclared: the standard AllocationInstruction folds the allocation
    // group the frame states flat.
    let mut registry = registry().as_ref().clone();
    registry.set_branch(dual.clone()).unwrap();
    let none = FixCodec::new(Arc::new(registry.clone())).with_branch(&dual);
    assert!(grouped(&none.one_line(frame, false).unwrap()));

    // Declared once: the dialect's own grammar, which declares no group.
    declare(&mut registry, "AllocIn");
    let once = FixCodec::new(Arc::new(registry.clone())).with_branch(&dual);
    let message = once.one_line(frame, false).unwrap();
    assert!(!grouped(&message));
    assert_eq!(message.by_tag(80).unwrap().as_f64(), Some(5.0));

    // Declared twice: no message, so no grammar of anyone's.
    declare(&mut registry, "AllocOut");
    assert!(registry.get_msgtype("J", Some(&dual)).is_none());
    let twice = FixCodec::new(Arc::new(registry)).with_branch(&dual);
    let message = twice.one_line(frame, false).unwrap();
    assert!(!grouped(&message));
    assert_eq!(message.anomalies().count(), 0);
}

#[test]
fn indexed_occurrences_are_built_by_index_and_a_gap_is_null() {
    let reader = reader();
    // Out of order and gapped: `[2]` before `[0]`, with `[1]` absent.
    let message = reader
        .one_line(b"MSGTYPE=D|PartyID[2]=third|PartyID[0]=first", false)
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
        .one_line(b"8=FIX.4.4|55=AAPL|35=D|9=100|10=000|", false)
        .unwrap();
    let names: Vec<&str> = message
        .as_field()
        .dtype()
        .as_fields()
        .expect("a struct root")
        .iter()
        .map(yggdryl::Field::name)
        .collect();
    // The header in rank order, the body, the trailer, and the crate's own
    // clock closing the message.
    assert_eq!(
        names,
        [
            "beginstring",
            "bodylength",
            "msgtype",
            "symbol",
            "version",
            "checksum",
            "timestamp"
        ],
        "{names:?}"
    );
}

#[test]
fn a_message_re_emits_from_its_entries_and_reads_back_equal() {
    let reader = reader();
    let row = "8=FIX.4.4|35=D|11=ORDER-1|55=AAPL|54=1|38=100|10=000|";
    let message = reader.one_line(row.as_bytes(), false).unwrap();

    // The emit is the wire record, so a translated code cannot leak into it.
    let bytes = message.into_bytes(b'|');
    assert_eq!(String::from_utf8(bytes.clone()).unwrap(), row);
    let again = reader
        .clone()
        .with_separator(b'|')
        .parse_fix_line(&bytes)
        .unwrap();
    assert_eq!(again.entries(), message.entries());
    assert_eq!(again.as_value(), message.as_value());

    assert_eq!(message.into_text('|').unwrap(), row);
    assert_eq!(message.version(), Some("4.4".parse::<Version>().unwrap()));
}

#[test]
fn a_version_never_renames_or_retypes_the_column_a_tag_lands_in() {
    let reader = reader();
    let old = reader
        .clone()
        .with_version("4.2".parse::<Version>().unwrap());
    let newest = reader
        .clone()
        .with_version("5.0.2".parse::<Version>().unwrap());

    // Tag 32 is `LastShares` typed `int` in 4.0, `LastShares` typed `Qty` in
    // 4.2 and `LastQty` from 4.3 on. A row read at 4.2 still builds the
    // dictionary's own column, because a tag that renamed itself per version
    // is a tag no two captures of one venue could be read together on.
    let at_42 = old.one_line(b"8=FIX.4.2|35=8|32=100|10=0|", false).unwrap();
    let at_new = newest
        .one_line(b"8=FIX.4.4|35=8|32=100|10=0|", false)
        .unwrap();
    let column = |held: &yggdryl::FixMsg| {
        let at = held
            .as_field()
            .index_of("lastqty")
            .expect("the tag 32 column");
        held.as_field().fields()[at].clone()
    };
    assert_eq!(column(&at_42).name(), column(&at_new).name());
    assert_eq!(column(&at_42).dtype(), column(&at_new).dtype());
    assert_eq!(at_42.by_tag(32).unwrap(), at_new.by_tag(32).unwrap());
    // The 4.2 spelling still reaches it - as an alias, and off the lineage
    // the column carries - so nothing the version knew is lost.
    assert!(at_42.get_by_name("lastshares").is_some());
    assert_eq!(
        column(&at_42)
            .as_fix()
            .name_at("4.2".parse::<Version>().unwrap()),
        Some("lastshares"),
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
    let message = reader.one_line(row.as_bytes(), false).unwrap();

    let field = message
        .as_field()
        .get_field_by_path("parties")
        .expect("the separate logical group");
    let DataType::List(item) = field.dtype() else {
        panic!("the group's own shape, got {}", field.dtype());
    };
    assert!(item.dtype().is_nested(), "a List of `item` Structs");
    let occurrences = message.by_name("parties").unwrap().as_sequence().unwrap();
    assert_eq!(occurrences.len(), 2, "one occurrence per delimiter");
    let ids: Vec<&str> = occurrences
        .iter()
        .map(|occurrence| occurrence.as_sequence().unwrap()[0].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["BUYSIDE", "VENUE"]);
    // The counter is a scalar column of its own and keeps what the frame
    // stated, beside the group the occurrences reach.
    assert_eq!(message.by_tag(453).unwrap(), &Scalar::from(2_i32));
    assert_eq!(
        message.by_path(&path("Parties[0].PartyRole")).unwrap(),
        &Scalar::from(1_i32)
    );
    assert_eq!(
        message.by_path(&path("Parties[1].PartyRole")).unwrap(),
        &Scalar::from(17_i32)
    );
    // A member lives in its occurrence and nowhere else: nothing is flat at
    // the root, and the field after the group is the order's own again.
    assert!(message.get_by_tag(448).is_none(), "no flat party id");
    assert!(message.get_by_tag(452).is_none(), "no flat party role");
    assert_eq!(message.by_tag(54).unwrap().as_str(), Some("1"));
    assert!(message.anomalies().next().is_none(), "the count is met");
    // The entries keep the arrival: each member is a child of the counter.
    let counter = message
        .entries()
        .iter()
        .find(|entry| entry.key().as_bytes() == b"453")
        .expect("the counter's entry");
    assert_eq!(counter.children().len(), 6);
    assert_eq!(message.into_bytes(b'|'), row.as_bytes());
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
    let message = reader.one_line(row.as_bytes(), false).unwrap();
    let alternates = message
        .by_name("secaltidgrp")
        .unwrap()
        .as_sequence()
        .unwrap();
    let ids: Vec<&str> = alternates
        .iter()
        .map(|occurrence| occurrence.as_sequence().unwrap()[0].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["US0378331005", "US5949181045", "MSFT.O"]);
    let anomalies: Vec<String> = message.anomalies().map(|held| held.to_string()).collect();
    assert!(
        anomalies
            .iter()
            .any(|held| held == "secaltidgrp (454) states 2 occurrences and holds 3"),
        "{anomalies:?}"
    );
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
    registry
        .insert_definition(FixCategory::Groups, subs)
        .unwrap();
    registry
        .insert_definition(FixCategory::Groups, parties)
        .unwrap();
    let registry = Arc::new(registry);

    let row = "35=D|55=AAPL|453=2|448=A|452=1|802=2|523=S1|803=1|523=S2|803=2|448=B|452=3|10=0|";
    let message = FixCodec::new(registry)
        .one_line(row.as_bytes(), false)
        .unwrap();
    let occurrences = message.by_name("parties").unwrap().as_sequence().unwrap();
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
    assert!(message.anomalies().next().is_none());
}

#[test]
fn a_state_code_is_read_through_the_name_its_field_gives_it() {
    let reader = reader();
    // `D` is one letter in two code sets: Restated as an `ExecType`, which
    // leaves the order replaced, and AcceptedForBidding as an `OrdStatus`,
    // which is an acknowledgement. The column reads the name the field gives
    // the code, so each lands where its own specification puts it.
    let restated = reader
        .one_line(b"8=FIX.4.4|35=8|150=D|10=0|", true)
        .unwrap();
    assert_eq!(
        restated.by_tag(150).unwrap().as_str(),
        Some(State::from_spelling("Restated").unwrap().as_str())
    );
    // The derived state follows the typed column, not the letter.
    assert_eq!(
        restated.by_tag(yggdryl::STATE_TAG).unwrap().as_str(),
        Some(State::from_spelling("Restated").unwrap().as_str())
    );
    let bidding = reader.one_line(b"8=FIX.4.4|35=8|39=D|10=0|", true).unwrap();
    assert_eq!(
        bidding.by_tag(39).unwrap().as_str(),
        Some(State::from_spelling("AcceptedForBidding").unwrap().as_str())
    );
    // A code both sets spell alike reads alike, whichever field carries it.
    let new = reader
        .one_line(b"8=FIX.4.4|35=8|39=0|150=0|10=0|", false)
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
        .one_line(
            b"MSGTYPE=D|453=2|453[0]=448=BUYSIDE|453[1]=448=VENUE",
            false,
        )
        .unwrap();
    let by_name = reader
        .one_line(
            b"MSGTYPE=D|NOPARTYIDS=2|NOPARTYIDS[0]=PARTYID=BUYSIDE|NOPARTYIDS[1]=PARTYID=VENUE",
            false,
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
            [
                "beginstring",
                "msgtype",
                "nopartyids",
                "parties",
                "version",
                "timestamp"
            ],
            "the counter and the group, beside the three children every message has"
        );
        assert_eq!(message.by_tag(453).unwrap(), &Scalar::from(2_i32));
        let occurrences = message.by_name("parties").unwrap().as_sequence().unwrap();
        assert_eq!(occurrences.len(), 2);
        assert!(message.anomalies().next().is_none());
    }
    assert_eq!(
        by_tag.by_name("parties").unwrap(),
        by_name.by_name("parties").unwrap()
    );
}

#[test]
fn an_unnamed_occurrence_preserves_raw_input_under_the_declared_component() {
    let message = reader()
        .one_line(
            b"MSGTYPE=D|NOPARTYIDS=2|NOPARTYIDS[0]=ONE|NOPARTYIDS[1]=TWO",
            false,
        )
        .unwrap();
    assert_eq!(message.by_tag(453).unwrap(), &Scalar::from(2_i32));
    let occurrences = message.by_name("parties").unwrap().as_sequence().unwrap();
    assert_eq!(occurrences, &[Scalar::Null, Scalar::Null]);
    let group = message.as_field().get_field_by_path("parties").unwrap();
    let DataType::List(item) = group.dtype() else {
        panic!("{}", group.dtype());
    };
    assert_eq!(item.name(), "party");
    assert!(matches!(item.dtype(), DataType::Struct(_)));
    assert!(item.is_nullable());
    let bytes = message.into_bytes(b'|');
    let raw = std::str::from_utf8(&bytes).unwrap();
    assert!(raw.contains("NOPARTYIDS[0]=ONE"), "{raw}");
    assert!(raw.contains("NOPARTYIDS[1]=TWO"), "{raw}");
    assert!(message.anomalies().next().is_none());
}

#[test]
fn a_renamed_group_builds_one_column_under_the_name_the_dictionary_holds() {
    let reader = reader()
        .clone()
        .with_version("4.2".parse::<Version>().unwrap());
    // Tag 33 is `LinesOfText` before 4.4 and `NoLinesOfText` after, and the
    // message is read at 4.2 - but a field is one column under the one name
    // the dictionary holds it by, whatever version the row is read at. What
    // 4.2 called it stays readable through the field's lineage.
    let message = reader
        .one_line(
            b"MSGTYPE=B|NOLINESOFTEXT=2|NOLINESOFTEXT[0]=TEXT=a|NOLINESOFTEXT[1]=TEXT=b",
            false,
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
    assert_eq!(
        names,
        [
            "beginstring",
            "msgtype",
            "nolinesoftext",
            "linesoftextgrp",
            "version",
            "timestamp"
        ],
        "{names:?}"
    );
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
    assert_eq!(
        group.as_fix().name_at("4.2".parse().unwrap()),
        Some("linesoftext"),
        "the 4.2 spelling is still readable off the column",
    );
    assert!(message.anomalies().next().is_none());
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
    registry
        .insert_definition(FixCategory::Components, item)
        .unwrap();
    registry
        .insert_definition(FixCategory::Groups, group)
        .unwrap();
    let registry = Arc::new(registry);

    let message = FixCodec::new(registry)
        .one_line(b"35=D|55=AAPL|453=2|10=0|", false)
        .unwrap();
    let anomalies: Vec<String> = message.anomalies().map(|held| held.to_string()).collect();
    assert_eq!(
        anomalies,
        ["parties (453) states 2 occurrences and holds 0"],
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
        .one_line(
            b"recv 8=FIX.4.4\x019=61\x0135=0\x0149=XPAR\x0110=017\x01",
            false,
        )
        .expect("a numeric frame");
    for spelling in ["^A", "\\x01", "<SOH>", "{SOH}"] {
        let line = format!(
            "recv 8=FIX.4.4{spelling}9=61{spelling}35=0{spelling}49=XPAR{spelling}10=017{spelling} on session 3"
        );
        let held = reader
            .one_line(line.as_bytes(), false)
            .expect("the same frame, escaped");
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
        let held = reader
            .one_line(line.as_bytes(), false)
            .expect("a bridge row");
        let parties = held
            .get_by_name("parties")
            .and_then(Scalar::as_sequence)
            .unwrap_or_else(|| panic!("the group with packed separator {separator:?}"));
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
        .one_line(
            b"MSGTYPE=D|NOPARTYIDS=1|\
             NOPARTYIDS[0]=VENUEFLAG=XSymbol=TTFPARTYROLE=1",
            false,
        )
        .expect("a bridge row");

    // The counter pair arrived, so the occurrence rides under it in the
    // arrival record - as the bridge wrote it, residue included, because the
    // members below are this reader's reading of that one pair.
    let counter = message
        .entries()
        .iter()
        .find(|entry| entry.tag() == 453)
        .expect("the counter pair");
    let occurrence = counter
        .children()
        .iter()
        .find(|entry| entry.key().as_bytes() == b"NOPARTYIDS[0]")
        .expect("the occurrence the bridge wrote");
    assert_eq!(occurrence.tag(), 0);
    assert_eq!(
        occurrence.value().as_str(),
        Some("VENUEFLAG=XSymbol=TTFPARTYROLE=1")
    );
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
    let members = message
        .get_by_name("parties")
        .and_then(Scalar::as_sequence)
        .and_then(|parties| parties[0].as_sequence())
        .expect("one occurrence");
    assert_eq!(members.len(), 2);

    // A selected message's direct members outrank the wider global Parties
    // definition, even when the key addresses its numeric counter.
    let mut scoped = registry().as_ref().clone();
    let item = DataType::from_fields([scoped.field_by_tag(448).unwrap().clone()])
        .unwrap()
        .required_field("minimalparty");
    let mut group = DataType::list(item).nullable_field("minimalparties");
    group.as_fix_mut().set_counter(453).unwrap();
    scoped
        .insert_definition(FixCategory::Groups, group.clone())
        .unwrap();
    let mut definition = DataType::from_fields([scoped.field_by_tag(453).unwrap().clone(), group])
        .unwrap()
        .required_field("minimalpartiesmessage");
    definition.as_fix_mut().set_msgtype("ZMIN").unwrap();
    scoped
        .insert_definition(FixCategory::Messages, definition)
        .unwrap();
    let numeric_name = FixCodec::new(Arc::new(scoped))
        .one_line(
            b"MSGTYPE=ZMIN|#453=1|#453[0]=PARTYID=BUYSIDEPARTYROLE=1",
            false,
        )
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
    let flat: Vec<&FixEntry> = numeric_name
        .entries()
        .iter()
        .flat_map(|entry| std::iter::once(entry).chain(entry.children()))
        .collect();
    let party = flat
        .iter()
        .find(|entry| entry.key().as_bytes() == b"453[0]")
        .expect("the occurrence the bridge wrote");
    assert_eq!(party.value().as_str(), Some("PARTYID=BUYSIDEPARTYROLE=1"));
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
    let reader = FixCodec::new(Arc::clone(&registry));
    let schema = fix_schema(&registry, "fix").expect("the fixed schema");
    let held = reader
        .one_line(
            b"toBridge #NOPARTYIDS=1|#NOPARTYIDS[0]=PARTYID=BUYSIDE",
            false,
        )
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
            .one_line(
                format!("{frame}|35=D|{tag}={value}|10=0|").as_bytes(),
                false,
            )
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
    // same rule that keeps a dateless `UTCTimestamp` from becoming an instant
    // on the epoch day.
    assert_eq!(latest("07:39:12", 1079), Scalar::Null);
    assert_eq!(read("8=FIX.4.4", "10:15:30.000", 60), Scalar::Null);

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
}

/// A capture is never ordered by the epoch day.
///
/// The clock column is declared an instant however narrow the dictionary is,
/// so a clock field typed as text is read through FIX's own spelling. A
/// `TZTimeOnly` is a legal reading of that spelling and never a moment a
/// capture happened at, so a dateless value contributes nothing and the
/// ladder keeps walking - to the epoch itself, where a message with no clock
/// at all is stamped, which sorts first and visibly rather than among the
/// rows of whatever day it was read on.
#[test]
fn a_dateless_clock_never_becomes_the_capture_instant() {
    use yggdryl::{TimeUnit, Timezone};

    // A dictionary narrow enough to type the clock as text is what reaches
    // the reading at all: a full one has already made it an instant.
    let mut narrow = FixRegistry::new();
    let mut clock = DataType::utf8().nullable_field("transacttime");
    clock.as_fix_mut().set_tag(60).expect("a standard tag");
    narrow.insert(clock).expect("a fresh dictionary");
    let reader = FixCodec::new(Arc::new(narrow));
    let clocked = |value: &str| {
        reader
            .one_line(format!("8=FIX.4.4|35=D|60={value}|10=0|").as_bytes(), false)
            .expect("a readable message")
            .market_timestamp()
    };

    let instant = |count: i64| {
        Scalar::datetime64(count, TimeUnit::Nanosecond, Timezone::UTC).expect("a nanosecond count")
    };
    assert_eq!(
        clocked("07:39:12.123+05:30"),
        instant(0),
        "the epoch, never the epoch day"
    );
    // A dated one is the instant the clock column holds, which is what this
    // derivation exists to produce.
    assert_eq!(
        clocked("20240102-10:15:30.000"),
        instant(1_704_190_530_000_000_000),
    );
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
    let codec = codec();
    let named = |row: &[u8]| {
        codec
            .one_line(row, false)
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
        .one_line(br#"<Order ClOrdID="XML-1"/>"#, false)
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
        .map(|row| TextLine::new(0, TextBytes::from_bytes(row).expect("a capture page")))
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
    // what the record read said.
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
}

#[test]
fn a_row_inside_a_data_field_is_read_at_its_own_version_and_not_the_frames() {
    // A dictionary that reaches 5.0.2, holding one field whose code set spells
    // `Restated` two ways: the value 4.2 knew, and the one that replaced it.
    // Only one of the two is visible at a version, so which one answers is
    // exactly which version the read used.
    let mut scoped = FixRegistry::new();
    let mut exectype = DataType::utf8().nullable_field("exectype");
    exectype.as_fix_mut().set_tag(150).unwrap();
    exectype
        .as_fix_mut()
        .set_lineage(&[
            yggdryl::FixLineageEntry::new(yggdryl::FixPedigree::new(
                "4.2".parse::<Version>().unwrap(),
                None,
            ))
            .with_name("exectypeold"),
            yggdryl::FixLineageEntry::new(yggdryl::FixPedigree::new(
                "5.0.2".parse::<Version>().unwrap(),
                None,
            ))
            .with_name("exectype"),
        ])
        .unwrap();
    exectype
        .as_fix_mut()
        .set_codes(&[
            yggdryl::FixCode::new("RestatedOld", "1")
                .with_aliases(["Restated"])
                .with_deprecated("4.3".parse::<Version>().unwrap()),
            yggdryl::FixCode::new("Restated", "D")
                .with_since("4.3".parse::<Version>().unwrap(), None),
        ])
        .unwrap();
    scoped.insert(exectype).unwrap();
    let registry = Arc::new(scoped);
    assert_eq!(
        registry.newest().map(|held| held.version()),
        Some("5.0.2".parse::<Version>().unwrap())
    );

    // A 4.2 session carrying a row written to a later FIX, which is what a
    // bridge relaying into a long-lived session actually sends.
    let frame: &[u8] = b"8=FIX.4.2|9=0|35=UL|212=17|213=EXECTYPE=Restated|10=0|";

    // The frame's `BeginString` is the envelope's and stays the message's;
    // the row inside `XmlData` states no version of its own, so it is read at
    // the dictionary's newest rather than at the session's.
    let message = FixCodec::new(Arc::clone(&registry))
        .parse_fix_line(frame)
        .unwrap();
    assert_eq!(message.by_tag(8).unwrap().as_str(), Some("FIX.4.2"));
    assert_eq!(message.by_tag(150).unwrap().as_str(), Some("D"));

    // A pinned version is the caller speaking for the whole run and answers
    // for the nested row too.
    let dated = FixCodec::new(Arc::clone(&registry))
        .with_version("4.2".parse::<Version>().unwrap())
        .parse_fix_line(frame)
        .unwrap();
    assert_eq!(dated.by_tag(150).unwrap().as_str(), Some("1"));
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
    let message = reader.one_line(&frame, false).unwrap();

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
    // re-emits byte for byte: a reading of a value is not a second arrival.
    assert_eq!(message.by_tag(213).unwrap().as_bytes(), Some(&document[..]));
    assert_eq!(message.into_bytes(b'|'), frame);
}

#[test]
fn every_generated_message_carries_the_version_the_read_used() {
    let frame: &[u8] = b"8=FIX.4.2|35=D|55=AAPL|54=1|10=0|";

    // With nothing pinned, the frame's own `BeginString` is what answered it,
    // so the two agree and the message says so once.
    let read = reader().one_line(frame, false).unwrap();
    assert_eq!(read.by_tag(8).unwrap().as_str(), Some("FIX.4.2"));
    assert_eq!(read.version(), Some(Version::new(4, 2, 0)));

    // A pinned version is the caller speaking for the whole run: every
    // message the codec makes is read at that target and carries it, while
    // the frame keeps saying what the session said.
    let dated = codec()
        .with_version(Version::new(4, 4, 0))
        .one_line(frame, false)
        .unwrap();
    assert_eq!(dated.by_tag(8).unwrap().as_str(), Some("FIX.4.2"));
    assert_eq!(
        dated.by_tag(yggdryl::VERSION_TAG).unwrap().as_str(),
        Some("4.4")
    );
    assert_eq!(dated.version(), Some(Version::new(4, 4, 0)));

    // A row that dates itself not at all is read at the dictionary's newest,
    // and the `BeginString` it never stated is filled from the same answer.
    let newest = registry().newest().expect("the seed's newest").version();
    let bare = reader().one_line(b"MSGTYPE=D|SYMBOL=AAPL", false).unwrap();
    assert_eq!(bare.version(), Some(newest));
    assert_eq!(
        bare.by_tag(8).unwrap().as_str(),
        Some(format!("FIX.{newest}").as_str())
    );
}
