//! Which way a message moved: FIX's tag 385, read by the registry (decision
//! 14) through the rules the dictionary carries on it (decision 15).

use std::sync::Arc;

use yggdryl::fix::{MsgDirection, RECEIVE_PATTERNS, SEND_PATTERNS};
use yggdryl::{
    DataType, Field, FixCode, FixCodec, FixDirection, FixRegistry, MSGDIRECTION_TAG_NAME,
};

fn reading() -> MsgDirection {
    super::committed_registry().msgdirection()
}

#[test]
fn a_direction_is_the_verb_in_front_of_the_payload_and_nothing_else() {
    let reading = reading();
    // Read, as a code of tag 385's set.
    for (line, direction) in [
        ("sending >> 8=FIX.4.2|9=176|35=D|10=203|", Some("S")),
        ("recv 8=FIX.4.4|35=0|10=017|", Some("R")),
        ("Receiving XmlApi: <Execution ExecID='E1'/>", Some("R")),
        ("[OUT] 8=FIX.4.4|35=D|", Some("S")),
        ("(in) ACCOUNT=A1", Some("R")),
    ] {
        assert_eq!(reading.read_text(line), direction, "{line}");
    }

    // The verb in front of the payload is the verb, and the English `in` or
    // `out` beside it is no marker that can contradict it (decision 15):
    // these two answered nothing under decision 14's table.
    assert_eq!(reading.read_text("sending in session 3"), Some("S"));
    assert_eq!(reading.read_text("received out of order"), Some("R"));

    // These are the shapes that must read nothing.
    for line in [
        // The bare word selects only bracketed: a whole word in English, a
        // route endpoint and a session name are none of that shape, where a
        // word boundary alone would have been enough to get it wrong.
        "logged out 8=FIX.4.4|35=D|",
        "out 8=FIX.4.4|35=D|",
        "(DEBUG) IN : #CFICODE=ESVTFR|#ISINCODE=CH0012221716|",
        "direct:out 8=FIX.4.4|35=D|",
        "MCFID-IN-XPAR 8=FIX.4.4|35=D|",
        // Both verbs in one prefix: none a reading can prefer.
        "sending and receiving 8=FIX.4.4|35=D|",
        // A verb only inside the payload is the payload's word.
        "8=FIX.4.4|35=8|58=sent earlier|10=1|",
        "ACCOUNT=A1|TEXT=received late|",
        // No verb at all.
        "8=FIX.4.4|35=D|",
        "no level printed by this plugin",
    ] {
        assert_eq!(reading.read_text(line), None, "{line}");
    }
}

#[test]
fn the_reading_is_the_registrys_code_set_and_a_dictionary_without_the_field_answers_the_specifications()
 {
    // The committed dictionary types tag 385 as text carrying its code set,
    // and the reading answers those codes.
    let committed = reading();
    assert_eq!(committed.sent(), "S");
    assert_eq!(committed.recv(), "R");
    assert_eq!(committed.codes().collect::<Vec<_>>(), ["R", "S"]);
    assert_eq!(committed.field().as_fix().tag().unwrap(), Some(385));
    assert_eq!(committed.field().dtype(), &DataType::utf8());
    assert!(!committed.field().is_nullable());
    // Any spelling of a code resolves to the code; a spelling outside the
    // set resolves to nothing.
    for (spelling, code) in [
        ("S", Some("S")),
        ("Send", Some("S")),
        ("send", Some("S")),
        ("R", Some("R")),
        ("Receive", Some("R")),
        ("", None),
        ("sent", None),
        ("sideways", None),
    ] {
        assert_eq!(committed.code(spelling), code, "{spelling:?}");
    }

    // A registry without the field answers the specification's own set.
    let bare = FixRegistry::new().msgdirection();
    assert_eq!(bare.sent(), "S");
    assert_eq!(bare.recv(), "R");
    assert_eq!(bare.codes().collect::<Vec<_>>(), ["S", "R"]);
    assert_eq!(bare.field().as_fix().tag().unwrap(), Some(385));
    assert_eq!(bare.code("Receive"), Some("R"));
    assert_eq!(bare.read_bytes(b"recv 8=FIX.4.4|35=0|10=017|"), Some("R"));

    // A dictionary extending the set keeps the specification's two halves
    // under the names it gives them.
    let mut extended = FixRegistry::new();
    let mut field = DataType::utf8().nullable_field("MsgDirection");
    field.as_fix_mut().set_tag(385).unwrap();
    field
        .as_fix_mut()
        .set_codes(&[
            FixCode::new("Receive", "IN"),
            FixCode::new("Send", "OUT"),
            FixCode::new("Both", "B"),
        ])
        .unwrap();
    extended.insert(field).unwrap();
    let reading = extended.msgdirection();
    assert_eq!(reading.sent(), "OUT");
    assert_eq!(reading.recv(), "IN");
    // In the order the dictionary holds the set, which sorts it by value.
    assert_eq!(reading.codes().collect::<Vec<_>>(), ["B", "IN", "OUT"]);
    assert_eq!(reading.code("both"), Some("B"));
    assert_eq!(
        reading.read_bytes(b"sending >> 8=FIX.4.4|35=D|10=0|"),
        Some("OUT")
    );
}

