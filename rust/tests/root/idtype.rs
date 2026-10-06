//! `rust/src/idtype.rs`: the type of name an identifier is - one vocabulary for
//! security, operation and party identifiers - the FIX `SecurityIDSource(22)`
//! codes its security members carry, the field names that name them, the rule
//! each type holds its values to, which `Identifier::new` applies, and the
//! parent types a base lists and a parent reads back to its base.

use std::borrow::Cow;
use std::path::PathBuf;

use yggdryl::{DataType, Error, IdKey, IdType, Identifier, Scalar};

fn located<T>(result: yggdryl::Result<T>) -> (String, String) {
    match result.err().expect("a refusal") {
        Error::InvalidRecord { path, reason } => (path.to_string(), reason.to_string()),
        other => panic!("expected a located refusal, got {other}"),
    }
}

fn kind(text: &str) -> IdType {
    text.parse().unwrap()
}

/// One identifier of `text`'s type from `base`, validated by its type.
fn id(text: &str, value: &str) -> yggdryl::Result<Identifier> {
    Identifier::new(IdKey::base(kind(text)), value)
}

/// The security types FIX names, each with its code, in the code set's order.
fn fix_sources() -> Vec<(IdType, char)> {
    IdType::KNOWN
        .iter()
        .filter_map(|known| {
            known
                .fix_security_source()
                .map(|code| (known.clone(), code))
        })
        .collect()
}

#[test]
fn a_type_reads_codes_keys_and_names_and_keeps_any_other_word() {
    for (spelling, expected) in [
        ("isin", IdType::Isin),
        (" Isin ", IdType::Isin),
        ("ISIN", IdType::Isin),
        ("ISINNumber", IdType::Isin),
        ("isin_number", IdType::Isin),
        ("ISIN-Number", IdType::Isin),
        ("RIC Code", IdType::Ric),
        ("bbgsymb", IdType::Bloomberg),
        ("BloombergSymbol", IdType::Bloomberg),
        ("Bloomberg", IdType::Bloomberg),
        ("FinancialInstrumentGlobalIdentifier", IdType::Figi),
        ("Wertpapier", IdType::Wkn),
        ("Valoren", IdType::Valor),
        ("X-SWX-VALOR", IdType::Valor),
        ("x_swx_valor", IdType::Valor),
        ("Valorennummer", IdType::Valor),
        ("SIX Symbol", IdType::ExchSymb),
        ("Valorensymbol", IdType::ExchSymb),
        ("ClOrdID", IdType::ClOrdId),
        ("cl_ord_id", IdType::ClOrdId),
        ("#OrderID", IdType::OrderId),
        ("Executing Trader", IdType::ExecutingTrader),
        ("TradingVenueTransactionIdentifier", IdType::Tvtic),
    ] {
        let read: IdType = spelling.parse().unwrap();
        assert_eq!(read, expected, "{spelling:?}");
        assert!(read.is_known(), "{spelling:?}");
    }

    let house = kind("house-key");
    assert_eq!(house.as_str(), "housekey");
    assert!(!house.is_known());
    assert_eq!(house.fix_security_source(), None);
    assert_eq!(house.max_value_width(), 64);
    assert_eq!(house, "housekey");
    assert_eq!(house.to_string(), "housekey");
    assert_eq!(
        kind("House.Key").as_str(),
        "house.key",
        "a dot is part of a word"
    );
    assert_eq!(
        kind("a"),
        kind("A"),
        "a word folds: a wire code is read by `from_security_source`, not by a parse"
    );
    assert!(!kind("a").is_known());
    assert_eq!(kind("Z").as_str(), "z");
    assert_eq!(
        kind("4").as_str(),
        "4",
        "a code is a word where nothing asks for a code"
    );
    // The words a spelling is made of are the longest a code set names and
    // no more.
    assert_eq!(kind(&"k".repeat(64)).as_str(), "k".repeat(64));
    assert_eq!(kind(&"K".repeat(64)).as_str(), "k".repeat(64));

    assert_eq!(IdType::Isin.to_string(), "isin");
    assert_eq!(AsRef::<str>::as_ref(&IdType::Isin), "isin");
    assert_eq!(IdType::MdEntryRefId.as_str(), "mdentryrefid");
    assert_eq!(smol_str::SmolStr::from(IdType::ClOrdId), "clordid");
    assert_eq!(smol_str::SmolStr::from(house), "housekey");
}

#[test]
fn a_spelling_that_is_no_word_is_refused_and_names_what_was_expected() {
    for refused in [
        "",
        "   ",
        "_-#",
        "caf\u{e9}",
        "tab\tkey",
        "a/b",
        "a:b",
        "a=b",
        "x,y",
    ] {
        assert!(refused.parse::<IdType>().is_err(), "{refused:?}");
    }
    let refused = "".parse::<IdType>().unwrap_err().to_string();
    assert!(
        refused.contains("an identifier type of a word"),
        "{refused}"
    );
    let refused = "caf\u{e9}".parse::<IdType>().unwrap_err().to_string();
    assert!(
        refused.contains("ASCII letters, digits and '.'") && refused.contains("caf"),
        "{refused}"
    );
    let wide = "k".repeat(65);
    let refused = wide.parse::<IdType>().unwrap_err().to_string();
    assert!(refused.contains("at most 64 bytes"), "{refused}");
}

#[test]
fn types_compare_with_text_and_order_by_their_spelling() {
    assert_eq!(IdType::Isin, "isin");
    assert_eq!(IdType::Isin, *"isin");
    assert!(IdType::Isin != "ISIN", "a spelling is compared as folded");
    assert!(IdType::Isin != "cusip");
    assert!(kind("cusip") < kind("isin"));
    assert!(kind("cusip") < kind("house") && kind("house") < kind("isin"));
    let mut sorted = [
        kind("sedol"),
        kind("zzz"),
        kind("isin"),
        kind("aaa"),
        kind("cusip"),
    ];
    sorted.sort();
    assert_eq!(
        sorted.iter().map(IdType::as_str).collect::<Vec<_>>(),
        ["aaa", "cusip", "isin", "sedol", "zzz"]
    );
    // The same word, in whichever spelling it was read, is one value.
    assert_eq!(kind("ISIN_Number"), kind("isin"));
    let mut hashed = std::collections::HashSet::new();
    hashed.insert(kind("ClOrdID"));
    assert!(hashed.contains(&IdType::ClOrdId));
    assert!(hashed.contains(&kind("cl-ord-id")));
}

