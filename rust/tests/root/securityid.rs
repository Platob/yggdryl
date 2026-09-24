//! `rust/src/securityid.rs`: the source that names an instrument, the code it
//! gives it, the sorted set a market states, and the national number an ISIN
//! embeds. The registry a lifecycle learns with is a lifecycle's own -
//! nothing above it names one - so what it learns, refuses and costs is
//! reached through `yggdryl::internals` in [`internal`].

use std::path::PathBuf;

use yggdryl::securityid::embedded;
use yggdryl::{Error, IsinCode, Scalar, SecType, SecurityId, SecurityIds};

fn located<T>(result: yggdryl::Result<T>) -> (String, String) {
    match result.err().expect("a refusal") {
        Error::InvalidRecord { path, reason } => (path.to_string(), reason.to_string()),
        other => panic!("expected a located refusal, got {other}"),
    }
}

fn id(key: &str, code: &str) -> SecurityId {
    SecurityId::new(SecType::read(key).unwrap(), code).unwrap()
}

#[test]
fn a_sectype_reads_codes_keys_and_names_and_keeps_any_other_key() {
    assert_eq!(SecType::read("4").unwrap().as_str(), "ISIN");
    assert_eq!(SecType::read("A").unwrap().as_str(), "BLOOMBERG");
    assert_eq!(SecType::read("isin").unwrap().as_str(), "ISIN");
    assert_eq!(SecType::read(" Isin ").unwrap().as_str(), "ISIN");
    assert_eq!(SecType::read("ISINNumber").unwrap().as_str(), "ISIN");
    assert_eq!(SecType::read("isin_number").unwrap().as_str(), "ISIN");
    assert_eq!(SecType::read("RIC Code").unwrap().as_str(), "RIC");
    assert_eq!(SecType::read("bbgsymb").unwrap().as_str(), "BLOOMBERG");
    assert_eq!(
        SecType::read("BloombergSymbol").unwrap().as_str(),
        "BLOOMBERG"
    );
    assert_eq!(
        SecType::read("FinancialInstrumentGlobalIdentifier")
            .unwrap()
            .as_str(),
        "FIGI"
    );
    assert_eq!(SecType::read("Wertpapier").unwrap().as_str(), "WKN");
    assert_eq!(SecType::read("Valoren").unwrap().as_str(), "VALOR");

    let house = SecType::read("house-key").unwrap();
    assert_eq!(house.as_str(), "HOUSE-KEY");
    assert!(!house.is_known());
    assert_eq!(house.fix_source(), None);
    assert_eq!(house.max_code_width(), 32);
    assert!(
        SecType::read("a").unwrap().as_str() == "A" && !SecType::read("a").unwrap().is_known(),
        "a wire code does not fold: a lower-case letter is an unknown key"
    );
    assert_eq!(SecType::read("Z").unwrap().as_str(), "Z");

    for (key, code) in SecType::KNOWN {
        let read = SecType::read(key).unwrap();
        assert!(read.is_known());
        assert_eq!(read.fix_source(), Some(code));
        assert_eq!(SecType::from_fix_source(code), Some(read.clone()));
        assert_eq!(SecType::read(&code.to_string()).unwrap(), read);
        assert_eq!(SecType::read(&key.to_ascii_lowercase()).unwrap(), read);
    }
    assert_eq!(SecType::from_fix_source('Z'), None);
    assert_eq!(SecType::from_fix_source('a'), None);

    let (path, reason) = located(SecType::read("ticker"));
    assert_eq!(path, "ticker");
    assert!(reason.contains("set_ticker"), "{reason}");
    assert!(SecType::read("TICKER").is_err());
    let (path, reason) = located(SecType::read(""));
    assert_eq!(path, "");
    assert_eq!(
        reason,
        "expected a security identifier source of 1 to 32 ASCII bytes, got \"\""
    );
    let (path, reason) = located(SecType::read("caf\u{e9}"));
    assert_eq!(path, "caf\u{e9}");
    assert!(reason.contains("a non-ASCII byte 0xC3 at 3"), "{reason}");
    let wide = "k".repeat(33);
    let (path, reason) = located(SecType::read(&wide));
    assert_eq!(path, wide);
    assert!(reason.ends_with("got 33 bytes"), "{reason}");
    assert_eq!(
        SecType::read(&"k".repeat(32)).unwrap().as_str(),
        "K".repeat(32)
    );
    assert!(SecType::read("tab\tkey").is_err());

    assert!(SecType::read("CUSIP").unwrap() < SecType::read("ISIN").unwrap());
    assert_eq!(SecType::read("isin").unwrap().to_string(), "ISIN");
}