/// A registry whose tag 385 declares the specification's set and carries
/// the rules given, or none.
fn ruled(directions: &[FixDirection]) -> Arc<FixRegistry> {
    let mut registry = FixRegistry::new();
    registry.insert(ruled_field(directions)).unwrap();
    Arc::new(registry)
}

fn ruled_field(directions: &[FixDirection]) -> Field {
    let mut field = DataType::utf8().nullable_field("MsgDirection");
    field.as_fix_mut().set_tag(385).unwrap();
    field
        .as_fix_mut()
        .set_codes(&[FixCode::new("Receive", "R"), FixCode::new("Send", "S")])
        .unwrap();
    field.as_fix_mut().set_directions(directions).unwrap();
    field
}

#[test]
fn a_dictionary_without_the_property_reads_by_the_defaults_as_data_and_as_readings() {
    // The committed dictionary carries no `fix:directions`: the rules in
    // force are the crate's defaults, keyed by the set's two halves, and
    // they are data a caller can read.
    let committed = reading();
    assert!(
        committed.field().as_fix().directions().next().is_none(),
        "the committed dictionary reads by the defaults"
    );
    let defaults = [
        FixDirection::new("S", SEND_PATTERNS),
        FixDirection::new("R", RECEIVE_PATTERNS),
    ];
    assert_eq!(committed.directions(), defaults);
    assert_eq!(FixRegistry::new().msgdirection().directions(), defaults);
    assert_eq!(ruled(&[]).msgdirection().directions(), defaults);

    // A dictionary extending the set is read by the same defaults under the
    // codes it gives the two halves.
    let mut extended = FixRegistry::new();
    let mut field = DataType::utf8().nullable_field("MsgDirection");
    field.as_fix_mut().set_tag(385).unwrap();
    field
        .as_fix_mut()
        .set_codes(&[
            FixCode::new("Receive", "IN"),
            FixCode::new("Send", "OUT"),
            FixCode::new("Both", "B"),
        ])
        .unwrap();
    extended.insert(field).unwrap();
    let extended = extended.msgdirection();
    assert_eq!(
        extended.directions(),
        [
            FixDirection::new("OUT", SEND_PATTERNS),
            FixDirection::new("IN", RECEIVE_PATTERNS),
        ]
    );
    assert_eq!(extended.read_text("[OUT] 8=FIX.4.4|35=D|"), Some("OUT"));
    assert_eq!(extended.read_text("recv 8=FIX.4.4|35=0|"), Some("IN"));
}

