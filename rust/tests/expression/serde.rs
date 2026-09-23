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
