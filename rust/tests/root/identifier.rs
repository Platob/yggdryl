//! `rust/src/identifier.rs`: one identifier - a value under its key, the
//! source that gave it and the type of name it is - the sorted map of them,
//! whose base key is each type's answer, laid out as an Arrow
//! `map<utf8, utf8>` from the key as spelled to its value, and the parents a
//! chain gives the types it states.

use std::borrow::Cow;
use std::sync::Arc;

use arrow_array::Array;
use yggdryl::{
    ArrowCastOptions, DataType, Field, IdKey, IdSource, IdType, IdWord, Identifier, Identifiers,
    Scalar, Serie,
};

fn id(src: &str, kind: &str, value: &str) -> Identifier {
    Identifier::new(
        IdKey::new(src.parse().unwrap(), kind.parse().unwrap()),
        value,
    )
    .unwrap()
}

fn key(text: &str) -> IdKey {
    text.parse().unwrap()
}

fn kind(text: &str) -> IdType {
    text.parse().unwrap()
}

fn source(text: &str) -> IdSource {
    text.parse().unwrap()
}

/// A map of base identifiers, one per `(type, value)` pair.
fn base(pairs: &[(&str, &str)]) -> Identifiers {
    pairs
        .iter()
        .map(|(kind, value)| id("base", kind, value))
        .collect()
}

/// What a chain's follower takes from its predecessor, under the crate's own
/// parentage vocabulary.
fn follow(next: &mut Identifiers, previous: &Identifiers) -> bool {
    next.follow_parents(previous, IdType::parents, IdType::parent_of)
}

/// The states of one chain, in order: each states one of `values` as the
/// base key of `kind`, and takes its parents from the state before it as a
/// lifecycle does.
fn chain(kind: &str, values: &[&str]) -> Vec<Identifiers> {
    let mut states: Vec<Identifiers> = Vec::new();
    for value in values {
        let mut next = base(&[(kind, value)]);
        if let Some(previous) = states.last() {
            follow(&mut next, previous);
        }
        states.push(next);
    }
    states
}

/// The keys of `ids` as spelled, in the order the map holds them.
fn keys(ids: &Identifiers) -> Vec<String> {
    ids.iter().map(|held| held.key().to_string()).collect()
}

/// A map of text keys to text values, in the order given.
fn entries(pairs: &[(&str, &str)]) -> Scalar {
    Scalar::from_mapping(
        pairs
            .iter()
            .map(|(key, value)| (Scalar::from(*key), Scalar::from(*value))),
    )
    .unwrap()
}

/// A parents list of three, nearest first, for `orderid` alone: the
/// vocabulary is the caller's, `IdType::parents` is one reading of it.
fn three_parents(kind: &IdType) -> Cow<'static, [IdType]> {
    if kind == &IdType::OrderId {
        Cow::Owned(["near", "middle", "far"].map(self::kind).to_vec())
    } else {
        Cow::Borrowed(&[])
    }
}

/// The type and place each of the three custom parents is a parent of.
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
            Identifier::new(IdKey::base(IdType::OrderId), value).is_err(),
            "{value:?}"
        );
        assert!(
            Identifier::new(IdKey::base(IdType::Isin), value).is_err(),
            "{value:?}"
        );
    }
    let refused = Identifier::new(IdKey::base(IdType::OrderId), "n/a")
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
    let long = Identifier::new(IdKey::base(longest.parse().unwrap()), &"v".repeat(64)).unwrap();
    assert_eq!(long.kind().as_str(), "x".repeat(64));
    assert_eq!(long.value().len(), 64);
    let refused = "X".repeat(65).parse::<IdType>().unwrap_err().to_string();
    assert!(refused.contains("at most 64 bytes"), "{refused}");
    assert!(
        Identifier::new(IdKey::base(IdType::OrderId), &"9".repeat(65)).is_err(),
        "a value past 64 bytes"
    );
    assert!(Identifier::new(IdKey::base(IdType::Isin), &"9".repeat(65)).is_err());
}

/// Two sources are standards whose every value is a registered code: a
/// value under `bic` is a BIC and one under `legalentityidentifier` an LEI,
/// whatever type of name it is - a party's role, the account, a word no
/// member names - refused by its shape and located on the key, held
/// upper-cased where it is one. Every other source keeps its type's rule
/// alone.
#[test]
fn a_value_under_the_bic_or_lei_source_is_refused_where_it_is_not_that_code() {
    let refused = |key: &str, value: &str| {
        Identifier::new(self::key(key), value)
            .unwrap_err()
            .to_string()
    };
    assert_eq!(
        refused("bic:executingtrader", " T-1 "),
        "invalid record value at bic:executingtrader: a value under the bic source is a BIC: \
         expected eight or eleven characters, got \"T-1\""
    );
    assert_eq!(
        refused("legalentityidentifier:clientid", "CL"),
        "invalid record value at legalentityidentifier:clientid: a value under the \
         legalentityidentifier source is an LEI: expected twenty characters, got \"CL\""
    );
    let wide = refused("bic:account", "ACCOUNT-0001");
    assert!(
        wide.starts_with(
            "invalid record value at bic:account: a value under the bic source is a BIC: "
        ),
        "{wide}"
    );
    let foreign = refused("bic:executingfirm", "D\u{c9}UTDEFF");
    assert!(
        foreign.starts_with("invalid record value at bic:executingfirm: "),
        "{foreign}"
    );
    // A word no member names is checked too: FIX's unnamed roles are such
    // words, and the source is the standard of the value.
    assert!(!kind("partyrole99").is_party());
    assert!(Identifier::new(key("bic:partyrole99"), "ACC-1").is_err());
    assert!(Identifier::new(key("legalentityidentifier:partyrole99"), "ACC-1").is_err());
    // A key whose type has a rule of its own holds the value to both: an
    // ISIN is no BIC, and an LEI's type and source agree.
    assert!(Identifier::new(key("bic:isin"), "US0378331005").is_err());
    assert!(Identifier::new(key("legalentityidentifier:lei"), "HWUPKR0MPOU8FGXBT3").is_err());
    assert_eq!(
        id("legalentityidentifier", "lei", "hwupkr0mpou8fgxbt394").value(),
        "HWUPKR0MPOU8FGXBT394"
    );
    // Held as the code stores it: upper-cased, eight and eleven as stated,
    // a check digit that does not close and an unlisted country admitted.
    assert_eq!(
        id("bic", "executingfirm", " deutdeff500 ").to_string(),
        "bic:executingfirm=DEUTDEFF500"
    );
    assert_eq!(id("bic", "account", "DEUTDEFF").value(), "DEUTDEFF");
    assert_eq!(id("bic", "account", "deutzzff").value(), "DEUTZZFF");
    assert_eq!(
        id("legalentityidentifier", "clientid", "HWUPKR0MPOU8FGXBT395").value(),
        "HWUPKR0MPOU8FGXBT395"
    );
    // Every other source keeps the type's rule alone, case included.
    assert_eq!(id("proprietary", "executingtrader", "T-1").value(), "T-1");
    assert_eq!(id("base", "executingfirm", "deutdeff").value(), "deutdeff");
    assert_eq!(id("oms", "account", "acc-1").value(), "acc-1");
    // A type change goes through the same door, the source checked again.
    let firm = id("bic", "executingfirm", "DEUTDEFF");
    assert_eq!(
        firm.with_kind(kind("clientid")).unwrap().to_string(),
        "bic:clientid=DEUTDEFF"
    );
    // A bridge's key whose namespace folds to the source is held to it.
    assert!(Identifier::from_key("BIC_ClOrdID", "C-1").is_none());
    assert_eq!(
        Identifier::from_key("BIC_ClOrdID", "deutdeff")
            .unwrap()
            .to_string(),
        "bic:clordid=DEUTDEFF"
    );
}