#[test]
fn the_known_table_agrees_with_the_code_set_exactly() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../config/fix/codesets/securityidsourcecodeset.json");
    let document = yggdryl::from_json_scalar(std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        document.get_key_str("name").and_then(Scalar::as_str),
        Some("securityidsourcecodeset")
    );
    let codes = document.get_key_str("codes").unwrap();
    assert_eq!(codes.len(), SecType::KNOWN.len());
    let mut keys = Vec::new();
    for index in 0..codes.len() {
        let code = codes.get(index).unwrap();
        let value = code.get_key_str("value").and_then(Scalar::as_str).unwrap();
        let name = code.get_key_str("name").and_then(Scalar::as_str).unwrap();
        let mut chars = value.chars();
        let (Some(code), None) = (chars.next(), chars.next()) else {
            panic!("a one-character code, got {value:?}");
        };
        let key = SecType::from_fix_source(code)
            .unwrap_or_else(|| panic!("code {code} names a known source"));
        assert_eq!(key.fix_source(), Some(code), "one code per key");
        assert_eq!(
            SecType::KNOWN[index],
            (key.as_str(), code),
            "the code set's order"
        );
        assert_eq!(SecType::read(name).unwrap(), key, "{name} reads to {key}");
        assert_eq!(SecType::read(value).unwrap(), key, "{value} reads to {key}");
        keys.push(key);
    }
    let mut unique = keys.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), keys.len(), "one key per code");
}

#[test]
fn a_field_name_names_one_instruments_own_identifier_source() {
    let source = |name: &str| SecType::from_field_name(name).map(|key| key.as_str().to_owned());
    assert_eq!(source("isincode").as_deref(), Some("ISIN"));
    assert_eq!(source("#isincode").as_deref(), Some("ISIN"));
    assert_eq!(source("ISIN_Code").as_deref(), Some("ISIN"));
    assert_eq!(source("isin").as_deref(), Some("ISIN"));
    assert_eq!(source("ISINNumber").as_deref(), Some("ISIN"));
    assert_eq!(source("isin_number").as_deref(), Some("ISIN"));
    assert_eq!(source("security isin").as_deref(), Some("ISIN"));
    assert_eq!(source("SecurityISINCode").as_deref(), Some("ISIN"));
    assert_eq!(source("cusipcode").as_deref(), Some("CUSIP"));
    assert_eq!(source("sedol_code").as_deref(), Some("SEDOL"));
    assert_eq!(source("bloombergcode").as_deref(), Some("BLOOMBERG"));
    assert_eq!(source("bloomberg_symbol").as_deref(), Some("BLOOMBERG"));
    assert_eq!(source("bbgsymb").as_deref(), Some("BLOOMBERG"));
    assert_eq!(source("figicode").as_deref(), Some("FIGI"));
    assert_eq!(source("figi_id").as_deref(), Some("FIGI"));
    assert_eq!(source("ric").as_deref(), Some("RIC"));
    assert_eq!(source("RICCode").as_deref(), Some("RIC"));
    assert_eq!(source("wkn").as_deref(), Some("WKN"));
    assert_eq!(source("wertpapier").as_deref(), Some("WKN"));
    assert_eq!(source("valor").as_deref(), Some("VALOR"));
    assert_eq!(source("lei").as_deref(), Some("LEI"));
    assert_eq!(source("LegalEntityIdentifier").as_deref(), Some("LEI"));
    assert_eq!(source("exchange_symbol").as_deref(), Some("EXCHSYMB"));
    assert_eq!(source("cusip_number").as_deref(), Some("CUSIP"));

    for refused in [
        "ticker",
        "symbol",
        "symbolticker",
        "Symbol_Ticker",
        "#symbol",
        "legsecurityid",
        "leg_isin",
        "underlyingisin",
        "UnderlyingSecurityID",
        "contracusip",
        "relatedsedol",
        "benchmark_isin",
        "securityid",
        "security",
        "code",
        "id",
        "",
        "#",
        "price",
        "housekey",
        "isincodes",
    ] {
        assert_eq!(source(refused), None, "{refused:?} names no source");
    }
}

