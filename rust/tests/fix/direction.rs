//! Which way a message moved: FIX's tag 385, read by the registry (decision 14).

use std::sync::Arc;

use yggdryl::fix::MsgDirection;
use yggdryl::{DataType, FixCode, FixCodec, FixRegistry, MSGDIRECTION_TAG_NAME};

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

    // These are the shapes that must read nothing.
    for line in [
        // English that merely contains the letters.
        "sending in session 3",
        "received out of order",
        // A route endpoint and a session name, where a word boundary alone
        // would have been enough to get it wrong.
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