/// An identifier under `bic` or `legalentityidentifier` ranks by the lower
/// of its type's rank and its code's - a BIC's listed country, an LEI's
/// closing check digits - so a real code replaces a typo or an unassigned
/// country under its key whichever was stated first; the base key it fills
/// ranks as the statements holding its value, so it follows the real code
/// too.
#[test]
fn a_bic_or_lei_ranks_by_its_code_and_a_real_one_replaces_another_whatever_the_order() {
    const CLOSING: &str = "HWUPKR0MPOU8FGXBT394";
    const TYPO: &str = "HWUPKR0MPOU8FGXBT395";
    const OTHER: &str = "7LTWFZYICNSX8D621K86";
    let lei = |value: &str| id("legalentityidentifier", "clientid", value);
    let named = key("legalentityidentifier:clientid");

    let mut held: Identifiers = [lei(TYPO)].into_iter().collect();
    assert!(held.insert(lei(CLOSING)), "a closing code replaces a typo");
    assert_eq!(held.get_from(&named), Some(CLOSING));
    assert!(!held.insert(lei(TYPO)), "and is never replaced by it");
    assert!(!held.insert(lei(OTHER)), "two closing codes keep the first");
    assert_eq!(held.get_from(&named), Some(CLOSING));

    // A merge keeps the closing code under its key and as the answer
    // whichever map leads: the base key ranks as the code it holds.
    for later in [false, true] {
        let mut held: Identifiers = [lei(TYPO)].into_iter().collect();
        assert!(held.merge(&[lei(CLOSING)].into_iter().collect(), later));
        assert_eq!(held.get_from(&named), Some(CLOSING), "{later}");
        assert_eq!(held.get(&IdType::ClientId), Some(CLOSING), "{later}");
        let mut held: Identifiers = [lei(CLOSING)].into_iter().collect();
        assert!(!held.merge(&[lei(TYPO)].into_iter().collect(), later));
        assert_eq!(held.get_from(&named), Some(CLOSING), "{later}");
        assert_eq!(held.get(&IdType::ClientId), Some(CLOSING), "{later}");
    }

    // A BIC of a country ISO 3166 does not list ranks below a listed one.
    let bic = |value: &str| id("bic", "executingfirm", value);
    let firm = key("bic:executingfirm");
    let mut held: Identifiers = [bic("DEUTZZFF")].into_iter().collect();
    assert!(held.insert(bic("DEUTDEFF")));
    assert_eq!(held.get_from(&firm), Some("DEUTDEFF"));
    assert!(!held.insert(bic("DEUTZZFF")));
    // SWIFT's Kosovo is as real as a listed country.
    let mut held: Identifiers = [bic("BKOSXKPR")].into_iter().collect();
    assert!(
        !held.insert(bic("DEUTDEFF")),
        "two real codes keep the first"
    );

    // A map read raw closes the base key on the highest-ranked source.
    let closed = Identifiers::from_scalar(&entries(&[
        ("abc:clientid", "C-1"),
        ("legalentityidentifier:clientid", TYPO),
    ]))
    .unwrap();
    assert_eq!(
        closed.get(&IdType::ClientId),
        Some("C-1"),
        "the first among equals"
    );
    let closed = Identifiers::from_scalar(&entries(&[
        ("legalentityidentifier:clientid", TYPO),
        ("zzz:clientid", "C-1"),
    ]))
    .unwrap();
    assert_eq!(
        closed.get(&IdType::ClientId),
        Some("C-1"),
        "a typo ranks below a word"
    );

    // The base key a named code filled ranks as that code: a later real
    // code under the same key replaces the key and the answer with it.
    let mut held: Identifiers = [lei(TYPO)].into_iter().collect();
    assert_eq!(held.get(&IdType::ClientId), Some(TYPO));
    assert!(held.insert(lei(CLOSING)));
    assert_eq!(held.get(&IdType::ClientId), Some(CLOSING));
    assert_eq!(keys(&held), ["clientid", "legalentityidentifier:clientid"]);
}

/// The base key a named code fills ranks as the statements holding its
/// value - the highest rank among the keys of its type stating it, its
/// type's alone where none does - so a BIC of a listed country and an LEI
/// whose check digits close replace a typo as the type's answer too,
/// whichever was stated first, inserted or merged; two values of one rank
/// keep the order's rule.
#[test]
fn the_base_key_a_named_code_fills_follows_the_real_code_whatever_the_order() {
    const LISTED: &str = "DEUTDEFF";
    const UNLISTED: &str = "ABCDZZ11";
    const CLOSING: &str = "5493001KJTIIGC8Y1R12";
    const TYPO: &str = "5493001KJTIIGC8Y1R13";
    let bic = |value: &str| id("bic", "executingfirm", value);
    let cases = [
        (
            bic as fn(&str) -> Identifier,
            IdType::ExecutingFirm,
            UNLISTED,
            LISTED,
        ),
        (
            |value| id("legalentityidentifier", "account", value),
            IdType::Account,
            TYPO,
            CLOSING,
        ),
    ];
    for (code, kind, placeholder, real) in cases {
        // Inserted, in either order.
        let mut held = Identifiers::new();
        assert!(held.insert(code(placeholder)));
        assert_eq!(held.get(&kind), Some(placeholder), "{kind}");
        assert!(
            held.insert(code(real)),
            "{kind}: the real code replaces the typo"
        );
        assert_eq!(
            held.get(&kind),
            Some(real),
            "{kind}: and the answer with it"
        );
        assert_eq!(held.len(), 2, "{kind}: {held}");
        let mut held = Identifiers::new();
        assert!(held.insert(code(real)));
        assert!(
            !held.insert(code(placeholder)),
            "{kind}: never replaced by the typo"
        );
        assert_eq!(held.get(&kind), Some(real), "{kind}");
        assert!(!held.insert(Identifier::new(IdKey::base(kind.clone()), placeholder).unwrap()));
        assert_eq!(
            held.get(&kind),
            Some(real),
            "{kind}: nor by its base statement"
        );

        // Merged, either map leading, either later.
        for later in [false, true] {
            let mut held: Identifiers = [code(placeholder)].into_iter().collect();
            assert!(held.merge(&[code(real)].into_iter().collect(), later));
            assert_eq!(held.get(&kind), Some(real), "{kind} {later}");
            let mut held: Identifiers = [code(real)].into_iter().collect();
            assert!(
                !held.merge(&[code(placeholder)].into_iter().collect(), later),
                "{kind} {later}"
            );
            assert_eq!(held.get(&kind), Some(real), "{kind} {later}");
            assert_eq!(held.len(), 2, "{kind} {later}: {held}");
        }
    }

    // A real code replaces a typo another code source filled the answer
    // with: the answer ranks as the typo, whoever stated it.
    let mut held: Identifiers = [bic(UNLISTED)].into_iter().collect();
    assert!(held.insert(id("legalentityidentifier", "executingfirm", CLOSING)));
    assert_eq!(held.get(&IdType::ExecutingFirm), Some(CLOSING));
    // Two real codes of one rank keep the first as the answer, and a value
    // another source states makes no claim to be a code: it ranks as its
    // type, so of it and a real code the first stands too.
    let mut held: Identifiers = [bic(LISTED)].into_iter().collect();
    assert!(held.insert(id("legalentityidentifier", "executingfirm", CLOSING)));
    assert_eq!(held.get(&IdType::ExecutingFirm), Some(LISTED));
    let mut held: Identifiers = [id("proprietary", "executingfirm", "T-1")]
        .into_iter()
        .collect();
    assert!(held.insert(bic(LISTED)));
    assert_eq!(held.get(&IdType::ExecutingFirm), Some("T-1"));
    let mut held: Identifiers = [bic(LISTED)].into_iter().collect();
    assert!(held.insert(id("proprietary", "executingfirm", "T-1")));
    assert_eq!(held.get(&IdType::ExecutingFirm), Some(LISTED));
    // A typo another source also states is no longer the code's alone.
    let mut held: Identifiers = [bic(UNLISTED), id("proprietary", "executingfirm", UNLISTED)]
        .into_iter()
        .collect();
    assert!(held.insert(bic(LISTED)));
    assert_eq!(held.get(&IdType::ExecutingFirm), Some(UNLISTED));
}