#[test]
fn each_source_holds_its_code_to_its_own_rule() {
    let width = |key: &str| SecType::read(key).unwrap().max_code_width();
    assert_eq!(width("ISIN"), 12);
    assert_eq!(width("CUSIP"), 9);
    assert_eq!(width("SEDOL"), 7);
    assert_eq!(width("FIGI"), 12);
    assert_eq!(width("WKN"), 6);
    assert_eq!(width("VALOR"), 9);
    assert_eq!(width("ISOCCY"), 3);
    assert_eq!(width("ISOCTRY"), 2);
    assert_eq!(width("BLOOMBERG"), 32);
    assert_eq!(width("RIC"), 32);
    assert_eq!(width("HOUSE"), 32);

    let accepts = |key: &str, code: &str| SecType::read(key).unwrap().validate_code(code).is_ok();
    assert!(accepts("ISIN", "US0378331005"));
    assert!(accepts("ISIN", "us0378331005"));
    assert!(
        !accepts("ISIN", "US0378331006"),
        "the check digit must close"
    );
    assert!(accepts("CUSIP", "037833100"));
    assert!(!accepts("CUSIP", "037833101"));
    assert!(accepts("SEDOL", "0263494"));
    assert!(!accepts("SEDOL", "0263495"));
    assert!(accepts("FIGI", "BBG000B9XRY4"));
    assert!(!accepts("FIGI", "BBG000B9XRY5"));
    assert!(accepts("WKN", "716460"));
    assert!(accepts("WKN", "BASF11"));
    assert!(accepts("WKN", "basf11"));
    assert!(!accepts("WKN", "BASI11"), "no I");
    assert!(!accepts("WKN", "BASO11"), "no O");
    assert!(!accepts("WKN", "71646"));
    assert!(!accepts("WKN", "7164600"));
    assert!(accepts("VALOR", "3886335"));
    assert!(accepts("VALOR", "1"));
    assert!(accepts("VALOR", "123456789"));
    assert!(!accepts("VALOR", "0"));
    assert!(!accepts("VALOR", "03886335"), "no leading zero");
    assert!(!accepts("VALOR", "1234567890"));
    assert!(!accepts("VALOR", "38A6335"));
    assert!(accepts("BLOOMBERG", "AAPL US Equity"));
    assert!(accepts("BLOOMBERG", &"B".repeat(32)));
    assert!(!accepts("BLOOMBERG", &"B".repeat(33)));
    assert!(!accepts("BLOOMBERG", "AAPL\u{a0}US"));
    for null in ["", "null", "NULL", "none", "n/a", "[N/A]"] {
        assert!(!accepts("BLOOMBERG", null), "{null:?}");
        assert!(!accepts("HOUSE", null), "{null:?}");
        assert!(!accepts("ISIN", null), "{null:?}");
    }
    assert!(accepts("ISOCCY", "USD"));
    assert!(!accepts("ISOCCY", "USDX"));
    assert!(accepts("ISOCTRY", "US"));
    assert!(!accepts("ISOCTRY", "USA"));
    assert!(accepts("RIC", "AAPL.OQ"));
    assert!(accepts("HOUSE", &"h".repeat(32)));
    assert!(!accepts("HOUSE", &"h".repeat(33)));
    assert!(!accepts("HOUSE", "tab\tcode"));

    let (path, reason) = located(SecType::read("WKN").unwrap().validate_code("BASI11"));
    assert_eq!(path, "WKN");
    assert_eq!(
        reason,
        "expected a WKN code, got \"BASI11\", not six of [0-9A-HJ-NP-Z]"
    );
    let (path, reason) = located(SecType::read("BLOOMBERG").unwrap().validate_code("n/a"));
    assert_eq!(path, "BLOOMBERG");
    assert_eq!(
        reason,
        "expected a BLOOMBERG code, got \"n/a\", which states nothing"
    );
    assert!(matches!(
        SecType::read("ISIN").unwrap().validate_code("US0378331006"),
        Err(Error::InvalidDataType { kind: "isin", .. })
    ));
}

