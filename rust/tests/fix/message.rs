//! The message holder's setters, and the row read back into a message.

use super::OneMessage;

use std::sync::Arc;

use yggdryl::fix::{ENTRIES_COLUMN, UNMAPPED_COLUMN};

use yggdryl::{
    DataType, Field, FixCodec, FixEntry, FixMsg, FixRegistry, Scalar, fix_schema,
    fix_schema_carrying,
};

fn reader() -> (Arc<FixRegistry>, FixCodec) {
    let registry = super::committed_registry();
    let reader = FixCodec::new(Arc::clone(&registry));
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
        .map(|tag| (tag, message.by_tag(tag).expect("an indexed tag").clone()))
        .collect()
}

#[test]
fn a_set_value_is_typed_by_the_registry_field_and_appended_when_absent() {
    let (registry, reader) = reader();
    let mut message = reader.one_line(ORDER, false).unwrap();
    let before = message.as_field().fields().len();
    let declared = registry.get_field_by_tag(34).expect("MsgSeqNum");

    message.set(34, Scalar::from(7_i32)).unwrap();

    let fields = message.as_field().fields();
    assert_eq!(fields.len(), before + 1, "appended, not inserted");
    let child = fields.last().unwrap();
    assert_eq!(child.name(), declared.name(), "the dictionary's spelling");
    assert_eq!(child.dtype(), declared.dtype(), "the dictionary's type");
    assert_eq!(child.as_fix().tag().unwrap(), Some(34));
    assert!(!child.is_nullable(), "a stated value is non-null");
    assert_eq!(message.by_tag(34).unwrap().as_i128(), Some(7));
    assert_eq!(
        message.by_name("MsgSeqNum").unwrap().as_i128(),
        Some(7),
        "reached by name through the registry"
    );
}

#[test]
fn a_set_value_replaces_an_existing_child_in_place_and_keeps_the_tag_index() {
    let (_, reader) = reader();
    let mut message = reader.one_line(ORDER, false).unwrap();
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
    assert_eq!(message.by_tag(55).unwrap(), &Scalar::from("MSFT"));
    assert_eq!(message.by_tag(54).unwrap().as_str(), Some("2"));
    // Every other tag still reaches the child it reached before.
    for (tag, value) in before {
        if tag == 55 || tag == 54 {
            continue;
        }
        assert_eq!(message.by_tag(tag).unwrap(), &value, "tag {tag}");
    }
}

#[test]
fn a_set_leaves_the_entries_and_the_wire_untouched() {
    let (_, reader) = reader();
    let parsed = reader.one_line(ORDER, false).unwrap();
    let mut message = parsed.clone();
    message.set(55, Scalar::from("MSFT")).unwrap();
    message.set(38, Scalar::from(100.0_f64)).unwrap();
    assert!(message.remove(54).is_some());
    assert_eq!(message.entries(), parsed.entries());
    assert_eq!(message.into_bytes(b'|'), ORDER);
    assert_eq!(
        message.digest(),
        parsed.digest(),
        "the digest is the arrival record's"
    );
}

#[test]
fn a_null_is_stored_as_a_stated_null() {
    let (_, reader) = reader();
    let mut message = reader.one_line(ORDER, false).unwrap();
    message.set(55, Scalar::Null).unwrap();
    let at = message.as_field().index_of("symbol").unwrap();
    assert!(message.as_field().fields()[at].is_nullable());
    assert_eq!(message.get_by_tag(55), Some(&Scalar::Null));
}

#[test]
fn an_unknown_name_is_refused_and_the_message_stands() {
    let (_, reader) = reader();
    let mut message = reader.one_line(ORDER, false).unwrap();
    let before = message.clone();
    let refused = message.set("nosuchfield", Scalar::from("y")).unwrap_err();
    assert!(refused.to_string().contains("nosuchfield"), "{refused}");
    assert_eq!(message, before);
    // A value the field refuses is refused the same way.
    assert!(message.set(34, Scalar::from("not a number")).is_err());
    assert_eq!(message, before);
}

#[test]
fn an_unknown_name_still_reaches_the_child_spelled_that_way() {
    let (_, reader) = reader();
    let mut message = reader.one_line(ORDER, false).unwrap();
    let at = message
        .as_field()
        .index_of("venuething")
        .expect("the venue's own child");
    message.set("Venue_Thing", Scalar::from("8")).unwrap();
    let child = &message.as_field().fields()[at];
    assert_eq!(child.name(), "venuething", "the child keeps its own field");
    assert_eq!(child.dtype(), &DataType::Utf8);
    assert_eq!(message.by_name("venuething").unwrap(), &Scalar::from("8"));
}