#[test]
fn words_fold_to_lower_case_and_values_trim() {
    let party = Identifier::new(
        IdKey::new(
            "Proprietary".parse().unwrap(),
            "Executing Trader".parse().unwrap(),
        ),
        " trader1 ",
    )
    .unwrap();
    assert_eq!(party.kind(), "executingtrader");
    assert_eq!(party.src(), "proprietary");
    assert_eq!(party.kind(), &IdType::ExecutingTrader);
    assert_eq!(party.src(), &IdSource::Proprietary);
    assert_eq!(party.value(), "trader1");
    assert_eq!(party.to_string(), "proprietary:executingtrader=trader1");
    assert_eq!(
        party.key(),
        &IdKey::new(IdSource::Proprietary, IdType::ExecutingTrader)
    );
    assert_ne!(party.key(), &IdKey::base(IdType::ExecutingTrader));

    // A value keeps its case, except where its type is a code that folds.
    let order = id("fix", "orderid", "O-1a");
    assert_eq!(order.value(), "O-1a");
    let isin = Identifier::new(IdKey::base(IdType::Isin), " us0378331005 ").unwrap();
    assert_eq!(
        isin.to_string(),
        "isin=US0378331005",
        "a key from the base source is its type alone"
    );
    // Nothing upper case survives in a word, in a display or in a map.
    let held: Identifiers = [isin].into_iter().collect();
    let scalar = held.into_scalar();
    let entries = scalar.as_mapping().expect("a map");
    assert_eq!(entries[0].0.as_str(), Some("isin"));
    assert_eq!(entries[0].1.as_str(), Some("US0378331005"));
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
    assert!(matches!("FIX".parse::<IdSource>().unwrap(), IdSource::Base));
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
    assert_eq!(held.key(), "clordid", "the standard is the base source");
    assert_eq!(held.key(), &IdKey::base(IdType::ClOrdId));
    let bridged = id("firm.x", "house code", "hc-1");
    assert_eq!(bridged.key(), "firm.x:housecode");
    assert_eq!(
        bridged.key().to_string(),
        format!("{}:{}", bridged.src(), bridged.kind())
    );
    let isin = Identifier::new(IdKey::base(IdType::Isin), "US0378331005").unwrap();
    assert_eq!(isin.key(), "isin");
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
        Identifier::from_key(&held.key().to_string(), held.value()).unwrap(),
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
    assert!(id("base", "clordid", "Z") < id("oms", "clordid", "A"));
    assert!(
        id("oms", "clordid", "Z") < id("base", "orderid", "A"),
        "'oms:clordid' sorts before 'orderid'"
    );
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
    assert_eq!(explicit.to_string(), "clordid=C-1", "fix is the base");
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
    assert!(Identifier::from_key("fix:isin", "US037833100").is_none());
    assert_eq!(
        Identifier::from_key("fix:isin", "US0378331006")
            .unwrap()
            .to_string(),
        "isin=US0378331006",
        "a check digit is a rank, not a refusal"
    );
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
    assert_eq!(read("oms.ClOrdID", "C-1"), "oms:clordid=C-1");
    assert_eq!(read("OrderID", "O-1"), "orderid=O-1");
    assert_eq!(
        read(".OrderID", "O-1"),
        "orderid=O-1",
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
    assert_eq!(read("OrigClOrdID", "C-0"), "origclordid=C-0");
    assert_eq!(read("firm.OrigClOrdID", "C-0"), "firm:origclordid=C-0");
    // The longest name the key ends with answers: `secondaryorderid` before
    // `orderid`, `prevregtradeid` before `regtradeid` before `tradeid`.
    assert_eq!(
        read("OMSSecondaryOrderID", "V-1"),
        "oms:secondaryorderid=V-1"
    );
    assert_eq!(
        read("VenuePrevRegTradeID", "V-1"),
        "venue:prevregtradeid=V-1"
    );
    assert_eq!(read("VenueRegTradeID", "V-1"), "venue:regtradeid=V-1");
    assert_eq!(
        read("SecondaryFirmTradeID", "V-1"),
        "secondaryfirmtradeid=V-1"
    );
    // `origclordid` is a name of its own, so the source is `my` and not
    // `myorig`.
    assert_eq!(read("MyOrigClOrdID", "C-0"), "my:origclordid=C-0");
    // Digits belong to the source, and case never matters.
    assert_eq!(read("Desk7_ClOrdID", "V-1"), "desk7:clordid=V-1");
    assert_eq!(read("oMs.ClOrDiD", "V-1"), "oms:clordid=V-1");
    // The identifier of a key a whole security name spells is that type from
    // `base`, with a leading # dropped and every alias of its name.
    assert_eq!(read("ISINCode", "US0378331005"), "isin=US0378331005");
    assert_eq!(read("#ISINCODE", "US0378331005"), "isin=US0378331005");
    assert_eq!(read("ISIN_Number", "US0378331005"), "isin=US0378331005");
    assert_eq!(read("security_cusip", "037833100"), "cusip=037833100");
    assert_eq!(read("#isin", "US0378331005"), "isin=US0378331005");
    assert_eq!(
        read("fix:clordid", "C-1"),
        "clordid=C-1",
        "an explicit key reads as itself, fix as the base"
    );
}

/// A source the crate reserves - `base`, which `fix` spells, or `derived` - spelled before
/// the name a key ends with names no namespace: the key reads from `base`,
/// as a whole security name does. An explicit `src:type` keeps the source it
/// spells.
#[test]
fn a_reserved_source_before_the_name_a_key_ends_with_reads_as_none() {
    let read = |key: &str, value: &str| Identifier::from_key(key, value).map(|id| id.to_string());
    for key in [
        "Derived_ISIN",
        "DERIVED.ISIN",
        "FIX.ISIN",
        "fix_isin",
        "base.isin",
        ".derived.isin",
    ] {
        assert_eq!(
            read(key, "US0378331005").as_deref(),
            Some("isin=US0378331005"),
            "{key}"
        );
    }
    assert_eq!(
        read("Derived.InstrumentID", "dbi;X").as_deref(),
        Some("instrumentid=dbi;X")
    );
    assert_eq!(read("fix.ClOrdID", "C-1").as_deref(), Some("clordid=C-1"));
    // A source that only starts like a reserved one is a source of its own.
    assert_eq!(
        read("fixed.ClOrdID", "C-1").as_deref(),
        Some("fixed:clordid=C-1")
    );
    assert_eq!(
        read("derived:isin", "US0378331005").as_deref(),
        Some("derived:isin=US0378331005")
    );
    assert_eq!(
        read("fix:isin", "US0378331005").as_deref(),
        Some("isin=US0378331005")
    );
    assert_eq!(read("fix:clordid", "C-1").as_deref(), Some("clordid=C-1"));
}

/// A source and a type are each bounded as a word is, never the key they
/// spell together: a key folding past one word's width names its identifier
/// as its explicit spelling does.
#[test]
fn a_key_whose_source_and_type_each_fit_a_word_names_its_identifier() {
    let key = "venue.desk.bridge.namespace.of.forty.bytes.SecondaryIndividualAllocID";
    let read = Identifier::from_key(key, "A-1")
        .expect("a 42-byte source and a 26-byte type, each within 64 bytes");
    assert_eq!(
        read.to_string(),
        "venue.desk.bridge.namespace.of.forty.bytes:secondaryindividualallocid=A-1"
    );
    assert_eq!(
        Identifier::from_key(&read.key().to_string(), "A-1"),
        Some(read)
    );
    // The widest source a word holds, before an identifier name.
    let widest = "s".repeat(64);
    assert_eq!(
        Identifier::from_key(&format!("{widest}.OrderID"), "O-1").map(|id| id.to_string()),
        Some(format!("{widest}:orderid=O-1"))
    );
    // A source past a word's width names nothing.
    assert!(Identifier::from_key(&format!("{widest}s.OrderID"), "O-1").is_none());
}

#[test]
fn a_parentage_word_before_the_name_stays_part_of_the_type() {
    let read = |key: &str, value: &str| {
        Identifier::from_key(key, value).unwrap_or_else(|| panic!("{key:?} names an identifier"))
    };
    // What each key reads as, and the base and place its type is a parent of.
    for (key, expected, parent) in [
        ("origclordid", "origclordid=T-1", Some((IdType::ClOrdId, 0))),
        ("OrigClOrdID", "origclordid=T-1", Some((IdType::ClOrdId, 0))),
        (
            "ParentOrderID",
            "parentorderid=T-1",
            Some((IdType::OrderId, 0)),
        ),
        (
            "firm.x.parentorderid",
            "firm.x:parentorderid=T-1",
            Some((IdType::OrderId, 0)),
        ),
        ("origorderid", "origorderid=T-1", Some((IdType::OrderId, 1))),
        ("origtradeid", "origtradeid=T-1", Some((IdType::TradeId, 1))),
        ("OrigTradeID", "origtradeid=T-1", Some((IdType::TradeId, 1))),
        (
            "firm.OrigTradeID",
            "firm:origtradeid=T-1",
            Some((IdType::TradeId, 1)),
        ),
        // A word that names no parent stays part of the type all the same.
        ("originalorderid", "originalorderid=T-1", None),
        ("oms.OriginalOrderID", "oms:originalorderid=T-1", None),
        ("omsoriginalorderid", "oms:originalorderid=T-1", None),
        ("originorderid", "originorderid=T-1", None),
        ("originclordid", "originclordid=T-1", None),
        // A bridge's `parentclordid` - the hierarchy parent child orders
        // share - is no spelling of `clordid`'s one parent, `origclordid`:
        // a type of its own, a parent of nothing.
        ("firm.x.parentclordid", "firm.x:parentclordid=T-1", None),
        ("PARENTCLORDID", "parentclordid=T-1", None),
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
    // is refused where another instrument's word opens the key or stands
    // just before the type, a namespace in front of it or not.
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
        "OMS_UnderlyingISIN",
        "FIX.LegISIN",
        "firm.x.ContraCUSIP",
        "venue.benchmark.isin",
    ] {
        assert!(
            Identifier::from_key(key, "US0378331005").is_none(),
            "{key:?} names another instrument"
        );
    }
    // A vendor's source spelling names a type of this instrument's own.
    assert_eq!(
        Identifier::from_key("X-SWX-VALOR", "1221405")
            .expect("a Valor number")
            .to_string(),
        "valor=1221405"
    );
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
        // A byte no word holds names nothing.
        "Ordre_Num\u{e9}ro_OrderID",
        "OMS\tOrderID",
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
    // A check digit that does not close is a rank, never a refusal: the
    // shape is what the type holds to.
    assert_eq!(
        Identifier::from_key("ISINCode", "US0378331006")
            .unwrap()
            .to_string(),
        "isin=US0378331006"
    );
    assert_eq!(
        Identifier::from_key("firm.isin", "US0378331006")
            .unwrap()
            .to_string(),
        "firm:isin=US0378331006"
    );
    assert_eq!(
        Identifier::from_key("venue.cusip", "037833101")
            .unwrap()
            .to_string(),
        "venue:cusip=037833101"
    );
    assert!(
        Identifier::from_key("ISINCode", "US037833100").is_none(),
        "the shape"
    );
    assert!(
        Identifier::from_key("OrderID", &"9".repeat(65)).is_none(),
        "past the width"
    );
}
#[test]
fn a_map_holds_one_value_per_key_and_its_base_key_answers_the_type() {
    let mut ids = Identifiers::new();
    assert!(ids.insert(id("base", "isin", "US0378331005")));
    assert!(
        !ids.insert(id("FIX", "ISIN", "CH0012214059")),
        "fill only, fix being the base"
    );
    assert!(
        ids.insert(id("venue", "isin", "CH0012214059")),
        "another source"
    );
    assert!(ids.insert(id("derived", "cusip", "037833100")));
    assert_eq!(keys(&ids), ["cusip", "derived:cusip", "isin", "venue:isin"]);
    assert_eq!(
        ids.get(&IdType::Isin),
        Some("US0378331005"),
        "the base key answers"
    );
    assert_eq!(ids.get_from(&key("venue:isin")), Some("CH0012214059"));
    assert_eq!(ids.get_from(&key("oms:isin")), None);
    assert!(ids.contains_kind(&IdType::Cusip));
    assert!(!ids.contains_kind(&IdType::Sedol));
    assert_eq!(ids.of_kind(&IdType::Isin).count(), 2);
    assert!(ids.set(id("base", "isin", "CH0012221716")));
    assert!(!ids.set(id("base", "isin", "CH0012221716")), "no change");
    assert_eq!(ids.get(&IdType::Isin), Some("CH0012221716"));
    assert_eq!(
        ids.get_from(&key("venue:isin")),
        Some("CH0012214059"),
        "a base key set leaves the sources"
    );
    assert_eq!(
        ids.remove(&key("venue:isin"))
            .map(|held| held.value().to_owned()),
        Some("CH0012214059".into())
    );
    assert!(ids.remove(&key("venue:isin")).is_none());
    assert_eq!(ids.get(&IdType::Isin), Some("CH0012221716"));
    assert!(
        ids.remove(&IdKey::base(IdType::Isin)).is_some(),
        "the base key removes its type"
    );
    assert_eq!(ids.of_kind(&IdType::Isin).count(), 0);
    assert_eq!(
        ids.to_string(),
        "[cusip=037833100, derived:cusip=037833100]"
    );
    ids.clear();
    assert!(ids.is_empty());
    assert_eq!(ids.to_string(), "[]");
}

#[test]
fn a_named_source_fills_the_base_key_and_never_moves_it_after() {
    let mut ids = Identifiers::new();
    assert!(ids.insert(id("ullink", "isin", "US0378331005")));
    assert_eq!(keys(&ids), ["isin", "ullink:isin"]);
    assert_eq!(
        ids.get(&IdType::Isin),
        Some("US0378331005"),
        "the source fills the base key"
    );
    assert!(ids.set(id("ullink", "isin", "CH0012214059")));
    assert_eq!(
        ids.get(&IdType::Isin),
        Some("US0378331005"),
        "a named source moving leaves the answer"
    );
    assert_eq!(ids.get_from(&key("ullink:isin")), Some("CH0012214059"));
    assert!(ids.insert(id("oms", "isin", "CH0012221716")));
    assert_eq!(ids.get(&IdType::Isin), Some("US0378331005"));
    assert!(ids.remove(&key("ullink:isin")).is_some());
    assert_eq!(
        ids.get(&IdType::Isin),
        Some("US0378331005"),
        "nor does its removal"
    );
    assert!(ids.set(id("base", "isin", "CH0012214059")));
    assert_eq!(
        ids.to_string(),
        "[isin=CH0012214059, oms:isin=CH0012221716]",
        "a base key set replaces the answer and leaves the sources"
    );
    assert!(ids.remove(&IdKey::base(IdType::Isin)).is_some());
    assert!(ids.is_empty(), "removing the base key removes the type");
}

#[test]
fn a_map_displays_and_sorts_by_its_keys_as_they_are_spelled() {
    let statements = [
        id("base", "isin", "US0378331005"),
        id("venue", "instrumentid", "dbi;X"),
        id("base", "orderid", "O-1"),
        id("base", "clordid", "C-1"),
        id("derived", "cusip", "037833100"),
    ];
    let ids: Identifiers = statements.iter().cloned().collect();
    assert_eq!(
        ids.to_string(),
        "[clordid=C-1, cusip=037833100, derived:cusip=037833100, instrumentid=dbi;X, \
         isin=US0378331005, orderid=O-1, venue:instrumentid=dbi;X]"
    );
    // The same statements in any order are one map: equal, and one digest
    // feed.
    let reversed: Identifiers = statements.iter().rev().cloned().collect();
    assert_eq!(ids, reversed);
    assert_eq!(ids.as_slice(), reversed.as_slice());
    // Identifiers order by key, then value, which is how the map holds them.
    let mut loose: Vec<Identifier> = ids.iter().cloned().collect();
    loose.reverse();
    loose.sort();
    assert_eq!(loose.as_slice(), ids.as_slice());
}

#[test]
fn a_map_is_held_in_the_order_of_its_spelled_keys_and_every_lookup_finds_its_own() {
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
    // Each named source fills the base key of its type where it is empty.
    let held = [
        "a", "a.b:c", "a1:a", "a1:x", "a:b", "a:b.c", "a:x", "a:z", "ab:a", "b", "b.c", "c", "x",
        "z",
    ];
    // Any insertion order lands one map, in the order of the spelled keys.
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
        let named = IdKey::new(source(src), kind(ty));
        assert_eq!(forward.get_from(&named), Some("1"), "{src}:{ty}");
        assert_eq!(forward.get(&kind(ty)), Some("1"), "{ty}");
    }
    assert_eq!(forward.get_from(&key("a.b:x")), None);
    assert_eq!(forward.get_from(&key("a:b.d")), None);
    assert_eq!(forward.get_from(&key("a2:x")), None);
    assert_eq!(forward.get_from(&key("b:a")), None);
    // A removal keeps the rest in order and findable, the base key it
    // filled included.
    assert!(forward.remove(&key("a:b")).is_some());
    assert_eq!(forward.len(), held.len() - 1);
    assert_eq!(forward.get_from(&key("a:b.c")), Some("1"));
    assert_eq!(forward.get_from(&key("a:b")), None);
    assert_eq!(forward.get(&kind("b")), Some("1"));
    // Another value under a held key is the same name: insert keeps the
    // first, set replaces it in place.
    assert!(!forward.insert(id("a1", "x", "2")));
    assert!(forward.set(id("a1", "x", "2")));
    assert_eq!(forward.get_from(&key("a1:x")), Some("2"));
    assert_eq!(forward.get(&kind("x")), Some("1"));
    assert_eq!(forward.len(), held.len() - 1);
}

