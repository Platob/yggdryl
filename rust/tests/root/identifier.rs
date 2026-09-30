//! `rust/src/identifier.rs`: one identifier - a value, the source that gave it
//! and the type of name it is - keyed `src:type`, the sorted map of them laid
//! out as an Arrow map from that key to its row, and the parents a chain gives
//! the bases it states.

use std::borrow::Cow;
use std::sync::Arc;

use arrow_array::Array;
use yggdryl::{
    ArrowCastOptions, DataType, Field, IdSource, IdType, IdWord, Identifier, Identifiers, Scalar,
    Serie,
};

/// The text of cell `at` of a row.
fn cell(row: &Scalar, at: usize) -> String {
    row.get(at)
        .and_then(|cell| cell.as_str().map(str::to_owned))
        .expect("a text cell")
}

fn id(src: &str, kind: &str, value: &str) -> Identifier {
    Identifier::new(src.parse().unwrap(), kind.parse().unwrap(), value).unwrap()
}

fn kind(text: &str) -> IdType {
    text.parse().unwrap()
}

fn source(text: &str) -> IdSource {
    text.parse().unwrap()
}

/// A set of `fix` identifiers, one per `(type, value)` pair.
fn fix(pairs: &[(&str, &str)]) -> Identifiers {
    pairs
        .iter()
        .map(|(kind, value)| id("fix", kind, value))
        .collect()
}

/// What a chain's follower takes from its predecessor, under the crate's own
/// parentage vocabulary.
fn follow(next: &mut Identifiers, previous: &Identifiers) -> bool {
    next.follow_parents(previous, IdType::parents, IdType::parent_of)
}

/// The states of one chain, in order: each states one of `values` as the base
/// `base` from `fix`, and takes its parents from the state before it as a
/// lifecycle does.
fn chain(base: &str, values: &[&str]) -> Vec<Identifiers> {
    let mut states: Vec<Identifiers> = Vec::new();
    for value in values {
        let mut next = fix(&[(base, value)]);
        if let Some(previous) = states.last() {
            follow(&mut next, previous);
        }
        states.push(next);
    }
    states
}

/// The types of `ids` as `src:type` keys, in the order the set holds them.
fn keys(ids: &Identifiers) -> Vec<String> {
    ids.iter().map(|held| held.key().to_string()).collect()
}

/// A parents list of three, nearest first, for `orderid` alone: the
/// vocabulary is the caller's, `IdType::parents` is one reading of it.
fn three_parents(base: &IdType) -> Cow<'static, [IdType]> {
    if base == &IdType::OrderId {
        Cow::Owned(["near", "middle", "far"].map(kind).to_vec())
    } else {
        Cow::Borrowed(&[])
    }
}

/// The base and place of each of the three custom parents.
fn three_parent_of(parent: &IdType) -> Option<(IdType, usize)> {
    ["near", "middle", "far"]
        .iter()
        .position(|word| parent == word)
        .map(|at| (IdType::OrderId, at))
}

#[test]
fn a_value_that_states_nothing_is_refused_and_a_word_that_is_not_one_is_too() {
    for value in ["", "  ", "null", "NULL", "N/A", "none", "[N/A]"] {
        assert!(
            Identifier::new(IdSource::Fix, IdType::OrderId, value).is_err(),
            "{value:?}"
        );
        assert!(
            Identifier::new(IdSource::Fix, IdType::Isin, value).is_err(),
            "{value:?}"
        );
    }
    let refused = Identifier::new(IdSource::Fix, IdType::OrderId, "n/a")
        .unwrap_err()
        .to_string();
    assert!(refused.contains("text stating something"), "{refused}");
    for word in ["", "IS/IN", "IS:IN", "caf\u{e9}", "tab\tkey"] {
        assert!(word.parse::<IdType>().is_err(), "{word:?}");
        assert!(word.parse::<IdSource>().is_err(), "{word:?}");
    }
    // A word holds the longest name a FIX code set gives a party role or an
    // identifier source, and no more; so does a value, trimmed.
    let longest = "X".repeat(64);
    let long = Identifier::new(IdSource::Fix, longest.parse().unwrap(), &"v".repeat(64)).unwrap();
    assert_eq!(long.kind().as_str(), "x".repeat(64));
    assert_eq!(long.value().len(), 64);
    let refused = "X".repeat(65).parse::<IdType>().unwrap_err().to_string();
    assert!(refused.contains("at most 64 bytes"), "{refused}");
    assert!(
        Identifier::new(IdSource::Fix, IdType::OrderId, &"9".repeat(65)).is_err(),
        "a value past 64 bytes"
    );
    assert!(Identifier::new(IdSource::Fix, IdType::Isin, &"9".repeat(65)).is_err());
}

#[test]
fn words_fold_to_lower_case_and_values_trim() {
    let party = Identifier::new(
        "Proprietary".parse().unwrap(),
        "Executing Trader".parse().unwrap(),
        " trader1 ",
    )
    .unwrap();
    assert_eq!(party.kind(), "executingtrader");
    assert_eq!(party.src(), "proprietary");
    assert_eq!(party.kind(), &IdType::ExecutingTrader);
    assert_eq!(party.src(), &IdSource::Proprietary);
    assert_eq!(party.value(), "trader1");
    assert_eq!(party.to_string(), "proprietary:executingtrader=trader1");
    assert!(party.is_of(&IdSource::Proprietary, &IdType::ExecutingTrader));
    assert!(!party.is_of(&IdSource::Base, &IdType::ExecutingTrader));
    assert!(!party.is_of(&IdSource::Proprietary, &IdType::ClientId));

    // A value keeps its case, except where its type is a code that folds.
    let order = id("fix", "orderid", "O-1a");
    assert_eq!(order.value(), "O-1a");
    let isin = Identifier::new(IdSource::Base, IdType::Isin, " us0378331005 ").unwrap();
    assert_eq!(isin.to_string(), "base:isin=US0378331005");
    // Nothing upper case survives in a word, in a display or in a row.
    let row = isin.clone().into_scalar();
    assert_eq!(cell(&row, 0), "base");
    assert_eq!(cell(&row, 1), "isin");
    assert_eq!(cell(&row, 2), "US0378331005");
}

#[test]
fn a_word_no_member_names_is_an_other_word_and_never_a_member() {
    let other: IdType = "House Code".parse().unwrap();
    assert!(!other.is_known());
    assert_eq!(other.as_str(), "housecode");
    assert!(matches!(other, IdType::Other(_)));
    let IdType::Other(word) = other else {
        panic!("an other word");
    };
    let word: IdWord = word;
    assert_eq!(word.as_str(), "housecode");
    // A word a member names is never an `Other`.
    assert!(matches!("ISIN".parse::<IdType>().unwrap(), IdType::Isin));
    assert!(matches!("FIX".parse::<IdSource>().unwrap(), IdSource::Fix));
    assert!(matches!(
        "firm".parse::<IdSource>().unwrap(),
        IdSource::Other(_)
    ));
    // An identifier of an other type is held and displayed under its word.
    let held = id("firm", "house code", "hc-1");
    assert_eq!(held.to_string(), "firm:housecode=hc-1");
    assert_eq!(held.kind(), "housecode");
    assert_eq!(held.src(), "firm");
}

