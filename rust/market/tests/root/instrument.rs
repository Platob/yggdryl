//! `rust/market/src/instrument.rs`: one element per instrument keyed by its
//! cross code - a real ISIN for a security, `class:body` for everything no
//! agency numbers, minted its `QY` number - its listings nested, every fact
//! learned from statements and filled into the elements that leave it
//! unsaid, a placeholder re-keyed once its body arrives with the old code
//! among its aliases, the `resolve` waterfall and the economic match, and
//! the collection read from and written to the Arrow record surface.

use smol_str::SmolStr;
use yggdryl::graph::Element;
use yggdryl::{Ccy, Cfi, Country, Decimal, Fisn, Forex, Isin, Mic, Scalar, Uuid};
use yggdryl_market::graph::{Market, OrderEvent};
use yggdryl_market::{
    Characteristics, Exercise, IdKey, IdType, Identifier, Instrument, Instruments, Leg, Listing,
    MAX_CODE_WIDTH, MatchTier, Resolution, Unmatched,
};

const HOLCIM: &str = "CH0012214059";
const APPLE: &str = "US0378331005";
const NOVARTIS: &str = "CH0012005267";
const DIAGEO: &str = "GB0002374006";
const SAP: &str = "DE0007164600";
const STOXX: &str = "EU0009658145";

fn isin(text: &str) -> Isin {
    Isin::new(text).unwrap()
}

/// A real ISIN under `prefix` numbered zero.
fn numbered(prefix: &str) -> String {
    let body = format!("{prefix}000000000");
    format!("{body}{}", Isin::closing_digit(&body).unwrap())
}

fn id(kind: IdType, value: &str) -> Identifier {
    Identifier::new(IdKey::base(kind), value).unwrap()
}

/// An order at `unix` stating each of `codes`.
fn order(unix: i64, codes: &[(IdType, &str)]) -> OrderEvent {
    let mut event = OrderEvent::at(unix);
    for (kind, value) in codes {
        event
            .insert_securityid(id(kind.clone(), value))
            .expect("a plain holder takes every security identifier");
    }
    event
}

/// The security `text` numbers at `unix`, stating each of `codes`.
fn security(text: &str, unix: Option<i64>, codes: &[(IdType, &str)]) -> Instrument {
    codes.iter().fold(
        Instrument::for_security(isin(text))
            .unwrap()
            .with_updunix(unix),
        |entry, (kind, value)| entry.try_with_code(kind.clone(), value).unwrap(),
    )
}

/// The security `text` numbers, listed on `market` under `ticker`.
fn listed(text: &str, market: &str, ticker: Option<&str>) -> Instrument {
    Instrument::for_security(isin(text))
        .unwrap()
        .with_listing(Listing::new(mic(market)).with_ticker(ticker.map(SmolStr::new)))
        .unwrap()
}

fn mic(text: &str) -> Option<Mic> {
    Some(Mic::new(text).unwrap())
}

fn cfi(text: &str) -> Cfi {
    Cfi::new(text).unwrap()
}

fn ccy(text: &str) -> Option<Ccy> {
    Some(Ccy::new(text).unwrap())
}

fn fisn(text: &str) -> Option<Fisn> {
    Some(Fisn::new(text).unwrap())
}

fn forex(text: &str) -> Forex {
    Forex::new(text).unwrap()
}

fn decimal(text: &str) -> Option<Decimal> {
    Some(Decimal::parse(text).unwrap())
}

/// The characteristics of an option or a future: an expiry, a strike.
fn derivative(expiry: &str, strike: Option<&str>) -> Characteristics {
    Characteristics::default()
        .with_expiry(Some(expiry.parse().unwrap()))
        .with_strikepx(strike.and_then(decimal))
}

/// The FX pair instrument of `class` on `pair`, settling at `settles`.
fn fx(class: &str, pair: &str, settles: &[&str]) -> Instrument {
    let characteristics = Characteristics::default()
        .with_settle(settles.first().map(|text| text.parse().unwrap()))
        .with_settle2(settles.get(1).map(|text| text.parse().unwrap()));
    Instrument::for_body(cfi(class), Some(&forex(pair)), &characteristics, None, &[]).unwrap()
}

/// The cross identity the rule of the day gives `code`: the XXH3-64 as the
/// cross hash code, the UUID version 8 over it.
fn cross_of(code: &str) -> (u64, Uuid) {
    let hash = yggdryl::xxhash::xxh3(code.as_bytes());
    (hash, Uuid::from_v8(u128::from(hash)))
}

/// Every production of the cross code grammar, as the design's rows pin
/// them: the code, its XXH3-64 as literal hex, the minted number where the
/// instrument has no agency's; `crossuuid` and `uuid` the one identity the
/// rule of the day derives from the cross hash code.
#[test]
fn the_cross_code_examples_hash_as_pinned() {
    crate::install::installed();
    let apple = || {
        Instrument::for_security(isin(APPLE))
            .unwrap()
            .try_with_cficode(Some(cfi("ESVUFR")))
            .unwrap()
    };
    let call = |strike: &str| {
        Instrument::for_body(
            cfi("OCXXXX"),
            None,
            &derivative("2026-12-18", Some(strike))
                .with_exercise(Some(Exercise::American))
                .with_multiplier(decimal("100")),
            Some(APPLE),
            &[],
        )
        .unwrap()
    };
    let future = |month: &str| {
        Instrument::for_body(
            cfi("FFICSX"),
            None,
            &derivative(month, None),
            Some(STOXX),
            &[],
        )
        .unwrap()
    };
    let spread = |legs: &[&str]| {
        let legs: Vec<Leg> = legs.iter().map(|code| Leg::new(code, 1).unwrap()).collect();
        Instrument::for_body(
            cfi("KEXXXX"),
            None,
            &Characteristics::default(),
            None,
            &legs,
        )
        .unwrap()
    };
    let december = "FF:EU0009658145:2026-12";
    let march = "FF:EU0009658145:2027-03";
    let rows: [(Instrument, &str, &str, Option<&str>); 14] = [
        (apple(), APPLE, "27e376388c8738fd", None),
        (
            Instrument::for_security(isin(APPLE)).unwrap(),
            APPLE,
            "27e376388c8738fd",
            None,
        ),
        (
            Instrument::for_security(isin(STOXX))
                .unwrap()
                .try_with_cficode(Some(cfi("TIXXXX")))
                .unwrap(),
            STOXX,
            "fe3a11ff3d28a176",
            None,
        ),
        (
            fx("IFXXXP", "EUR/USD", &[]),
            "IF:EUR/USD",
            "f4bd070ccad3d90f",
            Some("QYLTVIRYHNX5"),
        ),
        (
            fx("JFTXFP", "EUR/USD", &["M3"]),
            "JF:EUR/USD:M3",
            "817c2b867e12a51a",
            Some("QYIJ9KBCDDV1"),
        ),
        (
            fx("JFTXFP", "EUR/USD", &["2027-01-15"]),
            "JF:EUR/USD:2027-01-15",
            "b1e8c6896a1c7a77",
            Some("QYI2FZFJUNE8"),
        ),
        (
            fx("SFXXXX", "USD/JPY", &["0", "M3"]),
            "SF:USD/JPY:0:M3",
            "7fffc4bbae889ca0",
            Some("QYKXOCJXPVF9"),
        ),
        (
            fx("ITKXXX", "XAU/USD", &[]),
            "IT:XAU/USD",
            "f0083480b6c87b8d",
            Some("QY900CSWCFZ9"),
        ),
        (
            call("200"),
            "OC:US0378331005:2026-12-18:200",
            "6f50390c1bd8f746",
            Some("QYK7DLZMSYS1"),
        ),
        (
            call("210"),
            "OC:US0378331005:2026-12-18:210",
            "3b15a30b31569400",
            Some("QYG1U5ULBQ73"),
        ),
        (
            Instrument::for_body(
                cfi("OPXXXX"),
                None,
                &derivative("2026-12-18", Some("200")),
                Some(APPLE),
                &[],
            )
            .unwrap(),
            "OP:US0378331005:2026-12-18:200",
            "abb15048d5817f7b",
            Some("QY6TB00ZO934"),
        ),
        (
            future("2026-12-18"),
            december,
            "4de3b6fc251f4c55",
            Some("QY4NFU6XFYI7"),
        ),
        (
            future("2027-03"),
            march,
            "1024ff3991480a91",
            Some("QY3KUS57QPX3"),
        ),
        (
            spread(&[december, march]),
            "KE:FF:EU0009658145:2026-12+FF:EU0009658145:2027-03",
            "69efa5da470620f0",
            Some("QY9THOC9IUF9"),
        ),
    ];
    for (instrument, code, hash, minted) in &rows {
        assert_eq!(instrument.get_crosscode(), *code);
        assert!(instrument.get_crosscode().len() <= MAX_CODE_WIDTH);
        assert_eq!(
            format!("{:016x}", instrument.get_crosshashcode()),
            *hash,
            "{code}"
        );
        let (crosshash, crossuuid) = cross_of(code);
        assert_eq!(instrument.get_crosshashcode(), crosshash, "{code}");
        assert_eq!(instrument.get_crossuuid(), crossuuid, "{code}");
        assert_eq!(
            instrument.get_uuid(),
            crossuuid,
            "{code}: uuid is crossuuid"
        );
        assert_eq!(instrument.minted_isin(), *minted, "{code}");
        assert!(!instrument.is_placeholder(), "{code}");
        match minted {
            Some(number) => {
                assert_eq!(instrument.isin(), Some(*number), "{code}: the base isin");
                assert!(instrument.is_own_mint(&isin(number)), "{code}");
                assert_eq!(Instrument::minted_number(code).as_str(), *number);
                assert!(Isin::is_minted(
                    number,
                    yggdryl::xxhash::xxh128(code.as_bytes())
                ));
                assert_eq!(
                    instrument
                        .securityids()
                        .get_from(&"yggdryl:isin".parse().unwrap()),
                    Some(*number),
                    "{code}: minted under its own source"
                );
                assert!(!instrument.is_own_mint(&isin("QYLTVIRYHNX5")) || *code == "IF:EUR/USD");
            }
            None => assert_eq!(instrument.isin(), Some(*code)),
        }
        assert_eq!(
            Instrument::from_scalar(&instrument.into_scalar()).unwrap(),
            *instrument,
            "{code}: round trip"
        );
    }
    // The strike spelled three ways is one code; a strategy stated in
    // either leg order is one code.
    for spelling in ["200.0", "200.00", "2.0E2"] {
        assert_eq!(
            call(spelling).get_crosscode(),
            "OC:US0378331005:2026-12-18:200",
            "{spelling}"
        );
    }
    assert_eq!(
        spread(&[march, december]).get_crosscode(),
        "KE:FF:EU0009658145:2026-12+FF:EU0009658145:2027-03"
    );
    // The multiplier and the exercise are facts, never key bytes: without
    // them the call keys the same and hashes otherwise.
    let bare = Instrument::for_body(
        cfi("OCXXXX"),
        None,
        &derivative("2026-12-18", Some("200")),
        Some(APPLE),
        &[],
    )
    .unwrap();
    assert_eq!(bare.get_crosscode(), call("200").get_crosscode());
    assert_eq!(bare.get_crossuuid(), call("200").get_crossuuid());
    assert_ne!(bare.get_hashcode(), call("200").get_hashcode());
    // A derivative stating a real ISIN and no body is keyed by the ISIN as
    // a placeholder: no mint, its own identity.
    let eurex = numbered("DE");
    let placeholder = Instrument::for_security(isin(&eurex))
        .unwrap()
        .try_with_cficode(Some(cfi("OCXXXX")))
        .unwrap()
        .try_with_underlying(Some(APPLE))
        .unwrap();
    assert_eq!(placeholder.get_crosscode(), eurex);
    assert!(placeholder.is_placeholder());
    assert_eq!(placeholder.minted_isin(), None);
    assert_eq!(placeholder.isin(), Some(eurex.as_str()));
    assert_eq!(placeholder.get_crosshashcode(), cross_of(&eurex).0);
    assert_eq!(placeholder.underlying(), Some(APPLE));
    assert_eq!(placeholder.underlying_uuid(), Some(cross_of(APPLE).1));
}