#[test]
fn a_statement_takes_back_the_derivation_of_its_type() {
    let mut ids = Identifiers::new();
    assert!(ids.insert(id("derived", "cusip", "037833100")));
    assert_eq!(keys(&ids), ["cusip", "derived:cusip"]);
    assert_eq!(ids.get(&IdType::Cusip), Some("037833100"));
    assert!(ids.is_derived(&IdType::Cusip));
    assert!(
        !ids.insert(id("derived", "cusip", "594918104")),
        "a derivation lands only where nothing of its type is held"
    );
    assert!(
        ids.insert(id("base", "cusip", "594918104")),
        "a base key holding only a derivation is replaced"
    );
    assert_eq!(ids.to_string(), "[cusip=594918104]");
    assert!(!ids.is_derived(&IdType::Cusip));
    // A statement of the same value takes the derivation back all the same.
    let mut same: Identifiers = [id("derived", "isin", "US0378331005")]
        .into_iter()
        .collect();
    assert!(same.set(id("base", "isin", "US0378331005")));
    assert_eq!(same.to_string(), "[isin=US0378331005]");
    // A named source takes it back too, and fills the base key it left.
    let mut named: Identifiers = [id("derived", "cusip", "037833100")].into_iter().collect();
    assert!(named.insert(id("venue", "cusip", "594918104")));
    assert_eq!(
        named.to_string(),
        "[cusip=594918104, venue:cusip=594918104]"
    );
    // A derivation is refused where a statement of its type is held.
    assert!(!named.insert(id("derived", "cusip", "037833100")));
    assert!(!named.set(id("derived", "cusip", "037833100")));
    assert!(!named.is_derived(&IdType::Cusip));
}

#[test]
fn a_derivations_echo_follows_it_and_leaves_with_it() {
    let mut ids: Identifiers = [id("derived", "isin", "US0378331005")]
        .into_iter()
        .collect();
    assert!(
        ids.set(id("derived", "isin", "CH0012214059")),
        "replacing a derivation moves its echo"
    );
    assert_eq!(
        ids.to_string(),
        "[derived:isin=CH0012214059, isin=CH0012214059]"
    );
    assert!(ids.is_derived(&IdType::Isin));
    assert!(!ids.set(id("derived", "isin", "CH0012214059")), "no change");
    assert!(ids.remove(&key("derived:isin")).is_some());
    assert!(ids.is_empty(), "its echo leaves with it");
    // Read beside a named source, the base key a derivation held passes to
    // the first source left when the derivation goes.
    let mut read = Identifiers::from_scalar(&entries(&[
        ("isin", "US0378331005"),
        ("derived:isin", "US0378331005"),
        ("ullink:isin", "CH0012214059"),
    ]))
    .unwrap();
    assert!(read.is_derived(&IdType::Isin));
    assert!(
        !read.set(id("derived", "isin", "CH0012221716")),
        "a source states the type"
    );
    assert!(read.remove(&key("derived:isin")).is_some());
    assert_eq!(
        read.to_string(),
        "[isin=CH0012214059, ullink:isin=CH0012214059]"
    );
}

#[test]
fn a_map_merges_by_key_and_the_later_one_replaces_what_differs() {
    let ids = base(&[("orderid", "O-1")]);
    let other: Identifiers = [
        id("base", "orderid", "O-2"),
        id("base", "clordid", "C-1"),
        id("venue", "orderid", "O-9"),
    ]
    .into_iter()
    .collect();
    let mut kept = ids.clone();
    assert!(kept.merge(&other, false));
    assert!(!kept.merge(&other, false), "nothing left to take");
    assert_eq!(
        kept.to_string(),
        "[clordid=C-1, orderid=O-1, venue:orderid=O-9]"
    );
    let mut later = ids.clone();
    assert!(later.merge(&other, true));
    assert!(!later.merge(&other, true), "nothing left to move");
    assert_eq!(
        later.to_string(),
        "[clordid=C-1, orderid=O-2, venue:orderid=O-9]"
    );
    // A derivation lands only where nothing of its type is held, later or
    // not, and its echo comes with it rather than as a statement.
    let derived: Identifiers = [
        id("derived", "cusip", "037833100"),
        id("derived", "valor", "3886335"),
    ]
    .into_iter()
    .collect();
    let mut merged = base(&[("cusip", "594918104")]);
    assert!(merged.merge(&derived, true));
    assert_eq!(
        merged.to_string(),
        "[cusip=594918104, derived:valor=3886335, valor=3886335]"
    );
    assert!(merged.is_derived(&IdType::Valor));
}

