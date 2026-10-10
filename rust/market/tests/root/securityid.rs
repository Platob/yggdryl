//! `rust/market/src/securityid.rs`: the national number an ISIN embeds as a derived
//! security [`Identifier`], and the shape a symbol reads as. The type of name
//! a security identifier is - its `SecurityIDSource(22)` code, its field
//! names, the rule its value follows - is `rust/market/tests/root/idtype.rs`'s, and
//! what a lifecycle learns between an instrument's identifiers is
//! `rust/market/tests/root/instrument.rs`'s.

use yggdryl::Isin;
use yggdryl_market::securityid::embedded;
use yggdryl_market::{IdKey, IdType, Identifier, Identifiers};

/// One security identifier of `key` - a type's name or its FIX source code -
/// from `base`, validated by its type.
fn id(key: &str, code: &str) -> Identifier {
    Identifier::new(
        IdKey::base(IdType::from_security_source(key).unwrap()),
        code,
    )
    .unwrap()
}

/// A set of security identifiers holds one value per source and type,
/// sorted, and answers a type however its spelling folds.
#[test]
fn security_identifiers_are_held_per_source_and_type_in_key_order() {
    crate::install::installed();
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
            "bloomberg=AAPL US Equity",
            "cusip=037833100",
            "house=b",
            "isin=US0378331005",
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
    // instrument, held beside the one the base key states; a derivation
    // never lands over a statement.
    let mut both = ids.clone();
    let venue = IdKey::new("venue".parse().unwrap(), IdType::Cusip);
    assert!(both.insert(Identifier::new(venue, "037833100").unwrap()));
    assert_eq!(both.get(&IdType::Cusip), Some("037833100"));
    assert_eq!(both.of_kind(&IdType::Cusip).count(), 2);
    let derived = IdKey::new("derived".parse().unwrap(), IdType::Cusip);
    assert!(!both.insert(Identifier::new(derived, "037833100").unwrap()));
}

fn isin(body: &str) -> Isin {
    let digit = Isin::closing_digit(body).unwrap();
    Isin::new(format!("{body}{digit}")).unwrap()
}

#[test]
fn embedded_names_the_national_number_a_canonical_isin_carries() {
    crate::install::installed();
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
    let unchecked = Isin::new("US0378331006").unwrap();
    assert!(!Isin::is_closed(unchecked.as_str()));
    assert!(none(&unchecked), "a bad ISIN check digit");
    let lower: Isin = serde_json::from_str("\"us0378331005\"").unwrap();
    assert!(none(&lower), "not canonical");
}

#[test]
fn a_symbol_reads_as_the_identifier_its_shape_is() {
    crate::install::installed();
    use yggdryl_market::securityid::SymbolCode;

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
