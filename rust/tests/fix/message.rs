//! The message holder's setters, and the row read back into a message.

use super::SoleMessage;

use std::sync::Arc;

use yggdryl::fix::FIXENTRIES_COLUMN;

use yggdryl::graph::{Element, Event};
use yggdryl::text::{TextBytes, TextLine};
use yggdryl::{
    DataType, Field, FixCodec, FixEntry, FixMsg, FixRegistry, Scalar, StructureType, fix_schema,
    fix_schema_carrying,
};

fn reader() -> (Arc<FixRegistry>, FixCodec) {
    let registry = super::committed_registry();
    let reader = super::fixed_codec(Arc::clone(&registry));
    (registry, reader)
}

const ORDER: &[u8] = b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|VenueThing=7|9999=x|10=0|";

/// Every tag a message's children carry, beside the value each holds.
fn stated(message: &FixMsg) -> Vec<(i32, Scalar)> {
    message
        .as_field()
        .fields()
        .iter()
        .filter_map(|child| child.as_fix().tag().ok().flatten())
        .map(|tag| (tag, message.by_tag(tag).expect("an indexed tag")))
        .collect()
}

#[test]
fn a_set_value_is_typed_by_the_registry_field_and_appended_when_absent() {
    let (registry, reader) = reader();
    let mut message = reader.sole_line(ORDER).unwrap();
    let before = message.as_field().fields().len();
    let declared = registry.get_field_by_tag(1).expect("Account");

    message.set(1, Scalar::from("A-1")).unwrap();

    let fields = message.as_field().fields();
    assert_eq!(fields.len(), before + 1, "appended, not inserted");
    let child = fields.last().unwrap();
    assert_eq!(child.name(), declared.name(), "the dictionary's spelling");
    assert_eq!(child.dtype(), declared.dtype(), "the dictionary's type");
    assert_eq!(child.as_fix().tag().unwrap(), Some(1));
    assert!(!child.is_nullable(), "a stated value is non-null");
    assert_eq!(message.by_tag(1).unwrap().as_str(), Some("A-1"));
    assert_eq!(
        message.by_name("Account").unwrap().as_str(),
        Some("A-1"),
        "reached by name through the registry"
    );

    // A typed fact lands on its holder and never in the row.
    message.set(34, Scalar::from(7_i64)).unwrap();
    assert_eq!(message.as_field().fields().len(), before + 1);
    assert_eq!(message.header().msgseqnum(), Some(7));
    assert_eq!(message.by_tag(34).unwrap().as_u64(), Some(7));
}

#[test]
fn a_set_value_replaces_an_existing_child_in_place_and_keeps_the_tag_index() {
    let (_, reader) = reader();
    let mut message = reader.sole_line(ORDER).unwrap();
    let before = stated(&message);
    let at = message
        .as_field()
        .index_of("symbol")
        .expect("the symbol child");

    message.set(55, Scalar::from("MSFT")).unwrap();
    message.set("Side", Scalar::from("2")).unwrap();

    assert_eq!(
        message.as_field().index_of("symbol"),
        Some(at),
        "same position"
    );
    assert_eq!(
        message.as_field().fields().len(),
        before.len() + 2,
        "two unknown children beside the tagged ones"
    );
    assert_eq!(message.by_tag(55).unwrap(), Scalar::from("MSFT"));
    assert_eq!(message.by_tag(54).unwrap().as_str(), Some("SELL"));
    // Content identity changes; every other tag still reaches its previous value.
    for (tag, value) in before {
        if tag == 55 || tag == 54 {
            continue;
        }
        if tag == yggdryl::CURRHASHCODE_TAG_NAME.0 {
            assert_ne!(message.by_tag(tag).unwrap(), value);
            continue;
        }
        assert_eq!(message.by_tag(tag).unwrap(), value, "tag {tag}");
    }
}

