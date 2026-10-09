//! The register's whole listing as the one member linking every crate sees
//! it, the market crate then the FIX crate installed as the command's
//! `main` installs them: the core's seventeen registered codes, its `state`
//! and the four enum kinds `yggdryl-market` claims keep the bytes, the
//! places in `DataTypeId`'s listing and the order among the datatypes the
//! core reserves for them.

use yggdryl::DataTypeId;

/// Claims what the command links, as its `main` does: the market crate's
/// kinds, then the FIX crate's names.
fn installed() {
    yggdryl_market::install().expect("yggdryl-market claims its kinds");
    yggdryl_fix::install().expect("yggdryl-fix claims its names");
}

/// The market kinds and `State` as the listing states them:
/// the code family's seventeen in byte order after `mediatype` and before
/// `uuid`, the enum family's five after the text family's last. This is the
/// order `DATA_TYPE_IDS` and `dataTypeIds` list, and what `all()` keeps
/// once a kind's byte is claimed rather than declared.
#[test]
fn the_market_kinds_keep_their_places_in_the_listing() {
    installed();
    // Every byte read through `from_u8`, the door that survives the kinds'
    // bytes being claimed rather than declared: the identifiers it answers,
    // in byte order, are the listing.
    let names: Vec<&str> = (0..=u8::MAX)
        .filter_map(DataTypeId::from_u8)
        .map(DataTypeId::as_str)
        .collect();
    let codes = [
        "lei", "bic", "elf", "dti", "fisn", "country", "ccy", "mic", "cfi", "isin", "cusip",
        "sedol", "bbg", "figi", "unit", "ric", "forex",
    ];
    let start = names.iter().position(|name| *name == "lei").unwrap();
    assert_eq!(&names[start..start + codes.len()], &codes);
    assert_eq!(names[start - 1], "mediatype");
    assert_eq!(names[start + codes.len()], "uuid");
    let enums = [
        "state",
        "marketdatakind",
        "side",
        "marketdatatype",
        "timeinforce",
    ];
    let start = names.iter().position(|name| *name == "state").unwrap();
    assert_eq!(&names[start..start + enums.len()], &enums);
    for (name, byte) in [
        ("lei", 0x6b),
        ("bic", 0x6c),
        ("elf", 0x6d),
        ("dti", 0x6e),
        ("fisn", 0x6f),
        ("country", 0x71),
        ("ccy", 0x72),
        ("mic", 0x73),
        ("cfi", 0x74),
        ("isin", 0x78),
        ("cusip", 0x79),
        ("sedol", 0x7a),
        ("bbg", 0x7b),
        ("figi", 0x7c),
        ("unit", 0x7d),
        ("ric", 0x7e),
        ("forex", 0x7f),
        ("state", 0xc1),
        ("marketdatakind", 0xc2),
        ("side", 0xc3),
        ("marketdatatype", 0xc4),
        ("timeinforce", 0xc5),
    ] {
        let id = DataTypeId::from_u8(byte).unwrap_or_else(|| panic!("{name} at {byte:#04x}"));
        assert_eq!(id.as_str(), name, "{byte:#04x}");
        assert_eq!(id.as_u8(), byte, "{name}");
    }
}

/// The order `DataType` sorts the market kinds and `State` in, among the
/// core leaves that sit between them: a third per-kind number beside the
/// identifier byte and the `Shape` position, neither of which it is - by
/// byte `lei` would sort before `country`. Caller-visible through both
/// bindings' comparisons, so it never moves; the numbers are the ranks
/// `datatype.rs` states.
#[test]
fn the_market_kinds_and_state_keep_their_order_among_the_core_datatypes() {
    installed();
    let ascending = [
        ("country", 30),
        ("ccy", 31),
        ("mic", 32),
        ("cfi", 33),
        ("uuid", 34),
        ("geography", 52),
        ("side", 53),
        ("state", 55),
        ("timeinforce", 56),
        ("url", 57),
        ("isin", 58),
        ("timezone", 59),
        ("mimetype", 60),
        ("mediatype", 61),
        ("cusip", 62),
        ("sedol", 63),
        ("bbg", 64),
        ("urn", 65),
        ("figi", 66),
        ("unit", 68),
        ("decimal", 69),
        ("bigdecimal", 70),
        ("ric", 71),
        ("forex", 72),
        ("marketdatakind", 73),
        ("marketdatatype", 74),
        ("lei", 75),
        ("bic", 76),
        ("elf", 77),
        ("dti", 78),
        ("fisn", 79),
    ];
    let parsed: Vec<yggdryl::DataType> = ascending
        .iter()
        .map(|(name, _)| {
            yggdryl::DataType::from_str(name).unwrap_or_else(|error| panic!("{name}: {error}"))
        })
        .collect();
    for (pair, ranks) in parsed.windows(2).zip(ascending.windows(2)) {
        assert!(ranks[0].1 < ranks[1].1);
        assert!(pair[0] < pair[1], "{:?} !< {:?}", pair[0], pair[1]);
    }
    // The order is the datatype's, not its byte's: `lei` (0x6b) sorts after
    // `country` (0x71) and after every code the market declared before it.
    let lei = yggdryl::DataType::from_str("lei").unwrap();
    let country = yggdryl::DataType::from_str("country").unwrap();
    assert!(lei.id().as_u8() < country.id().as_u8() && country < lei);
}