/// A security is keyed by a real ISIN alone - a number no agency gave is
/// refused - and nothing but the key moves its identity: a CFI learned or
/// refined, a listing, a ticker, a code, a stamp each move `hashcode`
/// alone, and the stamps and the sources move nothing at all.
#[test]
fn a_security_is_keyed_by_its_real_isin_and_only_the_key_moves_its_identity() {
    crate::install::installed();
    for refused in [numbered("ZZ"), "US0378331006".to_owned()] {
        let error = Instrument::for_security(isin(&refused))
            .unwrap_err()
            .to_string();
        assert!(error.contains("isin"), "{refused}: {error}");
    }
    let plain = Instrument::for_security(isin(APPLE)).unwrap();
    assert_eq!(plain.country(), Some(Country::new("US").unwrap()));
    assert_eq!(plain.countrycode(), None, "the prefix, held nowhere");
    assert_eq!(plain.class(), None);
    assert_eq!(
        plain.get(&IdType::Cusip),
        None,
        "nothing derived on its own"
    );
    let identity = (plain.get_crossuuid(), plain.get_crosshashcode());
    let mut hashes = vec![plain.get_hashcode()];
    let classified = plain.clone().try_with_cficode(Some(cfi("ESVUFR"))).unwrap();
    assert_eq!(classified.class(), Some("ES"));
    hashes.push(classified.get_hashcode());
    let listed = classified
        .clone()
        .with_listing(
            Listing::new(mic("XNAS"))
                .with_ticker(Some("AAPL".into()))
                .with_currency(ccy("USD")),
        )
        .unwrap();
    hashes.push(listed.get_hashcode());
    let coded = listed
        .clone()
        .try_with_code(IdType::Lei, "HWUPKR0MPOU8FGXBT394")
        .unwrap()
        .try_with_code(IdType::Ric, "AAPL.OQ")
        .unwrap();
    assert_eq!(coded.get(&IdType::Ric), Some("AAPL.OQ"));
    assert_eq!(
        coded
            .listing(mic("XNAS").as_ref())
            .unwrap()
            .get(&IdType::Ric),
        Some("AAPL.OQ"),
        "a listing code lands on the one listing"
    );
    assert_eq!(coded.securityids().get(&IdType::Ric), None);
    assert_eq!(
        coded.securityids().get(&IdType::Lei),
        Some("HWUPKR0MPOU8FGXBT394")
    );
    hashes.push(coded.get_hashcode());
    let named = coded
        .clone()
        .with_fisn(fisn("APPLE INC/SH"))
        .with_origccy(ccy("USD"));
    hashes.push(named.get_hashcode());
    for instrument in [&classified, &listed, &coded, &named] {
        assert_eq!(
            (instrument.get_crossuuid(), instrument.get_crosshashcode()),
            identity
        );
        assert_eq!(instrument.get_uuid(), instrument.get_crossuuid());
    }
    let distinct: std::collections::HashSet<u64> = hashes.iter().copied().collect();
    assert_eq!(distinct.len(), hashes.len(), "every fact moves hashcode");
    let stamped = named
        .clone()
        .with_updunix(Some(10))
        .with_firstunix(Some(5))
        .with_lastunix(Some(20));
    assert_eq!(
        stamped.get_hashcode(),
        named.get_hashcode(),
        "a stamp is fed nowhere"
    );
    let mut sourced = stamped.clone();
    sourced.set_srcuuids(vec![Uuid::from_v8(7)]);
    assert_eq!(
        sourced.get_hashcode(),
        stamped.get_hashcode(),
        "a source is fed nowhere"
    );
    assert_eq!(sourced.get_srcuuids(), [Uuid::from_v8(7)]);
    // The columns, in order.
    let field = Instrument::field();
    let names: Vec<&str> = field.fields().iter().map(|column| column.name()).collect();
    assert_eq!(
        names,
        [
            "uuid",
            "crossuuid",
            "crosscode",
            "hashcode",
            "crosshashcode",
            "srcuuids",
            "aliascodes",
            "placeholder",
            "isin",
            "cficode",
            "forexcode",
            "fisn",
            "countrycode",
            "currency",
            "origccy",
            "securityids",
            "underlying",
            "legs",
            "eusipacode",
            "characteristics",
            "listings",
            "metadata",
            "updunix",
            "firstunix",
            "lastunix",
        ]
    );
    assert!(!field.is_nullable());
    assert_eq!(Instrument::dtype(), field.dtype().clone());
}

/// A body is written whole or not at all: a class whose body the facts do
/// not spell is refused naming the class, a class that keys by no body is
/// refused, a leg is checked by shape, and a code past the width is refused
/// naming the width - the bounds on the legs, the listings, the listing
/// codes and the equivalents each by name.
#[test]
fn a_body_is_written_whole_or_refused_by_name_and_every_bound_is_named() {
    crate::install::installed();
    let none = Characteristics::default();
    let pair = forex("EUR/USD");
    for (class, forex, characteristics, underlying) in [
        ("IFXXXP", None, none.clone(), None),
        ("JFTXFP", Some(&pair), none.clone(), None),
        (
            "SFXXXX",
            Some(&pair),
            derivative("2026-12-18", None).with_settle(Some("0".parse().unwrap())),
            None,
        ),
        ("OCXXXX", None, derivative("2026-12-18", None), Some(APPLE)),
        ("OCXXXX", None, derivative("2026-12-18", Some("200")), None),
        ("FFICSX", None, none.clone(), Some(STOXX)),
        ("KEXXXX", None, none.clone(), None),
        ("ESVUFR", None, none.clone(), None),
        ("TIXXXX", Some(&pair), none.clone(), Some(APPLE)),
    ] {
        let error = Instrument::for_body(cfi(class), forex, &characteristics, underlying, &[])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("no body") && error.contains(class),
            "{class}: {error}"
        );
    }
    for (code, ratio) in [("", 1), ("ff:x", 1), ("KE:FF:X+FF:Y", 1), (APPLE, 0)] {
        assert!(Leg::new(code, ratio).is_err(), "{code:?} * {ratio}");
    }
    let leg = Leg::new("FF:EU0009658145:2026-12", 2).unwrap();
    assert_eq!((leg.code(), leg.ratio()), ("FF:EU0009658145:2026-12", 2));
    let ratioed = Instrument::for_body(
        cfi("KEXXXX"),
        None,
        &none,
        None,
        &[leg.clone(), Leg::new("FF:EU0009658145:2027-03", 1).unwrap()],
    )
    .unwrap();
    assert_eq!(
        ratioed.get_crosscode(),
        "KE:2*FF:EU0009658145:2026-12+FF:EU0009658145:2027-03"
    );
    assert_eq!(ratioed.legs().len(), 2);
    assert_eq!(
        ratioed.leg_uuids().collect::<Vec<_>>(),
        [
            cross_of("FF:EU0009658145:2026-12").1,
            cross_of("FF:EU0009658145:2027-03").1
        ]
    );
    // Six futures of twenty-three bytes each spell a strategy past the width.
    let months = [
        "2026-12", "2027-03", "2027-06", "2027-09", "2027-12", "2028-03",
    ];
    let legs: Vec<Leg> = months
        .iter()
        .map(|month| Leg::new(&format!("FF:EU0009658145:{month}"), 1).unwrap())
        .collect();
    let error = Instrument::for_body(cfi("KEXXXX"), None, &none, None, &legs)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(&MAX_CODE_WIDTH.to_string()) && error.contains("KE"),
        "{error}"
    );
    assert_eq!(MAX_CODE_WIDTH, 128);
    let too_many: Vec<Leg> = (0..=Instrument::MAX_LEGS)
        .map(|at| Leg::new(&format!("IF:EUR/US{at:02}"), 1).unwrap())
        .collect();
    assert!(
        Instrument::for_body(cfi("KEXXXX"), None, &none, None, &too_many).is_err(),
        "past {} legs",
        Instrument::MAX_LEGS
    );
    // An underlying or a leg that is no cross code.
    assert!(
        Instrument::for_security(isin(APPLE))
            .unwrap()
            .try_with_underlying(Some("not a code"))
            .is_err()
    );
    // The listings: at most six markets, the seventh refused by name.
    let markets = ["XNAS", "XNYS", "XLON", "XETR", "XSWX", "XPAR", "XTKS"];
    let mut apple = Instrument::for_security(isin(APPLE)).unwrap();
    for market in &markets[..Instrument::MAX_LISTINGS] {
        apple = apple.with_listing(Listing::new(mic(market))).unwrap();
    }
    assert_eq!(apple.listings().len(), Instrument::MAX_LISTINGS);
    let error = apple
        .clone()
        .with_listing(Listing::new(mic(markets[Instrument::MAX_LISTINGS])))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("listings") && error.contains("at most 6"),
        "{error}"
    );
    assert!(
        apple
            .clone()
            .with_listing(Listing::new(mic("XNAS")))
            .is_ok(),
        "a listed market restated"
    );
    // A listing code on an instrument of several listings is stated on its
    // market, never on the instrument.
    let error = apple
        .clone()
        .try_with_code(IdType::Ric, "AAPL.OQ")
        .unwrap_err()
        .to_string();
    assert!(error.contains("on its market"), "{error}");
    // The equivalents: twelve, the thirteenth refused by name; a key type
    // has its own door.
    let mut full = Instrument::for_security(isin(HOLCIM)).unwrap();
    for kind in [
        IdType::Quik,
        IdType::Dutch,
        IdType::Sicovam,
        IdType::Belgian,
        IdType::Common,
        IdType::ClearingHouse,
        IdType::FpmlSpec,
        IdType::Opra,
        IdType::FpmlUrl,
        IdType::Loc,
        IdType::MktAssigned,
        IdType::RedEntity,
    ] {
        full.set_code(kind, "X-1").unwrap();
    }
    let error = full
        .set_code(IdType::RedPair, "X-1")
        .unwrap_err()
        .to_string();
    assert!(error.contains("at most 12 equivalents"), "{error}");
    for key in [IdType::Isin, IdType::Cfi, IdType::Forex, IdType::Fisn] {
        let error = full.set_code(key.clone(), "X").unwrap_err().to_string();
        assert!(error.contains("own door"), "{key}: {error}");
    }
    assert!(
        full.set_code(IdType::ClOrdId, "X-1").is_err(),
        "no security type"
    );
    assert_eq!(full.securityids().get(&IdType::Quik), Some("X-1"));
}