#[test]
fn an_identifier_names_its_unique_key_src_type() {
    let held = id("Fix", "ClOrdID", "C-1");
    assert_eq!(held.key(), "fix:clordid");
    assert_eq!(
        held.key().to_string(),
        format!("{}:{}", held.src(), held.kind())
    );
    assert_eq!(id("firm.x", "house code", "hc-1").key(), "firm.x:housecode");
    let isin = Identifier::new(IdSource::Base, IdType::Isin, "US0378331005").unwrap();
    assert_eq!(isin.key(), "base:isin");
    // The key is the identity of a name, never of a value: two values of one
    // name share it, and the same type from another source does not.
    assert_eq!(
        id("fix", "orderid", "A").key(),
        id("fix", "orderid", "B").key()
    );
    assert_ne!(
        id("fix", "orderid", "A").key(),
        id("venue", "orderid", "A").key()
    );
    assert_ne!(
        id("fix", "orderid", "A").key(),
        id("fix", "clordid", "A").key()
    );
    // It is what `from_key` reads back.
    assert_eq!(
        Identifier::from_key(&held.key(), held.value()).unwrap(),
        held
    );
}

#[test]
fn identifiers_order_by_their_key_as_it_is_spelled_then_by_value() {
    // The order is the one of the text `src:type`, never of the pair: a source
    // that goes on past a shorter one with a byte below ':' - a '.' or a
    // digit - sorts before the shorter source's types, whereas as words it
    // sorts after it.
    assert!(source("a") < source("a.b"), "as words a prefix sorts first");
    assert!(source("a") < source("a1"), "as words a prefix sorts first");
    assert!(
        id("a.b", "c", "1") < id("a", "z", "1"),
        "'a.b:c' sorts before 'a:z'"
    );
    assert!(
        id("a1", "x", "1") < id("a", "x", "1"),
        "'a1:x' sorts before 'a:x'"
    );
    assert!(
        id("a1", "a", "1") < id("a", "b", "1"),
        "the source decides before any type does"
    );
    assert!(
        id("a", "z", "1") < id("ab", "a", "1"),
        "':' sorts before a letter"
    );
    // Equal keys order by value, and a key decides before any value does.
    assert!(id("fix", "orderid", "A") < id("fix", "orderid", "B"));
    assert!(id("fix", "orderid", "Z") < id("oms", "clordid", "A"));
    assert_eq!(
        id("a.b", "c", "1").cmp(&id("a.b", "c", "1")),
        std::cmp::Ordering::Equal
    );
    // Loose identifiers sort as a set holds them.
    let mut loose = [
        id("a", "z", "1"),
        id("a1", "x", "1"),
        id("a", "x", "2"),
        id("a", "x", "1"),
        id("a.b", "c", "1"),
        id("ab", "a", "1"),
    ];
    loose.sort();
    assert_eq!(
        loose.iter().map(ToString::to_string).collect::<Vec<_>>(),
        ["a.b:c=1", "a1:x=1", "a:x=1", "a:x=2", "a:z=1", "ab:a=1"]
    );
}

#[test]
fn an_explicit_key_names_its_source_and_its_type() {
    let explicit = Identifier::from_key("fix:clordid", "C-1").unwrap();
    assert_eq!(explicit.to_string(), "fix:clordid=C-1");
    assert_eq!(
        Identifier::from_key(" FIX:ClOrdID ", " C-1 ").unwrap(),
        explicit,
        "a key folds and a value trims"
    );
    // Both halves are words of their own vocabularies: any word does.
    let bridged = Identifier::from_key("oms:Instrument_Id", "dbi;X").unwrap();
    assert_eq!(bridged.to_string(), "oms:instrumentid=dbi;X");
    assert_eq!(
        Identifier::from_key("firm.x:house code", "hc-1")
            .unwrap()
            .to_string(),
        "firm.x:housecode=hc-1"
    );
    // The two halves are all it reads: what is no word, what its type
    // refuses and what states nothing read as none.
    assert!(Identifier::from_key("fix:isin", "US0378331006").is_none());
    assert!(Identifier::from_key("fix:clordid", "n/a").is_none());
    assert!(Identifier::from_key("fix:", "C-1").is_none());
    assert!(Identifier::from_key(":clordid", "C-1").is_none());
    assert!(Identifier::from_key("a:b:c", "C-1").is_none());
    assert!(Identifier::from_key("a/b:clordid", "C-1").is_none());
    assert!(Identifier::from_key("", "C-1").is_none());
}

#[test]
fn a_key_is_read_for_the_identifier_name_it_ends_with_and_the_source_before_it() {
    let read = |key: &str, value: &str| {
        Identifier::from_key(key, value)
            .unwrap_or_else(|| panic!("{key:?} names an identifier"))
            .to_string()
    };
    // The source is the rest of the folded key with its dots trimmed at the
    // ends and kept inside; nothing left is `base`.
    assert_eq!(
        read("firm.x.ParentOrderID", "P-1"),
        "firm.x:parentorderid=P-1"
    );
    assert_eq!(read("OMS_InstrumentID", "dbi;X"), "oms:instrumentid=dbi;X");
    assert_eq!(read("marketorderid", "O-1"), "market:orderid=O-1");
    assert_eq!(
        read("OMSDealerParentOrderID", "P-1"),
        "omsdealer:parentorderid=P-1"
    );
    assert_eq!(read("OMSUserID", "U-1"), "oms:userid=U-1");
    assert_eq!(read("fix.ClOrdID", "C-1"), "fix:clordid=C-1");
    assert_eq!(read("OrderID", "O-1"), "base:orderid=O-1");
    assert_eq!(
        read(".OrderID", "O-1"),
        "base:orderid=O-1",
        "a dot at the end is trimmed"
    );
    assert_eq!(read("firm.x.OrderID", "O-1"), "firm.x:orderid=O-1");
    assert_eq!(
        read("firm..x_OrderID", "O-1"),
        "firm..x:orderid=O-1",
        "a dot inside is kept"
    );
    // Folding lowercases and drops `_`, `-`, space and `#` wherever they are.
    assert_eq!(read("OMS Order-ID", "O-1"), "oms:orderid=O-1");
    assert_eq!(read("oms_order_id", "O-1"), "oms:orderid=O-1");
    assert_eq!(read("oms#orderid", "O-1"), "oms:orderid=O-1");
    assert_eq!(read("#oms.orderid", "O-1"), "oms:orderid=O-1");
    assert_eq!(read("oms.Account", "ACC-1"), "oms:account=ACC-1");
    assert_eq!(read("venue_cusip", "037833100"), "venue:cusip=037833100");
    assert_eq!(read("firm.isin", "us0378331005"), "firm:isin=US0378331005");
    assert_eq!(read("OrigClOrdID", "C-0"), "base:origclordid=C-0");
    assert_eq!(read("firm.OrigClOrdID", "C-0"), "firm:origclordid=C-0");
    // The longest name the key ends with answers: `origclordid` is a name of
    // its own, so the source is `my` and not `myorig`.
    assert_eq!(read("MyOrigClOrdID", "C-0"), "my:origclordid=C-0");
    // The identifier of a key a whole security name spells is that type from
    // `base`, with a leading # dropped and every alias of its name.
    assert_eq!(read("ISINCode", "US0378331005"), "base:isin=US0378331005");
    assert_eq!(read("#ISINCODE", "US0378331005"), "base:isin=US0378331005");
    assert_eq!(
        read("ISIN_Number", "US0378331005"),
        "base:isin=US0378331005"
    );
    assert_eq!(read("security_cusip", "037833100"), "base:cusip=037833100");
    assert_eq!(read("#isin", "US0378331005"), "base:isin=US0378331005");
    assert_eq!(
        read("fix:clordid", "C-1"),
        "fix:clordid=C-1",
        "an explicit key reads as itself"
    );
}