#[test]
fn the_known_words_are_unique_folded_and_read_back_as_themselves() {
    let mut seen = std::collections::HashSet::new();
    for known in &IdType::KNOWN {
        assert!(known.is_known(), "{known}");
        let spelling = known.as_str();
        assert_eq!(
            spelling,
            spelling.to_ascii_lowercase(),
            "{spelling} is folded"
        );
        assert!(
            spelling.bytes().all(|byte| byte.is_ascii_alphanumeric()),
            "{spelling}"
        );
        assert_eq!(&kind(spelling), known, "{spelling} reads as itself");
        assert_eq!(
            &kind(&spelling.to_ascii_uppercase()),
            known,
            "{spelling} reads upper case"
        );
        assert!(seen.insert(spelling), "{spelling} is named once");
    }
    assert!(IdType::KNOWN.contains(&IdType::Account));
    assert!(IdType::KNOWN.contains(&IdType::Party));
    assert!(IdType::KNOWN.contains(&IdType::InstrumentId));
    assert!(IdType::KNOWN.contains(&IdType::MdEntryId));
    assert!(IdType::KNOWN.contains(&IdType::MdEntryRefId));
    assert!(IdType::KNOWN.contains(&IdType::ExecutingTrader));
}

#[test]
fn the_fix_source_table_agrees_with_the_code_set_exactly() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../config/fix/codesets/securityidsourcecodeset.json");
    let document = yggdryl::from_json_scalar(std::fs::read(path).unwrap()).unwrap();
    assert_eq!(
        document.get_key_str("name").and_then(Scalar::as_str),
        Some("securityidsourcecodeset")
    );
    let codes = document.get_key_str("codes").unwrap();
    let fixed = fix_sources();
    assert_eq!(codes.len(), fixed.len());
    assert_eq!(fixed.len(), 33, "what FIX names stays FIX's thirty-three");
    let mut kinds = Vec::new();
    for (index, held) in fixed.iter().enumerate() {
        let code = codes.get(index).unwrap();
        let value = code.get_key_str("value").and_then(Scalar::as_str).unwrap();
        let name = code.get_key_str("name").and_then(Scalar::as_str).unwrap();
        let doc = code.get_key_str("doc").and_then(Scalar::as_str).unwrap();
        let mut chars = value.chars();
        let (Some(code), None) = (chars.next(), chars.next()) else {
            panic!("a one-character code, got {value:?}");
        };
        let known = IdType::from_fix_security_source(code)
            .unwrap_or_else(|| panic!("code {code} names a known type"));
        assert_eq!(known.fix_security_source(), Some(code), "one code per type");
        assert_eq!(*held, (known.clone(), code), "the code set's order");
        assert_eq!(kind(name), known, "{name} reads to {known}");
        assert_eq!(
            IdType::from_security_source(name).unwrap(),
            known,
            "{name} reads as a source"
        );
        assert_eq!(
            IdType::from_security_source(value).unwrap(),
            known,
            "{value} reads as a source"
        );
        assert_eq!(
            IdType::from_security_source(doc).unwrap(),
            known,
            "{doc:?}, the code set's prose name, reads as a source"
        );
        kinds.push(known);
    }
    let mut unique = kinds.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), kinds.len(), "one type per code");
}

#[test]
fn a_security_source_reads_a_code_or_any_spelling_and_refuses_a_ticker() {
    assert_eq!(IdType::from_security_source("4").unwrap(), IdType::Isin);
    assert_eq!(IdType::from_security_source(" 4 ").unwrap(), IdType::Isin);
    // A vendor's own source spelling reads as the type it names.
    assert_eq!(
        IdType::from_security_source("X-SWX-VALOR").unwrap(),
        IdType::Valor
    );
    assert_eq!(
        IdType::from_security_source("SIX Symbol").unwrap(),
        IdType::ExchSymb
    );
    assert_eq!(
        IdType::from_security_source("A").unwrap(),
        IdType::Bloomberg
    );
    assert_eq!(IdType::from_security_source("S").unwrap(), IdType::Figi);
    assert_eq!(IdType::from_security_source("isin").unwrap(), IdType::Isin);
    assert_eq!(
        IdType::from_security_source("ISINNumber").unwrap(),
        IdType::Isin
    );
    assert_eq!(
        IdType::from_security_source("RIC Code").unwrap(),
        IdType::Ric
    );
    assert_eq!(
        IdType::from_security_source("forex").unwrap(),
        IdType::Forex
    );
    // A wire code does not fold: a lower-case letter is a word, and a code
    // no standard names is one too.
    assert_eq!(IdType::from_security_source("a").unwrap().as_str(), "a");
    assert!(!IdType::from_security_source("a").unwrap().is_known());
    assert_eq!(IdType::from_security_source("Z").unwrap().as_str(), "z");
    assert_eq!(
        IdType::from_security_source("house-key").unwrap().as_str(),
        "housekey"
    );

    for (known, code) in fix_sources() {
        assert_eq!(IdType::from_fix_security_source(code), Some(known.clone()));
        assert_eq!(
            IdType::from_security_source(&code.to_string()).unwrap(),
            known
        );
        assert_eq!(IdType::from_security_source(known.as_str()).unwrap(), known);
    }
    assert_eq!(IdType::from_fix_security_source('Z'), None);
    assert_eq!(IdType::from_fix_security_source('a'), None);
    assert_eq!(IdType::from_fix_security_source('0'), None);
    assert_eq!(
        IdType::Forex.fix_security_source(),
        None,
        "FIX gives a pair no code"
    );
    assert_eq!(IdType::Cfi.fix_security_source(), None);
    assert_eq!(IdType::ClOrdId.fix_security_source(), None);
    assert_eq!(IdType::Isin.fix_security_source(), Some('4'));
    assert_eq!(IdType::Ric.fix_security_source(), Some('5'));
    assert_eq!(IdType::Dti.fix_security_source(), Some('Y'));

    for ticker in ["ticker", "TICKER", " Ticker "] {
        let (path, reason) = located(IdType::from_security_source(ticker));
        assert_eq!(path, "ticker", "{ticker:?}");
        assert!(reason.contains("set_ticker"), "{reason}");
    }
    let (path, reason) = located(kind("ticker").check_security());
    assert_eq!(path, "ticker");
    assert!(reason.contains("set_ticker"), "{reason}");
    assert!(IdType::Isin.check_security().is_ok());
    assert!(
        kind("symbol").check_security().is_ok(),
        "only a ticker is refused"
    );
    assert!(IdType::from_security_source("").is_err());
    assert!(IdType::from_security_source("caf\u{e9}").is_err());
    assert!(IdType::from_security_source(&"k".repeat(65)).is_err());
}