#[test]
fn a_rule_added_through_the_registry_changes_what_a_line_answers() {
    // A bridge that logs `TX`/`RX` is read by editing the dictionary, not
    // the crate; the code is any spelling of a code of the set, resolved
    // once, and the verb table no longer applies under a stated table.
    let registry = ruled(&[
        FixDirection::new("Send", [r"^TX\b"]),
        FixDirection::new("R", [r"^RX\b", r"(?i)<<<"]),
    ]);
    let reading = registry.msgdirection();
    assert_eq!(
        reading.directions(),
        [
            FixDirection::new("S", [r"^TX\b"]),
            FixDirection::new("R", [r"^RX\b", r"(?i)<<<"]),
        ]
    );
    assert_eq!(reading.read_text("TX 8=FIX.4.4|35=D|10=0|"), Some("S"));
    assert_eq!(reading.read_text("RX 8=FIX.4.4|35=D|10=0|"), Some("R"));
    assert_eq!(reading.read_text("09:12:03 <<< 8=FIX.4.4|35=D|"), Some("R"));
    assert_eq!(reading.read_text("TXT 8=FIX.4.4|35=D|"), None);
    assert_eq!(reading.read_text("sending >> 8=FIX.4.4|35=D|10=0|"), None);
    // A prefix two codes match answers nothing, whatever the rules.
    assert_eq!(reading.read_text("TX <<< 8=FIX.4.4|35=D|10=0|"), None);
    // The pattern is applied to the prose, never inside the payload: the
    // unanchored `<<<` would match the value, and the bound keeps it out.
    assert_eq!(reading.read_text("8=FIX.4.4|35=D|58=<<< late|10=0|"), None);
    assert_eq!(
        reading.read_text("ACCOUNT=A1|TEXT=<<< late|MSGTYPE=D|"),
        None
    );

    // The codec compiles the rules it was built with, once, and every door
    // fills tag 385 from them.
    let codec = FixCodec::new(Arc::clone(&registry));
    let message = codec
        .parse_line(b"RX 8=FIX.4.4|35=D|10=0|")
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(message.by_tag(385).unwrap().as_str(), Some("R"));
    let bridge = codec.parse_ullink_line(b"TX MSGTYPE=D|CLORDID=A").unwrap();
    assert_eq!(bridge.by_tag(385).unwrap().as_str(), Some("S"));
    let unmarked = codec
        .parse_line(b"sending >> 8=FIX.4.4|35=D|10=0|")
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(unmarked.get_by_tag(385), None);
}

#[test]
fn the_prose_in_front_of_a_jolokia_document_names_its_half_and_a_bare_document_nothing() {
    const ANSWERED: &str = concat!(
        r#"{"request":{"mbean":"com.ullink.ulbridge.sessioninterfaces.plugins:*","type":"read"},"#,
        r#""value":{"name":"send-test-request"},"status":200}"#,
    );
    const ASKED: &str = r#"{"mbean":"com.ullink.ulbridge:type=Bridge","type":"read"}"#;
    let reading = reading();
    // The envelope is prose in front of the payload, read by a default rule
    // like every other prose (decision 15); the echoed `request` key states
    // nothing.
    assert_eq!(reading.read_text(ANSWERED), None);
    assert_eq!(reading.read_text(ASKED), None);
    let answered = format!("2026-08-14 03:03:13.314 [23] [Jolokia] (DEBUG) Response: {ANSWERED}");
    assert_eq!(reading.read_text(&answered), Some("R"));
    let asked = format!("[Jolokia] (DEBUG) Request: {ASKED}");
    assert_eq!(reading.read_text(&asked), Some("S"));
    // With no document behind the prose at all, the prose still answers.
    assert_eq!(
        reading.read_text("[Jolokia] (DEBUG) Request: JmxReadRequest[attribute=null]"),
        Some("S")
    );
    assert_eq!(
        reading.read_text("Response: 8=FIX.4.4|35=0|10=0|"),
        Some("R")
    );
    // The word has to stand as the envelope's own: opened by whitespace or
    // the start, closed by its colon.
    assert_eq!(reading.read_text("HttpResponse: 8=FIX.4.4|35=0|"), None);
    assert_eq!(reading.read_text("Response 8=FIX.4.4|35=0|"), None);
    // And it is prose like any other: beside a verb naming the other half,
    // nothing.
    assert_eq!(
        reading.read_text("sending >> Response: 8=FIX.4.4|35=0|"),
        None
    );

    // On the line door the bare document fills nothing; on the batch door
    // the pin fills it.
    let codec = FixCodec::new(super::committed_registry());
    let bare = codec
        .parse_line(ANSWERED.as_bytes())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(bare.get_by_tag(385), None);
    let prosed = codec
        .parse_line(answered.as_bytes())
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(prosed.by_tag(385).unwrap().as_str(), Some("R"));
}