#[test]
fn a_parentage_word_before_the_name_stays_part_of_the_type() {
    let read = |key: &str, value: &str| {
        Identifier::from_key(key, value).unwrap_or_else(|| panic!("{key:?} names an identifier"))
    };
    // What each key reads as, and the base and place its type is a parent of.
    for (key, expected, parent) in [
        (
            "origclordid",
            "base:origclordid=T-1",
            Some((IdType::ClOrdId, 0)),
        ),
        (
            "OrigClOrdID",
            "base:origclordid=T-1",
            Some((IdType::ClOrdId, 0)),
        ),
        (
            "ParentOrderID",
            "base:parentorderid=T-1",
            Some((IdType::OrderId, 0)),
        ),
        (
            "firm.x.parentorderid",
            "firm.x:parentorderid=T-1",
            Some((IdType::OrderId, 0)),
        ),
        (
            "origorderid",
            "base:origorderid=T-1",
            Some((IdType::OrderId, 1)),
        ),
        (
            "origtradeid",
            "base:origtradeid=T-1",
            Some((IdType::TradeId, 1)),
        ),
        (
            "OrigTradeID",
            "base:origtradeid=T-1",
            Some((IdType::TradeId, 1)),
        ),
        (
            "fix.OrigTradeID",
            "fix:origtradeid=T-1",
            Some((IdType::TradeId, 1)),
        ),
        // A word that names no parent stays part of the type all the same.
        ("originalorderid", "base:originalorderid=T-1", None),
        ("oms.OriginalOrderID", "oms:originalorderid=T-1", None),
        ("omsoriginalorderid", "oms:originalorderid=T-1", None),
        ("originorderid", "base:originorderid=T-1", None),
        ("originclordid", "base:originclordid=T-1", None),
        // `parentclordid` is the other spelling of `clordid`'s one parent.
        (
            "firm.x.parentclordid",
            "firm.x:origclordid=T-1",
            Some((IdType::ClOrdId, 0)),
        ),
    ] {
        let keyed = read(key, "T-1");
        assert_eq!(keyed.to_string(), expected, "{key}");
        assert_eq!(
            keyed.kind().parent_of(),
            parent,
            "{key}: the type says its base"
        );
    }
    // A word that merely ends like a parentage word is part of the source.
    assert_eq!(
        read("parenthoodorderid", "O-1").to_string(),
        "parenthood:orderid=O-1"
    );
    assert_eq!(
        read("originatingorderid", "O-1").to_string(),
        "originating:orderid=O-1"
    );
}

#[test]
fn a_key_that_names_no_identifier_or_another_instruments_security_is_none() {
    // Another instrument's code is no identifier of this one: a security type
    // is refused where the key opens with the prefix of another instrument.
    for key in [
        "underlyingisin",
        "UnderlyingISIN",
        "legisin",
        "leg_isin",
        "legcusip",
        "contrasedol",
        "relatedfigi",
        "benchmark.isin",
        "underlying.instrumentid",
        "UnderlyingSecurityID",
    ] {
        assert!(
            Identifier::from_key(key, "US0378331005").is_none(),
            "{key:?} names another instrument"
        );
    }
    // Only a security type is refused there: an order identifier a leg or a
    // counterparty states is one of the element's own.
    assert_eq!(
        Identifier::from_key("contraorderid", "O-1")
            .unwrap()
            .to_string(),
        "contra:orderid=O-1"
    );
    assert_eq!(
        Identifier::from_key("leg.ClOrdID", "C-1")
            .unwrap()
            .to_string(),
        "leg:clordid=C-1"
    );
    // No identifier name at the key's end.
    for key in [
        "transversalkey",
        "TransversalKey",
        "price",
        "symbol",
        "ticker",
        "housekey",
        "oms.order.id",
        "securityid",
        "id",
        "orderidx",
        "a/b",
        "",
        "   ",
        "#",
    ] {
        assert!(
            Identifier::from_key(key, "X-1").is_none(),
            "{key:?} names none"
        );
    }
    // A value that states nothing, or that its type refuses.
    for value in ["", "   ", "null", "NULL", "none", "N/A", "[N/A]"] {
        assert!(
            Identifier::from_key("firm.x.ParentOrderID", value).is_none(),
            "{value:?}"
        );
        assert!(
            Identifier::from_key("ISINCode", value).is_none(),
            "{value:?}"
        );
        assert!(
            Identifier::from_key("fix:clordid", value).is_none(),
            "{value:?}"
        );
    }
    assert!(
        Identifier::from_key("ISINCode", "US0378331006").is_none(),
        "a bad check digit"
    );
    assert!(
        Identifier::from_key("firm.isin", "US0378331006").is_none(),
        "a bad check digit"
    );
    assert!(
        Identifier::from_key("venue.cusip", "037833101").is_none(),
        "a bad check digit"
    );
    assert!(
        Identifier::from_key("OrderID", &"9".repeat(65)).is_none(),
        "past the width"
    );
}

#[test]
fn a_set_holds_one_value_per_source_and_type_sorted_by_that_key() {
    let mut ids = Identifiers::new();
    assert!(ids.insert(id("fix", "isin", "US0378331005")));
    assert!(!ids.insert(id("FIX", "ISIN", "CH0012214059")), "fill only");
    assert!(
        ids.insert(id("venue", "isin", "CH0012214059")),
        "another source"
    );
    assert!(ids.insert(id("derived", "cusip", "037833100")));
    assert_eq!(ids.len(), 3);
    let keys: Vec<_> = ids
        .iter()
        .map(|held| (held.src().as_str(), held.kind().as_str()))
        .collect();
    assert_eq!(
        keys,
        [("derived", "cusip"), ("fix", "isin"), ("venue", "isin")]
    );
    assert_eq!(ids.get(&IdType::Isin), Some("US0378331005"));
    let venue: IdSource = "venue".parse().unwrap();
    assert_eq!(ids.get_from(&venue, &IdType::Isin), Some("CH0012214059"));
    assert_eq!(ids.get_from(&"oms".parse().unwrap(), &IdType::Isin), None);
    assert!(ids.contains_kind(&IdType::Cusip));
    assert!(!ids.contains_kind(&IdType::Sedol));
    assert_eq!(ids.of_kind(&IdType::Isin).count(), 2);
    assert!(ids.set(id("fix", "isin", "CH0012221716")));
    assert!(!ids.set(id("fix", "isin", "CH0012221716")), "no change");
    assert_eq!(ids.get(&IdType::Isin), Some("CH0012221716"));
    assert_eq!(
        ids.remove(&venue, &IdType::Isin)
            .map(|held| held.value().to_owned()),
        Some("CH0012214059".into())
    );
    assert!(ids.remove(&venue, &IdType::Isin).is_none());
    assert_eq!(ids.remove_kind(&IdType::Isin), 1);
    assert_eq!(ids.len(), 1);
    assert_eq!(ids.to_string(), "[derived:cusip=037833100]");
    ids.clear();
    assert!(ids.is_empty());
    assert_eq!(ids.to_string(), "[]");
}

