//! `rust/src/expression/parser.rs`: the edge cases this module is built to
//! get right.
//!
//! Four properties carry most of the weight, and each is asserted rather than
//! reviewed: text round-trips through the grammar, the scalar and vectorized
//! tiers agree on every operator including nulls and `nan`, a simplification
//! never changes what a row answers, and a pruning decision never loses a
//! row.

mod grammar {

    use yggdryl::expression::{Expression, Term};
    use yggdryl::{DataType, Scalar};

    /// The value a typed literal holds.
    fn literal_value(text: &str) -> Scalar {
        let term: Term = text
            .parse()
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        term.as_literal()
            .unwrap_or_else(|| panic!("{text}: not a literal"))
            .value()
            .clone()
    }

    #[test]
    fn a_typed_literal_reads_its_text_as_the_datatype_reads_it() {
        // A flag reads the one table a column of text is cast through.
        assert_eq!(literal_value("boolean 'yes'"), Scalar::from(true));
        assert_eq!(literal_value("boolean ' TRUE '"), Scalar::from(true));
        assert_eq!(literal_value("bool 'N'"), Scalar::from(false));
        assert_eq!(literal_value("bool 'off'"), Scalar::from(false));
        // An integer reads a signed, trimmed spelling and keeps the width.
        assert_eq!(literal_value("int32 ' 5 '"), Scalar::from(5_i32));
        assert_eq!(literal_value("int64 '+5'"), Scalar::from(5_i64));
        assert_eq!(literal_value("uint8 ' 200 '"), Scalar::from(200_u8));
        // A float reads what a column of text is cast through.
        assert_eq!(literal_value("float64 ' 1.5 '"), Scalar::from(1.5_f64));
        assert_eq!(literal_value("float32 '1e3'"), Scalar::from(1000.0_f32));
        assert_eq!(literal_value("float64 'inf'"), Scalar::from(f64::INFINITY));
        // A decimal reads an exponent and surrounding blanks at the declared
        // scale.
        assert_eq!(
            literal_value("decimal128(10,2) '1e2'"),
            Scalar::decimal128(10_000, 2)
        );
        assert_eq!(
            literal_value("decimal128(10,2) ' 1.50 '"),
            Scalar::decimal128(150, 2)
        );
        // A coefficient past 128 bits is a decimal256's, read exactly and
        // printed back as the text it was written from.
        let wide: Term = "decimal256(76,2) '340282366920938463463374607431768211456.25'"
            .parse()
            .unwrap();
        let literal = wide.as_literal().unwrap();
        assert_eq!(literal.dtype(), &DataType::decimal256(76, 2).unwrap());
        assert!(
            wide.to_string()
                .contains("340282366920938463463374607431768211456.25"),
            "{wide}"
        );
        assert_eq!(wide.to_string().parse::<Term>().unwrap(), wide);
    }

    #[test]
    fn a_typed_literal_refuses_text_its_datatype_cannot_hold_where_it_was_written() {
        for text in [
            "boolean 'maybe'",
            "int8 '1000'",
            "int32 '1.0'",
            "int32 'x'",
            "uint8 '-1'",
            "float64 'x'",
            "decimal128(10,2) '1.555'",
            // A coefficient past what the width holds was read as zero once.
            "decimal128(38,0) '340282366920938463463374607431768211456'",
        ] {
            let error = text.parse::<Term>().unwrap_err().to_string();
            assert!(error.contains("at byte "), "{text}: {error}");
        }
        // The refusal names the table a flag is read from and the text.
        let error = "boolean 'maybe'".parse::<Term>().unwrap_err().to_string();
        assert!(error.contains("yes/no"), "{error}");
        assert!(error.contains("maybe"), "{error}");
    }