#[test]
fn a_security_id_holds_source_and_code_in_one_inline_buffer() {
    assert_eq!(std::mem::size_of::<SecurityId>(), 24);
    let apple = id("isin", " us0378331005 ");
    assert_eq!(apple.to_string(), "ISIN:US0378331005");
    assert_eq!(apple.sectype().as_str(), "ISIN");
    assert_eq!(apple.code(), "US0378331005");
    assert!(apple.is_inline());
    assert_eq!(apple, id("4", "US0378331005"));
    assert_eq!(apple.clone(), apple);

    let bloomberg = id("A", "aapl us Equity");
    assert_eq!(
        bloomberg.code(),
        "aapl us Equity",
        "case and inner spaces kept"
    );
    assert_eq!(bloomberg.to_string(), "BLOOMBERG:aapl us Equity");
    let long = id("bloomberg", &"B".repeat(32));
    assert!(!long.is_inline());
    assert_eq!(long.code().len(), 32);
    assert_eq!(long.clone(), long);
    assert!(SecurityId::new(SecType::read("bloomberg").unwrap(), &"B".repeat(33)).is_err());

    let house = id("house-key", "hk-1");
    assert_eq!(house.to_string(), "HOUSE-KEY:hk-1");
    assert_eq!(house.sectype(), SecType::read("HOUSE-KEY").unwrap());
    assert_eq!(house.code(), "hk-1");
    assert!(house.is_inline());
    let widest = id(&"k".repeat(32), &"c".repeat(32));
    assert_eq!(widest.sectype().as_str(), "K".repeat(32));
    assert_eq!(widest.code(), "c".repeat(32));
    assert!(!widest.is_inline());

    let wkn = id("wkn", "basf11");
    assert_eq!(
        wkn.code(),
        "BASF11",
        "a case-folding source's code is upper-cased"
    );
    assert_eq!(id("figi", "bbg000b9xry4").code(), "BBG000B9XRY4");
    assert_eq!(id("cusip", "037833100").to_string(), "CUSIP:037833100");
    assert_eq!(id("sedol", "b4bnmy3").to_string(), "SEDOL:B4BNMY3");
    assert_eq!(id("valor", "3886335").to_string(), "VALOR:3886335");

    let mut ids = [
        id("isin", "US0378331005"),
        id("cusip", "037833100"),
        id("house", "b"),
        id("house", "a"),
        id("bloomberg", "AAPL US Equity"),
        id("isin", "DE0007164600"),
    ];
    ids.sort();
    assert_eq!(
        ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
        [
            "BLOOMBERG:AAPL US Equity",
            "CUSIP:037833100",
            "HOUSE:a",
            "HOUSE:b",
            "ISIN:DE0007164600",
            "ISIN:US0378331005",
        ],
        "ordered by source then code, never by the tag"
    );
    assert_eq!(
        id("isin", "US0378331005").partial_cmp(&id("isin", "US0378331005")),
        Some(std::cmp::Ordering::Equal)
    );
    assert_ne!(id("house", "a"), id("houses", "a"));
}