#[test]
fn a_set_displays_and_sorts_by_its_keys_as_they_are_spelled() {
    let ids: Identifiers = [
        id("base", "isin", "US0378331005"),
        id("venue", "instrumentid", "dbi;X"),
        id("fix", "orderid", "O-1"),
        id("fix", "clordid", "C-1"),
        id("derived", "cusip", "037833100"),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        ids.to_string(),
        "[base:isin=US0378331005, derived:cusip=037833100, fix:clordid=C-1, \
         fix:orderid=O-1, venue:instrumentid=dbi;X]"
    );
    // The same set in any order is one value: equal, and one digest feed.
    let reversed: Identifiers = ids.iter().rev().cloned().collect();
    assert_eq!(ids, reversed);
    assert_eq!(ids.as_slice(), reversed.as_slice());
    // Identifiers order by key, then value, which is how the set holds them.
    let mut loose: Vec<Identifier> = ids.iter().cloned().collect();
    loose.reverse();
    loose.sort();
    assert_eq!(loose.as_slice(), ids.as_slice());
    assert!(id("fix", "orderid", "A") < id("fix", "orderid", "B"));
    assert!(id("fix", "orderid", "Z") < id("oms", "clordid", "A"));
}

#[test]
fn a_set_is_held_in_the_order_of_its_spelled_keys_and_every_lookup_finds_its_own() {
    let spelled = [
        ("a", "z"),
        ("a1", "x"),
        ("a.b", "c"),
        ("a", "x"),
        ("ab", "a"),
        ("a", "b.c"),
        ("a", "b"),
        ("a1", "a"),
    ];
    let held = [
        "a.b:c", "a1:a", "a1:x", "a:b", "a:b.c", "a:x", "a:z", "ab:a",
    ];
    // Any insertion order lands one set, in the order of the text `src:type`.
    let mut forward = Identifiers::new();
    let mut backward = Identifiers::new();
    for (src, ty) in spelled {
        assert!(forward.insert(id(src, ty, "1")), "{src}:{ty}");
    }
    for (src, ty) in spelled.iter().rev() {
        assert!(backward.insert(id(src, ty, "1")), "{src}:{ty}");
    }
    assert_eq!(keys(&forward), held);
    assert_eq!(forward, backward);
    // The binary search a lookup runs is over that order: each key is found,
    // a key between two held ones is not, and a key held under another
    // source is another key.
    for (src, ty) in spelled {
        assert_eq!(
            forward.get_from(&source(src), &kind(ty)),
            Some("1"),
            "{src}:{ty}"
        );
    }
    assert_eq!(forward.get_from(&source("a.b"), &kind("x")), None);
    assert_eq!(forward.get_from(&source("a"), &kind("b.d")), None);
    assert_eq!(forward.get_from(&source("a2"), &kind("x")), None);
    assert_eq!(forward.get_from(&source("b"), &kind("a")), None);
    // A removal keeps the rest in order and findable.
    assert!(forward.remove(&source("a"), &kind("b")).is_some());
    assert_eq!(
        keys(&forward),
        ["a.b:c", "a1:a", "a1:x", "a:b.c", "a:x", "a:z", "ab:a"]
    );
    assert_eq!(forward.get_from(&source("a"), &kind("b.c")), Some("1"));
    assert_eq!(forward.get_from(&source("a"), &kind("b")), None);
    // Another value under a held key is the same name: insert keeps the
    // first, set replaces it in place.
    assert!(!forward.insert(id("a1", "x", "2")));
    assert!(forward.set(id("a1", "x", "2")));
    assert_eq!(forward.get_from(&source("a1"), &kind("x")), Some("2"));
    assert_eq!(forward.len(), 7);
}

#[test]
fn a_stated_source_answers_before_a_derived_one() {
    let mut ids = Identifiers::new();
    ids.insert(id("derived", "cusip", "037833100"));
    assert_eq!(ids.get(&IdType::Cusip), Some("037833100"));
    ids.insert(id("fix", "cusip", "594918104"));
    assert_eq!(ids.get(&IdType::Cusip), Some("594918104"));
    assert_eq!(
        ids.get_identifier(&IdType::Cusip)
            .map(|held| held.src().as_str()),
        Some("fix")
    );
    assert_eq!(
        ids.get_from(&IdSource::Derived, &IdType::Cusip),
        Some("037833100"),
        "the derived one is still held, and answers by its own key"
    );
    assert_eq!(ids.of_kind(&IdType::Cusip).count(), 2);
}

#[test]
fn a_set_carries_what_it_admits_and_never_replaces_what_it_states() {
    let previous = fix(&[
        ("clordid", "A"),
        ("orderid", "O-1"),
        ("mdentryrefid", "R-1"),
    ]);
    let mut next = fix(&[("clordid", "B")]);
    assert!(next.carry(&previous, |held| held.kind() != &IdType::MdEntryRefId));
    assert_eq!(next.get(&IdType::OrderId), Some("O-1"));
    assert_eq!(next.get(&IdType::MdEntryRefId), None, "not admitted");
    assert_eq!(next.get(&IdType::ClOrdId), Some("B"), "a stated key stands");
    assert_eq!(next.len(), 2);
    assert!(
        !next.carry(&previous, |held| held.kind() != &IdType::MdEntryRefId),
        "nothing left to take"
    );
    // The predicate is asked of each identifier of the predecessor, in key
    // order.
    let offered = std::cell::RefCell::new(Vec::new());
    let mut empty = Identifiers::new();
    assert!(empty.carry(&previous, |held| {
        offered.borrow_mut().push(held.kind().to_string());
        true
    }));
    assert_eq!(
        *offered.borrow(),
        ["clordid", "mdentryrefid", "orderid"],
        "in key order"
    );
    assert_eq!(empty, previous);
    // A source states its own chain: the same type from another source is
    // another identifier, so a set stating one carries the other's.
    let mut other: Identifiers = [id("oms", "clordid", "X")].into_iter().collect();
    assert!(other.carry(&previous, |held| held.kind() == &IdType::ClOrdId));
    assert_eq!(
        other.get_from(&"oms".parse().unwrap(), &IdType::ClOrdId),
        Some("X")
    );
    assert_eq!(other.get_from(&IdSource::Fix, &IdType::ClOrdId), Some("A"));
    assert_eq!(other.len(), 2);
    // Nothing admitted, nothing moved.
    let mut none = fix(&[("clordid", "B")]);
    assert!(!none.carry(&previous, |_| false));
    assert_eq!(none.len(), 1);
}