/// The entries are the row read as a tree, so a write is what they and the
/// wire re-emit: a replaced child spells its new value in place, an
/// appended one closes the wire, and a typed fact removed from its holder
/// leaves the wire.
#[test]
fn a_set_is_what_the_entries_and_the_wire_re_emit() {
    let (_, reader) = reader();
    let parsed = reader.sole_line(ORDER).unwrap();
    let mut message = parsed.clone();
    message.set(55, Scalar::from("MSFT")).unwrap();
    message.set(1, Scalar::from("A-1")).unwrap();
    assert_eq!(
        message
            .remove(54)
            .unwrap()
            .as_ref()
            .and_then(Scalar::as_str),
        Some("BUY")
    );
    let symbol = message
        .entries()
        .iter()
        .find(|entry| entry.tag() == 55)
        .expect("the symbol entry");
    assert_eq!(symbol.name(), "symbol");
    assert_eq!(symbol.value(), Some("MSFT"));
    assert_eq!(
        message
            .entries()
            .last()
            .map(|entry| (entry.tag(), entry.value())),
        Some((1, Some("A-1")))
    );
    assert_eq!(
        message.into_bytes(b'|'),
        b"8=FIX.4.4|35=D|11=A1|55=MSFT|venuething=7|9999=x|59=0|1=A-1|10=0|"
    );
    assert_ne!(message.digest(), parsed.digest());
    assert_ne!(message.entries(), parsed.entries());
}

#[test]
fn a_null_is_stored_as_a_stated_null() {
    let (_, reader) = reader();
    let mut message = reader.sole_line(ORDER).unwrap();
    message.set(55, Scalar::Null).unwrap();
    let at = message.as_field().index_of("symbol").unwrap();
    assert!(message.as_field().fields()[at].is_nullable());
    assert_eq!(message.get_by_tag(55), Some(Scalar::Null));
}

#[test]
fn an_unknown_name_is_refused_and_the_message_stands() {
    let (_, reader) = reader();
    let mut message = reader.sole_line(ORDER).unwrap();
    let before = message.clone();
    let refused = message.set("nosuchfield", Scalar::from("y")).unwrap_err();
    assert!(refused.to_string().contains("nosuchfield"), "{refused}");
    assert_eq!(message, before);
    // A value the field refuses is refused the same way.
    assert!(message.set(38, Scalar::from("not a number")).is_err());
    assert_eq!(message, before);
}

#[test]
fn an_unknown_name_still_reaches_the_child_spelled_that_way() {
    let (_, reader) = reader();
    let mut message = reader.sole_line(ORDER).unwrap();
    let at = message
        .as_field()
        .index_of("venuething")
        .expect("the venue's own child");
    message.set("Venue_Thing", Scalar::from("8")).unwrap();
    let child = &message.as_field().fields()[at];
    assert_eq!(child.name(), "venuething", "the child keeps its own field");
    assert_eq!(child.dtype(), &DataType::utf8());
    assert_eq!(message.by_name("venuething").unwrap(), Scalar::from("8"));
}

#[test]
fn a_bare_unknown_tag_is_appended_under_its_decimal_spelling() {
    let (_, reader) = reader();
    let mut message = reader.sole_line(ORDER).unwrap();
    message.set(7777, Scalar::from("custom")).unwrap();
    let child = message.as_field().fields().last().unwrap();
    assert_eq!(child.name(), "7777");
    assert_eq!(child.dtype(), &DataType::utf8());
    assert!(child.is_nullable());
    assert_eq!(message.by_tag(7777).unwrap(), Scalar::from("custom"));
    // A second write reaches the same child rather than a second one.
    message.set(7777, Scalar::from("again")).unwrap();
    assert_eq!(message.by_tag(7777).unwrap(), Scalar::from("again"));
    assert_eq!(
        message.as_field().index_of("7777").map(|at| at + 1),
        Some(message.as_field().fields().len())
    );
    // The one the line already carried is replaced where it stands.
    message.set(9999, Scalar::from("y")).unwrap();
    assert_eq!(message.by_tag(9999).unwrap(), Scalar::from("y"));
}

#[test]
fn several_values_land_with_one_rebuild_as_the_same_writes_would_one_at_a_time() {
    let (_, reader) = reader();
    let mut many = reader.sole_line(ORDER).unwrap();
    let mut one = many.clone();
    let writes = [
        (55, Scalar::from("MSFT")),
        (1, Scalar::from("A-1")),
        (7777, Scalar::from("first")),
        (7777, Scalar::from("second")),
        (1, Scalar::from("A-2")),
    ];
    for (tag, value) in writes.clone() {
        one.set(tag, value).unwrap();
    }
    many.set_many(writes).unwrap();
    assert_eq!(many, one);
    assert_eq!(many.by_tag(1).unwrap().as_str(), Some("A-2"));
    assert_eq!(many.by_tag(7777).unwrap(), Scalar::from("second"));

    // One refused write refuses them all, and the message stands.
    let before = many.clone();
    assert!(
        many.set_many([(55, Scalar::from("X")), (38, Scalar::from("not a number"))])
            .is_err()
    );
    assert_eq!(many, before);
    many.set_many(Vec::<(i32, Scalar)>::new()).unwrap();
    assert_eq!(many, before);
}