/// A placeholder keyed by its ISIN is re-keyed by the statement spelling
/// its body - the old code among its aliases, reached by either, the
/// identity the new code's - and a derivative written on the old code or
/// holding it as a leg is re-keyed after it; a code bound is kept by name.
#[test]
fn a_placeholder_is_rekeyed_by_its_body_and_its_dependents_follow() {
    crate::install::installed();
    let eurex = numbered("DE");
    let option = "OC:US0378331005:2026-12-18:200";
    let mut instruments = Instruments::new();
    instruments
        .merge(listed(APPLE, "XNAS", Some("AAPL")))
        .unwrap();
    let held = Instrument::for_security(isin(&eurex))
        .unwrap()
        .try_with_cficode(Some(cfi("OCXXXX")))
        .unwrap()
        .with_listing(Listing::new(mic("XEUR")).with_ticker(Some("ODAX".into())))
        .unwrap();
    assert!(instruments.merge(held).unwrap());
    assert!(instruments.get(&eurex).unwrap().is_placeholder());
    // A strategy holding the placeholder as a leg, and a future written on
    // it, both wait on it.
    let strategy = Instrument::for_body(
        cfi("KEXXXX"),
        None,
        &Characteristics::default(),
        None,
        &[
            Leg::new(&eurex, 1).unwrap(),
            Leg::new("FF:EU0009658145:2027-03", 1).unwrap(),
        ],
    )
    .unwrap();
    let strategy_code = strategy.get_crosscode().to_owned();
    assert!(instruments.merge(strategy).unwrap());
    let written = Instrument::for_body(
        cfi("FFICSX"),
        None,
        &derivative("2027-03", None),
        Some(&eurex),
        &[],
    )
    .unwrap();
    let written_code = written.get_crosscode().to_owned();
    assert!(instruments.merge(written).unwrap());
    assert_eq!(instruments.len(), 4);
    // The body arrives: the strike and the expiry on the option's number.
    let body = Instrument::for_security(isin(&eurex))
        .unwrap()
        .try_with_cficode(Some(cfi("OCXXXX")))
        .unwrap()
        .try_with_underlying(Some(APPLE))
        .unwrap()
        .try_with_characteristics(derivative("2026-12-18", Some("200")))
        .unwrap();
    assert_eq!(body.get_crosscode(), option);
    assert_eq!(body.isin(), Some(eurex.as_str()), "the number stays a fact");
    assert_eq!(body.minted_isin(), None, "a number stated, none minted");
    assert!(instruments.merge(body).unwrap());
    assert_eq!(instruments.len(), 4, "re-keyed, not added");
    let rekeyed = instruments.get(option).expect("under the new code");
    assert_eq!(
        instruments.get(&eurex).map(Element::get_crosscode),
        Some(option)
    );
    assert!(!rekeyed.is_placeholder());
    assert_eq!(rekeyed.aliascodes(), [eurex.as_str()]);
    assert_eq!(rekeyed.isin(), Some(eurex.as_str()));
    assert_eq!(
        rekeyed.get_crossuuid(),
        cross_of(option).1,
        "the new identity"
    );
    assert_eq!(
        rekeyed.ticker(mic("XEUR").as_ref()),
        Some("ODAX"),
        "the listing stays"
    );
    assert_eq!(rekeyed.underlying(), Some(APPLE));
    assert_eq!(
        instruments
            .get_by_uuid(cross_of(option).1)
            .map(Element::get_crosscode),
        Some(option)
    );
    // The cascade.
    assert!(
        instruments.get(&strategy_code).is_some(),
        "the old code is an alias"
    );
    let strategy = instruments.get(&strategy_code).unwrap();
    assert_eq!(
        strategy.get_crosscode(),
        format!("KE:FF:EU0009658145:2027-03+{option}"),
        "the legs in code order"
    );
    assert_eq!(strategy.aliascodes(), [strategy_code.as_str()]);
    assert_eq!(strategy.legs()[1].code(), option);
    let future = instruments.get(&written_code).unwrap();
    assert_eq!(future.get_crosscode(), format!("FF:{option}:2027-03"));
    assert_eq!(future.underlying(), Some(option));
    assert_eq!(future.aliascodes(), [written_code.as_str()]);
    // A placeholder's number alone, stated again, moves nothing more.
    assert!(
        !instruments
            .merge(
                Instrument::for_security(isin(&eurex))
                    .unwrap()
                    .try_with_cficode(Some(cfi("OCXXXX")))
                    .unwrap()
            )
            .unwrap()
    );
    // The gold join: an element filled before the re-key carries the old
    // code, which is now an alias of the instrument.
    let instrument = instruments.get(option).unwrap();
    assert!(
        instrument
            .aliascodes()
            .iter()
            .any(|alias| alias == eurex.as_str())
            || instrument.get_crosscode() == eurex
    );
    // Removing by an alias removes the instrument.
    assert!(instruments.remove(&eurex).is_some());
    assert_eq!(instruments.get(option), None);
    assert_eq!(instruments.len(), 3);
}

/// An FX instrument holds its listing codes among its security identifiers
/// while it is listed nowhere, and a listing code lands on the listing
/// once a market is stated; a foreign `QY` number read off a row is kept
/// as stated and is nobody's mint.
#[test]
fn a_listing_code_lands_on_the_listing_and_a_foreign_number_is_kept() {
    crate::install::installed();
    let spot = fx("IFXXXP", "EUR/USD", &[])
        .try_with_code(IdType::Ric, "EUR=")
        .unwrap();
    assert_eq!(spot.securityids().get(&IdType::Ric), Some("EUR="));
    assert_eq!(spot.get(&IdType::Ric), Some("EUR="));
    assert_eq!(spot.listings(), []);
    assert_eq!(spot.forexcode(), Some(forex("EUR/USD")));
    assert_eq!(spot.currency(), None, "a statement states, a fold derives");
    assert_eq!(spot.country(), None, "a pair has no country");
    let venue = spot
        .clone()
        .with_listing(Listing::new(mic("XOFF")))
        .unwrap()
        .try_with_code(IdType::Bloomberg, "EURUSD Curncy")
        .unwrap();
    let listing = venue.listing(mic("XOFF").as_ref()).unwrap();
    assert_eq!(
        listing.get(&IdType::Bloomberg),
        Some("EURUSD Curncy"),
        "stated with one listing, a listing code lands on it"
    );
    assert_eq!(
        listing.get(&IdType::Ric),
        Some("EUR="),
        "moved onto the first market"
    );
    assert_eq!(venue.securityids().get(&IdType::Ric), None);
    assert_eq!(venue.get(&IdType::Ric), Some("EUR="));
    // Learned on a market, the statement's listing codes land on the listing.
    let mut instruments = Instruments::new();
    instruments.merge(spot.clone()).unwrap();
    let mut traded = order(
        10,
        &[
            (IdType::Forex, "EUR/USD"),
            (IdType::Bloomberg, "EURUSD Curncy"),
        ],
    );
    traded.set_cficode(Some(cfi("IFXXXP")), true);
    traded.set_miccode(mic("XOFF"), true);
    traded.set_ticker(Some("EURUSD".into()), true);
    assert!(instruments.learn(&traded));
    let held = instruments.get("IF:EUR/USD").unwrap();
    assert_eq!(
        held.currency().map(Ccy::as_str),
        Some("USD"),
        "the quote leg, derived"
    );
    let listing = held
        .listing(mic("XOFF").as_ref())
        .expect("the market learned");
    assert_eq!(listing.ticker(), Some("EURUSD"));
    assert_eq!(listing.get(&IdType::Bloomberg), Some("EURUSD Curncy"));
    assert_eq!(
        listing.get(&IdType::Ric),
        Some("EUR="),
        "moved onto the market learned"
    );
    assert_eq!(held.securityids().get(&IdType::Bloomberg), None);
    assert_eq!(held.securityids().get(&IdType::Ric), None);
    assert_eq!(held.minted_isin(), Some("QYLTVIRYHNX5"));
    // A row stating another system's `QY` number under the pair.
    let mut cells = fx("ITKXXX", "XAU/USD", &[]).into_scalar();
    let foreign = "QYLTVIRYHNX5";
    let ids = Scalar::from_mapping([
        (Scalar::from("cfi"), Scalar::from("ITKXXX")),
        (Scalar::from("forex"), Scalar::from("XAU/USD")),
        (Scalar::from("isin"), Scalar::from(foreign)),
    ])
    .unwrap();
    cells = cells.with_field("securityids", ids).unwrap();
    cells = cells.with_field("isin", Scalar::from(foreign)).unwrap();
    let read = Instrument::from_scalar(&cells).unwrap();
    assert_eq!(read.get_crosscode(), "IT:XAU/USD");
    assert_eq!(read.isin(), Some(foreign), "kept as stated");
    assert_eq!(
        read.minted_isin(),
        None,
        "nothing minted over a number held"
    );
    assert!(!read.is_own_mint(&isin(foreign)));
    assert!(fx("IFXXXP", "EUR/USD", &[]).is_own_mint(&isin(foreign)));
}