#[test]
fn a_follower_takes_the_parents_of_the_bases_it_states_down_a_chain() {
    // `orderid` A, B, C, D: `parentorderid` is the value before the last
    // change, `origorderid` the chain's first.
    let orders = chain("orderid", &["A", "B", "C", "D"]);
    assert_eq!(
        orders[0].to_string(),
        "[fix:orderid=A]",
        "a first statement has no parent"
    );
    assert_eq!(
        orders[1].to_string(),
        "[fix:orderid=B, fix:origorderid=A, fix:parentorderid=A]"
    );
    assert_eq!(
        orders[2].to_string(),
        "[fix:orderid=C, fix:origorderid=A, fix:parentorderid=B]"
    );
    assert_eq!(
        orders[3].to_string(),
        "[fix:orderid=D, fix:origorderid=A, fix:parentorderid=C]"
    );
    let last = &orders[3];
    assert_eq!(
        last.get(&IdType::OrderId),
        Some("D"),
        "the base is the follower's own"
    );
    assert_eq!(last.get(&kind("parentorderid")), Some("C"));
    assert_eq!(
        last.get(&kind("origorderid")),
        Some("A"),
        "the chain's first, not the last step's"
    );

    // `clordid` A, B, C: FIX's `OrigClOrdID(41)` alone, the value before the
    // last change - a list of one has no first-of-chain to add.
    let clients = chain("clordid", &["A", "B", "C"]);
    assert_eq!(clients[0].to_string(), "[fix:clordid=A]");
    assert_eq!(clients[1].to_string(), "[fix:clordid=B, fix:origclordid=A]");
    assert_eq!(clients[2].to_string(), "[fix:clordid=C, fix:origclordid=B]");
    assert_eq!(clients[2].get(&IdType::OrigClOrdId), Some("B"));

    // Every base an element states follows by its own parents, in one pass.
    let previous = fix(&[("clordid", "A"), ("orderid", "O-1"), ("tradeid", "T-1")]);
    let mut next = fix(&[("clordid", "B"), ("orderid", "O-2"), ("tradeid", "T-1")]);
    assert!(follow(&mut next, &previous));
    assert_eq!(
        next.to_string(),
        "[fix:clordid=B, fix:orderid=O-2, fix:origclordid=A, fix:origorderid=O-1, \
         fix:parentorderid=O-1, fix:tradeid=T-1]",
        "an unchanged base over no parents has none"
    );
}

#[test]
fn an_unchanged_base_keeps_the_parents_of_the_step_before() {
    let orders = chain("orderid", &["A", "B", "C"]);
    // C again: each parent is the previous one, and a third statement is the
    // same set again.
    let mut again = fix(&[("orderid", "C")]);
    assert!(follow(&mut again, &orders[2]));
    assert_eq!(again, orders[2]);
    assert_eq!(again.get(&kind("parentorderid")), Some("B"));
    assert_eq!(again.get(&kind("origorderid")), Some("A"));
    assert!(!follow(&mut again, &orders[2]), "nothing left to fill");
    // The next change shifts from there, as if the repeat were not.
    let repeats = chain("orderid", &["A", "B", "B", "B", "C", "C", "D"]);
    assert_eq!(repeats[3], repeats[2]);
    assert_eq!(repeats[2].get(&kind("parentorderid")), Some("A"));
    assert_eq!(repeats[4].get(&kind("parentorderid")), Some("B"));
    assert_eq!(repeats[5], repeats[4]);
    let direct = chain("orderid", &["A", "B", "C", "D"]);
    assert_eq!(
        repeats[6], direct[3],
        "a repeated value moves nothing down the chain"
    );

    // Over a predecessor with no parents the base never changed: there is no
    // parent to inherit.
    let mut same = fix(&[("orderid", "A")]);
    assert!(!follow(&mut same, &fix(&[("orderid", "A")])));
    assert_eq!(same.len(), 1);
    let mut clients = fix(&[("clordid", "A")]);
    assert!(!follow(&mut clients, &fix(&[("clordid", "A")])));
    assert_eq!(clients.len(), 1);
}

#[test]
fn a_parent_the_follower_states_stands_and_only_an_absent_one_is_filled() {
    let orders = chain("orderid", &["A", "B", "C"]);
    // `parentorderid` stated: it stands, and the chain's first is filled.
    let mut stated = fix(&[("orderid", "D"), ("parentorderid", "X")]);
    assert!(follow(&mut stated, &orders[2]));
    assert_eq!(
        stated.to_string(),
        "[fix:orderid=D, fix:origorderid=A, fix:parentorderid=X]"
    );
    // `origorderid` stated: it stands, and the one before the last change is
    // filled.
    let mut origin = fix(&[("orderid", "D"), ("origorderid", "Y")]);
    assert!(follow(&mut origin, &orders[2]));
    assert_eq!(
        origin.to_string(),
        "[fix:orderid=D, fix:origorderid=Y, fix:parentorderid=C]"
    );
    // Both stated: nothing is absent.
    let mut both = fix(&[
        ("orderid", "D"),
        ("parentorderid", "X"),
        ("origorderid", "Y"),
    ]);
    assert!(!follow(&mut both, &orders[2]), "nothing absent");
    assert_eq!(both.len(), 3);
    assert_eq!(both.get(&kind("parentorderid")), Some("X"));
    assert_eq!(both.get(&kind("origorderid")), Some("Y"));

    // FIX's `OrigClOrdID(41)` states where a client order identifier came
    // from: the only parent there is, so there is nothing to fill.
    let first = fix(&[("clordid", "A")]);
    let mut replaced = fix(&[("clordid", "B"), ("origclordid", "Z")]);
    assert!(!follow(&mut replaced, &first));
    assert_eq!(replaced.get(&IdType::OrigClOrdId), Some("Z"));
    assert_eq!(replaced.len(), 2);
}

#[test]
fn a_follower_takes_parents_only_from_its_own_source_and_never_for_a_parent_type() {
    let previous: Identifiers = [
        id("fix", "orderid", "A"),
        id("venue", "orderid", "Q"),
        id("fix", "clordid", "C-1"),
    ]
    .into_iter()
    .collect();
    // Another source states another chain: the same type from a source the
    // predecessor did not state follows nothing of the others.
    let mut other: Identifiers = [id("oms", "clordid", "X"), id("oms", "orderid", "Y")]
        .into_iter()
        .collect();
    assert!(!follow(&mut other, &previous));
    assert_eq!(other.len(), 2);
    // Each source follows its own predecessor.
    let mut next: Identifiers = [id("fix", "orderid", "B"), id("venue", "orderid", "R")]
        .into_iter()
        .collect();
    assert!(follow(&mut next, &previous));
    assert_eq!(
        next.to_string(),
        "[fix:orderid=B, fix:origorderid=A, fix:parentorderid=A, \
         venue:orderid=R, venue:origorderid=Q, venue:parentorderid=Q]"
    );
    // A base the predecessor did not state has nothing to take, and a parent
    // type is no base: it is never given parents of its own.
    let mut quote = fix(&[("quoteid", "Q-2"), ("origclordid", "A")]);
    assert!(!follow(&mut quote, &previous));
    assert_eq!(quote.len(), 2);
    let mut chained = fix(&[("origclordid", "B")]);
    assert!(!follow(&mut chained, &fix(&[("origclordid", "A")])));
    assert_eq!(chained.len(), 1);
    let mut parented = fix(&[("parentorderid", "B")]);
    assert!(!follow(&mut parented, &fix(&[("parentorderid", "A")])));
    assert_eq!(parented.len(), 1);
    // An empty predecessor gives nothing, and an empty follower takes
    // nothing.
    assert!(!follow(&mut fix(&[("clordid", "B")]), &Identifiers::new()));
    assert!(!follow(&mut Identifiers::new(), &previous));
}