#[test]
fn the_consuming_twin_answers_what_the_setter_leaves() {
    let (_, reader) = reader();
    let mut set = reader.sole_line(ORDER).unwrap();
    let with = set.clone().with_value(55, Scalar::from("MSFT")).unwrap();
    set.set(55, Scalar::from("MSFT")).unwrap();
    assert_eq!(with, set);
}

#[test]
fn remove_answers_the_value_and_the_other_tags_still_reach_their_children() {
    let (_, reader) = reader();
    let mut message = reader.sole_line(ORDER).unwrap();
    let before = stated(&message);
    let count = message.as_field().fields().len();

    assert_eq!(message.remove(55).unwrap(), Some(Scalar::from("AAPL")));
    assert_eq!(message.get_by_tag(55), None);
    assert_eq!(message.as_field().fields().len(), count - 1);
    for (tag, value) in &before {
        if *tag == 55 {
            continue;
        }
        if *tag == yggdryl::CURRHASHCODE_TAG_NAME.0 {
            assert_ne!(message.by_tag(*tag).unwrap(), *value);
            continue;
        }
        assert_eq!(message.by_tag(*tag).unwrap(), *value, "tag {tag}");
    }
    // By name, by decimal, and a miss.
    assert_eq!(
        message.remove("VenueThing").unwrap(),
        Some(Scalar::from("7"))
    );
    assert_eq!(message.remove(9999).unwrap(), Some(Scalar::from("x")));
    assert_eq!(message.remove(55).unwrap(), None);
    assert_eq!(message.remove("nosuchfield").unwrap(), None);
    assert_eq!(message.as_field().fields().len(), count - 3);
    // The wire follows the row: what was removed is gone from it, and what
    // the dictionary derived for the order stays.
    assert_eq!(
        message.into_bytes(b'|'),
        b"8=FIX.4.4|35=D|11=A1|54=1|59=0|10=0|"
    );
}

#[test]
fn a_row_reads_back_into_the_message_that_made_it() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let parsed = reader.sole_line(ORDER).unwrap();
    let row = parsed.into_row(&schema).unwrap();

    let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();

    // The root a row reads back is the content row, not the fixed schema:
    // the typed facts are the holders' and never children of it.
    assert_eq!(held.as_field().name(), schema.name());
    assert_eq!(
        held.as_field()
            .fields()
            .iter()
            .map(Field::name)
            .collect::<Vec<_>>(),
        parsed
            .as_field()
            .fields()
            .iter()
            .map(Field::name)
            .collect::<Vec<_>>()
    );
    assert_eq!(held.entries(), parsed.entries());
    assert_eq!(held.digest(), parsed.digest());
    for tag in [8, 35, 11, 55, 54] {
        assert_eq!(
            held.by_tag(tag).unwrap(),
            parsed.by_tag(tag).unwrap(),
            "tag {tag}"
        );
    }
    // And it makes the row it came from, whole.
    assert_eq!(held.into_row(&schema).unwrap(), row);
    // The same message on the wire, and the code it digests to. The
    // sending time is the one fact a row cannot give back: the line stated
    // none, so it is intake's stand-in rather than something the message
    // said, the row carries the instant under `currunix` alone, and the message
    // a row makes stands one in again.
    assert!(!parsed.header().stated_sendingtime());
    assert!(!held.header().stated_sendingtime());
    assert_eq!(held.header().beginstring(), parsed.header().beginstring());
    assert_eq!(held.header().msgtype(), parsed.header().msgtype());
    assert_eq!(held.header().msgseqnum(), parsed.header().msgseqnum());
    assert_eq!(held.get_currunix(), parsed.get_currunix());
    assert_eq!(held.into_bytes(b'|'), parsed.into_bytes(b'|'));
    assert_eq!(held.get_currhashcode(), parsed.get_currhashcode());
}