/// The generic path: an order stating a pair and an `I*` class and no ISIN
/// creates the spot through `learn`, and `fill` writes its cross code as
/// `instcode` and its minted number as the ISIN, derived; a `J*` or `S*`
/// element states no settle, so it learns nothing; a security filled takes
/// its code too, and a stated `instcode` stands.
#[test]
fn an_order_learns_an_fx_spot_and_is_filled_with_its_code() {
    crate::install::installed();
    let mut instruments = Instruments::new();
    let mut spot = order(10, &[(IdType::Forex, "EUR/USD")]);
    spot.set_cficode(Some(cfi("IFXXXP")), true);
    spot.set_currency(Ccy::new("EUR").unwrap(), true);
    assert_eq!(spot.get_instcode(), None);
    assert!(instruments.learn(&spot));
    assert_eq!(instruments.len(), 1);
    let held = instruments
        .get("IF:EUR/USD")
        .expect("created from the order");
    assert_eq!(held.minted_isin(), Some("QYLTVIRYHNX5"));
    assert_eq!(held.forexcode(), Some(forex("EUR/USD")));
    assert_eq!(held.class(), Some("IF"));
    assert_eq!(
        held.currency().map(Ccy::as_str),
        Some("USD"),
        "the quote leg, never the dealt one"
    );
    assert_eq!(held.listings(), [], "no market stated");
    assert_eq!(
        (held.firstunix(), held.lastunix(), held.updunix()),
        (Some(10), Some(10), Some(10))
    );
    assert_eq!(
        instruments.get("QYLTVIRYHNX5").map(Element::get_crosscode),
        Some("IF:EUR/USD")
    );
    let mut later = order(20, &[(IdType::Forex, "EUR/USD")]);
    later.set_cficode(Some(cfi("IFXXXP")), true);
    assert!(instruments.fill(&mut later));
    assert_eq!(later.get_instcode(), Some("IF:EUR/USD"));
    assert_eq!(later.get_isincode(), Some("QYLTVIRYHNX5"));
    assert!(later.get_securityids().is_derived(&IdType::Isin));
    assert!(!instruments.fill(&mut later), "nothing left");
    // The minted number alone names the instrument.
    let mut numbered_only = order(30, &[(IdType::Isin, "QYLTVIRYHNX5")]);
    assert!(instruments.fill(&mut numbered_only));
    assert_eq!(numbered_only.get_instcode(), Some("IF:EUR/USD"));
    assert_eq!(
        numbered_only.get_securityids().get(&IdType::Forex),
        Some("EUR/USD")
    );
    assert!(numbered_only.get_securityids().is_derived(&IdType::Forex));
    // A derived number is learned back by nothing.
    assert!(instruments.learn(&later), "the instant moves");
    assert_eq!(instruments.len(), 1);
    assert_eq!(instruments.get("IF:EUR/USD").unwrap().lastunix(), Some(20));
    // A forward on a bare market element states no settle: nothing learned.
    let mut forward = order(40, &[(IdType::Forex, "EUR/USD")]);
    forward.set_cficode(Some(cfi("JFTXFP")), true);
    assert!(!instruments.learn(&forward));
    assert_eq!(
        instruments.resolve(&forward),
        Resolution::Unmatched(Unmatched::NoKey),
        "a class with no body spells no code"
    );
    assert!(!instruments.fill(&mut forward));
    assert_eq!(instruments.len(), 1);
    // A ticker-only security has no instrument.
    let mut ticked = OrderEvent::at(50);
    ticked.set_ticker(Some("AAPL".into()), true);
    ticked.set_miccode(mic("XNAS"), true);
    assert!(!instruments.learn(&ticked));
    assert_eq!(ticked.get_instcode(), None);
    // A security filled takes its code; a stated code stands.
    instruments
        .merge(listed(APPLE, "XNAS", Some("AAPL")))
        .unwrap();
    let mut apple = order(60, &[(IdType::Isin, APPLE)]);
    assert!(instruments.fill(&mut apple));
    assert_eq!(apple.get_instcode(), Some(APPLE));
    assert_eq!(apple.get_ticker(), Some("AAPL"));
    let mut stated = order(60, &[(IdType::Isin, APPLE)]);
    stated.set_instcode(Some("mine".into()), true);
    instruments.fill(&mut stated);
    assert_eq!(stated.get_instcode(), Some("mine"));
    // A spot whose pair the table does not hold ends the cascade by its code.
    let mut unknown = order(70, &[(IdType::Forex, "GBP/USD"), (IdType::Ric, "AAPL.OQ")]);
    unknown.set_cficode(Some(cfi("IFXXXP")), true);
    assert_eq!(
        instruments.resolve(&unknown),
        Resolution::Unmatched(Unmatched::UnknownCode {
            stated: "IF:GBP/USD".into()
        })
    );
    // `enrich` learns and fills in one.
    let mut cable = order(80, &[(IdType::Forex, "GBP/USD")]);
    cable.set_cficode(Some(cfi("IFXXXP")), true);
    assert!(instruments.enrich(&mut cable));
    assert_eq!(cable.get_instcode(), Some("IF:GBP/USD"));
    assert_eq!(
        cable.get_isincode(),
        Some(Instrument::minted_number("IF:GBP/USD").as_str())
    );
}

/// A statement is learned by its ISIN and filled into one naming it: the
/// listing facts on the listing its market names, the equivalents on the
/// instrument, the ISIN alone filling the rest as derivations, a fill never
/// learned back, the currency only on the same market with the same ticker.
#[test]
fn a_statement_is_learned_by_its_isin_and_filled_into_one_naming_it() {
    crate::install::installed();
    let mut stated = order(
        10,
        &[
            (IdType::Isin, HOLCIM),
            (IdType::Ric, "HOLN.S"),
            (IdType::Bloomberg, "HOLN SW Equity"),
            (IdType::Common, "C-1"),
        ],
    );
    stated.set_cficode(Some(cfi("ESVUFR")), true);
    stated.set_miccode(mic("XSWX"), true);
    stated.set_ticker(Some(SmolStr::new("HOLN")), true);
    stated.set_currency(Ccy::new("CHF").unwrap(), true);
    let mut instruments = Instruments::new();
    assert!(!instruments.is_dirty());
    assert!(instruments.learn(&stated));
    assert!(instruments.is_dirty());
    assert!(!instruments.learn(&stated), "nothing new");
    let row = instruments.get(HOLCIM).expect("an instrument");
    assert_eq!(row.updunix(), Some(10));
    assert_eq!(row.get(&IdType::Ric), Some("HOLN.S"));
    assert_eq!(row.get(&IdType::Bloomberg), Some("HOLN SW Equity"));
    assert_eq!(row.get(&IdType::Common), Some("C-1"));
    let listing = row.listing(mic("XSWX").as_ref()).expect("listed");
    assert_eq!(listing.get(&IdType::Ric), Some("HOLN.S"));
    assert_eq!(listing.ticker(), Some("HOLN"));
    assert_eq!(listing.currency().map(Ccy::as_str), Some("CHF"));
    assert_eq!(
        row.securityids().get(&IdType::Ric),
        None,
        "a listing code is the listing's"
    );
    assert_eq!(row.securityids().get(&IdType::Common), Some("C-1"));
    assert_eq!(
        row.cficode().map(|code| code.as_str().to_owned()),
        Some("ESVUFR".into())
    );
    assert_eq!(row.ticker(mic("XSWX").as_ref()), Some("HOLN"));
    assert_eq!(row.ticker(None), Some("HOLN"), "the single listing");
    assert_eq!(row.countrycode(), None, "no statement named a country");
    assert_eq!(
        row.country(),
        Some(Country::new("CH").unwrap()),
        "the prefix"
    );
    assert_eq!(row.forexcode(), None);
    assert_eq!(
        row.get(&IdType::Valor),
        Some("1221405"),
        "the number its ISIN embeds"
    );

    // The ISIN alone fills the rest, each as a derivation.
    let mut named = order(20, &[(IdType::Isin, HOLCIM)]);
    assert!(instruments.fill(&mut named));
    let ids = named.get_securityids();
    assert_eq!(ids.get(&IdType::Ric), Some("HOLN.S"));
    assert_eq!(ids.get(&IdType::Common), Some("C-1"));
    assert!(ids.is_derived(&IdType::Ric));
    assert!(!ids.is_derived(&IdType::Isin));
    assert_eq!(named.get_ticker(), Some("HOLN"));
    assert_eq!(
        named.get_cficode().map(|code| code.as_str().to_owned()),
        Some("ESVUFR".into())
    );
    assert_eq!(named.get_instcode(), Some(HOLCIM));
    assert!(
        named.get_currency().is_none(),
        "the currency fills only where the markets are stated and equal"
    );
    assert!(!instruments.fill(&mut named), "nothing left to fill");
    // What a fill derived is never learned back.
    assert!(instruments.learn(&named));
    let row = instruments.get(HOLCIM).unwrap();
    assert_eq!((row.updunix(), row.lastunix()), (Some(10), Some(20)));
    assert!(!instruments.learn(&named), "met again at the same instant");

    // On the listing's market, with the row's ticker, the currency fills too.
    let mut on_market = order(20, &[(IdType::Isin, HOLCIM)]);
    on_market.set_miccode(mic("XSWX"), true);
    assert!(instruments.fill(&mut on_market));
    assert_eq!(on_market.get_ticker(), Some("HOLN"));
    assert_eq!(on_market.get_currency().as_str(), "CHF");
    // On another market: the instrument's facts, none of the listing's.
    let mut elsewhere = order(20, &[(IdType::Isin, HOLCIM)]);
    elsewhere.set_miccode(mic("XLON"), true);
    assert!(instruments.fill(&mut elsewhere));
    assert_eq!(elsewhere.get_securityids().get(&IdType::Ric), None);
    assert_eq!(
        elsewhere.get_securityids().get(&IdType::Common),
        Some("C-1")
    );
    assert_eq!(elsewhere.get_ticker(), None);
    assert_eq!(elsewhere.get_instcode(), Some(HOLCIM));

    // A statement of its own stands; a refined CFI code refines it.
    let mut own = order(30, &[(IdType::Isin, HOLCIM), (IdType::Common, "C-9")]);
    own.set_cficode(Some(cfi("ESXUFR")), true);
    assert!(instruments.fill(&mut own));
    assert_eq!(own.get_securityids().get(&IdType::Common), Some("C-9"));
    assert_eq!(
        own.get_cficode().map(|code| code.as_str().to_owned()),
        Some("ESVUFR".into())
    );

    // A statement without an ISIN is learned by nothing.
    assert!(!instruments.learn(&order(
        30,
        &[(IdType::Ric, "HOLN.S"), (IdType::Belgian, "B-1")]
    )));
    assert_eq!(instruments.get(HOLCIM).unwrap().get(&IdType::Belgian), None);
    // A second market is a second listing; a statement naming no market
    // lands its listing facts on none of several.
    let mut london = order(40, &[(IdType::Isin, HOLCIM)]);
    london.set_miccode(mic("XLON"), true);
    london.set_ticker(Some("0QKY".into()), true);
    assert!(instruments.learn(&london));
    let row = instruments.get(HOLCIM).unwrap();
    assert_eq!(row.listings().len(), 2);
    assert_eq!(row.ticker(mic("XLON").as_ref()), Some("0QKY"));
    assert_eq!(row.ticker(None), None, "several listings");
    assert_eq!(
        row.listing(mic("XLON").as_ref())
            .unwrap()
            .currency()
            .map(Ccy::as_str),
        Some("GBP"),
        "the legal tender of the market's country"
    );
    let mut unmarked = order(50, &[(IdType::Isin, HOLCIM)]);
    unmarked.set_ticker(Some("ELSEWHERE".into()), true);
    assert!(instruments.learn(&unmarked), "the instant moves");
    let row = instruments.get(HOLCIM).unwrap();
    assert_eq!(row.ticker(mic("XSWX").as_ref()), Some("HOLN"));
    assert_eq!(row.ticker(mic("XLON").as_ref()), Some("0QKY"));
    assert_eq!(instruments.listings(HOLCIM).len(), 2);
    assert_eq!(
        instruments
            .get_listing(HOLCIM, &Mic::new("XLON").unwrap())
            .and_then(Listing::ticker),
        Some("0QKY")
    );
    assert!(
        instruments
            .remove_listing(HOLCIM, &Mic::new("XLON").unwrap())
            .is_some()
    );
    assert_eq!(instruments.listings(HOLCIM).len(), 1);
}