#[test]
fn a_field_name_names_one_instruments_own_identifier_type() {
    let named = |name: &str| IdType::from_field_name(name).map(|known| known.as_str().to_owned());
    assert_eq!(named("isincode").as_deref(), Some("isin"));
    assert_eq!(named("#isincode").as_deref(), Some("isin"));
    assert_eq!(named("ISIN_Code").as_deref(), Some("isin"));
    assert_eq!(named("isin").as_deref(), Some("isin"));
    assert_eq!(named("ISINNumber").as_deref(), Some("isin"));
    assert_eq!(named("isin_number").as_deref(), Some("isin"));
    assert_eq!(named("security isin").as_deref(), Some("isin"));
    assert_eq!(named("SecurityISINCode").as_deref(), Some("isin"));
    assert_eq!(named("cusipcode").as_deref(), Some("cusip"));
    assert_eq!(named("sedol_code").as_deref(), Some("sedol"));
    assert_eq!(named("bloombergcode").as_deref(), Some("bloomberg"));
    assert_eq!(named("bloomberg_symbol").as_deref(), Some("bloomberg"));
    assert_eq!(named("bbgsymb").as_deref(), Some("bloomberg"));
    assert_eq!(named("figicode").as_deref(), Some("figi"));
    assert_eq!(named("figi_id").as_deref(), Some("figi"));
    assert_eq!(named("ric").as_deref(), Some("ric"));
    assert_eq!(named("RICCode").as_deref(), Some("ric"));
    assert_eq!(named("wkn").as_deref(), Some("wkn"));
    assert_eq!(named("wertpapier").as_deref(), Some("wkn"));
    assert_eq!(named("valor").as_deref(), Some("valor"));
    assert_eq!(named("X-SWX-VALOR").as_deref(), Some("valor"));
    assert_eq!(named("Valorennummer").as_deref(), Some("valor"));
    assert_eq!(named("SIX_SYMBOL").as_deref(), Some("exchsymb"));
    assert_eq!(named("Valorensymbol").as_deref(), Some("exchsymb"));
    assert_eq!(named("lei").as_deref(), Some("lei"));
    assert_eq!(named("LegalEntityIdentifier").as_deref(), Some("lei"));
    assert_eq!(named("exchange_symbol").as_deref(), Some("exchsymb"));
    assert_eq!(named("cusip_number").as_deref(), Some("cusip"));
    assert_eq!(
        IdType::from_field_name("#ISINCODE"),
        Some(IdType::Isin),
        "a leading # is dropped"
    );

    for refused in [
        "ticker",
        "symbol",
        "symbolticker",
        "Symbol_Ticker",
        "#symbol",
        "legsecurityid",
        "leg_isin",
        "legisin",
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
        "clordid",
        "account",
        "instrumentcode",
    ] {
        assert_eq!(named(refused), None, "{refused:?} names no type");
    }
    // The bare spellings a bridge writes a code under name their type whole.
    for (name, expected) in [
        ("bbg", "bloomberg"),
        ("BBGCode", "bloomberg"),
        ("bbgsymbol", "bloomberg"),
        ("BloombergTicker", "bloomberg"),
        ("OpenFIGI", "figi"),
        ("ReutersCode", "ric"),
        ("reuters", "ric"),
        ("ExchSymbol", "exchsymb"),
        ("isinid", "isin"),
        ("cusipid", "cusip"),
        ("sedol_number", "sedol"),
        ("valor_id", "valor"),
        ("wkncode", "wkn"),
    ] {
        assert_eq!(named(name).as_deref(), Some(expected), "{name}");
    }
}

/// A code-suffixed spelling of a security type - `riccode`, `bbgsymbol`,
/// `cusipnumber` - names the type at the end of a key the way an `id`
/// spelling does, so a bridge's `OMS_RICCODE` reads as `oms:ric` and its
/// `ULLINK.ISINCODE` as `ullink:isin`; bare `ric` and `cfi` stay out of the
/// end-matching, so `GENERIC` and `OMS_RIC` name nothing, a ticker spelling
/// names no type, and another instrument's word still refuses the type.
#[test]
fn a_code_suffixed_security_spelling_reads_at_the_end_of_a_key() {
    let read = |key: &str, value: &str| Identifier::from_key(key, value).map(|id| id.to_string());
    for (key, value, expected) in [
        ("OMS_RICCODE", "AAPL.O", "oms:ric=AAPL.O"),
        (
            "ULLINK.ISINCODE",
            "US0378331005",
            "ullink:isin=US0378331005",
        ),
        ("OMS_CUSIPCODE", "037833100", "oms:cusip=037833100"),
        ("OMS_SEDOLNUMBER", "2046251", "oms:sedol=2046251"),
        (
            "firm.x.FIGICode",
            "BBG000B9XRY4",
            "firm.x:figi=BBG000B9XRY4",
        ),
        (
            "OMS_BBGSYMBOL",
            "AAPL US Equity",
            "oms:bloomberg=AAPL US Equity",
        ),
        (
            "OMS_BloombergTicker",
            "AAPL US Equity",
            "oms:bloomberg=AAPL US Equity",
        ),
        ("OMS_ReutersCode", "AAPL.O", "oms:ric=AAPL.O"),
        ("OMS_ExchSymbol", "AAPL", "oms:exchsymb=AAPL"),
        ("OMS_InstrumentCode", "dbi;X", "oms:instrumentid=dbi;X"),
        ("OMS_WKNCode", "865985", "oms:wkn=865985"),
        ("OMS_ValorNumber", "1221405", "oms:valor=1221405"),
        ("OMS_ValorenNumber", "1221405", "oms:valor=1221405"),
        ("OMS_SIXSymbol", "HOLN", "oms:exchsymb=HOLN"),
        ("OMS_ISINID", "US0378331005", "oms:isin=US0378331005"),
        ("BBGCODE", "AAPL US Equity", "bloomberg=AAPL US Equity"),
        ("BBG", "AAPL US Equity", "bloomberg=AAPL US Equity"),
        ("OpenFIGI", "BBG000B9XRY4", "figi=BBG000B9XRY4"),
        ("Reuters", "AAPL.O", "ric=AAPL.O"),
        ("ExchSymbol", "AAPL", "exchsymb=AAPL"),
        ("InstrumentCode", "dbi;X", "instrumentid=dbi;X"),
    ] {
        assert_eq!(read(key, value).as_deref(), Some(expected), "{key}");
    }
    for key in [
        "TICKER",
        "SYMBOL",
        "TICKERCODE",
        "CLIENT.SYMBOL",
        "GENERIC",
        "OMS_RIC",
        "OMS_CFI",
        "OMS_UnderlyingISINCode",
        "FIX.LegRICCode",
    ] {
        assert_eq!(read(key, "AAPL.O"), None, "{key}");
    }
}