#[test]
fn a_parents_list_of_three_shifts_the_middle_one_and_ends_with_the_chains_first_value() {
    // The vocabulary is the caller's: three parents for `orderid`, nearest
    // first.
    let mut states: Vec<Identifiers> = Vec::new();
    for value in ["A", "B", "C", "D", "E"] {
        let mut next = fix(&[("orderid", value)]);
        if let Some(previous) = states.last() {
            assert!(next.follow_parents(previous, three_parents, three_parent_of));
        }
        states.push(next);
    }
    let held = |at: usize| states[at].to_string();
    assert_eq!(held(0), "[fix:orderid=A]");
    assert_eq!(held(1), "[fix:far=A, fix:near=A, fix:orderid=B]");
    assert_eq!(
        held(2),
        "[fix:far=A, fix:middle=A, fix:near=B, fix:orderid=C]"
    );
    assert_eq!(
        held(3),
        "[fix:far=A, fix:middle=B, fix:near=C, fix:orderid=D]"
    );
    assert_eq!(
        held(4),
        "[fix:far=A, fix:middle=C, fix:near=D, fix:orderid=E]",
        "the near one is the value before the change, each middle one the near one of the step before, the last the chain's first"
    );

    // An unchanged base takes each parent at its own place.
    let mut again = fix(&[("orderid", "E")]);
    assert!(again.follow_parents(&states[4], three_parents, three_parent_of));
    assert_eq!(again, states[4]);
    assert!(!again.follow_parents(&states[4], three_parents, three_parent_of));

    // A parent the follower states stands, wherever it is in the list.
    let mut stated = fix(&[("orderid", "D"), ("middle", "X")]);
    assert!(stated.follow_parents(&states[2], three_parents, three_parent_of));
    assert_eq!(
        stated.to_string(),
        "[fix:far=A, fix:middle=X, fix:near=C, fix:orderid=D]"
    );

    // The last parent is the previous last one, else the farthest the
    // previous statement made, else the previous value.
    let from_far = |previous: &[(&str, &str)]| {
        let mut next = fix(&[("orderid", "C")]);
        next.follow_parents(&fix(previous), three_parents, three_parent_of);
        next.get(&kind("far")).map(str::to_owned)
    };
    assert_eq!(
        from_far(&[("orderid", "B"), ("far", "F"), ("near", "N")]),
        Some("F".to_owned()),
        "the previous last parent"
    );
    assert_eq!(
        from_far(&[("orderid", "B"), ("middle", "M"), ("near", "N")]),
        Some("M".to_owned()),
        "the farthest stated"
    );
    assert_eq!(
        from_far(&[("orderid", "B"), ("near", "N")]),
        Some("N".to_owned()),
        "the farthest stated"
    );
    assert_eq!(
        from_far(&[("orderid", "B")]),
        Some("B".to_owned()),
        "the previous value"
    );
    // The middle one is the previous near one, or nothing.
    let mut next = fix(&[("orderid", "C")]);
    assert!(next.follow_parents(
        &fix(&[("orderid", "B"), ("far", "F")]),
        three_parents,
        three_parent_of
    ));
    assert_eq!(next.to_string(), "[fix:far=F, fix:near=B, fix:orderid=C]");

    // A base no list names has no parent, and a list of none fills nothing.
    let mut quote = fix(&[("quoteid", "Q-2")]);
    assert!(!quote.follow_parents(&fix(&[("quoteid", "Q-1")]), three_parents, three_parent_of));
    let mut none = fix(&[("orderid", "B")]);
    assert!(!none.follow_parents(&fix(&[("orderid", "A")]), |_| Cow::Borrowed(&[]), |_| None));
    assert_eq!(none.len(), 1);
}

#[test]
fn an_element_stating_a_parent_but_not_its_base_takes_the_base_from_its_nearest_parent() {
    // `OrigClOrdID(41)` without `ClOrdID(11)`: the element is what it came from.
    let mut ids = fix(&[("origclordid", "A")]);
    assert!(ids.fill_parents(IdType::parent_of));
    assert_eq!(ids.get(&IdType::ClOrdId), Some("A"));
    assert_eq!(ids.len(), 2, "the parent stays beside the base it filled");
    assert!(!ids.fill_parents(IdType::parent_of), "nothing left to fill");

    // A stated base stands, whatever its parents say.
    let mut stated = fix(&[("clordid", "B"), ("origclordid", "A")]);
    assert!(!stated.fill_parents(IdType::parent_of));
    assert_eq!(stated.get(&IdType::ClOrdId), Some("B"));

    // The nearest parent wins: `origorderid` sorts first in the set, and
    // `parentorderid`, which is nearer, still fills the base.
    let mut both = fix(&[("origorderid", "O"), ("parentorderid", "P")]);
    assert_eq!(keys(&both), ["fix:origorderid", "fix:parentorderid"]);
    assert!(both.fill_parents(IdType::parent_of));
    assert_eq!(both.get(&IdType::OrderId), Some("P"));
    assert_eq!(both.len(), 3);
    let mut origin = fix(&[("origorderid", "O")]);
    assert!(origin.fill_parents(IdType::parent_of));
    assert_eq!(
        origin.get(&IdType::OrderId),
        Some("O"),
        "the farthest alone fills it"
    );
    // The same over a list of three: the nearest stated, not the nearest named.
    let mut custom = fix(&[("far", "F"), ("middle", "M")]);
    assert!(custom.fill_parents(three_parent_of));
    assert_eq!(custom.get(&IdType::OrderId), Some("M"));
    let mut far = fix(&[("far", "F")]);
    assert!(far.fill_parents(three_parent_of));
    assert_eq!(far.get(&IdType::OrderId), Some("F"));

    // Each base is filled from its own parents, in one pass.
    let mut many = fix(&[
        ("origorderid", "O"),
        ("origclordid", "C"),
        ("origtradeid", "T"),
    ]);
    assert!(many.fill_parents(IdType::parent_of));
    assert_eq!(many.get(&IdType::OrderId), Some("O"));
    assert_eq!(many.get(&IdType::ClOrdId), Some("C"));
    assert_eq!(many.get(&IdType::TradeId), Some("T"));
    assert_eq!(many.len(), 6);

    // The base is filled under the parent's own source, so another source's
    // base does not count as stated.
    let mut sources: Identifiers = [
        id("fix", "origclordid", "A"),
        id("venue", "clordid", "B"),
        id("oms", "parentorderid", "P"),
    ]
    .into_iter()
    .collect();
    assert!(sources.fill_parents(IdType::parent_of));
    assert_eq!(
        sources.get_from(&IdSource::Fix, &IdType::ClOrdId),
        Some("A")
    );
    assert_eq!(
        sources.get_from(&source("venue"), &IdType::ClOrdId),
        Some("B")
    );
    assert_eq!(
        sources.get_from(&source("oms"), &IdType::OrderId),
        Some("P")
    );
    assert_eq!(sources.len(), 5);

    // A parent value its base refuses fills nothing, and a closure naming no
    // parent fills nothing.
    let mut refused = fix(&[("parentisin", "NOT-AN-ISIN")]);
    assert!(!refused.fill_parents(IdType::parent_of));
    assert_eq!(refused.len(), 1);
    let mut none = fix(&[("origclordid", "A")]);
    assert!(!none.fill_parents(|_| None));
    assert_eq!(none.len(), 1);
    assert!(!Identifiers::new().fill_parents(IdType::parent_of));
    // A base is no parent, so a set of bases fills nothing.
    let mut bases = fix(&[("orderid", "O"), ("clordid", "C")]);
    assert!(!bases.fill_parents(IdType::parent_of));
    assert_eq!(bases.len(), 2);
}

