//! A symbol: the one text an instrument is keyed by, read off the codes a
//! statement names, strongest first, and the global one for none.

use yggdryl::graph::{MarketElement, MarketEventData};
use yggdryl::market::Symbol;
use yggdryl::{Bloomberg, Cusip, Isin, Sedol};

/// An element naming its instrument under every code.
fn named() -> MarketEventData {
    let mut element = MarketEventData::at(1);
    element.set_isincode(Some(Isin::new("US0378331005").expect("an ISIN")));
    element.set_symbolticker(Some("AAPL".to_owned()));
    element.set_cusipcode(Some(Cusip::new("037833100").expect("a CUSIP")));
    element.set_sedolcode(Some(Sedol::new("2046251").expect("a SEDOL")));
    element.set_bloombergcode(Some(
        Bloomberg::new("BBG000B9XRY4").expect("a Bloomberg id"),
    ));
    element
}

#[test]
fn the_global_symbol_is_the_default_and_names_no_instrument() {
    assert_eq!(Symbol::default(), Symbol::GLOBAL);
    assert!(Symbol::GLOBAL.is_global());
    assert_eq!(Symbol::GLOBAL.as_str(), "GLOBAL");
    assert_eq!(
        Symbol::new("  "),
        Symbol::GLOBAL,
        "blank text is the global symbol"
    );
    assert_eq!(Symbol::new(" AAPL "), Symbol::new("AAPL"), "trimmed");
    assert!(!Symbol::new("AAPL").is_global());
    assert_eq!(
        Symbol::of(&MarketEventData::at(1)),
        Symbol::GLOBAL,
        "nothing named"
    );
}

#[test]
fn a_statement_keys_the_strongest_code_it_names() {
    let mut element = named();
    assert_eq!(
        Symbol::of(&element).as_str(),
        "US0378331005",
        "the ISIN leads"
    );
    element.set_isincode(None);
    assert_eq!(Symbol::of(&element).as_str(), "AAPL", "then the ticker");
    element.set_symbolticker(Some("   ".to_owned()));
    assert_eq!(
        Symbol::of(&element).as_str(),
        "037833100",
        "a blank ticker names nothing"
    );
    element.set_cusipcode(None);
    assert_eq!(Symbol::of(&element).as_str(), "2046251");
    element.set_sedolcode(None);
    assert_eq!(Symbol::of(&element).as_str(), "BBG000B9XRY4");
    element.set_bloombergcode(None);
    assert_eq!(Symbol::of(&element), Symbol::GLOBAL);
}

#[test]
fn symbols_order_by_their_text_and_display_it() {
    let mut symbols = [Symbol::new("MSFT"), Symbol::GLOBAL, Symbol::new("AAPL")];
    symbols.sort();
    assert_eq!(
        symbols.iter().map(Symbol::as_str).collect::<Vec<_>>(),
        ["AAPL", "GLOBAL", "MSFT"]
    );
    assert_eq!(Symbol::new("AAPL").to_string(), "AAPL");
    assert_eq!(Symbol::new("AAPL").as_ref(), "AAPL");
}