#[test]
fn a_currency_pair_is_the_crates_own_type_and_its_value_lands_as_the_canonical_pair() {
    // FIX gives a currency pair no source code, so the type is the crate's:
    // one spelling stored, several read, and a value stored canonical.
    for spelling in [
        "FOREX",
        "forex",
        " Forex ",
        "forexcode",
        "ccypair",
        "CcyPair",
        "currency_pair",
    ] {
        assert_eq!(kind(spelling), IdType::Forex, "{spelling:?}");
    }
    assert_eq!(IdType::Forex.as_str(), "forex");
    for name in [
        "#FOREXCODE",
        "ForexCode",
        "#forex",
        "CurrencyPair",
        "SecurityForexID",
    ] {
        assert_eq!(
            IdType::from_field_name(name),
            Some(IdType::Forex),
            "{name:?}"
        );
    }
    // A currency is not a pair, and an ISO 4217 code is a type of its own.
    assert_eq!(IdType::from_field_name("Currency"), None);
    assert_eq!(kind("ISOCurrencyCode"), IdType::IsoCcy);
    assert_eq!(IdType::IsoCcy.as_str(), "isoccy");
    assert_eq!(IdType::Forex.max_value_width(), 7);

    // Every accepted spelling lands as the one stored pair.
    for spelling in [
        "EUR/USD",
        "eurusd",
        "EUR-USD",
        "eur.usd",
        "EUR_USD",
        " EUR/USD ",
    ] {
        let pair = id("forex", spelling).unwrap();
        assert_eq!(pair.value(), "EUR/USD", "{spelling}");
    }
    let pair = id("forex", "eurusd").unwrap();
    assert_eq!(pair.kind(), "forex");
    assert_eq!(pair.to_string(), "forex=EUR/USD");
    assert_eq!(pair, id("ccypair", " EUR/USD ").unwrap());

    // A symbol, a pair of one currency, a stranger and a null-like value are
    // refused by the pair's own rule or the null one.
    for value in ["EUR/EUR", "EUR/USD 1M", "ABC/USD", "EUR USD"] {
        let refused = id("forex", value).unwrap_err();
        assert!(
            matches!(refused, Error::InvalidDataType { kind: "forex", .. }),
            "{value}: {refused}"
        );
    }
    let refused = id("forex", "n/a").unwrap_err().to_string();
    assert!(
        refused.contains("states nothing") || refused.contains("text stating something"),
        "{refused}"
    );
}

