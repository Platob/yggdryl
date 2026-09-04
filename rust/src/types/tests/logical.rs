use super::super::DataType;
use super::{TimeUnit, UnionMode};
use crate::Timezone;

/// The whole registry, as the module documents it. A change to a mapping
/// changes what a stored schema string means, so it changes here first.
fn registered() -> Vec<(&'static str, DataType)> {
    let decimal64 = DataType::decimal64(18, 8).unwrap();
    vec![
        ("currency", DataType::Currency),
        ("country", DataType::Country),
        ("mic", DataType::Mic),
        ("exchange", DataType::Mic),
        ("cfi", DataType::Cfi),
        ("language", DataType::FixedAscii(2)),
        ("monthyear", DataType::FixedAscii(8)),
        ("tenor", DataType::FixedAscii(8)),
        ("pattern", DataType::Utf8),
        ("length", DataType::Int32),
        ("tagnum", DataType::Int32),
        ("seqnum", DataType::Int64),
        ("numingroup", DataType::Int32),
        ("dayofmonth", DataType::Int8),
        ("reserved100plus", DataType::Int32),
        ("reserved1000plus", DataType::Int32),
        ("reserved4000plus", DataType::Int32),
        ("qty", decimal64.clone()),
        ("price", decimal64.clone()),
        ("priceoffset", decimal64.clone()),
        ("percentage", decimal64),
        ("amt", DataType::decimal128(38, 8).unwrap()),
        (
            "utctimestamp",
            DataType::Timestamp(TimeUnit::Nanosecond, Some(Timezone::UTC)),
        ),
        (
            "tztimestamp",
            DataType::Timestamp(TimeUnit::Nanosecond, Some(Timezone::UTC)),
        ),
        ("utctimeonly", DataType::Time64(TimeUnit::Nanosecond)),
        ("localmkttime", DataType::Time32(TimeUnit::Second)),
        ("utcdateonly", DataType::Date32),
        ("localmktdate", DataType::Date32),
        ("tztimeonly", DataType::FixedAscii(16)),
        ("multiplecharvalue", DataType::Utf8),
        ("multiplestringvalue", DataType::Utf8),
        ("xid", DataType::Utf8),
        ("xidref", DataType::Utf8),
        ("data", DataType::Binary),
        ("xmldata", DataType::Binary),
    ]
}

#[test]
fn the_registry_is_the_documented_mapping_and_holds_no_repeat() {
    assert_eq!(DataType::LOGICAL_NAMES, registered().as_slice());
    let mut names: Vec<&str> = DataType::LOGICAL_NAMES
        .iter()
        .map(|(name, _)| *name)
        .collect();
    let registered = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), registered, "a name is registered twice");
    // Every stored name is already folded, so a lookup finds it verbatim.
    for name in names {
        assert!(
            name.bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit()),
            "{name} is not stored folded"
        );
        assert!(DataType::from_logical_name(name).is_ok(), "{name}");
    }
}

#[test]
fn a_name_folds_case_separators_and_surrounding_space() {
    for spelling in [
        "UTCTimestamp",
        "utctimestamp",
        " UTC_Timestamp ",
        "utc-timestamp",
        "UTC Timestamp",
    ] {
        assert_eq!(
            DataType::from_logical_name(spelling).unwrap(),
            DataType::Timestamp(TimeUnit::Nanosecond, Some(Timezone::UTC)),
            "{spelling}"
        );
    }
}

#[test]
fn the_grammar_resolves_a_name_and_displays_the_datatype_it_named() {
    for (spelling, dtype) in [
        ("Price", DataType::decimal64(18, 8).unwrap()),
        ("Amt", DataType::decimal128(38, 8).unwrap()),
        ("SeqNum", DataType::Int64),
        ("DayOfMonth", DataType::Int8),
        ("LocalMktDate", DataType::Date32),
        ("LocalMktTime", DataType::Time32(TimeUnit::Second)),
        ("UTCTimeOnly", DataType::Time64(TimeUnit::Nanosecond)),
        ("XMLData", DataType::Binary),
        ("data", DataType::Binary),
        ("Tenor", DataType::FixedAscii(8)),
    ] {
        let parsed: DataType = spelling.parse().unwrap();
        assert_eq!(parsed, dtype, "{spelling}");
        // One canonical spelling: a name displays as what it resolved to.
        assert_eq!(parsed.to_string(), dtype.to_string(), "{spelling}");
        assert_eq!(parsed.to_string().parse::<DataType>().unwrap(), parsed);
    }

    // A name types a column wherever a datatype is accepted, and a
    // postfix list still applies to it.
    let row: DataType = "struct<ccy: Currency, px: Price, legs: Qty[]>"
        .parse()
        .unwrap();
    assert_eq!(
        row.get_field_by_path("px")
            .map(|field| field.dtype().clone()),
        Some(DataType::decimal64(18, 8).unwrap())
    );
    assert_eq!(
        row.get_field_by_path("legs.item")
            .map(|field| field.dtype().clone()),
        Some(DataType::decimal64(18, 8).unwrap())
    );
}

/// The five FIX base types the Arrow/SQL grammar already owns keep their
/// meaning: a stored schema string never changes what it means.
#[test]
fn the_shared_base_type_spellings_keep_their_grammar_meaning() {
    for (spelling, dtype) in [
        ("int", DataType::Int32),
        ("float", DataType::Float32),
        ("char", DataType::Utf8),
        ("String", DataType::Utf8),
        ("Boolean", DataType::Boolean),
    ] {
        assert_eq!(spelling.parse::<DataType>().unwrap(), dtype, "{spelling}");
        assert!(
            !DataType::LOGICAL_NAMES
                .iter()
                .any(|(name, _)| *name == spelling.to_ascii_lowercase()),
            "{spelling} must not be registered"
        );
    }
}

#[test]
fn an_unregistered_name_is_refused_by_both_entry_points() {
    let error = DataType::from_logical_name("isin").unwrap_err().to_string();
    assert!(error.contains("currency"), "{error}");
    assert!(error.contains("\"isin\""), "{error}");
    // The grammar reports an unregistered word as unknown.
    let error = "isin".parse::<DataType>().unwrap_err().to_string();
    assert!(error.contains("unknown datatype \"isin\""), "{error}");
}

/// A registered name is inert everywhere but the grammar: it adds no
/// variant, so identity, family, and union type ids are untouched.
#[test]
fn a_name_adds_no_datatype_of_its_own() {
    let price = DataType::from_logical_name("price").unwrap();
    assert_eq!(price.id(), DataType::decimal64(18, 8).unwrap().id());
    assert_eq!(price.kind(), DataType::decimal64(18, 8).unwrap().kind());
    let union: DataType = "union(dense,0=px: Price,1=ccy: Currency)".parse().unwrap();
    let DataType::Union(_, mode) = &union else {
        panic!("a union, got {union}");
    };
    assert_eq!(*mode, UnionMode::Dense);

    // The prebuilt vocabularies are keyed by the same names.
    for (name, _) in crate::AsciiEnum::PREBUILT {
        assert!(
            DataType::LOGICAL_NAMES
                .iter()
                .any(|(other, _)| other == name),
            "{name} prebuilds nothing registered"
        );
    }
}