#[test]
fn a_data_field_that_is_not_text_is_held_as_the_decode_a_row_can_hold() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    // The one line no row can say what arrived on: `RawData(96)` carrying
    // bytes no text holds. Its length is stated, because that is what a data
    // field is for.
    let line: &[u8] = b"8=FIX.4.4\x0135=D\x0195=4\x0196=\xff\xfe A\x0110=000\x01";
    let parsed = reader.parse_fix_line(line).unwrap();

    // The row holds the bytes, typed as the data field is, and the entry
    // spells them as the text a column can read: the decode, so the wire
    // re-emits that rather than the bytes. That is the whole of what a row
    // cannot carry.
    assert_eq!(
        parsed.by_tag(96).unwrap(),
        Scalar::from(b"\xff\xfe A".to_vec())
    );
    let arrived = parsed
        .entries()
        .iter()
        .find(|entry| entry.tag() == 96)
        .expect("the data field");
    assert_eq!(arrived.value(), Some("\u{FFFD}\u{FFFD} A"));
    assert_ne!(parsed.into_bytes(1), line);

    // And the row round trip holds what the message holds.
    let row = parsed.into_row(&schema).unwrap();
    let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(held.entries(), parsed.entries());
    assert_eq!(held.digest(), parsed.digest());
    assert_eq!(held.into_row(&schema).unwrap(), row);
}

#[test]
fn the_same_line_read_as_text_is_the_decode_of_the_wire() {
    // A text line is text before the codec reads it. The bytes
    // that were not UTF-8 read as the Windows-1252 characters they are, so
    // the stated length - a count of wire bytes - reaches no boundary the
    // frame stated and is not honoured: the value stays what the frame cut,
    // and the message re-emits the line's text rather than the wire, beside
    // what the dictionary derived for the order. That the line was decoded
    // is the line's fact, and the line counts it.
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let wire: &[u8] = b"8=FIX.4.4\x0135=D\x0195=4\x0196=\xff\xfe A\x0110=000\x01";
    let line = TextLine::from_bytes(0, TextBytes::from_bytes(wire).unwrap()).unwrap();
    assert_eq!(line.decoded_byte_size(), 2);
    assert_eq!(
        line.body(),
        "8=FIX.4.4\u{1}35=D\u{1}95=4\u{1}96=\u{ff}\u{fe} A\u{1}10=000\u{1}"
    );

    let parsed = reader
        .parse_text_line(&line)
        .unwrap()
        .next()
        .expect("one message")
        .unwrap();
    let arrived = parsed
        .entries()
        .iter()
        .find(|entry| entry.tag() == 96)
        .expect("the data field");
    assert_eq!(arrived.value(), Some("\u{ff}\u{fe} A"));
    // `TimeInForce` is the dictionary's derivation for an order and an
    // ordinary child of the row, so it re-emits where the row carries it -
    // appended behind the content the line stated, in front of the trailer.
    assert_eq!(
        parsed.into_bytes(1),
        line.body()
            .replace("\u{1}10=", "\u{1}59=0\u{1}10=")
            .as_bytes()
    );
    assert_ne!(parsed.into_bytes(1), wire);

    // And the row round-trips whole, which the byte door's line cannot.
    let row = parsed.into_row(&schema).unwrap();
    let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(held.entries(), parsed.entries());
    assert_eq!(held.into_row(&schema).unwrap(), row);
}