#[test]
fn each_type_holds_its_value_to_its_own_rule() {
    let width = |text: &str| kind(text).max_value_width();
    assert_eq!(width("isin"), 12);
    assert_eq!(width("cusip"), 9);
    assert_eq!(width("sedol"), 7);
    assert_eq!(width("figi"), 12);
    assert_eq!(width("wkn"), 6);
    assert_eq!(width("cfi"), 6);
    assert_eq!(width("valor"), 9);
    assert_eq!(width("isoccy"), 3);
    assert_eq!(width("isoctry"), 2);
    assert_eq!(width("forex"), 7);
    assert_eq!(width("bloomberg"), 32);
    assert_eq!(width("ric"), 32);
    assert_eq!(width("lei"), 20);
    assert_eq!(width("dti"), 9);
    for unchecked in ["fpmlurl", "fpmlspec", "index", "isdacommodity", "100"] {
        assert_eq!(
            width(unchecked),
            64,
            "{unchecked}: a type no code bounds takes the identifier width"
        );
    }
    assert_eq!(width("house"), 64);
    assert_eq!(width("clordid"), 64);
    assert_eq!(width("executingtrader"), 64);

    let accepts = |text: &str, value: &str| id(text, value).is_ok();
    assert!(accepts("isin", "US0378331005"));
    assert!(accepts("isin", "us0378331005"));
    // The check digit is a rank, not a refusal: a number that does not
    // close is held, and ranks below one that does.
    assert!(accepts("isin", "US0378331006"));
    assert!(!accepts("isin", "US037833100"), "the shape");
    assert_eq!(IdType::Isin.rank("US0378331005"), 2);
    assert_eq!(IdType::Isin.rank("US0378331006"), 1);
    assert_eq!(IdType::Isin.rank("XX0000000001"), 0);
    assert_eq!(IdType::Isin.max_rank(), 2);
    assert!(IdType::Isin.is_real("US0378331005"));
    assert!(!IdType::Isin.is_real("US0378331006"));
    // A rank reads the text as its type does, folded: a lower-case spelling
    // the type holds ranks as its upper-case one, whichever the code.
    for (kind, lower, rank) in [
        (IdType::Isin, "us0378331005", 2),
        (IdType::Cusip, "38259p508", 1),
        (IdType::Sedol, "b0ybkj7", 1),
        (IdType::Figi, "bbg000blnq16", 1),
    ] {
        assert_eq!(kind.rank(lower), rank, "{lower}");
        assert_eq!(
            kind.rank(lower),
            kind.rank(&lower.to_ascii_uppercase()),
            "{lower}"
        );
    }
    assert!(IdType::Isin.is_real("us0378331005"));
    assert!(accepts("cusip", "037833100"));
    assert!(accepts("cusip", "037833101"));
    assert_eq!(IdType::Cusip.rank("037833100"), 1);
    assert_eq!(IdType::Cusip.rank("037833101"), 0);
    assert!(accepts("sedol", "0263494"));
    assert!(accepts("sedol", "0263495"));
    assert_eq!(IdType::Sedol.rank("0263495"), 0);
    assert!(accepts("figi", "BBG000B9XRY4"));
    assert!(accepts("figi", "BBG000B9XRY5"));
    assert_eq!(IdType::Figi.rank("BBG000B9XRY5"), 0);
    assert!(!accepts("figi", "BSG000B9XRY4"), "a reserved prefix");
    // An LEI and a DTI are held by their own code's shape and ranked by
    // their own check, folded first.
    assert!(accepts("lei", "HWUPKR0MPOU8FGXBT394"));
    assert!(accepts("lei", "HWUPKR0MPOU8FGXBT395"));
    assert!(!accepts("lei", "x"), "the shape");
    assert_eq!(IdType::Lei.rank("hwupkr0mpou8fgxbt394"), 1);
    assert_eq!(IdType::Lei.rank("HWUPKR0MPOU8FGXBT395"), 0);
    assert_eq!(IdType::Lei.max_rank(), 1);
    assert_eq!(
        id("lei", "hwupkr0mpou8fgxbt394").unwrap().value(),
        "HWUPKR0MPOU8FGXBT394"
    );
    assert!(accepts("dti", "X9J9K872S"));
    assert!(!accepts("dti", "A9J9K872S"), "a vowel");
    assert_eq!(IdType::Dti.rank("x9j9k872s"), 1);
    assert_eq!(IdType::Dti.rank("X9J9K872T"), 0);
    // The listed country and the detailed classification rank; every
    // other type has nothing partial about it.
    assert_eq!(IdType::IsoCtry.rank("CH"), 1);
    assert_eq!(IdType::IsoCtry.rank("XX"), 0);
    assert_eq!(IdType::IsoCcy.rank("XXX"), 0);
    assert_eq!(IdType::IsoCcy.rank("USDT"), 1);
    assert_eq!(IdType::Cfi.rank("ESVUFR"), 2);
    assert_eq!(IdType::Cfi.rank("ESXXXX"), 1);
    assert_eq!(IdType::Cfi.rank("XXXXXX"), 0);
    assert_eq!(IdType::Cfi.max_rank(), 2);
    assert_eq!(IdType::OrderId.rank("O-1"), 1);
    assert_eq!(IdType::OrderId.max_rank(), 1);
    assert!(IdType::Ric.is_real("AAPL.O"));
    assert!(accepts("wkn", "716460"));
    assert!(accepts("wkn", "BASF11"));
    assert!(accepts("wkn", "basf11"));
    assert!(!accepts("wkn", "BASI11"), "no I");
    assert!(!accepts("wkn", "BASO11"), "no O");
    assert!(!accepts("wkn", "71646"));
    assert!(!accepts("wkn", "7164600"));
    assert!(accepts("valor", "3886335"));
    assert!(accepts("valor", "1"));
    assert!(accepts("valor", "123456789"));
    assert!(!accepts("valor", "0"));
    assert!(!accepts("valor", "03886335"), "no leading zero");
    assert!(!accepts("valor", "1234567890"));
    assert!(!accepts("valor", "38A6335"));
    assert!(accepts("bloomberg", "AAPL US Equity"));
    assert!(accepts("bloomberg", &"B".repeat(32)));
    assert!(!accepts("bloomberg", &"B".repeat(33)));
    assert!(!accepts("bloomberg", "AAPL\u{a0}US"));
    for null in ["", "null", "NULL", "none", "n/a", "[N/A]"] {
        assert!(!accepts("bloomberg", null), "{null:?}");
        assert!(!accepts("house", null), "{null:?}");
        assert!(!accepts("isin", null), "{null:?}");
        assert!(!accepts("clordid", null), "{null:?}");
    }
    assert!(accepts("isoccy", "USD"));
    assert!(!accepts("isoccy", "USDX"));
    assert!(accepts("isoctry", "US"));
    assert!(!accepts("isoctry", "USA"));
    assert!(accepts("ric", "AAPL.OQ"));
    assert!(accepts("cfi", "ESVUFR"));
    assert!(accepts("cfi", "esvufr"));
    assert!(!accepts("cfi", "ESVUFRX"), "past the six it is wide");
    assert!(accepts("house", &"h".repeat(64)));
    assert!(!accepts("house", &"h".repeat(65)));
    assert!(accepts("clordid", "C-1"));
    assert!(accepts("clordid", &"c".repeat(64)));
    assert!(!accepts("clordid", &"c".repeat(65)));

    // Case folds only where a type is a code: the others keep it.
    assert_eq!(id("wkn", "basf11").unwrap().value(), "BASF11");
    assert_eq!(id("figi", "bbg000b9xry4").unwrap().value(), "BBG000B9XRY4");
    assert_eq!(id("sedol", "b4bnmy3").unwrap().value(), "B4BNMY3");
    assert_eq!(
        id("isin", " us0378331005 ").unwrap().value(),
        "US0378331005"
    );
    assert_eq!(id("clordid", "c-1").unwrap().value(), "c-1");
    assert_eq!(id("house", "Mixed Case").unwrap().value(), "Mixed Case");
    assert_eq!(
        id("bloomberg", "aapl us Equity").unwrap().value(),
        "aapl us Equity",
        "case and inner spaces kept"
    );

    let refused = id("wkn", "BASI11").unwrap_err();
    let Error::InvalidRecord { path, reason } = refused else {
        panic!("a located refusal");
    };
    assert_eq!(path, "wkn");
    assert_eq!(
        reason,
        "expected a wkn value, got \"BASI11\", not six of [0-9A-HJ-NP-Z]"
    );
    assert!(matches!(
        id("isin", "US037833100"),
        Err(Error::InvalidDataType { kind: "isin", .. })
    ));
    let (path, reason) = located(id("bloomberg", &"B".repeat(33)));
    assert_eq!(path, "bloomberg");
    assert!(
        reason.contains("33 bytes, over the 32 the type allows"),
        "{reason}"
    );
}

/// `IdType::rank` and `is_real` read the text as the type stores it: every
/// type that folds case ranks a lower-case spelling as the upper-case one,
/// whether or not the code's own `new` folds (a CFI, an ISO currency and an
/// ISO country do not).
#[test]
fn a_rank_reads_a_lower_case_spelling_as_the_upper_case_one_it_folds_to() {
    for (kind, upper, rank) in [
        (IdType::Isin, "US0378331005", 2),
        (IdType::Isin, "US0378331006", 1),
        (IdType::Isin, "XX0000000001", 0),
        (IdType::Cusip, "38259P508", 1),
        (IdType::Cusip, "38259P509", 0),
        (IdType::Sedol, "B0YBKJ7", 1),
        (IdType::Sedol, "B0YBKJ8", 0),
        (IdType::Figi, "BBG000BLNQ16", 1),
        (IdType::Figi, "BBG000BLNQ15", 0),
        (IdType::Cfi, "ESVUFR", 2),
        (IdType::Cfi, "ESXXXX", 1),
        (IdType::Cfi, "XXXXXX", 0),
        (IdType::IsoCcy, "USDT", 1),
        (IdType::IsoCcy, "XXX", 0),
        (IdType::IsoCtry, "CH", 1),
        (IdType::IsoCtry, "XX", 0),
    ] {
        let lower = upper.to_ascii_lowercase();
        assert_eq!(kind.rank(upper), rank, "{kind} {upper}");
        assert_eq!(kind.rank(&lower), rank, "{kind} {lower}");
        assert_eq!(kind.is_real(&lower), kind.is_real(upper), "{kind} {lower}");
    }
    // A type that does not fold case keeps its text as it is.
    assert_eq!(IdType::Ric.rank("aapl.o"), 1);
}

