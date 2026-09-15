//! What the bridge's own row header says about a message, as columns.

use std::sync::Arc;

use yggdryl::holder::Buffer;
use yggdryl::media::RecordOptions;
use yggdryl::media::text::TextOptions;
use yggdryl::{FixRegistry, IOMedia, Scalar, Timezone, Url};

fn registry() -> Arc<FixRegistry> {
    super::committed_registry()
}

/// One bridge line, framed exactly as the dataset suite frames the log.
fn lined(line: &str) -> yggdryl::FixMsg {
    let registry = registry();
    let codec = super::fixed_codec(registry);
    let mut options = TextOptions::new()
        .try_with_rowheader(yggdryl::ULBRIDGE_ROWHEADER)
        .expect("the bridge's row header compiles")
        .with_timezone(Timezone::UTC);
    options.start_rownum = Some(1);
    options.parse_mimetype = true;
    let mut bytes = line.as_bytes().to_vec();
    bytes.push(b'\n');
    let source = Buffer::from_bytes(bytes).with_media_type(
        Url::from_str("file:///bridge.log")
            .expect("a URL")
            .media_type(),
    );
    let reader = source
        .read_arrow_reader(&RecordOptions::from(options))
        .expect("a reader");
    let parsed = codec.parse_text_arrow_reader(reader).expect("a parse");
    codec
        .messages(parsed)
        .next()
        .expect("one message")
        .expect("parses")
}

const RECEIVING: &str = "2026-08-14 06:46:30.947 [402-e7254b20:9f015ed023:935] [ULMSG_BROKER_TO_DMZ] (DEBUG) Receiving : 8=FIX.4.2|9=55|35=0|49=ULB_DMZ|56=ULB_BRK|34=935|52=20260814-04:46:30.967|10=186|";

#[test]
fn the_brackets_three_parts_fill_the_fields_they_are_named_for() {
    let held = lined(RECEIVING);
    let text = |tag: i32| {
        held.get_by_tag(tag)
            .filter(|held| !held.is_null())
            .and_then(Scalar::as_str)
            .map(str::to_owned)
    };
    // Named for the field, so the registry's one namespace lands them and
    // nothing translates in between.
    assert_eq!(
        text(yggdryl::BRIDGESESSIONID_TAG_NAME.0).as_deref(),
        Some("e7254b20")
    );
    assert_eq!(
        text(yggdryl::MSGCTXID_TAG_NAME.0).as_deref(),
        Some("9f015ed023")
    );
    // The bracket's sequence number is the message's own, which it states too.
    assert_eq!(
        held.get_by_tag(34).and_then(Scalar::as_i128),
        Some(935),
        "MsgSeqNum(34)"
    );
}

#[test]
fn one_message_in_its_session_and_one_occurrence_of_it() {
    let held = lined(RECEIVING);
    let schema = yggdryl::fix_schema(&registry(), "fix").expect("a schema");
    let row = held.into_row(&schema).expect("a row");
    let values = row.as_sequence().expect("a row");
    let at = |name: &str| &values[schema.index_of(name).unwrap_or_else(|| panic!("{name}"))];

    // Concatenated rather than hashed, so a reader grepping the log for the
    // bracket finds the same string the column holds.
    assert_eq!(
        at("sessionmsgid").as_str(),
        Some("e7254b20:9f015ed023"),
        "the session and the message context"
    );
    assert_eq!(
        at("sessionmsgseqid").as_str(),
        Some("e7254b20:9f015ed023:935"),
        "and the occurrence"
    );
}

#[test]
fn a_line_with_no_bracket_names_no_message_rather_than_half_of_one() {
    // A name with a hole in it looks like a name, so two messages missing
    // different parts would join to each other.
    let held = lined(
        "2026-08-14 06:46:22.255 [23] [Jolokia] (DEBUG) Receiving : 8=FIX.4.4|35=D|11=A1|10=0|",
    );
    let schema = yggdryl::fix_schema(&registry(), "fix").expect("a schema");
    let row = held.into_row(&schema).expect("a row");
    let values = row.as_sequence().expect("a row");
    for name in ["sessionmsgid", "sessionmsgseqid", "bridgesessionid"] {
        assert!(
            values[schema.index_of(name).expect(name)].is_null(),
            "{name} is null where the bracket named nothing"
        );
    }
}

#[test]
fn the_two_legs_of_one_routed_message_meet_under_the_bracket_that_carried_them() {
    // The bridge receives a message and sends it on under one message
    // context, and neither leg names an instrument the dictionary knows. The
    // bracket is what they share, so it is what joins them - which is the
    // whole point of naming the joined identifiers after it.
    let receiving = lined(RECEIVING);
    let sending = lined(
        "2026-08-14 06:46:30.948 [403-e7254b20:9f015ed023:936] [ULMSG_DMZ_TO_BROKER] (DEBUG) Sending : 8=FIX.4.2|9=55|35=0|49=ULB_BRK|56=ULB_DMZ|34=936|52=20260814-04:46:30.968|10=187|",
    );
    let mut life = yggdryl::FixLifecycle::new(registry());
    let first = life.fill(receiving).expect("the first leg fills");
    let second = life.fill(sending).expect("the second leg fills");
    let code = |held: &yggdryl::FixMsg| {
        held.by_tag(yggdryl::CODE_TAG_NAME.0)
            .expect("a code")
            .as_str()
            .expect("text")
            .to_owned()
    };
    assert!(
        code(&first).ends_with("/e7254b20:9f015ed023"),
        "{}",
        code(&first)
    );
    assert_eq!(code(&first), code(&second), "one chain, not two");
    assert_eq!(life.alive(), 1);
    // And the second leg carries the first: a chain that joined is a chain
    // whose previous pair points back.
    assert_eq!(
        second.by_tag(yggdryl::PREVMSGHASH_TAG_NAME.0).unwrap(),
        first.msghash()
    );
}

#[test]
fn a_sequence_number_alone_never_joins_two_message_contexts() {
    // `sessionmsgseqid` is one occurrence's name, so it matches exactly and
    // joins nothing: two messages in one session under different contexts
    // are two chains however their sequence numbers line up.
    let mut life = yggdryl::FixLifecycle::new(registry());
    for line in [
        "2026-08-14 06:46:30.947 [402-e7254b20:9f015ed023:935] [ULMSG_BROKER_TO_DMZ] (DEBUG) Receiving : 8=FIX.4.2|9=55|35=0|49=ULB_DMZ|56=ULB_BRK|34=935|52=20260814-04:46:30.967|10=186|",
        "2026-08-14 06:46:30.949 [402-e7254b20:9f015ed024:935] [ULMSG_BROKER_TO_DMZ] (DEBUG) Receiving : 8=FIX.4.2|9=55|35=0|49=ULB_DMZ|56=ULB_BRK|34=935|52=20260814-04:46:30.969|10=186|",
    ] {
        life.fill(lined(line)).expect("it fills");
    }
    assert_eq!(life.alive(), 2);
}