#[test]
fn a_pattern_the_regex_crate_refuses_is_refused_by_the_setter_and_dropped_by_the_reading() {
    let mut field = ruled_field(&[FixDirection::new("S", ["^TX "])]);
    let before = field.clone();
    // The setter is the door: an unbalanced group, an empty pattern, an
    // empty code, a code outside the set, a code named twice - under one
    // spelling or two - and an entry stating no pattern are all refused,
    // and the field is unchanged.
    for (rules, reason) in [
        (
            vec![FixDirection::new("S", ["("])],
            "expected a valid byte regex, got \"(\"",
        ),
        (vec![FixDirection::new("S", [""])], "got an empty one"),
        (
            vec![FixDirection::new("", ["^TX "])],
            "expected a code of the set, one of R, S, got \"\"",
        ),
        (
            vec![FixDirection::new("Q", ["^QX "])],
            "expected a code of the set, one of R, S, got \"Q\"",
        ),
        (
            vec![
                FixDirection::new("S", ["^TX "]),
                FixDirection::new("S", ["^RX "]),
            ],
            "expected each code once, got \"S\" naming \"S\" twice",
        ),
        (
            vec![
                FixDirection::new("S", ["^TX "]),
                FixDirection::new("Send", ["<<<"]),
            ],
            "expected each code once, got \"Send\" naming \"S\" twice",
        ),
        (
            vec![
                FixDirection::new("R", ["^RX "]),
                FixDirection::new("r", ["<<<"]),
            ],
            "expected each code once, got \"r\" naming \"R\" twice",
        ),
        (
            vec![FixDirection::new("S", Vec::<&str>::new())],
            "at least one pattern",
        ),
    ] {
        let refused = field.as_fix_mut().set_directions(&rules).unwrap_err();
        assert!(refused.to_string().contains(reason), "{refused}");
        assert_eq!(field, before);
    }
    // A field declaring no set admits the specification's two halves under
    // either spelling, and names them when it refuses.
    let mut bare = DataType::utf8().nullable_field("MsgDirection");
    bare.as_fix_mut().set_tag(385).unwrap();
    bare.as_fix_mut()
        .set_directions(&[
            FixDirection::new("send", ["^TX "]),
            FixDirection::new("Receive", ["^RX "]),
        ])
        .unwrap();
    let refused = bare
        .as_fix_mut()
        .set_directions(&[FixDirection::new("Both", ["^BX "])])
        .unwrap_err();
    assert!(
        refused
            .to_string()
            .contains("expected a code of the set, one of S, R, got \"Both\""),
        "{refused}"
    );

    // A dictionary edited by hand reaches the reading without the door: the
    // pattern that does not compile, the entry naming no code of the set
    // and the entry naming a code again under another spelling are dropped
    // and the rest read - each drop warned about in the setter's words.
    field
        .as_fix_mut()
        .insert(
            "directions",
            concat!(
                r#"{"directions":[{"code":"S","patterns":["(","^TX "]},"#,
                r#"{"code":"Q","patterns":["^QX "]},{"code":"R","patterns":["^RX "]},"#,
                r#"{"code":"Send","patterns":["<<<"]}]}"#,
            ),
        )
        .unwrap();
    let mut registry = FixRegistry::new();
    registry.insert(field).unwrap();
    let (reading, warnings) = super::warned::during(|| registry.msgdirection());
    assert_eq!(
        reading.directions(),
        [
            FixDirection::new("S", ["^TX "]),
            FixDirection::new("R", ["^RX "]),
        ]
    );
    assert_eq!(reading.read_text("TX 8=FIX.4.4|35=D|"), Some("S"));
    assert_eq!(reading.read_text("QX 8=FIX.4.4|35=D|"), None);
    assert_eq!(reading.read_text("TX <<< 8=FIX.4.4|35=D|"), Some("S"));
    let door = |rules: &[FixDirection]| {
        ruled_field(&[])
            .as_fix_mut()
            .set_directions(rules)
            .unwrap_err()
            .to_string()
    };
    assert_eq!(
        warnings,
        [
            format!(
                "tag 385 fix:directions: {}",
                door(&[FixDirection::new("S", ["("])])
            ),
            format!(
                "tag 385 fix:directions: {}",
                door(&[FixDirection::new("Q", ["^QX "])])
            ),
            format!(
                "tag 385 fix:directions: {}",
                door(&[
                    FixDirection::new("S", ["^TX "]),
                    FixDirection::new("Send", ["<<<"])
                ])
            ),
        ]
    );
    // A property the field carries reads by what it states, however little
    // survives: a document the grammar refuses at its first entry, warned
    // about, states no rule and reads nothing - never the defaults, which
    // are the absent property's.
    let mut field = ruled_field(&[]);
    assert!(!field.as_fix().directions().is_stated());
    field
        .as_fix_mut()
        .insert("directions", r#"{"directions":[{"code":"S"}]}"#)
        .unwrap();
    assert!(field.as_fix().directions().is_stated());
    let mut registry = FixRegistry::new();
    registry.insert(field).unwrap();
    let (reading, warnings) = super::warned::during(|| registry.msgdirection());
    assert_eq!(reading.directions(), []);
    assert_eq!(reading.read_text("sending >> 8=FIX.4.4|35=D|10=0|"), None);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("expected every entry to state \"patterns\""),
        "{warnings:?}"
    );
}

