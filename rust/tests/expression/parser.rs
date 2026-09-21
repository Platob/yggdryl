//! `rust/src/expression/parser.rs`: the edge cases this module is built to
//! get right.
//!
//! Five properties carry most of the weight, and each is asserted rather than
//! reviewed: text round-trips through the grammar, the scalar and vectorized
//! tiers agree on every operator including nulls and `nan`, a simplification
//! never changes what a row answers, a free attribute never costs a backend
//! call, and a pruning decision never loses a row.

mod grammar {

    use yggdryl::expression::{Expression, Term};

    #[test]
    fn quoted_names_survive_every_encapsulator() {
        for text in ["\"odd name\" = 1", "`odd name` = 1"] {
            let parsed: Term = text.parse().unwrap();
            assert_eq!(parsed.columns(), vec!["odd name".to_owned()]);
            assert_eq!(parsed.to_string(), "\"odd name\" = 1");
        }
        // A doubled quote inside a quoted name is one quote, as SQL spells it.
        let parsed: Term = "\"say \"\"hi\"\"\" = 1".parse().unwrap();
        assert_eq!(parsed.columns(), vec!["say \"hi\"".to_owned()]);
        assert_eq!(parsed.to_string().parse::<Term>().unwrap(), parsed);
        // A reserved word is a column only when quoted, and prints quoted.
        let parsed: Term = "\"select\" = 1".parse().unwrap();
        assert_eq!(parsed.columns(), vec!["select".to_owned()]);
        assert_eq!(parsed.to_string(), "\"select\" = 1");
    }

    #[test]
    fn a_parse_failure_names_where_it_stopped() {
        let error = "a = ".parse::<Term>().unwrap_err();
        assert!(
            format!("{error}").contains("at byte 4"),
            "expected a byte position, got {error}"
        );
        let error = "a === 1".parse::<Term>().unwrap_err();
        assert!(format!("{error}").contains("at byte "), "{error}");
        let error = "nosuchfn(a)".parse::<Term>().unwrap_err();
        assert!(format!("{error}").contains("lower"), "{error}");
        let error = "&holder.nosuch".parse::<Term>().unwrap_err();
        assert!(format!("{error}").contains("partition"), "{error}");
        let error = "a in ()".parse::<Term>().unwrap_err();
        assert!(format!("{error}").contains("at least one"), "{error}");
        let error = "select a as".parse::<Expression>().unwrap_err();
        assert!(format!("{error}").contains("at byte "), "{error}");
    }

    #[test]
    fn nesting_past_the_limit_is_refused_not_crashed() {
        let deep = format!(
            "{}a{}",
            "(".repeat(yggdryl::expression::RECURSION_LIMIT + 8),
            ")".repeat(yggdryl::expression::RECURSION_LIMIT + 8)
        );
        let error = deep.parse::<Term>().unwrap_err();
        assert!(format!("{error}").contains("hard limit"), "{error}");
    }
}
