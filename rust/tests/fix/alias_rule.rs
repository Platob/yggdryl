//! `rust/src/fix/build.rs`: the `FIX:names` alias rule. An alias fills a
//! field only where neither the canonical name nor the tag arrived; among
//! aliases the first in `FIX:names` order wins; a losing or shadowed alias
//! stating the value its field holds is that field and leaves nothing, and
//! one stating another value is an anomaly kept in the message's metadata,
//! never a child and never on the wire. A write to the field states every
//! such key again, and a removal clears them.

use super::{committed_registry, fixed_codec};
use yggdryl::FixMsg;
use yggdryl::graph::Element;

fn parsed(line: &[u8]) -> FixMsg {
    fixed_codec(committed_registry())
        .parse_fix_line(line)
        .expect("a readable line")
}

fn orderid(held: &FixMsg) -> Option<String> {
    held.get_by_tag(37)
        .and_then(|value| value.as_str().map(str::to_owned))
}

fn child(held: &FixMsg, name: &str) -> Option<String> {
    held.as_field().index_of(name).and_then(|at| {
        held.as_value()
            .as_sequence()?
            .get(at)?
            .as_str()
            .map(str::to_owned)
    })
}

#[test]
fn an_alias_fills_the_field_where_nothing_else_named_it() {
    // `orderid` (37) lists `marketorderid` then `omsdealerorderid`.
    let held = parsed(b"8=FIX.4.4|35=8|17=E1|55=AAPL|MARKETORDERID=X|10=0|");
    assert_eq!(orderid(&held).as_deref(), Some("X"));
    assert_eq!(
        held.get_crosscode(),
        "10:0:X",
        "the order's own identifier names the chain, stored under its kind and no side"
    );
    assert_eq!(
        child(&held, "marketorderid"),
        None,
        "the alias is the field, not a child"
    );

    let held = parsed(b"8=FIX.4.4|35=8|17=E1|55=AAPL|OMSDEALERORDERID=Y|10=0|");
    assert_eq!(orderid(&held).as_deref(), Some("Y"));
    assert_eq!(held.get_crosscode(), "10:0:Y");
}

fn kept(held: &FixMsg, key: &str) -> Option<String> {
    held.metadata().get(key).map(ToString::to_string)
}

fn wire(held: &FixMsg) -> String {
    String::from_utf8(held.into_bytes(b'|')).expect("a text wire")
}

#[test]
fn the_first_alias_in_names_order_wins_and_the_other_is_kept_in_the_metadata() {
    for line in [
        &b"8=FIX.4.4|35=8|17=E1|55=AAPL|MARKETORDERID=X|OMSDEALERORDERID=Y|10=0|"[..],
        b"8=FIX.4.4|35=8|17=E1|55=AAPL|OMSDEALERORDERID=Y|MARKETORDERID=X|10=0|",
    ] {
        let held = parsed(line);
        assert_eq!(orderid(&held).as_deref(), Some("X"), "{line:?}");
        assert_eq!(held.get_crosscode(), "10:0:X", "{line:?}");
        assert_eq!(child(&held, "omsdealerorderid"), None, "{line:?}");
        assert_eq!(child(&held, "marketorderid"), None, "{line:?}");
        assert_eq!(
            kept(&held, "omsdealerorderid").as_deref(),
            Some("Y"),
            "the loser stating another value is kept: {line:?}"
        );
        assert_eq!(kept(&held, "marketorderid"), None, "{line:?}");
        let anomalies: Vec<&str> = held.anomalies().iter().map(|held| held.field()).collect();
        assert_eq!(anomalies, ["omsdealerorderid"], "{line:?}");
        assert!(
            held.anomalies()[0].reason().contains("orderid"),
            "{}",
            held.anomalies()[0].reason()
        );
        let wire = wire(&held);
        assert!(wire.contains("|37=X|"), "{wire}");
        assert!(
            !wire.to_ascii_lowercase().contains("omsdealerorderid"),
            "{wire}"
        );
    }
}

#[test]
fn the_canonical_name_or_the_tag_shadows_every_alias() {
    for line in [
        &b"8=FIX.4.4|35=8|17=E1|55=AAPL|ORDERID=A|MARKETORDERID=B|OMSDEALERORDERID=B|10=0|"[..],
        b"8=FIX.4.4|35=8|17=E1|55=AAPL|MARKETORDERID=B|37=A|OMSDEALERORDERID=B|10=0|",
        b"8=FIX.4.4|35=8|17=E1|55=AAPL|MARKETORDERID=B|OMSDEALERORDERID=B|ORDERID=A|10=0|",
    ] {
        let held = parsed(line);
        assert_eq!(orderid(&held).as_deref(), Some("A"), "{line:?}");
        assert_eq!(held.get_crosscode(), "10:0:A", "{line:?}");
        for name in ["marketorderid", "omsdealerorderid"] {
            assert_eq!(child(&held, name), None, "{line:?}");
            assert_eq!(kept(&held, name).as_deref(), Some("B"), "{name}: {line:?}");
        }
        let mut anomalies: Vec<&str> = held.anomalies().iter().map(|held| held.field()).collect();
        anomalies.sort_unstable();
        assert_eq!(anomalies, ["marketorderid", "omsdealerorderid"], "{line:?}");
        assert!(
            !wire(&held).to_ascii_lowercase().contains("orderid=b"),
            "{line:?}"
        );
    }
}

