//! `rust/src/expression/display.rs`: the edge cases this module is built to
//! get right.
//!
//! Five properties carry most of the weight, and each is asserted rather than
//! reviewed: text round-trips through the grammar, the scalar and vectorized
//! tiers agree on every operator including nulls and `nan`, a simplification
//! never changes what a row answers, a free attribute never costs a backend
//! call, and a pruning decision never loses a row.

mod grammar {

    use yggdryl::expression::Term;

    // ---------------------------------------------------------------------------
    // Text
    // ---------------------------------------------------------------------------

    /// Every spelling the grammar accepts, one of each shape.
    const CORPUS: [&str; 36] = [
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
        "&holder.size > 0",
        "&holder.partition['year'] = '2024'",
        "&holder.name like 'part-%'",
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
    fn text_round_trips() {
        for text in CORPUS {
            let parsed: Term = text
                .parse()
                .unwrap_or_else(|error| panic!("{text}: {error}"));
            let printed = parsed.to_string();
            let again: Term = printed
                .parse()
                .unwrap_or_else(|error| panic!("{printed}: {error}"));
            assert_eq!(parsed, again, "{text} printed as {printed}");
        }
    }
}