#[test]
fn a_bare_unknown_tag_is_appended_under_its_decimal_spelling() {
    let (_, reader) = reader();
    let mut message = reader.one_line(ORDER, false).unwrap();
    message.set(7777, Scalar::from("custom")).unwrap();
    let child = message.as_field().fields().last().unwrap();
    assert_eq!(child.name(), "7777");
    assert_eq!(child.dtype(), &DataType::Utf8);
    assert!(child.is_nullable());
    assert_eq!(message.by_tag(7777).unwrap(), &Scalar::from("custom"));
    // A second write reaches the same child rather than a second one.
    message.set(7777, Scalar::from("again")).unwrap();
    assert_eq!(message.by_tag(7777).unwrap(), &Scalar::from("again"));
    assert_eq!(
        message.as_field().index_of("7777").map(|at| at + 1),
        Some(message.as_field().fields().len())
    );
    // The one the line already carried is replaced where it stands.
    message.set(9999, Scalar::from("y")).unwrap();
    assert_eq!(message.by_tag(9999).unwrap(), &Scalar::from("y"));
}

#[test]
fn several_values_land_with_one_rebuild_as_the_same_writes_would_one_at_a_time() {
    let (_, reader) = reader();
    let mut many = reader.one_line(ORDER, false).unwrap();
    let mut one = many.clone();
    let writes = [
        (55, Scalar::from("MSFT")),
        (34, Scalar::from(7_i32)),
        (7777, Scalar::from("first")),
        (7777, Scalar::from("second")),
        (34, Scalar::from(8_i32)),
    ];
    for (tag, value) in writes.clone() {
        one.set(tag, value).unwrap();
    }
    many.set_many(writes).unwrap();
    assert_eq!(many, one);
    assert_eq!(many.by_tag(34).unwrap().as_i128(), Some(8));
    assert_eq!(many.by_tag(7777).unwrap(), &Scalar::from("second"));
    assert_eq!(many.into_bytes(b'|'), ORDER);

    // One refused write refuses them all, and the message stands.
    let before = many.clone();
    assert!(
        many.set_many([(55, Scalar::from("X")), (34, Scalar::from("not a number"))])
            .is_err()
    );
    assert_eq!(many, before);
    many.set_many(Vec::<(i32, Scalar)>::new()).unwrap();
    assert_eq!(many, before);
}

#[test]
fn the_consuming_twin_answers_what_the_setter_leaves() {
    let (_, reader) = reader();
    let mut set = reader.one_line(ORDER, false).unwrap();
    let with = set.clone().with_value(55, Scalar::from("MSFT")).unwrap();
    set.set(55, Scalar::from("MSFT")).unwrap();
    assert_eq!(with, set);
}

#[test]
fn remove_answers_the_value_and_the_other_tags_still_reach_their_children() {
    let (_, reader) = reader();
    let mut message = reader.one_line(ORDER, false).unwrap();
    let before = stated(&message);
    let count = message.as_field().fields().len();

    assert_eq!(message.remove(55), Some(Scalar::from("AAPL")));
    assert_eq!(message.get_by_tag(55), None);
    assert_eq!(message.as_field().fields().len(), count - 1);
    for (tag, value) in &before {
        if *tag == 55 {
            continue;
        }
        assert_eq!(message.by_tag(*tag).unwrap(), value, "tag {tag}");
    }
    // By name, by decimal, and a miss.
    assert_eq!(message.remove("VenueThing"), Some(Scalar::from("7")));
    assert_eq!(message.remove(9999), Some(Scalar::from("x")));
    assert_eq!(message.remove(55), None);
    assert_eq!(message.remove("nosuchfield"), None);
    assert_eq!(message.as_field().fields().len(), count - 3);
    assert_eq!(message.into_bytes(b'|'), ORDER, "the entries are untouched");
}

#[test]
fn a_row_reads_back_into_the_message_that_made_it() {
    let (registry, reader) = reader();
    let schema = fix_schema(&registry, "fix").unwrap();
    let parsed = reader.one_line(ORDER, false).unwrap();
    let row = parsed.into_row(&schema).unwrap();

    let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();

    assert_eq!(held.as_field(), &schema, "the root is the schema");
    assert_eq!(held.entries(), parsed.entries());
    assert_eq!(held.into_bytes(b'|'), ORDER);
    assert_eq!(held.digest(), parsed.digest());
    for tag in [8, 35, 11, 55, 54] {
        assert_eq!(
            held.by_tag(tag).unwrap(),
            parsed.by_tag(tag).unwrap(),
            "tag {tag}"
        );
    }
    assert_eq!(held.version(), parsed.version());
    // And it makes the row it came from, whole.
    assert_eq!(held.into_row(&schema).unwrap(), row);
}