#[test]
fn an_identifier_moves_to_another_type_of_its_source_as_that_type_stores_it() {
    let held = id("fix", "origclordid", "A");
    assert_eq!(
        held.with_kind(IdType::ClOrdId).unwrap().to_string(),
        "fix:clordid=A"
    );
    let cusip = id("venue", "parentcusip", "037833100");
    assert_eq!(
        cusip.with_kind(IdType::Cusip).unwrap().to_string(),
        "venue:cusip=037833100"
    );
    let lower = id("fix", "house", "us0378331005");
    assert_eq!(
        lower.with_kind(IdType::Isin).unwrap().value(),
        "US0378331005"
    );
    assert!(
        id("fix", "house", "US0378331006")
            .with_kind(IdType::Isin)
            .is_err()
    );
    assert!(
        id("fix", "house", "037833100")
            .with_kind(IdType::Isin)
            .is_err()
    );
}

#[test]
fn a_set_merges_what_it_does_not_state_and_leaves_what_it_does() {
    let mut ids = Identifiers::new();
    ids.insert(id("fix", "orderid", "O-1"));
    let mut other = Identifiers::new();
    other.insert(id("fix", "orderid", "O-2"));
    other.insert(id("fix", "clordid", "C-1"));
    other.insert(id("venue", "orderid", "O-9"));
    assert!(ids.merge(&other));
    assert!(!ids.merge(&other), "nothing left to take");
    assert_eq!(ids.get_from(&IdSource::Fix, &IdType::OrderId), Some("O-1"));
    assert_eq!(ids.get_from(&IdSource::Fix, &IdType::ClOrdId), Some("C-1"));
    assert_eq!(
        ids.get_from(&"venue".parse().unwrap(), &IdType::OrderId),
        Some("O-9")
    );
    assert_eq!(ids.len(), 3);
}

#[test]
fn an_identifier_is_a_row_of_its_source_its_type_and_its_value() {
    let held = id("fix", "clordid", "B");
    let row = held.clone().into_scalar();
    let cells: Vec<_> = (0..3).map(|at| cell(&row, at)).collect();
    assert_eq!(cells, ["fix", "clordid", "B"]);
    assert!(row.get(3).is_none(), "three cells and no more");
    assert_eq!(Identifier::from_scalar(&row).unwrap(), held);
    // An upper-case word is read folded like any other, and the value as
    // its type stores it.
    let upper = Scalar::from_sequence([
        Scalar::from("FIX"),
        Scalar::from("ISIN"),
        Scalar::from("us0378331005"),
    ]);
    assert_eq!(
        Identifier::from_scalar(&upper).unwrap().to_string(),
        "fix:isin=US0378331005"
    );
    // A row that is not three text cells, or whose cells are refused, is refused.
    assert!(
        Identifier::from_scalar(&Scalar::from_sequence([
            Scalar::from("fix"),
            Scalar::from("isin")
        ]))
        .is_err()
    );
    assert!(Identifier::from_scalar(&Scalar::from("fix:isin=US0378331005")).is_err());
    assert!(
        Identifier::from_scalar(&Scalar::from_sequence([
            Scalar::from("fix"),
            Scalar::from("isin"),
            Scalar::from("US0378331006"),
        ]))
        .is_err(),
        "a check digit"
    );
    assert!(
        Identifier::from_scalar(&Scalar::from_sequence([
            Scalar::from("fix"),
            Scalar::from("clordid"),
            Scalar::from("null"),
        ]))
        .is_err()
    );

    // Under its field the row crosses a column and back as the same value.
    let field = yggdryl::Field::new("id", Identifier::dtype(), false);
    let serie = yggdryl::Serie::from_scalars(std::sync::Arc::new(field), [row]).unwrap();
    assert_eq!(
        Identifier::from_scalar(&serie.scalar(0).unwrap()).unwrap(),
        held
    );
}

#[test]
fn a_set_is_a_sorted_map_scalar_keyed_src_type_in_key_order() {
    let ids: Identifiers = [
        id("fix", "isin", "US0378331005"),
        id("fix", "clordid", "B"),
        id("a", "z", "1"),
        id("a.b", "c", "2"),
        id("a1", "x", "3"),
    ]
    .into_iter()
    .collect();
    let scalar = ids.into_scalar();
    assert!(matches!(scalar, Scalar::SortedMap(_)), "{}", scalar.kind());
    let entries = scalar.as_mapping().expect("a map's entries");
    let held: Vec<_> = entries
        .iter()
        .map(|(key, _)| key.as_str().expect("a text key"))
        .collect();
    assert_eq!(held, ["a.b:c", "a1:x", "a:z", "fix:clordid", "fix:isin"]);
    // Each key is the unique key of the row it names, and each row three text
    // cells, `src`, `type` and `value`.
    assert_eq!(entries.len(), ids.len());
    for ((key, row), expected) in entries.iter().zip(&ids) {
        assert_eq!(key.as_str(), Some(expected.key().as_str()));
        assert_eq!(Identifier::from_scalar(row).unwrap(), *expected);
        let cells: Vec<_> = (0..3).map(|at| cell(row, at)).collect();
        assert_eq!(
            cells,
            [
                expected.src().as_str(),
                expected.kind().as_str(),
                expected.value()
            ]
        );
        assert!(row.get(3).is_none(), "three cells and no more");
    }
    assert_eq!(Identifiers::from_scalar(&scalar).unwrap(), ids);

    // An empty set is an empty sorted map, not a null and not a run.
    let empty = Identifiers::new().into_scalar();
    assert!(matches!(empty, Scalar::SortedMap(_)), "{}", empty.kind());
    assert_eq!(empty.as_mapping().map(<[_]>::len), Some(0));
    assert_eq!(
        Identifiers::from_scalar(&empty).unwrap(),
        Identifiers::new()
    );
}

#[test]
fn a_set_reads_back_from_a_map_or_a_sorted_map_in_any_order() {
    let ids: Identifiers = [
        id("fix", "isin", "US0378331005"),
        id("fix", "clordid", "B"),
        id("a", "z", "1"),
    ]
    .into_iter()
    .collect();
    // Entries in an order of their own, under a plain map.
    let plain = Scalar::from_mapping([
        (
            Scalar::from("fix:isin"),
            id("fix", "isin", "US0378331005").into_scalar(),
        ),
        (Scalar::from("a:z"), id("a", "z", "1").into_scalar()),
        (
            Scalar::from("fix:clordid"),
            id("fix", "clordid", "B").into_scalar(),
        ),
    ])
    .unwrap();
    assert!(matches!(plain, Scalar::Map(_)));
    assert_eq!(Identifiers::from_scalar(&plain).unwrap(), ids);
    assert_eq!(
        Identifiers::from_scalar(&plain).unwrap().to_string(),
        "[a:z=1, fix:clordid=B, fix:isin=US0378331005]",
        "the set sorts what it reads"
    );
    // The same entries as a sorted map, and an empty plain map.
    let Scalar::Map(entries) = plain.clone() else {
        panic!("a plain map");
    };
    assert_eq!(
        Identifiers::from_scalar(&Scalar::SortedMap(entries)).unwrap(),
        ids
    );
    assert_eq!(
        Identifiers::from_scalar(&Scalar::from_mapping([]).unwrap()).unwrap(),
        Identifiers::new()
    );
    // A word is read folded, a value as its type stores it.
    let upper = Scalar::from_mapping([(
        Scalar::from("fix:isin"),
        Scalar::from_sequence([
            Scalar::from("FIX"),
            Scalar::from("ISIN"),
            Scalar::from("us0378331005"),
        ]),
    )])
    .unwrap();
    assert_eq!(
        Identifiers::from_scalar(&upper).unwrap().to_string(),
        "[fix:isin=US0378331005]"
    );
}