#[test]
fn security_ids_fill_set_remove_merge_and_answer_every_spelling_of_a_source() {
    let mut ids = SecurityIds::default();
    assert!(ids.is_empty());
    assert_eq!(ids.len(), 0);
    assert!(ids.insert(id("cusip", "037833100")));
    assert!(ids.insert(id("isin", "US0378331005")));
    assert!(!ids.insert(id("isin", "DE0007164600")), "fill only");
    assert_eq!(ids.get("isin"), Some("US0378331005"));
    assert_eq!(ids.get("4"), ids.get("isin"));
    assert_eq!(ids.get("ISINNumber"), ids.get("isin"));
    assert_eq!(ids.get("1"), Some("037833100"));
    assert_eq!(ids.get_id("cusip"), Some(&id("cusip", "037833100")));
    assert!(ids.contains_key("Cusip"));
    assert!(!ids.contains_key("sedol"));
    assert!(!ids.contains_key("ticker"), "a ticker is never a source");
    assert_eq!(ids.get("caf\u{e9}"), None);
    assert_eq!(ids.len(), 2);
    assert_eq!(
        ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
        ["CUSIP:037833100", "ISIN:US0378331005"]
    );
    assert_eq!(&ids[..], ids.iter().as_slice(), "Deref to the sorted slice");
    assert_eq!(ids.first().map(|id| id.code()), Some("037833100"));

    assert!(
        ids.set(id("isin", "DE0007164600")),
        "a replacement changes the set"
    );
    assert!(
        !ids.set(id("isin", "DE0007164600")),
        "the same identifier does not"
    );
    assert!(
        ids.set(id("bloomberg", "BAS GY Equity")),
        "set fills an absent source"
    );
    assert_eq!(
        ids.iter()
            .map(|id| id.sectype().as_str().to_owned())
            .collect::<Vec<_>>(),
        ["BLOOMBERG", "CUSIP", "ISIN"]
    );
    assert_eq!(
        ids.remove(&SecType::read("cusip").unwrap()),
        Some(id("cusip", "037833100"))
    );
    assert_eq!(ids.remove(&SecType::read("cusip").unwrap()), None);
    assert_eq!(ids.len(), 2);

    let mut theirs = SecurityIds::default();
    theirs.insert(id("isin", "US0378331005"));
    theirs.insert(id("wkn", "BASF11"));
    theirs.insert(id("aa", "1"));
    assert!(ids.merge(&theirs));
    assert_eq!(
        ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
        [
            "AA:1",
            "BLOOMBERG:BAS GY Equity",
            "ISIN:DE0007164600",
            "WKN:BASF11"
        ],
        "this side wins"
    );
    assert!(!ids.merge(&theirs));
    assert!(!ids.merge(&SecurityIds::default()));
    let mut empty = SecurityIds::default();
    assert!(empty.merge(&theirs));
    assert_eq!(empty, theirs);
    assert_eq!(ids.clone(), ids);
    let mut by_reference = 0;
    for _ in &ids {
        by_reference += 1;
    }
    assert_eq!(by_reference, 4);
    assert_eq!(std::mem::size_of::<SecurityIds>(), 56);
}

