//! `rust/src/regex.rs`.

mod fractions {

    use yggdryl::{DataType, TimeUnit, Timezone};

    #[test]
    fn a_fraction_names_its_unit_at_either_decimal_sign_and_any_width() {
        let dtype = DataType::from_regex(
            concat!(
                r"(?<comma>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3}) ",
                r"(?<either>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}[.,]\d{3}) ",
                r"(?<grouped>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3}_\d{3}) ",
                r"(?<variable>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{1,9}) ",
                r"(?<narrow>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{1,5}) ",
                r"(?<four>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{4}) ",
                r"(?<clock>\d{2}:\d{2}:\d{2},\d{3}) ",
                r"(?<grouping>\d{1,3}(?:,\d{3})*) ",
                r"(?<decimal>\d+,\d+)"
            ),
            true,
        )
        .unwrap();
        let naive = |unit| DataType::DateTime64 {
            unit,
            timezone: Timezone::NAIVE,
        };

        // ISO 8601 names the comma and the full stop alike, so a log4j rowheader
        // is a timestamp column and not the string a capture falls back to when
        // nothing recognizes its syntax. A class over the two signs is the same
        // clock, and grouping widens the unit exactly as the full stop's does.
        assert_eq!(
            dtype.field("comma").unwrap().dtype(),
            &naive(TimeUnit::Millisecond)
        );
        assert_eq!(
            dtype.field("either").unwrap().dtype(),
            &naive(TimeUnit::Millisecond)
        );
        assert_eq!(
            dtype.field("grouped").unwrap().dtype(),
            &naive(TimeUnit::Microsecond)
        );

        // A capture admitting several widths publishes the widest, which is the
        // only resolution that holds every row it admits: at milliseconds a
        // five-digit row is no exact count and the reader would refuse it.
        assert_eq!(
            dtype.field("variable").unwrap().dtype(),
            &naive(TimeUnit::Nanosecond)
        );
        assert_eq!(
            dtype.field("narrow").unwrap().dtype(),
            &naive(TimeUnit::Microsecond)
        );

        // Every width between one and nine names a unit; the probe is no longer a
        // handful of spellings a pattern has to match exactly.
        assert_eq!(
            dtype.field("four").unwrap().dtype(),
            &naive(TimeUnit::Microsecond)
        );
        assert_eq!(
            dtype.field("clock").unwrap().dtype(),
            &DataType::time(TimeUnit::Millisecond).unwrap()
        );

        // The comma is a decimal sign inside a clock and nothing outside one: a
        // thousands group and a European decimal carry no calendar and no clock,
        // so they match no temporal spelling and stay text. Reading either as a
        // number is a separate decision this one does not make.
        assert_eq!(dtype.field("grouping").unwrap().dtype(), &DataType::utf8());
        assert_eq!(dtype.field("decimal").unwrap().dtype(), &DataType::utf8());
    }
}

mod captures {
    use yggdryl::{DataType, Error, Field, TimeUnit, Timezone};

    #[test]
    fn named_captures_keep_order_nullability_and_format_types() {
        let dtype = DataType::from_regex(
            concat!(
                r"(?<enabled>true|false) ",
                r"(?<id>[-+]?\d+) ",
                r"(?<price>[-+]?\d+\.\d+) ",
                r"(?<date>\d{4}-\d{2}-\d{2}) ",
                r"(?<clock>\d{2}:\d{2}:\d{2}\.\d{6}) ",
                r"(?<stamp>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z) ",
                r"(?<text>\S+)"
            ),
            true,
        )
        .unwrap();
        let fields = dtype.as_fields().unwrap();
        assert_eq!(
            fields.iter().map(Field::name).collect::<Vec<_>>(),
            ["enabled", "id", "price", "date", "clock", "stamp", "text"]
        );
        assert_eq!(dtype.field("enabled").unwrap().dtype(), &DataType::Boolean);
        assert_eq!(dtype.field("id").unwrap().dtype(), &DataType::Int64);
        assert_eq!(dtype.field("price").unwrap().dtype(), &DataType::Float64);
        assert_eq!(dtype.field("date").unwrap().dtype(), &DataType::date32());
        assert_eq!(
            dtype.field("clock").unwrap().dtype(),
            &DataType::Time64(TimeUnit::Microsecond)
        );
        assert_eq!(
            dtype.field("stamp").unwrap().dtype(),
            &DataType::DateTime64 {
                unit: TimeUnit::Second,
                timezone: Timezone::UTC,
            }
        );
        assert_eq!(dtype.field("text").unwrap().dtype(), &DataType::utf8());
        assert!(fields.iter().all(Field::is_nullable));
    }

    #[test]
    fn disabling_autotype_keeps_every_capture_utf8() {
        let dtype = DataType::from_regex(r"(?<id>\d+)-(?<flag>true|false)", false).unwrap();
        assert!(
            dtype
                .as_fields()
                .unwrap()
                .iter()
                .all(|field| field.dtype() == &DataType::utf8())
        );
    }

    #[test]
    fn malformed_and_over_nested_regexes_are_typed_errors() {
        assert!(matches!(
            DataType::from_regex("(?<id>", true),
            Err(Error::InvalidDataType { kind: "regex", .. })
        ));
        let nested = format!(
            "{}x{}",
            "(?:".repeat(DataType::PARSE_RECURSION_LIMIT + 1),
            ")".repeat(DataType::PARSE_RECURSION_LIMIT + 1)
        );
        assert!(matches!(
            DataType::from_regex(&nested, true),
            Err(Error::InvalidDataType { kind: "regex", .. })
        ));
    }
}