#[test]
fn a_map_keyed_other_than_its_rows_is_refused_on_that_key() {
    let located = |entries: Vec<(Scalar, Scalar)>| {
        Identifiers::from_scalar(&Scalar::from_mapping(entries).unwrap())
            .unwrap_err()
            .to_string()
    };
    let clordid = || id("fix", "clordid", "B").into_scalar();
    // A key naming another identity than its row.
    let refused = located(vec![(Scalar::from("fix:orderid"), clordid())]);
    assert!(refused.contains("$['fix:orderid']"), "{refused}");
    assert!(
        refused.contains("expected the key fix:clordid"),
        "{refused}"
    );
    // Only the folded spelling is the key: one identity has one key, so a
    // second spelling of it is refused and never read as a second row.
    let refused = located(vec![
        (
            Scalar::from("fix:isin"),
            id("fix", "isin", "US0378331005").into_scalar(),
        ),
        (
            Scalar::from("FIX:ISIN"),
            id("fix", "isin", "CH0012214059").into_scalar(),
        ),
    ]);
    assert!(refused.contains("$['FIX:ISIN']"), "{refused}");
    assert!(refused.contains("expected the key fix:isin"), "{refused}");
    let refused = located(vec![(Scalar::from("fix:clordid "), clordid())]);
    assert!(refused.contains("$['fix:clordid ']"), "{refused}");
    // A key that is no text names no row.
    assert!(
        Identifiers::from_scalar(
            &Scalar::from_mapping([(Scalar::from(1_i64), clordid())]).unwrap()
        )
        .is_err()
    );

    // A row an identifier refuses is located on its key: the check digit, a
    // value that states nothing, a word that is none, a missing cell.
    let row = |cells: &[&str]| Scalar::from_sequence(cells.iter().map(|text| Scalar::from(*text)));
    for (key, cells) in [
        ("fix:isin", row(&["fix", "isin", "US0378331006"])),
        ("fix:clordid", row(&["fix", "clordid", "null"])),
        ("fix:clordid", row(&["fix", "clordid", "  "])),
        ("fix:clordid", row(&["fix", "cl/ordid", "B"])),
        ("fix:clordid", row(&["fix", "clordid"])),
    ] {
        let refused = located(vec![(Scalar::from(key), cells)]);
        assert!(refused.contains(&format!("$['{key}']")), "{refused}");
    }

    // Two rows under one key never reach a set: the map refuses them as it
    // is built.
    assert!(
        Scalar::from_mapping([
            (
                Scalar::from("fix:isin"),
                id("fix", "isin", "US0378331005").into_scalar()
            ),
            (
                Scalar::from("fix:isin"),
                id("fix", "isin", "CH0012214059").into_scalar()
            ),
        ])
        .is_err()
    );
    // What is neither a map nor a sequence of identifier rows is refused,
    // one bare row included.
    assert!(Identifiers::from_scalar(&Scalar::from("x")).is_err());
    assert!(Identifiers::from_scalar(&clordid()).is_err());
}

#[test]
fn a_sequence_of_identifier_rows_reads_as_the_map_it_makes() {
    let clordid = || id("fix", "clordid", "B").into_scalar();
    let isin = id("fix", "isin", "US0378331005");
    let rows = Scalar::from_sequence([clordid(), isin.clone().into_scalar(), clordid()]);
    let read = Identifiers::from_scalar(&rows).expect("a sequence of rows");
    assert_eq!(read.len(), 2, "one identifier stated twice is one");
    assert_eq!(read.get_from(isin.src(), isin.kind()), Some("US0378331005"));
    assert_eq!(
        Identifiers::from_scalar(&Scalar::from_sequence([]))
            .unwrap()
            .len(),
        0
    );
    // Two values under one key are two readings, refused at the second.
    let other = Scalar::from_sequence([
        isin.into_scalar(),
        id("fix", "isin", "CH0012214059").into_scalar(),
    ]);
    let refused = Identifiers::from_scalar(&other).expect_err("one key, two values");
    assert!(refused.to_string().contains("$[1]"), "{refused}");
    // A cell that is no identifier row is refused at its place.
    let refused = Identifiers::from_scalar(&Scalar::from_sequence([Scalar::from("x")]))
        .expect_err("no identifier row");
    assert!(refused.to_string().contains("$[0]"), "{refused}");
}

#[test]
fn a_set_crosses_a_column_under_its_dtype_and_back() {
    let ids: Identifiers = [
        id("fix", "isin", "US0378331005"),
        id("fix", "clordid", "B"),
        id("a.b", "c", "2"),
        id("a1", "x", "3"),
    ]
    .into_iter()
    .collect();
    let field = Arc::new(Field::new("ids", Identifiers::dtype("identifier"), true));
    let serie = Serie::from_scalars(
        Arc::clone(&field),
        [
            ids.into_scalar(),
            Identifiers::new().into_scalar(),
            ids.into_scalar(),
        ],
    )
    .unwrap();
    assert_eq!(serie.len(), 3);
    for (row, expected) in [(0, &ids), (1, &Identifiers::new()), (2, &ids)] {
        let read = Identifiers::from_scalar(&serie.scalar(row).unwrap()).unwrap();
        assert_eq!(&read, expected, "row {row}");
    }

    // Through Arrow: a map whose keys are declared sorted, laid out as the
    // field states it, and read back under the same field as the same rows.
    let array = serie.clone().into_arrow_array().expect("a column");
    assert!(
        matches!(array.data_type(), arrow_schema::DataType::Map(_, true)),
        "{:?}",
        array.data_type()
    );
    let back = Serie::from_arrow_array(Some(&field), array, ArrowCastOptions::new()).unwrap();
    for (row, expected) in [(0, &ids), (1, &Identifiers::new()), (2, &ids)] {
        let read = Identifiers::from_scalar(&back.scalar(row).unwrap()).unwrap();
        assert_eq!(&read, expected, "row {row} after Arrow");
    }
}

#[test]
fn the_row_and_the_set_are_laid_out_as_the_columns_state_them() {
    let row = Identifier::dtype();
    let columns = row.as_fields().expect("a struct row");
    let names: Vec<_> = columns.iter().map(Field::name).collect();
    assert_eq!(
        names,
        ["src", "type", "value"],
        "struct<src, type, value> and nothing else"
    );
    assert!(columns.iter().all(|field| !field.is_nullable()));
    assert!(
        columns
            .iter()
            .all(|field| field.dtype() == &DataType::utf8())
    );
    for item in ["securityid", "identifier", "partyid"] {
        let set = Identifiers::dtype(item);
        // A map whose keys are held sorted, from the key text to the row.
        assert!(matches!(set, DataType::SortedMap(_)), "{set}");
        assert!(set.as_mapping().expect("a map").keys_sorted());
        let entries = set.map_entries().expect("a map's entries");
        assert_eq!(entries.name(), "entries");
        assert!(!entries.is_nullable());
        let pair = entries.dtype().as_fields().expect("a struct of two");
        let names: Vec<_> = pair.iter().map(Field::name).collect();
        assert_eq!(names, ["key", item]);
        assert!(!pair[0].is_nullable());
        assert_eq!(pair[0].dtype(), &DataType::utf8());
        assert!(!pair[1].is_nullable());
        assert_eq!(pair[1].dtype(), &Identifier::dtype());
    }
    assert_ne!(
        Identifiers::dtype("securityid"),
        Identifiers::dtype("partyid"),
        "the item names the set"
    );
}