#[test]
fn security_ids_round_trip_through_arrow_and_refuse_a_map_they_cannot_hold() {
    assert_eq!(SecurityIds::dtype(), yggdryl::IdMap::dtype());
    let mut ids = SecurityIds::default();
    ids.insert(id("isin", "US0378331005"));
    ids.insert(id("bloomberg", "AAPL US Equity"));
    ids.insert(id("house", "hk-1"));
    let scalar = ids.to_scalar();
    assert_eq!(scalar.kind(), "sorted_map");
    assert_eq!(
        scalar
            .as_mapping()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.as_str().unwrap(), value.as_str().unwrap()))
            .collect::<Vec<_>>(),
        [
            ("BLOOMBERG", "AAPL US Equity"),
            ("HOUSE", "hk-1"),
            ("ISIN", "US0378331005")
        ]
    );
    assert_eq!(SecurityIds::from_scalar(&scalar).unwrap(), ids);
    assert_eq!(
        SecurityIds::from_scalar(&SecurityIds::default().to_scalar()).unwrap(),
        SecurityIds::default()
    );

    let mapping = |entries: &[(&str, &str)]| match Scalar::from_mapping(
        entries
            .iter()
            .map(|(key, value)| (Scalar::from(*key), Scalar::from(*value))),
    )
    .unwrap()
    {
        Scalar::Map(entries) => Scalar::SortedMap(entries),
        other => other,
    };
    let refused = |scalar: Scalar| located(SecurityIds::from_scalar(&scalar));
    assert_eq!(refused(Scalar::from("ISIN")).0, "$");
    assert_eq!(
        refused(mapping(&[("ISIN", "US0378331005"), ("4", "US0378331005")])).0,
        "$[1].key",
        "one source under two spellings"
    );
    assert_eq!(
        refused(mapping(&[("ISIN", "US0378331005"), ("CUSIP", "037833100")])).0,
        "$[1].key",
        "out of order"
    );
    let unchecked = refused(mapping(&[("ISIN", "US0378331006")]));
    assert_eq!(unchecked.0, "$[0].value");
    assert!(unchecked.1.contains("check digit"), "{}", unchecked.1);
    let ticker = refused(mapping(&[("ticker", "AAPL")]));
    assert_eq!(ticker.0, "$[0].key");
    assert!(ticker.1.contains("set_ticker"), "{}", ticker.1);
    let wkn = located(SecurityIds::from_scalar(&mapping(&[("WKN", "BASI11")])));
    assert_eq!(wkn.0, "$[0].value");
    assert!(wkn.1.contains("[0-9A-HJ-NP-Z]"), "{}", wkn.1);
    let null = match Scalar::from_mapping([(Scalar::from("ISIN"), Scalar::Null)]).unwrap() {
        Scalar::Map(entries) => Scalar::SortedMap(entries),
        other => other,
    };
    assert_eq!(refused(null).0, "$[0].value");
}

fn isin(body: &str) -> IsinCode {
    let digit = IsinCode::closing_digit(body).unwrap();
    IsinCode::new(format!("{body}{digit}")).unwrap()
}

