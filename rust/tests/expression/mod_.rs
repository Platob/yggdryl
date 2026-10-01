//! `rust/src/expression/mod.rs`: the edge cases this module is built to get
//! right.
//!
//! Four properties carry most of the weight, and each is asserted rather than
//! reviewed: text round-trips through the grammar, the scalar and vectorized
//! tiers agree on every operator including nulls and `nan`, a simplification
//! never changes what a row answers, and a pruning decision never loses a
//! row.

mod grammar {

    use yggdryl::expression::{Expression, Filter, Selector};

    #[test]
    fn expressions_round_trip_and_name_their_clause() {
        for text in [
            "select *",
            "select a, b as c",
            "select lower(name) as name, price * 2 as doubled",
            "select i as total int64 not null, s utf8 null",
            "select nested.leg as leg, xs[1:3] as middle",
            "where a > 1",
            "where a = 1 and b is not null",
        ] {
            let parsed: Expression = text
                .parse()
                .unwrap_or_else(|error| panic!("{text}: {error}"));
            let printed = parsed.to_string();
            assert_eq!(printed, text, "{text} printed as {printed}");
            let again: Expression = printed.parse().unwrap();
            assert_eq!(parsed, again);
            let document = parsed.clone().into_json().unwrap();
            assert_eq!(Expression::from_json(&document).unwrap(), parsed, "{text}");
        }
        // Each clause is also its own type, and the keyword is optional there.
        let selector: Selector = "a, b as c".parse().unwrap();
        assert_eq!(selector.to_string(), "a, b as c");
        assert_eq!("select a, b as c".parse::<Selector>().unwrap(), selector);
        let filter: Filter = "a > 1".parse().unwrap();
        assert_eq!(filter.to_string(), "a > 1");
        assert_eq!("where a > 1".parse::<Filter>().unwrap(), filter);
        // An expression has to say which clause it is.
        let error = "a > 1".parse::<Expression>().unwrap_err().to_string();
        assert!(error.contains("select"), "{error}");
        assert!(error.contains("where"), "{error}");
        let error = "select".parse::<Expression>().unwrap_err().to_string();
        assert!(error.contains("projection"), "{error}");
    }

    #[test]
    fn unnest_is_one_of_the_closed_functions_under_its_duckdb_name() {
        use yggdryl::expression::Function;

        assert_eq!(Function::ALL.len(), 29);
        assert_eq!(Function::ALL.last(), Some(&Function::Unnest));
        assert_eq!(Function::Unnest.as_str(), "unnest");
        for spelling in ["unnest", "UNNEST", "explode"] {
            assert_eq!(
                Function::from_name(spelling),
                Some(Function::Unnest),
                "{spelling}"
            );
        }
        assert_eq!(Function::Unnest.arity(), (1, 1));
        assert!(Function::vocabulary().ends_with("slice, unnest"));
    }
}

/// The nine epoch functions: one plural spelling each, as Spark's Iceberg
/// DDL writes them, beside the four calendar parts they are not.
mod epoch_functions {
    use yggdryl::Term;
    use yggdryl::expression::Function;

    const NINE: [(Function, &str); 9] = [
        (Function::Years, "years"),
        (Function::Quarters, "quarters"),
        (Function::Months, "months"),
        (Function::Weeks, "weeks"),
        (Function::Days, "days"),
        (Function::Hours, "hours"),
        (Function::HalfHours, "hhours"),
        (Function::QuarterHours, "qhours"),
        (Function::Minutes, "minutes"),
    ];

    #[test]
    fn each_epoch_function_has_one_canonical_name_and_its_aliases() {
        for (function, name) in NINE {
            assert_eq!(function.as_str(), name);
            assert_eq!(Function::from_name(name), Some(function.clone()), "{name}");
            assert_eq!(
                Function::from_name(&name.to_ascii_uppercase()),
                Some(function.clone()),
                "{name}"
            );
            assert_eq!(function.arity(), (1, 1));
            assert!(function.is_epoch(), "{name}");
            assert!(!function.is_calendar(), "{name}");
            assert!(Function::ALL.contains(&function), "{name}");
            assert!(Function::vocabulary().contains(name), "{name}");
        }
        assert_eq!(
            Function::from_name("quarter_hours"),
            Some(Function::QuarterHours)
        );
        assert_eq!(Function::from_name("half_hours"), Some(Function::HalfHours));
        for calendar in [
            Function::Year,
            Function::Month,
            Function::Day,
            Function::Hour,
        ] {
            assert!(calendar.is_calendar());
            assert!(!calendar.is_epoch());
        }
    }

    #[test]
    fn an_epoch_call_parses_prints_and_serializes_under_its_name() {
        for (_, name) in NINE {
            let text = format!("{name}(ts) = 1");
            let term: Term = text.parse().unwrap();
            assert_eq!(term.to_string(), text);
            let document = term.clone().into_json().unwrap();
            let encoded = document.as_str();
            assert!(encoded.contains(&format!("\"{name}\"")), "{encoded}");
            assert_eq!(Term::from_json(&document).unwrap(), term, "{name}");
        }
        // The two spellings that are not the variant's own name still read
        // back under the canonical one.
        assert_eq!(
            "QUARTER_HOURS(ts)".parse::<Term>().unwrap().to_string(),
            "qhours(ts)"
        );
        assert_eq!(
            "half_hours(ts)".parse::<Term>().unwrap().to_string(),
            "hhours(ts)"
        );
    }
}
