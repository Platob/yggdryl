//! `rust/src/idkey.rs`: the key of an identifier - its source and its type -
//! spelled `src:type`, a key from the base source as its type alone, read
//! back from that spelling exactly and ordered by it.

use std::collections::HashSet;

use smol_str::SmolStr;
use yggdryl::{IdKey, IdSource, IdType, Identifier};

fn key(text: &str) -> IdKey {
    text.parse()
        .unwrap_or_else(|error| panic!("{text:?}: {error}"))
}

#[test]
fn every_key_of_two_member_words_round_trips_its_spelling_a_base_key_bare() {
    let mut spelled = HashSet::new();
    for src in &IdSource::KNOWN {
        for kind in &IdType::KNOWN {
            let held = IdKey::new(src.clone(), kind.clone());
            let text = held.to_string();
            let expected = if *src == IdSource::Base {
                kind.as_str().to_owned()
            } else {
                format!("{src}:{kind}")
            };
            assert_eq!(text, expected);
            assert_eq!(key(&text), held, "{text} reads back");
            assert_eq!(SmolStr::from(&held), text);
            assert!(held == *text.as_str(), "{text}: compared without building");
            assert_eq!(held.is_base(), *src == IdSource::Base, "{text}");
            assert!(spelled.insert(text), "one spelling per key");
        }
    }
    assert_eq!(spelled.len(), IdSource::KNOWN.len() * IdType::KNOWN.len());
}

#[test]
fn a_key_is_read_exactly_and_a_bare_word_is_a_base_key() {
    let isin = IdKey::base(IdType::Isin);
    for spelling in [
        "isin",
        "ISIN",
        " Isin ",
        "ISIN_Number",
        "isincode",
        "base:isin",
        "BASE:ISIN",
        "Base:Isin_Number",
        "fix:isin",
        "FIX:ISIN",
    ] {
        assert_eq!(key(spelling), isin, "{spelling:?}");
    }
    assert_eq!(isin.to_string(), "isin");
    assert_eq!(isin.src(), &IdSource::Base);
    assert_eq!(isin.kind(), &IdType::Isin);
    assert_eq!(
        key("fix:clordid"),
        IdKey::base(IdType::ClOrdId),
        "the standard is the base"
    );
    // A derivation and a named source keep the source they spell.
    let derived = key("DERIVED:Cusip");
    assert_eq!(derived, IdKey::new(IdSource::Derived, IdType::Cusip));
    assert_eq!(derived.to_string(), "derived:cusip");
    assert!(!derived.is_base());
    let bridged = key("ULLINK:ISIN");
    assert_eq!(bridged.to_string(), "ullink:isin");
    assert_eq!(bridged.src(), "ullink");
    assert_eq!(bridged.kind(), &IdType::Isin);
    assert_eq!(
        key("firm.x:House Code").to_string(),
        "firm.x:housecode",
        "each word folds as its own vocabulary does"
    );
    // Nothing is inferred: a word no member names is that type from the base
    // source, and it reads back as itself.
    let other = key("marketorderid");
    assert_eq!(other, IdKey::base("marketorderid".parse().unwrap()));
    assert_eq!(other.to_string(), "marketorderid");
    assert_eq!(key(&other.to_string()), other);
    assert_eq!(
        Identifier::from_key("marketorderid", "O-1")
            .unwrap()
            .key()
            .to_string(),
        "market:orderid",
        "the inferred reading is from_key's alone"
    );
}

#[test]
fn a_text_no_key_spells_is_refused_and_names_it() {
    for refused in [
        "",
        "   ",
        ":",
        "fix:",
        ":isin",
        "a:b:c",
        "a/b:isin",
        "isin:a/b",
        "caf\u{e9}",
        "is=in",
        "_-#",
    ] {
        let error = refused.parse::<IdKey>().expect_err(refused).to_string();
        assert!(
            error.contains("expected an identifier key src:type or type"),
            "{refused:?}: {error}"
        );
    }
    let error = "fix:".parse::<IdKey>().unwrap_err().to_string();
    assert!(error.contains("got \"fix:\""), "{error}");
    // Each word holds a word's width, and no more.
    let widest = "s".repeat(64);
    assert_eq!(
        key(&format!("{widest}:{widest}")).to_string(),
        format!("{widest}:{widest}")
    );
    assert!(format!("{widest}s:isin").parse::<IdKey>().is_err());
    assert!(format!("oms:{widest}s").parse::<IdKey>().is_err());
}

