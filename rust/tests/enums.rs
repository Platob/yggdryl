use std::collections::{BTreeSet, HashSet};

use arrow_schema::{IntervalUnit as ArrowIntervalUnit, TimeUnit as ArrowTimeUnit};
use yggdryl::{DataTypeId, DataTypeKind, Error, MediaType, MimeType, Scheme, TimeUnit, UnionMode};

#[path = "enums/mime.rs"]
mod mime;

#[test]
fn enums_module_reexports_the_public_vocabulary() {
    let id: yggdryl::enums::DataTypeId = DataTypeId::Int32;
    let kind: yggdryl::enums::DataTypeKind = DataTypeKind::Integer;
    let scheme: yggdryl::enums::Scheme = Scheme::HTTPS;
    let unit: yggdryl::enums::TimeUnit = TimeUnit::Nanosecond;
    let mode: yggdryl::enums::UnionMode = UnionMode::Dense;
    let mime: yggdryl::enums::MimeType = MimeType::JSON;
    let media: yggdryl::enums::MediaType = MediaType::from(mime.clone());

    assert_eq!(scheme, Scheme::HTTPS);
    assert_eq!(id, DataTypeId::Int32);
    assert_eq!(kind, DataTypeKind::Integer);
    assert_eq!(id.kind(), kind);
    assert_eq!(unit, TimeUnit::Nanosecond);
    assert_eq!(mode, UnionMode::Dense);
    assert_eq!(mime, MimeType::JSON);
    assert_eq!(media.base(), &MimeType::JSON);
}

#[test]
fn every_known_scheme_parses_to_its_static_value() {
    for (source, expected) in [
        ("HTTP", Scheme::HTTP),
        ("HTTPS", Scheme::HTTPS),
        ("FILE", Scheme::FILE),
        ("URN", Scheme::URN),
        ("POSTGRES", Scheme::POSTGRES),
        ("POSTGRESQL", Scheme::POSTGRESQL),
        ("MYSQL", Scheme::MYSQL),
        ("ARROW", Scheme::ARROW),
        ("SQL", Scheme::SQL),
        ("GLUE", Scheme::GLUE),
        ("ICEBERG", Scheme::ICEBERG),
        ("FIX", Scheme::FIX),
        ("FIELD", Scheme::FIELD),
        ("DTYPE", Scheme::DTYPE),
        ("S3", Scheme::S3),
        ("GS", Scheme::GS),
    ] {
        let parsed = Scheme::from_str(source).unwrap();
        assert_eq!(parsed, expected);
        assert!(parsed.is_known());
        assert_eq!(parsed.as_str(), source.to_ascii_lowercase());
    }
}