/// A placeholder never beats a real value in any merge, in any order: a
/// higher rank replaces a lower one through `insert`, `merge` and `close`
/// whichever was stated first, and among values of one rank the existing
/// rule stands - the leading one keeps on `insert`, the later one replaces
/// on `merge`, the first in key order answers on `close`.
#[test]
fn a_higher_ranked_value_replaces_a_lower_one_in_insert_merge_and_close_whatever_the_order() {
    const REAL: &str = "US0378331005";
    const OTHER: &str = "US5949181045";
    const MASKED: &str = "XX0000000001";
    assert!(IdType::Isin.rank(REAL) > IdType::Isin.rank(MASKED));

    // A base key restated: the real number replaces the masked one and the
    // masked one never replaces the real one.
    let mut held = base(&[("isin", MASKED)]);
    assert!(held.insert(id("base", "isin", REAL)));
    assert_eq!(held.to_string(), format!("[isin={REAL}]"));
    let mut held = base(&[("isin", REAL)]);
    assert!(!held.insert(id("base", "isin", MASKED)));
    assert_eq!(held.to_string(), format!("[isin={REAL}]"));
    // A named source outranking the base upgrades it and stands beside it
    // as evidence; one ranking below lands under its own key and leaves
    // the answer.
    let mut held = base(&[("isin", MASKED)]);
    assert!(held.insert(id("ullink", "isin", REAL)));
    assert_eq!(
        held.to_string(),
        format!("[isin={REAL}, ullink:isin={REAL}]")
    );
    let mut held = base(&[("isin", REAL)]);
    assert!(held.insert(id("ullink", "isin", MASKED)));
    assert_eq!(
        held.to_string(),
        format!("[isin={REAL}, ullink:isin={MASKED}]")
    );
    // A named key restated by a higher rank moves, and fills the base with it.
    let mut held: Identifiers = [id("ullink", "isin", MASKED)].into_iter().collect();
    assert!(held.insert(id("ullink", "isin", REAL)));
    assert_eq!(
        held.to_string(),
        format!("[isin={REAL}, ullink:isin={REAL}]")
    );
    assert!(!held.insert(id("ullink", "isin", MASKED)));
    // A derivation lands over a base that ranks below it - a registry's
    // real number over a masked statement - and never over one that does
    // not.
    let mut held = base(&[("isin", MASKED)]);
    assert!(held.insert(id("derived", "isin", REAL)));
    assert_eq!(held.get(&IdType::Isin), Some(REAL));
    assert!(held.is_derived(&IdType::Isin));
    let mut held = base(&[("isin", REAL)]);
    assert!(!held.insert(id("derived", "isin", MASKED)));
    assert_eq!(held.to_string(), format!("[isin={REAL}]"));
    // Two of one rank keep the leading one on insert.
    let mut held = base(&[("isin", REAL)]);
    assert!(!held.insert(id("base", "isin", OTHER)));
    assert_eq!(held.get(&IdType::Isin), Some(REAL));

    // A merge takes the higher rank whether or not the other map leads, and
    // only between equals does the later one replace.
    for later in [false, true] {
        let mut held = base(&[("isin", MASKED)]);
        assert!(held.merge(&base(&[("isin", REAL)]), later), "{later}");
        assert_eq!(held.get(&IdType::Isin), Some(REAL), "{later}");
        let mut held = base(&[("isin", REAL)]);
        assert!(!held.merge(&base(&[("isin", MASKED)]), later), "{later}");
        assert_eq!(held.get(&IdType::Isin), Some(REAL), "{later}");
        let mut held = base(&[("isin", REAL)]);
        assert_eq!(held.merge(&base(&[("isin", OTHER)]), later), later);
        assert_eq!(
            held.get(&IdType::Isin),
            Some(if later { OTHER } else { REAL })
        );
    }
    // A set is the explicit statement and stays unconditional.
    let mut held = base(&[("isin", REAL)]);
    assert!(held.set(id("base", "isin", MASKED)));
    assert_eq!(held.get(&IdType::Isin), Some(MASKED));

    // A map read raw closes each type on its highest-ranked named source,
    // the first in key order among equals, whatever order the entries came
    // in.
    let closed = Identifiers::from_scalar(&entries(&[
        ("abc:isin", MASKED),
        ("venue:isin", REAL),
        ("zzz:isin", OTHER),
    ]))
    .unwrap();
    assert_eq!(closed.get(&IdType::Isin), Some(REAL));
    let closed =
        Identifiers::from_scalar(&entries(&[("zzz:isin", REAL), ("abc:isin", MASKED)])).unwrap();
    assert_eq!(closed.get(&IdType::Isin), Some(REAL));
    let closed = Identifiers::from_scalar(&entries(&[
        ("zzz:isin", OTHER),
        ("abc:isin", MASKED),
        ("derived:isin", REAL),
    ]))
    .unwrap();
    assert_eq!(
        closed.get(&IdType::Isin),
        Some(OTHER),
        "a named source over a derivation of the same rank"
    );
}

/// A derivation ranks like any statement: a registry's real number a chain
/// derived stands over a masked or mistyped statement that comes after it -
/// under the base key, under a named source, carried along a chain or
/// merged from another map, whichever leads - and only a statement that
/// does not rank below it takes it back.
#[test]
fn a_statement_takes_back_a_derivation_only_where_it_does_not_rank_below_it() {
    const REAL: &str = "US0378331005";
    const OTHER: &str = "US5949181045";
    const TYPO: &str = "US0378331006";
    const MASKED: &str = "XX0000000001";
    let derived = || {
        let mut held = Identifiers::new();
        assert!(held.insert(id("derived", "isin", REAL)));
        held
    };
    let named =
        |value: &str| -> Identifiers { [id("ullink", "isin", value)].into_iter().collect() };
    for lower in [MASKED, TYPO] {
        // A base statement ranking below the derivation lands nowhere.
        let mut held = derived();
        assert!(!held.insert(id("base", "isin", lower)), "{lower}");
        assert_eq!(
            held.to_string(),
            format!("[derived:isin={REAL}, isin={REAL}]"),
            "{lower}"
        );
        // A named one stands under its own key as evidence and leaves the
        // answer and the derivation alone.
        let mut held = derived();
        assert!(held.insert(id("ullink", "isin", lower)), "{lower}");
        assert_eq!(
            held.to_string(),
            format!("[derived:isin={REAL}, isin={REAL}, ullink:isin={lower}]"),
            "{lower}"
        );
        assert!(held.is_derived(&IdType::Isin), "{lower}");
        // A merge, whichever map leads.
        for later in [false, true] {
            let mut held = derived();
            assert!(
                !held.merge(&base(&[("isin", lower)]), later),
                "{lower} {later}"
            );
            assert_eq!(held.get(&IdType::Isin), Some(REAL), "{lower} {later}");
            let mut held = derived();
            assert!(held.merge(&named(lower), later), "{lower} {later}");
            assert_eq!(held.get(&IdType::Isin), Some(REAL), "{lower} {later}");
            let mut held = base(&[("isin", lower)]);
            assert!(held.merge(&derived(), later), "{lower} {later}");
            assert_eq!(held.get(&IdType::Isin), Some(REAL), "{lower} {later}");
            assert!(held.is_derived(&IdType::Isin), "{lower} {later}");
        }
        // A whole map - the derivation, its echo and a lower named source -
        // merged into an empty one answers the derivation.
        let mut other = derived();
        assert!(other.insert(id("ullink", "isin", lower)));
        let mut held = Identifiers::new();
        assert!(held.merge(&other, true), "{lower}");
        assert_eq!(held.get(&IdType::Isin), Some(REAL), "{lower}");
        // A chain's lower statement never takes back the real number its
        // follower derived; a named source is carried as evidence.
        let mut next = derived();
        assert!(!next.carry(&base(&[("isin", lower)]), |_| true), "{lower}");
        assert_eq!(next.get(&IdType::Isin), Some(REAL), "{lower}");
        let mut next = derived();
        assert!(next.carry(&named(lower), |_| true), "{lower}");
        assert_eq!(next.get(&IdType::Isin), Some(REAL), "{lower}");
        assert_eq!(
            next.get_from(&"ullink:isin".parse().unwrap()),
            Some(lower),
            "{lower}"
        );
    }
    // A statement of the derivation's own rank takes it back.
    let mut held = derived();
    assert!(held.insert(id("base", "isin", OTHER)));
    assert_eq!(held.to_string(), format!("[isin={OTHER}]"));
    let mut held = derived();
    assert!(held.insert(id("ullink", "isin", OTHER)));
    assert_eq!(
        held.to_string(),
        format!("[isin={OTHER}, ullink:isin={OTHER}]")
    );
    // A map read raw closes on its derivation where that outranks every
    // named source of the type.
    let closed = Identifiers::from_scalar(&entries(&[
        ("abc:isin", MASKED),
        ("derived:isin", REAL),
        ("zzz:isin", TYPO),
    ]))
    .unwrap();
    assert_eq!(closed.get(&IdType::Isin), Some(REAL));
    assert!(closed.is_derived(&IdType::Isin));
}