/// No clock gates a merge: a stated valid value fills a fact the instrument
/// lacks and replaces one that differs whatever the time, the same value
/// moves nothing, a CFI refines or replaces, the ISIN's own prefix as a
/// country is held by no row, `updunix` is the later where a fact moved and
/// `firstunix`/`lastunix` the span.
#[test]
fn a_valid_statement_fills_and_replaces_whatever_the_time() {
    crate::install::installed();
    let mut instruments = Instruments::new();
    assert!(
        instruments
            .merge(
                security(HOLCIM, Some(10), &[(IdType::Common, "C-1")])
                    .try_with_cficode(Some(cfi("ESXXXX")))
                    .unwrap()
                    .with_firstunix(Some(10))
                    .with_lastunix(Some(10))
            )
            .unwrap()
    );
    // An older statement replaces.
    assert!(
        instruments
            .merge(
                security(HOLCIM, Some(5), &[(IdType::Common, "C-2")])
                    .try_with_cficode(Some(cfi("ESVUFR")))
                    .unwrap()
                    .with_firstunix(Some(5))
                    .with_lastunix(Some(5))
            )
            .unwrap()
    );
    let row = instruments.get(HOLCIM).unwrap();
    assert_eq!(row.get(&IdType::Common), Some("C-2"));
    assert_eq!(
        row.cficode().map(|code| code.as_str().to_owned()),
        Some("ESVUFR".into())
    );
    assert_eq!(
        (row.updunix(), row.firstunix(), row.lastunix()),
        (Some(10), Some(5), Some(10))
    );
    // The same value moves nothing.
    assert!(
        !instruments
            .merge(security(HOLCIM, Some(99), &[(IdType::Common, "C-2")]))
            .unwrap()
    );
    assert_eq!(instruments.get(HOLCIM).unwrap().updunix(), Some(10));
    // A compatible CFI refines, a contradicting one replaces.
    assert!(
        !instruments
            .merge(
                security(HOLCIM, None, &[])
                    .try_with_cficode(Some(cfi("ESXXXX")))
                    .unwrap()
            )
            .unwrap(),
        "an X contradicts nothing"
    );
    assert!(
        instruments
            .merge(
                security(HOLCIM, None, &[])
                    .try_with_cficode(Some(cfi("DBFTFR")))
                    .unwrap()
            )
            .unwrap()
    );
    assert_eq!(
        instruments
            .get(HOLCIM)
            .unwrap()
            .cficode()
            .map(|code| code.as_str().to_owned()),
        Some("DBFTFR".into())
    );
    // A stated country stands over the prefix; the prefix itself is held
    // by no row; an unlisted one is none.
    assert!(
        instruments
            .merge(security(HOLCIM, None, &[]).with_countrycode(Some(Country::new("LI").unwrap())))
            .unwrap()
    );
    assert_eq!(
        instruments
            .get(HOLCIM)
            .unwrap()
            .countrycode()
            .map(Country::as_str),
        Some("LI")
    );
    assert_eq!(
        instruments.get(HOLCIM).unwrap().country(),
        Some(Country::new("LI").unwrap())
    );
    assert!(
        instruments
            .merge(security(HOLCIM, None, &[]).with_countrycode(Some(Country::new("CH").unwrap())))
            .unwrap()
    );
    assert_eq!(instruments.get(HOLCIM).unwrap().countrycode(), None);
    assert_eq!(
        instruments.get(HOLCIM).unwrap().country(),
        Some(Country::new("CH").unwrap())
    );
    assert!(
        security(HOLCIM, None, &[])
            .with_countrycode(Some(Country::new("XX").unwrap()))
            .countrycode()
            .is_none()
    );
    // The origin currency is stated and never derived; a short name is a fact.
    assert!(
        instruments
            .merge(
                security(HOLCIM, None, &[])
                    .with_origccy(ccy("CHF"))
                    .with_fisn(fisn("HOLCIM/SH"))
            )
            .unwrap()
    );
    let row = instruments.get(HOLCIM).unwrap();
    assert_eq!(row.origccy().map(Ccy::as_str), Some("CHF"));
    assert_eq!(
        row.fisn().map(|name| name.as_str().to_owned()),
        Some("HOLCIM/SH".into())
    );
    assert_eq!(security(APPLE, None, &[]).origccy(), None);
    let mut filled = order(1, &[(IdType::Isin, HOLCIM)]);
    assert!(instruments.fill(&mut filled));
    assert_eq!(filled.get_origccy().as_str(), "CHF");
    assert_eq!(
        filled.get_securityids().get(&IdType::Fisn),
        Some("HOLCIM/SH")
    );
    // A product category and an underlying are instrument facts of a
    // security, merged by the same rule.
    assert!(
        instruments
            .merge(
                security(HOLCIM, None, &[])
                    .with_eusipacode(Some(yggdryl_market::Eusipa::new(1260).unwrap()))
                    .try_with_underlying(Some(APPLE))
                    .unwrap()
            )
            .unwrap()
    );
    let row = instruments.get(HOLCIM).unwrap();
    assert_eq!(row.eusipacode().map(|code| code.code()), Some(1260));
    assert_eq!(row.underlying(), Some(APPLE));
    assert_eq!(
        row.get_crosscode(),
        HOLCIM,
        "a security's underlying moves no key"
    );
    assert!(!row.is_placeholder());
}

/// The bound refuses a new instrument by name and a learn passes one over
/// with one warning; a known instrument keeps learning; a `ZZ` number names
/// no instrument.
#[test]
fn the_bound_refuses_a_new_instrument_and_learning_passes_one_over() {
    crate::install::installed();
    let mut instruments = Instruments::new().with_max_instruments(1);
    assert_eq!(instruments.max_instruments(), 1);
    assert_eq!(Instruments::DEFAULT_MAX_INSTRUMENTS, 16_384);
    assert_eq!(Instruments::new().max_instruments(), 16_384);
    instruments
        .merge(security(HOLCIM, Some(1), &[(IdType::Common, "A")]))
        .unwrap();
    let refused = instruments
        .merge(security(APPLE, Some(1), &[(IdType::Common, "B")]))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("at most 1 instruments"), "{refused}");
    assert!(!instruments.learn(&order(2, &[(IdType::Isin, APPLE), (IdType::Common, "B")])));
    assert!(
        instruments.learn(&order(2, &[(IdType::Isin, HOLCIM), (IdType::Common, "C")])),
        "a known instrument keeps learning"
    );
    assert_eq!(instruments.len(), 1);
    assert_eq!(instruments.rows(), 1);
    assert!(
        Instruments::new()
            .merge(security(HOLCIM, None, &[]))
            .is_ok()
    );
    assert!(Instrument::for_security(isin(&numbered("ZZ"))).is_err());
    // A loaded collection is clean; a full instrument passes a new type
    // over and stays clean; a held type restated moves.
    let mut full = Instrument::for_security(isin(HOLCIM)).unwrap();
    for kind in [
        IdType::Quik,
        IdType::Dutch,
        IdType::Sicovam,
        IdType::Belgian,
        IdType::Common,
        IdType::ClearingHouse,
        IdType::FpmlSpec,
        IdType::Opra,
        IdType::FpmlUrl,
        IdType::Loc,
        IdType::MktAssigned,
        IdType::RedEntity,
    ] {
        full.set_code(kind, "X-1").unwrap();
    }
    let mut seeded = Instruments::new();
    seeded.merge(full.with_updunix(Some(1))).unwrap();
    let mut holding = Instruments::from_arrow_reader(seeded.into_arrow_reader().unwrap()).unwrap();
    assert!(!holding.is_dirty(), "loaded");
    assert_eq!(
        holding.get(HOLCIM).unwrap().get(&IdType::RedEntity),
        Some("X-1")
    );
    assert!(
        !holding
            .merge(security(HOLCIM, Some(99), &[(IdType::RedPair, "X-1")]))
            .unwrap()
    );
    assert!(!holding.is_dirty(), "nothing moved");
    assert!(holding.learn(&order(
        99,
        &[(IdType::Isin, HOLCIM), (IdType::RedPair, "X-1")]
    )));
    let held = holding.get(HOLCIM).unwrap();
    assert_eq!((held.updunix(), held.lastunix()), (Some(1), Some(99)));
    assert_eq!(held.get(&IdType::RedPair), None);
    assert!(
        holding
            .merge(security(HOLCIM, Some(99), &[(IdType::Quik, "X-2")]))
            .unwrap()
    );
    assert_eq!(holding.get(HOLCIM).unwrap().get(&IdType::Quik), Some("X-2"));
    assert_eq!(holding.get(HOLCIM).unwrap().updunix(), Some(99));
    holding.clear();
    assert!(holding.is_empty());
    assert!(holding.is_dirty());
}

/// The cascade: a real ISIN decides alone, a miss ending it; then the code
/// the facts spell; then the minted number; then each lookup code in
/// order, one two instruments hold ending it; then the ticker on its
/// market; the listings of one instrument are one match.
#[test]
fn the_cascade_takes_the_isin_then_a_code_then_the_ticker_on_its_market() {
    crate::install::installed();
    let mut instruments = Instruments::new();
    instruments
        .merge(listed(APPLE, "XNAS", Some("AAPL")))
        .unwrap();
    instruments
        .merge(listed(DIAGEO, "XLON", Some("DGE")))
        .unwrap();
    instruments.merge(listed(SAP, "XETR", None)).unwrap();
    // The ISIN wins over a CUSIP naming Apple.
    let stated = order(1, &[(IdType::Isin, SAP), (IdType::Cusip, "037833100")]);
    let Resolution::Matched {
        entry,
        tier: MatchTier::Isin,
        derived: false,
        listing: true,
    } = instruments.resolve(&stated)
    else {
        panic!("{:?}", instruments.resolve(&stated))
    };
    assert_eq!(entry.get_crosscode(), SAP);
    // A CUSIP wins over a ticker naming Diageo.
    let mut coded = order(1, &[(IdType::Cusip, "037833100")]);
    coded.set_ticker(Some(SmolStr::new("DGE")), true);
    coded.set_miccode(mic("XLON"), true);
    let Resolution::Matched {
        entry,
        tier: MatchTier::Code(IdType::Cusip),
        derived: true,
        listing: false,
    } = instruments.resolve(&coded)
    else {
        panic!("{:?}", instruments.resolve(&coded))
    };
    assert_eq!(entry.get_crosscode(), APPLE, "Apple is not listed on XLON");
    // The ticker on its market last.
    let mut ticked = OrderEvent::at(1);
    ticked.set_ticker(Some(SmolStr::new("DGE")), true);
    ticked.set_miccode(mic("XLON"), true);
    let Resolution::Matched {
        entry,
        tier: MatchTier::Symbology,
        derived: true,
        listing: true,
    } = instruments.resolve(&ticked)
    else {
        panic!("{:?}", instruments.resolve(&ticked))
    };
    assert_eq!(entry.get_crosscode(), DIAGEO);
    // The same through the public lookups.
    assert_eq!(
        instruments
            .get_by_code(&IdType::Cusip, "037833100")
            .map(Element::get_crosscode),
        Some(APPLE)
    );
    assert_eq!(
        instruments
            .get_by_code(&IdType::Sedol, "0237400")
            .map(Element::get_crosscode),
        Some(DIAGEO),
        "the SEDOL its ISIN embeds"
    );
    assert_eq!(instruments.get_by_code(&IdType::Cusip, "not a cusip"), None);
    assert_eq!(instruments.get_by_code(&IdType::IsoCcy, "USD"), None);
    assert_eq!(
        instruments
            .get_by_ticker("DGE", mic("XLON").as_ref())
            .map(Element::get_crosscode),
        Some(DIAGEO)
    );
    assert_eq!(instruments.get_by_ticker("DGE", mic("XNAS").as_ref()), None);
    assert_eq!(
        instruments.get_by_ticker(" dge ", None),
        None,
        "exact, trimmed"
    );
    assert_eq!(
        instruments
            .get_by_ticker(" DGE ", None)
            .map(Element::get_crosscode),
        Some(DIAGEO)
    );
    // Nothing stated: no key; a ticker nobody holds: no candidate.
    assert_eq!(
        instruments.resolve(&OrderEvent::at(1)),
        Resolution::Unmatched(Unmatched::NoKey)
    );
    let mut unknown = OrderEvent::at(1);
    unknown.set_ticker(Some(SmolStr::new("ZZZZ")), true);
    assert_eq!(
        instruments.resolve(&unknown),
        Resolution::Unmatched(Unmatched::NoCandidate)
    );
    // A real ISIN the collection lacks ends the cascade and fills nothing.
    let mut stated = order(1, &[(IdType::Isin, NOVARTIS), (IdType::Cusip, "037833100")]);
    assert_eq!(
        instruments.resolve(&stated),
        Resolution::Unmatched(Unmatched::UnknownIsin {
            stated: isin(NOVARTIS)
        })
    );
    assert!(!instruments.fill(&mut stated));
    assert_eq!(stated.get_instcode(), None);
    // A code two instruments hold is ambiguous and stops the cascade.
    for text in [APPLE, SAP] {
        instruments
            .merge(security(text, Some(1), &[(IdType::Common, "C-1")]))
            .unwrap();
    }
    let mut common = order(1, &[(IdType::Common, "C-1")]);
    common.set_ticker(Some(SmolStr::new("DGE")), true);
    common.set_miccode(mic("XLON"), true);
    assert_eq!(
        instruments.resolve(&common),
        Resolution::Unmatched(Unmatched::Ambiguous {
            tier: MatchTier::Code(IdType::Common),
            codes: vec![SAP.into(), APPLE.into()],
        })
    );
    assert!(!instruments.fill(&mut common));
    assert_eq!(instruments.get_by_code(&IdType::Common, "C-1"), None);
    // The listings of one instrument are one match: a listing code on
    // either market names it.
    let mut hsbc = Instruments::new();
    hsbc.merge(
        listed("GB0005405286", "XLON", Some("HSBA"))
            .with_listing(Listing::new(mic("XHKG")).with_ticker(Some("5".into())))
            .unwrap(),
    )
    .unwrap();
    let mut by_sedol = order(1, &[(IdType::Sedol, "0540528")]);
    assert!(hsbc.fill(&mut by_sedol));
    assert_eq!(by_sedol.get_isincode(), Some("GB0005405286"));
    assert_eq!(
        by_sedol.get_ticker(),
        None,
        "two listings, no market stated"
    );
    let mut hong_kong = order(1, &[(IdType::Isin, "GB0005405286")]);
    hong_kong.set_miccode(mic("XHKG"), true);
    assert!(hsbc.fill(&mut hong_kong));
    assert_eq!(hong_kong.get_ticker(), Some("5"));
    assert_eq!(
        hong_kong.get_currency().as_str(),
        "HKD",
        "the market's legal tender"
    );
}