#[test]
fn a_ric_holds_its_value_to_the_ric_rule() {
    // FIX's SecurityIDSource 5 is a Refinitiv Identification Code, and its
    // value is validated as one: a token of printable ASCII, its case kept.
    assert_eq!(IdType::Ric.fix_security_source(), Some('5'));
    for value in ["AAPL.OQ", "VOD.L", ".SPX", "0#.FTSE", "EUR=", "ESc1"] {
        assert!(id("ric", value).is_ok(), "{value}");
    }
    assert_eq!(
        id("ric", "ESc1").unwrap().value(),
        "ESc1",
        "a RIC does not fold"
    );
    assert_eq!(
        Identifier::new(
            IdKey::base(IdType::from_security_source("5").unwrap()),
            "AAPL.OQ"
        )
        .unwrap()
        .to_string(),
        "ric=AAPL.OQ"
    );

    // An inner space splits the token, which another type would hold.
    let refused = id("ric", "AAPL OQ").unwrap_err();
    assert!(
        matches!(refused, Error::InvalidDataType { kind: "ric", .. }),
        "{refused}"
    );
    assert!(refused.to_string().contains("got 0x20 at 4"), "{refused}");
    assert!(id("house", "AAPL OQ").is_ok());
    assert!(id("ric", "IBM\t.N").is_err());
    assert!(id("ric", &"R".repeat(33)).is_err());
    assert!(id("ric", "n/a").is_err());
}

#[test]
fn a_security_identifier_reads_its_type_and_holds_its_value_canonically() {
    let apple = id("isin", " us0378331005 ").unwrap();
    assert_eq!(apple.to_string(), "isin=US0378331005");
    assert_eq!(apple.kind(), "isin");
    assert_eq!(apple.src(), "base");
    assert_eq!(apple.value(), "US0378331005");
    assert_eq!(
        apple,
        Identifier::new(
            IdKey::base(IdType::from_security_source("4").unwrap()),
            "US0378331005"
        )
        .unwrap()
    );
    assert_eq!(apple, id("ISINNumber", "US0378331005").unwrap());
    assert_eq!(
        Identifier::new(
            IdKey::new("ullink".parse().unwrap(), IdType::Isin),
            "US0378331005"
        )
        .unwrap()
        .src(),
        "ullink",
        "the source is whoever stated it"
    );
    assert_eq!(
        id("cusip", "037833100").unwrap().to_string(),
        "cusip=037833100"
    );
    assert_eq!(id("sedol", "b4bnmy3").unwrap().to_string(), "sedol=B4BNMY3");
    assert_eq!(id("valor", "3886335").unwrap().to_string(), "valor=3886335");
    assert_eq!(
        id("A", "aapl us Equity").unwrap().to_string(),
        "a=aapl us Equity",
        "a bare letter is a word to a parse: a code is read by from_security_source"
    );
    assert_eq!(
        Identifier::new(
            IdKey::base(IdType::from_security_source("A").unwrap()),
            "aapl us Equity"
        )
        .unwrap()
        .to_string(),
        "bloomberg=aapl us Equity"
    );

    // A type no member names is kept, folded as every type is.
    let house = id("house-key", "hk-1").unwrap();
    assert_eq!(house.to_string(), "housekey=hk-1");
    assert_eq!(house.value(), "hk-1");
    assert_eq!(
        id(&"K".repeat(64), "c").unwrap().kind().as_str(),
        "k".repeat(64)
    );
}

#[test]
fn a_bridge_and_a_parentage_spelling_are_words_no_member_names() {
    // The bridge types and the parent spellings were members once: a word
    // like any other now, read for what it is by `parent_of`.
    for spelling in [
        "ParentOrderID",
        "OMSDealerParentOrderID",
        "UlTraderClOrdID",
        "ExchangeClientOrderID",
        "TransversalKey",
        "origorderid",
        "origtradeid",
        "originclordid",
    ] {
        let read = kind(spelling);
        assert!(!read.is_known(), "{spelling:?}");
        assert!(matches!(read, IdType::Other(_)), "{spelling:?}");
        assert!(!read.is_security() && !read.is_party(), "{spelling:?}");
        assert_eq!(read.max_value_width(), 64, "{spelling:?}");
    }
    assert!(IdType::OrigClOrdId.is_known(), "the one parent FIX names");
    assert_eq!(
        kind("ParentClOrdID"),
        IdType::OrigClOrdId,
        "its other spelling"
    );
}

#[test]
fn a_base_type_lists_its_parents_nearest_first() {
    let parents = |base: &IdType| {
        base.parents()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    };
    // `clordid`: FIX's `OrigClOrdID(41)`, the client order identifier a
    // cancel/replace replaced, alone.
    assert_eq!(IdType::ClOrdId.parents().as_ref(), [IdType::OrigClOrdId]);
    assert_eq!(parents(&IdType::ClOrdId), ["origclordid"]);
    // Any other type the crate names, or a word ending in `id`: the value
    // before the last change, then the chain's first.
    assert_eq!(parents(&IdType::OrderId), ["parentorderid", "origorderid"]);
    assert_eq!(parents(&IdType::TradeId), ["parenttradeid", "origtradeid"]);
    assert_eq!(parents(&IdType::ExecId), ["parentexecid", "origexecid"]);
    assert_eq!(parents(&IdType::Isin), ["parentisin", "origisin"]);
    assert_eq!(parents(&IdType::Account), ["parentaccount", "origaccount"]);
    assert_eq!(parents(&kind("firmid")), ["parentfirmid", "origfirmid"]);
    assert_eq!(parents(&kind("Firm_ID")), ["parentfirmid", "origfirmid"]);
    // A word that names no identifier has none.
    for word in ["housekey", "house", "market", "z", "parenthood"] {
        assert!(kind(word).parents().is_empty(), "{word}");
    }
    // Parentage never nests: a parent type has no parents of its own.
    assert!(IdType::OrigClOrdId.parents().is_empty());
    for parent in [
        "origclordid",
        "parentorderid",
        "origorderid",
        "origtradeid",
        "parentisin",
        "parentfirmid",
    ] {
        assert!(kind(parent).parents().is_empty(), "{parent}");
    }
    // Every type the crate names has the two of its rule, but `clordid`'s one
    // and the parent FIX names, which has none.
    for known in &IdType::KNOWN {
        let expected = match known {
            IdType::ClOrdId => 1,
            IdType::OrigClOrdId => 0,
            _ => 2,
        };
        assert_eq!(known.parents().len(), expected, "{known}");
    }
    // A type the crate names answers borrowed from one table, so asking per
    // identifier allocates nothing; any other word builds its list.
    for known in &IdType::KNOWN {
        assert!(matches!(known.parents(), Cow::Borrowed(_)), "{known}");
    }
    assert!(matches!(kind("firmid").parents(), Cow::Owned(_)));
    let (first, second) = (IdType::OrderId.parents(), IdType::OrderId.parents());
    assert!(std::ptr::eq(first.as_ptr(), second.as_ptr()), "one table");
    // The parents are types, so each holds the width a type may be: a base
    // that cannot spell `parent` and itself in 64 bytes loses that parent.
    let widest = kind(&format!("{}id", "k".repeat(56)));
    assert_eq!(widest.as_str().len(), 58);
    assert_eq!(
        widest
            .parents()
            .iter()
            .map(|parent| parent.as_str().len())
            .collect::<Vec<_>>(),
        [64, 62]
    );
    let wider = kind(&format!("{}id", "k".repeat(62)));
    assert_eq!(wider.as_str().len(), 64);
    assert!(wider.parents().is_empty(), "neither parent fits");
}