/// A merge restating a named source the derivation outranks - a typo over
/// the masked number held as evidence, or another masked number on a later
/// merge - moves that source's own key alone: the derivation and its answer
/// stand whichever map leads, as a `set` of a named key leaves the answer,
/// and removing the derivation hands the base key to the highest-ranked
/// named source left, as a read map's close does.
#[test]
fn a_restated_named_source_never_takes_back_a_derivation_that_outranks_it() {
    const REAL: &str = "US0378331005";
    const TYPO: &str = "US0378331006";
    const MASKED: &str = "XX0000000001";
    const OTHER_MASKED: &str = "XX0000000003";
    assert!(IdType::Isin.rank(REAL) > IdType::Isin.rank(TYPO));
    assert!(IdType::Isin.rank(TYPO) > IdType::Isin.rank(MASKED));
    assert_eq!(IdType::Isin.rank(OTHER_MASKED), IdType::Isin.rank(MASKED));
    // The derivation, its echo and a masked named source held as evidence.
    let evidence = || {
        let mut held = Identifiers::new();
        assert!(held.insert(id("derived", "isin", REAL)));
        assert!(held.insert(id("ullink", "isin", MASKED)));
        assert_eq!(
            held.to_string(),
            format!("[derived:isin={REAL}, isin={REAL}, ullink:isin={MASKED}]")
        );
        held
    };
    let named =
        |value: &str| -> Identifiers { [id("ullink", "isin", value)].into_iter().collect() };
    for later in [false, true] {
        // The typo outranks the evidence and not the derivation.
        let mut held = evidence();
        assert!(held.merge(&named(TYPO), later), "{later}");
        assert_eq!(
            held.to_string(),
            format!("[derived:isin={REAL}, isin={REAL}, ullink:isin={TYPO}]"),
            "{later}"
        );
        assert!(held.is_derived(&IdType::Isin), "{later}");
        // The other way round, the typo's map takes the derivation.
        let mut held = named(TYPO);
        assert!(held.merge(&evidence(), later), "{later}");
        assert_eq!(held.get(&IdType::Isin), Some(REAL), "{later}");
        assert!(held.is_derived(&IdType::Isin), "{later}");
    }
    // Another masked number of the evidence's rank replaces it on a later
    // merge alone, the derivation still answering.
    let mut held = evidence();
    assert!(!held.merge(&named(OTHER_MASKED), false));
    assert!(held.merge(&named(OTHER_MASKED), true));
    assert_eq!(
        held.to_string(),
        format!("[derived:isin={REAL}, isin={REAL}, ullink:isin={OTHER_MASKED}]")
    );
    // A set of the named key moves that key alone; a set of the base key is
    // the explicit answer and takes the derivation back.
    let mut held = evidence();
    assert!(held.set(id("ullink", "isin", TYPO)));
    assert!(!held.set(id("ullink", "isin", TYPO)), "no change");
    assert_eq!(
        held.to_string(),
        format!("[derived:isin={REAL}, isin={REAL}, ullink:isin={TYPO}]")
    );
    assert!(held.set(id("base", "isin", MASKED)));
    assert_eq!(
        held.to_string(),
        format!("[isin={MASKED}, ullink:isin={TYPO}]")
    );
    // Removing the derivation hands the base key to the highest-ranked
    // named source left, whatever its place in key order.
    let mut held = evidence();
    assert!(held.insert(id("zzz", "isin", TYPO)));
    assert!(held.remove(&key("derived:isin")).is_some());
    assert_eq!(
        held.to_string(),
        format!("[isin={TYPO}, ullink:isin={MASKED}, zzz:isin={TYPO}]")
    );
}

#[test]
fn a_map_carries_what_it_admits_and_never_a_base_key_over_a_type_it_states() {
    let previous = base(&[
        ("clordid", "A"),
        ("orderid", "O-1"),
        ("mdentryrefid", "R-1"),
    ]);
    let mut next = base(&[("clordid", "B")]);
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
    // A map stating a type under any source holds its base key, so the
    // predecessor's base key of that type is not carried over it.
    let mut other: Identifiers = [id("oms", "clordid", "X")].into_iter().collect();
    assert!(!other.carry(&previous, |held| held.kind() == &IdType::ClOrdId));
    assert_eq!(other.get(&IdType::ClOrdId), Some("X"));
    assert_eq!(other.len(), 2);
    // A derivation is carried with its echo, and never over a statement.
    let derived: Identifiers = [id("derived", "cusip", "037833100")].into_iter().collect();
    let mut follower = Identifiers::new();
    assert!(follower.carry(&derived, |_| true));
    assert_eq!(follower, derived);
    let mut stated = base(&[("cusip", "594918104")]);
    assert!(!stated.carry(&derived, |_| true));
    assert_eq!(stated.to_string(), "[cusip=594918104]");
    // A chain's statement is carried over a derivation of its type, which
    // it takes back: a registry's reading of the follower never outlives
    // what the chain states.
    let mut filled: Identifiers = [id("derived", "cusip", "037833100")].into_iter().collect();
    assert!(filled.carry(&base(&[("cusip", "037833100")]), |_| true));
    assert_eq!(filled.to_string(), "[cusip=037833100]");
    assert!(!filled.is_derived(&IdType::Cusip));
    // Nothing admitted, nothing moved.
    let mut none = base(&[("clordid", "B")]);
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
        "[orderid=A]",
        "a first statement has no parent"
    );
    assert_eq!(
        orders[1].to_string(),
        "[orderid=B, origorderid=A, parentorderid=A]"
    );
    assert_eq!(
        orders[2].to_string(),
        "[orderid=C, origorderid=A, parentorderid=B]"
    );
    assert_eq!(
        orders[3].to_string(),
        "[orderid=D, origorderid=A, parentorderid=C]"
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
    assert_eq!(clients[0].to_string(), "[clordid=A]");
    assert_eq!(clients[1].to_string(), "[clordid=B, origclordid=A]");
    assert_eq!(clients[2].to_string(), "[clordid=C, origclordid=B]");
    assert_eq!(clients[2].get(&IdType::OrigClOrdId), Some("B"));

    // Every base an element states follows by its own parents, in one pass.
    let previous = base(&[("clordid", "A"), ("orderid", "O-1"), ("tradeid", "T-1")]);
    let mut next = base(&[("clordid", "B"), ("orderid", "O-2"), ("tradeid", "T-1")]);
    assert!(follow(&mut next, &previous));
    assert_eq!(
        next.to_string(),
        "[clordid=B, orderid=O-2, origclordid=A, origorderid=O-1, \
         parentorderid=O-1, tradeid=T-1]",
        "an unchanged base over no parents has none"
    );
}

#[test]
fn an_unchanged_base_keeps_the_parents_of_the_step_before() {
    let orders = chain("orderid", &["A", "B", "C"]);
    // C again: each parent is the previous one, and a third statement is the
    // same set again.
    let mut again = base(&[("orderid", "C")]);
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
    let mut same = base(&[("orderid", "A")]);
    assert!(!follow(&mut same, &base(&[("orderid", "A")])));
    assert_eq!(same.len(), 1);
    let mut clients = base(&[("clordid", "A")]);
    assert!(!follow(&mut clients, &base(&[("clordid", "A")])));
    assert_eq!(clients.len(), 1);
}

