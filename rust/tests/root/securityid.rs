//! `rust/src/securityid.rs`: the national number an ISIN embeds as a derived
//! security [`Identifier`], and the shape a symbol reads as. The type of name
//! a security identifier is - its `SecurityIDSource(22)` code, its field
//! names, the rule its value follows - is `rust/tests/root/idtype.rs`'s. The
//! registry a lifecycle learns with is a lifecycle's own - nothing above it
//! names one - so what it learns, refuses and costs is reached through
//! `yggdryl::internals` in [`internal`].

use yggdryl::securityid::embedded;
use yggdryl::{IdSource, IdType, Identifier, Identifiers, Isin};

/// One security identifier of `key` - a type's name or its FIX source code -
/// from `base`, validated by its type.
fn id(key: &str, code: &str) -> Identifier {
    Identifier::new(
        IdSource::Base,
        IdType::from_security_source(key).unwrap(),
        code,
    )
    .unwrap()
}

/// A set of security identifiers holds one value per source and type,
/// sorted, and answers a type however its spelling folds.
#[test]
fn security_identifiers_are_held_per_source_and_type_in_key_order() {
    let ids: Identifiers = [
        id("isin", "US0378331005"),
        id("cusip", "037833100"),
        id("house", "b"),
        id("bloomberg", "AAPL US Equity"),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
        [
            "base:bloomberg=AAPL US Equity",
            "base:cusip=037833100",
            "base:house=b",
            "base:isin=US0378331005",
        ]
    );
    assert_eq!(ids.get(&"Isin".parse().unwrap()), Some("US0378331005"));
    assert_eq!(
        ids.get(&"ISIN_Number".parse().unwrap()),
        Some("US0378331005")
    );
    assert_eq!(ids.get(&IdType::Sedol), None);
    assert_eq!(
        ids.get(&"4".parse().unwrap()),
        None,
        "a code is no word to look up"
    );
    // A source's own value of a type is another identifier from the same
    // instrument, held beside the one `base` stated.
    let mut both = ids.clone();
    assert!(both.insert(Identifier::new(IdSource::Derived, IdType::Cusip, "037833100").unwrap()));
    assert_eq!(both.get(&IdType::Cusip), Some("037833100"));
    assert_eq!(both.of_kind(&IdType::Cusip).count(), 2);
}

fn isin(body: &str) -> Isin {
    let digit = Isin::closing_digit(body).unwrap();
    Isin::new(format!("{body}{digit}")).unwrap()
}