#[test]
fn keys_order_by_their_spelling_a_base_key_spelled_bare() {
    let ordered = [
        "a.b:c",
        "a1:x",
        "a:x",
        "a:z",
        "ab:a",
        "cusip",
        "derived:cusip",
        "isin",
        "oms:instrumentid",
        "ullink:isin",
        "z",
    ];
    let mut keys: Vec<IdKey> = ordered.iter().rev().map(|text| key(text)).collect();
    keys.sort();
    assert_eq!(
        keys.iter().map(ToString::to_string).collect::<Vec<_>>(),
        ordered
    );
    // The order is the spelling's, never the pair's: a source going on past
    // a shorter one with a byte below ':' sorts before the shorter one's
    // types, and a base key sorts as its type alone.
    assert!(key("a.b:c") < key("a:z"));
    assert!(key("a1:x") < key("a:x"));
    assert!(key("a:z") < key("ab:a"), "':' sorts before a letter");
    assert!(key("isin") < key("ullink:isin"));
    assert!(key("derived:cusip") < key("isin"));
    assert!(key("ric") < key("ullink:isin") && key("ullink:isin") < key("valor"));
    for pair in keys.windows(2) {
        assert_eq!(
            pair[0].cmp(&pair[1]),
            pair[0].to_string().cmp(&pair[1].to_string())
        );
    }
    assert_eq!(
        key("isin").cmp(&key("BASE:ISIN")),
        std::cmp::Ordering::Equal
    );
}

#[test]
fn two_spellings_of_one_key_are_one_value_and_one_hash() {
    let mut held = HashSet::new();
    assert!(held.insert(key("isin")));
    assert!(!held.insert(key("BASE:ISIN_Number")));
    assert!(!held.insert(key("fix:isin")));
    assert!(held.insert(key("derived:isin")));
    assert!(held.insert(key("ullink:isin")));
    assert_eq!(held.len(), 3);
    assert_eq!(key("isin"), "isin");
    assert_eq!(key("isin"), *"isin");
    assert!(
        key("isin") != "base:isin",
        "a key compares with its spelling"
    );
    assert!(key("ullink:isin") != "ullink:isinx");
    assert!(key("ullink:isin") != "ullink:");
}

#[test]
fn a_key_moves_to_another_type_of_its_source() {
    let bridged = key("oms:orderid");
    assert_eq!(
        bridged
            .with_kind("parentorderid".parse().unwrap())
            .to_string(),
        "oms:parentorderid"
    );
    assert_eq!(
        IdKey::base(IdType::OrderId)
            .with_kind(IdType::ClOrdId)
            .to_string(),
        "clordid"
    );
    let identifier = Identifier::new(bridged.clone(), "O-1").unwrap();
    assert_eq!(identifier.key(), &bridged);
    assert_eq!(identifier.key(), "oms:orderid");
    assert_eq!(identifier.src(), bridged.src());
    assert_eq!(identifier.kind(), bridged.kind());
}

#[test]
fn a_key_of_two_member_words_is_one_static_string_and_any_other_is_spelled() {
    // A long pair of member words is past `SmolStr`'s inline width, and is
    // still borrowed from the one table every process spells once.
    let long = IdKey::new(IdSource::Proprietary, IdType::ExecutingTrader);
    let spelled = SmolStr::from(&long);
    assert_eq!(spelled, "proprietary:executingtrader");
    assert!(spelled.len() > 23);
    assert!(!spelled.is_heap_allocated(), "a static string");
    assert!(!SmolStr::from(&IdKey::base(IdType::SecondaryIndividualAllocId)).is_heap_allocated());
    // A word no member names is spelled: inline while it fits, on the heap
    // past it.
    let short = key("oms:orderid");
    assert!(!SmolStr::from(&short).is_heap_allocated());
    let other = key("venue.desk.bridge:secondaryindividualallocid");
    let spelled = SmolStr::from(&other);
    assert_eq!(spelled, "venue.desk.bridge:secondaryindividualallocid");
    assert!(spelled.is_heap_allocated());
    let widest = "s".repeat(64);
    let widest_key = key(&format!("{widest}:{widest}"));
    assert_eq!(SmolStr::from(&widest_key).len(), 129);
}