#[test]
fn a_row_carrying_its_captures_own_columns_returns_to_its_schema_whole() {
    let (registry, reader) = reader();
    // Nullable, because a message parsed on its own states none of them.
    let capture = DataType::from_fields([
        DataType::Utf8.nullable_field("url"),
        DataType::Int64.nullable_field("rownum"),
        DataType::Binary.nullable_field("body"),
    ])
    .unwrap()
    .required_field("line");
    let schema = fix_schema_carrying(&capture, &fix_schema(&registry, "fix").unwrap()).unwrap();
    let parsed = reader.one_line(ORDER, false).unwrap();

    // A parsed message has no capture columns: they are null in its row.
    let row = parsed.into_row(&schema).unwrap();
    let at = |name: &str| schema.index_of(name).unwrap();
    assert!(row.get(at("url")).unwrap().is_null());
    assert!(row.get(at("rownum")).unwrap().is_null());

    // Read back, the message holds them as children, and a written one
    // lands in its column: a column no tag names takes the child of its name.
    let mut held = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(held.into_row(&schema).unwrap(), row);
    held.set("url", Scalar::from("file:///capture.log"))
        .unwrap();
    held.set("rownum", Scalar::from(42_i64)).unwrap();
    let filled = held.into_row(&schema).unwrap();
    assert_eq!(
        filled.get(at("url")).unwrap().as_str(),
        Some("file:///capture.log")
    );
    assert_eq!(filled.get(at("rownum")).unwrap().as_i128(), Some(42));
    assert_eq!(filled.get(at("symbol")).unwrap().as_str(), Some("AAPL"));
    // The fixed columns are untouched by it, entries included.
    let again = FixMsg::from_row(Arc::clone(&registry), &schema, &filled).unwrap();
    assert_eq!(again.entries(), parsed.entries());
    assert_eq!(
        again.by_name("url").unwrap().as_str(),
        Some("file:///capture.log")
    );
}

#[test]
fn a_row_without_the_entries_column_has_no_entries() {
    let (registry, reader) = reader();
    let wide = fix_schema(&registry, "fix").unwrap();
    let columns: Vec<Field> = wide
        .fields()
        .iter()
        .filter(|column| !matches!(column.name(), ENTRIES_COLUMN | UNMAPPED_COLUMN))
        .cloned()
        .collect();
    let narrow = DataType::from_fields(columns)
        .unwrap()
        .required_field("fix");
    let parsed = reader.one_line(ORDER, false).unwrap();
    let row = parsed.into_row(&narrow).unwrap();

    let held = FixMsg::from_row(Arc::clone(&registry), &narrow, &row).unwrap();
    assert!(held.entries().is_empty());
    assert!(held.into_bytes(b'|').is_empty());
    assert_eq!(held.by_tag(55).unwrap(), parsed.by_tag(55).unwrap());
    assert_eq!(held.into_row(&narrow).unwrap(), row);
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
    let parsed = reader.one_line(DEEP, false).unwrap();
    fn depth(entries: &[FixEntry]) -> usize {
        entries
            .iter()
            .map(|entry| 1 + depth(entry.children()))
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
        match entry.get(4) {
            Some(tail) if tail.as_bytes().is_some_and(|bytes| !bytes.is_empty()) => true,
            Some(tail) => tail
                .as_sequence()
                .unwrap_or_default()
                .iter()
                .filter_map(Scalar::as_sequence)
                .any(leaf),
            None => false,
        }
    }
    let entries = row.get(schema.index_of(ENTRIES_COLUMN).unwrap()).unwrap();
    assert!(
        entries
            .as_sequence()
            .unwrap()
            .iter()
            .filter_map(Scalar::as_sequence)
            .any(leaf)
    );

    let held = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap();
    assert_eq!(held.entries(), parsed.entries());
    assert_eq!(held.into_bytes(b'|'), DEEP);
    assert_eq!(held.into_row(&schema).unwrap(), row);
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
    values[schema.index_of(ENTRIES_COLUMN).unwrap()] =
        Scalar::from_sequence([Scalar::from_sequence([
            Scalar::from(35_i32),
            Scalar::from(0_i32),
            Scalar::from("35"),
            Scalar::from("D"),
            Scalar::from(b"not json" as &[u8]),
        ])]);
    let row = Scalar::from_sequence(values);
    let refused = FixMsg::from_row(Arc::clone(&registry), &schema, &row).unwrap_err();
    assert!(!refused.to_string().is_empty());
}