#[test]
fn embedded_names_the_national_number_a_canonical_isin_carries() {
    let found = |text: &str| {
        embedded(&Isin::new(text).unwrap())
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(found("US0378331005"), ["derived:cusip=037833100"]);
    assert_eq!(found("CA1125851040"), ["derived:cusip=112585104"]);
    assert_eq!(found("GB0002634946"), ["derived:sedol=0263494"]);
    assert_eq!(found("IE00B4BNMY34"), ["derived:sedol=B4BNMY3"]);
    assert_eq!(found("JE00B4T3BW64"), ["derived:sedol=B4T3BW6"]);
    assert_eq!(found("DE0007164600"), ["derived:wkn=716460"]);
    assert_eq!(found("DE000BASF111"), ["derived:wkn=BASF11"]);
    assert_eq!(found("CH0038863350"), ["derived:valor=3886335"]);
    assert_eq!(found("LI0010737216"), ["derived:valor=1073721"]);
    assert!(found("XS0203470157").is_empty());
    assert!(found("FR0000120271").is_empty());

    let none = |isin: &Isin| embedded(isin).next().is_none();
    assert!(none(&isin("GB000263495")), "a bad embedded SEDOL check");
    assert!(none(&isin("GB010263494")), "a GB ISIN not starting GB00");
    assert!(none(&isin("IE01B4BNMY3")), "an IE ISIN not starting IE00");
    assert!(none(&isin("DE001716460")), "a DE ISIN not starting DE000");
    assert!(none(&isin("DE000BASI11")), "a WKN holding I");
    assert!(none(&isin("DE000BASO11")), "a WKN holding O");
    assert!(none(&isin("CH000000000")), "a Valor number of nothing");
    assert!(none(&Isin::default()), "an empty ISIN");
    let unchecked: Isin = serde_json::from_str("\"US0378331006\"").unwrap();
    assert!(Isin::new("US0378331006").is_err());
    assert!(none(&unchecked), "a bad ISIN check digit");
    let lower: Isin = serde_json::from_str("\"us0378331005\"").unwrap();
    assert!(none(&lower), "not canonical");
}

#[cfg(feature = "internals")]
mod internal {
    //! The registry, reached through `yggdryl::internals::securityid`.

    use yggdryl::graph::{Element, Market, OrderEvent};
    use yggdryl::internals::securityid::{
        ENTRY_CHARGE, MAX_KEYS_PER_INSTRUMENT, SecurityIdRegistry,
    };
    use yggdryl::{Cfi, IdType, Identifiers, Isin};

    use super::id;

    fn apple() -> OrderEvent {
        let mut event = OrderEvent::at(1);
        event
            .insert_securityid(id("isin", "US0378331005"))
            .expect("a plain holder takes every identifier");
        event.finalize();
        event
    }

    fn numbered(number: usize) -> OrderEvent {
        let body = format!("FR{number:09}");
        let digit = Isin::closing_digit(&body).unwrap();
        let mut event = OrderEvent::at(number as i64);
        event
            .insert_securityid(id("isin", &format!("{body}{digit}")))
            .expect("a plain holder takes every identifier");
        event.finalize();
        event
    }

    /// The `index`th of ten security types the crate names that check no
    /// code, each one the registry learns, in the order their keys sort.
    fn house(index: usize) -> IdType {
        [
            IdType::Belgian,
            IdType::ClearingHouse,
            IdType::Common,
            IdType::Cta,
            IdType::Dutch,
            IdType::ExchSymb,
            IdType::FpmlSpec,
            IdType::Opra,
            IdType::Quik,
            IdType::Sicovam,
        ][index]
            .clone()
    }

    fn stated(event: &mut OrderEvent, key: &str, code: &str) {
        event
            .insert_securityid(id(key, code))
            .expect("a plain holder takes every identifier");
    }

    fn code<'event>(event: &'event OrderEvent, key: &str) -> Option<&'event str> {
        event.get_securityids().get(&key.parse().unwrap())
    }

    #[test]
    fn learned_codes_are_local_validated_and_ambiguous_defaults_are_silent() {
        let mut codes = SecurityIdRegistry::default();
        let mut first = apple();
        first.set_cficode(Some(Cfi::new("ESXXXX").unwrap()), true);
        stated(&mut first, "bloomberg", "AAPL US Equity");
        codes.enrich(&mut first);
        let mut next = apple();
        codes.enrich(&mut next);
        assert!(
            next.get_cficode().is_none(),
            "coarse classifications are not learned"
        );
        assert_eq!(code(&next, "bloomberg"), code(&first, "bloomberg"));

        let mut precise = apple();
        precise.set_cficode(Some(Cfi::new("ESVUFR").unwrap()), true);
        stated(&mut precise, "sedol", "2046251");
        codes.enrich(&mut precise);
        let mut later = apple();
        codes.enrich(&mut later);
        assert_eq!(later.get_cficode(), precise.get_cficode());
        assert_eq!(code(&later, "sedol"), code(&precise, "sedol"));
        // A holder keeps a detailed classification or none, so the coarse
        // one `first` was given never stood, and the learned one did not
        // reach back into it either.
        assert!(
            first.get_cficode().is_none(),
            "earlier snapshots stay unchanged"
        );
        let mut coarse = apple();
        coarse.set_cficode(Some(Cfi::new("ESXXXX").unwrap()), true);
        codes.enrich(&mut coarse);
        assert_eq!(coarse.get_cficode(), precise.get_cficode());

        let mut conflict = apple();
        stated(&mut conflict, "bloomberg", "AAPL LN Equity");
        codes.enrich(&mut conflict);
        assert_eq!(code(&conflict, "bloomberg"), Some("AAPL LN Equity"));
        let mut after_conflict = apple();
        codes.enrich(&mut after_conflict);
        assert!(code(&after_conflict, "bloomberg").is_none());

        let mut independent = apple();
        SecurityIdRegistry::default().enrich(&mut independent);
        assert!(independent.get_cficode().is_none());
        assert!(code(&independent, "bloomberg").is_none());
    }

    #[test]
    fn an_element_without_an_isin_or_a_detailed_code_seeds_no_association() {
        // A security identifier is validated where it is made, so no empty
        // or unchecked code reaches a holder: what is left to refuse is an
        // element naming no ISIN, and a coarse classification.
        let mut codes = SecurityIdRegistry::default();
        let mut empty = OrderEvent::default();
        codes.enrich(&mut empty);
        assert_eq!(codes.instruments(), 0);
        assert_eq!(codes.reserved_bytes(), 0);
        let mut unnamed = OrderEvent::at(1);
        stated(&mut unnamed, "cusip", "037833100");
        codes.enrich(&mut unnamed);
        assert_eq!(codes.instruments(), 0, "nothing is learned without an ISIN");
        let mut observed = apple();
        observed.set_cficode(Some(Cfi::new("XXXXXX").unwrap()), true);
        codes.enrich(&mut observed);
        let mut later = apple();
        codes.enrich(&mut later);
        assert!(later.get_cficode().is_none());
        assert!(code(&later, "sedol").is_none());
        assert!(code(&later, "bloomberg").is_none());
        assert_eq!(
            later.get_securityids().get(&IdType::Cusip),
            Some("037833100"),
            "the CUSIP a US ISIN embeds is the element's own, not a learned one"
        );
    }

    #[test]
    fn the_byte_budget_bounds_new_instruments_and_keeps_learning_known_ones() {
        let mut codes = SecurityIdRegistry::with_budget(2 * ENTRY_CHARGE);
        let mut first = apple();
        stated(&mut first, "bloomberg", "AAPL US Equity");
        codes.enrich(&mut first);
        assert_eq!(
            (codes.instruments(), codes.reserved_bytes()),
            (1, ENTRY_CHARGE)
        );

        let mut same = apple();
        stated(&mut same, "sedol", "2046251");
        codes.enrich(&mut same);
        assert_eq!(
            (codes.instruments(), codes.reserved_bytes()),
            (1, ENTRY_CHARGE),
            "a known ISIN consumes no second reservation"
        );

        codes.enrich(&mut numbered(1));
        assert_eq!(
            (codes.instruments(), codes.reserved_bytes()),
            (2, 2 * ENTRY_CHARGE)
        );
        for number in 2..128 {
            codes.enrich(&mut numbered(number));
        }
        assert_eq!(
            (codes.instruments(), codes.reserved_bytes()),
            (2, 2 * ENTRY_CHARGE),
            "repeated unseen instruments cannot grow a full registry"
        );

        let mut learned_at_cap = apple();
        learned_at_cap.set_cficode(Some(Cfi::new("ESVUFR").unwrap()), true);
        codes.enrich(&mut learned_at_cap);
        let mut later = apple();
        codes.enrich(&mut later);
        assert_eq!(later.get_cficode(), learned_at_cap.get_cficode());
        assert_eq!(code(&later, "sedol"), code(&same, "sedol"));
        assert_eq!(code(&later, "bloomberg"), code(&first, "bloomberg"));
        assert_eq!(codes.reserved_bytes(), 2 * ENTRY_CHARGE);

        let mut overflow = SecurityIdRegistry::saturated();
        overflow.enrich(&mut apple());
        assert_eq!(
            overflow.instruments(),
            0,
            "reservation arithmetic is checked"
        );
    }

    #[test]
    fn the_registry_learns_no_word_of_a_venue_s_own() {
        // A private source's code and a letter FIX names nothing are words
        // whose spelling the entry charge does not count: the message that
        // states one holds it, and no other message is filled with it.
        let mut registry = SecurityIdRegistry::default();
        let mut stated = Identifiers::new();
        stated.insert(id("isin", "US0378331005"));
        stated.insert(id("100", "HOUSE-1"));
        stated.insert(id("Z", "VENUE-7"));
        registry.learn(&stated, None);
        assert_eq!(registry.instruments(), 1);
        let mut asked = Identifiers::new();
        asked.insert(id("isin", "US0378331005"));
        let mut cfi = None;
        assert!(!registry.fill(&mut asked, &mut cfi));
        assert_eq!(asked.len(), 1);
    }

    #[test]
    fn the_registry_learns_one_association_per_stated_source_up_to_the_cap() {
        assert_eq!(MAX_KEYS_PER_INSTRUMENT, 8);
        let mut registry = SecurityIdRegistry::default();
        let mut stated = Identifiers::new();
        stated.insert(id("isin", "US0378331005"));
        for index in 0..10 {
            stated.insert(id(house(index).as_str(), &format!("h{index}")));
        }
        assert_eq!(stated.len(), 11);
        registry.learn(&stated, None);
        assert_eq!(registry.instruments(), 1);

        let mut asked = Identifiers::new();
        asked.insert(id("isin", "US0378331005"));
        let mut cfi = None;
        assert!(registry.fill(&mut asked, &mut cfi));
        assert_eq!(
            asked.len(),
            1 + 8,
            "eight sources learned, the rest dropped"
        );
        assert!(asked.contains_kind(&house(0)));
        assert!(asked.contains_kind(&house(7)));
        assert!(!asked.contains_kind(&house(8)));
        assert!(cfi.is_none());
        assert!(!registry.fill(&mut asked, &mut cfi), "nothing left to fill");

        let mut conflicting = Identifiers::new();
        conflicting.insert(id("isin", "US0378331005"));
        conflicting.insert(id(house(0).as_str(), "other"));
        registry.learn(&conflicting, Some(&Cfi::new("ESVUFR").unwrap()));
        let mut again = Identifiers::new();
        again.insert(id("isin", "US0378331005"));
        again.insert(id(house(1).as_str(), "mine"));
        let mut cfi = Some(Cfi::new("ESXXXX").unwrap());
        assert!(registry.fill(&mut again, &mut cfi));
        assert!(!again.contains_kind(&house(0)), "two codes seen: ambiguous");
        assert_eq!(again.get(&house(1)), Some("mine"), "a stated source stands");
        assert_eq!(again.get(&house(2)), Some("h2"));
        assert_eq!(cfi.as_ref().map(Cfi::as_str), Some("ESVUFR"));

        let mut without_isin = Identifiers::new();
        without_isin.insert(id("cusip", "037833100"));
        registry.learn(&without_isin, None);
        assert!(!registry.fill(&mut without_isin, &mut None));
        assert_eq!(
            registry.instruments(),
            1,
            "nothing is learned without an ISIN"
        );
    }
}

