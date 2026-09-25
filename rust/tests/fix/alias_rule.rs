//! `rust/src/fix/build.rs`: the `FIX:names` alias rule. An alias fills a
//! field only where neither the canonical name nor the tag arrived; among
//! aliases the first in `FIX:names` order wins; a losing or shadowed alias
//! stays its own unmapped child, re-emits, and is no anomaly.

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
        "X",
        "the order's own identifier names the chain"
    );
    assert_eq!(
        child(&held, "marketorderid"),
        None,
        "the alias is the field, not a child"
    );

    let held = parsed(b"8=FIX.4.4|35=8|17=E1|55=AAPL|OMSDEALERORDERID=Y|10=0|");
    assert_eq!(orderid(&held).as_deref(), Some("Y"));
    assert_eq!(held.get_crosscode(), "Y");
}

#[test]
fn the_first_alias_in_names_order_wins_and_the_other_is_its_own_child() {
    for line in [
        &b"8=FIX.4.4|35=8|17=E1|55=AAPL|MARKETORDERID=X|OMSDEALERORDERID=Y|10=0|"[..],
        b"8=FIX.4.4|35=8|17=E1|55=AAPL|OMSDEALERORDERID=Y|MARKETORDERID=X|10=0|",
    ] {
        let held = parsed(line);
        let children: Vec<&str> = held
            .as_field()
            .fields()
            .iter()
            .map(yggdryl::Field::name)
            .collect();
        assert_eq!(
            orderid(&held).as_deref(),
            Some("X"),
            "{line:?}: {children:?} {:?}",
            held.anomalies()
        );
        assert_eq!(held.get_crosscode(), "X", "{line:?}");
        assert_eq!(
            child(&held, "omsdealerorderid").as_deref(),
            Some("Y"),
            "the loser keeps its own spelling: {line:?}"
        );
        assert_eq!(child(&held, "marketorderid"), None, "{line:?}");
        assert!(
            held.anomalies().is_empty(),
            "a shadowed alias is no anomaly: {line:?}"
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
        assert_eq!(held.get_crosscode(), "A", "{line:?}");
        assert_eq!(
            child(&held, "marketorderid").as_deref(),
            Some("B"),
            "{line:?}"
        );
        assert_eq!(
            child(&held, "omsdealerorderid").as_deref(),
            Some("B"),
            "{line:?}"
        );
        assert!(held.anomalies().is_empty(), "{line:?}");
    }
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
fn a_losing_alias_holds_through_a_row_and_a_second_pass() {
    // A row's residual entries carry no metadata: the untagged entry spelled
    // as an alias is rebuilt as the alias that lost, never as the field.
    let registry = committed_registry();
    let schema = yggdryl::fix_schema(&registry, "fix").expect("a schema");
    let held =
        parsed(b"8=FIX.4.4|35=8|17=E1|55=AAPL|37=L9|MARKETORDERID=L9|OMSDEALERORDERID=L9|10=0|");
    let row = held.into_row(&schema).expect("a row");
    let again = FixMsg::from_row(std::sync::Arc::clone(&registry), &schema, &row)
        .expect("the row's message");
    assert_eq!(orderid(&again).as_deref(), Some("L9"));
    for name in ["marketorderid", "omsdealerorderid"] {
        assert_eq!(child(&again, name).as_deref(), Some("L9"), "{name}");
        let at = again.as_field().index_of(name).expect("the alias child");
        assert_eq!(
            again.as_field().fields()[at].get_metadata("FIX:alias"),
            Some("orderid"),
            "{name} says what it lost to"
        );
    }
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
    assert!(again.anomalies().is_empty());
}