#[test]
fn embedded_names_the_national_number_a_canonical_isin_carries() {
    let found = |text: &str| {
        embedded(&IsinCode::new(text).unwrap())
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(found("US0378331005"), ["CUSIP:037833100"]);
    assert_eq!(found("CA1125851040"), ["CUSIP:112585104"]);
    assert_eq!(found("GB0002634946"), ["SEDOL:0263494"]);
    assert_eq!(found("IE00B4BNMY34"), ["SEDOL:B4BNMY3"]);
    assert_eq!(found("JE00B4T3BW64"), ["SEDOL:B4T3BW6"]);
    assert_eq!(found("DE0007164600"), ["WKN:716460"]);
    assert_eq!(found("DE000BASF111"), ["WKN:BASF11"]);
    assert_eq!(found("CH0038863350"), ["VALOR:3886335"]);
    assert_eq!(found("LI0010737216"), ["VALOR:1073721"]);
    assert!(found("XS0203470157").is_empty());
    assert!(found("FR0000120271").is_empty());

    let none = |isin: &IsinCode| embedded(isin).next().is_none();
    assert!(none(&isin("GB000263495")), "a bad embedded SEDOL check");
    assert!(none(&isin("GB010263494")), "a GB ISIN not starting GB00");
    assert!(none(&isin("IE01B4BNMY3")), "an IE ISIN not starting IE00");
    assert!(none(&isin("DE001716460")), "a DE ISIN not starting DE000");
    assert!(none(&isin("DE000BASI11")), "a WKN holding I");
    assert!(none(&isin("DE000BASO11")), "a WKN holding O");
    assert!(none(&isin("CH000000000")), "a Valor number of nothing");
    assert!(none(&IsinCode::default()), "an empty ISIN");
    let unchecked: IsinCode = serde_json::from_str("\"US0378331006\"").unwrap();
    assert!(IsinCode::new("US0378331006").is_err());
    assert!(none(&unchecked), "a bad ISIN check digit");
    let lower: IsinCode = serde_json::from_str("\"us0378331005\"").unwrap();
    assert!(none(&lower), "not canonical");
}

#[cfg(feature = "internals")]
mod internal {
    //! The registry, reached through `yggdryl::internals::securityid`.

    use yggdryl::graph::{MarketElement, MarketElementData, MarketEventData};
    use yggdryl::internals::securityid::{
        ENTRY_CHARGE, MAX_KEYS_PER_INSTRUMENT, SecurityIdRegistry,
    };
    use yggdryl::{BloombergCode, CfiCode, CusipCode, IsinCode, SecurityIds, SedolCode};

    use super::id;

    fn apple() -> MarketEventData {
        let mut event = MarketEventData::at(1);
        event.set_isincode(Some(IsinCode::new("US0378331005").unwrap()));
        event
    }

    fn numbered(number: usize) -> MarketEventData {
        let body = format!("FR{number:09}");
        let digit = IsinCode::closing_digit(&body).unwrap();
        let mut event = MarketEventData::at(number as i64);
        event.set_isincode(Some(IsinCode::new(format!("{body}{digit}")).unwrap()));
        event
    }

    #[test]
    fn learned_codes_are_local_validated_and_ambiguous_defaults_are_silent() {
        let mut codes = SecurityIdRegistry::default();
        let mut first = apple();
        first.set_cficode(Some(CfiCode::new("ESXXXX").unwrap()));
        first.set_bloombergcode(Some(BloombergCode::new("AAPL US Equity").unwrap()));
        codes.enrich(&mut first);
        let mut next = apple();
        codes.enrich(&mut next);
        assert!(
            next.get_cficode().is_none(),
            "coarse classifications are not learned"
        );
        assert_eq!(next.get_bloombergcode(), first.get_bloombergcode());

        let mut precise = apple();
        precise.set_cficode(Some(CfiCode::new("ESVUFR").unwrap()));
        precise.set_sedolcode(Some(SedolCode::new("2046251").unwrap()));
        codes.enrich(&mut precise);
        let mut later = apple();
        codes.enrich(&mut later);
        assert_eq!(later.get_cficode(), precise.get_cficode());
        assert_eq!(later.get_sedolcode(), precise.get_sedolcode());
        assert_eq!(
            first.get_cficode().unwrap().as_str(),
            "ESXXXX",
            "earlier snapshots stay unchanged"
        );
        let mut coarse = apple();
        coarse.set_cficode(Some(CfiCode::new("ESXXXX").unwrap()));
        codes.enrich(&mut coarse);
        assert_eq!(coarse.get_cficode(), precise.get_cficode());

        let mut conflict = apple();
        conflict.set_bloombergcode(Some(BloombergCode::new("AAPL LN Equity").unwrap()));
        codes.enrich(&mut conflict);
        assert_eq!(
            conflict.get_bloombergcode().unwrap().as_str(),
            "AAPL LN Equity"
        );
        let mut after_conflict = apple();
        codes.enrich(&mut after_conflict);
        assert!(after_conflict.get_bloombergcode().is_none());

        let mut independent = apple();
        SecurityIdRegistry::default().enrich(&mut independent);
        assert!(independent.get_cficode().is_none());
        assert!(independent.get_bloombergcode().is_none());
    }

    #[test]
    fn invalid_default_codes_cannot_seed_associations() {
        let mut codes = SecurityIdRegistry::default();
        let mut empty = MarketElementData::default();
        empty.set_isincode(Some(IsinCode::default()));
        codes.enrich(&mut empty);
        assert_eq!(codes.instruments(), 0);
        assert_eq!(codes.reserved_bytes(), 0);
        let mut observed = apple();
        observed.set_cficode(Some(CfiCode::new("XXXXXX").unwrap()));
        observed.set_cusipcode(Some(CusipCode::default()));
        observed.set_sedolcode(Some(SedolCode::default()));
        observed.set_bloombergcode(Some(BloombergCode::default()));
        codes.enrich(&mut observed);
        let mut later = apple();
        codes.enrich(&mut later);
        assert!(later.get_cficode().is_none());
        assert!(later.get_sedolcode().is_none());
        assert!(later.get_bloombergcode().is_none());
        assert_eq!(
            later.get_cusipcode().map(CusipCode::as_str),
            Some("037833100"),
            "the CUSIP a US ISIN embeds is the element's own, not a learned one"
        );
    }

    #[test]
    fn the_byte_budget_bounds_new_instruments_and_keeps_learning_known_ones() {
        let mut codes = SecurityIdRegistry::with_budget(2 * ENTRY_CHARGE);
        let mut first = apple();
        first.set_bloombergcode(Some(BloombergCode::new("AAPL US Equity").unwrap()));
        codes.enrich(&mut first);
        assert_eq!(
            (codes.instruments(), codes.reserved_bytes()),
            (1, ENTRY_CHARGE)
        );

        let mut same = apple();
        same.set_sedolcode(Some(SedolCode::new("2046251").unwrap()));
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
        learned_at_cap.set_cficode(Some(CfiCode::new("ESVUFR").unwrap()));
        codes.enrich(&mut learned_at_cap);
        let mut later = apple();
        codes.enrich(&mut later);
        assert_eq!(later.get_cficode(), learned_at_cap.get_cficode());
        assert_eq!(later.get_sedolcode(), same.get_sedolcode());
        assert_eq!(later.get_bloombergcode(), first.get_bloombergcode());
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
    fn the_registry_learns_one_association_per_stated_source_up_to_the_cap() {
        assert_eq!(MAX_KEYS_PER_INSTRUMENT, 8);
        let mut registry = SecurityIdRegistry::default();
        let mut stated = SecurityIds::default();
        stated.insert(id("isin", "US0378331005"));
        for index in 0..10 {
            stated.insert(id(&format!("HOUSE{index}"), &format!("h{index}")));
        }
        assert_eq!(stated.len(), 11);
        registry.learn(&stated, None);
        assert_eq!(registry.instruments(), 1);

        let mut asked = SecurityIds::default();
        asked.insert(id("isin", "US0378331005"));
        let mut cfi = None;
        assert!(registry.fill(&mut asked, &mut cfi));
        assert_eq!(
            asked.len(),
            1 + 8,
            "eight sources learned, the rest dropped"
        );
        assert!(asked.contains_key("HOUSE0"));
        assert!(asked.contains_key("HOUSE7"));
        assert!(!asked.contains_key("HOUSE8"));
        assert!(cfi.is_none());
        assert!(!registry.fill(&mut asked, &mut cfi), "nothing left to fill");

        let mut conflicting = SecurityIds::default();
        conflicting.insert(id("isin", "US0378331005"));
        conflicting.insert(id("HOUSE0", "other"));
        registry.learn(&conflicting, Some(&CfiCode::new("ESVUFR").unwrap()));
        let mut again = SecurityIds::default();
        again.insert(id("isin", "US0378331005"));
        again.insert(id("HOUSE1", "mine"));
        let mut cfi = Some(CfiCode::new("ESXXXX").unwrap());
        assert!(registry.fill(&mut again, &mut cfi));
        assert!(!again.contains_key("HOUSE0"), "two codes seen: ambiguous");
        assert_eq!(again.get("HOUSE1"), Some("mine"), "a stated source stands");
        assert_eq!(again.get("HOUSE2"), Some("h2"));
        assert_eq!(cfi.as_ref().map(CfiCode::as_str), Some("ESVUFR"));

        let mut without_isin = SecurityIds::default();
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