/// The economic tier matches a similar short name in the same currency,
/// weighed by `resolve` always and taken by a fill only where enabled.
#[test]
fn the_economic_tier_matches_a_similar_short_name_in_the_same_currency() {
    crate::install::installed();
    let usd = || Ccy::new("USD").unwrap();
    let named = |name: &str| {
        let mut element = order(1, &[(IdType::Fisn, name)]);
        element.set_currency(usd(), true);
        element
    };
    let instruments_of = |name: &str, code: &str| {
        let mut instruments = Instruments::new();
        instruments
            .merge(
                listed(APPLE, "XNAS", None)
                    .with_fisn(fisn(name))
                    .try_with_cficode(Some(cfi(code)))
                    .unwrap(),
            )
            .unwrap();
        instruments
    };
    let instruments = instruments_of("APPLE INC./SH", "ESVUFR");
    let Resolution::Matched {
        entry,
        tier: MatchTier::Economic { similarity },
        derived: true,
        listing: true,
    } = instruments.resolve(&named("APPLE INC/SH"))
    else {
        panic!("{:?}", instruments.resolve(&named("APPLE INC/SH")))
    };
    assert_eq!(entry.get_crosscode(), APPLE);
    assert!((similarity - 12.0 / 13.0).abs() < 1e-12, "{similarity}");
    assert!(similarity >= Instruments::DEFAULT_ECONOMIC_THRESHOLD);
    let plain = instruments_of("APPLE INC/SH", "ESVUFR");
    assert_eq!(
        plain.resolve(&named("APPLE INC/SH USD")),
        Resolution::Unmatched(Unmatched::BelowThreshold {
            best: 0.75,
            code: APPLE.into()
        })
    );
    let mut equity = named("APPLE INC/SH");
    equity.set_cficode(Some(cfi("ESVUFR")), true);
    assert_eq!(
        instruments_of("APPLE INC/SH", "DBFTFR").resolve(&equity),
        Resolution::Unmatched(Unmatched::CfiConflict {
            stated: 'E',
            held: 'D',
            code: APPLE.into()
        })
    );
    let mut euro = order(1, &[(IdType::Fisn, "APPLE INC/SH")]);
    euro.set_currency(Ccy::new("EUR").unwrap(), true);
    assert_eq!(
        plain.resolve(&euro),
        Resolution::Unmatched(Unmatched::NoCandidate)
    );
    let mut issued = Instruments::new();
    issued
        .merge(
            listed(APPLE, "XNAS", None)
                .with_fisn(fisn("APPLE INC/SH"))
                .with_origccy(ccy("USD")),
        )
        .unwrap();
    let mut other_origin = named("APPLE INC/SH");
    other_origin.set_origccy(Ccy::new("EUR").unwrap(), true);
    assert_eq!(
        issued.resolve(&other_origin),
        Resolution::Unmatched(Unmatched::CurrencyConflict {
            stated: Ccy::new("EUR").unwrap(),
            held: usd(),
            code: APPLE.into()
        })
    );
    let mut twins = instruments_of("APPLE INC/SH", "ESVUFR");
    let other = numbered("US");
    twins
        .merge(listed(&other, "XNYS", None).with_fisn(fisn("APPLE INC/SH")))
        .unwrap();
    assert_eq!(
        twins.resolve(&named("APPLE INC/SH")),
        Resolution::Unmatched(Unmatched::Ambiguous {
            tier: MatchTier::Economic { similarity: 1.0 },
            codes: vec![other.as_str().into(), APPLE.into()],
        })
    );
    // Taken by a fill only where enabled; the threshold refused by value.
    let mut instruments = instruments_of("APPLE INC./SH", "ESVUFR");
    assert!(!instruments.is_economic_match());
    assert_eq!(
        instruments.economic_threshold(),
        Instruments::DEFAULT_ECONOMIC_THRESHOLD
    );
    let mut off = named("APPLE INC/SH");
    assert!(
        !instruments.fill(&mut off),
        "a judgement no fill takes unasked"
    );
    assert_eq!(off.get_instcode(), None);
    instruments.set_economic_match(true);
    let mut on = named("APPLE INC/SH");
    assert!(instruments.fill(&mut on));
    assert_eq!(on.get_isincode(), Some(APPLE));
    assert_eq!(on.get_instcode(), Some(APPLE));
    assert!(on.get_securityids().is_derived(&IdType::Isin));
    let mut strict = instruments.try_with_economic_threshold(0.95).unwrap();
    assert!(!strict.fill(&mut named("APPLE INC/SH")));
    for refused in [0.0, 1.5, f64::NAN, -0.5] {
        let error = strict
            .set_economic_threshold(refused)
            .unwrap_err()
            .to_string();
        assert!(error.contains(&format!("{refused}")), "{refused}: {error}");
    }
    assert_eq!(strict.economic_threshold(), 0.95, "a refusal moves nothing");
    strict.clear();
    assert_eq!(
        (strict.economic_threshold(), strict.is_economic_match()),
        (0.95, true)
    );
    assert!(
        !Instruments::new()
            .with_economic_match(false)
            .is_economic_match()
    );
}

/// The collection round trips the record surface: every instrument as one
/// row in code order, read back equal and clean; a stream lacking the
/// `crosscode` column - the registry's row - is refused naming it; a stored
/// code the typed columns do not spell is refused at `$.crosscode`; a row a
/// column refuses is located on its row.
#[test]
fn the_collection_round_trips_the_record_surface_and_refuses_a_foreign_code() {
    crate::install::installed();
    let mut instruments = Instruments::new();
    instruments
        .merge(
            listed(APPLE, "XNAS", Some("AAPL"))
                .try_with_cficode(Some(cfi("ESVUFR")))
                .unwrap()
                .try_with_code(IdType::Ric, "AAPL.OQ")
                .unwrap()
                .with_fisn(fisn("APPLE INC/SH"))
                .with_updunix(Some(10))
                .with_firstunix(Some(1))
                .with_lastunix(Some(10)),
        )
        .unwrap();
    instruments.merge(fx("IFXXXP", "EUR/USD", &[])).unwrap();
    instruments
        .merge(
            Instrument::for_body(
                cfi("OCXXXX"),
                None,
                &derivative("2026-12-18", Some("200"))
                    .with_exercise(Some(Exercise::American))
                    .with_multiplier(decimal("100")),
                Some(APPLE),
                &[],
            )
            .unwrap()
            .with_listing(Listing::new(mic("XCBO")).with_ticker(Some("AAPL261218C200".into())))
            .unwrap(),
        )
        .unwrap();
    let codes: Vec<&str> = instruments.iter().map(Element::get_crosscode).collect();
    assert_eq!(
        codes,
        ["IF:EUR/USD", "OC:US0378331005:2026-12-18:200", APPLE],
        "code order"
    );
    let reader = instruments.into_arrow_reader().unwrap();
    assert_eq!(reader.schema().fields().len(), 25);
    let loaded = Instruments::from_arrow_reader(reader).unwrap();
    assert!(!loaded.is_dirty());
    assert_eq!(loaded.len(), 3);
    for held in instruments.iter() {
        let read = loaded
            .get(held.get_crosscode())
            .unwrap_or_else(|| panic!("{} loaded", held.get_crosscode()));
        assert_eq!(read, held, "{}", held.get_crosscode());
    }
    let option = loaded.get("OC:US0378331005:2026-12-18:200").unwrap();
    assert_eq!(
        option.characteristics().exercise(),
        Some(Exercise::American)
    );
    assert_eq!(option.characteristics().multiplier(), decimal("100"));
    assert_eq!(option.ticker(mic("XCBO").as_ref()), Some("AAPL261218C200"));
    assert_eq!(option.underlying_uuid(), Some(cross_of(APPLE).1));
    // A stored code the typed columns do not spell.
    let apple = instruments.get(APPLE).unwrap().into_scalar();
    let forged = apple.with_field("crosscode", Scalar::from(HOLCIM)).unwrap();
    let error = Instrument::from_scalar(&forged).unwrap_err().to_string();
    assert!(
        error.contains("$.crosscode") && error.contains(APPLE) && error.contains(HOLCIM),
        "{error}"
    );
    // A row written before the instrument's: no `crosscode` column.
    let schema = std::sync::Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
        "isin",
        arrow_schema::DataType::Utf8,
        true,
    )]));
    let batch = arrow_array::RecordBatch::try_new(
        std::sync::Arc::clone(&schema),
        vec![std::sync::Arc::new(arrow_array::StringArray::from(vec![
            Some(APPLE),
        ]))],
    )
    .unwrap();
    let error = Instruments::from_arrow_reader(yggdryl::arrow::batch_reader(schema, [batch]))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("$.crosscode") && error.contains("lay out afresh"),
        "{error}"
    );
    // A missing store's empty stream reads as nothing.
    let empty = std::sync::Arc::new(arrow_schema::Schema::empty());
    assert_eq!(
        Instruments::from_arrow_reader(yggdryl::arrow::batch_reader(empty, []))
            .unwrap()
            .len(),
        0
    );
    // A seeded collection is clean, unbound, and new holds none of it.
    let seeded = Instruments::seeded();
    assert!(!seeded.is_dirty());
    assert!(seeded.len() > 200);
    assert!(seeded.get(APPLE).is_some());
    assert!(Instruments::new().is_empty());
}