#[test]
fn a_parent_type_names_its_base_and_its_place_among_the_bases_parents() {
    for (spelling, base, place) in [
        ("origclordid", IdType::ClOrdId, 0),
        ("OrigClOrdID", IdType::ClOrdId, 0),
        ("parentorderid", IdType::OrderId, 0),
        ("ParentOrderID", IdType::OrderId, 0),
        ("origorderid", IdType::OrderId, 1),
        ("OrigOrderID", IdType::OrderId, 1),
        ("origtradeid", IdType::TradeId, 1),
        ("parentexecid", IdType::ExecId, 0),
        ("origexecid", IdType::ExecId, 1),
        ("parentisin", IdType::Isin, 0),
        ("origisin", IdType::Isin, 1),
        ("parentfirmid", kind("firmid"), 0),
        ("origfirmid", kind("firmid"), 1),
    ] {
        assert_eq!(
            kind(spelling).parent_of(),
            Some((base, place)),
            "{spelling}"
        );
    }
    // FIX gives a client order identifier the one parent, which
    // `parentclordid` spells too. `origin` and `original` are words of their
    // own, a base is no parent, and nothing but a type that has parents has
    // one.
    assert_eq!(
        kind("parentclordid").parent_of(),
        Some((IdType::ClOrdId, 0))
    );
    for spelling in [
        "originalorderid",
        "originorderid",
        "origintradeid",
        "originclordid",
        "originalcode",
        "originatingmarket",
        "origin",
        "orig",
        "parent",
        "original",
        "parenthood",
        "parentmarket",
        "clordid",
        "orderid",
        "isin",
        "executingtrader",
        "housekey",
    ] {
        assert_eq!(kind(spelling).parent_of(), None, "{spelling}");
    }
    // Parentage never nests: the parent of a parent is none.
    for spelling in [
        "parentparentorderid",
        "origparentorderid",
        "parentorigorderid",
        "origorigclordid",
        "parentorigclordid",
    ] {
        assert_eq!(kind(spelling).parent_of(), None, "{spelling}");
    }

    // The two readings agree. Every parent a base lists reads back to that
    // base at the place it is listed - for every type the crate names and for
    // words of its own - and a parent type is found at that place in its
    // base's list.
    let mut bases: Vec<IdType> = IdType::KNOWN.to_vec();
    bases.extend(["firmid", "deskids", "x.id", "tradeid2id", "invalidid"].map(kind));
    for base in &bases {
        for (place, parent) in base.parents().iter().enumerate() {
            assert_eq!(
                parent.parent_of(),
                Some((base.clone(), place)),
                "{parent} of {base}"
            );
        }
    }
    for spelling in ["origclordid", "parentorderid", "origorderid", "origtradeid"] {
        let parent = kind(spelling);
        let (base, place) = parent.parent_of().expect("a parent type");
        assert_eq!(base.parents()[place], parent, "{spelling}");
    }
    // A type the crate names is a base with parents or a parent with none,
    // never both: `origclordid` is the one parent among them.
    for known in &IdType::KNOWN {
        if known.parents().is_empty() {
            assert!(
                known.parent_of().is_some(),
                "{known}: no parents, so a parent"
            );
        } else {
            assert_eq!(known.parent_of(), None, "{known}: a base is no parent");
        }
    }
}

#[test]
fn a_type_is_a_security_a_party_or_neither() {
    for security in [
        IdType::Isin,
        IdType::Cusip,
        IdType::Sedol,
        IdType::Bloomberg,
        IdType::Figi,
        IdType::Ric,
        IdType::Lei,
        IdType::Forex,
        IdType::Cfi,
        IdType::InstrumentId,
    ] {
        assert!(security.is_security(), "{security}");
        assert!(!security.is_party(), "{security}");
    }
    for party in [
        IdType::Account,
        IdType::Party,
        IdType::UserId,
        IdType::ExecutingFirm,
        IdType::ClientId,
        IdType::ExecutingTrader,
        IdType::OrderOriginationTrader,
        IdType::ContraTrader,
        IdType::ClearingOrganization,
        IdType::DeskId,
        IdType::Algorithm,
    ] {
        assert!(party.is_party(), "{party}");
        assert!(!party.is_security(), "{party}");
    }
    for neither in [
        IdType::OrderId,
        IdType::ClOrdId,
        IdType::OrigClOrdId,
        IdType::ExecId,
        IdType::TradeId,
        IdType::MdEntryRefId,
        IdType::RegTradeId,
        IdType::Tvtic,
        kind("housekey"),
        kind("parentorderid"),
    ] {
        assert!(!neither.is_security(), "{neither}");
        assert!(!neither.is_party(), "{neither}");
    }
    // No type is both, every FIX security source is a security, and the party
    // table is the account, the party of no role, the user and the roles.
    for known in &IdType::KNOWN {
        assert!(!(known.is_security() && known.is_party()), "{known}");
        if known.fix_security_source().is_some() {
            assert!(known.is_security(), "{known}");
        }
    }
    assert_eq!(
        IdType::KNOWN
            .iter()
            .filter(|known| known.is_security())
            .count(),
        33 + 3
    );
    assert_eq!(
        IdType::KNOWN
            .iter()
            .filter(|known| known.is_party())
            .count(),
        3 + 21
    );
    // `PartyRole(452)` `21`'s name is the party role, never the security
    // type `SecurityIDSource(22)` `H` names in full.
    assert_eq!(kind("ClearingOrganization"), IdType::ClearingOrganization);
    assert_eq!(
        kind("Clearing House Clearing Organization"),
        IdType::ClearingHouse
    );
}