#[test]
fn a_captures_own_columns_never_reach_the_message() {
    let (registry, reader) = reader();
    // Nullable, because a message states none of them - ever.
    let capture = StructureType::from_fields([
        DataType::utf8().nullable_field("url"),
        DataType::Int64.nullable_field("rownum"),
        DataType::binary().nullable_field("body"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("line");
    let schema = fix_schema_carrying(&capture, &fix_schema(&registry, "fix").unwrap()).unwrap();
    let parsed = reader.sole_line(ORDER).unwrap();
    let at = |name: &str| schema.index_of(name).unwrap();

    // A parsed message has no capture columns: they are null in its row, and
    // a row read back off that one is the message it came from.
    let row = parsed.into_row(&schema).unwrap();
    for carrier in ["url", "rownum", "body", "sourceurl"] {
        assert!(row.get(at(carrier)).unwrap().is_null(), "{carrier}");
    }
    let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(held.entries(), parsed.entries());
    assert_eq!(held.into_row(&schema).unwrap(), row);

    // A row a reader put its own statements in - the one the crate tags
    // among them - says what the *line* was read from. Read back, the
    // message holds none of it: no child, no entry, nothing on the wire, and
    // nothing to answer by name or by tag.
    let mut columns = row.as_sequence().expect("a row").to_vec();
    columns[at("url")] = Scalar::from("file:///capture.log");
    columns[at("rownum")] = Scalar::from(42_i64);
    columns[at("body")] = Scalar::from(ORDER.to_vec());
    columns[at("sourceurl")] = Scalar::from(yggdryl::Url::from_str("file:///capture.log").unwrap());
    let carried = Scalar::from_sequence(columns);
    let again = FixMsg::from_row(Arc::clone(&registry), &schema, &carried).unwrap();
    assert_eq!(again.entries(), held.entries());
    assert_eq!(again.into_bytes(b'|'), held.into_bytes(b'|'));
    assert_eq!(again.by_tag(55).unwrap().as_str(), Some("AAPL"));
    for carrier in ["url", "rownum", "body", "sourceurl"] {
        assert!(again.as_field().index_of(carrier).is_none(), "{carrier}");
        assert!(again.get_by_name(carrier).is_none(), "{carrier}");
        assert!(
            !again.entries().iter().any(|entry| entry.name() == carrier),
            "{carrier}"
        );
        let wire = String::from_utf8_lossy(&again.into_bytes(b'|')).into_owned();
        assert!(!wire.contains(&format!("{carrier}=")), "{wire}");
    }
    assert!(again.capture().msgpluginid().is_none());
    assert!(
        again.get_by_tag(yggdryl::SOURCEURL_TAG_NAME.0).is_none(),
        "the object a line came out of is not a fact of the message"
    );

    // So a message alone cannot put them back: the row it writes states them
    // null, and only a reader holding the batch they arrived in can.
    let written = again.into_row(&schema).unwrap();
    for carrier in ["url", "rownum", "body", "sourceurl"] {
        assert!(written.get(at(carrier)).unwrap().is_null(), "{carrier}");
    }
    assert_eq!(written, row, "every other column is the message's own");
}

/// Writing one of the capture's own columns onto a message is refused.
///
/// Silence would leave a caller believing the message states where its line
/// came from, and a row child would put `sourceurl=` on the wire.
#[test]
fn writing_a_captures_own_column_onto_a_message_is_refused() {
    let (_registry, reader) = reader();
    let mut parsed = reader.sole_line(ORDER).unwrap();
    let (tag, name) = yggdryl::SOURCEURL_TAG_NAME;
    let refusal = parsed
        .set(tag, Scalar::from("file:///capture.log"))
        .unwrap_err()
        .to_string();
    assert!(refusal.contains(name), "{refusal}");
    assert!(refusal.contains("capture"), "{refusal}");
    // Removing it reaches nothing rather than refusing: there was never a
    // fact there to clear.
    assert_eq!(parsed.remove(tag).unwrap(), None);
    // Refused and unchanged: the row grew nothing and the wire is the line.
    assert_eq!(
        parsed.into_bytes(b'|'),
        reader.sole_line(ORDER).unwrap().into_bytes(b'|')
    );
}

#[test]
fn a_row_without_the_entries_group_has_no_entries() {
    let (registry, reader) = reader();
    let wide = fix_schema(&registry, "fix").unwrap();
    // The group and the counter that counts it are dropped together: a count
    // of a record the row does not carry is a number about nothing.
    let columns: Vec<Field> = wide
        .fields()
        .iter()
        .filter(|column| {
            column.name() != FIXENTRIES_COLUMN && column.name() != yggdryl::NOFIXENTRIES_TAG_NAME.1
        })
        .cloned()
        .collect();
    let narrow = StructureType::from_fields(columns)
        .map(DataType::from)
        .unwrap()
        .required_field("fix");
    let parsed = reader.sole_line(ORDER).unwrap();
    let row = parsed.into_row(&narrow).unwrap();

    let held = FixMsg::from_row(Arc::clone(&registry), &narrow, &row).unwrap();
    assert!(held.entries().is_empty());
    // The typed facts are the holders' and still on the wire - the frame
    // and the identifier the message lifted - and the content is gone with
    // the record, the side and the symbol among it.
    let wire = String::from_utf8(held.into_bytes(b'|')).unwrap();
    assert!(wire.starts_with("8=FIX.4.4|35=D|"), "{wire}");
    assert!(wire.contains("|11=A1|"), "{wire}");
    assert!(!wire.contains("54=") && !wire.contains("55="), "{wire}");
    // A row that dropped the record cannot give the content back, so the
    // row it makes is the one a message of typed facts alone fills - and
    // that row is its own fixed point.
    let again = held.into_row(&narrow).unwrap();
    let twice = FixMsg::from_row(Arc::clone(&registry), &narrow, &again).unwrap();
    assert_eq!(twice.into_row(&narrow).unwrap(), again);
}

#[test]
fn entries_folded_past_the_materialization_depth_read_back_whole() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    // A trade report's sides, each side's parties, each party's
    // sub-identifiers: four counters deep, one past the three levels a row
    // materializes, so the deepest level folds into the JSON leaf.
    const DEEP: &[u8] =
        b"8=FIX.4.4|35=AE|571=T1|552=1|54=1|453=1|448=P1|452=1|802=1|523=S1|803=1|10=0|";
    let parsed = reader.sole_line(DEEP).unwrap();
    fn depth(entries: &[FixEntry]) -> usize {
        entries
            .iter()
            .map(|entry| 1 + depth(entry.entries()))
            .max()
            .unwrap_or(0)
    }
    assert!(
        depth(parsed.entries()) >= 4,
        "the fixture nests past the materialized depth: {:?}",
        parsed.entries()
    );
    let row = parsed.into_row(&schema).unwrap();
    // The row holds a folded leaf somewhere under the entries column.
    fn leaf(entry: &[Scalar]) -> bool {
        match entry.get(3) {
            Some(tail) if tail.as_str().is_some_and(|text| !text.is_empty()) => true,
            Some(tail) => tail
                .as_sequence()
                .unwrap_or_default()
                .iter()
                .filter_map(Scalar::as_sequence)
                .any(leaf),
            None => false,
        }
    }
    let entries = row
        .get(schema.index_of(FIXENTRIES_COLUMN).unwrap())
        .unwrap();
    assert!(
        entries
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(Scalar::as_sequence)
            .any(leaf)
    );

    // The folded leaf decodes back into the message: every pair the four
    // levels state is stated again, at the depth it was folded from. The
    // shape around them - a group whose occurrences nest a second group -
    // is the one [`FixMsg::from_row`] names as not rebuilding entry for
    // entry, so what is pinned here is that nothing folded is lost.
    fn pairs(entries: &[FixEntry], out: &mut Vec<(i32, String)>) {
        for entry in entries {
            if entry.entries().is_empty() {
                if let Some(value) = entry.value() {
                    out.push((entry.tag(), value.to_owned()));
                }
            } else {
                pairs(entry.entries(), out);
            }
        }
    }
    let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    let (mut stated, mut rebuilt) = (Vec::new(), Vec::new());
    pairs(parsed.entries(), &mut stated);
    pairs(held.entries(), &mut rebuilt);
    assert_eq!(rebuilt, stated);
    assert!(
        stated.contains(&(523, "S1".to_owned())),
        "the folded level's own pair: {stated:?}"
    );
}

/// A group nested in an occurrence is one entry under its counter, as a
/// group at the root is: the counter beside it states nothing the entry
/// does not, so the wire carries each counter once.
#[test]
fn a_nested_group_re_emits_its_counter_once() {
    let (_, reader) = reader();
    const DEEP: &[u8] =
        b"8=FIX.4.4|35=AE|571=T1|552=1|54=1|453=1|448=P1|452=1|802=1|523=S1|803=1|10=0|";
    let parsed = reader.sole_line(DEEP).unwrap();
    let side = parsed
        .entries()
        .iter()
        .find(|entry| entry.tag() == 552)
        .expect("the sides")
        .entries()
        .first()
        .expect("one side");
    assert_eq!(
        side.entries()
            .iter()
            .filter(|member| member.tag() == 453)
            .count(),
        1,
        "{:?}",
        side.entries()
    );
    assert_eq!(parsed.into_bytes(b'|'), DEEP);
}

#[test]
fn an_entries_column_holding_no_entry_is_refused() {
    let (registry, _) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let mut values: Vec<Scalar> = schema
        .fields()
        .iter()
        .map(|column| column.default_value().unwrap())
        .collect();
    values[schema.index_of(FIXENTRIES_COLUMN).unwrap()] =
        Scalar::from_sequence([Scalar::from_sequence([
            Scalar::from(35_i32),
            Scalar::from("msgtype"),
            Scalar::from("D"),
            Scalar::from(b"not json" as &[u8]),
        ])]);
    let row = Scalar::from_sequence(values);
    let refused = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap_err();
    assert!(!refused.to_string().is_empty());
}

#[test]
fn folded_arrivals_refuse_malformed_shapes_instead_of_dropping_them() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let message = reader.sole_line(ORDER).unwrap();
    let original = message.into_row(&schema).unwrap();
    let at = schema.index_of(FIXENTRIES_COLUMN).unwrap();
    let row = |leaf: Scalar| {
        let mut tail = leaf;
        for _ in 0..3 {
            // No value of its own: an entry that heads others is what the
            // materialized levels above a fold hold.
            tail = Scalar::from_sequence([Scalar::from_sequence([
                Scalar::from(0_i32),
                Scalar::from("raw"),
                Scalar::Null,
                tail,
            ])]);
        }
        let mut values = original.as_sequence().unwrap().to_vec();
        values[at] = tail;
        Scalar::from_sequence(values)
    };
    for leaf in [
        // A leaf whose JSON is `null` folded nothing and says so with an
        // absent leaf, not with a document that decodes to nothing.
        "null",
        "true",
        "{}",
        "[1]",
        // Three members is one short and five is one over: an entry is
        // exactly the four the materialized levels hold.
        "[[0,\"name\",\"value\"]]",
        "[[0,\"name\",\"value\",[],0]]",
        "[[-1,\"name\",\"value\",[]]]",
        "[[2147483648,\"name\",\"value\",[]]]",
        "[[\"0\",\"name\",\"value\",[]]]",
        "[[0,1,\"value\",[]]]",
        "[[0,null,\"value\",[]]]",
        "[[0,\"name\",false,[]]]",
        "[[0,\"name\",\"value\",true]]",
        "[[0,\"name\",\"value\",[false]]]",
    ] {
        let error =
            FixMsg::from_row(Arc::clone(&registry), &schema, &row(Scalar::from(leaf))).unwrap_err();
        assert!(
            matches!(&error, yggdryl::Error::InvalidRecord { path, .. }
            if path.starts_with("$.fixentries[0].fixentries[0].fixentries[0].fixentries")),
            "{leaf}: {error}"
        );
    }
    // A null tag is the tag of a key that named no field, and a null value
    // is an entry that only heads others; the name is never null, because
    // an entry is reached by it.
    assert!(
        FixMsg::from_row(
            Arc::clone(&registry),
            &schema,
            &row(Scalar::from("[[null,\"\",null,[]]]")),
        )
        .is_ok()
    );
    // A folded pair the dictionary knows is read rather than refused; what
    // it reads back as is
    // `entries_folded_past_the_materialization_depth_read_back_whole`'s.
    assert!(
        FixMsg::from_row(
            Arc::clone(&registry),
            &schema,
            &row(Scalar::from("[[448,\"partyid\",\"P1\",[]]]")),
        )
        .is_ok()
    );
    assert!(FixMsg::from_row(Arc::clone(&registry), &schema, &row(Scalar::from("[]"))).is_ok());
    // An empty leaf and an absent one both mean nothing was folded - which is
    // what the nullable leaf buys over the empty-string-only spelling.
    assert!(FixMsg::from_row(Arc::clone(&registry), &schema, &row(Scalar::from(""))).is_ok());
    assert!(FixMsg::from_row(Arc::clone(&registry), &schema, &row(Scalar::Null)).is_ok());
    let undecodable = FixMsg::from_row(
        Arc::clone(&registry),
        &schema,
        &row(Scalar::from("not json")),
    )
    .unwrap_err();
    assert!(
        matches!(&undecodable, yggdryl::Error::InvalidRecord { path, .. }
            if path == "$.fixentries[0].fixentries[0].fixentries[0].fixentries"),
        "{undecodable}"
    );
    assert_eq!(message.into_row(&schema).unwrap(), original);
}