#[test]
fn known_and_custom_schemes_share_canonical_value_semantics() {
    let known = Scheme::from_str("POSTGRES").unwrap();
    let custom = Scheme::from_str("Acme+Wire").unwrap();

    assert_eq!(known, Scheme::POSTGRES);
    assert!(known.is_known());
    assert_eq!(custom.as_str(), "acme+wire");
    assert!(!custom.is_known());
    let displayed = custom.to_string();
    assert_eq!(displayed, "acme+wire");
    assert_eq!(Scheme::from_str(&displayed).unwrap(), custom);
    assert_eq!(
        serde_json::from_str::<Scheme>(r#""S3""#).unwrap(),
        Scheme::S3
    );
    assert_eq!(
        serde_json::from_str::<Scheme>(&serde_json::to_string(&custom).unwrap()).unwrap(),
        custom
    );
    assert_eq!(
        custom.stable_hash(),
        Scheme::from_str("ACME+WIRE").unwrap().stable_hash()
    );

    assert!(BTreeSet::from([known.clone()]).contains(&known));
    assert!(HashSet::from([custom.clone()]).contains(&custom));
}

#[test]
fn scheme_validation_reports_the_original_byte() {
    for invalid in ["", "1http", "http_compat", "http://"] {
        assert!(Scheme::from_str(invalid).is_err(), "{invalid:?}");
    }

    let error = Scheme::from_str("http_").unwrap_err();
    assert!(matches!(
        error,
        yggdryl::Error::Parse {
            target: "scheme",
            position: 4,
            ..
        }
    ));
}

#[test]
fn shared_static_enums_have_canonical_strings_and_serde() {
    assert_eq!(UnionMode::Dense.as_str(), "dense");
    assert_eq!(
        serde_json::from_str::<UnionMode>("\"sparse\"").unwrap(),
        UnionMode::Sparse
    );
}

#[test]
fn every_time_unit_parses_displays_and_serializes_canonically() {
    for (unit, canonical, json) in [
        (TimeUnit::Second, "s", "\"second\""),
        (TimeUnit::Millisecond, "ms", "\"millisecond\""),
        (TimeUnit::Microsecond, "us", "\"microsecond\""),
        (TimeUnit::Nanosecond, "ns", "\"nanosecond\""),
        (TimeUnit::YearMonth, "year_month", "\"year_month\""),
        (TimeUnit::DayTime, "day_time", "\"day_time\""),
        (
            TimeUnit::MonthDayNano,
            "month_day_nano",
            "\"month_day_nano\"",
        ),
    ] {
        assert_eq!(unit.as_str(), canonical);
        let borrowed: &str = unit.as_ref();
        assert_eq!(borrowed, canonical);
        assert!(std::ptr::eq(borrowed, unit.as_str()));
        assert_eq!(unit.to_string(), canonical);
        assert_eq!(TimeUnit::from_str(canonical).unwrap(), unit);
        assert_eq!(canonical.parse::<TimeUnit>().unwrap(), unit);

        assert_eq!(serde_json::to_string(&unit).unwrap(), json);
        assert_eq!(serde_json::from_str::<TimeUnit>(json).unwrap(), unit);
    }
}

#[test]
fn time_unit_parser_accepts_arrow_names_and_common_full_aliases() {
    for (source, expected) in [
        ("S", TimeUnit::Second),
        ("sec", TimeUnit::Second),
        ("Second", TimeUnit::Second),
        ("  Second\t", TimeUnit::Second),
        ("seconds", TimeUnit::Second),
        ("Millisecond", TimeUnit::Millisecond),
        ("MILLIS", TimeUnit::Millisecond),
        ("Microsecond", TimeUnit::Microsecond),
        ("micro seconds", TimeUnit::Microsecond),
        ("\u{b5}S", TimeUnit::Microsecond),
        ("\u{3bc}S", TimeUnit::Microsecond),
        ("Nanosecond", TimeUnit::Nanosecond),
        ("Nanoseconds", TimeUnit::Nanosecond),
        ("YearMonth", TimeUnit::YearMonth),
        ("year", TimeUnit::YearMonth),
        ("year_month", TimeUnit::YearMonth),
        ("year-month", TimeUnit::YearMonth),
        ("YEAR TO MONTH", TimeUnit::YearMonth),
        ("years-to-months", TimeUnit::YearMonth),
        ("DayTime", TimeUnit::DayTime),
        ("day", TimeUnit::DayTime),
        ("DAY_TIME", TimeUnit::DayTime),
        ("DAY TO SECOND", TimeUnit::DayTime),
        ("days to seconds", TimeUnit::DayTime),
        ("MonthDayNano", TimeUnit::MonthDayNano),
        ("month_day_nano", TimeUnit::MonthDayNano),
        ("month-day-nanosecond", TimeUnit::MonthDayNano),
    ] {
        assert_eq!(TimeUnit::from_str(source).unwrap(), expected, "{source:?}");
    }
}

#[test]
fn time_unit_serde_deserialization_reuses_the_public_parser() {
    for (json, expected) in [
        (r#""us""#, TimeUnit::Microsecond),
        (r#""MICROSECONDS""#, TimeUnit::Microsecond),
        (r#""MonthDayNano""#, TimeUnit::MonthDayNano),
    ] {
        assert_eq!(serde_json::from_str::<TimeUnit>(json).unwrap(), expected);
    }

    assert_eq!(
        serde_json::to_string(&TimeUnit::Microsecond).unwrap(),
        r#""microsecond""#
    );
}

#[test]
fn time_unit_parser_trims_boundaries_and_reports_original_junk_offsets() {
    for (source, expected_position) in [
        ("", 0),
        ("second!", 6),
        ("month/day/nano", 5),
        ("  ms!", 4),
        (" second! ", 7),
        ("\tmonth/day/nano ", 6),
        ("\u{b5}S!", 3),
        ("\u{3bc}S!", 3),
    ] {
        let error = TimeUnit::from_str(source).unwrap_err();
        assert!(
            matches!(
                &error,
                Error::Parse {
                    target: "time unit",
                    position,
                    ..
                } if *position == expected_position
            ),
            "unexpected error for {source:?}: {error}"
        );
    }
}

#[test]
fn time_unit_values_have_total_value_semantics_and_categories() {
    let temporal = [
        TimeUnit::Second,
        TimeUnit::Millisecond,
        TimeUnit::Microsecond,
        TimeUnit::Nanosecond,
    ];
    let interval = [
        TimeUnit::YearMonth,
        TimeUnit::DayTime,
        TimeUnit::MonthDayNano,
    ];

    for unit in temporal {
        assert!(unit.is_temporal());
        assert!(!unit.is_interval());
    }
    for unit in interval {
        assert!(!unit.is_temporal());
        assert!(unit.is_interval());
    }

    let ordered = BTreeSet::from([
        TimeUnit::MonthDayNano,
        TimeUnit::Second,
        TimeUnit::DayTime,
        TimeUnit::Nanosecond,
        TimeUnit::YearMonth,
        TimeUnit::Microsecond,
        TimeUnit::Millisecond,
    ]);
    assert_eq!(
        ordered.into_iter().collect::<Vec<_>>(),
        [
            TimeUnit::Second,
            TimeUnit::Millisecond,
            TimeUnit::Microsecond,
            TimeUnit::Nanosecond,
            TimeUnit::YearMonth,
            TimeUnit::DayTime,
            TimeUnit::MonthDayNano,
        ]
    );
    let hashed = HashSet::from([
        TimeUnit::Second,
        TimeUnit::Millisecond,
        TimeUnit::Microsecond,
        TimeUnit::Nanosecond,
        TimeUnit::YearMonth,
        TimeUnit::DayTime,
        TimeUnit::MonthDayNano,
    ]);
    assert_eq!(hashed.len(), 7);
    assert!(hashed.contains(&TimeUnit::DayTime));
}

#[test]
fn time_units_interoperate_with_both_arrow_unit_families() {
    for (native, arrow) in [
        (TimeUnit::Second, ArrowTimeUnit::Second),
        (TimeUnit::Millisecond, ArrowTimeUnit::Millisecond),
        (TimeUnit::Microsecond, ArrowTimeUnit::Microsecond),
        (TimeUnit::Nanosecond, ArrowTimeUnit::Nanosecond),
    ] {
        assert_eq!(TimeUnit::from(arrow), native);
        assert_eq!(TimeUnit::from_arrow_time(arrow), native);
        assert_eq!(ArrowTimeUnit::try_from(native).unwrap(), arrow);
        assert_eq!(native.into_arrow_time().unwrap(), arrow);
        assert!(matches!(
            ArrowIntervalUnit::try_from(native),
            Err(Error::InvalidDataType { .. })
        ));
        assert!(matches!(
            native.into_arrow_interval(),
            Err(Error::InvalidDataType { .. })
        ));
    }

    for (native, arrow) in [
        (TimeUnit::YearMonth, ArrowIntervalUnit::YearMonth),
        (TimeUnit::DayTime, ArrowIntervalUnit::DayTime),
        (TimeUnit::MonthDayNano, ArrowIntervalUnit::MonthDayNano),
    ] {
        assert_eq!(TimeUnit::from(arrow), native);
        assert_eq!(TimeUnit::from_arrow_interval(arrow), native);
        assert_eq!(ArrowIntervalUnit::try_from(native).unwrap(), arrow);
        assert_eq!(native.into_arrow_interval().unwrap(), arrow);
        assert!(matches!(
            ArrowTimeUnit::try_from(native),
            Err(Error::InvalidDataType { .. })
        ));
        assert!(matches!(
            native.into_arrow_time(),
            Err(Error::InvalidDataType { .. })
        ));
    }
}