#[test]
fn the_rules_round_trip_through_the_field_escapes_included() {
    // A pattern carries backslashes as a matter of course; the document
    // escapes them and the reader decodes them, so what was set is what is
    // read back, taken away, and compiled.
    let rules = [
        FixDirection::new("S", [r"(?i)(?:^|\s)tx\s", r#"say "out""#]),
        FixDirection::new("R", [r"(?i)(?:^|\s)rx\s"]),
    ];
    let mut field = ruled_field(&rules);
    assert_eq!(
        field.get_metadata("fix:directions"),
        Some(concat!(
            r#"{"directions":[{"code":"S","patterns":["(?i)(?:^|\\s)tx\\s","say \"out\""]},"#,
            r#"{"code":"R","patterns":["(?i)(?:^|\\s)rx\\s"]}]}"#,
        ))
    );
    let read: Vec<FixDirection> = field
        .as_fix()
        .directions()
        .map(|entry| entry.map(FixDirection::from))
        .collect::<yggdryl::Result<_>>()
        .unwrap();
    assert_eq!(read, rules);
    let entry = field.as_fix().directions().next_ok().unwrap();
    assert_eq!(entry.code(), "S");
    assert_eq!(
        entry.patterns().collect::<Vec<_>>(),
        [r"(?i)(?:^|\\s)tx\\s", r#"say \"out\""#],
        "still escaped as stored"
    );
    assert_eq!(
        entry.parse_patterns().unwrap(),
        [r"(?i)(?:^|\s)tx\s", r#"say "out""#]
    );
    let mut registry = FixRegistry::new();
    registry.insert(field.clone()).unwrap();
    let reading = registry.msgdirection();
    assert_eq!(reading.directions(), rules);
    assert_eq!(reading.read_text("tx 8=FIX.4.4|35=D|"), Some("S"));
    assert_eq!(reading.read_text(r#"say "out" 8=FIX.4.4|35=D|"#), Some("S"));
    assert_eq!(reading.read_text("09:00 rx 8=FIX.4.4|35=D|"), Some("R"));
    assert_eq!(
        field.as_fix_mut().remove_directions().unwrap(),
        Some(rules.to_vec())
    );
    assert_eq!(field.as_fix_mut().remove_directions().unwrap(), None);
    assert_eq!(field.get_metadata("fix:directions"), None);
}

#[test]
fn a_merge_lets_the_incoming_table_win_whole() {
    let stored = ruled_field(&[FixDirection::new("S", ["^TX "])]);
    let stored_text = stored.get_metadata("fix:directions").unwrap().to_owned();
    // Two tables have no order between them, so the incoming one is not
    // folded entry by entry: it replaces the stored one.
    let mut incoming = ruled_field(&[FixDirection::new("R", ["^RX "])]);
    let incoming_text = incoming.get_metadata("fix:directions").unwrap().to_owned();
    incoming.as_fix_mut().merge_with(&stored.as_fix()).unwrap();
    assert_eq!(
        incoming.get_metadata("fix:directions"),
        Some(incoming_text.as_str())
    );
    // The stored one keeps what only it has.
    let mut bare = ruled_field(&[]);
    bare.as_fix_mut().merge_with(&stored.as_fix()).unwrap();
    assert_eq!(
        bare.get_metadata("fix:directions"),
        Some(stored_text.as_str())
    );
}

#[test]
fn every_door_fills_tag_385_from_the_reading_and_the_pin_is_the_batch_doors() {
    let codec = FixCodec::new(super::committed_registry());
    let direction = |line: &[u8]| {
        codec
            .parse_line(line)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .get_by_tag(MSGDIRECTION_TAG_NAME.0)
            .and_then(|value| value.as_str().map(str::to_owned))
    };
    // A verb inside a `Text(58)` value is payload, and the offset the reader
    // already computed is what keeps it out of the reading.
    assert_eq!(
        direction(b"sending >> 8=FIX.4.2|35=D|58=received out of order|10=0|"),
        Some("S".into())
    );
    assert_eq!(
        direction(b"receiving << 8=FIX.4.2|35=D|10=0|"),
        Some("R".into())
    );
    // Both verbs in one prefix is a line no reading can prefer one of, and
    // the line door takes no pin: silence is silence.
    assert_eq!(
        direction(b"sending a received copy >> 8=FIX.4.2|35=D|10=0|"),
        None
    );
    assert_eq!(direction(b"8=FIX.4.2|35=D|10=0|"), None);
    // A frame stating 385 itself is never overridden by the reading.
    assert_eq!(
        direction(b"sending >> 8=FIX.4.2|35=D|385=R|10=0|"),
        Some("R".into())
    );
    // The single-dialect doors fill it too.
    let fixed = codec.parse_fix_line(b"recv 8=FIX.4.2|35=D|10=0|").unwrap();
    assert_eq!(
        fixed.by_tag(MSGDIRECTION_TAG_NAME.0).unwrap().as_str(),
        Some("R")
    );
    let bridge = codec
        .parse_ullink_line(b"sending >> MSGTYPE=D|CLORDID=A")
        .unwrap();
    assert_eq!(
        bridge.by_tag(MSGDIRECTION_TAG_NAME.0).unwrap().as_str(),
        Some("S")
    );
    // The fill is a row child and never an entry: the wire re-emits as it
    // arrived.
    assert!(
        fixed
            .entries()
            .iter()
            .all(|entry| entry.tag() != MSGDIRECTION_TAG_NAME.0)
    );
    assert_eq!(fixed.into_bytes(b'|'), b"8=FIX.4.2|35=D|10=0|");

    // The pin is a code of the set, any spelling, and a spelling outside the
    // set is refused naming the set.
    assert_eq!(codec.direction(), Some("S"));
    let pinned = codec.clone().try_with_direction(Some("Receive")).unwrap();
    assert_eq!(pinned.direction(), Some("R"));
    let unpinned = codec.clone().try_with_direction(None).unwrap();
    assert_eq!(unpinned.direction(), None);
    assert_eq!(
        codec
            .clone()
            .try_with_direction(Some(""))
            .unwrap()
            .direction(),
        None
    );
    let refused = codec
        .clone()
        .try_with_direction(Some("sideways"))
        .map(|_| ())
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("R, S") && refused.contains("sideways"),
        "{refused}"
    );

    // The pin fills silence on the batch door and never overrides a verb.
    let field = DataType::from_fields([DataType::binary().required_field("body")])
        .unwrap()
        .required_field("capture");
    let rows = yggdryl::Scalar::from_sequence([
        yggdryl::Scalar::from_sequence([yggdryl::Scalar::from(b"8=FIX.4.2|35=D|10=0|".to_vec())]),
        yggdryl::Scalar::from_sequence([yggdryl::Scalar::from(
            b"recv 8=FIX.4.2|35=D|10=0|".to_vec(),
        )]),
    ]);
    let batch_directions = |codec: &FixCodec| -> Vec<Option<String>> {
        let source = yggdryl::arrow::batch_from_value(&field, &rows).unwrap();
        let reader = codec
            .parse_text_arrow_reader(yggdryl::arrow::batch_reader(source.schema(), [source]))
            .unwrap();
        let schema = yggdryl::Field::from_arrow_schema("row", reader.schema().as_ref()).unwrap();
        let at = yggdryl::fix_column_of(&schema, MSGDIRECTION_TAG_NAME.0).unwrap();
        reader
            .map(|batch| batch.unwrap())
            .flat_map(|batch| {
                let value = yggdryl::arrow::batch_to_value(&batch).unwrap();
                value
                    .as_sequence()
                    .unwrap()
                    .iter()
                    .map(|row| row.as_sequence().unwrap()[at].as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    assert_eq!(
        batch_directions(&codec),
        [Some("S".into()), Some("R".into())]
    );
    assert_eq!(
        batch_directions(&pinned),
        [Some("R".into()), Some("R".into())]
    );
    assert_eq!(batch_directions(&unpinned), [None, Some("R".into())]);
    let _ = Arc::clone(codec.registry());
}