#[test]
fn an_alias_stating_its_fields_value_is_that_field_and_leaves_nothing() {
    let held = parsed(b"8=FIX.4.4|35=8|17=E1|55=AAPL|37=A|MARKETORDERID=A|10=0|");
    assert_eq!(orderid(&held).as_deref(), Some("A"));
    assert_eq!(child(&held, "marketorderid"), None);
    assert_eq!(kept(&held, "marketorderid"), None);
    assert!(held.anomalies().is_empty(), "{:?}", held.anomalies());
}

#[test]
fn a_write_states_every_kept_alias_again_and_a_removal_clears_them() {
    let mut held =
        parsed(b"8=FIX.4.4|35=8|17=E1|55=AAPL|37=A|MARKETORDERID=B|OMSDEALERORDERID=C|10=0|");
    assert_eq!(kept(&held, "marketorderid").as_deref(), Some("B"));
    assert_eq!(kept(&held, "omsdealerorderid").as_deref(), Some("C"));
    held.set(37, yggdryl::Scalar::from("Z")).expect("a write");
    assert_eq!(orderid(&held).as_deref(), Some("Z"));
    assert_eq!(kept(&held, "marketorderid").as_deref(), Some("Z"));
    assert_eq!(kept(&held, "omsdealerorderid").as_deref(), Some("Z"));
    // A key no tag explains is no sibling of anything.
    let mut venue = parsed(b"8=FIX.4.4|35=8|17=E1|55=AAPL|37=A|VENUE.NOTE=keep|10=0|");
    assert_eq!(kept(&venue, "venue.note").as_deref(), Some("keep"));
    venue.set(37, yggdryl::Scalar::from("Z")).expect("a write");
    assert_eq!(kept(&venue, "venue.note").as_deref(), Some("keep"));

    assert_eq!(
        held.remove(37).expect("a removal"),
        Some(yggdryl::Scalar::from("Z"))
    );
    assert_eq!(orderid(&held), None);
    assert_eq!(kept(&held, "marketorderid"), None);
    assert_eq!(kept(&held, "omsdealerorderid"), None);
}

#[test]
fn a_write_states_a_composed_bridge_voice_again() {
    let mut held = parsed(b"8=FIX.4.4|35=8|17=E1|55=AAPL|ULLINK.OFFERPX=11|10=0|");
    assert!(
        held.get_by_tag(133).is_some(),
        "the voice composed into OfferPx"
    );
    assert_eq!(kept(&held, "ullink.offerpx").as_deref(), Some("11"));
    held.set(133, yggdryl::Scalar::from("12.5"))
        .expect("a write");
    assert_eq!(kept(&held, "ullink.offerpx").as_deref(), Some("12.5"));
    held.remove(133).expect("a removal");
    assert_eq!(kept(&held, "ullink.offerpx"), None);
}

#[test]
fn the_other_aliased_fields_still_resolve_from_one_spelling() {
    // Every field with `FIX:names` resolves each alias to itself when that
    // alias arrives alone: the rule changes nothing for a lone spelling.
    let registry = committed_registry();
    let codec = fixed_codec(std::sync::Arc::clone(&registry));
    let mut checked = 0;
    for field in registry.iter() {
        let Some(tag) = field.as_fix().tag().ok().flatten() else {
            continue;
        };
        if field.dtype().is_nested() || tag == 37 {
            continue;
        }
        for alias in field.as_fix().names() {
            // An alias that is another field's canonical name reaches that
            // field, as it did before the rule.
            if registry
                .get_field_by_name(alias)
                .is_some_and(|reached| reached.name() != field.name())
            {
                continue;
            }
            // The market's alias states an ISO 10383 MIC or nothing at all.
            let value = if tag == yggdryl::MICCODE_TAG_NAME.0 {
                "XSWX"
            } else {
                "1"
            };
            let line = format!("8=FIX.4.4|35=D|11=A1|55=AAPL|{alias}={value}|10=0|");
            let Ok(held) = codec.parse_fix_line(line.as_bytes()) else {
                continue;
            };
            assert!(
                held.get_by_tag(tag).is_some(),
                "{alias} reaches {} ({tag})",
                field.name()
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 40,
        "the dictionary's aliased fields were exercised: {checked}"
    );
}

#[test]
fn a_kept_alias_holds_through_a_row_and_a_second_pass() {
    // The row's `metadata` column carries what the message kept, and an
    // alias stating its field's value left nothing to carry.
    let registry = committed_registry();
    let schema = yggdryl::fix_schema(&registry, "fix").expect("a schema");
    let held =
        parsed(b"8=FIX.4.4|35=8|17=E1|55=AAPL|37=L9|MARKETORDERID=L9|OMSDEALERORDERID=M1|10=0|");
    assert_eq!(kept(&held, "marketorderid"), None);
    assert_eq!(kept(&held, "omsdealerorderid").as_deref(), Some("M1"));
    let row = held.into_row(&schema).expect("a row");
    let again = FixMsg::from_row(std::sync::Arc::clone(&registry), &schema, &row)
        .expect("the row's message");
    assert_eq!(orderid(&again).as_deref(), Some("L9"));
    assert_eq!(child(&again, "omsdealerorderid"), None);
    assert_eq!(kept(&again, "omsdealerorderid").as_deref(), Some("M1"));
    // Root entries may move between the residual and the projected order;
    // every pair the wire stated is stated again, once.
    let pairs = |message: &FixMsg| {
        let mut pairs: Vec<Vec<u8>> = message
            .into_bytes(b'|')
            .split(|byte| *byte == b'|')
            .map(<[u8]>::to_vec)
            .collect();
        pairs.sort();
        pairs
    };
    assert_eq!(pairs(&again), pairs(&held));
    assert_eq!(again.into_row(&schema).expect("a row"), row);
}