/// Decision 10: an instrument keeps every source's identifiers under their
/// own `src:type` keys - carried by the learn path from a statement, kept by
/// a merge and a reload, the base key filled by the map's rule - and holds
/// the complementary facts no typed field has in its `metadata`: stated,
/// replaced by a differing value, kept by an equal one, fed to the content
/// code and never the key, every bound named, stored as the twenty-fifth
/// column and read back, a golden file's column no field reads landing
/// there under its name.
#[test]
fn sourced_identifiers_and_metadata_are_learned_stored_and_read_back() {
    crate::install::installed();
    use std::sync::Arc;
    use yggdryl_market::IdSource;
    let key = |src: &str, kind: IdType| IdKey::new(src.parse::<IdSource>().unwrap(), kind);
    let sourced =
        |src: &str, kind: IdType, value: &str| Identifier::new(key(src, kind), value).unwrap();
    let xnas = Mic::new("XNAS").unwrap();

    // A bridge's statement: the ISIN under the bridge's own source, a FIGI
    // under Bloomberg's, an LEI under the base key.
    let mut stated = order(1, &[(IdType::Lei, "HWUPKR0MPOU8FGXBT394")]);
    stated
        .insert_securityid(sourced("ullink", IdType::Isin, APPLE))
        .unwrap();
    stated
        .insert_securityid(sourced("bloomberg", IdType::Figi, "BBG000B9XRY4"))
        .unwrap();
    stated.set_miccode(Some(xnas.clone()), true);
    stated.set_cficode(Some(cfi("ESVUFR")), true);
    let mut instruments = Instruments::new();
    assert!(instruments.learn(&stated));
    let apple = instruments
        .get(APPLE)
        .expect("keyed by the ISIN the source filled");
    assert_eq!(
        apple.securityids().get_from(&key("ullink", IdType::Isin)),
        Some(APPLE),
        "the source's own key"
    );
    assert_eq!(apple.isin(), Some(APPLE));
    assert_eq!(apple.get(&IdType::Lei), Some("HWUPKR0MPOU8FGXBT394"));
    let listing = apple.listing(Some(&xnas)).expect("the stated market");
    assert_eq!(
        listing.codes().get_from(&key("bloomberg", IdType::Figi)),
        Some("BBG000B9XRY4"),
        "a listing code under its source, on the listing"
    );
    assert_eq!(apple.get(&IdType::Figi), Some("BBG000B9XRY4"));
    assert!(
        !instruments.learn(&stated),
        "the same statement moves nothing"
    );

    // Complementary facts: stated, kept while equal, replaced by a
    // differing value, a key left unsaid standing.
    let described = Instrument::for_security(isin(APPLE))
        .unwrap()
        .try_with_metadata("issuer", "Apple Inc.")
        .unwrap()
        .try_with_metadata("securitydesc", "APPLE INC COMMON STOCK")
        .unwrap();
    assert!(instruments.merge(described.clone()).unwrap());
    let meta = |instruments: &Instruments, key: &str| {
        instruments
            .get(APPLE)
            .unwrap()
            .metadata()
            .get(key)
            .map(SmolStr::as_str)
            .map(str::to_owned)
    };
    assert_eq!(meta(&instruments, "issuer").as_deref(), Some("Apple Inc."));
    assert!(
        !instruments.merge(described).unwrap(),
        "an equal value moves nothing"
    );
    assert!(
        instruments
            .merge(
                Instrument::for_security(isin(APPLE))
                    .unwrap()
                    .try_with_metadata("issuer", "APPLE INC")
                    .unwrap()
            )
            .unwrap()
    );
    assert_eq!(meta(&instruments, "issuer").as_deref(), Some("APPLE INC"));
    assert_eq!(
        meta(&instruments, "securitydesc").as_deref(),
        Some("APPLE INC COMMON STOCK"),
        "a key the statement leaves unsaid stands"
    );

    // Fed to the content code, never to the key or the identity.
    let bare = Instrument::for_security(isin(APPLE)).unwrap();
    let noted = bare
        .clone()
        .try_with_metadata("issuer", "APPLE INC")
        .unwrap();
    assert_ne!(bare.get_hashcode(), noted.get_hashcode());
    assert_eq!(
        (bare.get_crosscode(), bare.get_crossuuid(), bare.get_uuid()),
        (
            noted.get_crosscode(),
            noted.get_crossuuid(),
            noted.get_uuid()
        )
    );

    // Every bound by name; a learn past the bound passes the key over.
    let mut full = bare.clone();
    for at in 0..Instrument::MAX_METADATA {
        full.set_metadata(&format!("key{at}"), "value").unwrap();
    }
    let error = full
        .set_metadata("one-more", "value")
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("$.metadata") && error.contains("one-more"),
        "{error}"
    );
    full.set_metadata("key0", "replaced").unwrap();
    assert_eq!(full.remove_metadata("key0").as_deref(), Some("replaced"));
    assert_eq!(full.metadata().len(), Instrument::MAX_METADATA - 1);
    let wide = "v".repeat(Instrument::MAX_METADATA_WIDTH + 1);
    for (key, value) in [("", "value"), ("key", ""), ("key", wide.as_str())] {
        let error = bare
            .clone()
            .try_with_metadata(key, value)
            .unwrap_err()
            .to_string();
        assert!(error.contains("$.metadata"), "{key:?}={value:?}: {error}");
    }
    full.set_metadata("key0", "value").unwrap();
    let mut crowded = Instruments::new();
    crowded.merge(full).unwrap();
    assert!(
        !crowded
            .merge(bare.clone().try_with_metadata("extra", "value").unwrap())
            .unwrap(),
        "a key past the bound is passed over, the row kept"
    );
    assert_eq!(
        crowded.get(APPLE).unwrap().metadata().len(),
        Instrument::MAX_METADATA
    );

    // The record surface: the twenty-fifth column, read back whole.
    let reader = instruments.into_arrow_reader().unwrap();
    let schema = reader.schema();
    assert_eq!(schema.fields().len(), 25);
    assert_eq!(schema.field(21).name(), "metadata");
    let loaded = Instruments::from_arrow_reader(reader).unwrap();
    assert_eq!(loaded.get(APPLE), instruments.get(APPLE));
    assert_eq!(
        loaded
            .get(APPLE)
            .unwrap()
            .securityids()
            .get_from(&key("ullink", IdType::Isin)),
        Some(APPLE),
        "a sourced key survives a reload"
    );

    // A golden file's columns no field reads land in the metadata under
    // their names: a text cell as it is, any other as its JSON text.
    let batch = instruments
        .into_arrow_reader()
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    let mut fields: Vec<Arc<arrow_schema::Field>> =
        batch.schema().fields().iter().cloned().collect();
    let mut columns = batch.columns().to_vec();
    fields.push(Arc::new(arrow_schema::Field::new(
        "vendor",
        arrow_schema::DataType::Utf8,
        true,
    )));
    columns.push(Arc::new(arrow_array::StringArray::from(vec![Some(
        "ullink",
    )])));
    fields.push(Arc::new(arrow_schema::Field::new(
        "rank",
        arrow_schema::DataType::Int32,
        true,
    )));
    columns.push(Arc::new(arrow_array::Int32Array::from(vec![Some(7)])));
    let schema = Arc::new(arrow_schema::Schema::new(fields));
    let golden = arrow_array::RecordBatch::try_new(Arc::clone(&schema), columns).unwrap();
    let read =
        Instruments::from_arrow_reader(yggdryl::arrow::batch_reader(schema, [golden])).unwrap();
    let apple = read.get(APPLE).unwrap();
    assert_eq!(
        apple.metadata().get("vendor").map(SmolStr::as_str),
        Some("ullink")
    );
    assert_eq!(apple.metadata().get("rank").map(SmolStr::as_str), Some("7"));
    assert_eq!(
        apple.metadata().get("issuer").map(SmolStr::as_str),
        Some("APPLE INC"),
        "the stored map beside them"
    );
    assert_eq!(
        apple.securityids().get_from(&key("ullink", IdType::Isin)),
        Some(APPLE)
    );
}

/// Decision 11: the instrument holds its code once, as the crate's `Str`,
/// and every `instcode` a fill writes is a clone of it - for a code past the
/// inline capacity one shared allocation, which the two pointers being one
/// proves; a code within it is copied, the same text.
#[test]
fn a_filled_instcode_shares_the_instruments_one_allocation_of_its_code() {
    crate::install::installed();
    let code = "OC:US0378331005:2026-12-18:200";
    assert!(code.len() > yggdryl::INLINE_CAPACITY);
    let mut instruments = Instruments::new();
    instruments
        .merge(
            Instrument::for_body(
                cfi("OCXXXX"),
                None,
                &derivative("2026-12-18", Some("200")),
                Some(APPLE),
                &[],
            )
            .unwrap(),
        )
        .unwrap();
    let option = instruments.get(code).unwrap();
    let number = option.isin().unwrap().to_owned();
    let mut named = order(1, &[(IdType::Isin, &number)]);
    assert!(instruments.fill(&mut named));
    let filled = named.get_instcode().unwrap();
    assert_eq!(filled, code);
    assert_eq!(
        filled.as_ptr(),
        option.get_crosscode().as_ptr(),
        "the instrument's own allocation"
    );
    let mut later = order(2, &[(IdType::Isin, &number)]);
    assert!(instruments.fill(&mut later));
    assert_eq!(later.get_instcode().unwrap().as_ptr(), filled.as_ptr());
    instruments
        .merge(listed(APPLE, "XNAS", Some("AAPL")))
        .unwrap();
    let mut short = order(3, &[(IdType::Isin, APPLE)]);
    assert!(instruments.fill(&mut short));
    assert_eq!(short.get_instcode(), Some(APPLE));
}