#[test]
fn a_parent_the_follower_states_stands_and_only_an_absent_one_is_filled() {
    let orders = chain("orderid", &["A", "B", "C"]);
    // `parentorderid` stated: it stands, and the chain's first is filled.
    let mut stated = base(&[("orderid", "D"), ("parentorderid", "X")]);
    assert!(follow(&mut stated, &orders[2]));
    assert_eq!(
        stated.to_string(),
        "[orderid=D, origorderid=A, parentorderid=X]"
    );
    // `origorderid` stated: it stands, and the one before the last change is
    // filled.
    let mut origin = base(&[("orderid", "D"), ("origorderid", "Y")]);
    assert!(follow(&mut origin, &orders[2]));
    assert_eq!(
        origin.to_string(),
        "[orderid=D, origorderid=Y, parentorderid=C]"
    );
    // Both stated: nothing is absent.
    let mut both = base(&[
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
    let first = base(&[("clordid", "A")]);
    let mut replaced = base(&[("clordid", "B"), ("origclordid", "Z")]);
    assert!(!follow(&mut replaced, &first));
    assert_eq!(replaced.get(&IdType::OrigClOrdId), Some("Z"));
    assert_eq!(replaced.len(), 2);
}

#[test]
fn a_follower_takes_parents_only_from_its_own_source_and_never_for_a_parent_type() {
    let previous: Identifiers = [
        id("base", "orderid", "A"),
        id("venue", "orderid", "Q"),
        id("base", "clordid", "C-1"),
    ]
    .into_iter()
    .collect();
    // Another source states another chain: a source the predecessor did not
    // state follows nothing, while the base keys it filled follow the
    // predecessor's answers as every base key does.
    let mut other: Identifiers = [id("oms", "clordid", "X"), id("oms", "orderid", "Y")]
        .into_iter()
        .collect();
    assert!(follow(&mut other, &previous));
    assert_eq!(
        other.to_string(),
        "[clordid=X, oms:clordid=X, oms:orderid=Y, orderid=Y, origclordid=C-1, \
         origorderid=A, parentorderid=A]"
    );
    // Each source follows its own predecessor.
    let mut next: Identifiers = [id("base", "orderid", "B"), id("venue", "orderid", "R")]
        .into_iter()
        .collect();
    assert!(follow(&mut next, &previous));
    assert_eq!(
        next.to_string(),
        "[orderid=B, origorderid=A, parentorderid=A, \
         venue:orderid=R, venue:origorderid=Q, venue:parentorderid=Q]"
    );
    // A base the predecessor did not state has nothing to take, and a parent
    // type is no base: it is never given parents of its own.
    let mut quote = base(&[("quoteid", "Q-2"), ("origclordid", "A")]);
    assert!(!follow(&mut quote, &previous));
    assert_eq!(quote.len(), 2);
    let mut chained = base(&[("origclordid", "B")]);
    assert!(!follow(&mut chained, &base(&[("origclordid", "A")])));
    assert_eq!(chained.len(), 1);
    let mut parented = base(&[("parentorderid", "B")]);
    assert!(!follow(&mut parented, &base(&[("parentorderid", "A")])));
    assert_eq!(parented.len(), 1);
    // An empty predecessor gives nothing, and an empty follower takes
    // nothing.
    assert!(!follow(&mut base(&[("clordid", "B")]), &Identifiers::new()));
    assert!(!follow(&mut Identifiers::new(), &previous));
}

#[test]
fn a_parents_list_of_three_shifts_the_middle_one_and_ends_with_the_chains_first_value() {
    // The vocabulary is the caller's: three parents for `orderid`, nearest
    // first.
    let mut states: Vec<Identifiers> = Vec::new();
    for value in ["A", "B", "C", "D", "E"] {
        let mut next = base(&[("orderid", value)]);
        if let Some(previous) = states.last() {
            assert!(next.follow_parents(previous, three_parents, three_parent_of));
        }
        states.push(next);
    }
    let held = |at: usize| states[at].to_string();
    assert_eq!(held(0), "[orderid=A]");
    assert_eq!(held(1), "[far=A, near=A, orderid=B]");
    assert_eq!(held(2), "[far=A, middle=A, near=B, orderid=C]");
    assert_eq!(held(3), "[far=A, middle=B, near=C, orderid=D]");
    assert_eq!(
        held(4),
        "[far=A, middle=C, near=D, orderid=E]",
        "the near one is the value before the change, each middle one the near one of the step before, the last the chain's first"
    );

    // An unchanged base takes each parent at its own place.
    let mut again = base(&[("orderid", "E")]);
    assert!(again.follow_parents(&states[4], three_parents, three_parent_of));
    assert_eq!(again, states[4]);
    assert!(!again.follow_parents(&states[4], three_parents, three_parent_of));

    // A parent the follower states stands, wherever it is in the list.
    let mut stated = base(&[("orderid", "D"), ("middle", "X")]);
    assert!(stated.follow_parents(&states[2], three_parents, three_parent_of));
    assert_eq!(stated.to_string(), "[far=A, middle=X, near=C, orderid=D]");

    // The last parent is the previous last one, else the farthest the
    // previous statement made, else the previous value.
    let from_far = |previous: &[(&str, &str)]| {
        let mut next = base(&[("orderid", "C")]);
        next.follow_parents(&base(previous), three_parents, three_parent_of);
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
    let mut next = base(&[("orderid", "C")]);
    assert!(next.follow_parents(
        &base(&[("orderid", "B"), ("far", "F")]),
        three_parents,
        three_parent_of
    ));
    assert_eq!(next.to_string(), "[far=F, near=B, orderid=C]");

    // A base no list names has no parent, and a list of none fills nothing.
    let mut quote = base(&[("quoteid", "Q-2")]);
    assert!(!quote.follow_parents(&base(&[("quoteid", "Q-1")]), three_parents, three_parent_of));
    let mut none = base(&[("orderid", "B")]);
    assert!(!none.follow_parents(&base(&[("orderid", "A")]), |_| Cow::Borrowed(&[]), |_| None));
    assert_eq!(none.len(), 1);
}

#[test]
fn an_element_stating_a_parent_but_not_its_type_takes_the_type_from_its_nearest_parent() {
    // `OrigClOrdID(41)` without `ClOrdID(11)`: the element is what it came from.
    let mut ids = base(&[("origclordid", "A")]);
    assert!(ids.fill_parents(IdType::parent_of));
    assert_eq!(ids.get(&IdType::ClOrdId), Some("A"));
    assert_eq!(ids.len(), 2, "the parent stays beside the base it filled");
    assert!(!ids.fill_parents(IdType::parent_of), "nothing left to fill");

    // A stated base stands, whatever its parents say.
    let mut stated = base(&[("clordid", "B"), ("origclordid", "A")]);
    assert!(!stated.fill_parents(IdType::parent_of));
    assert_eq!(stated.get(&IdType::ClOrdId), Some("B"));

    // The nearest parent wins: `origorderid` sorts first in the set, and
    // `parentorderid`, which is nearer, still fills the base.
    let mut both = base(&[("origorderid", "O"), ("parentorderid", "P")]);
    assert_eq!(keys(&both), ["origorderid", "parentorderid"]);
    assert!(both.fill_parents(IdType::parent_of));
    assert_eq!(both.get(&IdType::OrderId), Some("P"));
    assert_eq!(both.len(), 3);
    let mut origin = base(&[("origorderid", "O")]);
    assert!(origin.fill_parents(IdType::parent_of));
    assert_eq!(
        origin.get(&IdType::OrderId),
        Some("O"),
        "the farthest alone fills it"
    );
    // The same over a list of three: the nearest stated, not the nearest named.
    let mut custom = base(&[("far", "F"), ("middle", "M")]);
    assert!(custom.fill_parents(three_parent_of));
    assert_eq!(custom.get(&IdType::OrderId), Some("M"));
    let mut far = base(&[("far", "F")]);
    assert!(far.fill_parents(three_parent_of));
    assert_eq!(far.get(&IdType::OrderId), Some("F"));

    // Each base is filled from its own parents, in one pass.
    let mut many = base(&[
        ("origorderid", "O"),
        ("origclordid", "C"),
        ("origtradeid", "T"),
    ]);
    assert!(many.fill_parents(IdType::parent_of));
    assert_eq!(many.get(&IdType::OrderId), Some("O"));
    assert_eq!(many.get(&IdType::ClOrdId), Some("C"));
    assert_eq!(many.get(&IdType::TradeId), Some("T"));
    assert_eq!(many.len(), 6);

    // A parent fills its type under its own source, and a base key another
    // source filled already answers the type.
    let mut sources: Identifiers = [
        id("base", "origclordid", "A"),
        id("venue", "clordid", "B"),
        id("oms", "parentorderid", "P"),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        keys(&sources),
        [
            "clordid",
            "oms:parentorderid",
            "origclordid",
            "parentorderid",
            "venue:clordid"
        ]
    );
    assert!(sources.fill_parents(IdType::parent_of));
    assert_eq!(
        sources.get(&IdType::ClOrdId),
        Some("B"),
        "venue's filled it first"
    );
    assert_eq!(sources.get_from(&key("oms:orderid")), Some("P"));
    assert_eq!(sources.get(&IdType::OrderId), Some("P"));
    assert_eq!(sources.len(), 7);

    // A parent value its base refuses fills nothing. The crate's vocabulary
    // names parents of chain identities alone, whose values are any text,
    // so the refusal is a caller's vocabulary naming `parentisin` a parent
    // of `isin` - which the crate's names a word of its own, filling
    // nothing either.
    let mut refused = base(&[("parentisin", "NOT-AN-ISIN")]);
    assert!(!refused.fill_parents(|kind| (kind == "parentisin").then_some((IdType::Isin, 0))));
    assert_eq!(refused.len(), 1);
    assert!(!refused.fill_parents(IdType::parent_of));
    assert_eq!(refused.len(), 1);
    let mut accepted = base(&[("parentisin", "US0378331005")]);
    assert!(accepted.fill_parents(|kind| (kind == "parentisin").then_some((IdType::Isin, 0))));
    assert_eq!(accepted.get(&IdType::Isin), Some("US0378331005"));
    // A closure naming no parent fills nothing.
    let mut none = base(&[("origclordid", "A")]);
    assert!(!none.fill_parents(|_| None));
    assert_eq!(none.len(), 1);
    assert!(!Identifiers::new().fill_parents(IdType::parent_of));
    // A base is no parent, so a set of bases fills nothing.
    let mut bases = base(&[("orderid", "O"), ("clordid", "C")]);
    assert!(!bases.fill_parents(IdType::parent_of));
    assert_eq!(bases.len(), 2);
}

#[test]
fn an_identifier_moves_to_another_type_of_its_source_as_that_type_stores_it() {
    let held = id("base", "origclordid", "A");
    assert_eq!(
        held.with_kind(IdType::ClOrdId).unwrap().to_string(),
        "clordid=A"
    );
    let cusip = id("venue", "parentcusip", "037833100");
    assert_eq!(
        cusip.with_kind(IdType::Cusip).unwrap().to_string(),
        "venue:cusip=037833100"
    );
    let lower = id("base", "house", "us0378331005");
    assert_eq!(
        lower.with_kind(IdType::Isin).unwrap().value(),
        "US0378331005"
    );
    assert_eq!(
        id("base", "house", "US0378331006")
            .with_kind(IdType::Isin)
            .unwrap()
            .value(),
        "US0378331006",
        "a check digit is a rank, not a refusal"
    );
    assert!(
        id("base", "house", "037833100")
            .with_kind(IdType::Isin)
            .is_err()
    );
}

#[test]
fn a_map_is_a_sorted_map_scalar_of_its_spelled_keys_to_its_values() {
    let ids: Identifiers = [
        id("base", "isin", "US0378331005"),
        id("base", "clordid", "B"),
        id("a", "z", "1"),
        id("a.b", "c", "2"),
        id("a1", "x", "3"),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        keys(&ids),
        ["a.b:c", "a1:x", "a:z", "c", "clordid", "isin", "x", "z"]
    );
    let scalar = ids.into_scalar();
    assert!(matches!(scalar, Scalar::SortedMap(_)), "{}", scalar.kind());
    let held: Vec<(&str, &str)> = scalar
        .as_mapping()
        .expect("a map's entries")
        .iter()
        .map(|(key, value)| {
            (
                key.as_str().expect("a text key"),
                value.as_str().expect("a text value"),
            )
        })
        .collect();
    assert_eq!(
        held,
        [
            ("a.b:c", "2"),
            ("a1:x", "3"),
            ("a:z", "1"),
            ("c", "2"),
            ("clordid", "B"),
            ("isin", "US0378331005"),
            ("x", "3"),
            ("z", "1"),
        ]
    );
    assert_eq!(
        Identifiers::from_scalar(&scalar).unwrap(),
        ids,
        "a map the crate wrote reads back unchanged"
    );

    // An empty map is an empty sorted map, not a null and not a run.
    let empty = Identifiers::new().into_scalar();
    assert!(matches!(empty, Scalar::SortedMap(_)), "{}", empty.kind());
    assert_eq!(empty.as_mapping().map(<[_]>::len), Some(0));
    assert_eq!(
        Identifiers::from_scalar(&empty).unwrap(),
        Identifiers::new()
    );
}

#[test]
fn a_map_reads_back_from_text_entries_in_any_order_and_closes_by_the_base_rule() {
    let pairs = [
        ("ULLINK:ISIN", "us0378331005"),
        ("a:z", "1"),
        ("Base:ClOrdID", "B"),
    ];
    let read = Identifiers::from_scalar(&entries(&pairs)).unwrap();
    assert_eq!(
        read.to_string(),
        "[a:z=1, clordid=B, isin=US0378331005, ullink:isin=US0378331005, z=1]",
        "each key read exactly, each value as its type stores it, each type closed"
    );
    let Scalar::Map(map) = entries(&pairs) else {
        panic!("a plain map");
    };
    assert_eq!(
        Identifiers::from_scalar(&Scalar::SortedMap(map)).unwrap(),
        read
    );
    // The spellings written before the base key was bare read as it.
    let old = Identifiers::from_scalar(&entries(&[
        ("fix:clordid", "B"),
        ("base:isin", "US0378331005"),
    ]))
    .unwrap();
    assert_eq!(old.to_string(), "[clordid=B, isin=US0378331005]");
    // A type with no base key takes its first named source's value in key
    // order, else its derivation's; a stated base key stands.
    let closed = Identifiers::from_scalar(&entries(&[
        ("venue:isin", "CH0012214059"),
        ("derived:isin", "US0378331005"),
        ("abc:isin", "CH0012221716"),
        ("derived:cusip", "037833100"),
    ]))
    .unwrap();
    assert_eq!(closed.get(&IdType::Isin), Some("CH0012221716"));
    assert!(!closed.is_derived(&IdType::Isin));
    assert_eq!(closed.get(&IdType::Cusip), Some("037833100"));
    assert!(closed.is_derived(&IdType::Cusip));
    let stated = Identifiers::from_scalar(&entries(&[
        ("isin", "US0378331005"),
        ("ullink:isin", "CH0012214059"),
    ]))
    .unwrap();
    assert_eq!(stated.get(&IdType::Isin), Some("US0378331005"));
    assert_eq!(stated.len(), 2);
    assert_eq!(
        Identifiers::from_scalar(&Scalar::from_mapping([]).unwrap()).unwrap(),
        Identifiers::new()
    );
}

#[test]
fn a_map_entry_no_identifier_reads_is_refused_on_its_key() {
    let refused = |scalar: Scalar| Identifiers::from_scalar(&scalar).unwrap_err().to_string();
    // A key that reads as no key.
    for text in ["fix:", ":isin", "a:b:c", "a/b", "transversal key!"] {
        let error = refused(entries(&[(text, "K-1")]));
        assert!(error.contains(&format!("$['{text}']")), "{error}");
        assert!(
            error.contains("expected an identifier key src:type or type"),
            "{error}"
        );
    }
    // A value that states nothing, or that its type refuses.
    for (text, value) in [
        ("isin", "US037833100"),
        ("clordid", "null"),
        ("clordid", "  "),
        ("ullink:isin", "037833100"),
    ] {
        let error = refused(entries(&[(text, value)]));
        assert!(error.contains(&format!("$['{text}']")), "{error}");
    }
    // Two spellings of one key with two values; the same value twice is one
    // identifier.
    let error = refused(entries(&[
        ("isin", "US0378331005"),
        ("BASE:ISIN", "CH0012214059"),
    ]));
    assert!(error.contains("$['BASE:ISIN']"), "{error}");
    assert!(error.contains("expected one value under isin"), "{error}");
    let one = Identifiers::from_scalar(&entries(&[
        ("isin", "US0378331005"),
        ("BASE:ISIN", "us0378331005"),
    ]))
    .unwrap();
    assert_eq!(one.to_string(), "[isin=US0378331005]");
    // A key or a value that is no text, the retired struct row included.
    assert!(
        Identifiers::from_scalar(
            &Scalar::from_mapping([(Scalar::from(1_i64), Scalar::from("B"))]).unwrap()
        )
        .is_err()
    );
    let error =
        refused(Scalar::from_mapping([(Scalar::from("clordid"), Scalar::from(1_i64))]).unwrap());
    assert!(error.contains("$['clordid']"), "{error}");
    let row = Scalar::from_sequence(["fix", "clordid", "B"].map(Scalar::from));
    let error = refused(Scalar::from_mapping([(Scalar::from("fix:clordid"), row)]).unwrap());
    assert!(
        error.contains("$['fix:clordid']") && error.contains("a text value"),
        "{error}"
    );
    // Neither a map nor a sequence of maps.
    assert!(Identifiers::from_scalar(&Scalar::from("x")).is_err());
}

#[test]
fn a_sequence_of_maps_reads_as_their_union() {
    let rows = Scalar::from_sequence([
        entries(&[("clordid", "B")]),
        entries(&[("ullink:isin", "US0378331005")]),
        entries(&[("clordid", "B")]),
    ]);
    let read = Identifiers::from_scalar(&rows).expect("a sequence of maps");
    assert_eq!(
        read.to_string(),
        "[clordid=B, isin=US0378331005, ullink:isin=US0378331005]"
    );
    assert_eq!(
        Identifiers::from_scalar(&Scalar::from_sequence([]))
            .unwrap()
            .len(),
        0
    );
    // Two values under one key are two readings, refused at the second.
    let error = Identifiers::from_scalar(&Scalar::from_sequence([
        entries(&[("isin", "US0378331005")]),
        entries(&[("isin", "CH0012214059")]),
    ]))
    .expect_err("one key, two values")
    .to_string();
    assert!(error.contains("$[1]['isin']"), "{error}");
    // A cell that is no map is refused at its place.
    let error = Identifiers::from_scalar(&Scalar::from_sequence([Scalar::from("x")]))
        .expect_err("no map")
        .to_string();
    assert!(error.contains("$[0]"), "{error}");
}

#[test]
fn a_map_crosses_a_column_under_its_dtype_and_back() {
    let ids: Identifiers = [
        id("base", "isin", "US0378331005"),
        id("ullink", "clordid", "B"),
        id("derived", "cusip", "037833100"),
        id("a1", "x", "3"),
    ]
    .into_iter()
    .collect();
    let field = Arc::new(Field::new("ids", Identifiers::dtype(), true));
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

    // Through Arrow: a map of two text columns whose keys are declared
    // sorted, read back under the same field as the same rows.
    let array = serie.clone().into_arrow_array().expect("a column");
    let arrow_schema::DataType::Map(pair, true) = array.data_type() else {
        panic!("a sorted map, got {:?}", array.data_type());
    };
    let arrow_schema::DataType::Struct(pair) = pair.data_type() else {
        panic!("a struct of entries");
    };
    assert_eq!(
        pair.iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>(),
        ["key", "value"]
    );
    assert!(
        pair.iter()
            .all(|field| field.data_type() == &arrow_schema::DataType::Utf8 && !field.is_nullable())
    );
    let back = Serie::from_arrow_array(Some(&field), array, ArrowCastOptions::new()).unwrap();
    for (row, expected) in [(0, &ids), (1, &Identifiers::new()), (2, &ids)] {
        let read = Identifiers::from_scalar(&back.scalar(row).unwrap()).unwrap();
        assert_eq!(&read, expected, "row {row} after Arrow");
    }
}

#[test]
fn a_map_is_laid_out_as_a_sorted_map_of_text_to_text() {
    let map = Identifiers::dtype();
    assert!(matches!(map, DataType::SortedMap(_)), "{map}");
    assert!(map.as_mapping().expect("a map").keys_sorted());
    let entries = map.map_entries().expect("a map's entries");
    assert_eq!(entries.name(), "entries");
    assert!(!entries.is_nullable());
    let pair = entries.dtype().as_fields().expect("a struct of two");
    let names: Vec<_> = pair.iter().map(Field::name).collect();
    assert_eq!(names, ["key", "value"]);
    for field in pair {
        assert!(!field.is_nullable());
        assert_eq!(field.dtype(), &DataType::utf8());
    }
}
