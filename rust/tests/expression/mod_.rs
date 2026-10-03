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

        assert_eq!(Function::ALL.len(), 27);
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

mod comparison {
    //! `Comparison::from_str`: the operator table the parser reads.

    use yggdryl::expression::Comparison;

    #[test]
    fn a_comparison_reads_every_operator_the_grammar_reads() {
        for (text, comparison) in [
            ("=", Comparison::Eq),
            (" = ", Comparison::Eq),
            ("<>", Comparison::NotEq),
            ("!=", Comparison::NotEq),
            ("<", Comparison::Lt),
            ("<=", Comparison::LtEq),
            (">", Comparison::Gt),
            (">=", Comparison::GtEq),
            ("is distinct from", Comparison::IsDistinctFrom),
            ("IS DISTINCT FROM", Comparison::IsDistinctFrom),
            ("is not distinct from", Comparison::IsNotDistinctFrom),
            ("is  not\tdistinct from", Comparison::IsNotDistinctFrom),
        ] {
            assert_eq!(
                text.parse::<Comparison>().expect(text),
                comparison,
                "{text:?}"
            );
        }
        for comparison in Comparison::ALL {
            assert_eq!(
                comparison.as_str().parse::<Comparison>().unwrap(),
                comparison
            );
        }
        for text in [
            "approximately",
            "==",
            "=<",
            "",
            "is",
            "is distinct",
            "= 1",
            "a = b",
        ] {
            let refused = text.parse::<Comparison>().expect_err(text).to_string();
            assert!(
                refused.contains("comparison")
                    || refused.contains("end of the expression")
                    || refused.contains("expected"),
                "{text:?}: {refused}"
            );
        }
    }
}

/// The seven epoch functions: one plural spelling each, as Spark's Iceberg
/// DDL writes them, beside the four calendar parts they are not. `minutes`
/// alone takes a second argument, the step it always states.
mod epoch_functions {
    use yggdryl::Term;
    use yggdryl::expression::Function;

    const SEVEN: [(Function, &str); 7] = [
        (Function::Years, "years"),
        (Function::Quarters, "quarters"),
        (Function::Months, "months"),
        (Function::Weeks, "weeks"),
        (Function::Days, "days"),
        (Function::Hours, "hours"),
        (Function::Minutes, "minutes"),
    ];

    #[test]
    fn each_epoch_function_has_one_canonical_name() {
        for (function, name) in SEVEN {
            assert_eq!(function.as_str(), name);
            assert_eq!(Function::from_name(name), Some(function.clone()), "{name}");
            assert_eq!(
                Function::from_name(&name.to_ascii_uppercase()),
                Some(function.clone()),
                "{name}"
            );
            let arity = if function == Function::Minutes {
                (2, 2)
            } else {
                (1, 1)
            };
            assert_eq!(function.arity(), arity, "{name}");
            assert!(function.is_epoch(), "{name}");
            assert!(!function.is_calendar(), "{name}");
            assert!(Function::ALL.contains(&function), "{name}");
            assert!(Function::vocabulary().contains(name), "{name}");
        }
        // The fixed sub-hour spellings are gone: a step is `minutes(x, n)`.
        for retired in ["qhours", "hhours", "quarter_hours", "half_hours", "minute"] {
            assert_eq!(Function::from_name(retired), None, "{retired}");
            assert!(
                !Function::vocabulary()
                    .split(", ")
                    .any(|name| name == retired),
                "{retired}"
            );
        }
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
        for (function, name) in SEVEN {
            let text = if function == Function::Minutes {
                format!("{name}(ts, 15) = 1")
            } else {
                format!("{name}(ts) = 1")
            };
            let term: Term = text.parse().unwrap();
            assert_eq!(term.to_string(), text);
            let document = term.clone().into_json().unwrap();
            let encoded = document.as_str();
            assert!(encoded.contains(&format!("\"{name}\"")), "{encoded}");
            assert_eq!(Term::from_json(&document).unwrap(), term, "{name}");
        }
        // `minutes` always states its step, and states one step.
        for text in [
            "minutes(ts)",
            "minutes(ts, 15, 2)",
            "qhours(ts)",
            "hhours(ts)",
        ] {
            let error = text.parse::<Term>().unwrap_err().to_string();
            assert!(
                error.contains(&text[..text.find('(').unwrap()]),
                "{text}: {error}"
            );
        }
    }
}