#[test]
fn every_fix_security_source_reads_by_its_code_and_its_name() {
    // FIX 4.4's and FIX 5.0's `SecurityIDSource(22)` values, each name as
    // the code set writes it in full, its remarks included.
    for (code, names, kind) in [
        ("1", &["CUSIP"][..], IdType::Cusip),
        ("2", &["SEDOL"], IdType::Sedol),
        ("3", &["QUIK"], IdType::Quik),
        ("4", &["ISIN number", "ISIN"], IdType::Isin),
        ("5", &["RIC code", "RIC"], IdType::Ric),
        (
            "6",
            &["ISO Currency Code", "ISO Currency Code (ISO 4217)"],
            IdType::IsoCcy,
        ),
        ("7", &["ISO Country Code"], IdType::IsoCtry),
        ("8", &["Exchange Symbol"], IdType::ExchSymb),
        (
            "9",
            &[
                "Consolidated Tape Association",
                "Consolidated Tape Association (CTA) Symbol (SIAC CTS/CQS line format)",
            ],
            IdType::Cta,
        ),
        ("A", &["Bloomberg Symbol"], IdType::Bloomberg),
        ("B", &["Wertpapier"], IdType::Wkn),
        ("C", &["Dutch"], IdType::Dutch),
        ("D", &["Valoren"], IdType::Valor),
        ("E", &["Sicovam"], IdType::Sicovam),
        ("F", &["Belgian"], IdType::Belgian),
        (
            "G",
            &["Common", "\"Common\" (Clearstream and Euroclear)"],
            IdType::Common,
        ),
        (
            "H",
            &["Clearing House / Clearing Organization"],
            IdType::ClearingHouse,
        ),
        (
            "I",
            &[
                "ISDA/FpML Product Specification",
                "ISDA/FpML Product Specification (XML in EncodedSecurityDesc <351>)",
                "ISDA/FpML product specification (XML in SecurityXML(1185))",
            ],
            IdType::FpmlSpec,
        ),
        (
            "J",
            &[
                "Option Price Reporting Authority",
                "Options Price Reporting Authority",
            ],
            IdType::Opra,
        ),
        (
            "K",
            &[
                "ISDA/FpML Product URL (URL in SecurityID)",
                "ISDA/FpML product URL (URL in SecurityID(48))",
            ],
            IdType::FpmlUrl,
        ),
        ("L", &["Letter of Credit"], IdType::Loc),
    ] {
        assert_eq!(IdType::from_security_source(code).unwrap(), kind, "{code}");
        for name in names {
            assert_eq!(IdType::from_security_source(name).unwrap(), kind, "{name}");
        }
        let wire = code.chars().next().unwrap();
        assert_eq!(kind.fix_security_source(), Some(wire), "{code}");
        assert!(kind.is_security(), "{code}");
    }
    // A source no member names is kept as it was stated: a private code,
    // 100 and above, a letter FIX gives nothing and a venue's own word are
    // each the word they fold to, never renamed.
    for (stated, typed) in [
        ("100", "100"),
        ("4321", "4321"),
        ("0", "0"),
        ("O", "o"),
        ("Z", "z"),
        ("House Key", "housekey"),
        ("c.u.s.i.p", "c.u.s.i.p"),
    ] {
        let kind = IdType::from_security_source(stated).unwrap();
        assert_eq!(kind.as_str(), typed, "{stated:?}");
        assert!(!kind.is_known(), "{stated:?}");
        assert_eq!(
            kind,
            typed.parse::<IdType>().unwrap(),
            "{stated:?} reads back"
        );
    }
    // A member naming another kind of identifier is no source.
    for other in [
        "ClOrdID",
        "Account",
        "Exchange",
        "ClearingOrganization",
        "Party",
    ] {
        let (path, reason) = located(IdType::from_security_source(other));
        assert_eq!(path, other.to_ascii_lowercase(), "{other}");
        assert!(reason.contains("a security identifier type"), "{reason}");
    }
    // What no word holds is refused, never reshaped into one.
    for refused in [
        "ticker",
        "caf\u{e9}",
        "House/Key",
        "(private)",
        "",
        &"x".repeat(65),
    ] {
        assert!(
            IdType::from_security_source(refused).is_err(),
            "{refused:?}"
        );
    }
}

#[test]
fn a_listing_type_and_the_datatype_a_column_of_each_type_declares() {
    for listing in [
        IdType::Ric,
        IdType::Bloomberg,
        IdType::ExchSymb,
        IdType::Cta,
        IdType::Sedol,
        IdType::Figi,
        IdType::MktAssigned,
        IdType::Fim,
        IdType::Umtf,
        IdType::InstrumentId,
    ] {
        assert!(listing.is_listing(), "{listing}");
    }
    for instrument in [
        IdType::Isin,
        IdType::Cusip,
        IdType::Valor,
        IdType::Wkn,
        IdType::Cfi,
        IdType::ClOrdId,
        kind("housecode"),
    ] {
        assert!(!instrument.is_listing(), "{instrument}");
    }
    for (known, dtype) in [
        (IdType::Isin, DataType::isin()),
        (IdType::Cusip, DataType::cusip()),
        (IdType::Sedol, DataType::sedol()),
        (IdType::Figi, DataType::figi()),
        (IdType::Ric, DataType::ric()),
        (IdType::Bloomberg, DataType::bbg()),
        (IdType::IsoCcy, DataType::ccy()),
        (IdType::IsoCtry, DataType::country()),
        (IdType::Cfi, DataType::cfi()),
        (IdType::Forex, DataType::forex()),
        (IdType::Lei, DataType::lei()),
        (IdType::Dti, DataType::dti()),
        (IdType::Valor, DataType::utf8()),
        (IdType::OrderId, DataType::utf8()),
        (kind("housecode"), DataType::utf8()),
    ] {
        assert_eq!(known.value_dtype(), dtype, "{known}");
    }
    // A market view's column names its type.
    for (column, expected) in [
        ("isincode", IdType::Isin),
        ("riccode", IdType::Ric),
        ("cficode", IdType::Cfi),
        ("forexcode", IdType::Forex),
        ("bloombergcode", IdType::Bloomberg),
        ("figicode", IdType::Figi),
    ] {
        assert_eq!(kind(column), expected, "{column}");
    }
}