/// A statement is derived: a row stating only its facts - no element column,
/// no `placeholder` - reads as the instrument those facts spell, keyed,
/// minted and digested as a built one is, under its struct or as the ordered
/// row; a stated code the columns do not spell is still refused.
#[test]
fn a_partial_statement_reads_as_the_instrument_its_facts_spell() {
    crate::install::installed();
    let built = listed(APPLE, "XNAS", Some("AAPL"))
        .try_with_cficode(Some(cfi("ESVUFR")))
        .unwrap();
    let listing = Scalar::from_struct([
        ("miccode", Scalar::from("XNAS")),
        ("ticker", Scalar::from("AAPL")),
    ])
    .unwrap();
    let stated = Scalar::from_struct([
        ("isin", Scalar::from(APPLE)),
        ("cficode", Scalar::from("ESVUFR")),
        ("listings", Scalar::from_sequence([listing])),
    ])
    .unwrap();
    let read = Instrument::from_scalar(&stated).unwrap();
    assert_eq!(
        read, built,
        "the key, the identity and the content code derived"
    );
    assert_eq!(read.get_crosscode(), APPLE);
    assert_eq!(read.get_uuid(), cross_of(APPLE).1);
    assert_eq!(read.get_hashcode(), built.get_hashcode());
    assert!(!read.is_placeholder());
    // A body: its code written, its number minted.
    let pair = Scalar::from_struct([
        ("cficode", Scalar::from("IFXXXP")),
        ("forexcode", Scalar::from("EUR/USD")),
    ])
    .unwrap();
    let read = Instrument::from_scalar(&pair).unwrap();
    assert_eq!(read, fx("IFXXXP", "EUR/USD", &[]));
    assert_eq!(read.minted_isin(), Some("QYLTVIRYHNX5"));
    // The ordered row, its element cells null.
    let field = Instrument::field();
    let at = |name: &str| {
        field
            .fields()
            .iter()
            .position(|child| child.name() == name)
            .unwrap()
    };
    let mut cells = vec![Scalar::Null; field.field_len()];
    cells[at("isin")] = Scalar::from(APPLE);
    let read = Instrument::from_scalar(&Scalar::from_sequence(cells)).unwrap();
    assert_eq!(read, Instrument::for_security(isin(APPLE)).unwrap());
    // Stating nothing the key is written from is still refused there, and
    // so is a code the columns do not spell.
    let error = Instrument::from_scalar(
        &Scalar::from_struct([("countrycode", Scalar::from("US"))]).unwrap(),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("$.crosscode"), "{error}");
    let forged = Scalar::from_struct([
        ("isin", Scalar::from(APPLE)),
        ("crosscode", Scalar::from(HOLCIM)),
    ])
    .unwrap();
    let error = Instrument::from_scalar(&forged).unwrap_err().to_string();
    assert!(
        error.contains("$.crosscode") && error.contains(HOLCIM),
        "{error}"
    );
    // A name no column has is refused by name.
    let foreign = Scalar::from_struct([
        ("isin", Scalar::from(APPLE)),
        ("ric", Scalar::from("AAPL.OQ")),
    ])
    .unwrap();
    let error = Instrument::from_scalar(&foreign).unwrap_err().to_string();
    assert!(error.contains("ric"), "{error}");
}

/// A listing code has one holder at any instant: stated on no market while
/// the instrument has several listings it is held among the identifiers, and
/// stated on its market it moves onto that listing and leaves them; a source's
/// own key of the type moves with the base key when a listing is created.
#[test]
fn a_listing_code_held_on_no_market_has_one_holder_once_its_market_is_stated() {
    crate::install::installed();
    let mut instruments = Instruments::new();
    instruments
        .merge(listed(HOLCIM, "XSWX", Some("HOLN")))
        .unwrap();
    instruments
        .merge(listed(HOLCIM, "XLON", Some("0QKY")))
        .unwrap();
    assert_eq!(instruments.get(HOLCIM).unwrap().listings().len(), 2);
    let unplaced = order(1, &[(IdType::Isin, HOLCIM), (IdType::Ric, "HOLN.S")]);
    assert!(instruments.learn(&unplaced));
    let held = instruments.get(HOLCIM).unwrap();
    assert_eq!(
        held.securityids().get(&IdType::Ric),
        Some("HOLN.S"),
        "two listings and no market: held among the identifiers"
    );
    assert!(
        held.listings()
            .iter()
            .all(|listing| listing.get(&IdType::Ric).is_none())
    );
    let mut placed = order(2, &[(IdType::Isin, HOLCIM), (IdType::Ric, "HOLN.S")]);
    placed.set_miccode(mic("XSWX"), true);
    assert!(instruments.learn(&placed));
    let held = instruments.get(HOLCIM).unwrap();
    let swiss = held.listing(mic("XSWX").as_ref()).unwrap();
    assert_eq!(
        swiss.get(&IdType::Ric),
        Some("HOLN.S"),
        "moved onto its market"
    );
    assert_eq!(
        held.securityids().get(&IdType::Ric),
        None,
        "one holder at any instant"
    );
    assert_eq!(held.get(&IdType::Ric), Some("HOLN.S"));
    assert_eq!(
        instruments
            .get_by_code(&IdType::Ric, "HOLN.S")
            .map(Element::get_crosscode),
        Some(HOLCIM)
    );
    assert!(
        !instruments.learn(&placed),
        "stated again where it is held, nothing moves"
    );
    // A source's statement of a listing type on no market, then the market:
    // the listing created takes the source's key with the base key.
    instruments.merge(fx("IFXXXP", "EUR/USD", &[])).unwrap();
    let sourced: IdKey = "ullink:ric".parse().unwrap();
    let mut quoted = order(3, &[(IdType::Forex, "EUR/USD")]);
    quoted
        .insert_securityid(Identifier::new(sourced.clone(), "EUR=").unwrap())
        .unwrap();
    quoted.set_cficode(Some(cfi("IFXXXP")), true);
    assert!(instruments.learn(&quoted));
    let spot = instruments.get("IF:EUR/USD").unwrap();
    assert_eq!(spot.securityids().get_from(&sourced), Some("EUR="));
    assert_eq!(spot.securityids().get(&IdType::Ric), Some("EUR="));
    let mut venued = order(4, &[(IdType::Forex, "EUR/USD")]);
    venued.set_cficode(Some(cfi("IFXXXP")), true);
    venued.set_miccode(mic("XOFF"), true);
    assert!(instruments.learn(&venued));
    let spot = instruments.get("IF:EUR/USD").unwrap();
    let venue = spot.listing(mic("XOFF").as_ref()).unwrap();
    assert_eq!(
        venue.codes().get_from(&sourced),
        Some("EUR="),
        "the source's key moved too"
    );
    assert_eq!(venue.get(&IdType::Ric), Some("EUR="));
    assert_eq!(spot.securityids().get(&IdType::Ric), None);
    assert_eq!(spot.securityids().get_from(&sourced), None);
}

/// The named struct a row answers nests named records - each listing, each
/// leg, the characteristics the struct of its own cells - what a caller reads
/// by name in every language; the snapshot streams the ordered runs, and both
/// read back as the instrument.
#[test]
fn the_named_row_nests_named_records_and_the_snapshot_ordered_runs() {
    crate::install::installed();
    let option = Instrument::for_body(
        cfi("OCXXXX"),
        None,
        &derivative("2026-12-18", Some("200")).with_exercise(Some(Exercise::American)),
        Some(APPLE),
        &[],
    )
    .unwrap()
    .with_listing(
        Listing::new(mic("XCBO"))
            .with_ticker(Some("AAPL261218C200".into()))
            .try_with_code(IdType::Ric, "AAPL261218C200.U")
            .unwrap(),
    )
    .unwrap();
    let spread = Instrument::for_body(
        cfi("KEXXXX"),
        None,
        &Characteristics::default(),
        None,
        &[Leg::new(APPLE, 1).unwrap(), Leg::new(HOLCIM, 2).unwrap()],
    )
    .unwrap();
    let row = option.into_scalar();
    let cells = row.as_struct().unwrap();
    let listings = cells["listings"].sequence_rows().unwrap();
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0], option.listings()[0].into_scalar());
    assert_eq!(
        listings[0].as_struct().unwrap()["ticker"],
        Scalar::from("AAPL261218C200"),
        "a listing by name"
    );
    assert_eq!(
        cells["characteristics"],
        option.characteristics().into_scalar()
    );
    assert_eq!(
        cells["characteristics"].as_struct().unwrap()["expiry"],
        Scalar::from("2026-12-18"),
        "a characteristic by name"
    );
    let legs = spread.into_scalar().as_struct().unwrap()["legs"]
        .sequence_rows()
        .unwrap()
        .to_vec();
    assert_eq!(
        legs,
        spread
            .legs()
            .iter()
            .map(Leg::into_scalar)
            .collect::<Vec<_>>()
    );
    // The legs sort bytewise by code, as the key writes them.
    assert_eq!(
        legs[0].as_struct().unwrap()["code"],
        Scalar::from(HOLCIM),
        "a leg by name"
    );
    assert_eq!(legs[0].as_struct().unwrap()["ratio"], Scalar::from(2_i32));
    assert_eq!(legs[1].as_struct().unwrap()["code"], Scalar::from(APPLE));
    for held in [&option, &spread] {
        assert_eq!(&Instrument::from_scalar(&held.into_scalar()).unwrap(), held);
    }
    // The snapshot keeps the ordered runs the row-to-batch reader lays out.
    let mut instruments = Instruments::new();
    instruments
        .merge(listed(APPLE, "XNAS", Some("AAPL")))
        .unwrap();
    instruments
        .merge(listed(HOLCIM, "XSWX", Some("HOLN")))
        .unwrap();
    instruments.merge(option.clone()).unwrap();
    instruments.merge(spread.clone()).unwrap();
    let back = Instruments::from_arrow_reader(instruments.into_arrow_reader().unwrap()).unwrap();
    assert!(back.iter().eq(instruments.iter()));
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::graph::Element;
    use yggdryl::internals::logging_warning::count;
    use yggdryl::{Isin, Scalar};
    use yggdryl_market::{Instrument, Instruments};

    use super::{APPLE, cfi, derivative, fx};

    const SITE: &str = "yggdryl_market::instrument";
    const DROPPED: &str = "instrument value dropped: it is no real code of its type";

    /// A body instrument's row - an FX pair's, an option's - merges back
    /// holding the number this crate minted for it under `yggdryl:isin`: the
    /// instrument's own derivation, minted again where the statement lands
    /// and warned of nowhere; a `QY` number of another system under the key
    /// is dropped by name as before.
    #[test]
    fn a_body_rows_own_minted_number_is_merged_back_without_a_warning() {
        crate::install::installed();
        let before = count(SITE, DROPPED, "isin");
        let mut instruments = Instruments::new();
        let spot = fx("IFXXXP", "EUR/USD", &[]);
        let option = Instrument::for_body(
            cfi("OCXXXX"),
            None,
            &derivative("2026-12-18", Some("200")),
            Some(APPLE),
            &[],
        )
        .unwrap();
        for held in [&spot, &option] {
            let row = Instrument::from_scalar(&held.into_scalar()).unwrap();
            assert!(
                row.minted_isin()
                    .is_some_and(|number| held.is_own_mint(&Isin::new(number).unwrap()))
            );
            assert!(instruments.merge(row).unwrap());
            let learned = instruments.get(held.get_crosscode()).unwrap();
            assert_eq!(learned.minted_isin(), held.minted_isin());
            assert!(
                !instruments
                    .merge(Instrument::from_scalar(&learned.into_scalar()).unwrap())
                    .unwrap(),
                "its own row stated again moves nothing"
            );
        }
        assert_eq!(
            count(SITE, DROPPED, "isin"),
            before,
            "the instrument's own number is its derivation, not a dropped value"
        );
        // Another system's number under the crate's key.
        let foreign = spot
            .into_scalar()
            .with_field(
                "securityids",
                Scalar::from_mapping([
                    (Scalar::from("cfi"), Scalar::from("IFXXXP")),
                    (Scalar::from("forex"), Scalar::from("EUR/USD")),
                    (Scalar::from("yggdryl:isin"), Scalar::from("QY0000000000")),
                ])
                .unwrap(),
            )
            .unwrap();
        let foreign = Instrument::from_scalar(&foreign).unwrap();
        assert_eq!(foreign.minted_isin(), Some("QY0000000000"));
        assert!(
            !instruments.merge(foreign).unwrap(),
            "nothing real to learn"
        );
        assert_eq!(
            count(SITE, DROPPED, "isin"),
            before + 1,
            "a number nobody here minted is dropped by name"
        );
    }
}