    /// A typed decimal literal reads through the one decimal text door:
    /// every exact spelling at any width, printed as the shortest text, and
    /// a whole part past the width refused rather than read as nothing.
    #[test]
    fn a_typed_decimal_literal_reads_every_exact_spelling() {
        let printed = |text: &str| text.parse::<Term>().unwrap().to_string();
        assert_eq!(printed("decimal128(9,2) '1e2'"), "decimal128(9,2) '100'");
        assert_eq!(printed("decimal128(9,2) ' 1.50 '"), "decimal128(9,2) '1.5'");
        assert_eq!(
            printed("decimal128(9,2) '1_000.5'"),
            "decimal128(9,2) '1000.5'"
        );
        let wide = format!("{}.5", "1".repeat(60));
        assert_eq!(
            printed(&format!("decimal256(76,2) '{wide}'")),
            format!("decimal256(76,2) '{wide}'")
        );
        for refused in [
            format!("decimal128(10,2) '{}.50'", "9".repeat(41)),
            "decimal128(9,2) '1.005'".to_owned(),
            "decimal128(9,2) '1,000'".to_owned(),
        ] {
            assert!(refused.parse::<Term>().is_err(), "{refused}");
        }
    }

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
    fn a_bracketed_location_part_reads_its_raw_text() {
        use yggdryl::expression::{Location, Plan, Source};

        let plan: Plan = "select * from [R&D].t".parse().unwrap();
        let Some(Source::Target(target)) = plan.source() else {
            panic!("expected a target source, got {:?}", plan.source());
        };
        assert_eq!(target.location(), &Location::parts(["R&D", "t"]));
        // `&` is tokenized for that reading alone; no term takes it.
        assert!("a & b".parse::<Term>().is_err());
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

mod joins {
    use yggdryl::JoinKind;
    use yggdryl::expression::{Expression, JoinKey, Plan, Source, Term};

    /// Parse, print, parse again: the second reading is the first.
    fn round_trip(text: &str) -> Plan {
        let parsed: Plan = text
            .parse()
            .unwrap_or_else(|error| panic!("{text}: {error}"));
        let printed = parsed.to_string();
        let again: Plan = printed
            .parse()
            .unwrap_or_else(|error| panic!("{printed}: {error}"));
        assert_eq!(parsed, again, "{text} printed as {printed}");
        parsed
    }

    #[test]
    fn a_join_clause_without_its_keys_is_refused_naming_what_was_expected() {
        for (text, expected) in [
            ("select * from trades join venues", "`on` or `using`"),
            (
                "select * from trades left join venues where id = 1",
                "`on` or `using`",
            ),
            (
                "select * from trades join venues using ()",
                "expected a name",
            ),
            ("select * from trades join venues using id", "\"(\""),
            (
                "select * from trades join venues on",
                "the end of the expression",
            ),
            ("select * from trades join", "a location"),
        ] {
            let error = text.parse::<Plan>().unwrap_err().to_string();
            assert!(error.contains(expected), "{text}: {error}");
            assert!(error.contains("at byte "), "{text}: {error}");
        }
    }

    #[test]
    fn an_on_conjunct_that_is_not_an_equality_is_refused_by_name() {
        for (text, named) in [
            ("select * from t join v on id = vid and px > 1", "px > 1"),
            ("select * from t join v on id", "id"),
            (
                "select * from t join v on id = vid or a = b",
                "id = vid or a = b",
            ),
            ("select * from t join v on id != vid", "id <> vid"),
        ] {
            let error = text.parse::<Plan>().unwrap_err().to_string();
            assert!(error.contains(&format!("`{named}`")), "{text}: {error}");
            assert!(error.contains("equalit"), "{text}: {error}");
        }
    }

    #[test]
    fn every_join_kind_prints_one_way_and_reads_back() {
        for (text, canonical, how) in [
            ("join", "inner join", JoinKind::Inner),
            ("inner join", "inner join", JoinKind::Inner),
            ("INNER JOIN", "inner join", JoinKind::Inner),
            ("left join", "left join", JoinKind::Left),
            ("left outer join", "left join", JoinKind::Left),
            ("right join", "right join", JoinKind::Right),
            ("Right Outer Join", "right join", JoinKind::Right),
            ("full join", "full join", JoinKind::Full),
            ("full outer join", "full join", JoinKind::Full),
            ("outer join", "full join", JoinKind::Full),
            ("semi join", "semi join", JoinKind::Semi),
            ("anti join", "anti join", JoinKind::Anti),
        ] {
            let plan = round_trip(&format!("select * from trades {text} venues using (venue)"));
            assert_eq!(
                plan.to_string(),
                format!("select * from trades {canonical} venues using (venue)"),
                "{text}"
            );
            let [join] = plan.joins() else {
                panic!("{text}: expected one join, got {:?}", plan.joins());
            };
            assert_eq!(join.how(), how, "{text}");
            assert_eq!(join.keys().to_string(), "venue");
        }
    }

    #[test]
    fn using_names_its_columns_and_on_its_equalities() {
        let plan = round_trip("select * from trades join venues using (venue, \"odd name\")");
        assert_eq!(
            plan.to_string(),
            "select * from trades inner join venues using (venue, \"odd name\")"
        );
        let keys = plan.joins()[0].keys();
        assert!(keys.iter().all(JoinKey::is_using));
        assert_eq!(keys.keys()[1].using_column(), Some("odd name"));
        // Two equalities are two keys; an `on` of bare columns on both sides
        // is the same plan `using` spells.
        let plan = round_trip(
            "select * from trades join venues on venue = mic and lower(desk) = desk where px > 0",
        );
        assert_eq!(
            plan.to_string(),
            "from trades inner join venues on venue = mic and lower(desk) = desk where px > 0"
        );
        let keys = plan.joins()[0].keys();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys.keys()[0].left().to_string(), "venue");
        assert_eq!(keys.keys()[0].right().to_string(), "mic");
        assert_eq!(keys.keys()[1].left().to_string(), "lower(desk)");
        assert_eq!(
            round_trip("select * from t join v on id = id and k = k"),
            round_trip("select * from t join v using (id, k)")
        );
        // A key mixing a pair and a shared column prints every key as an
        // equality.
        let plan = round_trip("select * from t join v on id = id and venue = mic");
        assert_eq!(
            plan.to_string(),
            "select * from t inner join v on id = id and venue = mic"
        );
        // An `or` inside one side stays inside it.
        let plan = round_trip("select * from t join v on (a or b) = flag");
        assert_eq!(
            plan.joins()[0].keys().keys()[0].left(),
            &"a or b".parse::<Term>().unwrap()
        );
    }

    #[test]
    fn a_plan_joining_on_a_shared_equality_reads_back_as_it_prints() {
        // The key list's own spelling is `tests/expression/join.rs`'s; here
        // the `on` clause carries it.
        let shared = JoinKey::using("a = b".parse::<Term>().unwrap());
        let plan = Plan::new()
            .read_from(Source::Plan(Box::new("select * from t".parse().unwrap())))
            .join(
                JoinKind::Inner,
                "select * from v".parse::<Plan>().unwrap(),
                [shared],
            )
            .unwrap();
        assert_eq!(
            plan.to_string(),
            "select * from (select * from t) inner join (select * from v) on (a = b) = (a = b)"
        );
        assert_eq!(round_trip(&plan.to_string()), plan);
    }

    #[test]
    fn joins_chain_left_to_right_over_targets_and_nested_plans() {
        let plan = round_trip(
            "select id, city from 'file:///lake/trades.parquet' with (media_type = 'application/vnd.apache.parquet') \
             left join (select venue, city from venues where active) using (venue) \
             semi join lake.flags on id = trade_id \
             where city is not null order by id limit 3",
        );
        assert_eq!(
            plan.to_string(),
            "select id, city from 'file:///lake/trades.parquet' with (media_type = \
             'application/vnd.apache.parquet') left join (select venue, city from venues where \
             active) using (venue) semi join lake.flags on id = trade_id where city is not null \
             order by id limit 3"
        );
        let [left, semi] = plan.joins() else {
            panic!("expected two joins, got {:?}", plan.joins());
        };
        assert_eq!(left.how(), JoinKind::Left);
        assert!(matches!(left.source(), Source::Plan(_)));
        assert_eq!(semi.how(), JoinKind::Semi);
        assert_eq!(semi.source().to_string(), "lake.flags");
        // A plan in a sequence keeps its joins.
        let expression: Expression = "select * from t join v using (id); select id"
            .parse()
            .unwrap();
        assert_eq!(
            expression.to_string().parse::<Expression>().unwrap(),
            expression
        );
    }
}
