//! `rust/src/expression/serde.rs`: the edge cases this module is built to
//! get right.
//!
//! Four properties carry most of the weight, and each is asserted rather than
//! reviewed: text round-trips through the grammar, the scalar and vectorized
//! tiers agree on every operator including nulls and `nan`, a simplification
//! never changes what a row answers, and a pruning decision never loses a
//! row.

mod grammar {

    use yggdryl::expression::{Filter, Selector, Term};

    // ---------------------------------------------------------------------------
    // Text
    // ---------------------------------------------------------------------------

    /// Every spelling the grammar accepts, one of each shape.
    const CORPUS: [&str; 33] = [
        "ccy = 'EUR' and price > 100",
        "a or b and c",
        "(a or b) and c",
        "not a",
        "x is null",
        "x is not null",
        "x is distinct from y",
        "x is not distinct from y",
        "x in (1, 2, 3)",
        "x not in (1, 2)",
        "x between 1 and 10",
        "x not between 1 and 10",
        "name like 'a%' escape '\\'",
        "name ilike 'A%'",
        "name not like 'a%'",
        "path glob '**/*.parquet'",
        "lower(name) = 'x'",
        "coalesce(a, b, 0) > 1",
        "cast(x as int32) = 1",
        "try_cast(x as decimal128(9,2)) > decimal128(9,2) '1.50'",
        "case when a then 1 else 2 end",
        "struct(1 as a, 'b' as b)",
        "[1, 2, 3]",
        "{'a': 1}",
        "trade.legs[0]['ccy'] = 'EUR'",
        "trade.legs[1:3]",
        "trade.legs[:-1]",
        "lower(name)[0]",
        "-x + 3 * 2 - 1",
        ":since <= ts",
        "legs[ccy = 'EUR'][0].price",
        "legs[active]",
        "size(legs[notes[v > 1][0].k = 'b'][size >= :floor]) > 1",
    ];

    #[test]
    fn documents_round_trip() {
        for text in CORPUS {
            let parsed: Term = text.parse().unwrap();
            let document = parsed.clone().into_json().unwrap();
            assert_eq!(Term::from_json(&document).unwrap(), parsed, "{text}");
        }
        let selector: Selector = "a as b int64 not null, c + 1".parse().unwrap();
        let document = selector.clone().into_json().unwrap();
        assert_eq!(Selector::from_json(&document).unwrap(), selector);
        let filter: Filter = "a > 1".parse().unwrap();
        let document = filter.clone().into_json().unwrap();
        assert_eq!(Filter::from_json(&document).unwrap(), filter);
    }

    #[test]
    fn a_serie_constructor_written_as_list_still_reads() {
        // The `[a, b]` constructor was written under `list` before the family
        // took its own name; that document reads as the same term, and writes
        // back under `serie`.
        let parsed: Term = "[a, 2]".parse().unwrap();
        let document = parsed.clone().into_json().unwrap();
        assert!(document.contains("\"serie\""), "{document}");
        let legacy = document.replacen("\"serie\"", "\"list\"", 1);
        assert_eq!(Term::from_json(&legacy).unwrap(), parsed, "{legacy}");
        let filter: Filter = "[a, 2] = b".parse().unwrap();
        let legacy = filter
            .clone()
            .into_json()
            .unwrap()
            .replacen("\"serie\"", "\"list\"", 1);
        assert_eq!(Filter::from_json(&legacy).unwrap(), filter, "{legacy}");
    }
}

/// An epoch function serializes under its spelling, and `minutes` carries
/// its step as the argument it is, so a stored term reads in any language's
/// parser as it prints.
mod epoch_function_tags {
    use yggdryl::Term;

    #[test]
    fn the_tag_is_the_spelling_and_the_step_an_argument() {
        for (text, tag) in [("minutes(t, 15)", "minutes"), ("weeks(t)", "weeks")] {
            let term: Term = text.parse().unwrap();
            let document = term.clone().into_json().unwrap();
            let encoded = document.as_str();
            assert!(encoded.contains(&format!("\"{tag}\"")), "{encoded}");
            assert!(!encoded.contains("qhours"), "{encoded}");
            assert_eq!(Term::from_json(&document).unwrap(), term);
            assert_eq!(Term::from_json(&document).unwrap().to_string(), text);
        }
    }
}

/// A plan's joins serialize as one `joins` list, omitted when there is none,
/// each `{how, source, keys}`, a key the `{left, right}` pair of the terms'
/// own documents - the shape an `order by` key's `{term, ...}` has.
mod joins {
    use yggdryl::Expression;
    use yggdryl::expression::{JoinKey, Plan, Term};

    #[test]
    fn a_plan_with_joins_round_trips_its_document() {
        let plan: Plan = "select id, city from trades \
                          left join (select venue, city from venues where active) using (venue) \
                          anti join 'file:///lake/halted.parquet' on lower(id) = halted_id \
                          where city is not null"
            .parse()
            .unwrap();
        let document = plan.clone().into_json().unwrap();
        assert_eq!(Plan::from_json(&document).unwrap(), plan, "{document}");
        let value: serde_json::Value = serde_json::from_str(&document).unwrap();
        let joins = value["joins"].as_array().unwrap();
        assert_eq!(joins.len(), 2);
        assert_eq!(joins[0]["how"], "left");
        assert!(joins[0]["source"]["plan"].is_object(), "{document}");
        assert_eq!(joins[1]["how"], "anti");
        assert!(joins[1]["source"]["target"].is_object(), "{document}");
        let key = &joins[1]["keys"][0];
        let left: Term = serde_json::from_value(key["left"].clone()).unwrap();
        let right: Term = serde_json::from_value(key["right"].clone()).unwrap();
        assert_eq!(left.to_string(), "lower(id)");
        assert_eq!(right.to_string(), "halted_id");
        // A `using` key is the same term on both sides.
        let using: JoinKey = serde_json::from_value(joins[0]["keys"][0].clone()).unwrap();
        assert_eq!(using, JoinKey::using(Term::column("venue")));
        // The same through the expression, and no `joins` key without one.
        let expression = Expression::Plan(Box::new(plan));
        let document = expression.clone().into_json().unwrap();
        assert_eq!(Expression::from_json(&document).unwrap(), expression);
        let plain = "select * from trades"
            .parse::<Plan>()
            .unwrap()
            .into_json()
            .unwrap();
        assert!(!plain.contains("joins"), "{plain}");
    }
}