/// A symbol reads as the identifier its shape is, by its length first and
/// its type's check second; a symbol only the length of one is nothing.
#[test]
fn a_symbol_reads_as_the_identifier_its_shape_is() {
    use yggdryl::securityid::SymbolCode;

    for (symbol, expected) in [
        ("US0378331005", "isin"),
        (" US0378331005 ", "isin"),
        ("BBG000BLNQ16", "figi"),
        ("037833100", "cusip"),
        ("B0YBKJ7", "sedol"),
        ("ESVUFR", "cfi"),
        ("CH0012214059_XSWX_CHF", "instrument"),
        ("CH0012214059_XSWX_USDT", "instrument"),
        ("CH0012214059_XSWX_BABYDOGE", "instrument"),
        ("AAPL.OQ", "ric"),
        ("HOLN SW Equity", "bloomberg"),
    ] {
        let kind = match SymbolCode::from_symbol(symbol) {
            Some(SymbolCode::Isin(_)) => "isin",
            Some(SymbolCode::Figi(_)) => "figi",
            Some(SymbolCode::Cusip(_)) => "cusip",
            Some(SymbolCode::Sedol(_)) => "sedol",
            Some(SymbolCode::Cfi(_)) => "cfi",
            Some(SymbolCode::Instrument { .. }) => "instrument",
            Some(SymbolCode::Ric(_)) => "ric",
            Some(SymbolCode::Bloomberg(_)) => "bloomberg",
            None => "none",
        };
        assert_eq!(kind, expected, "{symbol}");
    }
    for symbol in [
        "AAPL",
        "US0378331006",
        "037833101",
        "GOOGLE",
        "CH0012214059_XSWX_CH",
        "CH0012214059_XSWX_BABYDOGES",
        "HOLN SW",
        "EURUSD",
        "",
    ] {
        assert_eq!(SymbolCode::from_symbol(symbol), None, "{symbol:?}");
    }
    let Some(SymbolCode::Instrument { isin, mic, ccy }) =
        SymbolCode::from_symbol("CH0012214059_XSWX_CHF")
    else {
        panic!("an instrument key");
    };
    assert_eq!(
        (
            isin.as_ref().map(yggdryl::Isin::as_str),
            mic.as_ref().map(yggdryl::Mic::as_str),
            ccy.as_ref().map(yggdryl::Ccy::as_str)
        ),
        (Some("CH0012214059"), Some("XSWX"), Some("CHF"))
    );
    // A part its type refuses is none, and the others still answer.
    assert!(matches!(
        SymbolCode::from_symbol("CH0012214058_XSWX_CHF"),
        Some(SymbolCode::Instrument {
            isin: None,
            mic: Some(_),
            ccy: Some(_)
        })
    ));
}
